//! Fix commands for glyphs and outlines that the audit calls, one undo step each (M12.62; the
//! `own = true` rows of `21-qa-checks.toml`), and the calls `audit.fix` builds for the layer steps
//! of `docs/spec/21-qa.md` § 5 "Fix calls" that are not M3's.
//!
//! - `path.eraseOpenCorners` trims an open corner (segments `a` and `b` that cross just before
//!   they meet, joined by a short segment between them) back to the crossing point: the two
//!   nodes of the corner become one corner node there.
//! - `component.flatten` replaces a nested component by components of the innermost glyphs.
//!
//! The audit fix `fixCompatibility` reorders shapes with `glyph.setShapeOrder`.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_audit::Finding;
use fontcraft_components::propagate::base_layer;
use fontcraft_geom::{Affine, Piece, Point, Segment, Vec2, round_point};
use fontcraft_model::{Alignment, Component, Font, Glyph, Layer, MAX_COMPONENT_DEPTH, Node, NodeKind, Path, Shape, Transform};
use serde_json::{Value, json};

use super::common::grid;
use super::decompose::{MAX_LAYER_SHAPES, remap_hint};
use super::*;
use crate::params::{MAX_TARGETS, Params, TargetKind as K, Targets, kind};

/// Samples per piece when looking for the crossing of two segments.
const CROSSING_SAMPLES: usize = 64;
/// Newton steps that refine a sampled crossing.
const CROSSING_REFINE_STEPS: usize = 12;
/// How close (font units) a refined crossing must bring the two segments to replace the sampled one.
const CROSSING_TOLERANCE: f64 = 1e-6;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.eraseOpenCorners",
            "Erase Open Corners",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]]} trims each open corner (two segments that cross just before they meet, joined by a short segment, leaving a small loop outside the outline) back to the crossing point: the two segments are split there, curves keeping their shape, the loop is dropped, and the corner's two nodes become one corner node at the crossing. `points` names either node of each corner; without it every open corner of the layer is trimmed. A corner counts only when the crossing lies in the later half of the first segment and the earlier half of the second. A named node with no such corner is left as it is and listed in `skipped` → {erased: count, skipped: [[shape, node]], selection}",
            has_doc,
            erase_open_corners
        )
        .targets(&[K::Layer, K::Points]),
        cmd!(
            "component.flatten",
            "Flatten Components",
            [],
            None,
            "{glyph: name, layer?: id, components?: [indices] (default every nested component of the layer)} replaces each nested component (one whose base glyph is made only of components) by components of the innermost glyphs, transforms combined, so the outline does not change. The new components take the nested one's place in the shape list, in its base glyph's order, follow nesting up to 32 levels, and keep their place as drawn (automatic alignment off). A component whose base glyph has paths, or no components, is left as it is and listed in `skipped` → {replaced: count, skipped: [index], selection}",
            has_doc,
            flatten_components
        )
.targets(&[K::Layer, K::Shapes]),
    ]
}

// ── path.eraseOpenCorners ───────────────────────────────────────────────────────────────────────

fn erase_open_corners(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.eraseOpenCorners", v)?;
    let t = Targets::parse(&p)?;
    let step = grid(s.font()?);
    let (erased, skipped) = s.edit("Erase Open Corners", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut wanted: BTreeMap<usize, Vec<(usize, Point)>> = BTreeMap::new();
            for &(shape, node) in &t.points {
                let at = match parts.shapes.get(shape) {
                    Some(Shape::Path(path)) => path.nodes.get(node).filter(|n| n.on_curve()).map(Node::point),
                    _ => None,
                };
                let Some(at) = at else { return Err(p.bad(format!("`points`: [{shape}, {node}] is not an on-curve node of a path"))) };
                wanted.entry(shape).or_default().push((node, at));
            }
            let shapes: Vec<usize> = if t.points.is_empty() { (0..parts.shapes.len()).collect() } else { wanted.keys().copied().collect() };
            let mut erased = 0usize;
            let mut skipped = Vec::new();
            for shape in shapes {
                let Some(path) = parts.path_mut(shape) else { continue };
                let asked = wanted.remove(&shape);
                let (new, count, left) = erase_in_path(path, asked.as_deref(), step);
                *path = new;
                erased += count;
                skipped.extend(left.into_iter().map(|n| json!([shape, n])));
            }
            Ok((erased, skipped))
        })
    })?;
    let mut sel = Targets::new("path.eraseOpenCorners", t.glyph.clone(), t.layer.clone());
    sel.side = t.side;
    Ok(json!({"erased": erased, "skipped": skipped, "selection": sel.to_json()}))
}

