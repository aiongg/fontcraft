//! Corner and point helpers from the secondary reference (`docs/spec/90-secondary-references.md`):
//! Make Ink Trap, Loop Corner, Replace with Corner, Align Points to Background, Collapse Points
//! and Same Width / Same Height. The geometry is in `fontcraft_pathops::corner_helpers`; this
//! module reads the parameters, converts paths to contours and back, and keeps the data of every
//! node that survives (name, identifier, lock, user data) and of every path.

use std::collections::{BTreeMap, HashSet};

use fontcraft_geom::Point;
use fontcraft_model::{Font, Glyph, LayerSide, MAX_COORDINATE, Master, Node, NodeKind, Path};
use fontcraft_pathops::{self as po, corner_helpers as ch};
use serde_json::{Value, json};

use super::common::{grid, in_range, master_metrics};
use super::node::group;
use super::*;
use crate::params::{Params, TargetKind, Targets};

/// The master custom parameter that holds the default ink trap width.
pub const INK_TRAP_WIDTH_PARAM: &str = "inkTrapWidth";
/// Ink trap width, as a share of the em, for a master without [`INK_TRAP_WIDTH_PARAM`].
const DEFAULT_INK_TRAP_SHARE: f64 = 0.02;
/// Loop size, as a share of the em, for a master without stems.
const DEFAULT_LOOP_SHARE: f64 = 0.04;
/// Align Points to Background's default tolerance, in units.
pub const DEFAULT_ALIGN_TOLERANCE: f64 = 50.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.makeInkTrap",
            "Make Ink Trap",
            ["Path", "Nodes"],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", points: [[shape, node]] (corner nodes), width?: number (default: the master's inkTrapWidth parameter; 2% of the em without it)} turns each corner node into two nodes `width` units apart, moved sideways from the corner across its bisector and joined by a straight segment, which cuts a small notch at an inside corner. The handles next to the two nodes move with them. Smooth nodes, handles, open path ends and nearly straight corners are left alone; at least one node must take a trap → {selection: {glyph, layer, points}} (the new nodes)",
            has_doc,
            make_ink_trap
        )
        .targets(&[TargetKind::Layer, TargetKind::Points]),
        cmd!(
            "path.loopCorner",
            "Loop Corner",
            ["Path", "Nodes"],
            None,
            "{glyph: name, layer?: id, side?, points: [[shape, node]] (corner nodes), size?: number (default: half the master's first vertical stem; 4% of the em without stems)} replaces each corner node with a loop: the incoming segment goes on `size` units past the corner along its direction, the outgoing segment starts `size` units before it, and the two cross. Smooth nodes, handles, open path ends and nearly straight corners are left alone; at least one node must take a loop → {selection: {glyph, layer, points}} (the new nodes)",
            has_doc,
            loop_corner
        )
        .targets(&[TargetKind::Layer, TargetKind::Points]),
        cmd!(
            "path.replaceWithCorner",
            "Replace with Corner",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, segments?: [[shape, node]] (or segment: [shape, node]), shapes?: [index] (open paths)} deletes each segment, or the gap between the ends of each open path in `shapes`, and extends the two neighbouring segments along their end directions to the point where they meet, which becomes one corner node (the open path becomes closed). One segment or gap per path. Refused when the neighbours are within 5° of parallel or meet behind the segment → {selection: {glyph, layer, points}} (the corner nodes)",
            has_doc,
            replace_with_corner
        )
        .targets(&[TargetKind::Layer, TargetKind::Segments, TargetKind::Shapes]),
        cmd!(
            "path.alignToMask",
            "Align Points to Background",
            ["Path", "Align Points"],
            None,
            "{glyph: name, layer?: id, points: [[shape, node]], tolerance?: number (default 50, from 0.01 to 10,000)} moves each foreground point onto the nearest node or handle of the layer's background paths when one lies within `tolerance` units. A moved on-curve node takes its handles along unless they are among the points. Points with nothing in reach stay → {moved: count, selection: {glyph, layer, points}}",
            has_doc,
            align_to_mask
        )
        .targets(&[TargetKind::Layer, TargetKind::Points]),
        cmd!(
            "path.collapsePoints",
            "Collapse Points",
            ["Path", "Align Points"],
            None,
            "{glyph: name, layer?: id, side?, points: [[shape, node]]} moves the on-curve nodes among the points (at least two) to the centre of their bounding box, rounded to the grid, and retracts their handles to zero length → {x, y, selection: {glyph, layer, points}}",
            has_doc,
            collapse_points
        )
        .targets(&[TargetKind::Layer, TargetKind::Points]),
        cmd!(
            "path.scaleToSameSize",
            "Same Width / Same Height",
            ["Path", "Align Points"],
            None,
            "{glyph: name, layer?: id, side?, shapes: [index] (paths), dimension: \"width\" | \"height\", proportional?: bool (default true)} scales each path about the centre of its bounding box so that its width (or height) equals the largest among the paths; proportional scales both directions, otherwise only that one. With one path the target is the advance width, or for height the x-height (the cap height for an uppercase glyph). Factors beyond 1,000 are refused → {selection: {glyph, layer, shapes}}",
            has_doc,
            scale_to_same_size
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes]),
    ]
}

