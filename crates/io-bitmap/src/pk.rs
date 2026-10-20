//! TeX packed fonts (PK): reading and writing.
//!
//! Written from the format description in Tomas Rokicki's PKtype documentation (part of the TeX
//! distribution's public program documentation) and from the files the TeX tools `gftopk` and
//! `pktype` write and print for METAFONT output of our own. No code that reads or writes PK was
//! consulted.
//!
//! A PK file holds one font at one resolution: a preamble with the design size and the pixels per
//! point, then one packet per character with its code, TeX width, escapement, box and raster.
//! Rasters are a plain bitmap or nybble-coded run lengths over the rows read as one string of
//! pixels, with repeat counts for rows that repeat.

use crate::bdf::{check_for_writing, glyph_extent};
use crate::binary::{Cursor, Scale, code_name, set_bits};
use crate::{
    Bitmap, BitmapError, BitmapFont, BitmapGlyph, Import, MAX_BITMAP_SIDE, MAX_COORDINATE, MAX_FILE_BYTES, MAX_GLYPHS, PixelBudget, Result, Warnings,
};

pub(crate) const PRE: u8 = 247;
pub(crate) const ID: u8 = 89;
const XXX1: u8 = 240;
const XXX4: u8 = 243;
const YYY: u8 = 244;
const POST: u8 = 245;
const NO_OP: u8 = 246;
/// The `dyn_f` value of a raster stored as a plain bitmap.
const BITMAP_DYN_F: u8 = 14;
/// Most zero nybbles before a large run count: eight hexadecimal digits in all, so the count fits
/// in 32 bits.
const MAX_LARGE_RUN_ZEROS: u32 = 7;

/// Reads a PK file.
pub fn read_pk(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    let mut c = Cursor::new(bytes, "PK");
    if c.u8()? != PRE || c.u8()? != ID {
        return Err(c.error("not a PK file: it does not start with a PK preamble"));
    }
    let comment_len = c.u8()?;
    c.skip(usize::from(comment_len))?;
    let design_size = c.i32()?;
    let _checksum = c.u32()?;
    let (hppp, vppp) = (c.i32()?, c.i32()?);
    let scale = Scale::new("PK", design_size, hppp, vppp)?;
    let mut glyphs = Vec::new();
    let mut warnings = Warnings::default();
    let mut budget = PixelBudget::default();
    loop {
        if c.at_end() {
            warnings.push("the file ends without a postamble".into());
            break;
        }
        let flag = c.u8()?;
        match flag {
            0..=239 => {
                if glyphs.len() >= MAX_GLYPHS {
                    return Err(BitmapError::Limit { what: "glyphs", limit: MAX_GLYPHS });
                }
                glyphs.push(read_char(&mut c, flag, &mut budget)?);
            }
            XXX1..=XXX4 => {
                let len = c.uint(usize::from(flag - XXX1) + 1)?;
                c.skip(usize::try_from(len).map_err(|_| c.error("a special is too long"))?)?;
            }
            YYY => c.skip(4)?,
            POST => break,
            NO_OP => {}
            _ => return Err(c.error(format!("{flag} is not a PK command"))),
        }
    }
    Ok(Import { font: tex_font(glyphs, &scale), warnings: warnings.finish() })
}

/// The font a PK or GF file describes: no name or properties (the file has none), its size from
/// the design size and resolution, and its ascent and descent from the glyphs.
pub(crate) fn tex_font(glyphs: Vec<BitmapGlyph>, scale: &Scale) -> BitmapFont {
    let (ascent, descent) = glyph_extent(&glyphs);
    BitmapFont {
        name: String::new(),
        pixel_size: scale.pixel_size(),
        point_size: scale.design_points,
        resolution: scale.resolution(),
        depth: 1,
        ascent,
        descent,
        properties: Vec::new(),
        glyphs,
    }
}

/// A character's preamble values.
struct Preamble {
    code: u32,
    tfm: i32,
    /// Escapement in pixels times 2^16.
    dx: i64,
    width: i64,
    height: i64,
    hoff: i64,
    voff: i64,
}

