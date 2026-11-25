//! Cursivy: the optical correction of a slanted outline (`path.transformations.slantVsCursivy`,
//! docs/spec/11-filters.md › Cursivy, where the maths is written out).
//!
//! The input is the outline after a plain shear. A shear by `t` changes the thickness of a stroke
//! that runs diagonally: one whose normal, upright, is `n = (cos φ, sin φ)` ends up `1 / L` times
//! as thick, with `L = |(n.x, n.y − t·n.x)|`. The correction moves each curve along its normal by
//! half the difference to the thickness wanted, so the two sides of a round stroke, which each
//! take half of it, get back the thickness they had upright. Straight segments stay on the plain
//! shear, which keeps the horizontal width of a vertical stem.

use fontcraft_geom::{Point, Vec2};

use crate::PathOpError;
use crate::contour::{Contour, DIRECTION_EPSILON, Node, Piece, Ring, Seg};

/// Samples of a curve piece the correction is fitted to.
const SAMPLES: u32 = 16;
/// Most samples of one segment; a quadratic spline with many off-curve nodes has fewer per piece.
const MAX_SEGMENT_SAMPLES: u32 = 256;
/// The power of the cosine with which the thickness wanted falls from the full one (a diagonal)
/// to that of a sheared vertical stem (a side).
const SIDE_POWER: i32 = 4;
/// A curve moves at most this share of its chord, so details smaller than the strokes (brackets,
/// ink traps) keep their shape.
const MAX_SHIFT: f64 = 0.08;
/// A handle's length changes at most by this factor either way.
const MAX_HANDLE_CHANGE: f64 = 0.5;
/// How strongly the fit holds the handle lengths: a handle changed by its whole length costs as
/// much as missing the target by this share of the thicker stem at every sample.
const HANDLE_WEIGHT: f64 = 0.05;
/// Two segments whose directions at a node differ by at most this many degrees meet smoothly.
const SMOOTH_DEGREES: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursivyOptions {
    /// The shear already applied: how far x moves per unit of y (the tangent of the slant).
    pub shear: f64,
    /// Thickness of vertical and of horizontal strokes, in units of the outline.
    pub vertical_stem: f64,
    pub horizontal_stem: f64,
}

impl CursivyOptions {
    fn check(&self) -> Result<(), PathOpError> {
        if !(self.shear.is_finite() && self.vertical_stem.is_finite() && self.horizontal_stem.is_finite()) {
            return Err(PathOpError::NotFinite);
        }
        if self.vertical_stem <= 0.0 || self.horizontal_stem <= 0.0 {
            return Err(PathOpError::Invalid("the stem values must be above zero".into()));
        }
        Ok(())
    }

    /// How far outwards a curve with the direction `d` in the sheared outline moves.
    fn shift(&self, d: Vec2) -> f64 {
        let upright = Vec2::new(d.x - self.shear * d.y, d.y);
        let Some(n) = unit(Vec2::new(upright.y, -upright.x)) else { return 0.0 };
        let plain = 1.0 / Vec2::new(n.x, n.y - self.shear * n.x).hypot();
        let side = 1.0 / 1.0f64.hypot(self.shear);
        let wanted = 1.0 - (1.0 - side) * n.x.abs().powi(SIDE_POWER);
        let stem = self.vertical_stem * n.x * n.x + self.horizontal_stem * n.y * n.y;
        let shift = 0.5 * stem * (wanted - plain);
        if shift.is_finite() { shift } else { 0.0 }
    }
}

/// Where the correction puts the nodes of one layer's `contours`, which were sheared by
/// `options.shear`: `(contour, node, place)` for each node that moves. `moved(contour, node)`
/// says which nodes the shear moved; a segment is corrected when all its nodes were.
///
/// Closed contours with curves are corrected. Which side of a contour is inside follows from the
/// direction of the contour with the largest area. Corner nodes, and nodes between a curve and a
/// straight segment, stay where they are; handles keep their directions and change in length.
pub fn cursivy(
    contours: &[Contour],
    moved: impl Fn(usize, usize) -> bool,
    options: CursivyOptions,
) -> Result<Vec<(usize, usize, Point)>, PathOpError> {
    options.check()?;
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > crate::MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    let largest = rings.iter().map(Ring::area).max_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0);
    if largest == 0.0 || options.shear == 0.0 {
        return Ok(Vec::new());
    }
    // Outwards, seen along the path: to the right when outer contours run counter-clockwise.
    let outwards = if largest > 0.0 { 1.0 } else { -1.0 };
    let mut out = Vec::new();
    for (c, ring) in rings.iter().enumerate().filter(|(_, r)| r.closed) {
        let n = contours.get(c).map_or(0, |c| c.nodes.len());
        let fit = RingFit::new(ring, n, outwards, options, |i| moved(c, i));
        out.extend(fit.places().into_iter().map(|(i, p)| (c, i, p)));
    }
    Ok(out)
}

