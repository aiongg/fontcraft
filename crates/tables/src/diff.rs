//! Comparing two fonts table by table (M12.32; rows `diff`, `font.diff`). Oracles: AFDKO
//! sfntdiff, `fonttools diff`, fontc's `ttx_diff`.
//!
//! [`diff`] pairs the tables of two fonts by tag and gives one [`TableDiff`] per table of either,
//! in tag order:
//!
//! 1. A table on one side only is [`DiffStatus::OnlyFirst`] or [`DiffStatus::OnlySecond`].
//! 2. Tables with equal bytes are [`DiffStatus::Same`] without being decoded, unless
//!    [`DiffOptions::compare_identical`] is set.
//! 3. The rest are compared in the form [`DiffOptions::mode`] asks for: bytes (the first
//!    differing offset), a line diff of hex dumps, or a line diff of the decoded text. The text is
//!    the table's TTX ([`crate::ttx`]); a table no TTX writer reads is compared as its TTX hex, as
//!    a dump writes it. With a normalisation, [`DiffStatus`] is the verdict after it: a table
//!    whose bytes differ but whose normalised text is equal is `Same`, and
//!    [`TableDiff::bytes_equal`] still says the bytes differ.
//!
//! The compiler normalisation ([`DiffMode::NormalizeCompiler`]) is described in the `compiler`
//! module; it adds two pseudo-tables after `GPOS` and `GDEF`, named `GPOS=normalized` and
//! `GDEF=normalized`, holding the layout normaliser's text ([`crate::normalize`]). The dump
//! normalisation ([`DiffMode::NormalizeDump`]) compares the normalised dump of [`crate::text`].
//!
//! Every difference list is capped: [`crate::limits::MAX_DIFF_RECORDS`] removed or added lines
//! per table in the text modes, [`crate::limits::MAX_DIFF_HEX`] bytes of hex (in rows of 16) in
//! the hex mode, unless [`DiffOptions::all`] is set. The line diff itself is bounded in work; past
//! the bound, lines are compared position by position.
//!
//! This is the contract the Table Viewer's Compare With… builds on: one entry per table with its
//! [`DiffStatus`], sizes and, in the text modes, the unified diff lines and a similarity ratio.

mod compiler;
mod lines;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::limits::{MAX_DIFF_HEX, MAX_DIFF_RECORDS};
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// How table contents are compared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    /// Presence and byte equality only (`--detail tables`, `--summary`).
    Tables,
    /// Presence and bytes, with the first differing offset (`--detail offset`).
    Bytes,
    /// Bytes, with a line diff of the hex dumps (`--detail hex`).
    Hex,
    /// The TTX text of each table, line by line (`--detail fields`, the default).
    #[default]
    Text,
    /// The text after the compiler normalisation (`--normalize compiler`).
    NormalizeCompiler,
    /// The normalised dump (`--normalize dump`).
    NormalizeDump,
}

impl DiffMode {
    /// Whether tables are compared as decoded text.
    pub fn is_text(self) -> bool {
        matches!(self, DiffMode::Text | DiffMode::NormalizeCompiler | DiffMode::NormalizeDump)
    }
}

/// Tables that hold only hinting, left out with [`DiffOptions::no_hints`].
pub const HINT_TABLES: [Tag; 7] = [*b"fpgm", *b"prep", *b"cvt ", *b"cvar", *b"hdmx", *b"LTSH", *b"VDMX"];

/// The name of the layout normaliser's pseudo-table for `tag`.
pub fn pseudo_name(tag: Tag) -> String {
    format!("{}=normalized", tag_str(tag).trim_end())
}

