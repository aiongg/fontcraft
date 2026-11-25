//! Strokes, masks and shape order (`docs/spec/03a-edit-view-drawing.md` §§ 17–18): the action
//! behind the Stroke palette, Expand Outline, the Mask flag, Filter › Shape Order, and the
//! composite of a layer's shapes that the canvas draws and export compiles.
//!
//! A path's stroke and mask are drawing attributes ([`ShapeAttrs`]) stored as the source's format
//! version states them; [`stroke_style`] and [`mask_mode`] read them, and the commands write them
//! back in the same form. The axis a forced axis-aligned cap uses has no model field and no known
//! key in the reference format: it is kept as a native extra entry ([`CAP_AXIS_KEY`]). Reading
//! the attributes and the composite live in `fontcraft_compile::drawing`, which export uses too.

use std::collections::HashMap;

use fontcraft_geom::{BezPath, Point, Vec2, round_point};
use fontcraft_hinting::Renumbered;
use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Dimension, Family, Layer, LayerSide, Node, NodeKind, Path, Shape, ShapeAttrs};
use fontcraft_pathops::{Cap, CapAxis, Join, MAX_STROKE_WIDTH, MaskMode, SMOOTH_MAX_DEGREES, contours_from_bez};
use serde_json::{Value, json};

use super::common::{Scope, each_layer, grid, scope};
use super::shared::renumber_hints;
use super::*;
use crate::EngineError;
use crate::params::{MAX_TARGETS, Params, TargetKind, Targets};

pub use fontcraft_compile::components_opt::OpenCorners;
pub use fontcraft_compile::drawing::{CAP_AXIS_KEY, cap_axis, format_version, layer_outline, mask_mode, shapes_outline, stroke_style};

/// The keys `path.setStroke` reads besides the target.
const STROKE_KEYS: [&str; 8] = ["width", "height", "fill", "position", "join", "capStart", "capEnd", "capAxis"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.setStroke",
            "Set Stroke",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", shapes: [index] (paths), width?: number | null (font units, more than 0 and at most 10000; null removes the stroke and takes no other key), height?: number | null (the pen's vertical size; null: the same as width), fill?: bool (a closed path is also filled), position?: percent (−100 wholly left of the path direction, 0 centred, 100 wholly right), join?: \"miter\" | \"round\" | \"bevel\" | \"roundFull\", capStart?: \"butt\" | \"round\" | \"square\" | \"roundInset\" | \"axisAligned\", capEnd?: same as capStart, capAxis?: \"auto\" | \"x\" | \"y\" (the axis an axis-aligned cap cuts along)} write the stroke attributes of the paths; keys not given keep their values. Width and height may differ by a factor of at most 100. A locked glyph refuses it → {glyph, layer, shapes: [index], strokes: [{shape, width, height, position, join, capStart, capEnd, capAxis, fill}]}",
            has_doc,
            set_stroke
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes]),
        cmd!(
            "path.expandOutline",
            "Expand Outline",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", shapes?: [index] (stroked paths; default: every stroked path)} or {glyphs: [names], layer?: id | layers?: [ids]} replace each stroked path by the closed outlines of its stroke, rounded to the grid; the new paths keep the path's other attributes (mask, colour label) and have no stroke. Hints and corner components on the other paths keep pointing at the same nodes; those on an expanded path are removed. With one glyph, a layer without a stroked path, a listed shape that is not a stroked path, or a locked glyph is an error; with `glyphs` such layers are skipped → {glyph, layer, paths: count, selection: {glyph, layer, shapes: [index]}} or {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            expand_outline
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes])
        .context(&["canvas.path"]),
        cmd!(
            "path.expandOutlineForAllMasters",
            "Expand Outline for All Masters",
            [],
            None,
            "{glyph: name, shapes?: [index] (default: every stroked path)} or {glyphs: [names]} Expand Outline on the master layer of every master. A master layer without a stroked path at the given shapes is skipped and listed; with one glyph it is an error when no master layer changes → {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            expand_outline_all_masters
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes])
        .context(&["canvas.path"]),
        cmd!(
            "path.setMask",
            "Mask",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", shapes: [index] (paths), mask: bool} set or clear the paths' mask flag. A mask path subtracts its outline (its stroke outline when stroked) from every shape before it in shape order, components included; it stays an ordinary editable path. A locked glyph refuses it → {glyph, layer, shapes: [index], mask: bool}",
            has_doc,
            set_mask
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes])
        .context(&["canvas.path"]),
        cmd!(
            "filter.shapeOrder",
            "Shape Order",
            ["Filter"],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", order: [index] (every shape index of the layer once, in the new order: the first is drawn first, at the bottom)} reorder the layer's shapes. Masks act on the shapes before them, so the order decides what a mask cuts. The Shape Order dialog supplies `order`. A locked glyph refuses it → {glyph, layer, order: [index], selection: {glyph, layer}}",
            has_doc,
            shape_order
        )
        .targets(&[TargetKind::Layer]),
    ]
}

