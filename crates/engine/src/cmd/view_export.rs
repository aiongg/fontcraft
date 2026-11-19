//! Export Window Contents (M13.68). Row: `file.exportWindowContents`.
//!
//! The shell keeps its tabs, so the command takes a tab's contents as parameters: its text as a
//! display string, its master, the layers picked for its glyphs, its direction, kerning mode and
//! line width. The text is laid out as Edit View lays it out (`fontcraft_textlayout::layout`,
//! without the feature substitutions) and drawn with the proof writers
//! (`fontcraft_proof::write_vector`):
//!
//! - PDF: the glyphs filled black, like the preview of the text.
//! - SVG: the editing view, with each glyph in a light fill and a thin contour, and the edited
//!   glyphs' nodes, handles and guides (their own and their master's).
//!
//! The page spans every glyph's advance from the master's descender to its ascender, widened to
//! the outlines, with a tenth of the em around it. Guides are drawn across the page.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use base64::Engine as _;
use fontcraft_geom::{BezPath, PathEl, Point, Rect, Shape as _};
use fontcraft_model::{Font, Guide, Layer, Master, NodeKind, Shape, metric};
use fontcraft_proof::{Mark, Segment, VectorFormat, VectorGlyph, VectorInk, VectorPage};
use fontcraft_textlayout::{Direction, KerningMode, LENGTH_RANGE, Options, Text};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Longest display string, in bytes; the tab's item limit applies after it.
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Allowed em sizes, in points.
pub const SIZE_RANGE: std::ops::RangeInclusive<f64> = 1.0..=10_000.0;
/// Most entries of `layers` and of `edited`.
const MAX_PICKS: usize = fontcraft_textlayout::MAX_ITEMS;
/// The space left around the drawing, as a fraction of the em.
const MARGIN_EM: f64 = 0.1;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "file.exportWindowContents",
            "Export Window Contents…",
            ["File", "Export"],
            None,
            "{text: string (the tab's text as a display string: characters, /name for a glyph by name, line breaks), format: \"pdf\"|\"svg\", master?: id or name (default: the first master), layers?: {glyph: layer id} (glyphs shown in another layer than their master's), direction?: \"ltr\"|\"rtl\"|\"ttb\" (default ltr), kerning?: \"none\"|\"kerning\"|\"locked\" (default kerning), lineWidth?: font units (where lines wrap, 1 to 1000000, default 10000), edited?: [item index] (SVG: the glyphs that show nodes, handles and guides; default every glyph), size?: points per em (1 to 10000, default the units per em: one point per unit; scaled down so no side passes 14400 points), path?: file} write what a tab shows as one vector page. The text is laid out as Edit View lays it out, without feature substitutions. PDF fills the glyphs black, like the preview; SVG draws the editing view: each glyph in a light fill with a thin contour, and on the edited glyphs their nodes (filled squares on the curve, open circles off it), handles, and guides (the layer's own and its master's) across the page. Open paths are not filled. The page spans every glyph's advance from the master's descender to its ascender, widened to the outlines, with a tenth of the em around it. Glyphs the font lacks take room and draw nothing. Without a path the file comes back as dataBase64 → {format, width, height (points), scale (points per unit), glyphs: [names drawn, in text order], warnings, bytes, path? | dataBase64?}",
            has_doc,
            export_window
        ).files(const { &[FileParam::write("path")] }).writes_files(),
    ]
}

/// Where the tab's text puts one glyph.
struct Drawn<'f> {
    name: &'f str,
    layer: &'f Layer,
    x: f64,
    y: f64,
    /// Its text item, for `edited`.
    item: usize,
}

