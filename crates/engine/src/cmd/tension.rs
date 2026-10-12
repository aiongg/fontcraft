//! Curve tension and harmonizing (`docs/spec/90-secondary-references.md` › Tunni lines, Numeric
//! curve tension, Harmonize). Rows: `path.setCurveTension`, `path.harmonizeNodes`,
//! `path.harmonizeHandles`, `path.selectTunniLines`, and the master parameter `curveTension`.
//!
//! The geometry is `fontcraft_pathops::tension`. Tension is a handle's length as a percentage of
//! the distance from its node to the Tunni point, where the two handle lines of a segment meet.
//! A segment whose handle lines do not meet ahead of both nodes has no tension: Set Curve
//! Tension leaves it as it is and lists it. Harmonizing works on smooth nodes between two curve
//! segments, one node after the other in path order, so a node sees the changes made at the
//! nodes before it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fontcraft_geom::{CubicBez, Point, Segment, round_point};
use fontcraft_model::{Font, Layer, LayerParts, LayerSide, NodeKind, Path, Shape};
use fontcraft_pathops::tension::{self as tn, Joint, Order};
use serde_json::{Value, json};

use super::common::{Scope, each_layer, grid, in_range, scope};
use super::node::group;
use super::*;
use crate::params::{Params, TargetKind as K, Targets};

/// The master parameter holding the default tension, in percent.
pub const PARAMETER: &str = "curveTension";
/// The tension used when neither the command nor the master gives one.
pub const DEFAULT_TENSION: f64 = 55.0;
/// The range a tension is held to, in percent.
pub const MIN_TENSION: f64 = 1.0;
pub const MAX_TENSION: f64 = 100.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.setCurveTension",
            "Set Curve Tension",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]]} or {glyphs: [names], layer?: id | layers?: [ids]}, tension?: percent, relative?: bool = false. Sets both handles of each curve segment to `tension` percent of the distance from its node to the point where the two handle lines meet (1 to 100; default the master's curveTension parameter, else 55). With relative, `tension` is a signed step added to each handle's own tension, the result held to 1..100. A segment counts when one of its handles is in `points`, both its end nodes are, or it is in `segments`; with no list, every curve segment of the layer. A segment whose handle lines do not meet ahead of both nodes is left as it is and listed in `unchanged` → {glyphs, skipped, segments: count, unchanged: [{glyph, layer, segment: [shape, node], reason}], selection?}",
            has_doc,
            set_curve_tension
        )
        .targets(&[K::Glyphs, K::Layer, K::Points, K::Segments]),
        cmd!(
            "path.harmonizeNodes",
            "Harmonize Nodes",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]]} or {glyphs: [names], layer?: id | layers?: [ids]}. Moves each smooth node in `points` (every smooth node of the layer when there is no list) along the line between its two handles, which stay put, to where the curvature on both sides is equal. A node is harmonized only between two curve segments; one with a straight side at the node is left as it is and listed → {glyphs, skipped, nodes: count, unchanged: [{glyph, layer, node: [shape, node], reason}], selection?}",
            has_doc,
            harmonize_nodes
        )
        .targets(&[K::Glyphs, K::Layer, K::Points]),
        cmd!(
            "path.harmonizeHandles",
            "Harmonize Handles",
            ["Path", "Handles"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]]} or {glyphs: [names], layer?: id | layers?: [ids]}, order?: \"g2\" | \"g3\" = \"g2\". Keeps each smooth node in `points` (every smooth node of the layer when there is no list) where it is and divides the length of its two handles, their sum unchanged, so the curvature on both sides is equal. Order g3 also changes the lengths of the two neighbouring handles, along their own directions, so the rate of curvature change is equal too → {glyphs, skipped, nodes: count, unchanged: [{glyph, layer, node: [shape, node], reason}], selection?}",
            has_doc,
            harmonize_handles
        )
        .targets(&[K::Glyphs, K::Layer, K::Points]),
        cmd!(
            query "path.selectTunniLines",
            "Edit Tunni Lines",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]]} the Tunni lines of the curve segments that `points` or `segments` name (as for path.setCurveTension), or of every curve segment of the layer: each segment with its Tunni point (where the handle lines meet; null when they do not meet ahead of both nodes) and the tension of both handles in percent (null likewise) → {segments: [{segment: [shape, node], tunni: [x, y] | null, tension: [first, second] | null}], selection: {glyph, layer, side?, segments}}",
            has_doc,
            select_tunni_lines
        )
        .targets(&[K::Layer, K::Points, K::Segments]),
    ]
}

