//! Guides, the grid, the metric lines that parameters set, the CJK grids, measurement guides
//! and the Measurement tool's labels (`docs/spec/03a` § 20, `docs/spec/03b` › Measuring).
//!
//! | Item | Drawn as |
//! |---|---|
//! | local guide | line across the canvas, `guide_local` (or the guide's own colour) |
//! | global guide | the same, `guide_global`; only in the glyphs its filter takes in |
//! | guide knob | ring at the guide's position; filled when selected; a padlock when locked |
//! | guide name | at the guide's left end |
//! | measurement guide | dots where it crosses the outline, the distance between neighbouring dots |
//! | grid | lines every grid step, `grid_line`, once the step is `grid_min_spacing` points on screen |
//! | metric line a parameter sets (shoulder height, small-cap height, per-script values) | like a metric line |
//! | CJK grid, design grid | dotted lines, `cjk_grid`; the CJK guide as a box |
//! | Measurement tool | each point's coordinates, the deltas between on-curve nodes, the ruler, the overlapped stretches of outline in `measure_overlap` |
//!
//! The lines, crossings and coordinates come from engine queries (`guide.displayLines`,
//! `measure.ruler`, `measure.coordinates`), run once per frame by [`Extras::query`] before the
//! frame borrows the font. The Measurement tool covers every glyph of the tab's text
//! (`measurement.allVisibleGlyphs`), at most `MAX_PLACEMENTS` of them nearest the edited one. As
//! the overlap test is slow, each glyph's coordinates are kept until that glyph, a glyph it uses
//! or the masters change, and the last line's answer and ruler until the document changes.
//!
//! A global guide with a filter (a glyph predicate) shows only in the glyphs it matches
//! (`guides.globalScopeFilter`, evaluated by `fontcraft_glyphquery::guide_in_scope`). A filter
//! that does not read as a predicate leaves the guide shown everywhere.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};
use fontcraft_engine::Session;
use fontcraft_geom::Point;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Axis, Font, Glyph, Guide, Layer, Master, MetricDef, Shape as ModelShape};
use fontcraft_tools::{GuideRef, GuideScope, Overlay, OverlayKind, Selection};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::Painting;
use super::view::LayoutCache;
use crate::state::{DisplayOptions, Tab};

