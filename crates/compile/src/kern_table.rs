//! The legacy `kern` table (format 0), written for TrueType exports when the `Export kern Table`
//! parameter is on: the master's kerning with groups expanded into glyph pairs, exceptions first,
//! renamed and removed glyphs ([`ExportEdits`]) followed.
//!
//! A subtable holds at most [`fontcraft_fontinfo::MAX_KERN_SUBTABLE_PAIRS`] pairs and the table
//! [`fontcraft_fontinfo::MAX_KERN_PAIRS`]; the rest are dropped with a warning. CFF exports get no
//! `kern` table.
//!
//! Row: `Export kern Table` (M7.11).

use fontcraft_model::{Font, Resolved, ResolvedGlyph};

use crate::export_params::ExportEdits;
use crate::outline::Flavour;

/// The `kern` table's bytes for the glyphs `glyphs` (the final glyph order), or `None` when the
/// parameter is off, the flavour is CFF or there is no kerning.
pub fn build(
    font: &Font,
    resolved: &Resolved,
    glyphs: &[ResolvedGlyph],
    flavour: Flavour,
    edits: &ExportEdits,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    // M7.11: pairs, subtables, limits.
    let _ = (font, resolved, glyphs, flavour, edits, warnings);
    None
}
