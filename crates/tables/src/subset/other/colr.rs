//! Subsetting `COLR` and pruning `CPAL`, ported from fontTools' subsetter and its
//! `colorLib.builder` and `colorLib.unbuilder` (MIT, Copyright 2013 Google, Inc. and 2020
//! Google LLC).
//!
//! The closure adds the layer glyphs of version 0 records and every glyph a version 1 paint
//! graph draws (`PaintGlyph`, `PaintColrGlyph`), until nothing is added. The subset keeps the
//! records of the glyphs after that closure. Version 1 paint graphs are first expanded into
//! trees (each `PaintColrLayers` inlined, nested ones flattened, single layers unwrapped), then
//! rebuilt into a new layer list the way fontTools' `LayerListBuilder` does it: lists of more
//! than 255 layers become trees, and a run of 2 to 33 layers that an earlier list already
//! holds is reused, the longest run first. A table left without version 1 glyphs becomes
//! version 0. The clip list keeps the boxes of the kept glyphs; the variation index map and
//! store are kept whole.
//!
//! `CPAL` keeps only the palette entries the subset `COLR` uses (renumbered), unless the font
//! has an `SVG ` table, which may use any entry; without `COLR` it goes.
//!
//! Limits: paint nesting [`MAX_PAINT_DEPTH`], expanded paints [`MAX_COLR_PAINTS`], and the
//! layer reuse search [`MAX_REUSE_WORK`] (past it, layers are no longer reused).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::super::Plan;
use super::close_over;
use crate::reorder::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Deepest paint nesting followed.
pub const MAX_PAINT_DEPTH: usize = 64;
/// Most paints the expanded trees of one subset may hold.
pub const MAX_COLR_PAINTS: usize = 1 << 20;
/// Most slice comparisons the layer reuse search may make in one subset.
pub const MAX_REUSE_WORK: usize = 1 << 26;
/// Most layers one `PaintColrLayers` lists.
const MAX_LAYER_RUN: usize = 255;
/// Longest run of layers looked up for reuse, after fontTools' `_MAX_REUSE_LEN` (32): runs of
/// 2 up to `2 + 32 - 1` layers.
const MAX_REUSE_LEN: usize = 32;

/// What a paint's offset field points at.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Paint,
    ColorLine,
    VarColorLine,
    Affine,
    VarAffine,
}

/// The byte length of a paint of `format` and its offset fields.
fn layout(format: u8) -> Option<(usize, &'static [(usize, Kind)])> {
    use Kind::*;
    Some(match format {
        1 => (6, &[]),
        2 => (5, &[]),
        3 => (9, &[]),
        4 | 6 => (16, &[(1, ColorLine)]),
        5 | 7 => (20, &[(1, VarColorLine)]),
        8 => (12, &[(1, ColorLine)]),
        9 => (16, &[(1, VarColorLine)]),
        10 => (6, &[(1, Paint)]),
        11 => (3, &[]),
        12 => (7, &[(1, Paint), (4, Affine)]),
        13 => (7, &[(1, Paint), (4, VarAffine)]),
        14 | 16 | 28 => (8, &[(1, Paint)]),
        15 | 17 | 29 => (12, &[(1, Paint)]),
        18 | 30 => (12, &[(1, Paint)]),
        19 | 31 => (16, &[(1, Paint)]),
        20 | 24 => (6, &[(1, Paint)]),
        21 | 25 => (10, &[(1, Paint)]),
        22 | 26 => (10, &[(1, Paint)]),
        23 | 27 => (14, &[(1, Paint)]),
        32 => (8, &[(1, Paint), (5, Paint)]),
        _ => return None,
    })
}

fn u24(d: Data<'_>, at: usize) -> Result<usize> {
    Ok(usize::from(d.u8(at)?) << 16 | usize::from(d.u16(at + 1)?))
}

/// The parts of a COLR table this module reads.
struct Colr<'a> {
    d: Data<'a>,
    version: u16,
    /// Version 0 records: base glyph → (layer glyph, palette index) list.
    v0: BTreeMap<u16, Vec<(u16, u16)>>,
    /// Version 1 base glyphs and the positions of their paints, in record order.
    v1: Vec<(u16, usize)>,
    /// Positions of the layer list's paints.
    layers: Vec<usize>,
    clip_list: Option<usize>,
    var_index_map: Option<usize>,
    store: Option<usize>,
}

