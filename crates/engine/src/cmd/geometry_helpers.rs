//! Geometric helpers (`docs/spec/90-secondary-references.md`): Add Nodes at Intersections, Make
//! Parallel, Space Points Evenly (and by region), Round to Cluster, Make Arc, Insert Node at
//! Coordinate, Name Contour and Slice Along Guide.
//!
//! The geometry is in `fontcraft_pathops::geometry_helpers`; this module finds the nodes, applies
//! the results and rounds the points it creates or moves to the font's grid. Points it does not
//! touch keep their coordinates. Every command names its targets and refuses a locked glyph.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_geom::{CubicBez, Piece, Point, Segment, Vec2, round_point, round_to_grid};
use fontcraft_model::fc_keys::CONTOUR_NAME;
use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Family, LayerSide, MAX_COORDINATE, MAX_NAME_LEN, Node, NodeKind, Path, Shape};
use fontcraft_pathops::geometry_helpers as gh;
use serde_json::{Value, json};

use super::common::{each_layer, grid, in_range, scope};
use super::*;
use crate::params::{GuideScope, Params, TargetKind as K, Targets};

/// Default `within` of Round to Cluster, in units.
const CLUSTER_WITHIN: f64 = 1.0;
/// Default `within` of Space Points with `byRegion`, in units.
const REGION_WITHIN: f64 = 100.0;
/// Largest `within` either command accepts, in units.
const MAX_WITHIN: f64 = 10_000.0;
/// How far past the outlines the knife line of Slice Along Guide starts and ends, in units.
const SLICE_MARGIN: f64 = 10.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.findIntersections",
            "Add Nodes at Intersections",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", shapes?: [index] (default every path)} or {glyphs: [names], layer?: id | layers?: [ids]}. Inserts a node on each path wherever another path, or another segment of the same path, crosses or touches it, without changing the outline or removing anything. Locked paths are crossed but get no nodes. New nodes are rounded to the grid; a crossing that rounds onto an existing node adds nothing. With `glyphs`, a locked glyph or a layer where nothing crosses is listed in `skipped` → {glyphs, skipped, added: count, selection?: {glyph, layer, points} (one glyph: the new nodes)}",
            has_doc,
            find_intersections
        )
        .targets(&[K::Glyphs, K::Layer, K::Shapes]),
        cmd!(
            "path.makeParallel",
            "Make Parallel",
            ["Path", "Align Points"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]] (four on-curve nodes) | segments?: [[shape, node]] (two or four straight segments)}. With two straight segments between the nodes (they share no node), the segment holding the moved node turns about its other end, keeping its length, until it runs parallel to the other. With four straight segments that form a quadrilateral, the moved node goes where the shape becomes a parallelogram. The moved node is the last one in `points`, or the end node of the last segment in `segments`; its handles move with it, and it is rounded to the grid → {moved: [shape, node], selection}",
            has_doc,
            make_parallel
        )
        .targets(&[K::Layer, K::Points, K::Segments]),
        cmd!(
            "path.spacePoints",
            "Space Points Evenly",
            ["Path", "Align Points"],
            None,
            "{glyph: name, layer?: id, side?, points: [[shape, node]], byRegion?: bool = false, within?: units = 100 (0 to 10000)}. Three or more points: along the axis where they spread most, the lowest and highest stay and the others go to equal steps between them, in their order. byRegion: points whose axis coordinates are at most `within` apart form a region, and each region moves as one so that the centres of the regions are equally spaced (three regions or more). One on-curve node: it moves along the line through its two on-curve neighbours to the middle between them, keeping its distance from that line. An on-curve node's handles move with it unless they are listed too; moved points are rounded to the grid → {axis?: \"x\" | \"y\", selection}",
            has_doc,
            space_points
        )
        .targets(&[K::Layer, K::Points]),
        cmd!(
            "path.roundToCluster",
            "Round to Cluster",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]] (default every node of the unlocked paths)} or {glyphs: [names], layer?: id | layers?: [ids]}, within?: units = 1 (0 to 10000). For x and then y on their own: coordinates in increasing order form a cluster while they are at most `within` above the cluster's lowest one, and every coordinate of a cluster takes the value most of them already have (a tie goes to the one nearest their mean, then the lower). Handles are coordinates like any other. With `glyphs`, a locked glyph or a layer with nothing to change is listed in `skipped` → {glyphs, skipped, moved: count}",
            has_doc,
            round_to_cluster
        )
        .targets(&[K::Glyphs, K::Layer, K::Points]),
        cmd!(
            "path.makeArc",
            "Make Arc",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, segments: [[shape, node]]}. Each segment becomes an elliptical arc between its two nodes that leaves the first along the direction the path arrives there and reaches the second along the direction the path leaves there (at an open end without a neighbouring segment, the segment's own direction). Where those two tangent lines meet ahead of the nodes, the arc is a quarter of the ellipse they frame (a quarter circle when they meet square and at equal distances); where the directions are opposite, it is half an ellipse with a new node in the middle. Any other pair of directions has no such arc and refuses. The result is cubic, rounded to the grid; both nodes become smooth where a neighbouring segment gave the direction → {selection: {glyph, layer, shapes}}",
            has_doc,
            make_arc
        )
        .targets(&[K::Layer, K::Segments]),
        cmd!(
            "path.insertNodeAt",
            "Insert Node at Coordinate…",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, segment: [shape, node], x?: number | y?: number (exactly one)}. Inserts a node, without changing the outline, at every point where the segment crosses the vertical line at x or the horizontal line at y. New nodes are rounded to the grid. A segment that never crosses the line, or meets it only at its end nodes, refuses → {nodes: [[shape, node]], selection}",
            has_doc,
            insert_node_at
        )
        .targets(&[K::Layer, K::Segments]),
        cmd!(
            "path.nameContour",
            "Name Contour…",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, path: index, name: string (at most 255 bytes, no control characters; empty removes the name)}. Stores a name on the path, kept in the source as the path's `com.fontcraft.contourName` value; it stays with the path when paths are reordered → {name: string | null, selection: {glyph, layer, shapes}}",
            has_doc,
            name_contour
        )
        .targets(&[K::Layer]),
        cmd!(
            "guide.sliceContours",
            "Slice Along Guide",
            [],
            None,
            "{glyph: name, layer?: id, side?, guide: {scope: \"local\" | \"global\", index} (or guides: [one such guide]), addPointsOnly?: bool = false}. Cuts the paths of the layer along the guide's whole line, exactly as path.cutWithLine does with a line that reaches past every path. addPointsOnly: only inserts nodes where the line crosses the paths, without changing the outline. New nodes are rounded to the grid → {selection} (the nodes made on the line)",
            has_doc,
            slice_contours
        )
        .targets(&[K::Layer, K::Guides]),
    ]
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

/// Whether the segment's node kinds match its handles, so that rebuilding it from its pieces
/// keeps its shape: a line with no handles, a cubic with two, a quadratic with at least one.
fn well_formed(path: &Path, seg: &Segment) -> bool {
    match path.nodes.get(seg.end).map(|n| n.kind) {
        Some(NodeKind::Line) => seg.handles.is_empty(),
        Some(NodeKind::Curve) => seg.handles.len() == 2,
        Some(NodeKind::QCurve) => true,
        _ => false,
    }
}

