//! Curve tension and harmonizing of cubic segments.
//!
//! **Tension** is the length of a handle as a percentage of the distance from its node to the
//! Tunni point, where the lines through the two handles of a segment meet ahead of both nodes. A
//! quarter circle drawn with the usual handle factor has a tension of about 55.23 % on both
//! sides. A segment whose handle lines are parallel or meet behind a node, or that has a
//! zero-length handle, has no Tunni point and so no tension.
//!
//! **Harmonizing** acts on a smooth node between two cubic segments (a [`Joint`]). The curvature
//! of a cubic at its end node depends only on the handle length there and on how far the next
//! handle in stands off the tangent line (`κ = 2h / 3a²`), so:
//! - [`harmonize_node`] keeps the handles and slides the node along the line between them to the
//!   point where both curvatures are equal (G2);
//! - [`harmonize_handles`] keeps the node and splits the two handle lengths, their sum unchanged,
//!   so both curvatures are equal (G2). Order G3 then also equalises the rate of change of
//!   curvature along the outline by changing the lengths of the two far handles along their own
//!   directions, so the neighbouring nodes keep their tangents. G2 holds throughout.
//!
//! Curvature is compared by size: at an inflection (the far handles on opposite sides of the
//! tangent) the two sides get curvatures of equal size and opposite sign.

use fontcraft_geom::{CubicBez, Point, Vec2};

/// Lengths and heights below this, in font units, count as zero.
const EPS: f64 = 1e-9;
/// Most Newton steps [`harmonize_handles`] takes for order G3.
const MAX_G3_STEPS: usize = 32;
/// G3 stops when the two rates of curvature change differ by less than this share of the larger.
const G3_TOLERANCE: f64 = 1e-9;
/// G3 refuses a far handle that would have to shrink or grow past this factor.
const G3_MAX_FACTOR: f64 = 100.0;

/// The Tunni point of `c`: where the lines through its handles meet, ahead of both nodes.
pub fn tunni_point(c: CubicBez) -> Option<Point> {
    let (s, _, u1, _) = tunni_distances(c)?;
    Some(c.p0 + u1 * s)
}

/// The tension of both handles of `c`, in percent of the distance from each node to the Tunni
/// point.
pub fn tension(c: CubicBez) -> Option<(f64, f64)> {
    let (s, r, _, _) = tunni_distances(c)?;
    let a = (c.p1 - c.p0).hypot() * 100.0 / s;
    let b = (c.p2 - c.p3).hypot() * 100.0 / r;
    (a.is_finite() && b.is_finite()).then_some((a, b))
}

/// `c` with its handles moved along their own lines to the tensions `first` and `second` (in
/// percent). `None` when `c` has no Tunni point or a tension is not finite.
pub fn with_tension(c: CubicBez, first: f64, second: f64) -> Option<CubicBez> {
    if !(first.is_finite() && second.is_finite()) {
        return None;
    }
    let (s, r, u1, u2) = tunni_distances(c)?;
    let out = CubicBez::new(c.p0, c.p0 + u1 * (s * first / 100.0), c.p3 + u2 * (r * second / 100.0), c.p3);
    finite_cubic(out).then_some(out)
}

/// The distances from each node to the Tunni point and the two handle directions.
fn tunni_distances(c: CubicBez) -> Option<(f64, f64, Vec2, Vec2)> {
    let u1 = unit(c.p1 - c.p0)?;
    let u2 = unit(c.p2 - c.p3)?;
    let chord = c.p3 - c.p0;
    let cross = u1.cross(u2);
    if cross.abs() <= EPS {
        return None;
    }
    let s = chord.cross(u2) / cross;
    let r = -u1.cross(chord) / cross;
    (s > EPS && r > EPS && s.is_finite() && r.is_finite()).then_some((s, r, u1, u2))
}

/// Signed curvature at one end of a segment and its rate of change per unit of length, both
/// measured in the segment's own direction (positive curvature turns left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curvature {
    pub curvature: f64,
    pub rate: f64,
}

/// Curvature of `c` at its first node. `None` when the first handle has no length.
pub fn curvature_at_start(c: CubicBez) -> Option<Curvature> {
    let d1 = (c.p1 - c.p0) * 3.0;
    let d2 = ((c.p2 - c.p1) - (c.p1 - c.p0)) * 6.0;
    curvature(d1, d2, third(c))
}

/// Curvature of `c` at its last node. `None` when the last handle has no length.
pub fn curvature_at_end(c: CubicBez) -> Option<Curvature> {
    let d1 = (c.p3 - c.p2) * 3.0;
    let d2 = ((c.p3 - c.p2) - (c.p2 - c.p1)) * 6.0;
    curvature(d1, d2, third(c))
}