impl<'a> Colr<'a> {
    fn read(bytes: &'a [u8]) -> Result<Self> {
        let d = Data::new(*b"COLR", bytes);
        let version = d.u16(0)?;
        let base_count = usize::from(d.u16(2)?);
        let base_at = d.u32_usize(4)?;
        let layer_at = d.u32_usize(8)?;
        let layer_count = usize::from(d.u16(12)?);
        let mut v0 = BTreeMap::new();
        if base_at != 0 && base_count != 0 {
            for i in 0..base_count {
                let r = base_at + 6 * i;
                let (g, first, n) = (d.u16(r)?, usize::from(d.u16(r + 2)?), usize::from(d.u16(r + 4)?));
                if first + n > layer_count {
                    return Err(d.bad("has a color glyph whose layers run past the layer records"));
                }
                let mut list = Vec::with_capacity(n);
                for l in first..first + n {
                    list.push((d.u16(layer_at + 4 * l)?, d.u16(layer_at + 4 * l + 2)?));
                }
                v0.insert(g, list);
            }
        }
        let mut colr = Colr { d, version, v0, v1: Vec::new(), layers: Vec::new(), clip_list: None, var_index_map: None, store: None };
        if version == 0 {
            return Ok(colr);
        }
        let offset = |at: usize| -> Result<Option<usize>> { d.u32_usize(at).map(|o| (o != 0).then_some(o)) };
        if let Some(list) = offset(14)? {
            let count = d.u32_usize(list)?;
            d.slice(list + 4, count.saturating_mul(6))?;
            for i in 0..count {
                let r = list + 4 + 6 * i;
                colr.v1.push((d.u16(r)?, list + d.u32_usize(r + 2)?));
            }
        }
        if let Some(list) = offset(18)? {
            let count = d.u32_usize(list)?;
            d.slice(list + 4, count.saturating_mul(4))?;
            for i in 0..count {
                colr.layers.push(list + d.u32_usize(list + 4 + 4 * i)?);
            }
        }
        colr.clip_list = offset(22)?;
        colr.var_index_map = offset(26)?;
        colr.store = offset(30)?;
        Ok(colr)
    }

    /// The paints a paint at `at` points at.
    fn children(&self, at: usize, out: &mut Vec<usize>) -> Result<()> {
        let format = self.d.u8(at)?;
        if format == 1 {
            let (n, first) = (usize::from(self.d.u8(at + 1)?), self.d.u32_usize(at + 2)?);
            for i in first..first.saturating_add(n) {
                out.push(*self.layers.get(i).ok_or_else(|| self.d.bad("has a PaintColrLayers past the end of its layer list"))?);
            }
            return Ok(());
        }
        let (_, kids) = layout(format).ok_or_else(|| self.d.bad(format!("has a paint of format {format}")))?;
        for (pos, kind) in kids {
            if *kind == Kind::Paint {
                out.push(at + u24(self.d, at + pos)?);
            }
        }
        Ok(())
    }

    /// The glyph a `PaintGlyph` or `PaintColrGlyph` at `at` draws.
    fn paint_glyph(&self, at: usize) -> Result<Option<u16>> {
        match self.d.u8(at)? {
            10 => Ok(Some(self.d.u16(at + 4)?)),
            11 => Ok(Some(self.d.u16(at + 1)?)),
            _ => Ok(None),
        }
    }
}

/// Adds the glyphs the COLR records of `glyphs` use, until nothing is added.
pub(crate) fn close_glyphs(font: &SfntFont, glyphs: &mut BTreeSet<u32>, n: usize) -> Result<()> {
    let Some(bytes) = font.get(*b"COLR") else { return Ok(()) };
    let colr = Colr::read(bytes)?;
    let roots: HashMap<u16, usize> = colr.v1.iter().map(|(g, at)| (*g, *at)).collect();
    let mut visited: HashSet<usize> = HashSet::new();
    let mut stack = Vec::new();
    close_over(glyphs, n, true, |g, out| {
        let Ok(g) = u16::try_from(g) else { return Ok(()) };
        if let Some(layers) = colr.v0.get(&g) {
            out.extend(layers.iter().map(|(l, _)| u32::from(*l)));
        }
        if let Some(root) = roots.get(&g) {
            stack.push(*root);
            while let Some(at) = stack.pop() {
                if !visited.insert(at) {
                    continue;
                }
                if let Some(drawn) = colr.paint_glyph(at)? {
                    out.push(u32::from(drawn));
                }
                colr.children(at, &mut stack)?;
            }
        }
        Ok(())
    })
}

// ── Expanded paints ──

