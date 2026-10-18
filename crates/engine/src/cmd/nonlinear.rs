//! Filter › Non-Linear Transform: every point of a layer's unlocked paths and its anchors goes
//! through two expressions in `x` and `y` (the grammar of `fontcraft_geom::expr`) that give its
//! new x and new y. Rows: `filter.nonLinearTransform`.
//!
//! Mapping only the nodes of a curve is exact for an affine map and approximate for any other,
//! so each segment is checked first: the mapped curve is compared with mapped points of the
//! original curve at a quarter, half and three quarters of the way. A segment within the
//! tolerance keeps its nodes (only their positions change); one outside it is split in half
//! until each piece is within the tolerance, or [`MAX_SPLIT_DEPTH`] halvings deep. A straight
//! segment that the map bends turns into a curve first: a cubic one, or a quadratic one in a path
//! that has quadratic segments. An affine expression therefore changes positions only, exactly as
//! the affine transformations do.
//!
//! Like the other filters it changes whole layers: one `glyph` or a `glyphs` list, the first
//! master's layer unless `layer` or `layers` names others. Locked paths and anchors stay, and
//! components and images stay where they are (a non-linear map has no matrix to give them).
//! With `glyphs`, a locked glyph, a missing layer or a layer the map leaves as it is goes to
//! `skipped`. A run over many glyphs is one undo step, and moved points land on the font's grid.
//! A point that maps outside the coordinate range, an expression that fails at some point, or a
//! layer whose outline would grow past [`MAX_POINTS_PER_LAYER`] points fails the whole command.

use std::collections::HashMap;

use fontcraft_geom::expr::{Expr, ExprError};
use fontcraft_geom::{CubicBez, Line, ParamCurve, Point, QuadBez, round_point};
use fontcraft_model::{Layer, MAX_COORDINATE, Metrics, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

use super::common::{each_layer, grid, in_range, master_metrics, scope};
use super::*;
use crate::params::Params;

/// Most points (nodes and handles) one layer's paths may have after splitting.
pub const MAX_POINTS_PER_LAYER: usize = 1_000_000;
/// Most halvings of one piece of a segment (at most 1,024 pieces from it).
pub const MAX_SPLIT_DEPTH: usize = 10;
/// Most expression work one command does across all its layers, counted as the expressions'
/// source bytes for each point mapped (a byte is at least one evaluation step).
pub const MAX_TRANSFORM_WORK: usize = 1_000_000_000;
/// Default and range of `tolerance`, in font units.
const DEFAULT_TOLERANCE: f64 = 0.5;
const TOLERANCE_RANGE: std::ops::RangeInclusive<f64> = 0.01..=100.0;
/// Where a segment's mapped curve is compared with the mapped original.
const CHECK_AT: [f64; 3] = [0.25, 0.5, 0.75];
/// The names the expressions may use besides the functions.
const NAMES: &str = "x, y, width, xHeight, capHeight, ascender, descender, unitsPerEm, pi";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "filter.nonLinearTransform",
            "Non-Linear Transform…",
            ["Filter"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), x?: expression = \"x\", y?: expression = \"y\", tolerance?: number = 0.5 (0.01 to 100 units)} moves every point of the unlocked paths and every unlocked anchor to the new x and new y the two expressions give; at least one of `x` and `y` is needed. Expressions use numbers, + - * /, parentheses, min, max, abs, round, floor, ceil, sqrt, sin, cos, tan (radians), atan2 and the names x, y (the point), width (the layer's advance width), xHeight, capHeight, ascender, descender (the layer's master), unitsPerEm and pi; e.g. x: \"x + 20*sin(y/100)\". A segment whose mapped curve strays from the mapped outline by more than `tolerance` is split until it does not; a straight segment that bends becomes a curve. Components and images stay. Points land on the grid. A point mapped outside the coordinate range or an expression that fails changes nothing → {glyphs: [names], skipped: [{glyph, layer?, reason}], points: count (after the transform), split: count (segments split)}",
            has_doc,
            non_linear_transform
        )
        .targets(&[TargetKind::Glyphs, TargetKind::Layer]),
    ]
}