/// The default tension of `master`: its `curveTension` parameter when that is a number from 1 to
/// 100, else [`DEFAULT_TENSION`].
pub fn master_tension(font: &Font, master: &str) -> f64 {
    font.master(master)
        .ok()
        .and_then(|m| m.params.get(PARAMETER))
        .and_then(fontcraft_fontinfo::value::number)
        .filter(|v| (MIN_TENSION..=MAX_TENSION).contains(v))
        .unwrap_or(DEFAULT_TENSION)
}

// ── Addressing ─────────────────────────────────────────────────────────────────────────────────

/// A cubic segment as the indices of its four points on path `shape`.
#[derive(Clone, Copy, Debug)]
struct Cubic {
    shape: usize,
    start: usize,
    h1: usize,
    h2: usize,
    end: usize,
}

impl Cubic {
    fn of(shape: usize, path: &Path, seg: &Segment) -> Option<Self> {
        let &[h1, h2] = seg.handles.as_slice() else { return None };
        (path.nodes.get(seg.end).map(|n| n.kind) == Some(NodeKind::Curve)).then_some(Cubic { shape, start: seg.start, h1, h2, end: seg.end })
    }

    fn read(&self, path: &Path) -> Option<CubicBez> {
        let at = |i: usize| path.nodes.get(i).map(|n| n.point());
        Some(CubicBez::new(at(self.start)?, at(self.h1)?, at(self.h2)?, at(self.end)?))
    }
}

/// The cubic segments of `parts` that `t` names, in path order: those with a handle in `points`,
/// with both end nodes in `points`, or in `segments`. Every cubic segment when `t` is `None` or
/// has neither list.
fn curve_segments(parts: LayerParts, t: Option<&Targets>) -> Vec<Cubic> {
    let all = t.is_none_or(|t| t.points.is_empty() && t.segments.is_empty());
    let (points, ends) = t.map(|t| (group(&t.points), group(&t.segments))).unwrap_or_default();
    let empty = BTreeSet::new();
    let mut out = Vec::new();
    for (s, shape) in parts.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        let pts = points.get(&s).unwrap_or(&empty);
        let named_ends = ends.get(&s).unwrap_or(&empty);
        if !all && pts.is_empty() && named_ends.is_empty() {
            continue;
        }
        for seg in path.segments() {
            let Some(c) = Cubic::of(s, path, &seg) else { continue };
            let chosen =
                all || pts.contains(&c.h1) || pts.contains(&c.h2) || (pts.contains(&c.start) && pts.contains(&c.end)) || named_ends.contains(&c.end);
            if chosen {
                out.push(c);
            }
        }
    }
    out
}

/// A smooth node between two cubic segments.
#[derive(Clone, Copy, Debug)]
struct JointAt {
    shape: usize,
    node: usize,
    before: Cubic,
    after: Cubic,
}

impl JointAt {
    fn read(&self, path: &Path) -> Option<Joint> {
        Some(Joint { before: self.before.read(path)?, after: self.after.read(path)? })
    }
}

/// The smooth on-curve nodes of `parts` in `t`'s `points` (all of them when `t` is `None` or has
/// no points), in path order. A named node that is smooth but not between two cubic segments is
/// returned as `Err((shape, node))`; without a list such nodes are passed over.
fn joints(parts: LayerParts, t: Option<&Targets>) -> Vec<std::result::Result<JointAt, (usize, usize)>> {
    let all = t.is_none_or(|t| t.points.is_empty());
    let points: BTreeMap<usize, BTreeSet<usize>> = t.map(|t| group(&t.points)).unwrap_or_default();
    let mut out = Vec::new();
    for (s, shape) in parts.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        let named = points.get(&s);
        if !all && named.is_none() {
            continue;
        }
        let segments = path.segments();
        let by_end: HashMap<usize, &Segment> = segments.iter().map(|g| (g.end, g)).collect();
        let by_start: HashMap<usize, &Segment> = segments.iter().map(|g| (g.start, g)).collect();
        for (i, node) in path.nodes.iter().enumerate() {
            if !(node.on_curve() && node.smooth) || !(all || named.is_some_and(|n| n.contains(&i))) {
                continue;
            }
            let before = by_end.get(&i).and_then(|g| Cubic::of(s, path, g));
            let after = by_start.get(&i).and_then(|g| Cubic::of(s, path, g));
            match (before, after) {
                // A closed path of one segment meets itself at its only node.
                (Some(before), Some(after)) if before.h1 != after.h1 => out.push(Ok(JointAt { shape: s, node: i, before, after })),
                _ if all => {}
                _ => out.push(Err((s, i))),
            }
        }
    }
    out
}

