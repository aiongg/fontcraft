//! Corner components (`cornerComponent.*`): attach a corner glyph to nodes or to extra nodes,
//! swap its glyph, scale or mirror it, choose its alignment, and copy corners onto other nodes
//! with Edit › Copy and Edit › Paste.
//!
//! A corner component is a `Corner` hint on the layer (`fontcraft_model::corner`); outline
//! resolution fits it into the path. Commands address corners by their position in the layer's
//! hint list (`corners: [indices]`), the numbering `corner::decompose_corners` uses, and nodes
//! in the model's order (`points: [[shape, node]]`). The hint itself counts a closed path's
//! nodes in the file's order, start node last. Corners live on the foreground only: outline
//! resolution fits the foreground's hints.
//!
//! An edit keeps every other key of the stored hint and the hint's spelling (a version 2 source
//! writes pairs as `{x, y}` strings).

use std::collections::{BTreeMap, HashSet};

use fontcraft_geom::{Point, Segment};
use fontcraft_model::corner::{self, CornerAlignment, CornerHint, CornerTarget, MAX_CORNER_SCALE};
use fontcraft_model::plist::{Array, Dict, Layout, Number, Str, Value as Plist};
use fontcraft_model::{Font, Hint, Layer, LayerSide, MAX_COORDINATE, MAX_LAYER_HINTS, MAX_NAME_LEN, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, Targets};

/// What a corner glyph's name starts with.
pub const CORNER_PREFIX: &str = "_corner.";
/// How far, in font units, a point given as an extra node may lie from each of the two segments
/// that cross there.
pub const EXTRA_NODE_TOLERANCE: f64 = 1.0;
/// Most extra nodes one call may name: each is tested against every segment of the layer.
pub const MAX_EXTRA_NODES: usize = 64;
/// The `format` value of copied corners on the clipboard.
pub const CORNERS_FORMAT: &str = "fontcraft-corners";
/// The clipboard version this build writes, and the newest it reads.
const CORNERS_VERSION: u32 = 1;
/// Largest clipboard text of copied corners, in bytes.
const MAX_CORNERS_TEXT: usize = 1 << 20;
const NO_LISTS: &str = "addresses corners with `corners` and takes no target lists";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "cornerComponent.add",
            "Add Corner Component",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), points?: [[shape, node]], extraNodes?: [[x, y]], corner: name} attaches the corner glyph `corner` (its name starts with _corner.) to each node in `points` and to each extra node in `extraNodes`, unscaled and aligned left. A node needs a segment on each side: an off-curve node or the end of an open path is refused. An extra node is where segments of the layer's paths cross: a point within 1 unit of two segments and more than 1 unit from their end nodes; at most 64. A node that already holds a corner gets the new one in its place. Foreground only; at most 4,096 hints per layer. A locked glyph refuses it → {glyph, layer, corners: [hint indices], added: count, replaced: count, warning?: why the layer's corners cannot be fitted, selection: {glyph, layer, corners}}",
            has_doc,
            add
        )
        .targets(&[TargetKind::Points]),
        cmd!(
            "cornerComponent.setGlyph",
            "Replace Corner Component",
            [],
            None,
            "{glyph: name, layer?: id, corners: [hint indices], corner: name} gives the listed corner components the corner glyph `corner`, keeping their node, scale and alignment → {glyph, layer, corners, warning?, selection: {glyph, layer, corners}}",
            has_doc,
            set_glyph
        ),
        cmd!(
            "cornerComponent.setScale",
            "Set Corner Scale",
            [],
            None,
            "{glyph: name, layer?: id, corners: [hint indices], scaleX?: percent, scaleY?: percent, flip?: \"horizontal\" | \"vertical\"} sets the horizontal and vertical scale of the listed corner components; a negative value mirrors the corner on that axis. `flip` then negates the horizontal or vertical scale of each corner, as the Palette's mirror buttons do. Give at least one; a scale is not 0 and within ±10,000 percent → {glyph, layer, corners, warning?, selection: {glyph, layer, corners}}",
            has_doc,
            set_scale
        ),
        cmd!(
            "cornerComponent.setAlignment",
            "Set Corner Alignment",
            [],
            None,
            "{glyph: name, layer?: id, corners: [hint indices], mode: \"left\" | \"right\" | \"both\" | \"none\"} sets how the listed corner components follow a diagonal stem: left turns the corner to the outgoing stroke, right to the incoming one, both to the mean of the two, none leaves it unturned → {glyph, layer, corners, warning?, selection: {glyph, layer, corners}}",
            has_doc,
            set_alignment
        ),
    ]
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// Refuses `name` unless it is a corner glyph of `font` other than `host`.
fn check_corner_glyph(p: &Params, font: &Font, name: &str, host: &str) -> Result<()> {
    if name.len() <= CORNER_PREFIX.len() || !name.starts_with(CORNER_PREFIX) || name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`{name}` is not a corner glyph: a corner glyph's name starts with {CORNER_PREFIX}")));
    }
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("there is no glyph `{name}`")));
    }
    if name == host {
        return Err(p.bad(format!("a corner of `{name}` cannot be attached to `{name}` itself")));
    }
    Ok(())
}

