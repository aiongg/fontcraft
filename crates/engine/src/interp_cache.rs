//! Interpolation results kept between commands and frames (M9.20; M9.22 reads the compatibility
//! table).
//!
//! Contract (`docs/plans/M9.md` § 3 › Caching): one cache per document. It holds the
//! [`Designspace`], each glyph's sources and compatibility, and the instance layers the preview
//! draws, for one font revision. When the revision changes, entries of glyphs whose `Arc` changed
//! are dropped and the designspace is rebuilt only when the axes, masters, instances or font
//! parameters changed. Errors are not cached. The instance layers kept are capped at
//! [`MAX_INSTANCE_LAYERS`]; past it the oldest are dropped.
//!
//! Who uses it: the preview of instances and the master switcher (M9.20), the compatibility
//! indicators and the Master Compatibility view (M9.22). Commands may build a designspace
//! directly; the cache only saves work. It is to become a field of the document state once its
//! first user lands.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_interp::compat::{check_glyph, scope};
use fontcraft_interp::{Compat, Designspace, GlyphSources, InterpError, MasterPairs, ModelCache};
use fontcraft_model::{Font, Layer};

/// Most interpolated instance layers kept.
pub const MAX_INSTANCE_LAYERS: usize = 100_000;

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
    /// Keyed by glyph name and instance index.
    instance_layers: HashMap<(String, usize), Arc<Layer>>,
    models: ModelCache,
}

impl InterpCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the cache to `font` at `revision`, dropping what the change made stale.
    pub fn sync(&mut self, font: &Arc<Font>, revision: u64) {
        // M9.20: compare glyph and font-level pointers with the kept font; drop only what changed.
        if self.font.as_ref().is_some_and(|(f, r)| *r == revision && Arc::ptr_eq(f, font)) {
            return;
        }
        self.clear();
        self.font = Some((Arc::clone(font), revision));
    }

    /// The designspace of `font`.
    pub fn designspace(&mut self, font: &Arc<Font>, revision: u64) -> Result<Arc<Designspace>, InterpError> {
        self.sync(font, revision);
        // M9.20
        Err(InterpError::NotYetImplemented("the designspace cache"))
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
        let ds = match &self.designspace {
            Some(ds) => Arc::clone(ds),
            None => {
                let ds = Arc::new(Designspace::new(font)?);
                self.designspace = Some(Arc::clone(&ds));
                ds
            }
        };
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

    /// `glyph` interpolated at the instance at `instance` (an index into `Font::instances`).
    pub fn instance_layer(&mut self, font: &Arc<Font>, revision: u64, glyph: &str, instance: usize) -> Result<Arc<Layer>, InterpError> {
        self.sync(font, revision);
        // M9.20
        let _ = (glyph, instance, MAX_INSTANCE_LAYERS);
        Err(InterpError::NotYetImplemented("the instance layer cache"))
    }

    /// Drops the entries of `glyphs`.
    pub fn invalidate(&mut self, glyphs: &[String]) {
        for g in glyphs {
            self.sources.remove(g);
            self.compat.remove(g);
            self.instance_layers.retain(|(name, _), _| name != g);
        }
    }

    /// Drops everything.
    pub fn clear(&mut self) {
        self.font = None;
        self.designspace = None;
        self.sources.clear();
        self.compat.clear();
        self.scope = None;
        self.instance_layers.clear();
        self.models.clear();
    }
}

/// The active document's compatibility, through its [`InterpCache`]. The cache is kept per open
/// document and brought up to date with the font on every call.
impl crate::Session {
    fn interp_cache(&mut self) -> Option<(&mut InterpCache, Arc<Font>, u64)> {
        let doc = self.doc().ok()?;
        let (id, font, revision) = (doc.id, Arc::clone(&doc.font), doc.revision);
        let open: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.interp.retain(|id, _| open.contains(id));
        Some((self.interp.entry(id).or_default(), font, revision))
    }

    /// The compatibility of `glyph` in the active document; `None` when there is no document,
    /// no such glyph or no designspace to check against.
    pub fn glyph_compat(&mut self, glyph: &str) -> Option<Arc<Compat>> {
        let (cache, font, revision) = self.interp_cache()?;
        cache.compat(&font, revision, glyph).ok()
    }

    /// Whether the active document shows incompatibility indicators: not while its Enforce
    /// Compatibility Check parameter is present and unchecked, nor without a designspace.
    pub fn compat_indicators(&mut self) -> bool {
        let Some((cache, font, revision)) = self.interp_cache() else { return false };
        cache.indicators(&font, revision).unwrap_or(false)
    }

    /// Whether glyph `glyph` shows the incompatibility indicators: its layers cannot
    /// interpolate and the font does not turn the indicators off.
    pub fn shows_incompatible(&mut self, glyph: &str) -> bool {
        let Some((cache, font, revision)) = self.interp_cache() else { return false };
        cache.indicators(&font, revision).unwrap_or(false) && cache.compat(&font, revision, glyph).is_ok_and(|c| !c.ok)
    }

    /// The active document's glyphs whose layers cannot interpolate (empty without a
    /// designspace), which the Master Compatible smart-filter rule reads.
    pub fn incompatible_glyphs(&mut self) -> HashSet<String> {
        let Some((cache, font, revision)) = self.interp_cache() else { return HashSet::new() };
        cache.incompatible(&font, revision).map(|v| v.into_iter().collect()).unwrap_or_default()
    }
}
