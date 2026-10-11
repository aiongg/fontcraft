//! FontCraft features (L2): the generated pair kerning a compiled font gets, as a `GPOS` table.
//! Feature-code compiling, the other automatic features and the mark writer follow in M8.
//!
//! [`kern_feature`] writes one master's kerning (`compile.kernValueLookup`,
//! `compile.kernFeatureWriter`, `kerning.implicitKernFeature`, `autoFeature.kern`,
//! `autoFeature.dist`) as fontc's kern writer does, which follows ufo2ft's:
//!
//! 1. The master's stored pairs (those of the master it follows when its metrics are linked),
//!    left-to-right and right-to-left in one table in logical order (`resolve`).
//! 2. Every rule resolved to the value that applies to it: the pair, then each glyph side as
//!    itself and as its group, then 0. A glyph–group pair becomes one glyph pair per member; a
//!    group–group pair of 0 is left out; a zero exception stays. Values are rounded half up.
//! 3. Rules split by the scripts of their glyphs (from `cmap` and the Unicode Script_Extensions)
//!    into one lookup per group of scripts, rules with marks in lookups of their own; a rule
//!    whose glyphs mix strong directions is left out; right-to-left rules also move the glyph
//!    (`kern`).
//! 4. Each lookup goes to the `kern` feature of its scripts, or to `dist` for the Indic and
//!    Universal Shaping Engine scripts, Khmer and Myanmar; the default script gets the common
//!    lookups and those of the left-to-right scripts. Glyph pairs come before class pairs in
//!    each lookup; offsets that overflow are resolved when write-fonts writes the table
//!    (`gpos`).
//!
//! The source's feature-writer setting ([`kern_tags`]) can turn `kern` or `dist` off.
//!
//! Vertical kerning is not written: neither reference compiler writes `vkrn`.
//!
//! [`evaluate::PairEvaluator`] reads the pair kerning of a compiled font back, for tests and the
//! oracle comparison.
//!
//! Every count the work grows with is capped ([`KernLimits`]): the rules, the glyph pairs handed
//! to the table builder, the classes of a lookup, the class-table cells, the groups of scripts,
//! and the work as a whole. Each cap is checked before the work it bounds. The class-table cells
//! are counted from the builder's subtables once a lookup's rules are in, before the build
//! allocates the tables; the rules themselves are bounded by the rule and glyph-pair caps.
#![forbid(unsafe_code)]

pub mod evaluate;
mod gpos;
mod kern;
mod properties;
mod resolve;
mod unicode18;
mod writers;

use std::collections::HashMap;

use fontcraft_model::Font;
use fontcraft_spacing::kerning::KernIndex;
use write_fonts::tables::gpos::Gpos;
use write_fonts::types::GlyphId16;
pub use writers::{FEATURE_WRITERS_KEY, KernTags, kern_tags};

/// Most rules the kerning of one master may resolve to, glyph–group pairs expanded to their
/// members.
pub const MAX_KERN_RULES: usize = 2_000_000;
/// Most glyph pairs the lookups of one master may hold, group–glyph rules expanded to their
/// members. The table builder keeps about 400 bytes for each, so the cap bounds it at about
/// 200 MB; the largest corpus font, Inter, has 11,442.
pub const MAX_GLYPH_PAIRS: usize = 500_000;
/// Most cells of the class tables of one lookup: in each class subtable, first classes times
/// second classes plus one. The built table holds a 112-byte record for every cell.
pub const MAX_CLASS_CELLS: usize = 1_000_000;
/// Most class-table cells of all the lookups of one master together: about 220 MB of records.
/// Inter, the largest corpus font, has 25,317.
pub const MAX_TOTAL_CLASS_CELLS: usize = 2_000_000;
/// Most first classes, and most second classes, of one lookup: a class definition numbers its
/// classes with 16 bits, class 0 included.
pub const MAX_CLASSES_PER_LOOKUP: usize = 65_534;
/// Most groups of scripts the rules may fall into before they are merged.
pub const MAX_SCRIPT_SETS: usize = 1_000;
/// Most steps (pairs read, group members visited or copied, rules added, class cells) the
/// writer may take for one master.
pub const MAX_KERN_WORK: usize = 100_000_000;

/// The caps [`kern_feature`] works under. The default is the `MAX_…` constants; smaller caps
/// are for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernLimits {
    pub rules: usize,
    pub glyph_pairs: usize,
    pub class_cells: usize,
    pub total_class_cells: usize,
    pub classes_per_lookup: usize,
    pub script_sets: usize,
    pub work: usize,
}

