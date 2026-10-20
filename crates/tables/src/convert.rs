//! Format conversion of compiled fonts (M12.30; rows `convert`, `font.convert`). Oracles: AFDKO
//! otf2ttf and `tx`, fontTools `cffLib.CFFToCFF2` and `CFF2ToCFF`, `ttx`. Type 1 and bare CFF
//! reading come from `fontcraft-decompile` (M12.31).
//!
//! | From → to | Outlines | Other tables |
//! |---|---|---|
//! | CFF or static CFF2 → TrueType | charstrings drawn in font units as stated (fractions kept, no FontMatrix), cubic to quadratic within `max_error` units (fontTools' cu2qu), contours reversed; a `seac` glyph becomes a composite of its base and accent; hints dropped | `CFF `/`CFF2` and `VORG` removed; `glyf`, `loca`, `maxp` 1.0 and `post` (format 2 with names, or 3) built; `hmtx` left side bearings set to `xMin`; `head` and `hhea` bounds recalculated, as AFDKO otf2ttf does |
//! | TrueType → CFF or CFF2 | each glyph moved so that its `xMin` sits at its `hmtx` left side bearing, quadratic to cubic ([`QuadToCubic`]), contours reversed, components decomposed; instructions dropped | `glyf`, `loca`, `cvt `, `fpgm`, `prep`, `gasp`, `hdmx`, `LTSH`, `VDMX` removed; `maxp` 0.5; `post` format 3 (CFF) or 2 (CFF2) |
//! | CFF → CFF2 | charstrings unchanged but for the width, `endchar` and `dotsection`, and `seac` glyphs drawn out; hints kept | `post` gets the glyph names (format 2) unless format 3 is asked for |
//! | static CFF2 → CFF | the `hmtx` advance as the width; hints kept | `post` format 3, the names in the charset |
//! | → bare CFF or CFF2 | as above | only the outline table is written |
//!
//! The CFF writers lay the table out without subroutines; [`ConvertOptions::subroutinize`]
//! runs the subroutiniser over the result. Variable fonts (`gvar`, a blending `CFF2`) are
//! refused: instantiate them first. A conversion that fails returns an error and no bytes.
//!
//! AFDKO's otf2ttf (Apache-2.0, Copyright 2014 Adobe) and fontTools' `cffLib` converters (MIT)
//! were read and their rules written anew here.

mod cff_table;
mod outline;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use read_fonts::tables::name::Name;
use read_fonts::tables::post::{DEFAULT_GLYPH_NAMES, Post};
use read_fonts::types::GlyphId16;
use read_fonts::{FontData, FontRead};

use crate::sfnt::{Flavour, SfntFont, Tag, WriteOptions};
use crate::{Result, TablesError};
pub use outline::QuadToCubic;
use outline::{Contour, GlyfOutlines, PsOutlines};

/// Largest error bound accepted, in font units.
pub const MAX_ERROR_UNITS: f64 = 1_000.0;
/// The TrueType tables a CFF-flavoured font cannot use.
const TRUETYPE_ONLY: [Tag; 9] = [*b"glyf", *b"loca", *b"cvt ", *b"fpgm", *b"prep", *b"gasp", *b"hdmx", *b"LTSH", *b"VDMX"];

/// What [`convert`] writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConvertTarget {
    /// TrueType outlines, cubic curves converted within `max_error` font units.
    TrueType { max_error: f64 },
    /// `CFF ` outlines.
    Cff,
    /// `CFF2` outlines.
    Cff2,
    /// A bare CFF table, outside any sfnt.
    BareCff,
    /// A bare CFF2 table, outside any sfnt.
    BareCff2,
}

impl ConvertTarget {
    fn outlines(self) -> Outlines {
        match self {
            ConvertTarget::TrueType { .. } => Outlines::TrueType,
            ConvertTarget::Cff | ConvertTarget::BareCff => Outlines::Cff,
            ConvertTarget::Cff2 | ConvertTarget::BareCff2 => Outlines::Cff2,
        }
    }

    fn bare(self) -> bool {
        matches!(self, ConvertTarget::BareCff | ConvertTarget::BareCff2)
    }
}

/// `post` table formats a converted font can get.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostFormat {
    /// Format 2: glyph names (TrueType and CFF2 results).
    #[default]
    Names,
    /// Format 3: no glyph names.
    NoNames,
}

/// What [`convert_with`] does.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvertOptions {
    pub target: ConvertTarget,
    /// Reverse contours when the curve degree changes (cubic outlines run counter-clockwise,
    /// quadratic ones clockwise).
    pub reverse_direction: bool,
    /// TrueType to CFF: how quadratic curves become cubic.
    pub quad_to_cubic: QuadToCubic,
    /// TrueType to CFF with [`QuadToCubic::Optimized`]: the error bound, in font units.
    pub cubic_error: f64,
    /// The `post` format of TrueType and CFF2 results.
    pub post_format: PostFormat,
    /// Subroutinise CFF and CFF2 results; `None` subroutinises sfnt results and not bare ones,
    /// and leaves a font that keeps its flavour as it was.
    pub subroutinize: Option<bool>,
    /// Most subroutines per INDEX; `None` is the subroutiniser's default.
    pub max_subrs: Option<usize>,
    /// Keep stem hints when CFF outlines stay CFF; `false` removes them.
    pub hints: bool,
    /// `head.modified`, in seconds since 1904; `None` keeps it.
    pub modified: Option<i64>,
}

