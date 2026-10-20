//! A matrix on `CFF ` and `CFF2` outlines, following AFDKO rotatefont (`c/rotatefont/rotateFont.cpp`
//! in AFDKO 5.0.1, Apache License 2.0, Copyright 2014-2018 Adobe; see NOTICE). Written anew here.
//!
//! - Every absolute point is transformed and rounded halves up, in single precision as rotatefont
//!   computes it. A `CFF2` delta is transformed by the linear part only, as the absolute delta
//!   of its point, and rounded on its own; rotatefont reads the default master only.
//! - Hints follow the linear part ([`Hints`]). Kept stems are transformed as absolute edges and
//!   sorted again; hint and counter masks follow their stems. Dropped stems leave the masks.
//! - Flex stays flex unless every hint goes; then it becomes two curves.
//! - In each Private DICT a transformed glyph uses, the blue zones, `StdHW`, `StdVW` and the stem
//!   snaps change as rotatefont changes them; `defaultWidthX` and `nominalWidthX` are chosen
//!   again for the new widths. Subroutines go: the table is written without them.
//! - The two character codes of a `seac`-style `endchar` stay and the accent offset takes the
//!   linear part, which needs every glyph to take one matrix.

use super::cff::{DEFAULT_MATRIX, in_glyph, invalid, numbers, set_entry, subr_error};
use super::glyf::ot_round;
use crate::dehint::cff::{Entry, Table, encode_dict};
use crate::instance::cff2::{font_bbox, glyph_bounds, put_cs_number};
use crate::limits::{CharstringMemory, MAX_CFF_STACK, MAX_CFF2_STACK, MAX_CHARSTRING, MAX_CHARSTRING_MEMORY, MAX_SFNT};
use crate::{Result, TablesError};

const OP_FONT_BBOX: u16 = 5;
const OP_SUBRS: u16 = 19;
const OP_DEFAULT_WIDTH: u16 = 20;
const OP_NOMINAL_WIDTH: u16 = 21;
const OP_UNDERLINE_POSITION: u16 = 1203;
const OP_UNDERLINE_THICKNESS: u16 = 1204;
const OP_FONT_MATRIX: u16 = 1207;
const BLUE_VALUES: u16 = 6;
const OTHER_BLUES: u16 = 7;
const FAMILY_BLUES: u16 = 8;
const FAMILY_OTHER_BLUES: u16 = 9;
const STD_HW: u16 = 10;
const STD_VW: u16 = 11;
const STEM_SNAP_H: u16 = 1212;
const STEM_SNAP_V: u16 = 1213;
/// The alignment zones rotatefont writes when a turn leaves none that make sense.
const FIXED_BLUES: [f64; 4] = [-250.0, -250.0, 1100.0, 1100.0];
const MAX_COORDINATE: f64 = 32_767.0;
/// Most operands one written operator takes in a `CFF ` charstring, a width left aside.
const MAX_CFF_ARGS: usize = MAX_CFF_STACK - 1;

/// What a matrix's linear part `[a, b, c, d]` does to hints, by rotatefont's cases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hints {
    /// `b` and `c` are 0, `a` and `d` positive: every stem kept in place.
    Upright,
    /// `b` and `c` are 0, `a` and `d` negative: every stem kept, its edges swapped.
    HalfTurn,
    /// `a` and `d` are 0, `b` negative, `c` positive (a quarter turn clockwise): horizontal stems
    /// become vertical and the reverse.
    Clockwise,
    /// `a` and `d` are 0, `b` positive, `c` negative.
    Counterclockwise,
    /// `b` is 0 otherwise (a horizontal shear or a mirror): horizontal stems kept, vertical
    /// stems dropped.
    Horizontal,
    /// Any other matrix: every stem dropped.
    Dropped,
}

impl Hints {
    pub(super) fn of(m: [f64; 6]) -> Hints {
        let [a, b, c, d, ..] = m.map(|v| v as f32);
        if (a == 0.0 && d == 0.0) || (b == 0.0 && c == 0.0) {
            if a < 0.0 && d < 0.0 {
                return Hints::HalfTurn;
            }
            if a > 0.0 && d > 0.0 {
                return Hints::Upright;
            }
            if b < 0.0 && c > 0.0 {
                return Hints::Clockwise;
            }
            if b > 0.0 && c < 0.0 {
                return Hints::Counterclockwise;
            }
        }
        if b == 0.0 { Hints::Horizontal } else { Hints::Dropped }
    }
}

/// What [`apply_matrix`] needs besides the table.
pub(super) struct MatrixJob<'a> {
    /// The matrix the options give; the Private DICTs follow it.
    pub matrix: [f64; 6],
    /// Each glyph's own matrix (the same linear part, its own shift), `None` to leave it.
    pub glyphs: &'a [Option<[f64; 6]>],
    /// Each glyph's new advance width, `None` to keep the one its charstring gives.
    pub widths: &'a [Option<f64>],
    /// The factor units per em change by when the em scales with the matrix.
    pub em_factor: Option<f64>,
}

/// A transformed table and what the rest of the font needs from it.
pub(super) struct Matrixed {
    pub table: Vec<u8>,
    /// Each glyph's exact bounds; `None` for a glyph that draws nothing.
    pub bounds: Vec<Option<[f64; 4]>>,
    pub glyphs_changed: usize,
    pub hints_kept: bool,
    pub warnings: Vec<String>,
}

pub(super) fn apply_matrix(data: &[u8], cff2: bool, job: &MatrixJob<'_>) -> Result<Matrixed> {
    apply_matrix_within(data, cff2, job, MAX_SFNT.min(MAX_CHARSTRING_MEMORY))
}

