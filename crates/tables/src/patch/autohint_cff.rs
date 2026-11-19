//! Autohinting the charstrings of a compiled font's `CFF ` table (row `autohint.ps.runFont`):
//! each glyph's charstring is drawn into an outline, the caller's hinter adds stems, masks and
//! flex, and the charstring is written again with them. Every other table keeps its bytes.
//!
//! | Step | Rule |
//! |---|---|
//! | input | the `CFF ` table with its subroutine calls expanded; a `CFF2` table is refused |
//! | zones and stems | read from each glyph's Private DICT ([`cff_hint_setup`]); the caller turns them into the hinter's form |
//! | glyphs left as they are | not selected; already hinted (stem operators or masks) unless `hint_all`; accented with seac; drawing nothing; the hinter finding no stems or failing |
//! | width | the width operand is kept as it was |
//! | outline | coordinates written as found, every segment kept: the charstring draws exactly what it drew |
//! | stems | rounded to whole units unless the caller keeps fractions (`round_stems`) |
//! | check | each new charstring is read back: its outline and width must equal the old ones, else the glyph keeps its old charstring |
//! | subroutines | the table is subroutinised again, then every glyph's expanded charstring is compared with the one written |
//!
//! The hinter is passed in, so this layer knows nothing of the autohinter's options; the engine's
//! `autohint.ps.runFont` supplies it.

use fontcraft_cff::subr::{self, CharstringSet, SubrError, SubrOptions, TableRewrite};
use fontcraft_cff::{CharstringHints, MAX_SEGMENTS, Number, PathOp, PrivateDict, encode_charstring};
use read_fonts::ps::cff::blend::BlendState;
use read_fonts::ps::cff::dict::{Entry, entries};
use read_fonts::ps::cff::index::Index;
use read_fonts::types::{Fixed, GlyphId};
use read_fonts::{FontData, FontRead};

use crate::limits::{MAX_CFF_STACK, MAX_CHARSTRING, MAX_CHARSTRING_MEMORY};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

const CFF: [u8; 4] = *b"CFF ";
const CFF2: [u8; 4] = *b"CFF2";
const HEAD: [u8; 4] = *b"head";
/// Most entries read from one DICT.
pub(super) const MAX_DICT_ENTRIES: usize = 4_096;
/// Most Font DICTs read from an FDArray (the FDSelect holds a byte per Font DICT).
pub(super) const MAX_FONT_DICTS: usize = 256;

/// What the hinter needs to know about the font before it runs.
#[derive(Clone, Debug)]
pub struct CffHintSetup {
    /// One per Font DICT (one for a name-keyed font), in FDArray order.
    pub privates: Vec<PrivateDict>,
    /// Each glyph's name: the charset's, or `cidNNNNN` in a CID-keyed font.
    pub names: Vec<String>,
    pub cid: bool,
    /// The lowest and highest y of the font: the FontBBox, else −0.25 and 1.25 em.
    pub y_range: (f64, f64),
}

/// One glyph handed to the hinter.
#[derive(Clone, Copy, Debug)]
pub struct CffGlyph<'a> {
    pub gid: usize,
    pub name: &'a str,
    /// Its Font DICT, an index into [`CffHintSetup::privates`].
    pub fd: usize,
    /// Its outline as the charstring draws it.
    pub path: &'a [PathOp],
}

/// How [`autohint_cff`] runs.
#[derive(Clone, Copy, Debug, Default)]
pub struct CffAutohintOptions {
    /// Also hint glyphs whose charstring has hints, replacing them.
    pub hint_all: bool,
    /// Round stem edges to whole units.
    pub round_stems: bool,
}

/// A glyph that got hints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CffHinted {
    pub glyph: String,
    pub horizontal: usize,
    pub vertical: usize,
}

/// A selected glyph left as it was, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CffSkipped {
    pub glyph: String,
    pub reason: String,
}

