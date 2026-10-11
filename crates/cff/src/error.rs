//! Why a CFF table could not be built.

use std::fmt;

/// Why a CFF table could not be built. Building either succeeds or returns one of these; it never
/// panics, whatever the input.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The font has no glyphs. A CFF font needs at least `.notdef`.
    NoGlyphs,
    /// The font has more glyphs than a CFF table can hold.
    TooManyGlyphs(usize),
    /// The first glyph is not `.notdef`, which the charset requires.
    FirstGlyphNotNotdef(String),
    /// Two glyphs have the same name.
    DuplicateGlyphName(String),
    /// More distinct strings than 16-bit string ids can address.
    TooManyStrings,
    /// A string is longer than [`crate::MAX_STRING_LEN`] bytes, or empty where a name is required.
    BadStringLength { what: String, len: usize },
    /// A string has a character the field cannot hold (ASCII only, or Latin-1 only).
    BadCharacter { what: String, character: char },
    /// A number is NaN or infinite.
    NotFinite { what: String },
    /// A number is outside the range the field accepts.
    OutOfRange { what: String, value: f64 },
    /// A glyph has more path segments than [`crate::MAX_SEGMENTS`].
    TooManySegments { glyph: String, count: usize },
    /// A DICT operator has more operands than the 48 a DICT may give it.
    TooManyOperands { what: String, count: usize },
    /// The rounding tolerance is negative or not finite.
    BadRoundTolerance(f64),
    /// The table would be larger than [`crate::MAX_TABLE_SIZE`] bytes.
    TableTooLarge(usize),
    /// The offsets in the Top DICT did not reach a fixed point.
    LayoutDidNotSettle,
    /// A glyph's hints cannot be written ([`crate::CharstringHints::validate`]).
    BadHints { glyph: String, message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoGlyphs => write!(f, "a CFF font needs at least one glyph, `.notdef`"),
            Error::TooManyGlyphs(n) => {
                write!(f, "{n} glyphs is more than a CFF table can hold ({})", crate::MAX_GLYPHS)
            }
            Error::FirstGlyphNotNotdef(name) => {
                write!(f, "the first glyph must be `.notdef`, not `{name}`")
            }
            Error::DuplicateGlyphName(name) => write!(f, "two glyphs are named `{name}`"),
            Error::TooManyStrings => write!(f, "more distinct names and strings than a CFF table can hold"),
            Error::BadStringLength { what, len } => write!(f, "{what}: a length of {len} bytes is not allowed"),
            Error::BadCharacter { what, character } => {
                write!(f, "{what}: the character {character:?} cannot be stored in a CFF string")
            }
            Error::NotFinite { what } => write!(f, "{what}: the number is not finite"),
            Error::OutOfRange { what, value } => write!(f, "{what}: {value} is out of range"),
            Error::TooManySegments { glyph, count } => {
                write!(f, "glyph `{glyph}` has {count} path segments, more than the limit of {}", crate::MAX_SEGMENTS)
            }
            Error::TooManyOperands { what, count } => {
                write!(f, "{what}: {count} values is more than the 48 a CFF DICT entry can hold")
            }
            Error::BadRoundTolerance(t) => write!(f, "rounding tolerance {t} must be zero or more"),
            Error::TableTooLarge(n) => {
                write!(f, "the CFF table would be {n} bytes, more than the limit of {}", crate::MAX_TABLE_SIZE)
            }
            Error::LayoutDidNotSettle => write!(f, "the CFF table layout did not settle"),
            Error::BadHints { glyph, message } => write!(f, "the hints of `{glyph}` cannot be written: {message}"),
        }
    }
}

impl std::error::Error for Error {}
