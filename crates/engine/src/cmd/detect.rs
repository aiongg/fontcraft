//! Detect Composites and Detect Component, on `fontcraft_pathops::matcher` (M13.15). Rows:
//! `font.detectComposites`, `path.detectComponent` (`spec/inventory/90-secondary.toml`).
//!
//! A base glyph is one whose layer (the same layer id as the target's) holds closed paths and no
//! components. Its outline is matched against the target layer's closed, unlocked paths up to
//! translation (and mirroring with `flips`); the matched paths are replaced by a component of
//! the base glyph where the first of them was. Other paths, components, anchors and hints stay;
//! hints on replaced paths go.
//!
//! A new component keeps the default automatic alignment where aligning puts it where the outline
//! was (within the tolerance) and leaves the width as it is; alignment is turned off on the
//! components it would move, and on all new components of a layer whose width it would change.
//!
//! Detect Composites works across the font without comparing every glyph with every other: base
//! outlines are indexed by the signature of their largest contour, and a target only tries the
//! bases whose contour signatures it holds. Bases are tried from the largest (most nodes) down,
//! and a base must come strictly before its target in the order of [`Rank`] (nodes, combining
//! marks first, fewer suffixes first, glyph order), so the components made in one run can never
//! form a cycle.
//!
//! The base choice follows how composites are usually built: of glyphs with the same outline, a
//! combining mark is the base of the others; a glyph detected as a composite of two or more
//! glyphs serves as a base only for a glyph whose whole outline it is, or when it is a
//! combining mark (a stacked mark); of bases with the same outline matching the same paths, the
//! one named in the glyph's name, then a combining mark, then the one that needs the shortest
//! shift is used. Whether a base is a composite is worked out once per run, when a glyph needs
//! it. All the work of one run is capped by [`MAX_DETECT_WORK`].

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

use fontcraft_components::AnchorCache;
use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Alignment, Component, Font, Glyph, Layer, Node, NodeKind, Path, Shape, Transform};
use fontcraft_pathops::matcher::{MAX_PATTERN_NODES, Match, MatchOptions, Signature, match_outline_within, signature};
use fontcraft_pathops::{Contour, Node as PNode, NodeKind as PKind};
use serde_json::{Value, json};

use super::common::{Outcome, grid};
use super::decompose::remap_hint;
use super::*;
use crate::params::{MAX_TARGETS, Params, TargetKind, Targets, kind};

/// The largest `tolerance` accepted, in font units.
pub const MAX_DETECT_TOLERANCE: f64 = 10.0;
/// Node comparisons one Detect Composites or Detect Component run may make; past it the run
/// stops and says so.
pub const MAX_DETECT_WORK: usize = 200_000_000;
/// Alignment passes per layer before the new components that still move are all left unaligned.
const MAX_ALIGN_PASSES: usize = 8;
/// Distance slack for floating-point noise.
const EPSILON: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.detectComposites",
            "Detect Composites",
            ["Font"],
            None,
            "{glyphs?: [names] (default: every glyph), layer?: id or layers?: [ids] (default: every master layer), tolerance?: number (font units, 0 to 10, default 0.5), flips?: bool (default false: translation only)} in each layer, replace every group of closed, unlocked paths that repeats the whole outline of another glyph's layer with the same id (closed paths, no components) by a component of that glyph. Bases are tried from the largest down; a base must have fewer nodes than the glyph, or as many and be a combining mark (`…comb`) where the glyph is not, or have fewer suffixes, or come earlier in the font. A glyph that holds two or more other glyphs' outlines itself is a base only for a glyph whose whole outline it is, unless it is a combining mark. Of bases with the same outline, the one named in the glyph's name goes first, then a combining mark, then the one drawn closest to where the glyph has it. With `flips`, mirrored and half-turned copies are found too. A component's offset is rounded to the font's grid when the match still holds. New components keep automatic alignment where aligning leaves them and the width in place (within the tolerance); it is turned off on the others. Locked glyphs are skipped and listed; master layers that end up with different components are named in `warnings`. Past the work limit the run stops, keeps what it found and says so in `stopped` → {glyphs: [names], components: number, skipped: [{glyph, layer?, reason}], warnings: [string], stopped: string | null}",
            has_doc,
            detect_composites
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "path.detectComponent",
            "Detect Component",
            ["Path", "Convert"],
            None,
            "{glyph: name, layer?: id, shapes?: [indices] (`paths` is accepted in place of `shapes`; default: every path of the layer), tolerance?: number (0 to 10, default 0.5), flips?: bool} find the first glyph, in font order, whose layer with the same id has closed paths only and whose whole outline equals the chosen paths after a translation (or a flip with `flips`), and replace the chosen paths by a component of it where the first of them was. The component keeps automatic alignment unless aligning would move it or change the width. A chosen shape that is not a closed, unlocked path, or no such glyph, is an error → {glyph, layer, component: name, transform: [xx, yx, xy, yy, dx, dy], selection: {glyph, layer, shapes}}",
            has_doc,
            detect_component
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes])
        .context(&["canvas"]),
    ]
}

