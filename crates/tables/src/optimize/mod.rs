//! Size optimisation that keeps behaviour (M12.26; rows `optimize`, `font.optimize`). Oracles:
//! fontTools `otlLib.optimize` (its `gpos` module), `cffLib.width`, `varLib.varStore`, `varLib.iup`,
//! `ttLib.tables._g_l_y_f.dropImpliedOnCurvePoints` and `colorLib`. Subroutinising uses the CFF
//! subroutiniser of `fontcraft-cff` (M12.25).
//!
//! The steps, in the order they run:
//!
//! | Step | Table | What it does | Check |
//! |---|---|---|---|
//! | `desubroutinize` | `CFF `, `CFF2` | expands every subroutine call; no subroutines are left | the subroutiniser's expansion |
//! | `cffWidths` | `CFF ` | chooses `defaultWidthX` and `nominalWidthX` again (`cffLib.width`) and rewrites every width | each glyph's advance read back |
//! | `subroutinize` | `CFF `, `CFF2` | moves repeated charstring fragments into subroutines (compreffor's method) | every glyph expanded again and compared byte for byte |
//! | `dropImpliedOncurves` | `glyf` (and `gvar`) | removes on-curve points halfway between their off-curve neighbours at every location ([`glyf_gvar`]); off unless asked for | |
//! | `gvarIup` | `gvar` | keeps only the deltas IUP cannot infer, or writes every delta ([`glyf_gvar`]) | |
//! | `varstore` | `GDEF`, `GPOS` | merges and packs the shared item variation store again ([`varstore`]) | |
//! | `gposCompaction` | `GPOS` | splits class-kerning subtables into smaller, denser ones ([`gpos`]); off unless a level is given | `GPOS` read back |
//! | `colrLayerReuse` | `COLR` | builds the version 1 paint graphs again, sharing layer runs ([`colr`]) | |
//!
//! A step whose result is not smaller is not applied, and a table that would come out larger
//! than it went in is kept as it was, unless the step was asked for in a form that grows it
//! (subroutines expanded or capped, every `gvar` delta written) or another step needs it (a glyph
//! that lost points needs its `gvar` deltas renumbered). `cffWidths` expands the subroutines on
//! its way; a table that had them is subroutinised again. A step that fails leaves its tables as
//! they were, with a note.

pub mod colr;
pub mod glyf_gvar;
pub mod gpos;
pub mod numbered;
pub mod varstore;

pub use glyf_gvar::DeltaMode;

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
    /// Choose the CFF `defaultWidthX` and `nominalWidthX` that make the charstrings smallest.
    pub cff_widths: bool,
    /// Subroutinise CFF and CFF2 charstrings.
    pub subroutines: bool,
    /// Merge and pack the item variation store of `GDEF` and `GPOS` again.
    pub variation_store: bool,
    /// Round store deltas to multiples of this first (1: lossless).
    pub quantum: u32,
    /// Expand every CFF and CFF2 subroutine. Contradicts `subroutines`.
    pub desubroutinize: bool,
    /// Most subroutines in one INDEX, 1 to 65,535; `None` for the subroutiniser's default.
    pub max_subrs: Option<usize>,
    /// What happens to `gvar` deltas; `None` leaves `gvar` alone.
    pub gvar: Option<DeltaMode>,
    /// Remove implied on-curve points from `glyf`. Off by default, as in the reference tools:
    /// it renumbers points, and data outside `glyf` can number them.
    pub drop_implied_oncurves: bool,
    /// Build a version 1 `COLR` again with shared layers.
    pub colr_layer_reuse: bool,
    /// Split class-kerning subtables at this level, 1 to 9; 0 leaves them alone.
    pub gpos_compaction: u8,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            cff_widths: true,
            subroutines: true,
            variation_store: true,
            quantum: 1,
            desubroutinize: false,
            max_subrs: None,
            gvar: Some(DeltaMode::Optimize),
            drop_implied_oncurves: false,
            colr_layer_reuse: true,
            // The reference tool's level defaults to 0.
            gpos_compaction: 0,
        }
    }
}

