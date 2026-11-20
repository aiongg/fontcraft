//! Font View's view model: every glyph in glyph order, the ones shown in display order (the search and the sidebar
//! entries through `fontView.queryGlyphs`, then the List View sort), the selection, and what a cell or a row shows.
//!
//! Only the shown mode's data is projected: cell outlines in Grid View, cell texts in List View. Nothing is projected
//! while the panel is hidden. A cell outline is flattened once per glyph and master and kept until the glyph or a
//! glyph it uses as a component changes, so an edit flattens only the glyphs it touched.

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::behavior::kurbo::{BezPath, PathEl, Point, Shape as _};
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::{ColorLabel, Font, Glyph, GlyphProps, Prop, Shape, glyph_order, glyph_props, keeps_alternates, metric};
use serde_json::Value;

use super::ProjectCx;
use crate::app::font_view::{Column, Sort, ViewMode};
use crate::canvas::color;
use crate::shell::dock::PanelId;

/// Most points one cell outline keeps; the rest of a larger outline is left out of the cell.
pub const MAX_CELL_POINTS: usize = 8_192;
/// Line pieces per quadratic and per cubic segment: enough for a cell of the largest size.
const QUAD_STEPS: usize = 6;
const CUBIC_STEPS: usize = 8;
/// How deep components are followed to find the glyphs a cell outline depends on, and how many.
const MAX_DEPTH: usize = 8;
const MAX_DEPS: usize = 64;
/// Most cached outlines; past this the cache starts again empty.
const MAX_CACHED: usize = 65_536;

/// The twelve colour label names, in index order (`glyph.colorLabelIndices`).
pub const COLOR_LABEL_NAMES: [&str; 12] =
    ["Red", "Orange", "Brown", "Yellow", "Light Green", "Dark Green", "Cyan", "Blue", "Purple", "Pink", "Light Gray", "Dark Gray"];

/// What Font View shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FontViewModel {
    /// A font is open.
    pub has_font: bool,
    pub mode: ViewMode,
    /// The side of a grid cell, in points.
    pub cell_size: f64,
    /// Every glyph, in glyph order.
    pub rows: Vec<GlyphRow>,
    /// The rows shown, as indices into `rows`, in display order.
    pub visible: Vec<usize>,
    /// The selected rows, as indices into `rows`, in the order they were selected.
    pub selected: Vec<usize>,
    /// The row Shift extends from.
    pub anchor: Option<usize>,
    /// The List View columns shown, in order.
    pub columns: Vec<Column>,
    pub sort: Option<Sort>,
    /// The search field's text.
    pub search: String,
    /// The master the cells draw and the Components cell edits.
    pub master: Option<String>,
    /// The vertical extent a cell shows, in font units: the master's ascender and descender.
    pub ascender: f32,
    pub descender: f32,
    /// Why the query failed; every glyph is listed then.
    pub error: Option<String>,
}

/// One glyph as Font View lists it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphRow {
    pub name: String,
    /// Exported; a glyph that is not shows veiled.
    pub export: bool,
    /// The first code point as four to six hexadecimal digits.
    pub unicode: Option<String>,
    /// List View: the text of every column, by [`Column::index`]. Empty in Grid View.
    pub cells: Vec<String>,
    /// Grid View: the outline. `None` in List View.
    pub art: Option<Arc<CellArt>>,
}

/// A cell outline: the closed contours, flattened, in font units.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CellArt {
    /// The advance width.
    pub width: f32,
    pub contours: Vec<Vec<[f32; 2]>>,
    /// The glyph in colour, drawn in place of the outline (`color.preview.paletteDrivesFontView`).
    #[plain_data(assume = "a colour drawing behind an Arc, never changed after it is made")]
    pub color: Option<Arc<color::ColorArt>>,
}

impl FontViewModel {
    /// The row of the glyph named `name`.
    pub fn row_of(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.name == name)
    }

    /// The name of row `at`.
    pub fn name(&self, at: usize) -> Option<&str> {
        self.rows.get(at).map(|r| r.name.as_str())
    }
}