// ── Outlines ────────────────────────────────────────────────────────────────────────────────────

fn pkind(k: NodeKind) -> PKind {
    match k {
        NodeKind::Line => PKind::Line,
        NodeKind::Curve => PKind::Curve,
        NodeKind::QCurve => PKind::QCurve,
        NodeKind::OffCurve => PKind::OffCurve,
    }
}

fn contour(path: &Path) -> Contour {
    Contour::new(
        path.closed,
        path.nodes.iter().map(|n: &Node| PNode { point: Point::new(n.x, n.y), kind: pkind(n.kind), smooth: n.smooth, tag: None }).collect(),
    )
}

/// Where a glyph stands in the order that keeps one run free of cycles: a base must come
/// strictly before the glyph it is found in. Fewer nodes first; among as many nodes, a
/// combining mark (`…comb`) before other glyphs, a name with fewer suffixes before one with
/// more (`colon` before `colon.tf`), then the font's order. So of two glyphs with the same
/// outline, `acute` becomes a component of `acutecomb` and `colon.tf` of `colon`.
type Rank = (usize, bool, usize, usize);

fn rank(name: &str, nodes: usize, order: usize) -> Rank {
    (nodes, !is_comb(name), name.matches('.').count(), order)
}

/// A combining mark by name: the part before the first suffix ends in `comb`.
fn is_comb(name: &str) -> bool {
    name.split('.').next().unwrap_or(name).ends_with("comb")
}

/// Components a base must hold itself before it stops serving as part of a larger glyph.
const COMPOSITE_PARTS: usize = 2;

/// A glyph whose outline can become a component.
struct Base<'f> {
    name: &'f str,
    /// Position in the font's glyph order.
    order: usize,
    layer: &'f Layer,
    contours: Vec<Contour>,
    nodes: usize,
    /// The signature of every contour, sorted.
    sigs: Vec<Signature>,
    comb: bool,
    rank: Rank,
}

