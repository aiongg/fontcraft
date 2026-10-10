//! Glyph name forms and the naming rules (05-font-view.md § 4.3, § 4.4).

use std::fmt;

/// The code-point name of `cp`: `uni` and four uppercase hex digits in the BMP, `u` and five or
/// six above it.
pub fn code_point_name(cp: u32) -> String {
    if cp <= 0xFFFF { format!("uni{cp:04X}") } else { format!("u{cp:X}") }
}

/// The code point a name in exactly the [`code_point_name`] form stands for (`uni0410` → U+0410,
/// `u1F600` → U+1F600). Other spellings (`uni0041.sc`, lowercase digits, `u0041`) give `None`.
pub fn parse_code_point_name(name: &str) -> Option<u32> {
    let (digits, bmp) = if let Some(d) = name.strip_prefix("uni") { (d, true) } else { (name.strip_prefix('u')?, false) };
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)) {
        return None;
    }
    let cp = u32::from_str_radix(digits, 16).ok()?;
    char::from_u32(cp)?;
    let canonical = if bmp { cp <= 0xFFFF && digits.len() == 4 } else { cp > 0xFFFF && code_point_name(cp) == name };
    canonical.then_some(cp)
}

/// Why a glyph name is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    /// A character other than `A–Z a–z 0–9 _ - .`.
    Character,
    /// An exporting glyph's name must start with a letter.
    FirstCharacter,
}

/// An invalid glyph name, with the offending character and its position (in characters, from 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameError {
    pub problem: NameProblem,
    pub position: usize,
    pub character: Option<char>,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.problem, self.character) {
            (NameProblem::Empty, _) => write!(f, "the glyph name is empty"),
            (NameProblem::Character, Some(c)) => {
                write!(f, "illegal character {c:?} (U+{:04X}) at position {}", u32::from(c), self.position + 1)
            }
            (NameProblem::FirstCharacter, Some(c)) => write!(f, "the name starts with {c:?}; it must start with a letter"),
            _ => write!(f, "illegal glyph name"),
        }
    }
}

impl std::error::Error for NameError {}

/// Checks a glyph name against the naming rules: only `A–Z a–z 0–9 _ - .`, and a letter first.
/// A name starting with `_` and the `.notdef` and `.null` glyphs are exempt from the first rule,
/// as are glyphs that do not export (`exports = false`).
pub fn validate(name: &str, exports: bool) -> Result<(), NameError> {
    let mut chars = name.chars().enumerate();
    let Some((_, first)) = chars.clone().next() else {
        return Err(NameError { problem: NameProblem::Empty, position: 0, character: None });
    };
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.');
    if let Some((position, c)) = chars.find(|(_, c)| !allowed(*c)) {
        return Err(NameError { problem: NameProblem::Character, position, character: Some(c) });
    }
    let exempt = !exports || first == '_' || name == ".notdef" || name == ".null";
    if !first.is_ascii_alphabetic() && !exempt {
        return Err(NameError { problem: NameProblem::FirstCharacter, position: 0, character: Some(first) });
    }
    Ok(())
}

/// Splits `name` at its first dot: (`A`, `Some("sc.ss01")`) for `A.sc.ss01`.
pub fn split_first_dot(name: &str) -> (&str, Option<&str>) {
    name.split_once('.').map_or((name, None), |(base, suffix)| (base, Some(suffix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_point_names() {
        assert_eq!(code_point_name(0x41), "uni0041");
        assert_eq!(code_point_name(0x1F600), "u1F600");
        assert_eq!(code_point_name(0x10FFFF), "u10FFFF");
        assert_eq!(parse_code_point_name("uni0410"), Some(0x410));
        assert_eq!(parse_code_point_name("u1F600"), Some(0x1F600));
        for bad in ["uni041", "uni00410", "uni004a", "u0041", "u01F600", "uniD800", "u110000", "uni", "u", "", "uni0041.sc"] {
            assert_eq!(parse_code_point_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn naming_rules() {
        assert!(validate("A-cy.sc", true).is_ok());
        assert!(validate("_part.x", true).is_ok());
        assert!(validate(".notdef", true).is_ok());
        assert!(validate("1x", false).is_ok());
        assert_eq!(validate("", true).unwrap_err().problem, NameProblem::Empty);
        let e = validate("a b", true).unwrap_err();
        assert_eq!((e.position, e.character), (1, Some(' ')));
        let e = validate("aé", true).unwrap_err();
        assert_eq!((e.position, e.character), (1, Some('é')));
        assert_eq!(validate("1x", true).unwrap_err().problem, NameProblem::FirstCharacter);
        assert_eq!(validate(&"a".repeat(1 << 20), true), Ok(()));
    }
}