/// A paint in expanded form.
#[derive(Clone, Debug)]
enum Node {
    /// A `PaintColrLayers`, its layers inlined.
    Layers(Vec<Node>),
    /// Any other paint: its bytes with the offset fields zeroed, and what they point at.
    Paint { bytes: Vec<u8>, paints: Vec<(usize, Node)>, blobs: Vec<(usize, Kind, Vec<u8>)> },
}

struct Expand<'a, 'p> {
    colr: &'a Colr<'a>,
    plan: &'p Plan,
    left: usize,
}

impl Expand<'_, '_> {
    fn node(&mut self, at: usize, depth: usize) -> Result<Node> {
        if depth > MAX_PAINT_DEPTH {
            return Err(TablesError::invalid(format!("the 'COLR' table nests paints deeper than {MAX_PAINT_DEPTH}")));
        }
        self.left = self.left.checked_sub(1).ok_or(TablesError::TooLarge { what: "the expanded COLR paints", limit: MAX_COLR_PAINTS })?;
        let d = self.colr.d;
        let format = d.u8(at)?;
        if format == 1 {
            let mut kids = Vec::new();
            self.colr.children(at, &mut kids)?;
            let mut layers = Vec::with_capacity(kids.len());
            for k in kids {
                match self.node(k, depth + 1)? {
                    Node::Layers(inner) => layers.extend(inner),
                    other => layers.push(other),
                }
            }
            if layers.len() == 1
                && let Some(only) = layers.pop()
            {
                return Ok(only);
            }
            return Ok(Node::Layers(layers));
        }
        let (len, kids) = layout(format).ok_or_else(|| d.bad(format!("has a paint of format {format}")))?;
        let mut bytes = d.slice(at, len)?.to_vec();
        let mut paints = Vec::new();
        let mut blobs = Vec::new();
        for (pos, kind) in kids {
            let off = u24(d, at + pos)?;
            if off == 0 {
                return Err(d.bad(format!("has a paint of format {format} with a missing subtable")));
            }
            if let Some(field) = bytes.get_mut(*pos..pos + 3) {
                field.fill(0);
            }
            let target = at + off;
            match kind {
                Kind::Paint => paints.push((*pos, self.node(target, depth + 1)?)),
                Kind::ColorLine | Kind::VarColorLine => {
                    let stop = if *kind == Kind::ColorLine { 6 } else { 10 };
                    let stops = usize::from(d.u16(target + 1)?);
                    blobs.push((*pos, *kind, d.slice(target, 3 + stop * stops)?.to_vec()));
                }
                Kind::Affine => blobs.push((*pos, *kind, d.slice(target, 24)?.to_vec())),
                Kind::VarAffine => blobs.push((*pos, *kind, d.slice(target, 28)?.to_vec())),
            }
        }
        // Glyph references follow the output numbering.
        let glyph_at = match format {
            10 => Some(4),
            11 => Some(1),
            _ => None,
        };
        if let Some(g_at) = glyph_at {
            let old = Data::new(*b"COLR", &bytes).u16(g_at)?;
            let new = new_glyph(self.plan, old)?;
            bytes.get_mut(g_at..g_at + 2).ok_or_else(|| d.bad("has a short paint"))?.copy_from_slice(&new.to_be_bytes());
        }
        Ok(Node::Paint { bytes, paints, blobs })
    }
}

fn new_glyph(plan: &Plan, old: u16) -> Result<u16> {
    let new =
        plan.glyph_map.get(&u32::from(old)).ok_or_else(|| TablesError::invalid(format!("a color glyph uses glyph {old}, which the subset lacks")))?;
    to_u16(*new as usize, "a glyph id")
}

/// Calls `f` on every palette index field of `node`.
fn palette_fields(node: &mut Node, f: &mut dyn FnMut(&mut [u8])) {
    match node {
        Node::Layers(list) => list.iter_mut().for_each(|n| palette_fields(n, f)),
        Node::Paint { bytes, paints, blobs } => {
            if matches!(bytes.first(), Some(2 | 3))
                && let Some(field) = bytes.get_mut(1..3)
            {
                f(field);
            }
            for (_, kind, blob) in blobs.iter_mut() {
                let stop = match kind {
                    Kind::ColorLine => 6,
                    Kind::VarColorLine => 10,
                    _ => continue,
                };
                let count = Data::new(*b"COLR", blob).u16(1).map(usize::from).unwrap_or(0);
                for i in 0..count {
                    let at = 3 + stop * i + 2;
                    if let Some(field) = blob.get_mut(at..at + 2) {
                        f(field);
                    }
                }
            }
            paints.iter_mut().for_each(|(_, n)| palette_fields(n, f));
        }
    }
}

// ── Rebuilt paints ──

