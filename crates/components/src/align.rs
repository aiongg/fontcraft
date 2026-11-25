//! Automatic alignment (M6.2): which components of a layer are placed automatically, and where.
//!
//! Rows: `components.autoAlign*`, `components.pathOnLayerDisablesAutoAlign`,
//! `components.underscoreAnchorSnapsToPlainAnchor`, `components.autoAlignBaseThenMarks`,
//! `components.marksStackInOrder`, `components.alternativeAnchorSuffix`,
//! `components.multipleBasesSetSideBySide`, `components.marksBelongToPrecedingBase`,
//! `components.exitEntryAnchorsConnectBases`, `components.singleNumberComponentShiftsVertically`,
//! `components.fontWideDisableAutoAlign`, `fontInfo.disableAutomaticAlignment`.
//!
//! Contract (`docs/plans/M6.md` § 3 › Design decisions):
//! - A component is aligned when the font's Disable Automatic Alignment setting is off and its
//!   stored alignment is `Forced`, or `Default` on a layer that holds only components;
//!   `Horizontal` aligns it horizontally only; `Disabled` never. The single Number-category
//!   component behaves as `Horizontal` (a guess).
//! - Aligned positions are stored in the font. [`align_layer`] computes them and returns the
//!   layer with the new transforms and width written; it reads the targets from
//!   [`crate::propagate`]. The engine runs it through [`crate::metrics::realign`] after every
//!   font-changing command; opening a file does not align.
//! - Placement only moves components: scale, rotation, flip and slant are kept.
//! - Pure functions; a failure changes nothing.
//!
//! How a layer is laid out, component by component in shape order:
//! - A component is a *mark* when one of its underscore anchors (`_top`) finds its plain
//!   counterpart (`top`) among the anchors of the current *cluster*: the nearest base before it
//!   and the marks attached since. Its underscore anchor is put on that anchor. Its own plain
//!   anchors then replace those of the same name in the cluster, so marks stack. The component's
//!   `anchor` attribute picks an alternative target (`top_viet`), or the base of a ligature
//!   (`top_2`); a name that no longer exists falls back to the plain one.
//! - Every other component is a *base* and starts a new cluster. The first base sits at x = 0;
//!   each later one follows the advance of the bases before it, or, when the previous cluster has
//!   a `#exit` anchor and this base has `#entry`, is put so the two coincide. A base flipped
//!   horizontally is shifted by its advance so it stays inside its box. A base keeps its stored
//!   vertical position when it is flipped vertically, and sits on y = 0 otherwise.
//! - Anchors are read with the component's flips applied: a vertical flip swaps names containing
//!   `top` and `bottom`, a horizontal one `left`/`right` and `entry`/`exit`.
//! - Components that are not aligned keep their stored transform but still take part: their
//!   anchors serve later marks, and a base among them moves the pen to where it stands.
//! - The width is the pen after the last base, plus the `=+N` / `=-N` offsets of the layer's
//!   metrics keys (the left offset also shifts every component). It is written only on layers
//!   without paths; forced components on a layer with paths are placed, and the width stays.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_geom::Point;
use fontcraft_model::{Alignment, Component, Font, Glyph, Layer, MAX_COORDINATE, Shape, Transform};

use crate::propagate::{AnchorCache, EffectiveAnchor, base_layer as base_layer_of};
use crate::{ComponentsError, MAX_ALIGN_WORK, MAX_LAYER_COMPONENTS};

/// Most anchor targets kept while one layer is aligned, over all its clusters together.
pub(crate) const MAX_ALIGN_TARGETS: usize = 65_536;

/// A computed position this close to the stored one keeps the stored value, so float noise never
/// counts as a change.
const SAME: f64 = 1e-6;

/// How a component is placed automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignMode {
    /// Both directions.
    Full,
    /// Horizontally only; the vertical position is the designer's.
    Horizontal,
}

/// The automatic placement of one component.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Index into `Layer::shapes`.
    pub shape: usize,
    pub mode: AlignMode,
    /// The component's transform with the aligned position.
    pub transform: Transform,
}

/// How the component at shape index `shape` of `layer` is aligned; `None` when it is not
/// aligned or `shape` is not a component.
pub fn component_alignment(font: &Font, layer: &Layer, shape: usize) -> Option<AlignMode> {
    if alignment_off(font) {
        return None;
    }
    let Some(Shape::Component(c)) = layer.shapes.get(shape) else { return None };
    mode_of(font, &LayerFacts::of(layer), c)
}

