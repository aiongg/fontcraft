//! A writer for the `CFF2` table of a variable OpenType font (M9.13).
//!
//! Rows: `cff2.export`, the `CFF2` table row, `compile.cff2VariableBuild`.
//!
//! [`write_cff2`] turns a [`Cff2Font`] (the variation regions, the region lists each glyph's
//! blends use, and per glyph an outline whose numbers are a default plus one delta per region of
//! its list) into the bytes of a `CFF2` table:
//!
//! - **Charstrings**: the segments specialised as fontTools' `specializeCommands` does for a
//!   `CFF2` merge (`hlineto`, `vvcurveto`, `rcurveline` …, combined while fewer than
//!   [`MAX_STACK`] operands are on the stack, blended operands counted with their deltas), each
//!   varying argument written with `blend`, and `vsindex` first where a glyph uses a region list
//!   other than the first. Consecutive varying arguments share one `blend` while the operand stack
//!   has room; otherwise the run is split. [`Cff2Font::generic`] keeps one generic operator per
//!   segment instead. No subroutines (`crate::subr::subroutinize_table` adds them), no widths.
//! - **Hints** ([`Cff2Glyph::hints`]): laid out as the static encoder lays them out (stem
//!   operators, counter masks, the outline specialised in runs between hint masks), with each
//!   varying stem number blended. One hint structure serves every master, as in fontTools'
//!   merge of hinted masters; [`shared_hints`] merges the masters' static hints into it.
//! - **Top DICT**: FontMatrix when given, CharStrings, FDArray and `vstore`, then `maxstack`
//!   ([`MAX_STACK`]) only when a charstring needs more than the specification's default of
//!   [`SPEC_DEFAULT_STACK`] operands. One Font DICT, so no FDSelect.
//! - **Private DICT**: blue zones and stems, each varying value written with `blend`.
//! - **Variation store**: the item variation store with no items, one variation data per region
//!   list.
//!
//! Glyph names are not stored; the compiler writes them in `post`.
//!
//! Relative coordinates and the blend layout follow fontTools' `varLib.cff` and `cffLib`
//! (`CFF2CharStringMergePen`, `merge_PrivateDicts`, `commandsToProgram`; MIT, Copyright (c) 2017
//! Just van Rossum and contributors), written anew here.
//!
//! Every input is untrusted: counts are checked against the limits below and the static writer's
//! ([`crate::MAX_GLYPHS`], [`crate::MAX_SEGMENTS`], [`crate::MAX_COORDINATE`]); anything out of range
//! returns a [`Cff2Error`]; nothing panics.

use std::fmt;

use crate::bounds::Bounds;
use crate::charstring::{bounds, opcode, pen_commands};
use crate::hints::{BOTTOM_GHOST, STEMS_PER_OPERATOR, TOP_GHOST};
use crate::number::{encode_dict_int, encode_dict_number, encode_t2_number, round_with_tolerance};
use crate::specialize::{Cmd, RLINETO, RMOVETO, RRCURVETO, SpecArg, specialize};
use crate::{
    CharstringHints, Error, MAX_COORDINATE, MAX_COUNTER_MASKS, MAX_GLYPHS, MAX_SEGMENTS, MAX_STEM_HINTS, MAX_TABLE_SIZE, Mask, Number, PathOp,
    StemHint,
};

/// Most regions the variation store may hold (a 16-bit count).
pub const MAX_REGIONS: usize = 65_535;
/// Most region lists (`vsindex` values) the variation store may hold (a 16-bit count).
pub const MAX_REGION_LISTS: usize = 65_535;
/// Most axes a region may have (`fvar` holds at most this many in practice).
pub const MAX_AXES: usize = 64;
/// Most operands the charstring interpreter's stack holds (the ceiling of CFF2's `maxstack`): a
/// `blend` of `n` values with `k` regions needs `n * (k + 1) + 1`.
pub const MAX_STACK: usize = 513;
/// The operand stack a reader that follows the CFF2 specification assumes when the Top DICT has
/// no `maxstack`. fontTools never writes `maxstack`, so it is left out whenever this is enough.
pub const SPEC_DEFAULT_STACK: usize = 193;
/// Most regions a region list may have: one varying value blended over them fills the stack.
pub const MAX_BLEND_REGIONS: usize = MAX_STACK - 2;
/// Most values one Private DICT list (BlueValues and the like) may have.
pub const MAX_PRIVATE_LIST: usize = 14;

/// Charstring operators.
const OP_VSINDEX: u8 = 15;
const OP_BLEND: u8 = 16;
const OP_HINTMASK: u8 = 19;
const OP_CNTRMASK: u8 = 20;
/// DICT operators.
const DICT_BLEND: u8 = 23;
const DICT_MAXSTACK: u8 = 25;

/// One axis of a variation region, in normalized coordinates (`start <= peak <= end`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RegionAxis {
    pub start: f64,
    pub peak: f64,
    pub end: f64,
}

/// A number that varies: its value at the origin and one delta per region of the glyph's list.
/// No deltas means the number does not vary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blend {
    pub default: f64,
    pub deltas: Vec<f64>,
}

impl Blend {
    /// A number that does not vary.
    pub fn fixed(default: f64) -> Self {
        Blend { default, deltas: Vec::new() }
    }

    fn varies(&self) -> bool {
        self.deltas.iter().any(|d| *d != 0.0)
    }
}

/// A point whose coordinates vary.
pub type BlendPoint = (Blend, Blend);

/// One drawing step of a variable outline, as [`crate::PathOp`] with varying points (absolute
/// coordinates and absolute deltas; the writer makes them relative).
#[derive(Clone, Debug, PartialEq)]
pub enum BlendOp {
    MoveTo(BlendPoint),
    LineTo(BlendPoint),
    CurveTo(BlendPoint, BlendPoint, BlendPoint),
}

/// A glyph of a `CFF2` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Glyph {
    /// For messages only; `CFF2` stores no names.
    pub name: String,
    /// The region list its blends use (an index into [`Cff2Font::region_lists`]).
    pub vsindex: u16,
    pub path: Vec<BlendOp>,
    /// Its stem hints and masks, blended over the same regions as the path; `None` or no stems
    /// for an unhinted charstring.
    pub hints: Option<Cff2Hints>,
}

