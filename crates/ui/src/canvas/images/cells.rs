//! Placed images in Font View cells (`fontView.cell.imageDisplay`) and in the Space Center ([`space_center`]).
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

use craft::host::Host;
use craft::ui::contract::PlainData;
use fontcraft_engine::color::AssetStore;
use fontcraft_engine::imagery::Rgba8;
use fontcraft_model::{Image, LayerSide};

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

/// The images Font View cells or the Space Center draw, by glyph name.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CellImages {
    #[plain_data(assume = "placed images in font units; decoded pixels behind an Arc, never changed once made")]
    pub by_glyph: BTreeMap<String, Arc<PlacedImage>>,
}

/// What the cell images were last worked out from: the document and its revision, the master, Show Image, the file
/// cache's generation and the thumbnail side.
pub type CellsKey = (Option<(u64, u64)>, Option<String>, bool, u64, u32);

/// What one pass may still do: files to read and decode, thumbnails to make; and whether it left work for a later
/// pass or met a file still decoding.
struct Pass {
    decodes: usize,
    thumbs: usize,
    deferred: bool,
    waiting: bool,
}

impl Pass {
    fn new() -> Pass {
        Pass { decodes: MAX_DECODES_PER_PASS, thumbs: MAX_THUMBS_PER_PASS, deferred: false, waiting: false }
    }
}

/// How `image` is drawn at a thumbnail of at most `side` pixels: its file read through the cache within the pass's
/// budget, then shrunk. A file past the budget draws nothing yet.
fn load(
    images: &mut super::Images,
    assets: &Arc<dyn AssetStore>,
    host: &Host,
    window: Option<&egui::Context>,
    image: &Image,
    side: u32,
    pass: &mut Pass,
) -> PlacedImage {
    let loaded = match image.path.as_deref() {
        None => None,
        Some(path) => match images.files.fresh(path) {
            Some(found) => Some(found.clone()),
            None if pass.decodes > 0 => {
                pass.decodes -= 1;
                Some(images.files.get(assets, path, Some(host), window))
            }
            None => {
                pass.deferred = true;
                Some(Loaded::Pending { size: [0, 0] })
            }
        },
    };
    // A thumbnail is drawn, never the decoded file.
    let loaded = match (loaded, image.path.as_deref()) {
        (Some(Loaded::Ready { pixels, size }), Some(path)) => match images.thumbs.kept(path, side, &pixels) {
            Some(thumb) => Some(Loaded::Ready { pixels: thumb, size }),
            None if pass.thumbs > 0 => {
                pass.thumbs -= 1;
                Some(Loaded::Ready { pixels: images.thumbs.make(path, side, &pixels), size })
            }
            None => {
                pass.deferred = true;
                Some(Loaded::Pending { size })
            }
        },
        (other, _) => other,
    };
    pass.waiting |= matches!(loaded, Some(Loaded::Pending { .. }));
    PlacedImage::new(image, loaded.as_ref())
}

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
    let mut pass = Pass::new();
    app.images.thumbs.pass = app.images.thumbs.pass.wrapping_add(1);
    if let (Ok(font), Some(master)) = (app.session.font(), master.as_deref()) {
        for glyph in font.glyphs.iter() {
            if by_glyph.len() >= MAX_CELL_IMAGES {
                break;
            }
            let Some(layer) = glyph.layer(master) else { continue };
            if drawn_as_color(layer) {
                continue;
            }
            let Some(image) = side_image(layer, LayerSide::Foreground).filter(|_| shown(show, layer, LayerSide::Foreground)) else { continue };
            let placed = load(&mut app.images, &assets, &app.host, app.window.as_ref(), image, side, &mut pass);
            by_glyph.insert(glyph.name.clone(), Arc::new(placed));
        }
    }
    crate::app::models::set(&mut app.models.cell_images, CellImages { by_glyph });
    // Files still to read or decode: look again on the next pass.
    app.images.cells_seen = if pass.waiting { None } else { Some(key) };
    // A job wakes the window when it lands; files left for a later pass need a pass of their own.
    if pass.deferred
        && let Some(ctx) = &app.window
    {
        ctx.request_repaint();
    }
}

/// What the Space Center's images were last worked out from: the document and its revision, the glyphs drawn with
/// their layers, the file cache's generation and the thumbnail side.
pub type SpaceKey = (Option<(u64, u64)>, Vec<(String, String)>, u64, u32);

/// Works out the images the Space Center draws behind its glyphs (`view.setSpaceCenterDisplay {images}`): the
/// foreground image of each drawn glyph's layer, whether or not Show Image is on, at a thumbnail of the em at the
/// window's size. iColor and svg layers draw their image as their colour drawing instead. Runs after the projections,
/// only while the option is on and the window shows a font.
pub fn space_center(app: &mut FontCraftApp) {
    let hidden = app.view.workspace.layout().panel_state(&crate::shell::dock::PanelId::SpaceCenter) == craft::behavior::dock::PanelState::Hidden;
    let model = &app.models.space_center;
    if hidden || !model.has_font || !model.state.display.images || model.art.is_empty() {
        app.images.space_seen = None;
        if !app.models.space_images.by_glyph.is_empty() {
            crate::app::models::set(&mut app.models.space_images, CellImages::default());
        }
        return;
    }
    let drawn: Vec<(String, String)> = model.art.iter().map(|(name, art)| (name.clone(), art.layer.clone())).collect();
    let size = model.state.size;
    let side = thumb_side(size, app.window.as_ref().map_or(1.0, egui::Context::pixels_per_point));
    let key: SpaceKey = (app.session.active().map(|d| (d.id, d.revision)), drawn, app.images.files.generation(), side);
    if app.images.space_seen.as_ref() == Some(&key) {
        return;
    }
    let assets = app.session.assets();
    let mut by_glyph = BTreeMap::new();
    let mut pass = Pass::new();
    app.images.thumbs.pass = app.images.thumbs.pass.wrapping_add(1);
    if let Ok(font) = app.session.font() {
        for (name, layer_id) in &key.1 {
            let Some(layer) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == *layer_id)) else { continue };
            if drawn_as_color(layer) {
                continue;
            }
            let Some(image) = side_image(layer, LayerSide::Foreground) else { continue };
            let placed = load(&mut app.images, &assets, &app.host, app.window.as_ref(), image, side, &mut pass);
            by_glyph.insert(name.clone(), Arc::new(placed));
        }
    }
    crate::app::models::set(&mut app.models.space_images, CellImages { by_glyph });
    app.images.space_seen = if pass.waiting { None } else { Some(key) };
    if pass.deferred
        && let Some(ctx) = &app.window
    {
        ctx.request_repaint();
    }
}
