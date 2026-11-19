//! Node-reference upkeep: after a command changes the node structure of a layer with hints, the
//! references of that layer are remapped in the same undo step.
//!
//! Contract (`docs/plans/M10.md` § 3 › Node references). Each layer is remapped against the same
//! layer before the edit, so every master keeps its own links (`hinting.ps.masters.transferByNodeLink`
//! resolves them per master at export). A referenced node is found again by these steps, first
//! match wins:
//!
//! | Step | Finds the node when |
//! |---|---|
//! | 1 | the node at the same (path, node) index has the same place and class (on or off curve) |
//! | 2 | its path was permuted: every path of the layer reappears with the same node places and classes (paths reordered, reversed or given a new start node), or its own path at the same index does: the node of its class at its place |
//! | 3 | its path at the same index has the same number of nodes, of the same classes, in the same order: the node moved, and keeps its index |
//! | 4 | its path reappears permuted at another index (a path before it was removed or added): the node of its class at its place |
//! | 5 | otherwise (nodes inserted, deleted or merged): a node of the same class lies at its place, or within [`NEAR`] of it (grid rounding); among several, the one in the same path, then with the same neighbours, then the closest index |
//!
//! Places decide before the index only for whole-path permutations, so a path moved by exactly
//! the gap between two of its nodes, or onto the places of an identical path that moved too,
//! keeps its links on the nodes that moved rather than on the nodes that landed on their old
//! places.
//!
//! A node not found is gone. A PostScript end on a gone node becomes a number, the node's last y
//! (horizontal hints) or x (vertical hints). A TrueType instruction on a gone node is removed and
//! reported; with [`RemapMode::OverlapRemoval`] it moves to the nearest on-curve node the removal
//! created, and so does an instruction on an intersection extra node (the handbook's rule for
//! Remove Overlaps at export, `hinting.tt.export.overlapRemovalMovesHints`).
//!
//! Corner components (`Corner` hints) follow their nodes by the same steps. Their `origin` counts
//! the layer's shapes, components included, and a closed path's nodes in file order, so it is
//! stored anew when a shape before the path is added or removed. A corner on a gone node is
//! removed and reported, as a TrueType instruction is, and moves with them under
//! [`RemapMode::OverlapRemoval`] (a corner on an extra node lands on the node the removal made
//! there). Caps, segments and brushes are left alone.
//!
//! A layer's background is remapped against the background before the edit, apart from the
//! layer's own hints.
//!
//! Left alone: layers and backgrounds without hints, those whose stored hints the command changed
//! (a command that edits hints keeps its own references right), those the command lists in
//! [`Renumbered`] (it renumbered them itself, with [`renumber_side`]), references that dangled
//! before the edit, and hints that do not parse. A layer that needs no change keeps its value; a font
//! that needs none comes back as given.
//!
//! [`renumber_side`] is for a command that knows which paths it replaced (Expand Outline,
//! Decompose): the old paths' new indices are given, so no node is matched by place. The other
//! paths keep their nodes; references into a replaced path are gone, with the outcomes above.
//!
//! The engine runs [`remap_after_edit`] in `Session::realign_after`, after the component
//! re-alignment, so both land in the command's undo step. Rows: `hinting.ps.manual.linkOnDrop`
//! (links survive edits), `hinting.ps.masters.transferByNodeLink`,
//! `hinting.tt.export.overlapRemovalMovesHints` (the editor half).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::corner::{CornerHint, CornerTarget, corner_hint, is_corner};
use fontcraft_model::plist::Value;
use fontcraft_model::{Background, Font, Hint, Layer, LayerSide, Node, NodeKind, Path, Shape};

use crate::noderef::{Syntax, put};
use crate::{
    Edge, HintingError, NodeOrder, NodeRef, PsHint, TtInstruction, TtKind, TtPoint, ps_hints, set_ps_hints, set_tt_instructions, tt_instructions,
};

/// How far (font units, each axis) steps 2, 4 and 5 look for a node that an edit rounded to the grid.
pub const NEAR: f64 = 0.5;
/// Places closer than this are the same place.
const SAME: f64 = 1e-6;

/// What remapping did to one font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemapReport {
    /// Messages for removed TrueType instructions and corner components, one each: glyph, layer,
    /// what.
    pub removed: Vec<String>,
    /// PostScript edges turned into numbers.
    pub unlinked: usize,
    /// TrueType points and corner components moved to a node the overlap removal created.
    pub moved: usize,
}