/// A stem hint whose edge and width vary, as [`StemHint`] stores them (absolute values; a
/// ghost's width is the fixed −20 or −21).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendStem {
    pub edge: Blend,
    pub width: Blend,
}

/// The hints of a `CFF2` glyph: one structure for the whole design space, with only the stem
/// positions varying. Stems are written in the order given; masks name operations of
/// [`Cff2Glyph::path`] as [`CharstringHints::masks`] does. Flex is not kept: a `CFF2` merge draws
/// it as two curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Hints {
    pub h_stems: Vec<BlendStem>,
    pub v_stems: Vec<BlendStem>,
    pub masks: Vec<(usize, Mask)>,
    pub counters: Vec<Mask>,
}

impl Cff2Hints {
    pub fn is_empty(&self) -> bool {
        self.h_stems.is_empty() && self.v_stems.is_empty()
    }
}

/// The hints of one glyph in every master, merged into one structure: per stem (in the order
/// the charstring writes them) its value in each master, and the masks and counter masks all
/// masters share.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SharedHints {
    pub h_stems: Vec<Vec<StemHint>>,
    pub v_stems: Vec<Vec<StemHint>>,
    pub masks: Vec<(usize, Mask)>,
    pub counters: Vec<Mask>,
}

/// Merges the hints a static export writes for each master of a glyph (`masters`, in the
/// glyph's source order) into the one structure a `CFF2` charstring holds, as fontTools'
/// `CFF2CharStringMergePen` merges hinted masters: every master must have the same stem
/// operators with the same number of stems, and the same masks; only the stem numbers may
/// differ. Stems are normalised and sorted as the static encoder writes them, with `tolerance`.
/// The stems must also sort into the same order in every master and be the same kind (stem, top
/// or bottom ghost), so that a blend never joins two different stems. Flex is dropped. The error
/// says how the masters differ.
pub fn shared_hints(masters: &[CharstringHints], tolerance: f64) -> Result<SharedHints, String> {
    let kind = |s: &StemHint| {
        if s.width == TOP_GHOST {
            1u8
        } else if s.width == BOTTOM_GHOST {
            2
        } else {
            0
        }
    };
    let normalised: Vec<CharstringHints> = masters.iter().map(|h| h.normalised(tolerance)).collect();
    let orders: Vec<(Vec<usize>, Vec<usize>)> =
        masters.iter().map(|h| (crate::hints::sorted(&h.h_stems, tolerance).1, crate::hints::sorted(&h.v_stems, tolerance).1)).collect();
    let Some(base) = normalised.first() else { return Ok(SharedHints::default()) };
    let base_order = orders.first().cloned().unwrap_or_default();
    for (i, (n, order)) in normalised.iter().zip(&orders).enumerate().skip(1) {
        if n.h_stems.len() != base.h_stems.len() || n.v_stems.len() != base.v_stems.len() {
            return Err(format!(
                "master {i} has {} horizontal and {} vertical stems, the first {} and {}",
                n.h_stems.len(),
                n.v_stems.len(),
                base.h_stems.len(),
                base.v_stems.len()
            ));
        }
        if *order != base_order {
            return Err(format!("the stems of master {i} lie in a different order"));
        }
        let kinds = |list: &[StemHint]| list.iter().map(kind).collect::<Vec<u8>>();
        if kinds(&n.h_stems) != kinds(&base.h_stems) || kinds(&n.v_stems) != kinds(&base.v_stems) {
            return Err(format!("a ghost hint of master {i} is a stem or a ghost of the other kind in the first"));
        }
        if n.masks != base.masks || n.counters != base.counters {
            return Err(format!("the hint masks of master {i} differ from the first's"));
        }
    }
    let per_stem = |get: fn(&CharstringHints) -> &Vec<StemHint>| -> Vec<Vec<StemHint>> {
        (0..get(base).len()).map(|j| normalised.iter().filter_map(|n| get(n).get(j).copied()).collect()).collect()
    };
    Ok(SharedHints {
        h_stems: per_stem(|h| &h.h_stems),
        v_stems: per_stem(|h| &h.v_stems),
        masks: base.masks.clone(),
        counters: base.counters.clone(),
    })
}

/// Private DICT values; the lists and stems may vary with the regions of list 0. Lists hold
/// absolute values (the writer stores them as differences).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Private {
    pub blue_values: Vec<Blend>,
    pub other_blues: Vec<Blend>,
    pub family_blues: Vec<Blend>,
    pub family_other_blues: Vec<Blend>,
    pub std_hw: Option<Blend>,
    pub std_vw: Option<Blend>,
    pub stem_snap_h: Vec<Blend>,
    pub stem_snap_v: Vec<Blend>,
    /// Written when they vary or are not the CFF defaults (0.039625, 7, 1); the language group
    /// when not 0.
    pub blue_scale: Option<Blend>,
    pub blue_shift: Option<Blend>,
    pub blue_fuzz: Option<Blend>,
    pub language_group: Option<i64>,
}

/// Everything a `CFF2` table holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cff2Font {
    pub axis_count: u16,
    /// Each region has one entry per axis.
    pub regions: Vec<Vec<RegionAxis>>,
    /// Region lists: indices into `regions`. List 0 is the default `vsindex`.
    pub region_lists: Vec<Vec<u16>>,
    /// In glyph order; glyph 0 is `.notdef`.
    pub glyphs: Vec<Cff2Glyph>,
    pub private: Cff2Private,
    /// The FontMatrix, when the units per em are not 1000.
    pub font_matrix: Option<[f64; 6]>,
    /// As [`crate::Font::round_tolerance`], for relative coordinates and deltas.
    pub round_tolerance: f64,
    /// Write one generic operator per segment (`rmoveto`, `rlineto`, `rrcurveto`) instead of
    /// specialising the charstrings.
    pub generic: bool,
}

/// A written `CFF2` table and what the other tables need from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cff2Built {
    pub table: Vec<u8>,
    /// Per glyph, the bounds of its outline at the origin; `None` for an empty glyph.
    pub glyph_bounds: Vec<Option<Bounds>>,
    /// The glyphs written without their hints because a stem's blend does not fit the operand
    /// stack, in glyph order.
    pub unhinted: Vec<String>,
}

