//! Anchor propagation (M6.1): the effective anchors of a glyph's layer, its own anchors plus the
//! anchors its components pass on.
//!
//! Rows: `components.topBottomAnchorsPropagate`, `components.ownAnchorOverridesPropagated`,
//! `components.propagateAnchorsParameter`, `components.ligatureNumberedAnchors`,
//! `components.verticalFlipSwapsTopBottomAnchors`, `transform.mirrorComponentAnchors`,
//! `Propagate Anchors`, `compile.glyphsOutlinePreSteps` (the propagation half).
//!
//! Provenance: this module is written from the reference handbook's description, the behaviour
//! facts in `docs/plans/M6.md` § 2, and the outputs of the reference compilers observed as a
//! black box (§ 5's propagation measure is the specification). It is never written from fontc's
//! or glyphsLib's propagation source, which derive from the reference app's code; neither is read
//! or ported. The rules below were found by running glyphsLib's public `propagate_all_anchors` on
//! fonts built for the purpose and comparing its output with a model of these rules, on random
//! fonts and on every master layer of the corpus.
//!
//! Contract:
//! - [`effective_anchors`] gives a layer's anchors after propagation, in the composite's
//!   coordinates. A layer without components keeps its own anchors exactly as stored.
//! - Order: the order of first insertion into one list, keyed by name. Components are read in
//!   order, then the layer's own anchors; a later anchor of a name replaces the earlier one in
//!   its place, and an anchor removed and added again goes to the end.
//! - Layer choice for a component's base: the base's layer with the same id, else the base's
//!   master layer of the same master. A base without either passes nothing on.
//! - A component of a smart glyph passes on the anchors of the glyph's pole layers for the
//!   composite layer's master, interpolated at the component's values
//!   ([`fontcraft_model::smart::component_anchors`]). With one pole layer, or pole layers that
//!   cannot be interpolated, the base's layer is read as for any component.
//! - When [`propagate_anchors_enabled`] is false the result is the layer's own anchors.
//! - Errors: a cycle ([`ComponentsError::Cycle`]), nesting past [`MAX_COMPONENT_DEPTH`], more
//!   than [`MAX_LAYER_ANCHORS`] anchors or [`MAX_LAYER_COMPONENTS`] components, a transform that
//!   is not finite, a missing layer of the glyph asked for. A missing base glyph passes nothing
//!   on and is not an error (compile's preflight reports it).
//! - Propagated anchors are computed, never stored in the font.
//! - [`AnchorCache`] keeps results across edits. An entry stays valid while the glyph's `Arc`
//!   and the `Arc`s of every glyph it reaches through components are pointer-equal to those it
//!   was computed from; the font has no revision counter, so pointers are the key.
//!
//! The rules, for one layer with components (names are compared exactly, case included):
//!
//! | Step | Rule |
//! |---|---|
//! | Mark with own anchors | a glyph of category Mark whose layer has anchors of its own keeps only those (with `*origin` applied) |
//! | `*origin` | a base's `*origin` anchor is subtracted from its other anchors and dropped; the layer's own `*origin` does the same to its own anchors |
//! | anchor attribute | for every component after the first: when the base has `_X` and `X`, where `X` is the attribute up to its first underscore, its `X` takes the attribute's name |
//! | transform | the component's transform is applied, positions rounded to 6 decimals |
//! | flips | see [`flips`]; a horizontal flip swaps `left`/`right` and `exit`/`entry` in the name, a vertical one `top`/`bottom` |
//! | opposite names | a base that is not a Mark and has `_top` (after flips) passes on no `bottom` or `_bottom`; with `_bottom`, no `top` or `_top` |
//! | `exit` | a component with no underscore anchor left (other than a lone `_`), after the first, removes the `exit…` anchors gathered so far unless it has an anchor whose name starts with `exit` |
//! | underscore | only the first component passes on an underscore anchor, only its first one, and none when the layer has underscore anchors of its own |
//! | `entry` | names that start with `entry` (before flips) come only from the first component |
//! | ligatures | in a glyph of subcategory Ligature, each component that counts as bases (see [`base_count`]) numbers its other names: `X_N` becomes `X_(N+k)` and an unnumbered `X` (or `X_text`) becomes `X_(k+1)`, `k` being the bases before it; `_`, `entry` and `exit` names keep theirs |
//! | own anchors | the layer's own anchors replace or join the list; an own `_top` then removes `bottom` and `_bottom`, an own `_bottom` removes `top` and `_top` |
//!
//! [`MAX_COMPONENT_DEPTH`]: crate::MAX_COMPONENT_DEPTH
//! [`MAX_LAYER_ANCHORS`]: crate::MAX_LAYER_ANCHORS
//! [`MAX_LAYER_COMPONENTS`]: crate::MAX_LAYER_COMPONENTS

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_glyphdata::{Category, GlyphData, SubCategory};
use fontcraft_model::{Anchor, Component, Font, FontInfo, Glyph, Layer, Shape, param_flag, smart};

