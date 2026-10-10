//! Glyph names to text by the Adobe Glyph List specification.
//!
//! Ported from fontTools' `agl.toUnicode` (MIT) by way of fontc's `fontdrasil::agl` (fontdrasil
//! 1.0.0, MIT OR Apache-2.0): drop everything from the first `.`, split the rest at `_`, and map
//! each part through the glyph list, the `uniXXXX[YYYY…]` form or the `uXXXX`–`uXXXXXX` form. A
//! part that matches none maps to nothing. The Zapf Dingbats list is not used.

use crate::tables::Tables;

/// The characters a glyph name stands for (`f_f_i.alt` → "ffi").
pub fn to_unicode(tables: &Tables, name: &str) -> Vec<char> {
    let base = name.split_once('.').map_or(name, |(b, _)| b);
    base.split('_').flat_map(|part| part_to_unicode(tables, part)).collect()
}

fn part_to_unicode(tables: &Tables, part: &str) -> Vec<char> {
    if let Some(cps) = tables.agl(part) {
        return cps.into_iter().filter_map(char::from_u32).collect();
    }
    if let Some(chars) = uni_to_unicode(part) {
        return chars;
    }
    u_to_unicode(part).map(|c| vec![c]).unwrap_or_default()
}

fn is_upper_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'A'..=b'F').contains(&b)
}

/// `uni` followed by groups of four uppercase hex digits, none a surrogate.
fn uni_to_unicode(part: &str) -> Option<Vec<char>> {
    let digits = part.strip_prefix("uni")?;
    if !digits.bytes().all(is_upper_hex) || digits.len() % 4 != 0 {
        return None;
    }
    digits
        .as_bytes()
        .chunks(4)
        .map(|group| {
            let v = u32::from_str_radix(std::str::from_utf8(group).ok()?, 16).ok()?;
            if (0xD800..=0xDFFF).contains(&v) { None } else { char::from_u32(v) }
        })
        .collect()
}

/// `u` followed by four to six uppercase hex digits naming a Unicode scalar value.
fn u_to_unicode(part: &str) -> Option<char> {
    let digits = part.strip_prefix('u')?;
    if !digits.bytes().all(is_upper_hex) || !(4..=6).contains(&digits.len()) {
        return None;
    }
    char::from_u32(u32::from_str_radix(digits, 16).ok()?)
}

#[cfg(test)]
mod tests {
    // The specification's examples, as fontTools' tests state them.
    #[test]
    fn specification_examples() {
        let t = crate::tables::builtin().unwrap();
        for (name, text) in [
            ("Lcommaaccent", "Ļ"),
            ("uni20AC0308", "\u{20AC}\u{0308}"),
            ("u1040C", "\u{1040C}"),
            ("uniD801DC0C", ""),
            ("uni20ac", ""),
            ("Lcommaaccent_uni20AC0308_u1040C.alternate", "\u{013B}\u{20AC}\u{0308}\u{1040C}"),
            ("foo", ""),
            (".notdef", ""),
            ("longs_t", "ſt"),
            ("f_f_i.alt123", "ffi"),
            ("uni0041_uni0042_uni0043", "ABC"),
            ("uni004101", ""),
            ("u0000041", ""),
            ("u0041_uni0041_A.alt", "AAA"),
            ("union", "∪"),
            ("", ""),
        ] {
            assert_eq!(super::to_unicode(t, name).into_iter().collect::<String>(), text, "{name}");
        }
    }
}