/// One closed ring and how far each of its nodes moves.
struct RingFit<'a> {
    ring: &'a Ring,
    /// Nodes of the contour the ring was read from.
    len: usize,
    outwards: f64,
    options: CursivyOptions,
    /// Whether the segment that ends at each ring node is a curve whose nodes were all sheared.
    curves: Vec<bool>,
    /// How far each ring node moves.
    shifts: Vec<Vec2>,
}

impl<'a> RingFit<'a> {
    fn new(ring: &'a Ring, len: usize, outwards: f64, options: CursivyOptions, moved: impl Fn(usize) -> bool) -> Self {
        let mut fit = Self { ring, len, outwards, options, curves: Vec::new(), shifts: Vec::new() };
        let on = |j: Option<usize>| j.and_then(|j| ring.nodes.get(j)).and_then(|r| r.src).is_some_and(&moved);
        fit.curves = (0..ring.len())
            .map(|j| {
                let handles = ring.nodes.get(j).map_or(0, |r| r.ctrl.len());
                handles > 0 && on(Some(j)) && on(ring.prev(j)) && (0..handles).all(|k| fit.handle_index(j, k).is_some_and(&moved))
            })
            .collect();
        fit.shifts = (0..ring.len()).map(|j| fit.node_shift(j).unwrap_or(Vec2::ZERO)).collect();
        fit
    }

    /// The contour index of handle `k` of the segment that ends at ring node `j`.
    fn handle_index(&self, j: usize, k: usize) -> Option<usize> {
        let r = self.ring.nodes.get(j)?;
        let before = r.ctrl.len().checked_sub(k)?;
        (r.src? + self.len).checked_sub(before)?.checked_rem(self.len)
    }

    fn is_curve(&self, j: usize) -> bool {
        self.curves.get(j).copied().unwrap_or(false)
    }

    /// How far a point moves where the path has the direction `d`, on a curve with this chord.
    fn shift(&self, d: Vec2, chord: f64) -> f64 {
        let most = MAX_SHIFT * chord;
        self.options.shift(d).clamp(-most, most)
    }

    fn outward_normal(&self, d: Vec2) -> Option<Vec2> {
        unit(Vec2::new(d.y, -d.x)).map(|n| self.outwards * n)
    }

    /// How far ring node `j` moves: along the normal where two curves meet smoothly, otherwise
    /// not at all.
    fn node_shift(&self, j: usize) -> Option<Vec2> {
        let next = self.ring.next(j)?;
        if !(self.is_curve(j) && self.is_curve(next)) {
            return None;
        }
        let (before, after) = (self.ring.seg(j)?, self.ring.seg(next)?);
        let (a, b) = (unit(end_direction(&before)?)?, unit(start_direction(&after)?)?);
        if a.cross(b).atan2(a.dot(b)).abs().to_degrees() > SMOOTH_DEGREES {
            return None;
        }
        let chord = (before.to - before.from).hypot().min((after.to - after.from).hypot());
        Some(self.outward_normal(a + b)? * self.shift(a + b, chord))
    }

    /// The new places of the nodes that move, by contour index.
    fn places(&self) -> Vec<(usize, Point)> {
        let mut out = Vec::new();
        for (j, r) in self.ring.nodes.iter().enumerate() {
            if let (Some(i), Some(v)) = (r.src, self.shifts.get(j))
                && *v != Vec2::ZERO
            {
                out.push((i, r.node.point + *v));
            }
            if self.is_curve(j)
                && let Some(moves) = self.handles(j)
            {
                for (k, (h, v)) in r.ctrl.iter().zip(moves).enumerate() {
                    if let Some(i) = self.handle_index(j, k)
                        && v != Vec2::ZERO
                    {
                        out.push((i, h.point + v));
                    }
                }
            }
        }
        out
    }

