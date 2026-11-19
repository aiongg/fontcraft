//! The CFF-writing switches of the reference converter (M12.70; rows `convert`, `optimize`):
//! edits made to a finished `CFF ` or `CFF2` table, so that every writer (conversion, the
//! optimiser, the compilers) shares them.
//!
//! | Switch | Reference | Edit |
//! |---|---|---|
//! | [`CffWriteOptions::charstrings`] | `+T` / `-no_opt` | every path operator rewritten in the short forms ([`CharstringForm::Specialize`]) or as `rmoveto`, `rlineto` and `rrcurveto` only ([`CharstringForm::General`]) |
//! | [`CffWriteOptions::drop_dotsection`] | `-Z` | `dotsection` removed (the `seac` half of `-Z` needs glyph names and is the caller's) |
//! | [`CffWriteOptions::optimize_widths`] | `+W` / `-W` | `defaultWidthX` and `nominalWidthX` chosen again ([`crate::subr::optimize_widths_table`]) |
//! | [`CffWriteOptions::embed_minimal`] | `+E` | the descriptive Top DICT strings removed: `version`, `Notice`, `Copyright`, `FullName`, `FamilyName`, `Weight` |
//! | [`CffWriteOptions::dedupe_family_blues`] | `+F` | `FamilyBlues` and `FamilyOtherBlues` removed where they equal `BlueValues` and `OtherBlues` |
//! | [`CffWriteOptions::standard_encoding`] | `-std` | the `Encoding` removed, so the font has StandardEncoding |
//! | [`CffWriteOptions::keep_unique_id`] | `+q` / `-q` | `UniqueID` and `XUID` kept or removed |
//! | [`CffWriteOptions::warn_hints`] | `+H`, `+d` | glyphs that draw without stem hints, and hint masks that repeat the one before, reported |
//!
//! Charstrings are rewritten after every subroutine call is expanded; the table comes back
//! without subroutines and [`WriteReport::expanded`] says so, so the caller can subroutinise
//! again. [`reencode_width`] moves a glyph's width operand between two Private DICTs' width
//! pairs, for a font made name-keyed with one forced Font DICT.

use crate::raw::{RelSeg, encode_t2_number_exact, generalize, read_t2_number, write_segments};
use crate::specialize::{Cmd, RLINETO, RMOVETO, RRCURVETO, specialize};
use crate::subr::{CFF_STACK, CharstringSet, SubrError, TableRewrite, charstrings_of_table, desubroutinize, optimize_widths_table, rewrite_table};

/// Top DICT operators `+E` removes: `version`, `Notice`, `Copyright`, `FullName`, `FamilyName`,
/// `Weight`.
pub const EMBED_KEYS: [u16; 6] = [0, 1, 1200, 2, 3, 4];
/// `UniqueID` and `XUID`.
pub const UNIQUE_ID_KEYS: [u16; 2] = [13, 14];
/// The Top DICT `Encoding` operator.
const OP_ENCODING: u16 = 16;
/// Most glyph ids one warning lists before it says how many more there are.
const MAX_LISTED: usize = 20;

/// How path operators are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharstringForm {
    /// The shortest operators (`hlineto`, `vvcurveto`, `rcurveline` …), combined while the
    /// operand stack allows: the reference's default.
    Specialize,
    /// `rmoveto`, `rlineto` and `rrcurveto` only.
    General,
}

/// The switches; [`Default`] is what the reference converter does without any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CffWriteOptions {
    /// Rewrite every charstring's path operators; `None` keeps them as written.
    pub charstrings: Option<CharstringForm>,
    /// Remove `dotsection`.
    pub drop_dotsection: bool,
    /// Choose `defaultWidthX` and `nominalWidthX` again (`CFF ` only); `None` keeps the pair the
    /// writer chose, which for FontCraft's writers is already the optimised one.
    pub optimize_widths: Option<bool>,
    /// Remove the descriptive Top DICT strings ([`EMBED_KEYS`]).
    pub embed_minimal: bool,
    /// Remove `FamilyBlues` and `FamilyOtherBlues` that equal `BlueValues` and `OtherBlues`.
    pub dedupe_family_blues: bool,
    /// Remove a custom or expert `Encoding`.
    pub standard_encoding: bool,
    /// Keep `UniqueID` and `XUID`; `false` removes them.
    pub keep_unique_id: bool,
    /// Report glyphs without stem hints and repeated hint masks.
    pub warn_hints: bool,
}

