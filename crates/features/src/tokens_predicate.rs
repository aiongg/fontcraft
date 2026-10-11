//! Glyph predicate tokens `$[…]` (M8.6; `docs/spec/12-feature-code.md` § 3): the glyphs a
//! predicate selects, as a space-separated list inside the brackets of a class or rule. The
//! language and evaluator are `fontcraft_glyphquery::predicate`.
//!
//! Rows: `token.predicate.*`.

use crate::TargetValues;

/// The glyph names `predicate` (the text between `$[` and `]`) selects for the target, in the
/// font's order. The error is a message for the user, naming what could not be read.
pub fn glyphs(predicate: &str, values: &dyn TargetValues) -> Result<Vec<String>, String> {
    // M8.6: parse with `fontcraft_glyphquery::predicate::parse`, then `values.select`.
    let _ = (predicate, values);
    Err("glyph predicate tokens are not available yet".into())
}
