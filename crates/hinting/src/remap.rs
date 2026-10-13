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
//! | 2 | a node of the same class lies at its place, or within [`NEAR`] of it (grid rounding); among several, the one in the same path, then with the same neighbours, then the closest index |
//! | 3 | its path has the same number of nodes, of the same classes, in the same order: the node moved |
//!
//! A node not found is gone. A PostScript end on a gone node becomes a number, the node's last y
//! (horizontal hints) or x (vertical hints). A TrueType instruction on a gone node is removed and
//! reported; with [`RemapMode::OverlapRemoval`] it moves to the nearest on-curve node the removal
//! created, and so does an instruction on an intersection extra node (the handbook's rule for
//! Remove Overlaps at export, `hinting.tt.export.overlapRemovalMovesHints`).
//!
//! Left alone: layers without hints, layers whose stored hints the command changed (a command
//! that edits hints keeps its own references right), references that dangled before the edit,
//! hints that do not parse, and background layers. A layer that needs no change keeps its value;
//! a font that needs none comes back as given.
//!
//! The engine runs [`remap_after_edit`] in `Session::realign_after`, after the component
//! re-alignment, so both land in the command's undo step. Rows: `hinting.ps.manual.linkOnDrop`
//! (links survive edits), `hinting.ps.masters.transferByNodeLink`,
//! `hinting.tt.export.overlapRemovalMovesHints` (the editor half).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use fontcraft_model::{Font, Layer, Node, NodeKind, Path, Shape};

use crate::{Edge, HintingError, NodeRef, PsHint, TtInstruction, TtKind, TtPoint, ps_hints, set_ps_hints, set_tt_instructions, tt_instructions};

/// How far (font units, each axis) step 2 looks for a node that an edit rounded to the grid.
pub const NEAR: f64 = 0.5;
/// Places closer than this are the same place.
const SAME: f64 = 1e-6;

/// What remapping did to one font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemapReport {
    /// Messages for removed TrueType instructions, one per instruction: glyph, layer, what.
    pub removed: Vec<String>,
    /// PostScript edges turned into numbers.
    pub unlinked: usize,
    /// TrueType points moved to a node the overlap removal created.
    pub moved: usize,
}

impl RemapReport {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.unlinked == 0 && self.moved == 0
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

/// `new` with the hint references of every changed layer remapped against `old`. Returns `new`
/// unchanged (the same `Arc`s) when no layer with hints changed its node structure.
pub fn remap_after_edit(old: &Font, mut new: Font, mode: RemapMode) -> Result<(Font, RemapReport), HintingError> {
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
            if layer.info.hints.is_empty() {
                continue;
            }
            let Some(was) = prev.layers.iter().find(|l| l.id == layer.id) else { continue };
            if let Some(out) = remap_layer(was, layer, &g.name, mode, &mut report) {
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
    if old.info.hints != new.info.hints {
        return None;
    }
    let (old_paths, new_paths) = (paths(old), paths(new));
    if old_paths.len() == new_paths.len() && old_paths.iter().zip(&new_paths).all(|(a, b)| same_nodes(a, b)) {
        return None;
    }
    let ps = ps_hints(&old.info).ok()?;
    let tt = tt_instructions(&old.info).ok()?;
    let mut refs = BTreeSet::new();
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
    let mut m = Matcher { old: old_paths, new: new_paths, grid: None, created: None };
    // `None`: the node is gone. References that dangled before the edit are not in the map.
    let map: BTreeMap<NodeRef, Option<NodeRef>> = refs.iter().filter_map(|r| m.locate(*r).map(|found| (*r, found))).collect();
    let overlap = mode == RemapMode::OverlapRemoval;
    let has_intersections = tt.iter().any(|(_, t)| t.points().iter().any(|p| matches!(p, TtPoint::Intersection { .. })));
    if map.iter().all(|(k, v)| *v == Some(*k)) && !(overlap && has_intersections) {
        return None;
    }

    let mut local = RemapReport::default();
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
        local.removed.push(format!(
            "glyph `{glyph}`, layer `{}`: the {} instruction on {points} was removed: its node is gone",
            new.id,
            kind_name(&t.kind)
        ));
    }
    // A triple hint that lost a member is no longer one: the others stay as plain Stem instructions.
    for t in &mut kept {
        if let TtKind::Stem { triple, .. } = &mut t.kind
            && triple.is_some_and(|g| broken_triples.contains(&g))
        {
            *triple = None;
        }
    }

    if !ps_changed && !tt_changed {
        return None;
    }
    let mut out = new.clone();
    if ps_changed {
        set_ps_hints(&mut out.info, &ps_new).ok()?;
    }
    if tt_changed {
        set_tt_instructions(&mut out.info, &kept).ok()?;
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
    /// The new layer's nodes by grid cell, built on first use.
    grid: Option<HashMap<(i64, i64), Vec<NodeRef>>>,
    /// On-curve nodes of the new layer at places no old node had, built on first use.
    created: Option<Vec<NodeRef>>,
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
        if self.new_node(r).is_some_and(|n| on_curve(n) == o.on && (n.x - o.x).abs() <= SAME && (n.y - o.y).abs() <= SAME) {
            return Some(Some(r));
        }
        if let Some(found) = self.by_place(r, o) {
            return Some(Some(found));
        }
        let same_structure = match (self.old.get(r.path), self.new.get(r.path)) {
            (Some(a), Some(b)) => a.nodes.len() == b.nodes.len() && a.nodes.iter().zip(&b.nodes).all(|(m, n)| on_curve(m) == on_curve(n)),
            _ => false,
        };
        Some(same_structure.then_some(r))
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

    /// Step 2 of the module doc.
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