/// A paint ready to write; its child paints are ids into the builder's paint list.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Built {
    bytes: Vec<u8>,
    paints: Vec<(usize, u32)>,
    blobs: Vec<(usize, Vec<u8>)>,
}

/// A layer tree of [`n_ary_tree`].
enum Tree {
    Leaf(u32),
    List(Vec<Tree>),
}

/// fontTools' `build_n_ary_tree`: `leaves` as a tree whose lists hold at most `n` items.
fn n_ary_tree(leaves: Vec<Tree>, n: usize) -> Vec<Tree> {
    let len = leaves.len();
    if len <= n {
        return leaves;
    }
    let mut depth = 0u32;
    let mut cap = 1usize;
    while cap < len {
        cap = cap.saturating_mul(n);
        depth += 1;
    }
    let full = n.saturating_pow(depth.saturating_sub(1));
    let mut root: Vec<Tree> = Vec::new();
    let mut rest = leaves.into_iter().peekable();
    let mut unassigned = Vec::new();
    while rest.peek().is_some() {
        let mut subtree: Vec<Tree> = rest.by_ref().take(full).collect();
        if subtree.len() < full {
            unassigned = subtree;
            break;
        }
        while subtree.len() > n {
            let mut grouped = Vec::new();
            let mut it = subtree.into_iter().peekable();
            while it.peek().is_some() {
                grouped.push(Tree::List(it.by_ref().take(n).collect()));
            }
            subtree = grouped;
        }
        root.push(Tree::List(subtree));
    }
    if !unassigned.is_empty() {
        let subtree = n_ary_tree(unassigned, n);
        if subtree.len() <= n.saturating_sub(root.len()) {
            root.extend(subtree);
        } else {
            root.push(Tree::List(subtree));
        }
    }
    root
}

/// fontTools' `LayerListBuilder` with its `LayerReuseCache`.
struct Builder {
    paints: Vec<Built>,
    ids: HashMap<Built, u32>,
    layers: Vec<u32>,
    pool: HashMap<Vec<u32>, usize>,
    work_left: usize,
}

impl Builder {
    fn intern(&mut self, b: Built) -> Result<u32> {
        if let Some(id) = self.ids.get(&b) {
            return Ok(*id);
        }
        let id = u32::try_from(self.paints.len()).map_err(|_| TablesError::TooLarge { what: "the COLR paints", limit: u32::MAX as usize })?;
        self.ids.insert(b.clone(), id);
        self.paints.push(b);
        Ok(id)
    }

    fn colr_layers(&mut self, num: usize, first: usize) -> Result<u32> {
        let mut bytes = vec![1u8, u8::try_from(num).map_err(|_| TablesError::TooLarge { what: "a layer run", limit: 255 })?];
        bytes.extend_from_slice(&to_u32(first, "the COLR layer list")?.to_be_bytes());
        self.intern(Built { bytes, paints: Vec::new(), blobs: Vec::new() })
    }

    fn build(&mut self, node: Node) -> Result<u32> {
        match node {
            Node::Layers(list) => {
                let mut built = Vec::with_capacity(list.len());
                for n in list {
                    built.push(self.build(n)?);
                }
                self.layers_paint(built)
            }
            Node::Paint { bytes, paints, blobs } => {
                let mut kids = Vec::with_capacity(paints.len());
                for (pos, n) in paints {
                    kids.push((pos, self.build(n)?));
                }
                let blobs = blobs.into_iter().map(|(pos, _, b)| (pos, b)).collect();
                self.intern(Built { bytes, paints: kids, blobs })
            }
        }
    }

    /// The paint for a list of built layers (`_beforeBuildPaintColrLayers`).
    fn layers_paint(&mut self, layers: Vec<u32>) -> Result<u32> {
        if let [only] = layers.as_slice() {
            return Ok(*only);
        }
        let layers = self.try_reuse(layers)?;
        let is_tree = layers.len() > MAX_LAYER_RUN;
        let tree = n_ary_tree(layers.into_iter().map(Tree::Leaf).collect(), MAX_LAYER_RUN);
        let mut out = Vec::with_capacity(tree.len());
        for t in tree {
            out.push(self.tree_paint(t)?);
        }
        if let [only] = out.as_slice() {
            return Ok(*only);
        }
        let first = self.layers.len();
        let id = self.colr_layers(out.len(), first)?;
        self.layers.extend_from_slice(&out);
        if !is_tree {
            self.add(&out, first);
        }
        Ok(id)
    }

