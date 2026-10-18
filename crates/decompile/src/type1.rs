//! Type 1 fonts (PFA, PFB) read into a PostScript abstract font, and the abstract font read into
//! the model (formats `type1` and, through [`crate::bare_cff`], `cff`).
//!
//! A [`PsFont`] keeps what AFDKO `tx` keeps of a PostScript font: the top and private
//! dictionaries, and per glyph its width and a list of hint and path operations in charstring
//! order ([`PsOp`]), with hint replacement, counter groups, flex and `seac` kept as such.
//! [`PsFont::dump_dicts`] and [`PsFont::dump_glyphs`] write it in the form of `tx -dump -0` and
//! `tx -dump -6` (from the `## Top Dict` and `## glyph[tag]` lines on), which is how the reader is
//! compared with tx.
//!
//! [`to_font`] makes a one-master document: cubic outlines (flex as its two curves), advance
//! widths, code points from the glyph names (Adobe Glyph List), `seac` accents as two components,
//! each glyph's distinct stems as numeric layer hints (hint replacement and counter hints are not
//! kept), the Private dict as alignment zones, standard stems and the blue parameters. Multiple
//! master fonts are read at their default charstrings; the decrypted text form that `detype1`
//! writes is not read.
//!
//! Input is untrusted: sizes, counts, subroutine nesting, the operand stack and the work spent
//! on one glyph are capped by the limits below, and a glyph whose charstring cannot be run is
//! kept empty with a warning. The operations and work of the whole font are capped too; a font
//! past them is refused.
#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Layout, Number, Str, Value};
use fontcraft_model::{
    AlignmentZone, Component, Family, Font, FontInfo, Glyph, GlyphTable, Hint, Layer, MAX_COORDINATE, MAX_GLYPHS, Master, MetricDef, MetricValue,
    Shape, Source, SourceFormat, StemDef, Transform, UPM_RANGE, metric,
};
use skrifa::outline::OutlinePen;
use skrifa::raw::ps::encoding::PredefinedEncoding;

use crate::{CubicPen, DecompileError, Decompiled, MASTER, UniqueNames, clean_name};

/// Largest Type 1 or bare CFF file read, in bytes.
pub const MAX_PS_BYTES: usize = 64 << 20;
/// Most subroutines one font may have.
pub const MAX_SUBRS: usize = 65_536;
/// Deepest subroutine nesting (the Type 2 limit, used for Type 1 too).
pub const MAX_SUBR_DEPTH: usize = 10;
/// Most operands on the charstring stack (the Type 2 limit; Type 1 allows 24).
pub const MAX_STACK: usize = 48;
/// Most bytes, operators and operands one glyph's charstring may run, subroutines included.
pub const MAX_CHARSTRING_WORK: usize = 200_000;
/// Most hint and path operations kept for one glyph.
pub const MAX_GLYPH_OPS: usize = 50_000;
/// Most hint and path operations kept for a whole font, all glyphs together.
pub const MAX_FONT_OPS: usize = 20_000_000;
/// Most charstring work (see [`MAX_CHARSTRING_WORK`]) a whole font may spend, all glyphs together.
pub const MAX_FONT_WORK: usize = 500_000_000;
/// Most numbers in one dictionary array (BlueValues, StemSnapH…).
pub const MAX_DICT_ARRAY: usize = 64;

/// Where a [`PsFont`] came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PsKind {
    #[default]
    Type1,
    Cff,
    CffCid,
}

/// The top dictionary (and FontInfo) values tx reports.
#[derive(Clone, Debug, PartialEq)]
pub struct TopDict {
    pub version: Option<String>,
    pub notice: Option<String>,
    pub copyright: Option<String>,
    pub full_name: Option<String>,
    pub family_name: Option<String>,
    pub weight: Option<String>,
    pub is_fixed_pitch: bool,
    pub italic_angle: f64,
    pub underline_position: f64,
    pub underline_thickness: f64,
    pub paint_type: i64,
    pub unique_id: Option<i64>,
    pub font_bbox: [f64; 4],
    pub stroke_width: f64,
    pub fs_type: Option<i64>,
    pub base_font_name: Option<String>,
    /// `None`: the default, 0.001 0 0 0.001 0 0.
    pub font_matrix: Option<[f64; 6]>,
}

impl Default for TopDict {
    fn default() -> Self {
        Self {
            version: None,
            notice: None,
            copyright: None,
            full_name: None,
            family_name: None,
            weight: None,
            is_fixed_pitch: false,
            italic_angle: 0.0,
            underline_position: -100.0,
            underline_thickness: 50.0,
            paint_type: 0,
            unique_id: None,
            font_bbox: [0.0; 4],
            stroke_width: 0.0,
            fs_type: None,
            base_font_name: None,
            font_matrix: None,
        }
    }
}

/// The private dictionary values tx reports; `None` and empty lists are absent entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrivateDict {
    pub blue_values: Vec<f64>,
    pub other_blues: Vec<f64>,
    pub family_blues: Vec<f64>,
    pub family_other_blues: Vec<f64>,
    pub blue_scale: Option<f64>,
    pub blue_shift: Option<f64>,
    pub blue_fuzz: Option<f64>,
    pub std_hw: Option<f64>,
    pub std_vw: Option<f64>,
    pub stem_snap_h: Vec<f64>,
    pub stem_snap_v: Vec<f64>,
    pub force_bold: Option<bool>,
    pub language_group: Option<i64>,
    pub expansion_factor: Option<f64>,
}

const DEFAULT_BLUE_SCALE: f64 = 0.039625;
const DEFAULT_BLUE_SHIFT: f64 = 7.0;
const DEFAULT_BLUE_FUZZ: f64 = 1.0;
const DEFAULT_EXPANSION_FACTOR: f64 = 0.06;

/// What a stem hint is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StemKind {
    /// `hstem`, `vstem` (an edge hint when a horizontal stem's width is -20 or -21).
    Stem,
    /// Type 1 `hstem3`, `vstem3`.
    Stem3,
    /// A counter hint (Type 2 `cntrmask`).
    Counter,
}

/// One hint or path operation of a glyph, in absolute glyph coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum PsOp {
    /// A horizontal stem from `y` to `y + dy` (its edges in charstring order).
    HStem {
        y: f64,
        dy: f64,
        kind: StemKind,
    },
    /// A vertical stem from `x` to `x + dx`.
    VStem {
        x: f64,
        dx: f64,
        kind: StemKind,
    },
    /// The hints that follow replace the active ones.
    NewHints,
    /// The counter hints that follow form a new group.
    NewGroup,
    Move(f64, f64),
    Line(f64, f64),
    Curve([f64; 6]),
    /// Two curves (their six points) and the flex depth.
    Flex([f64; 12], f64),
    /// An accented glyph: the accent's offset from the base and the two standard codes.
    Seac {
        adx: f64,
        ady: f64,
        base: u8,
        accent: u8,
    },
}

impl PsOp {
    /// Whether every number of the operation is finite and within the model's coordinate range.
    /// Charstring arithmetic can build huge values whose sums overflow; such an operation is
    /// refused before it reaches the model.
    pub(crate) fn in_range(&self) -> bool {
        let ok = |v: &f64| v.abs() <= MAX_COORDINATE;
        match self {
            PsOp::HStem { y: a, dy: b, .. } | PsOp::VStem { x: a, dx: b, .. } | PsOp::Move(a, b) | PsOp::Line(a, b) => ok(a) && ok(b),
            PsOp::Seac { adx, ady, .. } => ok(adx) && ok(ady),
            PsOp::Curve(v) => v.iter().all(ok),
            PsOp::Flex(v, depth) => v.iter().all(ok) && depth.is_finite(),
            PsOp::NewHints | PsOp::NewGroup => true,
        }
    }
}