/// The target object, refused on the background and with target lists other than `points`
/// (and `points` too unless `points_allowed`); `why` says what the command takes instead.
fn foreground_targets(p: &Params, points_allowed: bool, why: &str) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.side != LayerSide::Foreground {
        return Err(p.bad("corner components sit on the foreground only"));
    }
    let others = !(t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty());
    if others || (!points_allowed && !t.points.is_empty()) {
        return Err(p.bad(why));
    }
    Ok(t)
}

/// `corners`: hint indices, at least one, each once.
fn corner_list(p: &Params) -> Result<Vec<usize>> {
    let list = p.array("corners")?;
    if list.is_empty() {
        return Err(p.bad("`corners` must not be empty"));
    }
    if list.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`corners` names {} corners; a layer holds at most {MAX_LAYER_HINTS} hints", list.len())));
    }
    let mut out = Vec::new();
    for v in list {
        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`corners` must hold hint indices, found {v}")))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// `extraNodes`: `[x, y]` positions within the coordinate limit.
fn extra_nodes(p: &Params) -> Result<Vec<Point>> {
    let Some(list) = p.opt_array("extraNodes")? else { return Ok(Vec::new()) };
    if list.len() > MAX_EXTRA_NODES {
        return Err(p.bad(format!("`extraNodes` names {} points; at most {MAX_EXTRA_NODES} are allowed", list.len())));
    }
    let mut out = Vec::new();
    for v in list {
        let pair = v.as_array().map(Vec::as_slice);
        let Some([x, y]) = pair else { return Err(p.bad(format!("`extraNodes` must hold [x, y] pairs, found {v}"))) };
        match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) if super::common::in_range(x) && super::common::in_range(y) => out.push(Point::new(x, y)),
            _ => return Err(p.bad(format!("`extraNodes`: {v} is not two numbers within ±{MAX_COORDINATE}"))),
        }
    }
    Ok(out)
}

/// The alignment a mode name stands for; `unused` (stored as 3) only when `stored` allows it.
fn parse_mode(p: &Params, mode: &str, stored: bool) -> Result<CornerAlignment> {
    match mode {
        "left" => Ok(CornerAlignment::Left),
        "right" => Ok(CornerAlignment::Right),
        "both" => Ok(CornerAlignment::Both),
        "none" => Ok(CornerAlignment::Unaligned),
        "unused" if stored => Ok(CornerAlignment::Unused),
        other => Err(p.bad(format!("`mode` must be left, right, both or none, got \"{other}\""))),
    }
}

