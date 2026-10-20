//! The crate's error type and its named limits (`docs/plans/M11.md` § 3 › Limits).
//!
//! Every count read from a font, an image, an SVG file or a command parameter is checked against
//! one of these limits before the work it bounds.

use fontcraft_model::ModelError;

use crate::assets::AssetError;

/// Most palettes in `Color Palettes`.
pub const MAX_PALETTES: usize = 256;
/// Most colours in one palette. Row 65,535 is the text colour, so it is never a palette row.
pub const MAX_PALETTE_ROWS: usize = 65_534;
/// Most colour layers of one glyph for one master.
pub const MAX_COLOR_LAYERS: usize = 1_024;
/// Most stops in one gradient ([`fontcraft_model::MAX_GRADIENT_STOPS`]).
pub const MAX_STOPS: usize = fontcraft_model::MAX_GRADIENT_STOPS;
/// Deepest paint graph built or followed.
pub const MAX_PAINT_DEPTH: usize = 64;
/// Most layer glyphs one export adds; with the source glyphs they also stay within
/// [`fontcraft_model::MAX_GLYPHS`].
pub const MAX_LAYER_GLYPHS: usize = 65_535;
/// Most bitmap strikes in one `sbix` or `CBLC` table.
pub const MAX_STRIKES: usize = 64;
/// Largest image file read for display or export (the same value as
/// `fontcraft_imagery::MAX_FILE_BYTES`, which this crate does not depend on).
pub const MAX_ASSET_BYTES: usize = 64 << 20;
/// Largest SVG file read for an svg layer.
pub const MAX_SVG_FILE_BYTES: usize = 16 << 20;
/// Deepest element nesting accepted in an SVG file.
pub const MAX_SVG_DEPTH: usize = 256;
/// Largest SVG document written for one glyph.
pub const MAX_SVG_DOCUMENT_BYTES: usize = 16 << 20;
/// Largest `SVG ` table written.
pub const MAX_SVG_TABLE_BYTES: usize = 512 << 20;
/// Most paint nodes one glyph's SVG document visits, references to other colour glyphs followed
/// included (Edit View's drawing has the same limit), so that references fanning out to other
/// references cannot multiply the work without bound.
pub const MAX_SVG_PAINT_VISITS: usize = 65_536;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ColorError {
    #[error("there is no glyph called `{0}`")]
    MissingGlyph(String),
    #[error("there is no master `{0}`")]
    MissingMaster(String),
    #[error("there is no layer `{layer}` in `{glyph}`")]
    MissingLayer { glyph: String, layer: String },
    /// A `Color Palettes` value that cannot be read: not a list of lists of colours, a ragged
    /// table, or a component out of range.
    #[error("the colour palettes cannot be read: {0}")]
    BadPalettes(String),
    #[error("there is no palette {0}")]
    NoSuchPalette(usize),
    #[error("there is no colour {0} in the palettes")]
    NoSuchRow(usize),
    /// A palette row that colour layers still use; nothing is removed.
    #[error("colour {row} is used by {}", .glyphs.join(", "))]
    RowInUse { row: usize, glyphs: Vec<String> },
    #[error("the last palette cannot be removed")]
    LastPalette,
    /// A gradient, a coordinate or a colour that is not a usable number.
    #[error("`{glyph}` has a paint that is not a usable number: {message}")]
    BadPaint { glyph: String, message: String },
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    #[error("the SVG file cannot be used: {0}")]
    BadSvg(String),
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error(transparent)]
    Model(#[from] ModelError),
    /// A function of this crate whose M11 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}