use crate::{ComponentsError, MAX_ANCHOR_NAME_LEN, MAX_COMPONENT_DEPTH, MAX_LAYER_ANCHORS, MAX_LAYER_COMPONENTS};

/// The font custom parameter that turns propagation off when its checkbox is off.
pub const PROPAGATE_ANCHORS_PARAM: &str = "Propagate Anchors";

/// Most anchors handled (read from a base and placed) in one call, nested composites included.
/// A layer may hold [`MAX_LAYER_COMPONENTS`] components of bases with [`MAX_LAYER_ANCHORS`]
/// anchors each; this bounds the product.
pub const MAX_PROPAGATION_WORK: usize = 20_000_000;

/// Where an effective anchor comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum AnchorSource {
    /// Stored on the layer itself.
    Own,
    /// Passed on by the component at shape index `component` (an index into `Layer::shapes`),
    /// whose base glyph is `base`.
    Component { component: usize, base: String },
}

/// One anchor of a layer after propagation, in the composite's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveAnchor {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub source: AnchorSource,
}

/// Whether the font propagates anchors: true unless the font's Propagate Anchors parameter is
/// present and off.
pub fn propagate_anchors_enabled(font: &Font) -> bool {
    font.info.params.get(PROPAGATE_ANCHORS_PARAM).and_then(param_flag).unwrap_or(true)
}

/// The effective anchors of `glyph`'s layer `layer` (a layer id; a master layer's id is its
/// master's id). Categories come from the glyphs and the built-in glyph database. See the module
/// doc.
pub fn effective_anchors(font: &Font, glyph: &str, layer: &str) -> Result<Vec<EffectiveAnchor>, ComponentsError> {
    let data = GlyphData::new().ok();
    effective_anchors_with(font, data.as_ref(), glyph, layer)
}

/// [`effective_anchors`] with the glyph database to read categories from (a font's custom glyph
/// data on top of the built-in one); `None` reads only the categories the glyphs store.
pub fn effective_anchors_with(font: &Font, data: Option<&GlyphData>, glyph: &str, layer: &str) -> Result<Vec<EffectiveAnchor>, ComponentsError> {
    let mut run = Run::new(font, data);
    let anchors = run.layer_anchors(glyph, layer)?;
    Ok(anchors.to_vec())
}

/// The effective anchors of every glyph's master layer of `master`, by glyph name, computed in
/// dependency order so each base is worked out once. For compile and for whole-font checks.
/// Glyphs without a layer for `master` are left out.
pub fn effective_anchors_for_master(font: &Font, master: &str) -> Result<HashMap<String, Vec<EffectiveAnchor>>, ComponentsError> {
    let data = GlyphData::new().ok();
    effective_anchors_for_master_with(font, data.as_ref(), master)
}

/// [`effective_anchors_for_master`] with the glyph database to read categories from.
pub fn effective_anchors_for_master_with(
    font: &Font,
    data: Option<&GlyphData>,
    master: &str,
) -> Result<HashMap<String, Vec<EffectiveAnchor>>, ComponentsError> {
    font.master(master)?;
    let mut run = Run::new(font, data);
    let mut out = HashMap::with_capacity(font.glyphs.len());
    for g in font.glyphs.iter() {
        let Some(layer) = g.layer(master) else { continue };
        let anchors = run.anchors_of(g, layer, 0)?;
        out.insert(g.name.clone(), anchors.to_vec());
    }
    Ok(out)
}

/// One cached result and what it was computed from.
#[derive(Clone, Debug)]
struct CacheEntry {
    /// Every glyph name the result read, with the glyph it found (`None`: missing then).
    glyphs: Vec<(String, Option<Arc<Glyph>>)>,
    info: Arc<FontInfo>,
    anchors: Arc<[EffectiveAnchor]>,
}