fn set(path: &mut Path, i: usize, p: Point) {
    if let Some(n) = path.nodes.get_mut(i) {
        (n.x, n.y) = (p.x, p.y);
    }
}

/// Writes every `(index, point)` rounded to the grid, or none of them when one is out of range.
fn write_all(path: &mut Path, moves: &[(usize, Point)], step: f64) -> bool {
    let moves: Vec<(usize, Point)> = moves.iter().map(|&(i, p)| (i, round_point(p, step))).collect();
    if !moves.iter().all(|(_, p)| in_range(p.x) && in_range(p.y)) {
        return false;
    }
    for (i, p) in moves {
        set(path, i, p);
    }
    true
}

// ── Running over one layer or many ─────────────────────────────────────────────────────────────

/// What a command did on one layer: how many segments or nodes changed, and which were left as
/// they were and why.
#[derive(Default)]
struct Done {
    changed: usize,
    unchanged: Vec<((usize, usize), String)>,
}

/// One layer's work: the layer, the side, and the target object (one `glyph` only). `Err` is why
/// the layer was left as it is.
type LayerOp<'a> = dyn Fn(&mut Layer, LayerSide, Option<&Targets>) -> std::result::Result<Done, String> + 'a;

enum Form {
    One(Targets),
    Many(Scope),
}

fn form(p: &Params, font: &Font, lists: &[&str]) -> Result<Form> {
    if p.get("glyph").is_some() && p.get("glyphs").is_some() {
        return Err(p.bad("give `glyph` or `glyphs`, not both"));
    }
    if p.get("glyph").is_some() && p.get("layers").is_none() {
        let t = Targets::parse(p)?;
        let given = [
            ("points", !t.points.is_empty()),
            ("segments", !t.segments.is_empty()),
            ("shapes", !t.shapes.is_empty()),
            ("anchors", !t.anchors.is_empty()),
            ("guides", !t.guides.is_empty()),
            ("annotations", !t.annotations.is_empty()),
        ];
        if let Some((key, _)) = given.iter().find(|(key, on)| *on && !lists.contains(key)) {
            return Err(p.bad(format!("does not take `{key}`")));
        }
        return Ok(Form::One(t));
    }
    if let Some(key) = ["points", "segments", "segment", "side"].iter().find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("`{key}` goes with one `glyph` and one `layer`")));
    }
    Ok(Form::Many(scope(p, font)?))
}

/// Runs `op` on the addressed layers as one undo step. `key` names the count in the result and
/// `what` the address of an unchanged item (`segment` or `node`).
fn run_layers(s: &mut Session, p: &Params, label: &str, lists: &[&str], key: &str, what: &str, op: &LayerOp<'_>) -> Result<Value> {
    let entry = |glyph: &str, layer: &str, (shape, index): (usize, usize), reason: &str| {
        let mut v = json!({"glyph": glyph, "layer": layer, "reason": reason});
        if let Some(o) = v.as_object_mut() {
            o.insert(what.into(), json!([shape, index]));
        }
        v
    };
    match form(p, s.font()?, lists)? {
        Form::One(t) => {
            let layer = t.layer_id(s.font()?)?;
            let done = s.edit(label, |font| {
                t.edit_layer(font, |l| {
                    // Editing a background the layer does not have would add an empty one.
                    if t.side == LayerSide::Background && l.info.background.is_none() {
                        return Err(p.bad("the layer has no background"));
                    }
                    let done = op(l, t.side, Some(&t)).map_err(|m| p.bad(m))?;
                    match done.unchanged.first() {
                        Some((_, why)) if done.changed == 0 => Err(p.bad(why.clone())),
                        _ => Ok(done),
                    }
                })
            })?;
            let unchanged: Vec<Value> = done.unchanged.iter().map(|(at, why)| entry(&t.glyph, &layer, *at, why)).collect();
            let mut out = json!({"glyphs": [t.glyph], "skipped": [], key: done.changed, "unchanged": unchanged});
            if t.has_lists()
                && let Some(o) = out.as_object_mut()
            {
                o.insert("selection".into(), t.to_json());
            }
            Ok(out)
        }
        Form::Many(scope) => {
            let single = scope.single;
            let mut count = 0;
            let mut unchanged = Vec::new();
            let out = s.edit(label, |font| {
                each_layer(font, &scope, p.command(), |name, layer| match op(layer, LayerSide::Foreground, None) {
                    Ok(done) if done.changed == 0 && !done.unchanged.is_empty() => {
                        let why = done.unchanged.first().map(|(_, why)| why.clone()).unwrap_or_default();
                        if single { Err(p.bad(why)) } else { Ok(Some(why)) }
                    }
                    Ok(done) => {
                        count += done.changed;
                        unchanged.extend(done.unchanged.iter().map(|(at, why)| entry(name, &layer.id, *at, why)));
                        Ok(None)
                    }
                    Err(why) if single => Err(p.bad(why)),
                    Err(why) => Ok(Some(why)),
                })
            })?;
            let mut v = out.to_json();
            if let Some(o) = v.as_object_mut() {
                o.insert(key.into(), json!(count));
                o.insert("unchanged".into(), Value::Array(unchanged));
            }
            Ok(v)
        }
    }
}

