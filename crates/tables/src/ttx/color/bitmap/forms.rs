//! TTX writer: bitmap images in the forms `ttx -z` writes (M12.7f). Ported from fontTools'
//! `ttLib.tables.E_B_D_T_` and `C_B_D_T_` (MIT): `_writeRawImageData`, `_writeRowImageData`,
//! `_writeBitwiseImageData`, `_writeExtFileImageData`, `_extFileName`, `_data2binary` and the
//! `getRow` of `ByteAlignedBitmapMixin` and `BitAlignedBitmapMixin`.
//!
//! The row and bitwise forms describe each pixel row: `row` gives it as hex, `bitwise` as one
//! character a pixel (`@` set, `.` clear), reading the image as if its depth were one bit, as
//! fontTools does. Colour images (formats 17, 18 and 19) have no rows and are written raw. The
//! external-file form names `bitmaps/strike<N>/<glyph><ext>`, relative to the dump's folder, and
//! hands the image to the caller through [`XmlWriter::image_file`].

use crate::{Result, TablesError};

use crate::ttx::{BitmapForm, XmlWriter};

/// A glyph's image and what the row forms need to lay it out.
pub(super) struct Image<'a> {
    /// The image bytes, after the glyph's metrics.
    pub bytes: &'a [u8],
    pub format: u16,
    /// The index of the strike, which names the external file's folder.
    pub strike: usize,
    /// The strike's bit depth.
    pub bit_depth: u8,
    /// Height and width: the glyph's own metrics, else those its index subtable gives.
    pub size: Option<(u8, u8)>,
    pub glyph: &'a str,
}

/// Writes the image data element of `image` in the writer's bitmap form.
pub(super) fn write_image(w: &mut XmlWriter, image: &Image<'_>) -> Result<()> {
    let colour = image.format >= 17;
    let form = match w.bitmap_form {
        // fontTools has no rows for colour images: it writes them raw in the row form and
        // fails in the bitwise form.
        BitmapForm::Row | BitmapForm::Bitwise if colour => BitmapForm::Raw,
        form => form,
    };
    match form {
        BitmapForm::Raw => {
            w.open("rawimagedata", &[]);
            w.hex(image.bytes);
            w.close("rawimagedata");
        }
        BitmapForm::Row | BitmapForm::Bitwise => {
            let bitwise = form == BitmapForm::Bitwise;
            let (height, width) = image.size.ok_or_else(|| {
                TablesError::invalid(format!(
                    "the format {} bitmap of '{}' has no metrics, in itself or in its index subtable",
                    image.format, image.glyph
                ))
            })?;
            let element = if bitwise { "bitwiseimagedata" } else { "rowimagedata" };
            let attrs = [("bitDepth", image.bit_depth.to_string()), ("height", height.to_string()), ("width", width.to_string())];
            let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.open(element, &attrs);
            for row in 0..usize::from(height) {
                let value = if bitwise {
                    binary(&get_row(image, row, 1, width, true)?, usize::from(width))
                } else {
                    hex_str(&get_row(image, row, image.bit_depth, width, false)?)
                };
                w.simple("row", &[("value", &value)]);
            }
            w.check()?;
            w.close(element);
        }
        BitmapForm::ExtFile => {
            let ext = if colour { ".png" } else { ".bin" };
            let path = format!("bitmaps/strike{}/{}{ext}", image.strike, file_name(image.glyph));
            w.simple("extfileimagedata", &[("value", &path)]);
            w.image_file(path, image.bytes.to_vec());
        }
    }
    Ok(())
}

