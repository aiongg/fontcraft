//! UFO and designspace sources (L3): reading into the model and writing back.
//!
//! Our own reader and writer (decision 0003), in the layout fontTools' `ufoLib` and
//! `designspaceLib` write, so that saving a document those tools wrote, unedited, changes no
//! byte (`roundtrip.ufoXmlFormatting`, `roundtrip.designspaceVerbatim`). Written from the UFO
//! specification, the designspace documentation and the fontTools source (MIT; see `NOTICE`).
//!
//! - [`glif`]: one glyph's file in one layer.
//! - [`names`]: the specification's user-name-to-file-name rule.
//! - [`ufo`]: a UFO directory, as one master.
//! - [`designspace`]: a designspace and its UFOs, as one document.
//!
//! The functions work on file contents by relative path; reading and writing directories is the
//! caller's job, so they also run in the browser.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fontcraft_model::ModelError;

pub mod designspace;
pub mod glif;
pub mod names;
pub mod ufo;

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