fn read_char(c: &mut Cursor<'_>, flag: u8, budget: &mut PixelBudget) -> Result<BitmapGlyph> {
    let (pre, end) = match flag & 7 {
        0..=3 => {
            let length = (usize::from(flag & 3) << 8) | usize::from(c.u8()?);
            let code = u32::from(c.u8()?);
            let end = c.pos.saturating_add(length);
            let tfm = c.uint(3)? as i32;
            let dx = i64::from(c.u8()?) << 16;
            let (width, height) = (i64::from(c.u8()?), i64::from(c.u8()?));
            let (hoff, voff) = (i64::from(c.i8()?), i64::from(c.i8()?));
            (Preamble { code, tfm, dx, width, height, hoff, voff }, end)
        }
        4..=6 => {
            let length = (usize::from(flag & 3) << 16) | usize::from(c.u16()?);
            let code = u32::from(c.u8()?);
            let end = c.pos.saturating_add(length);
            let tfm = c.uint(3)? as i32;
            let dx = i64::from(c.u16()?) << 16;
            let (width, height) = (i64::from(c.u16()?), i64::from(c.u16()?));
            let (hoff, voff) = (i64::from(c.i16()?), i64::from(c.i16()?));
            (Preamble { code, tfm, dx, width, height, hoff, voff }, end)
        }
        _ => {
            let length = c.u32()? as usize;
            let code = c.u32()?;
            let end = c.pos.saturating_add(length);
            let tfm = c.i32()?;
            let dx = i64::from(c.i32()?);
            let _dy = c.i32()?;
            let (width, height) = (i64::from(c.i32()?), i64::from(c.i32()?));
            let (hoff, voff) = (i64::from(c.i32()?), i64::from(c.i32()?));
            (Preamble { code, tfm, dx, width, height, hoff, voff }, end)
        }
    };
    let name = code_name(pre.code, false);
    let fail = |c: &Cursor<'_>, message: String| c.error(format!("character {}: {message}", pre.code));
    if end > c.len() || end < c.pos {
        return Err(fail(c, "its packet length does not fit its preamble or the file".into()));
    }
    let side = |v: i64| u16::try_from(v).ok().filter(|v| *v <= MAX_BITMAP_SIDE);
    let (Some(width), Some(height)) = (side(pre.width), side(pre.height)) else {
        return Err(fail(c, format!("its box is {} × {} pixels; the limit is {MAX_BITMAP_SIDE}", pre.width, pre.height)));
    };
    let left = -pre.hoff;
    let bottom = pre.voff - pre.height + 1;
    let advance = (pre.dx as f64 / crate::binary::SCALED_ONE).round() as i64;
    let in_range = |v: i64| v.abs() <= i64::from(MAX_COORDINATE);
    if ![left, bottom, advance, left + pre.width, bottom + pre.height].into_iter().all(in_range) {
        return Err(fail(c, "its offsets or escapement are out of range".into()));
    }
    let raster = c.take(end - c.pos)?;
    let stride = crate::stride(width, 1);
    budget.spend(stride.saturating_mul(usize::from(height)))?;
    let dyn_f = flag >> 4;
    let data = if width == 0 || height == 0 {
        Vec::new()
    } else if dyn_f == BITMAP_DYN_F {
        unpack_bits(raster, width, height).ok_or_else(|| fail(c, "its bitmap is shorter than its box".into()))?
    } else {
        unpack_runs(raster, dyn_f, flag & 8 != 0, width, height).map_err(|m| fail(c, m))?
    };
    Ok(BitmapGlyph {
        name,
        encoding: Some(pre.code),
        advance: advance as i32,
        scalable_width: Some(Scale::thousandths(pre.tfm)),
        bitmap: Bitmap { width, height, left: left as i32, bottom: bottom as i32, depth: 1, data },
    })
}

