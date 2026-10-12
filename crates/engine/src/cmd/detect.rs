//! Detect Composites and Detect Component, on `fontcraft_pathops::matcher` (M13.15). Rows:
//! `font.detectComposites`, `path.detectComponent` (`spec/inventory/90-secondary.toml`).
//!
//! A base glyph is one whose layer (the same layer id as the target's) holds closed paths and no
//! components. Its outline is matched against the target layer's closed, unlocked paths up to
//! translation (and mirroring with `flips`); the matched paths are replaced by a component of
//! the base glyph where the first of them was. Other paths, components, anchors and hints stay;
//! hints on replaced paths go.
//!
//! Detect Composites works across the font without comparing every glyph with every other: base
//! outlines are indexed by the signature of their largest contour, and a target only tries the
//! bases whose contour signatures it holds. Bases are tried from the largest (most nodes) down,
//! and a base must come strictly before its target in the order (nodes, then glyph order), so the
//! components made in one run can never form a cycle. All the work of one run is capped by
//! [`MAX_DETECT_WORK`].

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Component, Font, Layer, Node, NodeKind, Path, Shape, Transform};
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
/// Distance slack for floating-point noise.
const EPSILON: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.detectComposites",
            "Detect Composites",
            ["Font"],
            None,
            "{glyphs?: [names] (default: every glyph), layer?: id or layers?: [ids] (default: every master layer), tolerance?: number (font units, 0 to 10, default 0.5), flips?: bool (default false: translation only)} in each layer, replace every group of closed, unlocked paths that repeats the whole outline of another glyph's layer with the same id (closed paths, no components) by a component of that glyph. Bases are tried from the largest down; a base must have fewer nodes than the glyph, or as many and come earlier in the font. With `flips`, mirrored and half-turned copies are found too. A component's offset is rounded to the font's grid when the match still holds. Locked glyphs are skipped and listed; master layers that end up with different components are named in `warnings`. Past the work limit the run stops, keeps what it found and says so in `stopped` → {glyphs: [names], components: number, skipped: [{glyph, layer?, reason}], warnings: [string], stopped: string | null}",
            has_doc,
            detect_composites
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "path.detectComponent",
            "Detect Component",
            ["Path", "Convert"],
            None,
            "{glyph: name, layer?: id, shapes?: [indices] (`paths` is accepted in place of `shapes`; default: every path of the layer), tolerance?: number (0 to 10, default 0.5), flips?: bool} find the first glyph, in font order, whose layer with the same id has closed paths only and whose whole outline equals the chosen paths after a translation (or a flip with `flips`), and replace the chosen paths by a component of it where the first of them was. A chosen shape that is not a closed, unlocked path, or no such glyph, is an error → {glyph, layer, component: name, transform: [xx, yx, xy, yy, dx, dy], selection: {glyph, layer, shapes}}",
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

/// A glyph whose outline can become a component.
struct Base {
    name: String,
    /// Position in the font's glyph order.
    order: usize,
    contours: Vec<Contour>,
    nodes: usize,
    /// The signature of every contour, sorted.
    sigs: Vec<Signature>,
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

/// Every base outline for one layer id, indexed by the signature of its largest contour.
struct Bases {
    list: Vec<Base>,
    by_key: HashMap<Signature, Vec<usize>>,
}

impl Bases {
    fn new(font: &Font, id: &str) -> Self {
        let mut list = Vec::new();
        let mut by_key: HashMap<Signature, Vec<usize>> = HashMap::new();
        for (order, g) in font.glyphs.iter().enumerate() {
            let Some(layer) = g.layers.iter().find(|l| l.id == id) else { continue };
            let Some((contours, nodes)) = base_outline(layer) else { continue };
            let Some(key) = key_sig(&contours) else { continue };
            by_key.entry(key).or_default().push(list.len());
            list.push(Base { name: g.name.clone(), order, sigs: sorted_sigs(&contours), contours, nodes });
        }
        Self { list, by_key }
    }
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
/// or a matrix.
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
struct Detected {
    layer: Layer,
    components: Vec<String>,
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

/// Finds base outlines in the closed, unlocked paths of `layer` of glyph `name` (at `order`).
fn detect_layer(layer: &Layer, name: &str, order: usize, bases: &Bases, opts: &MatchOptions, grid: f64, work: &mut usize) -> Option<Detected> {
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
    let mut sigs = sorted_sigs(&contours);
    let mut seen = HashSet::new();
    let mut candidates: Vec<&Base> = Vec::new();
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
            if base.name != name && (base.nodes, base.order) < (nodes, order) && sub_multiset(&base.sigs, &sigs) {
                candidates.push(base);
            }
        }
    }
    candidates.sort_by_key(|b| (Reverse(b.nodes), !named_in(&b.name, name), b.order));

    // The contours not claimed yet, and their positions in `contours`.
    let mut alive: Vec<Contour> = contours.clone();
    let mut alive_at: Vec<usize> = (0..contours.len()).collect();
    let mut entries: Vec<(Vec<usize>, Component)> = Vec::new();
    for base in candidates {
        if *work == 0 || alive.is_empty() {
            break;
        }
        if !spend(work, sigs.len()) || !sub_multiset(&base.sigs, &sigs) {
            continue;
        }
        let found = match_outline_within(&base.contours, &alive, opts, work);
        if found.is_empty() {
            continue;
        }
        let mut claimed: HashSet<usize> = HashSet::new();
        for m in &found {
            let affine = rounded(m, &base.contours, &alive, grid, opts.tolerance, work);
            let shapes: Vec<usize> = m.runs.iter().filter_map(|r| alive_at.get(r.contour).and_then(|&c| slots.get(c)).copied()).collect();
            claimed.extend(m.runs.iter().map(|r| r.contour));
            entries.push((shapes, component(&base.name, affine)));
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
    let (layer, _) = replace_paths(layer, &entries);
    let components = entries.into_iter().map(|(_, c)| c.glyph).collect();
    Some(Detected { layer, components })
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
    let mut bases: HashMap<String, Bases> = HashMap::new();
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
                Some(d) => {
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
        let (new_layer, placed) = replace_paths(layer, &[(which, component(&base_name, affine))]);
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
