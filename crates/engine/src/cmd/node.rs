//! Node and segment editing (`docs/spec/03a` § 7 to § 16): moving, node types, converting,
//! deleting, inserting, connecting, closing, start nodes, path direction, locks, and the headless
//! rectangle and ellipse.
//!
//! Every command names its targets (decision 0004), refuses a locked glyph, rounds the points it
//! creates or moves to the font's grid (points it does not touch keep their coordinates), and
//! returns the `selection` afterwards. The "in All Masters" forms ([`masters`]) apply the same
//! addresses to every layer of the glyph that interpolates with the edited one; a layer whose
//! shapes or node types differ is left alone and listed in `skipped`.

mod masters;
mod matching;
mod moving;
mod ops;
mod refit;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fontcraft_geom::{Point, Rect, Shape as _, Vec2, round_point};
use fontcraft_model::guide_follow::placed_x;
use fontcraft_model::{Font, Layer, LayerParts, LayerPartsMut, LayerSide, MAX_COORDINATE, Node, NodeKind, Path, Shape};
use serde_json::{Map, Value, json};

pub(super) use self::matching::{Plan, undo_label};
use self::moving::Mode;
pub(super) use self::ops::keep_tangents;
use super::common::grid;
use super::*;
use crate::params::{GuideScope, Params, ResolvedTargets, TargetKind as K, Targets};

pub fn specs() -> Vec<CommandSpec> {
    const ALL: &[K] = &[K::Layer, K::Points, K::Segments, K::Shapes, K::Anchors, K::Guides, K::Annotations];
    let mut specs = vec![
        cmd!(
            "selection.move",
            "Move",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index], image?: bool, dx: number, dy: number, mode?: \"normal\" | \"pointsOnly\" | \"nudge\" | \"mirror\" (default normal), duplicate?: bool} moves what the targets name by (dx, dy) font units, within ±2,000,000. `image: true` moves the layer's placed image on that side too, unrounded; a locked image, or none, refuses it, and it cannot be duplicated. normal: the handles of moved nodes go with them, and the opposite handle of a smooth node turns to stay in line with a moved handle. In every mode but pointsOnly, the handle of a smooth node next to a line turns onto the line when the node or the line's other end moves. pointsOnly: only the named points move, and a handle alone keeps its angle. nudge: the handles of each segment with one moved end are scaled between its ends. mirror: as normal, but the opposite handle of a smooth node takes the same length as a moved handle, at the opposite angle. A segment moves its two end nodes. A locked component, or one aligned automatically, stays; one aligned only horizontally takes only dy; when nothing named moves, the command fails with a message naming the components. duplicate: the original stays and a copy moves (part of a path is copied as open paths of its selected segments), with the corner, cap and segment components and brushes that sit on it. Moved points are rounded to the grid → {selection}",
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
            "{glyph: name, layer?: id, side?, segments: [[shape, node]] (or segment: [shape, node]), orthogonal?: bool} turns each straight segment into a curve with handles at a third and two thirds of it (one handle at the middle of a quadratic one); the outline keeps its shape. The curve is quadratic in a path with quadratic segments, and in a path without curves when the layer's curves, else the font's, are. `orthogonal: true` bends a diagonal segment into a quarter of an oval instead: one handle horizontal and one vertical, the way round that continues the neighbouring segments better. Curve segments are left alone → {selection}",
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
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope, index}], annotations?: [index], hints?: [hint index], image?: bool, matchLayers?: bool} removes what the targets name; `image: true` removes the layer's placed image on that side (a locked image, or none, refuses it). `hints` are positions in the layer's hint list (corner components and other hints of the foreground); with `side: background` they are refused. A deleted node leaves the path closed: the segments it joined become one, refitted to the old curve; a curve segment that loses a handle becomes a line; a path with every point named, or fewer than two nodes left, is removed. `matchLayers: true` makes the same edit on every other master, intermediate and alternate layer of the glyph with the same shapes and node types, in the same undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take is refused (`points` and `shapes` only) → {selection, matched?: [{glyph, layer}], skipped?: [{glyph, layer, reason}]}",
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
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points?: [[shape, node]], shapes?: [index]} edit.delete in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take is refused → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            masters::delete
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
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points?: [[shape, node]], segments?: [[shape, node]]} path.deleteAndBreak in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take is refused → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            masters::delete_and_break
        )
        .targets(&[K::Layer, K::Points, K::Segments])
        .when("editView"),
        cmd!(
            "path.insertNode",
            "Insert Node",
            [],
            None,
            "{glyph: name, layer?: id, side?, segment: [shape, node], t?: number strictly between 0 and 1 (default 0.5), near?: [x, y] (the segment's point nearest to it), snap?: \"extremum\" | \"inflection\" (the one nearest to near, else to t), matchLayers?: bool} splits the segment without changing its shape (up to grid rounding). `matchLayers: true` makes the same edit on every other master, intermediate and alternate layer of the glyph with the same shapes and node types, in the same undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take is refused, the segment split at the same t → {node: [shape, node], selection, matched?: [{glyph, layer}], skipped?: [{glyph, layer, reason}]}",
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
            "{glyph: name, layer?: id, side?, point: [shape, node], matchLayers?: bool} makes the on-curve node the start node of its closed path; the outline does not change. `matchLayers: true` makes the same edit on every other master, intermediate and alternate layer of the glyph with the same shapes and node types, in the same undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take is refused → {selection, matched?: [{glyph, layer}], skipped?: [{glyph, layer, reason}]}",
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
            "{glyphs: [names] | glyph: name, layer?: id (default the first master's), side?, shapes?: [index] (with one glyph; default every path), matchLayers?: bool} reverses the direction of the paths; a closed path keeps its start node. `matchLayers: true` makes the same edit on every other master, intermediate and alternate layer of the glyph with the same shapes and node types, in the same undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take is refused → {reversed: count (on the layer named), selection?, matched?: [{glyph, layer}], skipped?: [{glyph, layer, reason}]}",
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
    ];
    specs.extend(masters::specs());
    specs
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