/// Most measurement guides measured per frame.
const MAX_MEASURED_GUIDES: usize = 64;
/// Most labels the Measurement tool draws per frame; a larger line shows the first ones.
const MAX_LABELS: usize = 5_000;
/// Most glyphs of the line the Measurement tool covers (the limit of `measure.ruler`).
const MAX_PLACEMENTS: usize = 200;
/// Most segments of overlapped stretches the Measurement tool draws per frame.
const MAX_OVERLAP_SEGMENTS: usize = 20_000;
/// Most glyphs one glyph's kept coordinates may depend on (itself and its components, nested);
/// a glyph with more is measured again after every edit.
const MAX_SOURCE_GLYPHS: usize = 64;
/// Most lines one grid draws in each direction.
const MAX_GRID_LINES: usize = 2_000;
/// Gap between the dots of a dotted line, and their radius, in points.
const DOT_SPACING: f32 = 4.0;
const DOT_RADIUS: f32 = 0.75;
/// Distance of a label from the point it names, in points.
const LABEL_OFFSET: f32 = 5.0;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Grid {
    step: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetricLine {
    kind: String,
    name: Option<String>,
    y: f64,
    source: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Cjk {
    #[serde(rename = "box")]
    bounds: [f64; 4],
    rows: Option<f64>,
    columns: Option<f64>,
    guide: Option<[f64; 4]>,
}

/// `guide.displayLines`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DisplayLines {
    width: f64,
    grid: Grid,
    metrics: Vec<MetricLine>,
    cjk: Option<Cjk>,
    #[serde(rename = "designGrid")]
    design_grid: Option<Vec<f64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Crossing {
    x: f64,
    y: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Gap {
    from: usize,
    to: usize,
    distance: f64,
}

/// `measure.ruler`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Ruler {
    crossings: Vec<Crossing>,
    gaps: Vec<Gap>,
    angle: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct CoordPoint {
    shape: usize,
    node: usize,
    x: f64,
    y: f64,
    #[serde(rename = "onCurve")]
    on_curve: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Delta {
    shape: usize,
    from: usize,
    to: usize,
    dy: f64,
    #[serde(rename = "italicDx")]
    italic_dx: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct AnchorPoint {
    x: f64,
    y: f64,
}

/// `measure.coordinates` of one glyph, in its own coordinates.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Coordinates {
    points: Vec<CoordPoint>,
    deltas: Vec<Delta>,
    anchors: Vec<AnchorPoint>,
    /// Stretches of outline inside an overlap, as polylines.
    overlaps: Vec<Vec<[f64; 2]>>,
    /// The overlap test stopped at the query's work limit, so `overlaps` may miss some.
    #[serde(rename = "overlapsTruncated")]
    overlaps_truncated: bool,
}

/// A glyph of the line with its coordinates; it sits at `x`, `y` from the edited glyph's origin.
#[derive(Clone, Debug)]
struct Placed {
    x: f64,
    y: f64,
    coords: Arc<Coordinates>,
}

/// A glyph of the line to measure: which layer and side, where it sits, and how far it is from
/// the edited glyph in the text.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    glyph: String,
    layer: String,
    background: bool,
    x: f64,
    y: f64,
    distance: usize,
}

impl Entry {
    fn key(&self) -> GlyphKey {
        (self.glyph.clone(), self.layer.clone(), self.background)
    }

    /// The entry as one of the queries' `placements`, at its place in the line or (`at` false)
    /// at the origin.
    fn placement(&self, at: bool) -> Value {
        let mut v = json!({"glyph": self.glyph, "layer": self.layer});
        if let Some(o) = v.as_object_mut() {
            if at {
                o.insert("x".into(), json!(self.x));
                o.insert("y".into(), json!(self.y));
            }
            if self.background {
                o.insert("side".into(), json!("background"));
            }
        }
        v
    }
}

/// The cache key of one glyph's coordinates: glyph, layer and side.
type GlyphKey = (String, String, bool);

/// The glyphs one glyph's measurement reads (itself and those its components use, by name; `None`
/// for a name that was missing), as they were. Holding the `Arc`s keeps them comparable by
/// pointer: a glyph that changes gets a new one.
type Sources = Vec<(String, Option<Arc<Glyph>>)>;

/// The font-level lists a measurement reads (the italic angle, smart components), as they were.
type FontLevel = (Arc<Vec<Master>>, Arc<Vec<MetricDef>>, Arc<Vec<Axis>>);

/// The Measurement tool's answers, kept across frames because the overlap test is slow.
#[derive(Default)]
struct MeasureCache {
    /// The document the entries belong to.
    doc: Option<u64>,
    level: Option<FontLevel>,
    /// Each glyph's coordinates and what they were measured from. Only complete answers are
    /// kept here, so an edit that leaves a glyph alone does not measure it again.
    glyphs: HashMap<GlyphKey, (Sources, Arc<Coordinates>)>,
    /// The last frame's line and answer, failures and partial answers included, so a line that
    /// cannot be measured in full is not measured again every frame.
    last: Option<(DocKey, Vec<Entry>, Arc<Vec<Placed>>)>,
    /// The last ruler query and its answer (`None` when it failed).
    ruler: Option<(DocKey, Value, Option<Arc<Ruler>>)>,
}

/// The active document, its revision and its font.
type DocKey = (u64, u64, usize);

thread_local! {
    static MEASURED: RefCell<MeasureCache> = RefCell::new(MeasureCache::default());
    /// How many glyphs `measure.coordinates` was asked to measure, for tests.
    static MEASURED_GLYPHS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many glyphs this thread has asked `measure.coordinates` to measure.
#[cfg(test)]
pub(crate) fn measured_glyphs() -> usize {
    MEASURED_GLYPHS.with(std::cell::Cell::get)
}

fn doc_key(session: &Session) -> Option<DocKey> {
    session.active().map(|d| (d.id, d.revision, Arc::as_ptr(&d.font) as usize))
}

/// [`Sources`] of glyph `name` in `font`. `None` when it uses more than [`MAX_SOURCE_GLYPHS`]
/// glyphs: such a glyph is measured again after every edit.
fn sources(font: &Font, name: &str) -> Option<Sources> {
    let mut out: Sources = Vec::new();
    let mut todo = vec![name.to_string()];
    while let Some(n) = todo.pop() {
        if out.iter().any(|(seen, _)| *seen == n) {
            continue;
        }
        if out.len() >= MAX_SOURCE_GLYPHS || todo.len() > MAX_SOURCE_GLYPHS {
            return None;
        }
        let glyph = font.glyphs.get(&n).cloned();
        for layer in glyph.iter().flat_map(|g| g.layers.iter()) {
            let background = layer.info.background.iter().flat_map(|b| b.shapes.iter());
            for shape in layer.shapes.iter().chain(background) {
                if let ModelShape::Component(c) = shape {
                    todo.push(c.glyph.clone());
                }
            }
        }
        out.push((n, glyph));
    }
    Some(out)
}

/// Whether `font` still has the glyphs of `sources` as they were.
fn unchanged(font: &Font, sources: &Sources) -> bool {
    sources.iter().all(|(name, was)| match (font.glyphs.get(name), was) {
        (Some(now), Some(was)) => Arc::ptr_eq(now, was),
        (None, None) => true,
        _ => false,
    })
}

/// The coordinates of every glyph of `line`, measured as layers of `target`'s document. Each
/// glyph's answer is kept until it, a glyph it uses or the masters change; the glyphs not kept
/// are measured in one query, nearest the edited glyph first, as its limits leave room.
fn coordinates(session: &mut Session, target: &Map<String, Value>, line: Vec<Entry>) -> Option<Arc<Vec<Placed>>> {
    let doc = doc_key(session)?;
    let hit = MEASURED.with(|m| m.borrow().last.as_ref().filter(|(k, l, _)| *k == doc && *l == line).map(|(_, _, v)| v.clone()));
    if hit.is_some() {
        return hit;
    }
    let missing: Vec<Entry> = {
        let font = session.font().ok()?;
        let level: FontLevel = (font.masters.clone(), font.metrics.clone(), font.axes.clone());
        MEASURED.with(|m| {
            let mut m = m.borrow_mut();
            let same_level =
                m.level.as_ref().is_some_and(|(a, b, c)| Arc::ptr_eq(a, &level.0) && Arc::ptr_eq(b, &level.1) && Arc::ptr_eq(c, &level.2));
            if m.doc != Some(doc.0) || !same_level {
                m.glyphs.clear();
                m.doc = Some(doc.0);
                m.level = Some(level);
            }
            m.glyphs.retain(|_, (sources, _)| unchanged(font, sources));
            let mut nearest: Vec<&Entry> = line.iter().collect();
            nearest.sort_by_key(|e| e.distance);
            let mut seen: HashSet<GlyphKey> = HashSet::new();
            nearest.into_iter().filter(|e| !m.glyphs.contains_key(&e.key()) && seen.insert(e.key())).cloned().collect()
        })
    };
    let mut fresh: HashMap<GlyphKey, Arc<Coordinates>> = HashMap::new();
    if !missing.is_empty() {
        let list: Vec<Value> = missing.iter().map(|e| e.placement(false)).collect();
        MEASURED_GLYPHS.with(|c| c.set(c.get().saturating_add(list.len())));
        let answer = session.execute("measure.coordinates", &with(target, "placements", Value::Array(list))).ok();
        let measured: Vec<Coordinates> =
            answer.and_then(|v| v.get("placements").cloned()).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let font = session.font().ok()?;
        for (e, coords) in missing.iter().zip(measured) {
            let key = e.key();
            let coords = Arc::new(coords);
            if !coords.overlaps_truncated
                && let Some(sources) = sources(font, &e.glyph)
            {
                MEASURED.with(|m| m.borrow_mut().glyphs.insert(key.clone(), (sources, coords.clone())));
            }
            fresh.insert(key, coords);
        }
    }
    let placed: Vec<Placed> = MEASURED.with(|m| {
        let mut m = m.borrow_mut();
        let used: Vec<GlyphKey> = line.iter().map(Entry::key).collect();
        let keep: HashSet<&GlyphKey> = used.iter().collect();
        m.glyphs.retain(|k, _| keep.contains(k));
        line.iter()
            .zip(used.iter())
            .filter_map(|(e, key)| {
                let coords = fresh.get(key).or_else(|| m.glyphs.get(key).map(|(_, c)| c))?.clone();
                Some(Placed { x: e.x, y: e.y, coords })
            })
            .collect()
    });
    let placed = Arc::new(placed);
    MEASURED.with(|m| m.borrow_mut().last = Some((doc, line, placed.clone())));
    Some(placed)
}

/// `measure.ruler` for `params`, kept while the document and the parameters stay the same.
fn ruler_crossings(session: &mut Session, params: Value) -> Option<Arc<Ruler>> {
    let doc = doc_key(session)?;
    let hit = MEASURED.with(|m| m.borrow().ruler.as_ref().filter(|(k, p, _)| *k == doc && *p == params).map(|(_, _, r)| r.clone()));
    if let Some(r) = hit {
        return r;
    }
    let r: Option<Arc<Ruler>> = query_as(session, "measure.ruler", params.clone()).map(Arc::new);
    MEASURED.with(|m| m.borrow_mut().ruler = Some((doc, params, r.clone())));
    r
}

/// The glyphs of the tab's text nearest the edited one, at most [`MAX_PLACEMENTS`], with their
/// layers and their positions from the edited glyph's origin, in text order. Without a layout,
/// the edited glyph alone.
fn line(session: &Session, layouts: &LayoutCache, tab: &Tab, layer: &str) -> Vec<Entry> {
    let alone = || {
        let glyph = tab.edited_glyph().unwrap_or_default().to_string();
        vec![Entry { glyph, layer: layer.to_string(), background: tab.edit_background, x: 0.0, y: 0.0, distance: 0 }]
    };
    let (Ok(font), Some(layout), Some(edited)) = (session.font(), super::view::text_layout(session, layouts, tab), tab.edited_index()) else {
        return alone();
    };
    let Some(origin) = layout.placed.get(edited) else { return alone() };
    let (ox, oy) = (origin.x, origin.y);
    let mut items: Vec<(usize, &Arc<str>, f64, f64)> =
        layout.placed.iter().enumerate().filter_map(|(i, p)| p.glyph.as_ref().map(|g| (i, g, p.x - ox, p.y - oy))).collect();
    items.sort_by_key(|(i, ..)| i.abs_diff(edited));
    items.truncate(MAX_PLACEMENTS);
    items.sort_by_key(|(i, ..)| *i);
    items
        .into_iter()
        .filter(|(_, _, x, y)| x.is_finite() && y.is_finite())
        .filter_map(|(i, glyph, x, y)| {
            let layer = super::view::shown_layer(font, tab, glyph)?;
            let background = i == edited && tab.edit_background;
            Some(Entry { glyph: glyph.to_string(), layer: layer.id.clone(), background, x, y, distance: i.abs_diff(edited) })
        })
        .collect()
}

/// What this module draws, read from the engine once per frame.
#[derive(Debug, Default)]
pub struct Extras {
    lines: Option<DisplayLines>,
    /// Measurement guides and where they cross the outline.
    guide_rulers: Vec<(GuideRef, Ruler)>,
    /// The Measurement tool's labels for each glyph of the line, while it is the active tool.
    coordinates: Option<Arc<Vec<Placed>>>,
    /// The Measurement tool's ruler and where it crosses the outline.
    ruler: Option<(Point, Point, Arc<Ruler>)>,
    /// The global guides, by index, whose filter leaves out the edited glyph.
    pub(crate) out_of_scope: Vec<usize>,
}

fn query_as<T: DeserializeOwned>(session: &mut Session, id: &str, params: Value) -> Option<T> {
    session.execute(id, &params).ok().and_then(|v| serde_json::from_value(v).ok())
}

/// `target` with `key` set to `value`.
fn with(target: &Map<String, Value>, key: &str, value: Value) -> Value {
    let mut m = target.clone();
    m.insert(key.into(), value);
    Value::Object(m)
}

impl Extras {
    /// Runs the queries the frame draws from: the display lines of the edited layer, the
    /// crossings of its measurement guides (while guides show), and, while `tool` is the
    /// Measurement tool, the coordinates and the crossings of its ruler (from `overlays`) over
    /// every glyph of the line `layouts` holds for the tab.
    pub fn query(session: &mut Session, tab: &Tab, display: &DisplayOptions, tool: &str, overlays: &[Overlay], layouts: &LayoutCache) -> Extras {
        let mut out = Extras::default();
        let Some(glyph) = tab.edited_glyph() else { return out };
        let data = session.glyph_data().ok();
        let (layer, measured) = {
            let Ok(font) = session.font() else { return out };
            let Some(layer) = super::view::edited_layer(font, tab) else { return out };
            let Some(master) = font.masters.iter().find(|m| m.id == layer.master) else { return out };
            let mut measured = Vec::new();
            if display.guides {
                if let (Some(data), Some(edited)) = (data.as_deref(), font.glyph(glyph)) {
                    out.out_of_scope = out_of_scope(&master.guides, edited, font, data, &master.id);
                }
                let hidden = &out.out_of_scope;
                let local = layer.info.guides.iter().enumerate().map(|(index, g)| (GuideScope::Local, index, g));
                let global =
                    master.guides.iter().enumerate().filter(|(index, _)| !hidden.contains(index)).map(|(index, g)| (GuideScope::Global, index, g));
                measured = local
                    .chain(global)
                    .filter(|(_, _, g)| g.show_measurement)
                    .map(|(scope, index, _)| GuideRef { scope, index })
                    .take(MAX_MEASURED_GUIDES)
                    .collect();
            }
            (layer.id.clone(), measured)
        };
        let mut target = Map::new();
        target.insert("glyph".into(), json!(glyph));
        target.insert("layer".into(), json!(layer));
        if tab.edit_background {
            target.insert("side".into(), json!("background"));
        }
        out.lines = query_as(session, "guide.displayLines", Value::Object(target.clone()));
        for g in measured {
            let params = with(&target, "guide", json!({"scope": g.scope.as_str(), "index": g.index}));
            if let Some(r) = query_as(session, "measure.ruler", params) {
                out.guide_rulers.push((g, r));
            }
        }
        if tool == "measurement" {
            let glyphs = line(session, layouts, tab, &layer);
            // Nearest first, so that a line past the element limit loses its farthest glyphs.
            let mut nearest: Vec<&Entry> = glyphs.iter().collect();
            nearest.sort_by_key(|e| e.distance);
            let placements: Vec<Value> = nearest.into_iter().map(|e| e.placement(true)).collect();
            out.coordinates = coordinates(session, &target, glyphs);
            target.insert("placements".into(), Value::Array(placements));
            let ruler = overlays.iter().find_map(|o| match o {
                Overlay::Line { kind: OverlayKind::Ruler, from, to } => Some((*from, *to)),
                _ => None,
            });
            if let Some((from, to)) = ruler {
                let mut params = target.clone();
                params.insert("from".into(), json!([from.x, from.y]));
                params.insert("to".into(), json!([to.x, to.y]));
                if let Some(r) = ruler_crossings(session, Value::Object(params)) {
                    out.ruler = Some((from, to, r));
                }
            }
        }
        out
    }
}

/// The global guides of `master`, by index, whose filter leaves out the edited glyph of `tab`.
pub(crate) fn hidden_global_guides(session: &Session, font: &Font, tab: &Tab) -> Vec<usize> {
    let Some(layer) = super::view::active_layer(font, tab) else { return Vec::new() };
    let Some(master) = font.masters.iter().find(|m| m.id == layer.master) else { return Vec::new() };
    if master.guides.iter().all(|g| g.filter.as_deref().is_none_or(|f| f.trim().is_empty())) {
        return Vec::new();
    }
    let (Some(glyph), Ok(data)) = (tab.edited_glyph().and_then(|g| font.glyph(g)), session.glyph_data()) else { return Vec::new() };
    out_of_scope(&master.guides, glyph, font, &data, &master.id)
}

fn out_of_scope(guides: &[Guide], glyph: &Glyph, font: &Font, data: &GlyphData, master: &str) -> Vec<usize> {
    guides.iter().enumerate().filter(|(_, g)| !in_scope(g, glyph, font, data, master)).map(|(index, _)| index).collect()
}

/// Whether a global guide shows in `glyph`: always without a filter, or with one that does not
/// read as a predicate.
fn in_scope(guide: &Guide, glyph: &Glyph, font: &Font, data: &GlyphData, master: &str) -> bool {
    let Some(text) = guide.filter.as_deref().map(str::trim).filter(|t| !t.is_empty()) else { return true };
    let Ok(scope) = fontcraft_glyphquery::parse_predicate(text) else { return true };
    let Ok(env) = fontcraft_glyphquery::Env::new(font, data, Some(master)) else { return true };
    fontcraft_glyphquery::guide_in_scope(Some(&scope), glyph, &env)
}

/// A number as a label: at most two decimals, without trailing zeros.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "?".into();
    }
    let r = (v * 100.0).round() / 100.0;
    let text = format!("{r:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn screen(c: &Painting, x: f64, y: f64) -> Pos2 {
    pos(c.view.to_screen(Point::new(x, y)))
}

/// A colour from a guide's `r,g,b,a` text (each from 0 to 1).
fn parse_color(text: &str) -> Option<Color32> {
    let parts: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| (0.0..=1.0).contains(v))).collect::<Option<_>>()?;
    let byte = |v: f64| (v * 255.0).round() as u8;
    match parts.as_slice() {
        [r, g, b, a] => Some(Color32::from_rgba_unmultiplied(byte(*r), byte(*g), byte(*b), byte(*a))),
        _ => None,
    }
}

/// The part of the line through `at` with direction `dir` (screen points) inside `rect`
/// (Liang–Barsky clipping), or `None` when it misses the rectangle.
fn clip(at: Pos2, dir: (f32, f32), rect: Rect) -> Option<(Pos2, Pos2)> {
    let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
    for (d, p, min, max) in [(dir.0, at.x, rect.left(), rect.right()), (dir.1, at.y, rect.top(), rect.bottom())] {
        if d.abs() < 1e-9 {
            if p < min || p > max {
                return None;
            }
            continue;
        }
        let (a, b) = ((min - p) / d, (max - p) / d);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
    }
    (lo.is_finite() && hi.is_finite() && lo <= hi)
        .then(|| (Pos2::new(at.x + lo * dir.0, at.y + lo * dir.1), Pos2::new(at.x + hi * dir.0, at.y + hi * dir.1)))
}

/// A guide's line on the canvas, its left end first (the lower end of a vertical guide).
fn guide_line(c: &Painting, g: &Guide) -> Option<(Pos2, Pos2)> {
    let at = screen(c, g.x, g.y);
    let a = if g.angle.is_finite() { g.angle.to_radians() } else { 0.0 };
    if !usable(at) {
        return None;
    }
    // Screen y grows downwards.
    let (a, b) = clip(at, (a.cos() as f32, -(a.sin() as f32)), c.rect)?;
    Some(if (a.x, -a.y) <= (b.x, -b.y) { (a, b) } else { (b, a) })
}

/// A padlock: a body and a shackle, centred on `at`.
fn padlock(at: Pos2, size: f32, color: Color32) -> Vec<Shape> {
    let body = Rect::from_center_size(Pos2::new(at.x, at.y + size * 0.35), egui::vec2(size * 1.6, size * 1.2));
    let shackle: Vec<Pos2> = (0..=8)
        .map(|k| {
            let t = std::f32::consts::PI * k as f32 / 8.0;
            Pos2::new(at.x + size * 0.55 * t.cos(), body.top() - size * 0.7 * t.sin())
        })
        .collect();
    vec![Shape::rect_filled(body, 1.0, color), Shape::line(shackle, Stroke::new(1.5, color))]
}

/// Behind the outline: the grid, the metric lines parameters set, the CJK and design grids.
pub fn draw_lines(c: &Painting, extras: &Extras, display: &DisplayOptions) {
    let Some(lines) = &extras.lines else { return };
    let tokens = c.tokens;
    let rect = c.rect;
    let (top_left, bottom_right) = (
        c.view.to_font(Point::new(f64::from(rect.left()), f64::from(rect.top()))),
        c.view.to_font(Point::new(f64::from(rect.right()), f64::from(rect.bottom()))),
    );
    let scale = c.view.scale();

    // The grid, once its step is wide enough on screen to read.
    let step = lines.grid.step;
    if step.is_finite() && step > 0.0 && step * scale >= f64::from(tokens.grid_min_spacing) {
        let stroke = Stroke::new(1.0, tokens.grid_line);
        for x in steps(top_left.x, bottom_right.x, step) {
            let sx = screen(c, x, 0.0).x;
            c.painter.line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], stroke);
        }
        for y in steps(bottom_right.y, top_left.y, step) {
            let sy = screen(c, 0.0, y).y;
            c.painter.line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], stroke);
        }
    }

    if display.metrics {
        let stroke = Stroke::new(tokens.metric_line_width, tokens.metric_line);
        for line in lines.metrics.iter().filter(|l| l.source == "parameter" && l.y.is_finite()) {
            let y = screen(c, 0.0, line.y).y;
            c.painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
            if display.metrics_names {
                let name = line.name.as_deref().unwrap_or(&line.kind);
                let font = FontId::proportional(tokens.metric_name_size);
                c.painter.text(
                    Pos2::new(rect.left() + 4.0, y - 2.0),
                    Align2::LEFT_BOTTOM,
                    format!("{name} {}", num(line.y)),
                    font,
                    tokens.metric_name,
                );
            }
        }
    }

    let dotted = |a: Pos2, b: Pos2| {
        if usable(a) && usable(b) {
            c.painter.extend(Shape::dotted_line(&[a, b], tokens.cjk_grid, DOT_SPACING, DOT_RADIUS));
        }
    };
    let ys: Vec<f64> = lines.metrics.iter().map(|l| l.y).filter(|y| y.is_finite()).collect();
    let body = match &lines.cjk {
        Some(cjk) => Some(cjk.bounds),
        None => ys.iter().copied().reduce(f64::min).zip(ys.iter().copied().reduce(f64::max)).map(|(lo, hi)| [0.0, lo, lines.width, hi]),
    };
    if let Some(cjk) = &lines.cjk {
        let [x0, y0, x1, y1] = cjk.bounds;
        for (count, vertical) in [(cjk.columns, true), (cjk.rows, false)] {
            let Some(n) = count.filter(|n| n.is_finite() && *n >= 1.0 && *n <= MAX_GRID_LINES as f64) else { continue };
            for k in 1..(n as usize) {
                let t = k as f64 / n;
                if vertical {
                    let x = x0 + (x1 - x0) * t;
                    dotted(screen(c, x, y0), screen(c, x, y1));
                } else {
                    let y = y0 + (y1 - y0) * t;
                    dotted(screen(c, x0, y), screen(c, x1, y));
                }
            }
        }
        if let Some([gx0, gy0, gx1, gy1]) = cjk.guide {
            let corners = vec![screen(c, gx0, gy0), screen(c, gx1, gy0), screen(c, gx1, gy1), screen(c, gx0, gy1)];
            if corners.iter().all(|p| usable(*p)) {
                c.painter.add(Shape::closed_line(corners, Stroke::new(1.0, tokens.cjk_grid)));
            }
        }
    }
    if let (Some(values), Some([x0, y0, x1, y1])) = (&lines.design_grid, body) {
        let h = values.first().copied().unwrap_or(0.0);
        let v = values.get(1).copied().unwrap_or(h);
        let readable = |s: f64| s.is_finite() && s > 0.0 && s * scale >= f64::from(tokens.grid_min_spacing);
        if readable(h) {
            for x in steps(x0, x1, h) {
                dotted(screen(c, x, y0), screen(c, x, y1));
            }
        }
        if readable(v) {
            for y in steps(y0, y1, v) {
                dotted(screen(c, x0, y), screen(c, x1, y));
            }
        }
    }
}

