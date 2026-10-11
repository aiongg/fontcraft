//! Override sets: otfautohint's fontinfo text, which gives some glyphs their own zones and stems
//! (`FDDict` blocks with a `GlyphSet`), and its counter-glyph lists.
//!
//! Ported from otfautohint's `fdTools.py` (`parseFontInfoFile`, `mergeFDDicts`,
//! `FDDictManager`) and `splitpsdicts.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). Package M10.13 fills the bodies marked `// M10.13:`. Limits:
//! [`crate::MAX_FONTINFO_BYTES`], [`crate::MAX_OVERRIDE_SETS`], [`crate::MAX_COUNTER_GLYPHS`].

use std::collections::BTreeMap;

use crate::{AutohintError, FdDict, MAX_FONTINFO_BYTES};

/// The parsed fontinfo: the override sets, which set each glyph uses, and the final merged
/// dictionary when the text gives one (`FinalFont`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontInfo {
    /// Index 0 is the font's own dictionary; the override sets follow in text order.
    pub dicts: Vec<FdDict>,
    /// Glyph name → index in [`FontInfo::dicts`]; glyphs not listed use 0.
    pub glyph_dict: BTreeMap<String, usize>,
    pub final_dict: Option<FdDict>,
}

impl FontInfo {
    /// The dictionary glyph `name` is hinted with.
    pub fn dict_for(&self, name: &str) -> Option<&FdDict> {
        self.dicts.get(self.glyph_dict.get(name).copied().unwrap_or(0))
    }
}

/// Parses fontinfo `text` against the font's dictionary `base`, for the glyphs in
/// `glyph_names`; `max_y` and `min_y` are the font's vertical extremes, which otfautohint uses
/// for zones a set leaves open (`parseFontInfoFile`).
pub fn parse(text: &str, base: &FdDict, glyph_names: &[String], max_y: f64, min_y: f64) -> Result<FontInfo, AutohintError> {
    if text.len() > MAX_FONTINFO_BYTES {
        return Err(AutohintError::LimitExceeded { what: "bytes of hinting settings", limit: MAX_FONTINFO_BYTES });
    }
    // M10.13: parseFontInfoFile.
    let _ = (glyph_names, max_y, min_y);
    Ok(FontInfo { dicts: vec![base.clone()], ..FontInfo::default() })
}

/// The fontinfo text of `info`, which [`parse`] reads back to the same sets (`FDDict.__str__`).
pub fn write(info: &FontInfo) -> String {
    // M10.13: FDDict.__str__ per set.
    let _ = info;
    String::new()
}

/// The dictionary that merges several, as `mergeFDDicts` builds the final font's.
pub fn merge(dicts: &[FdDict]) -> Result<FdDict, AutohintError> {
    // M10.13: mergeFDDicts.
    Ok(dicts.first().cloned().unwrap_or_default())
}
