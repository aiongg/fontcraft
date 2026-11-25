//! Match Masters and Relocate Start Points (M9.34, `glyph.matchMasters`,
//! `glyph.relocateStartPoints`, `docs/plans/M9.md` › M9.34).
//!
//! Match Masters makes a glyph's layers follow a reference layer without changing any outline:
//! on every other master, intermediate and alternate layer it reorders the paths and components,
//! turns paths round and moves start nodes. It never adds, removes or moves a node, so a layer
//! whose paths differ in structure from the reference's is named in `skipped` and left alone.
//!
//! On one target layer:
//! 1. Both layers must hold the same number of paths and the same components (by base glyph).
//!    Shapes the model keeps unread are refused; images keep their places.
//! 2. Each layer gets a frame: the box of its path nodes, scaled to a unit square, so masters of
//!    different weights and widths compare. A path's features are its closedness, its node
//!    count and node-kind counts (its structure), the mean of its on-curve nodes in the frame
//!    (its centroid) and its area in the frame.
//! 3. Paths pair up greedily, cheapest first, among pairs of equal structure; the cost is the
//!    squared distance between the centroids plus the squared difference of the areas.
//!    Components pair up by base glyph, nearest offset first, in the reference's order.
//! 4. Each target path takes the reference path's direction (by the sign of its area; an open
//!    path or one without area tries both directions) and the start node whose node kinds line
//!    up with the reference's and whose nodes lie nearest the reference's in the frames.
//! 5. The layer's shapes take the reference's order.
//!
//! Without `strong`, a layer already compatible with the reference (the same shapes in the same
//! order, node for node) is left alone; `strong` matches it all the same, which fixes start
//! nodes and orders that interpolate but do not correspond.
//!
//! Relocate Start Points gives each closed path the start node nearest the bottom-left corner of
//! the layer's outline (the box of its paths).

use fontcraft_geom::Shape as _;
use fontcraft_model::{Component, Font, Glyph, Layer, NodeKind, Path, Shape};
use serde_json::Value;

use super::common::{Outcome, in_range, scope};
use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind as K};

/// Most paths of one layer that Match Masters pairs up; the pairs grow with the square.
const MAX_MATCH_PATHS: usize = 1_000;
/// Node comparisons Match Masters may make on one layer while it chooses directions and starts.
const MAX_ALIGN_WORK: usize = 50_000_000;
/// A closed path with less area than this, in square units, has no direction.
const MIN_AREA: f64 = 1e-9;
/// Most master ids one `masters` list may hold.
const MAX_MASTER_IDS: usize = 1_024;

type Op<T> = std::result::Result<T, String>;
/// The new shapes of each layer a command changes, by layer id.
type Changes = Vec<(String, Vec<Shape>)>;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.matchMasters",
            "Match Masters",
            ["Glyph"],
            None,
            "{glyphs: [names] | glyph: name, reference?: id (a master or layer id; default the first master), masters?: [id] (default every master), strong?: bool = false}. Makes every master, intermediate and alternate layer of the glyphs (of the given masters) follow the reference layer: pairs paths of the same structure by centroid and area, turns each to the reference path's direction, moves its start node to line up with the reference's nodes, pairs components by base glyph and puts the shapes in the reference's order. Nodes are never added or removed; a layer it cannot match is named in `skipped` and left as it was. Without strong a layer already compatible with the reference is left alone. A locked glyph is skipped (an error with one `glyph`) → {glyphs: [changed names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            match_masters
        )
        .targets(&[K::Glyphs]),
        cmd!(
            "glyph.relocateStartPoints",
            "Relocate Start Points",
            ["Glyph"],
            None,
            "{glyphs: [names] | glyph: name, masters?: [id] (default every master)}. On every master, intermediate and alternate layer of the glyphs, moves each closed path's start to its on-curve node nearest the bottom-left corner of the layer's paths (ties: the lower, then the leftmost node). Open paths keep their start. A locked glyph is skipped (an error with one `glyph`) → {glyphs: [changed names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            relocate_start_points
        )
        .targets(&[K::Glyphs]),
    ]
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// The glyphs to work on, and whether one `glyph` was given.
fn glyph_scope(p: &Params, font: &Font) -> Result<(Vec<String>, bool)> {
    if let Some(key) = ["layer", "layers", "side"].iter().find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("takes no `{key}`: it acts on every master, intermediate and alternate layer (narrow it with `masters`)")));
    }
    let scope = scope(p, font)?;
    Ok((scope.glyphs, scope.single))
}