// ── Commands ───────────────────────────────────────────────────────────────────────────────────

fn set_curve_tension(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.setCurveTension", v)?;
    let relative = p.bool_or("relative", false)?;
    let given = p.opt_f64("tension")?;
    match (relative, given) {
        (true, None) => return Err(p.bad("`tension` is required with `relative`: it is the signed step")),
        (true, Some(t)) if t.abs() > MAX_TENSION => {
            return Err(p.bad(format!("a relative `tension` must be between -{MAX_TENSION} and {MAX_TENSION}")));
        }
        (false, Some(t)) if !(MIN_TENSION..=MAX_TENSION).contains(&t) => {
            return Err(p.bad(format!("`tension` must be between {MIN_TENSION} and {MAX_TENSION} percent")));
        }
        _ => {}
    }
    let font = s.font()?;
    let step = grid(font);
    let defaults: HashMap<String, f64> = font.masters.iter().map(|m| (m.id.clone(), master_tension(font, &m.id))).collect();
    let op = move |layer: &mut Layer, side: LayerSide, t: Option<&Targets>| -> std::result::Result<Done, String> {
        let list = curve_segments(layer.parts(side), t);
        if list.is_empty() {
            return Err(if t.is_some_and(Targets::has_lists) {
                "none of the targets is on a curve segment"
            } else {
                "the layer has no curve segments"
            }
            .into());
        }
        let value = given.unwrap_or_else(|| defaults.get(&layer.master).copied().unwrap_or(DEFAULT_TENSION));
        let mut done = Done::default();
        let mut parts = layer.parts_mut(side);
        for c in list {
            let Some(path) = parts.path_mut(c.shape) else { continue };
            let Some(bez) = c.read(path) else { continue };
            let new = tn::tension(bez).and_then(|(a, b)| {
                let (a, b) = if relative {
                    ((a + value).clamp(MIN_TENSION, MAX_TENSION), (b + value).clamp(MIN_TENSION, MAX_TENSION))
                } else {
                    (value, value)
                };
                tn::with_tension(bez, a, b)
            });
            let written = new.is_some_and(|n| write_all(path, &[(c.h1, n.p1), (c.h2, n.p2)], step));
            if written {
                done.changed += 1;
            } else {
                done.unchanged.push(((c.shape, c.end), "the handle lines do not meet ahead of both nodes".into()));
            }
        }
        Ok(done)
    };
    run_layers(s, &p, "Set Curve Tension", &["points", "segments"], "segments", "segment", &op)
}