    fn tree_paint(&mut self, t: Tree) -> Result<u32> {
        match t {
            Tree::Leaf(id) => Ok(id),
            Tree::List(items) => {
                let mut ids = Vec::with_capacity(items.len());
                for i in items {
                    ids.push(self.tree_paint(i)?);
                }
                self.layers_paint(ids)
            }
        }
    }

    /// The runs fontTools' `_reuse_ranges` lists for `n` layers.
    fn ranges(n: usize) -> impl Iterator<Item = (usize, usize)> {
        (0..n).flat_map(move |lb| (lb + 2..n.min(lb + 1 + MAX_REUSE_LEN) + 1).map(move |ub| (lb, ub)))
    }

    fn spend(&mut self, n: usize) -> bool {
        match self.work_left.checked_sub(n) {
            Some(left) => {
                self.work_left = left;
                true
            }
            None => {
                self.work_left = 0;
                false
            }
        }
    }

    fn try_reuse(&mut self, mut layers: Vec<u32>) -> Result<Vec<u32>> {
        loop {
            let mut ranges: Vec<(usize, usize)> = Self::ranges(layers.len()).collect();
            if !self.spend(ranges.len().saturating_mul(MAX_REUSE_LEN)) {
                return Ok(layers);
            }
            ranges.sort_by_key(|(lb, ub)| std::cmp::Reverse((ub - lb, *ub, *lb)));
            let found = ranges.into_iter().find_map(|(lb, ub)| layers.get(lb..ub).and_then(|run| self.pool.get(run)).map(|first| (lb, ub, *first)));
            let Some((lb, ub, first)) = found else { return Ok(layers) };
            let slice = self.colr_layers(ub - lb, first)?;
            layers.splice(lb..ub, std::iter::once(slice));
        }
    }

    fn add(&mut self, layers: &[u32], first: usize) {
        let ranges: Vec<(usize, usize)> = Self::ranges(layers.len()).collect();
        if !self.spend(ranges.len().saturating_mul(MAX_REUSE_LEN)) {
            return;
        }
        for (lb, ub) in ranges {
            if let Some(run) = layers.get(lb..ub) {
                self.pool.insert(run.to_vec(), lb + first);
            }
        }
    }
}

/// The palette entries a subset COLR uses, sorted (the foreground index 0xFFFF left out).
pub(crate) struct PaletteUse(Vec<u16>);

