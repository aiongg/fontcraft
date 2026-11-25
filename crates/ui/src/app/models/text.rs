//! The edit view's text, laid out: what the text layers paint and where the Text tool's clicks land.
//!
//! The layout is in text coordinates (the first line's baseline at y = 0). Everything the layers paint is shifted
//! into the edited glyph's font units, the canvas's coordinates, by the edited glyph's place in the text ([`origin`]).

use std::collections::HashMap;
use std::sync::Arc;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::stroke::OpenCorners;
use fontcraft_engine::compile::preview::PreviewFont;
use fontcraft_engine::model::Font;
use fontcraft_engine::prelude::*;
use fontcraft_engine::settings::UserSettings;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_spacing::kerning::{GroupSide, Position, facing, members};
use fontcraft_textlayout::{Direction, LENGTH_RANGE, Layout, Options, PreviewShaper, Shaping};

use super::ProjectCx;
use crate::app::ViewState;
use crate::canvas::text::{self, EditText};

/// Most glyphs of the text drawn besides the edited one; a longer text draws the first ones.
pub const MAX_DRAWN_GLYPHS: usize = 2000;
/// Most members drawn for one glyph by Show Group Members; a larger group shows its first ones in glyph order.
pub const MAX_MEMBERS: usize = 200;
/// The setting that sets where Edit View text wraps, in thousandths of an em.
pub const TEXT_VIEW_WIDTH: &str = "appearance.textViewWidth";
/// The setting's default, ten em.
pub const DEFAULT_TEXT_VIEW_WIDTH: f64 = 10_000.0;

/// The width at which Edit View text wraps, in font units: `appearance.textViewWidth` × `units_per_em` / 1000, kept
/// within the layout's [`LENGTH_RANGE`].
pub fn line_width(settings: &UserSettings, units_per_em: u16) -> f64 {
    let thousandths = settings.get(TEXT_VIEW_WIDTH).ok().and_then(|v| v.as_f64()).filter(|w| w.is_finite()).unwrap_or(DEFAULT_TEXT_VIEW_WIDTH);
    (thousandths * f64::from(units_per_em) / 1000.0).clamp(*LENGTH_RANGE.start(), *LENGTH_RANGE.end())
}

/// What the text layers draw, in the edited glyph's font units. `shown` is false while the canvas shows a glyph the
/// text does not hold; the other fields are empty then.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TextModel {
    pub shown: bool,
    pub text_mode: bool,
    /// The edited glyph and the item that shows it.
    pub edited_glyph: Option<String>,
    pub edited: Option<usize>,
    /// Where the edited glyph's origin sits in the text.
    pub origin: [f64; 2],
    /// The text's layout and what it was made from.
    #[plain_data(assume = "an owned layout value behind an Arc, never changed after it is made")]
    pub layout: Option<Arc<Layout>>,
    pub key: Option<LayoutKey>,
    /// The other glyphs of the text, each placed.
    pub glyphs: Vec<TextGlyph>,
    /// The other glyphs of the text drawn in colour (`canvas::color::text_colors`).
    pub colors: Vec<crate::canvas::color::PlacedColor>,
    /// The kerned gaps as `[x0, y0, x1, y1]` and whether the kerning closes the gap.
    pub kerned: Vec<([f64; 4], bool)>,
    /// The selected items' advance boxes.
    pub selection: Vec<[f64; 4]>,
    /// The text cursor's two ends, when nothing is selected.
    pub cursor: Option<[[f64; 2]; 2]>,
    /// Show Group Members: the members' outlines, placed.
    #[plain_data(assume = "kurbo paths: owned points")]
    pub members: Vec<BezPath>,
    /// View › Show Measurement Line, while it shows.
    pub measurement: Option<MeasurementLine>,
    /// View › Show Metrics in text mode: each glyph's sidebearings and width (`spacing.metricsOverlayInTextMode`).
    pub labels: Vec<MetricsLabel>,
    /// The shaper of the preview font the layout shaped with; kept across projections.
    #[plain_data(assume = "Arcs to a preview font and its shaper, never changed after they are made")]
    pub shaper: ShaperSlot,
}