fn export_window(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.exportWindowContents", v)?;
    let format = match p.str("format")? {
        "pdf" => VectorFormat::Pdf,
        "svg" => VectorFormat::Svg,
        other => return Err(p.bad(format!("`format` must be \"pdf\" or \"svg\", got \"{}\"", other.chars().take(64).collect::<String>()))),
    };
    let text = p.str("text")?;
    if text.is_empty() {
        return Err(p.bad("`text` is empty"));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(p.bad(format!("`text` is longer than {MAX_TEXT_BYTES} bytes")));
    }
    let direction = match p.opt_str("direction")? {
        None | Some("ltr") => Direction::Ltr,
        Some("rtl") => Direction::Rtl,
        Some("ttb") => Direction::Vertical,
        Some(other) => {
            return Err(p.bad(format!("`direction` must be \"ltr\", \"rtl\" or \"ttb\", got \"{}\"", other.chars().take(64).collect::<String>())));
        }
    };
    let kerning = match p.opt_str("kerning")? {
        None => KerningMode::Kerning,
        Some(name) => KerningMode::from_name(name).ok_or_else(|| {
            p.bad(format!("`kerning` must be \"none\", \"kerning\" or \"locked\", got \"{}\"", name.chars().take(64).collect::<String>()))
        })?,
    };
    let line_width = p.opt_f64("lineWidth")?.unwrap_or(fontcraft_textlayout::DEFAULT_LINE_WIDTH);
    if !LENGTH_RANGE.contains(&line_width) {
        return Err(p.bad(format!("`lineWidth` must be from {} to {} units", LENGTH_RANGE.start(), LENGTH_RANGE.end())));
    }
    let path = match p.opt_str("path")? {
        Some(path) if path.trim().is_empty() => return Err(p.bad("`path` is empty")),
        Some(path) => Some(PathBuf::from(path)),
        None => None,
    };
    let layers = layer_picks(&p)?;
    let index = if kerning.shows_kerning() { Some(s.kern_index()?) } else { None };
    let font = s.font()?;
    let master = match p.opt_str("master")? {
        Some(m) => font
            .masters
            .iter()
            .find(|x| x.id == m)
            .or_else(|| font.masters.iter().find(|x| x.name == m))
            .ok_or_else(|| p.bad(format!("no master has the id or name `{m}`")))?,
        None => font.masters.first().ok_or_else(|| p.bad("the font has no master"))?,
    };
    let upm = f64::from(font.units_per_em.max(1));
    let size = p.opt_f64("size")?.unwrap_or(upm);
    if !SIZE_RANGE.contains(&size) {
        return Err(p.bad(format!("`size` must be from {} to {} points per em", SIZE_RANGE.start(), SIZE_RANGE.end())));
    }

    let tab_text = Text::from_display_string(font, text).map_err(|e| p.bad(e.to_string()))?;
    let edited = edited_items(&p, tab_text.len())?;
    let options = Options { direction, kerning, line_width, index: index.as_deref(), layers: Some(&layers), cursive: true, ..Default::default() };
    let layout = fontcraft_textlayout::layout(font, &master.id, &tab_text, &options).map_err(|e| p.bad(e.to_string()))?;

    // What is drawn where, in font units.
    let mut drawn = Vec::new();
    for (item, placed) in layout.placed.iter().enumerate() {
        let first = placed.glyph.as_deref().map(|g| (g, placed.x, placed.y));
        for (name, x, y) in first.into_iter().chain(placed.extra.iter().map(|(g, x, y)| (g.as_ref(), *x, *y))) {
            if let Some((name, layer)) = shown_layer(font, master, &layers, name) {
                drawn.push(Drawn { name, layer, x, y, item });
            }
        }
    }

    let mut warnings = Vec::new();
    let mut outlines: HashMap<(&str, &str), Option<(usize, BezPath)>> = HashMap::new();
    let mut glyphs: Vec<VectorGlyph> = Vec::new();
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    let (low, high) = (descender.min(ascender), descender.max(ascender));
    let mut frame: Option<Rect> = None;
    let mut widen = |r: Rect| frame = Some(frame.map_or(r, |f| f.union(r)));
    // Advances of the items that draw nothing (missing characters) still take room.
    for placed in &layout.placed {
        if placed.glyph.is_none() && placed.advance > 0.0 {
            widen(Rect::new(placed.x, placed.y + low, placed.x + placed.advance, placed.y + high));
        }
    }
    for d in &drawn {
        let key = (d.name, d.layer.id.as_str());
        if !outlines.contains_key(&key) {
            let entry = match fontcraft_render::layer_outline(font, d.layer) {
                Ok(outline) => {
                    let outline = fontcraft_render::closed_contours(&outline);
                    let index = glyphs.len();
                    glyphs.push(VectorGlyph { name: d.name.to_string(), segments: segments(&outline) });
                    Some((index, outline))
                }
                Err(e) => {
                    warnings.push(format!("glyph `{}` is not drawn: {e}", d.name));
                    None
                }
            };
            outlines.insert(key, entry);
        }
        let width = if d.layer.width.is_finite() { d.layer.width.max(0.0) } else { 0.0 };
        widen(Rect::new(d.x, d.y + low, d.x + width, d.y + high));
        if let Some(Some((_, outline))) = outlines.get(&key)
            && !outline.elements().is_empty()
        {
            let b = outline.bounding_box();
            widen(Rect::new(d.x + b.x0, d.y + b.y0, d.x + b.x1, d.y + b.y1));
        }
    }
    let Some(frame) = frame else { return Err(p.bad("the text draws nothing")) };
    if ![frame.x0, frame.y0, frame.x1, frame.y1].iter().all(|v| v.is_finite()) {
        return Err(p.bad("the text has a coordinate or metric that is not a finite number"));
    }
    let frame = frame.inflate(upm * MARGIN_EM, upm * MARGIN_EM);
    let max_side = *fontcraft_proof::VECTOR_PAGE_SIDE.end();
    let longest = frame.width().max(frame.height());
    let scale = if longest * size / upm > max_side { max_side / longest } else { size / upm };
    let page_w = (frame.width() * scale).clamp(1.0, max_side);
    let page_h = (frame.height() * scale).clamp(1.0, max_side);
    let to_page = |x: f64, y: f64| [(x - frame.x0) * scale, (frame.y1 - y) * scale];

    let mut marks = Marks::default();
    let mut names = Vec::with_capacity(drawn.len());
    for d in &drawn {
        names.push(d.name);
        if let Some(Some((glyph, _))) = outlines.get(&(d.name, d.layer.id.as_str())) {
            let [x, y] = to_page(d.x, d.y);
            marks.push(Mark::Glyph { glyph: *glyph, x, y })?;
        }
    }
    let outlined = format == VectorFormat::Svg;
    if outlined {
        let mut seen = HashSet::new();
        for d in drawn.iter().filter(|d| edited.as_ref().is_none_or(|e| e.contains(&d.item))) {
            let guides = d.layer.info.guides.iter().chain(&master.guides);
            for g in guides {
                if let Some((from, to)) = guide_line(g, |x, y| to_page(d.x + x, d.y + y), [page_w, page_h])
                    && seen.insert(key_of(from, to))
                {
                    marks.push(Mark::Line { from, to, ink: VectorInk::Guide })?;
                }
            }
        }
        for d in drawn.iter().filter(|d| edited.as_ref().is_none_or(|e| e.contains(&d.item))) {
            nodes(d.layer, |x, y| to_page(d.x + x, d.y + y), &mut marks)?;
        }
    }

    let page = VectorPage {
        title: if font.family_name.is_empty() { "Window".to_string() } else { font.family_name.clone() },
        size: [page_w, page_h],
        scale,
        outlined,
        glyphs,
        marks: marks.0,
    };
    let bytes = fontcraft_proof::write_vector(&page, format)?;
    let extension = if format == VectorFormat::Pdf { "pdf" } else { "svg" };
    let mut out = json!({
        "format": extension,
        "width": page_w,
        "height": page_h,
        "scale": scale,
        "glyphs": names,
        "warnings": warnings,
        "bytes": bytes.len(),
    });
    if let Value::Object(m) = &mut out {
        match &path {
            Some(path) => {
                super::file_io::write_output(path, &bytes, &super::file_io::OutputRequest::default())?;
                m.insert("path".into(), json!(path.to_string_lossy()));
            }
            None => {
                m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(&bytes)));
            }
        }
    }
    Ok(out)
}