fn off(p: Point, step: f64) -> Node {
    let p = round_point(p, step);
    Node::new(p.x, p.y, NodeKind::OffCurve)
}

/// The nodes that draw `piece` after its start: its handles and its end, the end as `kind` of
/// the piece.
fn piece_nodes(piece: &Piece, step: f64) -> (Vec<Node>, Point, NodeKind) {
    match piece {
        Piece::Line(l) => (Vec::new(), l.p1, NodeKind::Line),
        Piece::Quad(q) => (vec![off(q.p1, step)], q.p2, NodeKind::QCurve),
        Piece::Cubic(c) => (vec![off(c.p1, step), off(c.p2, step)], c.p3, NodeKind::Curve),
    }
}

/// What replaces one segment's handles: the nodes that go before its end node (each with whether
/// it is a new node to report), and the end node's new kind.
struct Replacement {
    before: Vec<(Node, bool)>,
    end_kind: NodeKind,
}

/// `path` with the segments in `out` (by end node) redrawn: their handles go and the new nodes
/// go before their end node, or at the end of the list for the segment that ends at a closed
/// path's first node (it is drawn from there). Returns the indices of the reported new nodes.
fn rebuild(path: &Path, out: &BTreeMap<usize, (Segment, Replacement)>) -> (Path, Vec<usize>) {
    let skip: BTreeSet<usize> = out.values().flat_map(|(s, _)| s.handles.iter().copied()).collect();
    let mut nodes = Vec::with_capacity(path.nodes.len() + out.values().map(|(_, r)| r.before.len()).sum::<usize>());
    let mut fresh = Vec::new();
    let mut tail: Vec<(Node, bool)> = Vec::new();
    for (i, node) in path.nodes.iter().enumerate() {
        if skip.contains(&i) {
            continue;
        }
        let mut n = node.clone();
        if let Some((seg, r)) = out.get(&i) {
            if seg.end <= seg.start {
                tail.extend(r.before.iter().cloned());
            } else {
                for (b, new) in &r.before {
                    if *new {
                        fresh.push(nodes.len());
                    }
                    nodes.push(b.clone());
                }
            }
            n.kind = r.end_kind;
        }
        nodes.push(n);
    }
    for (b, new) in tail {
        if new {
            fresh.push(nodes.len());
        }
        nodes.push(b);
    }
    let mut p = path.clone();
    p.nodes = nodes;
    (p, fresh)
}

/// `path` with nodes inserted at the segment parameters in `cuts` (by segment end node), without
/// changing its shape. A cut that rounds onto an end node or onto the cut before it is dropped,
/// and so is every cut on a malformed segment. Returns the new path and the new nodes' indices.
fn split_path(path: &Path, cuts: &BTreeMap<usize, Vec<f64>>, step: f64) -> (Path, Vec<usize>) {
    let mut out = BTreeMap::new();
    for seg in path.segments() {
        let Some(ts) = cuts.get(&seg.end) else { continue };
        if !well_formed(path, &seg) {
            continue;
        }
        let (Some(a), Some(b)) = (path.nodes.get(seg.start), path.nodes.get(seg.end)) else { continue };
        let (ra, rb) = (round_point(a.point(), step), round_point(b.point(), step));
        let mut sorted = ts.clone();
        sorted.sort_by(f64::total_cmp);
        let mut kept = Vec::new();
        let mut last = ra;
        for t in sorted {
            let Some(q) = seg.eval(t).map(|p| round_point(p, step)) else { continue };
            if q == last || q == rb {
                continue;
            }
            kept.push(t);
            last = q;
        }
        if kept.is_empty() {
            continue;
        }
        let pieces = gh::split_pieces(&seg.pieces, &kept);
        let count = pieces.len();
        let mut before = Vec::new();
        let mut end_kind = NodeKind::Line;
        for (k, (piece, cut)) in pieces.iter().enumerate() {
            let (handles, end, kind) = piece_nodes(piece, step);
            before.extend(handles.into_iter().map(|h| (h, false)));
            if k + 1 == count {
                end_kind = kind;
            } else {
                let next_is_line = pieces.get(k + 1).is_some_and(|(p, _)| matches!(p, Piece::Line(_)));
                let p = round_point(end, step);
                let mut node = Node::new(p.x, p.y, kind);
                node.smooth = !(matches!(piece, Piece::Line(_)) && next_is_line);
                before.push((node, *cut));
            }
        }
        out.insert(seg.end, (seg, Replacement { before, end_kind }));
    }
    if out.is_empty() {
        return (path.clone(), Vec::new());
    }
    rebuild(path, &out)
}

/// The index before and after `i` along the path (wrapping when it is closed).
fn neighbours(path: &Path, i: usize) -> (Option<usize>, Option<usize>) {
    let n = path.nodes.len();
    let prev = if i > 0 {
        Some(i - 1)
    } else if path.closed && n > 1 {
        Some(n - 1)
    } else {
        None
    };
    let next = if i + 1 < n {
        Some(i + 1)
    } else if path.closed && n > 1 {
        Some(0)
    } else {
        None
    };
    (prev, next)
}

/// Moves node `i` by `d`; an on-curve node takes its handles along, except those in `listed`.
fn move_node(path: &mut Path, i: usize, d: Vec2, listed: &BTreeSet<usize>) {
    if d == Vec2::ZERO {
        return;
    }
    let on = path.nodes.get(i).is_some_and(Node::on_curve);
    let (prev, next) = neighbours(path, i);
    let mut targets = vec![i];
    if on {
        for j in [prev, next].into_iter().flatten() {
            if !listed.contains(&j) && path.nodes.get(j).is_some_and(|n| !n.on_curve()) && j != i {
                targets.push(j);
            }
        }
    }
    targets.dedup();
    for j in targets {
        if let Some(n) = path.nodes.get_mut(j) {
            n.x += d.x;
            n.y += d.y;
        }
    }
}

/// `within`: a distance from 0 to [`MAX_WITHIN`].
fn within(p: &Params, default: f64) -> Result<f64> {
    let v = p.opt_f64("within")?.unwrap_or(default);
    if !(v.is_finite() && (0.0..=MAX_WITHIN).contains(&v)) {
        return Err(p.bad(format!("`within` must be a distance from 0 to {MAX_WITHIN} units")));
    }
    Ok(v)
}

/// The paths of one side, by shape index.
fn paths(shapes: &[Shape]) -> impl Iterator<Item = (usize, &Path)> {
    shapes.iter().enumerate().filter_map(|(i, s)| match s {
        Shape::Path(p) => Some((i, p)),
        _ => None,
    })
}

fn group_by_shape(list: &[(usize, usize)]) -> BTreeMap<usize, Vec<usize>> {
    let mut out: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(s, n) in list {
        out.entry(s).or_default().push(n);
    }
    out
}

fn selection_points(t: &Targets, layer: &str, points: Vec<(usize, usize)>) -> Value {
    let mut sel = Targets::new("selection", t.glyph.clone(), Some(layer.to_string()));
    sel.side = t.side;
    sel.points = points;
    sel.to_json()
}