/// The aligned components of `layer` as (shape index, mode), in shape order.
pub fn aligned_components(font: &Font, layer: &Layer) -> Vec<(usize, AlignMode)> {
    if alignment_off(font) {
        return Vec::new();
    }
    let facts = LayerFacts::of(layer);
    layer
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Shape::Component(c) => mode_of(font, &facts, c).map(|m| (i, m)),
            _ => None,
        })
        .collect()
}

/// Whether any component of `layer` is aligned. Commands that move components or set a width
/// refuse on such layers (except the vertical part of `Horizontal`).
pub fn is_aligned(font: &Font, layer: &Layer) -> bool {
    !aligned_components(font, layer).is_empty()
}

/// The aligned placement of the components of `glyph`'s layer `layer`, in shape order; empty
/// when none is aligned.
pub fn aligned_placement(font: &Font, glyph: &str, layer: &str) -> Result<Vec<Placement>, ComponentsError> {
    let (g, l) = find_layer(font, glyph, layer)?;
    let Some(plan) = plan_layer(font, g, l, &mut AnchorCache::new())? else { return Ok(Vec::new()) };
    Ok(plan.parts.into_iter().filter_map(|p| p.mode.map(|mode| Placement { shape: p.shape, mode, transform: p.transform })).collect())
}

/// A component attached through its anchors to a glyph before it on the layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attachment {
    /// Index into `Layer::shapes`.
    pub shape: usize,
    /// Where the component is.
    pub stored: Point,
    /// Where its anchor pair puts it.
    pub wanted: Point,
    /// The component is aligned horizontally only.
    pub horizontal: bool,
}

/// What automatic alignment says about a layer as it is stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerAlignment {
    /// The components attached through anchors that automatic alignment applies to: the aligned
    /// ones, and on a layer of components only also those whose alignment is turned off.
    pub attachments: Vec<Attachment>,
    /// The components aligned now, in shape order.
    pub aligned: Vec<usize>,
    /// The width the aligned components give the layer; `None` when it is not derived (none is
    /// aligned, the layer has paths, or a component's glyph is missing).
    pub width: Option<f64>,
}

/// Where automatic alignment puts what is on `layer`, without changing it. Components whose
/// alignment is off stand where they are, as [`align_layer`] leaves them. Empty when the font
/// turns automatic alignment off.
pub fn layer_alignment(font: &Font, glyph: &Glyph, layer: &Layer, cache: &mut AnchorCache) -> Result<LayerAlignment, ComponentsError> {
    let Some(plan) = layout(font, glyph, layer, cache, true)? else { return Ok(LayerAlignment::default()) };
    let has_paths = LayerFacts::of(layer).has_paths;
    let mut out = LayerAlignment::default();
    for p in &plan.parts {
        let Some(Shape::Component(c)) = layer.shapes.get(p.shape) else { continue };
        if p.mode.is_some() {
            out.aligned.push(p.shape);
        }
        let applies = p.mode.is_some() || (!has_paths && c.info.alignment == Alignment::Disabled);
        if let Some(wanted) = p.attached.filter(|_| applies) {
            let [.., x, y] = c.affine().as_coeffs();
            let horizontal = p.mode == Some(AlignMode::Horizontal);
            let wanted = if horizontal { Point::new(wanted.x, y) } else { wanted };
            out.attachments.push(Attachment { shape: p.shape, stored: Point::new(x, y), wanted, horizontal });
        }
    }
    if !out.aligned.is_empty() && plan.parts.iter().all(|p| p.source.is_some()) {
        out.width = plan.width;
    }
    Ok(out)
}

/// `glyph`'s layer `layer` with its aligned components placed and its width and sidebearings
/// derived ([`crate::metrics::derived_metrics`]); `None` when nothing would change.
pub fn align_layer(font: &Font, glyph: &str, layer: &str) -> Result<Option<Layer>, ComponentsError> {
    let (g, l) = find_layer(font, glyph, layer)?;
    align_layer_cached(font, g, l, &mut AnchorCache::new())
}