impl Default for CffWriteOptions {
    fn default() -> Self {
        Self {
            charstrings: None,
            drop_dotsection: false,
            optimize_widths: None,
            embed_minimal: false,
            dedupe_family_blues: true,
            standard_encoding: false,
            keep_unique_id: true,
            warn_hints: true,
        }
    }
}

impl CffWriteOptions {
    /// Options that change no byte of a table and report nothing.
    pub fn none() -> Self {
        Self { dedupe_family_blues: false, warn_hints: false, ..Self::default() }
    }

    fn rewrites_charstrings(&self) -> bool {
        self.charstrings.is_some() || self.drop_dotsection
    }
}

/// What [`apply`] made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    pub bytes: Vec<u8>,
    /// The `warn_hints` findings, one line each.
    pub warnings: Vec<String>,
    /// Edits asked for that could not be made, one line each.
    pub notes: Vec<String>,
    /// The charstrings were expanded: the table has no subroutines left.
    pub expanded: bool,
    /// The DICT operators removed, each once.
    pub removed: Vec<u16>,
}

/// `table` (a `CFF ` table, or `CFF2` when `cff2`) with the switches of `o` applied.
pub fn apply(table: &[u8], cff2: bool, o: &CffWriteOptions) -> Result<WriteReport, SubrError> {
    let mut report = WriteReport::default();
    // The two switches on by default never make a conversion fail: a table they cannot read
    // is kept, with a note.
    if o.warn_hints {
        match charstrings_of_table(table, cff2).and_then(|set| desubroutinize(&set)).and_then(|set| hint_warnings(&set)) {
            Ok(w) => report.warnings = w,
            Err(e) => report.notes.push(format!("the hints were not checked: {e}")),
        }
    }
    let mut top: Vec<u16> = Vec::new();
    if o.embed_minimal {
        top.extend(EMBED_KEYS);
    }
    if !o.keep_unique_id {
        top.extend(UNIQUE_ID_KEYS);
    }
    if o.standard_encoding {
        top.push(OP_ENCODING);
    }
    let edit_dicts = !top.is_empty() || o.dedupe_family_blues;
    let mut bytes = if edit_dicts || o.rewrites_charstrings() {
        let rewrite = TableRewrite { top: &top, private: &[], dedupe_family_blues: o.dedupe_family_blues };
        let form = o.charstrings;
        let drop = o.drop_dotsection;
        let rewrites = o.rewrites_charstrings();
        let rewritten = rewrite_table(table, cff2, &rewrite, &mut |set: &CharstringSet| {
            if !rewrites {
                return Ok(set.clone());
            }
            let mut flat = desubroutinize(set)?;
            let max_stack = flat.max_stack;
            for (gid, code) in flat.charstrings.iter_mut().enumerate() {
                *code = rewrite_charstring(code, cff2, form, drop, max_stack).map_err(|e| in_glyph(e, gid))?;
            }
            Ok(flat)
        });
        let (bytes, removed) = match rewritten {
            Ok(r) => r,
            Err(e) if top.is_empty() && !rewrites => {
                report.notes.push(format!("FamilyBlues and FamilyOtherBlues were not checked: {e}"));
                (table.to_vec(), Vec::new())
            }
            Err(e) => return Err(e),
        };
        report.removed = removed;
        report.expanded = rewrites;
        if !rewrites && report.removed.is_empty() {
            // Nothing matched: the table stays byte for byte as it was.
            table.to_vec()
        } else {
            bytes
        }
    } else {
        table.to_vec()
    };
    if o.optimize_widths == Some(true) {
        if cff2 {
            report.notes.push("CFF2 charstrings hold no widths: there are none to optimise".into());
        } else {
            let (optimised, widths) = optimize_widths_table(&bytes)?;
            report.notes.extend(widths.kept);
            report.expanded = true;
            bytes = optimised;
        }
    }
    report.bytes = bytes;
    Ok(report)
}

