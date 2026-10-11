//! The generated pair kerning of a static master build (`kerning.implicitKernFeature`): the
//! compiled master's kerning written to `GPOS` by `fontcraft_features`, as fontc writes it for a
//! source without feature code.
//!
//! A source with feature code gets no `GPOS` before M8, which compiles feature code: whether
//! the code defines `kern` itself or holds `# Automatic Code` decides where the generated
//! lookups go, and the generated lookups depend on the code's language systems and
//! substitutions. Such a compile warns when the master has kerning.
//!
//! A glyph of the mark class is one of category `Mark` and subcategory `Nonspacing` or `Spacing
//! Combining` (after the glyph database filled what the glyph does not state); it is spacing when
//! it has an advance.

use fontcraft_features::{KernGlyph, KernRequest, MarkSpacing, kern_feature};
use fontcraft_model::{Font, Resolved, ResolvedGlyph};
use fontcraft_spacing::kerning::{Direction, KernIndex};
use write_fonts::tables::gpos::Gpos;

use crate::CompileError;

/// Whether the source holds feature code that is compiled: a snippet that is not switched off
/// and has more than white space.
pub(crate) fn has_feature_code(font: &Font) -> bool {
    let f = &font.features;
    f.prefixes.iter().chain(&f.classes).chain(&f.features).any(|s| !s.disabled && !s.code.trim().is_empty())
}

fn mark(g: &ResolvedGlyph) -> Option<MarkSpacing> {
    let is_mark = g.category.as_deref() == Some("Mark") && matches!(g.sub_category.as_deref(), Some("Nonspacing" | "Spacing Combining"));
    is_mark.then_some(if g.width == 0.0 { MarkSpacing::NonSpacing } else { MarkSpacing::Spacing })
}

/// The `GPOS` table of `resolved`'s master, from `font`'s kerning, for the glyphs `order` (the
/// final glyph order). `None` when there is no kerning to write.
pub(crate) fn generate(font: &Font, resolved: &Resolved, order: &[ResolvedGlyph], warnings: &mut Vec<String>) -> Result<Option<Gpos>, CompileError> {
    let index = KernIndex::new(font);
    let has_kerning =
        [Direction::Ltr, Direction::Rtl].into_iter().any(|d| index.table(d, &resolved.master).is_ok_and(|t| t.pairs().next().is_some()));
    if !has_kerning {
        return Ok(None);
    }
    if has_feature_code(font) {
        warnings.push("the font has feature code, which is compiled from M8: its kerning was not written to the font".to_string());
        return Ok(None);
    }
    let glyphs: Vec<KernGlyph> = order.iter().map(|g| KernGlyph { name: &g.name, codepoints: &g.unicodes, mark: mark(g) }).collect();
    let request = KernRequest { font, index: &index, master: &resolved.master, glyphs: &glyphs };
    let kern = kern_feature(&request).map_err(|e| CompileError::Table { table: "GPOS", message: e.to_string() })?;
    warnings.extend(kern.warnings);
    Ok(kern.gpos)
}
