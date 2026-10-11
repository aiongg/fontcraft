//! The legacy presentation-form code points of Arabic glyphs, for the *Use Arabic Presentation
//! Form Unicodes* parameter ([`crate::export_params`]).
//!
//! The glyph database records them: a positional form or a ligature (`beh-ar.init`,
//! `lam_alef-ar`) carries the presentation-form character as its legacy code point (the
//! `unicodeLegacy` attribute of the published glyph data), the same character Unicode's
//! compatibility decomposition (`<initial>`, `<medial>`, `<final>`, `<isolated>`) maps to that
//! letter and form. Only the Arabic presentation-form blocks count: FB50–FDFF and FE70–FEFF.
//!
//! Row: `Use Arabic Presentation Form Unicodes` (M7.9).

use std::ops::RangeInclusive;

use fontcraft_model::glyphdata::GlyphData;

/// Arabic Presentation Forms-A (from its first Arabic character) and Forms-B.
pub const RANGES: [RangeInclusive<u32>; 2] = [0xFB50..=0xFDFF, 0xFE70..=0xFEFF];

/// The presentation-form code point the glyph database gives the glyph called `name`, if it is
/// one of an Arabic presentation-form block.
pub fn code_point(data: &GlyphData, name: &str) -> Option<u32> {
    data.record(name)?.unicode_legacy.filter(|cp| RANGES.iter().any(|r| r.contains(cp)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_forms_and_ligatures_have_their_presentation_forms() {
        let data = GlyphData::new().unwrap();
        for (name, cp) in [
            ("beh-ar.isol", Some(0xFE8F)),
            ("beh-ar.fina", Some(0xFE90)),
            ("beh-ar.init", Some(0xFE91)),
            ("beh-ar.medi", Some(0xFE92)),
            ("lam_alef-ar", Some(0xFEFB)),
            ("lam_alef-ar.fina", Some(0xFEFC)),
            ("beh-ar", None),
            ("f_f_l", None),
            ("A", None),
        ] {
            assert_eq!(code_point(&data, name), cp, "{name}");
        }
    }
}
