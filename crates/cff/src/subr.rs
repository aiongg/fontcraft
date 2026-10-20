//! The CFF subroutiniser (M12.25; row `compile.cffSubroutinisation`).
//!
//! [`subroutinize`] moves charstring fragments that repeat across glyphs into global and local
//! subroutines; [`desubroutinize`] expands every call back. Both work on a [`CharstringSet`]
//! (the charstrings, subroutines, FDSelect and stack limits of one `CFF ` or `CFF2` table);
//! [`subroutinize_table`] and [`desubroutinize_table`] do the same to the bytes of a whole table,
//! copying everything else and laying the table out again.
//!
//! How it works:
//!
//! 1. Every charstring is run with its calls followed ([`desubroutinize`]) and cut into tokens: a
//!    number, an operator, or `hintmask`/`cntrmask` with its mask bytes. A token keeps its exact
//!    bytes, so expanding the result gives back the input byte for byte.
//! 2. A suffix array and its LCP array over all tokens give every repeated token sequence that
//!    is followed by different tokens in different places, with its number of occurrences. The
//!    tree of these sequences is walked twice and never stored: the first walk prices every
//!    sequence, the second records only the candidates kept.
//! 3. An iterative market chooses the encoding: each candidate has a price; dynamic programming
//!    finds the cheapest way to write every glyph and every candidate from tokens and calls; the
//!    prices follow the use each candidate gets; candidates that do not pay for themselves are
//!    dropped between rounds. This is the method of compreffor's `pyCompressor` (Apache-2.0,
//!    Copyright 2015 Google Inc.), ported: the savings formula, the price update with its
//!    constants, the cut-down heuristic, the placement into global and local INDEXes, the
//!    nesting cut and the ordering that gives the most used subroutines one-byte numbers.
//! 4. The result is expanded again and compared with the input; a difference is an error, never
//!    a changed outline.
//!
//! Differences from compreffor: occurrences are counted with the standard LCP-interval traversal
//! (compreffor drops some intervals), call sites are restricted so that the subroutine number
//! never overflows the operand stack, nesting depth is computed from the glyphs' actual calls,
//! a `blend` token belongs to the `vsindex` in force so that a shared subroutine always blends
//! the same way, and the work is single-threaded and bounded by the limits below.
//!
//! Every input is untrusted: counts, lengths and work are capped by the named limits, and
//! anything beyond them returns a [`SubrError`]; nothing panics. Memory grows linearly with the
//! tokens: a glyph is dropped as soon as its tokens are read, at most a few arrays of one entry
//! per token are held at once (about 30 bytes per token and 100 per candidate at the peak),
//! and the large arrays are charged to a budget of [`MAX_SUBR_MEMORY`]. A full CJK
//! font (65,535 glyphs, about 25 million tokens) fits twice over on a 64-bit target.

mod find;
mod flat;
mod market;
mod memory;
mod prune;
mod suffix;
mod table;

use std::fmt;

pub use prune::prune_unused;
pub use table::{
    SubrMode, TableSubset, WidthsReport, charstrings_of_table, desubroutinize_table, desubroutinize_table_within, optimize_widths_table,
    subroutinize_table, subset_table, subset_table_noted,
};