/// What [`autohint_cff`] did.
#[derive(Clone, Debug)]
pub struct CffAutohint {
    /// The font with its `CFF ` table written again; the input font when nothing was hinted.
    pub font: SfntFont,
    pub hinted: Vec<CffHinted>,
    pub skipped: Vec<CffSkipped>,
    /// Size of the `CFF ` table before and after.
    pub bytes_before: usize,
    pub bytes_after: usize,
}

pub(super) fn from_subr(e: SubrError) -> TablesError {
    match e {
        SubrError::Invalid(m) => TablesError::Invalid(format!("the CFF table: {m}")),
        SubrError::Unsupported(m) => TablesError::Unsupported(m),
        SubrError::TooLarge { what, limit } => TablesError::TooLarge { what, limit },
        other => TablesError::Invalid(other.to_string()),
    }
}

fn cff_data(font: &SfntFont) -> Result<&[u8]> {
    match font.get(CFF) {
        Some(data) => Ok(data),
        None if font.get(CFF2).is_some() => Err(TablesError::Unsupported("the font has a CFF2 table, which autohint_cff2 hints".into())),
        None => Err(TablesError::MissingTable("CFF ".into())),
    }
}

// ── The Private DICTs and the glyph names ───────────────────────────────────────────────────────

fn number(f: Fixed) -> Number {
    let v = f.to_f64();
    if v.fract() == 0.0 { Number::Int(v as i64) } else { Number::Real(v) }
}

/// The hinting values of one Private DICT, its blends (in a `CFF2` table) evaluated by `blend`.
/// Entries that cannot be read end the reading; what was read before them stays.
pub(super) fn read_private<'a>(bytes: &'a [u8], blend: Option<BlendState<'a>>) -> PrivateDict {
    let pairs = |values: &[(Fixed, Fixed)]| -> Vec<Number> { values.iter().flat_map(|(a, b)| [number(*a), number(*b)]).collect() };
    let mut d = PrivateDict::default();
    for entry in entries(bytes, blend).take(MAX_DICT_ENTRIES) {
        let Ok(entry) = entry else { break };
        match entry {
            Entry::BlueValues(b) => d.blue_values = pairs(b.values()),
            Entry::OtherBlues(b) => d.other_blues = pairs(b.values()),
            Entry::FamilyBlues(b) => d.family_blues = pairs(b.values()),
            Entry::FamilyOtherBlues(b) => d.family_other_blues = pairs(b.values()),
            Entry::BlueScale(f) => d.blue_scale = number(f),
            Entry::BlueShift(f) => d.blue_shift = number(f),
            Entry::BlueFuzz(f) => d.blue_fuzz = number(f),
            Entry::StdHw(f) => d.std_hw = Some(number(f)),
            Entry::StdVw(f) => d.std_vw = Some(number(f)),
            Entry::StemSnapH(s) => d.stem_snap_h = s.values().iter().map(|v| number(*v)).collect(),
            Entry::StemSnapV(s) => d.stem_snap_v = s.values().iter().map(|v| number(*v)).collect(),
            Entry::LanguageGroup(n) => d.language_group = Number::Int(i64::from(n)),
            _ => {}
        }
    }
    d
}

