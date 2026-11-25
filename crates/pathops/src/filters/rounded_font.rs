//! Rounded Font (`filter.roundedFont`, `filter.roundedFont.usesVerticalStems`,
//! `filter.roundedFont.overshoot`, docs/spec/11-filters.md › Rounded Font).
//!
//! A stem ending is a straight segment between two corners that point out of the filled area,
//! with straight, parallel segments on either side, no wider across those sides than the stem
//! nearest its width (plus [`STEM_TOLERANCE`]). Each ending becomes a half circle as wide as the stem. The half
//! circle covers the same area as the square end it replaces: its centre sits `r·π/4` inside the
//! old end, so its tip overshoots the old end by `r·(1 − π/4)`, about a tenth of the stem width.
//! The sides get shorter by the same `r·π/4`.

use std::f64::consts::{FRAC_PI_4, FRAC_PI_8};

use fontcraft_geom::{Point, Vec2};

use super::round_corners::fill_sides;
use super::{Args, Context, Filter, Item, Param, ParamKind, Slot, Value};
use crate::contour::{Contour, DIRECTION_EPSILON, Node, NodeKind, RNode, Ring, angle_degrees, round_to_grid};
use crate::{MAX_COORDINATE, MAX_NODES, PathOpError};

/// An ending up to this fraction wider than the stem still counts as a stem ending.
const STEM_TOLERANCE: f64 = 0.1;
/// The two sides of an ending may differ in direction by at most this many degrees.
const PARALLEL_DEGREES: f64 = 5.0;
/// An ending cut at more than this many degrees off square (across the sides) is not rounded.
/// It leaves room for the slanted cuts of italic stems.
const MAX_CUT_DEGREES: f64 = 20.0;
/// Endings narrower than this many units are left as they are.
const MIN_WIDTH: f64 = 1.0;
/// What must remain of a side after the endings on either end of it have taken their share.
const MIN_SIDE: f64 = 0.5;

pub struct RoundedFont;

const PARAMS: &[Param] = &[Param {
    key: "stem",
    label: "Stem",
    kind: ParamKind::Number { min: 0.0, max: MAX_COORDINATE },
    default: Value::Number(0.0),
    required: false,
    slot: Slot::Position(0),
    doc: "the widest stem whose endings are rounded, in units; 0 or left out: for each ending, the master's vertical stem nearest its width",
}];

const DOC: &str = "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), stem?: units (0 to 1000000; 0 or left out: the master's vertical stems)} turns stem endings into half circles with an overshoot. A stem ending is a straight segment between two outward corners whose neighbouring segments are straight and parallel, at most the stem width (plus a tenth) across; without `stem`, the vertical stem nearest the ending's width counts. The half circle covers the area of the square end it replaces, so its tip reaches about a tenth of the stem width past the old end. Endings are taken in path order; one whose sides are too short for it stays. Without `stem` the master needs a vertical stem. Components and locked paths are not changed; new points are rounded to the grid. Export string: RoundedFont; stem (stem optional) → {glyphs: [names], skipped: [{glyph, layer?, reason}]}";

impl Filter for RoundedFont {
    fn id(&self) -> &'static str {
        "filter.roundedFont"
    }

    fn name(&self) -> &'static str {
        "RoundedFont"
    }

    fn label(&self) -> &'static str {
        "Rounded Font"
    }

    fn doc(&self) -> &'static str {
        DOC
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn apply(&self, contours: &[Contour], args: &Args, cx: &Context) -> Result<Vec<Contour>, PathOpError> {
        let stem = args.number("stem");
        if stem > 0.0 { rounded_font(contours, stem, cx.grid) } else { rounded_font_stems(contours, &master_stems(cx)?, cx.grid) }
    }

    /// The stem is written only when one is given: `RoundedFont;` runs on the master's stems.
    fn write_items(&self, args: &Args) -> Vec<Item> {
        if args.number("stem") > 0.0 { super::write_items(PARAMS, args) } else { Vec::new() }
    }
}

