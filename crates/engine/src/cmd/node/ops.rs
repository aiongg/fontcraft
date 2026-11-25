//! Operations on one path's node list for the node and segment commands. Each reads a path and
//! returns the new path (or paths), or a message saying why it cannot; none panics on a
//! malformed list. Points an operation creates or moves are rounded to the grid step it is given.

use std::collections::BTreeSet;

use fontcraft_geom::{Piece, Point, Segment, Vec2, round_point};
use fontcraft_model::{Node, NodeKind, Path};

use super::refit;

/// Lengths below this are treated as zero.
pub(super) const EPS: f64 = 1e-9;

/// The node before `i` in path order (wrapping round a closed path).
pub(super) fn prev(path: &Path, i: usize) -> Option<usize> {
    let n = path.nodes.len();
    if i >= n {
        None
    } else if i > 0 {
        Some(i - 1)
    } else if path.closed && n > 1 {
        Some(n - 1)
    } else {
        None
    }
}

/// The node after `i` in path order (wrapping round a closed path).
pub(super) fn next(path: &Path, i: usize) -> Option<usize> {
    let n = path.nodes.len();
    if i + 1 < n {
        Some(i + 1)
    } else if i < n && path.closed && n > 1 {
        Some(0)
    } else {
        None
    }
}

pub(super) fn on(path: &Path, i: usize) -> bool {
    path.nodes.get(i).is_some_and(Node::on_curve)
}

pub(super) fn point(path: &Path, i: usize) -> Option<Point> {
    path.nodes.get(i).map(Node::point)
}

/// For an off-curve node: the on-curve node it belongs to, and that node's neighbour on the
/// other side (the opposite handle, or the other end of a straight segment).
pub(super) fn handle_owner(path: &Path, j: usize) -> Option<(usize, Option<usize>)> {
    if on(path, j) {
        return None;
    }
    if let Some(k) = prev(path, j).filter(|&k| on(path, k)) {
        return Some((k, prev(path, k)));
    }
    if let Some(k) = next(path, j).filter(|&k| on(path, k)) {
        return Some((k, next(path, k)));
    }
    None
}

fn with_kind(n: &Node, kind: NodeKind) -> Node {
    let mut c = n.clone();
    c.kind = kind;
    c
}

fn off(p: Point, step: f64) -> Node {
    let r = round_point(p, step);
    Node::new(r.x, r.y, NodeKind::OffCurve)
}

fn on_node(p: Point, kind: NodeKind, step: f64) -> Node {
    let r = round_point(p, step);
    Node::new(r.x, r.y, kind)
}

fn malformed() -> String {
    "its node list is malformed".into()
}

/// The on-curve nodes in path order, from the first one. In a closed path `segs[k]` runs from
/// `ons[k]` to `ons[k + 1]` (the last one back to `ons[0]`); in an open path the same holds
/// without the wrap.
pub(super) fn on_curve_order(path: &Path, segs: &[Segment]) -> Vec<usize> {
    let mut out: Vec<usize> = segs.iter().map(|s| s.start).collect();
    if !path.closed
        && let Some(last) = segs.last()
    {
        out.push(last.end);
    }
    out
}

/// The path drawn the other way round. A closed path keeps its start node.
pub(super) fn reversed(path: &Path) -> Result<Path, String> {
    let segs = path.segments();
    let Some(last) = segs.last() else { return Ok(path.clone()) };
    let node = |i: usize| path.nodes.get(i).ok_or_else(malformed);
    let mut nodes = Vec::with_capacity(path.nodes.len());
    if !path.closed {
        nodes.push(with_kind(node(last.end)?, NodeKind::Line));
    }
    for seg in segs.iter().rev() {
        for &h in seg.handles.iter().rev() {
            nodes.push(node(h)?.clone());
        }
        nodes.push(with_kind(node(seg.start)?, node(seg.end)?.kind));
    }
    if path.closed {
        // The start node was pushed last.
        nodes.rotate_right(1);
    }
    if nodes.len() != path.nodes.len() {
        return Err(malformed());
    }
    let mut out = path.clone();
    out.nodes = nodes;
    Ok(out)
}