fn selection_shapes(t: &Targets, layer: &str, shapes: Vec<usize>) -> Value {
    let mut sel = Targets::new("selection", t.glyph.clone(), Some(layer.to_string()));
    sel.side = t.side;
    sel.shapes = shapes;
    sel.to_json()
}

// ── Add Nodes at Intersections ──────────────────────────────────────────────────────────────────

/// Adds nodes where the chosen paths cross, taking the comparisons from `work`; returns the new
/// nodes' addresses.
fn add_intersections(
    shapes: &mut [Shape],
    chosen: Option<&BTreeSet<usize>>,
    step: f64,
    work: &mut usize,
) -> std::result::Result<Vec<(usize, usize)>, String> {
    let mut owners = Vec::new();
    let mut pieces = Vec::new();
    for (i, path) in paths(shapes) {
        if chosen.is_some_and(|c| !c.contains(&i)) {
            continue;
        }
        for seg in path.segments() {
            owners.push((i, seg.end, !path.info.locked));
            pieces.push(seg.pieces);
        }
    }
    let crossings = gh::segment_crossings_within(&pieces, work)?;
    let mut cuts: BTreeMap<usize, BTreeMap<usize, Vec<f64>>> = BTreeMap::new();
    for ((shape, end, open), ts) in owners.into_iter().zip(crossings) {
        if open && !ts.is_empty() {
            cuts.entry(shape).or_default().entry(end).or_default().extend(ts);
        }
    }
    let mut added = Vec::new();
    for (shape, per_segment) in cuts {
        let Some(Shape::Path(path)) = shapes.get_mut(shape) else { continue };
        let (new, fresh) = split_path(path, &per_segment, step);
        if !fresh.is_empty() {
            *path = new;
            added.extend(fresh.into_iter().map(|n| (shape, n)));
        }
    }
    Ok(added)
}

fn find_intersections(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.findIntersections", v)?;
    let step = grid(s.font()?);
    if p.get("glyphs").is_some() {
        let sc = scope(&p, s.font()?)?;
        let mut count = 0;
        // One budget for the whole command, however many layers it covers.
        let mut work = gh::MAX_CROSSING_WORK;
        let out = s.edit("Add Nodes at Intersections", |font| {
            each_layer(font, &sc, p.command(), |_, layer| {
                let shapes = layer.parts_mut(LayerSide::Foreground).shapes;
                match add_intersections(shapes, None, step, &mut work) {
                    Err(why) => Ok(Some(why)),
                    Ok(added) if added.is_empty() => Ok(Some("no paths cross".into())),
                    Ok(added) => {
                        count += added.len();
                        Ok(None)
                    }
                }
            })
        })?;
        let mut r = out.to_json();
        if let Some(o) = r.as_object_mut() {
            o.insert("added".into(), json!(count));
        }
        return Ok(r);
    }
    let t = Targets::parse(&p)?;
    if !(t.points.is_empty() && t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `shapes` as its only target list"));
    }
    let layer = t.layer_id(s.font()?)?;
    let chosen: Option<BTreeSet<usize>> = (!t.shapes.is_empty()).then(|| t.shapes.iter().copied().collect());
    let mut work = gh::MAX_CROSSING_WORK;
    let added = s.edit("Add Nodes at Intersections", |font| {
        t.edit_layer(font, |l| {
            if t.side == LayerSide::Background && l.info.background.is_none() {
                return Ok(Vec::new());
            }
            add_intersections(l.parts_mut(t.side).shapes, chosen.as_ref(), step, &mut work).map_err(|m| p.bad(m))
        })
    })?;
    let glyphs: Vec<&str> = if added.is_empty() { Vec::new() } else { vec![t.glyph.as_str()] };
    Ok(json!({"glyphs": glyphs, "skipped": [], "added": added.len(), "selection": selection_points(&t, &layer, added)}))
}

// ── Make Parallel ───────────────────────────────────────────────────────────────────────────────

fn make_parallel(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.makeParallel", v)?;
    let t = Targets::parse(&p)?;
    let step = grid(s.font()?);
    let layer = t.layer_id(s.font()?)?;
    let (target, to) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        let at = |(sh, n): (usize, usize)| r.parts.node(sh, n).filter(|n| n.on_curve()).map(Node::point);
        // The nodes, the straight segments between them as (shape, start, end), and the moved node.
        type Plan = (Vec<(usize, usize)>, Vec<(usize, usize, usize)>, (usize, usize));
        let (points, lines, moved): Plan = match (t.points.len(), t.segments.len()) {
            (4, 0) => {
                let set: BTreeSet<(usize, usize)> = t.points.iter().copied().collect();
                let mut lines = Vec::new();
                for (sh, path) in paths(r.parts.shapes) {
                    if !set.iter().any(|(s2, _)| *s2 == sh) {
                        continue;
                    }
                    for seg in path.segments() {
                        if seg.is_line() && set.contains(&(sh, seg.start)) && set.contains(&(sh, seg.end)) {
                            lines.push((sh, seg.start, seg.end));
                        }
                    }
                }
                let moved = *t.points.last().ok_or_else(|| p.bad("give four `points`"))?;
                (t.points.clone(), lines, moved)
            }
            (0, 2) | (0, 4) => {
                let mut lines = Vec::new();
                let mut points = Vec::new();
                for &(sh, end) in &t.segments {
                    let seg = r
                        .parts
                        .path(sh)
                        .and_then(|path| path.segment_ending_at(end))
                        .ok_or_else(|| p.bad(format!("segment [{sh}, {end}] does not resolve")))?;
                    if !seg.is_line() {
                        return Err(p.bad(format!("segment [{sh}, {end}] is not straight")));
                    }
                    lines.push((sh, seg.start, seg.end));
                    for n in [seg.start, seg.end] {
                        if !points.contains(&(sh, n)) {
                            points.push((sh, n));
                        }
                    }
                }
                let moved = *t.segments.last().ok_or_else(|| p.bad("give two or four `segments`"))?;
                (points, lines, moved)
            }
            _ => return Err(p.bad("give four on-curve `points`, or two or four straight `segments`")),
        };
        if points.len() != 4 {
            return Err(p.bad("the segments must have four different nodes between them"));
        }
        let pos = |a: (usize, usize)| at(a).ok_or_else(|| p.bad(format!("[{}, {}] is not an on-curve node", a.0, a.1)));
        let to = match lines.len() {
            2 => {
                let (mine, other): (Vec<_>, Vec<_>) = lines.iter().partition(|(sh, a, b)| (*sh, *a) == moved || (*sh, *b) == moved);
                let (Some(&(sh, a, b)), Some(&(osh, oa, ob))) = (mine.first(), other.first()) else {
                    return Err(p.bad("the moved node must end one of the two straight segments, and the other segment must not touch it"));
                };
                let fixed = if (sh, a) == moved { (sh, b) } else { (sh, a) };
                let reference = pos((osh, ob))? - pos((osh, oa))?;
                gh::parallel_end(pos(fixed)?, pos(moved)?, reference).ok_or_else(|| p.bad("a segment has no length"))?
            }
            4 => {
                // A quadrilateral: every node ends exactly two of the sides.
                let ends = |n: (usize, usize)| lines.iter().filter(|(sh, a, b)| (*sh, *a) == n || (*sh, *b) == n).count();
                if points.iter().any(|n| ends(*n) != 2) {
                    return Err(p.bad("the four straight segments do not form a quadrilateral"));
                }
                let adjacent: Vec<(usize, usize)> = lines
                    .iter()
                    .filter_map(|&(sh, a, b)| {
                        if (sh, a) == moved {
                            Some((sh, b))
                        } else if (sh, b) == moved {
                            Some((sh, a))
                        } else {
                            None
                        }
                    })
                    .collect();
                let (Some(&a), Some(&c)) = (adjacent.first(), adjacent.get(1)) else {
                    return Err(p.bad("the moved node must be a corner of the quadrilateral"));
                };
                let opposite = points
                    .iter()
                    .copied()
                    .find(|n| *n != moved && *n != a && *n != c)
                    .ok_or_else(|| p.bad("the quadrilateral has no opposite corner"))?;
                gh::parallelogram_corner(pos(a)?, pos(opposite)?, pos(c)?)
            }
            n => {
                return Err(
                    p.bad(format!("found {n} straight segments between the nodes: give two separate ones or the four sides of a quadrilateral"))
                );
            }
        };
        (moved, round_point(to, step))
    };
    if !(in_range(to.x) && in_range(to.y)) {
        return Err(p.bad(format!("the node would move beyond ±{MAX_COORDINATE}")));
    }
    s.edit("Make Parallel", |font| {
        t.edit_layer(font, |l| {
            let mut parts = l.parts_mut(t.side);
            let path = parts.path_mut(target.0).ok_or_else(|| p.bad("the moved node's path does not resolve"))?;
            let from = path.nodes.get(target.1).map(Node::point).ok_or_else(|| p.bad("the moved node does not resolve"))?;
            move_node(path, target.1, to - from, &BTreeSet::new());
            Ok(())
        })
    })?;
    Ok(json!({"moved": [target.0, target.1], "selection": selection_points(&t, &layer, vec![target])}))
}

