//! A writer for the `CFF2` table of a variable OpenType font (M9.13).
//!
//! Rows: `cff2.export`, the `CFF2` table row, `compile.cff2VariableBuild`.
//!
//! [`write_cff2`] turns a [`Cff2Font`] (the variation regions, the region lists each glyph's
//! blends use, and per glyph an outline whose numbers are a default plus one delta per region of
//! its list) into the bytes of a `CFF2` table:
//!
//! - **Charstrings**: one generic operator per segment (`rmoveto`, `rlineto`, `rrcurveto`), each
//!   varying argument written with `blend`, and `vsindex` first where a glyph uses a region list
//!   other than the first. Consecutive varying arguments share one `blend` while the operand stack
//!   ([`MAX_STACK`]) has room; otherwise the run is split. No hints, no subroutines, no widths.
//! - **Top DICT**: FontMatrix when given, CharStrings, FDArray and `vstore`. One Font DICT, so no
//!   FDSelect.
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
use crate::charstring::{bounds, pen_commands};
use crate::number::{encode_dict_int, encode_dict_number, encode_t2_number, round_with_tolerance};
use crate::{Error, MAX_COORDINATE, MAX_GLYPHS, MAX_SEGMENTS, MAX_TABLE_SIZE, Number, PathOp};

/// Most regions the variation store may hold (a 16-bit count).
pub const MAX_REGIONS: usize = 65_535;
/// Most region lists (`vsindex` values) the variation store may hold (a 16-bit count).
pub const MAX_REGION_LISTS: usize = 65_535;
/// Most axes a region may have (`fvar` holds at most this many in practice).
pub const MAX_AXES: usize = 64;
/// Most operands the charstring interpreter's stack holds (CFF2's `maxstack` default): a `blend`
/// of `n` values with `k` regions needs `n * (k + 1) + 1`.
pub const MAX_STACK: usize = 513;
/// Most values one Private DICT list (BlueValues and the like) may have.
pub const MAX_PRIVATE_LIST: usize = 14;

/// Charstring operators.
const OP_RLINETO: u8 = 5;
const OP_RRCURVETO: u8 = 8;
const OP_VSINDEX: u8 = 15;
const OP_BLEND: u8 = 16;
const OP_RMOVETO: u8 = 21;
/// DICT operators.
const DICT_BLEND: u8 = 23;

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
}

/// A written `CFF2` table and what the other tables need from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cff2Built {
    pub table: Vec<u8>,
    /// Per glyph, the bounds of its outline at the origin; `None` for an empty glyph.
    pub glyph_bounds: Vec<Option<Bounds>>,
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
}

impl fmt::Display for Cff2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cff2Error::Cff(e) => write!(f, "{e}"),
            Cff2Error::BadBlend { glyph, reason } => write!(f, "glyph `{glyph}`: {reason}"),
            Cff2Error::TooManyRegions(n) => write!(f, "{n} variation regions is more than a CFF2 table can hold ({MAX_REGIONS})"),
            Cff2Error::StackOverflow { glyph } => write!(f, "glyph `{glyph}` varies more values at once than a CFF2 charstring can hold"),
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
enum Arg {
    Plain(f64),
    Blend(f64, Vec<f64>),
}

/// How numbers are encoded: in a charstring or in a DICT.
#[derive(Clone, Copy, PartialEq)]
enum Context {
    Charstring,
    Dict,
}