/// The multiples of `step` from `lo` to `hi`, at most [`MAX_GRID_LINES`] of them.
fn steps(lo: f64, hi: f64, step: f64) -> impl Iterator<Item = f64> {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let first = if step > 0.0 && lo.is_finite() { (lo / step).ceil() } else { f64::INFINITY };
    (0..MAX_GRID_LINES).map(move |k| (first + k as f64) * step).take_while(move |v| v.is_finite() && *v <= hi)
}

/// Over the outline: the guides of `layer` and of its master, with knobs and names, and what
/// measurement guides measure.
pub fn draw_guides(c: &Painting, font: &Font, layer: &Layer, selection: &Selection, extras: &Extras, display: &DisplayOptions) {
    if !display.guides {
        return;
    }
    let tokens = c.tokens;
    let global: &[Guide] = font.master(&layer.master).map(|m| m.guides.as_slice()).unwrap_or_default();
    let local = layer.info.guides.iter().enumerate().map(|(index, g)| (GuideRef { scope: GuideScope::Local, index }, g));
    let global = global
        .iter()
        .enumerate()
        .filter(|(index, _)| !extras.out_of_scope.contains(index))
        .map(|(index, g)| (GuideRef { scope: GuideScope::Global, index }, g));
    let color_of = |r: GuideRef, g: &Guide| {
        g.color.as_deref().and_then(parse_color).unwrap_or(match r.scope {
            GuideScope::Local => tokens.guide_local,
            GuideScope::Global => tokens.guide_global,
        })
    };
    let mut colors = BTreeMap::new();
    for (r, g) in local.chain(global) {
        let color = color_of(r, g);
        colors.insert(r, color);
        let Some((left, right)) = guide_line(c, g) else { continue };
        c.painter.line_segment([left, right], Stroke::new(tokens.guide_width, color));
        let knob = screen(c, g.x, g.y);
        if usable(knob) {
            if g.locked {
                c.painter.extend(padlock(knob, tokens.guide_knob_radius, color));
            } else if selection.guides.contains(&r) {
                c.painter.circle_filled(knob, tokens.guide_knob_radius, color);
            } else {
                c.painter.circle_stroke(knob, tokens.guide_knob_radius, Stroke::new(tokens.guide_width, color));
            }
        }
        if let Some(name) = g.name.as_deref().filter(|n| !n.is_empty()) {
            let font = FontId::proportional(tokens.guide_name_size);
            c.painter.text(Pos2::new(left.x + LABEL_OFFSET, left.y - 2.0), Align2::LEFT_BOTTOM, name, font, color);
        }
    }
    for (r, ruler) in &extras.guide_rulers {
        let color = colors.get(r).copied().unwrap_or(tokens.measure_ruler);
        draw_crossings(c, ruler, color);
    }
}