/// An open corner of a path: the segments before, between and after, by place in the segment
/// list, and where the outer two cross.
struct Corner {
    a: usize,
    mid: usize,
    b: usize,
    ta: f64,
    tb: f64,
}

/// Erases the open corners of `path`: those at the nodes in `asked` (by index and position in the
/// unchanged path), or every one. Returns the new path, how many corners went, and the asked
/// nodes that had none.
fn erase_in_path(path: &Path, asked: Option<&[(usize, Point)]>, step: f64) -> (Path, usize, Vec<usize>) {
    let mut out = path.clone();
    let mut left: Vec<(usize, Point)> = asked.map(<[_]>::to_vec).unwrap_or_default();
    let mut count = 0usize;
    // Each erase removes a segment, so the loop ends.
    let rounds = path.segments().len();
    for _ in 0..rounds {
        let segs = out.segments();
        let corner = find_corners(&out, &segs).into_iter().find(|c| match asked {
            None => true,
            Some(_) => corner_nodes(&segs, c).iter().any(|n| out.nodes.get(*n).is_some_and(|node| left.iter().any(|(_, at)| *at == node.point()))),
        });
        let Some(c) = corner else { break };
        let ends: Vec<Point> = corner_nodes(&segs, &c).iter().filter_map(|n| out.nodes.get(*n).map(Node::point)).collect();
        let Some(next) = erase_corner(&out, &segs, &c, step) else { break };
        left.retain(|(_, at)| !ends.contains(at));
        out = next;
        count += 1;
    }
    (out, count, left.into_iter().map(|(n, _)| n).collect())
}

/// The two nodes of a corner: where the first segment ends and where the second starts.
fn corner_nodes(segs: &[Segment], c: &Corner) -> [usize; 2] {
    let mid = segs.get(c.mid);
    [mid.map_or(usize::MAX, |m| m.start), mid.map_or(usize::MAX, |m| m.end)]
}

/// The open corners of a path, in segment order.
fn find_corners(path: &Path, segs: &[Segment]) -> Vec<Corner> {
    let n = segs.len();
    let enough = if path.closed { n >= 4 } else { n >= 3 };
    if !enough {
        return Vec::new();
    }
    let last = if path.closed { n } else { n - 2 };
    let mut out = Vec::new();
    for k in 0..last {
        let (ia, im, ib) = (k, (k + 1) % n, (k + 2) % n);
        let (Some(a), Some(mid), Some(b)) = (segs.get(ia), segs.get(im), segs.get(ib)) else { continue };
        if a.end != mid.start || mid.end != b.start {
            continue;
        }
        let Some((ta, tb)) = crossing(a, b) else { continue };
        // The loop beyond the crossing is the smaller part of both segments.
        if ta > 0.5 && tb < 0.5 {
            out.push(Corner { a: ia, mid: im, b: ib, ta, tb });
        }
    }
    out
}

/// Where segments `a` and `b` cross, as their parameters, both strictly inside. Touching ends (a
/// middle segment of length zero) do not count.
fn crossing(a: &Segment, b: &Segment) -> Option<(f64, f64)> {
    let (pa, pb) = (polyline(a), polyline(b));
    for w in pa.windows(2) {
        let (Some(&(t0, p0)), Some(&(t1, p1))) = (w.first(), w.get(1)) else { continue };
        for x in pb.windows(2) {
            let (Some(&(u0, q0)), Some(&(u1, q1))) = (x.first(), x.get(1)) else { continue };
            if !boxes_overlap(p0, p1, q0, q1) {
                continue;
            }
            let Some((s, r)) = line_crossing(p0, p1, q0, q1) else { continue };
            let (ta, tb) = (t0 + (t1 - t0) * s, u0 + (u1 - u0) * r);
            let (ta, tb) = refine(a, b, ta, tb).unwrap_or((ta, tb));
            if ta > 0.0 && ta < 1.0 && tb > 0.0 && tb < 1.0 {
                return Some((ta, tb));
            }
        }
    }
    None
}

