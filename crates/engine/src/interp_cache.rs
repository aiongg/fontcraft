//! Interpolation results kept between commands and frames (M9.20; M9.22 reads the compatibility
//! table).
//!
//! Contract (`docs/plans/M9.md` § 3 › Caching): one cache per document. It holds the
//! [`Designspace`], each glyph's sources and compatibility, and the instance layers the preview
//! draws, for one font revision. [`InterpCache::sync`] brings it to a new revision by comparing
//! pointers with the font it holds:
//!
//! | What changed | What is dropped |
//! |---|---|
//! | nothing (the same font `Arc`) | nothing |
//! | axes, masters, instances, font info, metrics, stems, numbers or units per em | everything |
//! | some glyphs (their `Arc`s differ, or they were added or removed) | those glyphs' entries |
//! | the kerning, or a glyph's kerning groups | each instance's interpolated master and kerning |
//!
//! So an edit to one glyph costs one pass of pointer comparisons over the glyph table, and the
//! other glyphs keep their compatibility and instance layers.
//!
//! Errors are not cached, with one exception: a glyph an instance view could not interpolate is
//! remembered until the glyph or the font level changes, so a preview does not retry it every
//! frame. The instance layers kept are capped at [`MAX_INSTANCE_LAYERS`]; past it the oldest are
//! dropped.
//!
//! An instance view ([`InterpCache::instance_view`]) is a font the preview lays out and draws
//! like any other: one master at the instance, named [`view_master_id`], with the instance's
//! metrics and kerning, holding the glyphs asked for and the glyphs their components use, each
//! with its interpolated layer. Instance layers come from `fontcraft_interp::instance`'s
//! [`instance_font_with_notes`] run on a copy of the font reduced to the glyphs still missing, so
//! the preview applies exactly what a static instance applies: intermediate and alternate layers,
//! manual master weights, Local Interpolation and InterpolationWeightY
//! (`preview.respectsInstanceData`).
//!
//! Who uses it: the preview of instances and `preview.render` (M9.20), the compatibility
//! indicators and the Master Compatibility view (M9.22). Commands may build a designspace
//! directly; the cache only saves work.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use fontcraft_interp::compat::{check_glyph, scope};
use fontcraft_interp::instance::{instance_font_with_notes, options_for};
use fontcraft_interp::{Compat, Designspace, GlyphSources, InterpError, InterpOptions, MasterPairs, ModelCache};
use fontcraft_model::{Font, Glyph, GlyphTable, Kerning, Layer, MAX_COMPONENT_DEPTH, Master, Shape};

/// Most interpolated instance layers kept.
pub const MAX_INSTANCE_LAYERS: usize = 100_000;
/// Most glyphs one instance view holds: the glyphs asked for and the components they use.
pub const MAX_VIEW_GLYPHS: usize = 10_000;
/// Instance views kept at once; past it they are all dropped.
const MAX_VIEWS: usize = 64;
/// The start of the id of an instance view's one master; the instance's index follows.
pub const VIEW_MASTER_PREFIX: &str = "preview-instance-";

/// The id of the master of instance `instance`'s view. Caches keyed by layer id (the canvas's
/// composite outlines) keep the view's layers apart from the document's own through it.
pub fn view_master_id(instance: usize) -> String {
    format!("{VIEW_MASTER_PREFIX}{instance}")
}

/// An instance's interpolated master and kerning, both under [`view_master_id`].
struct InstanceBase {
    master: Master,
    kerning: Arc<Kerning>,
}

/// The interpolation cache of one document.
#[derive(Default)]
pub struct InterpCache {
    /// The font and revision the entries belong to.
    font: Option<(Arc<Font>, u64)>,
    designspace: Option<Arc<Designspace>>,
    sources: HashMap<String, Arc<GlyphSources>>,
    compat: HashMap<String, Arc<Compat>>,
    /// The masters the compatibility check covers ([`scope`]), built with the first check.
    scope: Option<Arc<MasterPairs>>,
    /// Keyed by glyph name and instance index, with the generation the layer was added in.
    instance_layers: HashMap<(String, usize), (Arc<Layer>, u64)>,
    /// The instance layers in the order they were added, oldest first. An entry whose
    /// generation differs from the map's is stale and skipped.
    instance_order: VecDeque<((String, usize), u64)>,
    generation: u64,
    /// Glyphs an instance view could not interpolate, until they change.
    instance_failed: HashSet<(String, usize)>,
    /// By instance index.
    instance_bases: HashMap<usize, Arc<InstanceBase>>,
    /// Instance views by instance and the glyphs they were asked for.
    views: HashMap<(usize, Vec<String>), Arc<Font>>,
    models: ModelCache,
}