/// An open path through consecutive segments of `path`.
fn run_path(path: &Path, run: &[&Segment]) -> Option<Path> {
    let first = run.first()?;
    let mut nodes = vec![with_kind(path.nodes.get(first.start)?, NodeKind::Line)];
    for seg in run {
        for &h in &seg.handles {
            nodes.push(path.nodes.get(h)?.clone());
        }
        nodes.push(path.nodes.get(seg.end)?.clone());
    }
    let mut out = Path::new(false, nodes);
    out.info = path.info.clone();
    if out.info.identifier.is_some() {
        out.info.identifier = None;
    }
    Some(out)
}

/// The segments not marked in `removed`, as runs of consecutive segments in path order. In a
/// closed path a run may go round the start node.
fn kept_runs<'a>(path: &Path, segs: &'a [Segment], removed: &[bool]) -> Vec<Vec<&'a Segment>> {
    let n = segs.len();
    let start = if path.closed {
        match removed.iter().position(|r| *r) {
            Some(r) => r + 1,
            None => return vec![segs.iter().collect()],
        }
    } else {
        0
    };
    let mut runs = Vec::new();
    let mut current = Vec::new();
    for k in 0..n {
        let i = (start + k) % n;
        let (Some(seg), Some(&gone)) = (segs.get(i), removed.get(i)) else { continue };
        if gone {
            if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(seg);
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// `path.deleteAndBreak` on one path: each node of `nodes` goes with the segments on both sides
/// of it, each segment that ends at a node of `segment_ends` goes alone, and what is left
/// becomes open paths. `None` when nothing is removed; an empty list when nothing is left.
pub(super) fn break_path(path: &Path, nodes: &BTreeSet<usize>, segment_ends: &BTreeSet<usize>) -> Option<Vec<Path>> {
    let segs = path.segments();
    if segs.is_empty() {
        return nodes.iter().any(|&i| on(path, i)).then(Vec::new);
    }
    let removed: Vec<bool> = segs.iter().map(|s| segment_ends.contains(&s.end) || nodes.contains(&s.start) || nodes.contains(&s.end)).collect();
    if !removed.iter().any(|r| *r) {
        return None;
    }
    Some(kept_runs(path, &segs, &removed).iter().filter_map(|run| run_path(path, run)).collect())
}

/// Copies of the segments of `path` whose two end nodes are both in `selected`, as open paths
/// (a closed path with every segment selected is copied whole). Each comes with the node of
/// `path` that each of its nodes copies; `None` for a whole copy.
pub(super) fn selected_segments(path: &Path, selected: &BTreeSet<usize>) -> Vec<(Path, Option<Vec<usize>>)> {
    let segs = path.segments();
    let removed: Vec<bool> = segs.iter().map(|s| !(selected.contains(&s.start) && selected.contains(&s.end))).collect();
    if path.closed && !segs.is_empty() && !removed.iter().any(|r| *r) {
        let mut whole = path.clone();
        if whole.info.identifier.is_some() {
            whole.info.identifier = None;
        }
        return vec![(whole, None)];
    }
    if !removed.iter().any(|r| !*r) {
        return Vec::new();
    }
    let sources = |run: &[&Segment]| -> Vec<usize> {
        let start = run.first().map(|s| s.start);
        start.into_iter().chain(run.iter().flat_map(|s| s.handles.iter().copied().chain([s.end]))).collect()
    };
    kept_runs(path, &segs, &removed).iter().filter_map(|run| Some((run_path(path, run)?, Some(sources(run))))).collect()
}

/// `path` with each straight segment that ends at a node of `ends` turned into a curve whose
/// handles lie on the line (at one third and two thirds; one handle at the middle of a quadratic
/// segment), so the outline keeps its shape. With `orthogonal`, see [`orthogonal_handles`]. The
/// curves are quadratic in a path with quadratic segments, and in one without curves when
/// `quadratic` says so. Returns the new path and the new index of every old node.
pub(super) fn curves_from_lines(path: &Path, ends: &BTreeSet<usize>, step: f64, quadratic: bool, orthogonal: bool) -> (Path, Vec<usize>) {
    let has = |kind: NodeKind| path.nodes.iter().any(|n| n.kind == kind);
    let quad = has(NodeKind::QCurve) || (quadratic && !has(NodeKind::Curve));
    let mut nodes = Vec::with_capacity(path.nodes.len() + 2 * ends.len());
    let mut map = Vec::with_capacity(path.nodes.len());
    let mut tail = Vec::new();
    for (i, node) in path.nodes.iter().enumerate() {
        let start = if ends.contains(&i) && node.on_curve() && path.ends_segment(i) {
            prev(path, i).filter(|&s| on(path, s)).and_then(|s| Some((s, point(path, s)?)))
        } else {
            None
        };
        let mut node = node.clone();
        if let Some((k, s)) = start {
            let e = node.point();
            node.kind = if quad { NodeKind::QCurve } else { NodeKind::Curve };
            let turned = if orthogonal { orthogonal_handles(path, k, i, quad) } else { None };
            let handles = match turned {
                Some((h1, h2)) => vec![off(h1, step), off(h2, step)],
                None if quad => vec![off(s.lerp(e, 0.5), step)],
                None => vec![off(s.lerp(e, 1.0 / 3.0), step), off(s.lerp(e, 2.0 / 3.0), step)],
            };
            // The handles of the segment that ends at a closed path's first node go at the end.
            if i == 0 && path.closed {
                tail = handles;
            } else {
                nodes.extend(handles);
            }
        }
        map.push(nodes.len());
        nodes.push(node);
    }
    nodes.extend(tail);
    let mut out = path.clone();
    out.nodes = nodes;
    (out, map)
}

/// The handles that turn the line from node `s` to node `e` into a quarter of an oval, one
/// horizontal and one vertical: the way round that continues the neighbouring segments better,
/// horizontal at the start when both do equally well. `None` for a horizontal or vertical line.
fn orthogonal_handles(path: &Path, s: usize, e: usize, quad: bool) -> Option<(Point, Point)> {
    /// The length of an oval's handles, as a share of its radii: for a cubic quarter, and for a
    /// quadratic quarter of two pieces (the tangent of 22.5 degrees).
    const CUBIC: f64 = 0.552_284_749_830_793_6;
    const QUADRATIC: f64 = std::f64::consts::SQRT_2 - 1.0;
    let (a, b) = (point(path, s)?, point(path, e)?);
    let d = b - a;
    if !(d.x.abs() > EPS && d.y.abs() > EPS && d.x.is_finite() && d.y.is_finite()) {
        return None;
    }
    let unit = |v: Vec2| {
        let l = v.hypot();
        if l > EPS && l.is_finite() { v / l } else { Vec2::ZERO }
    };
    let before = prev(path, s).filter(|&k| k != e).and_then(|k| point(path, k)).map_or(Vec2::ZERO, |p| unit(a - p));
    let after = next(path, e).filter(|&k| k != s).and_then(|k| point(path, k)).map_or(Vec2::ZERO, |p| unit(p - b));
    let (across, up) = (Vec2::new(d.x.signum(), 0.0), Vec2::new(0.0, d.y.signum()));
    let start_horizontal = before.dot(across) + after.dot(up) + EPS >= before.dot(up) + after.dot(across);
    let corner = if start_horizontal { Point::new(b.x, a.y) } else { Point::new(a.x, b.y) };
    let k = if quad { QUADRATIC } else { CUBIC };
    Some((a.lerp(corner, k), b.lerp(corner, k)))
}

/// Turns the two handles of the on-curve node `i` onto one line through it, along the bisector
/// of their directions, keeping their lengths. Handles already in line are left alone.
pub(super) fn align_handles(path: &mut Path, i: usize, step: f64) {
    let (Some(a), Some(b)) = (prev(path, i), next(path, i)) else { return };
    if on(path, a) || on(path, b) {
        return;
    }
    let (Some(p), Some(pa), Some(pb)) = (point(path, i), point(path, a), point(path, b)) else { return };
    let (va, vb) = (pa - p, pb - p);
    let (la, lb) = (va.hypot(), vb.hypot());
    if !(la > EPS && lb > EPS) {
        return;
    }
    let (ua, ub) = (va / la, vb / lb);
    if ua.cross(ub).abs() < 1e-9 && ua.dot(ub) < 0.0 {
        return;
    }
    let mut d = ub - ua;
    if d.hypot() < EPS {
        // Both handles point the same way: use the line between the neighbouring nodes.
        let (Some(pp), Some(nn)) = (prev(path, a).and_then(|k| point(path, k)), next(path, b).and_then(|k| point(path, k))) else { return };
        d = nn - pp;
    }
    let len = d.hypot();
    if !(len > EPS && len.is_finite()) {
        return;
    }
    let d = d / len;
    let (na, nb) = (round_point(p - d * la, step), round_point(p + d * lb, step));
    if let Some(n) = path.nodes.get_mut(a) {
        (n.x, n.y) = (na.x, na.y);
    }
    if let Some(n) = path.nodes.get_mut(b) {
        (n.x, n.y) = (nb.x, nb.y);
    }
}

/// The handle and the line of a tangent: a smooth on-curve node with a line on one side and a
/// handle on the other. Returns the handle and the line's other end. A handle that is also the
/// only handle of a smooth node at its segment's other end belongs to no single tangent.
fn tangent(path: &Path, k: usize) -> Option<(usize, usize)> {
    path.nodes.get(k).filter(|n| n.on_curve() && n.smooth)?;
    let (a, b) = (prev(path, k)?, next(path, k)?);
    let (handle, far, beyond) = match (on(path, a), on(path, b)) {
        (false, true) => (a, b, prev(path, a)),
        (true, false) => (b, a, next(path, b)),
        _ => return None,
    };
    let shared = beyond.filter(|&o| o != k).and_then(|o| path.nodes.get(o)).is_some_and(|n| n.on_curve() && n.smooth);
    (!shared).then_some((handle, far))
}

/// Turns the handle of the tangent `k` about its node onto the node's line, keeping its length.
/// A line or a handle without length has no direction: the handle then stays.
fn straighten_tangent(path: &mut Path, k: usize, step: f64) {
    let Some((handle, far)) = tangent(path, k) else { return };
    let (Some(pk), Some(pf), Some(ph)) = (point(path, k), point(path, far), point(path, handle)) else { return };
    let (line, length) = (pk - pf, (ph - pk).hypot());
    let span = line.hypot();
    if !(span > EPS && length > EPS && span.is_finite() && length.is_finite()) {
        return;
    }
    let to = round_point(pk + line * (length / span), step);
    if let Some(n) = path.nodes.get_mut(handle) {
        (n.x, n.y) = (to.x, to.y);
    }
}

/// After nodes of `path` moved from the places `old`: every tangent whose node, handle and line
/// end did not all move by the same offset has its handle turned onto its line
/// ([`straighten_tangent`]). Handles for which `stays` holds are left where they are.
pub(in crate::cmd) fn keep_tangents(path: &mut Path, old: &[Point], step: f64, stays: impl Fn(usize) -> bool) {
    const SAME: f64 = 1e-6;
    if old.len() != path.nodes.len() {
        return;
    }
    for k in 0..old.len() {
        let Some((handle, far)) = tangent(path, k) else { continue };
        if stays(handle) {
            continue;
        }
        let offset = |i: usize| Some(point(path, i)? - *old.get(i)?);
        let (Some(dk), Some(dh), Some(df)) = (offset(k), offset(handle), offset(far)) else { continue };
        if (dk - dh).hypot() > SAME || (dk - df).hypot() > SAME {
            straighten_tangent(path, k, step);
        }
    }
}

/// `path.toggleNodeType` on one path: switches each on-curve node of `nodes` to `to` (or to the
/// other type). A node turning smooth gets handles on straight sides and its handles lined up,
/// unless `keep_outline`; `quadratic` as for [`curves_from_lines`]. Returns the new path and the new index of every old node.
pub(super) fn toggle(path: &Path, nodes: &BTreeSet<usize>, to: Option<bool>, keep_outline: bool, step: f64, quadratic: bool) -> (Path, Vec<usize>) {
    let mut out = path.clone();
    let mut smoothed = Vec::new();
    for &i in nodes {
        let Some(n) = out.nodes.get_mut(i).filter(|n| n.on_curve()) else { continue };
        let smooth = to.unwrap_or(!n.smooth);
        if smooth == n.smooth {
            continue;
        }
        n.smooth = smooth;
        if smooth {
            smoothed.push(i);
        } else {
            n.tangent = false;
        }
    }
    let identity: Vec<usize> = (0..out.nodes.len()).collect();
    if keep_outline || smoothed.is_empty() {
        return (out, identity);
    }
    let mut ends = BTreeSet::new();
    for &i in &smoothed {
        // An open path's end has one side only: there is nothing to line up.
        let (Some(a), Some(b)) = (prev(&out, i), next(&out, i)) else { continue };
        if on(&out, a) && out.ends_segment(i) {
            ends.insert(i);
        }
        if on(&out, b) && out.ends_segment(b) {
            ends.insert(b);
        }
    }
    let (mut out, map) = curves_from_lines(&out, &ends, step, quadratic, false);
    for &i in &smoothed {
        if let Some(&k) = map.get(i) {
            align_handles(&mut out, k, step);
        }
    }
    (out, map)
}

/// The nodes that draw `pieces`: off-curve nodes and the on-curve node ending each piece.
/// On-curve nodes between pieces are smooth (they were inside one segment).
fn piece_nodes(pieces: &[Piece], step: f64) -> Vec<Node> {
    let mut out = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Line(l) => out.push(on_node(l.p1, NodeKind::Line, step)),
            Piece::Quad(q) => {
                out.push(off(q.p1, step));
                out.push(on_node(q.p2, NodeKind::QCurve, step));
            }
            Piece::Cubic(c) => {
                out.push(off(c.p1, step));
                out.push(off(c.p2, step));
                out.push(on_node(c.p3, NodeKind::Curve, step));
            }
        }
        if k + 1 < pieces.len()
            && !matches!(piece, Piece::Line(_))
            && let Some(last) = out.last_mut()
        {
            last.smooth = true;
        }
    }
    out
}