/// Why a `CFF2` table could not be built.
#[derive(Clone, Debug, PartialEq)]
pub enum Cff2Error {
    /// A check the static writer shares (glyph counts, coordinates, segments).
    Cff(Error),
    /// A glyph's blends do not have one delta per region of its list, or name a list that does
    /// not exist.
    BadBlend { glyph: String, reason: String },
    /// More regions or region lists than the variation store can hold.
    TooManyRegions(usize),
    /// A blend that needs more operands than [`MAX_STACK`].
    StackOverflow { glyph: String },
    /// A region list with more regions than one blend can hold ([`MAX_BLEND_REGIONS`]).
    TooManyBlendRegions(usize),
}

impl fmt::Display for Cff2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cff2Error::Cff(e) => write!(f, "{e}"),
            Cff2Error::BadBlend { glyph, reason } => write!(f, "glyph `{glyph}`: {reason}"),
            Cff2Error::TooManyRegions(n) => write!(f, "{n} variation regions is more than a CFF2 table can hold ({MAX_REGIONS})"),
            Cff2Error::StackOverflow { glyph } => write!(f, "glyph `{glyph}` varies more values at once than a CFF2 charstring can hold"),
            Cff2Error::TooManyBlendRegions(n) => {
                write!(f, "a region list has {n} variation regions; a CFF2 value blends over at most {MAX_BLEND_REGIONS}")
            }
        }
    }
}

impl std::error::Error for Cff2Error {}

impl From<Error> for Cff2Error {
    fn from(e: Error) -> Self {
        Cff2Error::Cff(e)
    }
}

/// An operand: a plain number, or a default with one delta per region.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Plain(f64),
    Blend(f64, Vec<f64>),
}

/// The reference's blend lists: a blended operand is never zero, adding a plain number moves its
/// default, two blended operands add delta by delta, and while pushed it takes its default, its
/// deltas and the count of one blend.
impl SpecArg for Arg {
    fn zero() -> Self {
        Arg::Plain(0.0)
    }
    fn is_zero(&self) -> bool {
        matches!(self, Arg::Plain(v) if *v == 0.0)
    }
    fn add(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Arg::Plain(a), Arg::Plain(b)) => Some(Arg::Plain(a + b)),
            (Arg::Plain(p), Arg::Blend(d, deltas)) | (Arg::Blend(d, deltas), Arg::Plain(p)) => Some(Arg::Blend(d + p, deltas.clone())),
            (Arg::Blend(a, da), Arg::Blend(b, db)) if da.len() == db.len() => {
                Some(Arg::Blend(a + b, da.iter().zip(db).map(|(x, y)| x + y).collect()))
            }
            (Arg::Blend(..), Arg::Blend(..)) => None,
        }
    }
    fn peak(&self) -> usize {
        match self {
            Arg::Plain(_) => 1,
            Arg::Blend(_, deltas) => deltas.len().saturating_add(2),
        }
    }
}

/// How numbers are encoded: in a charstring or in a DICT.
#[derive(Clone, Copy, PartialEq)]
enum Context {
    Charstring,
    Dict,
}

/// Appends `args` with `blend` operators, keeping the stack within [`MAX_STACK`]. `regions` is
/// the number of deltas each blended argument has. In a charstring a run of blended arguments
/// grows only while it stays below the limit, as fontTools' `_convertToBlendCmds` grows it, so a
/// subroutine call still fits; a DICT list fills the stack if it must, since readers take its
/// operands as one blend.
fn write_args(args: &[Arg], regions: usize, ctx: Context, what: &str, out: &mut Vec<u8>) -> Result<(), Cff2Error> {
    let style = OperandStyle { dict: ctx == Context::Dict, max_stack: MAX_STACK, fill_stack: false, exact_numbers: false };
    write_operands(args, regions, style, what, out)
}

/// How [`write_operands`] writes numbers and groups blended operands. The `CFF2` builder writes
/// with `max_stack` at [`MAX_STACK`] and both switches off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperandStyle {
    /// DICT operands (`blend` is operator 23, numbers in DICT form) rather than charstring ones.
    pub dict: bool,
    /// The operand stack the blends must fit.
    pub max_stack: usize,
    /// A run of blended charstring operands grows until it fills the stack, as a DICT run always
    /// does; otherwise it stays one below the limit after its first value, so a subroutine call
    /// still fits.
    pub fill_stack: bool,
    /// Charstring numbers through [`crate::raw::encode_t2_number_exact`] (no rounding to the
    /// nearest 16.16 value first).
    pub exact_numbers: bool,
}

/// Appends `args`, runs of blended ones as `blend` operators that fit `style.max_stack`.
/// `regions` is the number of deltas each blended argument must have.
pub fn write_operands(args: &[Arg], regions: usize, style: OperandStyle, what: &str, out: &mut Vec<u8>) -> Result<(), Cff2Error> {
    let number = |v: f64, out: &mut Vec<u8>| -> Result<(), Cff2Error> {
        match (style.dict, style.exact_numbers) {
            (false, false) => encode_t2_number(v, what, out)?,
            (false, true) => crate::number::encode_t2_number_exact(v, what, out)?,
            (true, _) if v.fract() == 0.0 && v.abs() < 2_147_483_648.0 => encode_dict_int(v as i64, what, out)?,
            (true, _) => encode_dict_number(Number::Real(v), what, out)?,
        }
        Ok(())
    };
    let max_stack = style.max_stack;
    let overflow = || Cff2Error::StackOverflow { glyph: what.to_string() };
    let per_value = regions.checked_add(1).ok_or_else(overflow)?;
    let mut depth = 0usize;
    let mut i = 0usize;
    while let Some(arg) = args.get(i) {
        match arg {
            Arg::Plain(v) => {
                number(*v, out)?;
                depth += 1;
                i += 1;
            }
            Arg::Blend(..) => {
                // The longest run of blended arguments starting here that fits the stack.
                let mut run: Vec<(&f64, &Vec<f64>)> = Vec::new();
                while let Some(Arg::Blend(d, deltas)) = args.get(i + run.len()) {
                    let n = run.len() + 1;
                    let need = depth.saturating_add(n.saturating_mul(per_value)).saturating_add(1);
                    let fits = if run.is_empty() || style.dict || style.fill_stack { need <= max_stack } else { need < max_stack };
                    if !fits {
                        break;
                    }
                    run.push((d, deltas));
                }
                if run.is_empty() {
                    return Err(overflow());
                }
                for (d, _) in &run {
                    number(**d, out)?;
                }
                for (_, deltas) in &run {
                    if deltas.len() != regions {
                        return Err(Cff2Error::BadBlend { glyph: what.to_string(), reason: "a blend has the wrong number of deltas".into() });
                    }
                    for v in deltas.iter() {
                        number(*v, out)?;
                    }
                }
                number(run.len() as f64, out)?;
                out.push(if style.dict { DICT_BLEND } else { OP_BLEND });
                depth += run.len();
                i += run.len();
            }
        }
    }
    Ok(())
}

