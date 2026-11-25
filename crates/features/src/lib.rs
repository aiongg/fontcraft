//! FontCraft features (L2): the generated pair kerning a compiled font gets, as a `GPOS` table,
//! the mark writer ([`marks`], M6.17), and feature code made ready for the compiler (M8).
//!
//! Feature code, for one [`Target`] (`docs/plans/M8.md` § 3, `docs/spec/12-feature-code.md` § 1):
//!
//! | Step | Module | Package |
//! |---|---|---|
//! | Automatic entries regenerated, Update and Update Entry | [`auto`] (generators in its submodules) | M8.9–M8.13 |
//! | Instance parameters applied to the entries | [`params`] | M8.3 |
//! | Tokens expanded against the target's values | [`tokens`], [`tokens_predicate`], [`target`] | M8.5, M8.6 |
//! | Syntax extensions and `#ifdef VARIABLE` lowered | [`lower`] | M8.4 |
//! | The variable syntax lowered (variable target) | [`variable`] | M9.11 |
//! | Entries assembled into virtual files with a source map | [`assemble`], [`source_map`] | M8.3 |
//! | Implicit features in neutral form | [`implicit`] | M8.7 |
//! | Compiling | `fontcraft_fea::compile` | M8.2 |
//!
//! Beside the pipeline: editor helpers ([`edit`], M8.15), snippets ([`snippets`], M8.16) and the
//! registered feature tags ([`registry`], M8.14). Every pass reads untrusted code within
//! [`MAX_ENTRY_BYTES`] per entry and [`MAX_ENTRIES`] entries, and reports problems as
//! diagnostics with byte spans, never a panic.
//!
//! The pair kerning:
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
//! 6. Vertical kerning becomes one `vkrn` lookup, in the default script, every script that has
//!    generated kerning, and the code's language systems of the scripts of the kerned glyphs. The
//!    reference compilers write none; the reference app does.
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

pub mod assemble;
pub mod auto;
pub mod edit;
pub mod evaluate;
pub mod fea_variable;
mod gpos;
pub mod implicit;
mod kern;
pub mod lower;
pub mod marks;
pub mod optical;
pub mod params;
mod properties;
pub mod registry;
mod resolve;
pub mod snippets;
pub mod source_map;
pub mod target;
pub mod tokens;
pub mod tokens_predicate;
mod unicode18;
pub mod var_info;
pub mod var_layout;
pub mod variable;
mod writers;

use std::collections::HashMap;

pub use assemble::{Assembly, assemble};
use fontcraft_model::Font;
use fontcraft_spacing::kerning::KernIndex;
pub use source_map::{EntryDiagnostic, EntryKind, EntryRange, EntryRef, OffsetMap, SourceMap};
pub use target::{GlyphMetric, MasterValues, Target, TargetValues};
use write_fonts::tables::gpos::Gpos;
use write_fonts::types::{GlyphId16, Tag};
pub use writers::{FEATURE_WRITERS_KEY, KernTags, append_tags, kern_tags};

/// Most bytes of code one prefix, class or feature may hold.
pub const MAX_ENTRY_BYTES: usize = 4 << 20;
/// Most prefixes, classes and features one font may hold together (the UFO feature splitter's
/// cap).
pub const MAX_ENTRIES: usize = 100_000;

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
    /// A count or size of the feature code beyond its cap.
    #[error("the feature code has more than {limit} {what}")]
    Limit { what: &'static str, limit: usize },
    /// The code has errors, mapped to the entries that hold them; never empty.
    #[error("{}", source_map::summary(.0))]
    Code(Vec<EntryDiagnostic>),
    /// A target or request this build cannot serve yet (an instance, the variable font).
    #[error("{0}")]
    Unsupported(String),
    /// A function whose M9 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
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
    /// The feature code's language systems, as (script, language) in code order
    /// ([`fontcraft_fea::FirstPass::language_systems`]); empty without feature code.
    pub language_systems: &'a [(Tag, Tag)],
    /// The feature code's `GSUB` alone ([`fontcraft_fea::FirstPass::gsub`]): unencoded glyphs it
    /// reaches take the scripts and directions of the glyphs they come from.
    pub gsub: Option<&'a [u8]>,
    /// The variable font's masters, `master` (the origin) first: every master's pairs are
    /// written, each value as the origin's plus deltas. `None` for a static font.
    pub variable: Option<&'a var_layout::VarLayout>,
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
    if req.variable.is_some() {
        return Err(FeatureError::Unsupported("variable kerning goes through the feature compiler, which writes the variation store".into()));
    }
    let (kerns, warnings) = kern_rules(req)?;
    let lookups = kerns.lookups.len();
    let gpos = gpos::build(kerns)?;
    Ok(KernFeature { lookups: if gpos.is_some() { lookups } else { 0 }, gpos, warnings })
}

