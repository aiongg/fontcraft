//! Text kept by the browser between visits: the page's `localStorage`, read and written by key.
//!
//! Meant for `craft::platform` (craft-platform, a new `src/storage.rs`): the web build has no settings folder, and
//! craft's [`SectionStore`](craft::host::SectionStore) keeps sections in memory there. On the desktop, and wherever the
//! browser refuses storage (private windows, storage turned off), [`BrowserStorage::open`] returns `None` and the
//! caller keeps nothing.
//!
//! `localStorage` holds a few million characters per site, shared by every key; a write over that fails and leaves
//! the old value in place.

/// A [`BrowserStorage`] write the browser refused: storage is full or turned off.
pub const STORAGE_REFUSED: &str = "The browser did not keep the data: its storage for this site is full or turned off";

/// The page's `localStorage`. Open it where it is used: the browser's handle cannot move between threads.
pub struct BrowserStorage {
    #[cfg(target_arch = "wasm32")]
    storage: web_sys::Storage,
}

#[cfg(target_arch = "wasm32")]
impl BrowserStorage {
    /// The page's storage, or `None` where the browser refuses it.
    pub fn open() -> Option<BrowserStorage> {
        let storage = web_sys::window()?.local_storage().ok()??;
        Some(BrowserStorage { storage })
    }

    /// The text kept under `key`, if any.
    pub fn get(&self, key: &str) -> Option<String> {
        self.storage.get_item(key).ok().flatten()
    }

    /// Keeps `value` under `key`, replacing what was there; [`STORAGE_REFUSED`] when the browser refuses it.
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.storage.set_item(key, value).map_err(|_| STORAGE_REFUSED.to_string())
    }

    /// Drops what is kept under `key`.
    pub fn remove(&self, key: &str) {
        let _ = self.storage.remove_item(key);
    }
}

/// Outside a browser: nothing opens, so the methods are never reached.
#[cfg(not(target_arch = "wasm32"))]
impl BrowserStorage {
    /// `None`: there is no browser.
    pub fn open() -> Option<BrowserStorage> {
        None
    }

    /// Nothing is kept.
    pub fn get(&self, _key: &str) -> Option<String> {
        None
    }

    /// [`STORAGE_REFUSED`]: nothing is kept.
    pub fn set(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err(STORAGE_REFUSED.to_string())
    }

    /// Nothing is kept.
    pub fn remove(&self, _key: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outside a browser there is no storage to open.
    #[test]
    fn the_desktop_has_no_browser_storage() {
        assert!(BrowserStorage::open().is_none());
    }
}
