//! Automatic feature code (M8.9; `docs/spec/12-feature-code.md` § 8): the prefixes, classes and
//! features Update writes from the glyph names, and the merge rules that decide what Update
//! keeps.
//!
//! Rows: `autoFeature.generalRule`, `autoFeature.languageSystems`, `autoClass.general`,
//! `fontInfo.features.updateDetectsNew`, `.optionalAutomatic`, `.manualNotUpdated`,
//! `glyphName.chainedSuffixes` (ordering), `Update Features`; each generator module serves the
//! `autoFeature.<tag>` rows of its tags.
//!
//! Contract:
//! - A [`Generator`] reads an [`AutoContext`] (the exporting glyphs in order, with their
//!   categories, scripts and code points, and the existing entries) and returns the entries it
//!   would write ([`AutoEntry`]), each with code of at most [`MAX_GENERATED_BYTES`]. It never
//!   reads the font directly, so the same generator serves Update and each export target.
//! - Each generator module has a `generators()` function listing its generators;
//!   [`generators`] joins them. Package agents fill their own module and leave this file alone.
//! - [`update`] applies the merge rules: manual entries kept; automatic entries replaced; new
//!   entries added in the reference's tag order; optional entries written only when present.

mod aalt;
mod ccmp;
mod classes;
mod cpsp;
mod figures;
mod fractions;
mod indic;
mod langsys;
mod ligatures;
mod locl;
mod suffix;

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Features, Font};

use crate::FeatureError;
use crate::source_map::{EntryKind, EntryRef};

pub use locl::locl_languages;

/// Most bytes of code one generated entry may hold.
pub const MAX_GENERATED_BYTES: usize = 4 << 20;

/// One glyph as the generators see it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoGlyph {
    pub name: String,
    pub codepoints: Vec<u32>,
    /// The effective category and subcategory (`Letter`, `Lowercase`), from the glyph or the
    /// glyph database.
    pub category: Option<String>,
    pub sub_category: Option<String>,
    /// The effective script (`latin`, `devanagari`).
    pub script: Option<String>,
}

/// What the generators read: the target's exporting glyphs in the font's order and the entries
/// the font has.
#[derive(Clone, Debug, Default)]
pub struct AutoContext {
    glyphs: Vec<AutoGlyph>,
    index: HashMap<String, usize>,
    existing: Arc<Features>,
}

impl AutoContext {
    /// Refused: more than `fontcraft_model::MAX_GLYPHS` glyphs.
    pub fn new(glyphs: Vec<AutoGlyph>, existing: Arc<Features>) -> Result<Self, FeatureError> {
        if glyphs.len() > fontcraft_model::MAX_GLYPHS {
            return Err(FeatureError::Limit { what: "glyphs", limit: fontcraft_model::MAX_GLYPHS });
        }
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        Ok(Self { glyphs, index, existing })
    }

    /// The context of `font`'s exporting glyphs (or of `glyphs`, an export target's glyph set,
    /// when given), with properties from `data`.
    pub fn from_font(font: &Font, data: &GlyphData, glyphs: Option<&[String]>) -> Result<Self, FeatureError> {
        // M8.9
        let _ = (data, glyphs);
        Self::new(Vec::new(), font.features.clone())
    }

    pub fn glyphs(&self) -> &[AutoGlyph] {
        &self.glyphs
    }

    pub fn glyph(&self, name: &str) -> Option<&AutoGlyph> {
        self.index.get(name).and_then(|&i| self.glyphs.get(i))
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn existing(&self) -> &Features {
        &self.existing
    }
}

/// An entry a generator writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoEntry {
    pub kind: EntryKind,
    /// The feature tag, or the class or prefix name.
    pub name: String,
    pub code: String,
    /// Written by Update only when the font already has the entry (`fontInfo.features.optionalAutomatic`).
    pub optional: bool,
}

/// One generator of automatic code.
pub trait Generator: Sync {
    /// The entry names (feature tags, class or prefix names) it may write, in the reference's
    /// order.
    fn names(&self) -> &'static [&'static str];

    /// The entries it writes for `cx`; none when the glyphs do not call for them.
    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError>;
}

/// Every generator, in the order Update runs them.
pub fn generators() -> Vec<&'static dyn Generator> {
    let mut all = Vec::new();
    all.extend(langsys::generators());
    all.extend(classes::generators());
    all.extend(aalt::generators());
    all.extend(ccmp::generators());
    all.extend(locl::generators());
    all.extend(suffix::generators());
    all.extend(figures::generators());
    all.extend(fractions::generators());
    all.extend(ligatures::generators());
    all.extend(indic::generators());
    all.extend(cpsp::generators());
    all
}

/// What [`update`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateReport {
    /// The entries after the update.
    pub features: Features,
    /// Entries added, as (kind, name).
    pub added: Vec<(EntryKind, String)>,
    /// Automatic entries whose code changed.
    pub updated: Vec<(EntryKind, String)>,
    /// Entries Update recognised but does not generate, with the reason.
    pub not_generated: Vec<(String, String)>,
}

/// Update: every generator run on `cx`, merged into `entries` by the rules above.
pub fn update(entries: &Features, cx: &AutoContext) -> Result<UpdateReport, FeatureError> {
    // M8.9
    let _ = cx;
    Ok(UpdateReport { features: entries.clone(), ..UpdateReport::default() })
}

/// Update Entry: one automatic entry regenerated. Refused: a manual entry, or one no generator
/// writes.
pub fn update_entry(entries: &Features, entry: EntryRef, cx: &AutoContext) -> Result<Features, FeatureError> {
    // M8.9
    let _ = (entry, cx);
    Ok(entries.clone())
}
