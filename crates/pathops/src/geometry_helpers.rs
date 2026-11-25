//! Intersections, parallel paths, point spacing, cluster rounding, arcs, node insertion, guide
//! slicing: the geometry of the small path helpers (`docs/spec/90-secondary-references.md`).
//!
//! Every function works on plain points and segment pieces ([`Piece`]), so the engine decides
//! which nodes they come from and where results go. Segment parameters follow
//! [`fontcraft_geom::Segment`]: 0 at the start node, 1 at the end node, spread evenly across the
//! pieces of a quadratic segment with several off-curve points.
//!
//! The geometry is our own, written from the user documentation's descriptions of what each
//! command does and from textbook Bézier and conic geometry.

pub use fontcraft_geom::crossings::{
    CROSSING_TOLERANCE, MAX_CROSSING_SEGMENTS, MAX_CROSSING_WORK, MAX_CROSSINGS, MAX_PAIR_WORK, SegmentCrossing, piece_crossings,
    segment_crossing_pairs_within, segment_crossings, segment_crossings_within,
};
use fontcraft_geom::crossings::{SAME_T, line_piece_crossings, piece_hull, subpiece};
use fontcraft_geom::{CubicBez, Line, Piece, Point, Vec2};

/// The cubic handle length, as a share of the tangent run, that draws a quarter circle (and,
/// through an affine map, a quarter ellipse).
pub const QUARTER_ARC_K: f64 = 0.552_284_749_830_793_4;

/// The segment parameters where the segment crosses the endless line through `origin` in
/// direction `dir`, sorted. A piece that lies along the line gives none.
pub fn line_crossings(pieces: &[Piece], origin: Point, dir: Vec2) -> Vec<f64> {
    let len = dir.hypot();
    if !(len.is_finite() && len > 0.0) {
        return Vec::new();
    }
    let d = dir / len;
    let n = pieces.len() as f64;
    let mut out = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        let Some(r) = piece_hull(p) else { continue };
        // A stretch of the line long enough to cross the whole piece.
        let along = [r.origin(), Point::new(r.x1, r.y0), Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)].map(|c| (c - origin).dot(d));
        let lo = along.iter().copied().fold(f64::INFINITY, f64::min) - 1.0;
        let hi = along.iter().copied().fold(f64::NEG_INFINITY, f64::max) + 1.0;
        if !(lo.is_finite() && hi.is_finite()) {
            continue;
        }
        let line = Line::new(origin + d * lo, origin + d * hi);
        out.extend(line_piece_crossings(p, line).into_iter().map(|(t, _)| (i as f64 + t) / n));
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|x, y| (*x - *y).abs() <= SAME_T);
    out
}

/// The segment parameters where the segment crosses the vertical line x = `value` (`vertical`)
/// or the horizontal line y = `value`.
pub fn axis_crossings(pieces: &[Piece], vertical: bool, value: f64) -> Vec<f64> {
    if vertical {
        line_crossings(pieces, Point::new(value, 0.0), Vec2::new(0.0, 1.0))
    } else {
        line_crossings(pieces, Point::new(0.0, value), Vec2::new(1.0, 0.0))
    }
}

/// The pieces of a segment cut at the segment parameters `ts`. Each result piece comes with
/// whether it ends at one of the cuts (as opposed to the segment's end or a point where two of
/// its original pieces meet). Parameters at 0, at 1 or not finite are ignored.
pub fn split_pieces(pieces: &[Piece], ts: &[f64]) -> Vec<(Piece, bool)> {
    let n = pieces.len();
    let mut locals: Vec<Vec<f64>> = vec![Vec::new(); n];
    if n > 0 {
        for &t in ts {
            if !(t.is_finite() && t > SAME_T && t < 1.0 - SAME_T) {
                continue;
            }
            let scaled = t * n as f64;
            let i = (scaled.floor() as usize).min(n - 1);
            let local = scaled - i as f64;
            if local > SAME_T
                && local < 1.0 - SAME_T
                && let Some(list) = locals.get_mut(i)
            {
                list.push(local);
            }
        }
    }
    let mut out = Vec::new();
    for (p, cuts) in pieces.iter().zip(locals.iter_mut()) {
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() <= SAME_T);
        let mut from = 0.0;
        for &c in cuts.iter() {
            out.push((subpiece(p, from, c), true));
            from = c;
        }
        out.push((if from == 0.0 { *p } else { subpiece(p, from, 1.0) }, false));
    }
    out
}

