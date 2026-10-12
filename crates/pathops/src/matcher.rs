//! Outline matching: where a pattern outline occurs in another outline, up to translation and,
//! when asked, flips (M13.15; M13.28 uses it for Find and Replace Outline).
//!
//! Method: a hash of each contour's point structure (node kinds and counts) finds candidates;
//! each candidate is compared exactly after the best translation (and flip), within a tolerance.
//! A closed pattern matches whole contours (Detect Composites: the outline of a base glyph found
//! in a composite's paths). An open pattern matches a run of consecutive nodes inside a contour
//! (Find Outline: a serif found in several glyphs); its end nodes match position and on-curve
//! status only, since the segment kind of an open contour's first node means nothing.
//!
//! A flipped match is the pattern mirrored and drawn the other way round (or turned half a turn,
//! which keeps the direction): the outline a mirrored component draws once decomposed.
//!
//! Work is bounded: every node comparison and every start position tried costs one unit of a
//! budget ([`MAX_MATCH_WORK`] for [`match_outline`]); when it runs out the matches found so far
//! are returned.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::{Affine, Point, Vec2};

use crate::{Contour, NodeKind};

/// Most nodes in a pattern.
pub const MAX_PATTERN_NODES: usize = 1_024;
/// Most matches one call returns.
pub const MAX_MATCHES: usize = 100_000;
/// Node comparisons one [`match_outline`] call may make.
pub const MAX_MATCH_WORK: usize = 50_000_000;
/// Slack added to every distance comparison, for floating-point noise.
const EPSILON: f64 = 1e-9;

/// How to match.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchOptions {
    /// Largest distance between a pattern node, moved by the match's transform, and its target
    /// node, in font units.
    pub tolerance: f64,
    /// Also try the pattern flipped horizontally, vertically and both.
    pub flips: bool,
    /// Stop after this many matches (at most [`MAX_MATCHES`]).
    pub max_matches: usize,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self { tolerance: 0.5, flips: false, max_matches: MAX_MATCHES }
    }
}

/// The target nodes one pattern contour matched: `len` consecutive nodes of contour `contour`
/// starting at `start` (wrapping around a closed contour). A whole-contour match covers every node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MatchedRun {
    pub contour: usize,
    pub start: usize,
    pub len: usize,
}

/// One occurrence of the pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// One run per pattern contour, in pattern order.
    pub runs: Vec<MatchedRun>,
    /// Maps the pattern onto the target: a translation, with a flip when `flips` found it.
    pub transform: Affine,
    /// The largest node distance after the transform.
    pub error: f64,
}

/// A contour's point structure, the same for every start node and every translation: the
/// candidate key for whole-contour matching.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Signature {
    pub closed: bool,
    pub nodes: usize,
    /// Nodes of each kind: line, curve, quadratic curve, off-curve.
    pub kinds: [usize; 4],
}

/// The signature of `contour`.
pub fn signature(contour: &Contour) -> Signature {
    let mut kinds = [0usize; 4];
    for n in &contour.nodes {
        if let Some(slot) = kinds.get_mut(kind_slot(n.kind)) {
            *slot += 1;
        }
    }
    Signature { closed: contour.closed, nodes: contour.nodes.len(), kinds }
}

fn kind_slot(k: NodeKind) -> usize {
    match k {
        NodeKind::Line => 0,
        NodeKind::Curve => 1,
        NodeKind::QCurve => 2,
        NodeKind::OffCurve => 3,
    }
}

/// Every occurrence of `pattern` in `target`, matches not overlapping, in target order. A pattern
/// over [`MAX_PATTERN_NODES`] nodes, or empty, finds nothing.
pub fn match_outline(pattern: &[Contour], target: &[Contour], options: &MatchOptions) -> Vec<Match> {
    let mut work = MAX_MATCH_WORK;
    match_outline_within(pattern, target, options, &mut work)
}