/// One glyph's metrics labels, in the edited glyph's units: its advance box, its two sidebearings (top and bottom in
/// vertical text; `None` for an empty layer) and its advance, and how far the neighbours' negative kerning reaches
/// into the box at its start and end.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct MetricsLabel {
    pub bounds: [f64; 4],
    pub sides: Option<(f64, f64)>,
    pub advance: f64,
    pub overlap: (f64, f64),
    pub vertical: bool,
}

/// A preview font and its shaper, or why the shaper could not be made. Two slots are equal when they hold the same
/// font and shaper.
#[derive(Clone, Default)]
pub struct ShaperSlot(Option<(Arc<PreviewFont>, ShaperResult)>);

type ShaperResult = Result<Arc<PreviewShaper>, String>;

impl PartialEq for ShaperSlot {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some((a, x)), Some((b, y))) => {
                Arc::ptr_eq(a, b)
                    && match (x, y) {
                        (Ok(x), Ok(y)) => Arc::ptr_eq(x, y),
                        (Err(x), Err(y)) => x == y,
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}

impl std::fmt::Debug for ShaperSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => f.write_str("ShaperSlot(None)"),
            Some((_, shaper)) => f.debug_tuple("ShaperSlot").field(&shaper.as_ref().map(|_| "shaper")).finish(),
        }
    }
}

impl ShaperSlot {
    /// The slot for `preview`: the cached shaper when it was made for the same font, else a new one.
    pub fn for_preview(&self, preview: Option<Arc<PreviewFont>>) -> ShaperSlot {
        let Some(preview) = preview else { return self.clone() };
        if let Some((font, _)) = &self.0
            && Arc::ptr_eq(font, &preview)
        {
            return self.clone();
        }
        let tags: Vec<fontcraft_textlayout::shaping::Tag> = preview.features.iter().map(|f| f.tag).collect();
        let shaper = PreviewShaper::new(Arc::from(preview.bytes.as_slice()), &preview.glyph_order, &preview.private_use, &tags)
            .map(Arc::new)
            .map_err(|e| e.to_string());
        ShaperSlot(Some((preview, shaper)))
    }

    fn shaper(&self, preview: Option<&Arc<PreviewFont>>) -> Option<&Result<Arc<PreviewShaper>, String>> {
        let (font, shaper) = self.0.as_ref()?;
        preview.filter(|p| Arc::ptr_eq(p, font)).map(|_| shaper)
    }
}

/// What the Features menu shows (`canvas::text::features`).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FeaturesModel {
    /// The document has a feature compile, and the font has not changed since.
    pub compiled: bool,
    pub current: bool,
    /// The compiled features: tag and table (`GSUB` or `GPOS`), each tag once.
    pub features: Vec<(String, String)>,
    /// The script and language pairs Localized Forms offers.
    pub language_systems: Vec<(String, String)>,
    pub active: Vec<String>,
    pub localized: bool,
    pub script: Option<String>,
    pub language: Option<String>,
    /// Why the text is not shaped as the features ask.
    pub shaping_error: Option<String>,
}

/// The measurement line (`measurementLine.semantics`, `measurementLine.display`), in the edited glyph's units: a
/// line at the chosen height on every line of the text, each glyph's sidebearings measured on it, and thin lines at
/// the edges of every advance.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MeasurementLine {
    /// The line's height above the baseline, in text coordinates.
    pub height: f64,
    /// The height of each line of the text.
    pub rules: Vec<f64>,
    pub glyphs: Vec<MeasuredGlyph>,
}

/// One glyph on the measurement line.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct MeasuredGlyph {
    /// The advance's left and right edges, and its bottom and top.
    pub x0: f64,
    pub x1: f64,
    pub bottom: f64,
    pub top: f64,
    /// The line's height there.
    pub y: f64,
    /// The left and right sidebearings on the line; `None` where the outline does not reach it.
    pub sides: Option<(f64, f64)>,
}