/// The master's vertical stems.
fn master_stems(cx: &Context) -> Result<Vec<f64>, PathOpError> {
    let stems: Vec<f64> = cx.vertical_stems.iter().copied().filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(MAX_COORDINATE)).collect();
    if stems.is_empty() {
        return Err(PathOpError::Invalid(
            "Rounded Font needs a vertical stem: give `stem`, or add a vertical stem to the master (Font Info › Masters › Stems)".into(),
        ));
    }
    Ok(stems)
}

/// The widest ending that counts as the end of the stem nearest `width` (the wider of two equally
/// near), of `stems`; 0 without stems.
fn widest_at(stems: &[f64], width: f64) -> f64 {
    let nearest = stems.iter().copied().min_by(|a, b| (a - width).abs().total_cmp(&(b - width).abs()).then(b.total_cmp(a)));
    nearest.map_or(0.0, |s| s * (1.0 + STEM_TOLERANCE))
}

/// Rounded Font on one layer's contours with endings up to `stem` units wide. See the module
/// docs for the geometry. Open contours and contours without area stay as they are.
pub fn rounded_font(contours: &[Contour], stem: f64, grid: f64) -> Result<Vec<Contour>, PathOpError> {
    rounded_font_stems(contours, &[stem], grid)
}

/// [`rounded_font`] with several stems: an ending is measured against the stem nearest its width.
pub fn rounded_font_stems(contours: &[Contour], stems: &[f64], grid: f64) -> Result<Vec<Contour>, PathOpError> {
    for stem in stems {
        if !stem.is_finite() {
            return Err(PathOpError::NotFinite);
        }
        if *stem <= 0.0 || *stem > MAX_COORDINATE {
            return Err(PathOpError::Invalid(format!("the stem ({stem}) must be more than 0 and at most {MAX_COORDINATE} units")));
        }
    }
    let total: usize = contours.iter().map(|c| c.nodes.len()).sum();
    if total > MAX_NODES {
        return Err(PathOpError::TooManyNodes(total));
    }
    let rings = contours.iter().map(Ring::from_contour).collect::<Result<Vec<_>, _>>()?;
    let sides = fill_sides(contours, &rings)?;
    let mut out = Vec::with_capacity(rings.len());
    let mut count = 0usize;
    for (i, (ring, original)) in rings.iter().zip(contours).enumerate() {
        let side = sides.get(i).copied().unwrap_or(0.0);
        let contour = match round_endings(ring, side, stems, grid) {
            Some(r) => r.to_contour(),
            None => original.clone(),
        };
        for n in &contour.nodes {
            crate::check(n.point.x)?;
            crate::check(n.point.y)?;
        }
        count = count.saturating_add(contour.nodes.len());
        if count > MAX_NODES {
            return Err(PathOpError::TooManyNodes(count));
        }
        out.push(contour);
    }
    Ok(out)
}

/// The rounding of one stem ending: the segment from node `a` to node `b`.
#[derive(Clone, Copy, Debug)]
struct Ending {
    /// Where node `a` moves to, on its incoming side.
    a: Point,
    /// Where node `b` moves to, on its outgoing side.
    b: Point,
    /// The tip of the half circle.
    tip: Point,
    /// The handles of the quarter circle from `a` to `tip`, then from `tip` to `b`.
    handles: [Point; 4],
    /// How far node `a` moves back along the incoming side, and that side's length.
    take_a: f64,
    room_a: f64,
    /// The same for node `b` and the outgoing side.
    take_b: f64,
    room_b: f64,
}

