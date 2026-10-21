//! The clean-up `audit.checkOutlines` runs on each layer its fix changed: every point rounded
//! (to integers, or to three decimals with `decimal`), segments shorter than one unit merged into
//! the point before them, and the original contour order and start points restored unless
//! `keepContourOrder` is false.
//!
//! Ported from AFDKO 5.0.1 (Apache License 2.0, Copyright 2015 Adobe): `run`,
//! `restore_contour_order`, `sort_contours` and `set_max_p` in `checkoutlinesufo.py`, and
//! `thresholdAttrGlyph` in `ufotools.py`, which draws through fontPens' `ThresholdPen`
//! (BSD 3-Clause, Copyright 2005-2026 Erik van Blokland, Tal Leming, Just van Rossum) and
//! fontTools' point-to-segment conversion.

use std::collections::HashMap;

use fontcraft_model::{Layer, Node, NodeKind, Path, Shape};

/// A segment whose end lies closer than this to the point before it is dropped.
const MERGE_DISTANCE: f64 = 1.0;

/// Cleans up `layer` after the fix; `original` is the same layer before it. Only paths take
/// part: components keep their places in the shape list. Returns how many on-curve points were
/// merged away.
pub(super) fn clean_up(layer: &mut Layer, original: Option<&Layer>, decimal: bool, keep_order: bool) -> usize {
    let slots: Vec<usize> = layer.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_))).map(|(i, _)| i).collect();
    let mut paths: Vec<Path> = layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    let mut merged = 0;
    for path in &mut paths {
        for node in &mut path.nodes {
            node.x = round(node.x, decimal);
            node.y = round(node.y, decimal);
        }
        merged += merge_short_segments(path);
    }
    if keep_order && let Some(original) = original {
        let old: Vec<&Path> = original
            .shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) => Some(p),
                _ => None,
            })
            .collect();
        paths = restore_order(paths, &old);
    }
    let mut paths = paths.into_iter();
    for &slot in &slots {
        if let (Some(shape), Some(path)) = (layer.shapes.get_mut(slot), paths.next()) {
            *shape = Shape::Path(path);
        }
    }
    merged
}

/// The reference's rounding: half to even, to an integer or to three decimals. Negative zero
/// becomes zero, as the reference's `int(round(x))` gives.
fn round(v: f64, decimal: bool) -> f64 {
    if !v.is_finite() {
        return v;
    }
    let r = if decimal {
        // Formatting rounds the exact binary value half to even, as the reference's round(x, 3).
        format!("{v:.3}").parse::<f64>().unwrap_or(v)
    } else {
        v.round_ties_even()
    };
    r + 0.0
}