impl InterpCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the cache to `font` at `revision`, dropping what the change made stale.
    pub fn sync(&mut self, font: &Arc<Font>, revision: u64) {
        let Some((old, _)) = self.font.take() else {
            self.clear();
            self.font = Some((Arc::clone(font), revision));
            return;
        };
        if !Arc::ptr_eq(&old, font) {
            if same_font_level(&old, font) {
                let changed = changed_glyphs(&old, font);
                if changed.groups || !same(&old.kerning, &font.kerning) {
                    self.instance_bases.clear();
                }
                self.invalidate(&changed.names);
                self.views.clear();
            } else {
                self.clear();
            }
        }
        self.font = Some((Arc::clone(font), revision));
    }

    /// The designspace of `font`.
    pub fn designspace(&mut self, font: &Arc<Font>, revision: u64) -> Result<Arc<Designspace>, InterpError> {
        self.sync(font, revision);
        self.designspace_of(font)
    }

    fn designspace_of(&mut self, font: &Font) -> Result<Arc<Designspace>, InterpError> {
        if let Some(ds) = &self.designspace {
            return Ok(Arc::clone(ds));
        }
        let ds = Arc::new(Designspace::new(font)?);
        self.designspace = Some(Arc::clone(&ds));
        Ok(ds)
    }

    /// The compatibility of `glyph`.
    ///
    /// A glyph whose sources cannot be read (two at one location, for one) cannot interpolate:
    /// it is incompatible, with no mismatch to show, as `interpolation.checkCompatibility`
    /// reports it. An unknown glyph and a font without a usable designspace are errors.
    pub fn compat(&mut self, font: &Arc<Font>, revision: u64, glyph: &str) -> Result<Arc<Compat>, InterpError> {
        self.sync(font, revision);
        if let Some(c) = self.compat.get(glyph) {
            return Ok(Arc::clone(c));
        }
        if font.glyph(glyph).is_none() {
            return Err(InterpError::UnknownGlyph(glyph.to_string()));
        }
        let (ds, scope) = self.designspace_and_scope(font)?;
        let sources = match self.sources.get(glyph) {
            Some(s) => Some(Arc::clone(s)),
            None => match GlyphSources::of(font, &ds, glyph) {
                Ok(s) => {
                    let s = Arc::new(s);
                    self.sources.insert(glyph.to_string(), Arc::clone(&s));
                    Some(s)
                }
                Err(_) => None,
            },
        };
        let result = match sources {
            Some(s) => check_glyph(font, &s, &scope),
            None => Compat { ok: false, ..Compat::default() },
        };
        let result = Arc::new(result);
        self.compat.insert(glyph.to_string(), Arc::clone(&result));
        Ok(result)
    }

    /// Whether the editor shows incompatibility indicators: off while the font's Enforce
    /// Compatibility Check parameter is present and unchecked.
    pub fn indicators(&mut self, font: &Arc<Font>, revision: u64) -> Result<bool, InterpError> {
        self.sync(font, revision);
        Ok(self.designspace_and_scope(font)?.1.indicators)
    }

    /// The glyphs of `font` whose layers cannot interpolate, in glyph order.
    pub fn incompatible(&mut self, font: &Arc<Font>, revision: u64) -> Result<Vec<String>, InterpError> {
        let mut out = Vec::new();
        for g in font.glyphs.iter() {
            if !self.compat(font, revision, &g.name)?.ok {
                out.push(g.name.clone());
            }
        }
        Ok(out)
    }

    /// The designspace and the compatibility scope, built once per font revision. The
    /// designspace is shared with [`InterpCache::designspace`]'s entry when that is already built.
    fn designspace_and_scope(&mut self, font: &Font) -> Result<(Arc<Designspace>, Arc<MasterPairs>), InterpError> {
        let ds = self.designspace_of(font)?;
        let pairs = match &self.scope {
            Some(p) => Arc::clone(p),
            None => {
                let p = Arc::new(scope(font, &ds));
                self.scope = Some(Arc::clone(&p));
                p
            }
        };
        Ok((ds, pairs))
    }

    /// `glyph` interpolated at the instance at `instance` (an index into `Font::instances`), as
    /// the static instance makes it. Its id and master are the origin master's.
    pub fn instance_layer(&mut self, font: &Arc<Font>, revision: u64, glyph: &str, instance: usize) -> Result<Arc<Layer>, InterpError> {
        self.sync(font, revision);
        let key = (glyph.to_string(), instance);
        if let Some((layer, _)) = self.instance_layers.get(&key) {
            return Ok(Arc::clone(layer));
        }
        if font.instances.get(instance).is_none() {
            return Err(InterpError::UnknownInstance(instance));
        }
        if font.glyph(glyph).is_none() {
            return Err(InterpError::UnknownGlyph(glyph.to_string()));
        }
        let options = options_for(font, instance)?;
        let layers = interpolate_glyphs(font, instance, &options, std::slice::from_ref(&key.0))?;
        let Some((_, layer)) = layers.into_iter().next() else {
            // The instance leaves out a glyph without a layer in the origin master.
            let origin = self.designspace_of(font)?.origin().map(|m| m.id.clone()).unwrap_or_default();
            return Err(InterpError::UnknownLayer { glyph: glyph.to_string(), layer: origin });
        };
        self.instance_failed.remove(&key);
        Ok(self.store(key, layer))
    }

    /// A font for previewing the instance at `instance`: one master at the instance (its id is
    /// [`view_master_id`]) with the instance's metrics and kerning, holding `glyphs` and the
    /// glyphs their components use, in that order, each with its interpolated layer as the
    /// master layer. A glyph that cannot be interpolated is left out. The view is kept until the
    /// font or the glyphs asked for change.
    pub fn instance_view(&mut self, font: &Arc<Font>, revision: u64, instance: usize, glyphs: &[String]) -> Result<Arc<Font>, InterpError> {
        self.sync(font, revision);
        if font.instances.get(instance).is_none() {
            return Err(InterpError::UnknownInstance(instance));
        }
        let key = (instance, glyphs.to_vec());
        if let Some(view) = self.views.get(&key) {
            return Ok(Arc::clone(view));
        }
        if glyphs.len() > MAX_VIEW_GLYPHS {
            return Err(InterpError::LimitExceeded { what: "glyphs in one preview", limit: MAX_VIEW_GLYPHS });
        }
        let base = self.base(font, instance)?;
        let options = options_for(font, instance)?;

        // The glyphs asked for, then the components they use, one nesting level at a time.
        let mut seen: HashSet<String> = HashSet::new();
        let mut level: Vec<String> = glyphs.iter().filter(|n| seen.insert((*n).clone())).cloned().collect();
        let mut order: Vec<String> = Vec::new();
        for _ in 0..=MAX_COMPONENT_DEPTH {
            if level.is_empty() {
                break;
            }
            self.fill_instance(font, instance, &options, &level);
            let mut next = Vec::new();
            for name in &level {
                let Some((layer, _)) = self.instance_layers.get(&(name.clone(), instance)) else { continue };
                for shape in &layer.shapes {
                    if let Shape::Component(c) = shape
                        && seen.len() < MAX_VIEW_GLYPHS
                        && seen.insert(c.glyph.clone())
                    {
                        next.push(c.glyph.clone());
                    }
                }
            }
            order.append(&mut level);
            level = next;
        }

        let id = view_master_id(instance);
        let mut out = Vec::with_capacity(order.len());
        for name in &order {
            let (Some(g), Some((layer, _))) = (font.glyphs.get(name), self.instance_layers.get(&(name.clone(), instance))) else { continue };
            let mut layer = Layer::clone(layer);
            layer.id.clone_from(&id);
            layer.master.clone_from(&id);
            out.push(Arc::new(Glyph { layers: vec![layer], ..Glyph::clone(g) }));
        }
        let mut view = Font::clone(font);
        view.masters = Arc::new(vec![base.master.clone()]);
        view.kerning = Arc::clone(&base.kerning);
        view.glyphs = GlyphTable::from_glyphs(out);
        let view = Arc::new(view);
        if self.views.len() >= MAX_VIEWS {
            self.views.clear();
        }
        self.views.insert(key, Arc::clone(&view));
        Ok(view)
    }

    /// Interpolates the glyphs of `names` at `instance` that are neither kept nor known to
    /// fail: all in one run, or one by one when the run fails, so one glyph that cannot be
    /// interpolated does not take the others with it.
    fn fill_instance(&mut self, font: &Font, instance: usize, options: &InterpOptions, names: &[String]) {
        let mut todo: Vec<String> = Vec::new();
        for name in names {
            let key = (name.clone(), instance);
            if self.instance_layers.contains_key(&key) || self.instance_failed.contains(&key) || font.glyph(name).is_none() || todo.contains(name) {
                continue;
            }
            todo.push(name.clone());
        }
        if todo.is_empty() {
            return;
        }
        let runs: Vec<Vec<String>> = match interpolate_glyphs(font, instance, options, &todo) {
            Ok(layers) => {
                self.keep(instance, &todo, layers);
                return;
            }
            Err(_) if todo.len() > 1 => todo.iter().map(|n| vec![n.clone()]).collect(),
            Err(_) => vec![],
        };
        for run in &runs {
            match interpolate_glyphs(font, instance, options, run) {
                Ok(layers) => self.keep(instance, run, layers),
                Err(_) => self.instance_failed.extend(run.iter().map(|n| (n.clone(), instance))),
            }
        }
        if runs.is_empty() {
            self.instance_failed.extend(todo.into_iter().map(|n| (n, instance)));
        }
    }

    /// Keeps the layers of a run over `asked`; a glyph the run gave no layer is remembered as
    /// failed.
    fn keep(&mut self, instance: usize, asked: &[String], layers: Vec<(String, Layer)>) {
        let mut given = HashSet::new();
        for (name, layer) in layers {
            given.insert(name.clone());
            self.store((name, instance), layer);
        }
        for name in asked.iter().filter(|n| !given.contains(*n)) {
            self.instance_failed.insert((name.clone(), instance));
        }
    }

    fn store(&mut self, key: (String, usize), layer: Layer) -> Arc<Layer> {
        self.generation = self.generation.wrapping_add(1);
        let layer = Arc::new(layer);
        self.instance_order.push_back((key.clone(), self.generation));
        self.instance_layers.insert(key, (Arc::clone(&layer), self.generation));
        while self.instance_layers.len() > MAX_INSTANCE_LAYERS {
            let Some((oldest, generation)) = self.instance_order.pop_front() else { break };
            if self.instance_layers.get(&oldest).is_some_and(|(_, g)| *g == generation) {
                self.instance_layers.remove(&oldest);
            }
        }
        // Re-added and dropped layers leave stale entries behind; keep the queue near the map.
        if self.instance_order.len() > 2 * MAX_INSTANCE_LAYERS {
            let kept = &self.instance_layers;
            self.instance_order.retain(|(k, g)| kept.get(k).is_some_and(|(_, x)| x == g));
        }
        layer
    }

    /// The instance's master and kerning, interpolated from a copy of the font whose glyphs have
    /// no layers: the kerning groups are all the kerning needs from them.
    fn base(&mut self, font: &Font, instance: usize) -> Result<Arc<InstanceBase>, InterpError> {
        if let Some(b) = self.instance_bases.get(&instance) {
            return Ok(Arc::clone(b));
        }
        let options = options_for(font, instance)?;
        let mut shells = Font::clone(font);
        shells.glyphs = GlyphTable::from_glyphs(
            font.glyphs
                .iter()
                .map(|g| {
                    let mut shell = Glyph::new(g.name.clone());
                    shell.kern_groups = g.kern_groups.clone();
                    Arc::new(shell)
                })
                .collect(),
        );
        let (out, _) = instance_font_with_notes(&shells, instance, &options)?;
        let id = view_master_id(instance);
        let mut master = out.masters.first().cloned().ok_or_else(|| InterpError::BadMasterSetup("the instance has no master".into()))?;
        master.id.clone_from(&id);
        let mut kerning = Kerning::clone(&out.kerning);
        for table in [&mut kerning.ltr, &mut kerning.rtl, &mut kerning.vertical] {
            for m in &mut table.0 {
                m.master.clone_from(&id);
            }
        }
        let base = Arc::new(InstanceBase { master, kerning: Arc::new(kerning) });
        self.instance_bases.insert(instance, Arc::clone(&base));
        Ok(base)
    }

    /// Drops the entries of `glyphs`.
    pub fn invalidate(&mut self, glyphs: &[String]) {
        if glyphs.is_empty() {
            return;
        }
        let names: HashSet<&str> = glyphs.iter().map(String::as_str).collect();
        for g in glyphs {
            self.sources.remove(g);
            self.compat.remove(g);
        }
        self.instance_layers.retain(|(name, _), _| !names.contains(name.as_str()));
        self.instance_failed.retain(|(name, _)| !names.contains(name.as_str()));
        self.views.clear();
    }

    /// Drops everything.
    pub fn clear(&mut self) {
        self.font = None;
        self.designspace = None;
        self.sources.clear();
        self.compat.clear();
        self.scope = None;
        self.instance_layers.clear();
        self.instance_order.clear();
        self.instance_failed.clear();
        self.instance_bases.clear();
        self.views.clear();
        self.models.clear();
    }
}