/// The layer of a glyph as a base outline: closed, non-empty paths and no components.
fn base_outline(layer: &Layer) -> Option<(Vec<Contour>, usize)> {
    let mut contours = Vec::new();
    let mut nodes = 0usize;
    for shape in &layer.shapes {
        match shape {
            Shape::Path(p) if p.closed && !p.nodes.is_empty() => {
                nodes = nodes.saturating_add(p.nodes.len());
                contours.push(contour(p));
            }
            Shape::Path(_) | Shape::Component(_) => return None,
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    (!contours.is_empty() && nodes <= MAX_PATTERN_NODES).then_some((contours, nodes))
}

fn sorted_sigs(contours: &[Contour]) -> Vec<Signature> {
    let mut sigs: Vec<Signature> = contours.iter().map(signature).collect();
    sigs.sort();
    sigs
}

/// The signature of the contour the matcher places first: the one with the most nodes.
fn key_sig(contours: &[Contour]) -> Option<Signature> {
    let (_, c) = contours.iter().enumerate().max_by_key(|(i, c)| (c.nodes.len(), Reverse(*i)))?;
    Some(signature(c))
}

/// Every base outline for one layer id, indexed by the signature of its largest contour, with
/// what detection made of each base's own layer once a run needed it.
struct Bases<'f> {
    list: Vec<Base<'f>>,
    by_key: HashMap<Signature, Vec<usize>>,
    by_name: HashMap<&'f str, usize>,
    /// How many components detection finds in each base's own layer, once worked out.
    parts: Vec<Option<usize>>,
    /// What detection made of each base's own layer, once worked out.
    found: Vec<Option<Detected>>,
}

impl<'f> Bases<'f> {
    fn new(font: &'f Font, id: &str) -> Self {
        let mut list = Vec::new();
        let mut by_key: HashMap<Signature, Vec<usize>> = HashMap::new();
        let mut by_name: HashMap<&'f str, usize> = HashMap::new();
        for (order, g) in font.glyphs.iter().enumerate() {
            let Some(layer) = g.layers.iter().find(|l| l.id == id) else { continue };
            let Some((contours, nodes)) = base_outline(layer) else { continue };
            let Some(key) = key_sig(&contours) else { continue };
            by_key.entry(key).or_default().push(list.len());
            by_name.entry(g.name.as_str()).or_insert(list.len());
            let name = g.name.as_str();
            list.push(Base {
                name,
                order,
                layer,
                sigs: sorted_sigs(&contours),
                contours,
                nodes,
                comb: is_comb(name),
                rank: rank(name, nodes, order),
            });
        }
        let (parts, found) = (vec![None; list.len()], (0..list.len()).map(|_| None).collect());
        Self { list, by_key, by_name, parts, found }
    }

    /// Whether base `b` holds at least [`COMPOSITE_PARTS`] components itself, so it does not
    /// stand for part of a glyph with `nodes` nodes. A combining mark stays a base (a stacked
    /// mark such as `circumflexacutecomb` is often made of other marks), and so does a glyph
    /// whose whole outline the base is.
    fn composite_for(&self, b: usize, nodes: usize) -> bool {
        let Some(base) = self.list.get(b) else { return false };
        base.nodes < nodes && !base.comb && self.parts.get(b).copied().flatten().unwrap_or(0) >= COMPOSITE_PARTS
    }

    /// How many components count for a base whose own layer detection made `found`: a base
    /// that is one other glyph's whole outline counts as that glyph does (at least one), so a
    /// copy of a composite is a composite too.
    fn parts_of(&self, found: Option<&Detected>) -> usize {
        let Some(d) = found else { return 0 };
        let whole = !d.layer.shapes.iter().any(|s| matches!(s, Shape::Path(_)));
        match d.components.as_slice() {
            [only] if whole => self.by_name.get(only.as_str()).and_then(|&a| self.parts.get(a).copied().flatten()).unwrap_or(0).max(1),
            list => list.len(),
        }
    }
}

/// Whether a run must know if `base` is a composite before it may use it in a glyph with
/// `nodes` nodes: for [`Bases::composite_for`], and for [`Bases::parts_of`] when the glyph is
/// a copy of the base.
fn needs_parts(base: &Base, nodes: usize) -> bool {
    base.nodes <= nodes && !base.comb
}

/// Whether every signature of `small` (sorted) is in `big` (sorted), counting repeats.
fn sub_multiset(small: &[Signature], big: &[Signature]) -> bool {
    let mut it = big.iter();
    small.iter().all(|s| it.by_ref().any(|b| b == s))
}

/// The base glyph's name without suffixes and `comb`, for preferring `acutecomb` in `Aacute`.
fn named_in(base: &str, target: &str) -> bool {
    let stem = base.split('.').next().unwrap_or(base);
    let stem = stem.strip_suffix("comb").unwrap_or(stem);
    !stem.is_empty() && target.split('.').next().unwrap_or(target).contains(stem)
}

// ── Placing components ──────────────────────────────────────────────────────────────────────────

/// The match's transform with its offset rounded to `grid` when every pattern node then still
/// has a node of its target contour within the tolerance.
fn rounded(m: &Match, pattern: &[Contour], target: &[Contour], grid: f64, tol: f64, work: &mut usize) -> Affine {
    if grid <= 0.0 || !grid.is_finite() {
        return m.transform;
    }
    let [a, b, c, d, e, f] = m.transform.as_coeffs();
    let r = Affine::new([a, b, c, d, (e / grid).round() * grid, (f / grid).round() * grid]);
    if r == m.transform {
        return m.transform;
    }
    for (pc, run) in pattern.iter().zip(&m.runs) {
        let Some(tc) = target.get(run.contour) else { return m.transform };
        let cost = pc.nodes.len().saturating_mul(tc.nodes.len());
        if *work < cost {
            return m.transform;
        }
        *work -= cost;
        let holds = pc.nodes.iter().all(|n| {
            let q = r * n.point;
            tc.nodes.iter().any(|t| (t.point - q).hypot() <= tol + EPSILON)
        });
        if !holds {
            return m.transform;
        }
    }
    r
}

/// A component of `glyph` placed by `affine`: a plain offset, an offset with a mirroring scale,
/// or a matrix, with the default automatic alignment ([`settle_alignment`] decides).
fn component(glyph: &str, affine: Affine) -> Component {
    let [a, b, c, d, e, f] = affine.as_coeffs();
    let mut out = Component::new(glyph, e, f);
    if (a, b, c, d) != (1.0, 0.0, 0.0, 1.0) {
        out.transform = if b == 0.0 && c == 0.0 && a.abs() == 1.0 && d.abs() == 1.0 {
            Transform::Decomposed { pos: (e, f), scale: (a, d), angle: 0.0, slant: (0.0, 0.0) }
        } else {
            Transform::Matrix([a, b, c, d, e, f])
        };
    }
    out
}

/// Turns automatic alignment off on the components at `created` (shape indices of `layer`) that
/// alignment would move by more than `tol`, and on all of them when it would change the width by
/// more than `tol` or cannot place them. A component turned off can change where the others go,
/// so the check repeats until nothing moves.
fn settle_alignment(font: &Font, glyph: &Glyph, layer: &mut Layer, created: &[usize], tol: f64, cache: &mut AnchorCache) {
    let disable = |layer: &mut Layer, i: usize| {
        if let Some(Shape::Component(c)) = layer.shapes.get_mut(i) {
            c.info.alignment = Alignment::Disabled;
        }
    };
    let offset = |shapes: &[Shape], i: usize| match shapes.get(i) {
        Some(Shape::Component(c)) => {
            let [.., e, f] = c.affine().as_coeffs();
            Some(Point::new(e, f))
        }
        _ => None,
    };
    for _ in 0..MAX_ALIGN_PASSES {
        let aligned = match fontcraft_components::align::align_layer_cached(font, glyph, layer, cache) {
            Ok(Some(aligned)) => aligned,
            Ok(None) => return,
            Err(_) => break,
        };
        let moved: Vec<usize> = created
            .iter()
            .copied()
            .filter(|&i| {
                let on = matches!(layer.shapes.get(i), Some(Shape::Component(c)) if c.info.alignment != Alignment::Disabled);
                on && offset(&layer.shapes, i).zip(offset(&aligned.shapes, i)).is_some_and(|(was, now)| (now - was).hypot() > tol + EPSILON)
            })
            .collect();
        if moved.is_empty() {
            if (aligned.width - layer.width).abs() <= tol + EPSILON {
                return;
            }
            break;
        }
        for i in moved {
            disable(layer, i);
        }
    }
    for &i in created {
        disable(layer, i);
    }
}

/// `layer` with the paths of each entry replaced by its component, which takes the place of the
/// first of them, and its hints renumbered (hints on replaced paths are dropped). Also returns
/// the new index of each component, in entry order.
fn replace_paths(layer: &Layer, entries: &[(Vec<usize>, Component)]) -> (Layer, Vec<usize>) {
    let mut at_first: BTreeMap<usize, usize> = BTreeMap::new();
    let mut gone: HashSet<usize> = HashSet::new();
    for (e, (shapes, _)) in entries.iter().enumerate() {
        if let Some(first) = shapes.iter().min() {
            at_first.insert(*first, e);
        }
        gone.extend(shapes.iter().copied());
    }
    let mut shapes = Vec::with_capacity(layer.shapes.len());
    let mut map: Vec<Option<usize>> = Vec::with_capacity(layer.shapes.len());
    let mut placed = vec![0usize; entries.len()];
    for (i, shape) in layer.shapes.iter().enumerate() {
        if let Some(&e) = at_first.get(&i)
            && let Some((_, c)) = entries.get(e)
        {
            if let Some(slot) = placed.get_mut(e) {
                *slot = shapes.len();
            }
            shapes.push(Shape::Component(c.clone()));
        }
        if gone.contains(&i) {
            map.push(None);
        } else {
            map.push(Some(shapes.len()));
            shapes.push(shape.clone());
        }
    }
    let mut out = layer.clone();
    out.shapes = shapes;
    if !layer.info.hints.is_empty() {
        let map = |i: usize| map.get(i).map_or(Some(i), |m| *m);
        out.info.hints = layer.info.hints.iter().filter_map(|h| remap_hint(h, &map)).collect();
    }
    (out, placed)
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

fn match_options(p: &Params) -> Result<MatchOptions> {
    let tolerance = p.opt_f64("tolerance")?.unwrap_or(0.5);
    if !(0.0..=MAX_DETECT_TOLERANCE).contains(&tolerance) {
        return Err(p.bad(format!("`tolerance` must be from 0 to {MAX_DETECT_TOLERANCE}, got {tolerance}")));
    }
    Ok(MatchOptions { tolerance, flips: p.bool_or("flips", false)?, ..MatchOptions::default() })
}

/// A list of shape indices under `key`, repeats dropped; `None` when absent.
fn indices(p: &Params, key: &str) -> Result<Option<Vec<usize>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_TARGETS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
    }
    let mut out: Vec<usize> = Vec::with_capacity(list.len());
    for v in list {
        let i = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| p.bad(format!("`{key}` must hold indices (whole numbers from 0), found {} ({v})", kind(v))))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(Some(out))
}