fn is_stroked(path: &Path) -> bool {
    stroke_style(&path.info.attrs).is_ok_and(|s| s.is_some())
}

// ── Set Stroke ──────────────────────────────────────────────────────────────────────────────────

/// What `path.setStroke` writes; `None` leaves a key as it is.
#[derive(Default)]
struct StrokeEdit {
    remove: bool,
    width: Option<f64>,
    /// `Some(None)` clears the height.
    height: Option<Option<f64>>,
    fill: Option<bool>,
    /// −1 to 1.
    position: Option<f64>,
    join: Option<Join>,
    cap_start: Option<Cap>,
    cap_end: Option<Cap>,
    cap_axis: Option<CapAxis>,
}

fn size(p: &Params, key: &str) -> Result<Option<f64>> {
    let Some(v) = p.opt_f64(key)? else { return Ok(None) };
    if !(v > 0.0 && v <= MAX_STROKE_WIDTH) {
        return Err(p.bad(format!("`{key}` must be more than 0 and at most {MAX_STROKE_WIDTH}, got {v}")));
    }
    Ok(Some(v))
}

fn named<T>(p: &Params, key: &str, parse: fn(&str) -> Option<T>, choices: &str) -> Result<Option<T>> {
    let Some(name) = p.opt_str(key)? else { return Ok(None) };
    parse(name).map(Some).ok_or_else(|| p.bad(format!("`{key}` must be one of {choices}, got \"{name}\"")))
}

impl StrokeEdit {
    fn read(p: &Params, raw: &Value) -> Result<Self> {
        let is_null = |key: &str| raw.get(key).is_some_and(Value::is_null);
        if !STROKE_KEYS.iter().any(|k| raw.get(k).is_some()) {
            return Err(p.bad(format!("give at least one of {}", STROKE_KEYS.join(", "))));
        }
        if is_null("width") {
            if let Some(k) = STROKE_KEYS.iter().skip(1).find(|k| raw.get(**k).is_some()) {
                return Err(p.bad(format!("`width: null` removes the stroke; `{k}` cannot be given with it")));
            }
            return Ok(Self { remove: true, ..Self::default() });
        }
        let position = match p.opt_f64("position")? {
            Some(v) if v.abs() > 100.0 => return Err(p.bad(format!("`position` must be from −100 to 100 (percent), got {v}"))),
            other => other.map(|v| v / 100.0),
        };
        Ok(Self {
            remove: false,
            width: size(p, "width")?,
            height: if is_null("height") { Some(None) } else { size(p, "height")?.map(Some) },
            fill: if p.get("fill").is_some() { Some(p.bool_or("fill", false)?) } else { None },
            position,
            join: named(p, "join", Join::from_name, "miter, round, bevel, roundFull")?,
            cap_start: named(p, "capStart", Cap::from_name, "butt, round, square, roundInset, axisAligned")?,
            cap_end: named(p, "capEnd", Cap::from_name, "butt, round, square, roundInset, axisAligned")?,
            cap_axis: named(p, "capAxis", CapAxis::from_name, "auto, x, y")?,
        })
    }

