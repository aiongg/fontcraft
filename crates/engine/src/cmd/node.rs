//! Node and segment editing (`docs/spec/03a` § 7 to § 16): moving, node types, converting,
//! deleting, inserting, connecting, closing, start nodes, path direction, locks, and the headless
//! rectangle and ellipse.
//!
//! Every command names its targets (decision 0004), refuses a locked glyph, rounds the points it
//! creates or moves to the font's grid (points it does not touch keep their coordinates), and
//! returns the `selection` afterwards. The "in All Masters" forms apply the same addresses to every
//! master layer of the glyph; a layer whose shapes or node types differ from the edited layer's
//! is left alone and listed in `skipped`.

mod moving;
mod ops;
mod refit;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fontcraft_geom::{Point, Rect, Shape as _, Vec2, round_point};
use fontcraft_model::{Font, Layer, LayerParts, LayerPartsMut, LayerSide, MAX_COORDINATE, Node, NodeKind, Path, Shape};
use serde_json::{Map, Value, json};

use self::moving::Mode;
use super::common::grid;
use super::*;
use crate::params::{GuideScope, Params, ResolvedTargets, TargetKind as K, Targets};

pub fn specs() -> Vec<CommandSpec> {
    const ALL: &[K] = &[K::Layer, K::Points, K::Segments, K::Shapes, K::Anchors, K::Guides, K::Annotations];
    vec![
        cmd!(
            "selection.move",
            "Move",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index], dx: number, dy: number, mode?: \"normal\" | \"pointsOnly\" | \"nudge\" (default normal), duplicate?: bool} moves what the targets name by (dx, dy) font units, within ±2,000,000. normal: the handles of moved nodes go with them, and the opposite handle of a smooth node turns to stay in line with a moved handle. pointsOnly: only the named points move, and a handle alone keeps its angle. nudge: the handles of each segment with one moved end are scaled between its ends. A segment moves its two end nodes. duplicate: the original stays and a copy moves (part of a path is copied as open paths of its selected segments). Moved points are rounded to the grid → {selection}",
            has_doc,
            selection_move
        )
        .targets(ALL),
        cmd!(
            "selection.setPosition",
            "Set Selection Position",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?, shapes?: [index], anchors?: [name], guides?: [{scope, index}], annotations?: [index], x?: number, y?: number, referencePoint?: \"bottomLeft\" | \"bottom\" | \"bottomRight\" | \"left\" | \"center\" | \"right\" | \"topLeft\" | \"top\" | \"topRight\" (default bottomLeft)} moves the selection so that the reference point of its bounding box lands on x and/or y (a single point: its coordinates). Whole paths count with their outline's bounds, components with their transformed outline → {dx, dy, selection}",
            has_doc,
            set_position
        )
        .targets(ALL),
        cmd!(
            query "selection.getInfo",
            "Get Selection Info",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?, shapes?: [index], anchors?: [name], guides?: [{scope, index}], annotations?: [index], referencePoint?: as selection.setPosition} what the Info box shows for a selection → {glyph, layer, referencePoint, x?, y?, width?, height?, node?: {x, y} (one node), handle?: {x, y, dx, dy, length, node: [shape, node]} (one handle, measured from its node), distance?: {dx, dy, length} (two nodes)}",
            has_doc,
            get_info
        )
        .targets(ALL),
        cmd!(
            "path.toggleNodeType",
            "Toggle Node Type",
            [],
            Some("Return"),
            "{glyph: name, layer?: id, side?, points: [[shape, node]], to?: \"smooth\" | \"corner\" (default: the other type, per node), keepOutline?: bool} switches the on-curve nodes among the points between smooth and corner. A node turning smooth gets handles on a straight side (at a third of the segment) and its two handles turned onto the bisector of their directions, keeping their lengths; keepOutline changes only the type → {selection}",
            has_doc,
            toggle_node_type
        )
        .targets(&[K::Layer, K::Points])
        .when("editView")
        .context(&["canvas.node"]),
        cmd!(
            "path.toggleNodeTypeKeepingOutline",
            "Toggle Node Type Keeping Outline",
            [],
            Some("Alt+Return"),
            "{glyph: name, layer?: id, side?, points: [[shape, node]], to?: \"smooth\" | \"corner\"} path.toggleNodeType that changes only the node types: no point moves → {selection}",
            has_doc,
            toggle_keeping_outline
        )
        .targets(&[K::Layer, K::Points])
        .when("editView"),
        cmd!(
            "path.convertSegmentToCurve",
            "Convert Segment to Curve",
            [],
            None,
            "{glyph: name, layer?: id, side?, segments: [[shape, node]] (or segment: [shape, node])} turns each straight segment into a curve with handles at a third and two thirds of it (one handle at the middle in a path with quadratic segments); the outline keeps its shape. Curve segments are left alone → {selection}",
            has_doc,
            convert_segment_to_curve
        )
        .targets(&[K::Layer, K::Segments])
        .context(&["canvas.segment"]),
        cmd!(
            "edit.delete",
            "Delete",
            [],
            Some("Delete"),
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope, index}], annotations?: [index]} removes what the targets name. A deleted node leaves the path closed: the segments it joined become one, refitted to the old curve; a curve segment that loses a handle becomes a line; a path with every point named, or fewer than two nodes left, is removed → {selection}",
            has_doc,
            delete
        )
        .targets(&[K::Layer, K::Points, K::Shapes, K::Anchors, K::Guides, K::Annotations])
        .when("editView"),
        cmd!(
            "edit.deleteInAllMasters",
            "Delete in All Masters",
            [],
            Some("Alt+Delete"),
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points?: [[shape, node]], shapes?: [index]} edit.delete in that layer and in every master layer of the glyph, by the same addresses; a layer whose shapes or nodes differ is left alone → {selection, skipped: [{layer, reason}]}",
            has_doc,
            delete_in_all_masters
        )
        .targets(&[K::Layer, K::Points, K::Shapes])
        .when("editView"),
        cmd!(
            "path.deleteAndBreak",
            "Delete and Break",
            [],
            Some("Shift+Delete"),
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]]} removes each named node with the segments on both sides of it, leaving the path open there. Two neighbouring named nodes, or a named segment, remove only the segment between them. What is left becomes open paths → {selection}",
            has_doc,
            delete_and_break
        )
        .targets(&[K::Layer, K::Points, K::Segments])
        .when("editView"),
        cmd!(
            "path.deleteAndBreakInAllMasters",
            "Delete and Break in All Masters",
            [],
            Some("Alt+Shift+Delete"),
            "{glyph: name, layer?: id (whose addresses are given), side?, points?: [[shape, node]], segments?: [[shape, node]]} path.deleteAndBreak in that layer and in every master layer, by the same addresses; a layer whose shapes or nodes differ is left alone → {selection, skipped: [{layer, reason}]}",
            has_doc,
            delete_and_break_in_all_masters
        )
        .targets(&[K::Layer, K::Points, K::Segments])
        .when("editView"),
        cmd!(
            "path.insertNode",
            "Insert Node",
            [],
            None,
            "{glyph: name, layer?: id, side?, segment: [shape, node], t?: number strictly between 0 and 1 (default 0.5), near?: [x, y] (the segment's point nearest to it), snap?: \"extremum\" | \"inflection\" (the one nearest to near, else to t)} splits the segment without changing its shape (up to grid rounding) → {node: [shape, node], selection}",
            has_doc,
            insert_node
        )
        .targets(&[K::Layer, K::Segments])
        .context(&["canvas.segment"]),
        cmd!(
            "path.connectNodes",
            "Connect Nodes",
            [],
            None,
            "{glyph: name, layer?: id, side?, points: [[shape, node], [shape, node]]} joins two open ends: of one path, which closes it, or of two paths, which become one at the lower index. Ends within half a grid step of each other become one node; otherwise a straight segment joins them → {selection}",
            has_doc,
            connect_nodes
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.closeOpenPaths",
            "Close Open Paths",
            [],
            None,
            "{glyph: name, layer?: id, side?, shapes?: [index], points?: [[shape, node]] (their paths)} closes each named open path from its last node to its first (ends that coincide become one node) → {selection}",
            has_doc,
            close_open_paths
        )
        .targets(&[K::Layer, K::Shapes, K::Points])
        .context(&["canvas.node", "canvas.path"]),
        cmd!(
            "path.makeNodeFirst",
            "Make Node First",
            [],
            None,
            "{glyph: name, layer?: id, side?, point: [shape, node]} makes the on-curve node the start node of its closed path; the outline does not change → {selection}",
            has_doc,
            make_node_first
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.reverseContours",
            "Reverse Contours",
            ["Path"],
            Some("Cmd+Ctrl+Alt+R"),
            "{glyphs: [names] | glyph: name, layer?: id (default the first master's), side?, shapes?: [index] (with one glyph; default every path)} reverses the direction of the paths; a closed path keeps its start node → {reversed: count, selection?}",
            has_doc,
            reverse_contours
        )
        .targets(&[K::Glyphs, K::Layer, K::Shapes])
        .context(&["canvas", "canvas.path"]),
        cmd!(
            "path.reverseContoursForAllMasters",
            "Reverse Contours for All Masters",
            ["Path"],
            None,
            "{glyphs: [names] | glyph: name, side?, shapes?: [index] (with one glyph; default every path)} path.reverseContours on every master layer; a layer where a named shape is not a path is left alone → {reversed: count, skipped: [{glyph, layer, reason}]}",
            has_doc,
            reverse_contours_for_all_masters
        )
        .targets(&[K::Glyphs, K::Shapes]),
        cmd!(
            "path.lockPath",
            "Lock Path",
            [],
            None,
            "{glyph: name, layer?: id, side?, shapes?: [index], points?: [[shape, node]] (their paths)} locks the paths and components: the pointer tools leave them alone; commands still change them → {selection}",
            has_doc,
            lock_path
        )
        .targets(&[K::Layer, K::Shapes, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.unlockPath",
            "Unlock Path",
            [],
            None,
            "{glyph: name, layer?: id, side?, shapes?: [index], points?: [[shape, node]] (their paths)} unlocks the paths and components → {selection}",
            has_doc,
            unlock_path
        )
        .targets(&[K::Layer, K::Shapes, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.addRectangle",
            "Add Rectangle",
            [],
            None,
            "{glyph: name, layer?: id, side?, x: number, y: number, width: number, height: number (not 0), fromCenter?: bool} appends a closed rectangle with its corner (or with fromCenter its centre) at (x, y), counter-clockwise from the bottom left, corners rounded to the grid. A locked glyph refuses it → {shape: index, selection}",
            has_doc,
            add_rectangle
        )
        .targets(&[K::Layer]),
        cmd!(
            "path.addEllipse",
            "Add Ellipse",
            [],
            None,
            "{glyph: name, layer?: id, side?, x: number, y: number, width: number, height: number (not 0), fromCenter?: bool} appends a closed ellipse in the box from (x, y) (or centred there), four smooth nodes at its extremes, counter-clockwise from the bottom, points rounded to the grid → {shape: index, selection}",
            has_doc,
            add_ellipse
        )
        .targets(&[K::Layer]),
    ]
}