/// Where `moved` goes so that the straight line from `fixed` to it runs parallel to
/// `reference`: the same length, turned to the nearer of the reference's two senses. `None`
/// when the reference or the line has no length.
pub fn parallel_end(fixed: Point, moved: Point, reference: Vec2) -> Option<Point> {
    let r = reference.hypot();
    let v = moved - fixed;
    let len = v.hypot();
    if !(r.is_finite() && r > 0.0 && len.is_finite() && len > 0.0) {
        return None;
    }
    let u = reference / r;
    let sign = if v.dot(u) < 0.0 { -1.0 } else { 1.0 };
    Some(fixed + u * (sign * len))
}

/// The fourth corner that makes `a`, `b`, `c` and it a parallelogram: the corner opposite `b`.
pub fn parallelogram_corner(a: Point, b: Point, c: Point) -> Point {
    a + (c - b)
}

/// New values along one axis: the lowest and highest stay, the others go to equal steps between
/// them in the order they had (ties keep their input order).
pub fn space_evenly(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut out = values.to_vec();
    if n < 3 {
        return out;
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| values.get(*a).unwrap_or(&0.0).total_cmp(values.get(*b).unwrap_or(&0.0)));
    let (Some(lo), Some(hi)) = (order.first().and_then(|i| values.get(*i)), order.last().and_then(|i| values.get(*i))) else { return out };
    let step = (hi - lo) / (n - 1) as f64;
    for (rank, i) in order.iter().enumerate() {
        if let Some(v) = out.get_mut(*i) {
            *v = lo + step * rank as f64;
        }
    }
    out
}

/// Groups of values in increasing order where each value is at most `within` above the one
/// before it; the result lists the indices of each group.
fn regions(values: &[f64], within: f64) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values.get(*a).unwrap_or(&0.0).total_cmp(values.get(*b).unwrap_or(&0.0)));
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for i in order {
        let v = values.get(i).copied().unwrap_or(0.0);
        match out.last_mut() {
            Some(group) if v - last <= within => group.push(i),
            _ => out.push(vec![i]),
        }
        last = v;
    }
    out
}

/// Space Regions: the values fall into regions (each value at most `within` from its neighbour
/// in the region); each region moves as one so that the centres of the regions' extents are
/// equally spaced, the first and last regions staying. Returns the new values, or `None` when
/// there are fewer than three regions.
pub fn space_regions(values: &[f64], within: f64) -> Option<Vec<f64>> {
    let groups = regions(values, within);
    if groups.len() < 3 {
        return None;
    }
    let centre = |g: &Vec<usize>| {
        let vs = g.iter().filter_map(|i| values.get(*i).copied());
        let lo = vs.clone().fold(f64::INFINITY, f64::min);
        let hi = vs.fold(f64::NEG_INFINITY, f64::max);
        (lo + hi) / 2.0
    };
    let centres: Vec<f64> = groups.iter().map(centre).collect();
    let spaced = space_evenly(&centres);
    let mut out = values.to_vec();
    for ((g, before), after) in groups.iter().zip(&centres).zip(&spaced) {
        for i in g {
            if let Some(v) = out.get_mut(*i) {
                *v += after - before;
            }
        }
    }
    Some(out)
}

