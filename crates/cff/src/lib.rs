//! A writer for the Compact Font Format table (`CFF `, version 1) of a static OpenType font
//! (L0, standalone).
//!
//! [`build`] turns a [`Font`] (names, Top DICT and Private DICT values, and per glyph a name, an
//! advance width and cubic outlines) into the bytes of a `CFF ` table. It writes what the
//! reference compiler (fontmake through ufo2ft and fontTools, at `--optimize-cff 1`) writes for the
//! same input, byte for byte:
//!
//! - **Charstrings** (Type 2): coordinates rounded with the reference's tolerance rule, then
//!   specialised into the short operators (`hlineto`, `hvcurveto`, `rcurveline` …) within the
//!   48-operand stack ([`specialize`] is a port of fontTools' specialiser). No subroutines:
//!   [`subr::subroutinize_table`] adds them to a built table.
//! - **Hints**: a glyph's stem hints, hint masks, counter masks and flex ([`hints`]); a glyph
//!   without them gets an unhinted charstring.
//! - **Widths**: `defaultWidthX` and `nominalWidthX` chosen by the reference's method unless the
//!   caller fixes them; a charstring stores its width only when it differs from the default.
//! - **FontBBox**: the union of the glyphs' drawn bounds (curve extremes included), rounded
//!   outwards.
//! - **Charset**: a predefined charset when the glyph order is a prefix of one, otherwise the
//!   smaller of format 0 and format 1/2.
//! - **Layout**: header, Name INDEX, Top DICT INDEX, String INDEX, an empty Global Subr INDEX,
//!   charset, CharStrings INDEX, Private DICT. Offsets in the Top DICT are found by repeating the
//!   layout until no position changes.
//!
//! CID-keyed fonts (ROS, FDArray, FDSelect, a charset of CIDs) are [`cid`]'s. Out of scope:
//! custom encodings (the standard encoding is implied), and the seldom-used Top
//! DICT entries (UniqueID, XUID, PostScript, BaseFontName, SyntheticBase). The `CFF2` table of a
//! variable font, with blends, is [`cff2`]'s. The subroutiniser, which works on any `CFF ` or
//! `CFF2` table, is [`subr`]'s.
//!
//! Every input is untrusted: counts, lengths and numbers are checked against the limits below, and
//! anything out of range returns an [`Error`]; nothing panics.
#![forbid(unsafe_code)]

mod bounds;
pub mod cff2;
mod charset;
mod charstring;
pub mod cid;
mod dict;
mod error;
pub mod hints;
mod index;
mod number;
mod specialize;
mod strings;
pub mod subr;
mod width;

use std::collections::HashSet;

pub use bounds::Bounds;
pub use error::Error;
pub use hints::{BOTTOM_GHOST, CharstringHints, MAX_COUNTER_MASKS, MAX_STEM_HINTS, Mask, StemHint, TOP_GHOST, writable_stems};
pub use number::Number;

use charset::Charset;
use dict::{Pointers, TopSids};
use number::ot_round;
use strings::Strings;

/// Most glyphs a CFF table can hold (its CharStrings INDEX count is 16-bit).
pub const MAX_GLYPHS: usize = 65_535;
/// Most path segments one glyph may have.
pub const MAX_SEGMENTS: usize = 65_536;
/// Longest string (font name, glyph name, Top DICT string), in bytes.
pub const MAX_STRING_LEN: usize = 65_535;
/// Largest absolute coordinate accepted. Charstring numbers must also fit in 16 bits after
/// differencing; [`build`] reports the ones that do not.
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// Largest absolute advance width accepted.
pub const MAX_ADVANCE: f64 = 65_535.0;
/// Largest table [`build`] writes, in bytes.
pub const MAX_TABLE_SIZE: usize = 1 << 30;
/// Layout passes before giving up (the reference settles in two or three).
const MAX_LAYOUT_PASSES: usize = 64;