/// The Private DICTs, glyph names and vertical extent of the font's `CFF ` table.
pub fn cff_hint_setup(font: &SfntFont) -> Result<CffHintSetup> {
    let data = cff_data(font)?;
    let cff = read_fonts::tables::cff::Cff::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the CFF table: {e}")))?;
    let top = cff.top_dicts().get(0).ok_or_else(|| TablesError::invalid("the CFF table has no Top DICT"))?;
    let (mut private, mut fd_array, mut bbox, mut cid) = (None, None, None, false);
    for entry in entries(top, None).take(MAX_DICT_ENTRIES) {
        match entry {
            Ok(Entry::PrivateDictRange(range)) => private = Some(range),
            Ok(Entry::FdArrayOffset(offset)) => fd_array = Some(offset),
            Ok(Entry::FontBbox(b)) => bbox = Some(b),
            Ok(Entry::Ros { .. }) => cid = true,
            Ok(_) => {}
            // What was read before an entry that cannot be read stays, as a renderer reads it.
            Err(_) => break,
        }
    }
    let read = |range: std::ops::Range<usize>| -> Result<PrivateDict> {
        data.get(range).map(|d| read_private(d, None)).ok_or_else(|| TablesError::invalid("a Private DICT lies outside the CFF table"))
    };
    let privates = match fd_array {
        Some(offset) => {
            let index = data
                .get(offset..)
                .and_then(|d| Index::new(d, false).ok())
                .ok_or_else(|| TablesError::invalid("the CFF table's FDArray cannot be read"))?;
            let count = index.count() as usize;
            if count == 0 || count > MAX_FONT_DICTS {
                return Err(TablesError::invalid(format!("the FDArray holds {count} Font DICTs")));
            }
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let dict = index.get(i).ok_or_else(|| TablesError::invalid(format!("Font DICT {i} cannot be read")))?;
                let range = entries(dict, None).take(MAX_DICT_ENTRIES).flatten().find_map(|e| match e {
                    Entry::PrivateDictRange(r) => Some(r),
                    _ => None,
                });
                out.push(match range {
                    Some(r) => read(r)?,
                    None => PrivateDict::default(),
                });
            }
            out
        }
        None => vec![match private {
            Some(r) => read(r)?,
            None => PrivateDict::default(),
        }],
    };

    let glyphs = cff_glyph_count(data)?;
    let charset = cff.charset(0);
    let names = (0..glyphs)
        .map(|g| {
            let sid = u32::try_from(g).ok().and_then(|g| charset.as_ref()?.string_id(GlyphId::new(g)));
            match sid {
                Some(sid) if cid => format!("cid{:05}", sid.to_u16()),
                Some(sid) => cff.string(sid).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_else(|| format!("glyph{g:05}")),
                None if g == 0 => ".notdef".to_string(),
                None => format!("glyph{g:05}"),
            }
        })
        .collect();

    let upm =
        font.get(HEAD).and_then(|h| h.get(18..20)).and_then(|b| <[u8; 2]>::try_from(b).ok()).map_or(1000.0, |b| f64::from(u16::from_be_bytes(b)));
    let upm = if upm > 0.0 { upm } else { 1000.0 };
    let y_range = match bbox {
        Some([_, y_min, _, y_max]) => (y_min.to_f64(), y_max.to_f64()),
        None => (-0.25 * upm, 1.25 * upm),
    };
    Ok(CffHintSetup { privates, names, cid, y_range })
}

fn cff_glyph_count(data: &[u8]) -> Result<usize> {
    subr::charstrings_of_table(data, false).map(|s| s.charstrings.len()).map_err(from_subr)
}

// ── Autohinting ─────────────────────────────────────────────────────────────────────────────────