/// `names` of `font` at `instance`, through the static instance of a copy of the font that holds
/// only those glyphs and no kerning. A glyph the instance leaves out has no entry.
fn interpolate_glyphs(font: &Font, instance: usize, options: &InterpOptions, names: &[String]) -> Result<Vec<(String, Layer)>, InterpError> {
    let mut reduced = Font::clone(font);
    reduced.glyphs = GlyphTable::from_glyphs(names.iter().filter_map(|n| font.glyphs.get(n).cloned()).collect());
    reduced.kerning = Arc::default();
    let (out, _) = instance_font_with_notes(&reduced, instance, options)?;
    Ok(out.glyphs.iter().filter_map(|g| g.layers.first().map(|l| (g.name.clone(), l.clone()))).collect())
}

fn same<T: PartialEq>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || a == b
}

/// Whether everything the designspace, the compatibility scope and the instance masters read
/// is the same in both fonts.
fn same_font_level(a: &Font, b: &Font) -> bool {
    a.units_per_em == b.units_per_em
        && same(&a.axes, &b.axes)
        && same(&a.masters, &b.masters)
        && same(&a.instances, &b.instances)
        && same(&a.info, &b.info)
        && same(&a.metrics, &b.metrics)
        && same(&a.stems, &b.stems)
        && same(&a.numbers, &b.numbers)
}