/// Checks a coordinate and its deltas.
fn check_blend(b: &Blend, regions: usize, glyph: &str) -> Result<(), Cff2Error> {
    let ok = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
    if !ok(b.default) || !b.deltas.iter().all(|d| ok(*d)) {
        return Err(Error::OutOfRange { what: format!("glyph `{glyph}`"), value: b.default }.into());
    }
    if !b.deltas.is_empty() && b.deltas.len() != regions {
        return Err(Cff2Error::BadBlend {
            glyph: glyph.to_string(),
            reason: format!("a value has {} deltas; its region list has {regions}", b.deltas.len()),
        });
    }
    Ok(())
}

/// The charstring of `glyph` (with `regions` regions in its list), its bounds at the origin, and
/// whether its hints were left out; specialised unless `generic`.
fn charstring(glyph: &Cff2Glyph, regions: usize, tolerance: f64, generic: bool) -> Result<(Vec<u8>, Option<Bounds>, bool), Cff2Error> {
    let name = glyph.name.as_str();
    if glyph.path.len() > MAX_SEGMENTS {
        return Err(Error::TooManySegments { glyph: name.to_string(), count: glyph.path.len() }.into());
    }
    let mut default_path: Vec<PathOp> = Vec::with_capacity(glyph.path.len());
    let mut last = (0.0f64, vec![0.0f64; regions], 0.0f64, vec![0.0f64; regions]);
    let mut commands: Vec<Cmd<Arg>> = Vec::with_capacity(glyph.path.len());
    let rel = |b: &Blend, last_default: &mut f64, last_deltas: &mut Vec<f64>, args: &mut Vec<Arg>| {
        let d = round_with_tolerance(b.default, tolerance);
        let rd = d - *last_default;
        *last_default = d;
        let deltas: Vec<f64> =
            if b.deltas.is_empty() { vec![0.0; regions] } else { b.deltas.iter().map(|v| round_with_tolerance(*v, tolerance)).collect() };
        let rel_deltas: Vec<f64> = deltas.iter().zip(last_deltas.iter()).map(|(v, p)| round_with_tolerance(v - p, tolerance)).collect();
        *last_deltas = deltas;
        if rel_deltas.iter().any(|v| *v != 0.0) {
            args.push(Arg::Blend(rd, rel_deltas));
        } else {
            args.push(Arg::Plain(rd));
        }
        d
    };
    for op in &glyph.path {
        let points: Vec<&BlendPoint> = match op {
            BlendOp::MoveTo(p) | BlendOp::LineTo(p) => vec![p],
            BlendOp::CurveTo(a, b, c) => vec![a, b, c],
        };
        let mut args = Vec::with_capacity(6);
        let mut defaults = Vec::with_capacity(3);
        for (x, y) in points {
            check_blend(x, regions, name)?;
            check_blend(y, regions, name)?;
            let dx = rel(x, &mut last.0, &mut last.1, &mut args);
            let dy = rel(y, &mut last.2, &mut last.3, &mut args);
            defaults.push((dx, dy));
        }
        let (code, step) = match (op, defaults.as_slice()) {
            (BlendOp::MoveTo(_), [p]) => (RMOVETO, PathOp::MoveTo(*p)),
            (BlendOp::LineTo(_), [p]) => (RLINETO, PathOp::LineTo(*p)),
            (BlendOp::CurveTo(..), [a, b, c]) => (RRCURVETO, PathOp::CurveTo(*a, *b, *c)),
            _ => return Err(Cff2Error::BadBlend { glyph: name.to_string(), reason: "a drawing step has the wrong number of points".into() }),
        };
        default_path.push(step);
        commands.push(Cmd { op: code, args });
    }
    let hints = glyph.hints.as_ref().filter(|h| !h.is_empty() && !commands.is_empty());
    let hinted = match hints {
        Some(h) => hinted_charstring(&commands, h, glyph.vsindex, regions, tolerance, generic, name)?,
        None => None,
    };
    let unhinted = hints.is_some() && hinted.is_none();
    let out = if let Some(bytes) = hinted {
        bytes
    } else if generic {
        encode(&commands, glyph.vsindex, regions, name)?
    } else {
        // The specialiser keeps every blend within the stack; were an argument swap to push one
        // past it, the generic form is written instead.
        match encode(&specialize(commands.clone(), MAX_STACK, false), glyph.vsindex, regions, name) {
            Err(Cff2Error::StackOverflow { .. }) => encode(&commands, glyph.vsindex, regions, name)?,
            other => other?,
        }
    };
    // The defaults are rounded already; the bounds pen sees the points the charstring draws.
    let drawn = bounds(&pen_commands(&default_path, 0.0));
    Ok((out, drawn, unhinted))
}