/// `reply` with the layers the edit matched and skipped, when it was made on several.
pub(super) fn with_matched(mut reply: Value, plan: Option<Plan>) -> Value {
    if let Some(plan) = plan {
        Plan::report(vec![plan], &mut reply);
    }
    reply
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

/// What a layer's shapes are, for matching layers: each path's closedness and node types, each
/// component's base glyph.
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

// ── Moving ──────────────────────────────────────────────────────────────────────────────────────

fn selection_move(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("selection.move", v)?;
    let t = Targets::parse(&p)?;
    let d = Vec2::new(offset(&p, "dx")?, offset(&p, "dy")?);
    let mode = match p.opt_str("mode")?.unwrap_or("normal") {
        "normal" => Mode::Normal,
        "pointsOnly" => Mode::PointsOnly,
        "nudge" => Mode::Nudge,
        "mirror" => Mode::Mirror,
        other => return Err(p.bad(format!("`mode` must be normal, pointsOnly, nudge or mirror, got \"{other}\""))),
    };
    let duplicate = p.bool_or("duplicate", false)?;
    let image = super::image::targeted(&p)?;
    if !t.has_lists() && !image {
        return Err(p.bad("give what to move: `points`, `segments`, `shapes`, `anchors`, `guides`, `annotations` or `image`"));
    }
    if image && duplicate {
        return Err(p.bad("a placed image cannot be duplicated: leave out `duplicate` or `image`"));
    }
    let step = grid(s.font()?);
    let sel = s.edit(if duplicate { "Duplicate" } else { "Move" }, |font| {
        let sel = if t.has_lists() { moving::apply(font, &t, d, mode, duplicate, step, !image)? } else { none_selected(&t) };
        if image {
            t.edit_layer(font, |l| super::image::move_by(&p, &t, l, d))?;
        }
        Ok(sel)
    })?;
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
            Some(Shape::Component(c)) => {
                font.component_outline(c, &r.layer.master).ok().filter(|o| !o.elements().is_empty()).map(|o| o.bounding_box())
            }
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
        pts.extend(guide.map(|g| Point::new(placed_x(g, r.layer.width), g.y)));
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
    s.edit("Set Position", |font| moving::apply(font, &t, d, Mode::Normal, false, step, true))?;
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

/// Moves the handle `h` of `path` by `d` as a drag does: smooth nodes stay smooth.
pub(super) fn move_handle(path: &mut Path, h: usize, d: Vec2, step: f64) {
    moving::move_path(path, &BTreeSet::from([h]), d, Mode::Normal, step);
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
    let quadratic = draws_quadratic(s.font()?, &t)?;
    let plan = Plan::of(s.font()?, &p, &t)?;
    let label = undo_label(&p, "Toggle Node Type");
    let points = s.edit(&label, |font| {
        // The new type of each node that changes, by shape.
        let mut types: BTreeMap<usize, BTreeMap<usize, bool>> = BTreeMap::new();
        let points = t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::new();
            let mut any = false;
            for (shape, set) in group(&t.points) {
                let Some(path) = parts.path_mut(shape) else { continue };
                let on = |i: usize| path.nodes.get(i).filter(|n| n.on_curve());
                any |= set.iter().any(|&i| on(i).is_some());
                let new: BTreeMap<usize, bool> = set
                    .iter()
                    .filter_map(|&i| {
                        let smooth = on(i)?.smooth;
                        let wanted = to.unwrap_or(!smooth);
                        (wanted != smooth).then_some((i, wanted))
                    })
                    .collect();
                let (changed, map) = ops::toggle(path, &new, keep, step, quadratic);
                *path = changed;
                out.extend(set.iter().filter_map(|&i| map.get(i).map(|&k| (shape, k))));
                types.insert(shape, new);
            }
            if !any {
                return Err(p.bad("none of `points` is an on-curve node"));
            }
            Ok(out)
        })?;
        if let Some(plan) = &plan {
            plan.apply(font, &p, |_, parts| {
                for (&shape, new) in &types {
                    let path = matching::path_at(parts, shape)?;
                    *path = ops::toggle(path, new, keep, step, quadratic).0;
                }
                Ok(())
            })?;
        }
        Ok(points)
    })?;
    let mut sel = t.clone();
    sel.points = points;
    Ok(with_matched(json!({ "selection": sel.to_json() }), plan))
}

