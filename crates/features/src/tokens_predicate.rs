//! Glyph predicate tokens `$[…]` (M8.6; `docs/spec/12-feature-code.md` § 3): the glyphs a
//! predicate selects, as a space-separated list inside the brackets of a class or rule. The
//! language and evaluator are `fontcraft_glyphquery::predicate`.
//!
//! Rows: `token.predicate.*`.
//!
//! In a class entry a token stands among the member names (`hyphen $[category == "Number"]`);
//! in prefix and feature code it must stand inside class brackets (`[$[case == lower]]`),
//! which [`check_placement`] tests. A token ends at the `]` that closes it outside quoted
//! text ([`token_len`]), so `$[name matches "[a-z]"]` is one token.

use fontcraft_glyphquery::predicate::{parse, predicate_len};

use crate::TargetValues;
use crate::source_map::EntryKind;

/// The glyph names `predicate` (the text between `$[` and `]`) selects for the target, in the
/// font's order. The error is a message for the user, naming what could not be read.
pub fn glyphs(predicate: &str, values: &dyn TargetValues) -> Result<Vec<String>, String> {
    let parsed = parse(predicate).map_err(|e| format!("the glyph predicate cannot be read: {e}"))?;
    values.select(&parsed)
}

/// The length in bytes of the token starting at `code[at..]` (`$[`, the predicate, `]`), or
/// `None` when no token starts there or nothing closes it.
pub fn token_len(code: &str, at: usize) -> Option<usize> {
    let after = code.get(at..)?.strip_prefix("$[")?;
    predicate_len(after).map(|len| len + 3)
}

/// Whether byte `at` of feature code lies inside class brackets. Comments, quoted text and other
/// predicate tokens are skipped.
pub fn inside_brackets(code: &str, at: usize) -> bool {
    let Some(before) = code.get(..at) else { return false };
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(rest) = before.get(i..) {
        let Some(c) = rest.chars().next() else { break };
        match c {
            '#' => {
                i += rest.find('\n').unwrap_or(rest.len());
                continue;
            }
            '"' => {
                i += 1 + rest.get(1..).and_then(|r| r.find('"')).map_or(rest.len(), |end| end + 1);
                continue;
            }
            '$' if rest.starts_with("$[") => {
                // A token not closed before `at` holds `at`; it is not inside brackets of its own.
                match token_len(before, i) {
                    Some(len) => {
                        i += len;
                        continue;
                    }
                    None => return depth > 0,
                }
            }
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += c.len_utf8();
    }
    depth > 0
}

/// Refuses a predicate token at byte `at` of an entry's code where the handbook does not allow
/// one: in a prefix or feature, outside class brackets. Anywhere in a class entry is fine.
pub fn check_placement(code: &str, at: usize, kind: EntryKind) -> Result<(), String> {
    if kind == EntryKind::Class || inside_brackets(code, at) {
        return Ok(());
    }
    Err("in prefix and feature code a glyph predicate token must stand inside class brackets, as in `[$[case == lower]]`".into())
}

#[cfg(test)]
#[path = "tokens_predicate_tests.rs"]
mod tests;