/// What a harmonize command does to one joint: the new joint and the points it moves.
fn harmonize(
    s: &mut Session,
    p: &Params,
    label: &str,
    f: impl Fn(Joint) -> std::result::Result<Joint, tn::Unchanged>,
    moved: impl Fn(&JointAt, &Joint) -> Vec<(usize, Point)>,
) -> Result<Value> {
    let step = grid(s.font()?);
    let op = move |layer: &mut Layer, side: LayerSide, t: Option<&Targets>| -> std::result::Result<Done, String> {
        let list = joints(layer.parts(side), t);
        if list.is_empty() {
            return Err(if t.is_some_and(Targets::has_lists) {
                "none of the targets is a smooth node"
            } else {
                "the layer has no smooth nodes between curve segments"
            }
            .into());
        }
        let mut done = Done::default();
        let mut parts = layer.parts_mut(side);
        for item in list {
            let j = match item {
                Ok(j) => j,
                Err(at) => {
                    done.unchanged.push((at, "the node is not between two curve segments".into()));
                    continue;
                }
            };
            let Some(path) = parts.path_mut(j.shape) else { continue };
            let Some(joint) = j.read(path) else { continue };
            match f(joint) {
                Ok(new) if write_all(path, &moved(&j, &new), step) => done.changed += 1,
                Ok(_) => done.unchanged.push(((j.shape, j.node), "the result is out of range".into())),
                Err(why) => done.unchanged.push(((j.shape, j.node), why.message().into())),
            }
        }
        Ok(done)
    };
    run_layers(s, p, label, &["points"], "nodes", "node", &op)
}

fn harmonize_nodes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.harmonizeNodes", v)?;
    harmonize(s, &p, "Harmonize Nodes", tn::harmonize_node, |j, new| vec![(j.node, new.node())])
}

fn harmonize_handles(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.harmonizeHandles", v)?;
    let order = match p.opt_str("order")? {
        None | Some("g2") => Order::G2,
        Some("g3") => Order::G3,
        Some(other) => return Err(p.bad(format!("`order` must be \"g2\" or \"g3\", got \"{other}\""))),
    };
    harmonize(
        s,
        &p,
        "Harmonize Handles",
        move |j| tn::harmonize_handles(j, order),
        move |j, new| {
            let mut v = vec![(j.before.h2, new.before.p2), (j.after.h1, new.after.p1)];
            if order == Order::G3 {
                v.extend([(j.before.h1, new.before.p1), (j.after.h2, new.after.p2)]);
            }
            v
        },
    )
}