/// [`match_outline`] with a caller's work budget, which it lowers by the work done. A budget of
/// zero afterwards means the search stopped early and more matches may exist.
pub fn match_outline_within(pattern: &[Contour], target: &[Contour], options: &MatchOptions, work: &mut usize) -> Vec<Match> {
    let total: usize = pattern.iter().map(|c| c.nodes.len()).sum();
    if pattern.is_empty() || total > MAX_PATTERN_NODES || pattern.iter().any(|c| c.nodes.is_empty()) {
        return Vec::new();
    }
    if !options.tolerance.is_finite() || pattern.iter().flat_map(|c| &c.nodes).any(|n| !n.point.x.is_finite() || !n.point.y.is_finite()) {
        return Vec::new();
    }
    let tol = options.tolerance.max(0.0);
    let max = options.max_matches.min(MAX_MATCHES);
    if max == 0 {
        return Vec::new();
    }
    let index = TargetIndex::new(target);
    let mut search =
        Search { target, index: &index, tol, used: target.iter().map(|c| vec![false; c.nodes.len()]).collect(), out: Vec::new(), max, work };
    for (flip, reversed) in variants(options.flips) {
        let Some(prepared) = prepare(pattern, flip, reversed) else { continue };
        search.run(&prepared, flip);
        if search.out.len() >= search.max || *search.work == 0 {
            break;
        }
    }
    let mut out = search.out;
    out.sort_by_key(|m| m.runs.iter().map(|r| (r.contour, r.start)).min());
    out
}

/// The pattern variants to try: the flip applied to it, and whether its node order is reversed.
fn variants(flips: bool) -> Vec<(Affine, bool)> {
    let mut v = vec![(Affine::IDENTITY, false)];
    if flips {
        v.push((Affine::FLIP_X, true));
        v.push((Affine::FLIP_Y, true));
        v.push((Affine::FLIP_X * Affine::FLIP_Y, false));
    }
    v
}

/// A pattern contour after its flip: positions, kinds, and what the candidate filters need.
struct Prepared {
    closed: bool,
    points: Vec<Point>,
    kinds: Vec<NodeKind>,
    sig: Signature,
    /// Width and height of the box around every node.
    size: (f64, f64),
}

fn prepare(pattern: &[Contour], flip: Affine, reversed: bool) -> Option<Vec<Prepared>> {
    let mut out = Vec::with_capacity(pattern.len());
    for c in pattern {
        let c = if reversed { c.reversed().ok()? } else { c.clone() };
        let points: Vec<Point> = c.nodes.iter().map(|n| flip * n.point).collect();
        out.push(Prepared {
            closed: c.closed,
            kinds: c.nodes.iter().map(|n| n.kind).collect(),
            sig: signature(&c),
            size: size(points.iter().copied())?,
            points,
        });
    }
    Some(out)
}

fn size(points: impl Iterator<Item = Point>) -> Option<(f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for p in points {
        b = Some(match b {
            None => (p.x, p.y, p.x, p.y),
            Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        });
    }
    b.map(|(x0, y0, x1, y1)| (x1 - x0, y1 - y0))
}

/// The target's closed contours by signature, and every contour's size.
struct TargetIndex {
    closed: HashMap<Signature, Vec<usize>>,
    sizes: Vec<Option<(f64, f64)>>,
}

impl TargetIndex {
    fn new(target: &[Contour]) -> Self {
        let mut closed: HashMap<Signature, Vec<usize>> = HashMap::new();
        for (i, c) in target.iter().enumerate() {
            if c.closed {
                closed.entry(signature(c)).or_default().push(i);
            }
        }
        Self { closed, sizes: target.iter().map(|c| size(c.nodes.iter().map(|n| n.point))).collect() }
    }
}

struct Search<'a, 'w> {
    target: &'a [Contour],
    index: &'a TargetIndex,
    tol: f64,
    /// Target nodes an earlier match took.
    used: Vec<Vec<bool>>,
    out: Vec<Match>,
    max: usize,
    work: &'w mut usize,
}

