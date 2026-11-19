//! Type 1 (PFA, PFB) and bare CFF inputs of the converter (M12.30b; rows `convert`,
//! `font.convert`). Oracle: AFDKO `tx` 5.0.1 (`-cff`, `-cff2`); the sfnt around the outlines
//! follows AFDKO makeotf's metric rules.
//!
//! [`read_postscript`] reads the font program with `fontcraft-decompile`'s readers and wraps it
//! in a CFF-flavoured sfnt, which [`super::convert_with`] then converts like any other font
//! ([`convert_postscript`]).
//!
//! | Input | `CFF ` table | Glyph order |
//! |---|---|---|
//! | Type 1 | written from the charstrings as `tx -cff` writes it: coordinates kept to two decimals, every segment kept, stems and hint replacement as hint masks, `hstem3`/`vstem3` as stems in one counter mask, flex as flex, `seac` as `seac`; widths by the width optimiser | `.notdef`, then the glyphs whose names are CFF standard strings in standard-string order, then the others in file order (as tx orders them) |
//! | bare CFF | the table as it is (one font) | the charset's |
//!
//! The other tables are made from the font program: `head` (revision from the version string,
//! bounds from the drawn glyphs, seac glyphs included), `hhea` and `OS/2` by makeotf's rules
//! (weight and width classes from the full name, typographic ascender from `d`, `H`, `O`, the
//! font box), `hmtx` (advances rounded, left side bearing at `xMin`), `maxp` 0.5, `name`
//! (Windows English records), `cmap` (code points from the glyph names, as makeotf assigns
//! them), `post` format 3.
//!
//! Input is untrusted. The readers cap the file size, subroutine nesting, the stack and the
//! work per glyph; the operations kept for the whole font are capped here so that they fit in
//! [`MAX_CHARSTRING_MEMORY`].

use std::collections::{BTreeSet, HashMap, HashSet};

use fontcraft_cff::{BuildOptions, CharstringHints, Mask, Number, PathOp, StemHint, Widths};
use fontcraft_decompile::bare_cff::{is_bare_cff, read_cff_with_ops};
use fontcraft_decompile::type1::{PrivateDict, PsFont, PsGlyph, PsOp, StemKind, TopDict, is_type1, read_type1_within};
use fontcraft_geom::{CubicBez, ParamCurveExtrema as _, Point};
use read_fonts::ps::encoding::PredefinedEncoding;
use read_fonts::ps::string::STANDARD_STRINGS;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::types::{GlyphId, NameId, Tag as WTag};

use super::cff_table::{SeacCall, with_seac};
use super::{ConvertOptions, Converted, convert_with};
use crate::limits::MAX_CHARSTRING_MEMORY;
use crate::sfnt::{Flavour, SfntFont};
use crate::{Result, TablesError};

/// Most hint and path operations kept for one font program: what fits in the charstring memory
/// limit of a converted table.
const MAX_PS_OPS: usize = MAX_CHARSTRING_MEMORY / std::mem::size_of::<PsOp>();
/// Most glyphs (a CFF CharStrings INDEX count is 16-bit).
const MAX_PS_GLYPHS: usize = 65_535;
/// Decimals tx keeps of a Type 1 coordinate.
const DECIMALS: f64 = 100.0;

/// What kind of font program a file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsSource {
    /// A PFA or PFB font.
    Type1,
    /// A `CFF ` table on its own.
    BareCff,
}

impl PsSource {
    /// The kind of font program `bytes` start like, if any.
    pub fn sniff(bytes: &[u8]) -> Option<PsSource> {
        if is_type1(bytes) {
            Some(PsSource::Type1)
        } else if is_bare_cff(bytes) {
            Some(PsSource::BareCff)
        } else {
            None
        }
    }
}

/// A font program read and wrapped in an sfnt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsInput {
    pub source: PsSource,
    /// A CFF-flavoured font with the program as its `CFF ` table.
    pub font: SfntFont,
    /// What was not carried over or had to be changed, one line each.
    pub notes: Vec<String>,
}

