//! Image and SVG files of a document, for display and export (`docs/plans/M11.md` § 3 › Where
//! image bytes come from).
//!
//! Rows: `image.storedAsRelativePath`, `color.images.referencedNotEmbedded`.
//!
//! Contract:
//! - Stored paths are relative to the document's folder, and may climb out of it with `..`: the
//!   document stores only a relative path, wherever the file is. An unsaved document stores
//!   absolute paths; a save into another folder (the first save included) rewrites every image
//!   path relative to the new file ([`rebase_image_paths`], built on [`relative_to`]). Undo steps
//!   keep the paths of the folder they were made in; undo and redo rewrite them for the
//!   document's current folder ([`rebased`]).
//! - [`FsAssets`] reads regular files at most [`fontcraft_color::MAX_ASSET_BYTES`] long and
//!   caches their bytes by absolute path and modification time, at most [`MAX_CACHE_BYTES`] in
//!   all, least recently used evicted. The cache is shared by every store of one session.
//! - [`Session::assets`] gives the store of the active document: the file store on the desktop
//!   and in the CLI, the files uploaded to the session in the web build.
//! - A path that is empty, holds a NUL, names a network share or a device, is relative in an
//!   unsaved document or cannot be read is an [`AssetError`], never a panic. Absolute paths and
//!   paths that climb out of the folder with `..` are read, except outside the automation read
//!   root while one is in force ([`crate::file_access`]): such an image is missing.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use fontcraft_color::{AssetError, AssetStore, MAX_ASSET_BYTES, MemoryAssets};
#[cfg(not(target_arch = "wasm32"))]
use fontcraft_model::{Layer, Shape};

use crate::Session;

/// Most bytes the session's file cache keeps.
pub const MAX_CACHE_BYTES: usize = 256 << 20;
/// Most files one Add Image call takes.
pub const MAX_ADD_IMAGE_FILES: usize = 10_000;

type CacheKey = (PathBuf, Option<SystemTime>);

/// File bytes by absolute path and modification time, shared by a session's stores.
#[derive(Debug, Default)]
pub struct ByteCache {
    entries: Mutex<HashMap<CacheKey, CacheEntry>>,
    /// Counts reads, so entries can be ordered by last use.
    tick: AtomicU64,
    /// The byte budget; `None` is [`MAX_CACHE_BYTES`] (tests use a smaller one).
    limit: Option<usize>,
}

#[derive(Debug)]
struct CacheEntry {
    bytes: Arc<[u8]>,
    /// When the entry was last handed out, counted in reads, for eviction.
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

    fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::Relaxed).saturating_add(1)
    }

    fn get(&self, key: &CacheKey) -> Option<Arc<[u8]>> {
        let tick = self.next_tick();
        let mut entries = self.entries.lock().ok()?;
        let entry = entries.get_mut(key)?;
        entry.last_used = tick;
        Some(Arc::clone(&entry.bytes))
    }

    /// Keeps `bytes` for `key`, drops older versions of the same file, then evicts the least
    /// recently used files until the cache is within [`MAX_CACHE_BYTES`].
    fn put(&self, key: CacheKey, bytes: Arc<[u8]>) {
        let limit = self.limit.unwrap_or(MAX_CACHE_BYTES);
        if bytes.len() > limit {
            return;
        }
        let tick = self.next_tick();
        let Ok(mut entries) = self.entries.lock() else { return };
        entries.retain(|(path, _), _| *path != key.0);
        entries.insert(key, CacheEntry { bytes, last_used: tick });
        let mut total: usize = entries.values().map(|v| v.bytes.len()).sum();
        while total > limit {
            let Some(oldest) = entries.iter().min_by_key(|(_, v)| v.last_used).map(|(k, _)| k.clone()) else { break };
            if let Some(gone) = entries.remove(&oldest) {
                total = total.saturating_sub(gone.bytes.len());
            }
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

    /// The file a stored path names, or `None` when the path is not usable: empty, holding a
    /// NUL, a network or device path ([`is_network_or_device`]), or relative while the document
    /// has no folder.
    pub fn resolve(&self, stored: &str) -> Option<PathBuf> {
        if stored.is_empty() || stored.contains('\0') || is_network_or_device(stored) {
            return None;
        }
        let path = Path::new(stored);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        Some(self.base.as_ref()?.join(path))
    }

    fn unusable(&self, stored: &str) -> AssetError {
        let message = if is_network_or_device(stored) {
            "network and device paths are not read"
        } else if self.base.is_none() && !stored.is_empty() {
            "a relative path needs a saved document"
        } else {
            "the path is empty or not usable"
        };
        AssetError::Unreadable { path: stored.to_string(), message: message.into() }
    }
}

/// Whether `stored` starts with two separators, `/` or `\` in any mix: a network share
/// (`\\host\share`, `//host/share`) or a device or verbatim path (`\\?\`, `\\.\`). A document
/// names image files; opening one of these could reach another machine or a device, so they
/// are refused on every platform.
pub fn is_network_or_device(stored: &str) -> bool {
    let mut chars = stored.chars();
    let separator = |c: Option<char>| matches!(c, Some('/' | '\\'));
    separator(chars.next()) && separator(chars.next())
}

impl FsAssets {
    /// The file `path` names and its metadata, refused as [`AssetStore::read`] refuses.
    fn checked(&self, path: &str) -> Result<(PathBuf, std::fs::Metadata), AssetError> {
        let file = self.resolve(path).ok_or_else(|| self.unusable(path))?;
        // An image outside the automation read root shows as missing.
        if crate::file_access::check_read(&file).is_err() {
            return Err(AssetError::Missing(path.to_string()));
        }
        let meta = std::fs::metadata(&file).map_err(|e| unreadable(path, e))?;
        if !meta.is_file() {
            return Err(AssetError::Unreadable { path: path.to_string(), message: "it is not a file".into() });
        }
        if meta.len() > u64::try_from(MAX_ASSET_BYTES).unwrap_or(u64::MAX) {
            return Err(AssetError::TooLarge { path: path.to_string(), size: meta.len(), limit: MAX_ASSET_BYTES });
        }
        Ok((file, meta))
    }
}

fn unreadable(path: &str, e: std::io::Error) -> AssetError {
    match e.kind() {
        std::io::ErrorKind::NotFound => AssetError::Missing(path.to_string()),
        _ => AssetError::Unreadable { path: path.to_string(), message: e.to_string() },
    }
}

impl AssetStore for FsAssets {
    fn read(&self, path: &str) -> Result<Arc<[u8]>, AssetError> {
        use std::io::Read;
        let (file, meta) = self.checked(path)?;
        let limit = u64::try_from(MAX_ASSET_BYTES).unwrap_or(u64::MAX);
        // A file without a modification time is read every time: nothing tells a stale copy.
        let key = meta.modified().ok().map(|t| (file.clone(), Some(t)));
        if let Some(bytes) = key.as_ref().and_then(|k| self.cache.get(k)) {
            return Ok(bytes);
        }
        let mut buf = Vec::new();
        // The file may grow between the size check and the read.
        std::fs::File::open(&file).and_then(|f| f.take(limit.saturating_add(1)).read_to_end(&mut buf)).map_err(|e| unreadable(path, e))?;
        if buf.len() > MAX_ASSET_BYTES {
            return Err(AssetError::TooLarge { path: path.to_string(), size: buf.len() as u64, limit: MAX_ASSET_BYTES });
        }
        let bytes: Arc<[u8]> = buf.into();
        if let Some(key) = key {
            self.cache.put(key, Arc::clone(&bytes));
        }
        Ok(bytes)
    }

    fn read_head(&self, path: &str, max: usize) -> Result<Arc<[u8]>, AssetError> {
        use std::io::Read;
        let (file, meta) = self.checked(path)?;
        if let Some(bytes) = meta.modified().ok().and_then(|t| self.cache.get(&(file.clone(), Some(t)))) {
            return Ok(bytes);
        }
        let mut buf = Vec::new();
        let max = u64::try_from(max.min(MAX_ASSET_BYTES)).unwrap_or(0);
        std::fs::File::open(&file).and_then(|f| f.take(max).read_to_end(&mut buf)).map_err(|e| unreadable(path, e))?;
        Ok(buf.into())
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).filter(|f| crate::file_access::check_read(f).is_ok()).and_then(|f| std::fs::metadata(f).ok()).is_some_and(|m| m.is_file())
    }

    fn version(&self, path: &str) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let (_, meta) = self.checked(path).ok()?;
        let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (modified.as_secs(), modified.subsec_nanos(), meta.len()).hash(&mut h);
        Some(h.finish())
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
            Arc::new(FsAssets::new(self.document_folder(), Arc::clone(&self.assets.cache)))
        }
    }

    /// The file a stored image path of the active document names, for revealing it in the file
    /// manager; `None` in the web build and for a path that is not usable.
    pub fn asset_file(&self, stored: &str) -> Option<PathBuf> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = stored;
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            FsAssets::new(self.document_folder(), Arc::clone(&self.assets.cache)).resolve(stored)
        }
    }

    /// The absolute folder of the active document, if it has been saved.
    #[cfg(not(target_arch = "wasm32"))]
    fn document_folder(&self) -> Option<PathBuf> {
        let path = self.active()?.path.as_deref()?;
        std::path::absolute(path).ok()?.parent().map(Path::to_path_buf)
    }

    /// The path Add Image stores for `file`. In a saved document: an absolute `file` relative
    /// to the document's folder, a relative one as given (it is already relative to that
    /// folder). In an unsaved document: absolute, a relative `file` taken from the working
    /// directory. In the web build: as given, the name of an uploaded file.
    pub(crate) fn stored_image_path(&self, file: &str) -> Option<String> {
        if file.is_empty() || file.contains('\0') {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        {
            Some(file.to_string())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = Path::new(file);
            match self.document_folder() {
                Some(base) if path.is_absolute() => Some(relative_to(file, None, &base).unwrap_or_else(|| file.to_string())),
                Some(_) => Some(file.to_string()),
                None if path.is_absolute() => Some(file.to_string()),
                None => std::path::absolute(path).ok()?.to_str().map(str::to_string),
            }
        }
    }
}