// ── Paths and contours ──────────────────────────────────────────────────────────────────────────

fn po_kind(k: NodeKind) -> po::NodeKind {
    match k {
        NodeKind::Line => po::NodeKind::Line,
        NodeKind::Curve => po::NodeKind::Curve,
        NodeKind::QCurve => po::NodeKind::QCurve,
        NodeKind::OffCurve => po::NodeKind::OffCurve,
    }
}

fn model_kind(k: po::NodeKind) -> NodeKind {
    match k {
        po::NodeKind::Line => NodeKind::Line,
        po::NodeKind::Curve => NodeKind::Curve,
        po::NodeKind::QCurve => NodeKind::QCurve,
        po::NodeKind::OffCurve => NodeKind::OffCurve,
    }
}

/// The path as a contour, each node tagged with its index.
fn to_contour(path: &Path) -> po::Contour {
    let nodes = path.nodes.iter().map(|n| po::Node { point: n.point(), kind: po_kind(n.kind), smooth: n.smooth, tag: None }).collect();
    po::Contour::new(path.closed, nodes).tagged()
}

/// A contour back as a path: a node with a tag keeps the data of the node it was (its
/// identifier only once), new nodes are plain, and the path keeps its own data.
fn to_path(c: &po::Contour, old: &Path) -> Path {
    let mut used = HashSet::new();
    let nodes = c
        .nodes
        .iter()
        .map(|n| {
            let kind = model_kind(n.kind);
            let mut m = match n.tag.filter(|t| used.insert(*t)).and_then(|t| old.nodes.get(t)) {
                Some(src) => src.clone(),
                None => Node::new(n.point.x, n.point.y, kind),
            };
            m.x = n.point.x;
            m.y = n.point.y;
            m.kind = kind;
            m.smooth = n.smooth && kind != NodeKind::OffCurve;
            m.tangent &= m.smooth;
            m
        })
        .collect();
    Path { closed: c.closed, nodes, info: old.info.clone(), extra: old.extra.clone() }
}

fn op_err(p: &Params, e: po::PathOpError) -> crate::EngineError {
    p.bad(e.to_string())
}

/// Refuses target lists other than `allowed`.
fn takes_only(p: &Params, t: &Targets, allowed: &[&str]) -> Result<()> {
    let given = [
        ("points", !t.points.is_empty()),
        ("segments", !t.segments.is_empty()),
        ("shapes", !t.shapes.is_empty()),
        ("anchors", !t.anchors.is_empty()),
        ("guides", !t.guides.is_empty()),
        ("annotations", !t.annotations.is_empty()),
    ];
    match given.iter().find(|(k, on)| *on && !allowed.contains(k)) {
        Some((k, _)) => Err(p.bad(format!("`{k}` is not taken here: give {}", allowed.join(" or ")))),
        None => Ok(()),
    }
}

fn selection(p: &Params, t: &Targets, points: Vec<(usize, usize)>) -> Value {
    let mut sel = Targets::new(p.command(), t.glyph.clone(), t.layer.clone());
    sel.side = t.side;
    sel.points = points;
    sel.to_json()
}

/// A positive size parameter within `max`, or `None` when absent.
fn opt_size(p: &Params, key: &str, min: f64, max: f64) -> Result<Option<f64>> {
    match p.opt_f64(key)? {
        None => Ok(None),
        Some(v) if v.is_finite() && v >= min && v <= max && v > 0.0 => Ok(Some(v)),
        Some(v) => Err(p.bad(format!("`{key}` must be above 0 and at most {max}, got {v}"))),
    }
}

// ── Ink traps and loops ─────────────────────────────────────────────────────────────────────────