/// The font with the selected glyphs of its `CFF ` table hinted by `hint`. `select` says which
/// glyphs (by name) to look at; `hint` returns the hints for one glyph's outline, `None` when it
/// found no stems, or why it failed.
pub fn autohint_cff(
    font: &SfntFont,
    setup: &CffHintSetup,
    options: &CffAutohintOptions,
    select: &dyn Fn(&str) -> bool,
    hint: &mut dyn FnMut(&CffGlyph<'_>) -> std::result::Result<Option<CharstringHints>, String>,
) -> Result<CffAutohint> {
    let data = cff_data(font)?;
    let set = subr::charstrings_of_table(data, false).map_err(from_subr)?;
    let mut flat = subr::desubroutinize_within(&set, MAX_CHARSTRING_MEMORY).map_err(from_subr)?;
    let (mut hinted, mut skipped) = (Vec::new(), Vec::new());
    let mut replaced: Vec<(usize, Vec<u8>)> = Vec::new();
    for (gid, code) in flat.charstrings.iter().enumerate() {
        let name = setup.names.get(gid).map_or("", String::as_str);
        if !select(name) {
            continue;
        }
        let skip = |skipped: &mut Vec<CffSkipped>, reason: &str| skipped.push(CffSkipped { glyph: name.to_string(), reason: reason.to_string() });
        let glyph = match decode(code) {
            Ok(g) => g,
            Err(e) => {
                skip(&mut skipped, &format!("its charstring cannot be read: {e}"));
                continue;
            }
        };
        if glyph.seac {
            skip(&mut skipped, "it is an accented glyph built with seac; decompose it first (tables.recompileCff)");
            continue;
        }
        if glyph.path.is_empty() {
            skip(&mut skipped, "the glyph draws nothing");
            continue;
        }
        if glyph.hinted && !options.hint_all {
            skip(&mut skipped, "the glyph already has hints");
            continue;
        }
        let input = CffGlyph { gid, name, fd: flat.fd_of(gid), path: &glyph.path };
        let mut hints = match hint(&input) {
            Ok(Some(h)) if !h.is_empty() => h,
            Ok(_) => {
                skip(&mut skipped, "the autohinter found no stems");
                continue;
            }
            Err(e) => {
                skip(&mut skipped, &e);
                continue;
            }
        };
        if options.round_stems {
            round_stems(&mut hints);
        }
        let bytes = match encode_charstring(name, &glyph.path, Some(&hints), glyph.width) {
            Ok(b) => b,
            Err(e) => {
                skip(&mut skipped, &format!("its hints cannot be written: {e}"));
                continue;
            }
        };
        if bytes.len() > MAX_CHARSTRING {
            skip(&mut skipped, "the hinted charstring is longer than a charstring may be");
            continue;
        }
        match decode(&bytes) {
            Ok(back) if back.path == glyph.path && back.width == glyph.width && back.hinted => {}
            _ => {
                skip(&mut skipped, "the hinted charstring would draw differently");
                continue;
            }
        }
        hinted.push(CffHinted { glyph: name.to_string(), horizontal: hints.h_stems.len(), vertical: hints.v_stems.len() });
        replaced.push((gid, bytes));
    }

    let bytes_before = data.len();
    if replaced.is_empty() {
        return Ok(CffAutohint { font: font.clone(), hinted, skipped, bytes_before, bytes_after: bytes_before });
    }
    for (gid, bytes) in replaced {
        if let Some(slot) = flat.charstrings.get_mut(gid) {
            *slot = bytes;
        }
    }
    let table = write_table(data, &flat)?;
    let bytes_after = table.len();
    let mut out = font.clone();
    out.insert(CFF, table);
    Ok(CffAutohint { font: out, hinted, skipped, bytes_before, bytes_after })
}

/// The table (`CFF ` or, when `flat` says so, `CFF2`) with `flat`'s charstrings, subroutinised,
/// checked against `flat`.
pub(super) fn write_table(data: &[u8], flat: &CharstringSet) -> Result<Vec<u8>> {
    let keep = TableRewrite { top: &[], private: &[], dedupe_family_blues: false };
    let (table, _) =
        subr::rewrite_table(data, flat.cff2, &keep, &mut |_: &CharstringSet| subr::subroutinize(flat, &SubrOptions::default())).map_err(from_subr)?;
    let back =
        subr::charstrings_of_table(&table, flat.cff2).and_then(|s| subr::desubroutinize_within(&s, MAX_CHARSTRING_MEMORY)).map_err(from_subr)?;
    let expected = &flat.charstrings;
    if let Some(gid) = (0..expected.len().max(back.charstrings.len())).find(|&g| back.charstrings.get(g) != expected.get(g)) {
        return Err(TablesError::invalid(format!(
            "glyph {gid} would draw differently after the CFF table is written again; the font was left as it was"
        )));
    }
    Ok(table)
}

/// Stem edges rounded to whole units, as a charstring without fractions stores them; a ghost keeps
/// its fixed width.
pub(super) fn round_stems(hints: &mut CharstringHints) {
    let round = |v: f64| (v + 0.5).floor();
    for s in hints.h_stems.iter_mut().chain(hints.v_stems.iter_mut()) {
        if s.is_ghost() {
            s.edge = round(s.edge);
        } else {
            let (a, b) = s.edges();
            let (a, b) = (round(a), round(b));
            s.edge = a;
            s.width = b - a;
        }
    }
}

// ── Reading a charstring ────────────────────────────────────────────────────────────────────────

/// A flat Type 2 charstring read as an outline.
#[derive(Debug, Default, PartialEq)]
struct Decoded {
    /// The width operand, relative to `nominalWidthX`.
    width: Option<f64>,
    /// Absolute coordinates. A moveto with nothing drawn after it is left out: it draws nothing,
    /// and the encoder joins successive movetos.
    path: Vec<PathOp>,
    /// It has stem operators or masks.
    hinted: bool,
    /// It ends with the four accent operands of seac.
    seac: bool,
}

// Charstring operators (`12 n` as 1200 + n).
const HSTEM: u16 = 1;
const VSTEM: u16 = 3;
const VMOVETO: u16 = 4;
const RLINETO: u16 = 5;
const HLINETO: u16 = 6;
const VLINETO: u16 = 7;
const RRCURVETO: u16 = 8;
const ENDCHAR: u16 = 14;
const HSTEMHM: u16 = 18;
const HINTMASK: u16 = 19;
const CNTRMASK: u16 = 20;
const RMOVETO: u16 = 21;
const HMOVETO: u16 = 22;
const VSTEMHM: u16 = 23;
const RCURVELINE: u16 = 24;
const RLINECURVE: u16 = 25;
const VVCURVETO: u16 = 26;
const HHCURVETO: u16 = 27;
const VHCURVETO: u16 = 30;
const HVCURVETO: u16 = 31;
const DOTSECTION: u16 = 1200;
const HFLEX: u16 = 1234;
const FLEX: u16 = 1235;
const HFLEX1: u16 = 1236;
const FLEX1: u16 = 1237;

/// The pen a charstring draws with.
struct Pen {
    x: f64,
    y: f64,
    path: Vec<PathOp>,
}

impl Pen {
    fn check(&self) -> std::result::Result<(), String> {
        if self.path.len() >= MAX_SEGMENTS { Err(format!("more than {MAX_SEGMENTS} path segments")) } else { Ok(()) }
    }

    fn move_by(&mut self, dx: f64, dy: f64) -> std::result::Result<(), String> {
        self.check()?;
        self.x += dx;
        self.y += dy;
        let p = (self.x, self.y);
        match self.path.last_mut() {
            Some(PathOp::MoveTo(last)) => *last = p,
            _ => self.path.push(PathOp::MoveTo(p)),
        }
        Ok(())
    }

    fn line_by(&mut self, dx: f64, dy: f64) -> std::result::Result<(), String> {
        self.check()?;
        if self.path.is_empty() {
            return Err("it draws before its first moveto".into());
        }
        self.x += dx;
        self.y += dy;
        self.path.push(PathOp::LineTo((self.x, self.y)));
        Ok(())
    }

    fn curve_by(&mut self, d: [f64; 6]) -> std::result::Result<(), String> {
        self.check()?;
        if self.path.is_empty() {
            return Err("it draws before its first moveto".into());
        }
        let [dxa, dya, dxb, dyb, dxc, dyc] = d;
        let a = (self.x + dxa, self.y + dya);
        let b = (a.0 + dxb, a.1 + dyb);
        let c = (b.0 + dxc, b.1 + dyc);
        (self.x, self.y) = c;
        self.path.push(PathOp::CurveTo(a, b, c));
        Ok(())
    }
}

fn arg(args: &[f64], i: usize) -> f64 {
    args.get(i).copied().unwrap_or(0.0)
}

/// Reads a charstring without subroutine calls.
fn decode(code: &[u8]) -> std::result::Result<Decoded, String> {
    let cut = || "the charstring is cut short".to_string();
    let byte = |i: usize| code.get(i).copied().ok_or_else(cut);
    let mut out = Decoded::default();
    let mut pen = Pen { x: 0.0, y: 0.0, path: Vec::new() };
    let mut stack: Vec<f64> = Vec::new();
    let mut stems = 0usize;
    let mut width_seen = false;
    let mut i = 0usize;
    while let Some(&b0) = code.get(i) {
        let value = match b0 {
            28 => {
                i += 3;
                Some(f64::from(i16::from_be_bytes([byte(i - 2)?, byte(i - 1)?])))
            }
            32..=246 => {
                i += 1;
                Some(f64::from(b0) - 139.0)
            }
            247..=250 => {
                i += 2;
                Some((f64::from(b0) - 247.0) * 256.0 + f64::from(byte(i - 1)?) + 108.0)
            }
            251..=254 => {
                i += 2;
                Some(-(f64::from(b0) - 251.0) * 256.0 - f64::from(byte(i - 1)?) - 108.0)
            }
            255 => {
                i += 5;
                Some(f64::from(i32::from_be_bytes([byte(i - 4)?, byte(i - 3)?, byte(i - 2)?, byte(i - 1)?])) / 65536.0)
            }
            _ => None,
        };
        if let Some(v) = value {
            if stack.len() >= MAX_CFF_STACK {
                return Err("more operands than the stack holds".into());
            }
            stack.push(v);
            continue;
        }
        let op = if b0 == 12 {
            i += 2;
            1200 + u16::from(byte(i - 1)?)
        } else {
            i += 1;
            u16::from(b0)
        };
        let mut args = std::mem::take(&mut stack);
        // The width is an extra first operand of the first operator that clears the stack.
        let clears = matches!(op, HSTEM | VSTEM | HSTEMHM | VSTEMHM | HINTMASK | CNTRMASK | RMOVETO | HMOVETO | VMOVETO | ENDCHAR);
        if clears && !width_seen {
            width_seen = true;
            let n = args.len();
            let has_width = match op {
                RMOVETO => n > 2,
                HMOVETO | VMOVETO => n > 1,
                ENDCHAR => n == 1 || n == 5,
                _ => n % 2 == 1,
            };
            if has_width && !args.is_empty() {
                out.width = Some(args.remove(0));
            }
        }
        let n = args.len();
        match op {
            HSTEM | VSTEM | HSTEMHM | VSTEMHM => {
                stems = stems.saturating_add(n / 2);
                out.hinted = true;
            }
            HINTMASK | CNTRMASK => {
                stems = stems.saturating_add(n / 2);
                out.hinted = true;
                i = i.checked_add(stems.div_ceil(8)).ok_or_else(cut)?;
                if i > code.len() {
                    return Err(cut());
                }
            }
            RMOVETO => pen.move_by(arg(&args, 0), arg(&args, 1))?,
            HMOVETO => pen.move_by(arg(&args, 0), 0.0)?,
            VMOVETO => pen.move_by(0.0, arg(&args, 0))?,
            RLINETO => {
                for p in args.chunks_exact(2) {
                    pen.line_by(arg(p, 0), arg(p, 1))?;
                }
            }
            HLINETO | VLINETO => {
                let mut horizontal = op == HLINETO;
                for &d in &args {
                    if horizontal {
                        pen.line_by(d, 0.0)?
                    } else {
                        pen.line_by(0.0, d)?
                    }
                    horizontal = !horizontal;
                }
            }
            RRCURVETO => {
                for c in args.chunks_exact(6) {
                    pen.curve_by([arg(c, 0), arg(c, 1), arg(c, 2), arg(c, 3), arg(c, 4), arg(c, 5)])?;
                }
            }
            RCURVELINE => {
                let curves = n.saturating_sub(2) / 6;
                for c in args.chunks_exact(6).take(curves) {
                    pen.curve_by([arg(c, 0), arg(c, 1), arg(c, 2), arg(c, 3), arg(c, 4), arg(c, 5)])?;
                }
                let rest = args.get(curves * 6..).unwrap_or_default();
                pen.line_by(arg(rest, 0), arg(rest, 1))?;
            }
            RLINECURVE => {
                let lines = n.saturating_sub(6) / 2;
                for p in args.chunks_exact(2).take(lines) {
                    pen.line_by(arg(p, 0), arg(p, 1))?;
                }
                let c = args.get(lines * 2..).unwrap_or_default();
                pen.curve_by([arg(c, 0), arg(c, 1), arg(c, 2), arg(c, 3), arg(c, 4), arg(c, 5)])?;
            }
            VVCURVETO | HHCURVETO => {
                let (first, rest) = if n % 2 == 1 { (arg(&args, 0), args.get(1..).unwrap_or_default()) } else { (0.0, args.as_slice()) };
                let mut off = first;
                for c in rest.chunks_exact(4) {
                    if op == VVCURVETO {
                        pen.curve_by([off, arg(c, 0), arg(c, 1), arg(c, 2), 0.0, arg(c, 3)])?;
                    } else {
                        pen.curve_by([arg(c, 0), off, arg(c, 1), arg(c, 2), arg(c, 3), 0.0])?;
                    }
                    off = 0.0;
                }
            }
            VHCURVETO | HVCURVETO => {
                let mut horizontal = op == HVCURVETO;
                let mut k = 0usize;
                while k + 4 <= n {
                    // A fifth operand on the last curve is its final off-direction delta.
                    let last = if n - k == 5 { arg(&args, k + 4) } else { 0.0 };
                    let (a, b, c, d) = (arg(&args, k), arg(&args, k + 1), arg(&args, k + 2), arg(&args, k + 3));
                    if horizontal {
                        pen.curve_by([a, 0.0, b, c, last, d])?;
                    } else {
                        pen.curve_by([0.0, a, b, c, d, last])?;
                    }
                    horizontal = !horizontal;
                    k += 4;
                }
            }
            FLEX => {
                let a = |k: usize| arg(&args, k);
                pen.curve_by([a(0), a(1), a(2), a(3), a(4), a(5)])?;
                pen.curve_by([a(6), a(7), a(8), a(9), a(10), a(11)])?;
            }
            HFLEX => {
                let a = |k: usize| arg(&args, k);
                pen.curve_by([a(0), 0.0, a(1), a(2), a(3), 0.0])?;
                pen.curve_by([a(4), 0.0, a(5), -a(2), a(6), 0.0])?;
            }
            HFLEX1 => {
                let a = |k: usize| arg(&args, k);
                pen.curve_by([a(0), a(1), a(2), a(3), a(4), 0.0])?;
                pen.curve_by([a(5), 0.0, a(6), a(7), a(8), -(a(1) + a(3) + a(7))])?;
            }
            FLEX1 => {
                let a = |k: usize| arg(&args, k);
                let dx = a(0) + a(2) + a(4) + a(6) + a(8);
                let dy = a(1) + a(3) + a(5) + a(7) + a(9);
                let (dx6, dy6) = if dx.abs() > dy.abs() { (a(10), -dy) } else { (-dx, a(10)) };
                pen.curve_by([a(0), a(1), a(2), a(3), a(4), a(5)])?;
                pen.curve_by([a(6), a(7), a(8), a(9), dx6, dy6])?;
            }
            DOTSECTION => {}
            ENDCHAR => {
                out.seac = n == 4;
                break;
            }
            other => {
                let name = if other >= 1200 { format!("12 {}", other - 1200) } else { other.to_string() };
                return Err(format!("operator {name} is not handled here"));
            }
        }
    }
    if matches!(pen.path.last(), Some(PathOp::MoveTo(_))) {
        pen.path.pop();
    }
    out.path = pen.path;
    Ok(out)
}

#[cfg(test)]
#[path = "autohint_cff_tests.rs"]
mod tests;
