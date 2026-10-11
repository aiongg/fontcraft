//! Number value and glyph property tokens, replaced by text before compiling (M8.5;
//! `docs/spec/12-feature-code.md` § 3).
//!
//! Rows: `token.kinds`, `token.expansionTiming`, `token.numberValue.*`, `token.glyphProperty.*`,
//! `kerning.kernCodeTokens`, and the data of `font.features.evaluateToken`.
//!
//! `$name`, `${name * 2 + 10}`, `${glyph:width}`, `${glyph:anchors.top.x / 2}` are evaluated
//! against a [`TargetValues`]; predicate tokens `$[…]` go to [`crate::tokens_predicate`].
//! Arithmetic: `+ - * /`, `*` and `/` first, parentheses, decimal and negative literals;
//! results rounded half away from zero; division by zero, a number that is not finite or a
//! result beyond ±[`MAX_TOKEN_VALUE`] is an error at the token.

use std::ops::Range;

use fontcraft_fea::Diagnostic;

use crate::TargetValues;
use crate::source_map::OffsetMap;

/// Most tokens in one entry.
pub const MAX_TOKENS: usize = 100_000;
/// Most bytes of one token's expression.
pub const MAX_EXPRESSION_BYTES: usize = 4_096;
/// Largest magnitude a number token may give: font units fit 16 bits.
pub const MAX_TOKEN_VALUE: f64 = 32_767.0;

/// What one token evaluates to.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    /// A number or property token, rounded.
    Number(i32),
    /// A predicate token: the glyphs it selects, in the font's order.
    Glyphs(Vec<String>),
}

/// `code` with every token replaced by its value, and the map back to `code`. Diagnostics have
/// no file; their spans are in `code` (the token's range).
pub fn expand(code: &str, values: &dyn TargetValues) -> Result<(String, OffsetMap), Vec<Diagnostic>> {
    // M8.5
    let _ = values;
    Ok((code.to_string(), OffsetMap::identity()))
}

/// Evaluates one token's text (`$name`, `${…}`, `$[…]`), for Option-click and
/// `font.features.evaluateToken`. The error is a message for the user.
pub fn evaluate(token: &str, values: &dyn TargetValues) -> Result<TokenValue, String> {
    // M8.5
    let _ = (token, values);
    Err("tokens cannot be evaluated yet".into())
}

/// The byte ranges of the tokens in `code`, in order, at most [`MAX_TOKENS`] (syntax colouring,
/// the token under the pointer).
pub fn token_ranges(code: &str) -> Vec<Range<usize>> {
    // M8.5
    let _ = code;
    Vec::new()
}