/// `path.insertNode` on one path: splits the segment ending at node `end` at `t` (strictly
/// between 0 and 1). Returns the new path and the index of the new node.
pub(super) fn insert_node(path: &Path, end: usize, t: f64, step: f64) -> Result<(Path, usize), String> {
    let seg = path.segment_ending_at(end).ok_or_else(|| format!("node {end} does not end a segment"))?;
    let (before, after) = seg.split(t).ok_or_else(|| "the segment cannot be split there".to_string())?;
    let mut mid = piece_nodes(&before, step);
    if let Some(n) = mid.last_mut() {
        n.smooth = !seg.is_line();
    }
    let mut rest = piece_nodes(&after, step);
    let end_kind = rest.pop().map(|n| n.kind).ok_or_else(malformed)?;
    let mut end_node = Some(with_kind(path.nodes.get(seg.end).ok_or_else(malformed)?, end_kind));
    let k = mid.len();
    if k == 0 {
        return Err(malformed());
    }
    let handles: BTreeSet<usize> = seg.handles.iter().copied().collect();
    let wraps = seg.end <= seg.start;
    let mut nodes = Vec::with_capacity(path.nodes.len() + k + rest.len());
    let mut index = None;
    mid.extend(rest);
    let mut fresh = Some(mid);
    for (i, n) in path.nodes.iter().enumerate() {
        if handles.contains(&i) {
            continue;
        }
        if i != seg.end {
            nodes.push(n.clone());
            continue;
        }
        if !wraps && let Some(f) = fresh.take() {
            index = Some(nodes.len() + k - 1);
            nodes.extend(f);
        }
        if let Some(e) = end_node.take() {
            nodes.push(e);
        }
    }
    // The segment that ends at a closed path's first node is drawn from the end of the list.
    if let Some(f) = fresh.take() {
        index = Some(nodes.len() + k - 1);
        nodes.extend(f);
    }
    let index = index.ok_or_else(malformed)?;
    let mut out = path.clone();
    out.nodes = nodes;
    Ok((out, index))
}