/// [`apply_matrix`] with the memory the expanded and the written charstrings may take together,
/// and one glyph's decoded form on its own.
pub(super) fn apply_matrix_within(data: &[u8], cff2: bool, job: &MatrixJob<'_>, max_bytes: usize) -> Result<Matrixed> {
    if job.matrix.iter().chain(job.glyphs.iter().flatten().flatten()).any(|v| !v.is_finite()) {
        return Err(TablesError::BadOptions("the matrix values must be finite".into()));
    }
    let flat = fontcraft_cff::subr::desubroutinize_table_within(data, cff2, max_bytes).map_err(subr_error)?;
    let mut memory = CharstringMemory::new(max_bytes);
    memory.charge(flat.len())?;
    let mut table = Table::read(&flat, cff2)?;
    let count = table.charstrings.len();
    if job.glyphs.len() != count || job.widths.len() != count {
        return Err(invalid(format!("the CFF table holds {count} glyphs, the font {}", job.glyphs.len())));
    }
    let case = Hints::of(job.matrix);
    let fd_of = |gid: usize| table.fd_select.as_ref().and_then(|s| s.get(gid).copied()).map_or(0, usize::from);
    let width_value = |p: usize, op: u16| -> Result<f64> {
        let entry = table.privates.get(p).and_then(|p| p.entries.iter().find(|e| e.op == op));
        Ok(match entry {
            Some(e) => numbers(&e.raw)?.and_then(|v| v.first().copied()).unwrap_or(0.0),
            None => 0.0,
        })
    };

    let shared = job.glyphs.iter().all(|g| *g == Some(job.matrix));
    let mut transformed_privates = vec![false; table.privates.len()];
    // Each glyph is written without its width first; the width defaults are chosen once every
    // width is known, and the width goes in front.
    let mut bodies = Vec::with_capacity(count);
    let mut widths = Vec::with_capacity(count);
    let mut changed = 0usize;
    let mut stems_dropped = false;
    for (gid, code) in table.charstrings.iter().enumerate() {
        let fd = fd_of(gid);
        let private = table.privates.get(fd).ok_or_else(|| invalid(format!("glyph {gid} names Font DICT {fd}, which does not exist")))?;
        let nominal = if cff2 { 0.0 } else { width_value(fd, OP_NOMINAL_WIDTH)? };
        let default = if cff2 { 0.0 } else { width_value(fd, OP_DEFAULT_WIDTH)? };
        let mut glyph_memory = CharstringMemory::new(max_bytes);
        let mut parser = Parser::new(cff2, &table.regions, private.vsindex, nominal, &mut glyph_memory);
        parser.run(code).map_err(|e| in_glyph(e, gid))?;
        let mut glyph = parser.out;
        let mut width = glyph.width.unwrap_or(default);
        if let Some(Some(m)) = job.glyphs.get(gid) {
            if glyph.seac.is_some() && !shared {
                return Err(TablesError::Unsupported(format!(
                    "glyph {gid} builds an accented character from two others (seac), which needs every glyph to take the same matrix"
                )));
            }
            stems_dropped |= glyph.stems.iter().any(|s| case == Hints::Dropped || (case == Hints::Horizontal && s.vertical));
            glyph = transform_glyph(&glyph, *m, case).map_err(|e| in_glyph(e, gid))?;
            if let Some(t) = transformed_privates.get_mut(fd) {
                *t = true;
            }
            changed += 1;
        }
        if let Some(Some(w)) = job.widths.get(gid) {
            width = *w;
        }
        let vsindex = glyph.vsindex.unwrap_or(private.vsindex);
        let regions = if cff2 { table.regions.get(vsindex).copied().unwrap_or(0) } else { 0 };
        let body = encode(&glyph, cff2, regions).map_err(|e| in_glyph(e, gid))?;
        memory.charge(body.len().saturating_add(std::mem::size_of::<Vec<u8>>()))?;
        bodies.push(body);
        widths.push(width);
    }

    let mut width_defaults = vec![(0.0, 0.0); table.privates.len()];
    if !cff2 {
        for (p, slot) in width_defaults.iter_mut().enumerate() {
            let all: Vec<i64> = widths.iter().enumerate().filter(|(g, _)| fd_of(*g) == p).map(|(_, w)| w.round() as i64).collect();
            if let Some((d, n)) = fontcraft_cff::optimize_widths(&all) {
                *slot = (d as f64, n as f64);
            }
        }
    }
    let mut charstrings = Vec::with_capacity(count);
    for (gid, (body, width)) in bodies.into_iter().zip(&widths).enumerate() {
        let (default, nominal) = width_defaults.get(fd_of(gid)).copied().unwrap_or((0.0, 0.0));
        let out = if !cff2 && *width != default {
            let mut out = Vec::with_capacity(body.len() + 5);
            put_cs_number(&mut out, width - nominal).map_err(|e| in_glyph(e, gid))?;
            out.extend(body);
            out
        } else {
            body
        };
        if out.len() > MAX_CHARSTRING {
            return Err(TablesError::TooLarge { what: "a transformed charstring", limit: MAX_CHARSTRING });
        }
        charstrings.push(out);
    }

    let mut warnings = Vec::new();
    let mut varied = 0usize;
    let mut privates = Vec::with_capacity(table.privates.len());
    let m32 = job.matrix.map(|v| v as f32);
    for (p, private) in table.privates.iter().enumerate() {
        let mut entries: Vec<Entry> = private.entries.iter().filter(|e| e.op != OP_SUBRS).cloned().collect();
        if transformed_privates.get(p).copied().unwrap_or(false) {
            varied += transform_private(&mut entries, case, m32)?;
        }
        if !cff2 {
            let (default, nominal) = width_defaults.get(p).copied().unwrap_or((0.0, 0.0));
            set_entry(&mut entries, OP_DEFAULT_WIDTH, (default != 0.0).then_some(vec![default]))?;
            set_entry(&mut entries, OP_NOMINAL_WIDTH, (nominal != 0.0).then_some(vec![nominal]))?;
        }
        privates.push(encode_dict(entries.iter(), &[])?);
    }
    if varied > 0 {
        warnings.push(format!("{varied} Private DICTs hold values that vary across the design space and are left as they were"));
    }
    if transformed_privates.iter().any(|t| *t) && matches!(case, Hints::Clockwise | Hints::Counterclockwise | Hints::Dropped) {
        warnings.push("the alignment zones are set to -250 -250 1100 1100: set them again for the turned design".into());
    }
    if stems_dropped {
        warnings.push(match case {
            Hints::Horizontal => "the matrix shears or mirrors horizontally: vertical stem hints are dropped".into(),
            _ => "the matrix turns the outlines by an angle that is not a quarter turn: every stem hint is dropped".into(),
        });
    }

    if let Some(factor) = job.em_factor {
        scale_em(&mut table, cff2, factor)?;
    }
    let first = table.write(&charstrings, &privates)?;
    let bounds = glyph_bounds(&first)?;
    let out = if cff2 {
        first
    } else {
        let bbox = font_bbox(&bounds);
        let values = (bbox != [0; 4]).then(|| bbox.iter().map(|v| *v as f64).collect());
        set_entry(&mut table.top, OP_FONT_BBOX, values)?;
        table.write(&charstrings, &privates)?
    };
    Ok(Matrixed { table: out, bounds, glyphs_changed: changed, hints_kept: !stems_dropped, warnings })
}