/// `layers`, or `layer`, or every master's id.
fn layer_ids(p: &Params, font: &Font) -> Result<Vec<String>> {
    match (p.opt_str("layer")?, p.opt_strings("layers")?) {
        (Some(_), Some(_)) => Err(p.bad("give `layer` or `layers`, not both")),
        (Some(id), None) => Ok(vec![id.to_string()]),
        (None, Some(ids)) => {
            if ids.len() > MAX_TARGETS {
                return Err(p.bad(format!("`layers` holds {} ids; at most {MAX_TARGETS} are allowed", ids.len())));
            }
            let mut out: Vec<String> = Vec::new();
            for id in ids {
                if !out.iter().any(|o| o == id) {
                    out.push(id.to_string());
                }
            }
            if out.is_empty() {
                return Err(p.bad("`layers` is empty"));
            }
            Ok(out)
        }
        (None, None) => Ok(font.masters.iter().map(|m| m.id.clone()).collect()),
    }
}

// ── Detect Composites ───────────────────────────────────────────────────────────────────────────

/// What detection made of one target layer.
#[derive(Clone)]
struct Detected {
    layer: Layer,
    components: Vec<String>,
    /// Shape indices of the new components in `layer`.
    created: Vec<usize>,
}

/// Takes `units` from `work`; when it holds fewer, empties it and returns false.
fn spend(work: &mut usize, units: usize) -> bool {
    match work.checked_sub(units) {
        Some(left) => {
            *work = left;
            true
        }
        None => {
            *work = 0;
            false
        }
    }
}

