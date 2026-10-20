//! METAFONT generic font files (GF): reading.
//!
//! Written from the format description in Knuth's GFtype documentation (part of the TeX
//! distribution's public program documentation) and from the files METAFONT writes, and `gftype`
//! prints, for programs of our own. No code that reads or writes GF was consulted.
//!
//! A GF file is a stream of commands: a preamble, one `boc` … `eoc` block per character that
//! paints its black pixels row by row from the top, and a postamble with the design size, the
//! pixels per point and one locator per character code with its escapement and TeX width. Pixel
//! (m, n) is the unit square whose lower-left corner is (m, n), so a character's columns run from
//! `min_m` to `max_m` - 1 and its rows from `max_n` down to `min_n`.

use std::collections::HashMap;

use crate::binary::{Cursor, Scale, code_name, set_bits};
use crate::pk::tex_font;
use crate::{Bitmap, BitmapError, BitmapGlyph, Import, MAX_BITMAP_SIDE, MAX_COORDINATE, MAX_FILE_BYTES, MAX_GLYPHS, PixelBudget, Result, Warnings};

pub(crate) const PRE: u8 = 247;
pub(crate) const ID: u8 = 131;
const PAINT1: u8 = 64;
const PAINT3: u8 = 66;
const BOC: u8 = 67;
const BOC1: u8 = 68;
const EOC: u8 = 69;
const SKIP0: u8 = 70;
const SKIP1: u8 = 71;
const SKIP3: u8 = 73;
const NEW_ROW_0: u8 = 74;
const NEW_ROW_164: u8 = 238;
const XXX1: u8 = 239;
const XXX4: u8 = 242;
const YYY: u8 = 243;
const NO_OP: u8 = 244;
const CHAR_LOC: u8 = 245;
const CHAR_LOC0: u8 = 246;
const POST: u8 = 248;
const POST_POST: u8 = 249;

/// Reads a GF file.
pub fn read_gf(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    let mut c = Cursor::new(bytes, "GF");
    if c.u8()? != PRE || c.u8()? != ID {
        return Err(c.error("not a GF file: it does not start with a GF preamble"));
    }
    let comment_len = c.u8()?;
    c.skip(usize::from(comment_len))?;
    let mut glyphs: Vec<BitmapGlyph> = Vec::new();
    let mut budget = PixelBudget::default();
    let mut warnings = Warnings::default();
    loop {
        let op = c.u8()?;
        match op {
            BOC | BOC1 => {
                if glyphs.len() >= MAX_GLYPHS {
                    return Err(BitmapError::Limit { what: "glyphs", limit: MAX_GLYPHS });
                }
                let (code, min_m, max_m, min_n, max_n) = if op == BOC {
                    let code = c.i32()?;
                    let _previous = c.i32()?;
                    (i64::from(code), i64::from(c.i32()?), i64::from(c.i32()?), i64::from(c.i32()?), i64::from(c.i32()?))
                } else {
                    let code = i64::from(c.u8()?);
                    let (del_m, max_m, del_n, max_n) = (i64::from(c.u8()?), i64::from(c.u8()?), i64::from(c.u8()?), i64::from(c.u8()?));
                    (code, max_m - del_m, max_m, max_n - del_n, max_n)
                };
                let Ok(code) = u32::try_from(code) else {
                    return Err(c.error(format!("character code {code} is negative")));
                };
                let bitmap = read_char(&mut c, (min_m, max_m, min_n, max_n), &mut budget).map_err(|e| match e {
                    BitmapError::Malformed { format, message } => BitmapError::Malformed { format, message: format!("character {code}: {message}") },
                    other => other,
                })?;
                glyphs.push(BitmapGlyph { name: code_name(code, false), encoding: Some(code), advance: 0, scalable_width: None, bitmap });
            }
            XXX1..=XXX4 => skip_special(&mut c, op)?,
            YYY => c.skip(4)?,
            NO_OP => {}
            POST => break,
            _ => return Err(c.error(format!("command {op} cannot stand between characters"))),
        }
    }
    let _last_eoc = c.i32()?;
    let design_size = c.i32()?;
    let _checksum = c.u32()?;
    let (hppp, vppp) = (c.i32()?, c.i32()?);
    let scale = Scale::new("GF", design_size, hppp, vppp)?;
    c.skip(16)?;
    // Escapement in pixels times 2^16 and TeX width, by character code modulo 256.
    let mut locators: HashMap<u32, (i64, i32)> = HashMap::new();
    loop {
        let op = c.u8()?;
        match op {
            CHAR_LOC => {
                let code = u32::from(c.u8()?);
                let dx = i64::from(c.i32()?);
                let _dy = c.i32()?;
                let width = c.i32()?;
                let _pointer = c.i32()?;
                locators.insert(code, (dx, width));
            }
            CHAR_LOC0 => {
                let code = u32::from(c.u8()?);
                let dx = i64::from(c.u8()?) << 16;
                let width = c.i32()?;
                let _pointer = c.i32()?;
                locators.insert(code, (dx, width));
            }
            NO_OP => {}
            POST_POST => break,
            _ => return Err(c.error(format!("command {op} cannot stand in the postamble"))),
        }
    }
    let mut unlocated = 0usize;
    for glyph in &mut glyphs {
        let residue = glyph.encoding.unwrap_or(0) % 256;
        match locators.get(&residue) {
            Some((dx, width)) => {
                let advance = (*dx as f64 / crate::binary::SCALED_ONE).round();
                glyph.advance = advance.clamp(-f64::from(MAX_COORDINATE), f64::from(MAX_COORDINATE)) as i32;
                glyph.scalable_width = Some(Scale::thousandths(*width));
            }
            None => {
                unlocated += 1;
                glyph.advance = glyph.bitmap.left.saturating_add(i32::from(glyph.bitmap.width)).max(0);
            }
        }
    }
    if unlocated > 0 {
        warnings.push(format!("{unlocated} characters have no locator in the postamble; they advance by their bitmap's right edge"));
    }
    Ok(Import { font: tex_font(glyphs, &scale), warnings: warnings.finish() })
}