impl RemapReport {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.unlinked == 0 && self.moved == 0
    }
}

/// The layer sides whose hint references a command renumbered itself, with what that did;
/// [`remap_after_edit`] leaves them alone.
#[derive(Clone, Debug, Default)]
pub struct Renumbered {
    /// (glyph, layer id, background).
    sides: HashSet<(String, String, bool)>,
    /// What renumbering removed or unlinked, reported with the upkeep's own.
    pub report: RemapReport,
}

impl Renumbered {
    pub fn insert(&mut self, glyph: &str, layer: &str, side: LayerSide) {
        self.sides.insert((glyph.to_string(), layer.to_string(), side == LayerSide::Background));
    }

    pub fn contains(&self, glyph: &str, layer: &str, side: LayerSide) -> bool {
        // Most commands list nothing: skip building the key.
        !self.sides.is_empty() && self.sides.contains(&(glyph.to_string(), layer.to_string(), side == LayerSide::Background))
    }

    pub fn is_empty(&self) -> bool {
        self.sides.is_empty() && self.report.is_empty()
    }

    /// Adds `other`'s sides and report to these.
    pub fn extend(&mut self, other: Renumbered) {
        self.sides.extend(other.sides);
        self.report.removed.extend(other.report.removed);
        self.report.unlinked += other.report.unlinked;
        self.report.moved += other.report.moved;
    }
}

/// How references to nodes the edit removed are treated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RemapMode {
    /// A removed node is gone (see the module doc).
    #[default]
    Edit,
    /// Remove Overlap: TrueType points that are gone, and intersection points, move to the
    /// nearest on-curve node the removal created.
    OverlapRemoval,
}

/// `new` with the hint references of every changed layer remapped against `old`, apart from the
/// sides in `skip`. Returns `new` unchanged (the same `Arc`s) when no layer with hints changed its
/// node structure.
pub fn remap_after_edit(old: &Font, mut new: Font, mode: RemapMode, skip: &Renumbered) -> Result<(Font, RemapReport), HintingError> {
    let mut report = RemapReport::default();
    let (now, before) = (new.glyphs.as_arcs(), old.glyphs.as_arcs());
    if std::ptr::eq(now, before) {
        return Ok((new, report));
    }
    let mut edits: Vec<(usize, Vec<(usize, Layer)>)> = Vec::new();
    for (i, g) in now.iter().enumerate() {
        let prev = match before.get(i) {
            Some(b) if Arc::ptr_eq(b, g) => continue,
            Some(b) if b.name == g.name => b,
            _ => match old.glyph(&g.name) {
                Some(b) if Arc::ptr_eq(b, g) => continue,
                Some(b) => b,
                None => continue,
            },
        };
        let mut layers = Vec::new();
        for (li, layer) in g.layers.iter().enumerate() {
            let own = !layer.info.hints.is_empty() && !skip.contains(&g.name, &layer.id, LayerSide::Foreground);
            let back =
                layer.info.background.as_ref().is_some_and(|b| !b.hints.is_empty()) && !skip.contains(&g.name, &layer.id, LayerSide::Background);
            if !own && !back {
                continue;
            }
            let Some(was) = prev.layers.iter().find(|l| l.id == layer.id) else { continue };
            let front = if own { remap_layer(was, layer, &g.name, mode, &mut report) } else { None };
            let back = if back { remap_background(was, front.as_ref().unwrap_or(layer), &g.name, mode, &mut report) } else { None };
            if let Some(out) = back.or(front) {
                layers.push((li, out));
            }
        }
        if !layers.is_empty() {
            edits.push((i, layers));
        }
    }
    for (i, layers) in edits {
        if let Some(mut g) = new.glyphs.get_mut_at(i) {
            for (li, layer) in layers {
                if let Some(slot) = g.layers.get_mut(li) {
                    *slot = layer;
                }
            }
        }
    }
    Ok((new, report))
}

/// The hints of `new` remapped against `old` (the same layer of glyph `glyph` before the edit);
/// `None` when nothing needs to change or the layer is left alone (module doc).
pub fn remap_layer(old: &Layer, new: &Layer, glyph: &str, mode: RemapMode, report: &mut RemapReport) -> Option<Layer> {
    remap_hints(old, new, glyph, &format!("layer `{}`", new.id), mode, None, report)
}

