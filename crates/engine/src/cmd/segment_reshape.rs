//! `path.reshapeSegment`: what an `Alt`-drag on a curve segment does with the Select tool
//! (`docs/spec/03a` § 7, row `move.dragSegment`). The end nodes stay and each handle keeps its
//! direction; only the two handle lengths change.
//!
//! The solve: with the handles at `p0 + a·u1` and `p3 + b·u2`, the segment's point at `t` is
//! linear in `a` and `b`, so landing it on `to` is a 2×2 least-squares problem. It is solved
//! through the eigenvectors of its normal matrix. Along the weaker one the step is damped once its
//! singular value falls below [`MIN_SPREAD`] times the stronger one's: as the two directions
//! approach parallel, an exact solve would swing the lengths by thousands of units for a small
//! move, and the damped one changes them smoothly and leaves the point near `to`. Parallel
//! directions take the smallest change of lengths that gets nearest.
//!
//! Lengths are held between [`MIN_HANDLE_LENGTH`] (or the grid step, if larger, so rounding does
//! not put a handle on its node) and [`MAX_HANDLE_RATIO`] times the segment's span, and within
//! the coordinate limit. A length already below the minimum may stay there. When one length is
//! held, the other is solved again with it fixed; when both are, the change of both is scaled
//! back together to where the first reaches its limit.

use fontcraft_geom::{Point, Vec2, round_point};
use fontcraft_model::{MAX_COORDINATE, NodeKind, Path};
use serde_json::{Value, json};

use super::common::{grid, in_range};
use super::*;
use crate::params::{Params, TargetKind as K, Targets};

/// The shortest handle the solve leaves, in font units, so a handle never collapses onto its
/// node or turns round through it.
pub const MIN_HANDLE_LENGTH: f64 = 1.0;
/// The longest handle the solve leaves, as a multiple of the larger of the distance between the
/// end nodes and the current handle lengths. Near the ends of the segment a small move of the
/// point needs very long handles; this keeps them in reach.
pub const MAX_HANDLE_RATIO: f64 = 10.0;
/// `t` must lie this far inside the segment: at its ends the point does not depend on the
/// handles.
pub const MIN_T: f64 = 0.01;
/// The ratio of the solve's weaker to its stronger singular value below which the step along the
/// weaker one is damped.
pub const MIN_SPREAD: f64 = 0.1;

const EPS: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.reshapeSegment",
            "Reshape Segment",
            [],
            None,
            "{glyph: name, layer?: id, side?, segment: [shape, node], t: number, to: [x, y]} changes the lengths of the two handles of the cubic segment that ends at `node`, keeping both end nodes and each handle's direction, so that the segment's point at `t` (0.01 to 0.99, from the start node) lands on `to`, or as near as the directions allow. Each handle stays between 1 unit (or the grid step, if larger) and 10 times the larger of the end nodes' distance and the current handle lengths, and within the coordinate limit; a handle already shorter may stay so. With nearly parallel handles the point moves only as far as moderate length changes take it. A straight segment is left as it is and returns `changed: false`; a quadratic segment is refused. The selection afterwards is the segment's two end nodes → {changed, selection}",
            has_doc,
            reshape_segment
        )
        .targets(&[K::Layer, K::Segments]),
    ]
}

