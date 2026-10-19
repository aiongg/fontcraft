//! Subsetting `CFF2` (M12.19b) and what pyftsubset does to `CFF ` and `CFF2` tables after the
//! glyphs are chosen: drop the Font DICTs no kept glyph selects, then expand every subroutine
//! (`desubroutinize`) or remove the unused ones and renumber the calls, or, without `hinting`,
//! remove the hints. Ported from fontTools' `subset/cff.py` and `cffLib/transforms.py` (MIT
//! License, Copyright 2013 Google, Inc.); the table work is in
//! [`fontcraft_cff::subr::subset_table`].
//!
//! A `CFF2` subset keeps the variation store, every `vsindex` and blend, and each kept Font DICT
//! with its Private DICT and local subroutines. `.notdef` (unless its outline is kept) and the
//! glyphs emptied under `retain_gids` get an empty charstring, as the reference writes them.
//! Without `hinting`, a font with hints comes back without subroutines; the reference keeps the
//! used ones there. A font without hints keeps its used subroutines either way.

use fontcraft_cff::subr::{SubrError, SubrMode, TableSubset, subset_table_noted};

use super::Plan;
use crate::dehint::{DehintOptions, GaspMode, dehint};
use crate::limits::MAX_CHARSTRING_MEMORY;
use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::{Result, TablesError};

/// The `CFF2` table of the subset.
pub(super) fn subset_cff2(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    let glyphs: Vec<usize> = plan.new_to_old.iter().map(|&old| old as usize).collect();
    let mut emptied: Vec<usize> = plan.new_to_old.iter().enumerate().filter(|(_, old)| plan.emptied.contains(old)).map(|(new, _)| new).collect();
    if plan.options.notdef_glyph && !plan.options.notdef_outline && plan.new_to_old.first() == Some(&0) {
        emptied.push(0);
    }
    let subset = TableSubset { glyphs: Some(&glyphs), emptied: &emptied, subroutines: mode(plan), max_expanded: MAX_CHARSTRING_MEMORY };
    let table = noted(plan, |notes| subset_table_noted(bytes, true, &subset, notes)).map_err(subr_error)?;
    without_hints(plan, table, *b"CFF2")
}

/// The steps after the glyphs of a `CFF ` table were chosen ([`super::core`]). A table the
/// generic reader does not handle keeps its subroutines, unless they were to be expanded.
pub(super) fn finish_cff(plan: &Plan, rebuilt: Vec<u8>) -> Result<Vec<u8>> {
    let mode = mode(plan);
    let subset = TableSubset { glyphs: None, emptied: &[], subroutines: mode, max_expanded: MAX_CHARSTRING_MEMORY };
    let table = match noted(plan, |notes| subset_table_noted(&rebuilt, false, &subset, notes)) {
        Ok(table) => table,
        Err(SubrError::Unsupported(_)) if mode != SubrMode::Expand => rebuilt,
        Err(e) => return Err(subr_error(e)),
    };
    without_hints(plan, table, *b"CFF ")
}

/// Runs `f`, adding the notes it writes to the plan's warnings.
fn noted<T>(plan: &Plan, f: impl FnOnce(&mut Vec<String>) -> T) -> T {
    let mut notes = Vec::new();
    let out = f(&mut notes);
    if let Ok(mut w) = plan.warnings.try_borrow_mut() {
        w.extend(notes);
    }
    out
}

fn mode(plan: &Plan) -> SubrMode {
    // Without `hinting` the unused subroutines go first, as in the reference: a font without
    // hints keeps its used ones, and hint removal flattens the charstrings of one with hints.
    if plan.options.desubroutinize { SubrMode::Expand } else { SubrMode::Prune }
}

/// `table` with its hints removed unless `hinting` is on.
fn without_hints(plan: &Plan, table: Vec<u8>, tag: Tag) -> Result<Vec<u8>> {
    if plan.options.hinting {
        return Ok(table);
    }
    let mut only = SfntFont::new(Flavour::Cff);
    only.insert(tag, table);
    let options = DehintOptions { glyphs: None, keep_font_hints: false, hinting_tables: Vec::new(), keep_flex: true, gasp: GaspMode::Keep };
    let mut stripped = dehint(&only, &options)?;
    stripped.remove(tag).ok_or_else(|| TablesError::invalid("removing the CFF hints lost the table"))
}

fn subr_error(e: SubrError) -> TablesError {
    match e {
        SubrError::Invalid(m) => TablesError::Invalid(format!("the CFF data is damaged: {m}")),
        SubrError::Unsupported(m) => TablesError::Unsupported(m),
        SubrError::TooLarge { what, limit } => TablesError::TooLarge { what, limit },
        SubrError::Mismatch { glyph } => TablesError::Invalid(format!("the charstring of glyph {glyph} changed while subsetting")),
    }
}

#[cfg(test)]
#[path = "cff2_tests.rs"]
mod tests;