/// A segment as points with their parameters.
fn polyline(seg: &Segment) -> Vec<(f64, Point)> {
    let steps = CROSSING_SAMPLES.saturating_mul(seg.pieces.len().max(1));
    (0..=steps)
        .filter_map(|i| {
            let t = i as f64 / steps as f64;
            seg.eval(t).filter(|p| p.is_finite()).map(|p| (t, p))
        })
        .collect()
}

fn boxes_overlap(p0: Point, p1: Point, q0: Point, q1: Point) -> bool {
    p0.x.max(p1.x) >= q0.x.min(q1.x) && q0.x.max(q1.x) >= p0.x.min(p1.x) && p0.y.max(p1.y) >= q0.y.min(q1.y) && q0.y.max(q1.y) >= p0.y.min(p1.y)
}

/// Where lines p0–p1 and q0–q1 cross, as fractions along each; parallel lines do not cross.
fn line_crossing(p0: Point, p1: Point, q0: Point, q1: Point) -> Option<(f64, f64)> {
    let (d, e) = (p1 - p0, q1 - q0);
    let den = d.cross(e);
    if den == 0.0 || !den.is_finite() {
        return None;
    }
    let w = q0 - p0;
    let (s, r) = (w.cross(e) / den, w.cross(d) / den);
    ((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&r)).then_some((s, r))
}

/// Newton steps on a(ta) = b(tb), from the sampled crossing; `None` when they do not settle.
fn refine(a: &Segment, b: &Segment, mut ta: f64, mut tb: f64) -> Option<(f64, f64)> {
    const H: f64 = 1e-7;
    for _ in 0..CROSSING_REFINE_STEPS {
        let f = a.eval(ta)? - b.eval(tb)?;
        if f.hypot() <= CROSSING_TOLERANCE {
            return Some((ta, tb));
        }
        let da = (a.eval((ta + H).min(1.0))? - a.eval((ta - H).max(0.0))?) / (2.0 * H);
        let db = (b.eval((tb + H).min(1.0))? - b.eval((tb - H).max(0.0))?) / (2.0 * H);
        // Solve [da, -db] · [dta, dtb] = -f.
        let det = da.cross(-db);
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let dta = (-f).cross(-db) / det;
        let dtb = da.cross(-f) / det;
        ta += dta;
        tb += dtb;
        if !(ta.is_finite() && tb.is_finite()) || !(0.0..=1.0).contains(&ta) || !(0.0..=1.0).contains(&tb) {
            return None;
        }
    }
    let f = a.eval(ta)? - b.eval(tb)?;
    (f.hypot() <= CROSSING_TOLERANCE).then_some((ta, tb))
}

/// The nodes that draw `pieces` after their start: handles and the on-curve end of each piece.
/// On-curve nodes between pieces are smooth.
fn piece_nodes(pieces: &[Piece], step: f64) -> Vec<Node> {
    let mut out = Vec::new();
    let node = |p: Point, kind: NodeKind| {
        let p = round_point(p, step);
        Node::new(p.x, p.y, kind)
    };
    for (k, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Line(l) => out.push(node(l.p1, NodeKind::Line)),
            Piece::Quad(q) => {
                out.push(node(q.p1, NodeKind::OffCurve));
                out.push(node(q.p2, NodeKind::QCurve));
            }
            Piece::Cubic(c) => {
                out.push(node(c.p1, NodeKind::OffCurve));
                out.push(node(c.p2, NodeKind::OffCurve));
                out.push(node(c.p3, NodeKind::Curve));
            }
        }
        if k + 1 < pieces.len()
            && !matches!(piece, Piece::Line(_))
            && let Some(last) = out.last_mut()
        {
            last.smooth = true;
        }
    }
    out
}