impl ConvertOptions {
    /// The defaults of the reference tools: contours reversed, merged cubic curves within one
    /// unit, glyph names kept, hints kept.
    pub fn new(target: ConvertTarget) -> Self {
        Self {
            target,
            reverse_direction: true,
            quad_to_cubic: QuadToCubic::Optimized,
            cubic_error: 1.0,
            post_format: PostFormat::Names,
            subroutinize: None,
            max_subrs: None,
            hints: true,
            modified: None,
        }
    }
}

/// What [`convert_with`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Converted {
    /// The file's bytes: an sfnt font, or a bare CFF or CFF2 table.
    pub bytes: Vec<u8>,
    /// The converted font; `None` for a bare table.
    pub font: Option<SfntFont>,
    pub glyphs: usize,
    /// Data of the input the target format could not hold, one line each.
    pub left_behind: Vec<String>,
}

/// The converted file's bytes: an sfnt font, or a bare CFF.
pub fn convert(font: &SfntFont, target: ConvertTarget) -> Result<Vec<u8>> {
    convert_with(font, &ConvertOptions::new(target)).map(|c| c.bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outlines {
    TrueType,
    Cff,
    Cff2,
}

const CFF: Tag = *b"CFF ";
const CFF2: Tag = *b"CFF2";

/// Converts `font` as `options` say.
pub fn convert_with(font: &SfntFont, options: &ConvertOptions) -> Result<Converted> {
    check_options(options)?;
    let from = if font.get(*b"glyf").is_some() {
        Outlines::TrueType
    } else if font.get(CFF).is_some() {
        Outlines::Cff
    } else if font.get(CFF2).is_some() {
        Outlines::Cff2
    } else {
        return Err(TablesError::MissingTable("glyf, CFF or CFF2".into()));
    };
    let to = options.target.outlines();
    if from == Outlines::TrueType && font.get(*b"gvar").is_some() && to != Outlines::TrueType {
        return Err(TablesError::Unsupported("converting a variable TrueType font: instantiate it first".into()));
    }
    if from == Outlines::Cff2 && font.get(*b"fvar").is_some() && to != Outlines::Cff2 {
        return Err(TablesError::Unsupported("converting a variable CFF2 font: instantiate it first".into()));
    }
    let mut left_behind = Vec::new();
    let mut work = font.clone();
    if !options.hints && from != Outlines::TrueType && to != Outlines::TrueType {
        let (dehinted, _) = crate::dehint::dehint_with_report(&work, &crate::dehint::DehintOptions::default())?;
        work = dehinted;
    }
    let mut changed_outlines = true;
    match (from, to) {
        (Outlines::TrueType, Outlines::TrueType) | (Outlines::Cff, Outlines::Cff) | (Outlines::Cff2, Outlines::Cff2) => changed_outlines = false,
        (Outlines::Cff | Outlines::Cff2, Outlines::TrueType) => {
            let ConvertTarget::TrueType { max_error } = options.target else { return Err(TablesError::invalid("the target is not TrueType")) };
            ps_to_truetype(&mut work, from == Outlines::Cff2, max_error, options, &mut left_behind)?;
        }
        (Outlines::TrueType, Outlines::Cff | Outlines::Cff2) => {
            truetype_to_cff(&mut work, options, &mut left_behind)?;
            if to == Outlines::Cff2 {
                cff_to_cff2(&mut work, options, &mut left_behind)?;
            }
        }
        (Outlines::Cff, Outlines::Cff2) => cff_to_cff2(&mut work, options, &mut left_behind)?,
        (Outlines::Cff2, Outlines::Cff) => cff2_to_cff(&mut work, &mut left_behind)?,
    }
    if to != Outlines::TrueType {
        let subroutinize = options.subroutinize.unwrap_or(changed_outlines && !options.target.bare());
        let tag = if to == Outlines::Cff2 { CFF2 } else { CFF };
        if let Some(table) = work.get(tag) {
            let table = if subroutinize {
                subroutinize_table(table, to == Outlines::Cff2, options)?
            } else if options.subroutinize == Some(false) {
                fontcraft_cff::subr::desubroutinize_table(table, to == Outlines::Cff2).map_err(cff_table::subr_error)?
            } else {
                table.to_vec()
            };
            work.insert(tag, table);
        }
    }
    let glyphs = num_glyphs(&work)?;
    if options.target.bare() {
        let tag = if to == Outlines::Cff2 { CFF2 } else { CFF };
        let bytes = work.get(tag).ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag)))?.to_vec();
        left_behind.push("every table but the outline table (a bare font program)".into());
        return Ok(Converted { bytes, font: None, glyphs, left_behind });
    }
    if let Some(seconds) = options.modified {
        let mut head = table(&work, *b"head")?.to_vec();
        head.get_mut(28..36).ok_or_else(|| TablesError::invalid("the head table is cut short"))?.copy_from_slice(&seconds.to_be_bytes());
        work.insert(*b"head", head);
    }
    let bytes = crate::write(&work, &WriteOptions::default())?;
    Ok(Converted { bytes, font: Some(work), glyphs, left_behind })
}