/// With the em scaled by `factor` (rotatefont `-fm`): an explicit FontMatrix is divided by it, and
/// in a `CFF ` Top DICT the underline position and thickness are multiplied by it.
fn scale_em(table: &mut Table<'_>, cff2: bool, factor: f64) -> Result<()> {
    if !factor.is_finite() || factor <= 0.0 {
        return Err(TablesError::BadOptions(format!("{factor} is not a scale factor")));
    }
    let divide = |entries: &mut Vec<Entry>| -> Result<()> {
        if let Some(e) = entries.iter().find(|e| e.op == OP_FONT_MATRIX)
            && let Some(m) = numbers(&e.raw)?
            && m.len() == 6
        {
            let m: Vec<f64> = m.iter().map(|v| v / factor).collect();
            let default = m.iter().zip(DEFAULT_MATRIX).all(|(v, d)| (v - d).abs() <= 1e-12);
            set_entry(entries, OP_FONT_MATRIX, (!default).then_some(m))?;
        }
        Ok(())
    };
    divide(&mut table.top)?;
    if let Some(fds) = table.font_dicts.as_mut() {
        for fd in fds.iter_mut() {
            divide(fd)?;
        }
    }
    if !cff2 {
        for (op, default) in [(OP_UNDERLINE_POSITION, -100.0), (OP_UNDERLINE_THICKNESS, 50.0)] {
            let value = match table.top.iter().find(|e| e.op == op) {
                Some(e) => match numbers(&e.raw)?.as_deref() {
                    Some([v]) => *v,
                    _ => continue,
                },
                None => default,
            };
            let scaled = ot_round(value * factor);
            set_entry(&mut table.top, op, (scaled != default).then_some(vec![scaled]))?;
        }
    }
    Ok(())
}

// ---------- values ----------

/// A charstring value: the default, then one delta per region; missing deltas are 0.
type V = Vec<f64>;
/// An absolute point.
type Pt = [V; 2];

fn at(v: &[f64], i: usize) -> f64 {
    v.get(i).copied().unwrap_or(0.0)
}

fn add(a: &[f64], b: &[f64]) -> V {
    (0..a.len().max(b.len())).map(|i| at(a, i) + at(b, i)).collect()
}

fn sub(a: &[f64], b: &[f64]) -> V {
    (0..a.len().max(b.len())).map(|i| at(a, i) - at(b, i)).collect()
}

fn neg(a: &[f64]) -> V {
    a.iter().map(|v| -v).collect()
}

fn is_zero(v: &[f64]) -> bool {
    v.iter().all(|x| *x == 0.0)
}

fn varies(v: &[f64]) -> bool {
    v.iter().skip(1).any(|x| *x != 0.0)
}

/// Rounds halves up a value computed in single precision, as rotatefont's `RND` does.
fn rnd(v: f32) -> f64 {
    (f64::from(v) + 0.5).floor()
}

// ---------- reading ----------

#[derive(Clone)]
struct Stem {
    vertical: bool,
    edges: [V; 2],
}

#[derive(Clone)]
enum Item {
    Move(Pt),
    Line(Pt),
    Curve([Pt; 3]),
    Flex([Pt; 6], V),
    /// The operator (19 hint mask, 20 counter mask) and its mask bytes.
    Mask(u8, Vec<u8>),
}

#[derive(Clone, Default)]
struct Glyph {
    /// The advance width the charstring gives (`CFF ` only).
    width: Option<f64>,
    vsindex: Option<usize>,
    stems: Vec<Stem>,
    items: Vec<Item>,
    /// The accent offset and the two character codes of a `seac`-style `endchar`.
    seac: Option<[f64; 4]>,
}

struct Parser<'t, 'm> {
    cff2: bool,
    regions: &'t [usize],
    vsindex: usize,
    nominal: f64,
    max_stack: usize,
    stack: Vec<V>,
    point: Pt,
    /// No stack-clearing operator yet: the next one may carry the width.
    first: bool,
    memory: &'m mut CharstringMemory,
    out: Glyph,
}

impl<'t, 'm> Parser<'t, 'm> {
    fn new(cff2: bool, regions: &'t [usize], vsindex: usize, nominal: f64, memory: &'m mut CharstringMemory) -> Self {
        let max_stack = if cff2 { MAX_CFF2_STACK } else { MAX_CFF_STACK };
        Parser {
            cff2,
            regions,
            vsindex,
            nominal,
            max_stack,
            stack: Vec::new(),
            point: [vec![0.0], vec![0.0]],
            first: true,
            memory,
            out: Glyph::default(),
        }
    }