/// Converts a font program read by [`read_postscript`]. Without an explicit
/// [`ConvertOptions::subroutinize`], sfnt results are subroutinised and bare tables are written
/// without subroutines, as `tx -cff` writes them.
pub fn convert_postscript(input: &PsInput, options: &ConvertOptions) -> Result<Converted> {
    let mut o = options.clone();
    if o.subroutinize.is_none() {
        o.subroutinize = Some(!o.target.bare());
    }
    let mut converted = convert_with(&input.font, &o)?;
    let mut notes = input.notes.clone();
    notes.append(&mut converted.left_behind);
    converted.left_behind = notes;
    Ok(converted)
}

fn unreadable(e: fontcraft_decompile::DecompileError) -> TablesError {
    TablesError::invalid(e.to_string())
}

/// Reads a PFA, PFB or bare CFF font program into a CFF-flavoured sfnt.
pub fn read_postscript(bytes: &[u8]) -> Result<PsInput> {
    read_postscript_with(bytes, &PsReadOptions::default())
}

/// How [`read_postscript_with`] lays out the glyphs of a Type 1 font; a bare CFF table keeps its
/// own. The defaults are the reference converter's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsReadOptions {
    /// Keep the file's glyph order, `.notdef` moved first (`tx +b`); otherwise `.notdef`, the
    /// standard-string names in standard-string order, then the rest in file order.
    pub keep_glyph_order: bool,
    /// Add no empty `.notdef` to a font without one (`tx -amnd`).
    pub allow_missing_notdef: bool,
}

/// [`read_postscript`] with the glyph order and `.notdef` switches of `options`.
pub fn read_postscript_with(bytes: &[u8], options: &PsReadOptions) -> Result<PsInput> {
    let source = PsSource::sniff(bytes).ok_or_else(|| TablesError::invalid("the file is neither a Type 1 font nor a bare CFF table"))?;
    let mut notes = Vec::new();
    let ps = match source {
        PsSource::Type1 => read_type1_within(bytes, &mut notes, MAX_PS_OPS).map_err(unreadable)?,
        PsSource::BareCff => {
            let fonts = bytes
                .get(2)
                .and_then(|&h| bytes.get(usize::from(h)..usize::from(h) + 2))
                .and_then(|b| <[u8; 2]>::try_from(b).ok())
                .map_or(0, u16::from_be_bytes);
            if fonts > 1 {
                return Err(TablesError::Unsupported(format!("a CFF font set of {fonts} fonts: convert holds one font")));
            }
            read_cff_with_ops(bytes, &mut notes, MAX_PS_OPS).map_err(unreadable)?
        }
    };
    if ps.glyphs.len() >= MAX_PS_GLYPHS {
        return Err(TablesError::TooLarge { what: "the glyph count of the font program", limit: MAX_PS_GLYPHS - 1 });
    }
    let (table, glyphs) = match source {
        PsSource::Type1 => type1_table(&ps, options, &mut notes)?,
        PsSource::BareCff => (bytes.to_vec(), bare_glyphs(&ps)),
    };
    let font = wrap(&ps, table, &glyphs)?;
    Ok(PsInput { source, font, notes })
}

// ── Glyphs ──────────────────────────────────────────────────────────────────────────────────────

/// What the sfnt tables need of a glyph, in the table's glyph order.
struct GlyphInfo {
    name: String,
    advance: u16,
    /// The integer box of the drawn glyph, rounded as makeotf rounds it; `None` draws nothing.
    bounds: Option<[i64; 4]>,
}

fn advance(width: f64) -> u16 {
    if width.is_finite() { (width + 0.5).floor().clamp(0.0, 65_535.0) as u16 } else { 0 }
}

/// The drawn bounds of a path: end points, and the extremes of each curve.
fn path_bounds(path: &[PathOp]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |x: f64, y: f64| {
        b = Some(match b {
            None => [x, y, x, y],
            Some([a, c, d, e]) => [a.min(x), c.min(y), d.max(x), e.max(y)],
        });
    };
    let mut current = (0.0, 0.0);
    for op in path {
        match *op {
            PathOp::MoveTo(p) | PathOp::LineTo(p) => {
                add(p.0, p.1);
                current = p;
            }
            PathOp::CurveTo(c1, c2, p) => {
                let r = CubicBez::new(Point::new(current.0, current.1), Point::new(c1.0, c1.1), Point::new(c2.0, c2.1), Point::new(p.0, p.1))
                    .bounding_box();
                add(r.x0, r.y0);
                add(r.x1, r.y1);
                current = p;
            }
        }
    }
    b.filter(|v| v.iter().all(|x| x.is_finite()))
}