/// Cell outlines kept across projections, by glyph name: the glyphs they were made from (the glyph and the glyphs
/// it uses as components) and the outline.
#[derive(Default)]
pub struct OutlineCache {
    master: String,
    /// What the colour drawings were made with (`canvas::color::stamp`).
    color: u64,
    entries: HashMap<String, (Vec<Arc<Glyph>>, Arc<CellArt>)>,
}

pub fn project(model: &mut Tracked<FontViewModel>, cache: &mut OutlineCache, files: &mut color::ColorFiles, cx: &mut ProjectCx<'_>) {
    let state = &cx.view.font_view;
    let shows = cx.view.workspace.layout.panel_state(&PanelId::FontView) != PanelState::Hidden;
    let Ok(font) = cx.session.font().cloned() else {
        super::set(model, FontViewModel::default());
        return;
    };
    if !shows {
        super::set(model, FontViewModel { has_font: true, ..FontViewModel::default() });
        return;
    }
    let master = font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
    let props = Props::of(cx);
    let order = glyph_order(&font);
    let arcs = font.glyphs.as_arcs();
    let by_name: HashMap<&str, &Arc<Glyph>> = arcs.iter().map(|g| (g.name.as_str(), g)).collect();
    let stamp = color::stamp(&font, &master);
    if cache.master != master || cache.color != stamp || cache.entries.len() > MAX_CACHED {
        *cache = OutlineCache { master: master.clone(), color: stamp, entries: HashMap::new() };
    }
    files.follow(cx.session.active().map(|d| d.id));
    let assets = cx.session.assets();
    let mode = color::cell_mode(&font, &master, false);
    let mut src = color::Sources { assets: assets.as_ref(), files, text: color::TEXT };
    let mut glyphs: Vec<&Arc<Glyph>> = Vec::with_capacity(order.len());
    let mut rows = Vec::with_capacity(order.len());
    for (id, &i) in order.iter().enumerate() {
        let Some(glyph) = arcs.get(i) else { continue };
        let mut row = GlyphRow {
            name: glyph.name.clone(),
            export: glyph.export,
            unicode: glyph.unicodes.first().map(|u| format!("{u:04X}")),
            ..GlyphRow::default()
        };
        match state.mode {
            ViewMode::Grid => row.art = Some(art(&font, glyph, &master, &by_name, cache, &mode, &mut src)),
            ViewMode::List => {
                let cell = Cell { font: &font, glyph, id, master: &master, props: props.get(glyph) };
                row.cells = Column::ALL.iter().map(|c| cell.text(*c)).collect();
            }
        }
        glyphs.push(glyph);
        rows.push(row);
    }
    let index: HashMap<&str, usize> = rows.iter().enumerate().map(|(i, r)| (r.name.as_str(), i)).collect();
    let mut error = None;
    let mut visible: Vec<usize> = match state.query() {
        None => (0..rows.len()).collect(),
        Some(query) => match run_query(cx, query, &master, &index) {
            Ok(v) => v,
            Err(e) => {
                error = Some(e);
                (0..rows.len()).collect()
            }
        },
    };
    if let Some(sort) = state.sort {
        let props = &props;
        let key = |i: usize| {
            let glyph = glyphs.get(i).copied()?;
            Some(Cell { font: &font, glyph, id: i, master: &master, props: props.get(glyph) }.sort_value(sort.column))
        };
        sort_rows(&mut visible, sort, key);
    }
    let selected = state.selected.iter().filter_map(|n| index.get(n.as_str()).copied()).collect();
    let anchor = state.anchor.as_deref().and_then(|n| index.get(n).copied());
    let (ascender, descender) = frame(&font, &master);
    let value = FontViewModel {
        has_font: true,
        mode: state.mode,
        cell_size: state.cell_size,
        visible,
        selected,
        anchor,
        columns: state.shown_columns(),
        sort: state.sort,
        search: state.search.query.clone(),
        master: Some(master),
        ascender,
        descender,
        error,
        rows,
    };
    super::set(model, value);
}