/// Checks `hints` against a path of `ops` operations whose blends have `regions` deltas: the
/// stem and mask limits, finite numbers in range, mask lengths, masks in path order from op 0.
fn check_hints(hints: &Cff2Hints, ops: usize, regions: usize, name: &str) -> Result<(), Cff2Error> {
    let bad = |reason: String| Cff2Error::BadBlend { glyph: name.to_string(), reason };
    let stems = hints.h_stems.len() + hints.v_stems.len();
    if stems > MAX_STEM_HINTS {
        return Err(bad(format!("{stems} stem hints, more than {MAX_STEM_HINTS}")));
    }
    if hints.counters.len() > MAX_COUNTER_MASKS {
        return Err(bad(format!("{} counter masks, more than {MAX_COUNTER_MASKS}", hints.counters.len())));
    }
    for s in hints.h_stems.iter().chain(&hints.v_stems) {
        check_blend(&s.edge, regions, name)?;
        check_blend(&s.width, regions, name)?;
    }
    for m in hints.masks.iter().map(|(_, m)| m).chain(&hints.counters) {
        if m.h.len() != hints.h_stems.len() || m.v.len() != hints.v_stems.len() {
            return Err(bad("a mask does not match the stems".into()));
        }
    }
    if hints.masks.iter().any(|(op, _)| *op >= ops.max(1)) {
        return Err(bad("a hint mask names a path operation the glyph does not have".into()));
    }
    if hints.masks.first().is_some_and(|(op, _)| *op != 0) {
        return Err(bad("the first hint mask must apply from the start of the outline".into()));
    }
    if hints.masks.windows(2).any(|w| matches!(w, [(a, _), (b, _)] if a >= b)) {
        return Err(bad("hint masks are not in path order".into()));
    }
    Ok(())
}

/// A blended number rounded as the outline's coordinates are: its default and its deltas.
fn rounded(b: &Blend, regions: usize, tolerance: f64) -> (f64, Vec<f64>) {
    let deltas = if b.deltas.is_empty() { vec![0.0; regions] } else { b.deltas.iter().map(|v| round_with_tolerance(*v, tolerance)).collect() };
    (round_with_tolerance(b.default, tolerance), deltas)
}

fn blend_arg(default: f64, deltas: Vec<f64>) -> Arg {
    if deltas.iter().any(|d| *d != 0.0) { Arg::Blend(default, deltas) } else { Arg::Plain(default) }
}

/// The arguments of one stem operator for `stems`, relative from 0 as the static encoder writes
/// them: each edge after the previous stem's `edge + width`, then the width.
fn stem_operator_args(stems: &[BlendStem], regions: usize, tolerance: f64) -> Vec<Arg> {
    let mut last = (0.0, vec![0.0; regions]);
    let mut args = Vec::with_capacity(stems.len() * 2);
    for s in stems {
        let (edge, edge_deltas) = rounded(&s.edge, regions, tolerance);
        let (width, width_deltas) = rounded(&s.width, regions, tolerance);
        let rel: Vec<f64> = edge_deltas.iter().zip(&last.1).map(|(e, p)| round_with_tolerance(e - p, tolerance)).collect();
        args.push(blend_arg(edge - last.0, rel));
        args.push(blend_arg(width, width_deltas.clone()));
        last = (edge + width, edge_deltas.iter().zip(&width_deltas).map(|(e, w)| e + w).collect());
    }
    args
}

/// The charstring of `commands` (one per path operation) with `hints`, as the static encoder
/// lays one out: the stem operators (`hstemhm`/`vstemhm` when there are masks), the counter
/// masks, then the outline specialised in runs between hint masks, which stay where they are,
/// as fontTools' specialiser keeps a merged charstring's masks. `None` when a stem's blend
/// cannot fit the operand stack: the glyph is then written without hints.
fn hinted_charstring(
    commands: &[Cmd<Arg>],
    hints: &Cff2Hints,
    vsindex: u16,
    regions: usize,
    tolerance: f64,
    generic: bool,
    name: &str,
) -> Result<Option<Vec<u8>>, Cff2Error> {
    check_hints(hints, commands.len(), regions, name)?;
    let is_blend = |a: &Arg| matches!(a, Arg::Blend(..));
    let replacement = !hints.masks.is_empty() || !hints.counters.is_empty();
    let mut blended = false;
    let mut head = Vec::new();
    for (stems, plain, hm) in [(&hints.h_stems, 1u8, 18u8), (&hints.v_stems, 3, 23)] {
        for chunk in stems.chunks(STEMS_PER_OPERATOR) {
            let args = stem_operator_args(chunk, regions, tolerance);
            blended |= args.iter().any(is_blend);
            match write_args(&args, regions, Context::Charstring, name, &mut head) {
                Err(Cff2Error::StackOverflow { .. }) => return Ok(None),
                other => other?,
            }
            head.push(if replacement { hm } else { plain });
        }
    }
    for m in &hints.counters {
        head.push(OP_CNTRMASK);
        head.extend(m.bytes());
    }
    let mut body = Vec::new();
    let run_out = |run: &[Cmd<Arg>], body: &mut Vec<u8>, blended: &mut bool| -> Result<(), Cff2Error> {
        if run.is_empty() {
            return Ok(());
        }
        *blended |= run.iter().any(|c| c.args.iter().any(is_blend));
        let mut bytes = Vec::new();
        let written = if generic {
            write_commands(run, regions, name, &mut bytes)
        } else {
            write_commands(&specialize(run.to_vec(), MAX_STACK, false), regions, name, &mut bytes)
        };
        match written {
            Err(Cff2Error::StackOverflow { .. }) if !generic => {
                bytes.clear();
                write_commands(run, regions, name, &mut bytes)?;
            }
            other => other?,
        }
        body.extend(bytes);
        Ok(())
    };
    let mut start = 0usize;
    for (op, m) in &hints.masks {
        run_out(commands.get(start..*op).unwrap_or(&[]), &mut body, &mut blended)?;
        body.push(OP_HINTMASK);
        body.extend(m.bytes());
        start = *op;
    }
    run_out(commands.get(start..).unwrap_or(&[]), &mut body, &mut blended)?;
    let mut out = Vec::with_capacity(head.len() + body.len() + 4);
    if blended && vsindex != 0 {
        write_args(&[Arg::Plain(f64::from(vsindex))], 0, Context::Charstring, name, &mut out)?;
        out.push(OP_VSINDEX);
    }
    out.extend(head);
    out.extend(body);
    Ok(Some(out))
}

/// Appends `commands` with their operators.
fn write_commands(commands: &[Cmd<Arg>], regions: usize, name: &str, out: &mut Vec<u8>) -> Result<(), Cff2Error> {
    for c in commands {
        write_args(&c.args, regions, Context::Charstring, name, out)?;
        let code = opcode(c.op)
            .ok_or_else(|| Cff2Error::BadBlend { glyph: name.to_string(), reason: "the specialiser left an operator unresolved".into() })?;
        out.push(code);
    }
    Ok(())
}