fn reshape_segment(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.reshapeSegment", v)?;
    let t = Targets::parse(&p)?;
    let &[(shape, end)] = t.segments.as_slice() else {
        return Err(p.bad("give one `segment`: [shape, node], the node that ends it"));
    };
    if !t.points.is_empty() || !t.shapes.is_empty() || !t.anchors.is_empty() || !t.guides.is_empty() || !t.annotations.is_empty() {
        return Err(p.bad("only `segment` names what to reshape"));
    }
    let at = p.f64("t")?;
    if !(MIN_T..=1.0 - MIN_T).contains(&at) {
        return Err(p.bad(format!("`t` must be from {MIN_T} to {}, got {at}", 1.0 - MIN_T)));
    }
    let to = target_point(&p)?;
    let (start, handles, step) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        let path = r.parts.path(shape).ok_or_else(|| p.bad(format!("shape {shape} is not a path")))?;
        let seg = path.segment_ending_at(end).ok_or_else(|| p.bad(format!("node {end} of path {shape} does not end a segment")))?;
        (seg.start, cubic_handles(path, &seg.handles, end), grid(font))
    };
    let mut selection = Targets::new("path.reshapeSegment", t.glyph.clone(), t.layer.clone());
    selection.side = t.side;
    selection.points = vec![(shape, start), (shape, end)];
    let Some((h1, h2)) = handles.map_err(|m| p.bad(m))? else {
        return Ok(json!({"changed": false, "selection": selection.to_json()}));
    };
    s.edit("Reshape Segment", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let path = parts.path_mut(shape).ok_or_else(|| p.bad(format!("shape {shape} is not a path")))?;
            let point = |i: usize| path.nodes.get(i).map(|n| n.point());
            let (Some(p0), Some(p1), Some(p2), Some(p3)) = (point(start), point(h1), point(h2), point(end)) else {
                return Err(p.bad("the segment's points do not resolve"));
            };
            let (n1, n2) = solve([p0, p1, p2, p3], at, to, step).ok_or_else(|| p.bad("the segment has no handle directions to keep"))?;
            let (n1, n2) = (round_point(n1, step), round_point(n2, step));
            if ![n1.x, n1.y, n2.x, n2.y].into_iter().all(in_range) {
                return Err(p.bad("the reshaped handles would lie beyond the coordinate limit"));
            }
            for (i, q) in [(h1, n1), (h2, n2)] {
                if let Some(node) = path.nodes.get_mut(i) {
                    (node.x, node.y) = (q.x, q.y);
                }
            }
            Ok(())
        })
    })?;
    Ok(json!({"changed": true, "selection": selection.to_json()}))
}

/// `to` as `[x, y]` within the coordinate limit.
fn target_point(p: &Params) -> Result<Point> {
    let bad = || p.bad("`to` must be [x, y]: two finite numbers within the coordinate limit");
    let list = p.array("to")?;
    let [x, y] = list.as_slice() else { return Err(bad()) };
    let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else { return Err(bad()) };
    if !(in_range(x) && in_range(y)) {
        return Err(bad());
    }
    Ok(Point::new(x, y))
}

/// The two handles of a cubic segment, `None` for a straight one, and an error for any other.
fn cubic_handles(path: &Path, handles: &[usize], end: usize) -> std::result::Result<Option<(usize, usize)>, String> {
    match handles {
        [] => Ok(None),
        &[h1, h2] if path.nodes.get(end).is_some_and(|n| n.kind == NodeKind::Curve) => Ok(Some((h1, h2))),
        _ => Err("only cubic curve segments can be reshaped; this one is quadratic".into()),
    }
}

/// The unit vector along the first of `candidates` that is not zero.
fn direction(candidates: [Vec2; 3]) -> Option<Vec2> {
    candidates.into_iter().find_map(|v| {
        let l = v.hypot();
        (l > EPS && l.is_finite()).then(|| v / l)
    })
}

/// The longest handle from `from` along the unit vector `u` whose end stays `margin` inside the
/// coordinate limit.
fn room(from: Point, u: Vec2, margin: f64) -> f64 {
    let limit = (MAX_COORDINATE - margin).max(0.0);
    let axis = |p: f64, d: f64| if d.abs() <= EPS { f64::INFINITY } else { ((limit - p * d.signum()) / d.abs()).max(0.0) };
    axis(from.x, u.x).min(axis(from.y, u.y))
}