/// Writes `COLR` subset to the plan's COLR closure; `None` when no color glyph is left.
pub(crate) fn subset_colr(plan: &Plan, bytes: &[u8], prune_palettes: bool) -> Result<Option<(Vec<u8>, Option<PaletteUse>)>> {
    let colr = Colr::read(bytes)?;
    let keep = |g: u16| plan.colr_glyphs.contains(&u32::from(g));
    let mut v0: BTreeMap<u16, Vec<(u16, u16)>> = BTreeMap::new();
    for (g, layers) in &colr.v0 {
        if keep(*g) {
            let mut list = Vec::with_capacity(layers.len());
            for (l, p) in layers {
                list.push((new_glyph(plan, *l)?, *p));
            }
            v0.insert(new_glyph(plan, *g)?, list);
        }
    }
    let mut expand = Expand { colr: &colr, plan, left: MAX_COLR_PAINTS };
    let mut v1: Vec<(u16, Node)> = Vec::new();
    for (g, at) in &colr.v1 {
        if keep(*g) {
            v1.push((new_glyph(plan, *g)?, expand.node(*at, 0)?));
        }
    }
    let version = if colr.version > 0 && !v1.is_empty() { 1 } else { 0 };
    if version == 0 && v0.is_empty() {
        return Ok(None);
    }

    let mut palette = None;
    if prune_palettes {
        let mut used = BTreeSet::new();
        for layers in v0.values() {
            used.extend(layers.iter().map(|(_, p)| *p));
        }
        let mut collect = |f: &mut [u8]| {
            used.insert(u16::from_be_bytes([f.first().copied().unwrap_or(0), f.get(1).copied().unwrap_or(0)]));
        };
        for (_, n) in &mut v1 {
            palette_fields(n, &mut collect);
        }
        used.remove(&0xFFFF);
        let map: HashMap<u16, u16> = used.iter().enumerate().map(|(new, old)| (*old, new as u16)).collect();
        let renumber = |p: u16| map.get(&p).copied().unwrap_or(p);
        for layers in v0.values_mut() {
            for (_, p) in layers.iter_mut() {
                *p = renumber(*p);
            }
        }
        let mut rewrite = |f: &mut [u8]| {
            let old = u16::from_be_bytes([f.first().copied().unwrap_or(0), f.get(1).copied().unwrap_or(0)]);
            f.copy_from_slice(&renumber(old).to_be_bytes());
        };
        for (_, n) in &mut v1 {
            palette_fields(n, &mut rewrite);
        }
        palette = Some(PaletteUse(used.into_iter().collect()));
    }

    let mut out = Vec::new();
    let header = if version == 0 { 14 } else { 34 };
    let base_at = header;
    let layer_at = base_at + 6 * v0.len();
    let layer_count: usize = v0.values().map(Vec::len).sum();
    let mut v0_bytes = Vec::with_capacity(6 * v0.len() + 4 * layer_count);
    let mut first = 0usize;
    for (g, layers) in &v0 {
        put_u16(&mut v0_bytes, *g);
        put_u16(&mut v0_bytes, to_u16(first, "the COLR layer records")?);
        put_u16(&mut v0_bytes, to_u16(layers.len(), "a color glyph's layers")?);
        first += layers.len();
    }
    for layers in v0.values() {
        for (g, p) in layers {
            put_u16(&mut v0_bytes, *g);
            put_u16(&mut v0_bytes, *p);
        }
    }
    put_u16(&mut out, version);
    put_u16(&mut out, to_u16(v0.len(), "the COLR base glyph records")?);
    put_u32(&mut out, if v0.is_empty() { 0 } else { to_u32(base_at, "the COLR table")? });
    put_u32(&mut out, if layer_count == 0 { 0 } else { to_u32(layer_at, "the COLR table")? });
    put_u16(&mut out, to_u16(layer_count, "the COLR layer records")?);
    if version == 0 {
        out.extend(v0_bytes);
        return Ok(Some((out, palette)));
    }

    let mut b = Builder { paints: Vec::new(), ids: HashMap::new(), layers: Vec::new(), pool: HashMap::new(), work_left: MAX_REUSE_WORK };
    let mut roots = Vec::with_capacity(v1.len());
    for (g, node) in v1 {
        roots.push((g, b.build(node)?));
    }
    roots.sort_by_key(|(g, _)| *g);
    let clips = clip_boxes(&colr, plan)?;
    let var_index_map = match colr.var_index_map {
        Some(at) => Some(index_map_bytes(colr.d, at)?),
        None => None,
    };
    let store = match colr.store {
        Some(at) => {
            let tail = bytes.get(at..).ok_or_else(|| colr.d.bad("has its variation store past its end"))?;
            Some(crate::subset::var::store::Store::read(*b"COLR", tail)?.whole()?.write()?)
        }
        None => None,
    };

    // Body: v0 records, base glyph list, layer list, clip list, index map, store, then paints.
    let mut body = v0_bytes;
    let base_list_at = header + body.len();
    let base_list_len = 4 + 6 * roots.len();
    let layer_list_at = base_list_at + base_list_len;
    let layer_list_len = if b.layers.is_empty() { 0 } else { 4 + 4 * b.layers.len() };
    let clip_at = layer_list_at + layer_list_len;
    let clip_bytes = clips.map(|c| write_clips(&c)).transpose()?;
    let map_at = clip_at + clip_bytes.as_ref().map_or(0, Vec::len);
    let store_at = map_at + var_index_map.as_ref().map_or(0, Vec::len);
    let paints_at = store_at + store.as_ref().map_or(0, Vec::len);
    let (pool, positions) = write_paints(&b.paints, paints_at)?;
    let pos = |id: u32| positions.get(id as usize).copied().ok_or_else(|| TablesError::invalid("a COLR paint was lost while writing"));

    put_u32(&mut body, to_u32(roots.len(), "the COLR base glyph list")?);
    for (g, id) in &roots {
        put_u16(&mut body, *g);
        put_u32(&mut body, to_u32(pos(*id)? - base_list_at, "the COLR table")?);
    }
    if !b.layers.is_empty() {
        put_u32(&mut body, to_u32(b.layers.len(), "the COLR layer list")?);
        for id in &b.layers {
            put_u32(&mut body, to_u32(pos(*id)? - layer_list_at, "the COLR table")?);
        }
    }
    for part in [&clip_bytes, &var_index_map, &store].into_iter().flatten() {
        body.extend_from_slice(part);
    }
    body.extend(pool);

    put_u32(&mut out, to_u32(base_list_at, "the COLR table")?);
    put_u32(&mut out, if layer_list_len == 0 { 0 } else { to_u32(layer_list_at, "the COLR table")? });
    put_u32(&mut out, if clip_bytes.is_some() { to_u32(clip_at, "the COLR table")? } else { 0 });
    put_u32(&mut out, if var_index_map.is_some() { to_u32(map_at, "the COLR table")? } else { 0 });
    put_u32(&mut out, if store.is_some() { to_u32(store_at, "the COLR table")? } else { 0 });
    out.extend(body);
    Ok(Some((out, palette)))
}