/// The `masters` list, checked against the font; `None` for every master.
fn master_filter(p: &Params, font: &Font) -> Result<Option<Vec<String>>> {
    if let Some(Value::Array(a)) = p.get("masters")
        && a.len() > MAX_MASTER_IDS
    {
        return Err(p.bad(format!("`masters` holds {} ids; at most {MAX_MASTER_IDS} are allowed", a.len())));
    }
    let Some(ids) = p.opt_strings("masters")? else { return Ok(None) };
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if !font.masters.iter().any(|m| m.id == id) {
            return Err(p.bad(format!("there is no master `{id}`")));
        }
        if !out.iter().any(|o| o == id) {
            out.push(id.to_string());
        }
    }
    if out.is_empty() {
        return Err(p.bad("`masters` is empty"));
    }
    Ok(Some(out))
}

/// A master layer, or an intermediate or alternate layer, of one of `masters` (any master
/// when `None`).
fn takes_part(l: &Layer, masters: Option<&[String]>) -> bool {
    let kind = l.is_master_layer() || !l.info.attrs.coordinates.is_empty() || !l.info.attrs.axis_rules.is_empty();
    kind && masters.is_none_or(|m| m.contains(&l.master))
}

/// Runs `f` on the glyphs' layers: `f` gets the glyph and returns the new shapes of each layer
/// it changes, adding skips. Changes are applied only after `f` has read the glyph. A locked
/// glyph is an error when `single`, else skipped.
fn each_glyph(font: &mut Font, glyphs: &[String], single: bool, mut f: impl FnMut(&str, &Glyph, &mut Outcome) -> Result<Changes>) -> Result<Outcome> {
    let mut out = Outcome::default();
    for name in glyphs {
        crate::work::check()?;
        let Some(g) = font.glyph(name) else { continue };
        let g: &Glyph = g;
        if g.info.locked {
            if single {
                return Err(EngineError::Locked(name.clone()));
            }
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        let changes = f(name, g, &mut out)?;
        if changes.is_empty() {
            continue;
        }
        let mut g = font.glyph_mut(name)?;
        for (id, shapes) in changes {
            if let Some(layer) = g.layers.iter_mut().find(|l| l.id == id) {
                layer.shapes = shapes;
            }
        }
        out.glyphs.push(name.clone());
    }
    Ok(out)
}

// ── Match Masters ───────────────────────────────────────────────────────────────────────────────

fn match_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.matchMasters", v)?;
    let font = s.font()?;
    let (glyphs, single) = glyph_scope(&p, font)?;
    let masters = master_filter(&p, font)?;
    let reference = match p.opt_str("reference")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let strong = p.bool_or("strong", false)?;
    let out = s.edit("Match Masters", |font| {
        each_glyph(font, &glyphs, single, |name, g, out| {
            let Some(base) = g.layers.iter().find(|l| l.id == reference) else {
                let why = format!("has no layer `{reference}` to match against");
                if single {
                    return Err(p.bad(format!("glyph `{name}` {why}")));
                }
                out.skip(name, None, why);
                return Ok(Vec::new());
            };
            let mut changes = Vec::new();
            for layer in g.layers.iter().filter(|l| l.id != base.id && takes_part(l, masters.as_deref())) {
                if !strong && compatible(base, layer) {
                    continue;
                }
                match match_layer(base, layer) {
                    Ok(Some(shapes)) => changes.push((layer.id.clone(), shapes)),
                    Ok(None) => {}
                    Err(why) => out.skip(name, Some(layer.id.as_str()), why),
                }
            }
            Ok(changes)
        })
    })?;
    Ok(out.to_json())
}

/// The paths, components and unread shapes of a layer, in order (images left out).
fn outline(layer: &Layer) -> impl Iterator<Item = &Shape> {
    layer.shapes.iter().filter(|s| !matches!(s, Shape::Image(_)))
}

/// Whether two layers have the same shapes in the same order, node kind for node kind, as the
/// compatibility check sees them (anchors aside).
fn compatible(a: &Layer, b: &Layer) -> bool {
    let (sa, sb): (Vec<&Shape>, Vec<&Shape>) = (outline(a).collect(), outline(b).collect());
    sa.len() == sb.len()
        && sa.iter().zip(&sb).all(|(x, y)| match (x, y) {
            (Shape::Path(p), Shape::Path(q)) => {
                p.closed == q.closed && p.nodes.len() == q.nodes.len() && p.nodes.iter().zip(&q.nodes).all(|(m, n)| m.kind == n.kind)
            }
            (Shape::Component(c), Shape::Component(d)) => c.glyph == d.glyph,
            (Shape::Raw(_), Shape::Raw(_)) => true,
            _ => false,
        })
}