fn in_glyph(e: SubrError, gid: usize) -> SubrError {
    match e {
        SubrError::Invalid(m) => SubrError::Invalid(format!("glyph {gid}: {m}")),
        SubrError::Unsupported(m) => SubrError::Unsupported(format!("glyph {gid}: {m}")),
        other => other,
    }
}

// ── Charstring tokens ───────────────────────────────────────────────────────────────────────────

/// One charstring token: a number with its bytes, or an operator (`12 n` as `1200 + n`) with
/// its bytes, a hint mask's data included.
#[derive(Clone, Debug, PartialEq)]
enum Tok<'a> {
    Num(f64, &'a [u8]),
    Op(u16, &'a [u8]),
}

/// The tokens of an expanded charstring. Hint masks take one bit per stem declared before them,
/// implicit `vstem` operands before the first mask included.
fn tokens(code: &[u8]) -> Result<Vec<Tok<'_>>, SubrError> {
    let cut = || SubrError::Invalid("a charstring is cut short".into());
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut stems = 0usize;
    let mut i = 0usize;
    while let Some(&b) = code.get(i) {
        if b == 28 || b >= 32 {
            let (v, next) = read_t2_number(code, i).ok_or_else(cut)?;
            out.push(Tok::Num(v, code.get(i..next).ok_or_else(cut)?));
            depth += 1;
            i = next;
            continue;
        }
        let (op, mut next) = if b == 12 { (1200 + u16::from(*code.get(i + 1).ok_or_else(cut)?), i + 2) } else { (u16::from(b), i + 1) };
        match op {
            // hstem, vstem, hstemhm, vstemhm
            1 | 3 | 18 | 23 => stems += depth / 2,
            // hintmask, cntrmask
            19 | 20 => {
                stems += depth / 2;
                next += stems.div_ceil(8);
            }
            10 | 29 | 11 => return Err(SubrError::Invalid("a subroutine call in an expanded charstring".into())),
            _ => {}
        }
        out.push(Tok::Op(op, code.get(i..next).ok_or_else(cut)?));
        depth = 0;
        i = next;
    }
    Ok(out)
}

/// Whether `op` clears the stack first in a charstring, and so may carry the width, and how
/// many operands it takes without one (`None`: any even number).
fn width_carrier(op: u16) -> Option<Option<usize>> {
    match op {
        1 | 3 | 18 | 23 | 19 | 20 => Some(None),
        21 => Some(Some(2)),
        4 | 22 => Some(Some(1)),
        14 => Some(Some(0)),
        _ => None,
    }
}

/// Whether the first stack-clearing operator, given `args` operands, carries a width.
fn has_width(op: u16, args: usize) -> bool {
    match width_carrier(op) {
        Some(None) => args % 2 == 1,
        // endchar: no operands, or the four of seac.
        Some(Some(0)) => args == 1 || args == 5,
        Some(Some(n)) => args == n + 1,
        None => false,
    }
}

fn is_path(op: u16) -> bool {
    matches!(op, 4 | 5 | 6 | 7 | 8 | 21 | 22 | 24 | 25 | 26 | 27 | 30 | 31)
}

/// A move or a segment waiting to be written.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Move(f64, f64),
    Seg(RelSeg),
}

