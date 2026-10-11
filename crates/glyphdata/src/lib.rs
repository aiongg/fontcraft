//! FontCraft's glyph database (L0, standalone): what a glyph name means.
//!
//! Given a glyph name (and the glyph's code points), [`GlyphData::lookup`] answers its
//! category, subcategory, code point, production name, script, case, direction and
//! description, and says where each value came from: a custom glyph data file, the built-in
//! database, or synthesis from the name (suffixes, ligatures, code-point names and the Adobe
//! Glyph List). The rules are those of `docs/spec/05-font-view.md` § 4.
//!
//! The built-in data is GlyphsInfo's `GlyphData.xml` and `GlyphData_Ideographs.xml`, the Adobe
//! Glyph List with AGLFN, and Unicode's general categories, bidi classes, scripts, canonical
//! decompositions and combining classes, compiled by
//! `cargo xtask glyphdata --generate` into `data/glyphdata.bin` (sources and licences:
//! `xtask/glyphdata.toml`, `ASSETS.md`, `NOTICE`). It is loaded on first use.
//!
//! [`sets`] holds the language and glyph-set data: the characters each language needs and the
//! glyph sets a new document can start from (`data/sets.bin`, from `cargo xtask glyphsets`).
//!
//! Custom glyph data files are untrusted input: [`xml::parse`] reads them within fixed limits
//! and fails with an error, never a panic.
#![forbid(unsafe_code)]

mod agl;
pub mod blob;
#[cfg(feature = "build")]
pub mod build;
mod data;
pub mod items;
mod names;
mod order;
mod record;
mod search;
pub mod sets;
mod tables;
mod types;
pub mod xml;

use std::fmt;

pub use data::{Found, GlyphData, Info, MAX_CODE_POINTS, MAX_CUSTOM_FILES, MAX_DECOMPOSITION_DEPTH, MAX_NAME_BYTES, Source, Sourced, Via};
pub use names::{NameError, NameProblem, code_point_name, parse_code_point_name, validate};
pub use order::SortKey;
pub use record::Record;
pub use search::{MAX_QUERY_BYTES, MAX_RESULTS, SortBy};
pub use types::{BidiClass, Case, Category, Direction, GeneralCategory, SubCategory};

/// Why glyph data could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A custom glyph data file is malformed.
    Xml { line: usize, message: String },
    /// An input is over one of the limits.
    TooLarge { what: &'static str, size: usize, limit: usize },
    /// The custom file at this position (from 0) failed.
    File { index: usize, error: Box<Error> },
    /// The built-in tables are damaged (a build problem, not user input).
    Blob(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Xml { line, message } => write!(f, "glyph data, line {line}: {message}"),
            Error::TooLarge { what, size, limit } => write!(f, "{what}: {size} is over the limit of {limit}"),
            Error::File { index, error } => write!(f, "custom glyph data file {}: {error}", index + 1),
            Error::Blob(m) => write!(f, "built-in glyph data: {m}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests;