/// `layers`: glyph name → layer id.
fn layer_picks(p: &Params) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let Some(map) = p.opt_object("layers")? else { return Ok(out) };
    if map.len() > MAX_PICKS {
        return Err(p.bad(format!("`layers` names at most {MAX_PICKS} glyphs")));
    }
    for (glyph, layer) in map {
        let Some(id) = layer.as_str() else { return Err(p.bad(format!("`layers.{glyph}` must be a layer id"))) };
        out.insert(glyph.clone(), id.to_string());
    }
    Ok(out)
}

/// `edited`: item indices of the text; `None` when every glyph is edited.
fn edited_items(p: &Params, items: usize) -> Result<Option<HashSet<usize>>> {
    let Some(list) = p.opt_array("edited")? else { return Ok(None) };
    if list.len() > MAX_PICKS {
        return Err(p.bad(format!("`edited` lists at most {MAX_PICKS} items")));
    }
    let mut out = HashSet::with_capacity(list.len());
    for v in list {
        match v.as_u64().and_then(|i| usize::try_from(i).ok()) {
            Some(i) if i < items => {
                out.insert(i);
            }
            _ => return Err(p.bad(format!("`edited` holds item indices below {items}, not {v}"))),
        }
    }
    Ok(Some(out))
}

/// The layer a tab shows for `name`: the one `layers` picks when the glyph has it, else its
/// layer of the master. The UI's canvas picks it the same way.
fn shown_layer<'f>(font: &'f Font, master: &Master, layers: &BTreeMap<String, String>, name: &str) -> Option<(&'f str, &'f Layer)> {
    let glyph = font.glyph(name)?;
    let picked = layers.get(name).and_then(|id| glyph.layers.iter().find(|l| l.id == *id));
    picked.or_else(|| glyph.layer(&master.id)).map(|l| (glyph.name.as_str(), l))
}

