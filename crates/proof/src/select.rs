//! Which glyphs a proof shows, and in what order (`--glyphs`, `--exclude-glyphs`, `--sort`).

use std::collections::HashMap;

use crate::{MAX_SHEET_GLYPHS, ProofError};

/// Most glyph indices a selection may walk through, ranges included.
const MAX_SELECTION_WORK: usize = 16 * MAX_SHEET_GLYPHS;

/// The order glyphs are shown in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphSort {
    /// Glyph order.
    #[default]
    Gid,
    /// Alphabetical by name.
    Name,
    /// By code point; glyphs without one follow in glyph order.
    Unicode,
}

/// A glyph selection. Each item is a glyph name or a range of two names, `a-b` or `a..b`, taken
/// by glyph index; a name that exists as written wins over reading it as a range.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Glyphs to show; empty shows them all.
    pub glyphs: Vec<String>,
    pub exclude: Vec<String>,
    pub sort: GlyphSort,
}

/// The names of the glyphs `selection` picks from face `face` of `bytes`, in its order, for
/// [`crate::ProofOptions::glyphs`].
pub fn select_glyphs(bytes: &[u8], face: u32, selection: &Selection) -> Result<Vec<String>, ProofError> {
    let loaded = crate::font::load(bytes, face)?;
    let font = crate::font::font_ref(&loaded.sfnt)?;
    let location = skrifa::instance::Location::default();
    let face = crate::font::Face::new(font, &location)?;
    let gids = select(&face.names, &face.unicodes, selection)?;
    Ok(gids.into_iter().map(|g| face.name(g).to_string()).collect())
}

/// The first glyph of each name.
pub(crate) fn name_index(names: &[String]) -> HashMap<&str, u32> {
    let mut index = HashMap::with_capacity(names.len());
    for (gid, name) in names.iter().enumerate() {
        index.entry(name.as_str()).or_insert(gid as u32);
    }
    index
}

/// The glyph ids `selection` picks, sorted.
pub(crate) fn select(names: &[String], unicodes: &[Option<u32>], selection: &Selection) -> Result<Vec<u32>, ProofError> {
    let index = name_index(names);
    let mut work = 0usize;
    let mut chosen = vec![selection.glyphs.is_empty(); names.len()];
    mark(&index, &selection.glyphs, &mut chosen, true, &mut work)?;
    mark(&index, &selection.exclude, &mut chosen, false, &mut work)?;
    let mut gids: Vec<u32> = chosen.iter().enumerate().filter(|(_, c)| **c).map(|(g, _)| g as u32).collect();
    if gids.len() > MAX_SHEET_GLYPHS {
        return Err(ProofError::BadOptions(format!("a proof shows at most {MAX_SHEET_GLYPHS} glyphs; select fewer")));
    }
    match selection.sort {
        GlyphSort::Gid => {}
        GlyphSort::Name => gids.sort_by(|a, b| names.get(*a as usize).cmp(&names.get(*b as usize)).then(a.cmp(b))),
        GlyphSort::Unicode => gids.sort_by_key(|g| (unicodes.get(*g as usize).copied().flatten().unwrap_or(u32::MAX), *g)),
    }
    Ok(gids)
}

fn mark(index: &HashMap<&str, u32>, items: &[String], chosen: &mut [bool], value: bool, work: &mut usize) -> Result<(), ProofError> {
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (first, last) = resolve(index, item)?;
        let (lo, hi) = if first <= last { (first, last) } else { (last, first) };
        *work = work.saturating_add((hi - lo) as usize + 1);
        if *work > MAX_SELECTION_WORK {
            return Err(ProofError::BadOptions(format!("the glyph selection covers more than {MAX_SELECTION_WORK} glyphs")));
        }
        for gid in lo..=hi {
            if let Some(slot) = chosen.get_mut(gid as usize) {
                *slot = value;
            }
        }
    }
    Ok(())
}

/// The glyph ids an item names: one glyph twice, or the two ends of a range.
fn resolve(index: &HashMap<&str, u32>, item: &str) -> Result<(u32, u32), ProofError> {
    if let Some(gid) = index.get(item) {
        return Ok((*gid, *gid));
    }
    for sep in ["..", "-"] {
        // Every split point, so names that contain the separator still pair up.
        for (at, _) in item.match_indices(sep) {
            let (a, b) = (item.get(..at), item.get(at + sep.len()..));
            if let (Some(a), Some(b)) = (a.and_then(|a| index.get(a)), b.and_then(|b| index.get(b))) {
                return Ok((*a, *b));
            }
        }
    }
    Err(ProofError::BadOptions(format!("the font has no glyph `{item}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["notdef", "zero", "one", "two", "three", "a-b", "A", "B"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ranges_exclusions_and_sorting() {
        let names = names();
        let unicodes = vec![None, Some(0x30), Some(0x31), Some(0x32), Some(0x33), None, Some(0x41), Some(0x42)];
        let pick = |glyphs: &[&str], exclude: &[&str], sort| {
            let sel =
                Selection { glyphs: glyphs.iter().map(|s| s.to_string()).collect(), exclude: exclude.iter().map(|s| s.to_string()).collect(), sort };
            select(&names, &unicodes, &sel)
        };
        assert_eq!(pick(&["zero-three"], &["two"], GlyphSort::Gid).unwrap(), vec![1, 2, 4]);
        assert_eq!(pick(&["three..one"], &[], GlyphSort::Gid).unwrap(), vec![2, 3, 4], "a reversed range is taken by index");
        assert_eq!(pick(&["a-b"], &[], GlyphSort::Gid).unwrap(), vec![5], "a name with a hyphen is a name first");
        assert_eq!(pick(&["B", "A", "zero"], &[], GlyphSort::Name).unwrap(), vec![6, 7, 1]);
        assert_eq!(pick(&[], &["notdef"], GlyphSort::Unicode).unwrap(), vec![1, 2, 3, 4, 6, 7, 5]);
        assert!(pick(&["nope"], &[], GlyphSort::Gid).unwrap_err().to_string().contains("no glyph `nope`"));
    }
}
