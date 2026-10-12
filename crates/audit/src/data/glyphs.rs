//! Unicode properties and production names, read from the built-in glyph database
//! (`fontcraft-glyphdata`: the Unicode Character Database and the glyph data it is built from).
//! Case partners follow Unicode's case mappings as the standard library carries them, the way
//! FontBakery's `case_mapping` swaps case: only one-character results count.

use std::sync::OnceLock;

use fontcraft_model::glyphdata::{GeneralCategory, GlyphData};

/// The built-in glyph database, opened once. `None` when its tables cannot be read, in which
/// case the checks that need it skip.
pub fn glyph_data() -> Option<&'static GlyphData> {
    static DATA: OnceLock<Option<GlyphData>> = OnceLock::new();
    DATA.get_or_init(|| GlyphData::new().ok()).as_ref()
}

pub fn general_category(cp: u32) -> Option<GeneralCategory> {
    glyph_data().map(|d| d.general_category(cp))
}

/// A letter: general category Lu, Ll, Lt, Lm or Lo.
pub fn is_letter(cp: u32) -> bool {
    matches!(general_category(cp), Some(GeneralCategory::Lu | GeneralCategory::Ll | GeneralCategory::Lt | GeneralCategory::Lm | GeneralCategory::Lo))
}

/// A combining mark: general category Mn, Mc or Me.
pub fn is_mark(cp: u32) -> bool {
    matches!(general_category(cp), Some(GeneralCategory::Mn | GeneralCategory::Mc | GeneralCategory::Me))
}

/// A non-spacing mark: general category Mn or Me.
pub fn is_nonspacing_mark(cp: u32) -> bool {
    matches!(general_category(cp), Some(GeneralCategory::Mn | GeneralCategory::Me))
}

/// The canonical combining class of `cp` (230 for a mark above).
pub fn combining_class(cp: u32) -> u8 {
    glyph_data().map_or(0, |d| d.canonical_combining_class(cp))
}

/// The other-case partner of a letter: the lowercase of an uppercase letter, the uppercase of a
/// lowercase one. `None` for other characters, for titlecase letters, and when the mapping gives
/// several characters (ß → SS) or the letter itself.
pub fn case_partner(cp: u32) -> Option<u32> {
    if !is_letter(cp) {
        return None;
    }
    let c = char::from_u32(cp)?;
    let swapped = if c.is_uppercase() {
        single(c.to_lowercase())
    } else if c.is_lowercase() {
        single(c.to_uppercase())
    } else {
        None
    }?;
    (swapped != c).then_some(u32::from(swapped))
}

fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// The name a glyph exports under by the glyph database (its own name when the database knows
/// no other).
pub fn production_name(name: &str, code_points: &[u32]) -> Option<String> {
    glyph_data().map(|d| d.production_name(name, code_points))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_and_case_partners() {
        assert!(is_letter(0x0041));
        assert!(is_letter(0x4E00));
        assert!(!is_letter(0x0031));
        assert!(is_nonspacing_mark(0x0301));
        assert!(is_mark(0x0301));
        assert!(!is_mark(0x00B4));
        assert_eq!(combining_class(0x0301), 230);
        assert_eq!(case_partner(0x0041), Some(0x0061));
        assert_eq!(case_partner(0x0061), Some(0x0041));
        assert_eq!(case_partner(0x00DF), None, "ß uppercases to two letters");
        assert_eq!(case_partner(0x01C5), None, "titlecase");
        assert_eq!(case_partner(0x0031), None);
        assert_eq!(case_partner(0x03C2), Some(0x03A3), "final sigma");
        assert_eq!(production_name("Adieresis", &[0x00C4]).as_deref(), Some("Adieresis"));
        assert_eq!(production_name("nbspace", &[]).as_deref(), Some("uni00A0"));
    }
}