    fn apply(&self, attrs: &mut ShapeAttrs) {
        if self.remove {
            clear_stroke(attrs);
            return;
        }
        if let Some(w) = self.width {
            attrs.stroke_width = Some(Dimension::Number(w));
        }
        if let Some(h) = self.height {
            attrs.stroke_height = h.map(Dimension::Number);
        }
        if let Some(f) = self.fill {
            attrs.fill = f;
        }
        if let Some(pos) = self.position {
            attrs.stroke_pos = Some(Dimension::Number(pos));
        }
        if let Some(j) = self.join {
            attrs.line_join = Some(j.code());
        }
        if let Some(c) = self.cap_start {
            attrs.line_cap_start = Some(c.code());
        }
        if let Some(c) = self.cap_end {
            attrs.line_cap_end = Some(c.code());
        }
        match self.cap_axis {
            None => {}
            Some(CapAxis::Auto) => {
                attrs.extra.remove(Family::Native, CAP_AXIS_KEY);
            }
            Some(axis) => attrs.extra.insert(Family::Native, CAP_AXIS_KEY, Plist::String(Str::from(axis.name()))),
        }
    }
}

/// Removes every stroke attribute: sizes, position, caps, join, fill and the forced cap axis.
fn clear_stroke(attrs: &mut ShapeAttrs) {
    attrs.stroke_width = None;
    attrs.stroke_height = None;
    attrs.stroke_pos = None;
    attrs.line_cap_start = None;
    attrs.line_cap_end = None;
    attrs.line_join = None;
    attrs.fill = false;
    attrs.extra.remove(Family::Native, CAP_AXIS_KEY);
}

/// The stroke attributes of a path as `path.setStroke` names them.
fn describe(index: usize, attrs: &ShapeAttrs) -> Value {
    let dim = |d: &Option<Dimension>| match d {
        None => Value::Null,
        Some(Dimension::Number(n)) => json!(n),
        Some(Dimension::Expression(e)) => json!(e),
    };
    let position = match &attrs.stroke_pos {
        None => json!(0.0),
        Some(Dimension::Number(n)) => json!(n * 100.0),
        Some(Dimension::Expression(e)) => json!(e),
    };
    let cap = |code: Option<u8>| code.and_then(Cap::from_code).unwrap_or_default().name();
    json!({
        "shape": index,
        "width": dim(&attrs.stroke_width),
        "height": dim(&attrs.stroke_height),
        "position": position,
        "join": attrs.line_join.and_then(Join::from_code).unwrap_or_default().name(),
        "capStart": cap(attrs.line_cap_start),
        "capEnd": cap(attrs.line_cap_end),
        "capAxis": cap_axis(attrs).name(),
        "fill": attrs.fill,
    })
}

/// The paths a shape-list command acts on: `shapes` must be given and name paths.
fn path_targets(p: &Params, s: &Session) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.shapes.is_empty() {
        return Err(p.bad("`shapes` is required: the paths to change"));
    }
    if !(t.points.is_empty() && t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `shapes` only"));
    }
    let r = t.resolve(s.font()?)?;
    if let Some(i) = t.shapes.iter().find(|i| r.parts.path(**i).is_none()) {
        return Err(p.bad(format!("shape {i} is not a path")));
    }
    Ok(t)
}

fn set_stroke(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("path.setStroke", value)?;
    let edit = StrokeEdit::read(&p, value)?;
    let t = path_targets(&p, s)?;
    let strokes = s.edit("Set Stroke", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::with_capacity(t.shapes.len());
            for &i in &t.shapes {
                let path = parts.path_mut(i).ok_or_else(|| p.bad(format!("shape {i} is not a path")))?;
                edit.apply(&mut path.info.attrs);
                if let Ok(Some(style)) = stroke_style(&path.info.attrs) {
                    style.check().map_err(|e| p.bad(e.to_string()))?;
                }
                path.info.tidy();
                out.push(describe(i, &path.info.attrs));
            }
            Ok(out)
        })
    })?;
    let layer = t.layer_id(s.font()?)?;
    Ok(json!({"glyph": t.glyph, "layer": layer, "shapes": t.shapes, "strokes": strokes}))
}

// ── Mask ────────────────────────────────────────────────────────────────────────────────────────