fn check_options(o: &ConvertOptions) -> Result<()> {
    let bound = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_ERROR_UNITS;
    if let ConvertTarget::TrueType { max_error } = o.target
        && !bound(max_error)
    {
        return Err(TablesError::BadOptions(format!("the conversion error must be above 0 and at most {MAX_ERROR_UNITS} units")));
    }
    if !bound(o.cubic_error) {
        return Err(TablesError::BadOptions(format!("the conversion error must be above 0 and at most {MAX_ERROR_UNITS} units")));
    }
    if let Some(n) = o.max_subrs
        && !(1..=fontcraft_cff::subr::MAX_SUBRS_PER_INDEX).contains(&n)
    {
        return Err(TablesError::BadOptions(format!("the subroutine limit must be 1 to {}", fontcraft_cff::subr::MAX_SUBRS_PER_INDEX)));
    }
    Ok(())
}

fn subroutinize_table(table: &[u8], cff2: bool, o: &ConvertOptions) -> Result<Vec<u8>> {
    let mut subr = fontcraft_cff::subr::SubrOptions::default();
    if let Some(n) = o.max_subrs {
        subr.max_subrs = n;
    }
    fontcraft_cff::subr::subroutinize_table(table, cff2, &subr).map_err(cff_table::subr_error)
}

// ── CFF or CFF2 to TrueType ─────────────────────────────────────────────────────────────────────

/// A glyph's encoded `glyf` data and its bounds.
type EncodedGlyph = (Vec<u8>, Option<[i16; 4]>);

/// A component of a composite made from a seac glyph: glyph id, x and y offsets.
type SeacPart = (usize, i32, i32);