/// The third derivative of `c`, the same everywhere along it.
fn third(c: CubicBez) -> Vec2 {
    ((c.p3 - c.p2) - (c.p2 - c.p1) * 2.0 + (c.p1 - c.p0)) * 6.0
}

/// κ = d1×d2 / |d1|³ and dκ/ds = (d1×d3 / |d1|³ − 3 (d1×d2)(d1·d2) / |d1|⁵) / |d1|.
fn curvature(d1: Vec2, d2: Vec2, d3: Vec2) -> Option<Curvature> {
    let speed = d1.hypot();
    if !(speed > EPS && speed.is_finite()) {
        return None;
    }
    let k = d1.cross(d2) / speed.powi(3);
    let dk = (d1.cross(d3) / speed.powi(3) - 3.0 * d1.cross(d2) * d1.dot(d2) / speed.powi(5)) / speed;
    (k.is_finite() && dk.is_finite()).then_some(Curvature { curvature: k, rate: dk })
}

/// A node between two cubic segments: `before` ends at it, `after` starts at it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    pub before: CubicBez,
    pub after: CubicBez,
}

impl Joint {
    pub fn node(&self) -> Point {
        self.before.p3
    }

    /// The curvature on both sides of the node.
    pub fn curvatures(&self) -> Option<(Curvature, Curvature)> {
        Some((curvature_at_end(self.before)?, curvature_at_start(self.after)?))
    }

    fn finite(&self) -> bool {
        finite_cubic(self.before) && finite_cubic(self.after)
    }
}

/// How far [`harmonize_handles`] goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Equal curvature on both sides.
    #[default]
    G2,
    /// Equal curvature and equal rate of change of curvature.
    G3,
}

/// Why a joint is left as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unchanged {
    /// The two segments do not meet at one node, or a handle at the node has no length.
    NotAJoint,
    /// One side is straight at the node (its far handle lies on the tangent) and the other is
    /// not, so no handle or node position gives both the same curvature.
    StraightSide,
    /// A far handle has no length, so it has no direction to change along (order G3).
    NoFarHandleDirection,
    /// No far handle lengths within reach equalise the rates of curvature change (order G3).
    NoSolution,
}

impl Unchanged {
    pub fn message(self) -> &'static str {
        match self {
            Self::NotAJoint => "the node is not between two curve segments with handles",
            Self::StraightSide => "one side is straight at the node and the other is not",
            Self::NoFarHandleDirection => "a neighbouring handle has no length",
            Self::NoSolution => "no handle lengths give the same rate of curvature change on both sides",
        }
    }
}

/// The handle lengths and stand-offs of a joint, measured on the line `u` through `on`.
struct Frame {
    /// The tangent direction, from the incoming handle towards the outgoing one.
    u: Vec2,
    /// How far the far handles stand off the tangent line (absolute values).
    before_height: f64,
    after_height: f64,
}

fn frame(j: &Joint, on: Point) -> Result<Frame, Unchanged> {
    let gap = (j.before.p3 - j.after.p0).hypot();
    if gap.is_nan() || gap > EPS {
        return Err(Unchanged::NotAJoint);
    }
    let u = unit(j.after.p1 - j.before.p2).ok_or(Unchanged::NotAJoint)?;
    Ok(Frame { u, before_height: u.cross(j.before.p1 - on).abs(), after_height: u.cross(j.after.p2 - on).abs() })
}

/// The share of the handle span that goes to the incoming side: the curvatures `2h / 3a²` are
/// equal when `a / b = √h₁ / √h₂`. `Ok(None)`: both sides are straight at the node already.
fn split(f: &Frame, span: f64) -> Result<Option<f64>, Unchanged> {
    let tiny = EPS * span.max(1.0);
    match (f.before_height > tiny, f.after_height > tiny) {
        (false, false) => Ok(None),
        (true, true) => {
            let (s1, s2) = (f.before_height.sqrt(), f.after_height.sqrt());
            Ok(Some(span * s1 / (s1 + s2)))
        }
        _ => Err(Unchanged::StraightSide),
    }
}

/// Moves the node along the line between its two handles, which stay put, to where the
/// curvature on both sides is equal.
pub fn harmonize_node(j: Joint) -> Result<Joint, Unchanged> {
    let (a, b) = (j.before.p2, j.after.p1);
    let span = (b - a).hypot();
    if !((j.node() - a).hypot() > EPS && (b - j.node()).hypot() > EPS) {
        return Err(Unchanged::NotAJoint);
    }
    let f = frame(&j, a)?;
    let Some(first) = split(&f, span)? else { return Ok(j) };
    let node = a + f.u * first;
    let out =
        Joint { before: CubicBez::new(j.before.p0, j.before.p1, j.before.p2, node), after: CubicBez::new(node, j.after.p1, j.after.p2, j.after.p3) };
    if out.finite() { Ok(out) } else { Err(Unchanged::NoSolution) }
}