/// The paints, in an order where every paint comes before the paints it points at, starting
/// at `base`; and each paint's position.
fn write_paints(paints: &[Built], base: usize) -> Result<(Vec<u8>, Vec<usize>)> {
    // A paint is interned after the paints it points at, so higher ids come first.
    let order: Vec<u32> = (0..paints.len() as u32).rev().collect();
    let mut positions = vec![0usize; paints.len()];
    let mut at = base;
    for id in &order {
        if let (Some(slot), Some(p)) = (positions.get_mut(*id as usize), paints.get(*id as usize)) {
            *slot = at;
            at += p.bytes.len();
        }
    }
    let mut blob_at: HashMap<&[u8], usize> = HashMap::new();
    let mut blobs: Vec<u8> = Vec::new();
    let blobs_base = at;
    let mut out = Vec::with_capacity(at - base);
    for id in &order {
        let Some(p) = paints.get(*id as usize) else { continue };
        let me = positions.get(*id as usize).copied().unwrap_or(0);
        let mut bytes = p.bytes.clone();
        let mut patch = |field: usize, target: usize| -> Result<()> {
            let off = target
                .checked_sub(me)
                .filter(|o| *o > 0 && *o < 1 << 24)
                .ok_or(TablesError::TooLarge { what: "a COLR paint offset", limit: (1 << 24) - 1 })?;
            let slot = bytes.get_mut(field..field + 3).ok_or_else(|| TablesError::invalid("a COLR paint is cut short"))?;
            slot.copy_from_slice((off as u32).to_be_bytes().get(1..4).unwrap_or_default());
            Ok(())
        };
        for (field, kid) in &p.paints {
            patch(*field, positions.get(*kid as usize).copied().unwrap_or(0))?;
        }
        for (field, blob) in &p.blobs {
            let target = match blob_at.get(blob.as_slice()) {
                Some(t) => *t,
                None => {
                    let t = blobs_base + blobs.len();
                    blobs.extend_from_slice(blob);
                    blob_at.insert(blob, t);
                    t
                }
            };
            patch(*field, target)?;
        }
        out.extend(bytes);
    }
    out.extend(blobs);
    Ok((out, positions))
}

/// Output glyph → clip box bytes, for the kept glyphs of the clip list.
fn clip_boxes(colr: &Colr<'_>, plan: &Plan) -> Result<Option<BTreeMap<u16, Vec<u8>>>> {
    let Some(at) = colr.clip_list else { return Ok(None) };
    let d = colr.d;
    let count = d.u32_usize(at + 1)?;
    d.slice(at + 5, count.saturating_mul(7))?;
    let mut out = BTreeMap::new();
    let mut budget = MAX_COLR_PAINTS;
    for i in 0..count {
        let r = at + 5 + 7 * i;
        let (start, end) = (d.u16(r)?, d.u16(r + 2)?);
        // A clip whose first glyph comes after its last covers no glyph, as in fontTools.
        if start > end {
            continue;
        }
        let boxed = at + u24(d, r + 4)?;
        let len = if d.u8(boxed)? == 2 { 13 } else { 9 };
        let bytes = d.slice(boxed, len)?;
        for g in plan.colr_glyphs.range(u32::from(start)..=u32::from(end)) {
            budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the COLR clip records", limit: MAX_COLR_PAINTS })?;
            out.insert(new_glyph(plan, *g as u16)?, bytes.to_vec());
        }
    }
    Ok(Some(out))
}

/// A clip list for `clips`: one record per run of consecutive glyphs sharing a box.
fn write_clips(clips: &BTreeMap<u16, Vec<u8>>) -> Result<Vec<u8>> {
    let mut runs: Vec<(u16, u16, &[u8])> = Vec::new();
    for (g, b) in clips {
        match runs.last_mut() {
            Some((_, end, last)) if u32::from(*end) + 1 == u32::from(*g) && *last == b.as_slice() => *end = *g,
            _ => runs.push((*g, *g, b)),
        }
    }
    let mut boxes: Vec<&[u8]> = Vec::new();
    let mut box_index: HashMap<&[u8], usize> = HashMap::new();
    for (_, _, b) in &runs {
        if !box_index.contains_key(b) {
            box_index.insert(b, boxes.len());
            boxes.push(b);
        }
    }
    let header = 5 + 7 * runs.len();
    let mut box_at = Vec::with_capacity(boxes.len());
    let mut at = header;
    for b in &boxes {
        box_at.push(at);
        at += b.len();
    }
    let mut out = vec![1u8];
    put_u32(&mut out, to_u32(runs.len(), "the COLR clip list")?);
    for (start, end, b) in &runs {
        put_u16(&mut out, *start);
        put_u16(&mut out, *end);
        let off = box_index.get(b).and_then(|i| box_at.get(*i)).copied().unwrap_or(0);
        if off >= 1 << 24 {
            return Err(TablesError::TooLarge { what: "the COLR clip list", limit: (1 << 24) - 1 });
        }
        out.extend_from_slice((off as u32).to_be_bytes().get(1..4).unwrap_or_default());
    }
    for b in boxes {
        out.extend_from_slice(b);
    }
    Ok(out)
}