fn int_box(b: [f64; 4]) -> [i64; 4] {
    fontcraft_compile::makeotf::metrics::int_box(&fontcraft_cff::Bounds { x_min: b[0], y_min: b[1], x_max: b[2], y_max: b[3] })
}

fn union(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Option<[f64; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]),
        (a, b) => a.or(b),
    }
}

/// A Type 1 coordinate as tx writes it: two decimals.
fn r2(v: f64) -> f64 {
    (v * DECIMALS).round() / DECIMALS
}

/// A `seac` call: the accent's offset, then the base and accent character codes.
type SeacArgs = (f64, f64, u8, u8);

/// The outline of a glyph's operations, and the seac call if it ends with one.
fn draw(ops: &[PsOp]) -> (Vec<PathOp>, Option<SeacArgs>) {
    let mut path = Vec::new();
    let mut seac = None;
    for op in ops {
        match *op {
            PsOp::Move(x, y) => path.push(PathOp::MoveTo((r2(x), r2(y)))),
            PsOp::Line(x, y) => path.push(PathOp::LineTo((r2(x), r2(y)))),
            PsOp::Curve(c) => path.push(PathOp::CurveTo((r2(c[0]), r2(c[1])), (r2(c[2]), r2(c[3])), (r2(c[4]), r2(c[5])))),
            PsOp::Flex(c, _) => {
                path.push(PathOp::CurveTo((r2(c[0]), r2(c[1])), (r2(c[2]), r2(c[3])), (r2(c[4]), r2(c[5]))));
                path.push(PathOp::CurveTo((r2(c[6]), r2(c[7])), (r2(c[8]), r2(c[9])), (r2(c[10]), r2(c[11]))));
            }
            PsOp::Seac { adx, ady, base, accent } => seac = Some((r2(adx), r2(ady), base, accent)),
            PsOp::HStem { .. } | PsOp::VStem { .. } | PsOp::NewHints | PsOp::NewGroup => {}
        }
    }
    (path, seac)
}

/// One set of active stems, from path operation `op` on.
struct Group {
    op: usize,
    h: Vec<usize>,
    v: Vec<usize>,
}

/// The index of `stem` in `stems`, added when it is new.
fn stem_index(stems: &mut Vec<StemHint>, stem: StemHint) -> usize {
    stems.iter().position(|s| *s == stem).unwrap_or_else(|| {
        stems.push(stem);
        stems.len() - 1
    })
}