fn set_mask(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("path.setMask", value)?;
    if p.get("mask").is_none() {
        return Err(p.bad("`mask` is required: true or false"));
    }
    let mask = p.bool_or("mask", false)?;
    let t = path_targets(&p, s)?;
    s.edit(if mask { "Mask" } else { "Unmask" }, |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            for &i in &t.shapes {
                let path = parts.path_mut(i).ok_or_else(|| p.bad(format!("shape {i} is not a path")))?;
                path.info.attrs.mask = mask.then_some(MaskMode::Subtract.code());
                path.info.tidy();
            }
            Ok(())
        })
    })?;
    let layer = t.layer_id(s.font()?)?;
    Ok(json!({"glyph": t.glyph, "layer": layer, "shapes": t.shapes, "mask": mask}))
}

// ── Shape Order ─────────────────────────────────────────────────────────────────────────────────

fn shape_order(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("filter.shapeOrder", value)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes `order`, not target lists"));
    }
    let list = p.opt_array("order")?.ok_or_else(|| p.bad("`order` is required: every shape index once, in the new order"))?;
    if list.len() > MAX_TARGETS {
        return Err(p.bad(format!("`order` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
    }
    let order: Vec<usize> = list
        .iter()
        .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad(format!("`order` must hold whole numbers from 0, found {v}"))))
        .collect::<Result<_>>()?;
    let count = t.resolve(s.font()?)?.parts.shapes.len();
    let mut seen = vec![false; count];
    for &i in &order {
        match seen.get_mut(i) {
            Some(slot) if !*slot => *slot = true,
            Some(_) => return Err(p.bad(format!("`order` names shape {i} twice"))),
            None => return Err(p.bad(format!("`order`: shape {i} does not resolve: the layer has {count} shapes"))),
        }
    }
    if order.len() != count {
        return Err(p.bad(format!("`order` must name all {count} shapes of the layer, got {}", order.len())));
    }
    s.edit("Shape Order", |font| {
        t.edit_layer(font, |layer| {
            let shapes = layer.parts_mut(t.side).shapes;
            let old = std::mem::take(shapes);
            let mut slots: Vec<Option<Shape>> = old.into_iter().map(Some).collect();
            for &i in &order {
                let shape = slots.get_mut(i).and_then(Option::take).ok_or_else(|| p.bad(format!("shape {i} does not resolve")))?;
                shapes.push(shape);
            }
            Ok(())
        })
    })?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "order": order, "selection": t.to_json()}))
}

// ── Expand Outline ──────────────────────────────────────────────────────────────────────────────

/// What Expand Outline did to one side of a layer.
#[derive(Default)]
struct Expansion {
    /// The shape indices of the new paths.
    new: Vec<usize>,
    /// The new shape index of every old shape; `None` for an expanded path.
    shapes: Vec<Option<usize>>,
}

/// Replaces the stroked paths of `shapes` (every one, or those at `only`) by the outlines of their
/// strokes, rounded to `grid`. Nothing changes when it fails; with `only`, an index that is not a
/// stroked path fails.
fn expand_shapes(shapes: &mut Vec<Shape>, only: Option<&[usize]>, grid: f64) -> std::result::Result<Expansion, String> {
    if let Some(only) = only {
        for &i in only {
            match shapes.get(i) {
                Some(Shape::Path(path)) if is_stroked(path) => {}
                Some(_) => return Err(format!("shape {i} is not a stroked path")),
                None => return Err(format!("shape {i} does not resolve: the layer has {} shapes", shapes.len())),
            }
        }
    }
    let mut expanded: HashMap<usize, Vec<Path>> = HashMap::new();
    for (i, shape) in shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        if only.is_some_and(|o| !o.contains(&i)) {
            continue;
        }
        let Some(style) = stroke_style(&path.info.attrs)? else { continue };
        let outline = fontcraft_compile::thickness::stroke_path(path, &style).map_err(|e| format!("shape {i}: {e}"))?;
        expanded.insert(i, to_paths(&outline, path, grid));
    }
    if expanded.is_empty() {
        return Ok(Expansion::default());
    }
    let old = std::mem::take(shapes);
    let mut out = Expansion { new: Vec::new(), shapes: Vec::with_capacity(old.len()) };
    for (i, shape) in old.into_iter().enumerate() {
        match expanded.remove(&i) {
            Some(paths) => {
                out.shapes.push(None);
                for path in paths {
                    out.new.push(shapes.len());
                    shapes.push(Shape::Path(path));
                }
            }
            None => {
                out.shapes.push(Some(shapes.len()));
                shapes.push(shape);
            }
        }
    }
    Ok(out)
}