// ── Helpers ─────────────────────────────────────────────────────────────────────────────────────

/// The addresses grouped by shape.
pub(super) fn group(addresses: &[(usize, usize)]) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut out: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &(s, n) in addresses {
        out.entry(s).or_default().insert(n);
    }
    out
}

/// An offset: a finite number within twice the coordinate limit.
fn offset(p: &Params, key: &str) -> Result<f64> {
    let v = p.f64(key)?;
    if v.abs() > 2.0 * MAX_COORDINATE {
        return Err(p.bad(format!("`{key}` must be within ±{}", 2.0 * MAX_COORDINATE)));
    }
    Ok(v)
}

/// A coordinate within the coordinate limit.
fn coordinate(p: &Params, key: &str) -> Result<Option<f64>> {
    let v = p.opt_f64(key)?;
    if v.is_some_and(|v| v.abs() > MAX_COORDINATE) {
        return Err(p.bad(format!("`{key}` must be within ±{MAX_COORDINATE}")));
    }
    Ok(v)
}

/// `[a, b]` of whole numbers from 0.
fn pair_param(p: &Params, key: &str) -> Result<Option<(usize, usize)>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let whole = |v: &Value| v.as_u64().and_then(|n| usize::try_from(n).ok());
    if let Value::Array(a) = v
        && let [s, n] = a.as_slice()
        && let (Some(s), Some(n)) = (whole(s), whole(n))
    {
        return Ok(Some((s, n)));
    }
    Err(p.bad(format!("`{key}` must be [shape, node], whole numbers from 0")))
}