/// Whether new curves on the layer `t` names are quadratic: as its own curves, else as the font's.
fn draws_quadratic(font: &Font, t: &Targets) -> Result<bool> {
    let r = t.resolve(font)?;
    Ok(fontcraft_model::curve_type::quadratic_curves(r.parts.shapes).unwrap_or_else(|| font.draws_quadratic()))
}

fn convert_segment_to_curve(s: &mut Session, v: &Value) -> Result<Value> {
    curve_segments(s, v, "path.convertSegmentToCurve")
}

fn curve_segments(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    if t.segments.is_empty() {
        return Err(p.bad("give `segments`: the straight segments to curve"));
    }
    let orthogonal = p.bool_or("orthogonal", false)?;
    let step = grid(s.font()?);
    let quadratic = draws_quadratic(s.font()?, &t)?;
    let plan = Plan::of(s.font()?, &p, &t)?;
    let label = undo_label(&p, "Convert to Curve");
    let segments = s.edit(&label, |font| {
        // The straight segments, by shape.
        let mut straight: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let segments = t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::new();
            for (shape, ends) in group(&t.segments) {
                let Some(path) = parts.path_mut(shape) else { continue };
                let lines: BTreeSet<usize> = path.segments().iter().filter(|g| g.is_line() && ends.contains(&g.end)).map(|g| g.end).collect();
                if lines.is_empty() {
                    out.extend(ends.iter().map(|&e| (shape, e)));
                    continue;
                }
                let (new, map) = ops::curves_from_lines(path, &lines, step, quadratic, orthogonal);
                *path = new;
                out.extend(ends.iter().filter_map(|&e| map.get(e).map(|&k| (shape, k))));
                straight.insert(shape, lines);
            }
            if straight.is_empty() {
                return Err(p.bad("none of `segments` is a straight segment"));
            }
            Ok(out)
        })?;
        if let Some(plan) = &plan {
            plan.apply(font, &p, |_, parts| {
                for (&shape, lines) in &straight {
                    let path = matching::path_at(parts, shape)?;
                    *path = ops::curves_from_lines(path, lines, step, quadratic, orthogonal).0;
                }
                Ok(())
            })?;
        }
        Ok(segments)
    })?;
    let mut sel = t.clone();
    sel.segments = segments;
    Ok(with_matched(json!({ "selection": sel.to_json() }), plan))
}

fn insert_node(s: &mut Session, v: &Value) -> Result<Value> {
    split_segment(s, v, "path.insertNode")
}