/// The glyphs that differ between two fonts.
struct Changed {
    /// Changed, added or removed.
    names: Vec<String>,
    /// Some kerning group changed with them.
    groups: bool,
}

fn changed_glyphs(old: &Font, new: &Font) -> Changed {
    let mut out = Changed { names: Vec::new(), groups: false };
    for g in new.glyphs.iter() {
        match old.glyphs.get(&g.name) {
            Some(o) if Arc::ptr_eq(o, g) => {}
            Some(o) => {
                out.groups |= o.kern_groups != g.kern_groups;
                out.names.push(g.name.clone());
            }
            None => {
                out.groups |= !g.kern_groups.is_empty();
                out.names.push(g.name.clone());
            }
        }
    }
    for o in old.glyphs.iter() {
        if new.glyphs.get(&o.name).is_none() {
            out.groups |= !o.kern_groups.is_empty();
            out.names.push(o.name.clone());
        }
    }
    out
}

/// The interpolation area's session slot: each open document's [`InterpCache`], by document id.
/// The caches are brought up to date with the font on every call.
#[derive(Default)]
struct InterpSlot(HashMap<u64, InterpCache>);

/// The active document's compatibility, designspace and instances, through its
/// [`InterpCache`].
pub trait SessionInterpExt {
    /// The compatibility of `glyph` in the active document; `None` when there is no document,
    /// no such glyph or no designspace to check against.
    fn glyph_compat(&mut self, glyph: &str) -> Option<Arc<Compat>>;

