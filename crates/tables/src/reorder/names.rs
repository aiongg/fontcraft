//! Glyph names of a compiled font, and the `post` table that stores them.

use std::collections::{HashMap, HashSet};

use read_fonts::tables::post::DEFAULT_GLYPH_NAMES;

use super::be::{Data, put_u16, put_u32, to_u16};
use super::cff;
use crate::limits::MAX_GLYPHS;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// `maxp.numGlyphs`.
pub(crate) fn num_glyphs(font: &SfntFont) -> Result<usize> {
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let n = usize::from(Data::new(*b"maxp", maxp).u16(4)?);
    if n == 0 {
        return Err(TablesError::invalid("the font has no glyphs"));
    }
    Ok(n)
}

/// The glyph names, by glyph id, the way fontTools derives its glyph order: the charset of a
/// `CFF ` table, else `post` names, else `glyphNNNNN` (glyph 0 is `.notdef`). A name that
/// repeats gets `#1`, `#2`… appended, so every name is unique.
pub(crate) fn glyph_names(font: &SfntFont) -> Result<Vec<String>> {
    let n = num_glyphs(font)?;
    let names = if let Some(cff_bytes) = font.get(*b"CFF ") {
        cff::parse(cff_bytes, n)?.glyph_names()
    } else {
        match font.get(*b"post").map(|p| post_names(p, n)).transpose()? {
            Some(Some(names)) => names,
            _ => fallback_names(n),
        }
    };
    Ok(unique(names))
}

fn fallback_names(n: usize) -> Vec<String> {
    (0..n).map(|g| if g == 0 { ".notdef".to_string() } else { format!("glyph{g:05}") }).collect()
}

fn unique(names: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::with_capacity(names.len());
    let mut counts: HashMap<String, usize> = HashMap::new();
    names
        .into_iter()
        .map(|name| {
            if seen.insert(name.clone()) {
                return name;
            }
            let count = counts.entry(name.clone()).or_insert(0);
            loop {
                *count += 1;
                let candidate = format!("{name}#{count}");
                if seen.insert(candidate.clone()) {
                    return candidate;
                }
            }
        })
        .collect()
}

/// Names stored in `post`: version 1 (the standard Macintosh order) or 2. `None` for versions
/// that store no names.
pub(crate) fn post_names(post: &[u8], n: usize) -> Result<Option<Vec<String>>> {
    let d = Data::new(*b"post", post);
    match d.u32(0)? {
        0x0001_0000 => Ok(Some((0..n).map(|g| DEFAULT_GLYPH_NAMES.get(g).map_or_else(|| format!("glyph{g:05}"), |s| s.to_string())).collect())),
        0x0002_0000 => {
            let count = usize::from(d.u16(32)?);
            let mut indices = Vec::with_capacity(count);
            for g in 0..count {
                indices.push(usize::from(d.u16(34 + 2 * g)?));
            }
            let mut strings = Vec::new();
            let mut at = 34 + 2 * count;
            while at < post.len() && strings.len() < MAX_GLYPHS {
                let len = usize::from(d.u8(at)?);
                strings.push(String::from_utf8_lossy(d.slice(at + 1, len)?).into_owned());
                at += 1 + len;
            }
            let names = (0..n)
                .map(|g| match indices.get(g) {
                    Some(i) if *i < DEFAULT_GLYPH_NAMES.len() => DEFAULT_GLYPH_NAMES.get(*i).map_or_else(String::new, |s| s.to_string()),
                    Some(i) => strings.get(i - DEFAULT_GLYPH_NAMES.len()).cloned().unwrap_or_default(),
                    None => String::new(),
                })
                .enumerate()
                .map(|(g, name)| if name.is_empty() { fallback_name(g) } else { name })
                .collect();
            Ok(Some(names))
        }
        _ => Ok(None),
    }
}

fn fallback_name(g: usize) -> String {
    if g == 0 { ".notdef".to_string() } else { format!("glyph{g:05}") }
}

/// A `post` version 2 table with `names` (by glyph id), keeping the 32-byte header of `post`.
pub(crate) fn write_post_v2(post: &[u8], names: &[String]) -> Result<Vec<u8>> {
    let header = Data::new(*b"post", post).slice(4, 28)?;
    let mut out = Vec::with_capacity(34 + names.len() * 12);
    put_u32(&mut out, 0x0002_0000);
    out.extend_from_slice(header);
    put_u16(&mut out, to_u16(names.len(), "the glyph count")?);
    let standard: HashMap<&str, usize> = DEFAULT_GLYPH_NAMES.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut extra: Vec<&str> = Vec::new();
    let mut extra_at: HashMap<&str, usize> = HashMap::new();
    for name in names {
        let index = match standard.get(name.as_str()) {
            Some(i) => *i,
            None => {
                let next = extra.len();
                let i = *extra_at.entry(name.as_str()).or_insert(next);
                if i == next {
                    extra.push(name.as_str());
                }
                DEFAULT_GLYPH_NAMES.len() + i
            }
        };
        put_u16(&mut out, to_u16(index, "the post name index")?);
    }
    for name in extra {
        let len = u8::try_from(name.len()).map_err(|_| TablesError::BadOptions(format!("the glyph name `{name}` is longer than 255 bytes")))?;
        out.push(len);
        out.extend_from_slice(name.as_bytes());
    }
    Ok(out)
}

/// `post` with its names in the new order: version 2 is permuted, version 1 becomes version 2,
/// versions without names are kept (`None`).
pub(crate) fn reorder_post(post: &[u8], new_to_old: &[u16]) -> Result<Option<Vec<u8>>> {
    if Data::new(*b"post", post).u32(0)? == 0x0002_5000 {
        return Err(TablesError::Unsupported("a post table of version 2.5".into()));
    }
    let Some(names) = post_names(post, new_to_old.len())? else { return Ok(None) };
    let reordered: Vec<String> = new_to_old.iter().map(|o| names.get(usize::from(*o)).cloned().unwrap_or_default()).collect();
    Ok(Some(write_post_v2(post, &reordered)?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn post_v2_round_trip_with_standard_and_repeated_names() {
        let mut post = vec![0u8; 32];
        post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        post[8] = 0xFF;
        let names: Vec<String> = [".notdef", "A", "a.alt", "a.alt", "zero"].iter().map(|s| s.to_string()).collect();
        let v2 = write_post_v2(&post, &names).unwrap();
        assert_eq!(&v2[4..32], &post[4..32], "the header after the version is kept");
        let back = post_names(&v2, 5).unwrap().unwrap();
        assert_eq!(back, names);
        assert_eq!(unique(back), [".notdef", "A", "a.alt", "a.alt#1", "zero"]);
        assert_eq!(v2.len(), 34 + 10 + 6, "one stored string for the two a.alt glyphs");
    }
}