/// Round to Cluster on one axis: values in increasing order join a cluster while they are at
/// most `within` above its lowest value; every value of a cluster takes the value most of its
/// members already have (a tie goes to the one nearest the cluster's mean, then the lower).
pub fn cluster_values(values: &[f64], within: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values.get(*a).unwrap_or(&0.0).total_cmp(values.get(*b).unwrap_or(&0.0)));
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut start = f64::NEG_INFINITY;
    for i in order {
        let v = values.get(i).copied().unwrap_or(0.0);
        match clusters.last_mut() {
            Some(c) if v - start <= within => c.push(i),
            _ => {
                clusters.push(vec![i]);
                start = v;
            }
        }
    }
    let mut out = values.to_vec();
    for c in clusters.iter().filter(|c| c.len() > 1) {
        let vs: Vec<f64> = c.iter().filter_map(|i| values.get(*i).copied()).collect();
        let mean = vs.iter().sum::<f64>() / vs.len() as f64;
        // Members are sorted, so equal values sit together.
        let mut best: Option<(usize, f64)> = None;
        let mut k = 0;
        while let Some(&v) = vs.get(k) {
            let run = vs.iter().skip(k).take_while(|w| **w == v).count().max(1);
            let better = match best {
                None => true,
                Some((count, b)) => run > count || (run == count && (v - mean).abs() < (b - mean).abs()),
            };
            if better {
                best = Some((run, v));
            }
            k += run;
        }
        let Some((_, value)) = best else { continue };
        for i in c {
            if let Some(o) = out.get_mut(*i) {
                *o = value;
            }
        }
    }
    out
}

/// Space Points with one point: where `point` goes so that it sits halfway between its
/// neighbours `prev` and `next`, measured along the line through them; its distance from that
/// line stays. `None` when the neighbours coincide.
pub fn centre_between(prev: Point, point: Point, next: Point) -> Option<Point> {
    let axis = next - prev;
    let len = axis.hypot();
    if !(len.is_finite() && len > 0.0) {
        return None;
    }
    let u = axis / len;
    let mid = prev.midpoint(next);
    Some(point + u * (mid - point).dot(u))
}