fn ps_to_truetype(font: &mut SfntFont, cff2: bool, max_error: f64, o: &ConvertOptions, left_behind: &mut Vec<String>) -> Result<()> {
    let upem = units_per_em(font)?;
    let n = num_glyphs(font)?;
    let tag = if cff2 { CFF2 } else { CFF };
    let data = table(font, tag)?.to_vec();
    let ps = PsOutlines::new(&data, upem)?;
    if ps.glyph_count() != n {
        return Err(TablesError::invalid(format!("maxp counts {n} glyphs but the outline table holds {}", ps.glyph_count())));
    }
    let names = glyph_names(font, n, Some(&ps));
    let mut metrics = Metrics::read(font, n)?;
    // A simple glyph is encoded as soon as it is converted: only its glyf bytes are kept.
    let mut budget = outline::Budget::truetype();
    let mut encoded: Vec<Option<EncodedGlyph>> = Vec::with_capacity(n);
    let mut sizes = vec![(0usize, 0usize); n];
    let (mut max_points, mut max_contours) = (0usize, 0usize);
    let mut glyf_len = 0usize;
    let mut simple = |contours: &[Contour], gid: usize| -> Result<EncodedGlyph> {
        let mut out = Vec::with_capacity(contours.len());
        for c in contours {
            let q = outline::to_quadratic(c, max_error).map_err(|e| in_glyph(e, gid))?;
            out.push(if o.reverse_direction { q.reversed() } else { q });
        }
        let points = outline::truetype_points(&out).map_err(|e| in_glyph(e, gid))?;
        let size = (points.iter().map(Vec::len).sum(), points.len());
        budget.points(size.0)?;
        max_points = max_points.max(size.0);
        max_contours = max_contours.max(size.1);
        if let Some(slot) = sizes.get_mut(gid) {
            *slot = size;
        }
        let glyph = outline::encode_simple(&points).map_err(|e| in_glyph(e, gid))?;
        glyf_len = glyf_len.saturating_add(glyph.0.len().next_multiple_of(2));
        if glyf_len > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the glyf table", limit: crate::limits::MAX_SFNT });
        }
        Ok(glyph)
    };
    // A seac glyph becomes a composite of its base and accent, as the reference draws seac as
    // two components; one with contours of its own, or whose parts are accented, is drawn out.
    let mut composites: Vec<(usize, Vec<SeacPart>)> = Vec::new();
    for gid in 0..n {
        let drawing = ps.draw(gid)?;
        let glyph = match drawing.seac {
            None => Some(simple(&drawing.contours, gid)?),
            Some(seac) => {
                let parts: Vec<(usize, f64, f64)> = [(seac.base, 0.0, 0.0), (seac.accent, seac.adx, seac.ady)]
                    .into_iter()
                    .filter_map(|(code, dx, dy)| ps.standard_glyph(code).map(|g| (g, dx, dy)))
                    .collect();
                if drawing.contours.is_empty() && parts.iter().all(|p| !ps.is_accented(p.0)) {
                    let place = |v: f64| -> Result<i32> {
                        let r = (v + 0.5).floor();
                        if !(r.is_finite() && (-32_768.0..=32_767.0).contains(&r)) {
                            return Err(TablesError::Unsupported(format!("glyph {gid}: a seac offset of {v} does not fit in glyf")));
                        }
                        Ok(r as i32)
                    };
                    let placed =
                        parts.iter().map(|&(g, dx, dy)| -> Result<(usize, i32, i32)> { Ok((g, place(dx)?, place(dy)?)) }).collect::<Result<_>>()?;
                    composites.push((gid, placed));
                    None
                } else {
                    Some(simple(&ps.flattened(gid)?, gid)?)
                }
            }
        };
        encoded.push(glyph);
    }
    let (mut composite_points, mut composite_contours, mut components, mut depth) = (0usize, 0usize, 0usize, 0u16);
    for (gid, parts) in &composites {
        let (gid, parts) = (*gid, parts.as_slice());
        let mut bbox: Option<[i32; 4]> = None;
        let mut placed = Vec::with_capacity(parts.len());
        let (mut points, mut contours) = (0usize, 0usize);
        for &(part, dx, dy) in parts {
            let id = u16::try_from(part).map_err(|_| TablesError::invalid("more than 65535 glyphs"))?;
            placed.push((id, dx, dy));
            let (p, c) = sizes.get(part).copied().unwrap_or((0, 0));
            points += p;
            contours += c;
            let Some(Some((_, Some(b)))) = encoded.get(part) else { continue };
            let moved = [i32::from(b[0]) + dx, i32::from(b[1]) + dy, i32::from(b[2]) + dx, i32::from(b[3]) + dy];
            bbox = Some(match bbox {
                None => moved,
                Some(u) => [u[0].min(moved[0]), u[1].min(moved[1]), u[2].max(moved[2]), u[3].max(moved[3])],
            });
        }
        let fit = |v: i32| i16::try_from(v).map_err(|_| TablesError::Unsupported(format!("glyph {gid}: its bounds do not fit in glyf")));
        let b = bbox.unwrap_or([0; 4]);
        let bbox = [fit(b[0])?, fit(b[1])?, fit(b[2])?, fit(b[3])?];
        let bytes = outline::encode_composite(&placed, bbox).map_err(|e| in_glyph(e, gid))?;
        composite_points = composite_points.max(points);
        composite_contours = composite_contours.max(contours);
        components = components.max(parts.len());
        depth = 1;
        if let Some(slot) = encoded.get_mut(gid) {
            *slot = Some((bytes, Some(bbox)));
        }
    }
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(n + 1);
    let mut bounds: Vec<Option<[i16; 4]>> = Vec::with_capacity(n);
    for (gid, e) in encoded.into_iter().enumerate() {
        let (bytes, bbox) = e.ok_or_else(|| TablesError::invalid("a glyph was left unwritten"))?;
        offsets.push(glyf.len());
        glyf.extend_from_slice(&bytes);
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
        if glyf.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the glyf table", limit: crate::limits::MAX_SFNT });
        }
        if let Some(b) = bbox {
            metrics.set_lsb(gid, b[0]);
        }
        bounds.push(bbox);
    }
    offsets.push(glyf.len());
    let long = glyf.len() > 0x1FFFE;
    let mut loca = Vec::with_capacity(offsets.len() * 4);
    for off in &offsets {
        if long {
            loca.extend_from_slice(&u32::try_from(*off).map_err(|_| TablesError::invalid("glyf is too large"))?.to_be_bytes());
        } else {
            loca.extend_from_slice(&u16::try_from(off / 2).map_err(|_| TablesError::invalid("glyf is too large"))?.to_be_bytes());
        }
    }

    let mut head = table(font, *b"head")?.to_vec();
    let bbox = union(&bounds);
    for (i, v) in bbox.iter().enumerate() {
        put_i16(&mut head, 36 + 2 * i, *v)?;
    }
    put_i16(&mut head, 50, i16::from(long))?;
    put_i16(&mut head, 52, 0)?;

    let mut maxp = vec![0u8; 32];
    maxp.get_mut(0..4).ok_or_else(|| TablesError::invalid("maxp"))?.copy_from_slice(&0x0001_0000u32.to_be_bytes());
    let count = u16::try_from(n).map_err(|_| TablesError::invalid("more than 65535 glyphs"))?;
    put_u16(&mut maxp, 4, count)?;
    put_u16(&mut maxp, 6, u16::try_from(max_points).unwrap_or(u16::MAX))?;
    put_u16(&mut maxp, 8, u16::try_from(max_contours).unwrap_or(u16::MAX))?;
    put_u16(&mut maxp, 10, u16::try_from(composite_points).unwrap_or(u16::MAX))?;
    put_u16(&mut maxp, 12, u16::try_from(composite_contours).unwrap_or(u16::MAX))?;
    put_u16(&mut maxp, 14, 1)?;
    put_u16(&mut maxp, 28, u16::try_from(components).unwrap_or(u16::MAX))?;
    put_u16(&mut maxp, 30, depth)?;

    let (post, dropped) = post_table(font.get(*b"post"), o.post_format, &names)?;
    if dropped {
        left_behind.push("glyph names (they do not fit in a post table)".into());
    }
    if o.post_format == PostFormat::NoNames {
        left_behind.push("glyph names (post format 3)".into());
    }
    left_behind.push("PostScript hints (TrueType outlines hold none)".into());
    let hhea = metrics.recalc_hhea(table(font, *b"hhea")?, &bounds)?;

    font.remove(tag);
    if font.remove(*b"VORG").is_some() {
        left_behind.push("the VORG table".into());
    }
    font.insert(*b"glyf", glyf);
    font.insert(*b"loca", loca);
    font.insert(*b"head", head);
    font.insert(*b"maxp", maxp);
    font.insert(*b"hhea", hhea);
    font.insert(*b"hmtx", metrics.write());
    font.insert(*b"post", post);
    font.flavour = Flavour::TrueType;
    Ok(())
}

