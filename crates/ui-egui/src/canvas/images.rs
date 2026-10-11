//! Placed images in Edit View (M11.13 draws them; M11.14 draws iColor and svg layers through the
//! same cache): decoded textures keyed by stored path and revision.
//!
//! Rows: `view.showImage`, `image.alwaysShownForEmptyGlyphs`, `image.storedAsRelativePath` (the
//! missing-image marker), `color.sbix.sizeScalesImage` (drawing).
//!
//! Contract (`docs/plans/M11.md` § 3 › Limits):
//! - Bytes come from the session's asset store (`Session::assets`), never from a file opened
//!   here, and are decoded by `fontcraft_imagery` within its limits.
//! - Textures are kept up to [`MAX_TEXTURE_BYTES`] of decoded pixels, least recently used
//!   evicted. A file that is missing, cannot be decoded or is a kept-only format (PDF, PSD) is
//!   cached as such, so it is drawn as a marker without being read again every frame.
//! - The key's revision is a number the app raises when files on disk may have changed, so a
//!   changed file is decoded again.

use std::collections::HashMap;

use egui::TextureHandle;
use fontcraft_engine::color::AssetStore;
use fontcraft_engine::imagery::ImageFormat;
use fontcraft_geom::Affine;
use fontcraft_model::Image;

use super::Painting;

/// Most bytes of decoded pixels kept as textures.
pub const MAX_TEXTURE_BYTES: usize = 512 << 20;

/// Which file, at which revision.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    /// The stored path, as the asset store takes it.
    pub path: String,
    pub revision: u64,
}

/// What the cache holds for a key.
#[derive(Clone)]
pub enum CachedImage {
    /// A texture and the image's size in pixels.
    Ready { texture: TextureHandle, size: [u32; 2] },
    /// A format kept as a reference but not decoded.
    Unsupported(ImageFormat),
    /// The file could not be read or decoded; the message says why.
    Failed(String),
}

struct Entry {
    image: CachedImage,
    /// Decoded bytes the entry holds (0 unless ready).
    bytes: usize,
    /// The cache's use counter when the entry was last asked for.
    last_used: u64,
}

/// Decoded image textures.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<ImageKey, Entry>,
    bytes: usize,
    uses: u64,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The image for `key`, decoded and uploaded on first use.
    pub fn get(&mut self, ctx: &egui::Context, assets: &dyn AssetStore, key: &ImageKey) -> Option<&CachedImage> {
        // M11.13: read, decode, upload, evict beyond MAX_TEXTURE_BYTES.
        let _ = (ctx, assets);
        self.uses = self.uses.saturating_add(1);
        let uses = self.uses;
        let entry = self.entries.get_mut(key)?;
        entry.last_used = uses;
        Some(&entry.image)
    }

    /// Decoded bytes held.
    pub fn len_bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Entries held, for tests.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[allow(dead_code)] // M11.13: entries are added when decoded.
    fn entry_bytes(&self, key: &ImageKey) -> usize {
        self.entries.get(key).map_or(0, |e| e.bytes)
    }
}

/// Draws `image` of the edited layer: `to_screen` maps font units to the screen; `revision` is
/// the app's file revision. A missing or undecodable file is drawn as a marker.
pub fn draw_layer_image(
    painting: &Painting,
    cache: &mut ImageCache,
    ctx: &egui::Context,
    assets: &dyn AssetStore,
    image: &Image,
    to_screen: Affine,
    revision: u64,
) {
    // M11.13
    let _ = (painting, cache, ctx, assets, image, to_screen, revision);
}