    /// Whether the active document shows incompatibility indicators: not while its Enforce
    /// Compatibility Check parameter is present and unchecked, nor without a designspace.
    fn compat_indicators(&mut self) -> bool;

    /// Whether glyph `glyph` shows the incompatibility indicators: its layers cannot
    /// interpolate and the font does not turn the indicators off.
    fn shows_incompatible(&mut self, glyph: &str) -> bool;

    /// The active document's glyphs whose layers cannot interpolate (empty without a
    /// designspace), which the Master Compatible smart-filter rule reads.
    fn incompatible_glyphs(&mut self) -> HashSet<String>;

    /// The active document's designspace.
    fn designspace(&mut self) -> Result<Arc<Designspace>, String>;

    /// Glyph `glyph` of the active document at the instance at `instance`
    /// ([`InterpCache::instance_layer`]).
    fn instance_layer(&mut self, glyph: &str, instance: usize) -> Result<Arc<Layer>, String>;

    /// The active document's view of the instance at `instance` holding `glyphs`
    /// ([`InterpCache::instance_view`]).
    fn instance_view(&mut self, instance: usize, glyphs: &[String]) -> Result<Arc<Font>, String>;
}

fn interp_cache(s: &mut crate::Session) -> Option<(&mut InterpCache, Arc<Font>, u64)> {
    let doc = s.doc().ok()?;
    let (id, font, revision) = (doc.id, Arc::clone(&doc.font), doc.revision);
    let open: Vec<u64> = s.documents().iter().map(|d| d.id).collect();
    let caches = &mut s.slot_mut::<InterpSlot>().0;
    caches.retain(|id, _| open.contains(id));
    Some((caches.entry(id).or_default(), font, revision))
}