/// `[x, y]` within the coordinate limit.
fn point_param(p: &Params, key: &str) -> Result<Option<Point>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let num = |v: &Value| v.as_f64().filter(|n| n.is_finite() && n.abs() <= MAX_COORDINATE);
    if let Value::Array(a) = v
        && let [x, y] = a.as_slice()
        && let (Some(x), Some(y)) = (num(x), num(y))
    {
        return Ok(Some(Point::new(x, y)));
    }
    Err(p.bad(format!("`{key}` must be [x, y], numbers within ±{MAX_COORDINATE}")))
}

/// An empty selection in the layer `t` names (after a deletion).
fn none_selected(t: &Targets) -> Targets {
    let mut out = Targets::new("selection", t.glyph.clone(), t.layer.clone());
    out.side = t.side;
    out
}

/// Removes the entries at `indices` from `v`.
fn remove_at<T>(v: &mut Vec<T>, indices: &BTreeSet<usize>) {
    let mut k = 0;
    v.retain(|_| {
        let keep = !indices.contains(&k);
        k += 1;
        keep
    });
}

fn is_open_end(path: &Path, n: usize) -> bool {
    !path.closed && (n == 0 || n + 1 == path.nodes.len()) && ops::on(path, n)
}

/// What a layer's shapes are, for matching layers in the "in All Masters" forms: each path's
/// closedness and node types, each component's base glyph.
#[derive(PartialEq)]
enum Sig {
    Path(bool, Vec<NodeKind>),
    Component(String),
    Other,
}

fn signature(parts: LayerParts) -> Vec<Sig> {
    parts
        .shapes
        .iter()
        .map(|s| match s {
            Shape::Path(p) => Sig::Path(p.closed, p.nodes.iter().map(|n| n.kind).collect()),
            Shape::Component(c) => Sig::Component(c.glyph.clone()),
            Shape::Image(_) | Shape::Raw(_) => Sig::Other,
        })
        .collect()
}

/// Runs `f` on the addressed side of the layer `t` names and of every master layer of its glyph
/// that has the same shapes. Returns the layers left alone.
fn in_all_masters(font: &mut Font, t: &Targets, mut f: impl FnMut(&mut LayerPartsMut) -> Result<()>) -> Result<Vec<Value>> {
    let (source, sig) = {
        let r = t.resolve(font)?;
        crate::check_unlocked(r.glyph)?;
        (r.layer.id.clone(), signature(r.parts))
    };
    let mut g = font.glyph_mut(&t.glyph)?;
    let mut skipped = Vec::new();
    for layer in g.layers.iter_mut() {
        if layer.id != source && layer.id != layer.master {
            continue;
        }
        if layer.parts(t.side).shapes.is_empty() {
            continue;
        }
        if signature(layer.parts(t.side)) != sig {
            skipped.push(json!({"layer": layer.id, "reason": "its shapes or node types differ from the edited layer's"}));
            continue;
        }
        let mut parts = layer.parts_mut(t.side);
        f(&mut parts)?;
    }
    Ok(skipped)
}

// ── Moving ──────────────────────────────────────────────────────────────────────────────────────

fn selection_move(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("selection.move", v)?;
    let t = Targets::parse(&p)?;
    let d = Vec2::new(offset(&p, "dx")?, offset(&p, "dy")?);
    let mode = match p.opt_str("mode")?.unwrap_or("normal") {
        "normal" => Mode::Normal,
        "pointsOnly" => Mode::PointsOnly,
        "nudge" => Mode::Nudge,
        other => return Err(p.bad(format!("`mode` must be normal, pointsOnly or nudge, got \"{other}\""))),
    };
    let duplicate = p.bool_or("duplicate", false)?;
    if !t.has_lists() {
        return Err(p.bad("give what to move: `points`, `segments`, `shapes`, `anchors`, `guides` or `annotations`"));
    }
    let step = grid(s.font()?);
    let sel = s.edit(if duplicate { "Duplicate" } else { "Move" }, |font| moving::apply(font, &t, d, mode, duplicate, step))?;
    Ok(json!({ "selection": sel.to_json() }))
}

/// The nine reference points of a bounding box, by name: (name, x side, y side), -1 for the
/// minimum, 0 for the centre, 1 for the maximum.
const REFERENCE_POINTS: [(&str, i8, i8); 9] = [
    ("bottomLeft", -1, -1),
    ("bottom", 0, -1),
    ("bottomRight", 1, -1),
    ("left", -1, 0),
    ("center", 0, 0),
    ("right", 1, 0),
    ("topLeft", -1, 1),
    ("top", 0, 1),
    ("topRight", 1, 1),
];

#[derive(Clone, Copy)]
struct RefPoint {
    name: &'static str,
    h: i8,
    v: i8,
}

impl RefPoint {
    fn parse(p: &Params) -> Result<Self> {
        let name = p.opt_str("referencePoint")?.unwrap_or("bottomLeft");
        REFERENCE_POINTS
            .iter()
            .find(|r| r.0 == name)
            .map(|&(name, h, v)| RefPoint { name, h, v })
            .ok_or_else(|| p.bad(format!("`referencePoint` must be one of {}, got \"{name}\"", REFERENCE_POINTS.map(|r| r.0).join(", "))))
    }