/// [`align_layer`] for a glyph and layer in hand, reading anchors through `cache`. The layer need
/// not be in `font` yet: only its components' glyphs are read from there.
pub fn align_layer_cached(font: &Font, glyph: &Glyph, layer: &Layer, cache: &mut AnchorCache) -> Result<Option<Layer>, ComponentsError> {
    let Some(plan) = plan_layer(font, glyph, layer, cache)? else { return Ok(None) };
    let moves = plan.parts.iter().any(|p| p.mode.is_some() && layer.shapes.get(p.shape).is_some_and(|s| !same_transform(s, &p.transform)));
    let resizes = plan.width.is_some_and(|w| w != layer.width);
    if !moves && !resizes {
        return Ok(None);
    }
    let mut out = layer.clone();
    for p in plan.parts.iter().filter(|p| p.mode.is_some()) {
        if let Some(Shape::Component(c)) = out.shapes.get_mut(p.shape) {
            c.transform = p.transform;
        }
    }
    if let Some(w) = plan.width {
        let before = out.width;
        out.width = w;
        fontcraft_model::guide_follow::follow_width(&mut out, before, fontcraft_model::guide_follow::BothGuides::of(font));
    }
    Ok(Some(out))
}

fn same_transform(shape: &Shape, t: &Transform) -> bool {
    matches!(shape, Shape::Component(c) if c.transform == *t)
}

pub(crate) fn find_layer<'a>(font: &'a Font, glyph: &str, layer: &str) -> Result<(&'a Glyph, &'a Layer), ComponentsError> {
    let g = font.glyph(glyph).ok_or_else(|| ComponentsError::MissingGlyph(glyph.to_string()))?;
    let l = g
        .layers
        .iter()
        .find(|l| l.id == layer)
        .ok_or_else(|| ComponentsError::MissingLayer { glyph: glyph.to_string(), layer: layer.to_string() })?;
    Ok((g, l))
}

/// The font-wide Disable Automatic Alignment setting is on.
pub(crate) fn alignment_off(font: &Font) -> bool {
    font.info.settings.disables_automatic_alignment == Some(true)
}

/// What decides a `Default` component's alignment.
struct LayerFacts {
    /// A path, or a drawn shape the model keeps raw, is on the layer.
    has_paths: bool,
    components: usize,
}

impl LayerFacts {
    fn of(layer: &Layer) -> Self {
        let has_paths = layer.shapes.iter().any(|s| matches!(s, Shape::Path(_) | Shape::Raw(_)));
        let components = layer.shapes.iter().filter(|s| matches!(s, Shape::Component(_))).count();
        Self { has_paths, components }
    }
}

fn mode_of(font: &Font, facts: &LayerFacts, c: &Component) -> Option<AlignMode> {
    match c.info.alignment {
        Alignment::Disabled => None,
        Alignment::Forced => Some(AlignMode::Full),
        Alignment::Horizontal => Some(AlignMode::Horizontal),
        Alignment::Default if facts.has_paths => None,
        Alignment::Default if facts.components == 1 && is_number(font, &c.glyph) => Some(AlignMode::Horizontal),
        Alignment::Default => Some(AlignMode::Full),
    }
}

/// Whether the glyph called `name` has the category Number: its stored category, else the
/// built-in glyph data's.
fn is_number(font: &Font, name: &str) -> bool {
    let Some(g) = font.glyph(name) else { return false };
    if let Some(category) = g.info.category.as_deref() {
        return category == "Number";
    }
    fontcraft_model::glyphdata::GlyphData::new()
        .ok()
        .and_then(|data| data.lookup(name, &g.unicodes).category)
        .is_some_and(|c| c.value == fontcraft_model::glyphdata::Category::Number)
}

/// One component as the layout leaves it.
pub(crate) struct Planned {
    pub(crate) shape: usize,
    /// `None`: not aligned, the stored transform is kept.
    pub(crate) mode: Option<AlignMode>,
    pub(crate) transform: Transform,
    /// Placed as a base (not attached through an anchor).
    pub(crate) base: bool,
    /// Where the anchor pair of an attached component puts it, aligned or not.
    attached: Option<Point>,
    /// The base glyph and the id of its layer that was read; `None` when the glyph is missing.
    pub(crate) source: Option<(String, String)>,
}

/// The layout of one layer.
pub(crate) struct LayerPlan {
    pub(crate) parts: Vec<Planned>,
    /// The derived width, with the offsets; `None` when the layer's width is not derived.
    pub(crate) width: Option<f64>,
}

/// Anchors of one cluster in the composite's coordinates, by name.
type Cluster = HashMap<String, Point>;

/// The layout of `layer`; `None` when no component of it is aligned.
pub(crate) fn plan_layer(font: &Font, glyph: &Glyph, layer: &Layer, cache: &mut AnchorCache) -> Result<Option<LayerPlan>, ComponentsError> {
    layout(font, glyph, layer, cache, false)
}