fn mode_name(a: CornerAlignment) -> &'static str {
    match a {
        CornerAlignment::Left => "left",
        CornerAlignment::Right => "right",
        CornerAlignment::Both => "both",
        CornerAlignment::Unaligned => "none",
        CornerAlignment::Unused => "unused",
    }
}

/// Refuses a scale factor of 0, or one that is not a number or beyond the model's limit.
fn check_factor(p: &Params, key: &str, f: f64) -> Result<()> {
    if !f.is_finite() || f == 0.0 || f.abs() > MAX_CORNER_SCALE {
        return Err(p.bad(format!("`{key}` must not be 0 and lie within ±{} percent, got {}", MAX_CORNER_SCALE * 100.0, f * 100.0)));
    }
    Ok(())
}

// ── Where a corner sits ─────────────────────────────────────────────────────────────────────────

/// A node index in the hint's numbering: on a closed path the model's node `m` is the file's
/// node `m - 1`, the start node last.
fn file_index(closed: bool, len: usize, node: usize) -> usize {
    if !closed {
        return node;
    }
    node.checked_add(len.saturating_sub(1)).and_then(|v| v.checked_rem(len)).unwrap_or(node)
}

/// The segments of each path of the layer that the call needs, built once.
#[derive(Default)]
struct PathSegments {
    by_shape: BTreeMap<usize, Vec<Segment>>,
}

impl PathSegments {
    fn of(&mut self, layer: &Layer, shape: usize) -> &[Segment] {
        self.by_shape.entry(shape).or_insert_with(|| match layer.shapes.get(shape) {
            Some(Shape::Path(path)) => path.segments(),
            _ => Vec::new(),
        })
    }
}

/// The target of a corner on node `node` (model order) of path `shape`.
fn node_target(p: &Params, layer: &Layer, segments: &mut PathSegments, shape: usize, node: usize) -> Result<CornerTarget> {
    let what = format!("`points`: [{shape}, {node}]");
    let path = match layer.shapes.get(shape) {
        Some(Shape::Path(path)) => path,
        Some(_) => return Err(p.bad(format!("{what}: shape {shape} is not a path"))),
        None => return Err(p.bad(format!("{what} does not resolve: the layer has {} shapes", layer.shapes.len()))),
    };
    match path.nodes.get(node) {
        None => return Err(p.bad(format!("{what} does not resolve: path {shape} has {} nodes", path.nodes.len()))),
        Some(n) if !n.on_curve() => return Err(p.bad(format!("{what} is an off-curve node; a corner sits on an on-curve node"))),
        Some(_) => {}
    }
    let (closed, len) = (path.closed, path.nodes.len());
    let segs = segments.of(layer, shape);
    if !(segs.iter().any(|s| s.end == node) && segs.iter().any(|s| s.start == node)) {
        return Err(p.bad(format!("{what} ends an open path; a corner needs a segment on each side of its node")));
    }
    Ok(CornerTarget::Node { shape, node: file_index(closed, len, node) })
}