/// A point in font units.
pub type Point = (f64, f64);

/// One drawing step of a glyph outline, in absolute font units, as a segment pen receives it.
///
/// A contour starts with `MoveTo` and is closed implicitly by a straight line back to its start:
/// do not repeat that closing line (the reference leaves it out), but do give a closing curve.
/// Quadratic outlines must be converted to cubics first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathOp {
    MoveTo(Point),
    LineTo(Point),
    CurveTo(Point, Point, Point),
}

/// A glyph: its name (the charset entry), advance width and outline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glyph {
    pub name: String,
    pub advance: f64,
    pub path: Vec<PathOp>,
    /// Stem hints, masks and flex; `None` writes an unhinted charstring.
    pub hints: Option<CharstringHints>,
}

/// Top DICT values. `Default` gives the CFF defaults, which are left out of the table; a string
/// that is `Some`, even empty, is written.
#[derive(Clone, Debug, PartialEq)]
pub struct TopDict {
    /// ASCII only.
    pub version: Option<String>,
    /// Latin-1.
    pub notice: Option<String>,
    /// Latin-1.
    pub copyright: Option<String>,
    /// Latin-1.
    pub full_name: Option<String>,
    /// Latin-1.
    pub family_name: Option<String>,
    /// ASCII only.
    pub weight: Option<String>,
    pub is_fixed_pitch: Number,
    pub italic_angle: Number,
    pub underline_position: Number,
    pub underline_thickness: Number,
    pub paint_type: Number,
    pub font_matrix: [Number; 6],
    pub stroke_width: Number,
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
            is_fixed_pitch: Number::Int(0),
            italic_angle: Number::Int(0),
            underline_position: Number::Int(-100),
            underline_thickness: Number::Int(50),
            paint_type: Number::Int(0),
            font_matrix: dict::DEFAULT_MATRIX,
            stroke_width: Number::Int(0),
        }
    }
}

/// Private DICT values. `Default` gives the CFF defaults; empty arrays and `None` are left out.
/// `defaultWidthX` and `nominalWidthX` come from [`Font::widths`].
#[derive(Clone, Debug, PartialEq)]
pub struct PrivateDict {
    pub blue_values: Vec<Number>,
    pub other_blues: Vec<Number>,
    pub family_blues: Vec<Number>,
    pub family_other_blues: Vec<Number>,
    pub blue_scale: Number,
    pub blue_shift: Number,
    pub blue_fuzz: Number,
    pub std_hw: Option<Number>,
    pub std_vw: Option<Number>,
    pub stem_snap_h: Vec<Number>,
    pub stem_snap_v: Vec<Number>,
    pub force_bold: Number,
    pub language_group: Number,
}

impl Default for PrivateDict {
    fn default() -> Self {
        Self {
            blue_values: Vec::new(),
            other_blues: Vec::new(),
            family_blues: Vec::new(),
            family_other_blues: Vec::new(),
            blue_scale: Number::Real(0.039625),
            blue_shift: Number::Int(7),
            blue_fuzz: Number::Int(1),
            std_hw: None,
            std_vw: None,
            stem_snap_h: Vec::new(),
            stem_snap_v: Vec::new(),
            force_bold: Number::Int(0),
            language_group: Number::Int(0),
        }
    }
}

/// How `defaultWidthX` and `nominalWidthX` are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Widths {
    /// The pair that makes the charstrings smallest, by the reference's method.
    #[default]
    Optimise,
    /// Given values (from a source that sets them).
    Fixed { default_width_x: i64, nominal_width_x: i64 },
}