/// Expand Outline on one side of `layer` (of glyph `glyph`), with the side's hints kept on their
/// nodes ([`renumber_hints`]): on an expanded path a PostScript end becomes a number at its node's
/// place, and a TrueType instruction or corner component is removed and reported in `done`. A side
/// with hints that changed shapes is added to `done`, so the session's hint upkeep leaves it
/// alone. Returns the indices of the new paths.
fn expand_side(
    layer: &mut Layer,
    side: LayerSide,
    only: Option<&[usize]>,
    grid: f64,
    glyph: &str,
    done: &mut Renumbered,
) -> std::result::Result<Vec<usize>, String> {
    let has_hints = match side {
        LayerSide::Foreground => !layer.info.hints.is_empty(),
        LayerSide::Background => layer.info.background.as_ref().is_some_and(|b| !b.hints.is_empty()),
    };
    let stroked = layer.parts(side).shapes.iter().any(|sh| matches!(sh, Shape::Path(p) if is_stroked(p)));
    let before = (has_hints && stroked).then(|| layer.clone());
    let e = expand_shapes(layer.parts_mut(side).shapes, only, grid)?;
    if e.new.is_empty() {
        return Ok(e.new);
    }
    if let Some(before) = before {
        // A reference past the old shapes pointed at nothing before and is left as it is.
        let map = |i: usize| e.shapes.get(i).map_or(Some(i), |m| *m);
        renumber_hints(&before, layer, side, glyph, &map, &mut done.report);
        done.insert(glyph, &layer.id, side);
    }
    Ok(e.new)
}

fn node_kind(k: fontcraft_pathops::NodeKind) -> NodeKind {
    match k {
        fontcraft_pathops::NodeKind::Line => NodeKind::Line,
        fontcraft_pathops::NodeKind::Curve => NodeKind::Curve,
        fontcraft_pathops::NodeKind::QCurve => NodeKind::QCurve,
        fontcraft_pathops::NodeKind::OffCurve => NodeKind::OffCurve,
    }
}

/// Stroke outlines as model paths with `template`'s attributes minus the stroke. Points are
/// rounded to `grid`; a line node that lands on the node before it goes, and so does a contour
/// left with fewer than three nodes.
fn to_paths(outline: &[BezPath], template: &Path, grid: f64) -> Vec<Path> {
    let mut info = template.info.clone();
    clear_stroke(&mut info.attrs);
    info.tidy();
    let mut out = Vec::new();
    for bez in outline {
        for contour in contours_from_bez(bez) {
            let mut nodes: Vec<Node> = Vec::with_capacity(contour.nodes.len());
            for n in &contour.nodes {
                let at = round_point(n.point, grid);
                let node = Node::new(at.x, at.y, node_kind(n.kind));
                let repeats = node.kind == NodeKind::Line && nodes.last().is_some_and(|prev| prev.on_curve() && prev.point() == at);
                if !repeats {
                    nodes.push(node);
                }
            }
            if contour.closed
                && nodes.len() > 1
                && nodes.last().is_some_and(|l| l.kind == NodeKind::Line)
                && nodes.first().is_some_and(|f| f.on_curve() && Some(f.point()) == nodes.last().map(Node::point))
            {
                nodes.pop();
            }
            if nodes.len() < 3 {
                continue;
            }
            mark_smooth(&mut nodes, contour.closed);
            let mut path = Path::new(contour.closed, nodes);
            path.info = info.clone();
            out.push(path);
        }
    }
    out
}