/// Changes the lengths of the two handles at the node, which stays put, so the curvature on both
/// sides is equal; with [`Order::G3`] the far handles also change length so the rate of
/// curvature change is equal too.
pub fn harmonize_handles(j: Joint, order: Order) -> Result<Joint, Unchanged> {
    let p = j.node();
    let a0 = (p - j.before.p2).hypot();
    let b0 = (j.after.p1 - p).hypot();
    if !(a0 > EPS && b0 > EPS) {
        return Err(Unchanged::NotAJoint);
    }
    let span = a0 + b0;
    let g2 = |j: Joint| -> Result<Joint, Unchanged> {
        let f = frame(&j, p)?;
        let Some(first) = split(&f, span)? else { return Ok(j) };
        let before = CubicBez::new(j.before.p0, j.before.p1, p - f.u * first, p);
        let after = CubicBez::new(p, p + f.u * (span - first), j.after.p2, j.after.p3);
        Ok(Joint { before, after })
    };
    let out = match order {
        Order::G2 => g2(j)?,
        Order::G3 => g3(j, &g2)?,
    };
    if out.finite() { Ok(out) } else { Err(Unchanged::NoSolution) }
}

/// Order G3: finds the far handle lengths, as factors of their current lengths, with the
/// smallest change that makes the rates of curvature change equal, re-running G2 at each step.
fn g3(j: Joint, g2: &dyn Fn(Joint) -> Result<Joint, Unchanged>) -> Result<Joint, Unchanged> {
    let far_before = j.before.p1 - j.before.p0;
    let far_after = j.after.p2 - j.after.p3;
    let (len_before, len_after) = (far_before.hypot(), far_after.hypot());
    if !(len_before > EPS && len_after > EPS) {
        return Err(Unchanged::NoFarHandleDirection);
    }
    let (dir_before, dir_after) = (far_before / len_before, far_after / len_after);
    let at = |x: f64, y: f64| -> Result<Joint, Unchanged> {
        let before = CubicBez::new(j.before.p0, j.before.p0 + dir_before * (len_before * x), j.before.p2, j.before.p3);
        let after = CubicBez::new(j.after.p0, j.after.p1, j.after.p3 + dir_after * (len_after * y), j.after.p3);
        g2(Joint { before, after })
    };
    // The difference of the two rates and the size it is judged against.
    let residual = |x: f64, y: f64| -> Result<(f64, f64, Joint), Unchanged> {
        let joint = at(x, y)?;
        let (k1, k2) = joint.curvatures().ok_or(Unchanged::NoSolution)?;
        Ok((k1.rate - k2.rate, k1.rate.abs().max(k2.rate.abs()), joint))
    };
    let (mut x, mut y) = (1.0_f64, 1.0_f64);
    for _ in 0..MAX_G3_STEPS {
        let (r, scale, joint) = residual(x, y)?;
        if r.abs() <= G3_TOLERANCE * scale || scale <= EPS * EPS {
            return Ok(joint);
        }
        let h = 1e-7;
        let jx = (residual(x + h, y)?.0 - r) / h;
        let jy = (residual(x, y + h)?.0 - r) / h;
        let norm = jx * jx + jy * jy;
        if !(norm.is_finite() && norm > 0.0) {
            return Err(Unchanged::NoSolution);
        }
        x -= r * jx / norm;
        y -= r * jy / norm;
        let reach = 1.0 / G3_MAX_FACTOR..=G3_MAX_FACTOR;
        if !(reach.contains(&x) && reach.contains(&y)) {
            return Err(Unchanged::NoSolution);
        }
    }
    let (r, scale, joint) = residual(x, y)?;
    if r.abs() <= G3_TOLERANCE * scale { Ok(joint) } else { Err(Unchanged::NoSolution) }
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len > EPS && len.is_finite()).then(|| v / len)
}