/// The rows `fontView.queryGlyphs` gives for `query`, in its order.
fn run_query(cx: &mut ProjectCx<'_>, query: Value, master: &str, index: &HashMap<&str, usize>) -> Result<Vec<usize>, String> {
    let Value::Object(mut params) = query else { return Err("the Font View query must be an object".into()) };
    if !master.is_empty() {
        params.entry("master").or_insert_with(|| Value::String(master.to_string()));
    }
    let result = cx.session.query("fontView.queryGlyphs", &Value::Object(params)).map_err(|e| e.to_string())?;
    let names = result.get("glyphs").and_then(Value::as_array).ok_or("the query gave no glyph list")?;
    Ok(names.iter().filter_map(Value::as_str).filter_map(|n| index.get(n).copied()).collect())
}

/// What a column sorts by.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum SortValue {
    Number(f64),
    Text(String),
    /// No value: sorts after every value in either order.
    None,
}

/// Sorts `visible` by `sort`; ties keep glyph order, rows without a value go last.
fn sort_rows(visible: &mut Vec<usize>, sort: Sort, key: impl Fn(usize) -> Option<SortValue>) {
    let mut keyed: Vec<(SortValue, usize)> = visible.iter().map(|&i| (key(i).unwrap_or(SortValue::None), i)).collect();
    keyed.sort_by(|(a, ai), (b, bi)| {
        let order = match (a, b) {
            (SortValue::None, SortValue::None) => std::cmp::Ordering::Equal,
            (SortValue::None, _) => return std::cmp::Ordering::Greater,
            (_, SortValue::None) => return std::cmp::Ordering::Less,
            _ => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
        };
        let order = if sort.ascending { order } else { order.reverse() };
        order.then(ai.cmp(bi))
    });
    *visible = keyed.into_iter().map(|(_, i)| i).collect();
}

/// The glyph properties the List View columns read: the session's cache, else computed from the glyph data.
struct Props {
    cache: Option<Arc<fontcraft_engine::model::PropsCache>>,
    data: Option<Arc<GlyphData>>,
    keep_alternates: bool,
}

impl Props {
    fn of(cx: &mut ProjectCx<'_>) -> Props {
        if cx.view.font_view.mode == ViewMode::Grid && cx.view.font_view.sort.is_none() {
            return Props { cache: None, data: None, keep_alternates: false };
        }
        let keep_alternates = cx.session.font().is_ok_and(keeps_alternates);
        let data = cx.session.glyph_data().ok();
        let cache = cx.session.effective_props().ok().filter(|c| data.as_deref().is_some_and(|d| c.is_for(d, keep_alternates)));
        Props { cache, data, keep_alternates }
    }

    fn get(&self, glyph: &Glyph) -> Option<Arc<GlyphProps>> {
        if let Some(p) = self.cache.as_ref().and_then(|c| c.get(glyph)) {
            return Some(Arc::clone(p));
        }
        self.data.as_deref().map(|d| Arc::new(glyph_props(glyph, d, self.keep_alternates)))
    }
}

/// One glyph's cells.
struct Cell<'a> {
    font: &'a Font,
    glyph: &'a Glyph,
    /// The position in glyph order.
    id: usize,
    master: &'a str,
    props: Option<Arc<GlyphProps>>,
}