/// A Type 1 glyph's hints as charstring hints: the distinct stems, a hint mask per replacement
/// set when there is more than one set, the `hstem3`/`vstem3` stems as one counter group (and
/// counter stems by their groups), flex on the curves a flex draws.
fn hints(ops: &[PsOp]) -> Option<CharstringHints> {
    let mut h = Vec::new();
    let mut v = Vec::new();
    let mut groups = vec![Group { op: 0, h: Vec::new(), v: Vec::new() }];
    let mut stem3 = (Vec::new(), Vec::new());
    let mut counters: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut flex = Vec::new();
    let mut n = 0usize;
    for op in ops {
        match *op {
            PsOp::HStem { y, dy, kind } | PsOp::VStem { x: y, dx: dy, kind } => {
                let horizontal = matches!(op, PsOp::HStem { .. });
                let list = if horizontal { &mut h } else { &mut v };
                let i = stem_index(list, StemHint { edge: r2(y), width: r2(dy) });
                let pick = |pair: &mut (Vec<usize>, Vec<usize>)| if horizontal { pair.0.push(i) } else { pair.1.push(i) };
                match kind {
                    StemKind::Counter => {
                        if counters.is_empty() {
                            counters.push((Vec::new(), Vec::new()));
                        }
                        if let Some(c) = counters.last_mut() {
                            pick(c);
                        }
                    }
                    StemKind::Stem | StemKind::Stem3 => {
                        if kind == StemKind::Stem3 {
                            pick(&mut stem3);
                        }
                        if let Some(g) = groups.last_mut() {
                            if horizontal { g.h.push(i) } else { g.v.push(i) }
                        }
                    }
                }
            }
            PsOp::NewHints => {
                if groups.last().is_some_and(|g| g.op == n) {
                    groups.pop();
                }
                groups.push(Group { op: n, h: Vec::new(), v: Vec::new() });
            }
            PsOp::NewGroup => counters.push((Vec::new(), Vec::new())),
            PsOp::Move(..) | PsOp::Line(..) | PsOp::Curve(_) => n += 1,
            PsOp::Flex(..) => {
                flex.push(n);
                n += 2;
            }
            PsOp::Seac { .. } => {}
        }
    }
    if h.is_empty() && v.is_empty() && flex.is_empty() {
        return None;
    }
    let (nh, nv) = (h.len(), v.len());
    let mask = |hs: &[usize], vs: &[usize]| Mask { h: (0..nh).map(|i| hs.contains(&i)).collect(), v: (0..nv).map(|i| vs.contains(&i)).collect() };
    let masks = if groups.len() > 1 { groups.iter().map(|g| (g.op, mask(&g.h, &g.v))).collect() } else { Vec::new() };
    let mut counter_masks = Vec::new();
    if !stem3.0.is_empty() || !stem3.1.is_empty() {
        counter_masks.push(mask(&stem3.0, &stem3.1));
    }
    counter_masks.extend(counters.iter().filter(|c| !c.0.is_empty() || !c.1.is_empty()).map(|c| mask(&c.0, &c.1)));
    Some(CharstringHints { h_stems: h, v_stems: v, masks, counters: counter_masks, flex })
}

/// The glyph order tx writes a Type 1 font in: `.notdef`, the standard-string names in
/// standard-string order, then the rest in file order.
fn type1_order(ps: &PsFont) -> Vec<usize> {
    let sids: HashMap<&str, usize> = STANDARD_STRINGS.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut order: Vec<(usize, usize, usize)> =
        ps.glyphs.iter().enumerate().map(|(i, g)| sids.get(g.name.as_str()).map_or((1, 0, i), |sid| (0, *sid, i))).collect();
    order.sort_unstable();
    order.into_iter().map(|(_, _, i)| i).collect()
}

fn number(v: f64) -> Number {
    if v.fract() == 0.0 && v.abs() < 2_147_483_648.0 { Number::Int(v as i64) } else { Number::Real(v) }
}

fn cff_top(top: &TopDict) -> fontcraft_cff::TopDict {
    let defaults = fontcraft_cff::TopDict::default();
    fontcraft_cff::TopDict {
        version: top.version.clone(),
        notice: top.notice.clone(),
        copyright: top.copyright.clone(),
        full_name: top.full_name.clone(),
        family_name: top.family_name.clone(),
        weight: top.weight.clone(),
        is_fixed_pitch: Number::Int(i64::from(top.is_fixed_pitch)),
        italic_angle: number(top.italic_angle),
        underline_position: number(top.underline_position),
        underline_thickness: number(top.underline_thickness),
        paint_type: Number::Int(top.paint_type),
        font_matrix: top.font_matrix.map_or(defaults.font_matrix, |m| m.map(number)),
        stroke_width: number(top.stroke_width),
    }
}

fn cff_private(p: &PrivateDict) -> fontcraft_cff::PrivateDict {
    let defaults = fontcraft_cff::PrivateDict::default();
    let list = |v: &[f64]| v.iter().copied().map(number).collect::<Vec<_>>();
    fontcraft_cff::PrivateDict {
        blue_values: list(&p.blue_values),
        other_blues: list(&p.other_blues),
        family_blues: list(&p.family_blues),
        family_other_blues: list(&p.family_other_blues),
        blue_scale: p.blue_scale.map_or(defaults.blue_scale, number),
        blue_shift: p.blue_shift.map_or(defaults.blue_shift, number),
        blue_fuzz: p.blue_fuzz.map_or(defaults.blue_fuzz, number),
        std_hw: p.std_hw.map(number),
        std_vw: p.std_vw.map(number),
        stem_snap_h: list(&p.stem_snap_h),
        stem_snap_v: list(&p.stem_snap_v),
        force_bold: Number::Int(i64::from(p.force_bold.unwrap_or(false))),
        language_group: p.language_group.map_or(defaults.language_group, Number::Int),
    }
}

