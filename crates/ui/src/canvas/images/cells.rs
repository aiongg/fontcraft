//! Placed images in Font View cells (`fontView.cell.imageDisplay`).
//!
//! A cell draws the image placed on its glyph's layer of Font View's master while View › Show Image is on, and always
//! when that layer has no paths and no components, behind the glyph, as Edit View draws it (its transform, crop and
//! opacity), clipped to the cell. A missing or unreadable file draws the missing-image marker, a format kept as a
//! reference the marker with its name.
//!
//! [`refresh`] works the images out after the projections, only while Font View shows and when the document, the
//! master, Show Image, the cell size or the decoded files moved. One pass decodes at most [`MAX_DECODES_PER_PASS`] files
//! (large ones on jobs, [`super::files`]); the others draw nothing yet and are taken on the next passes, so a font of
//! many images never stalls a frame. At most [`MAX_CELL_IMAGES`] glyphs show an image.
//!
//! A cell draws a thumbnail of its file no larger than the cell ([`Thumbs`]): a power of two from [`MIN_THUMB_SIDE`] to
//! [`MAX_THUMB_SIDE`] pixels on its longest side, made from the decoded file and kept per file and side, so a screenful
//! of cells holds little texture memory. One pass makes at most [`MAX_THUMBS_PER_PASS`] of them.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use craft::ui::contract::PlainData;
use fontcraft_engine::imagery::Rgba8;
use fontcraft_model::LayerSide;

use super::files::{Loaded, Pixels, shrunk};
use super::{PlacedImage, drawn_as_color, shown, side_image};
use crate::app::FontCraftApp;

/// Most files one pass reads and decodes for the cells.
pub const MAX_DECODES_PER_PASS: usize = 4;
/// Most glyphs whose cells show an image.
pub const MAX_CELL_IMAGES: usize = 20_000;
/// Smallest and largest thumbnail side, in pixels.
pub const MIN_THUMB_SIDE: u32 = 32;
pub const MAX_THUMB_SIDE: u32 = 1_024;
/// Most thumbnails one pass makes.
pub const MAX_THUMBS_PER_PASS: usize = 8;
/// Most thumbnails kept, and most bytes of their pixels; the ones used longest ago go first.
pub const MAX_THUMBS: usize = 4_096;
pub const MAX_THUMB_BYTES: usize = 128 << 20;

/// One thumbnail: made from `source`, the decoded file it shrinks, held weakly so a thumbnail never keeps a large
/// file's pixels alive.
struct Thumb {
    source: Weak<Rgba8>,
    pixels: Pixels,
    bytes: usize,
    /// The pass that last used it.
    used: u64,
}

/// The cells' thumbnails, by file path and longest side.
#[derive(Default)]
pub struct Thumbs {
    kept: HashMap<(String, u32), Thumb>,
    bytes: usize,
    pass: u64,
}

impl Thumbs {
    /// The thumbnail of the file at `path` decoded as `source`, at most `side` pixels on its longest side, when one
    /// made from that same decoding is kept.
    fn kept(&mut self, path: &str, side: u32, source: &Pixels) -> Option<Pixels> {
        let pass = self.pass;
        let thumb = self.kept.get_mut(&(path.to_string(), side)).filter(|t| std::ptr::eq(t.source.as_ptr(), Arc::as_ptr(&source.0)))?;
        thumb.used = pass;
        Some(thumb.pixels.clone())
    }

    /// Makes and keeps the thumbnail [`Self::kept`] finds; a file no larger than `side` is its own thumbnail.
    fn make(&mut self, path: &str, side: u32, source: &Pixels) -> Pixels {
        let pixels = match shrunk(&source.0, side) {
            Some(small) => Pixels(Arc::new(small)),
            None => source.clone(),
        };
        let bytes = pixels.0.pixels.len();
        let thumb = Thumb { source: Arc::downgrade(&source.0), pixels: pixels.clone(), bytes, used: self.pass };
        if let Some(old) = self.kept.insert((path.to_string(), side), thumb) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        while self.kept.len() > MAX_THUMBS || self.bytes > MAX_THUMB_BYTES {
            // The one used longest ago; the one just made was used in this pass and goes last.
            let Some(oldest) = self.kept.iter().min_by_key(|(_, t)| t.used).map(|(k, _)| k.clone()) else { break };
            if let Some(gone) = self.kept.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(gone.bytes);
            }
        }
        pixels
    }
}