/// `stored` (absolute, or relative to `old_base`) rewritten relative to `new_base`, for the first
/// save of a document; `None` leaves it as it is (on another drive, or not usable).
///
/// The result uses `/` between folders, as the file formats store it, and climbs with `..`
/// where needed. Both folders must be absolute; the rewrite is lexical and touches no file.
pub fn relative_to(stored: &str, old_base: Option<&Path>, new_base: &Path) -> Option<String> {
    if stored.is_empty() || stored.contains('\0') {
        return None;
    }
    let stored_path = Path::new(stored);
    let file_path = if stored_path.is_absolute() { normalize(stored_path)? } else { normalize(&old_base?.join(stored_path))? };
    let base_path = normalize(new_base)?;
    let file: Vec<Component> = file_path.components().collect();
    let base: Vec<Component> = base_path.components().collect();
    // A different root or drive has no relative path.
    if file.first() != base.first() {
        return None;
    }
    let common = file.iter().zip(&base).take_while(|(a, b)| a == b).count();
    // The folder itself is not a file.
    if file.len() <= common {
        return None;
    }
    let mut parts: Vec<&str> = vec![".."; base.len().saturating_sub(common)];
    for c in file.get(common..)? {
        parts.push(c.as_os_str().to_str()?);
    }
    Some(parts.join("/"))
}