/// One glyph of a [`PsFont`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PsGlyph {
    pub name: String,
    /// The character codes that select the glyph (empty for a CFF font with a predefined encoding).
    pub codes: Vec<u8>,
    pub width: f64,
    pub ops: Vec<PsOp>,
}

/// A PostScript font as tx's abstract font holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PsFont {
    pub kind: PsKind,
    pub font_name: String,
    pub top: TopDict,
    pub private: PrivateDict,
    pub glyphs: Vec<PsGlyph>,
}

// ---------------------------------------------------------------------------------------------
// Detection and entry points

/// Whether `bytes` start like a Type 1 font: a PFB segment marker or a PFA header.
pub fn is_type1(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x80, 0x01]) || bytes.starts_with(b"%!PS-AdobeFont") || bytes.starts_with(b"%!FontType1")
}

/// Reads a PFA or PFB font into the model.
pub fn decompile_type1(bytes: &[u8]) -> Result<Decompiled, DecompileError> {
    let mut warnings = Vec::new();
    let ps = read_type1(bytes, &mut warnings)?;
    to_font(&ps, warnings)
}

fn bad(msg: impl Into<String>) -> DecompileError {
    DecompileError::Unreadable(msg.into())
}

/// Reads a PFA or PFB font into a [`PsFont`]. Glyphs whose charstrings cannot be run are kept
/// empty and named in `warnings`.
pub fn read_type1(bytes: &[u8], warnings: &mut Vec<String>) -> Result<PsFont, DecompileError> {
    if bytes.len() > MAX_PS_BYTES {
        return Err(bad(format!("the font is {} bytes; the limit is {MAX_PS_BYTES}", bytes.len())));
    }
    let (clear, encrypted) = if bytes.starts_with(&[0x80, 0x01]) { pfb_parts(bytes)? } else { pfa_parts(bytes)? };
    let private = match encrypted {
        Some(e) => decrypt(&e, 55665, 4),
        None => Vec::new(),
    };
    let mut font = PsFont { kind: PsKind::Type1, ..Default::default() };
    let encoding = parse_clear(&clear, &mut font);
    // A font without an encrypted part keeps its private dictionary in the clear.
    let private_text: &[u8] = if private.is_empty() { &clear } else { &private };
    let (subrs, charstrings) = parse_private(private_text, &mut font)?;
    if charstrings.is_empty() {
        return Err(bad("the Type 1 font has no CharStrings"));
    }

    // `.notdef` first, as tx and the compilers order a Type 1 font.
    let mut order: Vec<usize> = (0..charstrings.len()).collect();
    if let Some(i) = charstrings.iter().position(|(n, _)| n == ".notdef") {
        order.retain(|&j| j != i);
        order.insert(0, i);
    }
    let mut codes: HashMap<&str, Vec<u8>> = HashMap::new();
    for code in 0..=255u8 {
        let name = match &encoding {
            Encoding::Standard => PredefinedEncoding::Standard.name(code),
            Encoding::Custom(map) => map.get(&code).map_or(".notdef", String::as_str),
        };
        if name != ".notdef" {
            codes.entry(name).or_default().push(code);
        }
    }
    let mut failed = Vec::new();
    let mut budget = FontBudget::new();
    for i in order {
        let Some((name, cs)) = charstrings.get(i) else { continue };
        let mut run = T1Run::new(&subrs);
        let result = run.run(cs, 0).map(|_| ());
        let finished = run.finish();
        if let Err(e) = result.and(finished) {
            failed.push(format!("{name} ({e})"));
        }
        budget.charge(run.ops.len(), run.work)?;
        font.glyphs.push(PsGlyph {
            name: name.clone(),
            codes: codes.get(name.as_str()).cloned().unwrap_or_default(),
            width: run.width,
            ops: run.ops,
        });
    }
    if !failed.is_empty() {
        warnings.push(format!("{} glyphs could not be read in full: {}", failed.len(), list(&failed)));
    }
    Ok(font)
}

pub(crate) fn list(items: &[String]) -> String {
    const SHOWN: usize = 8;
    let mut out: Vec<&str> = items.iter().take(SHOWN).map(String::as_str).collect();
    let more = items.len().saturating_sub(SHOWN);
    let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
    out.dedup();
    format!("{}{tail}", out.join(", "))
}

/// The clear text and the encrypted part of a PFB file (segments of type 1 and 2, up to type 3).
fn pfb_parts(bytes: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), DecompileError> {
    let mut clear = Vec::new();
    let mut binary = Vec::new();
    let mut pos = 0usize;
    while let Some(&[0x80, kind]) = bytes.get(pos..pos + 2) {
        if kind == 3 {
            break;
        }
        let len = bytes
            .get(pos + 2..pos + 6)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| bad("a PFB segment header is cut short"))?;
        let start = pos + 6;
        let end = start.checked_add(len as usize).filter(|&e| e <= bytes.len()).ok_or_else(|| bad("a PFB segment runs past the end of the file"))?;
        let data = bytes.get(start..end).unwrap_or_default();
        match kind {
            1 if binary.is_empty() => clear.extend_from_slice(data),
            1 => {}
            2 => binary.extend_from_slice(data),
            _ => return Err(bad(format!("unknown PFB segment type {kind}"))),
        }
        pos = end;
    }
    if clear.is_empty() {
        return Err(bad("the PFB file has no text segment"));
    }
    Ok((clear, (!binary.is_empty()).then_some(binary)))
}

/// The clear text and the encrypted part of a PFA file: hex after `eexec`, or binary.
fn pfa_parts(bytes: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), DecompileError> {
    let Some(at) = find(bytes, b"eexec") else {
        return Ok((bytes.to_vec(), None));
    };
    let clear = bytes.get(..at + 5).unwrap_or_default().to_vec();
    let mut pos = at + 5;
    while bytes.get(pos).is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
        pos += 1;
    }
    let rest = bytes.get(pos..).unwrap_or_default();
    let is_hex = rest.get(..4).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit));
    if !is_hex {
        return Ok((clear, Some(rest.to_vec())));
    }
    let mut out = Vec::with_capacity(rest.len() / 2);
    let mut high: Option<u8> = None;
    for &b in rest {
        let v = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => break,
        };
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    Ok((clear, Some(out)))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Type 1 decryption (eexec key 55665, charstring key 4330), dropping the first `skip` bytes.
pub(crate) fn decrypt(data: &[u8], key: u16, skip: usize) -> Vec<u8> {
    let mut r = key;
    let mut out = Vec::with_capacity(data.len());
    for &c in data {
        out.push(c ^ (r >> 8) as u8);
        r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
    }
    out.into_iter().skip(skip).collect()
}

// ---------------------------------------------------------------------------------------------
// PostScript tokens

#[derive(Clone, Debug, PartialEq)]
enum Tok<'a> {
    Name(&'a [u8]),
    Word(&'a [u8]),
    Num(f64),
    Str(Vec<u8>),
    Open,
    Close,
}

struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') || is_space(b)
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0c)
}