/// The ring with its stem endings rounded, or `None` when it has none. `side`: as in
/// [`fill_sides`].
fn round_endings(ring: &Ring, side: f64, stems: &[f64], grid: f64) -> Option<Ring> {
    let m = ring.len();
    if !ring.closed || side == 0.0 || m < 4 {
        return None;
    }
    // Two endings that share a corner cannot both be rounded, and the endings at both ends of a
    // side must leave something of it.
    let mut used = vec![false; m];
    let mut taken = vec![0.0f64; m];
    let mut endings: Vec<Option<Ending>> = vec![None; m];
    let mut found = false;
    for b in 0..m {
        let (Some(a), Some(next)) = (ring.prev(b), ring.next(b)) else { continue };
        if used.get(a).copied().unwrap_or(true) || used.get(b).copied().unwrap_or(true) {
            continue;
        }
        let Some(e) = ending(ring, a, b, side, stems) else { continue };
        let (Some(&ta), Some(&tb)) = (taken.get(a), taken.get(next)) else { continue };
        if ta + e.take_a > e.room_a - MIN_SIDE || tb + e.take_b > e.room_b - MIN_SIDE {
            continue;
        }
        for (j, take) in [(a, e.take_a), (next, e.take_b)] {
            if let Some(t) = taken.get_mut(j) {
                *t += take;
            }
        }
        for j in [a, b] {
            if let Some(u) = used.get_mut(j) {
                *u = true;
            }
        }
        if let Some(slot) = endings.get_mut(b) {
            *slot = Some(e);
        }
        found = true;
    }
    if !found {
        return None;
    }
    let round = |p: Point| round_to_grid(p, grid);
    let starts: Vec<Option<Point>> = (0..m).map(|a| ring.next(a).and_then(|b| endings.get(b).copied().flatten()).map(|e| e.a)).collect();
    let mut nodes: Vec<RNode> = Vec::with_capacity(m + 2 * endings.iter().flatten().count());
    // A tip before node 0 goes at the end of the list, so the contour keeps its start node.
    let mut last: Option<RNode> = None;
    for (j, r) in ring.nodes.iter().enumerate() {
        if let Some(e) = endings.get(j).copied().flatten() {
            let [h0, h1, h2, h3] = e.handles.map(|h| Node::off(round(h)));
            let tip = RNode::new(Node { point: round(e.tip), kind: NodeKind::Curve, smooth: true, tag: None }, vec![h0, h1]);
            if j == 0 {
                last = Some(tip);
            } else {
                nodes.push(tip);
            }
            nodes.push(RNode { node: Node { point: round(e.b), kind: NodeKind::Curve, smooth: true, ..r.node }, ctrl: vec![h2, h3], ..r.clone() });
        } else if let Some(p) = starts.get(j).copied().flatten() {
            nodes.push(RNode { node: Node { point: round(p), smooth: true, ..r.node }, ..r.clone() });
        } else {
            nodes.push(r.clone());
        }
    }
    nodes.extend(last);
    Some(Ring { closed: true, nodes })
}