/// The master's `inkTrapWidth`, when it is a usable number.
fn master_ink_trap_width(master: &Master) -> Option<f64> {
    use fontcraft_model::plist::Value as Plist;
    let v = match master.params.get(INK_TRAP_WIDTH_PARAM)? {
        Plist::Number(n) => n.value().as_f64(),
        Plist::String(s) => s.as_str().trim().parse().ok()?,
        _ => return None,
    };
    (v.is_finite() && v > 0.0 && v <= ch::MAX_INK_TRAP_WIDTH).then_some(v)
}

fn em_share(font: &Font, share: f64) -> f64 {
    f64::from(font.units_per_em) * share
}

/// Runs `f` on the on-curve nodes among `points`, path by path, and returns the nodes it
/// selects. Fails when no path took the operation.
fn per_path(
    s: &mut Session,
    p: &Params,
    t: &Targets,
    label: &str,
    nothing: &str,
    f: impl Fn(&po::Contour, &[usize]) -> std::result::Result<ch::Changed, po::PathOpError>,
) -> Result<Vec<(usize, usize)>> {
    s.edit(label, |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::new();
            for (shape, set) in group(&t.points) {
                let Some(path) = parts.path_mut(shape) else { continue };
                let nodes: Vec<usize> = set.into_iter().filter(|&i| path.nodes.get(i).is_some_and(Node::on_curve)).collect();
                if nodes.is_empty() {
                    continue;
                }
                let changed = f(&to_contour(path), &nodes).map_err(|e| op_err(p, e))?;
                if changed.nodes.is_empty() {
                    continue;
                }
                *path = to_path(&changed.contour, path);
                out.extend(changed.nodes.iter().map(|&n| (shape, n)));
            }
            if out.is_empty() {
                return Err(p.bad(nothing));
            }
            Ok(out)
        })
    })
}

fn make_ink_trap(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.makeInkTrap", v)?;
    let t = Targets::parse(&p)?;
    takes_only(&p, &t, &["points"])?;
    if t.points.is_empty() {
        return Err(p.bad("give `points`: the corner nodes to trap"));
    }
    let font = s.font()?;
    let step = grid(font);
    let width = match opt_size(&p, "width", 0.0, ch::MAX_INK_TRAP_WIDTH)? {
        Some(w) => w,
        None => {
            let r = t.resolve(font)?;
            master_ink_trap_width(r.master).unwrap_or_else(|| em_share(font, DEFAULT_INK_TRAP_SHARE))
        }
    };
    let picked = per_path(s, &p, &t, "Make Ink Trap", "none of `points` is a corner node that can take an ink trap", |c, nodes| {
        ch::make_ink_traps(c, nodes, width, step)
    })?;
    Ok(json!({ "selection": selection(&p, &t, picked) }))
}

/// Half the master's first vertical stem (or its first horizontal one), else a share of the em.
fn default_loop_size(font: &Font, master: &Master) -> f64 {
    let half = |horizontal: bool| {
        font.stem_widths(master, horizontal).first().map(|v| v / 2.0).filter(|v| v.is_finite() && *v > 0.0 && *v <= ch::MAX_LOOP_SIZE)
    };
    half(false).or_else(|| half(true)).unwrap_or_else(|| em_share(font, DEFAULT_LOOP_SHARE))
}

fn loop_corner(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.loopCorner", v)?;
    let t = Targets::parse(&p)?;
    takes_only(&p, &t, &["points"])?;
    if t.points.is_empty() {
        return Err(p.bad("give `points`: the corner nodes to loop"));
    }
    let font = s.font()?;
    let step = grid(font);
    let size = match opt_size(&p, "size", 0.0, ch::MAX_LOOP_SIZE)? {
        Some(v) => v,
        None => default_loop_size(font, t.resolve(font)?.master),
    };
    let picked = per_path(s, &p, &t, "Loop Corner", "none of `points` is a corner node that can take a loop", |c, nodes| {
        ch::loop_corners(c, nodes, size, step)
    })?;
    Ok(json!({ "selection": selection(&p, &t, picked) }))
}

// ── Replace with Corner ─────────────────────────────────────────────────────────────────────────