/// Whether two open ends are close enough to become one node when connected: within half a
/// grid step.
pub(super) fn coincide(a: Point, b: Point, step: f64) -> bool {
    a.distance(b) <= step.max(0.0) / 2.0 + 1e-6
}

/// Closes an open path. When its two ends coincide they become one node; otherwise a straight
/// segment joins them.
pub(super) fn close(path: &Path, step: f64) -> Result<Path, String> {
    if path.closed {
        return Err("it is already closed".into());
    }
    if path.nodes.iter().filter(|n| n.on_curve()).count() < 2 {
        return Err("it has fewer than two nodes".into());
    }
    let mut out = path.clone();
    out.closed = true;
    let (Some(first), Some(last)) = (path.nodes.first(), path.nodes.last()) else { return Err(malformed()) };
    if coincide(first.point(), last.point(), step) {
        let kind = last.kind;
        out.nodes.pop();
        if let Some(f) = out.nodes.first_mut() {
            f.kind = kind;
        }
    } else if let Some(f) = out.nodes.first_mut() {
        f.kind = NodeKind::Line;
    }
    Ok(out)
}

/// Joins two open paths at the open ends `a_end` of `a` and `b_end` of `b` (each the first or
/// the last node). `a` keeps its direction where it can. Returns the joined path and the
/// indices in it of the joined nodes (one when the ends coincide and become one node).
pub(super) fn join(a: &Path, a_end: usize, b: &Path, b_end: usize, step: f64) -> Result<(Path, Vec<usize>), String> {
    let a = if a_end == 0 && a.nodes.len() > 1 { reversed(a)? } else { a.clone() };
    let b = if b_end + 1 == b.nodes.len() && b.nodes.len() > 1 { reversed(b)? } else { b.clone() };
    let (Some(a_last), Some(b_first)) = (a.nodes.last(), b.nodes.first()) else { return Err(malformed()) };
    let at = a.nodes.len() - 1;
    let merge = coincide(a_last.point(), b_first.point(), step);
    let mut out = a.clone();
    let mut rest = b.nodes.into_iter();
    let joined = if merge {
        rest.next();
        vec![at]
    } else {
        if let Some(first) = rest.next() {
            out.nodes.push(with_kind(&first, NodeKind::Line));
        }
        vec![at, at + 1]
    };
    out.nodes.extend(rest);
    Ok((out, joined))
}