/// Deepest subroutine nesting the charstring formats allow: a glyph's own charstring is depth 0
/// and every call adds one.
pub const MAX_CALL_DEPTH: usize = 10;
/// Longest charstring or subroutine accepted or written, in bytes.
pub const MAX_CHARSTRING: usize = 65_535;
/// Most entries one subroutine INDEX may hold (the reach of a biased 16-bit subroutine number).
pub const MAX_SUBRS_PER_INDEX: usize = 65_535;
/// Default for [`SubrOptions::max_subrs`] (compreffor's limit).
pub const DEFAULT_MAX_SUBRS: usize = 65_533;
/// Most glyphs a set may hold.
pub const MAX_GLYPHS: usize = 65_535;
/// Most Font DICTs a set may hold.
pub const MAX_FONT_DICTS: usize = 256;
/// Largest operand stack accepted (CFF2's ceiling for `maxstack`).
pub const MAX_STACK_LIMIT: usize = 513;
/// Operand stack of a `CFF ` charstring.
pub const CFF_STACK: usize = 48;
/// Operand stack of a `CFF2` table that does not set `maxstack`: 513, as fontTools takes it (it
/// treats `maxstack` as deprecated and the stack as always 513; the operator's old default was
/// 193). Charstrings specialised as fontTools specialises them fill the stack up to 512.
pub const CFF2_DEFAULT_STACK: usize = 513;
/// Most operators one glyph may run while its calls are followed.
pub const MAX_OPS_PER_GLYPH: usize = 1 << 20;
/// Most operators all glyphs of one set may run together while their calls are followed.
pub const MAX_OPS_PER_SET: usize = 1 << 28;
/// Most tokens all expanded charstrings may hold together, one separator per glyph included: on
/// a 64-bit target over twice a full CJK font (Source Han Serif Bold: 25.5 million), on a 32-bit
/// one (wasm) what [`MAX_SUBR_MEMORY`] holds there.
pub const MAX_TOKENS: usize = if usize::BITS >= 64 { 1 << 26 } else { 1 << 24 };
/// Most bytes the subroutiniser's large arrays may take together. On a 32-bit target (wasm) it
/// is the memory a table's charstrings may take there, which leaves the rest of the 4 GiB address
/// space to the app.
#[cfg(target_pointer_width = "64")]
pub const MAX_SUBR_MEMORY: usize = 4 << 30;
/// Most bytes the subroutiniser's large arrays may take together. On a 32-bit target (wasm) it
/// is the memory a table's charstrings may take there, which leaves the rest of the 4 GiB address
/// space to the app.
#[cfg(not(target_pointer_width = "64"))]
pub const MAX_SUBR_MEMORY: usize = 512 << 20;
/// Most tokens the candidate subroutines may hold together, or the tokens of the text when there
/// are more; candidates are kept in order of their estimated saving until this is reached.
pub const MAX_CANDIDATE_TOKENS: usize = 1 << 23;
/// Most candidates the dynamic programming looks at for one token position.
pub const MAX_CANDIDATES_PER_POSITION: usize = 64;
/// Most rounds of price adjustment.
pub const MAX_ROUNDS: usize = 16;

/// The charstrings and subroutines of one `CFF ` or `CFF2` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharstringSet {
    /// `CFF2` charstrings: no `endchar`, `return` or width; `blend` and `vsindex` allowed.
    pub cff2: bool,
    /// One per glyph, in glyph order.
    pub charstrings: Vec<Vec<u8>>,
    pub global_subrs: Vec<Vec<u8>>,
    /// One entry per Font DICT (one for a name-keyed font), empty where it has no `Subrs`.
    pub local_subrs: Vec<Vec<Vec<u8>>>,
    /// The Font DICT of each glyph; empty when there is one Font DICT.
    pub fd_select: Vec<u16>,
    /// `CFF2`: the `vsindex` each Font DICT's Private DICT sets (0 when empty or missing).
    pub fd_vsindex: Vec<u16>,
    /// `CFF2`: the number of regions of each item variation data, by `vsindex`.
    pub regions: Vec<usize>,
    /// Operand stack size: [`CFF_STACK`], or the `CFF2` table's `maxstack`.
    pub max_stack: usize,
}

impl Default for CharstringSet {
    fn default() -> Self {
        CharstringSet {
            cff2: false,
            charstrings: Vec::new(),
            global_subrs: Vec::new(),
            local_subrs: vec![Vec::new()],
            fd_select: Vec::new(),
            fd_vsindex: Vec::new(),
            regions: Vec::new(),
            max_stack: CFF_STACK,
        }
    }
}

impl CharstringSet {
    /// Number of Font DICTs.
    pub fn fd_count(&self) -> usize {
        self.local_subrs.len().max(1)
    }

    /// The Font DICT of glyph `gid`.
    pub fn fd_of(&self, gid: usize) -> usize {
        self.fd_select.get(gid).map_or(0, |&fd| usize::from(fd))
    }

    /// Bytes of the CharStrings INDEX, the Global Subr INDEX and every local Subrs INDEX
    /// together: what subroutinising changes in the table.
    pub fn index_bytes(&self) -> usize {
        let index = |items: &[Vec<u8>]| {
            let data: usize = items.iter().map(Vec::len).sum();
            let head = if self.cff2 { 4 } else { 2 };
            if items.is_empty() {
                return head;
            }
            let off = match data + 1 {
                0..=0xFF => 1,
                0x100..=0xFFFF => 2,
                0x1_0000..=0xFF_FFFF => 3,
                _ => 4,
            };
            head + 1 + (items.len() + 1) * off + data
        };
        let locals: usize = self.local_subrs.iter().filter(|l| !l.is_empty()).map(|l| index(l)).sum();
        index(&self.charstrings) + index(&self.global_subrs) + locals
    }