/// Rows of a raster stored as one string of bits, `width` × `height` long.
fn unpack_bits(raster: &[u8], width: u16, height: u16) -> Option<Vec<u8>> {
    let (w, h) = (usize::from(width), usize::from(height));
    if raster.len().saturating_mul(8) < w * h {
        return None;
    }
    let stride = crate::stride(width, 1);
    let mut data = vec![0u8; stride * h];
    for y in 0..h {
        let row = data.get_mut(y * stride..(y + 1) * stride)?;
        for x in 0..w {
            if crate::binary::bit(raster, y * w + x) {
                set_bits(row, x, 1);
            }
        }
    }
    Some(data)
}

/// Reads the nybbles of a run-coded raster.
struct Nybbles<'a> {
    bytes: &'a [u8],
    next: usize,
}

enum Count {
    Run(usize),
    Repeat(usize),
}

impl Nybbles<'_> {
    fn nybble(&mut self) -> std::result::Result<u8, String> {
        let byte = self.bytes.get(self.next / 2).ok_or("its raster ends before its box is full")?;
        let value = if self.next.is_multiple_of(2) { byte >> 4 } else { byte & 0x0f };
        self.next += 1;
        Ok(value)
    }

    /// A packed number: a run count, or a repeat count for the current row.
    fn count(&mut self, dyn_f: u8) -> std::result::Result<Count, String> {
        let i = self.nybble()?;
        let two_nybble_max = 13u64.saturating_sub(u64::from(dyn_f)) * 16 + u64::from(dyn_f);
        let value = match i {
            0 => {
                let mut zeros = 1u32;
                let mut j = loop {
                    let n = self.nybble()?;
                    if n != 0 {
                        break u64::from(n);
                    }
                    zeros += 1;
                    if zeros > MAX_LARGE_RUN_ZEROS {
                        return Err("it has a run count beyond 32 bits".into());
                    }
                };
                for _ in 0..zeros {
                    j = j * 16 + u64::from(self.nybble()?);
                }
                (j + two_nybble_max).saturating_sub(15)
            }
            14 => return Ok(Count::Repeat(self.plain_count(dyn_f)?)),
            15 => return Ok(Count::Repeat(1)),
            i if i <= dyn_f => u64::from(i),
            i => (u64::from(i) - u64::from(dyn_f) - 1) * 16 + u64::from(self.nybble()?) + u64::from(dyn_f) + 1,
        };
        usize::try_from(value).map(Count::Run).map_err(|_| "it has a run count beyond the address space".into())
    }

    /// A packed number that must be a count, not another repeat.
    fn plain_count(&mut self, dyn_f: u8) -> std::result::Result<usize, String> {
        match self.count(dyn_f)? {
            Count::Run(n) => Ok(n),
            Count::Repeat(_) => Err("it has two repeat counts in a row".into()),
        }
    }
}

/// Rows of a run-coded raster. A run or repeat count that reaches past the last row is refused.
fn unpack_runs(raster: &[u8], dyn_f: u8, black_first: bool, width: u16, height: u16) -> std::result::Result<Vec<u8>, String> {
    let (w, h) = (usize::from(width), usize::from(height));
    let stride = crate::stride(width, 1);
    let overflow = || "a packed run overflows the glyph box".to_string();
    let mut data = Vec::with_capacity(stride * h);
    let mut row = vec![0u8; stride];
    let mut nybbles = Nybbles { bytes: raster, next: 0 };
    let (mut rows, mut x, mut repeat, mut black) = (0usize, 0usize, 0usize, black_first);
    while rows < h {
        let mut run = loop {
            match nybbles.count(dyn_f)? {
                Count::Run(n) => break n,
                Count::Repeat(_) if repeat != 0 => return Err("it has two repeat counts for one row".into()),
                Count::Repeat(n) => repeat = n,
            }
        };
        while run > 0 {
            if rows >= h {
                return Err(overflow());
            }
            let room = w - x;
            if run < room {
                if black {
                    set_bits(&mut row, x, run);
                }
                x += run;
                break;
            }
            if black {
                set_bits(&mut row, x, room);
            }
            run -= room;
            let copies = repeat.checked_add(1).filter(|n| rows.saturating_add(*n) <= h).ok_or_else(overflow)?;
            for _ in 0..copies {
                data.extend_from_slice(&row);
            }
            rows += copies;
            (x, repeat) = (0, 0);
            row.fill(0);
            if black {
                set_bits(&mut row, 0, w);
            }
            let whole = run / w;
            if whole > 0 {
                if rows.saturating_add(whole) > h {
                    return Err(overflow());
                }
                for _ in 0..whole {
                    data.extend_from_slice(&row);
                }
                rows += whole;
                run %= w;
            }
            row.fill(0);
        }
        black = !black;
    }
    Ok(data)
}