/// One glyph of the text: the item and its outline, placed.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct TextGlyph {
    pub item: usize,
    pub name: String,
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    /// The outline as Fill Preview fills it, when it has open corners that an export erases.
    #[plain_data(assume = "a kurbo path: owned points")]
    pub fill: Option<BezPath>,
    /// Where the glyph's origin lies, in the edited glyph's font units.
    pub at: [f64; 2],
}

/// What a layout depends on besides the text's items and options.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct LayoutKey {
    /// The document's id and revision.
    pub doc: (u64, u64),
    pub text: u64,
    pub master: String,
    /// The last feature compile (`features::compile_stamp`).
    pub compile: Option<(u64, u64, bool)>,
    /// The user settings' revision, for the line width.
    pub settings: u64,
}

pub fn project(model: &mut Tracked<TextModel>, files: &mut crate::canvas::color::ColorFiles, cx: &mut ProjectCx<'_>) {
    let shaper = model.shaper.for_preview(text::features::preview(cx.session, &cx.view.text.features));
    let mut value = build(cx.session, cx.view, &shaper).unwrap_or_default();
    let placed: Vec<(&str, [f64; 2])> = value.glyphs.iter().map(|g| (g.name.as_str(), g.at)).collect();
    value.colors = crate::canvas::color::text_colors(cx.session, cx.view, files, &placed);
    value.shaper = shaper;
    super::set(model, value);
}

/// The Features menu's model; the shaping error comes from the projected layout.
pub fn project_features(model: &mut Tracked<FeaturesModel>, text_model: &TextModel, cx: &mut ProjectCx<'_>) {
    let error = match &text_model.layout {
        Some(layout) => layout.shaping_error.clone(),
        None if cx.view.text.features.any() => layout(cx.session, cx.view, &text_model.shaper).and_then(|l| l.shaping_error.clone()),
        None => None,
    };
    let value = text::features::menu(cx.session, &cx.view.text.features, error);
    super::set(model, value);
}

/// The layout of the edit view's text in the current master, as the text commands left it, shaped with the active
/// preview features through `shaper` when a compile made a preview font.
pub fn layout(session: &mut Session, view: &ViewState, shaper: &ShaperSlot) -> Option<Arc<Layout>> {
    let index = if view.text.kerning.shows_kerning() { session.kern_index().ok() } else { None };
    let features = &view.text.features;
    let preview = text::features::preview(session, features);
    let font = session.font().ok()?;
    let master = crate::shell::masters::current_master(font, &view.layers)?.id.clone();
    let line_width = line_width(session.settings(), font.units_per_em);
    let active = features.active_tags();
    let found = shaper.shaper(preview.as_ref());
    let shaping = match found {
        Some(Ok(shaper)) => Some(Shaping { shaper: shaper.as_ref(), active: &active, localized: features.localized_tags() }),
        _ => None,
    };
    let options = Options {
        direction: view.text.direction.layout(),
        kerning: view.text.kerning,
        line_width,
        index: index.as_deref(),
        layers: Some(&view.layers.layers),
        shaping,
        cursive: true,
        ..Default::default()
    };
    let mut layout = fontcraft_textlayout::layout(font, &master, &view.text.text, &options).ok()?;
    match found {
        Some(Err(e)) => {
            layout.shaping_error.get_or_insert_with(|| e.clone());
        }
        None if features.any() => {
            layout.shaping_error.get_or_insert_with(|| "the features have not been compiled, so the text is not shaped".to_string());
        }
        _ => {}
    }
    Some(Arc::new(layout))
}

/// The layout key of the session and view as they are now.
pub fn key(session: &Session, view: &ViewState) -> Option<LayoutKey> {
    let font = session.font().ok()?;
    let doc = session.active().map(|d| (d.id, d.revision))?;
    let master = crate::shell::masters::current_master(font, &view.layers)?.id.clone();
    Some(LayoutKey {
        doc,
        text: view.text.revision,
        master,
        compile: text::features::compile_stamp(session),
        settings: session.settings().revision(),
    })
}