/// A layer's node box, as a unit square.
#[derive(Clone, Copy)]
struct Frame {
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
}

impl Frame {
    /// The frame of `paths`; a box thinner than one unit counts as one unit across.
    fn of(paths: &[&Path]) -> Op<Self> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        for n in paths.iter().flat_map(|p| &p.nodes) {
            if !(in_range(n.x) && in_range(n.y)) {
                return Err(format!("a node's coordinate ({}, {}) is not a number or out of range", n.x, n.y));
            }
            b = Some(b.map_or((n.x, n.y, n.x, n.y), |(x0, y0, x1, y1)| (x0.min(n.x), y0.min(n.y), x1.max(n.x), y1.max(n.y))));
        }
        let (x0, y0, x1, y1) = b.unwrap_or((0.0, 0.0, 1.0, 1.0));
        Ok(Self { x0, y0, w: (x1 - x0).max(1.0), h: (y1 - y0).max(1.0) })
    }

    fn at(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.x0) / self.w, (y - self.y0) / self.h)
    }
}

/// What pairing needs of one path.
struct Feature {
    closed: bool,
    len: usize,
    /// Off-curve, line, curve and quadratic nodes.
    kinds: [usize; 4],
    centroid: (f64, f64),
    area: f64,
}

impl Feature {
    fn new(path: &Path, frame: &Frame) -> Self {
        let mut kinds = [0usize; 4];
        for n in &path.nodes {
            let slot = match n.kind {
                NodeKind::OffCurve => kinds.get_mut(0),
                NodeKind::Line => kinds.get_mut(1),
                NodeKind::Curve => kinds.get_mut(2),
                NodeKind::QCurve => kinds.get_mut(3),
            };
            if let Some(slot) = slot {
                *slot += 1;
            }
        }
        // A path of control points only (a quadratic ring) is measured by its control points.
        let any_on = path.nodes.iter().any(|n| n.on_curve());
        let points: Vec<(f64, f64)> = path.nodes.iter().filter(|n| n.on_curve() || !any_on).map(|n| frame.at(n.x, n.y)).collect();
        let count = points.len().max(1) as f64;
        let (sx, sy) = points.iter().fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
        let area = if path.closed { raw_area(path).map_or(0.0, |a| a.abs() / (frame.w * frame.h)) } else { 0.0 };
        Self { closed: path.closed, len: path.nodes.len(), kinds, centroid: (sx / count, sy / count), area }
    }

    fn same_structure(&self, other: &Feature) -> bool {
        self.closed == other.closed && self.len == other.len && self.kinds == other.kinds
    }

    fn cost(&self, other: &Feature) -> f64 {
        let (dx, dy) = (self.centroid.0 - other.centroid.0, self.centroid.1 - other.centroid.1);
        let da = self.area - other.area;
        dx * dx + dy * dy + da * da
    }
}

/// The signed area of a path (positive: counter-clockwise, y up), if it can be read.
fn raw_area(path: &Path) -> Option<f64> {
    path.to_bez().ok().map(|b| b.area()).filter(|a| a.is_finite())
}

/// The direction of a closed path with area: `true` for counter-clockwise.
fn direction(path: &Path) -> Option<bool> {
    if !path.closed {
        return None;
    }
    raw_area(path).filter(|a| a.abs() >= MIN_AREA).map(|a| a > 0.0)
}

/// The new shapes of `target` matched to `base`, `None` when they already follow it, or why it
/// cannot be matched.
fn match_layer(base: &Layer, target: &Layer) -> Op<Option<Vec<Shape>>> {
    let (rs, ts): (Vec<&Shape>, Vec<&Shape>) = (outline(base).collect(), outline(target).collect());
    if rs.iter().chain(&ts).any(|s| matches!(s, Shape::Raw(_))) {
        return Err("holds a shape that cannot be read (a shape group or a path with unsupported nodes)".into());
    }
    let paths = |shapes: &[&Shape]| -> Vec<Path> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    };
    let comps = |shapes: &[&Shape]| -> Vec<Component> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Component(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    };
    let (rp, tp) = (paths(&rs), paths(&ts));
    if rp.len() != tp.len() {
        return Err(format!("has {} paths where the reference has {}", tp.len(), rp.len()));
    }
    if rp.len() > MAX_MATCH_PATHS {
        return Err(format!("has {} paths; at most {MAX_MATCH_PATHS} can be matched", rp.len()));
    }
    let matched_paths = match_paths(&rp, &tp)?;
    let matched_comps = match_components(&comps(&rs), &comps(&ts))?;
    let (mut np, mut nc) = (matched_paths.into_iter(), matched_comps.into_iter());
    let mut ordered = Vec::with_capacity(rs.len());
    for s in &rs {
        match s {
            Shape::Path(_) => ordered.extend(np.next().map(Shape::Path)),
            Shape::Component(_) => ordered.extend(nc.next().map(Shape::Component)),
            _ => {}
        }
    }
    let mut fill = ordered.into_iter();
    let mut shapes = Vec::with_capacity(target.shapes.len());
    for s in &target.shapes {
        if matches!(s, Shape::Image(_)) {
            shapes.push(s.clone());
        } else if let Some(next) = fill.next() {
            shapes.push(next);
        }
    }
    shapes.extend(fill);
    Ok((shapes != target.shapes).then_some(shapes))
}