impl<'a> Lexer<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn byte(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn regular(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.byte().is_some_and(|b| !is_delim(b)) {
            self.pos += 1;
        }
        self.data.get(start..self.pos).unwrap_or_default()
    }

    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            let b = self.byte()?;
            if is_space(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.byte().is_some_and(|b| b != b'\n' && b != b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        let b = self.byte()?;
        match b {
            b'/' => {
                self.pos += 1;
                Some(Tok::Name(self.regular()))
            }
            b'[' | b'{' => {
                self.pos += 1;
                Some(Tok::Open)
            }
            b']' | b'}' => {
                self.pos += 1;
                Some(Tok::Close)
            }
            b'(' => {
                self.pos += 1;
                Some(Tok::Str(self.string()))
            }
            b'<' | b'>' => {
                self.pos += 1;
                if b == b'<' && self.byte() != Some(b'<') {
                    // A hex string: kept as a word, its contents are never needed here.
                    while self.byte().is_some_and(|c| c != b'>') {
                        self.pos += 1;
                    }
                    self.pos += 1;
                    return Some(Tok::Word(b"<>"));
                }
                if self.byte() == Some(b) {
                    self.pos += 1;
                }
                Some(Tok::Word(if b == b'<' { b"<<" } else { b">>" }))
            }
            b')' => {
                self.pos += 1;
                Some(Tok::Word(b")"))
            }
            _ => {
                let text = self.regular();
                let number = std::str::from_utf8(text).ok().and_then(|t| t.parse::<f64>().ok()).filter(|v| v.is_finite());
                Some(match number {
                    Some(v) if text.first().is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.')) => Tok::Num(v),
                    _ => Tok::Word(text),
                })
            }
        }
    }

    fn peek(&mut self) -> Option<Tok<'a>> {
        let at = self.pos;
        let t = self.next();
        self.pos = at;
        t
    }

    /// The rest of a `( … )` string, escapes resolved.
    fn string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        while let Some(b) = self.byte() {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' if depth == 0 => break,
                b')' => {
                    depth -= 1;
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.byte() else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.byte() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xFF) as u8);
                        }
                        b'\r' | b'\n' => {}
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
        out
    }

    fn text(&mut self) -> Option<String> {
        match self.next()? {
            Tok::Str(s) => Some(latin1(&s)),
            Tok::Name(n) => Some(latin1(n)),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f64> {
        match self.next()? {
            Tok::Num(v) => Some(v),
            Tok::Open => self.array().into_iter().next(),
            _ => None,
        }
    }

    fn boolean(&mut self) -> Option<bool> {
        match self.next()? {
            Tok::Word(b"true") => Some(true),
            Tok::Word(b"false") => Some(false),
            _ => None,
        }
    }

    /// The numbers of an array whose opening bracket was just read, up to the closing one.
    fn array(&mut self) -> Vec<f64> {
        let mut out = Vec::new();
        while let Some(t) = self.next() {
            match t {
                Tok::Num(v) if out.len() < MAX_DICT_ARRAY => out.push(v),
                Tok::Num(_) => {}
                _ => break,
            }
        }
        out
    }

    fn array_value(&mut self) -> Vec<f64> {
        match self.next() {
            Some(Tok::Open) => self.array(),
            Some(Tok::Num(v)) => vec![v],
            _ => Vec::new(),
        }
    }

    /// `n` bytes after the single space that follows `RD`.
    fn binary(&mut self, n: usize) -> Option<&'a [u8]> {
        let start = self.pos + 1;
        let end = start.checked_add(n)?;
        let out = self.data.get(start..end)?;
        self.pos = end;
        Some(out)
    }
}

/// Text as Latin-1 (PostScript strings are bytes).
fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| char::from(c)).collect()
}

enum Encoding {
    Standard,
    Custom(BTreeMap<u8, String>),
}

fn parse_clear(text: &[u8], font: &mut PsFont) -> Encoding {
    let mut lx = Lexer::new(text);
    let mut encoding = Encoding::Standard;
    let top = &mut font.top;
    while let Some(t) = lx.next() {
        let Tok::Name(key) = t else { continue };
        match key {
            b"FontName" if font.font_name.is_empty() => font.font_name = lx.text().unwrap_or_default(),
            b"version" => top.version = lx.text(),
            b"Notice" => top.notice = lx.text(),
            b"Copyright" => top.copyright = lx.text(),
            b"FullName" => top.full_name = lx.text(),
            b"FamilyName" => top.family_name = lx.text(),
            b"Weight" if top.weight.is_none() => top.weight = lx.text(),
            b"isFixedPitch" => top.is_fixed_pitch = lx.boolean().unwrap_or(false),
            b"ItalicAngle" => top.italic_angle = lx.number().unwrap_or(0.0),
            b"UnderlinePosition" => top.underline_position = lx.number().unwrap_or(-100.0),
            b"UnderlineThickness" => top.underline_thickness = lx.number().unwrap_or(50.0),
            b"PaintType" => top.paint_type = lx.number().unwrap_or(0.0) as i64,
            b"UniqueID" => top.unique_id = lx.number().map(|v| v as i64),
            b"StrokeWidth" => top.stroke_width = lx.number().unwrap_or(0.0),
            b"FSType" => top.fs_type = lx.number().map(|v| v as i64),
            b"FontBBox" => {
                if let [a, b, c, d] = lx.array_value().as_slice() {
                    top.font_bbox = [*a, *b, *c, *d];
                }
            }
            b"FontMatrix" => {
                if let [a, b, c, d, e, f] = lx.array_value().as_slice() {
                    let m = [*a, *b, *c, *d, *e, *f];
                    top.font_matrix = (m != [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]).then_some(m);
                }
            }
            b"Encoding" => encoding = parse_encoding(&mut lx),
            _ => {}
        }
    }
    encoding
}

fn parse_encoding(lx: &mut Lexer) -> Encoding {
    match lx.peek() {
        Some(Tok::Word(b"StandardEncoding")) => return Encoding::Standard,
        Some(Tok::Num(_)) => {}
        _ => return Encoding::Standard,
    }
    let mut map = BTreeMap::new();
    let mut last: [Option<Tok>; 3] = [None, None, None];
    while let Some(t) = lx.next() {
        if t == Tok::Word(b"def") {
            break;
        }
        if t == Tok::Word(b"put")
            && let [Some(Tok::Word(b"dup")), Some(Tok::Num(code)), Some(Tok::Name(name))] = &last
            && (0.0..=255.0).contains(code)
        {
            map.insert(*code as u8, latin1(name));
        }
        last = [last[1].take(), last[2].take(), Some(t)];
    }
    Encoding::Custom(map)
}

type Charstrings = Vec<(String, Vec<u8>)>;

/// The Subrs array: one charstring per index, `None` where the font leaves a gap.
type Subrs = Vec<Option<Vec<u8>>>;
fn parse_private(text: &[u8], font: &mut PsFont) -> Result<(Subrs, Charstrings), DecompileError> {
    let mut lx = Lexer::new(text);
    let mut len_iv: i64 = 4;
    let mut subrs = Vec::new();
    let mut charstrings = Vec::new();
    let p = &mut font.private;
    while let Some(t) = lx.next() {
        let Tok::Name(key) = t else { continue };
        match key {
            b"BlueValues" => p.blue_values = lx.array_value(),
            b"OtherBlues" => p.other_blues = lx.array_value(),
            b"FamilyBlues" => p.family_blues = lx.array_value(),
            b"FamilyOtherBlues" => p.family_other_blues = lx.array_value(),
            b"BlueScale" => p.blue_scale = lx.number(),
            b"BlueShift" => p.blue_shift = lx.number(),
            b"BlueFuzz" => p.blue_fuzz = lx.number(),
            b"StdHW" => p.std_hw = lx.number(),
            b"StdVW" => p.std_vw = lx.number(),
            b"StemSnapH" => p.stem_snap_h = lx.array_value(),
            b"StemSnapV" => p.stem_snap_v = lx.array_value(),
            b"ForceBold" => p.force_bold = lx.boolean(),
            b"LanguageGroup" => p.language_group = lx.number().map(|v| v as i64),
            b"ExpansionFactor" => p.expansion_factor = lx.number(),
            b"lenIV" => len_iv = lx.number().map_or(4, |v| v as i64).clamp(-1, 64),
            b"Subrs" if subrs.is_empty() => subrs = read_subrs(&mut lx, len_iv),
            b"CharStrings" => {
                charstrings = read_charstrings(&mut lx, len_iv);
                break;
            }
            _ => {}
        }
    }
    Ok((subrs, charstrings))
}