/// One step run on one table: its size before and after, and whether the result was kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizeStep {
    /// `desubroutinize`, `cffWidths`, `subroutinize`, `dropImpliedOncurves`, `gvarIup`,
    /// `varstore`, `gposCompaction` or `colrLayerReuse`.
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
    if options.gpos_compaction > gpos::MAX_LEVEL {
        return Err(TablesError::BadOptions(format!("the GPOS compaction level must be 0 to {}, not {}", gpos::MAX_LEVEL, options.gpos_compaction)));
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
    if out.get(*b"glyf").is_some() && (options.drop_implied_oncurves || (options.gvar.is_some() && out.get(*b"gvar").is_some())) {
        optimize_glyf_gvar(&mut out, options, &mut steps, &mut notes);
    }
    if options.variation_store && out.get(*b"GDEF").is_some() {
        match varstore::run(&out, options.quantum) {
            Ok(Some(tables)) => {
                let before = out.get(*b"GDEF").map_or(0, <[u8]>::len);
                let after = tables.iter().find(|(t, _)| t == b"GDEF").map_or(before, |(_, b)| b.len());
                let applied = after < before;
                steps.push(OptimizeStep { step: "varstore", table: *b"GDEF", before, after, applied });
                if applied {
                    for (tag, bytes) in tables {
                        out.insert(tag, bytes);
                    }
                } else {
                    notes.push("GDEF: merging the variation store would not make it smaller, so it was kept".into());
                }
            }
            Ok(None) => {}
            Err(e) => notes.push(format!("GDEF and GPOS: variation store left as it was: {e}")),
        }
    }
    if options.gpos_compaction > 0 && out.get(*b"GPOS").is_some() {
        compact_gpos(&mut out, options.gpos_compaction, &mut steps, &mut notes);
    }
    if options.colr_layer_reuse && out.get(*b"COLR").is_some() {
        match colr::run(&out) {
            Ok(Some(table)) => {
                let before = out.get(*b"COLR").map_or(0, <[u8]>::len);
                let applied = table.len() < before;
                steps.push(OptimizeStep { step: "colrLayerReuse", table: *b"COLR", before, after: table.len(), applied });
                if applied {
                    out.insert(*b"COLR", table);
                }
            }
            Ok(None) => {}
            Err(e) => notes.push(format!("COLR: left as it was: {e}")),
        }
    }
    let bytes_after = crate::write(&out, &keep)?.len();
    Ok(OptimizeResult { font: out, bytes_before, bytes_after, steps, notes })
}

/// The `gposCompaction` step, applied when `GPOS` comes out smaller.
fn compact_gpos(out: &mut SfntFont, level: u8, steps: &mut Vec<OptimizeStep>, notes: &mut Vec<String>) {
    match gpos::run(out, level) {
        Ok(Some(c)) => {
            notes.extend(c.notes);
            let Some(table) = c.table else { return };
            let before = out.get(*b"GPOS").map_or(0, <[u8]>::len);
            let applied = table.len() < before;
            steps.push(OptimizeStep { step: "gposCompaction", table: *b"GPOS", before, after: table.len(), applied });
            if applied {
                out.insert(*b"GPOS", table);
            } else {
                notes.push(format!(
                    "GPOS: splitting {} class-kerning subtables into {} would not make it smaller, so it was kept",
                    c.split, c.produced
                ));
            }
        }
        Ok(None) => {}
        Err(e) => notes.push(format!("GPOS: class kerning left as it was: {e}")),
    }
}

/// The `glyf` and `gvar` steps: implied on-curve points, then the `gvar` deltas.
fn optimize_glyf_gvar(out: &mut SfntFont, options: &OptimizeOptions, steps: &mut Vec<OptimizeStep>, notes: &mut Vec<String>) {
    let mode = options.gvar.unwrap_or(DeltaMode::Keep);
    let result = match glyf_gvar::run(out, options.drop_implied_oncurves, mode) {
        Ok(r) => r,
        Err(e) => {
            notes.push(format!("glyf and gvar: left as they were: {e}"));
            return;
        }
    };
    let size = |out: &SfntFont, tag: &[u8; 4]| out.get(*tag).map_or(0, <[u8]>::len);
    let dropped = result.dropped_points > 0;
    if options.drop_implied_oncurves {
        let after = result.tables.iter().find(|(t, _)| t == b"glyf").map_or(size(out, b"glyf"), |(_, b)| b.len());
        steps.push(OptimizeStep { step: "dropImpliedOncurves", table: *b"glyf", before: size(out, b"glyf"), after, applied: dropped });
    }
    if result.hinted_kept > 0 {
        notes.push(format!(
            "glyf: {} glyphs keep their implied on-curve points: instructions, composites, anchors or carets number them",
            result.hinted_kept
        ));
    }
    if result.implied_skipped > 0 {
        notes.push(format!(
            "glyf: {} glyphs keep their implied on-curve points: testing them at every location needs too much work",
            result.implied_skipped
        ));
    }
    if result.iup_skipped > 0 {
        notes.push(format!("gvar: the deltas of {} glyphs were kept as they were: their contours are too long to optimise", result.iup_skipped));
    }
    for (tag, bytes) in result.tables {
        if &tag == b"gvar" {
            let before = size(out, b"gvar");
            let applied = dropped || mode == DeltaMode::All || (mode == DeltaMode::Optimize && bytes.len() < before);
            if options.gvar.is_some() {
                steps.push(OptimizeStep { step: "gvarIup", table: tag, before, after: bytes.len(), applied });
            }
            if applied {
                out.insert(tag, bytes);
            }
        } else if dropped {
            out.insert(tag, bytes);
        }
    }
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
