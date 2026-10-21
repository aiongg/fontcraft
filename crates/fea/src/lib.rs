//! FontCraft's feature compiler (L0, standalone): one OpenType feature file, assembled from
//! virtual files, compiled to `GSUB`, `GPOS`, `GDEF`, `BASE` and the values of the table blocks,
//! with generated lookups (kerning, mark attachment, feature variations) inserted where the code
//! asks for them.
//!
//! The compiler is fea-rs 1.0.0 (decision 0002; `docs/plans/M8.md` § 3). fea-rs is built on
//! `write-fonts` 0.52 and the workspace on 0.54, so this crate is the only one that sees 0.52
//! (the renamed `write-fonts-052` dependency). Nothing of fea-rs or of `write-fonts` 0.52 is in
//! the public interface: tables leave as bytes ([`Compiled`]), which the compiler adds with
//! `FontBuilder::add_raw`; generated lookups enter as this crate's own plain data
//! ([`GeneratedLookups`]); variation data enters through the [`FeaVariations`] trait. The two
//! exceptions are [`Tag`] and [`GlyphId16`]: they come from `font-types`, which both
//! `write-fonts` versions share, so they are the workspace's types too.
//!
//! Rows: `compile.featureCompilation`, `compile.expandFeatureIncludes`, `compile.layoutPacking`,
//! the `fea.*` rows of `spec/inventory/12b-feature-language.toml` (M8.2); the output side of
//! `fea.table.*`, `compile.nameFeaOverrides` and `compile.feaTableOverrides` (M8.8).
//!
//! Contract:
//! - [`compile`] takes a [`CompileRequest`]: the files ([`VirtualFiles`]: a root file and the
//!   files it includes, by path, plus an optional folder for includes read from disk), the glyph
//!   order, the generated lookups, the variation data (`None` for a static font) and the
//!   [`Limits`].
//! - Everything in the request is untrusted. Before fea-rs sees it, the request is checked
//!   against the limits: text size, included files and their depth, brace and bracket depth,
//!   then, on fea-rs's parse tree, class sizes, ligature lengths and the rules the code expands
//!   to (counted with saturation). `validate` and `compile` then run on a thread with a
//!   [`STACK_SIZE`] stack on native targets, under the caller's panic guard. On wasm, where a
//!   panic aborts, the same checks run with the lower caps of [`Limits::WASM`].
//! - A failure is a list of [`Diagnostic`]s, each naming a file of the request and a byte range
//!   in it (or no file, for a refusal of the request as a whole). Nothing panics, and a failed
//!   compile returns no tables.
//! - Generated lookups go to fea-rs's feature writer interface: at the `# Automatic Code` marker
//!   of their feature when the code has one, else after the hand-written lookups, as fontc
//!   places them ([`GeneratedLookups::append`] forces the second).
//! - `GDEF` comes from the code's `table GDEF` block when it has one ([`Compiled::gdef_from_code`]),
//!   else from the generated glyph classes and carets, as fontc does.
//! - Name ids of 256 and above that the code declares move to start at
//!   [`CompileRequest::first_name_id`], everywhere they are referenced, so they cannot collide
//!   with the source's own names.
//!
//! - A variable font: [`FeaVariations`] gives fea-rs the axes, masters and number values, and
//!   resolves the code's variable values (`fontcraft_features::var_info`, M9.12);
//!   [`GeneratedLookups::feature_variations`] holds the alternate layers' substitutions (M9.8)
//!   and [`Metric`] deltas the variable kerning and marks (M9.9). One variation store holds the
//!   code's and the generated deltas.
#![forbid(unsafe_code)]

mod check;
mod diagnostic;
mod files;
mod generated;
mod glyphs;
mod limits;
mod output;
mod provider;
mod resolve;
#[cfg(test)]
mod rows_tests;
mod run;
mod split;
mod variations;

pub use diagnostic::{Diagnostic, Severity};
pub use files::{IncludeCheck, VirtualFiles};
pub use generated::{
    Anchor, ClassPair, Condition, FeatureKey, FeatureVariation, GeneratedLookup, GeneratedLookups, GeneratedRules, GlyphClass, GlyphPair,
    LookupFlags, Metric, Region, RegionAxis, Value,
};
pub use glyphs::GlyphOrder;
pub use limits::{
    Limits, MAX_AXES, MAX_DELTAS, MAX_DIAGNOSTICS, MAX_FEATURE_VARIATIONS, MAX_GENERATED_LOOKUPS, MAX_GENERATED_RULES, MAX_GLYPHS, MAX_INCLUDE_DEPTH,
    MAX_INCLUDE_FILES, MAX_LOCATIONS, MAX_NESTING_DEPTH, MAX_PATH_BYTES, STACK_SIZE,
};
pub use output::{Compiled, FeatureSummary, FirstPass, HeadValues, LayoutTable, MetricsValues, Os2Values};
pub use variations::{Axis, FeaVariations, Location};
pub use write_fonts_052::types::{GlyphId16, Tag};

