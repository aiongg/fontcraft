//! Placed images in Font View cells (`fontView.cell.imageDisplay`).
//!
//! A cell draws the image placed on its glyph's layer of Font View's master while View › Show Image is on, and always
//! when that layer has no paths and no components, behind the glyph, as Edit View draws it (its transform, crop and
//! opacity), clipped to the cell. A missing or unreadable file draws the missing-image marker, a format kept as a
//! reference the marker with its name.
//!
//! [`refresh`] works the images out after the projections, only while Font View shows and when the document, the
//! master, Show Image or the decoded files moved. One pass decodes at most [`MAX_DECODES_PER_PASS`] files (large ones
//! on jobs, [`super::files`]); the others draw nothing yet and are taken on the next passes, so a font of many images
//! never stalls a frame. At most [`MAX_CELL_IMAGES`] glyphs show an image.

use std::collections::BTreeMap;
use std::sync::Arc;

use craft::ui::contract::PlainData;
use fontcraft_model::LayerSide;

use super::files::Loaded;
use super::{PlacedImage, drawn_as_color, shown, side_image};
use crate::app::FontCraftApp;

/// Most files one pass reads and decodes for the cells.
pub const MAX_DECODES_PER_PASS: usize = 4;
/// Most glyphs whose cells show an image.
pub const MAX_CELL_IMAGES: usize = 20_000;

/// The images Font View cells draw, by glyph name.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CellImages {
    #[plain_data(assume = "placed images in font units; decoded pixels behind an Arc, never changed once made")]
    pub by_glyph: BTreeMap<String, Arc<PlacedImage>>,
}

/// What the cell images were last worked out from: the document and its revision, the master, Show Image and the
/// file cache's generation.
pub type CellsKey = (Option<(u64, u64)>, Option<String>, bool, u64);

/// Works out the cell images when what they come from moved, while Font View shows.
pub fn refresh(app: &mut FontCraftApp) {
    if !app.font_view_shows() {
        return;
    }
    let master = app.models.font_view.master.clone();
    let show = app.view.display.image;
    let key: CellsKey = (app.session.active().map(|d| (d.id, d.revision)), master.clone(), show, app.images.files.generation());
    if app.images.cells_seen.as_ref() == Some(&key) {
        return;
    }
    let assets = app.session.assets();
    let mut by_glyph = BTreeMap::new();
    let (mut waiting, mut deferred) = (false, false);
    if let (Ok(font), Some(master)) = (app.session.font(), master.as_deref()) {
        let mut budget = MAX_DECODES_PER_PASS;
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
