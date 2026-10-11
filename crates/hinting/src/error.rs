//! The crate's error type and its named limits (`docs/plans/M10.md` § 3 › Limits).
//!
//! Every count read from a font, a parameter or a command parameter is checked against one of
//! these limits before the work it bounds.

/// Hints and instructions on one layer (the model's limit).
pub const MAX_LAYER_HINTS: usize = fontcraft_model::MAX_LAYER_HINTS;
/// Alignment zones accepted in a source, per master.
pub const MAX_SOURCE_ZONES: usize = 64;
/// Stem definitions accepted in a source.
pub const MAX_SOURCE_STEMS: usize = 64;
/// Stems written to StemSnapH or StemSnapV (the CFF limit); the rest are reported.
pub const MAX_PS_STEMS_WRITTEN: usize = 12;
/// TT zones per master.
pub const MAX_TT_ZONES: usize = 256;
/// TT stems per master.
pub const MAX_TT_STEMS: usize = 256;
/// Entries in the `cvt ` table.
pub const MAX_CVT_ENTRIES: usize = 4_096;
/// Highest PPM a delta may name (the reach of DELTAP with a delta base).
pub const MAX_DELTA_PPM: u16 = 255;
/// Columns of one delta table (masters or instances).
pub const MAX_DELTA_COLUMNS: usize = 64;
/// Pixel sizes of the Instructor.
pub const MIN_INSTRUCTOR_PPM: u16 = 4;
pub const MAX_INSTRUCTOR_PPM: u16 = 400;
/// TTFAutohint control instructions: text size and lines.
pub const MAX_CONTROL_TEXT: usize = 1 << 20;
pub const MAX_CONTROL_LINES: usize = 100_000;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum HintingError {
    #[error("there is no glyph called `{0}`")]
    MissingGlyph(String),
    #[error("there is no master or layer `{layer}` in `{glyph}`")]
    MissingLayer { glyph: String, layer: String },
    #[error("there is no master `{0}`")]
    MissingMaster(String),
    /// A node reference that names no node of the layer: a message, never an index panic.
    #[error("the hint refers to node {node} of path {path}, which the layer does not have")]
    DanglingNode { path: usize, node: usize },
    /// A stored hint whose value cannot be read as its kind.
    #[error("hint {index} cannot be read: {message}")]
    BadHint { index: usize, message: String },
    /// A parameter value that cannot be read (TTFZones, gasp Table…).
    #[error("the parameter `{name}` cannot be read: {message}")]
    BadParameter { name: String, message: String },
    /// A number that is not finite or out of range for its field.
    #[error("{what} is not a usable number")]
    BadNumber { what: String },
    /// An instruction that cannot be compiled (a point without an exported number…).
    #[error("`{glyph}`: instruction {index} cannot be compiled: {message}")]
    Compile { glyph: String, index: usize, message: String },
    /// TTFAutohint control instructions that cannot be read; `line` counts from 1.
    #[error("line {line} of the control instructions: {message}")]
    Control { line: usize, message: String },
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    #[error(transparent)]
    Assembly(#[from] fontcraft_ttasm::AsmError),
    #[error(transparent)]
    Autohint(#[from] fontcraft_autohint::AutohintError),
    /// A function of this crate whose M10 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}