/// The bytes of `commands`, with `vsindex` first when it is not 0 and something is blended.
fn encode(commands: &[Cmd<Arg>], vsindex: u16, regions: usize, name: &str) -> Result<Vec<u8>, Cff2Error> {
    let mut out = Vec::new();
    let blended = commands.iter().any(|c| c.args.iter().any(|a| matches!(a, Arg::Blend(..))));
    if blended && vsindex != 0 {
        write_args(&[Arg::Plain(f64::from(vsindex))], 0, Context::Charstring, name, &mut out)?;
        out.push(OP_VSINDEX);
    }
    write_commands(commands, regions, name, &mut out)?;
    Ok(out)
}

/// The most operands `charstring` (with `regions` deltas per blended value) has on the stack at
/// once, counting one more for a subroutine call the subroutiniser may add at the deepest point.
/// Reads only what this writer emits: no subroutine calls yet, stems before the first mask.
fn stack_need(charstring: &[u8], regions: usize) -> usize {
    let (mut depth, mut peak, mut stems) = (0usize, 0usize, 0usize);
    let mut last = 0i64;
    let mut i = 0usize;
    let byte = |at: usize| charstring.get(at).copied().map_or(0, i64::from);
    while let Some(&b0) = charstring.get(i) {
        let number = match b0 {
            28 => Some((3, ((byte(i + 1) << 8) | byte(i + 2)) as i16 as i64)),
            32..=246 => Some((1, i64::from(b0) - 139)),
            247..=250 => Some((2, (i64::from(b0) - 247) * 256 + byte(i + 1) + 108)),
            251..=254 => Some((2, -(i64::from(b0) - 251) * 256 - byte(i + 1) - 108)),
            255 => Some((5, ((byte(i + 1) << 8) | byte(i + 2)) as i16 as i64)),
            _ => None,
        };
        if let Some((len, value)) = number {
            depth += 1;
            peak = peak.max(depth);
            last = value;
            i += len;
            continue;
        }
        i += if b0 == 12 { 2 } else { 1 };
        match b0 {
            OP_BLEND => {
                let n = usize::try_from(last).unwrap_or(0);
                depth = depth.saturating_sub(n.saturating_mul(regions).saturating_add(1));
            }
            OP_VSINDEX => depth = depth.saturating_sub(1),
            1 | 3 | 18 | 23 => {
                stems += depth / 2;
                depth = 0;
            }
            OP_HINTMASK | OP_CNTRMASK => {
                stems += depth / 2;
                depth = 0;
                i += stems.div_ceil(8);
            }
            _ => depth = 0,
        }
    }
    peak.saturating_add(1)
}

/// Appends a CFF2 INDEX (32-bit count) of `items`.
fn write_index2<T: AsRef<[u8]>>(items: &[T], out: &mut Vec<u8>) -> Result<(), Cff2Error> {
    let count = u32::try_from(items.len()).map_err(|_| Error::TooManyGlyphs(items.len()))?;
    out.extend_from_slice(&count.to_be_bytes());
    if items.is_empty() {
        return Ok(());
    }
    let data: usize = items.iter().map(|i| i.as_ref().len()).fold(0usize, usize::saturating_add);
    let last = data.saturating_add(1);
    if last > MAX_TABLE_SIZE {
        return Err(Error::TableTooLarge(last).into());
    }
    let size: usize = if last < 0x100 {
        1
    } else if last < 0x1_0000 {
        2
    } else if last < 0x100_0000 {
        3
    } else {
        4
    };
    out.push(size as u8);
    let mut offset = 1usize;
    let push = |o: usize, out: &mut Vec<u8>| {
        let bytes = (o as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(4 - size..).unwrap_or(&bytes));
    };
    push(offset, out);
    for item in items {
        offset += item.as_ref().len();
        push(offset, out);
    }
    for item in items {
        out.extend_from_slice(item.as_ref());
    }
    Ok(())
}

/// A DICT offset operand in its five-byte form, so offsets can be filled in without moving data.
fn offset_operand(v: usize, out: &mut Vec<u8>) -> Result<(), Cff2Error> {
    let v = i32::try_from(v).map_err(|_| Error::TableTooLarge(v))?;
    out.push(29);
    out.extend_from_slice(&v.to_be_bytes());
    Ok(())
}

/// F2Dot14 of a normalized coordinate.
fn f2dot14(v: f64) -> Result<i16, Cff2Error> {
    if !v.is_finite() || !(-2.0..2.0).contains(&v) {
        return Err(Error::OutOfRange { what: "a variation region".into(), value: v }.into());
    }
    Ok((v * 16384.0).round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16)
}

/// The item variation store: no items, one variation data per region list.
fn variation_store(font: &Cff2Font) -> Result<Vec<u8>, Cff2Error> {
    let axes = usize::from(font.axis_count);
    let lists = font.region_lists.len();
    let mut out = Vec::new();
    out.extend_from_slice(&1u16.to_be_bytes());
    let header = 2 + 4 + 2 + 4 * lists;
    let region_list_len = 4 + font.regions.len() * axes * 6;
    out.extend_from_slice(&(header as u32).to_be_bytes());
    out.extend_from_slice(&(lists as u16).to_be_bytes());
    let mut at = header + region_list_len;
    for list in &font.region_lists {
        out.extend_from_slice(&(at as u32).to_be_bytes());
        at += 6 + 2 * list.len();
    }
    out.extend_from_slice(&font.axis_count.to_be_bytes());
    out.extend_from_slice(&(font.regions.len() as u16).to_be_bytes());
    for region in &font.regions {
        for a in region {
            for v in [a.start, a.peak, a.end] {
                out.extend_from_slice(&f2dot14(v)?.to_be_bytes());
            }
        }
    }
    for list in &font.region_lists {
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(list.len() as u16).to_be_bytes());
        for r in list {
            out.extend_from_slice(&r.to_be_bytes());
        }
    }
    Ok(out)
}