/// The new handles of the cubic `[p0, p1, p2, p3]` that put its point at `t` on `to` (or near
/// it), keeping each handle's direction, for a grid of `step`. A handle that lies on its node
/// takes the direction towards the other handle, else towards the other end. `None` when no
/// direction exists (all four points equal).
pub(crate) fn solve([p0, p1, p2, p3]: [Point; 4], t: f64, to: Point, step: f64) -> Option<(Point, Point)> {
    let u1 = direction([p1 - p0, p2 - p0, p3 - p0])?;
    let u2 = direction([p2 - p3, p1 - p3, p0 - p3])?;
    let (a0, b0) = ((p1 - p0).hypot(), (p2 - p3).hypot());
    let mt = 1.0 - t;
    let (m1, m2) = (u1 * (3.0 * mt * mt * t), u2 * (3.0 * mt * t * t));
    let current = p0.to_vec2() * (mt * mt * mt) + p1.to_vec2() * (3.0 * mt * mt * t) + p2.to_vec2() * (3.0 * mt * t * t) + p3.to_vec2() * (t * t * t);
    let r = to.to_vec2() - current;
    let moved = r.hypot();
    if moved.is_nan() || moved <= EPS {
        return Some((p1, p2));
    }
    // The normal matrix [[m1·m1, m1·m2], [m1·m2, m2·m2]], its eigenvalues (the squared singular
    // values) and unit eigenvectors.
    let (s11, s12, s22) = (m1.hypot2(), m1.dot(m2), m2.hypot2());
    let half = (s11 + s22) / 2.0;
    let spread = (((s11 - s22) / 2.0).powi(2) + s12 * s12).sqrt();
    let (strong, weak) = (half + spread, (half - spread).max(0.0));
    if strong.is_nan() || strong <= EPS {
        return None;
    }
    let v1 = if s12.abs() > EPS * strong {
        let v = Vec2::new(strong - s22, s12);
        v / v.hypot()
    } else if s11 >= s22 {
        Vec2::new(1.0, 0.0)
    } else {
        Vec2::new(0.0, 1.0)
    };
    let v2 = Vec2::new(-v1.y, v1.x);
    let g = Vec2::new(m1.dot(r), m2.dot(r));
    // Along the weaker eigenvector, weak / (weak² + damping²) instead of 1 / weak: the same while
    // the two singular values are of a size, bounded as the weaker one goes to zero.
    let damping = MIN_SPREAD * MIN_SPREAD * strong;
    let d = v1 * (v1.dot(g) / strong) + v2 * (v2.dot(g) * weak / (weak * weak + damping * damping));
    let (da, db) = (d.x, d.y);
    let shortest = MIN_HANDLE_LENGTH.max(if step.is_finite() { step } else { 0.0 });
    let longest = MAX_HANDLE_RATIO * (p3 - p0).hypot().max(a0).max(b0).max(MIN_HANDLE_LENGTH);
    let margin = if step.is_finite() { step.max(0.0) } else { 0.0 };
    let bounds = |l0: f64, from: Point, u: Vec2| {
        let hi = longest.min(room(from, u, margin));
        (shortest.min(l0).min(hi), hi)
    };
    let ((alo, ahi), (blo, bhi)) = (bounds(a0, p0, u1), bounds(b0, p3, u2));
    let (a, b) = (a0 + da, b0 + db);
    let fits = |l: f64, (lo, hi): (f64, f64)| l.is_finite() && l >= lo && l <= hi;
    let hold = |l: f64, (lo, hi): (f64, f64)| if l.is_finite() { l.clamp(lo, hi) } else { lo };
    let (ha, hb) = match (fits(a, (alo, ahi)), fits(b, (blo, bhi))) {
        (true, true) => (a, b),
        // One length held at a limit: solve the other alone, nearest to `to`.
        (false, true) => {
            let ha = hold(a, (alo, ahi));
            let hb = if s22 > EPS { b0 + m2.dot(r - m1 * (ha - a0)) / s22 } else { b };
            (ha, hold(hb, (blo, bhi)))
        }
        (true, false) => {
            let hb = hold(b, (blo, bhi));
            let ha = if s11 > EPS { a0 + m1.dot(r - m2 * (hb - b0)) / s11 } else { a };
            (hold(ha, (alo, ahi)), hb)
        }
        // Both held: scale the change back together to where the first one reaches its limit.
        (false, false) => {
            let reach = |l0: f64, dl: f64, (lo, hi): (f64, f64)| {
                let limit = if dl > 0.0 { hi } else { lo };
                if dl.abs() <= EPS { 1.0 } else { ((limit - l0) / dl).clamp(0.0, 1.0) }
            };
            let k = reach(a0, da, (alo, ahi)).min(reach(b0, db, (blo, bhi)));
            (hold(a0 + da * k, (alo, ahi)), hold(b0 + db * k, (blo, bhi)))
        }
    };
    Some((p0 + u1 * ha, p3 + u2 * hb))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn session(nodes: Value) -> Session {
        let mut s = Session::with_document();
        run(&mut s, "path.addPath", json!({"glyph": "A", "closed": false, "nodes": nodes}));
        s
    }

    fn points(s: &Session) -> Vec<Point> {
        let font = s.font().unwrap();
        let master = font.default_master().unwrap().id.clone();
        match font.glyph("A").unwrap().layer(&master).unwrap().shapes.last() {
            Some(fontcraft_model::Shape::Path(p)) => p.nodes.iter().map(|n| n.point()).collect(),
            other => panic!("not a path: {other:?}"),
        }
    }

    fn eval(p: &[Point], t: f64) -> Point {
        let mt = 1.0 - t;
        let v = p[0].to_vec2() * (mt * mt * mt)
            + p[1].to_vec2() * (3.0 * mt * mt * t)
            + p[2].to_vec2() * (3.0 * mt * t * t)
            + p[3].to_vec2() * (t * t * t);
        v.to_point()
    }

    /// A cubic from (0,0) to (300,300) whose first handle points up and second points left.
    fn bend() -> Value {
        json!([
            {"x": 0, "y": 0},
            {"x": 0, "y": 100, "type": "offcurve"}, {"x": 200, "y": 300, "type": "offcurve"}, {"x": 300, "y": 300, "type": "curve"},
        ])
    }

    #[test]
    fn reshaping_puts_the_point_at_t_on_to_and_keeps_the_directions_and_nodes() {
        let mut s = session(bend());
        let before = points(&s);
        assert_eq!(eval(&before, 0.5), Point::new(112.5, 187.5));
        let out = run(&mut s, "path.reshapeSegment", json!({"glyph": "A", "segment": [0, 3], "t": 0.5, "to": [100, 200]}));
        assert_eq!(out["changed"], json!(true));
        assert_eq!(out["selection"]["points"], json!([[0, 0], [0, 3]]));
        let after = points(&s);
        assert_eq!((after[0], after[3]), (before[0], before[3]), "the end nodes stay");
        assert_eq!(after[1].x, 0.0, "the first handle still points up");
        assert_eq!(after[2].y, 300.0, "the second handle still points left");
        let hit = eval(&after, 0.5);
        assert!((hit - Point::new(100.0, 200.0)).hypot() < 1.0, "the point at t is {hit:?}");

        // One undo step restores the segment.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(points(&s), before);
    }

    #[test]
    fn parallel_handles_change_together_and_lengths_stay_positive() {
        // A hump: both handles point up, so the system is singular.
        let hump = json!([
            {"x": 0, "y": 0},
            {"x": 0, "y": 100, "type": "offcurve"}, {"x": 300, "y": 100, "type": "offcurve"}, {"x": 300, "y": 0, "type": "curve"},
        ]);
        let mut s = session(hump.clone());
        run(&mut s, "path.reshapeSegment", json!({"glyph": "A", "segment": [0, 3], "t": 0.5, "to": [150, 150]}));
        let p = points(&s);
        assert_eq!((p[1], p[2]), (Point::new(0.0, 200.0), Point::new(300.0, 200.0)));

        // Pulled far below the nodes, the handles shrink to the shortest length, not through the node.
        let mut s = session(hump);
        run(&mut s, "path.reshapeSegment", json!({"glyph": "A", "segment": [0, 3], "t": 0.5, "to": [150, -400]}));
        let p = points(&s);
        assert_eq!((p[1], p[2]), (Point::new(0.0, MIN_HANDLE_LENGTH), Point::new(300.0, MIN_HANDLE_LENGTH)));
    }

    #[test]
    fn nearly_parallel_handles_change_smoothly_and_the_point_stays_near_to() {
        // Both handles point up; the second leans 1 unit in 100 (sin θ ≈ 0.01). A sideways move
        // asks for lengths an exact solve would swing by thousands of units.
        let pts = [Point::new(0.0, 0.0), Point::new(0.0, 100.0), Point::new(301.0, 100.0), Point::new(300.0, 0.0)];
        let at = eval(&pts, 0.5);
        let mut last: Option<(f64, f64)> = None;
        for k in 0..=10 {
            let to = at + Vec2::new(f64::from(k), 0.0);
            let (n1, n2) = solve(pts, 0.5, to, 1.0).unwrap();
            let (a, b) = ((n1 - pts[0]).hypot(), (n2 - pts[3]).hypot());
            assert!((a - 100.0).abs() < 50.0 && (b - 100.0).abs() < 50.0, "{k}: {a}, {b}");
            let hit = eval(&[pts[0], n1, n2, pts[3]], 0.5);
            assert!((hit - to).hypot() <= f64::from(k) + 1e-6, "{k}: the point moved away from `to`: {hit:?}");
            if let Some((pa, pb)) = last {
                assert!((a - pa).abs() < 10.0 && (b - pb).abs() < 10.0, "{k}: a jump from {pa}, {pb} to {a}, {b}");
            }
            last = Some((a, b));
        }
        // Along the handles the point follows.
        let to = at + Vec2::new(0.0, 30.0);
        let (n1, n2) = solve(pts, 0.5, to, 1.0).unwrap();
        assert!((eval(&[pts[0], n1, n2, pts[3]], 0.5) - to).hypot() < 0.5);
    }

    #[test]
    fn lines_are_left_alone_and_bad_parameters_are_named() {
        let mut s = session(json!([{"x": 0, "y": 0}, {"x": 300, "y": 0}]));
        let undo = |s: &Session| s.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len);
        let steps = undo(&s);
        let out = run(&mut s, "path.reshapeSegment", json!({"glyph": "A", "segment": [0, 1], "t": 0.5, "to": [150, 100]}));
        assert_eq!(out["changed"], json!(false));
        assert_eq!(undo(&s), steps, "no undo step for a line");

        let mut s = session(bend());
        let before = points(&s);
        let bad = [
            (json!({"glyph": "A", "segment": [0, 3], "t": 1.5, "to": [100, 200]}), "`t`"),
            (json!({"glyph": "A", "segment": [0, 3], "t": 0.5, "to": [100]}), "`to`"),
            (json!({"glyph": "A", "segment": [0, 3], "t": 0.5}), "`to`"),
            (json!({"glyph": "A", "t": 0.5, "to": [100, 200]}), "`segment`"),
            (json!({"glyph": "A", "segment": [0, 1], "t": 0.5, "to": [100, 200]}), "segment"),
            (json!({"glyph": "A", "segment": [0, 3], "t": 0.5, "to": [100, 2e9]}), "`to`"),
        ];
        for (params, word) in bad {
            let e = s.execute("path.reshapeSegment", &params).expect_err("refused").to_string();
            assert!(e.contains(word), "{params}: {e}");
        }
        assert_eq!(points(&s), before, "a refusal changes nothing");

        let quad = json!([{"x": 0, "y": 0}, {"x": 150, "y": 200, "type": "offcurve"}, {"x": 300, "y": 0, "type": "qcurve"}]);
        let mut s = session(quad);
        let e = s.execute("path.reshapeSegment", &json!({"glyph": "A", "segment": [0, 2], "t": 0.5, "to": [150, 150]})).expect_err("quadratic");
        assert!(e.to_string().contains("cubic"), "{e}");
    }
}