/// Row `row` of `image` read at `depth` bits a pixel, `width` pixels: the first pixel in the
/// highest bit of the first byte, or in the lowest with `reverse`. Formats 1 and 6 start every
/// row on a byte (a short image gives short rows); formats 2, 5 and 7 run the rows together, and
/// a row past the end of the image is an error, where fontTools fails.
fn get_row(image: &Image<'_>, row: usize, depth: u8, width: u8, reverse: bool) -> Result<Vec<u8>> {
    let row_bits = usize::from(depth) * usize::from(width);
    let data = image.bytes;
    let out: Vec<u8> = match image.format {
        1 | 6 => {
            let row_bytes = row_bits.div_ceil(8);
            let start = row.saturating_mul(row_bytes).min(data.len());
            let end = start.saturating_add(row_bytes).min(data.len());
            data.get(start..end).unwrap_or_default().to_vec()
        }
        2 | 5 | 7 => {
            let start = row * row_bits;
            let end = start + row_bits;
            let mut out = Vec::with_capacity(row_bits.div_ceil(8));
            for cur in (start..end).step_by(8) {
                let stop = (cur + 8).min(end);
                let bits = stop - cur;
                let cut = cur % 8;
                let (first, second) = (cur / 8, stop / 8);
                let taken = if first < second { 8 - cut } else { stop - cur };
                let Some(&byte) = data.get(first) else {
                    return Err(TablesError::invalid(format!("the bitmap of '{}' is shorter than its rows", image.glyph)));
                };
                let first_half = (u32::from(byte.reverse_bits()) >> cut) & ((1 << taken) - 1);
                let mut value = first_half;
                if first < second
                    && let Some(&next) = data.get(second)
                {
                    value = (first_half | (u32::from(next.reverse_bits()) << taken)) & ((1 << bits) - 1);
                }
                out.push(value as u8);
            }
            // The bytes come out with the first pixel lowest; fontTools turns them round.
            return Ok(if reverse { out } else { out.into_iter().map(u8::reverse_bits).collect() });
        }
        other => return Err(TablesError::invalid(format!("bitmap image format {other} has no rows"))),
    };
    Ok(if reverse { out.into_iter().map(u8::reverse_bits).collect() } else { out })
}

/// fontTools' `hexStr`: lowercase hex digits, nothing between them.
fn hex_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `_data2binary` with `@` for a set bit and `.` for a clear one: the lowest bit of each byte
/// first, `bits` characters at most.
fn binary(bytes: &[u8], bits: usize) -> String {
    let mut out = String::with_capacity(bits.min(bytes.len() * 8));
    let mut left = bits;
    for &byte in bytes {
        let take = left.min(8);
        for i in 0..take {
            out.push(if (byte >> i) & 1 == 1 { '@' } else { '.' });
        }
        left -= take;
    }
    out
}

/// `_extFileName`: the glyph name as it is, unless it is empty, `.` or `..` or holds `%`, `/`
/// or `\`; then `%` followed by the name's UTF-8 bytes percent-encoded, every byte but letters,
/// digits and `_.-~` written `%XX`. No such name can leave the strike's folder, and no two
/// names share a file.
pub(crate) fn file_name(glyph: &str) -> String {
    if !matches!(glyph, "" | "." | "..") && !glyph.contains(['%', '/', '\\']) {
        return glyph.to_string();
    }
    let mut out = String::from("%");
    for byte in glyph.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn image_file_names_cannot_leave_their_folder() {
        // As fontTools' `_extFileName` names them: names that could not be file names are
        // percent-encoded after a `%`.
        for (glyph, file) in
            [("A", "A"), (".notdef", ".notdef"), ("", "%"), (".", "%."), ("..", "%.."), ("a/b", "%a%2Fb"), ("50%", "%50%25"), ("é\\", "%%C3%A9%5C")]
        {
            assert_eq!(file_name(glyph), file, "{glyph}");
        }
    }

    #[test]
    fn a_bit_aligned_image_shorter_than_its_rows_is_an_error() {
        // Three rows of six bits need three bytes; with one, the second row takes the bits
        // there are and the third fails, as in fontTools.
        let image = Image { bytes: &[0b1011_0110], format: 2, strike: 0, bit_depth: 2, size: Some((3, 3)), glyph: "A" };
        assert_eq!(get_row(&image, 0, 2, 3, false).unwrap(), [0b1011_0100]);
        assert_eq!(get_row(&image, 1, 2, 3, false).unwrap(), [0b1000_0000]);
        assert!(get_row(&image, 2, 2, 3, false).unwrap_err().to_string().contains("shorter than its rows"));
        // A byte-aligned image gives short rows instead.
        let byte_aligned = Image { format: 1, ..image };
        assert_eq!(get_row(&byte_aligned, 0, 2, 3, false).unwrap(), [0b1011_0110]);
        assert_eq!(get_row(&byte_aligned, 1, 2, 3, false).unwrap(), Vec::<u8>::new());
    }
}