/// `req.master`'s kerning in the feature compiler's neutral form, with the warnings: each lookup
/// that skips marks gets a mark filtering set of the spacing marks when the font has any, as
/// fontc writes it beside a `GDEF` table.
pub fn kern_lookups(req: &KernRequest) -> Result<(fontcraft_fea::GeneratedLookups, Vec<String>), FeatureError> {
    let (kerns, warnings) = kern_rules(req)?;
    let mut spacing_marks: Vec<GlyphId16> = req
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, g)| g.mark == Some(MarkSpacing::Spacing))
        .filter_map(|(i, _)| u16::try_from(i).ok().map(GlyphId16::new))
        .collect();
    spacing_marks.sort_unstable();
    let mut out = fontcraft_fea::GeneratedLookups { axes: req.variable.map(|v| v.axes().to_vec()).unwrap_or_default(), ..Default::default() };
    for lookup in kerns.lookups {
        let (flags, filter_set) = match (lookup.ignore_marks, spacing_marks.is_empty()) {
            (true, true) => (fontcraft_fea::LookupFlags::IGNORE_MARKS, None),
            (true, false) => (0, Some(spacing_marks.clone())),
            (false, _) => (0, None),
        };
        out.lookups.push(fontcraft_fea::GeneratedLookup {
            class: String::new(),
            flags: fontcraft_fea::LookupFlags(flags),
            filter_set,
            rules: fontcraft_fea::GeneratedRules::Pair { glyph_pairs: lookup.glyph_pairs, class_pairs: lookup.class_pairs },
        });
    }
    for (key, indices) in kerns.features {
        out.features.insert(fontcraft_fea::FeatureKey { feature: key.feature, script: key.script, language: key.language }, indices);
    }
    Ok((out, warnings))
}

/// The kern writer's lookups and features, with the warnings.
fn kern_rules(req: &KernRequest) -> Result<(kern::Kerns, Vec<String>), FeatureError> {
    let mut budget = Budget::new(req.limits.work);
    let mut warnings = Vec::new();
    if !req.tags.any() {
        return Ok((kern::Kerns::default(), warnings));
    }
    let horizontal = req.tags.kern || req.tags.dist;
    let mut pairs = if horizontal { resolve::kern_pairs(req, false, &mut budget, &mut warnings)? } else { Vec::new() };
    let mut vertical = if req.tags.vkrn { resolve::kern_pairs(req, true, &mut budget, &mut warnings)? } else { Vec::new() };
    if pairs.is_empty() && vertical.is_empty() {
        return Ok((kern::Kerns::default(), warnings));
    }
    pairs.sort();
    vertical.sort();

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

    let code = kern::KernCode::new(req.language_systems, req.gsub);
    let mut skipped = kern::Skipped::default();
    let axes = req.variable.map(|v| v.axes().to_vec()).unwrap_or_default();
    let kerns = kern::finalize_kerning(&pairs, &vertical, &code, &char_map, marks, req.tags, &req.limits, &mut budget, &mut skipped, &axes)?;
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
    Ok((kerns, warnings))
}

#[cfg(test)]
mod tests;