/// Writes a black-and-white strike as a PK file. Each glyph is written under its code; glyphs
/// without one, and glyphs whose code an earlier glyph has, are left out. The design size is
/// chosen so that the file reads back at the strike's pixel size: pixels per em ÷ (dots per inch
/// ÷ 72.27).
pub fn write_pk(font: &BitmapFont) -> Result<Vec<u8>> {
    check_for_writing(font)?;
    let fail = |message: String| Err(BitmapError::Write(message));
    if font.depth != 1 {
        return fail("PK holds black-and-white bitmaps only".into());
    }
    if font.pixel_size == 0 {
        return fail("the strike's size is 0 pixels per em".into());
    }
    let dpi = |v: u32| if v == 0 { 72.0 } else { f64::from(v) };
    let (h_per_point, v_per_point) =
        (dpi(font.resolution.0) / crate::binary::TEX_POINTS_PER_INCH, dpi(font.resolution.1) / crate::binary::TEX_POINTS_PER_INCH);
    let design_points = f64::from(font.pixel_size) / v_per_point;
    let fixed = |v: f64, one: f64| {
        let v = (v * one).round();
        (v.is_finite() && v >= 1.0 && v <= f64::from(i32::MAX)).then_some(v as i32)
    };
    let (Some(ds), Some(hppp), Some(vppp)) = (
        fixed(design_points, crate::binary::FIX_WORD_ONE),
        fixed(h_per_point, crate::binary::SCALED_ONE),
        fixed(v_per_point, crate::binary::SCALED_ONE),
    ) else {
        return fail("the size and resolution do not fit a PK preamble".into());
    };
    let mut out = vec![PRE, ID];
    let comment = b"FontCraft";
    out.push(comment.len() as u8);
    out.extend_from_slice(comment);
    for v in [ds, 0, hppp, vppp] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut seen = std::collections::HashSet::new();
    for glyph in &font.glyphs {
        let Some(code) = glyph.encoding.filter(|c| seen.insert(*c)) else { continue };
        let tfm = match glyph.scalable_width {
            Some(sw) => f64::from(sw) / 1000.0,
            None => f64::from(glyph.advance) / f64::from(font.pixel_size),
        } * crate::binary::FIX_WORD_ONE;
        let tfm = if tfm.is_finite() { tfm.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32 } else { 0 };
        write_char(&mut out, code, tfm, glyph);
    }
    out.push(POST);
    while !out.len().is_multiple_of(4) {
        out.push(NO_OP);
    }
    Ok(out)
}

