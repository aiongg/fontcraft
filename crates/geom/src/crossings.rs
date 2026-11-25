//! Where the segments of paths cross: every pair of [`Piece`] lists, by a sweep along x, lines
//! against curves in closed form and curves against curves by subdivision, within a work budget.
//!
//! Segment parameters follow [`crate::Segment`]: 0 at the start node, 1 at the end node, spread
//! evenly across the pieces of a quadratic segment with several off-curve points.

use kurbo::{Line, ParamCurve, PathSeg, Point, Rect};

use crate::Piece;

/// Most segments [`segment_crossings`] compares in one call. Comparisons grow with the square of
/// the segments that overlap, so a layer with more is refused.
pub const MAX_CROSSING_SEGMENTS: usize = 20_000;
/// Most box tests for one pair of curved pieces. Two curves that run along each other (a shared
/// stretch rather than a crossing) never narrow down; past this many tests the pair counts as
/// touching, not crossing.
pub const MAX_PAIR_WORK: usize = 4_096;
/// Most work one [`segment_crossings`] call does: one unit per pair of segment boxes tested, per
/// pair of pieces compared and per subdivision step of two curves. Past it the call is refused.
/// A command that compares many layers shares one such budget through
/// [`segment_crossings_within`].
pub const MAX_CROSSING_WORK: usize = 50_000_000;
/// Most crossings one call reports.
pub const MAX_CROSSINGS: usize = 100_000;
/// Two curves cross where their sub-curves fit in boxes this small, in units.
pub const CROSSING_TOLERANCE: f64 = 1e-4;
/// Deepest subdivision of a curve pair.
const MAX_DEPTH: usize = 60;
/// Parameters closer than this are one cut.
pub const SAME_T: f64 = 1e-9;
/// The piece as a kurbo path segment.
pub fn path_seg(p: &Piece) -> PathSeg {
    match p {
        Piece::Line(l) => PathSeg::Line(*l),
        Piece::Quad(q) => PathSeg::Quad(*q),
        Piece::Cubic(c) => PathSeg::Cubic(*c),
    }
}

/// The part of `p` between the local parameters `a` and `b`.
pub fn subpiece(p: &Piece, a: f64, b: f64) -> Piece {
    match p {
        Piece::Line(l) => Piece::Line(l.subsegment(a..b)),
        Piece::Quad(q) => Piece::Quad(q.subsegment(a..b)),
        Piece::Cubic(c) => Piece::Cubic(c.subsegment(a..b)),
    }
}

/// The box around the piece's points (control points included), which holds the curve.
pub fn piece_hull(p: &Piece) -> Option<Rect> {
    let pts: Vec<Point> = match p {
        Piece::Line(l) => vec![l.p0, l.p1],
        Piece::Quad(q) => vec![q.p0, q.p1, q.p2],
        Piece::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    };
    crate::bounds(pts)
}

fn overlaps(a: Rect, b: Rect, slack: f64) -> bool {
    a.x0 <= b.x1 + slack && b.x0 <= a.x1 + slack && a.y0 <= b.y1 + slack && b.y0 <= a.y1 + slack
}

/// Local parameter pairs `(ta, tb)` where two pieces meet, each in 0..=1. A pair that runs along
/// itself (a shared stretch) gives none.
pub fn piece_crossings(a: &Piece, b: &Piece) -> Vec<(f64, f64)> {
    let mut work = usize::MAX;
    piece_crossings_within(a, b, &mut work).unwrap_or_default()
}

/// [`piece_crossings`] charging its subdivision steps to `work`; `None` when `work` runs out.
fn piece_crossings_within(a: &Piece, b: &Piece, work: &mut usize) -> Option<Vec<(f64, f64)>> {
    match (a, b) {
        (_, Piece::Line(l)) => Some(line_piece_crossings(a, *l)),
        (Piece::Line(l), _) => Some(line_piece_crossings(b, *l).into_iter().map(|(t, u)| (u, t)).collect()),
        _ => curves(a, b, work),
    }
}