    fn of(self, r: Rect) -> Point {
        let pick = |side: i8, lo: f64, hi: f64| match side {
            -1 => lo,
            0 => (lo + hi) / 2.0,
            _ => hi,
        };
        Point::new(pick(self.h, r.x0, r.x1), pick(self.v, r.y0, r.y1))
    }
}

fn path_bounds(path: &Path) -> Option<Rect> {
    path.to_bez().ok().filter(|b| !b.elements().is_empty()).map(|b| b.bounding_box())
}

/// The bounding box of what `t` names: whole paths and components by their outlines, other
/// points, anchors, guides and annotations by their positions.
fn selection_bounds(font: &Font, r: &ResolvedTargets, t: &Targets) -> Option<Rect> {
    let parts = r.parts;
    let mut rect: Option<Rect> = None;
    let mut add = |b: Rect| rect = Some(rect.map_or(b, |r| r.union(b)));
    let mut pts: Vec<Point> = Vec::new();
    for (s, set) in moving::points_by_shape(t, parts) {
        let Some(path) = parts.path(s) else { continue };
        if (0..path.nodes.len()).all(|i| set.contains(&i)) {
            if let Some(b) = path_bounds(path) {
                add(b);
            }
        } else {
            pts.extend(set.iter().filter_map(|&i| path.nodes.get(i).map(Node::point)));
        }
    }
    for &s in &t.shapes {
        let b = match parts.shapes.get(s) {
            Some(Shape::Path(path)) => path_bounds(path),
            Some(Shape::Component(c)) => font.outline(&c.glyph, &r.layer.master).ok().filter(|o| !o.elements().is_empty()).map(|mut o| {
                o.apply_affine(c.affine());
                o.bounding_box()
            }),
            _ => None,
        };
        if let Some(b) = b {
            add(b);
        }
    }
    pts.extend(t.anchors.iter().filter_map(|n| parts.anchor(n)).map(|(_, a)| Point::new(a.x, a.y)));
    for g in &t.guides {
        let guide = match g.scope {
            GuideScope::Local => parts.guides.get(g.index),
            GuideScope::Global => r.master.guides.get(g.index),
        };
        pts.extend(guide.map(|g| Point::new(g.x, g.y)));
    }
    pts.extend(t.annotations.iter().filter_map(|&i| parts.annotations.get(i)).map(|a| Point::new(a.x, a.y)));
    if let Some(b) = fontcraft_geom::bounds(pts) {
        add(b);
    }
    rect
}

fn set_position(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("selection.setPosition", v)?;
    let t = Targets::parse(&p)?;
    let (x, y) = (coordinate(&p, "x")?, coordinate(&p, "y")?);
    if x.is_none() && y.is_none() {
        return Err(p.bad("give `x`, `y` or both"));
    }
    let reference = RefPoint::parse(&p)?;
    if !t.has_lists() {
        return Err(p.bad("give what to place: `points`, `segments`, `shapes`, `anchors`, `guides` or `annotations`"));
    }
    let (d, step) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        let rect = selection_bounds(font, &r, &t).ok_or_else(|| p.bad("the selection has no position"))?;
        let at = reference.of(rect);
        (Vec2::new(x.map_or(0.0, |x| x - at.x), y.map_or(0.0, |y| y - at.y)), grid(font))
    };
    s.edit("Set Position", |font| moving::apply(font, &t, d, Mode::Normal, false, step))?;
    Ok(json!({ "dx": d.x, "dy": d.y, "selection": t.to_json() }))
}

fn get_info(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("selection.getInfo", v)?;
    let t = Targets::parse(&p)?;
    let reference = RefPoint::parse(&p)?;
    if !t.has_lists() {
        return Err(p.bad("give what to measure: `points`, `segments`, `shapes`, `anchors`, `guides` or `annotations`"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    let mut out = Map::new();
    out.insert("glyph".into(), json!(t.glyph));
    out.insert("layer".into(), json!(r.layer.id));
    out.insert("referencePoint".into(), json!(reference.name));
    if let Some(rect) = selection_bounds(font, &r, &t) {
        let at = reference.of(rect);
        out.insert("x".into(), json!(at.x));
        out.insert("y".into(), json!(at.y));
        out.insert("width".into(), json!(rect.width()));
        out.insert("height".into(), json!(rect.height()));
    }
    let only_points = t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty();
    let node = |(s, n): (usize, usize)| r.parts.path(s).and_then(|path| path.nodes.get(n).map(|node| (path, node)));
    match t.points.as_slice() {
        [one] if only_points => {
            if let Some((path, n)) = node(*one) {
                if n.on_curve() {
                    out.insert("node".into(), json!({"x": n.x, "y": n.y}));
                } else if let Some((k, _)) = ops::handle_owner(path, one.1)
                    && let Some(owner) = path.nodes.get(k)
                {
                    let (dx, dy) = (n.x - owner.x, n.y - owner.y);
                    out.insert("handle".into(), json!({"x": n.x, "y": n.y, "dx": dx, "dy": dy, "length": dx.hypot(dy), "node": [one.0, k]}));
                }
            }
        }
        [a, b] if only_points => {
            if let (Some((_, na)), Some((_, nb))) = (node(*a), node(*b))
                && na.on_curve()
                && nb.on_curve()
            {
                let (dx, dy) = (nb.x - na.x, nb.y - na.y);
                out.insert("distance".into(), json!({"dx": dx, "dy": dy, "length": dx.hypot(dy)}));
            }
        }
        _ => {}
    }
    Ok(Value::Object(out))
}

// ── Node types and segments ─────────────────────────────────────────────────────────────────────

fn toggle_node_type(s: &mut Session, v: &Value) -> Result<Value> {
    toggle(s, v, "path.toggleNodeType", false)
}

fn toggle_keeping_outline(s: &mut Session, v: &Value) -> Result<Value> {
    toggle(s, v, "path.toggleNodeTypeKeepingOutline", true)
}

fn toggle(s: &mut Session, v: &Value, command: &'static str, keep_outline: bool) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    let to = match p.opt_str("to")? {
        None => None,
        Some("smooth") => Some(true),
        Some("corner") => Some(false),
        Some(other) => return Err(p.bad(format!("`to` must be smooth or corner, got \"{other}\""))),
    };
    let keep = keep_outline || p.bool_or("keepOutline", false)?;
    if t.points.is_empty() {
        return Err(p.bad("give `points`: the nodes to switch"));
    }
    let step = grid(s.font()?);
    let points = s.edit("Toggle Node Type", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::new();
            let mut any = false;
            for (shape, set) in group(&t.points) {
                let Some(path) = parts.path_mut(shape) else { continue };
                let nodes: BTreeSet<usize> = set.iter().copied().filter(|&i| ops::on(path, i)).collect();
                if nodes.is_empty() {
                    out.extend(set.iter().map(|&i| (shape, i)));
                    continue;
                }
                any = true;
                let (new, map) = ops::toggle(path, &nodes, to, keep, step);
                *path = new;
                out.extend(set.iter().filter_map(|&i| map.get(i).map(|&k| (shape, k))));
            }
            if !any {
                return Err(p.bad("none of `points` is an on-curve node"));
            }
            Ok(out)
        })
    })?;
    let mut sel = t.clone();
    sel.points = points;
    Ok(json!({ "selection": sel.to_json() }))
}