fn flush(pending: &mut Vec<Pending>, form: CharstringForm, max_stack: usize, out: &mut Vec<u8>) -> Result<(), SubrError> {
    if pending.is_empty() {
        return Ok(());
    }
    let what = "a charstring operand";
    let encode = |e: crate::Error| SubrError::Invalid(e.to_string());
    match form {
        CharstringForm::Specialize => {
            let cmds: Vec<Cmd> = pending
                .iter()
                .map(|p| match *p {
                    Pending::Move(x, y) => Cmd { op: RMOVETO, args: vec![x, y] },
                    Pending::Seg(RelSeg::Line(x, y)) => Cmd { op: RLINETO, args: vec![x, y] },
                    Pending::Seg(RelSeg::Curve(c)) => Cmd { op: RRCURVETO, args: c.to_vec() },
                })
                .collect();
            for c in specialize(cmds, max_stack, false) {
                for &a in &c.args {
                    encode_t2_number_exact(a, what, out).map_err(encode)?;
                }
                out.push(crate::charstring::opcode(c.op).ok_or_else(|| SubrError::Invalid("the specialiser left an unknown operator".into()))?);
            }
        }
        CharstringForm::General => {
            let mut run: Vec<RelSeg> = Vec::new();
            for p in pending.iter() {
                match *p {
                    Pending::Move(x, y) => {
                        write_segments(out, &run, max_stack).map_err(encode)?;
                        run.clear();
                        encode_t2_number_exact(x, what, out).map_err(encode)?;
                        encode_t2_number_exact(y, what, out).map_err(encode)?;
                        out.push(21);
                    }
                    Pending::Seg(s) => run.push(s),
                }
            }
            write_segments(out, &run, max_stack).map_err(encode)?;
        }
    }
    pending.clear();
    Ok(())
}

/// An expanded charstring with its path operators written in `form` (`None` keeps them) and,
/// when `drop_dotsection`, without `dotsection`. Hints, masks, flex, the width and `endchar`
/// are copied as they are.
pub fn rewrite_charstring(
    code: &[u8],
    cff2: bool,
    form: Option<CharstringForm>,
    drop_dotsection: bool,
    max_stack: usize,
) -> Result<Vec<u8>, SubrError> {
    let toks = tokens(code)?;
    let mut out = Vec::with_capacity(code.len() + 8);
    let mut stack: Vec<(f64, &[u8])> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut width_open = !cff2;
    for t in &toks {
        let (op, bytes) = match t {
            Tok::Num(v, b) => {
                stack.push((*v, *b));
                continue;
            }
            Tok::Op(op, b) => (*op, *b),
        };
        if form.is_some() && matches!(op, 15 | 16) {
            return Err(SubrError::Unsupported("rewriting charstrings that blend (a variable CFF2 table)".into()));
        }
        if width_open && width_carrier(op).is_some() {
            width_open = false;
            if has_width(op, stack.len()) && !stack.is_empty() {
                let (_, w) = stack.remove(0);
                out.extend_from_slice(w);
            }
        }
        match form {
            Some(_) if is_path(op) => {
                let args: Vec<f64> = stack.iter().map(|(v, _)| *v).collect();
                match op {
                    21 => pending.push(Pending::Move(args.first().copied().unwrap_or(0.0), args.get(1).copied().unwrap_or(0.0))),
                    22 => pending.push(Pending::Move(args.first().copied().unwrap_or(0.0), 0.0)),
                    4 => pending.push(Pending::Move(0.0, args.first().copied().unwrap_or(0.0))),
                    _ => {
                        let segs = generalize(op, &args)?.unwrap_or_default();
                        pending.extend(segs.into_iter().map(Pending::Seg));
                    }
                }
            }
            _ => {
                if let Some(f) = form {
                    flush(&mut pending, f, max_stack, &mut out)?;
                }
                if !(op == 1200 && drop_dotsection) {
                    for (_, b) in &stack {
                        out.extend_from_slice(b);
                    }
                    out.extend_from_slice(bytes);
                }
            }
        }
        stack.clear();
    }
    if let Some(f) = form {
        flush(&mut pending, f, max_stack, &mut out)?;
    }
    // Operands after the last operator have no meaning, but are kept as found.
    for (_, b) in &stack {
        out.extend_from_slice(b);
    }
    Ok(out)
}