impl CacheEntry {
    fn is_current(&self, font: &Font) -> bool {
        Arc::ptr_eq(&self.info, &font.info)
            && self.glyphs.iter().all(|(name, seen)| match (seen, font.glyphs.get(name)) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            })
    }
}

/// A cache of [`effective_anchors`] results. See the module doc for when an entry is valid.
#[derive(Clone, Debug, Default)]
pub struct AnchorCache {
    /// Keyed by (glyph, layer id).
    entries: HashMap<(String, String), CacheEntry>,
}

impl AnchorCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`effective_anchors`], from the cache when the entry is still valid.
    pub fn get(&mut self, font: &Font, glyph: &str, layer: &str) -> Result<Arc<[EffectiveAnchor]>, ComponentsError> {
        let key = (glyph.to_string(), layer.to_string());
        if let Some(entry) = self.entries.get(&key)
            && entry.is_current(font)
        {
            return Ok(entry.anchors.clone());
        }
        let data = GlyphData::new().ok();
        let mut run = Run::new(font, data.as_ref());
        run.reached = Some(HashMap::new());
        let anchors = run.layer_anchors(glyph, layer)?;
        let glyphs = run.reached.take().map(|m| m.into_iter().collect()).unwrap_or_default();
        self.entries.insert(key, CacheEntry { glyphs, info: font.info.clone(), anchors: anchors.clone() });
        Ok(anchors)
    }

    /// Drops the entries `font` no longer matches.
    pub fn retain_current(&mut self, font: &Font) {
        self.entries.retain(|_, entry| entry.is_current(font));
    }

    /// How many results are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What propagation needs to know about a glyph's classification.
#[derive(Clone, Copy, Debug, Default)]
struct Kind {
    mark: bool,
    ligature: bool,
}

/// One propagation computation: memoised results per (glyph, layer id).
struct Run<'a> {
    font: &'a Font,
    data: Option<&'a GlyphData>,
    enabled: bool,
    memo: HashMap<(String, String), Arc<[EffectiveAnchor]>>,
    counts: HashMap<(String, String), i64>,
    kinds: HashMap<String, Kind>,
    /// Glyphs whose layers are being worked out, outermost first (cycle detection).
    chain: Vec<String>,
    work: usize,
    /// When set, every glyph name looked up, with what was found (for [`AnchorCache`]).
    reached: Option<HashMap<String, Option<Arc<Glyph>>>>,
}

/// An anchor on its way through one component: its name now, its position, its name before the
/// flips (after the anchor attribute) and its name without the attribute, after the flips.
struct Moving {
    name: String,
    x: f64,
    y: f64,
    before_flip: String,
    plain: String,
}

impl<'a> Run<'a> {
    fn new(font: &'a Font, data: Option<&'a GlyphData>) -> Self {
        Run {
            font,
            data,
            enabled: propagate_anchors_enabled(font),
            memo: HashMap::new(),
            counts: HashMap::new(),
            kinds: HashMap::new(),
            chain: Vec::new(),
            work: 0,
            reached: None,
        }
    }