fn write_char(out: &mut Vec<u8>, code: u32, tfm: i32, glyph: &BitmapGlyph) {
    let b = &glyph.bitmap;
    let (w, h) = (i64::from(b.width), i64::from(b.height));
    let hoff = -i64::from(b.left);
    let voff = i64::from(b.bottom) + h - 1;
    let advance = i64::from(glyph.advance);
    let (dyn_f, black_first, raster) = encode_raster(b);
    let flag_high = (dyn_f << 4) | if black_first { 8 } else { 0 };
    let tfm_short = (0..1 << 24).contains(&i64::from(tfm));
    let len = raster.len();
    let fits = |v: i64, lo: i64, hi: i64| (lo..=hi).contains(&v);
    if code <= 255 && tfm_short && len + 8 < 1024 && fits(advance, 0, 255) && w <= 255 && h <= 255 && fits(hoff, -128, 127) && fits(voff, -128, 127) {
        let pl = len + 8;
        out.push(flag_high | (pl >> 8) as u8);
        out.push(pl as u8);
        out.push(code as u8);
        let [_, t1, t2, t3] = tfm.to_be_bytes();
        out.extend([t1, t2, t3, advance as u8, w as u8, h as u8, hoff as i8 as u8, voff as i8 as u8]);
    } else if code <= 255
        && tfm_short
        && len + 13 < 3 << 16
        && fits(advance, 0, 65_535)
        && w <= 65_535
        && h <= 65_535
        && fits(hoff, -32_768, 32_767)
        && fits(voff, -32_768, 32_767)
    {
        let pl = len + 13;
        out.push(flag_high | 4 | (pl >> 16) as u8);
        out.extend_from_slice(&(pl as u16).to_be_bytes());
        out.push(code as u8);
        let [_, t1, t2, t3] = tfm.to_be_bytes();
        out.extend([t1, t2, t3]);
        for v in [advance, w, h, hoff, voff] {
            out.extend_from_slice(&(v as i16).to_be_bytes());
        }
    } else {
        out.push(flag_high | 7);
        out.extend_from_slice(&((len + 28) as u32).to_be_bytes());
        out.extend_from_slice(&code.to_be_bytes());
        out.extend_from_slice(&tfm.to_be_bytes());
        for v in [advance << 16, 0, w, h, hoff, voff] {
            out.extend_from_slice(&(v as i32).to_be_bytes());
        }
    }
    out.extend_from_slice(&raster);
}

/// The raster of a bitmap: run-coded with the `dyn_f` that packs it smallest, or a plain bitmap
/// when that is no larger. Returns `dyn_f`, whether the first run is black, and the bytes.
fn encode_raster(b: &Bitmap) -> (u8, bool, Vec<u8>) {
    let (w, h) = (usize::from(b.width), usize::from(b.height));
    let pixel = |i: usize| b.pixel((i % w.max(1)) as u16, (i / w.max(1)) as u16) != 0;
    let total = w * h;
    if total == 0 {
        return (BITMAP_DYN_F, false, Vec::new());
    }
    let mut runs = Vec::new();
    let mut current = pixel(0);
    let mut length = 0usize;
    for i in 0..total {
        if pixel(i) == current {
            length += 1;
        } else {
            runs.push(length);
            current = !current;
            length = 1;
        }
    }
    runs.push(length);
    let best = (0u8..=13).map(|dyn_f| (runs.iter().map(|n| nybbles_for(*n, dyn_f)).sum::<usize>(), dyn_f)).min();
    let bitmap_bytes = total.div_ceil(8);
    match best {
        Some((nybbles, dyn_f)) if nybbles.div_ceil(2) < bitmap_bytes => {
            let mut packed = Vec::with_capacity(nybbles);
            for n in &runs {
                push_count(&mut packed, *n, dyn_f);
            }
            let bytes = packed.chunks(2).map(|p| (p.first().copied().unwrap_or(0) << 4) | p.get(1).copied().unwrap_or(0)).collect();
            (dyn_f, pixel(0), bytes)
        }
        _ => {
            let mut bytes = vec![0u8; bitmap_bytes];
            for i in (0..total).filter(|i| pixel(*i)) {
                set_bits(&mut bytes, i, 1);
            }
            (BITMAP_DYN_F, false, bytes)
        }
    }
}

fn two_nybble_max(dyn_f: u8) -> usize {
    (13 - usize::from(dyn_f)) * 16 + usize::from(dyn_f)
}

/// Hexadecimal digits of the normalised value of a large run count.
fn large_digits(n: usize, dyn_f: u8) -> Vec<u8> {
    let mut j = n + 15 - two_nybble_max(dyn_f);
    let mut digits = Vec::new();
    while j > 0 {
        digits.push((j % 16) as u8);
        j /= 16;
    }
    digits.reverse();
    digits
}