/// The hints of `side` of `new` renumbered for an edit that replaced or renumbered whole paths of
/// that side and left the other paths' nodes as they were: `paths` holds the new index of each old
/// path (paths only, in shape order), `None` for a replaced one. `old` is the layer before the edit
/// and `new` must still hold the side's hints as they were. `None` when nothing needs to change.
pub fn renumber_side(old: &Layer, new: &Layer, side: LayerSide, glyph: &str, paths: &[Option<usize>], report: &mut RemapReport) -> Option<Layer> {
    match side {
        LayerSide::Foreground => remap_hints(old, new, glyph, &format!("layer `{}`", new.id), RemapMode::Edit, Some(paths), report),
        LayerSide::Background => background(old, new, glyph, RemapMode::Edit, Some(paths), report),
    }
}

/// Whether the node references of `hint` are kept by this module when the hint sits on a layer
/// side whose shapes are `shapes`: PostScript hints, TrueType instructions and corner components on
/// a path. Other hints (caps, segments, brushes, corners on anything else) are left to the command.
pub fn follows_nodes(hint: &Hint, shapes: &[Shape]) -> bool {
    if crate::ps::is_ps(hint) || crate::tt::is_tt(hint) {
        return true;
    }
    match corner_hint(hint) {
        Ok(Some(c)) => {
            let shape = match c.target {
                CornerTarget::Node { shape, .. } | CornerTarget::Intersection { shape, .. } => shape,
            };
            matches!(shapes.get(shape), Some(Shape::Path(_)))
        }
        _ => false,
    }
}

/// `new` with the hints of its background remapped against the background of `old` (the same
/// layer before the edit), as [`remap_layer`] remaps the layer's own; `None` when nothing needs
/// to change or the background is left alone (module doc).
pub fn remap_background(old: &Layer, new: &Layer, glyph: &str, mode: RemapMode, report: &mut RemapReport) -> Option<Layer> {
    background(old, new, glyph, mode, None, report)
}

/// [`remap_background`], with the old paths' new indices when the command knows them.
fn background(old: &Layer, new: &Layer, glyph: &str, mode: RemapMode, known: Option<&[Option<usize>]>, report: &mut RemapReport) -> Option<Layer> {
    let (was, now) = (old.info.background.as_ref()?, new.info.background.as_ref()?);
    if now.hints.is_empty() || was.hints != now.hints || was.shapes == now.shapes {
        return None;
    }
    // The hint readers work on a layer: a stand-in holding the background's shapes and hints.
    let stand_in = |b: &Background| {
        let mut l = Layer::new(new.master.clone(), new.width);
        l.id.clone_from(&new.id);
        l.shapes = b.shapes.clone();
        l.info.hints = b.hints.clone();
        l
    };
    let mut fixed = remap_hints(&stand_in(was), &stand_in(now), glyph, &format!("background of layer `{}`", new.id), mode, known, report)?;
    let mut out = new.clone();
    out.info.background.as_mut()?.hints = std::mem::take(&mut fixed.info.hints);
    Some(out)
}