/// What [`diff`] compares.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffOptions {
    pub mode: DiffMode,
    /// Tables to compare; empty compares all.
    pub tables: Vec<Tag>,
    /// Tables to leave out.
    pub skip: Vec<Tag>,
    /// List every difference instead of stopping at [`crate::limits::MAX_DIFF_RECORDS`] records
    /// and [`crate::limits::MAX_DIFF_HEX`] hex bytes per table.
    pub all: bool,
    /// Unchanged lines shown around each difference; `usize::MAX` shows every line
    /// (`--detail verbose`). Default 3.
    pub context: usize,
    /// Decode and compare tables whose bytes are equal too.
    pub compare_identical: bool,
    /// With a normalisation: leave out the hinting tables ([`HINT_TABLES`]), and in the compiler
    /// normalisation glyph instructions and the hinting fields of `maxp`.
    pub no_hints: bool,
    /// With the compiler normalisation: leave out `head.fontRevision` and name IDs 3 and 5.
    pub no_version: bool,
    /// Strip Extension lookup wrappers before comparing; `None` strips them in the compiler
    /// normalisation only.
    pub unwrap_extensions: Option<bool>,
    /// In the compiler normalisation, the share of `glyf` coordinates and `gvar` deltas, in
    /// percent of the second font's, that may differ by one unit without counting. Default 0.1.
    pub off_by_one_budget: f64,
    /// A table is [`TableDiff::oversized`] when its size in the second font exceeds the first's by
    /// more than this share, in percent. Default 10.
    pub size_threshold: f64,
    /// Keep the two texts that were compared in [`TableDiff::texts`].
    pub keep_texts: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        DiffOptions {
            mode: DiffMode::default(),
            tables: Vec::new(),
            skip: Vec::new(),
            all: false,
            context: 3,
            compare_identical: false,
            no_hints: false,
            no_version: false,
            unwrap_extensions: None,
            off_by_one_budget: 0.1,
            size_threshold: 10.0,
            keep_texts: false,
        }
    }
}

/// The verdict for one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffStatus {
    Same,
    Different,
    OnlyFirst,
    OnlySecond,
}

/// One table's comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct TableDiff {
    pub tag: Tag,
    /// The tag as text without trailing spaces, or a pseudo-table's name ([`pseudo_name`]).
    pub name: String,
    pub status: DiffStatus,
    /// Whether both fonts have the table with the same bytes (false for a pseudo-table).
    pub bytes_equal: bool,
    /// The first byte offset at which the tables differ (the shorter length when one is a prefix
    /// of the other); `None` when the bytes are equal or one side lacks the table.
    pub first_offset: Option<usize>,
    /// Sizes in bytes on each side; `None` where the table is missing, and for a pseudo-table.
    pub size_a: Option<usize>,
    pub size_b: Option<usize>,
    /// Whether the second font's table is larger than the first's by more than
    /// [`DiffOptions::size_threshold`].
    pub oversized: bool,
    /// In the text modes, for a table on both sides: the share of lines the two texts have in
    /// common, from 0 to 1 (1 when equal).
    pub similarity: Option<f64>,
    /// Difference lines in the mode's form: a unified diff (`@@ -a,b +c,d @@` headers, then
    /// lines starting with ` `, `-` or `+`) in the hex and text modes; empty otherwise.
    pub lines: Vec<String>,
    /// Removed or added lines left out by the limits.
    pub truncated: usize,
    /// The texts compared, with [`DiffOptions::keep_texts`], in the text modes.
    pub texts: Option<(String, String)>,
}

impl TableDiff {
    fn new(tag: Tag, name: String, status: DiffStatus) -> Self {
        TableDiff {
            tag,
            name,
            status,
            bytes_equal: false,
            first_offset: None,
            size_a: None,
            size_b: None,
            oversized: false,
            similarity: None,
            lines: Vec::new(),
            truncated: 0,
            texts: None,
        }
    }
}

/// Whether nothing differs: every entry is [`DiffStatus::Same`].
pub fn identical(diffs: &[TableDiff]) -> bool {
    diffs.iter().all(|d| d.status == DiffStatus::Same)
}