/// Reads one expression parameter and checks its names.
fn expression(p: &Params, key: &str) -> Result<Option<Expr>> {
    let Some(text) = p.opt_str(key)? else { return Ok(None) };
    let e = Expr::parse(text).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    if let Some(name) = e.names().into_iter().find(|n| Vars::default().get(n).is_none()) {
        return Err(p.bad(format!("`{key}`: unknown name `{name}`; the names are {NAMES}")));
    }
    Ok(Some(e))
}

/// The values the names stand for at one point.
#[derive(Clone, Copy, Default)]
struct Vars {
    x: f64,
    y: f64,
    width: f64,
    x_height: f64,
    cap_height: f64,
    ascender: f64,
    descender: f64,
    upm: f64,
}

impl Vars {
    fn new(width: f64, m: &Metrics, upm: f64) -> Self {
        Self { x: 0.0, y: 0.0, width, x_height: m.x_height, cap_height: m.cap_height, ascender: m.ascender, descender: m.descender, upm }
    }

    fn get(&self, name: &str) -> Option<f64> {
        Some(match name {
            "x" => self.x,
            "y" => self.y,
            "width" => self.width,
            "xHeight" => self.x_height,
            "capHeight" => self.cap_height,
            "ascender" => self.ascender,
            "descender" => self.descender,
            "unitsPerEm" => self.upm,
            "pi" => std::f64::consts::PI,
            _ => return None,
        })
    }
}

/// A segment's curve, or one piece of it.
#[derive(Clone, Copy, Debug)]
enum Bez {
    Line(Line),
    Quad(QuadBez),
    Cubic(CubicBez),
}

impl Bez {
    fn eval(&self, t: f64) -> Point {
        match self {
            Bez::Line(l) => l.eval(t),
            Bez::Quad(q) => q.eval(t),
            Bez::Cubic(c) => c.eval(t),
        }
    }

    fn subdivide(&self) -> (Bez, Bez) {
        match self {
            Bez::Line(l) => {
                let (a, b) = l.subdivide();
                (Bez::Line(a), Bez::Line(b))
            }
            Bez::Quad(q) => {
                let (a, b) = q.subdivide();
                (Bez::Quad(a), Bez::Quad(b))
            }
            Bez::Cubic(c) => {
                let (a, b) = c.subdivide();
                (Bez::Cubic(a), Bez::Cubic(b))
            }
        }
    }

    /// Points in node order after the start: handles, then the end.
    fn points(&self) -> Vec<Point> {
        match self {
            Bez::Line(l) => vec![l.p1],
            Bez::Quad(q) => vec![q.p1, q.p2],
            Bez::Cubic(c) => vec![c.p1, c.p2, c.p3],
        }
    }

    fn end_kind(&self) -> NodeKind {
        match self {
            Bez::Line(_) => NodeKind::Line,
            Bez::Quad(_) => NodeKind::QCurve,
            Bez::Cubic(_) => NodeKind::Curve,
        }
    }
}

/// A straight piece as a curve of the same shape, so the map can bend it.
fn bend(l: Line, quadratic: bool) -> Bez {
    if quadratic {
        Bez::Quad(QuadBez::new(l.p0, l.p0.midpoint(l.p1), l.p1))
    } else {
        Bez::Cubic(CubicBez::new(l.p0, l.p0.lerp(l.p1, 1.0 / 3.0), l.p0.lerp(l.p1, 2.0 / 3.0), l.p1))
    }
}

/// The pieces of a segment from `start` through `handles` to an end node of `kind`; `None` for
/// a run of nodes that is not a segment (a curve node without two handles, a line node with
/// handles), which is mapped node by node.
fn pieces(start: Point, handles: &[Point], end: Point, kind: NodeKind) -> Option<Vec<Bez>> {
    match (kind, handles) {
        (NodeKind::Line | NodeKind::QCurve, []) => Some(vec![Bez::Line(Line::new(start, end))]),
        (NodeKind::Curve, [a, b]) => Some(vec![Bez::Cubic(CubicBez::new(start, *a, *b, end))]),
        (NodeKind::QCurve, _) => {
            let mut out = Vec::with_capacity(handles.len());
            let mut from = start;
            for (k, c) in handles.iter().enumerate() {
                let to = handles.get(k + 1).map_or(end, |next| c.midpoint(*next));
                out.push(Bez::Quad(QuadBez::new(from, *c, to)));
                from = to;
            }
            Some(out)
        }
        _ => None,
    }
}