fn segments(path: &BezPath) -> Vec<Segment> {
    let xy = |p: Point| [p.x, p.y];
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) => Segment::Move(xy(p)),
            PathEl::LineTo(p) => Segment::Line(xy(p)),
            PathEl::QuadTo(c, p) => Segment::Quad(xy(c), xy(p)),
            PathEl::CurveTo(a, b, p) => Segment::Cubic(xy(a), xy(b), xy(p)),
            PathEl::ClosePath => Segment::Close,
        })
        .collect()
}

/// The marks of the page, within the writers' limit.
#[derive(Default)]
struct Marks(Vec<Mark>);

impl Marks {
    fn push(&mut self, mark: Mark) -> Result<()> {
        if self.0.len() >= fontcraft_proof::MAX_VECTOR_MARKS {
            return Err(EngineError::Unsupported(format!(
                "drawing more than {} glyphs, nodes, handles and guides on one page; export less text or fewer edited glyphs",
                fontcraft_proof::MAX_VECTOR_MARKS
            )));
        }
        self.0.push(mark);
        Ok(())
    }
}

/// The nodes and handles of the layer's paths; `at` turns font units around the glyph's origin
/// into page points. A handle joins a control point to the on-curve point next to it, and two
/// control points of a quadratic run to each other.
fn nodes(layer: &Layer, at: impl Fn(f64, f64) -> [f64; 2], marks: &mut Marks) -> Result<()> {
    for shape in &layer.shapes {
        let Shape::Path(path) = shape else { continue };
        let ns = &path.nodes;
        let count = ns.len();
        let off = |i: usize| ns.get(i).is_some_and(|n| n.kind == NodeKind::OffCurve);
        for (i, n) in ns.iter().enumerate() {
            let next = if i + 1 < count {
                Some(i + 1)
            } else if path.closed && count > 1 {
                Some(0)
            } else {
                None
            };
            let Some(j) = next else { continue };
            let Some(m) = ns.get(j) else { continue };
            let handle = match (off(i), off(j)) {
                (true, false) | (false, true) => true,
                // Two control points in a row: a handle only in a quadratic run (whose segment
                // ends in a qcurve node, or which has no on-curve node at all).
                (true, true) => {
                    (1..=count).map(|k| (j + k) % count).find(|&k| !off(k)).and_then(|k| ns.get(k)).is_none_or(|e| e.kind == NodeKind::QCurve)
                }
                (false, false) => false,
            };
            if handle {
                marks.push(Mark::Line { from: at(n.x, n.y), to: at(m.x, m.y), ink: VectorInk::Handle })?;
            }
        }
        for n in ns {
            marks.push(Mark::Node { at: at(n.x, n.y), on_curve: n.kind != NodeKind::OffCurve })?;
        }
    }
    Ok(())
}