/// The Private DICT, its varying values blended over region list 0's `regions` regions.
fn private_dict(p: &Cff2Private, regions: usize) -> Result<Vec<u8>, Cff2Error> {
    let mut out = Vec::new();
    let what = "the Private DICT";
    let arg = |b: &Blend| -> Result<Arg, Cff2Error> {
        if !b.default.is_finite() || !b.deltas.iter().all(|d| d.is_finite()) {
            return Err(Error::NotFinite { what: what.into() }.into());
        }
        if b.varies() {
            if b.deltas.len() != regions {
                return Err(Cff2Error::BadBlend { glyph: what.into(), reason: "a value has the wrong number of deltas".into() });
            }
            Ok(Arg::Blend(b.default, b.deltas.clone()))
        } else {
            Ok(Arg::Plain(b.default))
        }
    };
    let list = |values: &[Blend], op: &[u8], out: &mut Vec<u8>| -> Result<(), Cff2Error> {
        if values.is_empty() {
            return Ok(());
        }
        if values.len() > MAX_PRIVATE_LIST {
            return Err(Error::TooManyOperands { what: what.into(), count: values.len() }.into());
        }
        if let Some(v) = values.iter().find(|v| !v.deltas.is_empty() && v.deltas.len() != regions) {
            return Err(Cff2Error::BadBlend {
                glyph: what.into(),
                reason: format!("a list value has {} deltas; region list 0 has {regions}", v.deltas.len()),
            });
        }
        // Each value is stored as the difference from the one before it. When any varies, every
        // value goes into one blend: readers take a list's operands as a single blend.
        let blended = values.iter().any(Blend::varies);
        if blended && values.len().saturating_mul(regions + 1).saturating_add(1) > MAX_STACK {
            return Err(Cff2Error::StackOverflow { glyph: what.into() });
        }
        let mut prev = Blend { default: 0.0, deltas: vec![0.0; regions] };
        let mut args = Vec::with_capacity(values.len());
        for v in values {
            let deltas: Vec<f64> = if v.deltas.is_empty() { vec![0.0; regions] } else { v.deltas.clone() };
            let rel = Blend { default: v.default - prev.default, deltas: deltas.iter().zip(&prev.deltas).map(|(a, b)| a - b).collect() };
            if blended {
                if !rel.default.is_finite() || !rel.deltas.iter().all(|d| d.is_finite()) {
                    return Err(Error::NotFinite { what: what.into() }.into());
                }
                args.push(Arg::Blend(rel.default, rel.deltas));
            } else {
                args.push(arg(&rel)?);
            }
            prev = Blend { default: v.default, deltas };
        }
        write_args(&args, regions, Context::Dict, what, out)?;
        out.extend_from_slice(op);
        Ok(())
    };
    list(&p.blue_values, &[6], &mut out)?;
    list(&p.other_blues, &[7], &mut out)?;
    list(&p.family_blues, &[8], &mut out)?;
    list(&p.family_other_blues, &[9], &mut out)?;
    let plain = |v: f64, op: &[u8], out: &mut Vec<u8>| -> Result<(), Cff2Error> {
        write_args(&[Arg::Plain(v)], 0, Context::Dict, what, out)?;
        out.extend_from_slice(op);
        Ok(())
    };
    for (value, default, op) in [(&p.blue_scale, 0.039625, 9u8), (&p.blue_shift, 7.0, 10), (&p.blue_fuzz, 1.0, 11)] {
        if let Some(b) = value.as_ref().filter(|b| b.varies() || b.default != default) {
            write_args(&[arg(b)?], regions, Context::Dict, what, &mut out)?;
            out.extend_from_slice(&[12, op]);
        }
    }
    for (stem, op) in [(&p.std_hw, 10u8), (&p.std_vw, 11u8)] {
        if let Some(b) = stem {
            write_args(&[arg(b)?], regions, Context::Dict, what, &mut out)?;
            out.push(op);
        }
    }
    list(&p.stem_snap_h, &[12, 12], &mut out)?;
    list(&p.stem_snap_v, &[12, 13], &mut out)?;
    if let Some(v) = p.language_group.filter(|v| *v != 0) {
        plain(v as f64, &[12, 17], &mut out)?;
    }
    Ok(out)
}

/// Checks the counts and the regions of `font`.
fn validate(font: &Cff2Font) -> Result<(), Cff2Error> {
    if font.glyphs.is_empty() {
        return Err(Error::NoGlyphs.into());
    }
    if font.glyphs.len() > MAX_GLYPHS {
        return Err(Error::TooManyGlyphs(font.glyphs.len()).into());
    }
    if font.regions.len() > MAX_REGIONS {
        return Err(Cff2Error::TooManyRegions(font.regions.len()));
    }
    if font.region_lists.is_empty() || font.region_lists.len() > MAX_REGION_LISTS {
        return Err(Cff2Error::TooManyRegions(font.region_lists.len()));
    }
    if usize::from(font.axis_count) > MAX_AXES || font.axis_count == 0 {
        return Err(Error::OutOfRange { what: "the axis count".into(), value: f64::from(font.axis_count) }.into());
    }
    if !font.round_tolerance.is_finite() || font.round_tolerance < 0.0 {
        return Err(Error::BadRoundTolerance(font.round_tolerance).into());
    }
    if let Some(r) = font.regions.iter().find(|r| r.len() != usize::from(font.axis_count)) {
        return Err(Cff2Error::BadBlend { glyph: String::new(), reason: format!("a region has {} axes; the font has {}", r.len(), font.axis_count) });
    }
    for list in &font.region_lists {
        if list.len() > MAX_BLEND_REGIONS {
            return Err(Cff2Error::TooManyBlendRegions(list.len()));
        }
        if list.iter().any(|&r| usize::from(r) >= font.regions.len()) {
            return Err(Cff2Error::BadBlend { glyph: String::new(), reason: "a region list names a region that does not exist".into() });
        }
    }
    Ok(())
}