/// The map of one command run: both expressions, the shared work budget, and one layer's
/// names, grid and point count.
struct Warp<'a> {
    x: &'a Expr,
    y: &'a Expr,
    cost: usize,
    work: &'a mut usize,
    tolerance: f64,
    step: f64,
    vars: Vars,
    points: usize,
    split: usize,
}

impl Warp<'_> {
    fn map(&mut self, pt: Point) -> std::result::Result<Point, String> {
        *self.work = self
            .work
            .checked_sub(self.cost)
            .ok_or("the transform reached its work limit; nothing was changed: run it on fewer glyphs or with a larger tolerance")?;
        let vars = Vars { x: pt.x, y: pt.y, ..self.vars };
        let fail = |key: &str, e: ExprError| format!("at ({}, {}), `{key}`: {e}", pt.x, pt.y);
        let nx = self.x.eval(|n| vars.get(n)).map_err(|e| fail("x", e))?;
        let ny = self.y.eval(|n| vars.get(n)).map_err(|e| fail("y", e))?;
        if !(in_range(nx) && in_range(ny)) {
            return Err(format!("the point ({}, {}) maps to ({nx}, {ny}), outside ±{MAX_COORDINATE}", pt.x, pt.y));
        }
        Ok(Point::new(nx, ny))
    }

    fn map_bez(&mut self, b: Bez) -> std::result::Result<Bez, String> {
        Ok(match b {
            Bez::Line(l) => Bez::Line(Line::new(self.map(l.p0)?, self.map(l.p1)?)),
            Bez::Quad(q) => Bez::Quad(QuadBez::new(self.map(q.p0)?, self.map(q.p1)?, self.map(q.p2)?)),
            Bez::Cubic(c) => Bez::Cubic(CubicBez::new(self.map(c.p0)?, self.map(c.p1)?, self.map(c.p2)?, self.map(c.p3)?)),
        })
    }

    /// Whether `mapped` stays within the tolerance of the mapped curve of `original`.
    fn close(&mut self, original: &Bez, mapped: &Bez) -> std::result::Result<bool, String> {
        for t in CHECK_AT {
            let exact = self.map(original.eval(t))?;
            if (exact - mapped.eval(t)).hypot() > self.tolerance {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn count(&mut self, n: usize) -> std::result::Result<(), String> {
        self.points = self.points.saturating_add(n);
        if self.points > MAX_POINTS_PER_LAYER {
            return Err(format!("the transformed outline would have more than {MAX_POINTS_PER_LAYER} points; use a larger tolerance"));
        }
        Ok(())
    }

    /// Maps `b`, splitting it until each piece is within the tolerance, into `out`.
    fn fit(&mut self, b: Bez, depth: usize, quadratic: bool, out: &mut Vec<Bez>) -> std::result::Result<(), String> {
        let mapped = self.map_bez(b)?;
        if self.close(&b, &mapped)? || depth >= MAX_SPLIT_DEPTH {
            self.count(mapped.points().len())?;
            out.push(mapped);
            return Ok(());
        }
        if let Bez::Line(l) = b {
            return self.fit(bend(l, quadratic), depth + 1, quadratic, out);
        }
        let (a, c) = b.subdivide();
        self.fit(a, depth + 1, quadratic, out)?;
        self.fit(c, depth + 1, quadratic, out)
    }

    /// A node moved to `pt` on the grid.
    fn moved(&self, node: &Node, pt: Point) -> Node {
        let q = round_point(pt, self.step);
        let mut n = node.clone();
        (n.x, n.y) = (q.x, q.y);
        n
    }

    fn map_node(&mut self, node: &Node) -> std::result::Result<Node, String> {
        let pt = self.map(node.point())?;
        self.count(1)?;
        Ok(self.moved(node, pt))
    }

    /// The nodes that replace the segment from `nodes[start]` through `handles` to `nodes[end]`
    /// (the start node itself is the previous segment's), and whether it was split.
    fn segment(
        &mut self,
        nodes: &[Node],
        start: usize,
        handles: &[usize],
        end: usize,
        quadratic: bool,
    ) -> std::result::Result<(Vec<Node>, bool), String> {
        let (Some(s), Some(e)) = (nodes.get(start), nodes.get(end)) else { return Ok((Vec::new(), false)) };
        let hs: Vec<&Node> = handles.iter().filter_map(|&i| nodes.get(i)).collect();
        let hp: Vec<Point> = hs.iter().map(|n| n.point()).collect();
        let in_place = |w: &mut Self| -> std::result::Result<(Vec<Node>, bool), String> {
            let mut out = Vec::with_capacity(hs.len() + 1);
            for n in hs.iter().copied().chain(std::iter::once(e)) {
                out.push(w.map_node(n)?);
            }
            Ok((out, false))
        };
        let Some(original) = pieces(s.point(), &hp, e.point(), e.kind) else { return in_place(self) };
        let ms = self.map(s.point())?;
        let mut mh = Vec::with_capacity(hp.len());
        for h in &hp {
            mh.push(self.map(*h)?);
        }
        let me = self.map(e.point())?;
        let mapped = pieces(ms, &mh, me, e.kind).unwrap_or_default();
        let mut keep = mapped.len() == original.len();
        for (o, m) in original.iter().zip(&mapped) {
            if !keep {
                break;
            }
            keep = self.close(o, m)?;
        }
        if keep {
            return in_place(self);
        }
        let mut fitted = Vec::new();
        for piece in original {
            self.fit(piece, 0, quadratic, &mut fitted)?;
        }
        let last = fitted.len().saturating_sub(1);
        let mut out = Vec::new();
        for (i, b) in fitted.iter().enumerate() {
            let pts = b.points();
            let Some((end_pt, handles)) = pts.split_last() else { continue };
            out.extend(handles.iter().map(|h| self.moved(&Node::new(0.0, 0.0, NodeKind::OffCurve), *h)));
            let node = if i == last {
                let mut n = self.moved(e, *end_pt);
                n.kind = b.end_kind();
                n
            } else {
                let mut n = self.moved(&Node::new(0.0, 0.0, b.end_kind()), *end_pt);
                n.smooth = !matches!(b, Bez::Line(_));
                n
            };
            out.push(node);
        }
        self.split += 1;
        Ok((out, true))
    }

    /// The nodes of `path` after the map.
    fn path(&mut self, path: &Path) -> std::result::Result<Vec<Node>, String> {
        let nodes = &path.nodes;
        let Some(first) = nodes.iter().position(Node::on_curve) else {
            return nodes.iter().map(|n| self.map_node(n)).collect();
        };
        let quadratic = nodes.iter().any(|n| n.kind == NodeKind::QCurve);
        let count = nodes.len();
        let mut out = Vec::with_capacity(count);
        if !path.closed {
            for n in nodes.iter().take(first + 1) {
                out.push(self.map_node(n)?);
            }
        }
        let steps = if path.closed { count } else { count.saturating_sub(first + 1) };
        let mut prev = first;
        let mut pending: Vec<usize> = Vec::new();
        let mut last_chunk = (0usize, false);
        for step in 1..=steps {
            let idx = (first + step) % count;
            let Some(node) = nodes.get(idx) else { continue };
            if !node.on_curve() {
                pending.push(idx);
                continue;
            }
            let (chunk, split) = self.segment(nodes, prev, &pending, idx, quadratic)?;
            last_chunk = (chunk.len(), split);
            out.extend(chunk);
            pending.clear();
            prev = idx;
        }
        // An open path's trailing handles end no segment.
        for &i in &pending {
            if let Some(n) = nodes.get(i) {
                out.push(self.map_node(n)?);
            }
        }
        if path.closed {
            // The walk ends with the segment into the first on-curve node. Unsplit, it keeps the
            // original order: the handles before that node and the node itself go back to the
            // front. Split, the whole segment goes to the front.
            let k = if last_chunk.1 { last_chunk.0 } else { first + 1 }.min(out.len());
            out.rotate_right(k);
        }
        Ok(out)
    }
}

/// Moves one layer's unlocked paths and anchors; `None` when they stay where they were.
fn warp_layer(w: &mut Warp, layer: &mut Layer) -> std::result::Result<Option<String>, String> {
    let mut new_paths: Vec<(usize, Vec<Node>)> = Vec::new();
    for (i, shape) in layer.shapes.iter().enumerate() {
        if let Shape::Path(path) = shape
            && !path.info.locked
        {
            new_paths.push((i, w.path(path)?));
        }
    }
    let mut new_anchors: Vec<(usize, Point)> = Vec::new();
    for (i, a) in layer.anchors.iter().enumerate() {
        if !a.info.locked {
            let q = round_point(w.map(Point::new(a.x, a.y))?, w.step);
            new_anchors.push((i, q));
        }
    }
    if new_paths.is_empty() && new_anchors.is_empty() {
        return Ok(Some("the layer has no unlocked paths or anchors".into()));
    }
    let mut changed = false;
    for (i, nodes) in new_paths {
        if let Some(Shape::Path(path)) = layer.shapes.get_mut(i)
            && path.nodes != nodes
        {
            path.nodes = nodes;
            changed = true;
        }
    }
    for (i, q) in new_anchors {
        if let Some(a) = layer.anchors.get_mut(i)
            && (a.x, a.y) != (q.x, q.y)
        {
            (a.x, a.y) = (q.x, q.y);
            changed = true;
        }
    }
    Ok(if changed { None } else { Some("nothing moved".into()) })
}

fn non_linear_transform(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.nonLinearTransform", v)?;
    let (ex, ey) = (expression(&p, "x")?, expression(&p, "y")?);
    if ex.is_none() && ey.is_none() {
        return Err(p.bad("give `x`, `y` or both: the expressions for the new x and the new y"));
    }
    let identity = |name: &str| Expr::parse(name).map_err(|e| p.bad(e.to_string()));
    let ex = match ex {
        Some(e) => e,
        None => identity("x")?,
    };
    let ey = match ey {
        Some(e) => e,
        None => identity("y")?,
    };
    let tolerance = p.opt_f64("tolerance")?.unwrap_or(DEFAULT_TOLERANCE);
    if !TOLERANCE_RANGE.contains(&tolerance) {
        return Err(p.bad(format!("`tolerance` must be between {} and {} units, got {tolerance}", TOLERANCE_RANGE.start(), TOLERANCE_RANGE.end())));
    }
    let font = s.font()?;
    let scope = scope(&p, font)?;
    let metrics: HashMap<String, Metrics> = font.masters.iter().map(|m| (m.id.clone(), master_metrics(font, m))).collect();
    let fallback = font.masters.first().map(|m| master_metrics(font, m)).unwrap_or_default();
    let upm = f64::from(font.units_per_em);
    let step = grid(font);
    let cost = ex.source().len().saturating_add(ey.source().len()).max(1);
    let mut work = MAX_TRANSFORM_WORK;
    let (mut points, mut split) = (0usize, 0usize);
    let out = s.edit("Non-Linear Transform", |font| {
        each_layer(font, &scope, p.command(), |name, layer| {
            let m = metrics.get(&layer.master).copied().unwrap_or(fallback);
            let mut w = Warp { x: &ex, y: &ey, cost, work: &mut work, tolerance, step, vars: Vars::new(layer.width, &m, upm), points: 0, split: 0 };
            let result = warp_layer(&mut w, layer).map_err(|e| p.bad(format!("glyph `{name}`, layer `{}`: {e}", layer.id)))?;
            if result.is_none() {
                points = points.saturating_add(w.points);
                split = split.saturating_add(w.split);
            }
            Ok(result)
        })
    })?;
    let mut v = out.to_json();
    if let Some(o) = v.as_object_mut() {
        o.insert("points".into(), json!(points));
        o.insert("split".into(), json!(split));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Anchor, Glyph};

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn node(x: f64, y: f64, kind: NodeKind) -> Node {
        Node::new(x, y, kind)
    }

    /// A closed rectangle from (100, 0) to (500, 700), lines only, starting at its first corner.
    fn rectangle() -> Path {
        Path::new(
            true,
            vec![
                node(100.0, 0.0, NodeKind::Line),
                node(500.0, 0.0, NodeKind::Line),
                node(500.0, 700.0, NodeKind::Line),
                node(100.0, 700.0, NodeKind::Line),
            ],
        )
    }

    /// A closed bowl of two cubic segments whose list starts with the handles of the segment
    /// into node 2, the way source files often store closed curves.
    fn bowl() -> Path {
        Path::new(
            true,
            vec![
                node(100.0, 300.0, NodeKind::OffCurve),
                node(200.0, 0.0, NodeKind::OffCurve),
                node(300.0, 0.0, NodeKind::Curve),
                node(400.0, 0.0, NodeKind::OffCurve),
                node(500.0, 300.0, NodeKind::OffCurve),
                node(300.0, 600.0, NodeKind::Curve),
            ],
        )
    }

    /// A document without a grid (so results are exact) holding glyph `g` with `paths` and an
    /// anchor `top` at (300, 700).
    fn session(paths: Vec<Path>) -> Session {
        let mut s = Session::with_document();
        s.edit("fixture", |f| {
            Arc::make_mut(&mut f.info).settings.grid_length = Some(0.0);
            let master = f.default_master()?.id.clone();
            let mut g = Glyph::new("g");
            let mut l = Layer::new(master, 600.0);
            l.shapes.extend(paths.into_iter().map(Shape::Path));
            l.anchors.push(Anchor::new("top", 300.0, 700.0));
            g.layers.push(l);
            f.glyphs.remove("g");
            f.glyphs.push(g);
            Ok(())
        })
        .unwrap();
        s
    }

    fn paths(s: &Session) -> Vec<Path> {
        s.font().unwrap().glyph("g").unwrap().layers[0]
            .shapes
            .iter()
            .filter_map(|sh| match sh {
                Shape::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    fn undo_steps(s: &Session) -> usize {
        s.active().unwrap().history.undo.len()
    }

    fn anchor(s: &Session) -> (f64, f64) {
        let a = &s.font().unwrap().glyph("g").unwrap().layers[0].anchors[0];
        (a.x, a.y)
    }

    #[test]
    fn filter_non_linear_transform_identity_changes_nothing() {
        let mut s = session(vec![rectangle(), bowl()]);
        let (before, steps) = (paths(&s), undo_steps(&s));
        let out = run(&mut s, "filter.nonLinearTransform", json!({"glyph": "g", "x": "x", "y": "y"}));
        assert_eq!(paths(&s), before);
        assert_eq!(anchor(&s), (300.0, 700.0));
        assert_eq!(out["glyphs"], json!([]));
        assert_eq!(out["skipped"][0]["reason"], "nothing moved");
        assert_eq!(undo_steps(&s), steps, "an identity map makes no undo step");
    }

    #[test]
    fn filter_non_linear_transform_shear_equals_the_affine_slant() {
        let mut affine = session(vec![rectangle(), bowl()]);
        run(&mut affine, "path.transformations", json!({"glyphs": ["g"], "slant": 10.0}));
        let mut warped = session(vec![rectangle(), bowl()]);
        let k = 10f64.to_radians().tan();
        let out = run(&mut warped, "filter.nonLinearTransform", json!({"glyph": "g", "x": format!("x + {k}*y")}));
        assert_eq!(out["split"], 0, "an affine map splits nothing");
        let (a, b) = (paths(&affine), paths(&warped));
        assert_eq!(a.len(), b.len());
        for (pa, pb) in a.iter().zip(&b) {
            assert_eq!(pa.nodes.len(), pb.nodes.len());
            for (na, nb) in pa.nodes.iter().zip(&pb.nodes) {
                assert_eq!(na.kind, nb.kind);
                assert!((na.x - nb.x).abs() < 1e-6 && (na.y - nb.y).abs() < 1e-6, "{na:?} vs {nb:?}");
            }
        }
        let (ax, ay) = anchor(&warped);
        assert!((ax - (300.0 + k * 700.0)).abs() < 1e-6 && ay == 700.0);
    }

    #[test]
    fn filter_non_linear_transform_sine_warp_splits_and_follows_the_curve() {
        let mut s = session(vec![rectangle()]);
        let steps = undo_steps(&s);
        let out = run(&mut s, "filter.nonLinearTransform", json!({"glyph": "g", "x": "x + 40*sin(y/100)", "tolerance": 0.5}));
        assert_eq!(out["glyphs"], json!(["g"]));
        assert_eq!(out["split"], 2, "the two vertical sides bend; the horizontal ones only move");
        let p = paths(&s).remove(0);
        assert!(p.closed);
        assert!(p.nodes.len() > 8, "the bent sides were subdivided: {} nodes", p.nodes.len());
        // The horizontal sides stay straight lines.
        assert!(p.nodes.iter().filter(|n| n.kind == NodeKind::Line).count() >= 2);
        // Every on-curve node lies exactly on the warped rectangle, and every cubic's midpoint
        // within the tolerance of it.
        let warped_x = |y: f64, edge: f64| edge + 40.0 * (y / 100.0).sin();
        let on_edge = |pt: Point, slack: f64| {
            [100.0, 500.0].iter().any(|&e| (pt.x - warped_x(pt.y, e)).abs() <= slack)
                || ((pt.y.abs() <= slack || (pt.y - 700.0).abs() <= slack) && (90.0..=550.0).contains(&pt.x))
        };
        for n in p.nodes.iter().filter(|n| n.on_curve()) {
            assert!(on_edge(n.point(), 1e-6), "{n:?} is off the warped outline");
        }
        let count = p.nodes.len();
        for (i, n) in p.nodes.iter().enumerate() {
            if n.kind != NodeKind::Curve {
                continue;
            }
            let at = |k: usize| p.nodes[(i + count - k) % count].point();
            let c = CubicBez::new(at(3), at(2), at(1), n.point());
            assert!(on_edge(c.eval(0.5), 0.6), "a midpoint strays: {:?}", c.eval(0.5));
        }
        assert_eq!(undo_steps(&s), steps + 1);
    }

    #[test]
    fn filter_non_linear_transform_keeps_a_closed_list_order_when_unsplit() {
        let mut s = session(vec![bowl()]);
        run(&mut s, "filter.nonLinearTransform", json!({"glyph": "g", "x": "x + 10", "y": "y * 2"}));
        let p = paths(&s).remove(0);
        let kinds: Vec<NodeKind> = p.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, bowl().nodes.iter().map(|n| n.kind).collect::<Vec<_>>());
        assert_eq!((p.nodes[0].x, p.nodes[0].y), (110.0, 600.0));
        assert_eq!((p.nodes[5].x, p.nodes[5].y), (310.0, 1200.0));
    }

    #[test]
    fn filter_non_linear_transform_refuses_hostile_expressions_and_changes_nothing() {
        let mut s = session(vec![rectangle()]);
        let (before, steps) = (paths(&s), undo_steps(&s));
        let deep = format!("{}x{}", "(".repeat(200), ")".repeat(200));
        let long = "x+".repeat(500_000) + "x";
        let cases = [
            json!({"glyph": "g", "x": long}),
            json!({"glyph": "g", "x": deep}),
            json!({"glyph": "g", "x": "x + z"}),
            json!({"glyph": "g", "x": "x / (y - y)"}),
            json!({"glyph": "g", "y": "sqrt(y - 1000)"}),
            json!({"glyph": "g", "x": "x * 1e300 * 1e300"}),
            json!({"glyph": "g", "x": "x * 1e7"}),
            json!({"glyph": "g", "x": "x +"}),
            json!({"glyph": "g"}),
            json!({"glyph": "g", "x": "x", "tolerance": 0.0}),
        ];
        for params in cases {
            assert!(s.execute("filter.nonLinearTransform", &params).is_err(), "accepted {params}");
            assert_eq!(paths(&s), before);
        }
        assert_eq!(undo_steps(&s), steps);
    }

    #[test]
    fn filter_non_linear_transform_wild_warp_stays_bounded() {
        let mut s = session(vec![rectangle()]);
        let out = run(&mut s, "filter.nonLinearTransform", json!({"glyph": "g", "x": "x + 300*sin(y*7)", "tolerance": 0.01}));
        let n = paths(&s)[0].nodes.len();
        // Two bent sides, each at most 2^MAX_SPLIT_DEPTH cubics of three points, plus the rest.
        assert!(n <= 2 * (1 << MAX_SPLIT_DEPTH) * 3 + 8, "{n} nodes");
        assert_eq!(out["points"], n, "anchors are not counted: {out}");
    }
}