impl Search<'_, '_> {
    fn charge(&mut self, units: usize) -> bool {
        if *self.work < units {
            *self.work = 0;
            return false;
        }
        *self.work -= units;
        true
    }

    /// Finds the matches of one pattern variant. The pattern contour with the most nodes is
    /// placed first; every position it fits gives the translation the other contours must share.
    fn run(&mut self, pattern: &[Prepared], flip: Affine) {
        let Some((a, anchor)) = pattern.iter().enumerate().max_by_key(|(i, p)| (p.points.len(), std::cmp::Reverse(*i))) else { return };
        let target = self.target;
        for (ci, tc) in target.iter().enumerate() {
            let len = tc.nodes.len();
            let starts = if anchor.closed {
                let fits = tc.closed && signature(tc) == anchor.sig && self.size_fits(anchor, ci);
                if fits { len } else { 0 }
            } else if len < anchor.points.len() {
                0
            } else if tc.closed {
                len
            } else {
                len - anchor.points.len() + 1
            };
            for start in 0..starts {
                if self.out.len() >= self.max || !self.charge(1) {
                    return;
                }
                let (Some(tp), Some(pp)) = (tc.nodes.get(start), anchor.points.first()) else { continue };
                let t0 = tp.point - *pp;
                let anchor_run = MatchedRun { contour: ci, start, len: anchor.points.len() };
                let mut claimed: HashSet<(usize, usize)> = HashSet::new();
                if !self.fits(anchor, anchor_run, t0, 2.0 * self.tol, &claimed) {
                    continue;
                }
                claim(&mut claimed, anchor_run, len);
                let mut runs: Vec<Option<MatchedRun>> = vec![None; pattern.len()];
                if let Some(slot) = runs.get_mut(a) {
                    *slot = Some(anchor_run);
                }
                let mut complete = true;
                for (j, pc) in pattern.iter().enumerate() {
                    if j == a {
                        continue;
                    }
                    match self.find_run(pc, t0, &claimed) {
                        Some(run) => {
                            claim(&mut claimed, run, target.get(run.contour).map_or(0, |c| c.nodes.len()));
                            if let Some(slot) = runs.get_mut(j) {
                                *slot = Some(run);
                            }
                        }
                        None => {
                            complete = false;
                            break;
                        }
                    }
                }
                if !complete {
                    continue;
                }
                let runs: Vec<MatchedRun> = runs.into_iter().flatten().collect();
                let Some((t, error)) = best_translation(pattern, target, &runs) else { continue };
                if error > self.tol + EPSILON {
                    continue;
                }
                for &(c, n) in &claimed {
                    if let Some(u) = self.used.get_mut(c).and_then(|v| v.get_mut(n)) {
                        *u = true;
                    }
                }
                self.out.push(Match { runs, transform: Affine::translate(t) * flip, error });
                if anchor.closed {
                    break;
                }
            }
        }
    }

    /// Whether the pattern contour's box and the target contour's box have sizes a match allows.
    fn size_fits(&self, p: &Prepared, ci: usize) -> bool {
        let Some(Some((w, h))) = self.index.sizes.get(ci) else { return false };
        let slack = 2.0 * self.tol + EPSILON;
        (w - p.size.0).abs() <= slack && (h - p.size.1).abs() <= slack
    }

    /// A free run of the target that `pc` moved by `t` lands on within twice the tolerance (the
    /// best translation is at most the tolerance from any node's own one).
    fn find_run(&mut self, pc: &Prepared, t: Vec2, claimed: &HashSet<(usize, usize)>) -> Option<MatchedRun> {
        let target = self.target;
        let limit = 2.0 * self.tol;
        let first = *pc.points.first()? + t;
        let candidates: Vec<usize> = if pc.closed {
            self.index.closed.get(&pc.sig).cloned().unwrap_or_default().into_iter().filter(|&ci| self.size_fits(pc, ci)).collect()
        } else {
            (0..target.len()).collect()
        };
        for ci in candidates {
            let tc = target.get(ci)?;
            let len = tc.nodes.len();
            let starts = if pc.closed {
                len
            } else if len < pc.points.len() {
                0
            } else if tc.closed {
                len
            } else {
                len - pc.points.len() + 1
            };
            for start in 0..starts {
                if !self.charge(1) {
                    return None;
                }
                let near = tc.nodes.get(start).is_some_and(|n| (n.point - first).hypot() <= limit + EPSILON);
                let run = MatchedRun { contour: ci, start, len: pc.points.len() };
                if near && self.fits(pc, run, t, limit, claimed) {
                    return Some(run);
                }
            }
        }
        None
    }