/// [`remap_layer`] for a layer or a background's stand-in; `place` names it in messages. `known`:
/// the new index of each old path, when the command gives it ([`renumber_side`]).
fn remap_hints(
    old: &Layer,
    new: &Layer,
    glyph: &str,
    place: &str,
    mode: RemapMode,
    known: Option<&[Option<usize>]>,
    report: &mut RemapReport,
) -> Option<Layer> {
    if old.info.hints != new.info.hints {
        return None;
    }
    let (old_paths, new_paths) = (paths(old), paths(new));
    let (old_slots, new_slots) = (path_slots(old), path_slots(new));
    // A corner's origin counts shapes, so it changes when only the shapes around a path do.
    let slots_moved = old_slots != new_slots && old.info.hints.iter().any(is_corner);
    if !slots_moved && old_paths.len() == new_paths.len() && old_paths.iter().zip(&new_paths).all(|(a, b)| same_nodes(a, b)) {
        return None;
    }
    let ps = ps_hints(old).ok()?;
    let tt = tt_instructions(old).ok()?;
    // The stored values count in file order: a reference to the start node of a closed path whose
    // node count changed is stored differently although its model index stayed.
    let (old_order, new_order) = (NodeOrder::of(old), NodeOrder::of(new));
    let corners = corners(old, &old_slots, &old_order);
    let mut refs = BTreeSet::new();
    for c in &corners {
        refs.extend(c.point.node());
        if let TtPoint::Intersection { a, b } = c.point {
            refs.extend([a, b]);
        }
    }
    for (_, h) in &ps {
        refs.extend([Some(h.origin), h.other].into_iter().flatten().filter_map(|e| match e {
            Edge::Node(r) => Some(r),
            Edge::Value(_) => None,
        }));
    }
    for (_, t) in &tt {
        for p in t.points() {
            match p {
                TtPoint::Node(r) => {
                    refs.insert(r);
                }
                TtPoint::Intersection { a, b } => {
                    refs.extend([a, b]);
                }
                TtPoint::Lsb | TtPoint::Rsb => {}
            }
        }
    }
    if refs.is_empty() {
        return None;
    }
    let mut m = Matcher { old: old_paths, new: new_paths, known, grid: None, created: None, keys: None, path_match: HashMap::new() };
    // `None`: the node is gone. References that dangled before the edit are not in the map.
    let map: BTreeMap<NodeRef, Option<NodeRef>> = refs.iter().filter_map(|r| m.locate(*r).map(|found| (*r, found))).collect();
    let overlap = mode == RemapMode::OverlapRemoval;
    let has_intersections = tt.iter().any(|(_, t)| t.points().iter().any(|p| matches!(p, TtPoint::Intersection { .. })))
        || corners.iter().any(|c| matches!(c.point, TtPoint::Intersection { .. }));
    let reordered = refs.iter().any(|r| old_order.closed_len(r.path) != new_order.closed_len(r.path));
    if map.iter().all(|(k, v)| *v == Some(*k)) && !(overlap && has_intersections) && !reordered && !slots_moved {
        return None;
    }

    let mut local = RemapReport::default();
    // Per corner hint (by its index in the stored hints): its new value, `None` to remove it.
    let mut corner_edits: BTreeMap<usize, Option<Value>> = BTreeMap::new();
    for c in &corners {
        let Some((after, reattached)) = m.point_after(c.point, &map, overlap) else {
            corner_edits.insert(c.index, None);
            local.removed.push(format!(
                "glyph `{glyph}`, {place}: the corner component `{}` on {} was removed: its node is gone",
                c.hint.name,
                point_text(&c.point)
            ));
            continue;
        };
        let stored = |r: NodeRef| {
            let f = new_order.to_file(r);
            new_slots.get(f.path).map(|shape| (*shape, f.node))
        };
        let target =
            match after {
                TtPoint::Node(r) => stored(r).map(|(shape, node)| CornerTarget::Node { shape, node }),
                TtPoint::Intersection { a, b } => stored(a)
                    .zip(stored(b))
                    .map(|((shape, node), (other_shape, other_node))| CornerTarget::Intersection { shape, node, other_shape, other_node }),
                TtPoint::Lsb | TtPoint::Rsb => None,
            };
        let Some(target) = target.filter(|t| *t != c.hint.target) else { continue };
        let Some(mut value) = old.info.hints.get(c.index).map(|h| h.value.clone()) else { continue };
        let Value::Dict(d) = &mut value else { continue };
        let items = match target {
            CornerTarget::Node { shape, node } => vec![shape, node],
            CornerTarget::Intersection { shape, node, other_shape, other_node } => vec![shape, node, other_shape, other_node],
        };
        let origin = Syntax::of(d.get("origin")).indexes(&items);
        put(d, "origin", origin);
        local.moved += usize::from(reattached);
        corner_edits.insert(c.index, Some(value));
    }
    let mut ps_changed = false;
    let ps_new: Vec<PsHint> = ps
        .into_iter()
        .map(|(_, mut h)| {
            let horizontal = h.horizontal;
            for edge in std::iter::once(&mut h.origin).chain(h.other.as_mut()) {
                let Edge::Node(r) = *edge else { continue };
                match map.get(&r) {
                    Some(Some(n)) if *n != r => {
                        *edge = Edge::Node(*n);
                        ps_changed = true;
                    }
                    Some(None) => {
                        if let Some(node) = r.resolve(old) {
                            *edge = Edge::Value(if horizontal { node.y } else { node.x });
                            local.unlinked += 1;
                            ps_changed = true;
                        }
                    }
                    _ => {}
                }
            }
            h
        })
        .collect();

    let mut tt_changed = false;
    let mut broken_triples = BTreeSet::new();
    let mut kept: Vec<TtInstruction> = Vec::with_capacity(tt.len());
    for (_, mut t) in tt {
        let mut gone = Vec::new();
        for p in std::iter::once(&mut t.target).chain(t.origin.as_mut()).chain(t.other.as_mut()) {
            let before = *p;
            match m.point_after(before, &map, overlap) {
                Some((after, reattached)) => {
                    if after != before {
                        local.moved += usize::from(reattached);
                        *p = after;
                        tt_changed = true;
                    }
                }
                None => gone.push(before),
            }
        }
        if gone.is_empty() {
            kept.push(t);
            continue;
        }
        tt_changed = true;
        if let TtKind::Stem { triple: Some(group), .. } = t.kind {
            broken_triples.insert(group);
        }
        let points = gone.iter().map(point_text).collect::<Vec<_>>().join(", ");
        local.removed.push(format!("glyph `{glyph}`, {place}: the {} instruction on {points} was removed: its node is gone", kind_name(&t.kind)));
    }
    // A triple hint that lost a member is no longer one: the others stay as plain Stem instructions.
    for t in &mut kept {
        if let TtKind::Stem { triple, .. } = &mut t.kind
            && triple.is_some_and(|g| broken_triples.contains(&g))
        {
            *triple = None;
        }
    }

    if !ps_changed && !tt_changed && !reordered && corner_edits.is_empty() {
        return None;
    }
    let mut out = new.clone();
    if !corner_edits.is_empty() {
        // Before the splices below, while the stored hints still line up with `old`'s.
        let hints = std::mem::take(&mut out.info.hints);
        out.info.hints = hints
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut h)| match corner_edits.remove(&i) {
                Some(None) => None,
                Some(Some(value)) => {
                    h.value = value;
                    Some(h)
                }
                None => Some(h),
            })
            .collect();
    }
    if ps_changed || reordered {
        set_ps_hints(&mut out, &ps_new).ok()?;
    }
    if tt_changed || reordered {
        set_tt_instructions(&mut out, &kept).ok()?;
    }
    if out.info.hints == new.info.hints {
        return None;
    }
    report.removed.extend(local.removed);
    report.unlinked += local.unlinked;
    report.moved += local.moved;
    Some(out)
}

