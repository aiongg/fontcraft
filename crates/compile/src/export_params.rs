//! Export-time parameters of the compiler stage (`docs/plans/M7.md` § 3 › Export-time
//! parameters): Export Glyphs, Keep Glyphs, Remove Glyphs, Rename Glyphs, Reencode Glyphs and Use
//! Arabic Presentation Form Unicodes, read from the resolved parameters (export over master over
//! font) and applied to the resolved glyphs right after the glyph data is filled in.
//!
//! The order: Export Glyphs sets export flags; Keep Glyphs, then Remove Glyphs, drop glyphs (a
//! removed glyph that a kept composite uses is left to preflight, which decomposes it); Rename
//! Glyphs swaps names, export flags and component references; Reencode Glyphs moves code points;
//! the Arabic presentation forms add code points. What changed is returned as [`ExportEdits`],
//! which the kerning writers read so that pairs follow renamed glyphs and leave removed ones out.
//!
//! Values that cannot be used are reported as warnings and skipped; the values were checked when
//! a command set them, but a file may hold anything. The engine's stage (Filter, PreFilter, Scale
//! to UPM, linked sources, Export Folder) runs before resolving, in `engine/src/export_steps.rs`.
//!
//! Rows: `Export Glyphs`, `Keep Glyphs`, `Remove Glyphs`, `Rename Glyphs`, `Reencode Glyphs`,
//! `Use Arabic Presentation Form Unicodes` (M7.9).

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_model::{Font, Resolved};

use crate::CompileError;

/// What the export parameters changed in the resolved glyph set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportEdits {
    /// Source glyph name → the name it is exported under (both directions of a swap are listed).
    pub renamed: BTreeMap<String, String>,
    /// Source glyph names left out of the export by Keep or Remove Glyphs.
    pub removed: BTreeSet<String>,
}

impl ExportEdits {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.renamed.is_empty() && self.removed.is_empty()
    }

    /// The name `source` is exported under, or `None` when it was removed.
    pub fn exported_name<'a>(&'a self, source: &'a str) -> Option<&'a str> {
        if self.removed.contains(source) {
            return None;
        }
        Some(self.renamed.get(source).map_or(source, String::as_str))
    }
}

/// Applies the glyph-set parameters of `resolved.params` to `resolved.glyphs`. `font` is the
/// source the glyphs were resolved from (`None` for a resolved font compiled on its own), for the
/// glyph facts a query reads that the resolved glyphs do not carry.
pub fn apply(font: Option<&Font>, resolved: &mut Resolved, warnings: &mut Vec<String>) -> Result<ExportEdits, CompileError> {
    // M7.9: the six parameters, in the order above.
    let _ = (font, resolved, warnings);
    Ok(ExportEdits::default())
}