impl SessionInterpExt for crate::Session {
    fn glyph_compat(&mut self, glyph: &str) -> Option<Arc<Compat>> {
        let (cache, font, revision) = interp_cache(self)?;
        cache.compat(&font, revision, glyph).ok()
    }

    fn compat_indicators(&mut self) -> bool {
        let Some((cache, font, revision)) = interp_cache(self) else { return false };
        cache.indicators(&font, revision).unwrap_or(false)
    }

    fn shows_incompatible(&mut self, glyph: &str) -> bool {
        let Some((cache, font, revision)) = interp_cache(self) else { return false };
        cache.indicators(&font, revision).unwrap_or(false) && cache.compat(&font, revision, glyph).is_ok_and(|c| !c.ok)
    }

    fn incompatible_glyphs(&mut self) -> HashSet<String> {
        let Some((cache, font, revision)) = interp_cache(self) else { return HashSet::new() };
        cache.incompatible(&font, revision).map(|v| v.into_iter().collect()).unwrap_or_default()
    }

    fn designspace(&mut self) -> Result<Arc<Designspace>, String> {
        let (cache, font, revision) = interp_cache(self).ok_or("no document open")?;
        cache.designspace(&font, revision).map_err(|e| e.to_string())
    }

    fn instance_layer(&mut self, glyph: &str, instance: usize) -> Result<Arc<Layer>, String> {
        let (cache, font, revision) = interp_cache(self).ok_or("no document open")?;
        cache.instance_layer(&font, revision, glyph, instance).map_err(|e| e.to_string())
    }

    fn instance_view(&mut self, instance: usize, glyphs: &[String]) -> Result<Arc<Font>, String> {
        let (cache, font, revision) = interp_cache(self).ok_or("no document open")?;
        cache.instance_view(&font, revision, instance, glyphs).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Axis, Instance, Path};

    use super::*;