fn replace_with_corner(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.replaceWithCorner", v)?;
    let t = Targets::parse(&p)?;
    takes_only(&p, &t, &["segments", "shapes"])?;
    if t.segments.is_empty() && t.shapes.is_empty() {
        return Err(p.bad("give `segments`, or open paths in `shapes`: what to replace with a corner"));
    }
    // One segment or gap per path: each replacement renumbers the path's nodes.
    let mut jobs: BTreeMap<usize, Option<usize>> = BTreeMap::new();
    for &(shape, node) in &t.segments {
        if jobs.insert(shape, Some(node)).is_some() {
            return Err(p.bad(format!("path {shape} is named twice: give one segment per path")));
        }
    }
    for &shape in &t.shapes {
        if jobs.insert(shape, None).is_some() {
            return Err(p.bad(format!("path {shape} is named twice: give one segment per path")));
        }
    }
    let step = grid(s.font()?);
    let picked = s.edit("Replace with Corner", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::new();
            for (&shape, &segment) in &jobs {
                let path = parts.path_mut(shape).ok_or_else(|| p.bad(format!("shape {shape} is not a path")))?;
                if segment.is_none() && path.closed {
                    return Err(p.bad(format!("path {shape} is closed: it has no gap between its ends")));
                }
                let changed = ch::replace_with_corner(&to_contour(path), segment, step).map_err(|e| p.bad(format!("path {shape}: {e}")))?;
                *path = to_path(&changed.contour, path);
                out.extend(changed.nodes.iter().map(|&n| (shape, n)));
            }
            Ok(out)
        })
    })?;
    Ok(json!({ "selection": selection(&p, &t, picked) }))
}

// ── Align Points to Background ──────────────────────────────────────────────────────────────────

/// The off-curve nodes next to on-curve node `i`: the handles of its two segments.
pub(super) fn handles_of(path: &Path, i: usize) -> Vec<usize> {
    let n = path.nodes.len();
    let prev = if i > 0 {
        Some(i - 1)
    } else if path.closed && n > 1 {
        Some(n - 1)
    } else {
        None
    };
    let next = if i + 1 < n {
        Some(i + 1)
    } else if path.closed && n > 1 {
        Some(0)
    } else {
        None
    };
    [prev, next].into_iter().flatten().filter(|&k| k != i && path.nodes.get(k).is_some_and(|node| node.kind == NodeKind::OffCurve)).collect()
}

fn align_to_mask(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.alignToMask", v)?;
    let t = Targets::parse(&p)?;
    takes_only(&p, &t, &["points"])?;
    if t.side != LayerSide::Foreground {
        return Err(p.bad("points align to the background: give foreground points"));
    }
    if t.points.is_empty() {
        return Err(p.bad("give `points`: the points to align"));
    }
    let tolerance = opt_size(&p, "tolerance", ch::MIN_ALIGN_TOLERANCE, ch::MAX_ALIGN_TOLERANCE)?.unwrap_or(DEFAULT_ALIGN_TOLERANCE);
    let font = s.font()?;
    let (targets, wanted) = {
        let r = t.resolve(font)?;
        let back = r.layer.parts(LayerSide::Background);
        let targets: Vec<Point> = (0..back.shapes.len()).filter_map(|i| back.path(i)).flat_map(|path| path.nodes.iter().map(Node::point)).collect();
        if targets.is_empty() {
            return Err(p.bad("the layer's background has no paths to align to"));
        }
        let wanted: Vec<Point> = t.points.iter().filter_map(|&(s, n)| r.parts.path(s).and_then(|path| path.nodes.get(n)).map(Node::point)).collect();
        (targets, wanted)
    };
    let found = po::corner_helpers::nearest_within(&wanted, &targets, tolerance).map_err(|e| op_err(&p, e))?;
    let moves: Vec<((usize, usize), Point)> = t.points.iter().copied().zip(found).filter_map(|(a, f)| f.map(|f| (a, f))).collect();
    let selected: HashSet<(usize, usize)> = t.points.iter().copied().collect();
    let moved = moves.len();
    s.edit("Align Points to Background", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(LayerSide::Foreground);
            for &((shape, node), to) in &moves {
                let Some(path) = parts.path_mut(shape) else { continue };
                let Some(from) = path.nodes.get(node).map(Node::point) else { continue };
                let d = to - from;
                let followers = if path.nodes.get(node).is_some_and(Node::on_curve) { handles_of(path, node) } else { Vec::new() };
                for k in followers.into_iter().filter(|&k| !selected.contains(&(shape, k))) {
                    if let Some(h) = path.nodes.get_mut(k) {
                        let (x, y) = (h.x + d.x, h.y + d.y);
                        if !(in_range(x) && in_range(y)) {
                            return Err(p.bad(format!("a handle would move beyond ±{MAX_COORDINATE}")));
                        }
                        h.x = x;
                        h.y = y;
                    }
                }
                if let Some(n) = path.nodes.get_mut(node) {
                    n.x = to.x;
                    n.y = to.y;
                }
            }
            Ok(())
        })
    })?;
    Ok(json!({ "moved": moved, "selection": selection(&p, &t, t.points.clone()) }))
}