/// Whether every node of `p` keeps its reference.
fn map_keeps(p: TtPoint, map: &BTreeMap<NodeRef, Option<NodeRef>>) -> bool {
    let keeps = |r: NodeRef| map.get(&r).is_none_or(|v| *v == Some(r));
    match p {
        TtPoint::Node(r) => keeps(r),
        TtPoint::Intersection { a, b } => keeps(a) && keeps(b),
        TtPoint::Lsb | TtPoint::Rsb => true,
    }
}

fn kind_name(kind: &TtKind) -> &'static str {
    match kind {
        TtKind::Snap { .. } => "Snap",
        TtKind::Stem { .. } => "Stem",
        TtKind::Shift => "Shift",
        TtKind::Interpolate => "Interpolate",
        TtKind::Delta { .. } => "Delta",
    }
}

fn point_text(p: &TtPoint) -> String {
    match p {
        TtPoint::Node(r) => format!("node ({}, {})", r.path, r.node),
        TtPoint::Intersection { a, b } => format!("the intersection ({}, {}, {}, {})", a.path, a.node, b.path, b.node),
        TtPoint::Lsb => "lsb".to_string(),
        TtPoint::Rsb => "rsb".to_string(),
    }
}

/// The shape index of each path of the layer, in shape order.
fn path_slots(layer: &Layer) -> Vec<usize> {
    layer.shapes.iter().enumerate().filter_map(|(i, s)| matches!(s, Shape::Path(_)).then_some(i)).collect()
}

/// A corner component the remap can follow: its index in the stored hints, the hint, and the
/// node or extra node it sits on in the model's order.
struct Corner {
    index: usize,
    hint: CornerHint,
    point: TtPoint,
}