    fn glyph(name: &str, thin: f64, black: f64) -> Arc<Glyph> {
        let mut g = Glyph::new(name);
        for (master, w) in [("thin", thin), ("black", black)] {
            let mut l = Layer::new(master, w + 100.0);
            l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, w, 700.0))];
            g.layers.push(l);
        }
        Arc::new(g)
    }

    /// Thin (100) and Black (900), an instance at 500; `a` 100 to 300 wide, `b` 200 to 600.
    fn font() -> Arc<Font> {
        let mut f = Font::new("Cache");
        f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
        f.masters = Arc::new(
            [("thin", 100.0), ("black", 900.0)]
                .iter()
                .map(|(id, w)| Master { id: (*id).into(), name: (*id).into(), location: vec![*w], ..Master::default() })
                .collect(),
        );
        f.instances = Arc::new(vec![Instance { name: "Medium".into(), location: vec![500.0], ..Instance::default() }]);
        f.glyphs = GlyphTable::from_glyphs(vec![glyph("a", 100.0, 300.0), glyph("b", 200.0, 600.0)]);
        Arc::new(f)
    }

    fn width(layer: &Layer) -> f64 {
        layer.width
    }

    #[test]
    fn an_edit_to_one_glyph_keeps_the_other_glyphs_entries() {
        let f = font();
        let mut cache = InterpCache::new();
        let a = cache.instance_layer(&f, 1, "a", 0).unwrap();
        let b = cache.instance_layer(&f, 1, "b", 0).unwrap();
        assert_eq!((width(&a), width(&b)), (300.0, 500.0));
        let compat_b = cache.compat(&f, 1, "b").unwrap();
        let ds = cache.designspace(&f, 1).unwrap();

        // Edit `a` only, as a command does: a new font sharing every other `Arc`.
        let mut edited = Font::clone(&f);
        let mut g = Glyph::clone(edited.glyphs.get("a").unwrap());
        g.layers[1].width = 600.0;
        let at = edited.glyphs.position("a").unwrap();
        let mut glyphs: Vec<Arc<Glyph>> = edited.glyphs.iter().cloned().collect();
        glyphs[at] = Arc::new(g);
        edited.glyphs = GlyphTable::from_glyphs(glyphs);
        let edited = Arc::new(edited);

        let a2 = cache.instance_layer(&edited, 2, "a", 0).unwrap();
        assert_eq!(width(&a2), 400.0, "the edited glyph is interpolated again");
        assert!(Arc::ptr_eq(&cache.instance_layer(&edited, 2, "b", 0).unwrap(), &b), "the other glyph's layer is kept");
        assert!(Arc::ptr_eq(&cache.compat(&edited, 2, "b").unwrap(), &compat_b));
        assert!(Arc::ptr_eq(&cache.designspace(&edited, 2).unwrap(), &ds), "the designspace is kept");

        // A change to the masters drops everything.
        let mut moved = Font::clone(&edited);
        Arc::make_mut(&mut moved.masters)[1].location = vec![700.0];
        let moved = Arc::new(moved);
        assert!(!Arc::ptr_eq(&cache.designspace(&moved, 3).unwrap(), &ds));
        assert!(!Arc::ptr_eq(&cache.instance_layer(&moved, 3, "b", 0).unwrap(), &b));
    }

    #[test]
    fn an_instance_view_holds_the_glyphs_and_their_components_under_its_own_master() {
        let mut f = Font::clone(&font());
        let mut composite = Glyph::new("c");
        for master in ["thin", "black"] {
            let mut l = Layer::new(master, 500.0);
            l.shapes = vec![Shape::Component(fontcraft_model::Component::new("a", 0.0, 0.0))];
            composite.layers.push(l);
        }
        f.glyphs.push(composite);
        let f = Arc::new(f);
        let mut cache = InterpCache::new();
        let view = cache.instance_view(&f, 1, 0, &["c".to_string()]).unwrap();
        let id = view_master_id(0);
        assert_eq!(view.masters.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
        assert_eq!(view.glyphs.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["c", "a"]);
        assert_eq!(view.glyph("a").unwrap().layer(&id).unwrap().width, 300.0);
        assert!(Arc::ptr_eq(&cache.instance_view(&f, 1, 0, &["c".to_string()]).unwrap(), &view), "the view is kept");
        assert!(cache.instance_view(&f, 1, 5, &["c".to_string()]).is_err());
    }
}