impl Default for KernLimits {
    fn default() -> Self {
        Self {
            rules: MAX_KERN_RULES,
            glyph_pairs: MAX_GLYPH_PAIRS,
            class_cells: MAX_CLASS_CELLS,
            total_class_cells: MAX_TOTAL_CLASS_CELLS,
            classes_per_lookup: MAX_CLASSES_PER_LOOKUP,
            script_sets: MAX_SCRIPT_SETS,
            work: MAX_KERN_WORK,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FeatureError {
    #[error("the kerning has more than {limit} {what}")]
    TooMany { what: &'static str, limit: usize },
    #[error("the kerning cannot be written: {0}")]
    Kerning(String),
}

/// A mark glyph: one with an advance (spacing) or without.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkSpacing {
    Spacing,
    NonSpacing,
}

/// One glyph of the compiled font, at its glyph id.
#[derive(Clone, Copy, Debug)]
pub struct KernGlyph<'a> {
    pub name: &'a str,
    pub codepoints: &'a [u32],
    /// `Some` for a glyph of the mark class.
    pub mark: Option<MarkSpacing>,
}

/// What [`kern_feature`] writes from.
pub struct KernRequest<'a> {
    pub font: &'a Font,
    /// The font's kerning index ([`KernIndex::new`]).
    pub index: &'a KernIndex,
    /// The master compiled.
    pub master: &'a str,
    /// The glyphs in the compiled font's order: a glyph's index is its glyph id.
    pub glyphs: &'a [KernGlyph<'a>],
    /// The features the source's feature-writer setting asks for ([`kern_tags`]).
    pub tags: KernTags,
    pub limits: KernLimits,
}

/// The generated kerning.
#[derive(Debug)]
pub struct KernFeature {
    /// `None` when there is no kerning to write.
    pub gpos: Option<Gpos>,
    /// How many lookups the table has.
    pub lookups: usize,
    /// What the user should know: left-out rules, values that did not fit.
    pub warnings: Vec<String>,
}

/// What the writer may still do ([`KernLimits::work`]).
pub(crate) struct Budget {
    left: usize,
    limit: usize,
}

impl Budget {
    pub(crate) fn new(limit: usize) -> Self {
        Self { left: limit, limit }
    }

    pub(crate) fn spend(&mut self, n: usize) -> Result<(), FeatureError> {
        self.left = self.left.checked_sub(n).ok_or(FeatureError::TooMany { what: "steps of work", limit: self.limit })?;
        Ok(())
    }

    /// Spends one step for each glyph of a side.
    pub(crate) fn spend_glyphs(&mut self, n: u64) -> Result<(), FeatureError> {
        self.spend(usize::try_from(n).unwrap_or(usize::MAX))
    }
}

/// The `GPOS` table of `req.master`'s kerning.
pub fn kern_feature(req: &KernRequest) -> Result<KernFeature, FeatureError> {
    let mut budget = Budget::new(req.limits.work);
    let mut warnings = Vec::new();
    if !req.tags.any() {
        return Ok(KernFeature { gpos: None, lookups: 0, warnings });
    }
    let mut pairs = resolve::kern_pairs(req, &mut budget, &mut warnings)?;
    if pairs.is_empty() {
        return Ok(KernFeature { gpos: None, lookups: 0, warnings });
    }
    pairs.sort();

    let mut char_map: HashMap<u32, GlyphId16> = HashMap::new();
    let mut marks: HashMap<GlyphId16, MarkSpacing> = HashMap::new();
    for (i, g) in req.glyphs.iter().enumerate() {
        let Ok(id) = u16::try_from(i) else { break };
        let gid = GlyphId16::new(id);
        for cp in g.codepoints {
            char_map.entry(*cp).or_insert(gid);
        }
        if let Some(spacing) = g.mark {
            marks.insert(gid, spacing);
        }
    }

    let mut skipped = kern::Skipped::default();
    let kerns = kern::finalize_kerning(&pairs, &char_map, marks, req.tags, &req.limits, &mut budget, &mut skipped)?;
    if skipped.ambiguous_direction > 0 {
        warnings.push(format!(
            "{} kerning rules were left out: their glyphs mix left-to-right and right-to-left characters",
            skipped.ambiguous_direction
        ));
    }
    if skipped.mixed_direction > 0 {
        warnings
            .push(format!("{} parts of kerning rules were left out: their sides belong to scripts of opposite directions", skipped.mixed_direction));
    }
    let lookups = kerns.lookups.len();
    let gpos = gpos::build(kerns)?;
    Ok(KernFeature { lookups: if gpos.is_some() { lookups } else { 0 }, gpos, warnings })
}

#[cfg(test)]
mod tests;
