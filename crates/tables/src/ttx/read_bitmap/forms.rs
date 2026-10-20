//! TTX reader: bitmap image data in the forms `ttx -z` writes (M12.7e). `rawimagedata` holds
//! the image as hex; `rowimagedata` holds one hex `row` element per pixel row; and
//! `bitwiseimagedata` holds one `row` per pixel row with a character for each bit. Ported from
//! fontTools' `ttLib.tables.E_B_D_T_` (MIT): `_readRowImageData`, `_readBitwiseImageData`,
//! `_binary2data`, `_reverseBytes` and the `setRows` of `ByteAlignedBitmapMixin` and
//! `BitAlignedBitmapMixin`.

use crate::{Result, TablesError};

use super::super::Element;
use super::super::read::{int_lit, read_hex};

/// The widest row the image forms may describe, in pixels: the range of the width in a glyph
/// metrics record.
pub(crate) const MAX_ROW_PIXELS: i64 = 255;
/// The most `row` elements one image may hold: the range of the height in a glyph metrics
/// record.
pub(crate) const MAX_ROWS: usize = 255;
/// The deepest pixel the image forms may describe, in bits (colour strikes use 32).
pub(crate) const MAX_BIT_DEPTH: i64 = 32;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

/// The image bytes of `data`, the image data element of a glyph in image format `format`.
pub(crate) fn image_data(data: &Element, format: i64) -> Result<Vec<u8>> {
    match data.name.as_str() {
        "rawimagedata" => read_hex(&data.text),
        "rowimagedata" | "bitwiseimagedata" if format >= 17 => {
            Err(TablesError::Unsupported(format!("a colour bitmap (image format {format}) given as '{}'; only 'rawimagedata' is read", data.name)))
        }
        "rowimagedata" => rows(data, format, false),
        "bitwiseimagedata" => rows(data, format, true),
        // The file is read in, as `rawimagedata`, by whoever reads the dump from its folder.
        "extfileimagedata" => Err(TablesError::Unsupported(
            "a bitmap image kept in an external file ('extfileimagedata') is read only with the dump file, from the dump's folder".into(),
        )),
        other => Err(TablesError::Unsupported(format!("bitmap images given as '{other}'"))),
    }
}

/// An attribute of a row or bitwise image element, as an integer.
fn number(data: &Element, name: &str) -> Result<i64> {
    int_lit(data.attribute(name).ok_or_else(|| bad(format!("'{}' without its '{name}' attribute", data.name)))?)
}

/// fontTools' `_reverseBytes` for one byte: the bit order reversed.
fn reverse(byte: u8) -> u8 {
    byte.reverse_bits()
}

/// A bitwise row as bytes, the first character in the lowest bit of the first byte
/// (`_binary2data`): a space, a full stop and `0` are clear bits, any other character a set bit.
fn binary_row(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len().div_ceil(8));
    let (mut byte, mut bits) = (0u8, 0u32);
    for c in text.chars() {
        if !matches!(c, ' ' | '.' | '0') {
            byte |= 1 << bits;
        }
        bits += 1;
        if bits == 8 {
            out.push(byte);
            (byte, bits) = (0, 0);
        }
    }
    if bits > 0 {
        out.push(byte);
    }
    out
}

/// The image of a row or bitwise element (`bitwise` set), packed as image format `format` packs
/// its pixels: formats 1 and 6 start every row on a byte, formats 2, 5 and 7 run the rows
/// together bit by bit.
fn rows(data: &Element, format: i64, bitwise: bool) -> Result<Vec<u8>> {
    let depth = number(data, "bitDepth")?;
    let width = number(data, "width")?;
    // Read and checked as fontTools reads it; the rows themselves give the height.
    number(data, "height")?;
    if !(1..=MAX_BIT_DEPTH).contains(&depth) {
        return Err(bad(format!("a bitmap image of bit depth {depth}; it must be 1 to {MAX_BIT_DEPTH}")));
    }
    if !(0..=MAX_ROW_PIXELS).contains(&width) {
        return Err(bad(format!("a bitmap image {width} pixels wide; it must be 0 to {MAX_ROW_PIXELS}")));
    }
    let mut lines = Vec::new();
    for row in data.children.iter().filter(|c| c.name == "row") {
        if lines.len() >= MAX_ROWS {
            return Err(TablesError::TooLarge { what: "the rows of a bitmap image", limit: MAX_ROWS });
        }
        let value = row.attribute("value").ok_or_else(|| bad("a bitmap image row without a value"))?;
        // Each row as bytes whose first pixel is in the lowest bit, as the bit packing wants it.
        let bytes = if bitwise { binary_row(value) } else { read_hex(value)?.into_iter().map(reverse).collect() };
        lines.push(bytes);
    }
    let packed = match format {
        1 | 6 => lines.into_iter().flatten().map(reverse).collect(),
        2 | 5 | 7 => bit_rows(&lines, usize::try_from(depth * width).unwrap_or(0)),
        _ => return Err(bad(format!("bitmap image format {format} has no row data"))),
    };
    Ok(packed)
}

/// `BitAlignedBitmapMixin.setRows`: rows of `row_bits` bits each, laid end to end. A row's bits
/// past `row_bits` that share its last byte spill into the start of the next row, as they do in
/// fontTools; `ttx` never writes such bits.
fn bit_rows(lines: &[Vec<u8>], row_bits: usize) -> Vec<u8> {
    // Both factors are capped: at most 255 rows of 255 pixels of 32 bits.
    let total = (lines.len() * row_bits).div_ceil(8);
    let mut out = vec![0u8; total];
    for (row, data) in lines.iter().enumerate() {
        let start = row * row_bits;
        let end = start + row_bits;
        for (cur, &byte) in (start..end).step_by(8).zip(data) {
            let stop = (cur + 8).min(end);
            let cut = cur % 8;
            let (first, second) = (cur / 8, stop / 8);
            let taken = if first < second { 8 - cut } else { stop - cur };
            let byte = u32::from(byte);
            if let Some(o) = out.get_mut(first) {
                *o |= ((byte & ((1 << taken) - 1)) << cut) as u8;
            }
            if first < second
                && let Some(o) = out.get_mut(second)
            {
                *o |= ((byte >> taken) & ((1 << (8 - taken)) - 1)) as u8;
            }
        }
    }
    out.into_iter().map(reverse).collect()
}
