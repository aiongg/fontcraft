//! `path.reshapeSegment`: what an `Alt`-drag on a curve segment does with the Select tool
//! (`docs/spec/03a` § 7, row `move.dragSegment`). The end nodes stay and each handle keeps its
//! direction; only the two handle lengths change.
//!
//! The solve: with the handles at `p0 + a·u1` and `p3 + b·u2`, the segment's point at `t` is
//! linear in `a` and `b`, so landing it on `to` is a 2×2 linear system. When the two directions
//! are parallel the system is singular and the smallest change of lengths that gets nearest to
//! `to` is taken instead. Lengths are held between [`MIN_HANDLE_LENGTH`] and
//! [`MAX_HANDLE_RATIO`] times the segment's span; when one length is held, the other is solved
//! again with it fixed.

use fontcraft_geom::{Point, Vec2, round_point};
use fontcraft_model::{NodeKind, Path};
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

const EPS: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.reshapeSegment",
            "Reshape Segment",
            [],
            None,
            "{glyph: name, layer?: id, side?, segment: [shape, node], t: number, to: [x, y]} changes the lengths of the two handles of the cubic segment that ends at `node`, keeping both end nodes and each handle's direction, so that the segment's point at `t` (0.01 to 0.99, from the start node) lands on `to`, or as near as the directions allow. Each handle stays between 1 unit and 10 times the larger of the end nodes' distance and the current handle lengths. A straight segment is left as it is and returns `changed: false`; a quadratic segment is refused. The selection afterwards is the segment's two end nodes → {changed, selection}",
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
            let (n1, n2) = solve([p0, p1, p2, p3], at, to).ok_or_else(|| p.bad("the segment has no handle directions to keep"))?;
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

/// The new handles of the cubic `[p0, p1, p2, p3]` that put its point at `t` on `to`, keeping
/// each handle's direction. A handle that lies on its node takes the direction towards the other
/// handle, else towards the other end. `None` when no direction exists (all four points equal).
pub(crate) fn solve([p0, p1, p2, p3]: [Point; 4], t: f64, to: Point) -> Option<(Point, Point)> {
    let u1 = direction([p1 - p0, p2 - p0, p3 - p0])?;
    let u2 = direction([p2 - p3, p1 - p3, p0 - p3])?;
    let (a0, b0) = ((p1 - p0).hypot(), (p2 - p3).hypot());
    let mt = 1.0 - t;
    let (m1, m2) = (u1 * (3.0 * mt * mt * t), u2 * (3.0 * mt * t * t));
    let current = p0.to_vec2() * (mt * mt * mt) + p1.to_vec2() * (3.0 * mt * mt * t) + p2.to_vec2() * (3.0 * mt * t * t) + p3.to_vec2() * (t * t * t);
    let r = to.to_vec2() - current;
    let det = m1.x * m2.y - m1.y * m2.x;
    let (da, db) = if det.abs() > 1e-6 * m1.hypot() * m2.hypot() {
        ((r.x * m2.y - r.y * m2.x) / det, (m1.x * r.y - m1.y * r.x) / det)
    } else {
        let n = m1.hypot2() + m2.hypot2();
        if n <= EPS {
            return None;
        }
        (m1.dot(r) / n, m2.dot(r) / n)
    };
    let max = MAX_HANDLE_RATIO * (p3 - p0).hypot().max(a0).max(b0).max(MIN_HANDLE_LENGTH);
    let hold = |l: f64| if l.is_finite() { l.clamp(MIN_HANDLE_LENGTH, max) } else { MIN_HANDLE_LENGTH };
    let (a, b) = (a0 + da, b0 + db);
    let (mut ha, mut hb) = (hold(a), hold(b));
    // One length held at a limit: solve the other alone, nearest to `to`.
    if ha != a && hb == b && m2.hypot2() > EPS {
        hb = hold(b0 + m2.dot(r - m1 * (ha - a0)) / m2.hypot2());
    } else if hb != b && ha == a && m1.hypot2() > EPS {
        ha = hold(a0 + m1.dot(r - m2 * (hb - b0)) / m1.hypot2());
    }
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