fn build(session: &mut Session, view: &ViewState, shaper: &ShaperSlot) -> Option<TextModel> {
    let font = session.font().ok()?;
    if !text::shows_text(font, view) {
        return None;
    }
    let key = key(session, view)?;
    let layout = layout(session, view, shaper)?;
    let font = session.font().ok()?;
    let state = &view.text;
    let edited = state.edited_index()?;
    let origin = layout.placed.get(edited).map_or([0.0, 0.0], |p| [p.x, p.y]);
    let shift = Affine::translate((-origin[0], -origin[1]));
    let master = key.master.as_str();
    let descender = crate::shell::masters::current_master(font, &view.layers)
        .map_or(layout.ascender - layout.line_height, |m| super::canvas::vertical_span(font, m).0);
    let heights = (descender.min(layout.ascender), layout.ascender);
    let move_box = |[x0, y0, x1, y1]: [f64; 4]| [x0 - origin[0], y0 - origin[1], x1 - origin[0], y1 - origin[1]];

    let corners = OpenCorners::of(font);
    let mut outlines: HashMap<&str, Option<(BezPath, Option<BezPath>)>> = HashMap::new();
    let mut glyphs = Vec::new();
    for (item, placed) in layout.placed.iter().enumerate() {
        if item == edited {
            continue;
        }
        let Some(name) = placed.glyph.as_deref() else { continue };
        if glyphs.len() >= MAX_DRAWN_GLYPHS {
            break;
        }
        let outline = outlines.entry(name).or_insert_with(|| {
            let outline = font.outline(name, master).ok()?;
            let fill = corners.filled(master, name, &outline);
            Some((outline, fill))
        });
        if let Some((outline, fill)) = outline {
            let place = |path: &BezPath| (shift * Affine::translate((placed.x, placed.y))) * path;
            glyphs.push(TextGlyph {
                item,
                name: name.to_string(),
                outline: place(outline),
                fill: fill.as_ref().map(place),
                at: [placed.x - origin[0], placed.y - origin[1]],
            });
        }
    }

    let mut kerned = Vec::new();
    if state.kerning.shows_kerning() {
        for p in &layout.placed {
            if p.kern != 0.0 && p.kern.is_finite() {
                kerned.push((move_box(band(&layout, p.line, (p.offset - p.kern, p.offset), heights)), p.kern < 0.0));
            }
        }
    }
    let selection: Vec<[f64; 4]> = state
        .text
        .selection()
        .filter_map(|i| layout.placed.get(i))
        .map(|p| move_box(band(&layout, p.line, (p.offset, p.offset + p.advance), heights)))
        .filter(|[x0, y0, x1, y1]| x1 > x0 && y1 > y0)
        .collect();
    let cursor = state.text.selection().is_empty().then(|| {
        let (x, y) = layout.caret(state.text.cursor());
        let lh = layout.line_height;
        let (a, b) = match layout.direction {
            Direction::Vertical => ([x - lh / 2.0, y], [x + lh / 2.0, y]),
            _ => ([x, y + heights.0], [x, y + heights.1]),
        };
        [[a[0] - origin[0], a[1] - origin[1]], [b[0] - origin[0], b[1] - origin[1]]]
    });
    let members = if state.group_members { group_members(font, state, &layout, master, shift) } else { Vec::new() };
    let measurement = (state.measurement && text::text_mode(view) && layout.direction != Direction::Vertical)
        .then(|| measurement_line(font, view, &layout, origin));
    let labels = if text::text_mode(view) && view.display.metrics { metrics_labels(font, view, &layout, origin, heights.0) } else { Vec::new() };
    Some(TextModel {
        shown: true,
        text_mode: text::text_mode(view),
        edited_glyph: state.current().map(str::to_string),
        edited: Some(edited),
        origin,
        layout: Some(layout.clone()),
        key: Some(key),
        glyphs,
        colors: Vec::new(),
        kerned,
        selection,
        cursor,
        members,
        measurement,
        labels,
        shaper: ShaperSlot::default(),
    })
}