    fn glyph(&mut self, name: &str) -> Option<&'a Arc<Glyph>> {
        let found = self.font.glyphs.get(name);
        if let Some(reached) = &mut self.reached {
            reached.entry(name.to_string()).or_insert_with(|| found.cloned());
        }
        found
    }

    fn charge(&mut self, n: usize) -> Result<(), ComponentsError> {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_PROPAGATION_WORK {
            return Err(ComponentsError::LimitExceeded { what: "anchor propagation steps", limit: MAX_PROPAGATION_WORK });
        }
        Ok(())
    }

    /// The top-level request: the layer with id `layer` of `glyph`.
    fn layer_anchors(&mut self, glyph: &str, layer: &str) -> Result<Arc<[EffectiveAnchor]>, ComponentsError> {
        let g = self.glyph(glyph).ok_or_else(|| ComponentsError::MissingGlyph(glyph.to_string()))?;
        let l = g
            .layers
            .iter()
            .find(|l| l.id == layer)
            .ok_or_else(|| ComponentsError::MissingLayer { glyph: glyph.to_string(), layer: layer.to_string() })?;
        self.anchors_of(g, l, 0)
    }

    fn kind(&mut self, g: &Glyph) -> Kind {
        if let Some(k) = self.kinds.get(&g.name) {
            return *k;
        }
        let stored_cat = g.info.category.as_deref();
        let stored_sub = g.info.sub_category.as_deref();
        let looked_up = if stored_cat.is_none() || stored_sub.is_none() { self.data.map(|d| d.lookup(&g.name, &g.unicodes)) } else { None };
        let mark = match stored_cat {
            Some(c) => c == "Mark",
            None => looked_up.as_ref().and_then(|i| i.category.as_ref()).is_some_and(|c| c.value == Category::Mark),
        };
        let ligature = match stored_sub {
            Some(s) => s == "Ligature",
            None => looked_up.as_ref().and_then(|i| i.sub_category.as_ref()).is_some_and(|s| s.value == SubCategory::Ligature),
        };
        let k = Kind { mark, ligature };
        self.kinds.insert(g.name.clone(), k);
        k
    }

    /// The effective anchors of `layer` of `g`, memoised.
    fn anchors_of(&mut self, g: &'a Glyph, layer: &'a Layer, depth: usize) -> Result<Arc<[EffectiveAnchor]>, ComponentsError> {
        let key = (g.name.clone(), layer.id.clone());
        if let Some(found) = self.memo.get(&key) {
            return Ok(found.clone());
        }
        if depth > MAX_COMPONENT_DEPTH {
            return Err(ComponentsError::TooDeep { glyph: g.name.clone(), limit: MAX_COMPONENT_DEPTH });
        }
        if let Some(start) = self.chain.iter().position(|n| *n == g.name) {
            return Err(ComponentsError::Cycle(self.chain.get(start..).map(<[String]>::to_vec).unwrap_or_default()));
        }
        self.chain.push(g.name.clone());
        let result = self.compute(g, layer, depth);
        self.chain.pop();
        let anchors: Arc<[EffectiveAnchor]> = result?.into();
        self.memo.insert(key, anchors.clone());
        Ok(anchors)
    }

    fn compute(&mut self, g: &'a Glyph, layer: &'a Layer, depth: usize) -> Result<Vec<EffectiveAnchor>, ComponentsError> {
        if layer.anchors.len() > MAX_LAYER_ANCHORS {
            return Err(ComponentsError::LimitExceeded { what: "anchors on one layer", limit: MAX_LAYER_ANCHORS });
        }
        self.charge(layer.anchors.len())?;
        let own = || layer.anchors.iter().map(|a| own_anchor(a.name.clone(), a.x, a.y)).collect::<Vec<_>>();
        let comps: Vec<(usize, &'a Component)> =
            layer.shapes.iter().enumerate().filter_map(|(i, s)| if let Shape::Component(c) = s { Some((i, c)) } else { None }).collect();
        if comps.len() > MAX_LAYER_COMPONENTS {
            return Err(ComponentsError::LimitExceeded { what: "components on one layer", limit: MAX_LAYER_COMPONENTS });
        }
        if comps.is_empty() || !self.enabled {
            return Ok(own());
        }
        let kind = self.kind(g);
        if kind.mark && !layer.anchors.is_empty() {
            return Ok(origin_adjusted(own()));
        }
        let own_under: HashSet<&str> = layer.anchors.iter().map(|a| a.name.as_str()).filter(|n| n.starts_with('_')).collect();
        let mut out = Ordered::default();
        let mut before: i64 = 0;
        for (i, &(shape, comp)) in comps.iter().enumerate() {
            let Some(base) = self.glyph(&comp.glyph) else { continue };
            let Some(base_layer) = base_layer(base, layer) else { continue };
            let inherited = match self.smart_anchors(base, comp, &layer.master) {
                Some(anchors) => anchors,
                None => self.anchors_of(base, base_layer, depth + 1)?,
            };
            self.charge(inherited.len())?;
            let bases = self.base_count(base, base_layer, depth + 1)?;
            let base_kind = self.kind(base);
            let moved = move_through(&g.name, comp, i, &inherited)?;
            let drop = opposite_names(if base_kind.mark { None } else { Some(base_layer.anchors.as_slice()) }, flips(&matrix(comp)));
            let moved: Vec<Moving> = moved.into_iter().filter(|m| !drop.contains(m.plain.as_str())).collect();
            if i > 0 && !moved.iter().any(|m| is_attaching(&m.name)) && !moved.iter().any(|m| m.before_flip.starts_with("exit")) {
                out.remove_where(|n| n.starts_with("exit"));
            }
            let mut seen_under = false;
            for m in moved {
                let mut name = m.name;
                if name.starts_with('_') {
                    if i != 0 || !own_under.is_empty() || seen_under {
                        continue;
                    }
                    seen_under = true;
                }
                if m.before_flip.starts_with("entry") && i != 0 {
                    continue;
                }
                if kind.ligature && bases > 0 && !name.starts_with('_') && !name.starts_with("entry") && !name.starts_with("exit") {
                    name = ligature_name(&name, before);
                }
                check_name(&name)?;
                out.insert(EffectiveAnchor { name, x: m.x, y: m.y, source: AnchorSource::Component { component: shape, base: comp.glyph.clone() } })?;
            }
            before = before.saturating_add(bases);
        }
        for a in origin_adjusted(own()) {
            out.insert(a)?;
        }
        let own_drop = opposite_names(Some(layer.anchors.as_slice()), (false, false));
        out.remove_where(|n| own_drop.contains(n));
        Ok(out.into_vec())
    }

    /// The anchors a smart component passes on: its base's pole layers' anchors interpolated at
    /// its values. `None` when `base` is not a smart glyph, has one pole layer for `master`, or
    /// cannot be interpolated; the base's layer then passes on its anchors as for any component.
    fn smart_anchors(&mut self, base: &Glyph, comp: &Component, master: &str) -> Option<Arc<[EffectiveAnchor]>> {
        if base.info.parts.is_empty() {
            return None;
        }
        let anchors = smart::component_anchors(self.font, comp, master).ok().flatten()?;
        Some(anchors.into_iter().map(|a| own_anchor(a.name, a.x, a.y)).collect())
    }

    /// How many ligature parts `g`'s layer counts as when it is a component of a ligature. See
    /// the module doc's table and [`anchor_base_count`].
    fn base_count(&mut self, g: &'a Glyph, layer: &'a Layer, depth: usize) -> Result<i64, ComponentsError> {
        let key = (g.name.clone(), layer.id.clone());
        if let Some(n) = self.counts.get(&key) {
            return Ok(*n);
        }
        if depth > MAX_COMPONENT_DEPTH {
            return Err(ComponentsError::TooDeep { glyph: g.name.clone(), limit: MAX_COMPONENT_DEPTH });
        }
        let kind = self.kind(g);
        let has_components = layer.shapes.iter().any(|s| matches!(s, Shape::Component(_)));
        let n = if !has_components {
            if kind.mark {
                0
            } else {
                let own: Vec<EffectiveAnchor> = layer.anchors.iter().map(|a| own_anchor(a.name.clone(), a.x, a.y)).collect();
                anchor_base_count(&own, kind.ligature)
            }
        } else if kind.mark && !layer.anchors.is_empty() {
            0
        } else {
            let mut sum: i64 = 0;
            for s in &layer.shapes {
                let Shape::Component(c) = s else { continue };
                let Some(base) = self.glyph(&c.glyph) else { continue };
                let Some(bl) = base_layer(base, layer) else { continue };
                sum = sum.saturating_add(self.base_count(base, bl, depth + 1)?);
            }
            if sum > 0 {
                sum
            } else {
                let anchors = self.anchors_of(g, layer, depth)?;
                anchor_base_count(&anchors, kind.ligature)
            }
        };
        self.counts.insert(key, n);
        Ok(n)
    }
}