// ── Space Points ────────────────────────────────────────────────────────────────────────────────

fn space_points(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.spacePoints", v)?;
    let t = Targets::parse(&p)?;
    let by_region = p.bool_or("byRegion", false)?;
    let within = within(&p, REGION_WITHIN)?;
    let step = grid(s.font()?);
    let layer = t.layer_id(s.font()?)?;
    // The new position of each point.
    type Moves<'a> = (Vec<((usize, usize), Point)>, Option<&'a str>);
    let (moves, axis): Moves = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        let at = |(sh, n): (usize, usize)| r.parts.node(sh, n).map(Node::point).ok_or_else(|| p.bad(format!("[{sh}, {n}] is not a node")));
        match t.points.as_slice() {
            [] => return Err(p.bad("give `points`: one on-curve node, or three or more points")),
            [(sh, n)] => {
                let path = r.parts.path(*sh).ok_or_else(|| p.bad("the point is not on a path"))?;
                if !path.nodes.get(*n).is_some_and(Node::on_curve) {
                    return Err(p.bad("a single point must be an on-curve node"));
                }
                let on_side = |forward: bool| {
                    let mut i = *n;
                    for _ in 0..path.nodes.len() {
                        let (prev, next) = neighbours(path, i);
                        i = if forward { next? } else { prev? };
                        if i == *n {
                            return None;
                        }
                        if path.nodes.get(i).is_some_and(Node::on_curve) {
                            return path.nodes.get(i).map(Node::point);
                        }
                    }
                    None
                };
                let (Some(prev), Some(next)) = (on_side(false), on_side(true)) else {
                    return Err(p.bad("the node needs an on-curve neighbour on each side"));
                };
                let to = gh::centre_between(prev, at((*sh, *n))?, next).ok_or_else(|| p.bad("the node's two neighbours are at the same place"))?;
                (vec![((*sh, *n), round_point(to, step))], None)
            }
            [_, _] => return Err(p.bad("give one on-curve node, or three or more points")),
            list => {
                let pts: Vec<Point> = list.iter().map(|a| at(*a)).collect::<Result<_>>()?;
                let spread = |f: fn(&Point) -> f64| {
                    let vs = pts.iter().map(f);
                    vs.clone().fold(f64::NEG_INFINITY, f64::max) - vs.fold(f64::INFINITY, f64::min)
                };
                let horizontal = spread(|q| q.x) >= spread(|q| q.y);
                let values: Vec<f64> = pts.iter().map(|q| if horizontal { q.x } else { q.y }).collect();
                let spaced = if by_region {
                    gh::space_regions(&values, within).ok_or_else(|| p.bad("the points form fewer than three regions"))?
                } else {
                    gh::space_evenly(&values)
                };
                let moves = list
                    .iter()
                    .zip(pts)
                    .zip(spaced)
                    .map(|((a, q), v)| {
                        let v = round_to_grid(v, step);
                        (*a, if horizontal { Point::new(v, q.y) } else { Point::new(q.x, v) })
                    })
                    .collect();
                (moves, Some(if horizontal { "x" } else { "y" }))
            }
        }
    };
    if moves.iter().any(|(_, q)| !(in_range(q.x) && in_range(q.y))) {
        return Err(p.bad(format!("a point would move beyond ±{MAX_COORDINATE}")));
    }
    let listed = group_by_shape(&t.points);
    s.edit("Space Points", |font| {
        t.edit_layer(font, |l| {
            let mut parts = l.parts_mut(t.side);
            for ((sh, n), to) in &moves {
                let path = parts.path_mut(*sh).ok_or_else(|| p.bad("a point's path does not resolve"))?;
                let from = path.nodes.get(*n).map(Node::point).ok_or_else(|| p.bad("a point does not resolve"))?;
                let mine: BTreeSet<usize> = listed.get(sh).map(|v| v.iter().copied().collect()).unwrap_or_default();
                move_node(path, *n, *to - from, &mine);
            }
            Ok(())
        })
    })?;
    let mut out = json!({"selection": selection_points(&t, &layer, t.points.clone())});
    if let (Some(axis), Some(o)) = (axis, out.as_object_mut()) {
        o.insert("axis".into(), json!(axis));
    }
    Ok(out)
}

// ── Round to Cluster ────────────────────────────────────────────────────────────────────────────