    fn run(&mut self, code: &[u8]) -> Result<()> {
        let cut = || invalid("the charstring is cut short");
        let byte = |k: usize| code.get(k).copied().ok_or_else(cut);
        let mut i = 0usize;
        while let Some(&b0) = code.get(i) {
            let number = match b0 {
                28 => {
                    i += 3;
                    Some(f64::from(i16::from_be_bytes([byte(i - 2)?, byte(i - 1)?])))
                }
                32..=246 => {
                    i += 1;
                    Some(f64::from(i32::from(b0) - 139))
                }
                247..=250 => {
                    i += 2;
                    Some(f64::from((i32::from(b0) - 247) * 256 + i32::from(byte(i - 1)?) + 108))
                }
                251..=254 => {
                    i += 2;
                    Some(f64::from(-(i32::from(b0) - 251) * 256 - i32::from(byte(i - 1)?) - 108))
                }
                255 => {
                    i += 5;
                    let bits = i32::from_be_bytes([byte(i - 4)?, byte(i - 3)?, byte(i - 2)?, byte(i - 1)?]);
                    Some(f64::from(bits) / 65536.0)
                }
                _ => None,
            };
            if let Some(v) = number {
                if self.stack.len() >= self.max_stack {
                    return Err(invalid(format!("more than {} operands are on the stack", self.max_stack)));
                }
                self.stack.push(vec![v]);
                continue;
            }
            i += 1;
            let n = self.stack.len();
            let zero = vec![0.0];
            match b0 {
                1 | 18 | 3 | 23 => {
                    self.take_width(n % 2 == 1);
                    self.stems(b0 == 3 || b0 == 23)?;
                }
                19 | 20 => {
                    self.take_width(n % 2 == 1);
                    if !self.stack.is_empty() {
                        self.stems(true)?;
                    }
                    let len = self.out.stems.len().div_ceil(8);
                    let mask = code.get(i..i.saturating_add(len)).ok_or_else(cut)?;
                    self.push(Item::Mask(b0, mask.to_vec()))?;
                    i += len;
                }
                21 => {
                    self.take_width(n > 2);
                    let [dx, dy] = self.args::<2>()?;
                    self.move_by(&dx, &dy)?;
                }
                22 => {
                    self.take_width(n > 1);
                    let [d] = self.args::<1>()?;
                    self.move_by(&d, &zero)?;
                }
                4 => {
                    self.take_width(n > 1);
                    let [d] = self.args::<1>()?;
                    self.move_by(&zero, &d)?;
                }
                5 => {
                    let args = self.args_in_groups(2)?;
                    for [dx, dy] in args.as_chunks::<2>().0 {
                        self.line_by(dx, dy)?;
                    }
                }
                6 | 7 => {
                    let args = self.args_in_groups(1)?;
                    for (k, d) in args.iter().enumerate() {
                        if (k % 2 == 0) == (b0 == 6) {
                            self.line_by(d, &zero)?;
                        } else {
                            self.line_by(&zero, d)?;
                        }
                    }
                }
                8 => {
                    let args = self.args_in_groups(6)?;
                    for c in args.as_chunks::<6>().0 {
                        self.curve_by(c)?;
                    }
                }
                24 => {
                    let args = self.take_all();
                    if args.len() < 8 || !(args.len() - 2).is_multiple_of(6) {
                        return Err(invalid("rcurveline takes 6n + 2 operands"));
                    }
                    let (curves, line) = args.split_at(args.len() - 2);
                    for c in curves.as_chunks::<6>().0 {
                        self.curve_by(c)?;
                    }
                    if let [dx, dy] = line {
                        self.line_by(dx, dy)?;
                    }
                }
                25 => {
                    let args = self.take_all();
                    if args.len() < 8 || !(args.len() - 6).is_multiple_of(2) {
                        return Err(invalid("rlinecurve takes 2n + 6 operands"));
                    }
                    let (lines, curve) = args.split_at(args.len() - 6);
                    for [dx, dy] in lines.as_chunks::<2>().0 {
                        self.line_by(dx, dy)?;
                    }
                    self.curve_by(curve)?;
                }
                26 | 27 => {
                    let args = self.take_all();
                    let (first, rest) = args.split_at(args.len() % 4);
                    if rest.is_empty() || first.len() > 1 {
                        return Err(invalid("vvcurveto and hhcurveto take 4n or 4n + 1 operands"));
                    }
                    let mut lead = first.first().cloned().unwrap_or_else(|| zero.clone());
                    for [p, q, r, s] in rest.as_chunks::<4>().0 {
                        let c6 = if b0 == 26 {
                            [lead.clone(), p.clone(), q.clone(), r.clone(), zero.clone(), s.clone()]
                        } else {
                            [p.clone(), lead.clone(), q.clone(), r.clone(), s.clone(), zero.clone()]
                        };
                        self.curve_by(&c6)?;
                        lead = zero.clone();
                    }
                }
                30 | 31 => {
                    let args = self.take_all();
                    if args.len() < 4 || args.len() % 4 > 1 {
                        return Err(invalid("hvcurveto and vhcurveto take 4n or 4n + 1 operands"));
                    }
                    let groups = args.len() / 4;
                    let mut horizontal = b0 == 31;
                    for g in 0..groups {
                        let a = |k: usize| args.get(g * 4 + k).cloned().unwrap_or_else(|| vec![0.0]);
                        let last = if g + 1 == groups { args.get(groups * 4).cloned() } else { None };
                        let extra = last.unwrap_or_else(|| zero.clone());
                        let c6 =
                            if horizontal { [a(0), zero.clone(), a(1), a(2), extra, a(3)] } else { [zero.clone(), a(0), a(1), a(2), a(3), extra] };
                        self.curve_by(&c6)?;
                        horizontal = !horizontal;
                    }
                }
                12 => {
                    let b1 = byte(i)?;
                    i += 1;
                    match b1 {
                        34..=37 => self.flex(b1)?,
                        _ => return Err(TablesError::Unsupported(format!("the charstring operator 12 {b1}"))),
                    }
                }
                14 if !self.cff2 => {
                    self.take_width(n == 1 || n == 5);
                    let args = self.take_all();
                    match args.as_slice() {
                        [] => {}
                        [p, q, r, s] => self.out.seac = Some([at(p, 0), at(q, 0), at(r, 0), at(s, 0)]),
                        _ => return Err(invalid("endchar takes 0 or 4 operands besides the width")),
                    }
                    return Ok(());
                }
                15 if self.cff2 => {
                    let [v] = self.args::<1>()?;
                    let v = at(&v, 0);
                    if v.fract() != 0.0 || v < 0.0 || v >= self.regions.len() as f64 {
                        return Err(invalid(format!("vsindex {v} names no item variation data")));
                    }
                    self.vsindex = v as usize;
                    self.out.vsindex = Some(self.vsindex);
                }
                16 if self.cff2 => self.blend()?,
                10 | 11 | 29 => return Err(invalid("a subroutine call is left after expansion")),
                _ => return Err(TablesError::Unsupported(format!("the charstring operator {b0}"))),
            }
        }
        if !self.stack.is_empty() {
            return Err(invalid("operands are left without an operator"));
        }
        Ok(())
    }