/// The anchors `comp` (the `index`-th component of a layer of `glyph`) passes on from its base's
/// effective anchors, in the composite's coordinates, before the rules that depend on the
/// rest of the layer.
fn move_through(glyph: &str, comp: &Component, index: usize, inherited: &[EffectiveAnchor]) -> Result<Vec<Moving>, ComponentsError> {
    let m = matrix(comp);
    if m.iter().any(|v| !v.is_finite()) {
        return Err(ComponentsError::BadPlacement { glyph: glyph.to_string() });
    }
    let (xf, yf) = flips(&m);
    let adjusted = origin_adjusted(inherited.to_vec());
    let rename_to = comp.info.anchor.as_deref().filter(|_| index > 0).and_then(|attr| {
        let x = attr.split('_').next().unwrap_or(attr);
        let under = format!("_{x}");
        let has = |n: &str| adjusted.iter().any(|a| a.name == n);
        (has(&under) && has(x)).then(|| (x.to_string(), attr.to_string()))
    });
    let mut out = Vec::with_capacity(adjusted.len());
    for a in adjusted {
        let plain = rename_flipped(&a.name, xf, yf);
        let before_flip = match &rename_to {
            Some((x, attr)) if a.name == *x => attr.clone(),
            _ => a.name,
        };
        let name = rename_flipped(&before_flip, xf, yf);
        let [xx, yx, xy, yy, dx, dy] = m;
        let x = round6(xx * a.x + xy * a.y + dx);
        let y = round6(yx * a.x + yy * a.y + dy);
        if !(x.is_finite() && y.is_finite()) {
            return Err(ComponentsError::BadPlacement { glyph: glyph.to_string() });
        }
        out.push(Moving { name, x, y, before_flip, plain });
    }
    Ok(out)
}