/// The closed, unlocked paths of a target layer and the bases that may stand in them.
struct Target {
    /// Shape index of each contour.
    slots: Vec<usize>,
    contours: Vec<Contour>,
    nodes: usize,
    /// Indices into [`Bases::list`].
    candidates: Vec<usize>,
}

/// The paths of `layer` of glyph `name` (at `order`) to look in, and the bases whose contour
/// signatures they hold that come before the glyph ([`Rank`]); `None` when there is nothing
/// to look in or the work ran out.
fn target(layer: &Layer, name: &str, order: usize, bases: &Bases, work: &mut usize) -> Option<Target> {
    let mut slots = Vec::new();
    let mut contours = Vec::new();
    for (i, shape) in layer.shapes.iter().enumerate() {
        if let Shape::Path(p) = shape
            && p.closed
            && !p.nodes.is_empty()
            && !p.info.locked
        {
            slots.push(i);
            contours.push(contour(p));
        }
    }
    if contours.is_empty() {
        return None;
    }
    let nodes: usize = contours.iter().map(|c| c.nodes.len()).sum();
    let own = rank(name, nodes, order);
    let sigs = sorted_sigs(&contours);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for sig in &sigs {
        if !seen.insert(*sig) {
            continue;
        }
        for &b in bases.by_key.get(sig).map_or(&[][..], Vec::as_slice) {
            // Glyphs sharing a key signature are each looked at once per layer: charged, so many
            // of them in many layers reach the work limit instead of running on.
            if !spend(work, 1usize.saturating_add(sigs.len())) {
                return None;
            }
            let Some(base) = bases.list.get(b) else { continue };
            if base.name != name && base.rank < own && sub_multiset(&base.sigs, &sigs) {
                candidates.push(b);
            }
        }
    }
    Some(Target { slots, contours, nodes, candidates })
}