    fn charge(&mut self, values: usize) -> Result<()> {
        self.memory.charge(values.saturating_mul(8).saturating_add(64))
    }

    fn push(&mut self, item: Item) -> Result<()> {
        let size = match &item {
            Item::Move(p) | Item::Line(p) => p[0].len() + p[1].len(),
            Item::Curve(c) => c.iter().map(|p| p[0].len() + p[1].len()).sum(),
            Item::Flex(f, d) => f.iter().map(|p| p[0].len() + p[1].len()).sum::<usize>() + d.len(),
            Item::Mask(_, m) => m.len(),
        };
        self.charge(size)?;
        self.out.items.push(item);
        Ok(())
    }

    fn take_all(&mut self) -> Vec<V> {
        std::mem::take(&mut self.stack)
    }

    fn take_width(&mut self, extra: bool) {
        if self.cff2 || !self.first {
            return;
        }
        self.first = false;
        if extra && !self.stack.is_empty() {
            let w = self.stack.remove(0);
            self.out.width = Some(self.nominal + at(&w, 0));
        }
    }

    fn args<const N: usize>(&mut self) -> Result<[V; N]> {
        let args = self.take_all();
        args.try_into().map_err(|a: Vec<V>| invalid(format!("an operator takes {N} operands, not {}", a.len())))
    }

    fn args_in_groups(&mut self, group: usize) -> Result<Vec<V>> {
        let args = self.take_all();
        if args.is_empty() || !args.len().is_multiple_of(group) {
            return Err(invalid(format!("an operator takes a multiple of {group} operands, not {}", args.len())));
        }
        Ok(args)
    }

    fn stems(&mut self, vertical: bool) -> Result<()> {
        let args = self.take_all();
        if !args.len().is_multiple_of(2) {
            return Err(invalid("stem hints take pairs of operands"));
        }
        let mut edge = vec![0.0];
        for [a, b] in args.as_chunks::<2>().0 {
            let e0 = add(&edge, a);
            let e1 = add(&e0, b);
            self.charge(e0.len() + e1.len())?;
            self.out.stems.push(Stem { vertical, edges: [e0, e1.clone()] });
            edge = e1;
        }
        Ok(())
    }

    fn moved(&self, dx: &[f64], dy: &[f64]) -> Pt {
        [add(&self.point[0], dx), add(&self.point[1], dy)]
    }

    fn move_by(&mut self, dx: &[f64], dy: &[f64]) -> Result<()> {
        self.point = self.moved(dx, dy);
        self.push(Item::Move(self.point.clone()))
    }

    fn line_by(&mut self, dx: &[f64], dy: &[f64]) -> Result<()> {
        self.point = self.moved(dx, dy);
        self.push(Item::Line(self.point.clone()))
    }

    fn curve_by(&mut self, d: &[V]) -> Result<()> {
        let [dxa, dya, dxb, dyb, dxc, dyc] = d else { return Err(invalid("a curve takes six operands")) };
        let p1 = self.moved(dxa, dya);
        let p2 = [add(&p1[0], dxb), add(&p1[1], dyb)];
        let p3 = [add(&p2[0], dxc), add(&p2[1], dyc)];
        self.point = p3.clone();
        self.push(Item::Curve([p1, p2, p3]))
    }

    /// `hflex` (34), `flex` (35), `hflex1` (36) and `flex1` (37), as six points and a depth.
    fn flex(&mut self, op: u8) -> Result<()> {
        let a = self.take_all();
        let need = match op {
            34 => 7,
            35 => 13,
            36 => 9,
            _ => 11,
        };
        if a.len() != need {
            return Err(invalid(format!("a flex operator takes {need} operands, not {}", a.len())));
        }
        let g = |k: usize| a.get(k).cloned().unwrap_or_else(|| vec![0.0]);
        let zero = vec![0.0];
        let steps: Vec<[V; 2]> = match op {
            34 => vec![[g(0), zero.clone()], [g(1), g(2)], [g(3), zero.clone()], [g(4), zero.clone()], [g(5), neg(&g(2))], [g(6), zero]],
            35 => (0..6).map(|k| [g(2 * k), g(2 * k + 1)]).collect(),
            36 => {
                let back = neg(&add(&add(&g(1), &g(3)), &g(7)));
                vec![[g(0), g(1)], [g(2), g(3)], [g(4), zero.clone()], [g(5), zero], [g(6), g(7)], [g(8), back]]
            }
            _ => {
                let mut dx = vec![0.0];
                let mut dy = vec![0.0];
                for k in 0..5 {
                    dx = add(&dx, &g(2 * k));
                    dy = add(&dy, &g(2 * k + 1));
                }
                let mut d: Vec<[V; 2]> = (0..5).map(|k| [g(2 * k), g(2 * k + 1)]).collect();
                d.push(if at(&dx, 0).abs() > at(&dy, 0).abs() { [g(10), neg(&dy)] } else { [neg(&dx), g(10)] });
                d
            }
        };
        let depth = if op == 35 { g(12) } else { vec![50.0] };
        let mut points: Vec<Pt> = Vec::with_capacity(6);
        for [dx, dy] in &steps {
            self.point = self.moved(dx, dy);
            points.push(self.point.clone());
        }
        let points: [Pt; 6] = points.try_into().map_err(|_| invalid("a flex has six points"))?;
        self.push(Item::Flex(points, depth))
    }

    fn blend(&mut self) -> Result<()> {
        let n = self.stack.last().map(|v| at(v, 0)).ok_or_else(|| invalid("blend has no operand"))?;
        if n.fract() != 0.0 || n < 0.0 || n >= self.max_stack as f64 {
            return Err(invalid(format!("blend of {n} values")));
        }
        let n = n as usize;
        let k = *self.regions.get(self.vsindex).ok_or_else(|| invalid("blend without item variation data"))?;
        let need = k.checked_add(1).and_then(|r| r.checked_mul(n)).and_then(|v| v.checked_add(1));
        let Some(need) = need.filter(|need| *need <= self.stack.len()) else {
            return Err(invalid(format!("blend of {n} values needs more operands than the stack holds")));
        };
        let base = self.stack.len() - need;
        let operands: Vec<V> = self.stack.drain(base..).collect();
        if operands.iter().any(|v| varies(v)) {
            return Err(invalid("a blend of values that already vary"));
        }
        for j in 0..n {
            let mut v = Vec::with_capacity(k + 1);
            v.push(operands.get(j).map_or(0.0, |o| at(o, 0)));
            for r in 0..k {
                v.push(operands.get(n + j * k + r).map_or(0.0, |o| at(o, 0)));
            }
            self.stack.push(v);
        }
        Ok(())
    }
}