/// The target of a corner on the extra node at `at`: the two segments that cross there, the
/// earlier path (then the earlier node) first.
fn extra_target(p: &Params, layer: &Layer, segments: &mut PathSegments, at: Point) -> Result<CornerTarget> {
    let mut near: Vec<(f64, usize, usize)> = Vec::new();
    for (shape, s) in layer.shapes.iter().enumerate() {
        let Shape::Path(path) = s else { continue };
        let (closed, len) = (path.closed, path.nodes.len());
        for seg in segments.of(layer, shape) {
            let Some(b) = seg.bounds() else { continue };
            if !b.inflate(EXTRA_NODE_TOLERANCE, EXTRA_NODE_TOLERANCE).contains(at) {
                continue;
            }
            let Some(n) = seg.nearest(at) else { continue };
            let off_ends = [0.0, 1.0].iter().all(|&t| seg.eval(t).is_some_and(|e| e.distance(at) > EXTRA_NODE_TOLERANCE));
            if n.distance <= EXTRA_NODE_TOLERANCE && off_ends {
                near.push((n.distance, shape, file_index(closed, len, seg.end)));
            }
        }
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut pair: Vec<(usize, usize)> = near.iter().take(2).map(|&(_, shape, node)| (shape, node)).collect();
    pair.sort_unstable();
    match pair.as_slice() {
        &[(shape, node), (other_shape, other_node)] => Ok(CornerTarget::Intersection { shape, node, other_shape, other_node }),
        _ => Err(p.bad(format!("`extraNodes`: no two segments of the layer cross at ({}, {})", at.x, at.y))),
    }
}

/// Puts `corner` on its target: in place of a corner already there, else after the layer's
/// hints. `targets` mirrors the hints' corner targets. Returns the hint index and whether a
/// corner was replaced.
fn put(p: &Params, hints: &mut Vec<Hint>, targets: &mut Vec<Option<CornerTarget>>, corner: &CornerHint) -> Result<(usize, bool)> {
    if let Some(i) = targets.iter().position(|t| *t == Some(corner.target))
        && let Some(slot) = hints.get_mut(i)
    {
        *slot = corner.to_hint();
        return Ok((i, true));
    }
    if hints.len() >= MAX_LAYER_HINTS {
        return Err(p.bad(format!("the layer already holds {MAX_LAYER_HINTS} hints, the most allowed")));
    }
    hints.push(corner.to_hint());
    targets.push(Some(corner.target));
    Ok((hints.len() - 1, false))
}

/// The corner target of each hint, `None` for other hints and for corners that cannot be read.
fn corner_targets(hints: &[Hint]) -> Vec<Option<CornerTarget>> {
    hints.iter().map(|h| corner::corner_hint(h).ok().flatten().map(|c| c.target)).collect()
}

/// Why the corners of the layer cannot be fitted, if they cannot.
fn fit_warning(font: &Font, glyph: &str, layer: &str) -> Option<String> {
    let l = font.glyph(glyph)?.layers.iter().find(|l| l.id == layer)?;
    corner::insert_corners(font, l).err().map(|e| e.to_string())
}

/// The selection a command returns: the layer and the corners.
fn selection(p: &Params, t: &Targets, layer: &str, corners: &[usize]) -> Value {
    let mut v = Targets::new(p.command(), t.glyph.clone(), Some(layer.to_string())).to_json();
    if let Some(o) = v.as_object_mut() {
        o.insert("corners".into(), json!(corners));
    }
    v
}

/// The common part of a result: glyph, layer, corners, warning and selection.
fn result(s: &Session, p: &Params, t: &Targets, layer: &str, corners: &[usize]) -> Result<Value> {
    let mut out = json!({ "glyph": t.glyph, "layer": layer, "corners": corners, "selection": selection(p, t, layer, corners) });
    if let Some(w) = fit_warning(s.font()?, &t.glyph, layer)
        && let Some(o) = out.as_object_mut()
    {
        o.insert("warning".into(), json!(w));
    }
    Ok(out)
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn add(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cornerComponent.add", v)?;
    let name = p.str("corner")?.to_string();
    let mut t = foreground_targets(&p, true, "takes nodes as `points` and no other target list")?;
    let extra = extra_nodes(&p)?;
    let mut points = Vec::new();
    for &pt in &t.points {
        if !points.contains(&pt) {
            points.push(pt);
        }
    }
    if points.is_empty() && extra.is_empty() {
        return Err(p.bad("give the nodes as `points` or the extra nodes as `extraNodes`"));
    }
    if points.len() + extra.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("a layer holds at most {MAX_LAYER_HINTS} hints")));
    }
    check_corner_glyph(&p, s.font()?, &name, &t.glyph)?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    let (corners, added, replaced) = s.edit("Add Corner Component", |font| {
        t.edit_layer(font, |l| {
            let mut segments = PathSegments::default();
            let mut new = Vec::new();
            for &(shape, node) in &points {
                new.push(node_target(&p, l, &mut segments, shape, node)?);
            }
            for &at in &extra {
                new.push(extra_target(&p, l, &mut segments, at)?);
            }
            let mut targets = corner_targets(&l.info.hints);
            let (mut corners, mut added, mut replaced) = (Vec::new(), 0usize, 0usize);
            for target in new {
                let corner = CornerHint { target, ..CornerHint::new(name.clone(), 0, 0) };
                let (i, was) = put(&p, &mut l.info.hints, &mut targets, &corner)?;
                if was {
                    replaced += 1
                } else {
                    added += 1
                }
                if !corners.contains(&i) {
                    corners.push(i);
                }
            }
            Ok((corners, added, replaced))
        })
    })?;
    let mut out = result(s, &p, &t, &layer, &corners)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("added".into(), json!(added));
        o.insert("replaced".into(), json!(replaced));
    }
    Ok(out)
}