/// Compares `a` with `b`, one entry per table of either, in tag order; with the compiler
/// normalisation, the layout normaliser's pseudo-tables follow `GPOS` and `GDEF`.
pub fn diff(a: &SfntFont, b: &SfntFont, options: &DiffOptions) -> Result<Vec<TableDiff>> {
    if !options.tables.is_empty() && !options.skip.is_empty() {
        return Err(TablesError::BadOptions("give the tables to compare or the tables to leave out, not both".into()));
    }
    if !options.off_by_one_budget.is_finite() || !(0.0..=100.0).contains(&options.off_by_one_budget) {
        return Err(TablesError::BadOptions("the off-by-one budget must be a percentage from 0 to 100".into()));
    }
    if !options.size_threshold.is_finite() || options.size_threshold < 0.0 {
        return Err(TablesError::BadOptions("the size threshold must be a percentage of 0 or more".into()));
    }
    let normalizing = matches!(options.mode, DiffMode::NormalizeCompiler | DiffMode::NormalizeDump);
    let tags: BTreeSet<Tag> = a
        .tables
        .keys()
        .chain(b.tables.keys())
        .copied()
        .filter(|t| options.tables.is_empty() || options.tables.contains(t))
        .filter(|t| !options.skip.contains(t))
        .filter(|t| !(normalizing && options.no_hints && HINT_TABLES.contains(t)))
        .collect();
    let context = match options.mode {
        DiffMode::NormalizeCompiler => Some(compiler::Context::new(
            a,
            b,
            options.unwrap_extensions.unwrap_or(true),
            options.no_hints,
            options.no_version,
            options.off_by_one_budget,
        )?),
        _ => None,
    };
    let mut out = Vec::with_capacity(tags.len());
    for tag in tags {
        out.push(compare(a, b, tag, options, context.as_ref())?);
        if context.is_some()
            && (tag == *b"GPOS" || tag == *b"GDEF")
            && let Some(entry) = compare_normalized(a, b, tag, options)?
        {
            out.push(entry);
        }
    }
    Ok(out)
}

fn compare(a: &SfntFont, b: &SfntFont, tag: Tag, options: &DiffOptions, ctx: Option<&compiler::Context>) -> Result<TableDiff> {
    let name = tag_str(tag).trim_end().to_string();
    let (da, db) = match (a.get(tag), b.get(tag)) {
        (Some(da), Some(db)) => (da, db),
        (da, db) => {
            let status = if da.is_some() { DiffStatus::OnlyFirst } else { DiffStatus::OnlySecond };
            let mut entry = TableDiff::new(tag, name, status);
            entry.size_a = da.map(<[u8]>::len);
            entry.size_b = db.map(<[u8]>::len);
            return Ok(entry);
        }
    };
    let bytes_equal = da == db;
    let mut entry = TableDiff::new(tag, name, if bytes_equal { DiffStatus::Same } else { DiffStatus::Different });
    entry.bytes_equal = bytes_equal;
    entry.size_a = Some(da.len());
    entry.size_b = Some(db.len());
    entry.oversized = oversized(da.len(), db.len(), options.size_threshold);
    if !bytes_equal {
        entry.first_offset = Some(da.iter().zip(db).position(|(x, y)| x != y).unwrap_or(da.len().min(db.len())));
    }
    match options.mode {
        DiffMode::Tables | DiffMode::Bytes => {}
        DiffMode::Hex => {
            if !bytes_equal {
                let limit = if options.all { usize::MAX } else { MAX_DIFF_HEX / HEX_ROW };
                let (lines, truncated) = hex_diff(da, db, options.context, limit);
                entry.lines = lines;
                entry.truncated = truncated;
            }
        }
        DiffMode::Text | DiffMode::NormalizeCompiler | DiffMode::NormalizeDump => {
            if bytes_equal && !options.compare_identical {
                entry.similarity = Some(1.0);
                return Ok(entry);
            }
            let (ta, tb) = match (options.mode, ctx) {
                (DiffMode::NormalizeCompiler, Some(ctx)) => ctx.texts(tag, a, b)?,
                (DiffMode::NormalizeDump, _) => (dump_text(a, tag)?, dump_text(b, tag)?),
                _ => (ttx_text(a, tag)?, ttx_text(b, tag)?),
            };
            text_diff(&mut entry, ta, tb, options);
        }
    }
    Ok(entry)
}

/// The layout normaliser's pseudo-table for `tag` (`GPOS` or `GDEF`); `None` for a `GDEF` whose
/// normalised text is empty on both sides.
fn compare_normalized(a: &SfntFont, b: &SfntFont, tag: Tag, options: &DiffOptions) -> Result<Option<TableDiff>> {
    let text = |font: &SfntFont| -> String {
        if font.get(tag).is_none() {
            return String::new();
        }
        let options = crate::normalize::NormalizeOptions { tables: vec![tag], location: Vec::new() };
        crate::normalize::normalize(font, &options).unwrap_or_else(|e| format!("the layout normaliser failed: {e}\n"))
    };
    let (ta, tb) = (text(a), text(b));
    if tag == *b"GDEF" && ta.is_empty() && tb.is_empty() {
        return Ok(None);
    }
    let status = match (a.get(tag).is_some(), b.get(tag).is_some()) {
        (true, false) => DiffStatus::OnlyFirst,
        (false, true) => DiffStatus::OnlySecond,
        _ => DiffStatus::Same,
    };
    let mut entry = TableDiff::new(tag, pseudo_name(tag), status);
    if status == DiffStatus::Same {
        text_diff(&mut entry, ta, tb, options);
    }
    Ok(Some(entry))
}