/// One segment of a rebuilt path: its handles and end node, and the end node's index before.
struct Entry {
    nodes: Vec<Node>,
    was: Option<usize>,
}

/// `path` with corner `c` erased; `None` when the path cannot be rebuilt.
fn erase_corner(path: &Path, segs: &[Segment], c: &Corner, step: f64) -> Option<Path> {
    let (a, b) = (segs.get(c.a)?, segs.get(c.b)?);
    let (before, _) = a.split(c.ta)?;
    let (_, after) = b.split(c.tb)?;
    let mut head = piece_nodes(&before, step);
    if let Some(x) = head.last_mut() {
        x.smooth = false;
    }
    let mut tail = piece_nodes(&after, step);
    let end_kind = tail.pop()?.kind;
    let mut end = path.nodes.get(b.end)?.clone();
    end.kind = end_kind;
    tail.push(end);
    let mut entries: Vec<Entry> = Vec::with_capacity(segs.len());
    let mut x_entry = None;
    for (k, seg) in segs.iter().enumerate() {
        if k == c.mid {
            continue;
        }
        if k == c.a {
            x_entry = Some(entries.len());
            entries.push(Entry { nodes: std::mem::take(&mut head), was: None });
        } else if k == c.b {
            entries.push(Entry { nodes: std::mem::take(&mut tail), was: Some(seg.end) });
        } else {
            let mut nodes: Vec<Node> = seg.handles.iter().filter_map(|&h| path.nodes.get(h).cloned()).collect();
            nodes.push(path.nodes.get(seg.end)?.clone());
            entries.push(Entry { nodes, was: Some(seg.end) });
        }
    }
    let first = path.nodes.iter().position(Node::on_curve)?;
    let mut nodes = Vec::with_capacity(path.nodes.len());
    if path.closed {
        // The entry ending at the old start node, or at the crossing when that node went, is
        // drawn last, so its end node comes first.
        let start = entries.iter().position(|e| e.was == Some(first)).or(x_entry)?;
        let count = entries.len();
        let last = entries.get(start)?;
        nodes.push(last.nodes.last()?.clone());
        for i in 1..=count {
            let e = entries.get((start + i) % count)?;
            if i == count {
                nodes.extend(e.nodes.iter().take(e.nodes.len().saturating_sub(1)).cloned());
            } else {
                nodes.extend(e.nodes.iter().cloned());
            }
        }
    } else {
        if first != 0 {
            return None;
        }
        nodes.push(path.nodes.first()?.clone());
        for e in &entries {
            nodes.extend(e.nodes.iter().cloned());
        }
    }
    let mut out = path.clone();
    out.nodes = nodes;
    Some(out)
}

// ── component.flatten ───────────────────────────────────────────────────────────────────────────

/// A transform as stored: a move stays a move, anything else becomes a matrix.
fn transform_of(m: Affine) -> Transform {
    let [xx, yx, xy, yy, dx, dy] = m.as_coeffs();
    if xx == 1.0 && yx == 0.0 && xy == 0.0 && yy == 1.0 { Transform::translate(dx, dy) } else { Transform::Matrix(m.as_coeffs()) }
}

/// Does the layer hold components and no paths?
fn only_components(layer: &Layer) -> bool {
    layer.shapes.iter().any(|s| matches!(s, Shape::Component(_))) && !layer.shapes.iter().any(|s| matches!(s, Shape::Path(_)))
}