/// The layer of `base` that a component on `layer` reads: the one with the same id, else the
/// master layer of the same master.
fn base_layer<'g>(base: &'g Glyph, layer: &Layer) -> Option<&'g Layer> {
    base.layers.iter().find(|l| l.id == layer.id).or_else(|| base.layer(&layer.master))
}

fn own_anchor(name: String, x: f64, y: f64) -> EffectiveAnchor {
    EffectiveAnchor { name, x, y, source: AnchorSource::Own }
}

/// `anchors` with the first `*origin` anchor's position subtracted from the others, and every
/// `*origin` anchor dropped.
fn origin_adjusted(anchors: Vec<EffectiveAnchor>) -> Vec<EffectiveAnchor> {
    let Some((ox, oy)) = anchors.iter().find(|a| a.name == "*origin").map(|a| (a.x, a.y)) else { return anchors };
    anchors
        .into_iter()
        .filter(|a| a.name != "*origin")
        .map(|mut a| {
            a.x -= ox;
            a.y -= oy;
            a
        })
        .collect()
}

/// An underscore anchor in the sense of the `exit` rule: a lone `_` does not count.
fn is_attaching(name: &str) -> bool {
    name.starts_with('_') && name.len() > 1
}

/// The names removed because of `_top` or `_bottom` among `anchors` (renamed by the flips).
fn opposite_names(anchors: Option<&[Anchor]>, (xf, yf): (bool, bool)) -> HashSet<&'static str> {
    let mut drop = HashSet::new();
    let Some(anchors) = anchors else { return drop };
    for a in anchors.iter().filter(|a| a.name.starts_with('_')) {
        match rename_flipped(&a.name, xf, yf).as_str() {
            "_top" => drop.extend(["bottom", "_bottom"]),
            "_bottom" => drop.extend(["top", "_top"]),
            _ => {}
        }
    }
    drop
}

/// The component's transform as `[xx, yx, xy, yy, dx, dy]`, with entries within 1e-12 of 0 set
/// to 0 so that a decomposed 180° rotation reads as the exact matrix a file would store.
fn matrix(comp: &Component) -> [f64; 6] {
    let mut m = comp.affine().as_coeffs();
    for v in m.iter_mut().take(4) {
        if v.abs() < 1e-12 {
            *v = 0.0;
        }
    }
    m
}

/// Whether a transform flips anchors horizontally and vertically, for renaming them.
///
/// The rule reproduces the reference compilers' renaming on every matrix tried: with `θ` the
/// angle of the transformed x axis, the transform is taken apart as `M · R(−θ)`, and an axis is
/// flipped when its diagonal entry there is negative. A transform whose x axis points exactly
/// left (a 180° rotation, a horizontal mirror) swaps both answers.
fn flips(m: &[f64; 6]) -> (bool, bool) {
    let [a, b, c, d, _, _] = *m;
    let t = b.atan2(a);
    let (sin, cos) = t.sin_cos();
    let mut xf = a * cos - c * sin < 0.0;
    let mut yf = b * sin + d * cos < 0.0;
    if b == 0.0 && a < 0.0 {
        xf = !xf;
        yf = !yf;
    }
    (xf, yf)
}

/// `name` as a flipped component passes it on.
fn rename_flipped(name: &str, xf: bool, yf: bool) -> String {
    let mut n = name.to_string();
    if xf {
        if n.contains("left") {
            n = n.replace("left", "right");
        } else if n.contains("right") {
            n = n.replace("right", "left");
        }
        if n.contains("exit") {
            n = n.replace("exit", "entry");
        } else if n.contains("entry") {
            n = n.replace("entry", "exit");
        }
    }
    if yf {
        if n.contains("bottom") {
            n = n.replace("bottom", "top");
        } else if n.contains("top") {
            n = n.replace("top", "bottom");
        }
    }
    n
}

