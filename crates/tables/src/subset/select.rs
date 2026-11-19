//! The reference converter's glyph selection in a subset (M12.73; [`super::GlyphFilter`]):
//! excluded glyphs, Font DICTs, the glyphs a GOADB lists and a random share. It runs on the
//! initial glyph set, before `.notdef` and the closure are added (`closure::plan`).

use std::collections::BTreeSet;

use super::{GlyphFilter, RandomShare};
use crate::Result;
use crate::sfnt::SfntFont;

/// Narrows `glyphs` as `filter` says. `names` holds the glyph names by id; `everything` says
/// that no other selection was given, so the filter starts from every glyph of the font.
pub(super) fn apply(font: &SfntFont, filter: &GlyphFilter, names: &[String], everything: bool, glyphs: &mut BTreeSet<u32>) -> Result<()> {
    if !filter.is_set() {
        return Ok(());
    }
    let n = names.len();
    if everything {
        glyphs.extend(0..u32::try_from(n).unwrap_or(u32::MAX));
    }
    glyphs.retain(|g| !filter.exclude.contains(g));
    if let Some(dicts) = &filter.font_dicts {
        let fds = crate::convert::font_dict_of_glyphs(font, dicts, n)?;
        glyphs.retain(|g| fds.get(*g as usize).is_some_and(|fd| dicts.keep.as_ref().is_none_or(|k| k.contains(fd)) && !dicts.exclude.contains(fd)));
    }
    if let Some(listed) = &filter.goadb {
        glyphs.retain(|g| names.get(*g as usize).is_some_and(|name| listed.contains(name)));
    }
    if let Some(share) = filter.random {
        *glyphs = random_share(glyphs, share, names);
    }
    Ok(())
}

/// `share` of `candidates`, rounded to whole glyphs and at least one when there are any,
/// chosen without repeats by a partial Fisher-Yates shuffle.
pub fn random_share(candidates: &BTreeSet<u32>, share: RandomShare, names: &[String]) -> BTreeSet<u32> {
    let mut pool: Vec<u32> = candidates.iter().copied().collect();
    let m = pool.len() as u64;
    let wanted = (m * u64::from(share.millionths.min(1_000_000)) + 500_000) / 1_000_000;
    let count = usize::try_from(wanted.max(1).min(m)).unwrap_or(pool.len());
    let mut rng = SplitMix64(share.seed.unwrap_or_else(|| font_seed(names)));
    for i in 0..count {
        let left = (pool.len() - i) as u64;
        let j = i + usize::try_from(rng.next_u64() % left).unwrap_or(0);
        pool.swap(i, j);
    }
    pool.into_iter().take(count).collect()
}

/// A seed from the glyph names (FNV-1a), so the same font gives the same choice.
fn font_seed(names: &[String]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for name in names {
        for b in name.bytes().chain([0]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

/// The SplitMix64 generator (Steele, Lea and Flood, 2014): small, fast and the same on every
/// platform, which a repeatable choice needs.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}