/// `edit.delete` on one path: removes the on-curve nodes of `selected` and refits the segments
/// they joined into one; a curve segment that loses a handle (its end nodes kept) becomes a
/// line. The handle of a smooth node that has a new line on its other side turns onto it. `None`
/// when the path is to be removed: every node selected, or fewer than two on-curve nodes left.
pub(super) fn delete_points(path: &Path, selected: &BTreeSet<usize>, step: f64) -> Result<Option<Path>, String> {
    if (0..path.nodes.len()).all(|i| selected.contains(&i)) {
        return Ok(None);
    }
    let segs = path.segments();
    if segs.is_empty() {
        return Ok((!selected.iter().any(|&i| on(path, i))).then(|| path.clone()));
    }
    let ons = on_curve_order(path, &segs);
    let m = ons.len();
    let kept: Vec<usize> = (0..m).filter(|&k| ons.get(k).is_some_and(|i| !selected.contains(i))).collect();
    if kept.len() < 2 {
        return Ok(None);
    }
    let mut pairs: Vec<(usize, usize)> = kept
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some((*a, *b)),
            _ => None,
        })
        .collect();
    if path.closed
        && let (Some(&last), Some(&first)) = (kept.last(), kept.first())
    {
        pairs.push((last, first + m));
    }
    let mut built: Vec<(Vec<Node>, NodeKind)> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        let run: Vec<&Segment> = (a..b).filter_map(|k| segs.get(k % m)).collect();
        let piece = match run.as_slice() {
            [one] => {
                let end_kind = path.nodes.get(one.end).map(|n| n.kind).ok_or_else(malformed)?;
                if one.handles.iter().any(|h| selected.contains(h)) {
                    (Vec::new(), NodeKind::Line)
                } else {
                    let handles = one.handles.iter().filter_map(|&h| path.nodes.get(h).cloned()).collect();
                    (handles, end_kind)
                }
            }
            _ => {
                let pieces: Vec<Piece> = run.iter().flat_map(|s| s.pieces.iter().copied()).collect();
                if pieces.iter().all(|p| matches!(p, Piece::Line(_))) {
                    (Vec::new(), NodeKind::Line)
                } else {
                    match refit::fit(&pieces) {
                        Some(c) => (vec![off(c.p1, step), off(c.p2, step)], NodeKind::Curve),
                        None => (Vec::new(), NodeKind::Line),
                    }
                }
            }
        };
        built.push(piece);
    }
    let node_at = |pos: usize| ons.get(pos % m).and_then(|&i| path.nodes.get(i)).cloned().ok_or_else(malformed);
    let first = kept.first().copied().ok_or_else(malformed)?;
    let mut first_node = node_at(first)?;
    first_node.kind = match (path.closed, built.last()) {
        (true, Some((_, kind))) => *kind,
        _ => NodeKind::Line,
    };
    let mut nodes = vec![first_node];
    let count = pairs.len();
    // The ends of the segments that became another one: a tangent there has a new line.
    let mut rebuilt = BTreeSet::new();
    for (k, (&(a, b), (handles, kind))) in pairs.iter().zip(built).enumerate() {
        let start = nodes.len().saturating_sub(1);
        let same = b == a + 1 && segs.get(a % m).is_some_and(|s| s.handles.len() == handles.len());
        nodes.extend(handles);
        if path.closed && k + 1 == count {
            // The last segment ends at the first node: its handles close the list.
            if !same {
                rebuilt.extend([start, 0]);
            }
            break;
        }
        nodes.push(with_kind(&node_at(b)?, kind));
        if !same {
            rebuilt.extend([start, nodes.len().saturating_sub(1)]);
        }
    }
    let mut out = path.clone();
    out.nodes = nodes;
    for k in rebuilt {
        straighten_tangent(&mut out, k, step);
    }
    Ok(Some(out))
}