// ---------- transforming ----------

fn transform_point(p: &Pt, m: [f64; 6]) -> Result<Pt> {
    let [a, b, c, d, e, f] = m.map(|v| v as f32);
    let (x, y) = (&p[0], &p[1]);
    let (x0, y0) = (at(x, 0) as f32, at(y, 0) as f32);
    let mut nx = vec![rnd(a * x0 + c * y0 + e)];
    let mut ny = vec![rnd(b * x0 + d * y0 + f)];
    for i in 1..x.len().max(y.len()) {
        nx.push(ot_round(m[0] * at(x, i) + m[2] * at(y, i)));
        ny.push(ot_round(m[1] * at(x, i) + m[3] * at(y, i)));
    }
    if at(&nx, 0).abs() > MAX_COORDINATE || at(&ny, 0).abs() > MAX_COORDINATE {
        return Err(invalid(format!("a point would be at ({}, {}), outside ±32,767", at(&nx, 0), at(&ny, 0))));
    }
    Ok([nx, ny])
}

/// An edge `offset + RND(scale · edge)`, its deltas scaled and rounded on their own.
fn edge(v: &[f64], offset: f32, scale: f32) -> V {
    let mut out = vec![f64::from(offset) + rnd(scale * at(v, 0) as f32)];
    out.extend(v.iter().skip(1).map(|d| ot_round(f64::from(scale) * d)));
    out
}

fn transform_glyph(g: &Glyph, m: [f64; 6], case: Hints) -> Result<Glyph> {
    let [a, b, c, d, e, f] = m.map(|v| v as f32);
    let mut stems = Vec::with_capacity(g.stems.len());
    let mut kept = Vec::with_capacity(g.stems.len());
    for s in &g.stems {
        // (vertical afterwards, offset, scale, edges swapped)
        let how = match (case, s.vertical) {
            (Hints::Upright, true) => Some((true, e, a, false)),
            (Hints::Upright, false) => Some((false, f, d, false)),
            (Hints::HalfTurn, true) => Some((true, e, a, true)),
            (Hints::HalfTurn, false) => Some((false, f, d, true)),
            (Hints::Clockwise, true) => Some((false, f, b, true)),
            (Hints::Clockwise, false) => Some((true, e, c, false)),
            (Hints::Counterclockwise, true) => Some((false, f, b, false)),
            (Hints::Counterclockwise, false) => Some((true, e, c, true)),
            (Hints::Horizontal, false) => Some((false, f, d, d < 0.0)),
            (Hints::Horizontal, true) | (Hints::Dropped, _) => None,
        };
        match how {
            Some((vertical, offset, scale, swap)) => {
                let [e0, e1] = &s.edges;
                let (x0, x1) = if swap { (e1, e0) } else { (e0, e1) };
                stems.push(Stem { vertical, edges: [edge(x0, offset, scale), edge(x1, offset, scale)] });
                kept.push(true);
            }
            None => kept.push(false),
        }
    }
    let mut items = Vec::with_capacity(g.items.len());
    for item in &g.items {
        let t = |p: &Pt| transform_point(p, m);
        match item {
            Item::Move(p) => items.push(Item::Move(t(p)?)),
            Item::Line(p) => items.push(Item::Line(t(p)?)),
            Item::Curve([p1, p2, p3]) => items.push(Item::Curve([t(p1)?, t(p2)?, t(p3)?])),
            Item::Flex([p1, p2, p3, p4, p5, p6], depth) => {
                let (q1, q2, q3, q4, q5, q6) = (t(p1)?, t(p2)?, t(p3)?, t(p4)?, t(p5)?, t(p6)?);
                if case == Hints::Dropped {
                    items.push(Item::Curve([q1, q2, q3]));
                    items.push(Item::Curve([q4, q5, q6]));
                } else {
                    items.push(Item::Flex([q1, q2, q3, q4, q5, q6], depth.clone()));
                }
            }
            Item::Mask(op, bytes) => {
                if stems.is_empty() {
                    continue;
                }
                let mut out = vec![0u8; stems.len().div_ceil(8)];
                let mut j = 0usize;
                for (i, keep) in kept.iter().enumerate() {
                    if !keep {
                        continue;
                    }
                    let set = bytes.get(i / 8).is_some_and(|byte| byte & (0x80 >> (i % 8)) != 0);
                    if set && let Some(o) = out.get_mut(j / 8) {
                        *o |= 0x80 >> (j % 8);
                    }
                    j += 1;
                }
                items.push(Item::Mask(*op, out));
            }
        }
    }
    let seac = g.seac.map(|[adx, ady, bchar, achar]| {
        let (x, y) = (adx as f32, ady as f32);
        [rnd(a * x + c * y), rnd(b * x + d * y), bchar, achar]
    });
    Ok(Glyph { width: g.width, vsindex: g.vsindex, stems, items, seac })
}