/// The candidates of `t` whose composite status is not known yet but needed.
fn unknown_parts(t: &Target, bases: &Bases) -> Vec<usize> {
    t.candidates
        .iter()
        .copied()
        .filter(|&b| bases.list.get(b).is_some_and(|base| needs_parts(base, t.nodes)) && bases.parts.get(b).is_some_and(Option::is_none))
        .collect()
}

/// Works out what detection makes of the layers of the bases in `roots`, and of the bases they
/// need first, without recursion: a base is detected once every candidate whose composite
/// status it needs is known, and candidates always rank lower, so this ends. Past the work
/// limit the remaining bases count as having no components.
fn work_out(bases: &mut Bases, roots: Vec<usize>, opts: &MatchOptions, grid: f64, work: &mut usize) {
    let mut stack = roots;
    let mut pending: HashMap<usize, Target> = HashMap::new();
    while let Some(&b) = stack.last() {
        if bases.parts.get(b).is_none_or(Option::is_some) {
            stack.pop();
            continue;
        }
        let Some((layer, name, order)) = bases.list.get(b).map(|base| (base.layer, base.name, base.order)) else {
            stack.pop();
            continue;
        };
        let t = match pending.remove(&b) {
            Some(t) => Some(t),
            None if *work > 0 => target(layer, name, order, bases, work),
            None => None,
        };
        let Some(t) = t else {
            if let Some(slot) = bases.parts.get_mut(b) {
                *slot = Some(0);
            }
            stack.pop();
            continue;
        };
        let unknown = unknown_parts(&t, bases);
        if unknown.is_empty() || !spend(work, unknown.len()) {
            let found = if *work > 0 { finish(layer, name, t, bases, opts, grid, work) } else { None };
            let count = bases.parts_of(found.as_ref());
            if let Some(slot) = bases.parts.get_mut(b) {
                *slot = Some(count);
            }
            if let Some(slot) = bases.found.get_mut(b) {
                *slot = found;
            }
            stack.pop();
        } else {
            pending.insert(b, t);
            stack.extend(unknown);
        }
    }
}

/// See [`preference`].
type Pref = (bool, bool, f64, Rank);

/// How well a base that matched fits the glyph, among bases with the same outline; smaller is
/// better. Named in the glyph's name, then a combining mark, then the shorter shift (the copy
/// drawn where the glyph uses it, such as `two.numr` in a fraction), then [`Rank`].
fn preference(base: &Base, target: &str, m: &Match) -> Pref {
    let [.., e, f] = m.transform.as_coeffs();
    (!named_in(base.name, target), !base.comb, e.hypot(f).round(), base.rank)
}

fn better(a: &Pref, b: &Pref) -> bool {
    a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)).then(a.3.cmp(&b.3)).is_lt()
}

/// The target contours a match covers.
fn covered(m: &Match) -> Vec<usize> {
    let mut out: Vec<usize> = m.runs.iter().map(|r| r.contour).collect();
    out.sort_unstable();
    out
}