/// What [`compile`] and [`validate`] work on.
#[derive(Clone, Copy)]
pub struct CompileRequest<'a> {
    pub files: &'a VirtualFiles,
    pub glyph_order: &'a GlyphOrder,
    /// The implicit features (kerning, marks) and feature variations to merge with the code;
    /// `None` adds nothing.
    pub generated: Option<&'a GeneratedLookups>,
    /// Axes, master locations and number values of a variable font; `None` for a static font.
    pub variations: Option<&'a dyn FeaVariations>,
    /// The first name id free in the font's `name` table: name ids of 256 and above that the
    /// code declares are moved up to start there. 256 moves nothing.
    pub first_name_id: u16,
    /// `false` leaves `GPOS` out of the result (the Edit View preview font, which shapes with
    /// substitutions only).
    pub gpos: bool,
    /// Each run of a feature block's substitutions of one type gets its own lookup, as the
    /// feature file specification groups rules; `false` lets a single substitution join a
    /// neighbouring multiple or ligature substitution's lookup, as fea-rs and feaLib do.
    pub lookup_per_rule_type: bool,
    pub limits: Limits,
}

impl<'a> CompileRequest<'a> {
    /// A request for a static font with every table, no generated lookups and the default limits.
    pub fn new(files: &'a VirtualFiles, glyph_order: &'a GlyphOrder) -> Self {
        Self {
            files,
            glyph_order,
            generated: None,
            variations: None,
            first_name_id: 256,
            gpos: true,
            lookup_per_rule_type: false,
            limits: Limits::default(),
        }
    }
}

/// Compiles the request's files to layout tables. Errors and warnings fea-rs reports, and
/// refusals of the pre-checks, come back as diagnostics; at most [`Limits::diagnostics`] of them.
pub fn compile(req: &CompileRequest) -> Result<Compiled, Vec<Diagnostic>> {
    COMPILES.with(|n| n.set(n.get().saturating_add(1)));
    match run::run(req, run::Mode::Build)? {
        run::Output::Compiled(compiled) => Ok(*compiled),
        run::Output::FirstPass(_) => Err(vec![Diagnostic::request(Severity::Error, "the compiler stopped early")]),
    }
}

thread_local! {
    static COMPILES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times [`compile`] has been called on this thread: a test hook for callers that
/// must compile the code once per edit. [`first_pass`] and [`validate`] are not counted.
pub fn compiles_on_this_thread() -> u64 {
    COMPILES.with(std::cell::Cell::get)
}

/// Parses and validates the request's files and compiles `GSUB` alone, without the generated
/// lookups: what the feature writers read from the code before [`compile`] merges their lookups
/// (fontc's first pass). The request's generated lookups are not used.
pub fn first_pass(req: &CompileRequest) -> Result<FirstPass, Vec<Diagnostic>> {
    match run::run(req, run::Mode::FirstPass)? {
        run::Output::FirstPass(first) => Ok(*first),
        run::Output::Compiled(_) => Err(vec![Diagnostic::request(Severity::Error, "the compiler stopped early")]),
    }
}

/// Parses and validates the request's files without building tables: the diagnostics
/// [`compile`] would report about the code itself (`font.features.listErrors`). A list without
/// errors means the code is valid.
pub fn validate(req: &CompileRequest) -> Vec<Diagnostic> {
    match run::run(req, run::Mode::Validate) {
        Ok(run::Output::Compiled(compiled)) => compiled.warnings,
        Ok(run::Output::FirstPass(first)) => first.warnings,
        Err(diagnostics) => diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_fails_with_a_message_not_a_panic() {
        let files = VirtualFiles::new("features.fea", "feature liga { sub f i by f_x; } liga;").unwrap();
        let order = GlyphOrder::new(vec![".notdef".into(), "f".into(), "i".into(), "f_i".into()]).unwrap();
        let result = compile(&CompileRequest::new(&files, &order));
        assert!(result.is_err_and(|d| !d.is_empty()));
    }
}