/// Appends `args` with `blend` operators, keeping the stack within [`MAX_STACK`]. `regions` is
/// the number of deltas each blended argument has.
fn write_args(args: &[Arg], regions: usize, ctx: Context, what: &str, out: &mut Vec<u8>) -> Result<(), Cff2Error> {
    let number = |v: f64, out: &mut Vec<u8>| -> Result<(), Cff2Error> {
        match ctx {
            Context::Charstring => encode_t2_number(v, what, out)?,
            Context::Dict if v.fract() == 0.0 && v.abs() < 2_147_483_648.0 => encode_dict_int(v as i64, what, out)?,
            Context::Dict => encode_dict_number(Number::Real(v), what, out)?,
        }
        Ok(())
    };
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
                    if need > MAX_STACK {
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
                out.push(if ctx == Context::Charstring { OP_BLEND } else { DICT_BLEND });
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

/// The charstring of `glyph` (with `regions` regions in its list) and its bounds at the origin.
fn charstring(glyph: &Cff2Glyph, regions: usize, tolerance: f64) -> Result<(Vec<u8>, Option<Bounds>), Cff2Error> {
    let name = glyph.name.as_str();
    if glyph.path.len() > MAX_SEGMENTS {
        return Err(Error::TooManySegments { glyph: name.to_string(), count: glyph.path.len() }.into());
    }
    let mut default_path: Vec<PathOp> = Vec::with_capacity(glyph.path.len());
    let mut last = (0.0f64, vec![0.0f64; regions], 0.0f64, vec![0.0f64; regions]);
    let mut commands: Vec<(u8, Vec<Arg>)> = Vec::with_capacity(glyph.path.len());
    let mut any_blend = false;
    let rel = |b: &Blend, last_default: &mut f64, last_deltas: &mut Vec<f64>, args: &mut Vec<Arg>, any: &mut bool| {
        let d = round_with_tolerance(b.default, tolerance);
        let rd = d - *last_default;
        *last_default = d;
        let deltas: Vec<f64> =
            if b.deltas.is_empty() { vec![0.0; regions] } else { b.deltas.iter().map(|v| round_with_tolerance(*v, tolerance)).collect() };
        let rel_deltas: Vec<f64> = deltas.iter().zip(last_deltas.iter()).map(|(v, p)| round_with_tolerance(v - p, tolerance)).collect();
        *last_deltas = deltas;
        if rel_deltas.iter().any(|v| *v != 0.0) {
            *any = true;
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
            let dx = rel(x, &mut last.0, &mut last.1, &mut args, &mut any_blend);
            let dy = rel(y, &mut last.2, &mut last.3, &mut args, &mut any_blend);
            defaults.push((dx, dy));
        }
        let (code, step) = match (op, defaults.as_slice()) {
            (BlendOp::MoveTo(_), [p]) => (OP_RMOVETO, PathOp::MoveTo(*p)),
            (BlendOp::LineTo(_), [p]) => (OP_RLINETO, PathOp::LineTo(*p)),
            (BlendOp::CurveTo(..), [a, b, c]) => (OP_RRCURVETO, PathOp::CurveTo(*a, *b, *c)),
            _ => return Err(Cff2Error::BadBlend { glyph: name.to_string(), reason: "a drawing step has the wrong number of points".into() }),
        };
        default_path.push(step);
        commands.push((code, args));
    }
    let mut out = Vec::new();
    if any_blend && glyph.vsindex != 0 {
        write_args(&[Arg::Plain(f64::from(glyph.vsindex))], 0, Context::Charstring, name, &mut out)?;
        out.push(OP_VSINDEX);
    }
    for (code, args) in &commands {
        write_args(args, regions, Context::Charstring, name, &mut out)?;
        out.push(*code);
    }
    // The defaults are rounded already; the bounds pen sees the points the charstring draws.
    let drawn = bounds(&pen_commands(&default_path, 0.0));
    Ok((out, drawn))
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
        if list.len() > MAX_REGIONS || list.iter().any(|&r| usize::from(r) >= font.regions.len()) {
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
    for g in &font.glyphs {
        let Some(list) = font.region_lists.get(usize::from(g.vsindex)) else {
            return Err(Cff2Error::BadBlend { glyph: g.name.clone(), reason: format!("region list {} does not exist", g.vsindex) });
        };
        let (bytes, b) = charstring(g, list.len(), font.round_tolerance)?;
        charstrings.push(bytes);
        glyph_bounds.push(b);
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
    let top_len = matrix.len() + 3 * 5 + 1 + 2 + 1;
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
    Ok(Cff2Built { table: out, glyph_bounds })
}

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
    }

    #[test]
    fn deltas_that_do_not_match_the_region_list_are_refused() {
        let glyph = Cff2Glyph {
            name: "a".into(),
            vsindex: 0,
            path: vec![BlendOp::MoveTo((Blend { default: 0.0, deltas: vec![1.0, 2.0] }, Blend::fixed(0.0)))],
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