fn select_tunni_lines(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.selectTunniLines", v)?;
    if p.get("glyphs").is_some() {
        return Err(p.bad("acts on one glyph: give `glyph`, not `glyphs`"));
    }
    let Form::One(t) = form(&p, s.font()?, &["points", "segments"])? else {
        return Err(p.bad("`glyph` is required"));
    };
    let r = t.resolve(s.font()?)?;
    let list = curve_segments(r.parts, Some(&t));
    let segments: Vec<Value> = list
        .iter()
        .filter_map(|c| {
            let bez = c.read(r.parts.path(c.shape)?)?;
            let tunni = tn::tunni_point(bez).map(|q| json!([q.x, q.y]));
            let tension = tn::tension(bez).map(|(a, b)| json!([a, b]));
            Some(json!({"segment": [c.shape, c.end], "tunni": tunni, "tension": tension}))
        })
        .collect();
    let mut selection = Targets::new(p.command(), t.glyph.clone(), Some(r.layer.id.clone()));
    selection.side = t.side;
    selection.segments = list.iter().map(|c| (c.shape, c.end)).collect();
    Ok(json!({"segments": segments, "selection": selection.to_json()}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Str, Value as Plist};
    use fontcraft_model::{Glyph, Node};

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn node(x: f64, y: f64, kind: NodeKind, smooth: bool) -> Node {
        let mut n = Node::new(x, y, kind);
        n.smooth = smooth;
        n
    }

    /// One open cubic from (0, 0) to (400, 400) whose handle lines meet at (0, 400), at 25 % on
    /// the first handle and 75 % on the second.
    fn meeting() -> Path {
        Path::new(
            false,
            vec![
                node(0.0, 0.0, NodeKind::Line, false),
                node(0.0, 100.0, NodeKind::OffCurve, false),
                node(100.0, 400.0, NodeKind::OffCurve, false),
                node(400.0, 400.0, NodeKind::Curve, false),
            ],
        )
    }

    /// One open cubic whose handles are parallel.
    fn parallel() -> Path {
        Path::new(
            false,
            vec![
                node(500.0, 0.0, NodeKind::Line, false),
                node(500.0, 100.0, NodeKind::OffCurve, false),
                node(800.0, 100.0, NodeKind::OffCurve, false),
                node(800.0, 0.0, NodeKind::Curve, false),
            ],
        )
    }

    /// A closed bowl with a smooth node 3 at the origin between two unbalanced curve segments.
    fn bowl() -> Path {
        Path::new(
            true,
            vec![
                node(-300.0, 300.0, NodeKind::Line, false),
                node(-300.0, 120.0, NodeKind::OffCurve, false),
                node(-100.0, 0.0, NodeKind::OffCurve, false),
                node(0.0, 0.0, NodeKind::Curve, true),
                node(250.0, 0.0, NodeKind::OffCurve, false),
                node(400.0, 150.0, NodeKind::OffCurve, false),
                node(400.0, 400.0, NodeKind::Curve, false),
            ],
        )
    }

    /// A document without a grid (so results are exact) holding glyph `g` with `paths`.
    fn session(paths: Vec<Path>) -> Session {
        let mut s = Session::with_document();
        s.edit("fixture", |f| {
            Arc::make_mut(&mut f.info).settings.grid_length = Some(0.0);
            let master = f.default_master()?.id.clone();
            for name in ["g", "h"] {
                let mut g = Glyph::new(name);
                let mut l = Layer::new(master.clone(), 600.0);
                l.shapes.extend(paths.iter().cloned().map(Shape::Path));
                g.layers.push(l);
                // A new document seeds `g` and `h`; the fixture replaces them.
                f.glyphs.remove(name);
                f.glyphs.push(g);
            }
            Ok(())
        })
        .unwrap();
        s
    }

    fn path(s: &Session, glyph: &str, shape: usize) -> Path {
        match &s.font().unwrap().glyph(glyph).unwrap().layers[0].shapes[shape] {
            Shape::Path(p) => p.clone(),
            other => panic!("not a path: {other:?}"),
        }
    }

    fn cubic(p: &Path, start: usize) -> CubicBez {
        let at = |i: usize| p.nodes[i % p.nodes.len()].point();
        CubicBez::new(at(start), at(start + 1), at(start + 2), at(start + 3))
    }

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * a.abs().max(b.abs()).max(1e-300)
    }

    #[test]
    fn set_curve_tension_puts_both_handles_at_the_percentage() {
        let mut s = session(vec![meeting()]);
        let r = run(&mut s, "path.setCurveTension", json!({"glyph": "g", "tension": 55}));
        assert_eq!(r["segments"], 1, "{r}");
        let p = path(&s, "g", 0);
        assert_eq!((p.nodes[1].x, p.nodes[1].y), (0.0, 220.0), "55 % of the 400 units to the Tunni point");
        assert_eq!((p.nodes[2].x, p.nodes[2].y), (180.0, 400.0));
        let (a, b) = tn::tension(cubic(&p, 0)).unwrap();
        assert!((a - 55.0).abs() < 1e-9 && (b - 55.0).abs() < 1e-9, "{a} {b}");
    }

    #[test]
    fn relative_tension_steps_each_handle_and_stays_in_range() {
        let mut s = session(vec![meeting()]);
        run(&mut s, "path.setCurveTension", json!({"glyph": "g", "segments": [[0, 3]], "tension": 5, "relative": true}));
        let (a, b) = tn::tension(cubic(&path(&s, "g", 0), 0)).unwrap();
        assert!((a - 30.0).abs() < 1e-9 && (b - 80.0).abs() < 1e-9, "25 + 5 and 75 + 5: {a} {b}");
        run(&mut s, "path.setCurveTension", json!({"glyph": "g", "points": [[0, 1]], "tension": 50, "relative": true}));
        let (a, b) = tn::tension(cubic(&path(&s, "g", 0), 0)).unwrap();
        assert!((a - 80.0).abs() < 1e-9 && (b - 100.0).abs() < 1e-9, "the second handle is held at 100: {a} {b}");
        let e = s.execute("path.setCurveTension", &json!({"glyph": "g", "relative": true})).unwrap_err();
        assert!(e.to_string().contains("signed step"), "{e}");
    }

    #[test]
    fn parallel_handles_are_left_alone_and_listed() {
        let mut s = session(vec![meeting(), parallel()]);
        let r = run(&mut s, "path.setCurveTension", json!({"glyph": "g", "tension": 60}));
        assert_eq!(r["segments"], 1, "{r}");
        assert_eq!(r["unchanged"][0]["segment"], json!([1, 3]), "{r}");
        assert_eq!(path(&s, "g", 1), parallel(), "the parallel segment keeps its handles");
        let before = s.doc().unwrap().font.clone();
        let e = s.execute("path.setCurveTension", &json!({"glyph": "g", "segments": [[1, 3]], "tension": 60})).unwrap_err();
        assert!(e.to_string().contains("do not meet"), "{e}");
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "a refused command changes nothing");
    }

    #[test]
    fn tension_defaults_to_the_master_parameter() {
        let mut s = session(vec![meeting()]);
        s.edit("parameter", |f| {
            let masters = Arc::make_mut(&mut f.masters);
            masters[0].params.set(PARAMETER, Plist::String(Str::from("60".to_string())));
            Ok(())
        })
        .unwrap();
        run(&mut s, "path.setCurveTension", json!({"glyphs": ["g", "h"]}));
        for glyph in ["g", "h"] {
            let (a, b) = tn::tension(cubic(&path(&s, glyph, 0), 0)).unwrap();
            assert!((a - 60.0).abs() < 1e-9 && (b - 60.0).abs() < 1e-9, "{glyph}: {a} {b}");
        }
        let font = s.font().unwrap();
        assert_eq!(master_tension(font, "no such master"), DEFAULT_TENSION);
    }

    #[test]
    fn harmonized_nodes_have_equal_curvature_on_both_sides() {
        let mut s = session(vec![bowl()]);
        let r = run(&mut s, "path.harmonizeNodes", json!({"glyph": "g"}));
        assert_eq!(r["nodes"], 1, "{r}");
        let p = path(&s, "g", 0);
        let k1 = tn::curvature_at_end(cubic(&p, 0)).unwrap();
        let k2 = tn::curvature_at_start(cubic(&p, 3)).unwrap();
        assert!(close(k1.curvature, k2.curvature, 1e-6), "{k1:?} {k2:?}");
        assert_eq!((p.nodes[2].x, p.nodes[4].x), (-100.0, 250.0), "the handles stay");
        assert!(p.nodes[3].x > 0.0 && p.nodes[3].y == 0.0, "the node slides along the handle line: {:?}", p.nodes[3]);
    }

    #[test]
    fn harmonized_handles_g3_match_curvature_and_its_rate() {
        let mut s = session(vec![bowl()]);
        let r = run(&mut s, "path.harmonizeHandles", json!({"glyph": "g", "points": [[0, 3]], "order": "g3"}));
        assert_eq!(r["nodes"], 1, "{r}");
        let p = path(&s, "g", 0);
        let k1 = tn::curvature_at_end(cubic(&p, 0)).unwrap();
        let k2 = tn::curvature_at_start(cubic(&p, 3)).unwrap();
        assert!(close(k1.curvature, k2.curvature, 1e-6), "{k1:?} {k2:?}");
        assert!(close(k1.rate, k2.rate, 1e-6), "{k1:?} {k2:?}");
        assert_eq!((p.nodes[3].x, p.nodes[3].y), (0.0, 0.0), "the node stays");
    }

    #[test]
    fn a_corner_or_straight_side_is_reported_not_changed() {
        let mut bent = bowl();
        bent.nodes[5] = node(400.0, 0.0, NodeKind::OffCurve, false);
        let mut s = session(vec![bent.clone()]);
        let e = s.execute("path.harmonizeHandles", &json!({"glyph": "g", "points": [[0, 3]]})).unwrap_err();
        assert!(e.to_string().contains("straight"), "{e}");
        let r = run(&mut s, "path.harmonizeNodes", json!({"glyphs": ["g", "h"]}));
        assert_eq!(r["skipped"].as_array().map(Vec::len), Some(2), "{r}");
        assert_eq!(path(&s, "g", 0), bent);
    }

    #[test]
    fn tunni_lines_of_the_layer_are_listed_and_selected() {
        let mut s = session(vec![meeting(), parallel()]);
        let r = run(&mut s, "path.selectTunniLines", json!({"glyph": "g"}));
        assert_eq!(r["segments"][0]["tunni"], json!([0.0, 400.0]), "{r}");
        assert_eq!(r["segments"][1]["tunni"], Value::Null, "{r}");
        assert_eq!(r["selection"]["segments"], json!([[0, 3], [1, 3]]), "{r}");
        let one = run(&mut s, "path.selectTunniLines", json!({"glyph": "g", "points": [[1, 2]]}));
        assert_eq!(one["selection"]["segments"], json!([[1, 3]]), "{one}");
    }
}