fn nybbles_for(n: usize, dyn_f: u8) -> usize {
    if n <= usize::from(dyn_f) {
        1
    } else if n <= two_nybble_max(dyn_f) {
        2
    } else {
        2 * large_digits(n, dyn_f).len() - 1
    }
}

fn push_count(out: &mut Vec<u8>, n: usize, dyn_f: u8) {
    let d = usize::from(dyn_f);
    if n <= d {
        out.push(n as u8);
    } else if n <= two_nybble_max(dyn_f) {
        let v = n - d - 1;
        out.push((v / 16 + d + 1) as u8);
        out.push((v % 16) as u8);
    } else {
        let digits = large_digits(n, dyn_f);
        out.extend(std::iter::repeat_n(0, digits.len() - 1));
        out.extend(digits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PK preamble: design size 10 points, 72.27 dpi both ways (one pixel per point).
    fn preamble() -> Vec<u8> {
        let mut out = vec![PRE, ID, 2, b'h', b'i'];
        for v in [10i32 << 20, 0, 1 << 16, 1 << 16] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out
    }

    fn rows(b: &Bitmap) -> Vec<String> {
        (0..b.height).map(|y| (0..b.width).map(|x| if b.pixel(x, y) != 0 { '#' } else { '.' }).collect()).collect()
    }

    /// The bytes `gftopk` writes for a 3 × 4 character drawn in METAFONT (code 65, escapement 4
    /// pixels, TeX width 0.04 of the design size, top row one pixel above the baseline), as a
    /// plain bitmap; and the same character run-coded by hand.
    #[test]
    fn pk_reads_bitmap_and_run_coded_characters() {
        let mut bytes = preamble();
        // flag 0xE8: dyn_f 14 (bitmap); pl 10; cc 65; tfm 0x00A3D6; dm 4; w 3; h 4; hoff 0; voff 1.
        bytes.extend([0xe8, 10, 65, 0x00, 0xa3, 0xd6, 4, 3, 4, 0, 1, 0xfd, 0x20]);
        // The same shape as code 66, run-coded with dyn_f 1, black first (flag 0x18). The rows
        // read as one string are "######.#..#.": runs of 6 black, 1 white, 1 black, 2 white,
        // 1 black, 1 white. With dyn_f 1, a 1 is one nybble and 2..=193 take two: n - 2 = 16 × (i - 2)
        // + low. Nybbles 2 4 | 1 | 1 | 2 0 | 1 | 1 → 0x24 0x11 0x20 0x11; pl is 8 + 4.
        bytes.extend([0x18, 12, 66, 0x00, 0xa3, 0xd6, 4, 3, 4, 0, 1, 0x24, 0x11, 0x20, 0x11]);
        bytes.extend([POST, NO_OP]);
        let import = read_pk(&bytes).unwrap();
        let font = import.font;
        assert_eq!(font.pixel_size, 10);
        assert_eq!(font.resolution, (72, 72));
        assert_eq!(font.glyphs.len(), 2);
        for g in &font.glyphs {
            assert_eq!(rows(&g.bitmap), ["###", "###", ".#.", ".#."], "{}", g.name);
            assert_eq!((g.bitmap.left, g.bitmap.bottom, g.advance), (0, -2, 4));
            assert_eq!(g.scalable_width, Some(40));
        }
        assert_eq!(font.glyphs[0].name, "A");
        assert_eq!(font.glyphs[1].encoding, Some(66));
        assert_eq!((font.ascent, font.descent), (2, 2));
    }

    /// Repeat counts copy a row; a large run count spans rows.
    #[test]
    fn pk_reads_repeat_and_large_counts() {
        // A 2 × 20 box: rows 0..=18 are "#.", row 19 is "##". With dyn_f 13 (flag 0xD8, black
        // first): black 1; repeat 18 for the current row (E, then 18 as a large count: 18 + 15 -
        // 13 = 0x14, written 0 1 4); white 1 ends row 0, which is sent 19 times; black 2 fills
        // row 19.
        let mut raster = Vec::new();
        let nybbles = [1u8, 0xe, 0, 1, 4, 1, 2];
        for pair in nybbles.chunks(2) {
            raster.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
        }
        let mut bytes = preamble();
        let flag = (13 << 4) | 8;
        bytes.extend([flag, (8 + raster.len()) as u8, 1, 0, 0, 0, 2, 2, 20, 0, 19]);
        bytes.extend(&raster);
        bytes.push(POST);
        let g = &read_pk(&bytes).unwrap().font.glyphs[0];
        let r = rows(&g.bitmap);
        assert_eq!(r.len(), 20);
        assert!(r[..19].iter().all(|row| row == "#."));
        assert_eq!(r[19], "##");
    }

    #[test]
    fn pk_refuses_runs_past_the_box_and_hostile_sizes() {
        // A 2 × 2 box with one white run of 5 pixels.
        let mut bytes = preamble();
        bytes.extend([0xd0, 9, 1, 0, 0, 0, 2, 2, 2, 0, 1, 0x50, POST]);
        let err = read_pk(&bytes).unwrap_err().to_string();
        assert!(err.contains("overflows the glyph box"), "{err}");
        // A repeat count larger than the rows left.
        let mut bytes = preamble();
        bytes.extend([0xd8, 9, 1, 0, 0, 0, 2, 2, 2, 0, 1, 0xf5, POST]);
        assert!(read_pk(&bytes).unwrap_err().to_string().contains("overflows"));
        // A long-form character 100,000 pixels wide.
        let mut bytes = preamble();
        bytes.extend([0xe7]);
        for v in [28i32, 1, 0, 0, 0, 100_000, 1, 0, 0] {
            bytes.extend_from_slice(&v.to_be_bytes());
        }
        bytes.push(POST);
        assert!(read_pk(&bytes).unwrap_err().to_string().contains("limit"));
        // A packet length past the end of the file.
        let mut bytes = preamble();
        bytes.extend([0xe3, 0xff, 1, 0, 0, 0]);
        assert!(read_pk(&bytes).is_err());
        // A zero design size.
        let mut bytes = vec![PRE, ID, 0];
        bytes.extend([0u8; 16]);
        assert!(read_pk(&bytes).is_err());
    }

    #[test]
    fn pk_written_reads_back() {
        let mut glyphs = Vec::new();
        let shapes: [(&str, u32, &[&str]); 3] =
            [("A", 65, &["..#..", ".#.#.", "#...#", "#####", "#...#"]), ("solid", 200, &["####"; 40]), ("empty", 32, &[])];
        for (name, code, shape) in shapes {
            let width = shape.first().map_or(0, |r| r.len()) as u16;
            let mut bitmap = Bitmap::blank(width, shape.len() as u16, -1, -1, 1).unwrap();
            for (y, row) in shape.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    if c == '#' {
                        bitmap.set_pixel(x as u16, y as u16, 1);
                    }
                }
            }
            glyphs.push(BitmapGlyph { name: name.into(), encoding: Some(code), advance: 6, scalable_width: None, bitmap });
        }
        let font = BitmapFont {
            name: "test".into(),
            pixel_size: 12,
            point_size: 12.0,
            resolution: (96, 96),
            depth: 1,
            ascent: 10,
            descent: 2,
            properties: Vec::new(),
            glyphs,
        };
        let bytes = write_pk(&font).unwrap();
        assert!(bytes.len().is_multiple_of(4));
        let back = read_pk(&bytes).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        let back = back.font;
        assert_eq!(back.pixel_size, 12);
        assert_eq!(back.resolution, (96, 96));
        for (a, b) in font.glyphs.iter().zip(&back.glyphs) {
            assert_eq!(a.encoding, b.encoding);
            assert_eq!(a.advance, b.advance);
            assert_eq!(rows(&a.bitmap), rows(&b.bitmap), "{}", a.name);
            if a.bitmap.height > 0 {
                assert_eq!((a.bitmap.left, a.bitmap.bottom), (b.bitmap.left, b.bitmap.bottom));
            }
        }
        assert_eq!(back.glyphs[0].scalable_width, Some(500));
    }
}