/// The bytes of the delta-set index map at `at`.
fn index_map_bytes(d: Data<'_>, at: usize) -> Result<Vec<u8>> {
    let format = d.u8(at)?;
    let size = usize::from((d.u8(at + 1)? & 0x30) >> 4) + 1;
    let (count, header) = if format == 0 { (usize::from(d.u16(at + 2)?), 4) } else { (d.u32_usize(at + 2)?, 6) };
    Ok(d.slice(at, header + count.saturating_mul(size))?.to_vec())
}

// ── CPAL ──

/// `CPAL` for the subset font: kept whole when the font has an `SVG ` table, pruned to the
/// entries in `used` (in their order) otherwise, and `None` when it has no entries left or no
/// `COLR` table uses it.
pub(crate) fn prune_cpal(output: &SfntFont, bytes: &[u8], used: Option<&PaletteUse>) -> Result<Option<Vec<u8>>> {
    if output.tables.contains_key(b"SVG ") {
        return Ok(Some(bytes.to_vec()));
    }
    let Some(PaletteUse(used)) = used else { return Ok(None) };
    let d = Data::new(*b"CPAL", bytes);
    let version = d.u16(0)?;
    let entries = usize::from(d.u16(2)?);
    let palettes = usize::from(d.u16(4)?);
    let records_at = d.u32_usize(8)?;
    if let Some(bad) = used.iter().find(|i| usize::from(**i) >= entries) {
        return Err(d.bad(format!("has no palette entry {bad}, which the COLR table uses")));
    }
    if used.is_empty() {
        return Ok(None);
    }
    let mut colors = Vec::with_capacity(4 * palettes * used.len());
    for p in 0..palettes {
        let first = usize::from(d.u16(12 + 2 * p)?);
        for i in used {
            colors.extend_from_slice(d.slice(records_at + 4 * (first + usize::from(*i)), 4)?);
        }
    }
    let header = 12 + 2 * palettes + if version >= 1 { 12 } else { 0 };
    let mut out = Vec::new();
    put_u16(&mut out, version);
    put_u16(&mut out, to_u16(used.len(), "the palette entry count")?);
    put_u16(&mut out, to_u16(palettes, "the palette count")?);
    put_u16(&mut out, to_u16(palettes * used.len(), "the color record count")?);
    put_u32(&mut out, to_u32(header, "the CPAL table")?);
    for p in 0..palettes {
        put_u16(&mut out, to_u16(p * used.len(), "the color record index")?);
    }
    if version >= 1 {
        let after = 12 + 2 * palettes;
        let (types_at, labels_at, entry_labels_at) = (d.u32_usize(after)?, d.u32_usize(after + 4)?, d.u32_usize(after + 8)?);
        let mut tail = Vec::new();
        let mut at = header + colors.len();
        let (mut types, mut labels, mut entry_labels) = (0u32, 0u32, 0u32);
        if types_at != 0 {
            types = to_u32(at, "the CPAL table")?;
            tail.extend_from_slice(d.slice(types_at, 4 * palettes)?);
            at = header + colors.len() + tail.len();
        }
        if labels_at != 0 {
            labels = to_u32(at, "the CPAL table")?;
            tail.extend_from_slice(d.slice(labels_at, 2 * palettes)?);
            at = header + colors.len() + tail.len();
        }
        if entry_labels_at != 0 {
            entry_labels = to_u32(at, "the CPAL table")?;
            for i in used {
                put_u16(&mut tail, d.u16(entry_labels_at + 2 * usize::from(*i))?);
            }
        }
        for o in [types, labels, entry_labels] {
            put_u32(&mut out, o);
        }
        out.extend(colors);
        out.extend(tail);
    } else {
        out.extend(colors);
    }
    Ok(Some(out))
}