/// An expanded `CFF ` charstring whose width was relative to the width pair `from`
/// (`defaultWidthX`, `nominalWidthX`), written for the pair `to`. The advance stays the same.
pub fn reencode_width(code: &[u8], from: (f64, f64), to: (f64, f64)) -> Result<Vec<u8>, SubrError> {
    let toks = tokens(code)?;
    let mut args = 0usize;
    let mut first: Option<(f64, usize)> = None;
    let mut width: Option<(f64, usize)> = None;
    for t in &toks {
        match t {
            Tok::Num(v, b) => {
                if args == 0 {
                    first = Some((*v, b.len()));
                }
                args += 1;
            }
            Tok::Op(op, _) => {
                if width_carrier(*op).is_none() {
                    return Err(SubrError::Invalid("the charstring does not start with a hint, a move or endchar".into()));
                }
                if has_width(*op, args) {
                    width = first;
                }
                break;
            }
        }
    }
    let advance = match width {
        Some((w, _)) => from.1 + w,
        None => from.0,
    };
    let skip = width.map_or(0, |(_, len)| len);
    let mut out = Vec::with_capacity(code.len() + 5);
    if advance != to.0 {
        let operands = args.saturating_sub(usize::from(width.is_some()));
        if operands >= CFF_STACK {
            return Err(SubrError::Invalid("a width operand would overflow the argument stack".into()));
        }
        encode_t2_number_exact(advance - to.1, "a glyph width", &mut out).map_err(|e| SubrError::Invalid(e.to_string()))?;
    }
    out.extend_from_slice(code.get(skip..).unwrap_or_default());
    Ok(out)
}

// ── Hint warnings ───────────────────────────────────────────────────────────────────────────────

/// What `warn_hints` finds in a set of expanded charstrings, by glyph id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HintFindings {
    /// Glyphs that draw a path without declaring a stem hint (`+H`).
    pub unhinted: Vec<usize>,
    /// Glyphs with a hint mask that repeats the one before it (`+d`).
    pub repeated_masks: Vec<usize>,
}

/// The `warn_hints` findings of `set`, whose charstrings have no subroutine calls.
pub fn hint_findings(set: &CharstringSet) -> Result<HintFindings, SubrError> {
    let mut found = HintFindings::default();
    for (gid, code) in set.charstrings.iter().enumerate() {
        let toks = tokens(code).map_err(|e| in_glyph(e, gid))?;
        let (mut stems, mut draws, mut repeated) = (false, false, false);
        let mut depth = 0usize;
        let mut last_mask: Option<&[u8]> = None;
        for t in &toks {
            match t {
                Tok::Num(..) => depth += 1,
                Tok::Op(op, bytes) => {
                    match *op {
                        1 | 3 | 18 | 23 => stems = true,
                        19 => {
                            // Operands before the first mask are implicit vstems.
                            stems |= depth >= 2;
                            let mask = bytes.get(1..).unwrap_or_default();
                            repeated |= last_mask == Some(mask);
                            last_mask = Some(mask);
                        }
                        20 => stems |= depth >= 2,
                        op if is_path(op) && !matches!(op, 4 | 21 | 22) => draws = true,
                        1234..=1237 => draws = true,
                        _ => {}
                    }
                    depth = 0;
                }
            }
        }
        if draws && !stems {
            found.unhinted.push(gid);
        }
        if repeated {
            found.repeated_masks.push(gid);
        }
    }
    Ok(found)
}

fn hint_warnings(set: &CharstringSet) -> Result<Vec<String>, SubrError> {
    let found = hint_findings(set)?;
    let mut out = Vec::new();
    if !found.unhinted.is_empty() {
        out.push(format!("glyphs that draw without stem hints: {} (glyph ids {})", found.unhinted.len(), listed(&found.unhinted)));
    }
    if !found.repeated_masks.is_empty() {
        out.push(format!(
            "glyphs with a hint mask that repeats the one before it: {} (glyph ids {})",
            found.repeated_masks.len(),
            listed(&found.repeated_masks)
        ));
    }
    Ok(out)
}

/// At most [`MAX_LISTED`] glyph ids, and how many more there are.
pub fn listed(gids: &[usize]) -> String {
    let shown: Vec<String> = gids.iter().take(MAX_LISTED).map(usize::to_string).collect();
    match gids.len().checked_sub(MAX_LISTED) {
        Some(more) if more > 0 => format!("{} and {more} more", shown.join(", ")),
        _ => shown.join(", "),
    }
}

#[cfg(test)]
mod tests;
