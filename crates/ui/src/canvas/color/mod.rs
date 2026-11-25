//! Colour glyphs on the canvas and in the previews: the edited glyph drawn in colour behind its outline, the edit
//! view's other glyphs in colour, Font View cells in colour, and the gradient knobs of the selected paths of a
//! full-colour layer.
//!
//! Rows: `color.preview.editViewOtherGlyphs`, `color.preview.paletteDrivesFontView`, `color.layered.masterColor`,
//! `color.layered.editLikeMasters`, `color.layered.stackingOrder`, `color.fullColor.gradientCanvasKnobs`,
//! `color.fullColor.imageOnlyEffects` (drawing), `color.sbix.sizeScalesImage` (drawing).
//!
//! When the canvas draws in colour ([`mode`]), from the layer the edited glyph shows (`shell::masters::shown_layer`):
//!
//! | That layer | The canvas draws |
//! |---|---|
//! | a colour layer | the edited glyph's colour drawing of that layer's kind (that very layer for iColor and svg layers); every other glyph of the text with colour layers in that master in colour |
//! | a master layer with a Master Color (a layered font) | every glyph filled in the Master Color of each shown master (that one and those whose layer eye is on in the edited glyph), the first master on top |
//! | any other layer | nothing in colour |
//!
//! What a glyph draws (`fontcraft_engine::color_drawing`, [`content`]):
//!
//! | Kind | Drawn |
//! |---|---|
//! | palette and full-colour layers | the paint graph through `fontcraft_render::color::artwork`: fills, gradients, strokes, and the shadows, inner shadows and pattern images of full-colour paths; references to other colour glyphs followed as the compiled font draws them |
//! | svg layer | the file's outlines in their colours and gradients |
//! | iColor layer | its image, placed by the layer's image transform |
//!
//! A glyph other than the edited one shows the kind of the shown layer when it has it, else vector layers, then svg,
//! then iColor (of several iColor layers, the one of the shown layer's size, else the largest). Palette colours come
//! from palette 0 of the master's palettes (the font's when the master has none); the text colour (`*`) is black on
//! the light canvas. A colour with no opacity paints nothing, which is how a removed fill shows.
//!
//! Content is found once per engine revision ([`project`], [`text_colors`], [`cell_color`]); files are read through
//! [`ColorFiles`]. The [`paint`] module rasterises vector content into one texture per glyph and zoom step, kept while
//! the content and the step stay, and draws iColor images as textured quads.

pub mod knobs;
pub mod paint;
pub mod tokens;

use std::collections::HashMap;
use std::sync::Arc;

use craft::ui::contract::PlainData;
pub use fontcraft_engine::color_drawing::{
    Bitmap, ColorArt, ColorFiles, Content, MASTER_COLOR_DARK, Mode, PlacedColor, Sources, TEXT, cell_color, cell_mode, content, has_color,
    master_color, mode, stamp,
};

use crate::app::ViewState;
use crate::app::models::ProjectCx;

/// Most glyphs of the edit view's text drawn in colour besides the edited one.
pub const MAX_TEXT_COLOR_GLYPHS: usize = 1000;

/// What the canvas draws in colour for the edited glyph.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct ColorPicture {
    #[plain_data(assume = "artwork in font units: owned paths, numbers and shared decoded pixels")]
    pub art: ColorArt,
    /// The text colour (`*`), straight RGBA from 0 to 1: the plain fill of a glyph past the work budget.
    pub text: [f32; 4],
}

/// The colour picture of the edited glyph, from the layer the Layers panel shows for it.
pub fn project(cx: &ProjectCx<'_>, files: &mut ColorFiles) -> Option<ColorPicture> {
    files.follow(cx.session.active().map(|d| d.id));
    let font = cx.session.font().ok()?;
    let glyph = crate::shell::masters::shown_glyph(font, cx.view)?;
    let layer = crate::shell::masters::shown_layer(font, &cx.view.layers, glyph)?;
    // The canvas is light: black text colour, Master Color rather than Master Color Dark.
    let mode = mode(font, glyph, layer, false)?;
    let assets = cx.session.assets();
    let mut src = Sources { assets: assets.as_ref(), files, text: TEXT, palette: 0 };
    let content = content(font, glyph, &mode, Some(layer), &mut src).filter(|c| !c.is_empty())?;
    Some(ColorPicture { art: ColorArt::new(content), text: TEXT })
}

/// A glyph of the edit view's text besides the edited one: its name, the own layer of the item that shows it, and its
/// origin in the edited glyph's font units.
pub type TextPlace<'a> = (&'a str, Option<&'a str>, [f64; 2]);

/// The glyphs of the edit view's text drawn in colour. The drawing of each name and layer is found once.
pub fn text_colors(session: &fontcraft_engine::Session, view: &ViewState, files: &mut ColorFiles, glyphs: &[TextPlace<'_>]) -> Vec<PlacedColor> {
    files.follow(session.active().map(|d| d.id));
    let Ok(font) = session.font() else { return Vec::new() };
    let Some(edited) = crate::shell::masters::shown_glyph(font, view) else { return Vec::new() };
    let Some(layer) = crate::shell::masters::shown_layer(font, &view.layers, edited) else { return Vec::new() };
    let Some(mode) = mode(font, edited, layer, false) else { return Vec::new() };
    let assets = session.assets();
    let mut src = Sources { assets: assets.as_ref(), files, text: TEXT, palette: 0 };
    let mut found: HashMap<(&str, Option<&str>), Option<Arc<ColorArt>>> = HashMap::new();
    let mut out = Vec::new();
    for &(name, own, at) in glyphs {
        if out.len() >= MAX_TEXT_COLOR_GLYPHS {
            break;
        }
        let art = found.entry((name, own)).or_insert_with(|| {
            let glyph = font.glyph(name).filter(|g| has_color(g, &mode))?;
            let layer = own.and_then(|id| glyph.layers.iter().find(|l| l.id == id));
            content(font, glyph, &mode, layer, &mut src).filter(|c| !c.is_empty()).map(|c| Arc::new(ColorArt::new(c)))
        });
        if let Some(art) = art {
            out.push(PlacedColor { at, art: Arc::clone(art) });
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