impl Cell<'_> {
    fn prop(&self, column: Column) -> Option<&Prop> {
        let p = self.props.as_deref()?;
        Some(match column {
            Column::ProductionName => &p.production_name,
            Column::Script => &p.script,
            Column::Category => &p.category,
            Column::SubCategory => &p.sub_category,
            Column::Case => &p.case,
            Column::Direction => &p.direction,
            Column::SortName => &p.sort_name,
            _ => return None,
        })
    }

    fn text(&self, column: Column) -> String {
        let glyph = self.glyph;
        if let Some(prop) = self.prop(column) {
            return prop.value.clone().unwrap_or_default();
        }
        match column {
            Column::Id => self.id.to_string(),
            Column::Char => first_char(glyph).map(String::from).unwrap_or_default(),
            Column::Name => glyph.name.clone(),
            Column::Unicode => glyph.unicodes.iter().map(|u| format!("{u:04X}")).collect::<Vec<_>>().join(" "),
            Column::Export => if glyph.export { "Yes" } else { "No" }.into(),
            Column::Color => glyph.info.color.map(color_name).unwrap_or_default(),
            Column::Tags => glyph.info.tags.join(", "),
            Column::Note => glyph.info.note.lines().next().unwrap_or_default().to_string(),
            Column::Components => components(glyph, self.master).join(", "),
            Column::LastChanged => glyph.info.last_change.clone().unwrap_or_default(),
            Column::Lsb | Column::Rsb | Column::Width => self.number(column).map(|n| format!("{}", n.round())).unwrap_or_default(),
            Column::LeftGroup => glyph.kern_groups.left.clone().unwrap_or_default(),
            Column::RightGroup => glyph.kern_groups.right.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The metric columns' values in the shown master.
    fn number(&self, column: Column) -> Option<f64> {
        let layer = self.glyph.layer(self.master)?;
        if column == Column::Width {
            return Some(layer.width);
        }
        let outline = self.font.outline_of(self.glyph, self.master).ok()?;
        if outline.elements().is_empty() {
            return None;
        }
        let bounds = outline.bounding_box();
        match column {
            Column::Lsb => Some(bounds.x0),
            Column::Rsb => Some(layer.width - bounds.x1),
            _ => None,
        }
        .filter(|n| n.is_finite())
    }

    fn sort_value(&self, column: Column) -> SortValue {
        let glyph = self.glyph;
        match column {
            Column::Id => SortValue::Number(self.id as f64),
            Column::Unicode => glyph.unicodes.first().map_or(SortValue::None, |u| SortValue::Number(f64::from(*u))),
            Column::Char => first_char(glyph).map_or(SortValue::None, |c| SortValue::Number(f64::from(u32::from(c)))),
            Column::Lsb | Column::Rsb | Column::Width => self.number(column).map_or(SortValue::None, SortValue::Number),
            Column::Color => match glyph.info.color {
                Some(ColorLabel::Index(i)) => SortValue::Number(f64::from(i)),
                Some(ColorLabel::Rgba(_)) => SortValue::Number(f64::from(u8::MAX)),
                None => SortValue::None,
            },
            Column::Export => SortValue::Number(if glyph.export { 0.0 } else { 1.0 }),
            _ => {
                let text = self.text(column);
                if text.is_empty() { SortValue::None } else { SortValue::Text(text.to_lowercase()) }
            }
        }
    }
}

/// The glyph's first code point as a character, unless it is a control character.
fn first_char(glyph: &Glyph) -> Option<char> {
    glyph.unicodes.first().and_then(|u| char::from_u32(*u)).filter(|c| !c.is_control())
}

fn color_name(color: ColorLabel) -> String {
    match color {
        ColorLabel::Index(i) => COLOR_LABEL_NAMES.get(usize::from(i)).map_or_else(|| i.to_string(), |n| (*n).to_string()),
        ColorLabel::Rgba([r, g, b, a]) => {
            let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02X}{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b), byte(a))
        }
    }
}

/// The glyphs the master layer uses as components, in order.
fn components<'g>(glyph: &'g Glyph, master: &str) -> Vec<&'g str> {
    let Some(layer) = glyph.layer(master) else { return Vec::new() };
    layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Component(c) => Some(c.glyph.as_str()),
            _ => None,
        })
        .collect()
}

/// The master's ascender and descender, in font units.
fn frame(font: &Font, master: &str) -> (f32, f32) {
    let upm = f64::from(font.units_per_em);
    let (a, d) = match font.master(master) {
        Ok(m) => (font.metric_pos(m, metric::ASCENDER, upm * 0.8), font.metric_pos(m, metric::DESCENDER, -upm * 0.2)),
        Err(_) => (upm * 0.8, -upm * 0.2),
    };
    (a as f32, d as f32)
}