/// The target paths, each turned and restarted, in the order of the reference paths they pair
/// with.
fn match_paths(rp: &[Path], tp: &[Path]) -> Op<Vec<Path>> {
    let (rr, tr): (Vec<&Path>, Vec<&Path>) = (rp.iter().collect(), tp.iter().collect());
    let (rf, tf) = (Frame::of(&rr)?, Frame::of(&tr)?);
    let rfe: Vec<Feature> = rp.iter().map(|p| Feature::new(p, &rf)).collect();
    let tfe: Vec<Feature> = tp.iter().map(|p| Feature::new(p, &tf)).collect();
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (r, a) in rfe.iter().enumerate() {
        for (t, b) in tfe.iter().enumerate() {
            if a.same_structure(b) {
                pairs.push((a.cost(b), r, t));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut of_ref: Vec<Option<usize>> = vec![None; rp.len()];
    let mut taken = vec![false; tp.len()];
    for (_, r, t) in pairs {
        let free_ref = of_ref.get(r).is_some_and(Option::is_none);
        let free_target = taken.get(t).is_some_and(|x| !x);
        if free_ref && free_target {
            if let Some(slot) = of_ref.get_mut(r) {
                *slot = Some(t);
            }
            if let Some(slot) = taken.get_mut(t) {
                *slot = true;
            }
        }
    }
    let work = rp.iter().fold(0usize, |w, p| w.saturating_add(p.nodes.len().saturating_mul(p.nodes.len()).saturating_mul(2)));
    if work > MAX_ALIGN_WORK {
        return Err("has too many nodes to match".into());
    }
    let mut out = Vec::with_capacity(rp.len());
    for (r, t) in of_ref.iter().enumerate() {
        let (Some(reference), Some(f)) = (rp.get(r), rfe.get(r)) else { continue };
        let Some(target) = t.and_then(|t| tp.get(t)) else {
            return Err(format!("has no path like the reference's path {r} ({} nodes, {})", f.len, if f.closed { "closed" } else { "open" }));
        };
        let aligned = align(reference, &rf, target, &tf).ok_or_else(|| format!("has no start node that lines up with the reference's path {r}"))?;
        out.push(aligned);
    }
    Ok(out)
}

/// `target` turned and restarted to follow `reference` node for node: the reference's direction
/// where both have area, else whichever direction fits better; then the start node whose node
/// kinds line up and whose nodes lie nearest the reference's in the frames. On a tie the path
/// keeps its direction and start. `None` when no direction and start line the kinds up.
fn align(reference: &Path, rf: &Frame, target: &Path, tf: &Frame) -> Option<Path> {
    let n = reference.nodes.len();
    if target.nodes.len() != n || reference.closed != target.closed {
        return None;
    }
    let mut reversed = target.clone();
    reversed.reverse();
    let mut candidates = match (direction(reference), direction(target)) {
        (Some(want), Some(have)) if want == have => vec![target.clone()],
        (Some(_), Some(_)) => vec![reversed],
        _ => vec![target.clone(), reversed],
    };
    let rp: Vec<(f64, f64)> = reference.nodes.iter().map(|m| rf.at(m.x, m.y)).collect();
    let mut best: Option<(f64, usize, usize)> = None;
    for (c, q) in candidates.iter_mut().enumerate() {
        // The kind of an open path's first node starts no segment: it follows the reference's.
        if !q.closed
            && let (Some(first), Some(want)) = (q.nodes.first_mut(), reference.nodes.first())
            && first.on_curve()
            && want.on_curve()
        {
            first.kind = want.kind;
        }
        let starts = if q.closed { n } else { n.min(1) };
        for k in 0..starts {
            let Some(cost) = fit(reference, &rp, q, tf, k) else { continue };
            if best.is_none_or(|(b, _, _)| cost < b) {
                best = Some((cost, c, k));
            }
        }
    }
    let (_, c, k) = best?;
    let mut out = candidates.into_iter().nth(c)?;
    out.nodes.rotate_left(k);
    Some(out)
}

/// How far `q`, started at node `k`, lies from the reference's nodes `rp` (in the frames), or
/// `None` when its node kinds do not line up.
fn fit(reference: &Path, rp: &[(f64, f64)], q: &Path, tf: &Frame, k: usize) -> Option<f64> {
    let n = q.nodes.len();
    if n == 0 {
        return Some(0.0);
    }
    let mut cost = 0.0;
    for (i, (m, &(rx, ry))) in reference.nodes.iter().zip(rp).enumerate() {
        let node = q.nodes.get((k + i) % n)?;
        if node.kind != m.kind {
            return None;
        }
        let (x, y) = tf.at(node.x, node.y);
        cost += (x - rx) * (x - rx) + (y - ry) * (y - ry);
    }
    Some(cost)
}

/// The target components in the reference's order: each reference component takes the unused
/// target component of the same base glyph whose offset is nearest.
fn match_components(rc: &[Component], tc: &[Component]) -> Op<Vec<Component>> {
    if rc.len() != tc.len() {
        return Err(format!("has {} components where the reference has {}", tc.len(), rc.len()));
    }
    let offset = |c: &Component| {
        let [_, _, _, _, e, f] = c.affine().as_coeffs();
        (e, f)
    };
    let mut taken = vec![false; tc.len()];
    let mut out = Vec::with_capacity(rc.len());
    for r in rc {
        let (rx, ry) = offset(r);
        let pick = tc
            .iter()
            .enumerate()
            .filter(|(i, c)| c.glyph == r.glyph && taken.get(*i).is_some_and(|x| !x))
            .map(|(i, c)| {
                let (x, y) = offset(c);
                (i, (x - rx) * (x - rx) + (y - ry) * (y - ry))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let Some((i, _)) = pick else {
            return Err(format!("has no component of `{}` to pair with the reference's", r.glyph));
        };
        if let Some(slot) = taken.get_mut(i) {
            *slot = true;
        }
        out.extend(tc.get(i).cloned());
    }
    Ok(out)
}

// ── Relocate Start Points ───────────────────────────────────────────────────────────────────────

fn relocate_start_points(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.relocateStartPoints", v)?;
    let font = s.font()?;
    let (glyphs, single) = glyph_scope(&p, font)?;
    let masters = master_filter(&p, font)?;
    let out = s.edit("Relocate Start Points", |font| {
        each_glyph(font, &glyphs, single, |name, g, out| {
            let mut changes = Vec::new();
            for layer in g.layers.iter().filter(|l| takes_part(l, masters.as_deref())) {
                match relocate_in(layer) {
                    Ok(Some(shapes)) => changes.push((layer.id.clone(), shapes)),
                    Ok(None) => {}
                    Err(why) => out.skip(name, Some(layer.id.as_str()), why),
                }
            }
            Ok(changes)
        })
    })?;
    Ok(out.to_json())
}

/// The layer's shapes with each closed path started at its on-curve node nearest the
/// bottom-left corner of the layer's paths; `None` when every path already starts there.
fn relocate_in(layer: &Layer) -> Op<Option<Vec<Shape>>> {
    let mut corner: Option<(f64, f64)> = None;
    for shape in &layer.shapes {
        let Shape::Path(path) = shape else { continue };
        let bez = path.to_bez()?;
        if bez.elements().is_empty() {
            continue;
        }
        let r = bez.bounding_box();
        corner = Some(corner.map_or((r.x0, r.y0), |(x, y)| (x.min(r.x0), y.min(r.y0))));
    }
    let Some((cx, cy)) = corner else { return Ok(None) };
    let mut shapes = layer.shapes.clone();
    let mut changed = false;
    for shape in shapes.iter_mut() {
        let Shape::Path(path) = shape else { continue };
        if !path.closed {
            continue;
        }
        let d = |x: f64, y: f64| (x - cx) * (x - cx) + (y - cy) * (y - cy);
        let start = path
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.on_curve())
            .min_by(|(i, a), (j, b)| d(a.x, a.y).total_cmp(&d(b.x, b.y)).then(a.y.total_cmp(&b.y)).then(a.x.total_cmp(&b.x)).then(i.cmp(j)))
            .map(|(i, _)| i);
        if let Some(k) = start.filter(|&k| k > 0) {
            path.nodes.rotate_left(k);
            changed = true;
        }
    }
    Ok(changed.then_some(shapes))
}