/// Dots at a ruler's crossings and the distance of each gap at its middle.
fn draw_crossings(c: &Painting, ruler: &Ruler, color: Color32) {
    let tokens = c.tokens;
    let points: Vec<Pos2> = ruler.crossings.iter().map(|p| screen(c, p.x, p.y)).collect();
    for p in points.iter().filter(|p| usable(**p)) {
        c.painter.circle_filled(*p, tokens.measure_crossing_radius, color);
    }
    let font = FontId::proportional(tokens.measure_label_size);
    for gap in ruler.gaps.iter().take(MAX_LABELS) {
        let (Some(a), Some(b)) = (points.get(gap.from), points.get(gap.to)) else { continue };
        let mid = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        if usable(mid) {
            c.painter.text(Pos2::new(mid.x, mid.y - LABEL_OFFSET), Align2::CENTER_BOTTOM, num(gap.distance), font.clone(), color);
        }
    }
}

/// Over everything: the Measurement tool's coordinates, deltas and ruler.
pub fn draw_measurement(c: &Painting, extras: &Extras) {
    let tokens = c.tokens;
    let font = FontId::proportional(tokens.measure_label_size);
    if let Some(list) = &extras.coordinates {
        let overlap = Stroke::new(tokens.outline_width, tokens.measure_overlap);
        let mut segments = 0usize;
        'lines: for placed in list.iter() {
            for line in &placed.coords.overlaps {
                let pts: Vec<Pos2> = line.iter().take(MAX_OVERLAP_SEGMENTS).map(|[x, y]| screen(c, x + placed.x, y + placed.y)).collect();
                for pair in pts.windows(2) {
                    segments = segments.saturating_add(1);
                    if segments > MAX_OVERLAP_SEGMENTS {
                        break 'lines;
                    }
                    if let [a, b] = pair
                        && usable(*a)
                        && usable(*b)
                    {
                        c.painter.line_segment([*a, *b], overlap);
                    }
                }
            }
        }
        let mut labels = 0usize;
        for placed in list.iter() {
            if labels >= MAX_LABELS {
                break;
            }
            labels = labels.saturating_add(draw_coordinates(c, placed, MAX_LABELS.saturating_sub(labels), &font));
        }
    }
    if let Some((from, to, ruler)) = &extras.ruler {
        let (a, b) = (pos(c.view.to_screen(*from)), pos(c.view.to_screen(*to)));
        // The line itself is an overlay of the tool (`selection::overlay_shapes`).
        if usable(a)
            && usable(b)
            && let Some(angle) = ruler.angle
        {
            c.painter.text(
                Pos2::new(b.x + LABEL_OFFSET, b.y - LABEL_OFFSET),
                Align2::LEFT_BOTTOM,
                format!("{}°", num(angle)),
                font.clone(),
                tokens.measure_ruler,
            );
        }
        draw_crossings(c, ruler, tokens.measure_ruler);
    }
}