/// Each glyph's metrics labels (`spacing.metricsOverlayInTextMode`), at most [`MAX_DRAWN_GLYPHS`]. `descender` is
/// the bottom of the box the labels hang under, above the baseline.
fn metrics_labels(font: &Font, view: &ViewState, layout: &Layout, origin: [f64; 2], descender: f64) -> Vec<MetricsLabel> {
    use fontcraft_spacing::{Field, metric};
    let vertical = layout.direction == Direction::Vertical;
    let (a, b) = if vertical { (Field::Top, Field::Bottom) } else { (Field::Left, Field::Right) };
    let mut sides: HashMap<&str, Option<(f64, f64)>> = HashMap::new();
    let mut out = Vec::new();
    for (i, p) in layout.placed.iter().enumerate() {
        let Some(name) = p.glyph.as_deref() else { continue };
        if out.len() >= MAX_DRAWN_GLYPHS {
            break;
        }
        let measured = *sides.entry(name).or_insert_with(|| {
            let layer = crate::shell::masters::shown_layer(font, &view.layers, font.glyph(name)?)?;
            match (metric(font, layer, a), metric(font, layer, b)) {
                (Ok(Some(a)), Ok(Some(b))) if a.is_finite() && b.is_finite() => Some((a, b)),
                _ => None,
            }
        });
        let [x0, y0, x1, y1] = band(layout, p.line, (p.offset, p.offset + p.advance), (descender, layout.ascender));
        // Negative kerning lays a neighbour over the advance: the labels keep to the part it does not cover.
        let reach = |kern: f64| if kern.is_finite() { kern.min(0.0).abs() } else { 0.0 };
        let next = layout.placed.get(i + 1).filter(|n| n.line == p.line).map_or(0.0, |n| reach(n.kern));
        let (start, end) = (reach(p.kern), next);
        let overlap = if layout.direction == Direction::Rtl { (end, start) } else { (start, end) };
        out.push(MetricsLabel {
            bounds: [x0 - origin[0], y0 - origin[1], x1 - origin[0], y1 - origin[1]],
            sides: measured,
            advance: p.advance,
            overlap,
            vertical,
        });
    }
    out
}

/// The measurement line's height: the one set, else half the x-height of the current master.
pub fn measurement_height(font: &Font, view: &ViewState) -> f64 {
    if let Some(y) = view.text.measurement_height.filter(|y| y.is_finite()) {
        return y;
    }
    let upm = f64::from(font.units_per_em);
    let x_height = crate::shell::masters::current_master(font, &view.layers)
        .map_or(upm * 0.5, |m| font.metric_pos(m, fontcraft_engine::model::metric::X_HEIGHT, upm * 0.5));
    if x_height.is_finite() { x_height / 2.0 } else { 0.0 }
}

fn measurement_line(font: &Font, view: &ViewState, layout: &Layout, origin: [f64; 2]) -> MeasurementLine {
    let height = measurement_height(font, view);
    let rules = layout.lines.iter().map(|l| l.baseline + height - origin[1]).collect();
    let mut sides: HashMap<&str, Option<(f64, f64)>> = HashMap::new();
    let mut glyphs = Vec::new();
    for p in &layout.placed {
        let Some(name) = p.glyph.as_deref() else { continue };
        if glyphs.len() >= MAX_DRAWN_GLYPHS {
            break;
        }
        let measured = *sides.entry(name).or_insert_with(|| {
            let glyph = font.glyph(name)?;
            let layer = crate::shell::masters::shown_layer(font, &view.layers, glyph)?;
            match fontcraft_spacing::sidebearings_at_height(font, layer, height) {
                Ok(Some((l, r))) if l.is_finite() && r.is_finite() => Some((l, r)),
                _ => None,
            }
        });
        glyphs.push(MeasuredGlyph {
            x0: p.x - origin[0],
            x1: p.x + p.advance - origin[0],
            bottom: p.y + layout.ascender - layout.line_height - origin[1],
            top: p.y + layout.ascender - origin[1],
            y: p.y + height - origin[1],
            sides: measured,
        });
    }
    MeasurementLine { height, rules, glyphs }
}

