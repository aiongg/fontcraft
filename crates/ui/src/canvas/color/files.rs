//! The files colour glyphs draw from, read once per document: iColor images decoded, svg files turned into outlines,
//! pattern images decoded. A file changed on disk shows once the document changes or the cache fills and empties.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_engine::color::AssetStore;
use fontcraft_engine::color::svg_draw::{self, SvgShape};
use fontcraft_engine::imagery::{self, Rgba8};
use fontcraft_geom::Affine;
use fontcraft_model::{Family, ShapeAttrs};
use fontcraft_render::color::artwork::Pattern;

/// Most files kept; past it the cache empties.
pub const MAX_COLOR_FILES: usize = 512;
/// Most bytes of decoded pixels kept; past it the cache empties.
pub const MAX_COLOR_FILE_BYTES: usize = 256 << 20;
/// Most pixels an iColor image or a pattern image is decoded to.
pub const MAX_COLOR_IMAGE_PIXELS: u64 = 1 << 24;
/// Longest side of an iColor image as a texture; a larger image is drawn from a smaller copy.
pub const MAX_BITMAP_SIDE: u32 = 4096;
/// The shape attribute that names a path's pattern image (the attributes commands' key).
pub const PATTERN_IMAGE_KEY: &str = "patternImage";

/// An iColor image ready to draw: its pixels (straight RGBA, rows top to bottom), and how many pixels of the file
/// each pixel spans, across and down (1 unless the file was larger than [`MAX_BITMAP_SIDE`]).
#[derive(Clone, Debug)]
pub struct Bitmap {
    pub image: Arc<Rgba8>,
    pub step: (f64, f64),
}

#[derive(Default)]
pub struct ColorFiles {
    doc: Option<u64>,
    bitmaps: HashMap<String, Option<Bitmap>>,
    svgs: HashMap<(String, [u64; 6]), Arc<Vec<SvgShape>>>,
    patterns: HashMap<String, Option<Pattern>>,
    bytes: usize,
}

impl ColorFiles {
    /// Empties the cache when `doc` is another document than the one it was filled for.
    pub fn follow(&mut self, doc: Option<u64>) {
        if self.doc != doc {
            *self = ColorFiles { doc, ..ColorFiles::default() };
        }
    }

    /// Makes room for `size` more bytes and one more file.
    fn room(&mut self, size: usize) {
        let files = self.bitmaps.len() + self.svgs.len() + self.patterns.len();
        if files >= MAX_COLOR_FILES || self.bytes.saturating_add(size) > MAX_COLOR_FILE_BYTES {
            let doc = self.doc;
            *self = ColorFiles { doc, ..ColorFiles::default() };
        }
        self.bytes = self.bytes.saturating_add(size);
    }

    /// The iColor image at `path`; `None` when it cannot be read or decoded.
    pub fn bitmap(&mut self, assets: &dyn AssetStore, path: &str) -> Option<Bitmap> {
        if let Some(found) = self.bitmaps.get(path) {
            return found.clone();
        }
        let decoded = assets.read(path).ok().and_then(|b| imagery::decode_rgba(&b, MAX_COLOR_IMAGE_PIXELS).ok()).and_then(fit);
        let size = decoded.as_ref().map_or(0, |b| b.image.pixels.len());
        self.room(size);
        self.bitmaps.insert(path.to_string(), decoded.clone());
        decoded
    }

    /// The outlines of the svg file at `path` placed by `place`; empty when it cannot be read.
    pub fn svg(&mut self, assets: &dyn AssetStore, path: &str, place: Affine) -> Arc<Vec<SvgShape>> {
        let key = (path.to_string(), place.as_coeffs().map(f64::to_bits));
        if let Some(found) = self.svgs.get(&key) {
            return Arc::clone(found);
        }
        let shapes = Arc::new(assets.read(path).ok().and_then(|b| svg_draw::shapes(&b, place).ok()).unwrap_or_default());
        self.room(0);
        self.svgs.insert(key, Arc::clone(&shapes));
        shapes
    }

    /// The pattern image a path's attributes name, straight pixels as decoded, one image pixel per font unit; `None`
    /// when they name none or it cannot be read (a PDF pattern is drawn by image export only).
    pub fn pattern(&mut self, assets: &dyn AssetStore, attrs: &ShapeAttrs) -> Option<Pattern> {
        let file = attrs.extra.get(Family::Native, PATTERN_IMAGE_KEY)?.as_str()?.trim();
        if file.is_empty() {
            return None;
        }
        if let Some(known) = self.patterns.get(file) {
            return known.clone();
        }
        let pattern = assets
            .read(file)
            .ok()
            .and_then(|b| imagery::decode_rgba(&b, MAX_COLOR_IMAGE_PIXELS).ok())
            .filter(whole)
            .map(|image| Pattern { image: Arc::new(image), unit: 1.0 });
        let size = pattern.as_ref().map_or(0, |p| p.image.pixels.len());
        self.room(size);
        self.patterns.insert(file.to_string(), pattern.clone());
        pattern
    }
}

/// Whether `image` has pixels and exactly `width × height` of them.
fn whole(image: &Rgba8) -> bool {
    let expected = u64::from(image.width) * u64::from(image.height) * 4;
    image.width > 0 && image.height > 0 && u64::try_from(image.pixels.len()).ok() == Some(expected)
}

/// `image` as an iColor bitmap, shrunk by nearest pixels to [`MAX_BITMAP_SIDE`] on its longer side.
fn fit(image: Rgba8) -> Option<Bitmap> {
    if !whole(&image) {
        return None;
    }
    let (w, h) = (image.width, image.height);
    if w <= MAX_BITMAP_SIDE && h <= MAX_BITMAP_SIDE {
        return Some(Bitmap { image: Arc::new(image), step: (1.0, 1.0) });
    }
    let k = f64::from(w.max(h)) / f64::from(MAX_BITMAP_SIDE);
    let (nw, nh) = (((f64::from(w) / k).floor() as u32).max(1), ((f64::from(h) / k).floor() as u32).max(1));
    let mut pixels = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in 0..nh {
        let sy = ((f64::from(y) + 0.5) * f64::from(h) / f64::from(nh)) as usize;
        for x in 0..nw {
            let sx = ((f64::from(x) + 0.5) * f64::from(w) / f64::from(nw)) as usize;
            let at = (sy.min(h as usize - 1) * w as usize + sx.min(w as usize - 1)) * 4;
            pixels.extend_from_slice(image.pixels.get(at..at + 4).unwrap_or(&[0, 0, 0, 0]));
        }
    }
    let step = (f64::from(w) / f64::from(nw), f64::from(h) / f64::from(nh));
    Some(Bitmap { image: Arc::new(Rgba8 { width: nw, height: nh, pixels }), step })
}