/// Rounds to 6 decimals, as the reference compilers do after a transform.
fn round6(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r.is_finite() { r } else { v }
}

/// The text before the first underscore and the number after it, when the rest is an integer.
fn split_number(name: &str) -> (&str, Option<i64>) {
    match name.split_once('_') {
        Some((stem, rest)) => (stem, parse_integer(rest)),
        None => (name, None),
    }
}

/// An integer as the reference compilers read one: optional spaces around, an optional sign,
/// ASCII digits with single underscores between them. `None` for anything else, or a number
/// beyond 64 bits.
fn parse_integer(text: &str) -> Option<i64> {
    let t = text.trim();
    let (negative, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let mut value: i64 = 0;
    for ch in digits.chars().filter(|c| *c != '_') {
        let d = ch.to_digit(10)?;
        value = value.checked_mul(10)?.checked_add(i64::from(d))?;
    }
    Some(if negative { -value } else { value })
}

/// The name an anchor takes in a ligature when `before` bases come before its component.
fn ligature_name(name: &str, before: i64) -> String {
    match split_number(name) {
        (stem, Some(n)) => format!("{stem}_{}", n.saturating_add(before)),
        (stem, None) => format!("{stem}_{}", before.saturating_add(1)),
    }
}

/// How many ligature parts a layer's anchors say it has, when its components do not say.
///
/// After `*origin`: a ligature counts 1 when it has an anchor and no underscore anchor, else 0.
/// Any other glyph takes the largest of: 1 for each unnumbered name (an `entry…` or `exit…` name
/// counts as unnumbered), unless the layer has underscore anchors or the name starts with `#`;
/// `N` for a name `X_N`, and `N + 1` for `caret…_N`. Underscore names count nothing.
fn anchor_base_count(anchors: &[EffectiveAnchor], ligature: bool) -> i64 {
    let anchors = origin_adjusted(anchors.to_vec());
    let under = anchors.iter().any(|a| a.name.starts_with('_'));
    if ligature {
        return i64::from(!under && anchors.iter().any(|a| !a.name.starts_with('#')));
    }
    let mut count: i64 = 0;
    for a in anchors.iter().filter(|a| !a.name.starts_with('_')) {
        let number = if a.name.starts_with("entry") || a.name.starts_with("exit") { None } else { split_number(&a.name).1 };
        match number {
            None if !under && !a.name.starts_with('#') => count = count.max(1),
            None => {}
            Some(n) if a.name.starts_with("caret") => count = count.max(n.saturating_add(1)),
            Some(n) => count = count.max(n),
        }
    }
    count
}

fn check_name(name: &str) -> Result<(), ComponentsError> {
    if name.len() > MAX_ANCHOR_NAME_LEN {
        return Err(ComponentsError::LimitExceeded { what: "bytes in a propagated anchor name", limit: MAX_ANCHOR_NAME_LEN });
    }
    Ok(())
}

/// Anchors keyed by name in order of first insertion: a later anchor of a name replaces the
/// earlier one in its place; a removed name added again goes to the end.
#[derive(Default)]
struct Ordered {
    items: Vec<Option<EffectiveAnchor>>,
    index: HashMap<String, usize>,
}

impl Ordered {
    fn insert(&mut self, a: EffectiveAnchor) -> Result<(), ComponentsError> {
        if let Some(&i) = self.index.get(&a.name)
            && let Some(slot) = self.items.get_mut(i)
        {
            *slot = Some(a);
            return Ok(());
        }
        if self.index.len() >= MAX_LAYER_ANCHORS {
            return Err(ComponentsError::LimitExceeded { what: "anchors on one layer", limit: MAX_LAYER_ANCHORS });
        }
        self.index.insert(a.name.clone(), self.items.len());
        self.items.push(Some(a));
        Ok(())
    }

    fn remove_where(&mut self, mut pred: impl FnMut(&str) -> bool) {
        let gone: Vec<String> = self.index.keys().filter(|n| pred(n)).cloned().collect();
        for name in gone {
            if let Some(i) = self.index.remove(&name)
                && let Some(slot) = self.items.get_mut(i)
            {
                *slot = None;
            }
        }
    }

    fn into_vec(self) -> Vec<EffectiveAnchor> {
        self.items.into_iter().flatten().collect()
    }
}