/// Changes one Private DICT as rotatefont changes it for `case`; 1 when a value varies across the
/// design space and the DICT is left as it was, else 0.
fn transform_private(entries: &mut Vec<Entry>, case: Hints, m: [f32; 6]) -> Result<usize> {
    let [a, b, c, d, _, f] = m;
    let mut values = Vec::new();
    for (op, delta) in [
        (BLUE_VALUES, true),
        (OTHER_BLUES, true),
        (FAMILY_BLUES, true),
        (FAMILY_OTHER_BLUES, true),
        (STD_HW, false),
        (STD_VW, false),
        (STEM_SNAP_H, true),
        (STEM_SNAP_V, true),
    ] {
        let Some(e) = entries.iter().find(|e| e.op == op) else {
            values.push(None);
            continue;
        };
        let Some(v) = numbers(&e.raw)? else { return Ok(1) };
        let mut total = 0.0;
        values.push(Some(if delta {
            v.iter()
                .map(|x| {
                    total += x;
                    total
                })
                .collect()
        } else {
            v
        }));
    }
    let [blues, other, family, family_other, hw, vw, snap_h, snap_v]: [Option<Vec<f64>>; 8] =
        values.try_into().map_err(|_| invalid("the Private DICT values"))?;
    let write_delta = |entries: &mut Vec<Entry>, op: u16, values: Option<Vec<f64>>| -> Result<()> {
        let values = values.filter(|v| !v.is_empty()).map(|v| {
            let mut previous = 0.0;
            v.iter()
                .map(|x| {
                    let out = x - previous;
                    previous = *x;
                    out
                })
                .collect()
        });
        set_entry(entries, op, values)
    };
    let plain = |entries: &mut Vec<Entry>, op: u16, v: Option<f64>| set_entry(entries, op, v.map(|v| vec![v]));
    let one = |v: &Option<Vec<f64>>| v.as_ref().and_then(|v| v.first().copied());
    let map = |v: &Option<Vec<f64>>, g: &dyn Fn(f64) -> f64| v.as_ref().map(|v| v.iter().map(|x| g(*x)).collect::<Vec<f64>>());
    let reversed = |v: Option<Vec<f64>>| {
        v.map(|mut v| {
            v.reverse();
            v
        })
    };
    let ty = |v: f64| rnd(d * v as f32 + f);
    let sx = |v: f64| rnd(a * v as f32);
    let sy = |v: f64| rnd(d * v as f32);
    let s1 = |v: f64| rnd(b * v as f32);
    let s2 = |v: f64| rnd(c * v as f32);
    match case {
        Hints::Upright | Hints::Horizontal => {
            write_delta(entries, BLUE_VALUES, map(&blues, &ty))?;
            write_delta(entries, OTHER_BLUES, map(&other, &ty))?;
            write_delta(entries, FAMILY_BLUES, map(&family, &ty))?;
            write_delta(entries, FAMILY_OTHER_BLUES, map(&family_other, &ty))?;
            plain(entries, STD_HW, one(&hw).map(sy))?;
            plain(entries, STD_VW, one(&vw).map(sx))?;
            write_delta(entries, STEM_SNAP_H, map(&snap_h, &sy))?;
            write_delta(entries, STEM_SNAP_V, map(&snap_v, &sx))?;
        }
        Hints::HalfTurn => {
            write_delta(entries, BLUE_VALUES, reversed(map(&blues, &ty)))?;
            write_delta(entries, OTHER_BLUES, reversed(map(&other, &ty)))?;
            write_delta(entries, FAMILY_BLUES, reversed(map(&family, &ty)))?;
            write_delta(entries, FAMILY_OTHER_BLUES, reversed(map(&family_other, &ty)))?;
            plain(entries, STD_HW, one(&hw).map(|v| -sy(v)))?;
            plain(entries, STD_VW, one(&vw).map(|v| -sx(v)))?;
            write_delta(entries, STEM_SNAP_H, map(&snap_h, &|v| -sy(v)))?;
            write_delta(entries, STEM_SNAP_V, map(&snap_v, &|v| -sx(v)))?;
        }
        Hints::Clockwise | Hints::Counterclockwise => {
            let sign = if case == Hints::Clockwise { 1.0 } else { -1.0 };
            write_delta(entries, BLUE_VALUES, Some(FIXED_BLUES.to_vec()))?;
            for op in [OTHER_BLUES, FAMILY_BLUES, FAMILY_OTHER_BLUES] {
                set_entry(entries, op, None)?;
            }
            // Each standard width is replaced only when the other one is set, as rotatefont does.
            if let Some(h) = one(&hw) {
                plain(entries, STD_VW, Some(sign * s2(h)))?;
            }
            if let Some(v) = one(&vw) {
                plain(entries, STD_HW, Some(-sign * s1(v)))?;
            }
            write_delta(entries, STEM_SNAP_V, map(&snap_h, &|v| sign * s2(v)))?;
            write_delta(entries, STEM_SNAP_H, map(&snap_v, &|v| -sign * s1(v)))?;
        }
        Hints::Dropped => {
            write_delta(entries, BLUE_VALUES, Some(FIXED_BLUES.to_vec()))?;
            for op in [OTHER_BLUES, FAMILY_BLUES, FAMILY_OTHER_BLUES, STD_HW, STD_VW, STEM_SNAP_H, STEM_SNAP_V] {
                set_entry(entries, op, None)?;
            }
        }
    }
    Ok(0)
}

// ---------- writing ----------

struct Writer {
    cff2: bool,
    /// Regions of the glyph's item variation data.
    regions: usize,
    out: Vec<u8>,
}

impl Writer {
    /// Most operands one operator may take, each as a blend when any of them varies.
    fn max_args(&self, blended: bool) -> usize {
        if self.cff2 && blended { (MAX_CFF2_STACK - 1) / (self.regions + 1) } else { MAX_CFF_ARGS }
    }

    fn op(&mut self, args: &[V], op: &[u8]) -> Result<()> {
        if self.cff2 && args.iter().any(|v| varies(v)) {
            if args.iter().any(|v| v.len() > self.regions + 1) {
                return Err(invalid("a value has more deltas than its item variation data has regions"));
            }
            if args.len() > self.max_args(true) {
                return Err(TablesError::Unsupported(format!("{} varying operands with {} regions do not fit the stack", args.len(), self.regions)));
            }
            for v in args {
                put_cs_number(&mut self.out, at(v, 0))?;
            }
            for v in args {
                for r in 1..=self.regions {
                    put_cs_number(&mut self.out, at(v, r))?;
                }
            }
            put_cs_number(&mut self.out, args.len() as f64)?;
            self.out.push(16);
        } else {
            for v in args {
                put_cs_number(&mut self.out, at(v, 0))?;
            }
        }
        self.out.extend_from_slice(op);
        Ok(())
    }
}

/// Operands gathered for one operator, written when the next group would not fit.
struct Run {
    op: u8,
    args: Vec<V>,
}

fn flush(w: &mut Writer, run: &mut Option<Run>) -> Result<()> {
    if let Some(r) = run.take() {
        w.op(&r.args, &[r.op])?;
    }
    Ok(())
}