/// The `CFF ` table tx writes for a Type 1 font, and its glyphs.
fn type1_table(ps: &PsFont, options: &PsReadOptions, notes: &mut Vec<String>) -> Result<(Vec<u8>, Vec<GlyphInfo>)> {
    let order = if options.keep_glyph_order {
        let notdef = ps.glyphs.iter().position(|g| g.name == ".notdef");
        notdef.into_iter().chain((0..ps.glyphs.len()).filter(|i| Some(*i) != notdef)).collect()
    } else {
        type1_order(ps)
    };
    let has_notdef = ps.glyphs.iter().any(|g| g.name == ".notdef");
    // Glyph 0 of a CFF table is .notdef whatever it draws: without an added one (`-amnd`) the
    // first glyph takes that place and loses its own name.
    let first = order.first().and_then(|&i| ps.glyphs.get(i));
    let zero = match first {
        Some(g) if !has_notdef && options.allow_missing_notdef => {
            notes.push(format!("the font has no .notdef glyph: its first glyph, {}, is glyph 0 and is named .notdef", g.name));
            PsGlyph { name: ".notdef".into(), ..g.clone() }
        }
        _ => {
            if !has_notdef {
                notes.push("the font has no .notdef glyph: an empty one was added".into());
            }
            PsGlyph { name: ".notdef".into(), ..PsGlyph::default() }
        }
    };
    let glyphs: Vec<&PsGlyph> = if has_notdef {
        order.iter().filter_map(|&i| ps.glyphs.get(i)).collect()
    } else if options.allow_missing_notdef {
        std::iter::once(&zero).chain(order.iter().skip(1).filter_map(|&i| ps.glyphs.get(i))).collect()
    } else {
        std::iter::once(&zero).chain(order.iter().filter_map(|&i| ps.glyphs.get(i))).collect()
    };
    let by_name: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();

    let mut drawn = Vec::with_capacity(glyphs.len());
    let mut cff_glyphs = Vec::with_capacity(glyphs.len());
    let mut seac_calls = Vec::new();
    let (mut dropped_hints, mut missing_parts) = (0usize, 0usize);
    for (gid, g) in glyphs.iter().enumerate() {
        let (path, seac) = draw(&g.ops);
        let mut glyph_hints = if seac.is_some() { None } else { hints(&g.ops) };
        if let Some(h) = &glyph_hints {
            let writable =
                fontcraft_cff::writable_stems(&h.h_stems, 0.0).into_iter().chain(fontcraft_cff::writable_stems(&h.v_stems, 0.0)).all(|ok| ok);
            if !writable || h.validate(&g.name, &path).is_err() {
                glyph_hints = None;
                dropped_hints += 1;
            }
        }
        let width = advance(g.width);
        if let Some((adx, ady, base, accent)) = seac {
            let part = |code: u8| by_name.get(PredefinedEncoding::Standard.name(code)).copied();
            match (part(base), part(accent)) {
                (Some(b), Some(a)) => seac_calls.push((gid, b, a, SeacCall { width: i64::from(width), adx, ady, base, accent })),
                _ => missing_parts += 1,
            }
        }
        drawn.push(path_bounds(&path));
        cff_glyphs.push(fontcraft_cff::Glyph { name: g.name.clone(), advance: f64::from(width), path, hints: glyph_hints });
    }
    if dropped_hints > 0 {
        notes.push(format!(
            "the hints of {dropped_hints} glyphs, which the CFF format cannot hold as they are (overlapping stems in one set, or out of range)"
        ));
    }
    if missing_parts > 0 {
        notes.push(format!("{missing_parts} accented glyphs name a base or accent the font lacks; they were left empty"));
    }
    let name = Some(ps.font_name.clone()).filter(|n| !n.is_empty()).or_else(|| ps.top.full_name.clone()).unwrap_or_else(|| "Untitled".into());
    let font = fontcraft_cff::Font {
        name,
        top: cff_top(&ps.top),
        private: cff_private(&ps.private),
        glyphs: cff_glyphs,
        widths: Widths::Optimise,
        round_tolerance: 0.0,
        specialise: true,
    };
    let built = fontcraft_cff::build_with(&font, BuildOptions { keep_topology: true })
        .map_err(|e| TablesError::Unsupported(format!("the CFF table cannot be written: {e}")))?;
    // A seac glyph's box is its base's and its moved accent's.
    for (gid, base, accent, call) in &seac_calls {
        let moved = drawn.get(*accent).copied().flatten().map(|b| [b[0] + call.adx, b[1] + call.ady, b[2] + call.adx, b[3] + call.ady]);
        let b = union(drawn.get(*base).copied().flatten(), moved);
        if let Some(slot) = drawn.get_mut(*gid) {
            *slot = b;
        }
    }
    let calls: Vec<(usize, SeacCall)> = seac_calls.iter().map(|(gid, _, _, call)| (*gid, *call)).collect();
    let table = with_seac(&built.table, &calls, built.default_width_x, built.nominal_width_x)?;
    let info = font
        .glyphs
        .iter()
        .zip(&drawn)
        .map(|(g, b)| GlyphInfo { name: g.name.clone(), advance: advance(g.advance), bounds: b.map(int_box) })
        .collect();
    Ok((table, info))
}