/// Where `p` meets the line piece `l`: (parameter on `p`, parameter on `l`).
pub fn line_piece_crossings(p: &Piece, l: Line) -> Vec<(f64, f64)> {
    if l.p0 == l.p1 {
        return Vec::new();
    }
    path_seg(p)
        .intersect_line(l)
        .into_iter()
        .filter(|h| h.segment_t.is_finite() && h.line_t.is_finite())
        .filter(|h| (-SAME_T..=1.0 + SAME_T).contains(&h.segment_t) && (-SAME_T..=1.0 + SAME_T).contains(&h.line_t))
        .map(|h| (h.segment_t.clamp(0.0, 1.0), h.line_t.clamp(0.0, 1.0)))
        .collect()
}

/// Two curved pieces by subdivision: halve the larger sub-curve while the boxes overlap, until
/// both fit in [`CROSSING_TOLERANCE`]. Each step also takes one unit of `budget`; `None` when it
/// runs out.
fn curves(a: &Piece, b: &Piece, budget: &mut usize) -> Option<Vec<(f64, f64)>> {
    struct Job {
        a: Piece,
        ta: (f64, f64),
        b: Piece,
        tb: (f64, f64),
        depth: usize,
    }
    let mut stack = vec![Job { a: *a, ta: (0.0, 1.0), b: *b, tb: (0.0, 1.0), depth: 0 }];
    let mut found: Vec<(f64, f64)> = Vec::new();
    let mut work = 0;
    while let Some(j) = stack.pop() {
        work += 1;
        if work > MAX_PAIR_WORK {
            return Some(Vec::new());
        }
        *budget = budget.checked_sub(1)?;
        let (Some(ra), Some(rb)) = (piece_hull(&j.a), piece_hull(&j.b)) else { continue };
        if !overlaps(ra, rb, CROSSING_TOLERANCE) {
            continue;
        }
        let size = |r: Rect| r.width().max(r.height());
        let (sa, sb) = (size(ra), size(rb));
        if (sa <= CROSSING_TOLERANCE && sb <= CROSSING_TOLERANCE) || j.depth >= MAX_DEPTH {
            found.push(((j.ta.0 + j.ta.1) / 2.0, (j.tb.0 + j.tb.1) / 2.0));
            continue;
        }
        let depth = j.depth + 1;
        if sa >= sb {
            let m = (j.ta.0 + j.ta.1) / 2.0;
            stack.push(Job { a: subpiece(&j.a, 0.0, 0.5), ta: (j.ta.0, m), b: j.b, tb: j.tb, depth });
            stack.push(Job { a: subpiece(&j.a, 0.5, 1.0), ta: (m, j.ta.1), b: j.b, tb: j.tb, depth });
        } else {
            let m = (j.tb.0 + j.tb.1) / 2.0;
            stack.push(Job { a: j.a, ta: j.ta, b: subpiece(&j.b, 0.0, 0.5), tb: (j.tb.0, m), depth });
            stack.push(Job { a: j.a, ta: j.ta, b: subpiece(&j.b, 0.5, 1.0), tb: (m, j.tb.1), depth });
        }
    }
    // Neighbouring boxes around one crossing report it more than once.
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for f in found {
        let near = out.iter().any(|o| a.eval(o.0).distance(a.eval(f.0)) <= 10.0 * CROSSING_TOLERANCE);
        if !near {
            out.push(f);
        }
    }
    Some(out)
}

/// The box around a segment's pieces.
fn segment_hull(pieces: &[Piece]) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for p in pieces {
        let r = piece_hull(p)?;
        out = Some(out.map_or(r, |o| o.union(r)));
    }
    out
}