/// The glyph and the glyphs its master layer reaches through components, at most [`MAX_DEPS`] of them.
fn dependencies(glyph: &Arc<Glyph>, master: &str, by_name: &HashMap<&str, &Arc<Glyph>>) -> Vec<Arc<Glyph>> {
    let mut out = vec![Arc::clone(glyph)];
    let mut next = vec![(Arc::clone(glyph), 0usize)];
    while let Some((g, depth)) = next.pop() {
        if depth >= MAX_DEPTH {
            continue;
        }
        for name in components(&g, master) {
            if out.len() >= MAX_DEPS {
                return out;
            }
            if let Some(base) = by_name.get(name)
                && !out.iter().any(|d| Arc::ptr_eq(d, base))
            {
                out.push(Arc::clone(base));
                next.push((Arc::clone(base), depth + 1));
            }
        }
    }
    out
}

/// The cell outline of `glyph`: the cached one while none of the glyphs it was made from changed.
fn art(
    font: &Font,
    glyph: &Arc<Glyph>,
    master: &str,
    by_name: &HashMap<&str, &Arc<Glyph>>,
    cache: &mut OutlineCache,
    mode: &color::Mode,
    src: &mut color::Sources<'_>,
) -> Arc<CellArt> {
    let deps = dependencies(glyph, master, by_name);
    if let Some((held, art)) = cache.entries.get(&glyph.name)
        && held.len() == deps.len()
        && held.iter().zip(&deps).all(|(a, b)| Arc::ptr_eq(a, b))
    {
        return Arc::clone(art);
    }
    let width = glyph.layer(master).map_or(0.0, |l| l.width) as f32;
    let contours = font.outline_of(glyph, master).map(|p| flatten(&p)).unwrap_or_default();
    let color = color::cell_color(font, glyph, mode, src);
    let art = Arc::new(CellArt { width: if width.is_finite() { width } else { 0.0 }, contours, color });
    cache.entries.insert(glyph.name.clone(), (deps, Arc::clone(&art)));
    art
}

/// The closed contours of `path` as polygons, at most [`MAX_CELL_POINTS`] points in all.
pub fn flatten(path: &BezPath) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut current: Vec<[f32; 2]> = Vec::new();
    let mut last = Point::ZERO;
    let mut budget = MAX_CELL_POINTS;
    let mut done = 0usize;
    let point = |p: Point| [p.x as f32, p.y as f32];
    for el in path.elements() {
        if budget == 0 {
            break;
        }
        match *el {
            PathEl::MoveTo(p) => {
                if current.len() > 2 {
                    done += current.len();
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
                current.push(point(p));
                last = p;
            }
            PathEl::LineTo(p) => {
                current.push(point(p));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                for step in 1..=QUAD_STEPS {
                    let t = step as f64 / QUAD_STEPS as f64;
                    let u = 1.0 - t;
                    current.push(point((last.to_vec2() * (u * u) + c.to_vec2() * (2.0 * u * t) + p.to_vec2() * (t * t)).to_point()));
                }
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                for step in 1..=CUBIC_STEPS {
                    let t = step as f64 / CUBIC_STEPS as f64;
                    let u = 1.0 - t;
                    let v = last.to_vec2() * (u * u * u)
                        + c1.to_vec2() * (3.0 * u * u * t)
                        + c2.to_vec2() * (3.0 * u * t * t)
                        + p.to_vec2() * (t * t * t);
                    current.push(point(v.to_point()));
                }
                last = p;
            }
            PathEl::ClosePath => {
                if current.len() > 2 {
                    done += current.len();
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
            }
        }
        budget = MAX_CELL_POINTS.saturating_sub(done + current.len());
    }
    if current.len() > 2 {
        out.push(current);
    }
    out.retain(|c| c.iter().all(|[x, y]| x.is_finite() && y.is_finite()));
    out
}
