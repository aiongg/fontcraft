//! GOADB files: final name, source name and optional code point override per line, as AFDKO's
//! makeotf and `tx -gf` read them (M12.15 builds the parser; M12.40 reuses it; row
//! `compile.goadbRenaming`).
//!
//! A line holds two or three fields separated by spaces or tabs: the final name, the source
//! name, and code points written `uniXXXX` or `uXXXX` to `uXXXXXX` (uppercase hex), several
//! joined by commas. `#` starts a comment that runs to the end of the line; blank lines are
//! skipped. The column order was checked by running `tx -gf` (the first column is the name the
//! glyph gets). A final or source name given twice is an error, as is a code point that is not
//! a Unicode scalar value. The same parser reads the two-column rename maps of `glyphs rename
//! --map` (new name, then old name).

use std::collections::HashSet;

use crate::limits::{MAX_GLYPHS, MAX_LIST_LINE};
use crate::{Result, TablesError};

/// One line of a GOADB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoadbEntry {
    /// The name in the compiled font.
    pub final_name: String,
    /// The name in the source.
    pub source_name: String,
    /// Code points that replace the source's; empty keeps them.
    pub code_points: Vec<u32>,
}

/// A whole GOADB, in file order (which is the glyph order it asks for).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Goadb {
    pub entries: Vec<GoadbEntry>,
}

impl Goadb {
    /// The final name of the glyph called `source` in the source.
    pub fn final_name(&self, source: &str) -> Option<&str> {
        self.entries.iter().find(|e| e.source_name == source).map(|e| e.final_name.as_str())
    }

    /// `(from, to)` pairs: source name to final name, or the other way with `reverse`.
    pub fn pairs(&self, reverse: bool) -> Vec<(String, String)> {
        self.entries
            .iter()
            .map(|e| if reverse { (e.final_name.clone(), e.source_name.clone()) } else { (e.source_name.clone(), e.final_name.clone()) })
            .collect()
    }
}

/// Most code points one GOADB line may assign.
const MAX_CODE_POINTS_PER_LINE: usize = 64;

/// Parses GOADB text: lines of at most [`crate::limits::MAX_LIST_LINE`] bytes, at most
/// [`crate::limits::MAX_GLYPHS`] entries, `#` comments.
pub fn parse(text: &str) -> Result<Goadb> {
    let mut entries = Vec::new();
    let mut finals = HashSet::new();
    let mut sources = HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        if raw.len() > MAX_LIST_LINE {
            return Err(TablesError::BadOptions(format!("line {line_no} is longer than {MAX_LIST_LINE} bytes")));
        }
        let content = raw.split('#').next().unwrap_or_default();
        let fields: Vec<&str> = content.split_whitespace().collect();
        let (final_name, source_name, codes) = match fields.as_slice() {
            [] => continue,
            [one] => return Err(TablesError::BadOptions(format!("line {line_no}: `{one}` needs a second name"))),
            [a, b] => (*a, *b, None),
            [a, b, c] => (*a, *b, Some(*c)),
            _ => return Err(TablesError::BadOptions(format!("line {line_no} has more than three fields"))),
        };
        if entries.len() >= MAX_GLYPHS {
            return Err(TablesError::TooLarge { what: "the glyph list", limit: MAX_GLYPHS });
        }
        let code_points = match codes {
            Some(field) => parse_code_points(field).map_err(|e| TablesError::BadOptions(format!("line {line_no}: {e}")))?,
            None => Vec::new(),
        };
        if !finals.insert(final_name.to_string()) {
            return Err(TablesError::BadOptions(format!("line {line_no}: the final name `{final_name}` is given twice")));
        }
        if !sources.insert(source_name.to_string()) {
            return Err(TablesError::BadOptions(format!("line {line_no}: the source name `{source_name}` is given twice")));
        }
        entries.push(GoadbEntry { final_name: final_name.to_string(), source_name: source_name.to_string(), code_points });
    }
    Ok(Goadb { entries })
}

/// `uni0041,u1F600`: comma-separated code points in the two GOADB spellings.
fn parse_code_points(field: &str) -> std::result::Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for item in field.split(',') {
        if out.len() >= MAX_CODE_POINTS_PER_LINE {
            return Err(format!("more than {MAX_CODE_POINTS_PER_LINE} code points"));
        }
        let hex = if let Some(h) = item.strip_prefix("uni") {
            (h.len() == 4).then_some(h)
        } else if let Some(h) = item.strip_prefix('u') {
            (4..=6).contains(&h.len()).then_some(h)
        } else {
            None
        };
        let Some(hex) = hex.filter(|h| h.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c))) else {
            return Err(format!("`{item}` is not a code point (uniXXXX or uXXXX to uXXXXXX, uppercase hex)"));
        };
        let value = u32::from_str_radix(hex, 16).map_err(|_| format!("`{item}` is not a code point"))?;
        if char::from_u32(value).is_none() {
            return Err(format!("`{item}` is not a Unicode scalar value"));
        }
        out.push(value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn goadb_lines_comments_and_overrides() {
        let text = "# order\n.notdef .notdef\nA  a_dev\tuni0041\n\nsmiley dev.smile u1F600,uni263A # two\n";
        let g = parse(text).unwrap();
        assert_eq!(g.entries.len(), 3);
        assert_eq!(g.entries[1], GoadbEntry { final_name: "A".into(), source_name: "a_dev".into(), code_points: vec![0x41] });
        assert_eq!(g.entries[2].code_points, vec![0x1F600, 0x263A]);
        assert_eq!(g.final_name("dev.smile"), Some("smiley"));
        assert_eq!(g.pairs(false)[1], ("a_dev".to_string(), "A".to_string()));
        assert_eq!(g.pairs(true)[1], ("A".to_string(), "a_dev".to_string()));
    }

    #[test]
    fn bad_goadb_lines_are_refused_with_the_line_number() {
        for (text, needle) in [
            ("A\n", "line 1"),
            ("A a b c\n", "more than three"),
            ("A a uni41\n", "uni41"),
            ("A a uni00e9\n", "uppercase"),
            ("A a uD800\n", "scalar"),
            ("A a\nA b\n", "line 2: the final name"),
            ("A a\nB a\n", "source name"),
        ] {
            let e = parse(text).unwrap_err().to_string();
            assert!(e.contains(needle), "{text:?}: {e}");
        }
        let long = format!("A {}\n", "x".repeat(MAX_LIST_LINE));
        assert!(parse(&long).is_err());
    }
}