/// [`plan_layer`]; with `always` also for a layer none of whose components is aligned.
fn layout(font: &Font, glyph: &Glyph, layer: &Layer, cache: &mut AnchorCache, always: bool) -> Result<Option<LayerPlan>, ComponentsError> {
    if alignment_off(font) {
        return Ok(None);
    }
    let facts = LayerFacts::of(layer);
    if facts.components > MAX_LAYER_COMPONENTS {
        return Err(ComponentsError::LimitExceeded { what: "components on one layer", limit: MAX_LAYER_COMPONENTS });
    }
    let modes: Vec<(usize, &Component, Option<AlignMode>)> = layer
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Shape::Component(c) => Some((i, c, mode_of(font, &facts, c))),
            _ => None,
        })
        .collect();
    if !always && modes.iter().all(|(_, _, m)| m.is_none()) {
        return Ok(None);
    }
    let derives_width = !facts.has_paths;
    let (left, right) = if derives_width { crate::metrics::offsets(glyph, layer) } else { (0.0, 0.0) };
    let bad = || ComponentsError::BadPlacement { glyph: glyph.name.clone() };

    let mut anchors_of: HashMap<(&str, &str), Arc<[EffectiveAnchor]>> = HashMap::new();
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut targets = 0usize;
    let mut work = 0usize;
    let mut pen = left;
    let mut parts = Vec::with_capacity(modes.len());
    for (shape, c, mode) in modes {
        if c.transform.numbers().iter().any(|v| !v.is_finite()) {
            return Err(bad());
        }
        let [a, b, cc, d, stored_x, stored_y] = c.affine().as_coeffs();
        let base_glyph = font.glyph(&c.glyph);
        let base_layer = base_glyph.and_then(|g| base_layer_of(g, layer, c));
        let width = base_layer.map_or(0.0, |l| l.width);
        let anchors: Arc<[EffectiveAnchor]> = match base_layer {
            Some(bl) => match anchors_of.get(&(c.glyph.as_str(), bl.id.as_str())) {
                Some(found) => found.clone(),
                None => {
                    let found = cache.get(font, &c.glyph, &bl.id)?;
                    anchors_of.insert((c.glyph.as_str(), bl.id.as_str()), found.clone());
                    found
                }
            },
            None => Arc::from(Vec::new()),
        };
        work = work.saturating_add(anchors.len()).saturating_add(1);
        if work > MAX_ALIGN_WORK {
            return Err(ComponentsError::LimitExceeded { what: "alignment steps on one layer", limit: MAX_ALIGN_WORK });
        }
        let (flip_x, flip_y) = (a < 0.0, d < 0.0);
        let local: Vec<(String, Point)> =
            anchors.iter().map(|an| (swap_names(&an.name, flip_x, flip_y), Point::new(a * an.x + cc * an.y, b * an.x + d * an.y))).collect();

        let attach = find_target(&local, c.info.anchor.as_deref(), &clusters);
        let attached = attach.map(|(_, own, target)| Point::new(target.x - own.x, target.y - own.y));
        let (tx, ty, cluster_at, is_base) = match attach {
            Some((cluster, own, target)) => {
                let (x, y) = match mode {
                    Some(AlignMode::Full) => (target.x - own.x, target.y - own.y),
                    Some(AlignMode::Horizontal) => (target.x - own.x, stored_y),
                    None => (stored_x, stored_y),
                };
                (x, y, cluster, false)
            }
            None => {
                let joined =
                    clusters.last().and_then(|prev| prev.get("#exit")).copied().zip(local.iter().find(|(n, _)| n == "#entry").map(|(_, p)| *p));
                let x = match (mode, joined) {
                    (None, _) => stored_x,
                    (Some(_), Some((exit, entry))) => exit.x - entry.x,
                    (Some(_), None) => pen + (-a * width).max(0.0),
                };
                let y = match (mode, joined) {
                    (Some(AlignMode::Full), Some((exit, entry))) => exit.y - entry.y,
                    (Some(AlignMode::Full), None) if d >= 0.0 => 0.0,
                    _ => stored_y,
                };
                let box_start = x + (a * width).min(0.0);
                pen = box_start + width * a.hypot(b);
                clusters.push(Cluster::new());
                (x, y, clusters.len().saturating_sub(1), true)
            }
        };
        let (tx, ty) = (keep_if_same(tx, stored_x), keep_if_same(ty, stored_y));
        if !(in_range(tx) && in_range(ty) && in_range(pen)) {
            return Err(bad());
        }
        if let Some(cluster) = clusters.get_mut(cluster_at) {
            for (name, p) in local.into_iter().filter(|(n, _)| !n.starts_with('_')) {
                if cluster.insert(name, Point::new(p.x + tx, p.y + ty)).is_none() {
                    targets += 1;
                    if targets > MAX_ALIGN_TARGETS {
                        return Err(ComponentsError::LimitExceeded { what: "anchor targets on one layer", limit: MAX_ALIGN_TARGETS });
                    }
                }
            }
        }
        let transform = if mode.is_some() { with_translation(&c.transform, tx, ty) } else { c.transform };
        let source = base_glyph.zip(base_layer).map(|(g, l)| (g.name.clone(), l.id.clone()));
        parts.push(Planned { shape, mode, transform, base: is_base, attached, source });
    }
    let width = if derives_width {
        let w = keep_if_same(pen + right, layer.width);
        if !in_range(w) {
            return Err(bad());
        }
        Some(w)
    } else {
        None
    };
    Ok(Some(LayerPlan { parts, width }))
}

