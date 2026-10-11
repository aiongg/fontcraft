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

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_interp::{Compat, Designspace, GlyphSources, InterpError, ModelCache};
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
    pub fn compat(&mut self, font: &Arc<Font>, revision: u64, glyph: &str) -> Result<Arc<Compat>, InterpError> {
        self.sync(font, revision);
        // M9.22
        let _ = glyph;
        Err(InterpError::NotYetImplemented("the compatibility cache"))
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
        self.instance_layers.clear();
        self.models.clear();
    }
}