/// For each segment (a list of pieces), the segment parameters where another segment crosses
/// or touches it, sorted, repeats removed. A segment is not compared with itself; segments that
/// share an end node meet there at parameter 0 or 1, which callers drop as an existing node.
/// Errors when there are more than [`MAX_CROSSING_SEGMENTS`] segments, more than
/// [`MAX_CROSSINGS`] crossings, or the comparisons take more than [`MAX_CROSSING_WORK`].
pub fn segment_crossings(segments: &[Vec<Piece>]) -> Result<Vec<Vec<f64>>, String> {
    let mut work = MAX_CROSSING_WORK;
    segment_crossings_within(segments, &mut work)
}

/// [`segment_crossings`], taking its comparisons from `work`, which is left with what remains.
/// Errors when `work` runs out.
pub fn segment_crossings_within(segments: &[Vec<Piece>], work: &mut usize) -> Result<Vec<Vec<f64>>, String> {
    let pairs = segment_crossing_pairs_within(segments, work)?;
    let mut out: Vec<Vec<f64>> = vec![Vec::new(); segments.len()];
    for c in pairs {
        if let Some(list) = out.get_mut(c.a) {
            list.push(c.ta);
        }
        if let Some(list) = out.get_mut(c.b) {
            list.push(c.tb);
        }
    }
    for list in &mut out {
        list.sort_by(f64::total_cmp);
        list.dedup_by(|x, y| (*x - *y).abs() <= SAME_T);
    }
    Ok(out)
}

/// Where two segments cross or touch: their indexes in the list, `a` before `b`, and the
/// parameter on each, across all of its pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentCrossing {
    pub a: usize,
    pub ta: f64,
    pub b: usize,
    pub tb: f64,
}

/// Every place where two segments (lists of pieces) cross or touch, with the limits and the
/// work accounting of [`segment_crossings_within`]. A crossing found from neighbouring boxes
/// may be listed twice with nearly the same parameters.
pub fn segment_crossing_pairs_within(segments: &[Vec<Piece>], work: &mut usize) -> Result<Vec<SegmentCrossing>, String> {
    if segments.len() > MAX_CROSSING_SEGMENTS {
        return Err(format!("the layer has {} segments; at most {MAX_CROSSING_SEGMENTS} can be compared", segments.len()));
    }
    let mut out: Vec<SegmentCrossing> = Vec::new();
    // Sweep along x: only segments whose boxes overlap in x are compared.
    let mut order: Vec<(usize, Rect)> = segments.iter().enumerate().filter_map(|(i, s)| segment_hull(s).map(|r| (i, r))).collect();
    order.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
    let too_much = || "the segments overlap too much to compare: the search reached its work limit".to_string();
    for (k, (i, ri)) in order.iter().enumerate() {
        for (j, rj) in order.iter().skip(k + 1) {
            if rj.x0 > ri.x1 + CROSSING_TOLERANCE {
                break;
            }
            *work = work.checked_sub(1).ok_or_else(too_much)?;
            if !overlaps(*ri, *rj, CROSSING_TOLERANCE) {
                continue;
            }
            let (Some(a), Some(b)) = (segments.get(*i), segments.get(*j)) else { continue };
            let (na, nb) = (a.len() as f64, b.len() as f64);
            for (pa, piece_a) in a.iter().enumerate() {
                for (pb, piece_b) in b.iter().enumerate() {
                    *work = work.checked_sub(1).ok_or_else(too_much)?;
                    for (ta, tb) in piece_crossings_within(piece_a, piece_b, work).ok_or_else(too_much)? {
                        if out.len() >= MAX_CROSSINGS {
                            return Err(format!("the paths cross more than {MAX_CROSSINGS} times"));
                        }
                        let (ta, tb) = ((pa as f64 + ta) / na, (pb as f64 + tb) / nb);
                        out.push(if i < j { SegmentCrossing { a: *i, ta, b: *j, tb } } else { SegmentCrossing { a: *j, ta: tb, b: *i, tb: ta } });
                    }
                }
            }
        }
    }
    Ok(out)
}
