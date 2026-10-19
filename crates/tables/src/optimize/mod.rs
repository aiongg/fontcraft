//! Size optimisation that keeps behaviour (M12.26; rows `optimize`, `font.optimize`). Oracles:
//! fontTools `otlLib.optimize`, `cffLib.width`, `varLib.varStore`. Subroutinising uses the CFF
//! subroutiniser of `fontcraft-cff` (M12.25).
//!
//! The steps, in the order they run on each `CFF ` and `CFF2` table:
//!
//! | Step | What it does | Check |
//! |---|---|---|
//! | `desubroutinize` | expands every subroutine call; no subroutines are left | the subroutiniser's expansion |
//! | `cffWidths` | chooses `defaultWidthX` and `nominalWidthX` again (`cffLib.width`) and rewrites every width | each glyph's advance read back |
//! | `subroutinize` | moves repeated charstring fragments into subroutines (compreffor's method) | every glyph expanded again and compared byte for byte |
//!
//! A step whose result is not smaller is not applied, and a table that would come out larger
//! than it went in is kept as it was, unless subroutines were expanded or capped on request.
//! `cffWidths` expands the subroutines on its way; a table that had them is subroutinised again.
//! A table the subroutiniser refuses (one beyond its limits, an operator it does not handle) is
//! left as it was, with a note. GPOS compaction and variation store merging are not built: a font
//! that has those tables gets a note and keeps them.

use crate::sfnt::{SfntFont, tag_str};
use crate::{Result, TableOrder, TablesError, Tag, WriteOptions};
use fontcraft_cff::subr::{
    DEFAULT_MAX_SUBRS, MAX_SUBRS_PER_INDEX, SubrOptions, charstrings_of_table, desubroutinize_table, optimize_widths_table, subroutinize_table,
};

#[cfg(test)]
mod tests;

/// The optimisations [`optimize`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptimizeOptions {
    /// Compact GPOS lookups into subtables of the best size. Not built: a note says so.
    pub gpos: bool,
    /// Choose the CFF `defaultWidthX` and `nominalWidthX` that make the charstrings smallest.
    pub cff_widths: bool,
    /// Subroutinise CFF and CFF2 charstrings.
    pub subroutines: bool,
    /// Merge and sort the rows of variation stores. Not built: a note says so.
    pub variation_store: bool,
    /// Expand every CFF and CFF2 subroutine. Contradicts `subroutines`.
    pub desubroutinize: bool,
    /// Most subroutines in one INDEX, 1 to 65,535; `None` for the subroutiniser's default.
    pub max_subrs: Option<usize>,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self { gpos: true, cff_widths: true, subroutines: true, variation_store: true, desubroutinize: false, max_subrs: None }
    }
}

/// One step run on one table: its size before and after, and whether the result was kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizeStep {
    /// `desubroutinize`, `cffWidths` or `subroutinize`.
    pub step: &'static str,
    pub table: Tag,
    pub before: usize,
    pub after: usize,
    pub applied: bool,
}

/// The optimised font and the sizes before and after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizeResult {
    pub font: SfntFont,
    /// File sizes, the tables in their input order.
    pub bytes_before: usize,
    pub bytes_after: usize,
    pub steps: Vec<OptimizeStep>,
    /// Steps skipped or tables kept, one sentence each.
    pub notes: Vec<String>,
}

/// Optimises `font`.
pub fn optimize(font: &SfntFont, options: &OptimizeOptions) -> Result<OptimizeResult> {
    if options.desubroutinize && options.subroutines {
        return Err(TablesError::BadOptions("subroutinizing and desubroutinizing contradict each other: choose one".into()));
    }
    if let Some(n) = options.max_subrs
        && !(1..=MAX_SUBRS_PER_INDEX).contains(&n)
    {
        return Err(TablesError::BadOptions(format!("the subroutine limit must be 1 to {MAX_SUBRS_PER_INDEX}, not {n}")));
    }
    let keep = WriteOptions { order: TableOrder::Keep };
    let bytes_before = crate::write(font, &keep)?.len();
    let mut out = font.clone();
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    for (tag, cff2) in [(*b"CFF ", false), (*b"CFF2", true)] {
        let Some(original) = font.get(tag) else { continue };
        if let Some(table) = optimize_cff(original, tag, cff2, options, &mut steps, &mut notes) {
            out.insert(tag, table);
        }
    }
    if options.gpos && font.get(*b"GPOS").is_some() {
        notes.push("GPOS compaction is not built yet: GPOS was left as it was".into());
    }
    if options.variation_store && font.get(*b"fvar").is_some() {
        notes.push("variation store merging is not built yet: the variation stores were left as they were".into());
    }
    let bytes_after = crate::write(&out, &keep)?.len();
    Ok(OptimizeResult { font: out, bytes_before, bytes_after, steps, notes })
}

/// The steps on one `CFF ` or `CFF2` table; `None` when the table stays as it was.
fn optimize_cff(
    original: &[u8],
    tag: Tag,
    cff2: bool,
    options: &OptimizeOptions,
    steps: &mut Vec<OptimizeStep>,
    notes: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let name = tag_str(tag);
    let had_subrs = match charstrings_of_table(original, cff2) {
        Ok(set) => !set.global_subrs.is_empty() || set.local_subrs.iter().any(|l| !l.is_empty()),
        Err(e) => {
            notes.push(format!("{name}: left as it was: {e}"));
            return None;
        }
    };
    let mut current = original.to_vec();
    let mut record = |step: &'static str, before: usize, after: usize, applied: bool| {
        steps.push(OptimizeStep { step, table: tag, before, after, applied });
    };
    if options.desubroutinize {
        match desubroutinize_table(&current, cff2) {
            Ok(t) => {
                record("desubroutinize", current.len(), t.len(), true);
                current = t;
            }
            Err(e) => {
                notes.push(format!("{name}: subroutines kept: {e}"));
                return None;
            }
        }
    }
    let mut flattened = false;
    if options.cff_widths && !cff2 {
        match optimize_widths_table(&current) {
            Ok((t, report)) => {
                notes.extend(report.kept.iter().map(|k| format!("{name}: widths kept: {k}")));
                if report.changed > 0 {
                    record("cffWidths", current.len(), t.len(), true);
                    current = t;
                    flattened = had_subrs && !options.desubroutinize;
                } else {
                    record("cffWidths", current.len(), current.len(), false);
                }
            }
            Err(e) => notes.push(format!("{name}: widths kept: {e}")),
        }
    }
    if options.subroutines || flattened {
        let subr = SubrOptions { max_subrs: options.max_subrs.unwrap_or(DEFAULT_MAX_SUBRS), ..SubrOptions::default() };
        match subroutinize_table(&current, cff2, &subr) {
            Ok(t) if t.len() < current.len() || options.max_subrs.is_some() || flattened => {
                record("subroutinize", current.len(), t.len(), true);
                current = t;
            }
            Ok(t) => {
                record("subroutinize", current.len(), t.len(), false);
                notes.push(format!("{name}: subroutines would not make the table smaller, so it keeps its charstrings"));
            }
            Err(e) => notes.push(format!("{name}: written without new subroutines: {e}")),
        }
    }
    if current.len() > original.len() && !options.desubroutinize && options.max_subrs.is_none() {
        notes.push(format!(
            "{name}: the optimised table came out larger ({} bytes, not {}), so it was kept as it was",
            current.len(),
            original.len()
        ));
        return None;
    }
    (current != original).then_some(current)
}