/// The `CFF2` table for `font`, with each glyph's bounds at the origin.
pub fn write_cff2(font: &Cff2Font) -> Result<Cff2Built, Cff2Error> {
    validate(font)?;
    let mut charstrings = Vec::with_capacity(font.glyphs.len());
    let mut glyph_bounds = Vec::with_capacity(font.glyphs.len());
    let mut unhinted = Vec::new();
    let mut stack = 0usize;
    for g in &font.glyphs {
        let Some(list) = font.region_lists.get(usize::from(g.vsindex)) else {
            return Err(Cff2Error::BadBlend { glyph: g.name.clone(), reason: format!("region list {} does not exist", g.vsindex) });
        };
        let (bytes, b, dropped) = charstring(g, list.len(), font.round_tolerance, font.generic)?;
        stack = stack.max(stack_need(&bytes, list.len()));
        charstrings.push(bytes);
        glyph_bounds.push(b);
        if dropped {
            unhinted.push(g.name.clone());
        }
    }
    let base_regions = font.region_lists.first().map_or(0, Vec::len);
    let private = private_dict(&font.private, base_regions)?;
    let vstore = variation_store(font)?;

    // Layout: header, Top DICT, Global Subr INDEX (empty), vstore, CharStrings, FDArray, Private.
    // Offsets are five-byte operands, so the Top DICT's length is known before they are.
    let mut matrix = Vec::new();
    if let Some(m) = font.font_matrix {
        let args: Vec<Arg> = m.iter().map(|v| Arg::Plain(*v)).collect();
        if args.iter().any(|a| matches!(a, Arg::Plain(v) if !v.is_finite())) {
            return Err(Error::NotFinite { what: "the FontMatrix".into() }.into());
        }
        write_args(&args, 0, Context::Dict, "the FontMatrix", &mut matrix)?;
        matrix.extend_from_slice(&[12, 7]);
    }
    let mut maxstack = Vec::new();
    if stack > SPEC_DEFAULT_STACK {
        encode_dict_int(MAX_STACK as i64, "maxstack", &mut maxstack)?;
        maxstack.push(DICT_MAXSTACK);
    }
    let top_len = matrix.len() + 3 * 5 + 1 + 2 + 1 + maxstack.len();
    let header_len = 5usize;
    let global_subrs_len = 4usize;
    let vstore_at = header_len + top_len + global_subrs_len;
    let mut charstrings_index = Vec::new();
    write_index2(&charstrings, &mut charstrings_index)?;
    let charstrings_at = vstore_at + 2 + vstore.len();
    let fdarray_at = charstrings_at + charstrings_index.len();
    let mut font_dict = Vec::new();
    let private_at = fdarray_at + 4 + 1 + 2 + 10 + 1;
    offset_operand(private.len(), &mut font_dict)?;
    offset_operand(private_at, &mut font_dict)?;
    font_dict.push(18);
    let mut fdarray = Vec::new();
    write_index2(&[font_dict], &mut fdarray)?;

    let mut top = matrix;
    offset_operand(charstrings_at, &mut top)?;
    top.push(17);
    offset_operand(fdarray_at, &mut top)?;
    top.extend_from_slice(&[12, 36]);
    offset_operand(vstore_at, &mut top)?;
    top.push(24);
    top.extend_from_slice(&maxstack);

    let mut out = Vec::with_capacity(private_at + private.len());
    out.extend_from_slice(&[2, 0, header_len as u8]);
    let top_len16 = u16::try_from(top.len()).map_err(|_| Error::TableTooLarge(top.len()))?;
    out.extend_from_slice(&top_len16.to_be_bytes());
    out.extend_from_slice(&top);
    out.extend_from_slice(&0u32.to_be_bytes());
    let vstore_len = u16::try_from(vstore.len()).map_err(|_| Cff2Error::TooManyRegions(font.regions.len()))?;
    out.extend_from_slice(&vstore_len.to_be_bytes());
    out.extend_from_slice(&vstore);
    out.extend_from_slice(&charstrings_index);
    out.extend_from_slice(&fdarray);
    if out.len() != private_at {
        return Err(Error::LayoutDidNotSettle.into());
    }
    out.extend_from_slice(&private);
    if out.len() > MAX_TABLE_SIZE {
        return Err(Error::TableTooLarge(out.len()).into());
    }
    Ok(Cff2Built { table: out, glyph_bounds, unhinted })
}

#[cfg(test)]
#[path = "cff2_tests.rs"]
mod cff2_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn a_varying_private_list_is_one_blend_of_every_value() {
        let p = Cff2Private {
            blue_values: vec![Blend::fixed(-15.0), Blend::fixed(0.0), Blend { default: 500.0, deltas: vec![20.0] }, Blend::fixed(515.0)],
            ..Cff2Private::default()
        };
        let bytes = private_dict(&p, 1).unwrap();
        // Differences -15 15 500 15, their deltas 0 0 20 -20, 4 blend, BlueValues.
        let want = vec![124, 154, 248, 136, 154, 139, 139, 159, 119, 143, DICT_BLEND, 6];
        assert_eq!(bytes, want);
        // A list that cannot be one blend within the stack is refused.
        let many = Cff2Private { blue_values: vec![Blend { default: 1.0, deltas: vec![1.0; 40] }; 14], ..Cff2Private::default() };
        assert!(matches!(private_dict(&many, 40), Err(Cff2Error::StackOverflow { .. })));
        // A value with more deltas than the region list has regions is refused, not cut short.
        let extra =
            Cff2Private { blue_values: vec![Blend { default: 500.0, deltas: vec![20.0, 5.0] }, Blend::fixed(515.0)], ..Cff2Private::default() };
        assert!(matches!(private_dict(&extra, 1), Err(Cff2Error::BadBlend { .. })));
    }

    #[test]
    fn deltas_that_do_not_match_the_region_list_are_refused() {
        let glyph = Cff2Glyph {
            name: "a".into(),
            vsindex: 0,
            path: vec![BlendOp::MoveTo((Blend { default: 0.0, deltas: vec![1.0, 2.0] }, Blend::fixed(0.0)))],
            hints: None,
        };
        let font = Cff2Font {
            axis_count: 1,
            regions: vec![vec![RegionAxis { start: 0.0, peak: 1.0, end: 1.0 }]],
            region_lists: vec![vec![0]],
            glyphs: vec![Cff2Glyph { name: ".notdef".into(), ..Cff2Glyph::default() }, glyph],
            round_tolerance: 0.01,
            ..Cff2Font::default()
        };
        assert!(matches!(write_cff2(&font), Err(Cff2Error::BadBlend { .. })));
        let bad_list = Cff2Font { region_lists: vec![vec![3]], ..font };
        assert!(matches!(write_cff2(&bad_list), Err(Cff2Error::BadBlend { .. })));
    }
}