// ── TrueType to CFF ─────────────────────────────────────────────────────────────────────────────

fn truetype_to_cff(font: &mut SfntFont, o: &ConvertOptions, left_behind: &mut Vec<String>) -> Result<()> {
    let head = table(font, *b"head")?.to_vec();
    let n = num_glyphs(font)?;
    let long = u16_at(&head, 50).ok_or_else(|| TablesError::invalid("the head table is cut short"))? != 0;
    let names = glyph_names(font, n, None);
    let glyf_bytes = table(font, *b"glyf")?.to_vec();
    let loca_bytes = table(font, *b"loca")?.to_vec();
    let outlines = GlyfOutlines::new(&glyf_bytes, &loca_bytes, long, n)?;
    let mut metrics = Metrics::read(font, n)?;
    let mut instructions = false;
    let mut budget = outline::Budget::new();
    let mut glyphs = Vec::with_capacity(n);
    for (gid, name) in names.iter().enumerate() {
        let contours = outlines.contours(gid, &mut instructions, &mut budget)?;
        // The references draw a TrueType glyph moved so that its stated xMin sits at the left
        // side bearing of hmtx; the two differ in fonts whose metrics were edited by hand.
        let shift = outlines.x_min(gid).map_or(0, |x| i32::from(metrics.lsb(gid)) - i32::from(x));
        let mut cubic: Vec<Contour> = Vec::with_capacity(contours.len());
        for c in &contours {
            let c = if shift == 0 { c.clone() } else { c.translated(f64::from(shift), 0.0) };
            let k = outline::to_cubic(&c, o.quad_to_cubic, o.cubic_error).map_err(|e| in_glyph(e, gid))?;
            cubic.push(if o.reverse_direction { k.reversed() } else { k });
        }
        let path = outline::path_ops(&cubic).map_err(|e| in_glyph(e, gid))?;
        glyphs.push(fontcraft_cff::Glyph { name: name.clone(), advance: f64::from(metrics.advance(gid)), path, hints: None });
    }
    let cff_font = fontcraft_cff::Font {
        name: postscript_name(font),
        top: top_dict(font),
        private: fontcraft_cff::PrivateDict::default(),
        glyphs,
        widths: fontcraft_cff::Widths::Optimise,
        round_tolerance: 0.5,
        specialise: true,
    };
    let built = fontcraft_cff::build(&cff_font).map_err(|e| TablesError::Unsupported(format!("the CFF table cannot be written: {e}")))?;

    let mut bounds: Vec<Option<[i16; 4]>> = Vec::with_capacity(n);
    for (gid, b) in built.glyph_bounds.iter().enumerate() {
        let b = b.map(|b| b.round_with_tolerance(0.5)).map(|b| b.map(|v| v.clamp(-32_768, 32_767) as i16));
        if let Some(b) = b {
            metrics.set_lsb(gid, b[0]);
        }
        bounds.push(b);
    }
    let mut head = head;
    for (i, v) in built.font_bbox.iter().enumerate() {
        put_i16(&mut head, 36 + 2 * i, (*v).clamp(-32_768, 32_767) as i16)?;
    }
    put_i16(&mut head, 50, 0)?;
    put_i16(&mut head, 52, 0)?;
    let mut maxp = vec![0u8; 6];
    maxp.get_mut(0..4).ok_or_else(|| TablesError::invalid("maxp"))?.copy_from_slice(&0x0000_5000u32.to_be_bytes());
    put_u16(&mut maxp, 4, u16::try_from(n).map_err(|_| TablesError::invalid("more than 65535 glyphs"))?)?;
    let hhea = metrics.recalc_hhea(table(font, *b"hhea")?, &bounds)?;
    let (post, _) = post_table(font.get(*b"post"), PostFormat::NoNames, &names)?;

    if instructions || [*b"fpgm", *b"prep", *b"cvt "].iter().any(|t| font.get(*t).is_some()) {
        left_behind.push("TrueType instructions".into());
    }
    for tag in TRUETYPE_ONLY {
        if font.remove(tag).is_some() && !matches!(&tag, b"glyf" | b"loca" | b"cvt " | b"fpgm" | b"prep") {
            left_behind.push(format!("the {} table", crate::sfnt::tag_str(tag).trim_end()));
        }
    }
    font.insert(CFF, built.table);
    font.insert(*b"head", head);
    font.insert(*b"maxp", maxp);
    font.insert(*b"hhea", hhea);
    font.insert(*b"hmtx", metrics.write());
    font.insert(*b"post", post);
    font.flavour = Flavour::Cff;
    Ok(())
}