/// The layer's corner components whose origin names a path; `slots` and `order` are the layer's.
/// Others are left alone.
fn corners(layer: &Layer, slots: &[usize], order: &NodeOrder) -> Vec<Corner> {
    let at = |shape: usize, node: usize| slots.binary_search(&shape).ok().map(|path| order.to_model(NodeRef::new(path, node)));
    layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter_map(|(index, h)| {
            let hint = corner_hint(h).ok()??;
            let point = match hint.target {
                CornerTarget::Node { shape, node } => TtPoint::Node(at(shape, node)?),
                CornerTarget::Intersection { shape, node, other_shape, other_node } => {
                    TtPoint::Intersection { a: at(shape, node)?, b: at(other_shape, other_node)? }
                }
            };
            Some(Corner { index, hint, point })
        })
        .collect()
}

/// The layer's paths in shape order, as node references count them.
fn paths(layer: &Layer) -> Vec<&Path> {
    layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn on_curve(n: &Node) -> bool {
    n.kind != NodeKind::OffCurve
}

fn same_place(a: &Node, b: &Node) -> bool {
    (a.x - b.x).abs() <= SAME && (a.y - b.y).abs() <= SAME
}

fn same_nodes(a: &Path, b: &Path) -> bool {
    std::ptr::eq(a, b) || (a.nodes.len() == b.nodes.len() && a.nodes.iter().zip(&b.nodes).all(|(m, n)| m.kind == n.kind && same_place(m, n)))
}

/// The grid cell of a place; coordinates are finite and bounded by the model's check, and the
/// casts saturate in any case.
fn cell(x: f64, y: f64) -> (i64, i64) {
    (x.floor() as i64, y.floor() as i64)
}

/// The nodes before and after `i` in `path` (wrapping on a closed path).
fn neighbours(path: &Path, i: usize) -> [Option<&Node>; 2] {
    let len = path.nodes.len();
    let prev = match i.checked_sub(1) {
        Some(p) => path.nodes.get(p),
        None if path.closed && len > 1 => path.nodes.last(),
        None => None,
    };
    let next = match path.nodes.get(i + 1) {
        Some(n) => Some(n),
        None if path.closed && len > 1 => path.nodes.first(),
        None => None,
    };
    [prev, next]
}

/// Where a node was, and its class.
#[derive(Clone, Copy)]
struct Place {
    x: f64,
    y: f64,
    on: bool,
}

impl Place {
    fn of(n: &Node) -> Self {
        Self { x: n.x, y: n.y, on: on_curve(n) }
    }
}

struct Matcher<'a> {
    old: Vec<&'a Path>,
    new: Vec<&'a Path>,
    /// The new index of each old path, when the command gives it: it replaces steps 1 to 5.
    known: Option<&'a [Option<usize>]>,
    /// The new layer's nodes by grid cell, built on first use.
    grid: Option<HashMap<(i64, i64), Vec<NodeRef>>>,
    /// On-curve nodes of the new layer at places no old node had, built on first use.
    created: Option<Vec<NodeRef>>,
    /// The paths' node places, built on first use.
    keys: Option<Keys>,
    /// How references into each old path are found (module doc, steps 2 to 5).
    path_match: HashMap<usize, PathMatch>,
}

/// A path's node places and classes, sorted, so that two permutations of the same nodes have the
/// same key. Places are counted in steps of [`SAME`].
type PathKey = Vec<(bool, i64, i64)>;

struct Keys {
    old: Vec<PathKey>,
    new: Vec<PathKey>,
    /// How many new paths have each key.
    new_count: HashMap<PathKey, usize>,
    /// Every old path reappears permuted, one to one.
    rearranged: bool,
}

impl Keys {
    fn of(old: &[&Path], new: &[&Path]) -> Self {
        let (old, new): (Vec<PathKey>, Vec<PathKey>) = (old.iter().map(|p| path_key(p)).collect(), new.iter().map(|p| path_key(p)).collect());
        let count = |keys: &[PathKey]| {
            let mut c: HashMap<PathKey, usize> = HashMap::new();
            for k in keys {
                *c.entry(k.clone()).or_default() += 1;
            }
            c
        };
        let new_count = count(&new);
        let rearranged = old.len() == new.len() && count(&old) == new_count;
        Self { old, new, new_count, rearranged }
    }
}

/// How references into one old path are found after step 1.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PathMatch {
    /// Steps 2 and 4: the path's nodes were permuted; find each by its place.
    Permuted,
    /// Step 3: same classes in the same order; the node keeps its index.
    KeepIndex,
    /// Step 5: the structure changed; find the node by its place or treat it as gone.
    Changed,
}