/// The direction a piece leaves its start in, from the first control point that differs from it.
pub(super) fn start_tangent(piece: &Piece) -> Option<Vec2> {
    let pts: Vec<Point> = match piece {
        Piece::Line(l) => vec![l.p0, l.p1],
        Piece::Quad(q) => vec![q.p0, q.p1, q.p2],
        Piece::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    };
    let (&p0, rest) = pts.split_first()?;
    rest.iter().map(|&q| q - p0).find(|v| v.hypot() > EPS).map(|v| v / v.hypot())
}

/// The direction from a piece's end back into it.
pub(super) fn end_tangent(piece: &Piece) -> Option<Vec2> {
    let reversed = match piece {
        Piece::Line(l) => Piece::Line(fontcraft_geom::Line::new(l.p1, l.p0)),
        Piece::Quad(q) => Piece::Quad(fontcraft_geom::QuadBez::new(q.p2, q.p1, q.p0)),
        Piece::Cubic(c) => Piece::Cubic(fontcraft_geom::CubicBez::new(c.p3, c.p2, c.p1, c.p0)),
    };
    start_tangent(&reversed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64, y: f64, kind: NodeKind) -> Node {
        Node::new(x, y, kind)
    }

    fn coords(p: &Path) -> Vec<(f64, f64, NodeKind)> {
        p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect()
    }

    #[test]
    fn reversing_twice_gives_the_path_back() {
        let curve = Path::new(
            true,
            vec![
                n(0.0, 0.0, NodeKind::Line),
                n(100.0, 0.0, NodeKind::Line),
                n(100.0, 50.0, NodeKind::OffCurve),
                n(50.0, 100.0, NodeKind::OffCurve),
                n(0.0, 100.0, NodeKind::Curve),
            ],
        );
        let r = reversed(&curve).unwrap();
        assert_eq!(r.nodes.first().map(|n| (n.x, n.y)), Some((0.0, 0.0)), "the start node stays");
        assert!(r.to_bez().is_ok());
        assert_eq!(coords(&reversed(&r).unwrap()), coords(&curve));
        let open = Path::new(false, curve.nodes.clone());
        let r = reversed(&open).unwrap();
        assert_eq!(r.nodes.first().map(|n| (n.x, n.y, n.kind)), Some((0.0, 100.0, NodeKind::Line)));
        assert!(r.to_bez().is_ok());
        assert_eq!(coords(&reversed(&r).unwrap()), coords(&open));
    }

    #[test]
    fn breaking_a_closed_path_at_a_node_leaves_one_open_path() {
        let square = Path::rect(0.0, 0.0, 100.0, 100.0);
        let pieces = break_path(&square, &BTreeSet::from([1]), &BTreeSet::new()).unwrap();
        assert_eq!(pieces.len(), 1);
        let p = pieces.first().unwrap();
        assert!(!p.closed);
        assert_eq!(p.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>(), [(100.0, 100.0), (0.0, 100.0), (0.0, 0.0)]);
    }
}