/// Top DICT strings and numbers from `name`, `head` and `post`, as compilers fill them.
fn top_dict(font: &SfntFont) -> fontcraft_cff::TopDict {
    use fontcraft_cff::Number;
    let latin = |s: Option<String>| s.map(|s| s.chars().filter(|c| u32::from(*c) <= 0xFF && !c.is_control()).collect::<String>());
    let ascii = |s: Option<String>| s.map(|s| s.chars().filter(|c| c.is_ascii() && !c.is_control()).collect::<String>());
    let defaults = fontcraft_cff::TopDict::default();
    let post = font.get(*b"post");
    let number = |v: f64| if v.fract() == 0.0 { Number::Int(v as i64) } else { Number::Real(v) };
    let italic_angle = post.and_then(|p| u32_at(p, 4)).map_or(defaults.italic_angle, |a| number(f64::from(a as i32) / 65_536.0));
    let underline = post.and_then(|p| Some((u16_at(p, 8)?, u16_at(p, 10)?)));
    fontcraft_cff::TopDict {
        copyright: latin(name_string(font, 0)),
        notice: latin(name_string(font, 7)),
        full_name: latin(name_string(font, 4)),
        family_name: latin(name_string(font, 16).or_else(|| name_string(font, 1))),
        version: ascii(font.get(*b"head").and_then(|h| u32_at(h, 4)).map(|rev| format!("{:.3}", f64::from(rev as i32) / 65_536.0))),
        italic_angle,
        underline_position: underline.map_or(defaults.underline_position, |(pos, _)| Number::Int(i64::from(pos as i16))),
        underline_thickness: underline.map_or(defaults.underline_thickness, |(_, thick)| Number::Int(i64::from(thick as i16))),
        is_fixed_pitch: if post.and_then(|p| u32_at(p, 12)).is_some_and(|v| v != 0) { Number::Int(1) } else { defaults.is_fixed_pitch },
        ..defaults
    }
}

// ── CFF and CFF2 ────────────────────────────────────────────────────────────────────────────────

fn cff_to_cff2(font: &mut SfntFont, o: &ConvertOptions, left_behind: &mut Vec<String>) -> Result<()> {
    let n = num_glyphs(font)?;
    let upem = units_per_em(font)?;
    let data = table(font, CFF)?.to_vec();
    let ps = PsOutlines::new(&data, upem)?;
    let names = glyph_names(font, n, Some(&ps));
    let (cff2, dropped_dotsection) = cff_table::cff_to_cff2(&data, &|code| ps.standard_glyph(code))?;
    if dropped_dotsection {
        left_behind.push("dotsection hints (CFF2 has none)".into());
    }
    let (post, dropped) = post_table(font.get(*b"post"), o.post_format, &names)?;
    if dropped {
        left_behind.push("glyph names (they do not fit in a post table)".into());
    }
    if o.post_format == PostFormat::NoNames {
        left_behind.push("glyph names (post format 3)".into());
    }
    font.remove(CFF);
    font.insert(CFF2, cff2);
    font.insert(*b"post", post);
    Ok(())
}

fn cff2_to_cff(font: &mut SfntFont, left_behind: &mut Vec<String>) -> Result<()> {
    let n = num_glyphs(font)?;
    let upem = units_per_em(font)?;
    let data = table(font, CFF2)?.to_vec();
    let mut ps = PsOutlines::new(&data, upem)?;
    // The union of each glyph's bounds is the bounds of all contours together; a contour
    // without segments adds nothing to either.
    let mut bounds: Option<[f64; 4]> = None;
    for gid in 0..ps.glyph_count().min(n) {
        let mut contours = ps.contours(gid)?;
        contours.retain(|c| !c.segs.is_empty());
        if let Some(b) = outline::drawn_bounds(&contours) {
            bounds = Some(match bounds {
                None => b,
                Some(u) => [u[0].min(b[0]), u[1].min(b[1]), u[2].max(b[2]), u[3].max(b[3])],
            });
        }
    }
    let font_bbox = bounds.map(|b| [b[0].floor() as i64, b[1].floor() as i64, b[2].ceil() as i64, b[3].ceil() as i64]).unwrap_or([0; 4]);
    let names = post_names(font, n);
    let metrics = Metrics::read(font, n)?;
    let (cff, by_name) = cff_table::cff2_to_cff(&data, &metrics.advances, names.as_deref(), &postscript_name(font), font_bbox)?;
    if !by_name {
        left_behind.push("glyph names (the CFF table is CID-keyed)".into());
    }
    let (post, _) = post_table(font.get(*b"post"), PostFormat::NoNames, &[])?;
    font.remove(CFF2);
    font.insert(CFF, cff);
    font.insert(*b"post", post);
    Ok(())
}

// ── Shared table work ───────────────────────────────────────────────────────────────────────────

fn in_glyph(e: TablesError, gid: usize) -> TablesError {
    match e {
        TablesError::Invalid(m) => TablesError::Invalid(format!("glyph {gid}: {m}")),
        TablesError::Unsupported(m) => TablesError::Unsupported(format!("glyph {gid}: {m}")),
        other => other,
    }
}