/// The components of the innermost glyphs that `c` (placed by `outer`) draws, for `layer`.
/// `None` when nesting goes deeper than [`MAX_COMPONENT_DEPTH`] or past `budget` components.
fn innermost(font: &Font, layer: &Layer, c: &Component, outer: Affine, depth: usize, budget: &mut usize, out: &mut Vec<Component>) -> Option<()> {
    if depth > MAX_COMPONENT_DEPTH {
        return None;
    }
    let base = font.glyph(&c.glyph)?;
    let bl = base_layer(base, layer, c)?;
    for inner in bl.shapes.iter().filter_map(|s| match s {
        Shape::Component(d) => Some(d),
        _ => None,
    }) {
        *budget = budget.checked_sub(1)?;
        let m = outer * inner.affine();
        let deeper = font.glyph(&inner.glyph).and_then(|g| base_layer(g, layer, inner)).is_some_and(only_components);
        if deeper {
            innermost(font, layer, inner, m, depth + 1, budget, out)?;
        } else {
            let mut placed = inner.clone();
            placed.transform = transform_of(m);
            placed.info.alignment = Alignment::Disabled;
            out.push(placed);
        }
    }
    Some(())
}

fn flatten_components(s: &mut Session, v: &Value) -> Result<Value> {
    const ID: &str = "component.flatten";
    let p = Params::new(ID, v)?;
    let mut t = Targets::new(ID, p.str("glyph")?, p.opt_str("layer")?.map(str::to_string));
    let chosen: Option<BTreeSet<usize>> = match p.opt_array("components")? {
        None => None,
        Some(list) => {
            if list.len() > MAX_TARGETS {
                return Err(p.bad(format!("`components` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
            }
            let mut set = BTreeSet::new();
            for v in list {
                let i = v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| p.bad(format!("`components` must hold indices (whole numbers from 0), found {} ({v})", kind(v))))?;
                set.insert(i);
            }
            Some(set)
        }
    };
    let (replaced, skipped, added) = s.edit("Flatten Components", |font| {
        let (layer, replaced, skipped, added) = {
            let r = t.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            let layer = r.layer;
            if let Some(list) = &chosen {
                for &i in list {
                    match layer.shapes.get(i) {
                        Some(Shape::Component(_)) => {}
                        Some(_) => return Err(p.bad(format!("shape {i} is not a component"))),
                        None => return Err(p.bad(format!("shape {i} does not resolve: the layer has {} shapes", layer.shapes.len()))),
                    }
                }
            }
            let mut shapes = Vec::with_capacity(layer.shapes.len());
            let mut map: Vec<Option<usize>> = Vec::with_capacity(layer.shapes.len());
            let (mut replaced, mut skipped, mut added) = (0usize, Vec::new(), Vec::new());
            let mut budget = MAX_LAYER_SHAPES;
            for (i, shape) in layer.shapes.iter().enumerate() {
                let Shape::Component(c) = shape else {
                    map.push(Some(shapes.len()));
                    shapes.push(shape.clone());
                    continue;
                };
                let named = chosen.as_ref().is_none_or(|set| set.contains(&i));
                let nested = font.glyph(&c.glyph).and_then(|g| base_layer(g, layer, c)).is_some_and(only_components);
                if !named || !nested {
                    if named && chosen.is_some() {
                        skipped.push(i);
                    }
                    map.push(Some(shapes.len()));
                    shapes.push(shape.clone());
                    continue;
                }
                let mut parts = Vec::new();
                innermost(font, layer, c, c.affine(), 1, &mut budget, &mut parts).ok_or_else(|| {
                    p.bad(format!("shape {i}: the components nest deeper than {MAX_COMPONENT_DEPTH} levels or exceed {MAX_LAYER_SHAPES}"))
                })?;
                map.push(None);
                for part in parts {
                    added.push(shapes.len());
                    shapes.push(Shape::Component(part));
                }
                replaced += 1;
            }
            if shapes.len() > MAX_LAYER_SHAPES {
                return Err(p.bad(format!("the layer would hold {} shapes; at most {MAX_LAYER_SHAPES} are allowed", shapes.len())));
            }
            let mut out = layer.clone();
            out.shapes = shapes;
            if !layer.info.hints.is_empty() {
                let remap = |i: usize| map.get(i).map_or(Some(i), |m| *m);
                out.info.hints = layer.info.hints.iter().filter_map(|h| remap_hint(h, &remap)).collect();
            }
            (out, replaced, skipped, added)
        };
        t.edit_layer(font, |l| {
            *l = layer;
            Ok(())
        })?;
        Ok((replaced, skipped, added))
    })?;
    t.layer = Some(t.layer_id(s.font()?)?);
    t.shapes = added;
    Ok(json!({"replaced": replaced, "skipped": skipped, "selection": t.to_json()}))
}

// ── Calls of audit.fix ──────────────────────────────────────────────────────────────────────────

/// The calls one layer step makes, and why findings were left out.
#[derive(Default)]
pub(super) struct StepCalls {
    pub calls: Vec<(&'static str, Value)>,
    pub skipped: Vec<(&'static str, String)>,
}

/// The `audit.fix` calls of the layer steps this module adds (`docs/spec/21-qa.md` § 5 "Fix
/// calls", steps 7 to 11, 14, 23, 25, 26, 28 and 30 to 33) for `findings` on one layer of
/// `glyph`. `None` for a fix this module does not build.
pub(super) fn step_calls(fix: &str, font: &Font, glyph: &Glyph, layer: &Layer, findings: &[Finding]) -> Option<StepCalls> {
    let (g, l) = (glyph.name.as_str(), layer.id.as_str());
    // A finding that says it cannot be repaired automatically is left alone.
    let findings: Vec<&Finding> = findings.iter().filter(|f| f.fix.is_some()).collect();
    let mut out = StepCalls::default();
    let components = || -> Vec<usize> {
        let set: BTreeSet<usize> = findings
            .iter()
            .flat_map(|f| f.location.components.iter().chain(&f.location.shapes).copied())
            .filter(|&i| matches!(layer.shapes.get(i), Some(Shape::Component(_))))
            .collect();
        set.into_iter().collect()
    };
    let points = || -> Vec<[usize; 2]> {
        let set: BTreeSet<[usize; 2]> = findings.iter().flat_map(|f| f.location.points.iter().copied()).collect();
        set.into_iter().collect()
    };
    let mut on_components = |command: &'static str, key: &str| {
        let list = components();
        if !list.is_empty() {
            let mut params = json!({"glyph": g, "layer": l});
            if let Some(o) = params.as_object_mut() {
                o.insert(key.into(), json!(list));
            }
            out.calls.push((command, params));
        }
    };
    match fix {
        "clearGlyph" if !layer.shapes.is_empty() => {
            let all: Vec<usize> = (0..layer.shapes.len()).collect();
            out.calls.push(("edit.delete", json!({"glyph": g, "layer": l, "shapes": all})));
        }
        "clearGlyph" => {}
        "flattenComponents" => on_components("component.flatten", "components"),
        "removeDuplicateComponents" => on_components("edit.delete", "shapes"),
        "decomposeComponents" => on_components("component.decompose", "components"),
        "realignComponents" => on_components("component.enableAutomaticAlignment", "components"),
        "eraseOpenCorners" => {
            let list = points();
            if !list.is_empty() {
                out.calls.push(("path.eraseOpenCorners", json!({"glyph": g, "layer": l, "points": list})));
            }
        }
        "alignHandles" => {
            for [shape, node] in points() {
                let Some(Shape::Path(path)) = layer.shapes.get(shape) else { continue };
                match aligned_handles(path, node) {
                    Some(moves) => {
                        for (h, at) in moves {
                            out.calls.push(("selection.setPosition", json!({"glyph": g, "layer": l, "points": [[shape, h]], "x": at.x, "y": at.y})));
                        }
                    }
                    None => {
                        out.skipped.push(("check.outlines.kinkedSmoothNode", format!("path {shape}, node {node}: its handles have no common line")))
                    }
                }
            }
        }
        "snapHandles" => {
            for [shape, h] in points() {
                let Some(Shape::Path(path)) = layer.shapes.get(shape) else { continue };
                if let Some(at) = snapped_handle(path, h) {
                    out.calls.push(("selection.setPosition", json!({"glyph": g, "layer": l, "points": [[shape, h]], "x": at.x, "y": at.y})));
                }
            }
        }
        "snapToMetric" => {
            let mut done = BTreeSet::new();
            for f in &findings {
                let (Some(&[shape, node]), Some(&metric)) = (f.location.points.first(), f.data.get("metric")) else { continue };
                let Some(n) = layer.node(shape, node) else { continue };
                let dy = metric - n.y;
                if dy != 0.0 && dy.is_finite() && done.insert([shape, node]) {
                    out.calls.push(("selection.move", json!({"glyph": g, "layer": l, "points": [[shape, node]], "dx": 0, "dy": dy})));
                }
            }
        }
        "fixCompatibility" => {
            // The shape order goes first; the start points the same check run proposed name
            // shapes by their place before it, so they follow their shapes to the new place.
            let mut moved_to: Option<Vec<usize>> = None;
            for f in findings.iter().filter(|f| f.code == "contour-order") {
                let order = &f.location.order;
                if moved_to.is_some() || order.len() != layer.shapes.len() {
                    out.skipped.push((f.check, "the result does not say how to make the masters compatible".to_string()));
                    continue;
                }
                let mut inverse = vec![0; order.len()];
                for (to, &from) in order.iter().enumerate() {
                    if let Some(slot) = inverse.get_mut(from) {
                        *slot = to;
                    }
                }
                out.calls.push(("glyph.setShapeOrder", json!({"glyph": g, "layer": l, "order": order})));
                moved_to = Some(inverse);
            }
            // Then the paths to reverse, and the start nodes counted after the reversal.
            let mut reverse = BTreeSet::new();
            let mut starts = Vec::new();
            for f in findings.iter().filter(|f| f.code != "contour-order") {
                let reversed = f.data.get("reverse").is_some_and(|r| *r != 0.0);
                let start = match (f.code, f.location.points.first()) {
                    ("wrong-start-point", Some(&[shape, node])) if !reversed => Some((shape, node)),
                    ("wrong-start-point", Some(&[shape, node])) => match layer.shapes.get(shape) {
                        Some(Shape::Path(path)) => reversed_index(path, node).map(|n| (shape, n)),
                        _ => None,
                    },
                    _ => None,
                };
                let Some((shape, node)) = start else {
                    out.skipped.push((f.check, "the result does not say how to make the masters compatible".to_string()));
                    continue;
                };
                let shape = moved_to.as_ref().and_then(|m| m.get(shape).copied()).unwrap_or(shape);
                if reversed {
                    reverse.insert(shape);
                }
                starts.push((shape, node));
            }
            if !reverse.is_empty() {
                let shapes: Vec<usize> = reverse.into_iter().collect();
                out.calls.push(("path.reverseContours", json!({"glyph": g, "layer": l, "shapes": shapes})));
            }
            for (shape, node) in starts {
                out.calls.push(("path.makeNodeFirst", json!({"glyph": g, "layer": l, "point": [shape, node]})));
            }
        }
        "removeDuplicateAnchors" => {
            let names: BTreeSet<&str> = findings.iter().flat_map(|f| f.location.anchors.iter().map(String::as_str)).collect();
            for name in names {
                let Some(first) = layer.anchors.iter().find(|a| a.name == name) else { continue };
                out.calls.push(("anchor.remove", json!({"glyph": g, "layer": l, "anchors": [name]})));
                out.calls.push(("anchor.add", json!({"glyph": g, "layer": l, "name": name, "x": first.x, "y": first.y})));
            }
        }
        "addAnchors" => out.calls.push(("glyph.setAnchors", json!({"glyph": g, "layer": l}))),
        "updateMetrics" => {
            let keyed = glyph.info.metrics_keys.width.is_some() || layer.info.metrics_keys.width.is_some();
            let other = if keyed {
                None
            } else if findings.iter().any(|f| f.check == "check.metrics.whitespaceWidths") {
                font.glyphs.as_arcs().iter().find(|x| x.unicodes.contains(&0x20)).map(|x| x.name.clone())
            } else if findings.iter().any(|f| f.check == "check.metrics.accentedWidths") {
                layer.shapes.iter().find_map(|s| match s {
                    Shape::Component(c) => Some(c.glyph.clone()),
                    _ => None,
                })
            } else {
                None
            };
            if let Some(other) = other.filter(|o| o != g) {
                out.calls.push(("spacing.setWidth", json!({"glyphs": [g], "layer": l, "value": format!("={other}")})));
            }
            out.calls.push(("glyph.updateMetrics", json!({"glyphs": [g], "layer": l})));
        }
        "zeroMarkWidths" if layer.width != 0.0 => out.calls.push(("spacing.setWidth", json!({"glyphs": [g], "layer": l, "value": 0}))),
        "zeroMarkWidths" => {}
        _ => return None,
    }
    Some(out)
}

/// Where node `k` of `path` stands once `path.reverseContours` has reversed it: the same order
/// that command builds (a closed path keeps its start node).
fn reversed_index(path: &Path, k: usize) -> Option<usize> {
    let segs = path.segments();
    let last = segs.last()?;
    let mut order = Vec::with_capacity(path.nodes.len());
    if !path.closed {
        order.push(last.end);
    }
    for seg in segs.iter().rev() {
        order.extend(seg.handles.iter().rev());
        order.push(seg.start);
    }
    if path.closed {
        order.rotate_right(1);
    }
    if order.len() != path.nodes.len() {
        return None;
    }
    order.iter().position(|&i| i == k)
}

/// The node before and after `i` on its path, wrapping on a closed path.
fn neighbours(path: &Path, i: usize) -> (Option<usize>, Option<usize>) {
    let n = path.nodes.len();
    if n < 2 || i >= n {
        return (None, None);
    }
    let prev = if i > 0 { Some(i - 1) } else { path.closed.then_some(n - 1) };
    let next = if i + 1 < n { Some(i + 1) } else { path.closed.then_some(0) };
    (prev, next)
}

fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.hypot();
    (len > 0.0 && len.is_finite()).then(|| v / len)
}

/// Where the handles of smooth node `i` go to lie on one line through it, keeping their
/// lengths: the line along the mean of the incoming and outgoing directions, or along the
/// straight segment when one side is a line, which does not move.
fn aligned_handles(path: &Path, i: usize) -> Option<Vec<(usize, Point)>> {
    let node = path.nodes.get(i).filter(|n| n.on_curve())?.point();
    let (pi, ni) = neighbours(path, i);
    let (pi, ni) = (pi?, ni?);
    let (before, after) = (path.nodes.get(pi)?, path.nodes.get(ni)?);
    let (len_in, len_out) = ((node - before.point()).hypot(), (after.point() - node).hypot());
    let (d_in, d_out) = (unit(node - before.point())?, unit(after.point() - node)?);
    let (h_in, h_out) = (!before.on_curve(), !after.on_curve());
    let d = match (h_in, h_out) {
        (true, true) => unit(d_in + d_out)?,
        (true, false) => d_out,
        (false, true) => d_in,
        (false, false) => return None,
    };
    let mut out = Vec::new();
    if h_in {
        out.push((pi, node - d * len_in));
    }
    if h_out {
        out.push((ni, node + d * len_out));
    }
    out.iter().all(|(_, p)| p.is_finite()).then_some(out)
}

/// Handle `h` turned onto the horizontal or vertical through its node, whichever is nearer,
/// keeping its length.
fn snapped_handle(path: &Path, h: usize) -> Option<Point> {
    let handle = path.nodes.get(h).filter(|n| !n.on_curve())?.point();
    let (pi, ni) = neighbours(path, h);
    let owner = [pi, ni].into_iter().flatten().filter_map(|k| path.nodes.get(k)).find(|n| n.on_curve())?.point();
    let v = handle - owner;
    let len = v.hypot();
    let at = if v.x.abs() >= v.y.abs() { Point::new(owner.x + len.copysign(v.x), owner.y) } else { Point::new(owner.x, owner.y + len.copysign(v.y)) };
    at.is_finite().then_some(at)
}