/// Round to Cluster on the listed nodes of `shapes` (every node of the unlocked paths when
/// `only` is `None`); returns how many coordinates changed.
fn cluster(shapes: &mut [Shape], only: Option<&BTreeMap<usize, Vec<usize>>>, within: f64) -> usize {
    let mut at: Vec<(usize, usize)> = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        let Shape::Path(path) = s else { continue };
        match only {
            Some(map) => at.extend(map.get(&i).into_iter().flatten().filter(|n| **n < path.nodes.len()).map(|n| (i, *n))),
            None if !path.info.locked => at.extend((0..path.nodes.len()).map(|n| (i, n))),
            None => {}
        }
    }
    let get = |shapes: &[Shape], (s, n): (usize, usize)| match shapes.get(s) {
        Some(Shape::Path(p)) => p.nodes.get(n).map(Node::point),
        _ => None,
    };
    let pts: Vec<Point> = at.iter().filter_map(|a| get(shapes, *a)).collect();
    if pts.len() != at.len() {
        return 0;
    }
    let xs = gh::cluster_values(&pts.iter().map(|q| q.x).collect::<Vec<_>>(), within);
    let ys = gh::cluster_values(&pts.iter().map(|q| q.y).collect::<Vec<_>>(), within);
    let mut changed = 0;
    for (((s, n), x), y) in at.into_iter().zip(xs).zip(ys) {
        if let Some(Shape::Path(path)) = shapes.get_mut(s)
            && let Some(node) = path.nodes.get_mut(n)
        {
            changed += usize::from(node.x != x) + usize::from(node.y != y);
            node.x = x;
            node.y = y;
        }
    }
    changed
}

fn round_to_cluster(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.roundToCluster", v)?;
    let within = within(&p, CLUSTER_WITHIN)?;
    if p.get("glyphs").is_some() {
        let sc = scope(&p, s.font()?)?;
        let mut count = 0;
        let out = s.edit("Round to Cluster", |font| {
            each_layer(font, &sc, p.command(), |_, layer| {
                let changed = cluster(layer.parts_mut(LayerSide::Foreground).shapes, None, within);
                count += changed;
                Ok((changed == 0).then(|| "no coordinates are close together".into()))
            })
        })?;
        let mut r = out.to_json();
        if let Some(o) = r.as_object_mut() {
            o.insert("moved".into(), json!(count));
        }
        return Ok(r);
    }
    let t = Targets::parse(&p)?;
    if !(t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `points` as its only target list"));
    }
    let only = (!t.points.is_empty()).then(|| group_by_shape(&t.points));
    let changed = s.edit("Round to Cluster", |font| {
        t.edit_layer(font, |l| {
            if t.side == LayerSide::Background && l.info.background.is_none() {
                return Ok(0);
            }
            Ok(cluster(l.parts_mut(t.side).shapes, only.as_ref(), within))
        })
    })?;
    let glyphs: Vec<&str> = if changed == 0 { Vec::new() } else { vec![t.glyph.as_str()] };
    Ok(json!({"glyphs": glyphs, "skipped": [], "moved": changed}))
}

// ── Make Arc ────────────────────────────────────────────────────────────────────────────────────

/// The direction of travel at the end of a piece (`end`) or at its start.
fn tangent(piece: &Piece, end: bool) -> Option<Vec2> {
    for h in [1e-4, 1e-2, 0.5] {
        let d = if end { piece.eval(1.0) - piece.eval(1.0 - h) } else { piece.eval(h) - piece.eval(0.0) };
        if d.hypot() > 1e-9 && d.hypot().is_finite() {
            return Some(d);
        }
    }
    None
}

/// The arc for the segment that ends at `end`, as the nodes before its end node; and whether a
/// neighbouring segment gave the direction at the start and at the end.
fn arc_nodes(path: &Path, end: usize, step: f64) -> std::result::Result<(Segment, Vec<Node>, bool, bool), String> {
    let segs = path.segments();
    let seg = segs.iter().find(|s| s.end == end).cloned().ok_or_else(|| format!("node {end} does not end a segment"))?;
    let prev = segs.iter().find(|s| s.end == seg.start && s.end != seg.end);
    let next = segs.iter().find(|s| s.start == seg.end && s.end != seg.end);
    let (p0, p1) = match (path.nodes.get(seg.start), path.nodes.get(seg.end)) {
        (Some(a), Some(b)) => (a.point(), b.point()),
        _ => return Err("the segment does not resolve".into()),
    };
    let own_start = seg.pieces.first().and_then(|p| tangent(p, false));
    let own_end = seg.pieces.last().and_then(|p| tangent(p, true));
    let from_prev = prev.and_then(|s| s.pieces.last()).and_then(|p| tangent(p, true));
    let from_next = next.and_then(|s| s.pieces.first()).and_then(|p| tangent(p, false));
    let d0 = from_prev.or(own_start).ok_or("the direction at the segment's start is not defined")?;
    let d1 = from_next.or(own_end).ok_or("the direction at the segment's end is not defined")?;
    let arcs: Vec<CubicBez> = gh::elliptical_arc(p0, d0, p1, d1)
        .ok_or_else(|| format!("no elliptical arc leaves node {} and reaches node {end} along the path's directions there", seg.start))?;
    if arcs.iter().any(|c| [c.p1, c.p2, c.p3].iter().any(|q| !(in_range(q.x) && in_range(q.y)))) {
        return Err(format!("the arc would reach beyond ±{MAX_COORDINATE}"));
    }
    let mut nodes = Vec::new();
    let count = arcs.len();
    for (k, c) in arcs.iter().enumerate() {
        nodes.push(off(c.p1, step));
        nodes.push(off(c.p2, step));
        if k + 1 < count {
            let q = round_point(c.p3, step);
            let mut n = Node::new(q.x, q.y, NodeKind::Curve);
            n.smooth = true;
            nodes.push(n);
        }
    }
    Ok((seg, nodes, from_prev.is_some(), from_next.is_some()))
}

fn make_arc(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.makeArc", v)?;
    let t = Targets::parse(&p)?;
    if t.segments.is_empty() {
        return Err(p.bad("give `segments`: the segments to turn into arcs"));
    }
    let step = grid(s.font()?);
    let layer = t.layer_id(s.font()?)?;
    let by_shape = group_by_shape(&t.segments);
    s.edit("Make Arc", |font| {
        t.edit_layer(font, |l| {
            let mut parts = l.parts_mut(t.side);
            for (sh, ends) in &by_shape {
                let path = parts.path_mut(*sh).ok_or_else(|| p.bad(format!("shape {sh} is not a path")))?;
                let mut ends = ends.clone();
                // From the last segment back, so that the end nodes still to come keep their index.
                ends.sort_unstable_by(|a, b| b.cmp(a));
                for end in ends {
                    let (seg, before, smooth_start, smooth_end) =
                        arc_nodes(path, end, step).map_err(|m| p.bad(format!("segment [{sh}, {end}]: {m}")))?;
                    // Where the two nodes are after the rebuild: the handles before them are gone, and
                    // the new nodes go before the end node unless the segment wraps to the tail.
                    let wraps = seg.end <= seg.start;
                    let shift = |i: usize, added: usize| {
                        let kept = i.saturating_sub(seg.handles.iter().filter(|h| **h < i).count());
                        if !wraps && i >= seg.end { kept + added } else { kept }
                    };
                    let (start, end_at) = (shift(seg.start, before.len()), shift(seg.end, before.len()));
                    let mut out = BTreeMap::new();
                    out.insert(seg.end, (seg, Replacement { before: before.into_iter().map(|n| (n, false)).collect(), end_kind: NodeKind::Curve }));
                    let (new, _) = rebuild(path, &out);
                    *path = new;
                    for (i, smooth) in [(start, smooth_start), (end_at, smooth_end)] {
                        if smooth && let Some(n) = path.nodes.get_mut(i) {
                            n.smooth = true;
                        }
                    }
                }
            }
            Ok(())
        })
    })?;
    Ok(json!({"selection": selection_shapes(&t, &layer, by_shape.keys().copied().collect())}))
}