fn table(font: &SfntFont, tag: Tag) -> Result<&[u8]> {
    font.get(tag).ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag).trim_end().to_string()))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn put_u16(b: &mut [u8], at: usize, v: u16) -> Result<()> {
    b.get_mut(at..at + 2).ok_or_else(|| TablesError::invalid("a table is cut short"))?.copy_from_slice(&v.to_be_bytes());
    Ok(())
}

fn put_i16(b: &mut [u8], at: usize, v: i16) -> Result<()> {
    put_u16(b, at, v as u16)
}

fn units_per_em(font: &SfntFont) -> Result<u16> {
    let upem = u16_at(table(font, *b"head")?, 18).ok_or_else(|| TablesError::invalid("the head table is cut short"))?;
    if !(crate::limits::MIN_UNITS_PER_EM..=crate::limits::MAX_UNITS_PER_EM).contains(&upem) {
        return Err(TablesError::invalid(format!("units per em {upem} is out of range")));
    }
    Ok(upem)
}

fn num_glyphs(font: &SfntFont) -> Result<usize> {
    let n = usize::from(u16_at(table(font, *b"maxp")?, 4).ok_or_else(|| TablesError::invalid("the maxp table is cut short"))?);
    if n == 0 {
        return Err(TablesError::invalid("the font has no glyphs"));
    }
    Ok(n)
}

fn union(bounds: &[Option<[i16; 4]>]) -> [i16; 4] {
    let mut it = bounds.iter().flatten();
    let Some(first) = it.next() else { return [0; 4] };
    it.fold(*first, |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

/// Advances and left side bearings from `hmtx`.
struct Metrics {
    advances: Vec<u16>,
    lsbs: Vec<i16>,
    long: usize,
}

impl Metrics {
    fn read(font: &SfntFont, n: usize) -> Result<Self> {
        let hhea = table(font, *b"hhea")?;
        let hmtx = table(font, *b"hmtx")?;
        let long = usize::from(u16_at(hhea, 34).ok_or_else(|| TablesError::invalid("the hhea table is cut short"))?).min(n);
        if long == 0 {
            return Err(TablesError::invalid("hhea has no horizontal metrics"));
        }
        let mut advances = Vec::with_capacity(n);
        let mut lsbs = Vec::with_capacity(n);
        let cut = || TablesError::invalid("the hmtx table is cut short");
        for gid in 0..n {
            if gid < long {
                advances.push(u16_at(hmtx, 4 * gid).ok_or_else(cut)?);
                lsbs.push(u16_at(hmtx, 4 * gid + 2).ok_or_else(cut)? as i16);
            } else {
                advances.push(advances.last().copied().unwrap_or(0));
                lsbs.push(u16_at(hmtx, 4 * long + 2 * (gid - long)).unwrap_or(0) as i16);
            }
        }
        Ok(Metrics { advances, lsbs, long })
    }

    fn advance(&self, gid: usize) -> u16 {
        self.advances.get(gid).copied().unwrap_or(0)
    }

    fn lsb(&self, gid: usize) -> i16 {
        self.lsbs.get(gid).copied().unwrap_or(0)
    }

    fn set_lsb(&mut self, gid: usize, v: i16) {
        if let Some(slot) = self.lsbs.get_mut(gid) {
            *slot = v;
        }
    }

    fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.advances.len() * 4);
        for (gid, (adv, lsb)) in self.advances.iter().zip(&self.lsbs).enumerate() {
            if gid < self.long {
                out.extend_from_slice(&adv.to_be_bytes());
            }
            out.extend_from_slice(&lsb.to_be_bytes());
        }
        out
    }

    /// `hhea` with `advanceWidthMax`, the side bearing minimums and `xMaxExtent` recalculated
    /// from the glyphs that draw something, as fontTools does on saving.
    fn recalc_hhea(&self, hhea: &[u8], bounds: &[Option<[i16; 4]>]) -> Result<Vec<u8>> {
        let mut out = hhea.to_vec();
        let max_advance = self.advances.iter().copied().max().unwrap_or(0);
        put_u16(&mut out, 10, max_advance)?;
        let (mut min_lsb, mut min_rsb, mut max_extent) = (i32::MAX, i32::MAX, i32::MIN);
        for (gid, b) in bounds.iter().enumerate() {
            let Some(b) = b else { continue };
            let width = i32::from(b[2]) - i32::from(b[0]);
            let lsb = i32::from(self.lsbs.get(gid).copied().unwrap_or(0));
            let adv = i32::from(self.advance(gid));
            min_lsb = min_lsb.min(lsb);
            min_rsb = min_rsb.min(adv - lsb - width);
            max_extent = max_extent.max(lsb + width);
        }
        let clamp = |v: i32| if v == i32::MAX || v == i32::MIN { 0 } else { v.clamp(-32_768, 32_767) as i16 };
        put_i16(&mut out, 12, clamp(min_lsb))?;
        put_i16(&mut out, 14, clamp(min_rsb))?;
        put_i16(&mut out, 16, clamp(max_extent))?;
        Ok(out)
    }
}