fn charstring(data: &[u8], len_iv: i64) -> Vec<u8> {
    if len_iv < 0 { data.to_vec() } else { decrypt(data, 4330, len_iv as usize) }
}

fn is_rd(t: &Tok) -> bool {
    matches!(t, Tok::Word(b"RD" | b"-|"))
}

fn is_trailer(t: &Tok) -> bool {
    matches!(t, Tok::Word(b"NP" | b"|" | b"ND" | b"|-" | b"noaccess" | b"put" | b"def" | b"readonly" | b"executeonly"))
}

fn read_subrs(lx: &mut Lexer, len_iv: i64) -> Vec<Option<Vec<u8>>> {
    let count = match lx.next() {
        Some(Tok::Num(n)) if n >= 0.0 => (n as usize).min(MAX_SUBRS),
        _ => return Vec::new(),
    };
    let mut subrs = vec![None; count];
    while let Some(t) = lx.peek() {
        if t == Tok::Word(b"array") || is_trailer(&t) {
            lx.next();
            continue;
        }
        if t != Tok::Word(b"dup") {
            break;
        }
        // `dup /CharStrings` can follow the last subroutine: that `dup` is left for the caller.
        let at = lx.pos;
        lx.next();
        let (Some(Tok::Num(index)), Some(Tok::Num(len))) = (lx.next(), lx.next()) else {
            lx.pos = at;
            break;
        };
        if !lx.next().is_some_and(|t| is_rd(&t)) || len < 0.0 {
            break;
        }
        let Some(data) = lx.binary(len as usize) else { break };
        if index >= 0.0
            && let Some(slot) = subrs.get_mut(index as usize)
        {
            *slot = Some(charstring(data, len_iv));
        }
    }
    subrs
}

