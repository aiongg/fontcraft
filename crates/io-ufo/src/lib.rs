//! UFO and designspace sources (L3): reading into the model and writing back.
//!
//! Our own reader and writer (decision 0003), in the layout fontTools' `ufoLib` and
//! `designspaceLib` write, so that saving a document those tools wrote, unedited, changes no
//! byte (`roundtrip.ufoXmlFormatting`, `roundtrip.designspaceVerbatim`). Written from the UFO
//! specification, the designspace documentation and the fontTools source (MIT; see `NOTICE`).
//!
//! - [`glif`]: one glyph's file in one layer.
//! - [`names`]: the specification's user-name-to-file-name rule.
//! - `ufo1`: a UFO 1's font information and lib in their UFO 2 form, read as a UFO 3 source.
//! - [`ufo`]: a UFO directory, as one master; `fontinfo` maps its font information to the
//!   reference's properties and parameters, `design` its blue zones, stem snaps and guidelines to
//!   the master's fields, and `libkeys` the lib keys glyphsLib writes for the rest (settings,
//!   parameters with no font-info key, a master's icon and name parts).
//! - [`features`]: `features.fea`, split into the model's prefixes, classes and features as
//!   glyphsLib writes them.
//! - [`designspace`]: a designspace and its UFOs, as one document; its axes, instances and
//!   variable fonts are the model's.
//! - [`zip`]: reading a UFOZ, a UFO in a ZIP archive.
//!
//! The functions work on file contents by relative path; reading and writing directories is the
//! caller's job, so they also run in the browser.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fontcraft_model::ModelError;

mod colorglyphs;
mod colorkeys;
mod design;
pub mod designspace;
pub mod features;
mod fontinfo;
mod gdef;
pub mod glif;
pub mod glyphkeys;
pub mod images;
pub mod kerning;
pub(crate) mod libkeys;
pub mod names;
mod pathkeys;
mod shapekeys;
pub mod ufo;
mod ufo1;
pub mod zip;

/// A UFO's files: relative path (with `/`) → contents.
pub type Files = BTreeMap<String, Vec<u8>>;

#[derive(Debug, thiserror::Error)]
pub enum UfoError {
    #[error("{file}: {error}")]
    Syntax { file: String, error: fontcraft_plist::Error },
    #[error("{at}: {message}")]
    Bad { at: String, message: String },
    #[error("not supported yet: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Model(#[from] ModelError),
}

pub type Result<T> = std::result::Result<T, UfoError>;

pub(crate) fn bad(at: &str, message: impl Into<String>) -> UfoError {
    UfoError::Bad { at: at.to_string(), message: message.into() }
}