/// Finds base outlines in the paths of `t` (from `layer` of glyph `name`): bases from the
/// largest down; a base that is itself a composite of several glyphs is left out where it
/// would cover only part of the glyph ([`Bases::composite_for`]); of the bases with the same
/// outline that match the same paths, the one [`preference`] puts first is used.
fn finish(layer: &Layer, name: &str, t: Target, bases: &Bases, opts: &MatchOptions, grid: f64, work: &mut usize) -> Option<Detected> {
    let Target { slots, contours, nodes, candidates } = t;
    let mut list: Vec<&Base> = candidates.iter().filter(|&&b| !bases.composite_for(b, nodes)).filter_map(|&b| bases.list.get(b)).collect();
    list.sort_by_key(|b| (Reverse(b.nodes), !named_in(b.name, name), !b.comb, b.rank));
    let mut tried = vec![false; list.len()];

    // The contours not claimed yet, and their positions in `contours`.
    let mut sigs = sorted_sigs(&contours);
    let mut alive: Vec<Contour> = contours.clone();
    let mut alive_at: Vec<usize> = (0..contours.len()).collect();
    let mut entries: Vec<(Vec<usize>, Component)> = Vec::new();
    for k in 0..list.len() {
        if *work == 0 || alive.is_empty() {
            break;
        }
        let (Some(&base), Some(false)) = (list.get(k), tried.get(k).copied()) else { continue };
        if !spend(work, sigs.len()) || !sub_multiset(&base.sigs, &sigs) {
            continue;
        }
        let found = match_outline_within(&base.contours, &alive, opts, work);
        let Some(first) = found.first() else { continue };
        let paths = covered(first);
        let mut best = (base, preference(base, name, first), found.clone());
        for j in k.saturating_add(1)..list.len() {
            let (Some(&twin), Some(false)) = (list.get(j), tried.get(j).copied()) else { continue };
            if twin.sigs != base.sigs {
                continue;
            }
            if !spend(work, 1) {
                break;
            }
            let theirs = match_outline_within(&twin.contours, &alive, opts, work);
            let Some(m) = theirs.first().filter(|m| covered(m) == paths) else { continue };
            if let Some(slot) = tried.get_mut(j) {
                *slot = true;
            }
            let pref = preference(twin, name, m);
            if better(&pref, &best.1) {
                best = (twin, pref, theirs);
            }
        }
        let (base, _, found) = best;
        let mut claimed: HashSet<usize> = HashSet::new();
        for m in &found {
            let affine = rounded(m, &base.contours, &alive, grid, opts.tolerance, work);
            let shapes: Vec<usize> = m.runs.iter().filter_map(|r| alive_at.get(r.contour).and_then(|&c| slots.get(c)).copied()).collect();
            claimed.extend(m.runs.iter().map(|r| r.contour));
            entries.push((shapes, component(base.name, affine)));
        }
        spend(work, alive.len());
        let (mut keep, mut keep_at) = (Vec::new(), Vec::new());
        for (i, (c, at)) in alive.into_iter().zip(alive_at).enumerate() {
            if !claimed.contains(&i) {
                keep.push(c);
                keep_at.push(at);
            }
        }
        (alive, alive_at) = (keep, keep_at);
        sigs = sorted_sigs(&alive);
    }
    if entries.is_empty() {
        return None;
    }
    let (layer, created) = replace_paths(layer, &entries);
    let components = entries.into_iter().map(|(_, c)| c.glyph).collect();
    Some(Detected { layer, components, created })
}

/// What detection makes of `layer` of glyph `name` (at `order`). A base glyph's layer is worked
/// out once per run and kept, since other glyphs may need to know whether it is a composite.
fn detect_layer(layer: &Layer, name: &str, order: usize, bases: &mut Bases, opts: &MatchOptions, grid: f64, work: &mut usize) -> Option<Detected> {
    if let Some(&b) = bases.by_name.get(name)
        && bases.list.get(b).is_some_and(|base| std::ptr::eq(base.layer, layer))
    {
        work_out(bases, vec![b], opts, grid, work);
        return bases.found.get(b).cloned().flatten();
    }
    let t = target(layer, name, order, bases, work)?;
    let unknown = unknown_parts(&t, bases);
    work_out(bases, unknown, opts, grid, work);
    if *work == 0 {
        return None;
    }
    finish(layer, name, t, bases, opts, grid, work)
}