// ── Insert Node at Coordinate ───────────────────────────────────────────────────────────────────

fn insert_node_at(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.insertNodeAt", v)?;
    let t = Targets::parse(&p)?;
    let &[(shape, end)] = t.segments.as_slice() else {
        return Err(p.bad("give one `segment`: [shape, node], the node that ends it"));
    };
    let (vertical, value) = match (p.opt_f64("x")?, p.opt_f64("y")?) {
        (Some(x), None) => (true, x),
        (None, Some(y)) => (false, y),
        _ => return Err(p.bad("give exactly one of `x` and `y`")),
    };
    if !in_range(value) {
        return Err(p.bad(format!("the coordinate must be within ±{MAX_COORDINATE}")));
    }
    let what = format!("{} = {value}", if vertical { "x" } else { "y" });
    let step = grid(s.font()?);
    let layer = t.layer_id(s.font()?)?;
    let added = s.edit("Insert Node at Coordinate", |font| {
        t.edit_layer(font, |l| {
            let mut parts = l.parts_mut(t.side);
            let path = parts.path_mut(shape).ok_or_else(|| p.bad("`segment` does not resolve"))?;
            let seg = path.segment_ending_at(end).ok_or_else(|| p.bad("`segment` does not resolve"))?;
            let ts = gh::axis_crossings(&seg.pieces, vertical, value);
            if ts.is_empty() {
                return Err(p.bad(format!("the segment does not reach {what}")));
            }
            let (new, fresh) = split_path(path, &BTreeMap::from([(end, ts)]), step);
            if fresh.is_empty() {
                return Err(p.bad(format!("the segment meets {what} only at its end nodes")));
            }
            *path = new;
            Ok(fresh.into_iter().map(|n| (shape, n)).collect::<Vec<_>>())
        })
    })?;
    let nodes: Vec<Value> = added.iter().map(|(s, n)| json!([s, n])).collect();
    Ok(json!({"nodes": nodes, "selection": selection_points(&t, &layer, added)}))
}

// ── Name Contour ────────────────────────────────────────────────────────────────────────────────

fn name_contour(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.nameContour", v)?;
    let t = Targets::parse(&p)?;
    let shape = p.get("path").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad("`path` must be a shape index"))?;
    let name = p.str("name")?;
    if name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`name` is longer than {MAX_NAME_LEN} bytes")));
    }
    if name.chars().any(char::is_control) {
        return Err(p.bad("`name` contains a control character"));
    }
    let layer = t.layer_id(s.font()?)?;
    s.edit("Name Contour", |font| {
        t.edit_layer(font, |l| {
            let mut parts = l.parts_mut(t.side);
            let path = parts.path_mut(shape).ok_or_else(|| p.bad(format!("shape {shape} is not a path")))?;
            if name.is_empty() {
                path.extra.remove(Family::Native, CONTOUR_NAME);
            } else {
                path.extra.insert(Family::Native, CONTOUR_NAME, Plist::String(Str::from(name)));
            }
            Ok(())
        })
    })?;
    let name = if name.is_empty() { Value::Null } else { json!(name) };
    Ok(json!({"name": name, "selection": selection_shapes(&t, &layer, vec![shape])}))
}

// ── Slice Along Guide ───────────────────────────────────────────────────────────────────────────

fn slice_contours(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.sliceContours", v)?;
    let t = Targets::parse(&p)?;
    // The shell passes the selected guide as a one-entry `guides` list.
    let g = match (p.get("guide"), t.guides.as_slice()) {
        (Some(_), []) => super::guide::guide_ref(&p, "guide")?,
        (None, [one]) => *one,
        _ => return Err(p.bad("give one guide: `guide`, or `guides` with one entry")),
    };
    if !(t.points.is_empty() && t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes a guide, no other target lists"));
    }
    let points_only = p.bool_or("addPointsOnly", false)?;
    let step = grid(s.font()?);
    let layer = t.layer_id(s.font()?)?;
    let (origin, dir, bounds) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        let list = match g.scope {
            GuideScope::Local => r.parts.guides,
            GuideScope::Global => r.master.guides.as_slice(),
        };
        let guide = list.get(g.index).ok_or_else(|| p.bad(format!("{} guide {} does not resolve", g.scope.as_str(), g.index)))?;
        let a = guide.angle.to_radians();
        if !(guide.x.is_finite() && guide.y.is_finite() && a.is_finite()) {
            return Err(p.bad("the guide's position or angle is not a number"));
        }
        let bounds = fontcraft_geom::bounds(paths(r.parts.shapes).flat_map(|(_, path)| path.nodes.iter().map(Node::point)));
        // cos 90° is 6e-17, not 0: keep a vertical or horizontal guide exactly so.
        let exact = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
        (Point::new(guide.x, guide.y), Vec2::new(exact(a.cos()), exact(a.sin())), bounds)
    };
    let Some(bounds) = bounds else {
        return Ok(json!({ "selection": selection_points(&t, &layer, Vec::new()) }));
    };
    if !points_only {
        let corners = [bounds.origin(), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x0, bounds.y1), Point::new(bounds.x1, bounds.y1)];
        let along = corners.map(|c| (c - origin).dot(dir));
        let lo = along.iter().copied().fold(f64::INFINITY, f64::min) - SLICE_MARGIN;
        let hi = along.iter().copied().fold(f64::NEG_INFINITY, f64::max) + SLICE_MARGIN;
        let (from, to) = (origin + dir * lo, origin + dir * hi);
        if ![from.x, from.y, to.x, to.y].into_iter().all(in_range) {
            return Err(p.bad("the guide lies too far from the outlines"));
        }
        let mut knife = json!({"glyph": t.glyph, "from": [from.x, from.y], "to": [to.x, to.y]});
        if let Some(o) = knife.as_object_mut() {
            o.insert("layer".into(), json!(layer));
            if t.side == LayerSide::Background {
                o.insert("side".into(), json!(t.side.as_str()));
            }
        }
        return s.execute("path.cutWithLine", &knife);
    }
    let added = s.edit("Slice Along Guide", |font| {
        t.edit_layer(font, |l| {
            let shapes = l.parts_mut(t.side).shapes;
            let mut added = Vec::new();
            for (i, shape) in shapes.iter_mut().enumerate() {
                let Shape::Path(path) = shape else { continue };
                let cuts: BTreeMap<usize, Vec<f64>> = path
                    .segments()
                    .into_iter()
                    .map(|seg| (seg.end, gh::line_crossings(&seg.pieces, origin, dir)))
                    .filter(|(_, ts)| !ts.is_empty())
                    .collect();
                if cuts.is_empty() {
                    continue;
                }
                let (new, fresh) = split_path(path, &cuts, step);
                if !fresh.is_empty() {
                    *path = new;
                    added.extend(fresh.into_iter().map(|n| (i, n)));
                }
            }
            Ok(added)
        })
    })?;
    Ok(json!({ "selection": selection_points(&t, &layer, added) }))
}