/// Runs `f` on each listed corner's stored dictionary, as one undo step named `label`.
fn edit_corners(s: &mut Session, p: &Params, label: &str, f: impl Fn(&CornerHint, &mut Dict) -> Result<()>) -> Result<Value> {
    let mut t = foreground_targets(p, false, NO_LISTS)?;
    let corners = corner_list(p)?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    s.edit(label, |font| {
        t.edit_layer(font, |l| {
            let count = l.info.hints.len();
            for &i in &corners {
                let hint = l.info.hints.get_mut(i).ok_or_else(|| p.bad(format!("`corners`: {i} does not resolve: the layer has {count} hints")))?;
                let corner = corner::corner_hint(hint)
                    .map_err(|m| p.bad(format!("`corners`: hint {i}: {m}")))?
                    .ok_or_else(|| p.bad(format!("`corners`: hint {i} is not a corner component")))?;
                let Plist::Dict(d) = &mut hint.value else { return Err(p.bad(format!("`corners`: hint {i} is not a corner component"))) };
                f(&corner, d)?;
            }
            Ok(())
        })
    })?;
    result(s, p, &t, &layer, &corners)
}

fn set_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cornerComponent.setGlyph", v)?;
    let name = p.str("corner")?;
    let host = p.str("glyph")?;
    check_corner_glyph(&p, s.font()?, name, host)?;
    edit_corners(s, &p, "Replace Corner Component", |_, d| {
        set_entry(d, "name", Some(Plist::String(Str::new(name))));
        Ok(())
    })
}

fn set_scale(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cornerComponent.setScale", v)?;
    let x = p.opt_f64("scaleX")?.map(|v| v / 100.0);
    let y = p.opt_f64("scaleY")?.map(|v| v / 100.0);
    let flip = match p.opt_str("flip")? {
        None => None,
        Some("horizontal") => Some(true),
        Some("vertical") => Some(false),
        Some(other) => return Err(p.bad(format!("`flip` must be \"horizontal\" or \"vertical\", got \"{other}\""))),
    };
    if x.is_none() && y.is_none() && flip.is_none() {
        return Err(p.bad("give `scaleX`, `scaleY` or `flip`"));
    }
    if let Some(x) = x {
        check_factor(&p, "scaleX", x)?;
    }
    if let Some(y) = y {
        check_factor(&p, "scaleY", y)?;
    }
    edit_corners(s, &p, "Set Corner Scale", |c, d| {
        let (mut sx, mut sy) = (x.unwrap_or(c.scale.x), y.unwrap_or(c.scale.y));
        match flip {
            Some(true) => sx = -sx,
            Some(false) => sy = -sy,
            None => {}
        }
        check_factor(&p, "scaleX", sx)?;
        check_factor(&p, "scaleY", sy)?;
        let value = (sx != 1.0 || sy != 1.0).then(|| pair(d, sx, sy));
        set_entry(d, "scale", value);
        Ok(())
    })
}