/// Marks on-curve nodes smooth where a handle leaves them in the direction the path arrives in
/// (within the Tidy up Paths angle).
fn mark_smooth(nodes: &mut [Node], closed: bool) {
    let count = nodes.len();
    let points: Vec<(Point, bool)> = nodes.iter().map(|n| (n.point(), n.on_curve())).collect();
    let at = |i: usize| points.get(i).copied();
    for (i, node) in nodes.iter_mut().enumerate() {
        if !node.on_curve() || (!closed && (i == 0 || i + 1 == count)) {
            continue;
        }
        let (Some((prev, prev_on)), Some((next, next_on))) = (at((i + count - 1) % count), at((i + 1) % count)) else { continue };
        if prev_on && next_on {
            continue;
        }
        let here = node.point();
        let (a, b): (Vec2, Vec2) = (here - prev, next - here);
        let (la, lb) = (a.hypot(), b.hypot());
        if la <= 0.0 || lb <= 0.0 {
            continue;
        }
        let angle = a.cross(b).atan2(a.dot(b)).abs().to_degrees();
        node.smooth = angle <= SMOOTH_MAX_DEGREES;
    }
}

fn bad_for(command: &'static str) -> impl Fn(String) -> EngineError {
    move |message| EngineError::BadParams { command: command.into(), message }
}

fn expand_outline(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "path.expandOutline";
    let p = Params::new(ID, value)?;
    let font = s.font()?;
    let step = grid(font);
    if p.get("glyphs").is_some() {
        if let Some(k) = ["shapes", "side"].iter().find(|k| p.get(k).is_some()) {
            return Err(p.bad(format!("`{k}` goes with one `glyph`; with `glyphs` every stroked path is expanded")));
        }
        let sc = scope(&p, font)?;
        let (out, done) = s.edit("Expand Outline", |font| {
            let mut done = Renumbered::default();
            let out = each_layer(font, &sc, ID, |name, layer| {
                Ok(match expand_side(layer, LayerSide::Foreground, None, step, name, &mut done) {
                    Ok(v) if v.is_empty() => Some("the layer has no stroked path".into()),
                    Ok(_) => None,
                    Err(e) => Some(e),
                })
            })?;
            Ok((out, done))
        })?;
        s.hints_renumbered(done);
        return Ok(out.to_json());
    }
    let mut t = Targets::parse(&p)?;
    if !(t.points.is_empty() && t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `shapes` only"));
    }
    let only = (!t.shapes.is_empty()).then(|| t.shapes.clone());
    let bad = bad_for(ID);
    let (new, done) = s.edit("Expand Outline", |font| {
        let mut done = Renumbered::default();
        let new = t.edit_layer(font, |layer| {
            let new = expand_side(layer, t.side, only.as_deref(), step, &t.glyph, &mut done).map_err(&bad)?;
            if new.is_empty() {
                return Err(bad("the layer has no stroked path".into()));
            }
            Ok(new)
        })?;
        Ok((new, done))
    })?;
    s.hints_renumbered(done);
    t.layer = Some(t.layer_id(s.font()?)?);
    let count = new.len();
    t.shapes = new;
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "paths": count, "selection": t.to_json()}))
}

fn expand_outline_all_masters(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "path.expandOutlineForAllMasters";
    let p = Params::new(ID, value)?;
    if let Some(k) = ["layer", "layers", "side"].iter().find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("`{k}` is not taken: every master layer is expanded")));
    }
    let font = s.font()?;
    let step = grid(font);
    let mut sc: Scope = scope(&p, font)?;
    sc.layers = font.masters.iter().map(|m| m.id.clone()).collect();
    let only: Option<Vec<usize>> = match p.opt_array("shapes")? {
        None => None,
        Some(_) if !sc.single => return Err(p.bad("`shapes` goes with one `glyph`")),
        Some(_) => Some(Targets::parse(&p)?.shapes),
    };
    let (out, done) = s.edit("Expand Outline for All Masters", |font| {
        let mut done = Renumbered::default();
        let out = each_layer(font, &sc, ID, |name, layer| {
            Ok(match expand_side(layer, LayerSide::Foreground, only.as_deref(), step, name, &mut done) {
                Ok(v) if v.is_empty() => Some("the layer has no stroked path".into()),
                Ok(_) => None,
                Err(e) => Some(e),
            })
        })?;
        Ok((out, done))
    })?;
    s.hints_renumbered(done);
    if sc.single && out.glyphs.is_empty() {
        let reasons: Vec<String> = out.skipped.iter().filter_map(|v| v.get("reason").and_then(Value::as_str)).map(str::to_string).collect();
        return Err(p.bad(format!("no master layer has a stroked path to expand ({})", reasons.join("; "))));
    }
    Ok(out.to_json())
}