fn extend_run(w: &mut Writer, run: &mut Option<Run>, op: u8, group: Vec<V>) -> Result<()> {
    if let Some(r) = run.as_ref() {
        let blended = r.args.iter().chain(&group).any(|v| varies(v));
        if r.op != op || r.args.len() + group.len() > w.max_args(blended) {
            flush(w, run)?;
        }
    }
    match run {
        Some(r) => r.args.extend(group),
        None => *run = Some(Run { op, args: group }),
    }
    Ok(())
}

/// The charstring of `g` without its width.
fn encode(g: &Glyph, cff2: bool, regions: usize) -> Result<Vec<u8>> {
    let mut w = Writer { cff2, regions, out: Vec::new() };
    if cff2 && let Some(v) = g.vsindex {
        w.op(&[vec![v as f64]], &[15])?;
    }
    // Horizontal stems first, then vertical, each sorted; masks follow the new order.
    let mut order: Vec<usize> = (0..g.stems.len()).collect();
    let key = |k: usize| g.stems.get(k).map(|s| (s.vertical, at(&s.edges[0], 0), at(&s.edges[1], 0)));
    order.sort_by(|&i, &j| key(i).partial_cmp(&key(j)).unwrap_or(std::cmp::Ordering::Equal));
    let mut new_index = vec![0usize; g.stems.len()];
    for (new, old) in order.iter().enumerate() {
        if let Some(slot) = new_index.get_mut(*old) {
            *slot = new;
        }
    }
    let masked = !g.stems.is_empty() && g.items.iter().any(|i| matches!(i, Item::Mask(..)));
    for vertical in [false, true] {
        let op = match (vertical, masked) {
            (false, false) => 1,
            (false, true) => 18,
            (true, false) => 3,
            (true, true) => 23,
        };
        let mut args: Vec<V> = Vec::new();
        let mut previous = vec![0.0];
        for s in order.iter().filter_map(|i| g.stems.get(*i)).filter(|s| s.vertical == vertical) {
            let blended = args.iter().chain(&s.edges).any(|v| varies(v));
            if args.len() + 2 > w.max_args(blended) {
                w.op(&args, &[op])?;
                args.clear();
                previous = vec![0.0];
            }
            args.push(sub(&s.edges[0], &previous));
            args.push(sub(&s.edges[1], &s.edges[0]));
            previous = s.edges[1].clone();
        }
        if !args.is_empty() {
            w.op(&args, &[op])?;
        }
    }

    let mut point: Pt = [vec![0.0], vec![0.0]];
    let mut run: Option<Run> = None;
    for item in &g.items {
        match item {
            Item::Move(p) => {
                flush(&mut w, &mut run)?;
                let (dx, dy) = (sub(&p[0], &point[0]), sub(&p[1], &point[1]));
                if is_zero(&dx) {
                    w.op(&[dy], &[4])?;
                } else if is_zero(&dy) {
                    w.op(&[dx], &[22])?;
                } else {
                    w.op(&[dx, dy], &[21])?;
                }
                point = p.clone();
            }
            Item::Line(p) => {
                let (dx, dy) = (sub(&p[0], &point[0]), sub(&p[1], &point[1]));
                if is_zero(&dx) && !is_zero(&dy) {
                    flush(&mut w, &mut run)?;
                    w.op(&[dy], &[7])?;
                } else if is_zero(&dy) && !is_zero(&dx) {
                    flush(&mut w, &mut run)?;
                    w.op(&[dx], &[6])?;
                } else {
                    extend_run(&mut w, &mut run, 5, vec![dx, dy])?;
                }
                point = p.clone();
            }
            Item::Curve([p1, p2, p3]) => {
                let d = [
                    sub(&p1[0], &point[0]),
                    sub(&p1[1], &point[1]),
                    sub(&p2[0], &p1[0]),
                    sub(&p2[1], &p1[1]),
                    sub(&p3[0], &p2[0]),
                    sub(&p3[1], &p2[1]),
                ];
                let [dxa, dya, dxb, dyb, dxc, dyc] = &d;
                let single = if is_zero(dxa) && is_zero(dxc) {
                    Some((26, [dya, dxb, dyb, dyc]))
                } else if is_zero(dya) && is_zero(dyc) {
                    Some((27, [dxa, dxb, dyb, dxc]))
                } else if is_zero(dya) && is_zero(dxc) {
                    Some((31, [dxa, dxb, dyb, dyc]))
                } else if is_zero(dxa) && is_zero(dyc) {
                    Some((30, [dya, dxb, dyb, dxc]))
                } else {
                    None
                };
                match single {
                    Some((op, args)) => {
                        flush(&mut w, &mut run)?;
                        w.op(&args.map(Clone::clone), &[op])?;
                    }
                    None => extend_run(&mut w, &mut run, 8, d.to_vec())?,
                }
                point = p3.clone();
            }
            Item::Flex(points, depth) => {
                flush(&mut w, &mut run)?;
                let mut args = Vec::with_capacity(13);
                for p in points {
                    args.push(sub(&p[0], &point[0]));
                    args.push(sub(&p[1], &point[1]));
                    point = p.clone();
                }
                args.push(depth.clone());
                w.op(&args, &[12, 35])?;
            }
            Item::Mask(..) if g.stems.is_empty() => {}
            Item::Mask(op, bytes) => {
                flush(&mut w, &mut run)?;
                let mut out = vec![0u8; g.stems.len().div_ceil(8)];
                for (old, new) in new_index.iter().enumerate() {
                    let set = bytes.get(old / 8).is_some_and(|b| b & (0x80 >> (old % 8)) != 0);
                    if set && let Some(o) = out.get_mut(new / 8) {
                        *o |= 0x80 >> (new % 8);
                    }
                }
                w.op(&[], &[*op])?;
                w.out.extend_from_slice(&out);
            }
        }
    }
    flush(&mut w, &mut run)?;
    if !cff2 {
        match g.seac {
            Some(s) => w.op(&s.map(|v| vec![v]), &[14])?,
            None => w.op(&[], &[14])?,
        }
    }
    Ok(w.out)
}

#[cfg(test)]
#[path = "cff_matrix_tests.rs"]
mod tests;