    /// Whether every node of `pc` moved by `t` is within `limit` of its node in `run`, with the
    /// same kind, and that node is free.
    fn fits(&mut self, pc: &Prepared, run: MatchedRun, t: Vec2, limit: f64, claimed: &HashSet<(usize, usize)>) -> bool {
        let Some(tc) = self.target.get(run.contour) else { return false };
        let len = tc.nodes.len();
        let m = pc.points.len();
        if pc.closed && (!tc.closed || len != m) {
            return false;
        }
        if m == 0 || m > len {
            return false;
        }
        for (i, (p, k)) in pc.points.iter().zip(&pc.kinds).enumerate() {
            if !self.charge(1) {
                return false;
            }
            let Some(at) = node_at(tc, run.start, i) else { return false };
            if self.used.get(run.contour).and_then(|u| u.get(at)).copied().unwrap_or(true) || claimed.contains(&(run.contour, at)) {
                return false;
            }
            let Some(node) = tc.nodes.get(at) else { return false };
            let end = !pc.closed && (i == 0 || i + 1 == m);
            let kind_ok = if end { node.on_curve() == (*k != NodeKind::OffCurve) } else { node.kind == *k };
            if !kind_ok || (node.point - (*p + t)).hypot() > limit + EPSILON {
                return false;
            }
        }
        true
    }
}

/// The index of the `i`th node of a run starting at `start` in `tc`.
fn node_at(tc: &Contour, start: usize, i: usize) -> Option<usize> {
    let len = tc.nodes.len();
    let at = start.checked_add(i)?;
    if tc.closed { at.checked_rem(len) } else { (at < len).then_some(at) }
}

fn claim(claimed: &mut HashSet<(usize, usize)>, run: MatchedRun, len: usize) {
    for i in 0..run.len {
        let at = run.start.checked_add(i).and_then(|v| v.checked_rem(len));
        if let Some(at) = at {
            claimed.insert((run.contour, at));
        }
    }
}