fn skip_special(c: &mut Cursor<'_>, op: u8) -> Result<()> {
    let len = c.uint(usize::from(op - XXX1) + 1)?;
    c.skip(usize::try_from(len).map_err(|_| c.error("a special is too long"))?)
}

/// Paints one character's pixels, from just after its `boc` through its `eoc`. A paint that
/// falls outside the box the `boc` states is refused.
fn read_char(c: &mut Cursor<'_>, (min_m, max_m, min_n, max_n): (i64, i64, i64, i64), budget: &mut PixelBudget) -> Result<Bitmap> {
    let coordinate = |v: i64| v.abs() <= i64::from(MAX_COORDINATE);
    if ![min_m, max_m, min_n, max_n].into_iter().all(coordinate) {
        return Err(c.error("its box is out of range"));
    }
    let (width, height) = ((max_m - min_m).max(0), (max_n - min_n + 1).max(0));
    let side = |v: i64| u16::try_from(v).ok().filter(|v| *v <= MAX_BITMAP_SIDE);
    let (Some(w), Some(h)) = (side(width), side(height)) else {
        return Err(c.error(format!("its box is {width} × {height} pixels; the limit is {MAX_BITMAP_SIDE}")));
    };
    let (w, h) = if w == 0 || h == 0 { (0, 0) } else { (w, h) };
    budget.spend(crate::stride(w, 1).saturating_mul(usize::from(h)))?;
    let mut bitmap = Bitmap::blank(w, h, min_m as i32, min_n as i32, 1).ok_or_else(|| c.error("its box is too large"))?;
    let stride = bitmap.stride();
    let (mut m, mut n, mut black) = (min_m, max_n, false);
    loop {
        let op = c.u8()?;
        let paint = match op {
            0..PAINT1 => Some(i64::from(op)),
            PAINT1..=PAINT3 => Some(i64::from(c.uint(usize::from(op - PAINT1) + 1)?)),
            EOC => break,
            SKIP0..=SKIP3 => {
                let rows = if op == SKIP0 { 0 } else { i64::from(c.uint(usize::from(op - SKIP1) + 1)?) };
                (m, n, black) = (min_m, n - rows - 1, false);
                None
            }
            NEW_ROW_0..=NEW_ROW_164 => {
                (m, n, black) = (min_m + i64::from(op - NEW_ROW_0), n - 1, true);
                None
            }
            XXX1..=XXX4 => {
                skip_special(c, op)?;
                None
            }
            YYY => {
                c.skip(4)?;
                None
            }
            NO_OP => None,
            _ => return Err(c.error(format!("command {op} cannot stand inside a character"))),
        };
        let Some(d) = paint else { continue };
        if black && d > 0 {
            if n < min_n || n > max_n || m < min_m || m + d > max_m || w == 0 {
                return Err(c.error("a paint runs outside the glyph box"));
            }
            let row = usize::try_from(max_n - n).unwrap_or(0);
            let start = usize::try_from(m - min_m).unwrap_or(0);
            if let Some(bytes) = bitmap.data.get_mut(row * stride..(row + 1) * stride) {
                set_bits(bytes, start, usize::try_from(d).unwrap_or(0));
            }
        }
        m += d;
        black = !black;
    }
    Ok(bitmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(b: &Bitmap) -> Vec<String> {
        (0..b.height).map(|y| (0..b.width).map(|x| if b.pixel(x, y) != 0 { '#' } else { '.' }).collect()).collect()
    }

    /// The preamble METAFONT writes, with a two-byte comment.
    fn preamble() -> Vec<u8> {
        vec![PRE, ID, 2, b'h', b'i']
    }

    /// The postamble: design size 10 points, one pixel per point, then `locators`.
    fn postamble(out: &mut Vec<u8>, locators: &[u8]) {
        out.push(POST);
        for v in [0i32, 10 << 20, 0, 1 << 16, 1 << 16, 0, 3, -2, 1] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(locators);
        out.extend([POST_POST, 0, 0, 0, 0, ID, 223, 223, 223, 223]);
    }

    /// The commands METAFONT writes for a 3 × 2 block on the baseline over a 1 × 2 stem below it
    /// (`gftype` shows them as: boc1 65, 0<=m<=3, -2<=n<=1; paint (0)3; newrow 0 paint 3;
    /// newrow 1 paint 1; newrow 1 paint 1; eoc), with escapement 4 pixels and TeX width 0.04.
    #[test]
    fn gf_reads_painted_characters() {
        let mut bytes = preamble();
        bytes.extend([BOC1, 65, 3, 3, 3, 1, 0, 3, NEW_ROW_0, 3, NEW_ROW_0 + 1, 1, NEW_ROW_0 + 1, 1, EOC]);
        // A second character with the long boc, a special and a skip over one blank row:
        // code 300, columns -1..=0, rows 2 down to 0; row 2 "#.", row 1 blank, row 0 ".#".
        bytes.push(BOC);
        for v in [300i32, -1, -1, 1, 0, 2] {
            bytes.extend_from_slice(&v.to_be_bytes());
        }
        // paint 0 turns the switch black, paint 1 blackens column -1; skip1 1 moves down two
        // rows to row 0; paint 1 (white), paint 1 (black) blackens column 0.
        bytes.extend([XXX1, 1, b'x', 0, 1, SKIP1, 1, 1, 1, EOC]);
        let mut locators = vec![CHAR_LOC0, 65, 4];
        locators.extend_from_slice(&41_943i32.to_be_bytes());
        locators.extend_from_slice(&0i32.to_be_bytes());
        postamble(&mut bytes, &locators);
        let import = read_gf(&bytes).unwrap();
        let font = import.font;
        assert_eq!(font.pixel_size, 10);
        let a = &font.glyphs[0];
        assert_eq!(rows(&a.bitmap), ["###", "###", ".#.", ".#."]);
        assert_eq!((a.bitmap.left, a.bitmap.bottom, a.advance, a.scalable_width), (0, -2, 4, Some(40)));
        let b = &font.glyphs[1];
        assert_eq!(b.encoding, Some(300));
        assert_eq!(rows(&b.bitmap), ["#.", "..", ".#"]);
        assert_eq!((b.bitmap.left, b.bitmap.bottom), (-1, 0));
        // Code 300 has no locator of its own (300 mod 256 = 44 has none either).
        assert_eq!(import.warnings.len(), 1);
    }

    #[test]
    fn gf_refuses_paint_outside_the_box_and_hostile_boxes() {
        let mut bytes = preamble();
        // A 3-column box with a black paint of 4.
        bytes.extend([BOC1, 65, 3, 3, 0, 0, 0, 4, EOC]);
        postamble(&mut bytes, &[]);
        let err = read_gf(&bytes).unwrap_err().to_string();
        assert!(err.contains("outside the glyph box"), "{err}");
        // A skip below the box, then a paint.
        let mut bytes = preamble();
        bytes.extend([BOC1, 65, 3, 3, 0, 0, SKIP0, NEW_ROW_0, 1, EOC]);
        postamble(&mut bytes, &[]);
        assert!(read_gf(&bytes).is_err());
        // A box 2,000,000 columns wide.
        let mut bytes = preamble();
        bytes.push(BOC);
        for v in [65i32, -1, 0, 2_000_000, 0, 0] {
            bytes.extend_from_slice(&v.to_be_bytes());
        }
        bytes.push(EOC);
        postamble(&mut bytes, &[]);
        assert!(read_gf(&bytes).is_err());
        // A file cut off inside a character.
        let mut bytes = preamble();
        bytes.extend([BOC1, 65, 3, 3, 0, 0, 0]);
        assert!(read_gf(&bytes).is_err());
    }
}