fn convert_segment_to_curve(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.convertSegmentToCurve", v)?;
    let t = Targets::parse(&p)?;
    if t.segments.is_empty() {
        return Err(p.bad("give `segments`: the straight segments to curve"));
    }
    let step = grid(s.font()?);
    let segments = s.edit("Convert to Curve", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::new();
            let mut any = false;
            for (shape, ends) in group(&t.segments) {
                let Some(path) = parts.path_mut(shape) else { continue };
                let lines: BTreeSet<usize> = path.segments().iter().filter(|g| g.is_line() && ends.contains(&g.end)).map(|g| g.end).collect();
                if lines.is_empty() {
                    out.extend(ends.iter().map(|&e| (shape, e)));
                    continue;
                }
                any = true;
                let (new, map) = ops::curves_from_lines(path, &lines, step);
                *path = new;
                out.extend(ends.iter().filter_map(|&e| map.get(e).map(|&k| (shape, k))));
            }
            if !any {
                return Err(p.bad("none of `segments` is a straight segment"));
            }
            Ok(out)
        })
    })?;
    let mut sel = t.clone();
    sel.segments = segments;
    Ok(json!({ "selection": sel.to_json() }))
}

fn insert_node(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.insertNode", v)?;
    let t = Targets::parse(&p)?;
    let &[(shape, end)] = t.segments.as_slice() else {
        return Err(p.bad("give one `segment`: [shape, node], the node that ends it"));
    };
    let near = point_param(&p, "near")?;
    let snap = match p.opt_str("snap")? {
        None => None,
        Some("extremum") => Some(true),
        Some("inflection") => Some(false),
        Some(other) => return Err(p.bad(format!("`snap` must be extremum or inflection, got \"{other}\""))),
    };
    let given = p.opt_f64("t")?;
    let (at, step) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        let seg = r.parts.path(shape).and_then(|path| path.segment_ending_at(end)).ok_or_else(|| p.bad("`segment` does not resolve"))?;
        let mut at = match near {
            Some(pt) => seg.nearest(pt).map(|n| n.t).ok_or_else(|| p.bad("`near` has no nearest point on the segment"))?,
            None => given.unwrap_or(0.5),
        };
        if let Some(extremum) = snap {
            let candidates = if extremum { seg.extremes() } else { seg.inflections() };
            let base = at;
            let key = |c: f64| match near {
                Some(pt) => seg.eval(c).map_or(f64::INFINITY, |q| q.distance(pt)),
                None => (c - base).abs(),
            };
            at = candidates
                .into_iter()
                .min_by(|a, b| key(*a).total_cmp(&key(*b)))
                .ok_or_else(|| p.bad(format!("the segment has no {}", if extremum { "extremum" } else { "inflection" })))?;
        }
        if !(at > 1e-9 && at < 1.0 - 1e-9) {
            return Err(p.bad("`t` must lie strictly between 0 and 1"));
        }
        (at, grid(font))
    };
    let index = s.edit("Insert Node", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let path = parts.path_mut(shape).ok_or_else(|| p.bad("`segment` does not resolve"))?;
            let (new, index) = ops::insert_node(path, end, at, step).map_err(|m| p.bad(m))?;
            *path = new;
            Ok(index)
        })
    })?;
    let mut sel = none_selected(&t);
    sel.points = vec![(shape, index)];
    Ok(json!({ "node": [shape, index], "selection": sel.to_json() }))
}

// ── Deleting ────────────────────────────────────────────────────────────────────────────────────

/// `edit.delete` on one side of a layer.
fn delete_in(parts: &mut LayerPartsMut, t: &Targets, step: f64) -> std::result::Result<(), String> {
    let mut remove: BTreeSet<usize> = t.shapes.iter().copied().collect();
    for (s, set) in group(&t.points) {
        if remove.contains(&s) {
            continue;
        }
        let Some(Shape::Path(path)) = parts.shapes.get(s) else { continue };
        match ops::delete_points(path, &set, step).map_err(|m| format!("path {s}: {m}"))? {
            Some(new) => {
                if let Some(slot) = parts.shapes.get_mut(s) {
                    *slot = Shape::Path(new);
                }
            }
            None => {
                remove.insert(s);
            }
        }
    }
    remove_at(&mut *parts.shapes, &remove);
    parts.anchors.retain(|a| !t.anchors.contains(&a.name));
    let local: BTreeSet<usize> = t.guides.iter().filter(|g| g.scope == GuideScope::Local).map(|g| g.index).collect();
    remove_at(&mut *parts.guides, &local);
    let annotations: BTreeSet<usize> = t.annotations.iter().copied().collect();
    remove_at(&mut *parts.annotations, &annotations);
    Ok(())
}