fn set_alignment(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cornerComponent.setAlignment", v)?;
    let mode = parse_mode(&p, p.str("mode")?, false)?;
    // Left is the default and is written without `options`, as the reference writes it.
    let value = (mode != CornerAlignment::Left).then(|| Plist::Number(Number::int(mode.option())));
    edit_corners(s, &p, "Set Corner Alignment", |_, d| {
        set_entry(d, "options", value.clone());
        Ok(())
    })
}

// ── The stored dictionary ───────────────────────────────────────────────────────────────────────

/// Sets `key` to `value`, or removes it for `None`. A new key goes where alphabetical order puts
/// it, as the reference's files order a hint's keys.
fn set_entry(d: &mut Dict, key: &str, value: Option<Plist>) {
    let at = d.entries.iter().position(|(k, _)| k.as_str() == key);
    match (at, value) {
        (Some(_), Some(v)) => d.insert(key, v),
        (Some(i), None) => {
            d.remove_at(i);
        }
        (None, Some(v)) => {
            let i = d.entries.iter().position(|(k, _)| k.as_str() > key).unwrap_or(d.entries.len());
            d.insert_at(i, key, v);
        }
        (None, None) => {}
    }
}

/// A number pair in the hint's spelling: a `{x, y}` string when its origin is one (version 2),
/// else a list.
fn pair(d: &Dict, x: f64, y: f64) -> Plist {
    if matches!(d.get("origin"), Some(Plist::String(_))) {
        return Plist::String(Str::new(format!("{{{x}, {y}}}")));
    }
    Plist::Array(Array { items: vec![number(x), number(y)], layout: Layout::Compact, ..Default::default() })
}

/// A whole number as an integer, else as a float.
fn number(v: f64) -> Plist {
    if v.fract() == 0.0 && v.abs() <= MAX_CORNER_SCALE {
        return Plist::Number(Number::int(v as i64));
    }
    Number::float(v).map_or(Plist::Number(Number::int(1)), Plist::Number)
}

// ── Copy and paste ──────────────────────────────────────────────────────────────────────────────

/// Copied corners as clipboard text.
#[derive(Serialize, Deserialize)]
struct Clip {
    format: String,
    version: u32,
    corners: Vec<ClipCorner>,
}

/// One copied corner: what it draws, without the node it sat on.
#[derive(Serialize, Deserialize)]
struct ClipCorner {
    name: String,
    /// Horizontal and vertical scale as factors.
    scale: [f64; 2],
    alignment: String,
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
}

/// The copied-corners text in `json` or `text`, if either holds some.
fn corners_text<'a>(p: &Params<'a>) -> Result<Option<&'a str>> {
    for key in ["json", "text"] {
        if let Some(text) = p.opt_str(key)?
            && text.trim_start().starts_with('{')
            && text.contains(CORNERS_FORMAT)
            && serde_json::from_str::<Header>(text).is_ok_and(|h| h.format.as_deref() == Some(CORNERS_FORMAT))
        {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

/// The corners of clipboard text, checked: pasted text is untrusted.
fn decode(p: &Params, text: &str) -> Result<Vec<CornerHint>> {
    if text.len() > MAX_CORNERS_TEXT {
        return Err(p.bad(format!("the copied corners are {} bytes; the limit is {MAX_CORNERS_TEXT}", text.len())));
    }
    let clip: Clip = serde_json::from_str(text).map_err(|e| p.bad(format!("the copied corners cannot be read: {e}")))?;
    if clip.version > CORNERS_VERSION {
        return Err(p.bad(format!("the copied corners have version {}; this version reads up to {CORNERS_VERSION}", clip.version)));
    }
    if clip.corners.is_empty() || clip.corners.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("the clipboard must hold 1 to {MAX_LAYER_HINTS} corners, it holds {}", clip.corners.len())));
    }
    let mut out = Vec::new();
    for c in clip.corners {
        let [x, y] = c.scale;
        check_factor(p, "scale", x)?;
        check_factor(p, "scale", y)?;
        let mut corner = CornerHint::new(c.name, 0, 0);
        corner.scale = fontcraft_geom::Vec2::new(x, y);
        corner.alignment = parse_mode(p, &c.alignment, true)?;
        out.push(corner);
    }
    Ok(out)
}