fn split_segment(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
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
    let plan = Plan::of(s.font()?, &p, &t)?;
    let index = s.edit(&undo_label(&p, "Insert Node"), |font| {
        let index = t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let path = parts.path_mut(shape).ok_or_else(|| p.bad("`segment` does not resolve"))?;
            let (new, index) = ops::insert_node(path, end, at, step).map_err(|m| p.bad(m))?;
            *path = new;
            Ok(index)
        })?;
        if let Some(plan) = &plan {
            plan.apply(font, &p, |_, parts| {
                let path = matching::path_at(parts, shape)?;
                *path = ops::insert_node(path, end, at, step)?.0;
                Ok(())
            })?;
        }
        Ok(index)
    })?;
    let mut sel = none_selected(&t);
    sel.points = vec![(shape, index)];
    Ok(with_matched(json!({ "node": [shape, index], "selection": sel.to_json() }), plan))
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
    remove_targets(s, v, "edit.delete")
}

fn remove_targets(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    if !t.segments.is_empty() {
        return Err(p.bad("takes `points`, not `segments`: name a segment's handles to straighten it"));
    }
    let image = super::image::targeted(&p)?;
    let hints = hint_list(&p)?;
    if !hints.is_empty() && t.side != LayerSide::Foreground {
        return Err(p.bad("`hints` belong to the foreground: give them without `side: background`"));
    }
    if !t.has_lists() && !image && hints.is_empty() {
        return Err(p.bad("give what to delete: `points`, `shapes`, `anchors`, `guides`, `annotations`, `hints` or `image`"));
    }
    let step = grid(s.font()?);
    let plan = if t.points.is_empty() && t.shapes.is_empty() { None } else { Plan::of(s.font()?, &p, &t)? };
    s.edit(&undo_label(&p, "Delete"), |font| {
        let master = t.resolve(font)?.master.id.clone();
        t.edit_layer(font, |layer| {
            if image {
                super::image::remove(&p, &t, layer)?;
            }
            let count = layer.info.hints.len();
            if let Some(&i) = hints.iter().find(|&&i| i >= count) {
                return Err(p.bad(format!("`hints`: {i} does not resolve: the layer has {count} hints")));
            }
            // Hints first: their indices name the layer as it was given.
            remove_at(&mut layer.info.hints, &hints);
            delete_in(&mut layer.parts_mut(t.side), &t, step).map_err(|m| p.bad(m))
        })?;
        if let Some(plan) = &plan {
            // Nodes and shapes correspond by index across the layers; the other objects need not.
            let mut outline = none_selected(&t);
            outline.points.clone_from(&t.points);
            outline.shapes.clone_from(&t.shapes);
            plan.apply(font, &p, |_, parts| delete_in(parts, &outline, step))?;
        }
        let global: BTreeSet<usize> = t.guides.iter().filter(|g| g.scope == GuideScope::Global).map(|g| g.index).collect();
        if !global.is_empty()
            && let Some(m) = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == master)
        {
            remove_at(&mut m.guides, &global);
        }
        Ok(())
    })?;
    Ok(with_matched(json!({ "selection": none_selected(&t).to_json() }), plan))
}

/// `hints?`: positions in the layer's hint list, at most [`MAX_TARGETS`](crate::params::MAX_TARGETS).
fn hint_list(p: &Params) -> Result<BTreeSet<usize>> {
    let Some(v) = p.get("hints") else { return Ok(BTreeSet::new()) };
    let list = v.as_array().ok_or_else(|| p.bad("`hints` must be a list of hint indices"))?;
    if list.len() > crate::params::MAX_TARGETS {
        return Err(p.bad(format!("`hints` holds {} entries; at most {} are allowed", list.len(), crate::params::MAX_TARGETS)));
    }
    list.iter()
        .map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`hints` must hold hint indices, found {v}"))))
        .collect()
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
    break_at(s, v, "path.deleteAndBreak")
}

fn break_at(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    check_breakable(s, &p, &t)?;
    let plan = Plan::of(s.font()?, &p, &t)?;
    s.edit(&undo_label(&p, "Delete and Break"), |font| {
        t.edit_layer(font, |layer| Ok(break_in(&mut layer.parts_mut(t.side), &t)))?;
        match &plan {
            Some(plan) => plan.apply(font, &p, |_, parts| {
                break_in(parts, &t);
                Ok(())
            }),
            None => Ok(()),
        }
    })?;
    Ok(with_matched(json!({ "selection": none_selected(&t).to_json() }), plan))
}

// ── Opening, closing, start nodes, direction ────────────────────────────────────────────────────

