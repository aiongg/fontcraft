//! Where image and SVG bytes come from (`docs/plans/M11.md` § 3 › Where image bytes come from).
//!
//! The compiler and the UI never open files themselves: they ask an [`AssetStore`] for the bytes
//! of a stored path (an [`fontcraft_model::Image::path`], relative to the document's folder, or
//! absolute until an unsaved document is first saved). The store resolves and caches.
//!
//! | Store | Where | Used by |
//! |---|---|---|
//! | `fontcraft_engine::assets::FsAssets` | files beside the document, cached by path and modification time | the desktop app, the CLI |
//! | [`MemoryAssets`] | bytes held in memory | tests, the web build (uploaded files) |
//!
//! This keeps `fontcraft-compile` and the UI building for wasm and makes export reproducible in
//! tests.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::MAX_ASSET_BYTES;

/// Why an asset could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AssetError {
    #[error("the file `{0}` was not found")]
    Missing(String),
    #[error("the file `{path}` is {size} bytes; at most {limit} are read")]
    TooLarge { path: String, size: u64, limit: usize },
    #[error("the file `{path}` cannot be read: {message}")]
    Unreadable { path: String, message: String },
    /// A store whose M11 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// Stored path → bytes. Implementations cap every read at [`MAX_ASSET_BYTES`] and never panic
/// on a path: an empty, absolute, `..`-climbing or non-UTF-8 path is a [`AssetError`].
pub trait AssetStore: Send + Sync {
    /// The bytes of the file at `path`, shared so a cache can hand out the same buffer.
    fn read(&self, path: &str) -> Result<Arc<[u8]>, AssetError>;
    /// Whether `path` names a file the store can read, without reading it.
    fn exists(&self, path: &str) -> bool;
}

/// Files held in memory, keyed by stored path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryAssets {
    files: BTreeMap<String, Arc<[u8]>>,
}

impl MemoryAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the file at `path`. A file over [`MAX_ASSET_BYTES`] is kept, and
    /// [`AssetStore::read`] refuses it, as a file store would.
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Arc<[u8]>>) {
        self.files.insert(path.into(), bytes.into());
    }

    pub fn remove(&mut self, path: &str) -> Option<Arc<[u8]>> {
        self.files.remove(path)
    }

    /// The stored paths, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

impl AssetStore for MemoryAssets {
    fn read(&self, path: &str) -> Result<Arc<[u8]>, AssetError> {
        let bytes = self.files.get(path).ok_or_else(|| AssetError::Missing(path.to_string()))?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err(AssetError::TooLarge { path: path.to_string(), size: bytes.len() as u64, limit: MAX_ASSET_BYTES });
        }
        Ok(Arc::clone(bytes))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
}