// ── Collapse Points ─────────────────────────────────────────────────────────────────────────────

fn collapse_points(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.collapsePoints", v)?;
    let t = Targets::parse(&p)?;
    takes_only(&p, &t, &["points"])?;
    let step = grid(s.font()?);
    let (centre, picked) = s.edit("Collapse Points", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut shapes = Vec::new();
            let mut contours = Vec::new();
            let mut addresses = Vec::new();
            for (shape, set) in group(&t.points) {
                let Some(path) = parts.path_mut(shape) else { continue };
                let k = contours.len();
                addresses.extend(set.iter().filter(|&&n| path.nodes.get(n).is_some_and(Node::on_curve)).map(|&n| (k, n)));
                contours.push(to_contour(path));
                shapes.push(shape);
            }
            if addresses.len() < 2 {
                return Err(p.bad("give at least two on-curve nodes in `points` to collapse"));
            }
            let centre = ch::collapse_points(&mut contours, &addresses, step).map_err(|e| op_err(&p, e))?;
            for (shape, c) in shapes.iter().zip(&contours) {
                if let Some(path) = parts.path_mut(*shape) {
                    *path = to_path(c, path);
                }
            }
            let picked = addresses.iter().filter_map(|&(k, n)| shapes.get(k).map(|&s| (s, n))).collect::<Vec<_>>();
            Ok((centre, picked))
        })
    })?;
    Ok(json!({ "x": centre.x, "y": centre.y, "selection": selection(&p, &t, picked) }))
}

// ── Same Width / Same Height ────────────────────────────────────────────────────────────────────

/// Whether the glyph counts as uppercase: its case says so, or one of its code points is an
/// uppercase letter.
fn is_uppercase(glyph: &Glyph) -> bool {
    glyph.info.case.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("upper") || c.eq_ignore_ascii_case("uppercase"))
        || glyph.unicodes.iter().filter_map(|&u| char::from_u32(u)).any(char::is_uppercase)
}

fn scale_to_same_size(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.scaleToSameSize", v)?;
    let t = Targets::parse(&p)?;
    takes_only(&p, &t, &["shapes"])?;
    let dimension = match p.str("dimension")? {
        "width" => ch::Dimension::Width,
        "height" => ch::Dimension::Height,
        other => return Err(p.bad(format!("`dimension` must be width or height, got \"{other}\""))),
    };
    let proportional = p.bool_or("proportional", true)?;
    if t.shapes.is_empty() {
        return Err(p.bad("give `shapes`: the paths to scale"));
    }
    let font = s.font()?;
    let step = grid(font);
    let target = {
        let r = t.resolve(font)?;
        let mut extents = Vec::with_capacity(t.shapes.len());
        for &shape in &t.shapes {
            let path = r.parts.path(shape).ok_or_else(|| p.bad(format!("shape {shape} is not a path")))?;
            let bounds = ch::contour_bounds(&to_contour(path)).map_err(|e| op_err(&p, e))?;
            extents.push(bounds.map_or(0.0, |b| ch::extent(b, dimension)));
        }
        match (extents.as_slice(), dimension) {
            ([_], ch::Dimension::Width) => r.layer.width,
            ([_], ch::Dimension::Height) => {
                let m = master_metrics(font, r.master);
                if is_uppercase(r.glyph) { m.cap_height } else { m.x_height }
            }
            (all, _) => all.iter().copied().fold(0.0, f64::max),
        }
    };
    if !(target.is_finite() && target > 0.0) {
        return Err(p.bad(format!("the target size ({target} units) is not above 0")));
    }
    s.edit("Same Size", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            for &shape in &t.shapes {
                let Some(path) = parts.path_mut(shape) else { continue };
                let scaled =
                    ch::scale_to_size(&to_contour(path), dimension, target, proportional, step).map_err(|e| p.bad(format!("path {shape}: {e}")))?;
                *path = to_path(&scaled, path);
            }
            Ok(())
        })
    })?;
    let mut sel = Targets::new("path.scaleToSameSize", t.glyph.clone(), t.layer.clone());
    sel.side = t.side;
    sel.shapes = t.shapes.clone();
    Ok(json!({ "selection": sel.to_json() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_round_trips_through_a_contour_with_its_node_data() {
        let mut path =
            Path::new(true, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(10.0, 0.0, NodeKind::Line), Node::new(10.0, 10.0, NodeKind::Line)]);
        if let Some(n) = path.nodes.get_mut(1) {
            n.locked = true;
        }
        assert_eq!(to_path(&to_contour(&path), &path), path);
    }
}