/// The thumbnail side for cells `cell` points wide at `pixels_per_point`.
pub fn thumb_side(cell: f64, pixels_per_point: f32) -> u32 {
    let px = cell * f64::from(pixels_per_point);
    let px = if px.is_finite() { px.round().clamp(1.0, f64::from(MAX_THUMB_SIDE)) as u32 } else { MAX_THUMB_SIDE };
    px.clamp(MIN_THUMB_SIDE, MAX_THUMB_SIDE).next_power_of_two()
}

/// The images Font View cells draw, by glyph name.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CellImages {
    #[plain_data(assume = "placed images in font units; decoded pixels behind an Arc, never changed once made")]
    pub by_glyph: BTreeMap<String, Arc<PlacedImage>>,
}

/// What the cell images were last worked out from: the document and its revision, the master, Show Image, the file
/// cache's generation and the thumbnail side.
pub type CellsKey = (Option<(u64, u64)>, Option<String>, bool, u64, u32);

/// Works out the cell images when what they come from moved, while Font View shows.
pub fn refresh(app: &mut FontCraftApp) {
    if !app.font_view_shows() {
        return;
    }
    let master = app.models.font_view.master.clone();
    let show = app.view.display.image;
    let side = thumb_side(app.models.font_view.cell_size, app.window.as_ref().map_or(1.0, egui::Context::pixels_per_point));
    let key: CellsKey = (app.session.active().map(|d| (d.id, d.revision)), master.clone(), show, app.images.files.generation(), side);
    if app.images.cells_seen.as_ref() == Some(&key) {
        return;
    }
    let assets = app.session.assets();
    let mut by_glyph = BTreeMap::new();
    let (mut waiting, mut deferred) = (false, false);
    app.images.thumbs.pass = app.images.thumbs.pass.wrapping_add(1);
    if let (Ok(font), Some(master)) = (app.session.font(), master.as_deref()) {
        let mut budget = MAX_DECODES_PER_PASS;
        let mut thumbs_left = MAX_THUMBS_PER_PASS;
        for glyph in font.glyphs.iter() {
            if by_glyph.len() >= MAX_CELL_IMAGES {
                break;
            }
            let Some(layer) = glyph.layer(master) else { continue };
            if drawn_as_color(layer) {
                continue;
            }
            let Some(image) = side_image(layer, LayerSide::Foreground).filter(|_| shown(show, layer, LayerSide::Foreground)) else { continue };
            let loaded = match image.path.as_deref() {
                None => None,
                Some(path) => match app.images.files.fresh(path) {
                    Some(found) => Some(found.clone()),
                    None if budget > 0 => {
                        budget -= 1;
                        Some(app.images.files.get(&assets, path, Some(&*app.host), app.window.as_ref()))
                    }
                    None => {
                        deferred = true;
                        Some(Loaded::Pending { size: [0, 0] })
                    }
                },
            };
            // The cell draws a thumbnail, never the decoded file.
            let loaded = match (loaded, image.path.as_deref()) {
                (Some(Loaded::Ready { pixels, size }), Some(path)) => match app.images.thumbs.kept(path, side, &pixels) {
                    Some(thumb) => Some(Loaded::Ready { pixels: thumb, size }),
                    None if thumbs_left > 0 => {
                        thumbs_left -= 1;
                        Some(Loaded::Ready { pixels: app.images.thumbs.make(path, side, &pixels), size })
                    }
                    None => {
                        deferred = true;
                        Some(Loaded::Pending { size })
                    }
                },
                (other, _) => other,
            };
            waiting |= matches!(loaded, Some(Loaded::Pending { .. }));
            by_glyph.insert(glyph.name.clone(), Arc::new(PlacedImage::new(image, loaded.as_ref())));
        }
    }
    crate::app::models::set(&mut app.models.cell_images, CellImages { by_glyph });
    // Files still to read or decode: look again on the next pass.
    app.images.cells_seen = if waiting { None } else { Some(key) };
    // A job wakes the window when it lands; files left for a later pass need a pass of their own.
    if deferred && let Some(ctx) = &app.window {
        ctx.request_repaint();
    }
}