/// `edit.copy` with `corners`: the listed corner components as clipboard text, without their
/// nodes.
pub(crate) fn copy_corners(s: &Session, p: &Params) -> Result<Value> {
    let t = foreground_targets(p, false, NO_LISTS)?;
    let corners = corner_list(p)?;
    let r = t.resolve(s.font()?)?;
    let hints = &r.layer.info.hints;
    let mut clip = Clip { format: CORNERS_FORMAT.into(), version: CORNERS_VERSION, corners: Vec::new() };
    for &i in &corners {
        let hint = hints.get(i).ok_or_else(|| p.bad(format!("`corners`: {i} does not resolve: the layer has {} hints", hints.len())))?;
        let c = corner::corner_hint(hint)
            .map_err(|m| p.bad(format!("`corners`: hint {i}: {m}")))?
            .ok_or_else(|| p.bad(format!("`corners`: hint {i} is not a corner component")))?;
        clip.corners.push(ClipCorner { name: c.name, scale: [c.scale.x, c.scale.y], alignment: mode_name(c.alignment).into() });
    }
    let text = serde_json::to_string(&clip).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({ "json": text, "corners": clip.corners.len() }))
}

/// `edit.paste` of copied corners onto the nodes in `points`; `None` when the clipboard text
/// holds no corners. One copied corner goes onto every node; several go onto as many nodes, in
/// order. Any other selection is refused.
pub(crate) fn paste_onto_nodes(s: &mut Session, p: &Params) -> Result<Option<Value>> {
    let Some(text) = corners_text(p)? else { return Ok(None) };
    let copied = decode(p, text)?;
    let mut t =
        foreground_targets(p, true, "the clipboard holds corner components: they paste onto nodes, given as `points`, and onto nothing else")?;
    let mut points = Vec::new();
    for &pt in &t.points {
        if !points.contains(&pt) {
            points.push(pt);
        }
    }
    if points.is_empty() {
        return Err(p.bad("the clipboard holds corner components: select the nodes to paste them onto (`points`)"));
    }
    if copied.len() > 1 && copied.len() != points.len() {
        return Err(p.bad(format!("the clipboard holds {} corner components; select one node for each, or copy one corner", copied.len())));
    }
    let font = s.font()?;
    let mut seen = HashSet::new();
    for c in &copied {
        if seen.insert(c.name.as_str()) {
            check_corner_glyph(p, font, &c.name, &t.glyph)?;
        }
    }
    let layer = t.layer_id(font)?;
    t.layer = Some(layer.clone());
    let corners = s.edit("Paste", |font| {
        t.edit_layer(font, |l| {
            let mut segments = PathSegments::default();
            let mut targets = corner_targets(&l.info.hints);
            let mut corners = Vec::new();
            for (k, &(shape, node)) in points.iter().enumerate() {
                let target = node_target(p, l, &mut segments, shape, node)?;
                let Some(source) = copied.get(k).or(copied.first()) else { continue };
                let corner = CornerHint { target, ..source.clone() };
                let (i, _) = put(p, &mut l.info.hints, &mut targets, &corner)?;
                if !corners.contains(&i) {
                    corners.push(i);
                }
            }
            Ok(corners)
        })
    })?;
    let mut out = result(s, p, &t, &layer, &corners)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("pasted".into(), json!(points.len()));
        o.insert("flavour".into(), json!("corners"));
    }
    Ok(Some(out))
}