fn detect_composites(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("font.detectComposites", value)?;
    let opts = match_options(&p)?;
    let font = s.font()?;
    let glyphs = match p.opt_glyphs("glyphs", font)? {
        Some(list) => list,
        None => font.glyphs.iter().map(|g| g.name.clone()).collect(),
    };
    let ids = layer_ids(&p, font)?;
    let grid = grid(font);
    let mut out = Outcome::default();
    let mut warnings: Vec<String> = Vec::new();
    let mut changes: Vec<(String, Layer)> = Vec::new();
    let mut count = 0usize;
    let mut stopped: Option<String> = None;
    let mut work = MAX_DETECT_WORK;
    let mut bases: HashMap<String, Bases<'_>> = HashMap::new();
    let mut anchors = AnchorCache::new();
    for name in &glyphs {
        if work == 0 {
            stopped = Some(format!("the work limit was reached before glyph `{name}`; the glyphs from there on were not searched"));
            break;
        }
        let (Some(order), Some(glyph)) = (font.glyphs.position(name), font.glyph(name)) else { continue };
        if glyph.info.locked {
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        let mut per_layer: Vec<Vec<String>> = Vec::new();
        let mut changed = false;
        for id in &ids {
            let Some(layer) = glyph.layers.iter().find(|l| l.id == *id) else { continue };
            let base_list = bases.entry(id.clone()).or_insert_with(|| Bases::new(font, id));
            match detect_layer(layer, name, order, base_list, &opts, grid, &mut work) {
                Some(mut d) => {
                    settle_alignment(font, glyph, &mut d.layer, &d.created, opts.tolerance, &mut anchors);
                    count += d.components.len();
                    per_layer.push(d.components);
                    changes.push((name.clone(), d.layer));
                    changed = true;
                }
                None => per_layer.push(Vec::new()),
            }
        }
        if changed {
            out.glyphs.push(name.clone());
            if per_layer.windows(2).any(|w| w.first() != w.get(1)) {
                warnings.push(format!("glyph `{name}`: its layers got different components, so its masters may no longer be compatible"));
            }
        }
    }
    if work == 0 && stopped.is_none() {
        stopped = Some("the work limit was reached; the last glyphs searched may hold more matches".to_string());
    }
    if !changes.is_empty() {
        s.edit("Detect Composites", |font| {
            for (name, layer) in changes {
                let mut g = font.glyph_mut(&name)?;
                if let Some(slot) = g.layers.iter_mut().find(|l| l.id == layer.id) {
                    *slot = layer;
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({"glyphs": out.glyphs, "components": count, "skipped": out.skipped, "warnings": warnings, "stopped": stopped}))
}

// ── Detect Component ────────────────────────────────────────────────────────────────────────────

fn detect_component(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "path.detectComponent";
    let p = Params::new(ID, value)?;
    let opts = MatchOptions { max_matches: 1, ..match_options(&p)? };
    let chosen = match (indices(&p, "shapes")?, indices(&p, "paths")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `shapes` or `paths`, not both")),
        (a, b) => a.or(b),
    };
    let mut t = Targets::new(ID, p.str("glyph")?, p.opt_str("layer")?.map(str::to_string));
    let (new_layer, base_name, affine) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        crate::check_unlocked(r.glyph)?;
        let layer = r.layer;
        let which: Vec<usize> = match chosen {
            Some(list) => list,
            None => layer.shapes.iter().enumerate().filter(|(_, sh)| matches!(sh, Shape::Path(_))).map(|(i, _)| i).collect(),
        };
        if which.is_empty() {
            return Err(p.bad("the layer has no paths"));
        }
        let mut contours = Vec::with_capacity(which.len());
        for &i in &which {
            match layer.shapes.get(i) {
                Some(Shape::Path(path)) if path.info.locked => return Err(p.bad(format!("path {i} is locked"))),
                Some(Shape::Path(path)) if path.closed && !path.nodes.is_empty() => contours.push(contour(path)),
                Some(Shape::Path(_)) => return Err(p.bad(format!("path {i} is open or empty; only closed paths make a component"))),
                Some(_) => return Err(p.bad(format!("shape {i} is not a path"))),
                None => return Err(p.bad(format!("shape {i} does not resolve: the layer has {} shapes", layer.shapes.len()))),
            }
        }
        let sigs = sorted_sigs(&contours);
        let grid = grid(font);
        let mut work = MAX_DETECT_WORK;
        let mut found: Option<(String, Affine)> = None;
        for g in font.glyphs.iter() {
            if g.name == t.glyph || work == 0 {
                continue;
            }
            let Some(base_layer) = g.layers.iter().find(|l| l.id == layer.id) else { continue };
            let Some((base, _)) = base_outline(base_layer) else { continue };
            if sorted_sigs(&base) != sigs {
                continue;
            }
            if let Some(m) = match_outline_within(&base, &contours, &opts, &mut work).first() {
                found = Some((g.name.clone(), rounded(m, &base, &contours, grid, opts.tolerance, &mut work)));
                break;
            }
        }
        let Some((base_name, affine)) = found else {
            let why = if work == 0 { "the search reached its work limit before finding" } else { "there is" };
            return Err(p.bad(format!("{why} no glyph whose whole outline equals the chosen paths")));
        };
        let (mut new_layer, placed) = replace_paths(layer, &[(which, component(&base_name, affine))]);
        settle_alignment(font, r.glyph, &mut new_layer, &placed, opts.tolerance, &mut AnchorCache::new());
        t.layer = Some(layer.id.clone());
        t.shapes = placed;
        (new_layer, base_name, affine)
    };
    s.edit("Detect Component", |font| {
        t.edit_layer(font, |l| {
            *l = new_layer;
            Ok(())
        })
    })?;
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "component": base_name, "transform": affine.as_coeffs(), "selection": t.to_json()}))
}