/// A static, name-keyed CFF font.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// The PostScript font name (the Name INDEX entry). Latin-1.
    pub name: String,
    pub top: TopDict,
    pub private: PrivateDict,
    /// In glyph order; the first must be `.notdef`. Names are unique and Latin-1.
    pub glyphs: Vec<Glyph>,
    pub widths: Widths,
    /// Coordinates within this distance of an integer are rounded to it: 0 keeps every fraction,
    /// 0.5 (the reference's default) rounds everything.
    pub round_tolerance: f64,
    /// Specialise operators (the reference's `--optimize-cff 1` and above). When false, every
    /// segment is a generic `rmoveto`, `rlineto` or `rrcurveto` of its own.
    pub specialise: bool,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            name: String::new(),
            top: TopDict::default(),
            private: PrivateDict::default(),
            glyphs: Vec::new(),
            widths: Widths::Optimise,
            round_tolerance: 0.5,
            specialise: true,
        }
    }
}

/// A built table and the values derived on the way, which other tables need to agree with.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    /// The `CFF ` table.
    pub table: Vec<u8>,
    /// The Top DICT FontBBox (`[0, 0, 0, 0]` when no glyph draws anything).
    pub font_bbox: [i64; 4],
    pub default_width_x: i64,
    pub nominal_width_x: i64,
    /// Each glyph's drawn bounds, in glyph order; `None` for a glyph that draws nothing. Round
    /// with [`Bounds::round_with_tolerance`] for `head` and `hmtx` as the reference does.
    pub glyph_bounds: Vec<Option<Bounds>>,
}

/// Choices of [`build_with`] beyond the font itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// Keep every segment the outline draws when specialising: a curve whose handles lie on its
    /// ends, a line of no length and a line that continues the one before in the same
    /// direction stay, as AFDKO's `tx` writes them (fontTools' `preserveTopology`).
    pub keep_topology: bool,
}

/// Build the `CFF ` table for `font`.
pub fn build(font: &Font) -> Result<Built, Error> {
    build_with(font, BuildOptions::default())
}