/// The part of guide `g` inside a page of `size` points; `at` turns font units around the
/// glyph's origin into page points.
fn guide_line(g: &Guide, at: impl Fn(f64, f64) -> [f64; 2], size: [f64; 2]) -> Option<([f64; 2], [f64; 2])> {
    if ![g.x, g.y, g.angle].iter().all(|v| v.is_finite()) {
        return None;
    }
    let [px, py] = at(g.x, g.y);
    let rad = g.angle.to_radians();
    // Page y grows downwards, so the counter-clockwise angle turns the other way.
    let (dx, dy) = (rad.cos(), -rad.sin());
    let [w, h] = size;
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (start, d, max) in [(px, dx, w), (py, dy, h)] {
        if d.abs() < 1e-12 {
            if !(0.0..=max).contains(&start) {
                return None;
            }
            continue;
        }
        let (a, b) = (-start / d, (max - start) / d);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    if !(t0.is_finite() && t1.is_finite()) || t0 >= t1 {
        return None;
    }
    Some(([px + t0 * dx, py + t0 * dy], [px + t1 * dx, py + t1 * dy]))
}

/// A line rounded to a thousandth of a point, so a guide shown at the same place by several
/// glyphs is drawn once.
fn key_of(from: [f64; 2], to: [f64; 2]) -> [i64; 4] {
    let r = |v: f64| (v * 1000.0).round() as i64;
    let [a, b] = from;
    let [c, d] = to;
    [r(a), r(b), r(c), r(d)]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn guide(x: f64, y: f64, angle: f64) -> Guide {
        Guide { x, y, angle, ..Default::default() }
    }

    #[test]
    fn guides_cross_the_whole_page() {
        let id = |x: f64, y: f64| [x, 100.0 - y];
        // A horizontal guide at y = 40 runs from edge to edge.
        assert_eq!(guide_line(&guide(30.0, 40.0, 0.0), id, [200.0, 100.0]), Some(([0.0, 60.0], [200.0, 60.0])));
        // A vertical one, outside the page, is dropped.
        assert_eq!(guide_line(&guide(300.0, 0.0, 90.0), id, [200.0, 100.0]), None);
        // A diagonal through the page's centre meets two edges.
        let (from, to) = guide_line(&guide(100.0, 50.0, 45.0), id, [200.0, 100.0]).unwrap();
        assert!((from[0] - 50.0).abs() < 1e-9 && (from[1] - 100.0).abs() < 1e-9, "{from:?}");
        assert!((to[0] - 150.0).abs() < 1e-9 && to[1].abs() < 1e-9, "{to:?}");
        assert_eq!(guide_line(&guide(f64::NAN, 0.0, 0.0), id, [200.0, 100.0]), None);
    }
}
