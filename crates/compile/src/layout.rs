//! The layout stage (M8.7): the font's feature code for one target compiled through
//! `fontcraft_fea` with the generated kerning and mark attachment, giving `GSUB`, `GPOS`,
//! `GDEF`, `BASE` and the table-block values (`docs/plans/M8.md` § 4, M8.7 and M8.8).
//!
//! Rows: the `GSUB`, `GPOS` and `GDEF` rows of `spec/inventory/40-tables.toml`,
//! `compile.featureWriterModes`, `compile.skipFeatures`, `compile.kernQuantisation`,
//! `implicit.markerMissing`, `implicit.markZeroWidth`, `Keep Kerning in one Lookup`,
//! `Use Extension Kerning`.
//!
//! The stage runs after the glyph order and replaces the `kern` and `marks` stages: Update
//! (unless the target's Update Features parameter is off) → instance parameters → tokens →
//! lowering → assembly (`fontcraft_features`), the implicit features
//! (`fontcraft_features::implicit`), then `fontcraft_fea::compile`. A compile error stops the
//! export with the diagnostics mapped to the entries ([`CompileError::Features`]), as fontc
//! stops. M8.8 applies the table-block values after the table builders and moves the code's
//! name ids above the source's.

use fontcraft_fea::Compiled as FeaCompiled;
use fontcraft_features::EntryDiagnostic;
use fontcraft_model::{Font, Resolved, ResolvedGlyph};

use crate::CompileError;

/// How the layout stage runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutOptions {
    /// Regenerate the automatic entries for the target's glyph set before compiling (the
    /// handbook's behaviour; the Update Features parameter turns it off).
    pub regenerate_automatic: bool,
    /// Compile `GPOS`; `false` for the Edit View preview font, which shapes with substitutions
    /// only.
    pub gpos: bool,
    /// Add the generated kerning and mark attachment.
    pub implicit: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self { regenerate_automatic: true, gpos: true, implicit: true }
    }
}

/// The layout stage's result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutTables {
    /// The tables as bytes and the table-block values.
    pub compiled: FeaCompiled,
    /// The compiler's warnings, mapped to the entries.
    pub diagnostics: Vec<EntryDiagnostic>,
}

/// Compiles the feature code of `resolved`'s target (a master, or an instance at a master) for
/// the glyphs of `order`. `font` is the source the target was resolved from (its entries,
/// kerning, anchors and number values); `None` for a source without feature code.
pub fn build(
    font: Option<&Font>,
    resolved: &Resolved,
    order: &[ResolvedGlyph],
    opts: &LayoutOptions,
    warnings: &mut Vec<String>,
) -> Result<LayoutTables, CompileError> {
    // M8.7
    let _ = (font, resolved, order, opts, warnings);
    Ok(LayoutTables::default())
}