/// [`build`] with `options`.
pub fn build_with(font: &Font, options: BuildOptions) -> Result<Built, Error> {
    validate(font)?;

    let (default_width_x, nominal_width_x) = match font.widths {
        Widths::Fixed { default_width_x, nominal_width_x } => (default_width_x, nominal_width_x),
        Widths::Optimise => {
            let widths: Vec<i64> = font.glyphs.iter().map(|g| ot_round(g.advance) as i64).collect();
            width::optimize(&widths).ok_or(Error::NoGlyphs)?
        }
    };

    let mut charstrings = Vec::with_capacity(font.glyphs.len());
    let mut glyph_bounds = Vec::with_capacity(font.glyphs.len());
    let mut font_box: Option<Bounds> = None;
    for g in &font.glyphs {
        let width = if g.advance == default_width_x as f64 { None } else { Some(ot_round(g.advance - nominal_width_x as f64)) };
        let built = charstring::build(&g.name, &g.path, g.hints.as_ref(), width, font.round_tolerance, font.specialise, options.keep_topology)?;
        if let Some(b) = built.bounds {
            font_box = Some(font_box.map_or(b, |f| f.union(b)));
        }
        glyph_bounds.push(built.bounds);
        charstrings.push(built.bytes);
    }
    let font_bbox = font_box.map_or([0; 4], Bounds::round_out);

    // Glyph names take their string ids first, then the Top DICT strings in entry order.
    let mut strings = Strings::new();
    let names: Vec<Vec<u8>> = font.glyphs.iter().map(|g| latin1(&g.name)).collect();
    let name_refs: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
    let charset = charset::build(&name_refs, &mut strings)?;
    let mut sid = |s: &Option<String>| s.as_ref().map(|s| strings.sid(&latin1(s))).transpose();
    let sids = TopSids {
        version: sid(&font.top.version)?,
        notice: sid(&font.top.notice)?,
        copyright: sid(&font.top.copyright)?,
        full_name: sid(&font.top.full_name)?,
        family_name: sid(&font.top.family_name)?,
        weight: sid(&font.top.weight)?,
    };

    let private = dict::private_dict(&font.private, default_width_x, nominal_width_x)?;
    let name_index = {
        let mut out = Vec::new();
        index::write_index(&[latin1(&font.name)], &mut out)?;
        out
    };
    let string_index = {
        let mut out = Vec::new();
        index::write_index(strings.custom(), &mut out)?;
        out
    };
    let global_subrs: [u8; 2] = [0, 0];
    let charset_bytes: &[u8] = match &charset {
        Charset::Custom(bytes) => bytes,
        Charset::Predefined(_) => &[],
    };
    let charstrings_len = index::index_len(charstrings.iter().map(Vec::len));

    // Repeat the layout until no position moves: the Top DICT holds offsets in their shortest
    // form, so its size depends on them. The first pass has no offsets at all, as the reference's.
    let mut at = Pointers {
        charset: match charset {
            Charset::Predefined(id) => Some(i64::from(id)),
            Charset::Custom(_) => None,
        },
        ..Pointers::default()
    };
    let mut last_positions: Option<[usize; 4]> = None;
    let mut settled: Option<(Vec<u8>, usize)> = None;
    for _ in 0..MAX_LAYOUT_PASSES {
        let top = dict::top_dict(&font.top, &sids, font_bbox, &at)?;
        let top_index_len = index::index_len(std::iter::once(top.len()));
        let charset_at = 4 + name_index.len() + top_index_len + string_index.len() + global_subrs.len();
        let charstrings_at = charset_at + charset_bytes.len();
        let private_at = charstrings_at + charstrings_len;
        let end = private_at + private.len();
        let positions = [charset_at, charstrings_at, private_at, end];
        if end > MAX_TABLE_SIZE {
            return Err(Error::TableTooLarge(end));
        }
        if last_positions == Some(positions) {
            settled = Some((top, end));
            break;
        }
        last_positions = Some(positions);
        if matches!(charset, Charset::Custom(_)) {
            at.charset = Some(charset_at as i64);
        }
        at.charstrings = Some(charstrings_at as i64);
        at.private = Some((private.len() as i64, private_at as i64));
    }
    let (top, end) = settled.ok_or(Error::LayoutDidNotSettle)?;

    let mut table = Vec::with_capacity(end);
    table.extend_from_slice(&[1, 0, 4, index::off_size(end)]);
    table.extend_from_slice(&name_index);
    index::write_index(&[top], &mut table)?;
    table.extend_from_slice(&string_index);
    table.extend_from_slice(&global_subrs);
    table.extend_from_slice(charset_bytes);
    index::write_index(&charstrings, &mut table)?;
    table.extend_from_slice(&private);
    if table.len() != end {
        return Err(Error::LayoutDidNotSettle);
    }
    Ok(Built { table, font_bbox, default_width_x, nominal_width_x, glyph_bounds })
}

/// Latin-1 bytes of a string already checked by [`validate`].
fn latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect()
}

fn check_string(s: &str, what: &str, ascii_only: bool, allow_empty: bool) -> Result<(), Error> {
    let len = s.chars().count();
    if len > MAX_STRING_LEN || (len == 0 && !allow_empty) {
        return Err(Error::BadStringLength { what: what.to_string(), len });
    }
    let limit = if ascii_only { 0x7F } else { 0xFF };
    if let Some(c) = s.chars().find(|&c| u32::from(c) > limit) {
        return Err(Error::BadCharacter { what: what.to_string(), character: c });
    }
    Ok(())
}

fn check_number(n: Number, what: &str) -> Result<(), Error> {
    match n {
        Number::Real(r) if !r.is_finite() => Err(Error::NotFinite { what: what.to_string() }),
        Number::Int(i) if i32::try_from(i).is_err() => Err(Error::OutOfRange { what: what.to_string(), value: i as f64 }),
        _ => Ok(()),
    }
}