    fn validate(&self) -> Result<(), SubrError> {
        let glyphs = self.charstrings.len();
        if glyphs > MAX_GLYPHS {
            return Err(SubrError::TooLarge { what: "glyphs in a CFF table", limit: MAX_GLYPHS });
        }
        let fds = self.fd_count();
        if fds > MAX_FONT_DICTS {
            return Err(SubrError::TooLarge { what: "Font DICTs in a CFF table", limit: MAX_FONT_DICTS });
        }
        if !self.fd_select.is_empty() && self.fd_select.len() != glyphs {
            return Err(SubrError::Invalid(format!("the FDSelect covers {} glyphs, not {glyphs}", self.fd_select.len())));
        }
        if let Some(fd) = self.fd_select.iter().find(|&&fd| usize::from(fd) >= fds) {
            return Err(SubrError::Invalid(format!("a glyph selects Font DICT {fd}, but there are {fds}")));
        }
        if !(1..=MAX_STACK_LIMIT).contains(&self.max_stack) {
            return Err(SubrError::Invalid(format!("an operand stack of {} entries", self.max_stack)));
        }
        let subr_lists = std::iter::once(&self.global_subrs).chain(self.local_subrs.iter());
        if subr_lists.clone().any(|l| l.len() > MAX_SUBRS_PER_INDEX) {
            return Err(SubrError::TooLarge { what: "subroutines in one INDEX", limit: MAX_SUBRS_PER_INDEX });
        }
        let too_long = self.charstrings.iter().chain(subr_lists.flatten()).any(|c| c.len() > MAX_CHARSTRING);
        if too_long {
            return Err(SubrError::TooLarge { what: "bytes in one charstring", limit: MAX_CHARSTRING });
        }
        Ok(())
    }
}

/// Settings of [`subroutinize`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubrOptions {
    /// Most subroutines in one INDEX (the global one, or one Font DICT's local one); at most
    /// [`MAX_SUBRS_PER_INDEX`].
    pub max_subrs: usize,
    /// Rounds of price adjustment, 1 to [`MAX_ROUNDS`].
    pub rounds: usize,
}

impl Default for SubrOptions {
    fn default() -> Self {
        SubrOptions { max_subrs: DEFAULT_MAX_SUBRS, rounds: 4 }
    }
}

/// Why charstrings could not be subroutinised or expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubrError {
    /// The table or a charstring is malformed.
    Invalid(String),
    /// Valid, but uses something this module does not handle (arithmetic operators, several
    /// fonts in one table).
    Unsupported(String),
    /// An input or the work it needs is beyond one of the limits.
    TooLarge { what: &'static str, limit: usize },
    /// The subroutinised charstring of this glyph does not expand to its input. The input should
    /// be kept; nothing was changed.
    Mismatch { glyph: usize },
}

impl fmt::Display for SubrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubrError::Invalid(m) => write!(f, "invalid CFF data: {m}"),
            SubrError::Unsupported(m) => write!(f, "not supported in CFF data: {m}"),
            SubrError::TooLarge { what, limit } => write!(f, "too many {what} (the limit is {limit})"),
            SubrError::Mismatch { glyph } => {
                write!(f, "the subroutinised charstring of glyph {glyph} does not expand to the original; the font was left as it was")
            }
        }
    }
}

impl std::error::Error for SubrError {}

/// The set with every subroutine call expanded and no subroutines left.
pub fn desubroutinize(set: &CharstringSet) -> Result<CharstringSet, SubrError> {
    desubroutinize_within(set, usize::MAX)
}

/// [`desubroutinize`], refused once the expanded charstrings hold more than `max_bytes`
/// together. Each glyph is expanded and charged before the next one is, so memory stays near
/// the limit.
pub fn desubroutinize_within(set: &CharstringSet, max_bytes: usize) -> Result<CharstringSet, SubrError> {
    set.validate()?;
    let mut budget = MAX_OPS_PER_SET;
    let mut left = max_bytes;
    let mut charstrings = Vec::with_capacity(set.charstrings.len());
    for gid in 0..set.charstrings.len() {
        let bytes = flat::flatten(set, gid, &mut budget).map_err(|e| in_glyph(e, gid))?.bytes;
        left = left.checked_sub(bytes.len()).ok_or(SubrError::TooLarge { what: "bytes in the expanded charstrings", limit: max_bytes })?;
        charstrings.push(bytes);
    }
    Ok(CharstringSet { charstrings, global_subrs: Vec::new(), local_subrs: vec![Vec::new(); set.fd_count()], ..set.clone() })
}