#[cfg(test)]
mod tests {
    use fontcraft_testkit::{oval, rect};

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn add(s: &mut Session, glyph: &str, nodes: Value, closed: bool) -> usize {
        run(s, "path.addPath", json!({ "glyph": glyph, "nodes": nodes, "closed": closed }))["shape"].as_u64().unwrap() as usize
    }

    fn line(points: &[(f64, f64)]) -> Value {
        Value::Array(points.iter().map(|(x, y)| json!({ "x": x, "y": y })).collect())
    }

    fn path(s: &Session, glyph: &str, shape: usize) -> Path {
        let font = s.font().unwrap();
        let id = font.default_master().unwrap().id.clone();
        let layer = font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == id).unwrap();
        match &layer.shapes[shape] {
            Shape::Path(p) => p.clone(),
            other => panic!("not a path: {other:?}"),
        }
    }

    fn xy(p: &Path) -> Vec<(f64, f64)> {
        p.nodes.iter().map(|n| (n.x, n.y)).collect()
    }

    #[test]
    fn find_intersections_adds_a_node_on_each_path_where_they_cross() {
        let mut s = Session::with_document();
        add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
        add(&mut s, "A", rect(50.0, 50.0, 150.0, 150.0), true);
        let before = (path(&s, "A", 0), path(&s, "A", 1));
        let r = run(&mut s, "path.findIntersections", json!({"glyph": "A"}));
        assert_eq!(r["added"], json!(4), "{r}");
        let (a, b) = (path(&s, "A", 0), path(&s, "A", 1));
        assert_eq!(a.nodes.len(), 6);
        assert_eq!(b.nodes.len(), 6);
        assert!(xy(&a).contains(&(100.0, 50.0)) && xy(&a).contains(&(50.0, 100.0)), "{:?}", xy(&a));
        assert!(xy(&b).contains(&(100.0, 50.0)) && xy(&b).contains(&(50.0, 100.0)), "{:?}", xy(&b));
        // The outline is the same: every old node is still there.
        assert!(xy(&before.0).iter().all(|q| xy(&a).contains(q)) && xy(&before.1).iter().all(|q| xy(&b).contains(q)));
        // Running it again finds only the nodes already there.
        let r = run(&mut s, "path.findIntersections", json!({"glyph": "A"}));
        assert_eq!(r["added"], json!(0));
        let r = run(&mut s, "path.findIntersections", json!({"glyphs": ["A"]}));
        assert_eq!(r["skipped"][0]["reason"], json!("no paths cross"));
    }

    /// The comparison budget is one per command: layers that each fit it alone stop fitting
    /// once the layers before them have used it.
    #[test]
    fn find_intersections_on_many_layers_shares_one_work_budget() {
        // Nearly parallel arches: every pair's boxes overlap and each pair needs subdivision.
        let arches: Vec<Shape> = (0..80)
            .map(|i| {
                let y = f64::from(i) * 0.01;
                let node = |x: f64, y: f64, kind| Node::new(x, y, kind);
                Shape::Path(Path::new(
                    false,
                    vec![
                        node(0.0, y, NodeKind::Line),
                        node(300.0, y + 600.0, NodeKind::OffCurve),
                        node(700.0, y + 600.0, NodeKind::OffCurve),
                        node(1000.0, y, NodeKind::Curve),
                    ],
                ))
            })
            .collect();
        let mut used = gh::MAX_CROSSING_WORK;
        add_intersections(&mut arches.clone(), None, 1.0, &mut used).unwrap();
        let used = gh::MAX_CROSSING_WORK - used;
        assert!(used > 0 && used < gh::MAX_CROSSING_WORK / 2, "one layer takes {used}");
        let layers = gh::MAX_CROSSING_WORK / used + 1;
        let names: Vec<String> = (0..layers).map(|i| format!("arches{i}")).collect();
        let mut s = Session::with_document();
        s.edit("fixture", |font| {
            let id = font.default_master()?.id.clone();
            for name in &names {
                let mut layer = fontcraft_model::Layer::new(&id, 1000.0);
                layer.shapes = arches.clone();
                let mut g = fontcraft_model::Glyph::new(name);
                g.layers.push(layer);
                font.glyphs.push(g);
            }
            Ok(())
        })
        .unwrap();
        let r = run(&mut s, "path.findIntersections", json!({"glyphs": names}));
        let reasons: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|k| k["reason"].as_str().unwrap()).collect();
        assert_eq!(reasons.len(), layers, "{r}");
        assert_eq!(reasons.first(), Some(&"no paths cross"), "the first layer fits the budget");
        assert!(reasons.last().unwrap().contains("work limit"), "{r}");
    }

    #[test]
    fn make_parallel_turns_the_last_segment_and_makes_a_parallelogram() {
        let mut s = Session::with_document();
        add(&mut s, "A", line(&[(0.0, 0.0), (100.0, 0.0)]), false);
        add(&mut s, "A", line(&[(0.0, 50.0), (100.0, 80.0)]), false);
        let r = run(&mut s, "path.makeParallel", json!({"glyph": "A", "segments": [[0, 1], [1, 1]]}));
        assert_eq!(r["moved"], json!([1, 1]));
        // Same length (√(100² + 30²) ≈ 104.4), now horizontal, rounded to the grid.
        assert_eq!(xy(&path(&s, "A", 1)), vec![(0.0, 50.0), (104.0, 50.0)]);
        assert_eq!(xy(&path(&s, "A", 0)), vec![(0.0, 0.0), (100.0, 0.0)], "the other segment stays");

        add(&mut s, "B", line(&[(0.0, 0.0), (100.0, 0.0), (120.0, 90.0), (10.0, 100.0)]), true);
        run(&mut s, "path.makeParallel", json!({"glyph": "B", "points": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
        assert_eq!(xy(&path(&s, "B", 0)), vec![(0.0, 0.0), (100.0, 0.0), (120.0, 90.0), (20.0, 90.0)]);

        // Two segments on one node, and curves, are refused.
        assert!(s.execute("path.makeParallel", &json!({"glyph": "B", "segments": [[0, 1], [0, 2]]})).is_err());
        add(&mut s, "O", oval(100.0, 100.0, 50.0, 50.0), true);
        assert!(s.execute("path.makeParallel", &json!({"glyph": "O", "segments": [[0, 0], [0, 3]]})).is_err());
    }

    #[test]
    fn space_points_evenly_by_axis_by_region_and_one_node_between_its_neighbours() {
        let mut s = Session::with_document();
        add(&mut s, "A", line(&[(0.0, 0.0), (10.0, 2.0), (30.0, 4.0), (90.0, 5.0)]), false);
        let r = run(&mut s, "path.spacePoints", json!({"glyph": "A", "points": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
        assert_eq!(r["axis"], json!("x"));
        assert_eq!(xy(&path(&s, "A", 0)), vec![(0.0, 0.0), (30.0, 2.0), (60.0, 4.0), (90.0, 5.0)]);

        add(&mut s, "B", line(&[(0.0, 0.0), (40.0, 30.0), (100.0, 0.0)]), false);
        run(&mut s, "path.spacePoints", json!({"glyph": "B", "points": [[0, 1]]}));
        assert_eq!(xy(&path(&s, "B", 0)), vec![(0.0, 0.0), (50.0, 30.0), (100.0, 0.0)]);

        // Three stems' edges; the middle stem moves as one to the centre.
        add(&mut s, "m", line(&[(50.0, 0.0), (130.0, 0.0), (260.0, 0.0), (340.0, 0.0), (550.0, 0.0), (630.0, 0.0)]), false);
        let all: Vec<Value> = (0..6).map(|n| json!([0, n])).collect();
        run(&mut s, "path.spacePoints", json!({"glyph": "m", "points": all, "byRegion": true}));
        let xs: Vec<f64> = xy(&path(&s, "m", 0)).iter().map(|q| q.0).collect();
        assert_eq!(xs, vec![50.0, 130.0, 300.0, 380.0, 550.0, 630.0]);

        assert!(s.execute("path.spacePoints", &json!({"glyph": "B", "points": [[0, 0], [0, 1]]})).is_err());
        assert!(s.execute("path.spacePoints", &json!({"glyph": "B", "points": [[0, 0]]})).is_err(), "an open end has one neighbour");
    }

    #[test]
    fn round_to_cluster_aligns_nearly_equal_coordinates() {
        let mut s = Session::with_document();
        add(&mut s, "A", line(&[(0.0, 0.0), (100.0, 1.0), (101.0, 100.0), (0.0, 99.0)]), true);
        let r = run(&mut s, "path.roundToCluster", json!({"glyph": "A"}));
        assert_eq!(r["moved"], json!(3));
        assert_eq!(xy(&path(&s, "A", 0)), vec![(0.0, 0.0), (100.0, 0.0), (100.0, 99.0), (0.0, 99.0)]);
        // With a smaller distance nothing is close enough.
        add(&mut s, "B", line(&[(0.0, 0.0), (100.0, 1.0), (101.0, 100.0)]), true);
        let r = run(&mut s, "path.roundToCluster", json!({"glyph": "B", "within": 0.5}));
        assert_eq!(r["moved"], json!(0));
    }

    #[test]
    fn make_arc_draws_a_quarter_circle_between_square_tangents() {
        let mut s = Session::with_document();
        add(&mut s, "A", line(&[(-100.0, 0.0), (0.0, 0.0), (100.0, 100.0), (100.0, 200.0)]), false);
        run(&mut s, "path.makeArc", json!({"glyph": "A", "segments": [[0, 2]]}));
        let p = path(&s, "A", 0);
        assert_eq!(xy(&p), vec![(-100.0, 0.0), (0.0, 0.0), (55.0, 0.0), (100.0, 45.0), (100.0, 100.0), (100.0, 200.0)]);
        assert_eq!(p.nodes[4].kind, NodeKind::Curve);
        assert!(p.nodes[1].smooth && p.nodes[4].smooth);
        let seg = p.segment_ending_at(4).unwrap();
        let mid = seg.eval(0.5).unwrap();
        assert!((mid.distance(Point::new(0.0, 100.0)) - 100.0).abs() < 0.5, "{mid:?}");

        // Directions that run the same way have no arc.
        add(&mut s, "B", line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 10.0), (300.0, 10.0)]), false);
        assert!(s.execute("path.makeArc", &json!({"glyph": "B", "segments": [[0, 2]]})).is_err());
    }

    #[test]
    fn insert_node_at_puts_a_node_where_the_segment_crosses_the_coordinate() {
        let mut s = Session::with_document();
        add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
        let r = run(&mut s, "path.insertNodeAt", json!({"glyph": "A", "segment": [0, 1], "x": 30}));
        assert_eq!(r["nodes"], json!([[0, 1]]));
        assert_eq!(xy(&path(&s, "A", 0)), vec![(0.0, 0.0), (30.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        assert!(s.execute("path.insertNodeAt", &json!({"glyph": "A", "segment": [0, 2], "x": 500})).is_err());
        assert!(s.execute("path.insertNodeAt", &json!({"glyph": "A", "segment": [0, 2], "x": 1, "y": 1})).is_err());

        // A curve: the node lands on the curve at that height, and the outline keeps its shape.
        add(&mut s, "O", oval(100.0, 100.0, 100.0, 100.0), true);
        let before = path(&s, "O", 0);
        let r = run(&mut s, "path.insertNodeAt", json!({"glyph": "O", "segment": [0, 3], "y": 150}));
        let n = r["nodes"][0][1].as_u64().unwrap() as usize;
        let after = path(&s, "O", 0);
        assert_eq!(after.nodes.len(), before.nodes.len() + 3);
        assert_eq!(after.nodes[n].y, 150.0);
        assert!(after.nodes[n].smooth);
    }

    #[test]
    fn name_contour_stores_and_removes_the_name() {
        let mut s = Session::with_document();
        add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
        run(&mut s, "path.nameContour", json!({"glyph": "A", "path": 0, "name": "bowl"}));
        assert_eq!(path(&s, "A", 0).extra.get(Family::Native, CONTOUR_NAME).and_then(|v| v.as_str()), Some("bowl"));
        assert!(s.execute("path.nameContour", &json!({"glyph": "A", "path": 1, "name": "x"})).is_err());
        assert!(s.execute("path.nameContour", &json!({"glyph": "A", "path": 0, "name": "a\u{7}"})).is_err());
        run(&mut s, "path.nameContour", json!({"glyph": "A", "path": 0, "name": ""}));
        assert!(path(&s, "A", 0).extra.get(Family::Native, CONTOUR_NAME).is_none());
    }

    #[test]
    fn slice_along_guide_equals_the_knife_on_the_same_line() {
        let setup = || {
            let mut s = Session::with_document();
            add(&mut s, "A", rect(0.0, 0.0, 100.0, 100.0), true);
            add(&mut s, "A", rect(20.0, 20.0, 80.0, 80.0), true);
            run(&mut s, "guide.add", json!({"glyph": "A", "x": 50, "y": 0, "angle": 90}));
            s
        };
        let mut sliced = setup();
        run(&mut sliced, "guide.sliceContours", json!({"glyph": "A", "guide": {"scope": "local", "index": 0}}));
        let mut knifed = setup();
        run(&mut knifed, "path.cutWithLine", json!({"glyph": "A", "from": [50, -10], "to": [50, 110]}));
        let shapes = |s: &Session| {
            let font = s.font().unwrap();
            font.glyph("A").unwrap().layers[0].shapes.clone()
        };
        assert_eq!(shapes(&sliced), shapes(&knifed));
        assert_ne!(shapes(&sliced), shapes(&setup()));

        let mut points = setup();
        let r = run(&mut points, "guide.sliceContours", json!({"glyph": "A", "guide": {"scope": "local", "index": 0}, "addPointsOnly": true}));
        assert_eq!(r["selection"]["points"].as_array().map(Vec::len), Some(4));
        assert_eq!(path(&points, "A", 0).nodes.len(), 6);
        assert!(xy(&path(&points, "A", 1)).contains(&(50.0, 20.0)));
    }
}