fn delete(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.delete", v)?;
    let t = Targets::parse(&p)?;
    if !t.segments.is_empty() {
        return Err(p.bad("takes `points`, not `segments`: name a segment's handles to straighten it"));
    }
    if !t.has_lists() {
        return Err(p.bad("give what to delete: `points`, `shapes`, `anchors`, `guides` or `annotations`"));
    }
    let step = grid(s.font()?);
    s.edit("Delete", |font| {
        let master = t.resolve(font)?.master.id.clone();
        t.edit_layer(font, |layer| delete_in(&mut layer.parts_mut(t.side), &t, step).map_err(|m| p.bad(m)))?;
        let global: BTreeSet<usize> = t.guides.iter().filter(|g| g.scope == GuideScope::Global).map(|g| g.index).collect();
        if !global.is_empty()
            && let Some(m) = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == master)
        {
            remove_at(&mut m.guides, &global);
        }
        Ok(())
    })?;
    Ok(json!({ "selection": none_selected(&t).to_json() }))
}

fn delete_in_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.deleteInAllMasters", v)?;
    let t = Targets::parse(&p)?;
    if !(t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes only `points` and `shapes`"));
    }
    if t.points.is_empty() && t.shapes.is_empty() {
        return Err(p.bad("give what to delete: `points` or `shapes`"));
    }
    let step = grid(s.font()?);
    let skipped = s.edit("Delete in All Masters", |font| in_all_masters(font, &t, |parts| delete_in(parts, &t, step).map_err(|m| p.bad(m))))?;
    Ok(json!({ "selection": none_selected(&t).to_json(), "skipped": skipped }))
}

/// `path.deleteAndBreak` on one side of a layer. Returns whether anything changed.
fn break_in(parts: &mut LayerPartsMut, t: &Targets) -> bool {
    let points = group(&t.points);
    let named = group(&t.segments);
    let shapes: BTreeSet<usize> = points.keys().chain(named.keys()).copied().collect();
    let mut replaced: BTreeMap<usize, Vec<Path>> = BTreeMap::new();
    for s in shapes {
        let Some(Shape::Path(path)) = parts.shapes.get(s) else { continue };
        let selected: BTreeSet<usize> = points.get(&s).map(|set| set.iter().copied().filter(|&i| ops::on(path, i)).collect()).unwrap_or_default();
        let mut ends: BTreeSet<usize> = named.get(&s).cloned().unwrap_or_default();
        let segs = path.segments();
        ends.extend(segs.iter().filter(|g| selected.contains(&g.start) && selected.contains(&g.end)).map(|g| g.end));
        let in_segment: BTreeSet<usize> = segs.iter().filter(|g| ends.contains(&g.end)).flat_map(|g| [g.start, g.end]).collect();
        let nodes: BTreeSet<usize> = selected.difference(&in_segment).copied().collect();
        if let Some(pieces) = ops::break_path(path, &nodes, &ends) {
            replaced.insert(s, pieces);
        }
    }
    if replaced.is_empty() {
        return false;
    }
    let old = std::mem::take(&mut *parts.shapes);
    for (i, shape) in old.into_iter().enumerate() {
        match replaced.remove(&i) {
            Some(pieces) => parts.shapes.extend(pieces.into_iter().map(Shape::Path)),
            None => parts.shapes.push(shape),
        }
    }
    true
}

/// Checks that `t` names something `path.deleteAndBreak` acts on: an on-curve node or a segment.
fn check_breakable(s: &Session, p: &Params, t: &Targets) -> Result<()> {
    let r = t.resolve(s.font()?)?;
    if t.segments.is_empty() && !t.points.iter().any(|&(sh, n)| r.parts.node(sh, n).is_some_and(Node::on_curve)) {
        return Err(p.bad("give on-curve nodes in `points`, or `segments`"));
    }
    Ok(())
}

fn delete_and_break(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.deleteAndBreak", v)?;
    let t = Targets::parse(&p)?;
    check_breakable(s, &p, &t)?;
    s.edit("Delete and Break", |font| t.edit_layer(font, |layer| Ok(break_in(&mut layer.parts_mut(t.side), &t))))?;
    Ok(json!({ "selection": none_selected(&t).to_json() }))
}

fn delete_and_break_in_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.deleteAndBreakInAllMasters", v)?;
    let t = Targets::parse(&p)?;
    check_breakable(s, &p, &t)?;
    let skipped = s.edit("Delete and Break in All Masters", |font| {
        in_all_masters(font, &t, |parts| {
            break_in(parts, &t);
            Ok(())
        })
    })?;
    Ok(json!({ "selection": none_selected(&t).to_json(), "skipped": skipped }))
}

// ── Opening, closing, start nodes, direction ────────────────────────────────────────────────────

fn connect_nodes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.connectNodes", v)?;
    let t = Targets::parse(&p)?;
    let &[first, second] = t.points.as_slice() else {
        return Err(p.bad("give two open ends in `points`"));
    };
    let (a, b) = if first.0 <= second.0 { (first, second) } else { (second, first) };
    let step = grid(s.font()?);
    let joined = s.edit("Connect Nodes", |font| {
        t.edit_layer(font, |layer| {
            let parts = layer.parts_mut(t.side);
            let end = |(sh, n): (usize, usize)| match parts.shapes.get(sh) {
                Some(Shape::Path(path)) if is_open_end(path, n) => Ok(path.clone()),
                _ => Err(p.bad(format!("[{sh}, {n}] is not an open end of a path"))),
            };
            let (pa, pb) = (end(a)?, end(b)?);
            if a.0 == b.0 {
                let closed = ops::close(&pa, step).map_err(|m| p.bad(format!("path {}: {m}", a.0)))?;
                let last = closed.nodes.len().saturating_sub(1);
                let merged = closed.nodes.len() < pa.nodes.len();
                if let Some(slot) = parts.shapes.get_mut(a.0) {
                    *slot = Shape::Path(closed);
                }
                Ok(if merged { vec![(a.0, 0)] } else { vec![(a.0, last), (a.0, 0)] })
            } else {
                let (path, at) = ops::join(&pa, a.1, &pb, b.1, step).map_err(|m| p.bad(m))?;
                if let Some(slot) = parts.shapes.get_mut(a.0) {
                    *slot = Shape::Path(path);
                }
                if b.0 < parts.shapes.len() {
                    parts.shapes.remove(b.0);
                }
                Ok(at.into_iter().map(|n| (a.0, n)).collect())
            }
        })
    })?;
    let mut sel = none_selected(&t);
    sel.points = joined;
    Ok(json!({ "selection": sel.to_json() }))
}