/// The set with repeated charstring fragments moved into subroutines. Existing subroutines are
/// expanded first. The result is checked by expanding it again; a glyph that does not come back
/// unchanged makes this return [`SubrError::Mismatch`].
pub fn subroutinize(set: &CharstringSet, options: &SubrOptions) -> Result<CharstringSet, SubrError> {
    subroutinize_measured(set, options, MAX_SUBR_MEMORY).map(|(result, _)| result)
}

/// What one run of the subroutiniser took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubrStats {
    /// Tokens of all expanded charstrings, one separator per glyph included.
    pub tokens: usize,
    /// Repeated sequences kept as candidate subroutines.
    pub candidates: usize,
    /// The most bytes charged to the memory budget at once.
    pub peak_memory: usize,
}

/// [`subroutinize`] within a memory budget of `max_memory` bytes, with what it took.
pub fn subroutinize_measured(set: &CharstringSet, options: &SubrOptions, max_memory: usize) -> Result<(CharstringSet, SubrStats), SubrError> {
    set.validate()?;
    let glyphs = set.charstrings.len();
    let fd_of: Vec<usize> = (0..glyphs).map(|g| set.fd_of(g)).collect();
    let params = market::Params {
        fd_count: set.fd_count(),
        cff2: set.cff2,
        max_stack: set.max_stack,
        max_subrs: options.max_subrs.clamp(1, MAX_SUBRS_PER_INDEX),
        rounds: options.rounds.clamp(1, MAX_ROUNDS),
    };
    let mut mem = memory::Memory::new(max_memory);
    let mut budget = MAX_OPS_PER_SET;
    let next = |gid: usize| flat::flatten(set, gid, &mut budget).map_err(|e| in_glyph(e, gid));
    let out = market::subroutinize(glyphs, next, &fd_of, &params, &mut mem)?;
    let stats = SubrStats { tokens: out.tokens, candidates: out.candidates, peak_memory: mem.peak() };
    let result = CharstringSet { charstrings: out.charstrings, global_subrs: out.global, local_subrs: out.local, ..set.clone() };
    // The glyphs are expanded again, one at a time, rather than kept from the first expansion.
    let (mut before, mut after) = (MAX_OPS_PER_SET, MAX_OPS_PER_SET);
    for gid in 0..glyphs {
        let original = flat::flatten(set, gid, &mut before).map_err(|e| in_glyph(e, gid))?;
        let again = flat::flatten(&result, gid, &mut after).map_err(|_| SubrError::Mismatch { glyph: gid })?;
        if again.bytes != original.bytes {
            return Err(SubrError::Mismatch { glyph: gid });
        }
    }
    Ok((result, stats))
}

fn in_glyph(e: SubrError, gid: usize) -> SubrError {
    match e {
        SubrError::Invalid(m) => SubrError::Invalid(format!("glyph {gid}: {m}")),
        SubrError::Unsupported(m) => SubrError::Unsupported(format!("glyph {gid}: {m}")),
        other => other,
    }
}

/// The bias added to a subroutine number for an INDEX of `count` subroutines.
fn bias(count: usize) -> i64 {
    if count < 1240 {
        107
    } else if count < 33_900 {
        1131
    } else {
        32_768
    }
}

/// Appends a charstring integer in its shortest form (`v` within ±32,767).
fn push_int(v: i64, out: &mut Vec<u8>) -> Result<(), SubrError> {
    let byte = |x: i64| u8::try_from(x).map_err(|_| SubrError::Invalid(format!("the integer {v} cannot be written")));
    if (-107..=107).contains(&v) {
        out.push(byte(v + 139)?);
    } else if (108..=1131).contains(&v) {
        let w = v - 108;
        out.extend_from_slice(&[byte((w >> 8) + 247)?, byte(w & 0xFF)?]);
    } else if (-1131..=-108).contains(&v) {
        let w = -v - 108;
        out.extend_from_slice(&[byte((w >> 8) + 251)?, byte(w & 0xFF)?]);
    } else {
        let short = i16::try_from(v).map_err(|_| SubrError::Invalid(format!("the integer {v} cannot be written")))?;
        out.push(28);
        out.extend_from_slice(&short.to_be_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
