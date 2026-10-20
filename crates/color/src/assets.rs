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
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Stored path → bytes. A stored path is relative to the document's folder and may climb out
/// of it with `..`; an unsaved document stores absolute paths. Implementations cap every read at
/// [`MAX_ASSET_BYTES`] and never panic on a path: a path they cannot resolve (an empty one,
/// one holding a NUL, or a relative one with no document folder) is an [`AssetError`].
pub trait AssetStore: Send + Sync {
    /// The bytes of the file at `path`, shared so a cache can hand out the same buffer.
    fn read(&self, path: &str) -> Result<Arc<[u8]>, AssetError>;
    /// Whether `path` names a file the store can read, without reading it.
    fn exists(&self, path: &str) -> bool;
    /// The start of the file at `path`, at least `max` bytes of it when it is that long, for
    /// reading a header without reading a large file whole. Refused as [`AssetStore::read`]
    /// refuses. A store that holds whole files gives the whole file.
    fn read_head(&self, path: &str, max: usize) -> Result<Arc<[u8]>, AssetError> {
        let _ = max;
        self.read(path)
    }

    /// A value that changes whenever the file at `path` changes (from its modification time and
    /// size, say), found without reading the file; `None` when the store cannot tell or cannot
    /// read the file. A cache of decoded images checks it to keep what is still current.
    fn version(&self, path: &str) -> Option<u64> {
        let _ = path;
        None
    }
}

/// The version the next write to any [`MemoryAssets`] gets. One counter for every store, so a
/// store rebuilt from scratch or cloned never repeats a version another write had.
static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// Files held in memory, keyed by stored path, each with the version of its last write.
#[derive(Clone, Debug, Default)]
pub struct MemoryAssets {
    files: BTreeMap<String, (Arc<[u8]>, u64)>,
}

/// Two stores are equal when they hold the same bytes at the same paths, whatever their versions.
impl PartialEq for MemoryAssets {
    fn eq(&self, other: &Self) -> bool {
        self.files.len() == other.files.len() && self.files.iter().zip(&other.files).all(|((p, (a, _)), (q, (b, _)))| p == q && a == b)
    }
}

impl MemoryAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the file at `path`. A file over [`MAX_ASSET_BYTES`] is kept, and
    /// [`AssetStore::read`] refuses it, as a file store would.
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Arc<[u8]>>) {
        let version = NEXT_VERSION.fetch_add(1, Ordering::Relaxed);
        self.files.insert(path.into(), (bytes.into(), version));
    }

    pub fn remove(&mut self, path: &str) -> Option<Arc<[u8]>> {
        self.files.remove(path).map(|(bytes, _)| bytes)
    }

    /// The stored paths, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

impl AssetStore for MemoryAssets {
    fn read(&self, path: &str) -> Result<Arc<[u8]>, AssetError> {
        let (bytes, _) = self.files.get(path).ok_or_else(|| AssetError::Missing(path.to_string()))?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err(AssetError::TooLarge { path: path.to_string(), size: bytes.len() as u64, limit: MAX_ASSET_BYTES });
        }
        Ok(Arc::clone(bytes))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Every write gets a new version, so a file replaced by one of the same length is told apart.
    fn version(&self, path: &str) -> Option<u64> {
        self.files.get(path).map(|(_, version)| *version)
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetStore, MemoryAssets};

    #[test]
    fn replacing_a_file_changes_its_version() {
        let mut assets = MemoryAssets::new();
        assets.insert("a.png", vec![1u8, 2, 3]);
        let first = assets.version("a.png");
        // Same length, and the freed buffer may be reused for the new one.
        assets.insert("a.png", vec![4u8, 5, 6]);
        let second = assets.version("a.png");
        assert!(first.is_some() && second.is_some() && first != second, "{first:?} {second:?}");
        assert_eq!(assets.version("missing.png"), None);
        let mut other = MemoryAssets::new();
        other.insert("a.png", vec![4u8, 5, 6]);
        assert_eq!(assets, other, "equality compares the bytes, not the versions");
    }
}