    /// How far each handle of the segment that ends at ring node `j` moves: with the node it
    /// belongs to and along its own direction, by the least-squares fit of the curve's movement
    /// to the correction at samples of the curve. A handle between two others (a quadratic
    /// spline) moves by the correction for the direction of the control polygon there.
    fn handles(&self, j: usize) -> Option<Vec<Vec2>> {
        let seg = self.ring.seg(j)?;
        let last = seg.ctrl.len().checked_sub(1)?;
        let zero = Vec2::ZERO;
        let d0 = self.shifts.get(self.ring.prev(j)?).copied().unwrap_or(zero);
        let d1 = self.shifts.get(j).copied().unwrap_or(zero);
        let chord = (seg.to - seg.from).hypot();
        let point = |k: usize| seg.ctrl.get(k).map(|n| n.point);
        if last == 0 {
            return Some(vec![0.5 * (d0 + d1)]);
        }
        // Without the fit: the end handles follow their nodes.
        let mut moves: Vec<Vec2> = (0..=last)
            .map(|k| {
                if k == 0 {
                    d0
                } else if k == last {
                    d1
                } else {
                    let along = point(k + 1).zip(k.checked_sub(1).and_then(point)).map_or(zero, |(b, a)| b - a);
                    self.outward_normal(along).map_or(zero, |n| n * self.shift(along, chord))
                }
            })
            .collect();
        let (first_handle, last_handle) = (point(0)? - seg.from, point(last)? - seg.to);
        // The movement of the curve is linear in the movement of its points, so it is the sum of
        // what the moves above give and of the two handles' movements along their directions.
        let given = movement(&seg, d0, &moves, d1);
        let alone = |k: usize, v: Vec2| {
            let mut m = vec![zero; last + 1];
            if let Some(x) = m.get_mut(k) {
                *x = v;
            }
            movement(&seg, zero, &m, zero)
        };
        let (by_first, by_last) = (alone(0, first_handle), alone(last, last_handle));
        let pieces = seg.pieces();
        let per_piece = u32::try_from(pieces.len()).map_or(1, |n| (MAX_SEGMENT_SAMPLES / n.max(1)).clamp(2, SAMPLES));
        let (mut a, mut b, mut c, mut r, mut s, mut samples) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for (i, piece) in pieces.iter().enumerate() {
            let (Some(g), Some(f), Some(l)) = (given.get(i), by_first.get(i), by_last.get(i)) else { continue };
            for k in 1..per_piece {
                let t = f64::from(k) / f64::from(per_piece);
                let d = direction_at(piece, t);
                let Some(n) = self.outward_normal(d) else { continue };
                let miss = self.shift(d, chord) - g.eval(t).to_vec2().dot(n);
                let (x, y) = (f.eval(t).to_vec2().dot(n), l.eval(t).to_vec2().dot(n));
                a += x * x;
                b += x * y;
                c += y * y;
                r += x * miss;
                s += y * miss;
                samples += 1.0;
            }
        }
        let hold = samples * (HANDLE_WEIGHT * self.options.vertical_stem.max(self.options.horizontal_stem)).powi(2);
        let (a, c) = (a + hold, c + hold);
        let det = a * c - b * b;
        if det.is_finite() && det > 0.0 {
            let x = ((c * r - b * s) / det).clamp(-MAX_HANDLE_CHANGE, MAX_HANDLE_CHANGE);
            let y = ((a * s - b * r) / det).clamp(-MAX_HANDLE_CHANGE, MAX_HANDLE_CHANGE);
            if let Some(m) = moves.first_mut() {
                *m += x * first_handle;
            }
            if let Some(m) = moves.last_mut() {
                *m += y * last_handle;
            }
        }
        Some(moves)
    }
}

/// The movement of a segment's points as a curve of the segment's kind: at each parameter, how
/// far the point of the segment moves.
fn movement(seg: &Seg<'_>, from: Vec2, handles: &[Vec2], to: Vec2) -> Vec<Piece> {
    let ctrl: Vec<Node> = handles.iter().map(|v| Node::off(v.to_point())).collect();
    Seg { from: from.to_point(), ctrl: &ctrl, to: to.to_point(), kind: seg.kind }.pieces()
}

fn direction_at(piece: &Piece, t: f64) -> Vec2 {
    let (a, b, c) = piece.deriv_coeffs();
    (a * t + b) * t + c
}