fn read_charstrings(lx: &mut Lexer, len_iv: i64) -> Charstrings {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    while let Some(t) = lx.peek() {
        match t {
            Tok::Num(_) | Tok::Word(b"dict" | b"dup" | b"begin") => {
                lx.next();
            }
            t if is_trailer(&t) => {
                lx.next();
            }
            Tok::Name(name) => {
                lx.next();
                let Some(Tok::Num(len)) = lx.next() else { break };
                if !lx.next().is_some_and(|t| is_rd(&t)) || len < 0.0 {
                    break;
                }
                let Some(data) = lx.binary(len as usize) else { break };
                if out.len() >= MAX_GLYPHS {
                    break;
                }
                let name = latin1(name);
                if seen.insert(name.clone()) {
                    out.push((name, charstring(data, len_iv)));
                }
            }
            _ => break,
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Charstrings: hint bookkeeping shared by Type 1 and Type 2

/// Collects a glyph's hints and writes them out in tx's order: the first set before the first
/// path operation, each replacement set after a [`PsOp::NewHints`].
#[derive(Default)]
pub(crate) struct Hints {
    pub emitted: bool,
    pub replacing: bool,
    pub pending: Vec<PsOp>,
}

impl Hints {
    pub fn stem(&mut self, op: PsOp) {
        if !op.in_range() {
            return;
        }
        if self.emitted {
            self.replacing = true;
        }
        if self.pending.len() < MAX_GLYPH_OPS {
            self.pending.push(op);
        }
    }

    /// Before a path operation. Fails, writing nothing, when the glyph would pass
    /// [`MAX_GLYPH_OPS`].
    pub fn flush(&mut self, ops: &mut Vec<PsOp>) -> Result<(), String> {
        if (!self.emitted || self.replacing) && ops.len().saturating_add(self.pending.len()).saturating_add(1) > MAX_GLYPH_OPS {
            return Err(TOO_MANY_OPS.into());
        }
        if !self.emitted {
            ops.append(&mut self.pending);
            self.emitted = true;
        } else if self.replacing {
            ops.push(PsOp::NewHints);
            ops.append(&mut self.pending);
        }
        self.replacing = false;
        Ok(())
    }
}

pub(crate) const TOO_MANY_OPS: &str = "too many operations";

/// What a whole font has spent so far, charged after each glyph: the font is refused once it
/// passes either limit.
pub(crate) struct FontBudget {
    ops: usize,
    work: usize,
    max_ops: usize,
    max_work: usize,
}

impl FontBudget {
    pub fn new() -> Self {
        Self::with_limits(MAX_FONT_OPS, MAX_FONT_WORK)
    }

    pub fn with_limits(max_ops: usize, max_work: usize) -> Self {
        Self { ops: 0, work: 0, max_ops, max_work }
    }

    pub fn charge(&mut self, ops: usize, work: usize) -> Result<(), DecompileError> {
        self.ops = self.ops.saturating_add(ops);
        self.work = self.work.saturating_add(work);
        if self.ops > self.max_ops {
            return Err(bad(format!("the glyphs hold more than {} hint and path operations, the limit for one font", self.max_ops)));
        }
        if self.work > self.max_work {
            return Err(bad(format!("the charstrings take more than {} steps to run, the limit for one font", self.max_work)));
        }
        Ok(())
    }
}

/// Reads a charstring number at `cs[i]` (first byte `b`, already consumed); Type 2 numbers 28
/// and 255 are handled by the caller.
pub(crate) fn short_number(b: u8, cs: &[u8], i: &mut usize) -> Option<f64> {
    match b {
        32..=246 => Some(f64::from(b) - 139.0),
        247..=250 => {
            let b1 = *cs.get(*i)?;
            *i += 1;
            Some(f64::from(u16::from(b - 247) * 256 + u16::from(b1)) + 108.0)
        }
        251..=254 => {
            let b1 = *cs.get(*i)?;
            *i += 1;
            Some(-f64::from(u16::from(b - 251) * 256 + u16::from(b1)) - 108.0)
        }
        _ => None,
    }
}

pub(crate) fn be_i32(cs: &[u8], i: &mut usize) -> Option<i32> {
    let b = cs.get(*i..*i + 4)?;
    *i += 4;
    Some(i32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
}

enum Flow {
    Next,
    Return,
    End,
}

struct T1Run<'a> {
    subrs: &'a [Option<Vec<u8>>],
    stack: Vec<f64>,
    ps: Vec<f64>,
    x: f64,
    y: f64,
    sbx: f64,
    sby: f64,
    width: f64,
    flex: Option<Vec<(f64, f64)>>,
    hints: Hints,
    ops: Vec<PsOp>,
    work: usize,
}

impl<'a> T1Run<'a> {
    fn new(subrs: &'a [Option<Vec<u8>>]) -> Self {
        Self {
            subrs,
            stack: Vec::new(),
            ps: Vec::new(),
            x: 0.0,
            y: 0.0,
            sbx: 0.0,
            sby: 0.0,
            width: 0.0,
            flex: None,
            hints: Hints::default(),
            ops: Vec::new(),
            work: 0,
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        if !self.hints.emitted {
            self.hints.flush(&mut self.ops)?;
        }
        Ok(())
    }

    fn push(&mut self, v: f64) -> Result<(), String> {
        if self.stack.len() >= MAX_STACK {
            return Err("operand stack overflow".into());
        }
        self.stack.push(v);
        Ok(())
    }

    fn pop(&mut self) -> Result<f64, String> {
        self.stack.pop().ok_or_else(|| "operand stack underflow".to_string())
    }

    /// The last `n` operands; the stack is cleared.
    fn take<const N: usize>(&mut self) -> Result<[f64; N], String> {
        let len = self.stack.len();
        if len < N {
            return Err("too few operands".into());
        }
        let mut out = [0.0; N];
        for (o, v) in out.iter_mut().zip(self.stack.iter().skip(len - N)) {
            *o = *v;
        }
        self.stack.clear();
        Ok(out)
    }

    fn emit(&mut self, op: PsOp) -> Result<(), String> {
        if self.ops.len() >= MAX_GLYPH_OPS {
            return Err(TOO_MANY_OPS.into());
        }
        if !op.in_range() {
            return Err(format!("a point lies outside the coordinate range of ±{MAX_COORDINATE}"));
        }
        self.hints.flush(&mut self.ops)?;
        self.ops.push(op);
        Ok(())
    }

    fn move_by(&mut self, dx: f64, dy: f64) -> Result<(), String> {
        self.x += dx;
        self.y += dy;
        if let Some(points) = &mut self.flex {
            if points.len() < 16 {
                points.push((self.x, self.y));
            }
            return Ok(());
        }
        self.emit(PsOp::Move(self.x, self.y))
    }

    fn line_by(&mut self, dx: f64, dy: f64) -> Result<(), String> {
        self.x += dx;
        self.y += dy;
        self.emit(PsOp::Line(self.x, self.y))
    }

    fn curve_by(&mut self, d: [f64; 6]) -> Result<(), String> {
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        let (x3, y3) = (x2 + d[4], y2 + d[5]);
        self.x = x3;
        self.y = y3;
        self.emit(PsOp::Curve([x1, y1, x2, y2, x3, y3]))
    }

    fn run(&mut self, cs: &[u8], depth: usize) -> Result<Flow, String> {
        if depth > MAX_SUBR_DEPTH {
            return Err("subroutines nest too deeply".into());
        }
        let mut i = 0usize;
        while let Some(&b) = cs.get(i) {
            i += 1;
            self.work += 1;
            if self.work > MAX_CHARSTRING_WORK {
                return Err("the charstring does too much work".into());
            }
            let flow = match b {
                32..=254 => {
                    let v = short_number(b, cs, &mut i).ok_or("a number is cut short")?;
                    self.push(v)?;
                    Flow::Next
                }
                255 => {
                    let v = be_i32(cs, &mut i).ok_or("a number is cut short")?;
                    self.push(f64::from(v))?;
                    Flow::Next
                }
                12 => {
                    let b1 = *cs.get(i).ok_or("an operator is cut short")?;
                    i += 1;
                    self.escape(b1)?
                }
                10 => {
                    let index = self.pop()?;
                    let subrs = self.subrs;
                    let sub =
                        (index >= 0.0).then(|| subrs.get(index as usize)).flatten().and_then(|s| s.as_deref()).ok_or("a subroutine is missing")?;
                    match self.run(sub, depth + 1)? {
                        Flow::End => Flow::End,
                        _ => Flow::Next,
                    }
                }
                _ => self.op(b)?,
            };
            match flow {
                Flow::Next => {}
                Flow::Return => return Ok(Flow::Return),
                Flow::End => return Ok(Flow::End),
            }
        }
        Ok(Flow::Return)
    }

    fn op(&mut self, b: u8) -> Result<Flow, String> {
        match b {
            1 => {
                let [y, dy] = self.take()?;
                self.hints.stem(PsOp::HStem { y: y + self.sby, dy, kind: StemKind::Stem });
            }
            3 => {
                let [x, dx] = self.take()?;
                self.hints.stem(PsOp::VStem { x: x + self.sbx, dx, kind: StemKind::Stem });
            }
            4 => {
                let [dy] = self.take()?;
                self.move_by(0.0, dy)?;
            }
            5 => {
                let [dx, dy] = self.take()?;
                self.line_by(dx, dy)?;
            }
            6 => {
                let [dx] = self.take()?;
                self.line_by(dx, 0.0)?;
            }
            7 => {
                let [dy] = self.take()?;
                self.line_by(0.0, dy)?;
            }
            8 => {
                let d = self.take::<6>()?;
                self.curve_by(d)?;
            }
            9 => self.stack.clear(),
            11 => return Ok(Flow::Return),
            13 => {
                let [sbx, wx] = self.take()?;
                (self.sbx, self.sby, self.width, self.x, self.y) = (sbx, 0.0, wx, sbx, 0.0);
            }
            14 => {
                self.stack.clear();
                return Ok(Flow::End);
            }
            21 => {
                let [dx, dy] = self.take()?;
                self.move_by(dx, dy)?;
            }
            22 => {
                let [dx] = self.take()?;
                self.move_by(dx, 0.0)?;
            }
            30 => {
                let [dy1, dx2, dy2, dx3] = self.take()?;
                self.curve_by([0.0, dy1, dx2, dy2, dx3, 0.0])?;
            }
            31 => {
                let [dx1, dx2, dy2, dy3] = self.take()?;
                self.curve_by([dx1, 0.0, dx2, dy2, 0.0, dy3])?;
            }
            other => return Err(format!("unknown operator {other}")),
        }
        Ok(Flow::Next)
    }

    fn escape(&mut self, b: u8) -> Result<Flow, String> {
        match b {
            0 => self.stack.clear(),
            1 | 2 => {
                let v = self.take::<6>()?;
                for &[a, d] in v.as_chunks::<2>().0 {
                    let horizontal = b == 2;
                    self.hints.stem(if horizontal {
                        PsOp::HStem { y: a + self.sby, dy: d, kind: StemKind::Stem3 }
                    } else {
                        PsOp::VStem { x: a + self.sbx, dx: d, kind: StemKind::Stem3 }
                    });
                }
            }
            6 => {
                let [asb, adx, ady, base, accent] = self.take()?;
                let code = |v: f64| v.clamp(0.0, 255.0) as u8;
                self.emit(PsOp::Seac { adx: adx + self.sbx - asb, ady, base: code(base), accent: code(accent) })?;
                return Ok(Flow::End);
            }
            7 => {
                let [sbx, sby, wx, _wy] = self.take()?;
                (self.sbx, self.sby, self.width, self.x, self.y) = (sbx, sby, wx, sbx, sby);
            }
            12 => {
                let b = self.pop()?;
                let a = self.pop()?;
                let v = if b == 0.0 { 0.0 } else { a / b };
                self.push(if v.is_finite() { v } else { 0.0 })?;
            }
            16 => {
                let other = self.pop()?;
                let n = self.pop()?;
                if !(0.0..=MAX_STACK as f64).contains(&n) {
                    return Err("bad othersubr argument count".into());
                }
                let n = n as usize;
                let start = self.stack.len().checked_sub(n).ok_or("too few othersubr arguments")?;
                let args = self.stack.split_off(start);
                self.othersubr(other, &args)?;
            }
            17 => {
                let v = self.ps.pop().unwrap_or(0.0);
                self.push(v)?;
            }
            33 => {
                let [x, y] = self.take()?;
                self.x = x;
                self.y = y;
            }
            other => return Err(format!("unknown operator 12 {other}")),
        }
        Ok(Flow::Next)
    }

    fn othersubr(&mut self, other: f64, args: &[f64]) -> Result<(), String> {
        match (other as i64, args) {
            (0, &[depth, ex, ey]) => {
                let points = self.flex.take().unwrap_or_default();
                if let [_, (x1, y1), (x2, y2), (x3, y3), (x4, y4), (x5, y5), (x6, y6)] = points.as_slice() {
                    self.emit(PsOp::Flex([*x1, *y1, *x2, *y2, *x3, *y3, *x4, *y4, *x5, *y5, *x6, *y6], depth))?;
                } else {
                    self.emit(PsOp::Line(ex, ey))?;
                }
                self.x = ex;
                self.y = ey;
                self.ps = vec![ey, ex];
            }
            (1, _) => {
                self.flex = Some(Vec::new());
                self.ps.clear();
            }
            (2, _) => self.ps.clear(),
            (3, &[subr]) => {
                if self.hints.emitted {
                    self.hints.replacing = true;
                }
                self.hints.pending.clear();
                self.ps = vec![subr];
            }
            _ => self.ps = args.to_vec(),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// The dump in tx's form

/// A number as tx prints it: `%g`, six significant digits.
pub fn fmt_num(v: f64) -> String {
    fmt_g(v, 6)
}

/// `v` in C's `%.{precision}g` form.
pub fn fmt_g(v: f64, precision: usize) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v == 0.0 {
        return "0".into();
    }
    let p = precision.clamp(1, 17);
    let sci = format!("{:.*e}", p - 1, v);
    let Some((mantissa, exp)) = sci.split_once('e') else { return sci };
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: String| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if exp < -4 || exp >= p as i32 {
        let m = trim(mantissa.to_string());
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{m}e{sign}{:02}", exp.abs());
    }
    let decimals = (p as i32 - 1 - exp).max(0) as usize;
    trim(format!("{v:.decimals$}"))
}

fn fmt_list(v: &[f64]) -> String {
    format!("{{{}}}", v.iter().map(|x| fmt_num(*x)).collect::<Vec<_>>().join(","))
}

impl PsFont {
    /// The dictionaries as `tx -dump -0` prints them, from `## Top Dict` on.
    pub fn dump_dicts(&self) -> String {
        let mut out = String::new();
        let mut line = |key: &str, value: String| {
            let _ = writeln!(out, "{key:<20}{value}");
        };
        let t = &self.top;
        out_line_str(&mut line, "version", &t.version);
        out_line_str(&mut line, "Notice", &t.notice);
        out_line_str(&mut line, "Copyright", &t.copyright);
        out_line_str(&mut line, "FullName", &t.full_name);
        out_line_str(&mut line, "FamilyName", &t.family_name);
        out_line_str(&mut line, "Weight", &t.weight);
        if t.is_fixed_pitch {
            line("isFixedPitch", "true".into());
        }
        if t.italic_angle != 0.0 {
            line("ItalicAngle", fmt_num(t.italic_angle));
        }
        if t.underline_position != -100.0 {
            line("UnderlinePosition", fmt_num(t.underline_position));
        }
        if t.underline_thickness != 50.0 {
            line("UnderlineThickness", fmt_num(t.underline_thickness));
        }
        if t.paint_type != 0 {
            line("PaintType", t.paint_type.to_string());
        }
        if let Some(id) = t.unique_id {
            line("UniqueID", id.to_string());
        }
        line("FontBBox", fmt_list(&t.font_bbox));
        if t.stroke_width != 0.0 {
            line("StrokeWidth", fmt_num(t.stroke_width));
        }
        if let Some(fs) = t.fs_type {
            line("FSType", fs.to_string());
        }
        out_line_str(&mut line, "BaseFontName", &t.base_font_name);
        let kind = match self.kind {
            PsKind::Type1 => "Type 1 (name-keyed)",
            PsKind::Cff => "CFF (name-keyed)",
            PsKind::CffCid => "CFF (CID-keyed)",
        };
        line("sup.srcFontType", kind.into());
        let upem = self.units_per_em();
        if upem != 1000 {
            line("sup.UnitsPerEm", upem.to_string());
        }
        line("sup.nGlyphs", self.glyphs.len().to_string());
        let mut out = format!("## Top Dict\n{out}");
        let _ = writeln!(out, "## FontDict[0]");
        let _ = writeln!(out, "{:<20}\"{}\"", "FontName", self.font_name);
        if let Some(m) = &t.font_matrix {
            let _ = writeln!(out, "{:<20}{{{}}}", "FontMatrix", m.iter().map(|v| fmt_g(f64::from(*v as f32), 8)).collect::<Vec<_>>().join(","));
        }
        let _ = writeln!(out, "## Private");
        let p = &self.private;
        let mut line = |key: &str, value: String| {
            let _ = writeln!(out, "{key:<20}{value}");
        };
        for (key, v) in [
            ("BlueValues", &p.blue_values),
            ("OtherBlues", &p.other_blues),
            ("FamilyBlues", &p.family_blues),
            ("FamilyOtherBlues", &p.family_other_blues),
        ] {
            if !v.is_empty() {
                line(key, fmt_list(v));
            }
        }
        for (key, v, default) in [
            ("BlueScale", p.blue_scale, DEFAULT_BLUE_SCALE),
            ("BlueShift", p.blue_shift, DEFAULT_BLUE_SHIFT),
            ("BlueFuzz", p.blue_fuzz, DEFAULT_BLUE_FUZZ),
        ] {
            if let Some(v) = v.filter(|v| *v != default) {
                line(key, fmt_num(v));
            }
        }
        for (key, v) in [("StdHW", p.std_hw), ("StdVW", p.std_vw)] {
            if let Some(v) = v {
                line(key, fmt_num(v));
            }
        }
        for (key, v) in [("StemSnapH", &p.stem_snap_h), ("StemSnapV", &p.stem_snap_v)] {
            if !v.is_empty() {
                line(key, fmt_list(v));
            }
        }
        if p.force_bold == Some(true) {
            line("ForceBold", "true".into());
        }
        if let Some(g) = p.language_group.filter(|g| *g != 0) {
            line("LanguageGroup", g.to_string());
        }
        if let Some(e) = p.expansion_factor.filter(|e| *e != DEFAULT_EXPANSION_FACTOR) {
            line("ExpansionFactor", fmt_num(e));
        }
        out
    }

    /// The glyphs as `tx -dump -6` prints them, from `## glyph[tag]` on.
    pub fn dump_glyphs(&self) -> String {
        let mut out = String::from("## glyph[tag] {name,encoding,path}\n");
        for (i, g) in self.glyphs.iter().enumerate() {
            let enc = if g.codes.is_empty() { "-".to_string() } else { g.codes.iter().map(|c| format!("0x{c:02X}")).collect::<Vec<_>>().join("+") };
            let _ = writeln!(out, "glyph[{i}] {{{},{enc},", g.name);
            let _ = writeln!(out, "  {} width", fmt_num(g.width));
            for op in &g.ops {
                let n = fmt_num;
                let text = match op {
                    PsOp::HStem { y, dy, kind } => stem_text(*y, *dy, *kind, true),
                    PsOp::VStem { x, dx, kind } => stem_text(*x, *dx, *kind, false),
                    PsOp::NewHints => "newhints".into(),
                    PsOp::NewGroup => "newgroup".into(),
                    PsOp::Move(x, y) => format!("{} {} move", n(*x), n(*y)),
                    PsOp::Line(x, y) => format!("{} {} line", n(*x), n(*y)),
                    PsOp::Curve(c) => format!("{} curve", c.iter().map(|v| n(*v)).collect::<Vec<_>>().join(" ")),
                    PsOp::Flex(c, depth) => format!("{} {} flex", c.iter().map(|v| n(*v)).collect::<Vec<_>>().join(" "), n(*depth)),
                    PsOp::Seac { adx, ady, base, accent } => format!("{} {} {base} {accent} seac", n(*adx), n(*ady)),
                };
                let _ = writeln!(out, "  {text}");
            }
            let _ = writeln!(out, "  endchar}}");
        }
        out
    }

    /// Units per em from the font matrix (1000 when it is the default or not a plain scale).
    pub fn units_per_em(&self) -> u16 {
        let Some([a, b, c, d, _, _]) = self.top.font_matrix else { return 1000 };
        if b != 0.0 || c != 0.0 || a <= 0.0 || (a - d).abs() > 1e-9 {
            return 1000;
        }
        let upem = (1.0 / a).round();
        if upem >= f64::from(*UPM_RANGE.start()) && upem <= f64::from(*UPM_RANGE.end()) { upem as u16 } else { 1000 }
    }
}

fn out_line_str(line: &mut impl FnMut(&str, String), key: &str, v: &Option<String>) {
    if let Some(v) = v {
        line(key, format!("\"{v}\""));
    }
}

fn stem_text(pos: f64, delta: f64, kind: StemKind, horizontal: bool) -> String {
    let n = fmt_num;
    let axis = if horizontal { 'h' } else { 'v' };
    match kind {
        StemKind::Stem if horizontal && delta == -20.0 => format!("{} topedge", n(pos)),
        StemKind::Stem if horizontal && delta == -21.0 => format!("{} bottomedge", n(pos + delta)),
        StemKind::Stem => format!("{} {} {axis}stem", n(pos), n(pos + delta)),
        StemKind::Stem3 => format!("{} {} {axis}stem3", n(pos), n(pos + delta)),
        StemKind::Counter => format!("{} {} {axis}cntr", n(pos), n(pos + delta)),
    }
}

// ---------------------------------------------------------------------------------------------
// The model

fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::Number(Number::int(v as i64))
    } else {
        Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
    }
}

/// A numeric layer hint: `place` is (position, width); ghost hints have width 0 at their edge.
fn hint(horizontal: bool, kind: &str, place: (f64, f64)) -> Hint {
    let mut d = Dict::new();
    if horizontal {
        d.insert("horizontal", Value::Number(Number::int(1)));
    }
    d.insert("place", Value::Array(Array { items: vec![number(place.0), number(place.1)], layout: Layout::Compact, ..Default::default() }));
    d.insert("type", Value::String(Str::new(kind)));
    Hint { family: Family::Glyphs, value: Value::Dict(d) }
}

/// The distinct stems of a glyph as layer hints, in first-seen order; counter hints are left out.
pub(crate) fn layer_hints(ops: &[PsOp]) -> Vec<Hint> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for op in ops {
        let (horizontal, pos, delta, kind) = match op {
            PsOp::HStem { y, dy, kind } => (true, *y, *dy, *kind),
            PsOp::VStem { x, dx, kind } => (false, *x, *dx, *kind),
            _ => continue,
        };
        if kind == StemKind::Counter {
            continue;
        }
        let (kind, place) = if horizontal && delta == -20.0 {
            ("TopGhost", (pos, 0.0))
        } else if horizontal && delta == -21.0 {
            ("BottomGhost", (pos + delta, 0.0))
        } else if delta < 0.0 {
            ("Stem", (pos + delta, -delta))
        } else {
            ("Stem", (pos, delta))
        };
        if out.len() < fontcraft_model::MAX_LAYER_HINTS && seen.insert((horizontal, kind, place.0.to_bits(), place.1.to_bits())) {
            out.push(hint(horizontal, kind, place));
        }
    }
    out
}

fn draw(ops: &[PsOp]) -> Vec<fontcraft_model::Path> {
    let mut pen = CubicPen::default();
    let f = |v: f64| v as f32;
    for op in ops {
        match op {
            PsOp::Move(x, y) => pen.move_to(f(*x), f(*y)),
            PsOp::Line(x, y) => pen.line_to(f(*x), f(*y)),
            PsOp::Curve(c) => pen.curve_to(f(c[0]), f(c[1]), f(c[2]), f(c[3]), f(c[4]), f(c[5])),
            PsOp::Flex(c, _) => {
                pen.curve_to(f(c[0]), f(c[1]), f(c[2]), f(c[3]), f(c[4]), f(c[5]));
                pen.curve_to(f(c[6]), f(c[7]), f(c[8]), f(c[9]), f(c[10]), f(c[11]));
            }
            _ => {}
        }
    }
    pen.finish()
}

/// Reads a [`PsFont`] into a one-master document (see the module notes).
pub fn to_font(ps: &PsFont, mut warnings: Vec<String>) -> Result<Decompiled, DecompileError> {
    // Names, made unique where the font repeats one.
    let mut unique = UniqueNames::default();
    let mut names = Vec::with_capacity(ps.glyphs.len());
    let mut by_ps_name: HashMap<&str, String> = HashMap::new();
    for (i, g) in ps.glyphs.iter().enumerate() {
        let base = Some(clean_name(&g.name)).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("glyph{i:05}"));
        let name = unique.add(base);
        by_ps_name.entry(g.name.as_str()).or_insert_with(|| name.clone());
        names.push(name);
    }

    let data = fontcraft_model::glyphdata::GlyphData::new().ok();
    if data.is_none() {
        warnings.push("the glyph database could not be loaded: no code points were assigned".into());
    }
    let mut used = HashSet::new();
    let mut seac_missing = 0usize;
    let mut glyphs = Vec::with_capacity(ps.glyphs.len());
    for (g, name) in ps.glyphs.iter().zip(&names) {
        let mut layer = Layer::new(MASTER, if g.width.abs() <= MAX_COORDINATE { g.width } else { 0.0 });
        layer.shapes.extend(draw(&g.ops).into_iter().map(Shape::Path));
        for op in &g.ops {
            if let PsOp::Seac { adx, ady, base, accent } = op {
                let base = by_ps_name.get(PredefinedEncoding::Standard.name(*base));
                let accent = by_ps_name.get(PredefinedEncoding::Standard.name(*accent));
                let (Some(base), Some(accent)) = (base, accent) else {
                    seac_missing += 1;
                    continue;
                };
                for (glyph, dx, dy) in [(base, 0.0, 0.0), (accent, *adx, *ady)] {
                    layer.shapes.push(Shape::Component(Component {
                        glyph: glyph.clone(),
                        transform: Transform::Matrix([1.0, 0.0, 0.0, 1.0, dx, dy]),
                        info: Default::default(),
                        extra: Default::default(),
                    }));
                }
            }
        }
        layer.shapes.retain(|s| !matches!(s, Shape::Path(p) if p.to_bez().is_err()));
        let hints = layer_hints(&g.ops);
        if !hints.is_empty() {
            layer.info.hints = hints;
        }
        let mut glyph = Glyph::new(name.clone());
        if let Some(data) = &data
            && !name.contains('.')
        {
            let text = data.name_to_text(name);
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && used.insert(u32::from(c))
            {
                glyph.unicodes = vec![u32::from(c)];
            }
        }
        glyph.layers.push(layer);
        glyphs.push(glyph);
    }
    if seac_missing > 0 {
        warnings.push(format!("{seac_missing} accented glyphs name a base or accent the font lacks; they were left empty"));
    }

    let top = &ps.top;
    let units_per_em = ps.units_per_em();
    if top.font_matrix.is_some() && units_per_em == 1000 && top.font_matrix != Some([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]) {
        warnings.push("the font matrix is not a plain scale: outlines were read untransformed at 1000 units per em".into());
    }
    let family_name = [&top.family_name, &top.full_name].into_iter().flatten().next().cloned().unwrap_or_else(|| ps.font_name.clone());
    let style = top.weight.clone().filter(|w| !w.trim().is_empty()).unwrap_or_else(|| "Regular".into());

    let (defs, values, zones) = metrics(ps, &glyphs);
    let mut master = Master { id: MASTER.into(), name: clean_name(&style), metric_values: values, zones, ..Default::default() };
    let p = &ps.private;
    let mut stems = Vec::new();
    for (horizontal, snaps, std) in [(true, &p.stem_snap_h, p.std_hw), (false, &p.stem_snap_v, p.std_vw)] {
        let widths: Vec<f64> = if snaps.is_empty() { std.into_iter().collect() } else { snaps.clone() };
        for (i, w) in widths.into_iter().enumerate() {
            stems.push(StemDef { name: format!("{}Stem{i}", if horizontal { 'h' } else { 'v' }), horizontal, ..Default::default() });
            master.stem_values.push(w);
        }
    }
    for (key, v) in [("blueScale", p.blue_scale), ("blueShift", p.blue_shift), ("blueFuzz", p.blue_fuzz)] {
        if let Some(v) = v {
            master.params.set(key, number(v));
        }
    }
    if !p.family_blues.is_empty() || !p.family_other_blues.is_empty() {
        warnings.push("FamilyBlues and FamilyOtherBlues were not read".into());
    }

    let mut info = FontInfo::default();
    if let Some(c) = top.copyright.as_ref().or(top.notice.as_ref()) {
        info.properties.set("copyrights", c, true);
    }
    let version = top.version.as_deref().and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(1.0);

    // The glyph order, as an opened binary records it.
    let mut extra = fontcraft_model::Extra::new();
    let mut param = Dict::new();
    param.insert("name", Value::String("glyphOrder".into()));
    param.insert(
        "value",
        Value::Array(Array {
            items: names.iter().map(|n| Value::String(Str::new(n.as_str()))).collect(),
            layout: Layout::Multiline,
            ..Default::default()
        }),
    );
    extra.push(
        Family::Glyphs,
        "customParameters",
        Value::Array(Array { items: vec![Value::Dict(param)], layout: Layout::Multiline, ..Default::default() }),
    );

    let font = Font {
        source: Source { format: SourceFormat::Binary, ..Default::default() },
        family_name: clean_name(&family_name),
        units_per_em,
        version_major: version.trunc().min(f64::from(u16::MAX)) as u16,
        version_minor: (version.fract() * 1000.0).round().clamp(0.0, f64::from(u16::MAX)) as u16,
        axes: Arc::default(),
        metrics: Arc::new(defs),
        stems: Arc::new(stems),
        numbers: Arc::default(),
        masters: Arc::new(vec![master]),
        instances: Arc::default(),
        glyphs: GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect()),
        info: Arc::new(info),
        kerning: Arc::default(),
        features: Arc::default(),
        groups: Arc::default(),
        display_strings: Arc::default(),
        note: String::new(),
        files: Arc::default(),
        extra: Arc::new(extra),
    };
    font.check()?;
    Ok(Decompiled { font, warnings })
}

/// The highest or lowest on-curve or off-curve point of a glyph's paths.
fn extreme(glyphs: &[Glyph], names: &[&str], top: bool) -> Option<f64> {
    names.iter().find_map(|name| {
        let g = glyphs.iter().find(|g| g.name == *name)?;
        let ys = g
            .layers
            .first()?
            .shapes
            .iter()
            .filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None })
            .flat_map(|p| p.nodes.iter().map(|n| n.y));
        if top { ys.reduce(f64::max) } else { ys.reduce(f64::min) }
    })
}

