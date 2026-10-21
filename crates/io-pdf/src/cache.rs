//! The parsed first pages of recently drawn PDFs, shared by everything in the process that draws
//! or measures a placed PDF (the engine's image commands, the strike rasteriser, Edit View and
//! Font View), so the same bytes are parsed once.
//!
//! Entries are keyed by the file's length and a 64-bit hash of its contents, never by a path: the
//! same file under two paths is one entry, and a changed file is a new one. A failed parse is
//! kept too, so a file that cannot be read is not parsed again on every frame. The cache holds
//! at most [`MAX_PDF_CACHE_ENTRIES`] entries and [`MAX_PDF_CACHE_BYTES`] of parsed drawings,
//! least recently used dropped first; a drawing larger than that is returned without being kept.

use std::hash::Hasher;
use std::sync::{Arc, Mutex, PoisonError};

use crate::PdfError;
use crate::content::{PageDrawing, Painted, Seg};

/// Most PDFs whose parsed first page is kept.
pub const MAX_PDF_CACHE_ENTRIES: usize = 16;
/// Most bytes of parsed drawings kept, by the estimate of [`drawing_bytes`].
pub const MAX_PDF_CACHE_BYTES: usize = 64 << 20;

pub(crate) type Parsed = Result<Arc<PageDrawing>, PdfError>;

/// Which file: its length and the hash of its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    len: usize,
    hash: u64,
}

impl Key {
    pub(crate) fn of(data: &[u8]) -> Self {
        // `DefaultHasher::new` uses fixed keys, so equal bytes hash equally within the process.
        let mut h = std::hash::DefaultHasher::new();
        h.write(data);
        Self { len: data.len(), hash: h.finish() }
    }
}

struct Entry {
    key: Key,
    parsed: Parsed,
    bytes: usize,
    last_used: u64,
}

/// A bounded set of parsed pages. The process shares one ([`first_page`]); tests make their own.
pub(crate) struct PageCache {
    entries: Vec<Entry>,
    bytes: usize,
    uses: u64,
}

impl PageCache {
    pub(crate) const fn new() -> Self {
        Self { entries: Vec::new(), bytes: 0, uses: 0 }
    }

    fn lookup(&mut self, key: Key) -> Option<Parsed> {
        self.uses = self.uses.saturating_add(1);
        let uses = self.uses;
        let entry = self.entries.iter_mut().find(|e| e.key == key)?;
        entry.last_used = uses;
        Some(entry.parsed.clone())
    }

    fn insert(&mut self, key: Key, parsed: Parsed) {
        let bytes = match &parsed {
            Ok(drawing) => drawing_bytes(drawing),
            Err(_) => 0,
        };
        if bytes > MAX_PDF_CACHE_BYTES || self.entries.iter().any(|e| e.key == key) {
            return;
        }
        self.uses = self.uses.saturating_add(1);
        self.entries.push(Entry { key, parsed, bytes, last_used: self.uses });
        self.bytes = self.bytes.saturating_add(bytes);
        while self.entries.len() > MAX_PDF_CACHE_ENTRIES || self.bytes > MAX_PDF_CACHE_BYTES {
            let Some(oldest) = self.entries.iter().enumerate().min_by_key(|(_, e)| e.last_used).map(|(i, _)| i) else { break };
            let gone = self.entries.swap_remove(oldest);
            self.bytes = self.bytes.saturating_sub(gone.bytes);
        }
    }

    /// The parsed page of `data`: the kept one, else `parse(data)`, which is then kept.
    pub(crate) fn get(cache: &Mutex<Self>, data: &[u8], parse: impl FnOnce(&[u8]) -> Result<PageDrawing, PdfError>) -> Parsed {
        let key = Key::of(data);
        if let Some(hit) = lock(cache).lookup(key) {
            return hit;
        }
        // Parsing runs outside the lock, so a slow file does not hold up the window's drawing.
        let parsed = parse(data).map(Arc::new);
        lock(cache).insert(key, parsed.clone());
        parsed
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn holds(&self, data: &[u8]) -> bool {
        let key = Key::of(data);
        self.entries.iter().any(|e| e.key == key)
    }
}

/// A panic while the lock was held leaves entries that are each whole, so the cache stays usable.
pub(crate) fn lock(cache: &Mutex<PageCache>) -> std::sync::MutexGuard<'_, PageCache> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) static SHARED: Mutex<PageCache> = Mutex::new(PageCache::new());

/// The first page of `data`, parsed once for the process while it stays in the shared cache.
pub(crate) fn first_page(data: &[u8]) -> Parsed {
    PageCache::get(&SHARED, data, crate::load::first_page_drawing)
}

/// About how much memory a parsed drawing takes.
fn drawing_bytes(d: &PageDrawing) -> usize {
    let names = d.dropped.elements.keys().chain(d.dropped.attributes.keys()).map(|k| k.len().saturating_add(48)).sum::<usize>();
    d.painted
        .iter()
        .map(|p| p.segs.capacity().saturating_mul(size_of::<Seg>()).saturating_add(size_of::<Painted>()))
        .fold(size_of::<PageDrawing>().saturating_add(names), usize::saturating_add)
}