/// The direction of the first handle that is not on the segment's start, else of the chord.
fn start_direction(seg: &Seg<'_>) -> Option<Vec2> {
    seg.ctrl.iter().map(|n| n.point).chain([seg.to]).map(|p| p - seg.from).find(|v| v.hypot() > DIRECTION_EPSILON)
}

fn end_direction(seg: &Seg<'_>) -> Option<Vec2> {
    seg.ctrl.iter().rev().map(|n| n.point).chain([seg.from]).map(|p| seg.to - p).find(|v| v.hypot() > DIRECTION_EPSILON)
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len.is_finite() && len > DIRECTION_EPSILON).then(|| v / len)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::contour::NodeKind;
    use crate::fixtures::rect;
    use fontcraft_geom::{Affine, BezPath, ParamCurveNearest as _};

    /// An ellipse of four cubic quarters, counter-clockwise from the right, or clockwise.
    fn ellipse(rx: f64, ry: f64, ccw: bool) -> Contour {
        let k = 0.552_284_749_8;
        let on = |x: f64, y: f64| Node { smooth: true, ..Node::new(Point::new(300.0 + rx * x, 250.0 + ry * y), NodeKind::Curve) };
        let off = |x: f64, y: f64| Node::off(Point::new(300.0 + rx * x, 250.0 + ry * y));
        let c = Contour::new(
            true,
            vec![
                on(1.0, 0.0),
                off(1.0, k),
                off(k, 1.0),
                on(0.0, 1.0),
                off(-k, 1.0),
                off(-1.0, k),
                on(-1.0, 0.0),
                off(-1.0, -k),
                off(-k, -1.0),
                on(0.0, -1.0),
                off(k, -1.0),
                off(1.0, -k),
            ],
        );
        if ccw { c } else { c.reversed().unwrap() }
    }

    /// `contours` sheared by `degrees` and, with `stems`, corrected.
    fn slanted(contours: &[Contour], degrees: f64, stems: Option<(f64, f64)>) -> Vec<Contour> {
        let shear = degrees.to_radians().tan();
        let m = Affine::new([1.0, 0.0, shear, 1.0, 0.0, 0.0]);
        let mut out = contours.to_vec();
        for n in out.iter_mut().flat_map(|c| &mut c.nodes) {
            n.point = m * n.point;
        }
        if let Some((vertical_stem, horizontal_stem)) = stems {
            for (c, n, p) in cursivy(&out, |_, _| true, CursivyOptions { shear, vertical_stem, horizontal_stem }).unwrap() {
                out[c].nodes[n].point = p;
            }
        }
        out
    }

    fn nearest(p: Point, path: &BezPath) -> f64 {
        path.segments().map(|s| s.nearest(p, 1e-6).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
    }

    /// For each curve of `upright[0]` with a point whose normal is 45° from the axes: the
    /// thickness of the stroke there, after against before.
    fn diagonals(upright: &[Contour], after: &[Contour]) -> Vec<f64> {
        let (outer, inner) = (upright[0].to_bez().unwrap(), upright[1].to_bez().unwrap());
        let (outer_after, inner_after) = (after[0].to_bez().unwrap(), after[1].to_bez().unwrap());
        let mut out = Vec::new();
        for (s, sa) in outer.segments().zip(outer_after.segments()) {
            let (fontcraft_geom::PathSeg::Cubic(c), fontcraft_geom::PathSeg::Cubic(ca)) = (s, sa) else { continue };
            let (p, pa) = (Piece::Cubic(c), Piece::Cubic(ca));
            let off = |t: f64| {
                let d = unit(direction_at(&p, t)).unwrap_or(Vec2::ZERO);
                (d.x.abs() - d.y.abs()).abs()
            };
            let t = (2..=98).map(|k| f64::from(k) / 100.0).min_by(|a, b| off(*a).total_cmp(&off(*b))).unwrap();
            if off(t) < 0.1 {
                out.push(nearest(pa.eval(t), &inner_after) / nearest(p.eval(t), &inner));
            }
        }
        out
    }

    #[test]
    fn round_strokes_keep_their_thickness_and_straight_stems_their_width() {
        for (rx, ry, v, h) in [(250.0, 250.0, 80.0, 80.0), (250.0, 260.0, 90.0, 70.0), (250.0, 250.0, 160.0, 140.0), (220.0, 360.0, 90.0, 60.0)] {
            let o = [ellipse(rx, ry, true), ellipse(rx - v, ry - h, false)];
            for degrees in [12.0, -12.0] {
                let plain = diagonals(&o, &slanted(&o, degrees, None));
                assert!(plain.len() == 4 && plain.iter().all(|r| (r - 1.0).abs() > 0.05), "{plain:?}");
                let out = slanted(&o, degrees, Some((v, h)));
                let ratios = diagonals(&o, &out);
                assert!(ratios.len() == 4 && ratios.iter().all(|r| (r - 1.0).abs() < 0.03), "{ratios:?}");
                // The top and the bottom keep their heights, and the sides their places.
                for (a, b) in slanted(&o, degrees, None).iter().zip(&out) {
                    for k in [0, 3, 6, 9] {
                        assert!((a.nodes[k].point - b.nodes[k].point).hypot() < 1e-6, "{k}");
                    }
                }
                // The same with the directions turned round.
                let turned: Vec<Contour> = o.iter().map(|c| c.reversed().unwrap()).collect();
                let ratios = diagonals(&turned, &slanted(&turned, degrees, Some((v, h))));
                assert!(ratios.iter().all(|r| (r - 1.0).abs() < 0.03), "{ratios:?}");
            }
        }
        let stem = [rect(100.0, 0.0, 180.0, 700.0)];
        assert_eq!(slanted(&stem, 12.0, Some((80.0, 70.0))), slanted(&stem, 12.0, None));
    }

    #[test]
    fn a_quadratic_o_is_corrected_too() {
        // Two off-curve nodes per quarter of a circle, on the tangents at its ends.
        let quarters = |r: f64, ccw: bool| {
            let k = 0.448;
            let (on, off) = (
                |x: f64, y: f64| Node { smooth: true, ..Node::new(Point::new(r * x, r * y), NodeKind::QCurve) },
                |x: f64, y: f64| Node::off(Point::new(r * x, r * y)),
            );
            let c = Contour::new(
                true,
                vec![
                    on(1.0, 0.0),
                    off(1.0, k),
                    off(k, 1.0),
                    on(0.0, 1.0),
                    off(-k, 1.0),
                    off(-1.0, k),
                    on(-1.0, 0.0),
                    off(-1.0, -k),
                    off(-k, -1.0),
                    on(0.0, -1.0),
                    off(k, -1.0),
                    off(1.0, -k),
                ],
            );
            if ccw { c } else { c.reversed().unwrap() }
        };
        let o = [quarters(250.0, true), quarters(170.0, false)];
        let out = slanted(&o, 12.0, Some((80.0, 80.0)));
        // The implied on-curve point in the middle of each quarter is the diagonal.
        for k in [0, 3, 6, 9] {
            let mid = |c: &Contour| c.nodes[k + 1].point.midpoint(c.nodes[k + 2].point);
            let was = nearest(mid(&o[0]), &o[1].to_bez().unwrap());
            let now = nearest(mid(&out[0]), &out[1].to_bez().unwrap());
            assert!((now / was - 1.0).abs() < 0.03, "{k}: {was} {now}");
        }
    }

    #[test]
    fn only_what_moved_is_corrected_and_hostile_input_is_refused() {
        let o = [ellipse(250.0, 250.0, true), ellipse(170.0, 170.0, false)];
        let options = CursivyOptions { shear: 0.2, vertical_stem: 80.0, horizontal_stem: 80.0 };
        // The first quarter alone: its two handles.
        let out = cursivy(&o, |c, n| c == 0 && n <= 3, options).unwrap();
        assert_eq!(out.iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(), [(0, 1), (0, 2)]);
        for bad in [
            CursivyOptions { shear: f64::NAN, ..options },
            CursivyOptions { vertical_stem: 0.0, ..options },
            CursivyOptions { horizontal_stem: f64::INFINITY, ..options },
        ] {
            assert!(cursivy(&o, |_, _| true, bad).is_err(), "{bad:?}");
        }
        // Coincident nodes, a slant next to 90° and one open path: finite results.
        let dot = Contour::new(true, vec![Node::new(Point::ZERO, NodeKind::Curve), Node::off(Point::ZERO), Node::off(Point::ZERO)]);
        let open = Contour::new(false, o[0].nodes[..4].to_vec());
        let steep = CursivyOptions { shear: 1e9, ..options };
        for (_, _, p) in cursivy(&[o[0].clone(), dot, open], |_, _| true, steep).unwrap() {
            assert!(p.is_finite());
        }
    }
}