fn path_key(p: &Path) -> PathKey {
    // Coordinates are finite and bounded by the model's check; the casts saturate in any case.
    let step = |v: f64| (v / SAME).round() as i64;
    let mut k: PathKey = p.nodes.iter().map(|n| (on_curve(n), step(n.x), step(n.y))).collect();
    k.sort_unstable();
    k
}

impl Matcher<'_> {
    fn old_node(&self, r: NodeRef) -> Option<&Node> {
        self.old.get(r.path)?.nodes.get(r.node)
    }

    fn new_node(&self, r: NodeRef) -> Option<&Node> {
        self.new.get(r.path)?.nodes.get(r.node)
    }

    /// Where `r` is after the edit: `Some(Some(new))`, `Some(None)` when its node is gone, `None`
    /// when it dangled before.
    fn locate(&mut self, r: NodeRef) -> Option<Option<NodeRef>> {
        let o = self.old_node(r).map(Place::of)?;
        if let Some(paths) = self.known {
            return Some(paths.get(r.path).copied().flatten().map(|path| NodeRef::new(path, r.node)).filter(|n| self.new_node(*n).is_some()));
        }
        if self.new_node(r).is_some_and(|n| on_curve(n) == o.on && (n.x - o.x).abs() <= SAME && (n.y - o.y).abs() <= SAME) {
            return Some(Some(r));
        }
        match self.path_match(r.path) {
            PathMatch::KeepIndex => Some(Some(r)),
            PathMatch::Permuted | PathMatch::Changed => Some(self.by_place(r, o)),
        }
    }

    fn path_match(&mut self, path: usize) -> PathMatch {
        if let Some(m) = self.path_match.get(&path) {
            return *m;
        }
        let m = self.decide(path);
        self.path_match.insert(path, m);
        m
    }

    fn decide(&mut self, path: usize) -> PathMatch {
        let Some(old) = self.old.get(path).copied() else { return PathMatch::Changed };
        let (old_paths, new_paths) = (&self.old, &self.new);
        let keys = self.keys.get_or_insert_with(|| Keys::of(old_paths, new_paths));
        let Some(mine) = keys.old.get(path) else { return PathMatch::Changed };
        if keys.rearranged || keys.new.get(path) == Some(mine) {
            return PathMatch::Permuted;
        }
        let other_index = keys.new_count.contains_key(mine);
        match self.new.get(path) {
            Some(b) if old.nodes.len() == b.nodes.len() && old.nodes.iter().zip(&b.nodes).all(|(m, n)| on_curve(m) == on_curve(n)) => {
                PathMatch::KeepIndex
            }
            _ if other_index => PathMatch::Permuted,
            _ => PathMatch::Changed,
        }
    }

    fn grid(&mut self) -> &HashMap<(i64, i64), Vec<NodeRef>> {
        let new = &self.new;
        self.grid.get_or_insert_with(|| {
            let mut grid: HashMap<(i64, i64), Vec<NodeRef>> = HashMap::new();
            for (pi, p) in new.iter().enumerate() {
                for (ni, n) in p.nodes.iter().enumerate() {
                    grid.entry(cell(n.x, n.y)).or_default().push(NodeRef::new(pi, ni));
                }
            }
            grid
        })
    }

    /// Steps 2, 4 and 5 of the module doc.
    fn by_place(&mut self, r: NodeRef, o: Place) -> Option<NodeRef> {
        let (cx, cy) = cell(o.x, o.y);
        let mut near: Vec<NodeRef> = Vec::new();
        let grid = self.grid();
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                if let Some(list) = grid.get(&(cx.saturating_add(dx), cy.saturating_add(dy))) {
                    near.extend(list);
                }
            }
        }
        let mut best: Vec<(f64, NodeRef)> = near
            .into_iter()
            .filter_map(|c| {
                let n = self.new_node(c)?;
                let (dx, dy) = ((n.x - o.x).abs(), (n.y - o.y).abs());
                (on_curve(n) == o.on && dx <= NEAR && dy <= NEAR).then_some((dx.max(dy), c))
            })
            .collect();
        let closest = best.iter().map(|(d, _)| *d).fold(f64::INFINITY, f64::min);
        best.retain(|(d, _)| *d <= closest + SAME);
        if best.len() <= 1 {
            return best.first().map(|(_, c)| *c);
        }
        let old_side: Vec<&Node> = self.old.get(r.path).map(|p| neighbours(p, r.node).into_iter().flatten().collect()).unwrap_or_default();
        best.into_iter()
            .map(|(_, c)| {
                let matched = self
                    .new
                    .get(c.path)
                    .map(|p| {
                        let side: Vec<&Node> = neighbours(p, c.node).into_iter().flatten().collect();
                        old_side.iter().filter(|o| side.iter().any(|n| same_place(n, o))).count()
                    })
                    .unwrap_or(0);
                ((c.path != r.path, usize::MAX - matched, c.node.abs_diff(r.node)), c)
            })
            .min_by_key(|(k, _)| *k)
            .map(|(_, c)| c)
    }

    /// `p` after the edit, and whether it moved to a node the overlap removal created; `None`
    /// when it is removed.
    fn point_after(&mut self, p: TtPoint, map: &BTreeMap<NodeRef, Option<NodeRef>>, overlap: bool) -> Option<(TtPoint, bool)> {
        // A reference not in the map dangled before the edit and is kept as it was.
        let after = |r: NodeRef| map.get(&r).copied().unwrap_or(Some(r));
        match p {
            TtPoint::Lsb | TtPoint::Rsb => Some((p, false)),
            TtPoint::Node(r) => match after(r) {
                Some(n) => Some((TtPoint::Node(n), false)),
                None if overlap => {
                    let o = self.old_node(r).map(Place::of)?;
                    self.nearest_created(o.x, o.y).map(|n| (TtPoint::Node(n), true))
                }
                None => None,
            },
            TtPoint::Intersection { a, b } => {
                if overlap && !map_keeps(p, map) {
                    let (x, y) = self.intersection(a, b)?;
                    return self.nearest_created(x, y).map(|n| (TtPoint::Node(n), true));
                }
                Some((TtPoint::Intersection { a: after(a)?, b: after(b)? }, false))
            }
        }
    }

    /// Where the segments ending at `a` and `b` cross, by their chords; the middle of the two
    /// end nodes when the chords are parallel.
    fn intersection(&self, a: NodeRef, b: NodeRef) -> Option<(f64, f64)> {
        let chord = |r: NodeRef| -> Option<((f64, f64), (f64, f64))> {
            let path = self.old.get(r.path)?;
            let end = path.nodes.get(r.node)?;
            let len = path.nodes.len();
            let start = (1..len).map(|k| (r.node + len - k) % len).filter_map(|i| path.nodes.get(i)).find(|n| on_curve(n))?;
            Some(((start.x, start.y), (end.x, end.y)))
        };
        let ((x1, y1), (x2, y2)) = chord(a)?;
        let ((x3, y3), (x4, y4)) = chord(b)?;
        let d = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
        if d.abs() < SAME {
            return Some(((x2 + x4) / 2.0, (y2 + y4) / 2.0));
        }
        let t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / d;
        let p = (x1 + t * (x2 - x1), y1 + t * (y2 - y1));
        (p.0.is_finite() && p.1.is_finite()).then_some(p)
    }

    fn nearest_created(&mut self, x: f64, y: f64) -> Option<NodeRef> {
        if self.created.is_none() {
            let mut old_places: HashMap<(i64, i64), Vec<&Node>> = HashMap::new();
            for n in self.old.iter().flat_map(|p| &p.nodes) {
                old_places.entry(cell(n.x, n.y)).or_default().push(n);
            }
            let mut created = Vec::new();
            for (pi, p) in self.new.iter().enumerate() {
                for (ni, n) in p.nodes.iter().enumerate() {
                    let (cx, cy) = cell(n.x, n.y);
                    let known = (-1..=1i64)
                        .flat_map(|dx| (-1..=1i64).map(move |dy| (cx.saturating_add(dx), cy.saturating_add(dy))))
                        .filter_map(|k| old_places.get(&k))
                        .flatten()
                        .any(|o| same_place(o, n));
                    if on_curve(n) && !known {
                        created.push(NodeRef::new(pi, ni));
                    }
                }
            }
            self.created = Some(created);
        }
        let created = self.created.as_deref().unwrap_or_default();
        created.iter().filter_map(|c| self.new_node(*c).map(|n| ((n.x - x).hypot(n.y - y), *c))).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, c)| c)
    }
}