/// The glyphs of a bare CFF font, in its own order.
fn bare_glyphs(ps: &PsFont) -> Vec<GlyphInfo> {
    let by_name: HashMap<&str, usize> = ps.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let drawn: Vec<(Option<[f64; 4]>, Option<SeacArgs>)> = ps
        .glyphs
        .iter()
        .map(|g| {
            let (path, seac) = draw(&g.ops);
            (path_bounds(&path), seac)
        })
        .collect();
    ps.glyphs
        .iter()
        .zip(&drawn)
        .map(|(g, (own, seac))| {
            let mut b = *own;
            if let Some((adx, ady, base, accent)) = seac {
                let part = |code: u8| by_name.get(PredefinedEncoding::Standard.name(code)).and_then(|&i| drawn.get(i)).and_then(|d| d.0);
                let moved = part(*accent).map(|a| [a[0] + adx, a[1] + ady, a[2] + adx, a[3] + ady]);
                b = union(union(b, part(*base)), moved);
            }
            GlyphInfo { name: g.name.clone(), advance: advance(g.width), bounds: b.map(int_box) }
        })
        .collect()
}

// ── The sfnt around the program ─────────────────────────────────────────────────────────────────

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn i16_of(v: i64) -> i16 {
    v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

fn i16_f(v: f64) -> i16 {
    if v.is_finite() { (v + 0.5).floor().clamp(-32_768.0, 32_767.0) as i16 } else { 0 }
}

fn dump<T: write_fonts::FontWrite + write_fonts::validate::Validate>(tag: &str, table: &T) -> Result<Vec<u8>> {
    write_fonts::dump_table(table).map_err(|e| TablesError::invalid(format!("the {tag} table cannot be written: {e}")))
}

/// `font_revision` from a Type 1 version string such as `001.002`.
fn revision(version: Option<&str>) -> f64 {
    version.and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && (0.0..32_768.0).contains(v)).unwrap_or(1.0)
}