/// Make Arc: cubic Béziers along an elliptical arc from `p0`, leaving it in direction `d0`, to
/// `p1`, arriving in direction `d1`.
///
/// When the two tangent lines meet ahead of both points, the arc is a quarter of an ellipse: the
/// affine image of a quarter circle whose corner is the meeting point, so it bulges towards that
/// point (one cubic). When the directions are opposite, the arc is half an ellipse whose conjugate
/// semi-diameters are half the chord and the tangent at `p0` of the same length (two cubics, with
/// a new on-curve point between them). Any other case has no elliptical arc and gives `None`.
pub fn elliptical_arc(p0: Point, d0: Vec2, p1: Point, d1: Vec2) -> Option<Vec<CubicBez>> {
    let (l0, l1) = (d0.hypot(), d1.hypot());
    if !(l0.is_finite() && l0 > 0.0 && l1.is_finite() && l1 > 0.0) || p0 == p1 {
        return None;
    }
    let (u0, u1) = (d0 / l0, d1 / l1);
    let k = QUARTER_ARC_K;
    let cross = u0.cross(u1);
    if cross.abs() < 1e-9 {
        if u0.dot(u1) >= 0.0 {
            return None;
        }
        let c = p0.midpoint(p1);
        let a = p0 - c;
        let b = u0 * a.hypot();
        if a.cross(b).abs() < 1e-9 * a.hypot2().max(1.0) {
            return None;
        }
        let m = c + b;
        return Some(vec![CubicBez::new(p0, p0 + b * k, m + a * k, m), CubicBez::new(m, m - a * k, p1 + b * k, p1)]);
    }
    // p0 + s·u0 = p1 − r·u1
    let w = p1 - p0;
    let s = w.cross(u1) / cross;
    let r = u0.cross(w) / cross;
    if !(s.is_finite() && r.is_finite() && s > 1e-9 && r > 1e-9) {
        return None;
    }
    let x = p0 + u0 * s;
    Some(vec![CubicBez::new(p0, p0 + (x - p0) * k, p1 + (x - p1) * k, p1)])
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::{ParamCurve as _, QuadBez};

    use super::*;

    #[test]
    fn crossing_curves_meet_once_and_a_shared_stretch_gives_nothing() {
        let a = Piece::Cubic(CubicBez::new((0.0, 0.0), (30.0, 100.0), (70.0, 100.0), (100.0, 0.0)));
        let b = Piece::Line(Line::new((0.0, 50.0), (100.0, 50.0)));
        assert_eq!(piece_crossings(&a, &b).len(), 2);
        let c = Piece::Quad(QuadBez::new((50.0, -10.0), (60.0, 50.0), (50.0, 110.0)));
        let hits = piece_crossings(&a, &c);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let (ta, tb) = hits[0];
        assert!(a.eval(ta).distance(c.eval(tb)) < 0.01);
        assert!(piece_crossings(&a, &a).is_empty());
    }

    #[test]
    fn many_overlapping_segments_are_refused_quickly() {
        // Nearly parallel arches 0.01 apart: every pair's boxes overlap and each pair needs
        // subdivision before it can be told apart.
        let segments: Vec<Vec<Piece>> = (0..MAX_CROSSING_SEGMENTS)
            .map(|i| {
                let y = f64::from(u32::try_from(i).unwrap_or(0)) * 0.01;
                vec![Piece::Cubic(CubicBez::new((0.0, y), (300.0, y + 600.0), (700.0, y + 600.0), (1000.0, y)))]
            })
            .collect();
        let start = std::time::Instant::now();
        let refused = segment_crossings(&segments);
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "{:?}", start.elapsed());
        assert!(refused.is_err_and(|e| e.contains("overlap too much")));
        // Two of them still compare.
        assert!(segment_crossings(segments.get(..2).unwrap_or_default()).is_ok());
    }

    #[test]
    fn spacing_clusters_and_regions() {
        assert_eq!(space_evenly(&[0.0, 90.0, 10.0, 30.0]), vec![0.0, 90.0, 30.0, 60.0]);
        assert_eq!(cluster_values(&[100.0, 101.0, 100.0, 200.0, 199.5], 1.0), vec![100.0, 100.0, 100.0, 199.5, 199.5]);
        // Three stems of an m, each two edges 80 apart; the middle one off centre.
        let r = space_regions(&[50.0, 130.0, 300.0, 380.0, 550.0, 630.0], 100.0).unwrap_or_default();
        assert_eq!(r, vec![50.0, 130.0, 300.0, 380.0, 550.0, 630.0]);
        let r = space_regions(&[50.0, 130.0, 260.0, 340.0, 550.0, 630.0], 100.0).unwrap_or_default();
        assert_eq!(r, vec![50.0, 130.0, 300.0, 380.0, 550.0, 630.0]);
        assert!(space_regions(&[0.0, 10.0], 5.0).is_none());
    }

    #[test]
    fn a_quarter_arc_with_square_tangents_is_a_quarter_circle() {
        let arcs = elliptical_arc(Point::new(100.0, 0.0), Vec2::new(0.0, 1.0), Point::new(0.0, 100.0), Vec2::new(-1.0, 0.0)).unwrap_or_default();
        assert_eq!(arcs.len(), 1);
        let mid = arcs[0].eval(0.5);
        assert!((mid.to_vec2().hypot() - 100.0).abs() < 0.05, "{mid:?}");
        assert!(elliptical_arc(Point::new(0.0, 0.0), Vec2::new(1.0, 0.0), Point::new(100.0, 10.0), Vec2::new(1.0, 0.0)).is_none());
        let half = elliptical_arc(Point::new(0.0, 0.0), Vec2::new(0.0, 1.0), Point::new(100.0, 0.0), Vec2::new(0.0, -1.0)).unwrap_or_default();
        assert_eq!(half.len(), 2);
        assert!((half[0].p3.y - 50.0).abs() < 1e-9 && (half[0].p3.x - 50.0).abs() < 1e-9);
    }
}