/// The shapes `t` names directly or through its points.
fn named_shapes(t: &Targets) -> BTreeSet<usize> {
    t.shapes.iter().copied().chain(t.points.iter().map(|&(s, _)| s)).collect()
}

fn close_open_paths(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.closeOpenPaths", v)?;
    let t = Targets::parse(&p)?;
    let shapes = named_shapes(&t);
    if shapes.is_empty() {
        return Err(p.bad("give the paths to close in `shapes` (or `points` on them)"));
    }
    let step = grid(s.font()?);
    let closed = s.edit("Close Path", |font| {
        t.edit_layer(font, |layer| {
            let parts = layer.parts_mut(t.side);
            let mut closed = Vec::new();
            for &sh in &shapes {
                if let Some(Shape::Path(path)) = parts.shapes.get_mut(sh)
                    && !path.closed
                    && path.nodes.iter().filter(|n| n.on_curve()).count() >= 2
                {
                    *path = ops::close(path, step).map_err(|m| p.bad(format!("path {sh}: {m}")))?;
                    closed.push(sh);
                }
            }
            if closed.is_empty() {
                return Err(p.bad("none of the paths is open"));
            }
            Ok(closed)
        })
    })?;
    let mut sel = none_selected(&t);
    sel.shapes = closed;
    Ok(json!({ "selection": sel.to_json() }))
}

fn make_node_first(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.makeNodeFirst", v)?;
    let mut t = Targets::parse(&p)?;
    if let Some(pt) = pair_param(&p, "point")?
        && !t.points.contains(&pt)
    {
        t.points.push(pt);
    }
    let &[(shape, node)] = t.points.as_slice() else {
        return Err(p.bad("give one `point`: [shape, node]"));
    };
    s.edit("Make Node First", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let path = parts.path_mut(shape).ok_or_else(|| p.bad("`point` is not on a path"))?;
            if !path.closed {
                return Err(p.bad("the path is open: its start is its first end"));
            }
            if !ops::on(path, node) || node >= path.nodes.len() {
                return Err(p.bad("a handle cannot be a start node"));
            }
            path.nodes.rotate_left(node);
            Ok(())
        })
    })?;
    let mut sel = none_selected(&t);
    sel.points = vec![(shape, 0)];
    Ok(json!({ "selection": sel.to_json() }))
}

/// Reverses the paths at `shapes` (every path when empty). Returns how many.
fn reverse_in(parts: &mut LayerPartsMut, shapes: &[usize]) -> std::result::Result<usize, String> {
    let targets: Vec<usize> = if shapes.is_empty() { (0..parts.shapes.len()).collect() } else { shapes.to_vec() };
    let mut count = 0;
    for i in targets {
        match parts.shapes.get_mut(i) {
            Some(Shape::Path(path)) => {
                *path = ops::reversed(path).map_err(|m| format!("path {i}: {m}"))?;
                count += 1;
            }
            Some(_) => {}
            None => return Err(format!("shape {i} does not resolve")),
        }
    }
    if !shapes.is_empty() && count == 0 {
        return Err("none of `shapes` is a path".into());
    }
    Ok(count)
}

/// The target objects of a command that takes `glyph` or `glyphs`.
fn glyph_targets(s: &Session, p: &Params, v: &Value, command: &'static str) -> Result<Vec<Targets>> {
    if p.get("glyph").is_some() {
        return Ok(vec![Targets::parse(p)?]);
    }
    let names = p.opt_glyphs("glyphs", s.font()?)?.ok_or_else(|| p.bad("give `glyphs` (or one `glyph`)"))?;
    match names.as_slice() {
        [] => Err(p.bad("`glyphs` is empty")),
        [one] => {
            let mut obj = v.as_object().cloned().unwrap_or_default();
            obj.remove("glyphs");
            obj.insert("glyph".into(), json!(one));
            let single = Value::Object(obj);
            Ok(vec![Targets::parse(&Params::new(command, &single)?)?])
        }
        _ => {
            if p.get("shapes").is_some() {
                return Err(p.bad("`shapes` names shapes of one glyph: give one `glyph`"));
            }
            let layer = p.opt_str("layer")?.map(str::to_string);
            let side = match p.opt_str("side")? {
                None => LayerSide::Foreground,
                Some(side) => {
                    LayerSide::parse(side).ok_or_else(|| p.bad(format!("`side` must be \"foreground\" or \"background\", got \"{side}\"")))?
                }
            };
            Ok(names
                .iter()
                .map(|n| {
                    let mut t = Targets::new(command, n.clone(), layer.clone());
                    t.side = side;
                    t
                })
                .collect())
        }
    }
}

fn reverse_contours(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.reverseContours", v)?;
    let list = glyph_targets(s, &p, v, "path.reverseContours")?;
    let count = s.edit("Reverse Contours", |font| {
        let mut count = 0;
        for t in &list {
            count += t.edit_layer(font, |layer| {
                if layer.parts(t.side).shapes.is_empty() {
                    return Ok(0);
                }
                reverse_in(&mut layer.parts_mut(t.side), &t.shapes).map_err(|m| p.bad(m))
            })?;
        }
        Ok(count)
    })?;
    let mut out = json!({ "reversed": count });
    if let ([t], Some(o)) = (list.as_slice(), out.as_object_mut()) {
        o.insert("selection".into(), t.to_json());
    }
    Ok(out)
}