/// Two open ends of paths to join, the one on the lower shape first.
#[derive(Clone, Copy)]
struct Ends {
    a: (usize, usize),
    b: (usize, usize),
}

impl Ends {
    /// The paths with the two ends, which may be the same one.
    fn paths<'s>(&self, shapes: &'s [Shape]) -> std::result::Result<(&'s Path, &'s Path), String> {
        let end = |(sh, n): (usize, usize)| match shapes.get(sh) {
            Some(Shape::Path(path)) if is_open_end(path, n) => Ok(path),
            _ => Err(format!("[{sh}, {n}] is not an open end of a path")),
        };
        Ok((end(self.a)?, end(self.b)?))
    }

    /// Whether the two ends lie on each other.
    fn meet(&self, shapes: &[Shape], step: f64) -> bool {
        let Ok((pa, pb)) = self.paths(shapes) else { return false };
        match (ops::point(pa, self.a.1), ops::point(pb, self.b.1)) {
            (Some(a), Some(b)) => ops::coincide(a, b, step),
            _ => false,
        }
    }

    /// Joins the ends, into one node with `merge`. Returns the joined nodes.
    fn join(&self, shapes: &mut Vec<Shape>, merge: bool) -> std::result::Result<Vec<(usize, usize)>, String> {
        let Ends { a, b } = *self;
        let (pa, pb) = self.paths(shapes)?;
        if a.0 == b.0 {
            let closed = ops::close(pa, merge).map_err(|m| format!("path {}: {m}", a.0))?;
            let last = closed.nodes.len().saturating_sub(1);
            if let Some(slot) = shapes.get_mut(a.0) {
                *slot = Shape::Path(closed);
            }
            return Ok(if merge { vec![(a.0, 0)] } else { vec![(a.0, last), (a.0, 0)] });
        }
        let (path, at) = ops::join(pa, a.1, pb, b.1, merge)?;
        if let Some(slot) = shapes.get_mut(a.0) {
            *slot = Shape::Path(path);
        }
        if b.0 < shapes.len() {
            shapes.remove(b.0);
        }
        Ok(at.into_iter().map(|n| (a.0, n)).collect())
    }
}

fn connect_nodes(s: &mut Session, v: &Value) -> Result<Value> {
    connect(s, v, "path.connectNodes")
}

fn connect(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    let &[first, second] = t.points.as_slice() else {
        return Err(p.bad("give two open ends in `points`"));
    };
    let ends = if first.0 <= second.0 { Ends { a: first, b: second } } else { Ends { a: second, b: first } };
    let font = s.font()?;
    let step = grid(font);
    let plan = Plan::of(font, &p, &t)?;
    // Ends that lie on each other on some of the layers only are joined by a segment on all.
    let merge = {
        let r = t.resolve(font)?;
        ends.meet(r.parts.shapes, step) && plan.as_ref().is_none_or(|plan| plan.layers(font).iter().all(|l| ends.meet(l.parts(t.side).shapes, step)))
    };
    let joined = s.edit(&undo_label(&p, "Connect Nodes"), |font| {
        let joined = t.edit_layer(font, |layer| ends.join(layer.parts_mut(t.side).shapes, merge).map_err(|m| p.bad(m)))?;
        if let Some(plan) = &plan {
            plan.apply(font, &p, |_, parts| ends.join(parts.shapes, merge).map(|_| ()))?;
        }
        Ok(joined)
    })?;
    let mut sel = none_selected(&t);
    sel.points = joined;
    Ok(with_matched(json!({ "selection": sel.to_json() }), plan))
}

/// The shapes `t` names directly or through its points.
fn named_shapes(t: &Targets) -> BTreeSet<usize> {
    t.shapes.iter().copied().chain(t.points.iter().map(|&(s, _)| s)).collect()
}

fn close_open_paths(s: &mut Session, v: &Value) -> Result<Value> {
    close_paths(s, v, "path.closeOpenPaths")
}

/// Whether the path at `shape` is open and its ends lie on each other; `None` when it cannot be closed.
fn closing_merges(shapes: &[Shape], shape: usize, step: f64) -> Option<bool> {
    let Some(Shape::Path(path)) = shapes.get(shape) else { return None };
    let (first, last) = (path.nodes.first()?, path.nodes.last()?);
    let closable = !path.closed && path.nodes.iter().filter(|n| n.on_curve()).count() >= 2;
    closable.then(|| ops::coincide(first.point(), last.point(), step))
}