/// One glyph's coordinates, anchors and deltas, at most `limit` labels. Returns how many it drew.
fn draw_coordinates(c: &Painting, placed: &Placed, limit: usize, font: &FontId) -> usize {
    let tokens = c.tokens;
    let (ox, oy, coords) = (placed.x, placed.y, &placed.coords);
    let mut drawn = 0usize;
    let mut at: BTreeMap<(usize, usize), Pos2> = BTreeMap::new();
    for p in coords.points.iter().take(limit) {
        let s = screen(c, p.x + ox, p.y + oy);
        at.insert((p.shape, p.node), s);
        if usable(s) {
            let color = if p.on_curve { tokens.measure_on_curve } else { tokens.measure_off_curve };
            c.painter.text(
                Pos2::new(s.x + LABEL_OFFSET, s.y - LABEL_OFFSET),
                Align2::LEFT_BOTTOM,
                format!("{}, {}", num(p.x), num(p.y)),
                font.clone(),
                color,
            );
        }
        drawn = drawn.saturating_add(1);
    }
    for a in coords.anchors.iter().take(limit.saturating_sub(drawn)) {
        let s = screen(c, a.x + ox, a.y + oy);
        if usable(s) {
            c.painter.text(
                Pos2::new(s.x + LABEL_OFFSET, s.y - LABEL_OFFSET),
                Align2::LEFT_BOTTOM,
                format!("{}, {}", num(a.x), num(a.y)),
                font.clone(),
                tokens.measure_on_curve,
            );
        }
        drawn = drawn.saturating_add(1);
    }
    for d in coords.deltas.iter().take(limit.saturating_sub(drawn)) {
        drawn = drawn.saturating_add(1);
        let (Some(a), Some(b)) = (at.get(&(d.shape, d.from)), at.get(&(d.shape, d.to))) else { continue };
        let mid = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        if usable(mid) {
            c.painter.text(mid, Align2::CENTER_CENTER, format!("{}, {}", num(d.italic_dx), num(d.dy)), font.clone(), tokens.measure_delta);
        }
    }
    drawn
}