fn reverse_contours_for_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.reverseContoursForAllMasters", v)?;
    let list = glyph_targets(s, &p, v, "path.reverseContoursForAllMasters")?;
    let (count, skipped) = s.edit("Reverse Contours for All Masters", |font| {
        let mut count = 0;
        let mut skipped = Vec::new();
        for t in &list {
            let mut g = font.glyph_mut(&t.glyph)?;
            crate::check_unlocked(&g)?;
            for layer in g.layers.iter_mut().filter(|l| l.id == l.master) {
                if layer.parts(t.side).shapes.is_empty() {
                    continue;
                }
                let id = layer.id.clone();
                match reverse_in(&mut layer.parts_mut(t.side), &t.shapes) {
                    Ok(n) => count += n,
                    Err(why) => skipped.push(json!({"glyph": t.glyph, "layer": id, "reason": why})),
                }
            }
        }
        Ok((count, skipped))
    })?;
    Ok(json!({ "reversed": count, "skipped": skipped }))
}

// ── Locks ───────────────────────────────────────────────────────────────────────────────────────

fn lock_path(s: &mut Session, v: &Value) -> Result<Value> {
    set_lock(s, v, "path.lockPath", true)
}

fn unlock_path(s: &mut Session, v: &Value) -> Result<Value> {
    set_lock(s, v, "path.unlockPath", false)
}

fn set_lock(s: &mut Session, v: &Value, command: &'static str, locked: bool) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    let shapes = named_shapes(&t);
    if shapes.is_empty() {
        return Err(p.bad("give the paths in `shapes` (or `points` on them)"));
    }
    s.edit(if locked { "Lock Path" } else { "Unlock Path" }, |font| {
        t.edit_layer(font, |layer| {
            let parts = layer.parts_mut(t.side);
            for &i in &shapes {
                match parts.shapes.get_mut(i) {
                    Some(Shape::Path(path)) if path.info.locked != locked => path.info.locked = locked,
                    Some(Shape::Component(c)) if c.info.locked != locked => c.info.locked = locked,
                    _ => {}
                }
            }
            Ok(())
        })
    })?;
    let mut sel = none_selected(&t);
    sel.shapes = shapes.into_iter().collect();
    Ok(json!({ "selection": sel.to_json() }))
}

// ── Primitives ──────────────────────────────────────────────────────────────────────────────────

/// Bézier handle length, as a fraction of the radius, of a quarter ellipse drawn with one cubic.
const KAPPA: f64 = 0.552_284_749_830_793_4;

fn add_rectangle(s: &mut Session, v: &Value) -> Result<Value> {
    add_primitive(s, v, "path.addRectangle", false)
}

fn add_ellipse(s: &mut Session, v: &Value) -> Result<Value> {
    add_primitive(s, v, "path.addEllipse", true)
}

fn size(p: &Params, key: &str) -> Result<f64> {
    let v = p.f64(key)?;
    if v == 0.0 || v.abs() > 2.0 * MAX_COORDINATE {
        return Err(p.bad(format!("`{key}` must not be 0 and must be within ±{}", 2.0 * MAX_COORDINATE)));
    }
    Ok(v)
}

fn ellipse(x0: f64, y0: f64, x1: f64, y1: f64, step: f64) -> Path {
    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let node = |x: f64, y: f64, kind: NodeKind| {
        let r = round_point(Point::new(x, y), step);
        let mut n = Node::new(r.x, r.y, kind);
        n.smooth = kind == NodeKind::Curve;
        n
    };
    let (on, off) = (NodeKind::Curve, NodeKind::OffCurve);
    Path::new(
        true,
        vec![
            node(cx, y0, on),
            node(cx + kx, y0, off),
            node(x1, cy - ky, off),
            node(x1, cy, on),
            node(x1, cy + ky, off),
            node(cx + kx, y1, off),
            node(cx, y1, on),
            node(cx - kx, y1, off),
            node(x0, cy + ky, off),
            node(x0, cy, on),
            node(x0, cy - ky, off),
            node(cx - kx, y0, off),
        ],
    )
}

fn add_primitive(s: &mut Session, v: &Value, command: &'static str, is_ellipse: bool) -> Result<Value> {
    let p = Params::new(command, v)?;
    let glyph = p.str("glyph")?.to_string();
    let x = coordinate(&p, "x")?.ok_or_else(|| p.bad("`x` is required"))?;
    let y = coordinate(&p, "y")?.ok_or_else(|| p.bad("`y` is required"))?;
    let (w, h) = (size(&p, "width")?, size(&p, "height")?);
    let (x, y) = if p.bool_or("fromCenter", false)? { (x - w / 2.0, y - h / 2.0) } else { (x, y) };
    let (x0, x1, y0, y1) = (x.min(x + w), x.max(x + w), y.min(y + h), y.max(y + h));
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it adds a path to the layer"));
    }
    let (layer, step) = {
        let font = s.font()?;
        (t.layer_id(font)?, grid(font))
    };
    let path = if is_ellipse {
        ellipse(x0, y0, x1, y1, step)
    } else {
        let (a, b) = (round_point(Point::new(x0, y0), step), round_point(Point::new(x1, y1), step));
        Path::rect(a.x, a.y, b.x, b.y)
    };
    t.layer = Some(layer.clone());
    let data = s.glyph_data().ok();
    let index = s.edit(if is_ellipse { "Add Ellipse" } else { "Add Rectangle" }, |font| {
        let is_master = font.master(&layer).is_ok();
        let width = super::widths::new_layer_width(font, data.as_deref(), &glyph, &layer);
        let mut g = font.glyph_mut(&glyph)?;
        if is_master && !g.layers.iter().any(|l| l.id == layer) {
            crate::check_unlocked(&g)?;
            g.layers.push(Layer::new(&layer, width));
        }
        drop(g);
        t.edit_layer(font, |l| {
            let shapes = l.parts_mut(t.side).shapes;
            shapes.push(Shape::Path(path));
            Ok(shapes.len() - 1)
        })
    })?;
    t.shapes = vec![index];
    Ok(json!({ "shape": index, "selection": t.to_json() }))
}