/// Fills `entry` from the line comparison of `ta` and `tb`.
fn text_diff(entry: &mut TableDiff, ta: String, tb: String, options: &DiffOptions) {
    {
        let la: Vec<&str> = ta.lines().collect();
        let lb: Vec<&str> = tb.lines().collect();
        if la == lb {
            entry.status = DiffStatus::Same;
            entry.similarity = Some(1.0);
        } else {
            entry.status = DiffStatus::Different;
            entry.similarity = Some(lines::quick_ratio(&la, &lb));
            let runs = lines::script(la.len(), lb.len(), |i, j| la.get(i) == lb.get(j), lines::MAX_DIFF_WORK);
            let limit = if options.all { usize::MAX } else { MAX_DIFF_RECORDS };
            let line = |l: &[&str], i: usize| l.get(i).map_or_else(String::new, |s| (*s).to_string());
            let (out, truncated) = lines::unified(&runs, options.context, limit, |i| line(&la, i), |j| line(&lb, j));
            entry.lines = out;
            entry.truncated = truncated;
        }
    }
    if options.keep_texts {
        entry.texts = Some((ta, tb));
    }
}

/// Bytes per hex row.
const HEX_ROW: usize = 16;

/// The unified diff of the two tables' hex dumps, rows of 16 bytes compared as rows.
fn hex_diff(a: &[u8], b: &[u8], context: usize, limit: usize) -> (Vec<String>, usize) {
    fn rows(data: &[u8]) -> usize {
        data.len().div_ceil(HEX_ROW)
    }
    fn row(data: &[u8], i: usize) -> &[u8] {
        let start = i.saturating_mul(HEX_ROW);
        data.get(start..data.len().min(start.saturating_add(HEX_ROW))).unwrap_or_default()
    }
    let runs = lines::script(rows(a), rows(b), |i, j| row(a, i) == row(b, j), lines::MAX_DIFF_WORK);
    lines::unified(&runs, context, limit, |i| hex_row(i, row(a, i)), |j| hex_row(j, row(b, j)))
}

/// One hex row: offset, bytes in two groups of eight, and the printable ASCII.
fn hex_row(index: usize, bytes: &[u8]) -> String {
    let mut s = format!("{:08x} ", index.saturating_mul(HEX_ROW));
    for i in 0..HEX_ROW {
        if i == 8 {
            s.push(' ');
        }
        match bytes.get(i) {
            Some(b) => s.push_str(&format!(" {b:02x}")),
            None => s.push_str("   "),
        }
    }
    s.push_str("  |");
    s.extend(bytes.iter().map(|&b| if (0x20..0x7f).contains(&b) { char::from(b) } else { '.' }));
    s.push('|');
    s
}

/// The TTX text of one table, without the document's declaration and root element.
fn ttx_text(font: &SfntFont, tag: Tag) -> Result<String> {
    let options = crate::ttx::DumpOptions { tables: vec![tag], no_version: true, ..Default::default() };
    let text = crate::ttx::dump(font, &options)?;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("<?xml") || trimmed.starts_with("<ttFont") || trimmed.starts_with("</ttFont>") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

/// The normalised dump of one table ([`crate::text::view`] with `normalize`).
fn dump_text(font: &SfntFont, tag: Tag) -> Result<String> {
    let options = crate::text::ViewOptions { view: String::new(), tables: vec![tag], glyphs: Vec::new(), normalize: true };
    crate::text::view(font, &options)
}

/// Whether `b` bytes is larger than `a` by more than `percent` of `b`, as fontc's `ttx_diff`
/// measures it: `1 - a / b > percent / 100`.
fn oversized(a: usize, b: usize, percent: f64) -> bool {
    b > a && b > 0 && 1.0 - a as f64 / b as f64 > percent / 100.0
}