/// Drops each segment whose end point lies closer than [`MERGE_DISTANCE`] to the end of the last
/// segment kept, as the reference's threshold pen does. A closed path is drawn from its first
/// on-curve node; its closing segment is dropped only when it is a curve, since a straight
/// closing segment is implied. A closed path made only of off-curve nodes is left alone.
/// Returns the number of on-curve nodes removed.
fn merge_short_segments(path: &mut Path) -> usize {
    let Some(first_on) = path.nodes.iter().position(Node::on_curve) else { return 0 };
    if !path.closed && first_on != 0 {
        return 0;
    }
    let mut nodes = path.nodes.clone();
    nodes.rotate_left(first_on);
    let mut rest = nodes.into_iter();
    let Some(start) = rest.next() else { return 0 };
    let mut out = vec![start.clone()];
    let mut last = (start.x, start.y);
    let mut pending: Vec<Node> = Vec::new();
    let mut removed = 0;
    for node in rest {
        if !node.on_curve() {
            pending.push(node);
            continue;
        }
        if distance(last, (node.x, node.y)) >= MERGE_DISTANCE {
            last = (node.x, node.y);
            out.append(&mut pending);
            out.push(node);
        } else {
            pending.clear();
            removed += 1;
        }
    }
    if !path.closed {
        out.append(&mut pending);
    } else {
        // `pending` holds the off-curve nodes of the closing segment, which ends at `start`. A
        // straight closing segment is implied and never dropped.
        if start.kind == NodeKind::Line || distance(last, (start.x, start.y)) >= MERGE_DISTANCE {
            out.append(&mut pending);
        } else {
            pending.clear();
            if let Some(first) = out.first_mut() {
                first.kind = NodeKind::Line;
            }
        }
        // A last node on the start point merges with it, as the point pen does on closing.
        if out.len() > 1
            && let Some(end) = out.last()
            && end.on_curve()
            && out.first().is_some_and(|f| f.x == end.x && f.y == end.y)
            && let Some(end) = out.pop()
        {
            removed += 1;
            let tail = out.iter().rposition(Node::on_curve).map_or(0, |i| i + 1);
            let offs: Vec<Node> = out.drain(tail..).collect();
            if let Some(first) = out.first_mut() {
                *first = end;
            }
            out.extend(offs);
        }
    }
    path.nodes = out;
    removed
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The reference's top point of a contour, the key its sort and its second matching pass use.
/// It starts from the first node and compares the x of the best point with the y of the next on
/// a tie, as the reference does; kept so the order matches its output.
fn max_point(path: &Path) -> (f64, f64) {
    let Some(first) = path.nodes.first() else { return (0.0, 0.0) };
    let mut max = (first.x, first.y);
    for n in path.nodes.iter().filter(|n| n.on_curve()) {
        if max.1 < n.y || (max.1 == n.y && max.0 < n.y) {
            max = (n.x, n.y);
        }
    }
    max
}

/// Puts the fixed contours back into the order of the original ones. The new contours are first
/// sorted by their top point and node count. Then each is matched to the first original contour
/// with as many nodes, or else to the first original contour with an on-curve node on its top
/// point (and then starts where that contour started, when it has a node there); the rest go to
/// the end. Ties keep the sorted order.
fn restore_order(paths: Vec<Path>, original: &[&Path]) -> Vec<Path> {
    let mut new: Vec<(Path, (f64, f64))> = paths
        .into_iter()
        .map(|p| {
            let m = max_point(&p);
            (p, m)
        })
        .collect();
    new.sort_by(|(a, am), (b, bm)| am.1.total_cmp(&bm.1).then(am.0.total_cmp(&bm.0)).then(a.nodes.len().cmp(&b.nodes.len())));
    // The first original contour with each node count, and the first with an on-curve node at
    // each point, looked up in constant time so a layer of many contours stays linear.
    let mut by_count: HashMap<usize, usize> = HashMap::new();
    let mut by_point: HashMap<(u64, u64), usize> = HashMap::new();
    for (j, o) in original.iter().enumerate() {
        by_count.entry(o.nodes.len()).or_insert(j);
        for n in o.nodes.iter().filter(|n| n.on_curve()) {
            if let Some(key) = point_key(n.x, n.y) {
                by_point.entry(key).or_insert(j);
            }
        }
    }
    let mut order: Vec<(usize, usize)> = Vec::with_capacity(new.len());
    let mut matched = vec![false; new.len()];
    for (i, (path, _)) in new.iter().enumerate() {
        if let Some(&j) = by_count.get(&path.nodes.len()) {
            order.push((j, i));
            if let Some(m) = matched.get_mut(i) {
                *m = true;
            }
        }
    }
    for (i, (path, max)) in new.iter_mut().enumerate() {
        if matched.get(i).copied().unwrap_or(true) {
            continue;
        }
        let Some(&j) = point_key(max.0, max.1).and_then(|key| by_point.get(&key)) else { continue };
        order.push((j, i));
        if let Some(m) = matched.get_mut(i) {
            *m = true;
        }
        let (Some(old_start), Some(new_start)) = (original.get(j).and_then(|o| o.nodes.first()), path.nodes.first()) else { continue };
        if path.closed
            && !(old_start.x == new_start.x && old_start.y == new_start.y)
            && let Some(at) = path.nodes.iter().position(|n| n.on_curve() && n.x == old_start.x && n.y == old_start.y)
        {
            path.nodes.rotate_left(at);
        }
    }
    let end = new.len();
    for (i, done) in matched.iter().enumerate() {
        if !done {
            order.push((end, i));
        }
    }
    order.sort_unstable();
    let mut slots: Vec<Option<Path>> = new.into_iter().map(|(p, _)| Some(p)).collect();
    order.into_iter().filter_map(|(_, i)| slots.get_mut(i).and_then(Option::take)).collect()
}

/// A point as a map key that matches as `==` on its coordinates does: zero and negative zero are
/// one key, and a point with a NaN coordinate has none.
fn point_key(x: f64, y: f64) -> Option<(u64, u64)> {
    (!x.is_nan() && !y.is_nan()).then(|| ((x + 0.0).to_bits(), (y + 0.0).to_bits()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(points: &[(f64, f64)]) -> Path {
        Path::new(true, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect())
    }

    fn xy(path: &Path) -> Vec<(f64, f64)> {
        path.nodes.iter().map(|n| (n.x, n.y)).collect()
    }

    /// Rounding is half to even, short segments merge into the point before them, and the
    /// contour order follows the reference's matching (checked against AFDKO 5.0.1's
    /// checkoutlinesufo on the same outlines).
    #[test]
    fn rounds_merges_and_restores_the_reference_order() {
        assert_eq!(round(60.5, false), 60.0);
        assert_eq!(round(61.5, false), 62.0);
        assert_eq!(round(-0.4, false).to_bits(), 0.0f64.to_bits());
        assert_eq!(round(300.2504, true), 300.25);

        let mut p = closed(&[(0.0, 0.0), (100.0, 0.0), (100.4, 0.5), (100.0, 100.0), (0.0, 100.0)]);
        assert_eq!(merge_short_segments(&mut p), 1);
        assert_eq!(xy(&p), [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);

        let top = closed(&[(400.0, 600.0), (500.0, 600.0), (500.0, 701.0), (400.0, 701.0)]);
        let low = closed(&[(10.0, 10.0), (60.0, 10.0), (60.0, 60.0), (10.0, 60.0)]);
        let a = closed(&[(100.0, 100.0), (300.0, 100.0), (300.0, 300.0), (100.0, 300.0)]);
        let b = closed(&[(201.0, 200.0), (350.0, 200.0), (350.0, 400.0), (201.0, 400.0)]);
        let union =
            closed(&[(100.0, 100.0), (300.0, 100.0), (300.0, 200.0), (350.0, 200.0), (350.0, 400.0), (201.0, 400.0), (201.0, 300.0), (100.0, 300.0)]);
        let original = [&top, &a, &b, &low];
        let fixed = restore_order(vec![top.clone(), union.clone(), low.clone()], &original);
        assert_eq!(fixed.iter().map(xy).collect::<Vec<_>>(), [xy(&low), xy(&top), xy(&union)]);
    }

    /// Contours whose node count matches no original contour fall back to the top-point match;
    /// on a layer of many of them that lookup stays linear (it compared every pair of contours
    /// once, which hung on a crafted layer).
    #[test]
    fn many_contours_are_matched_in_linear_time() {
        const N: usize = 60_000;
        let original: Vec<Path> = (0..N)
            .map(|k| {
                let x = k as f64 * 10.0;
                closed(&[(x, 0.0), (x + 5.0, 0.0), (x + 5.0, 5.0), (x, 5.0)])
            })
            .collect();
        let fixed: Vec<Path> = (0..N)
            .rev()
            .map(|k| {
                let x = k as f64 * 10.0;
                closed(&[(x, 0.0), (x + 2.0, 0.0), (x + 5.0, 0.0), (x + 5.0, 5.0), (x, 5.0)])
            })
            .collect();
        let refs: Vec<&Path> = original.iter().collect();
        let out = restore_order(fixed, &refs);
        assert_eq!(out.len(), N);
        for (k, path) in out.iter().enumerate() {
            assert_eq!(path.nodes.first().map(|n| n.x), Some(k as f64 * 10.0), "contour {k}");
        }
    }
}
