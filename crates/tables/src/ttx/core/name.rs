//! `name`: the records in file order, each decoded as fontTools decodes it (with its repairs of
//! badly encoded UTF-16), or written as escaped bytes when it cannot be decoded.

use crate::Result;

use super::super::{XmlWriter, py_hex};
use super::Rd;

/// The legacy Macintosh Roman characters for bytes 0x80 to 0xFF, as Python's `mac_roman` codec
/// maps them.
const MAC_ROMAN_HIGH: &str =
    "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{a0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf16,
    Ascii,
    Latin1,
    MacRoman,
    /// A legacy encoding decoded here only when every byte is ASCII.
    AsciiCompatible,
}

/// fontTools' `getEncoding` for a record; `None` where it knows no encoding.
fn encoding(platform: u16, encoding: u16, language: u16) -> Option<Encoding> {
    use Encoding::*;
    match (platform, encoding) {
        (0, 0..=6) => Some(Utf16),
        (1, 0) => Some(match language {
            15 | 17 | 18 | 24..=28 | 36..=40 => AsciiCompatible,
            _ => MacRoman,
        }),
        (1, 1 | 2 | 3 | 6 | 7 | 25 | 29 | 35 | 37) => Some(AsciiCompatible),
        (2, 0) => Some(Ascii),
        (2, 1) => Some(Utf16),
        (2, 2) => Some(Latin1),
        (3, 0 | 1 | 10) => Some(Utf16),
        (3, 2..=6) => Some(AsciiCompatible),
        _ => None,
    }
}

fn is_ascii_text(b: u32) -> bool {
    (0x20..=0x7E).contains(&b) || matches!(b, 0x09 | 0x0A | 0x0D)
}

/// fontTools' `toUnicode`; `None` when the bytes do not decode.
fn decode(bytes: &[u8], enc: Option<Encoding>) -> Option<String> {
    let enc = enc.unwrap_or(Encoding::Ascii);
    let mut bytes = bytes.to_vec();
    if enc == Encoding::Utf16 && bytes.len() % 2 == 1 {
        if bytes.last() == Some(&0) {
            bytes.pop();
        } else if bytes.iter().enumerate().all(|(i, b)| if i % 2 == 1 { *b == 0 } else { is_ascii_text(u32::from(*b)) }) {
            bytes.insert(0, 0);
        } else if bytes.first() == Some(&0) && bytes.iter().skip(1).all(|b| is_ascii_text(u32::from(*b))) {
            bytes = bytes.iter().skip(1).flat_map(|b| [0, *b]).collect();
        }
    }
    let text: String = match enc {
        Encoding::Utf16 => {
            if bytes.len() % 2 != 0 {
                return None;
            }
            let units = bytes.chunks(2).map(|p| u16::from_be_bytes([p.first().copied().unwrap_or(0), p.get(1).copied().unwrap_or(0)]));
            char::decode_utf16(units).collect::<std::result::Result<String, _>>().ok()?
        }
        Encoding::Latin1 => bytes.iter().map(|&b| b as char).collect(),
        Encoding::MacRoman => bytes
            .iter()
            .map(|&b| if b < 0x80 { Some(b as char) } else { MAC_ROMAN_HIGH.chars().nth(usize::from(b - 0x80)) })
            .collect::<Option<String>>()?,
        Encoding::Ascii | Encoding::AsciiCompatible => {
            if !bytes.is_ascii() {
                return None;
            }
            bytes.iter().map(|&b| b as char).collect()
        }
    };
    let looks_double = text.chars().enumerate().all(|(i, c)| if i % 2 == 0 { c == '\0' } else { is_ascii_text(c as u32) });
    if looks_double {
        return Some(text.chars().skip(1).step_by(2).collect());
    }
    Some(text)
}

/// fontTools' `escape8bit`: printable ASCII as is, everything else as a character reference.
fn escape_8bit(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if (32..=127).contains(&b) && !matches!(b, b'<' | b'&' | b'>') {
            out.push(b as char);
        } else {
            out.push_str(&format!("&#{b};"));
        }
    }
    out
}

pub(super) fn write(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let mut r = Rd::new(data);
    let _format = r.u16()?;
    let count = r.u16()?;
    let string_offset = usize::from(r.u16()?);
    let strings = data.get(string_offset..).unwrap_or_default();
    let mut records = Rd::at(data, 6);
    for _ in 0..count {
        if records.rest().len() < 12 {
            continue;
        }
        let platform = records.u16()?;
        let enc_id = records.u16()?;
        let language = records.u16()?;
        let name_id = records.u16()?;
        let length = usize::from(records.u16()?);
        let offset = usize::from(records.u16()?);
        let Some(bytes) = strings.get(offset..offset.saturating_add(length)) else { continue };
        let enc = encoding(platform, enc_id, language);
        let text = decode(bytes, enc);
        let (name_id, platform_s, enc_s, lang) = (name_id.to_string(), platform.to_string(), enc_id.to_string(), py_hex(i64::from(language)));
        let mut attrs: Vec<(&str, &str)> = vec![("nameID", &name_id), ("platformID", &platform_s), ("platEncID", &enc_s), ("langID", &lang)];
        let compatible = matches!(enc, Some(Encoding::Utf16 | Encoding::Ascii | Encoding::Latin1));
        if text.is_none() || !compatible {
            attrs.push(("unicode", if text.is_some() { "True" } else { "False" }));
        }
        w.open("namerecord", &attrs);
        match &text {
            Some(t) => w.text(t),
            None => w.text_raw(&escape_8bit(bytes), &bytes.iter().map(|&b| b as char).collect::<String>()),
        }
        w.close("namerecord");
    }
    Ok(())
}