fn close_paths(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    let shapes = named_shapes(&t);
    if shapes.is_empty() {
        return Err(p.bad("give the paths to close in `shapes` (or `points` on them)"));
    }
    let font = s.font()?;
    let step = grid(font);
    let plan = Plan::of(font, &p, &t)?;
    // The paths to close, and whether their ends become one node: where they lie on each other on every layer.
    let merges: BTreeMap<usize, bool> = {
        let r = t.resolve(font)?;
        let others = plan.as_ref().map(|plan| plan.layers(font)).unwrap_or_default();
        let merges = |sh: usize| {
            let here = closing_merges(r.parts.shapes, sh, step)?;
            Some((sh, here && others.iter().all(|l| closing_merges(l.parts(t.side).shapes, sh, step) == Some(true))))
        };
        shapes.iter().filter_map(|&sh| merges(sh)).collect()
    };
    if merges.is_empty() {
        return Err(p.bad("none of the paths is open"));
    }
    let close = |shapes: &mut [Shape]| -> std::result::Result<(), String> {
        for (&sh, &merge) in &merges {
            match shapes.get_mut(sh) {
                Some(Shape::Path(path)) => *path = ops::close(path, merge).map_err(|m| format!("path {sh}: {m}"))?,
                _ => return Err(format!("shape {sh} is not a path")),
            }
        }
        Ok(())
    };
    s.edit(&undo_label(&p, "Close Path"), |font| {
        t.edit_layer(font, |layer| close(layer.parts_mut(t.side).shapes).map_err(|m| p.bad(m)))?;
        match &plan {
            Some(plan) => plan.apply(font, &p, |_, parts| close(parts.shapes)),
            None => Ok(()),
        }
    })?;
    let mut sel = none_selected(&t);
    sel.shapes = merges.keys().copied().collect();
    Ok(with_matched(json!({ "selection": sel.to_json() }), plan))
}

fn make_node_first(s: &mut Session, v: &Value) -> Result<Value> {
    start_at(s, v, "path.makeNodeFirst")
}

fn start_at(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let mut t = Targets::parse(&p)?;
    if let Some(pt) = pair_param(&p, "point")?
        && !t.points.contains(&pt)
    {
        t.points.push(pt);
    }
    let &[(shape, node)] = t.points.as_slice() else {
        return Err(p.bad("give one `point`: [shape, node]"));
    };
    let plan = Plan::of(s.font()?, &p, &t)?;
    s.edit(&undo_label(&p, "Make Node First"), |font| {
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
        })?;
        match &plan {
            Some(plan) => plan.apply(font, &p, |_, parts| {
                let path = matching::path_at(parts, shape)?;
                if node >= path.nodes.len() {
                    return Err(format!("path {shape} has no node {node}"));
                }
                path.nodes.rotate_left(node);
                Ok(())
            }),
            None => Ok(()),
        }
    })?;
    let mut sel = none_selected(&t);
    sel.points = vec![(shape, 0)];
    Ok(with_matched(json!({ "selection": sel.to_json() }), plan))
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
    let (count, plans) = s.edit("Reverse Contours", |font| {
        let mut count = 0;
        let mut plans = Vec::new();
        for t in &list {
            let plan = Plan::of(font, &p, t)?;
            let reversed = t.edit_layer(font, |layer| {
                if layer.parts(t.side).shapes.is_empty() {
                    return Ok(0);
                }
                reverse_in(&mut layer.parts_mut(t.side), &t.shapes).map_err(|m| p.bad(m))
            })?;
            count += reversed;
            if let Some(plan) = plan {
                if reversed > 0 {
                    plan.apply(font, &p, |_, parts| reverse_in(parts, &t.shapes).map(|_| ()))?;
                }
                plans.push(plan);
            }
        }
        Ok((count, plans))
    })?;
    let mut out = json!({ "reversed": count });
    if p.bool_or("matchLayers", false)? {
        Plan::report(plans, &mut out);
    }
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
            let path = super::attributes::style_new_path(l, path);
            let shapes = l.parts_mut(t.side).shapes;
            shapes.push(Shape::Path(path));
            Ok(shapes.len() - 1)
        })
    })?;
    t.shapes = vec![index];
    Ok(json!({ "shape": index, "selection": t.to_json() }))
}
