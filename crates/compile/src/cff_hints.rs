//! PostScript hints at export: per glyph, manual hints when the hint-source layer has any,
//! resolved through node links in the master being built; else the autohinter, unless Autohint
//! is off, the glyph is in Disable autohinting for glyphs, or Export PostScript Hints is off.
//! Widths made positive, ghosts with the reserved widths (−20 top, −21 bottom), overlapping hints
//! split into masks by the autohinter's substitution step, flex where the flex test passes
//! (`docs/plans/M10.md` § 3 › PostScript export).
//!
//! M10.14 writes the manual half and the hook in `cff.rs`; M10.15 the autohint stage. Rows:
//! `hinting.ps.export.*`, `hinting.ps.masters.*` (export), `export.autohintCff`,
//! `hinting.ps.export.autohintOption`, `hinting.ps.export.autohintParameter`.

use fontcraft_autohint::FdDict;
use fontcraft_cff::CharstringHints;
use fontcraft_hinting::HintSettings;
use fontcraft_model::{Font, ResolvedGlyph};

use crate::CompileError;

/// The zones and stems of `settings` in the autohinter's validated form.
pub fn autohint_dict(settings: &HintSettings) -> Result<FdDict, CompileError> {
    // M10.15: ZoneSet and StemSet from the settings, then FdDict::new.
    let _ = settings;
    Ok(FdDict::default())
}

/// The charstring hints of `glyph` (its outline as prepared for CFF is `path_len` operations
/// long), or `None` for an unhinted charstring. Warnings go to `warnings`.
pub fn glyph_hints(
    font: &Font,
    settings: &HintSettings,
    glyph: &ResolvedGlyph,
    path_len: usize,
    autohint: Option<&FdDict>,
    warnings: &mut Vec<String>,
) -> Result<Option<CharstringHints>, CompileError> {
    // M10.14: manual hints; M10.15: the autohinter when `autohint` is given.
    let _ = (font, settings, glyph, path_len, autohint, warnings);
    Ok(None)
}