/// The sfnt: `CFF ` plus the tables a font needs, made from the font program.
fn wrap(ps: &PsFont, cff: Vec<u8>, glyphs: &[GlyphInfo]) -> Result<SfntFont> {
    let n =
        u16::try_from(glyphs.len()).map_err(|_| TablesError::TooLarge { what: "the glyph count of the font program", limit: MAX_PS_GLYPHS - 1 })?;
    let upem = ps.units_per_em();
    let upm = i64::from(upem);
    let top = &ps.top;

    // Code points from the names, the first glyph of each kept.
    let mut used = HashSet::new();
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();
    for (gid, g) in glyphs.iter().enumerate() {
        if let Some(c) = fontcraft_compile::makeotf::name_code_point(&g.name).and_then(char::from_u32)
            && used.insert(c)
        {
            mappings.push((c, GlyphId::new(gid as u32)));
        }
    }
    let box_of = |c: u32| -> Option<[i64; 4]> {
        let gid = mappings.iter().find(|(m, _)| u32::from(*m) == c)?.1.to_u32() as usize;
        Some(glyphs.get(gid).and_then(|g| g.bounds).unwrap_or([0; 4]))
    };
    let font_box = glyphs
        .iter()
        .filter_map(|g| g.bounds)
        .filter(|b| *b != [0; 4])
        .fold(None, |acc: Option<[i64; 4]>, b| Some(acc.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])));
    let fb = font_box.unwrap_or([0; 4]);

    // makeotf: heights of o and O less their overshoot, ascender from d and the cap height.
    let height = |c: u32| box_of(c).map_or(0, |b| b[3] + b[1]);
    let (x_height, cap_height) = (height(0x6F), height(0x4F));
    let mut ascender = box_of(0x64).map_or(0, |b| b[3]).max(cap_height);
    if ascender == 0 {
        ascender = fb[3].abs();
    }
    ascender = ascender.min(upm);
    let descender = ascender - upm;
    let line_gap = upm * 1200 / 1000 - ascender + descender;

    let (weight_class, width_class) = fontcraft_compile::makeotf::metrics::classes(top.full_name.as_deref());
    let bold = weight_class >= 700;
    let italic = top.italic_angle != 0.0;

    let mut head = Vec::with_capacity(54);
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    head.extend_from_slice(&((revision(top.version.as_deref()) * 65_536.0).round() as i32).to_be_bytes());
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    be16(&mut head, 0x0003);
    be16(&mut head, upem);
    head.extend_from_slice(&[0u8; 16]);
    for v in fb {
        head.extend_from_slice(&i16_of(v).to_be_bytes());
    }
    be16(&mut head, u16::from(bold) | (u16::from(italic) << 1));
    be16(&mut head, 3);
    head.extend_from_slice(&2i16.to_be_bytes());
    head.extend_from_slice(&[0, 0, 0, 0]);

    let mut hmtx = Vec::with_capacity(glyphs.len() * 4);
    let (mut adv_max, mut min_lsb, mut min_rsb, mut max_extent) = (0u16, i64::MAX, i64::MAX, i64::MIN);
    for g in glyphs {
        let lsb = g.bounds.map_or(0, |b| b[0]);
        be16(&mut hmtx, g.advance);
        hmtx.extend_from_slice(&i16_of(lsb).to_be_bytes());
        adv_max = adv_max.max(g.advance);
        if let Some(b) = g.bounds {
            min_lsb = min_lsb.min(b[0]);
            min_rsb = min_rsb.min(i64::from(g.advance) - b[2]);
            max_extent = max_extent.max(b[2]);
        }
    }
    let none = |v: i64, extreme: i64| if v == extreme { 0 } else { v };
    let (rise, run) = if italic { (upm, i64::from(i16_f(-(top.italic_angle.to_radians().tan()) * upm as f64))) } else { (1, 0) };
    let mut hhea = Vec::with_capacity(36);
    hhea.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [ascender, descender, line_gap] {
        hhea.extend_from_slice(&i16_of(v).to_be_bytes());
    }
    be16(&mut hhea, adv_max);
    for v in [none(min_lsb, i64::MAX), none(min_rsb, i64::MAX), none(max_extent, i64::MIN), rise, run, 0, 0, 0, 0, 0, 0] {
        hhea.extend_from_slice(&i16_of(v).to_be_bytes());
    }
    be16(&mut hhea, n);

    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    be16(&mut maxp, n);

    let code_points: BTreeSet<u32> = mappings.iter().map(|(c, _)| u32::from(*c)).collect();
    let nonzero: Vec<i64> = glyphs.iter().map(|g| i64::from(g.advance)).filter(|a| *a != 0).collect();
    let avg = if nonzero.is_empty() { 0 } else { (nonzero.iter().sum::<i64>() + nonzero.len() as i64 / 2) / nonzero.len() as i64 };
    let em = |v: i64| i16_of(upm * v / 1000);
    let mut selection = SelectionFlags::empty();
    if bold {
        selection |= SelectionFlags::BOLD;
    }
    if italic {
        selection |= SelectionFlags::ITALIC;
    }
    if !bold && !italic && weight_class == 400 {
        selection |= SelectionFlags::REGULAR;
    }
    let ranges = fontcraft_compile::ranges::words::<4>(&fontcraft_compile::ranges::unicode_range_bits(&code_points));
    let pages = fontcraft_compile::ranges::words::<2>(&fontcraft_compile::ranges::code_page_bits(&code_points));
    let os2 = Os2 {
        x_avg_char_width: i16_of(avg),
        us_weight_class: weight_class,
        us_width_class: width_class,
        fs_type: top.fs_type.and_then(|t| u16::try_from(t).ok()).unwrap_or(4),
        y_subscript_x_size: em(650),
        y_subscript_y_size: em(600),
        y_subscript_y_offset: em(75),
        y_superscript_x_size: em(650),
        y_superscript_y_size: em(600),
        y_superscript_y_offset: em(350),
        y_strikeout_size: i16_f(top.underline_thickness),
        y_strikeout_position: if x_height > 0 { i16_of(x_height / 2) } else { em(250) },
        ul_unicode_range_1: ranges[0],
        ul_unicode_range_2: ranges[1],
        ul_unicode_range_3: ranges[2],
        ul_unicode_range_4: ranges[3],
        ach_vend_id: WTag::new(b"NONE"),
        fs_selection: selection,
        us_first_char_index: code_points.first().map_or(0, |c| (*c).min(0xFFFF) as u16),
        us_last_char_index: code_points.last().map_or(0, |c| (*c).min(0xFFFF) as u16),
        s_typo_ascender: i16_of(ascender),
        s_typo_descender: i16_of(descender),
        s_typo_line_gap: i16_of(line_gap),
        us_win_ascent: fb[3].clamp(0, 65_535) as u16,
        us_win_descent: (-fb[1]).clamp(0, 65_535) as u16,
        ul_code_page_range_1: Some(pages[0]),
        ul_code_page_range_2: Some(pages[1]),
        sx_height: Some(i16_of(x_height)),
        s_cap_height: Some(i16_of(cap_height)),
        us_default_char: Some(0),
        us_break_char: Some(32),
        us_max_context: Some(0),
        ..Default::default()
    };

    let ps_name = Some(ps.font_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled".into());
    let family = top.family_name.clone().unwrap_or_else(|| ps_name.clone());
    let style = match (bold, italic) {
        (true, true) => "Bold Italic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (false, false) => "Regular",
    };
    let version = fontcraft_compile::makeotf::metrics::version_string(revision(top.version.as_deref()));
    let mut records: Vec<(u16, String)> = Vec::new();
    if let Some(c) = top.copyright.clone().or_else(|| top.notice.clone()) {
        records.push((0, c));
    }
    records.push((1, family));
    records.push((2, style.into()));
    records.push((3, format!("{};{ps_name}", version.trim_start_matches("Version "))));
    records.push((4, top.full_name.clone().unwrap_or_else(|| ps_name.clone())));
    records.push((5, version));
    records.push((6, ps_name));
    let name = Name::new(records.into_iter().map(|(id, text)| NameRecord::new(3, 1, 0x409, NameId::new(id), text.into())).collect());

    let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
    post.extend_from_slice(&((top.italic_angle * 65_536.0).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32).to_be_bytes());
    // makeotf: post holds the top of the underline; the Type 1 value is its centre.
    post.extend_from_slice(&i16_f(top.underline_position + top.underline_thickness / 2.0).to_be_bytes());
    post.extend_from_slice(&i16_f(top.underline_thickness).to_be_bytes());
    post.extend_from_slice(&u32::from(top.is_fixed_pitch).to_be_bytes());
    post.extend_from_slice(&[0u8; 16]);

    let cmap = Cmap::from_mappings(mappings).map_err(|e| TablesError::invalid(format!("the cmap table cannot be written: {e}")))?;
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", cff);
    font.insert(*b"head", head);
    font.insert(*b"hhea", hhea);
    font.insert(*b"hmtx", hmtx);
    font.insert(*b"maxp", maxp);
    font.insert(*b"OS/2", dump("OS/2", &os2)?);
    font.insert(*b"name", dump("name", &name)?);
    font.insert(*b"cmap", dump("cmap", &cmap)?);
    font.insert(*b"post", post);
    Ok(font)
}

#[cfg(test)]
#[path = "postscript_tests.rs"]
mod tests;
