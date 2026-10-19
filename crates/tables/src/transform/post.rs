//! A `post` table of version 2 written again the way fontTools writes one it has loaded: a port
//! of `decode_format_2_0`, `unpackPStrings` and `encode_format_2_0` from fontTools'
//! `ttLib/tables/_p_o_s_t.py` (fontTools 4.66.1, MIT License, Copyright (c) 2000 Just van
//! Rossum and the fontTools authors).
//!
//! Stored names that are standard Macintosh names leave the string list, and the glyphs that
//! use them point at the standard name instead. Strings past the highest index any glyph uses
//! are dropped; unused strings below it stay. Names are compared as bytes.

use std::collections::HashMap;

use read_fonts::tables::post::DEFAULT_GLYPH_NAMES;

use crate::{Result, TablesError};

const HEADER: usize = 32;
const STANDARD: usize = 258;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at.checked_add(2)?).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_be_bytes)
}

/// `post` as fontTools compiles it after loading it, for a font of `num_glyphs` glyphs. A table
/// of another version comes back as it is.
pub(crate) fn recompile(post: &[u8], num_glyphs: usize) -> Result<Vec<u8>> {
    let header = post.get(..HEADER).ok_or_else(|| TablesError::invalid("the 'post' table is cut short"))?;
    if header.get(..4) != Some(&[0, 2, 0, 0][..]) {
        return Ok(post.to_vec());
    }
    // decode_format_2_0: a stored count above the glyph count is taken to be wrong.
    let count = usize::from(u16_at(post, HEADER).ok_or_else(|| TablesError::invalid("the 'post' table is cut short"))?).min(num_glyphs);
    let mut indices = Vec::with_capacity(count);
    for g in 0..count {
        let at = HEADER + 2 + 2 * g;
        indices.push(usize::from(u16_at(post, at).ok_or_else(|| TablesError::invalid("the 'post' glyph name indices are cut short"))?));
    }
    let strings_at = HEADER + 2 + 2 * count;
    let data = post.get(strings_at..).unwrap_or_default();
    let max_index = indices.iter().copied().max().unwrap_or(0);
    let extra = unpack_strings(data, max_index.saturating_sub(STANDARD - 1));
    let ps_names: Vec<&[u8]> = (0..num_glyphs)
        .map(|g| match indices.get(g) {
            Some(&i) if i >= STANDARD => extra.get(i - STANDARD).copied().unwrap_or_default(),
            Some(&i) => DEFAULT_GLYPH_NAMES.get(i).map_or(&b""[..], |s| s.as_bytes()),
            None => &b""[..],
        })
        .collect();

    // encode_format_2_0
    let standard: HashMap<&[u8], usize> = DEFAULT_GLYPH_NAMES.iter().enumerate().map(|(i, s)| (s.as_bytes(), i)).collect();
    let mut kept: Vec<&[u8]> = extra.iter().copied().filter(|n| !standard.contains_key(n)).collect();
    let mut kept_at: HashMap<&[u8], usize> = kept.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let n = u16::try_from(num_glyphs).map_err(|_| TablesError::invalid("the font has more than 65,535 glyphs"))?;
    let mut out = Vec::with_capacity(strings_at + data.len());
    out.extend_from_slice(header);
    out.extend_from_slice(&n.to_be_bytes());
    for name in ps_names {
        let index = if let Some(i) = kept_at.get(name) {
            STANDARD + i
        } else if let Some(i) = standard.get(name) {
            *i
        } else {
            kept_at.insert(name, kept.len());
            kept.push(name);
            STANDARD + kept.len() - 1
        };
        let index = u16::try_from(index).map_err(|_| TablesError::invalid("the 'post' table would store more than 65,277 names"))?;
        out.extend_from_slice(&index.to_be_bytes());
    }
    for name in kept {
        out.push(u8::try_from(name.len()).map_err(|_| TablesError::invalid("a 'post' glyph name is longer than 255 bytes"))?);
        out.extend_from_slice(name);
    }
    Ok(out)
}

/// `unpackPStrings`: `n` Pascal strings, each empty where the data runs out.
fn unpack_strings(data: &[u8], n: usize) -> Vec<&[u8]> {
    let mut out = Vec::with_capacity(n.min(data.len().saturating_add(1)));
    let mut index = 0usize;
    for _ in 0..n {
        let length = data.get(index).map_or(0, |b| usize::from(*b));
        index = index.saturating_add(1);
        let end = index.saturating_add(length);
        out.push(if end > data.len() { &b""[..] } else { data.get(index..end).unwrap_or_default() });
        index = end;
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn post_v2(indices: &[u16], strings: &[&str]) -> Vec<u8> {
        let mut p = vec![0u8; HEADER];
        p[..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
        p[8..10].copy_from_slice(&(-100i16).to_be_bytes());
        p.extend_from_slice(&(indices.len() as u16).to_be_bytes());
        for i in indices {
            p.extend_from_slice(&i.to_be_bytes());
        }
        for s in strings {
            p.push(s.len() as u8);
            p.extend_from_slice(s.as_bytes());
        }
        p
    }

    #[test]
    fn standard_names_stored_as_strings_move_to_their_standard_index() {
        // `.null` and `A` stored as strings; `zz` unused below the highest index, `tail` past it.
        let post = post_v2(&[0, 258, 259, 261], &[".null", "A", "zz", "a.alt", "tail"]);
        let out = recompile(&post, 4).unwrap();
        assert_eq!(out, post_v2(&[0, 1, 36, 259], &["zz", "a.alt"]));
        assert_eq!(&out[..HEADER], &post[..HEADER], "the header is kept");
    }

    #[test]
    fn other_versions_and_clean_tables_are_kept() {
        let mut v3 = vec![0u8; HEADER];
        v3[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        assert_eq!(recompile(&v3, 3).unwrap(), v3);
        let clean = post_v2(&[0, 3, 258], &["a.alt"]);
        assert_eq!(recompile(&clean, 3).unwrap(), clean);
    }

    #[test]
    fn a_short_index_list_names_the_rest_with_an_empty_string() {
        let out = recompile(&post_v2(&[0], &[]), 2).unwrap();
        assert_eq!(out, post_v2(&[0, 258], &[""]));
    }
}