fn check_point(p: Point, glyph: &str) -> Result<(), Error> {
    for v in [p.0, p.1] {
        if !v.is_finite() {
            return Err(Error::NotFinite { what: format!("glyph `{glyph}`") });
        }
        if v.abs() > MAX_COORDINATE {
            return Err(Error::OutOfRange { what: format!("glyph `{glyph}`"), value: v });
        }
    }
    Ok(())
}

fn validate(font: &Font) -> Result<(), Error> {
    let first = font.glyphs.first().ok_or(Error::NoGlyphs)?;
    if font.glyphs.len() > MAX_GLYPHS {
        return Err(Error::TooManyGlyphs(font.glyphs.len()));
    }
    if first.name != ".notdef" {
        return Err(Error::FirstGlyphNotNotdef(first.name.clone()));
    }
    if !font.round_tolerance.is_finite() || font.round_tolerance < 0.0 {
        return Err(Error::BadRoundTolerance(font.round_tolerance));
    }
    for g in &font.glyphs {
        if let Some(h) = &g.hints {
            h.validate(&g.name, &g.path)?;
            // Overlaps that only rounding creates show in the hints as written.
            h.normalised(font.round_tolerance).validate(&g.name, &g.path)?;
        }
    }
    check_string(&font.name, "font name", false, false)?;
    let t = &font.top;
    for (s, what, ascii) in [
        (&t.version, "version", true),
        (&t.notice, "Notice", false),
        (&t.copyright, "Copyright", false),
        (&t.full_name, "FullName", false),
        (&t.family_name, "FamilyName", false),
        (&t.weight, "Weight", true),
    ] {
        if let Some(s) = s {
            check_string(s, what, ascii, true)?;
        }
    }
    let p = &font.private;
    let numbers = [t.is_fixed_pitch, t.italic_angle, t.underline_position, t.underline_thickness, t.paint_type, t.stroke_width]
        .into_iter()
        .chain(t.font_matrix)
        .chain([p.blue_scale, p.blue_shift, p.blue_fuzz, p.force_bold, p.language_group])
        .chain(p.std_hw)
        .chain(p.std_vw)
        .chain(p.blue_values.iter().copied())
        .chain(p.other_blues.iter().copied())
        .chain(p.family_blues.iter().copied())
        .chain(p.family_other_blues.iter().copied())
        .chain(p.stem_snap_h.iter().copied())
        .chain(p.stem_snap_v.iter().copied());
    for n in numbers {
        check_number(n, "DICT value")?;
    }
    if let Widths::Fixed { default_width_x, nominal_width_x } = font.widths {
        for (v, what) in [(default_width_x, "defaultWidthX"), (nominal_width_x, "nominalWidthX")] {
            if (v as f64).abs() > MAX_ADVANCE {
                return Err(Error::OutOfRange { what: what.to_string(), value: v as f64 });
            }
        }
    }
    let mut seen: HashSet<&str> = HashSet::with_capacity(font.glyphs.len());
    for g in &font.glyphs {
        check_string(&g.name, "glyph name", false, false)?;
        if !seen.insert(g.name.as_str()) {
            return Err(Error::DuplicateGlyphName(g.name.clone()));
        }
        if !g.advance.is_finite() {
            return Err(Error::NotFinite { what: format!("advance of `{}`", g.name) });
        }
        if g.advance.abs() > MAX_ADVANCE {
            return Err(Error::OutOfRange { what: format!("advance of `{}`", g.name), value: g.advance });
        }
        if g.path.len() > MAX_SEGMENTS {
            return Err(Error::TooManySegments { glyph: g.name.clone(), count: g.path.len() });
        }
        for op in &g.path {
            match *op {
                PathOp::MoveTo(a) | PathOp::LineTo(a) => check_point(a, &g.name)?,
                PathOp::CurveTo(a, b, c) => {
                    check_point(a, &g.name)?;
                    check_point(b, &g.name)?;
                    check_point(c, &g.name)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
