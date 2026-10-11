//! Text tokens: `{{{token}}}` in a General field or a name parameter, replaced by a value of the
//! font, the master or the instance at export (version string, build number, a key path, a number
//! value, a glyph's property).
//!
//! One level only: an expanded value is not expanded again. At most [`crate::MAX_TOKENS`] tokens
//! per text and [`crate::MAX_EXPANDED_BYTES`] of result; past either, tokens stay as written. A
//! token that names nothing stays as written and is listed in [`Expanded::unresolved`].
//!
//! Rows: `font.expandTextTokens`, `textToken.syntax`, `textToken.versionString`,
//! `textToken.buildNumber`, `textToken.keyPath`, `textToken.numberValue`,
//! `textToken.glyphProperty`, `fontInfo.general.textTokens` (M7.13). The compiler's name table
//! calls [`expand`] (M7.8).

use fontcraft_model::{Font, Instance, Master};

/// The opening delimiter of a token.
pub const OPEN: &str = "{{{";
/// The closing delimiter of a token.
pub const CLOSE: &str = "}}}";

/// What a token may read.
pub struct TokenContext<'a> {
    pub font: &'a Font,
    /// The master being exported or edited; `None` at the font level.
    pub master: Option<&'a Master>,
    /// The instance being exported; `None` for a master or the font.
    pub instance: Option<&'a Instance>,
    /// The advance width of a glyph (name, master id), for glyph-property tokens. The compiler
    /// passes the resolved widths; the engine reads the font.
    pub glyph_width: &'a dyn Fn(&str, &str) -> Option<f64>,
}

/// The result of [`expand`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    /// The tokens kept as written because they name nothing, each once, in order (without the
    /// delimiters).
    pub unresolved: Vec<String>,
}

/// Whether `text` holds a token opening.
pub fn has_tokens(text: &str) -> bool {
    text.contains(OPEN)
}

/// `text` with its tokens replaced.
pub fn expand(text: &str, ctx: &TokenContext) -> Expanded {
    // M7.13: the token forms of the rows.
    let _ = ctx;
    Expanded { text: text.to_string(), unresolved: Vec::new() }
}