/// The translation that brings the runs' nodes closest to the pattern's (the middle of the
/// spread of node offsets on each axis), and the largest node distance after it.
fn best_translation(pattern: &[Prepared], target: &[Contour], runs: &[MatchedRun]) -> Option<(Vec2, f64)> {
    let mut offsets = Vec::new();
    for (pc, run) in pattern.iter().zip(runs) {
        let tc = target.get(run.contour)?;
        for (i, p) in pc.points.iter().enumerate() {
            let node = tc.nodes.get(node_at(tc, run.start, i)?)?;
            offsets.push(node.point - *p);
        }
    }
    let first = *offsets.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for d in &offsets {
        (x0, y0, x1, y1) = (x0.min(d.x), y0.min(d.y), x1.max(d.x), y1.max(d.y));
    }
    let t = Vec2::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let error = offsets.iter().map(|d| (*d - t).hypot()).fold(0.0, f64::max);
    error.is_finite().then_some((t, error))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::Node;

    fn poly(points: &[(f64, f64)]) -> Contour {
        Contour::new(true, points.iter().map(|&(x, y)| Node::new(Point::new(x, y), NodeKind::Line)).collect())
    }

    fn moved(c: &Contour, a: Affine) -> Contour {
        let mut c = c.clone();
        for n in &mut c.nodes {
            n.point = a * n.point;
        }
        c
    }

    /// An L shape with no symmetry.
    fn ell() -> Contour {
        poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 20.0), (20.0, 20.0), (20.0, 150.0), (0.0, 150.0)])
    }

    #[test]
    fn finds_a_translated_two_contour_pattern_whatever_the_start_node() {
        let square = poly(&[(200.0, 0.0), (240.0, 0.0), (240.0, 40.0), (200.0, 40.0)]);
        let pattern = [ell(), square.clone()];
        let shift = Affine::translate((300.0, 50.0));
        let mut rotated = moved(&square, shift);
        rotated.nodes.rotate_left(2);
        let other = poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
        let target = [other, moved(&ell(), shift), rotated];
        let found = match_outline(&pattern, &target, &MatchOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].runs, vec![MatchedRun { contour: 1, start: 0, len: 6 }, MatchedRun { contour: 2, start: 2, len: 4 }]);
        assert_eq!(found[0].transform, shift);
    }

    #[test]
    fn a_mirrored_copy_is_found_only_with_flips() {
        let mirrored = moved(&ell(), Affine::translate((500.0, 0.0)) * Affine::FLIP_X).reversed().unwrap();
        let target = [mirrored];
        assert!(match_outline(&[ell()], &target, &MatchOptions::default()).is_empty());
        let found = match_outline(&[ell()], &target, &MatchOptions { flips: true, ..MatchOptions::default() });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].transform, Affine::translate((500.0, 0.0)) * Affine::FLIP_X);
    }

    #[test]
    fn tolerance_and_overlap_limit_the_matches() {
        let target = [moved(&ell(), Affine::translate((0.3, 0.0))), ell()];
        let found = match_outline(&[ell()], &target, &MatchOptions::default());
        assert_eq!(found.len(), 2);
        let strict = match_outline(&[ell()], &target, &MatchOptions { tolerance: 0.1, ..MatchOptions::default() });
        assert_eq!(strict.len(), 2, "each node is within 0.1 of a translated copy: the offset is uniform");
        let mut bent = ell();
        bent.nodes[3].point.x += 2.0;
        assert!(match_outline(&[ell()], &[bent], &MatchOptions::default()).is_empty());
        let one = match_outline(&[ell()], &target, &MatchOptions { max_matches: 1, ..MatchOptions::default() });
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn an_open_pattern_matches_a_run_inside_closed_contours() {
        // A notch: down, across, up.
        let notch = Contour::new(
            false,
            [(0.0, 0.0), (0.0, -10.0), (10.0, -10.0), (10.0, 0.0)].iter().map(|&(x, y)| Node::new(Point::new(x, y), NodeKind::Line)).collect(),
        );
        let shape = poly(&[(0.0, 0.0), (0.0, 100.0), (40.0, 100.0), (40.0, 90.0), (50.0, 90.0), (50.0, 100.0), (100.0, 100.0), (100.0, 0.0)]);
        let found = match_outline(&[notch], &[shape], &MatchOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].runs[0], MatchedRun { contour: 0, start: 2, len: 4 });
        assert_eq!(found[0].transform, Affine::translate((40.0, 100.0)));
    }

    #[test]
    fn work_is_bounded_and_bad_patterns_find_nothing() {
        let target: Vec<Contour> = (0..50).map(|_| ell()).collect();
        let mut work = 30;
        let found = match_outline_within(&[ell()], &target, &MatchOptions::default(), &mut work);
        assert_eq!(work, 0);
        assert!(found.len() < 50);
        assert!(match_outline(&[], &target, &MatchOptions::default()).is_empty());
        let mut nan = ell();
        nan.nodes[0].point.x = f64::NAN;
        assert!(match_outline(&[nan], &target, &MatchOptions::default()).is_empty());
        let huge = poly(&[(0.0, 0.0); MAX_PATTERN_NODES + 1]);
        assert!(match_outline(&[huge], &target, &MatchOptions::default()).is_empty());
    }
}
