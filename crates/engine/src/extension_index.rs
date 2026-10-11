//! Extension indexes: JSON lists of modules and macros the Extension Manager offers (M13.10
//! fills the bodies). Plan: `docs/plans/M13.md` § 3 (extension indexes), § 4 › M13.10; owner
//! decision M13 § 7 question 5 (local files and HTTPS indexes, no main index).
//!
//! Format (at most [`MAX_INDEX_BYTES`]): `{"format": "fontcraft.extensions", "version": 1,
//! "entries": [{"id", "kind", "version", "titles": {lang: text}, "descriptions": {lang: text},
//! "vendor", "url", "sha256", "size", "homepage"?, "funding"?, "screenshot"?}]}`. The engine reads
//! bytes the shell hands it; fetching an HTTPS index is the desktop app's job. An entry is
//! installed only after its download matches `size` and `sha256`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The `format` value of an index file.
pub const INDEX_FORMAT: &str = "fontcraft.extensions";
/// The newest index format version this build reads.
pub const INDEX_FORMAT_VERSION: u32 = 1;
/// Largest index file, in bytes.
pub const MAX_INDEX_BYTES: usize = 16 << 20;
/// Most entries in one index.
pub const MAX_INDEX_ENTRIES: usize = 10_000;
/// Longest string in an entry, in characters.
pub const MAX_INDEX_STRING: usize = 4_096;
/// Largest screenshot shown, in bytes.
pub const MAX_SCREENSHOT_BYTES: usize = 8 << 20;

/// Why an index or a download is refused.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum IndexError {
    #[error("the index is {0} bytes; at most {MAX_INDEX_BYTES}")]
    TooLarge(usize),
    #[error("the index is not valid JSON: {0}")]
    Json(String),
    #[error("not an extension index: {0}")]
    Format(String),
    #[error("the index is format version {0}; this version of the app reads up to {INDEX_FORMAT_VERSION}")]
    NewerVersion(u64),
    #[error("entry {entry}: {message}")]
    Entry { entry: usize, message: String },
    #[error("the download of `{id}` does not match its index entry: {message}")]
    Mismatch { id: String, message: String },
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

/// What an index entry installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Filter,
    Reporter,
    Check,
    Macro,
}

/// One extension an index offers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub kind: EntryKind,
    pub version: String,
    /// Titles by language tag (`en`, `de`…).
    pub titles: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
    pub vendor: String,
    /// Where the module or macro file is downloaded from (a path for a local index).
    pub url: String,
    /// SHA-256 of the file, lowercase hex.
    pub sha256: String,
    /// Size of the file, in bytes.
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
}

impl Entry {
    /// Checks downloaded `bytes` against `size` and `sha256`.
    pub fn verify(&self, bytes: &[u8]) -> Result<(), IndexError> {
        if bytes.len() as u64 != self.size {
            return Err(IndexError::Mismatch { id: self.id.clone(), message: format!("{} bytes, expected {}", bytes.len(), self.size) });
        }
        // M13.10: SHA-256.
        Err(IndexError::Unsupported("checking downloads".into()))
    }

    /// The title in `language`, else English, else the id.
    pub fn title(&self, language: &str) -> &str {
        self.titles.get(language).or_else(|| self.titles.get("en")).map_or(self.id.as_str(), String::as_str)
    }
}

/// A parsed index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub entries: Vec<Entry>,
}

impl Index {
    /// Reads an index file, checking format, version and every limit; a bad entry refuses the
    /// whole index with its number.
    pub fn parse(bytes: &[u8]) -> Result<Index, IndexError> {
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(IndexError::TooLarge(bytes.len()));
        }
        // M13.10.
        Err(IndexError::Unsupported("extension indexes".into()))
    }

    /// The entries of several indexes, in order; for an id listed more than once the first wins.
    pub fn merge(indexes: &[Index]) -> Index {
        let _ = indexes;
        // M13.10.
        Index::default()
    }
}