/// An absolute `path` with `.` dropped and `..` applied; `None` for a relative path or one that
/// climbs above its root.
fn normalize(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// Rewrites every image path of `font` for a document about to be written to `new_path`, last
/// saved at `old_path` (`None`: never saved): each becomes relative to the new file's folder.
/// Nothing changes when the folder stays the same. Returns how many paths changed.
#[cfg(not(target_arch = "wasm32"))]
pub fn rebase_image_paths(font: &mut fontcraft_model::Font, old_path: Option<&Path>, new_path: &Path) -> usize {
    let folder = |p: &Path| std::path::absolute(p).ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let Some(new_base) = folder(new_path) else { return 0 };
    let old_base = old_path.and_then(folder);
    if old_base.as_deref() == Some(new_base.as_path()) {
        return 0;
    }
    let mut changed = 0;
    for i in 0..font.glyphs.as_arcs().len() {
        // Only glyphs with images are copied for editing.
        if !font.glyphs.as_arcs().get(i).is_some_and(|g| g.layers.iter().any(has_image)) {
            continue;
        }
        let Some(mut glyph) = font.glyphs.get_mut_at(i) else { continue };
        for layer in glyph.layers.iter_mut().filter(|l| has_image(l)) {
            for image in images_mut(layer) {
                if let Some(stored) = &image.path
                    && let Some(rebased) = relative_to(stored, old_base.as_deref(), &new_base)
                    && rebased != *stored
                {
                    image.path = Some(rebased);
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// `font` with its image paths rewritten from `old_path`'s folder to `new_path`'s, for a font kept
/// from before a save into another folder (an undo step, a batch's starting point). The same
/// font when nothing changes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebased(font: Arc<fontcraft_model::Font>, old_path: Option<&Path>, new_path: Option<&Path>) -> Arc<fontcraft_model::Font> {
    let Some(new_path) = new_path else { return font };
    if old_path == Some(new_path) {
        return font;
    }
    let mut copy = fontcraft_model::Font::clone(&font);
    if rebase_image_paths(&mut copy, old_path, new_path) == 0 { font } else { Arc::new(copy) }
}

/// The web build stores uploaded file names, which no save rewrites.
#[cfg(target_arch = "wasm32")]
pub(crate) fn rebased(font: Arc<fontcraft_model::Font>, _: Option<&Path>, _: Option<&Path>) -> Arc<fontcraft_model::Font> {
    font
}

/// Whether `layer` holds a placed image, an image shape, or either in its background.
#[cfg(not(target_arch = "wasm32"))]
fn has_image(layer: &Layer) -> bool {
    let shapes = |s: &[Shape]| s.iter().any(|s| matches!(s, Shape::Image(_)));
    layer.info.image.is_some() || shapes(&layer.shapes) || layer.info.background.as_ref().is_some_and(|b| b.image.is_some() || shapes(&b.shapes))
}

/// Every image of `layer`: the placed image, the background's, and image shapes on both sides.
#[cfg(not(target_arch = "wasm32"))]
fn images_mut(layer: &mut Layer) -> Vec<&mut fontcraft_model::Image> {
    fn shape_image(s: &mut Shape) -> Option<&mut fontcraft_model::Image> {
        match s {
            Shape::Image(i) => Some(&mut **i),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let info = &mut *layer.info;
    out.extend(info.image.as_mut());
    if let Some(b) = info.background.as_mut() {
        out.extend(b.image.as_mut());
        out.extend(b.shapes.iter_mut().filter_map(shape_image));
    }
    out.extend(layer.shapes.iter_mut().filter_map(shape_image));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn paths_are_rewritten_relative_to_the_new_folder() {
        let new = Path::new("/fonts/Sans");
        assert_eq!(relative_to("/fonts/Sans/images/A.png", None, new).as_deref(), Some("images/A.png"));
        assert_eq!(relative_to("/scans/A.png", None, new).as_deref(), Some("../../scans/A.png"));
        assert_eq!(relative_to("images/A.png", Some(Path::new("/old")), new).as_deref(), Some("../../old/images/A.png"));
        assert_eq!(relative_to("../x/./A.png", Some(Path::new("/fonts/Sans/sub")), new).as_deref(), Some("x/A.png"));
        // Not usable: relative with no old folder, climbing above the root, empty, the folder.
        assert_eq!(relative_to("A.png", None, new), None);
        assert_eq!(relative_to("/../A.png", None, new), None);
        assert_eq!(relative_to("", None, new), None);
        assert_eq!(relative_to("/fonts/Sans", None, new), None);
    }

    #[test]
    fn network_and_device_paths_are_refused_with_a_message() {
        let store = FsAssets::new(Some(std::env::temp_dir()), Arc::new(ByteCache::default()));
        for stored in [r"\\host\share\A.png", "//host/share/A.png", r"\\?\C:\A.png", r"\\.\PhysicalDrive0", r"/\host\A.png", r"\/host/A.png"] {
            assert_eq!(store.resolve(stored), None, "{stored}");
            assert!(!store.exists(stored), "{stored}");
            match store.read(stored) {
                Err(AssetError::Unreadable { message, .. }) => assert!(message.contains("network and device"), "{stored}: {message}"),
                other => panic!("{stored}: {other:?}"),
            }
        }
        // An absolute path and one that climbs out of the folder are still resolved.
        assert!(store.resolve("../A.png").is_some());
        let absolute = std::env::temp_dir().join("A.png");
        assert_eq!(store.resolve(absolute.to_str().unwrap()), Some(absolute.clone()));
    }

    #[test]
    fn the_cache_evicts_the_least_recently_used_file() {
        let cache = ByteCache { limit: Some(1_000), ..ByteCache::default() };
        let chunk = 500;
        let key = |n: &str| (PathBuf::from(n), None);
        cache.put(key("a"), vec![0u8; chunk].into());
        cache.put(key("b"), vec![0u8; chunk].into());
        assert!(cache.get(&key("a")).is_some());
        cache.put(key("c"), vec![0u8; 16].into());
        assert!(cache.get(&key("b")).is_none(), "b was used least recently");
        assert!(cache.get(&key("a")).is_some() && cache.get(&key("c")).is_some());
        assert!(cache.len_bytes() <= 1_000);
        // A newer version of a file replaces the older one.
        cache.put((PathBuf::from("a"), Some(SystemTime::UNIX_EPOCH)), vec![1u8; 4].into());
        assert!(cache.get(&key("a")).is_none());
    }
}