/// The rounding of the segment from node `a` to node `b` if it is a stem ending, else `None`.
fn ending(ring: &Ring, a: usize, b: usize, side: f64, stems: &[f64]) -> Option<Ending> {
    if ring.nodes.get(a)?.node.smooth || ring.nodes.get(b)?.node.smooth {
        return None;
    }
    let (end, before, after) = (ring.seg(b)?, ring.seg(a)?, ring.seg(ring.next(b)?)?);
    if [&end, &before, &after].iter().any(|s| s.kind != NodeKind::Line || s.is_curve()) {
        return None;
    }
    let (pa, pb) = (end.from, end.to);
    let across = pb - pa;
    let (room_a, room_b) = (before.from.distance(before.to), after.from.distance(after.to));
    if room_a <= DIRECTION_EPSILON || room_b <= DIRECTION_EPSILON {
        return None;
    }
    // `t1` runs along the incoming side towards `a`, `t2` along the outgoing side away from `b`.
    let t1 = (before.to - before.from) / room_a;
    let t2 = (after.to - after.from) / room_b;
    if angle_degrees(t1, -t2) > PARALLEL_DEGREES {
        return None;
    }
    if t1.cross(across) * side <= 0.0 || across.cross(t2) * side <= 0.0 {
        return None;
    }
    // The stem's direction, pointing out through the ending.
    let d = (t1 - t2).normalize();
    let width = across.cross(d).abs();
    if !(MIN_WIDTH..=widest_at(stems, width)).contains(&width) {
        return None;
    }
    let (sa, sb) = (pa.to_vec2().dot(d), pb.to_vec2().dot(d));
    if (sa - sb).abs() > width * MAX_CUT_DEGREES.to_radians().tan() {
        return None;
    }
    let r = width / 2.0;
    // The half circle's centre line: r·π/4 inside the middle of the old end.
    let centre = (sa + sb) / 2.0 - r * FRAC_PI_4;
    let take_a = (sa - centre) / t1.dot(d);
    let take_b = (sb - centre) / (-t2).dot(d);
    if !(take_a.is_finite() && take_b.is_finite() && take_a > 0.0 && take_b > 0.0) {
        return None;
    }
    let (a2, b2) = (pa - t1 * take_a, pb + t2 * take_b);
    let chord = b2 - a2;
    let half = chord.hypot() / 2.0;
    if half < MIN_WIDTH / 2.0 {
        return None;
    }
    let u = chord / (2.0 * half);
    let out: Vec2 = (d - u * d.dot(u)).normalize();
    let tip = a2.midpoint(b2) + out * half;
    let k = 4.0 / 3.0 * FRAC_PI_8.tan() * half;
    Some(Ending { a: a2, b: b2, tip, handles: [a2 + out * k, tip - u * k, tip + u * k, b2 + out * k], take_a, room_a, take_b, room_b })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::fixtures::rect;

    fn poly(points: &[(f64, f64)]) -> Contour {
        Contour::new(true, points.iter().map(|&(x, y)| Node::new(Point::new(x, y), NodeKind::Line)).collect())
    }

    fn on_curve(c: &Contour) -> Vec<Point> {
        c.nodes.iter().filter(|n| n.on_curve()).map(|n| n.point).collect()
    }

    fn close(a: Point, b: (f64, f64)) -> bool {
        a.distance(Point::new(b.0, b.1)) < 1e-6
    }

    #[test]
    fn a_bar_gets_half_round_ends_that_keep_its_area() {
        let bar = rect(0.0, 0.0, 80.0, 700.0).tagged();
        let out = rounded_font(std::slice::from_ref(&bar), 80.0, 0.0).unwrap();
        let c = &out[0];
        let (inset, over) = (40.0 * FRAC_PI_4, 40.0 * (1.0 - FRAC_PI_4));
        let points = on_curve(c);
        let expected = [(0.0, inset), (40.0, -over), (80.0, inset), (80.0, 700.0 - inset), (40.0, 700.0 + over), (0.0, 700.0 - inset)];
        assert_eq!(points.len(), 6, "{points:?}");
        for (p, e) in points.iter().zip(expected) {
            assert!(close(*p, e), "{p:?} is not {e:?}");
        }
        assert_eq!(c.nodes.len(), 14);
        // The contour keeps its start node, now smooth, with its tag.
        assert!(c.nodes[0].smooth && c.nodes[0].kind == NodeKind::Line && c.nodes[0].tag == Some(0));
        let area = Ring::from_contour(c).unwrap().area();
        // The cubic quarter circles hold a little more than true ones.
        assert!((area - 56_000.0).abs() < 5.0, "{area}");
        // On the grid every new point is whole.
        let out = rounded_font(std::slice::from_ref(&bar), 80.0, 1.0).unwrap();
        assert!(out[0].nodes.iter().all(|n| n.point.x.fract() == 0.0 && n.point.y.fract() == 0.0));
        // Drawn the other way round, the same endings are rounded.
        let reversed = rounded_font(&[bar.reversed().unwrap()], 80.0, 0.0).unwrap();
        assert_eq!(on_curve(&reversed[0]).len(), 6);
    }

    #[test]
    fn only_endings_up_to_the_stem_width_between_outward_corners_are_rounded() {
        // An L whose arm ends are 50 wide; the inner corner at (50, 50) is no ending.
        let l = poly(&[(0.0, 0.0), (200.0, 0.0), (200.0, 50.0), (50.0, 50.0), (50.0, 200.0), (0.0, 200.0)]);
        let out = rounded_font(std::slice::from_ref(&l), 50.0, 0.0).unwrap();
        let points = on_curve(&out[0]);
        assert_eq!(points.len(), 8, "{points:?}");
        assert!(points.iter().any(|p| close(*p, (0.0, 0.0))) && points.iter().any(|p| close(*p, (50.0, 50.0))));
        // Ends wider than the stem stay square.
        assert_eq!(rounded_font(std::slice::from_ref(&l), 40.0, 0.0).unwrap()[0], l);
        // A counter has no endings, and an open path stays.
        let counter = rect(10.0, 10.0, 60.0, 190.0).reversed().unwrap();
        let open = Contour::new(false, rect(0.0, 0.0, 50.0, 300.0).nodes);
        let out = rounded_font(&[rect(0.0, 0.0, 70.0, 200.0), counter.clone(), open.clone()], 50.0, 0.0).unwrap();
        assert_eq!(out[1], counter);
        assert_eq!(out[2], open);
    }

    #[test]
    fn an_ending_is_measured_against_the_stem_nearest_its_width() {
        let stems = [60.0, 100.0];
        // 64 is the end of a 60 stem (up to 66) and 98 of a 100 stem; 72 is too wide for the stem
        // nearest it.
        for (width, rounded) in [(50.0, true), (64.0, true), (72.0, false), (98.0, true), (109.0, true), (111.0, false)] {
            let bar = rect(0.0, 0.0, width, 700.0);
            let out = rounded_font_stems(std::slice::from_ref(&bar), &stems, 0.0).unwrap();
            assert_eq!(out[0] != bar, rounded, "{width}");
        }
    }

    #[test]
    fn endings_share_their_sides_and_need_room_on_them() {
        let out = rounded_font(&[rect(0.0, 0.0, 80.0, 80.0)], 80.0, 0.0).unwrap();
        assert_eq!(on_curve(&out[0]).len(), 6);
        // A stem whose sides are too short for its rounding stays.
        let spike = poly(&[(40.0, -200.0), (80.0, 0.0), (80.0, 20.0), (0.0, 20.0), (0.0, 0.0)]);
        assert_eq!(rounded_font(std::slice::from_ref(&spike), 80.0, 0.0).unwrap()[0], spike);
    }

    #[test]
    fn a_slanted_stem_is_rounded_across_its_width() {
        // An italic stem: sides leaning 10 degrees, ends horizontal.
        let lean = 700.0 * 10f64.to_radians().tan();
        let stem = poly(&[(0.0, 0.0), (80.0, 0.0), (80.0 + lean, 700.0), (lean, 700.0)]);
        let out = rounded_font(&[stem], 80.0, 0.0).unwrap();
        let points = on_curve(&out[0]);
        assert_eq!(points.len(), 6);
        // The tip lies past the old end.
        assert!(points[1].y < 0.0 && points[4].y > 700.0, "{points:?}");
    }

    #[test]
    fn a_missing_stem_and_hostile_stems_are_errors() {
        let bar = [rect(0.0, 0.0, 80.0, 700.0)];
        let f = RoundedFont;
        let args = Args::defaults(PARAMS);
        let err = f.apply(&bar, &args, &Context::default()).unwrap_err();
        assert!(err.to_string().contains("vertical stem"), "{err}");
        let nan = Context { vertical_stems: &[f64::NAN, -3.0], ..Context::default() };
        assert!(f.apply(&bar, &args, &nan).is_err());
        let stems = Context { vertical_stems: &[60.0, 80.0], ..Context::default() };
        assert_eq!(f.apply(&bar, &args, &stems).unwrap(), rounded_font(&bar, 80.0, 0.0).unwrap());
        for s in [f64::NAN, f64::INFINITY, 0.0, -1.0, MAX_COORDINATE * 2.0] {
            assert!(rounded_font(&bar, s, 0.0).is_err(), "{s}");
        }
        // A tip that would land beyond the coordinate limit is an error.
        let edge = [rect(0.0, MAX_COORDINATE - 1000.0, 80.0, MAX_COORDINATE)];
        assert!(rounded_font(&edge, 80.0, 0.0).is_err());
    }

    #[test]
    fn the_stem_is_written_only_when_given() {
        let f = RoundedFont;
        assert!(f.write_items(&Args::defaults(PARAMS)).is_empty());
        let args = f.read_items(&[Item::positional("72")]).unwrap();
        assert_eq!(args.number("stem"), 72.0);
        assert_eq!(f.write_items(&args), vec![Item::positional("72")]);
    }
}