/// A `post` table of `format` with the header of `old`; answers whether names had to be
/// dropped because they do not fit.
fn post_table(old: Option<&[u8]>, format: PostFormat, names: &[String]) -> Result<(Vec<u8>, bool)> {
    let mut head = vec![0u8; 32];
    if let Some(src) = old.and_then(|o| o.get(..32)) {
        head.copy_from_slice(src);
    }
    let three = |mut h: Vec<u8>| {
        if let Some(v) = h.get_mut(0..4) {
            v.copy_from_slice(&0x0003_0000u32.to_be_bytes());
        }
        h
    };
    if format == PostFormat::NoNames || names.is_empty() {
        return Ok((three(head), false));
    }
    let mut out = head.clone();
    if let Some(v) = out.get_mut(0..4) {
        v.copy_from_slice(&0x0002_0000u32.to_be_bytes());
    }
    let count = u16::try_from(names.len()).map_err(|_| TablesError::invalid("more than 65535 glyphs"))?;
    out.extend_from_slice(&count.to_be_bytes());
    let mut strings = Vec::new();
    let mut extra = 0usize;
    let mut seen = std::collections::HashMap::new();
    for name in names {
        let index = match DEFAULT_GLYPH_NAMES.iter().position(|d| d == name) {
            Some(i) => i,
            None => match seen.get(name) {
                Some(i) => *i,
                None => {
                    let bytes = name.as_bytes();
                    let Ok(len) = u8::try_from(bytes.len()) else { return Ok((three(head), true)) };
                    strings.push(len);
                    strings.extend_from_slice(bytes);
                    let i = 258 + extra;
                    extra += 1;
                    seen.insert(name.clone(), i);
                    i
                }
            },
        };
        let Ok(index) = u16::try_from(index) else { return Ok((three(head), true)) };
        if index > 32_767 {
            return Ok((three(head), true));
        }
        out.extend_from_slice(&index.to_be_bytes());
    }
    out.extend_from_slice(&strings);
    Ok((out, false))
}

/// Glyph names from `post` (format 1 or 2) only, `.notdef` first; `None` without them.
fn post_names(font: &SfntFont, n: usize) -> Option<Vec<String>> {
    let post = Post::read(FontData::new(font.get(*b"post")?)).ok()?;
    let mut names = Vec::with_capacity(n);
    for gid in 0..n {
        names.push(post.glyph_name(GlyphId16::new(u16::try_from(gid).ok()?))?.to_string());
    }
    Some(unique_names(names))
}

/// One name per glyph: from `post`, then the CFF charset, then `glyphNNNNN`; `.notdef` first,
/// repeats made unique with `#n`, characters a PostScript name cannot hold replaced by `_`.
fn glyph_names(font: &SfntFont, n: usize, ps: Option<&PsOutlines<'_>>) -> Vec<String> {
    let post = font.get(*b"post").and_then(|p| Post::read(FontData::new(p)).ok());
    let names = (0..n)
        .map(|gid| {
            let from_post = post.as_ref().and_then(|p| p.glyph_name(GlyphId16::new(u16::try_from(gid).ok()?)).map(str::to_string));
            from_post.or_else(|| ps.and_then(|ps| ps.name(gid))).unwrap_or_else(|| format!("glyph{gid:05}"))
        })
        .collect();
    unique_names(names)
}

fn unique_names(names: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::with_capacity(names.len());
    let mut out = Vec::with_capacity(names.len());
    for (gid, name) in names.into_iter().enumerate() {
        let clean: String = name.chars().take(63).map(|c| if c.is_ascii_graphic() && !"[](){}<>/%".contains(c) { c } else { '_' }).collect();
        let base = if gid == 0 {
            ".notdef".to_string()
        } else if clean.is_empty() || clean == ".notdef" {
            format!("glyph{gid:05}")
        } else {
            clean
        };
        let mut candidate = base.clone();
        let mut k = 1usize;
        while seen.contains(&candidate) {
            candidate = format!("{base}#{k}");
            k += 1;
        }
        seen.insert(candidate.clone());
        out.push(candidate);
    }
    out
}

/// An English (or the first) string of `name` with this ID.
fn name_string(font: &SfntFont, id: u16) -> Option<String> {
    let name = Name::read(FontData::new(font.get(*b"name")?)).ok()?;
    let data = name.string_data();
    let records: Vec<_> = name.name_record().iter().filter(|r| r.name_id().to_u16() == id).collect();
    let pick = records
        .iter()
        .find(|r| r.platform_id() == 3 && r.language_id() == 0x409)
        .or_else(|| records.iter().find(|r| r.platform_id() == 1 && r.language_id() == 0))
        .or_else(|| records.first())?;
    let s: String = pick.string(data).ok()?.chars().collect();
    Some(s).filter(|s| !s.trim().is_empty())
}

/// The PostScript name (name ID 6) as a CFF font name: printable ASCII without the delimiters,
/// at most 63 characters.
fn postscript_name(font: &SfntFont) -> String {
    let raw = name_string(font, 6).unwrap_or_default();
    let clean: String = raw.chars().filter(|c| c.is_ascii_graphic() && !"[](){}<>/%".contains(*c)).take(63).collect();
    if clean.is_empty() { "Untitled".to_string() } else { clean }
}