/// Vertical metrics measured on the usual glyphs, with the Private dict's zones as overshoots
/// where a zone starts at a metric, and as extra zones otherwise.
fn metrics(ps: &PsFont, glyphs: &[Glyph]) -> (Vec<MetricDef>, Vec<MetricValue>, Vec<AlignmentZone>) {
    let [_, ymin, _, ymax] = ps.top.font_bbox;
    let ascender = extreme(glyphs, &["d", "l", "h"], true).unwrap_or(ymax);
    let cap = extreme(glyphs, &["H", "I"], true).unwrap_or(0.0);
    let x_height = extreme(glyphs, &["x", "z"], true).unwrap_or(0.0);
    let descender = extreme(glyphs, &["p", "q"], false).unwrap_or(ymin);
    let mut defs = vec![
        MetricDef::new(metric::ASCENDER),
        MetricDef::new(metric::CAP_HEIGHT),
        MetricDef::new(metric::X_HEIGHT),
        MetricDef::new(metric::BASELINE),
        MetricDef::new(metric::DESCENDER),
    ];
    let mut values: Vec<MetricValue> = [ascender, cap, x_height, 0.0, descender].map(|pos| MetricValue { pos, over: 0.0 }).to_vec();
    let mut zones = Vec::new();
    let p = &ps.private;
    for (i, &[lo, hi]) in p.blue_values.as_chunks::<2>().0.iter().enumerate() {
        if i == 0 && lo <= 0.0 && hi >= 0.0 {
            if let Some(v) = values.get_mut(3) {
                v.over = lo;
            }
            continue;
        }
        let slot = values.iter_mut().take(3).find(|v| v.pos == lo && v.over == 0.0);
        match slot {
            Some(v) => v.over = hi - lo,
            None => zones.push(AlignmentZone { pos: lo, size: hi - lo }),
        }
    }
    for &[lo, hi] in p.other_blues.as_chunks::<2>().0 {
        match values.get_mut(4).filter(|v| v.pos == hi && v.over == 0.0) {
            Some(v) => v.over = lo - hi,
            None => zones.push(AlignmentZone { pos: hi, size: lo - hi }),
        }
    }
    if ps.top.italic_angle != 0.0 {
        defs.push(MetricDef::new(metric::ITALIC_ANGLE));
        values.push(MetricValue { pos: ps.top.italic_angle, over: 0.0 });
    }
    (defs, values, zones)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_division_past_the_float_range_pushes_zero() {
        let max = [255, 0x7F, 0xFF, 0xFF, 0xFF];
        // 2147483647, then 1 divided by 2147483647 thirty-four times: a subnormal quotient.
        let mut cs = [max.as_slice(), &[140]].concat();
        for _ in 0..34 {
            cs.extend(max);
            cs.extend([12, 12]);
        }
        // 2147483647 / that is infinite; then `0 rmoveto` uses it as x.
        cs.extend([12, 12, 139, 21, 14]);
        let mut run = T1Run::new(&[]);
        run.run(&cs, 0).unwrap();
        assert_eq!(run.ops, vec![PsOp::Move(0.0, 0.0)]);
    }

    #[test]
    fn repeated_long_names_stay_unique_and_within_the_name_limit() {
        let long = "a".repeat(fontcraft_model::MAX_NAME_LEN);
        // Two long names that differ only in their last byte share one numbering.
        let other = format!("{}b", "a".repeat(fontcraft_model::MAX_NAME_LEN - 1));
        let mut glyphs = Vec::new();
        for name in [long.as_str(), other.as_str(), "a"] {
            glyphs.extend((0..3_000).map(|_| PsGlyph { name: name.to_string(), ..Default::default() }));
        }
        let ps = PsFont { glyphs, ..Default::default() };
        let font = to_font(&ps, Vec::new()).unwrap().font;
        assert_eq!(font.glyphs.len(), 9_000);
        let names: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names.len(), 9_000);
        assert!(names.iter().all(|n| n.len() <= fontcraft_model::MAX_NAME_LEN));
        assert!(names.contains("a.2999"));
    }
}
