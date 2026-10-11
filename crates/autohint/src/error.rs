//! The crate's error type and its named limits (`docs/plans/M10.md` § 3 › Limits).
//!
//! Every count read from a glyph, a zone set or an override file is checked against one of these
//! limits before the work it bounds.

/// Segments per direction; a direction with more is left unhinted and reported, as otfautohint's
/// `--max-segments` default does ([`crate::Options::max_segments`] may lower it, never raise it).
pub const MAX_SEGMENTS: usize = 100;
/// Path elements (lines and curves) in one glyph; the points limit of the plan is 65,535.
pub const MAX_ELEMENTS: usize = 65_535;
/// Stems one glyph may carry in one direction. Type 2 charstrings allow 96 stem hints in all.
pub const MAX_STEMS: usize = 96;
/// Zones accepted in a source (BlueValues, OtherBlues and the family zones each).
pub const MAX_ZONES: usize = 64;
/// Masters hinted together by [`crate::hint_masters`].
pub const MAX_MASTERS: usize = 64;
/// Longest override-set text (fontinfo), in bytes.
pub const MAX_FONTINFO_BYTES: usize = 1 << 20;
/// Most override sets in one fontinfo text.
pub const MAX_OVERRIDE_SETS: usize = 256;
/// Most glyph names in one counter-glyph list (otfautohint's extension limit).
pub const MAX_COUNTER_GLYPHS: usize = 64;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AutohintError {
    /// Zones that overlap, a top zone with a negative size, fewer than the 4 BlueValues
    /// otfautohint needs unless [`crate::Options::allow_no_blues`] is set…
    #[error("the alignment zones cannot be used: {0}")]
    BadZones(String),
    /// No stem in either direction, unless [`crate::Options::allow_no_blues`] is set.
    #[error("the standard stems cannot be used: {0}")]
    BadStems(String),
    /// A coordinate that is not finite or lies outside the model's range.
    #[error("`{glyph}` has a coordinate that is not a usable number")]
    BadCoordinate { glyph: String },
    /// The masters given to [`crate::hint_masters`] do not have the same path structure.
    #[error("the masters of `{glyph}` are not compatible: {reason}")]
    Incompatible { glyph: String, reason: String },
    /// An override set or fontinfo text that cannot be read; `line` counts from 1.
    #[error("line {line} of the hinting settings: {message}")]
    FontInfo { line: usize, message: String },
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    /// A function of this crate whose M10 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}