fn finite_cubic(c: CubicBez) -> bool {
    [c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KAPPA: f64 = 0.552_284_749_830_793_4;

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    /// A bowl: the node at the origin, tangent along x, both far handles above the tangent.
    fn bowl() -> Joint {
        Joint {
            before: CubicBez::new(pt(-300.0, 300.0), pt(-300.0, 120.0), pt(-100.0, 0.0), pt(0.0, 0.0)),
            after: CubicBez::new(pt(0.0, 0.0), pt(250.0, 0.0), pt(400.0, 150.0), pt(400.0, 400.0)),
        }
    }

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * a.abs().max(b.abs()).max(1e-300)
    }

    #[test]
    fn quarter_circle_has_the_circle_tension() {
        let r = 500.0;
        let c = CubicBez::new(pt(r, 0.0), pt(r, KAPPA * r), pt(KAPPA * r, r), pt(0.0, r));
        let (a, b) = tension(c).unwrap();
        assert!((a - 55.228_474_983).abs() < 1e-6 && (b - a).abs() < 1e-9, "{a} {b}");
        assert_eq!(tunni_point(c), Some(pt(r, r)));
    }

    #[test]
    fn setting_tension_moves_handles_along_their_lines() {
        let c = CubicBez::new(pt(0.0, 0.0), pt(0.0, 100.0), pt(300.0, 400.0), pt(400.0, 400.0));
        let out = with_tension(c, 55.0, 55.0).unwrap();
        let (a, b) = tension(out).unwrap();
        assert!((a - 55.0).abs() < 1e-9 && (b - 55.0).abs() < 1e-9, "{a} {b}");
        assert!((out.p1.x).abs() < 1e-9 && (out.p2.y - 400.0).abs() < 1e-9, "the handles keep their directions: {out:?}");
        assert!((out.p1.y - 220.0).abs() < 1e-9, "55 % of the 400 units to the Tunni point: {out:?}");
    }

    #[test]
    fn parallel_or_diverging_handles_have_no_tension() {
        let parallel = CubicBez::new(pt(0.0, 0.0), pt(0.0, 100.0), pt(300.0, 100.0), pt(300.0, 0.0));
        assert_eq!(tension(parallel), None);
        assert_eq!(with_tension(parallel, 55.0, 55.0), None);
        let diverging = CubicBez::new(pt(0.0, 0.0), pt(-50.0, 100.0), pt(350.0, 100.0), pt(300.0, 0.0));
        assert_eq!(tunni_point(diverging), None);
        let retracted = CubicBez::new(pt(0.0, 0.0), pt(0.0, 0.0), pt(300.0, 100.0), pt(300.0, 0.0));
        assert_eq!(tension(retracted), None);
    }

    #[test]
    fn harmonized_node_has_equal_curvature_on_both_sides() {
        let j = bowl();
        let (k1, k2) = j.curvatures().unwrap();
        assert!(!close(k1.curvature, k2.curvature, 1e-3), "the fixture starts unbalanced");
        let out = harmonize_node(j).unwrap();
        let (k1, k2) = out.curvatures().unwrap();
        assert!(close(k1.curvature, k2.curvature, 1e-6), "{k1:?} {k2:?}");
        assert_eq!((out.before.p2, out.after.p1), (j.before.p2, j.after.p1), "the handles stay");
        assert!(out.node().y.abs() < 1e-12 && out.node().x > -100.0 && out.node().x < 250.0, "the node slides between them: {:?}", out.node());
    }

    #[test]
    fn harmonized_handles_keep_the_node_and_the_span() {
        let j = bowl();
        let out = harmonize_handles(j, Order::G2).unwrap();
        let (k1, k2) = out.curvatures().unwrap();
        assert!(close(k1.curvature, k2.curvature, 1e-6), "{k1:?} {k2:?}");
        assert_eq!(out.node(), j.node());
        let span = (out.after.p1 - out.before.p2).hypot();
        assert!((span - 350.0).abs() < 1e-9, "{span}");
    }

    #[test]
    fn g3_also_matches_the_rate_of_curvature_change() {
        let j = bowl();
        let out = harmonize_handles(j, Order::G3).unwrap();
        let (k1, k2) = out.curvatures().unwrap();
        assert!(close(k1.curvature, k2.curvature, 1e-6), "{k1:?} {k2:?}");
        assert!(close(k1.rate, k2.rate, 1e-6), "{k1:?} {k2:?}");
        assert_eq!(out.node(), j.node());
        // The far handles keep their directions, so the neighbouring nodes keep their tangents.
        assert!((out.before.p1.x - j.before.p1.x).abs() < 1e-9 && (out.after.p2.x - j.after.p2.x).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn a_straight_side_cannot_be_matched() {
        let mut j = bowl();
        j.after.p2 = pt(400.0, 0.0);
        assert_eq!(harmonize_node(j), Err(Unchanged::StraightSide));
        assert_eq!(harmonize_handles(j, Order::G2), Err(Unchanged::StraightSide));
    }

    #[test]
    fn non_finite_input_is_refused_not_panicked_on() {
        let mut j = bowl();
        j.before.p1 = pt(f64::NAN, 0.0);
        assert!(harmonize_node(j).is_err());
        assert!(harmonize_handles(j, Order::G3).is_err());
        assert_eq!(tension(CubicBez::new(pt(0.0, 0.0), pt(f64::INFINITY, 1.0), pt(1.0, 1.0), pt(1.0, 0.0))), None);
    }
}