/// The anchor a component attaches to: (cluster index, the component's own underscore anchor
/// with its transform's linear part applied, the target in composite coordinates).
fn find_target(local: &[(String, Point)], attr: Option<&str>, clusters: &[Cluster]) -> Option<(usize, Point, Point)> {
    let current_at = clusters.len().checked_sub(1)?;
    let current = clusters.get(current_at)?;
    let own = |name: &str| local.iter().find(|(n, _)| n.strip_prefix('_') == Some(name)).map(|(_, p)| *p);
    if let Some(attr) = attr.filter(|a| !a.is_empty()) {
        let plain = attr.split(['_', '.']).next().unwrap_or(attr);
        if let Some(mine) = own(plain).or_else(|| own(attr)) {
            if let Some(t) = current.get(attr) {
                return Some((current_at, mine, *t));
            }
            if let Some((n, t)) = ligature_part(attr, plain).and_then(|n| clusters.get(n).and_then(|c| c.get(plain)).map(|t| (n, *t))) {
                return Some((n, mine, t));
            }
            if let Some(t) = current.get(plain) {
                return Some((current_at, mine, *t));
            }
        }
    }
    local.iter().find_map(|(name, p)| {
        let wanted = name.strip_prefix('_').filter(|w| !w.is_empty())?;
        current.get(wanted).map(|t| (current_at, *p, *t))
    })
}

/// The cluster index a numbered ligature anchor names: `top_2` with plain name `top` → 1.
fn ligature_part(attr: &str, plain: &str) -> Option<usize> {
    let digits = attr.strip_prefix(plain)?.strip_prefix('_')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<usize>().ok()?.checked_sub(1)
}

/// `name` with the side words swapped that a flip exchanges: `top`/`bottom` for a vertical one,
/// `left`/`right` and `entry`/`exit` for a horizontal one.
pub(crate) fn swap_names(name: &str, flip_x: bool, flip_y: bool) -> String {
    const VERTICAL: [(&str, &str); 2] = [("top", "bottom"), ("bottom", "top")];
    const HORIZONTAL: [(&str, &str); 4] = [("left", "right"), ("right", "left"), ("entry", "exit"), ("exit", "entry")];
    if !flip_x && !flip_y {
        return name.to_string();
    }
    let pairs: Vec<(&str, &str)> = VERTICAL.iter().filter(|_| flip_y).chain(HORIZONTAL.iter().filter(|_| flip_x)).copied().collect();
    let mut out = String::with_capacity(name.len() + 3);
    let mut rest = name;
    while !rest.is_empty() {
        if let Some((from, to)) = pairs.iter().find(|(from, _)| rest.starts_with(from)) {
            out.push_str(to);
            rest = rest.get(from.len()..).unwrap_or("");
        } else {
            let mut chars = rest.chars();
            if let Some(ch) = chars.next() {
                out.push(ch);
            }
            rest = chars.as_str();
        }
    }
    out
}

fn with_translation(t: &Transform, x: f64, y: f64) -> Transform {
    match *t {
        Transform::Decomposed { scale, angle, slant, .. } => Transform::Decomposed { pos: (x, y), scale, angle, slant },
        Transform::Matrix([a, b, c, d, _, _]) => Transform::Matrix([a, b, c, d, x, y]),
    }
}

fn keep_if_same(computed: f64, stored: f64) -> f64 {
    if (computed - stored).abs() <= SAME { stored } else { computed }
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}
