//! FontCraft shaping (L2): text shaped through a compiled layout font with harfrust, the
//! HarfBuzz project's Rust port (`docs/plans/M8.md` § 3, The shaper), plus the bidirectional
//! runs of the Unicode Bidirectional Algorithm (UAX #9) and script runs from the Unicode
//! Script_Extensions property.
//!
//! Rows carried for other packages: `text.shaping` (Edit View, M8.19) and `textPreview.shaping`,
//! `textPreview.unicodeBidi` (Text Preview, M8.20). `textlayout` uses this crate (a same-layer
//! chain in `xtask/src/layers.rs`).
//!
//! Contract:
//! - The font is a layout font from `fontcraft_compile::preview` (bytes). Glyphs without a code
//!   point are reached through private-use code points (planes 15 and 16) that the preview font
//!   keeps beside `cmap`, not in it: [`ShapingFont::new`] takes that map, and the shaper's
//!   character-to-glyph step reads it, so a glyph typed by name in Edit View is shaped like any
//!   other.
//! - Advances can come live from the model ([`ShapeRequest::advances`]), so width edits show
//!   without compiling the layout again.
//! - Shape plans are cached per font, direction, script, language and feature list, at most
//!   [`MAX_PLANS`] per font.
//! - Everything is checked before harfrust runs: the text length, the feature count, the
//!   direction. harfrust's own `max_len` and `max_ops` stop runaway lookups; a stop or a failed
//!   allocation is a [`ShapeError`], and the caller lays the line out by code point with a
//!   message.
//! - Builds for wasm: harfrust with `std` only, never its `icu` feature.
//!
//! How a caller shapes a line of mixed text (Text Preview, M8.20):
//! 1. [`bidi::paragraph_runs`] splits the line into runs of one direction, in visual order.
//! 2. [`script::itemize`] splits each run's text into runs of one script.
//! 3. [`shape`] shapes each script run with that direction and script; for a right-to-left
//!    bidi run, its script runs are drawn last to first.
//!
//! Edit View (M8.19) keeps one direction per line and shapes each script run of it with its
//! active features, `GPOS` features off, live advances, and the glyphs typed by name passed as
//! their private-use code points.
#![forbid(unsafe_code)]

pub mod bidi;
mod font;
pub mod script;

pub use font::{Direction, FeatureSetting, MAX_LANGUAGE_BYTES, MAX_LOCATION_AXES, ShapeRequest, ShapedGlyph, ShapingFont, shape};
pub use read_fonts::types::Tag;

/// Most bytes of text one [`shape`] call takes: Text Preview's item cap of a million characters
/// at up to four bytes each.
pub const MAX_TEXT_BYTES: usize = 4 << 20;
/// Most feature settings in one request.
pub const MAX_FEATURES: usize = 256;
/// Most private-use code points a font may map: one per glyph.
pub const MAX_PRIVATE_USE: usize = 65_535;
/// Most shape plans cached per font; the least recently used goes first.
pub const MAX_PLANS: usize = 64;

/// Why a run could not be shaped. The caller lays the run out by code point and shows the
/// message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShapeError {
    #[error("the preview font cannot be read: {0}")]
    Font(String),
    #[error("the text is longer than {limit} bytes")]
    TextTooLong { limit: usize },
    #[error("more than {limit} features were asked for")]
    TooManyFeatures { limit: usize },
    #[error("the preview font maps more than {limit} glyphs to private-use code points")]
    TooManyPrivateUse { limit: usize },
    /// harfrust refused the text or stopped: `max_ops` reached, an allocation failed.
    #[error("the text could not be shaped: {0}")]
    Failed(String),
    #[error("shaping is not available yet")]
    NotBuilt,
}

#[cfg(test)]
mod tests;
