//! Image and SVG files of a document, for display and export (`docs/plans/M11.md` § 3 › Where
//! image bytes come from; M11.12 fills the bodies).
//!
//! Rows: `image.storedAsRelativePath`, `color.images.referencedNotEmbedded`.
//!
//! Contract:
//! - Stored paths are relative to the document's folder. An unsaved document stores absolute
//!   paths; its first save rewrites them relative to the new file ([`relative_to`]).
//! - [`FsAssets`] reads files at most [`fontcraft_color::MAX_ASSET_BYTES`] long and caches their
//!   bytes by absolute path and modification time, at most [`MAX_CACHE_BYTES`] in all, least
//!   recently used evicted. The cache is shared by every store of one session.
//! - [`Session::assets`] gives the store of the active document: the file store on the desktop
//!   and in the CLI, the files uploaded to the session in the web build.
//! - A path that is empty, climbs out of the folder with `..`, or cannot be read is an
//!   [`AssetError`], never a panic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use fontcraft_color::{AssetError, AssetStore, MemoryAssets};

use crate::Session;

/// Most bytes the session's file cache keeps.
pub const MAX_CACHE_BYTES: usize = 256 << 20;
/// Most files one Add Image call takes.
pub const MAX_ADD_IMAGE_FILES: usize = 10_000;

/// File bytes by absolute path and modification time, shared by a session's stores.
#[derive(Debug, Default)]
pub struct ByteCache {
    entries: Mutex<HashMap<(PathBuf, Option<SystemTime>), CacheEntry>>,
}

#[derive(Debug)]
struct CacheEntry {
    bytes: Arc<[u8]>,
    /// When the entry was last handed out, counted in reads, for eviction.
    #[allow(dead_code)] // M11.12: least recently used eviction.
    last_used: u64,
}

impl ByteCache {
    /// The bytes cached in all.
    pub fn len_bytes(&self) -> usize {
        self.entries.lock().map_or(0, |e| e.values().map(|v| v.bytes.len()).sum())
    }

    /// Forgets every file, so the next read goes to disk.
    pub fn clear(&self) {
        if let Ok(mut e) = self.entries.lock() {
            e.clear();
        }
    }
}

/// Files beside a document.
#[derive(Clone, Debug)]
pub struct FsAssets {
    /// The document's folder; `None` for an unsaved document, which stores absolute paths.
    pub base: Option<PathBuf>,
    cache: Arc<ByteCache>,
}

impl FsAssets {
    pub fn new(base: Option<PathBuf>, cache: Arc<ByteCache>) -> Self {
        Self { base, cache }
    }

    /// The file a stored path names, or `None` when the path is not usable.
    pub fn resolve(&self, stored: &str) -> Option<PathBuf> {
        // M11.12
        let _ = stored;
        None
    }
}

impl AssetStore for FsAssets {
    fn read(&self, path: &str) -> Result<Arc<[u8]>, AssetError> {
        // M11.12: read through `self.cache`.
        let _ = &self.cache;
        let _ = path;
        Err(AssetError::NotYetImplemented("reading image files"))
    }

    fn exists(&self, path: &str) -> bool {
        // M11.12
        let _ = path;
        false
    }
}

/// The session's asset state: the file cache, and the files uploaded in the web build.
#[derive(Debug, Default)]
pub struct SessionAssets {
    pub cache: Arc<ByteCache>,
    /// Files added to the session in memory (the web build's uploads, tests).
    pub memory: MemoryAssets,
}

impl Session {
    /// The asset store of the active document (see the module documentation).
    pub fn assets(&self) -> Arc<dyn AssetStore> {
        #[cfg(target_arch = "wasm32")]
        {
            Arc::new(self.assets.memory.clone())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let base = self.active().and_then(|d| d.path.as_deref()).and_then(Path::parent).map(Path::to_path_buf);
            Arc::new(FsAssets::new(base, Arc::clone(&self.assets.cache)))
        }
    }
}

/// `stored` (absolute, or relative to `old_base`) rewritten relative to `new_base`, for the first
/// save of a document; `None` leaves it as it is (on another drive, or not usable).
pub fn relative_to(stored: &str, old_base: Option<&Path>, new_base: &Path) -> Option<String> {
    // M11.12
    let _ = (stored, old_base, new_base);
    None
}