/// The rectangle `a0..a1` along line `line` covers, from `low` to `high` above the baseline (a column one line
/// height wide in vertical text), as `[x0, y0, x1, y1]` in text coordinates.
pub fn band(layout: &Layout, line: usize, (a0, a1): (f64, f64), (low, high): (f64, f64)) -> [f64; 4] {
    let (a0, a1) = (a0.min(a1), a0.max(a1));
    let base = layout.lines.get(line).map_or(0.0, |l| l.baseline);
    let lh = layout.line_height;
    match layout.direction {
        Direction::Ltr => [a0, base + low, a1, base + high],
        Direction::Rtl => [-a1, base + low, -a0, base + high],
        Direction::Vertical => [base - lh / 2.0, -a1, base + lh / 2.0, -a0],
    }
}

/// The glyph item whose advance box, across the line's full height, holds the text point (x, y).
pub fn item_at(layout: &Layout, x: f64, y: f64) -> Option<usize> {
    let line = layout.placed.get(layout.hit(x, y)).or(layout.placed.last())?.line;
    let items = layout.lines.get(line)?.items.clone();
    let full = (layout.ascender - layout.line_height, layout.ascender);
    items.into_iter().find(|&i| {
        layout.placed.get(i).is_some_and(|p| {
            let [x0, y0, x1, y1] = band(layout, p.line, (p.offset, p.offset + p.advance), full);
            p.glyph.is_some() && (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
        })
    })
}

/// One member Show Group Members draws: the glyph, the item it stands in for, the side its group is on, and where
/// its origin goes in text coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub glyph: String,
    pub item: usize,
    pub side: GroupSide,
    pub x: f64,
    pub y: f64,
}

/// The members to draw (`kerning.groupMembersDisplayScope`): the edited glyph with the group of its side that faces
/// the glyph before it, and that glyph, on the same line, with the group of its side that faces the edited glyph.
/// Each member is placed so its facing side lines up with the glyph it stands in for.
pub fn members_shown(font: &Font, state: &EditText, layout: &Layout, master: &str) -> Vec<Member> {
    let Some(edited) = state.edited_index() else { return Vec::new() };
    let direction = state.direction.layout();
    let line = layout.placed.get(edited).map(|p| p.line);
    let before = edited.checked_sub(1).filter(|i| layout.placed.get(*i).map(|p| p.line) == line);
    let mut out = Vec::new();
    for (item, position) in [(before, Position::First), (Some(edited), Position::Second)] {
        let Some(item) = item else { continue };
        let side = facing(direction, position);
        let Some(placed) = layout.placed.get(item) else { continue };
        let Some(name) = placed.glyph.as_deref() else { continue };
        let Some(group) = font.glyph(name).and_then(|g| side.of(&g.kern_groups)) else { continue };
        for member in members(font, side, group).into_iter().filter(|m| *m != name).take(MAX_MEMBERS) {
            let Some(width) = font.glyph(member).and_then(|g| g.layer(master)).map(|l| l.width) else { continue };
            let x = match side {
                GroupSide::Right => placed.x + placed.advance - width,
                GroupSide::Left | GroupSide::Top | GroupSide::Bottom => placed.x,
            };
            if x.is_finite() {
                out.push(Member { glyph: member.to_string(), item, side, x, y: placed.y });
            }
        }
    }
    out
}

fn group_members(font: &Font, state: &EditText, layout: &Layout, master: &str, shift: Affine) -> Vec<BezPath> {
    members_shown(font, state, layout, master)
        .into_iter()
        .filter_map(|m| {
            let mut outline = font.outline(&m.glyph, master).ok()?;
            outline.apply_affine(shift * Affine::translate((m.x, m.y)));
            Some(outline)
        })
        .collect()
}
