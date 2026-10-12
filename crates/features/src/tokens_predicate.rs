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

use fontcraft_glyphquery::predicate::{parse, predicate_len_within};

use crate::TargetValues;
use crate::source_map::EntryKind;
use crate::tokens::MAX_EXPRESSION_BYTES;

/// The glyph names `predicate` (the text between `$[` and `]`) selects for the target, in the
/// font's order. The error is a message for the user, naming what could not be read.
pub fn glyphs(predicate: &str, values: &dyn TargetValues) -> Result<Vec<String>, String> {
    let parsed = parse(predicate).map_err(|e| format!("the glyph predicate cannot be read: {e}"))?;
    values.select(&parsed)
}

/// The length in bytes of the token starting at `code[at..]` (`$[`, the predicate, `]`), or
/// `None` when no token starts there or nothing closes it within [`MAX_EXPRESSION_BYTES`].
pub fn token_len(code: &str, at: usize) -> Option<usize> {
    let after = code.get(at..)?.strip_prefix("$[")?;
    predicate_len_within(after, MAX_EXPRESSION_BYTES).map(|len| len + 3)
}

/// Refuses a predicate token where the handbook does not allow one: in a prefix or feature,
/// outside class brackets (`in_brackets`, found by the token scan). Anywhere in a class entry
/// is fine.
pub fn check_placement(in_brackets: bool, kind: EntryKind) -> Result<(), String> {
    if kind == EntryKind::Class || in_brackets {
        return Ok(());
    }
    Err("in prefix and feature code a glyph predicate token must stand inside class brackets, as in `[$[case == lower]]`".into())
}

#[cfg(test)]
#[path = "tokens_predicate_tests.rs"]
mod tests;
