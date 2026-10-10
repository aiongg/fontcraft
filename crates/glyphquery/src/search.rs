//! The search field of Font View (`docs/spec/05-font-view.md` § 6.1).
//!
//! A term matches a glyph by name, by code point written in hexadecimal, by the character itself,
//! or by its note. Plain terms match names and notes by substring, ignoring letter case unless
//! Match Case is on; a hexadecimal term of four to six digits (`00FE`, `U+00FE`) equals a code
//! point; a one-character term equals a character. With Regex on, the term is a regular
//! expression that must match a whole field: the name, the note, a code point's hexadecimal form
//! or a character.

use serde::{Deserialize, Serialize};

use crate::predicate::Pattern;
use crate::{MAX_SEARCH_BYTES, QueryError, fail};
use fontcraft_model::Glyph;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Search {
    pub query: String,
    /// The fields to match; empty means all of them.
    #[serde(default)]
    pub fields: Vec<SearchField>,
    #[serde(default)]
    pub match_case: bool,
    #[serde(default)]
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchField {
    Name,
    /// The hexadecimal code point and the character.
    Unicode,
    Note,
}

/// A search term ready to test glyphs with.
pub struct Matcher {
    how: How,
    name: bool,
    unicode: bool,
    note: bool,
    match_case: bool,
}

enum How {
    Everything,
    Text { needle: String, code_point: Option<u32>, character: Option<u32> },
    Regex(Pattern),
}

/// `00FE`, `U+00FE`, `0x00fe`: four to six hexadecimal digits.
fn hex_code_point(term: &str) -> Option<u32> {
    let digits = ["U+", "u+", "0x", "0X"].iter().find_map(|p| term.strip_prefix(p)).unwrap_or(term);
    if !(4..=6).contains(&digits.len()) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().filter(|&cp| cp <= 0x10FFFF)
}

impl Matcher {
    pub fn new(search: &Search) -> Result<Matcher, QueryError> {
        if search.query.len() > MAX_SEARCH_BYTES {
            return fail(format!("the search term is {} bytes long; at most {MAX_SEARCH_BYTES} are allowed", search.query.len()));
        }
        let all = search.fields.is_empty();
        let has = |f: SearchField| all || search.fields.contains(&f);
        let how = if search.query.is_empty() {
            How::Everything
        } else if search.regex {
            How::Regex(Pattern::new(&search.query, !search.match_case)?)
        } else {
            let mut chars = search.query.chars();
            let character = match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c as u32),
                _ => None,
            };
            let needle = if search.match_case { search.query.clone() } else { search.query.to_lowercase() };
            How::Text { needle, code_point: hex_code_point(&search.query), character }
        };
        Ok(Matcher {
            how,
            name: has(SearchField::Name),
            unicode: has(SearchField::Unicode),
            note: has(SearchField::Note),
            match_case: search.match_case,
        })
    }

    /// Whether the matcher lets every glyph through.
    pub fn is_everything(&self) -> bool {
        matches!(self.how, How::Everything)
    }

    pub fn matches(&self, glyph: &Glyph) -> bool {
        let note = glyph.info.note.as_str();
        match &self.how {
            How::Everything => true,
            How::Text { needle, code_point, character } => {
                let contains =
                    |field: &str| if self.match_case { field.contains(needle.as_str()) } else { field.to_lowercase().contains(needle.as_str()) };
                (self.name && contains(&glyph.name))
                    || (self.note && !note.is_empty() && contains(note))
                    || (self.unicode && glyph.unicodes.iter().any(|cp| Some(*cp) == *code_point || Some(*cp) == *character))
            }
            How::Regex(p) => {
                (self.name && p.is_match(&glyph.name))
                    || (self.note && !note.is_empty() && p.is_match(note))
                    || (self.unicode
                        && glyph.unicodes.iter().any(|&cp| {
                            p.is_match(&format!("{cp:04X}")) || char::from_u32(cp).is_some_and(|c| p.is_match(c.encode_utf8(&mut [0; 4])))
                        }))
            }
        }
    }
}
