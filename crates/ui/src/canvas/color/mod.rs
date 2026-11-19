//! Colour glyphs on the canvas: the edited glyph drawn in colour behind its outline, and the gradient knobs of the
//! selected paths of a full-colour layer.
//!
//! Rows: `color.preview.editViewOtherGlyphs` (the edited glyph), `color.layered.masterColor`,
//! `color.layered.editLikeMasters`, `color.layered.stackingOrder`, `color.fullColor.gradientCanvasKnobs`.
//!
//! When the canvas draws in colour ([`mode`]), from the layer the edited glyph shows (`shell::masters::shown_layer`):
//!
//! | That layer | The canvas draws |
//! |---|---|
//! | a palette or full-colour layer | the glyph's colour layers in that layer's master, through the paint graph |
//! | a master layer with a Master Color (a layered font) | the glyph filled in the Master Color of each shown master (that one and those whose layer eye is on), the first master on top |
//! | any other layer | nothing in colour |
//!
//! Palette colours come from palette 0 of the master's palettes (the font's when the master has none); the text
//! colour (`*`) is black on the light canvas. A colour with no opacity paints nothing, which is how a removed fill
//! shows. Components of colour glyphs on a full-colour layer are drawn as that glyph's colour layers, moved by the
//! component's transform, as the compiled font draws them ([`content`]).
//!
//! The fills are found once per engine revision ([`project`]). The [`paint`] layer rasterises them into one texture
//! per zoom step, kept while the fills and the step stay.

pub mod content;
pub mod knobs;
pub mod paint;
pub mod raster;
pub mod tokens;

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::ui::contract::PlainData;
use fontcraft_engine::cmd::stroke::layer_outline;
use fontcraft_engine::color::export;
use fontcraft_engine::color::layers::{self as kinds, ColorLayerKind};
use fontcraft_model::{Font, Glyph, Layer, Master};

use self::content::{Drawable, Leaf, concat};
use crate::app::models::ProjectCx;

/// The Master Color parameter's dark-appearance counterpart.
pub const MASTER_COLOR_DARK: &str = "Master Color Dark";

/// What the canvas draws in colour: fills in the edited glyph's font units, bottom first.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ColorPicture {
    #[plain_data(assume = "fills in font units: owned paths and numbers")]
    pub fills: Vec<Drawable>,
    /// Changes whenever the fills do: the key of their texture.
    pub key: u64,
    /// The text colour (`*`), straight RGBA from 0 to 1: the plain fill of a glyph past the work budget.
    pub text: [f32; 4],
}

/// How the canvas draws in colour.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// A colour layer of `master` is the shown layer; `kind` is its kind.
    Layers { master: String, kind: ColorLayerKind },
    /// A layered font: these masters' layers, bottom first, each filled in its colour (straight RGBA from 0 to 1).
    Layered { masters: Vec<(String, [f32; 4])> },
}

/// The colour a master is drawn in: its Master Color Dark on a dark canvas when it has a readable one, else its Master
/// Color.
pub fn master_color(master: &Master, dark: bool) -> Option<[f32; 4]> {
    let dark_value = dark.then(|| master.params.get(MASTER_COLOR_DARK).and_then(export::master_color)).flatten();
    let c = dark_value.or_else(|| master.params.get(export::MASTER_COLOR).and_then(export::master_color))?;
    Some(content::straight(c))
}

/// How the canvas draws `glyph` in colour while it shows `layer`; `None` for the ordinary drawing.
pub fn mode(font: &Font, glyph: &Glyph, layer: &Layer, dark: bool) -> Option<Mode> {
    if let Some(kind) = kinds::kind(layer) {
        return Some(Mode::Layers { master: layer.master.clone(), kind });
    }
    if !layer.is_master_layer() {
        return None;
    }
    let shown = |id: &str| id == layer.master || glyph.layer(id).is_some_and(|l| l.info.visible);
    // The first master is drawn on top, so it comes last.
    let masters: Vec<(String, [f32; 4])> =
        font.masters.iter().rev().filter(|m| shown(&m.id)).filter_map(|m| master_color(m, dark).map(|c| (m.id.clone(), c))).collect();
    masters.iter().any(|(id, _)| *id == layer.master).then_some(Mode::Layered { masters })
}

/// The fills `glyph` draws in `mode`. iColor and svg layers draw nothing yet.
pub fn fills(font: &Font, glyph: &Glyph, mode: &Mode, text: [f32; 4]) -> Option<Vec<Drawable>> {
    match mode {
        Mode::Layered { masters } => Some(
            masters
                .iter()
                .filter_map(|(id, c)| {
                    let layer = glyph.layer(id)?;
                    let outline = layer_outline(font, layer).ok().map(concat)?;
                    (!outline.elements().is_empty()).then(|| Drawable { clips: vec![outline], leaf: Leaf::Solid(*c) })
                })
                .collect(),
        ),
        Mode::Layers { master, kind: ColorLayerKind::Palette(_) | ColorLayerKind::FullColor } => content::vector(font, glyph, master, text),
        Mode::Layers { .. } => None,
    }
}

/// The colour picture of the edited glyph, from the layer the Layers panel shows for it.
pub fn project(cx: &ProjectCx<'_>) -> Option<ColorPicture> {
    let font = cx.session.font().ok()?;
    let glyph = crate::shell::masters::shown_glyph(font, cx.view)?;
    let layer = crate::shell::masters::shown_layer(font, &cx.view.layers, glyph)?;
    // The canvas is light: black text colour, Master Color rather than Master Color Dark.
    let text = [0.0, 0.0, 0.0, 1.0];
    let mode = mode(font, glyph, layer, false)?;
    let fills = fills(font, glyph, &mode, text)?;
    if fills.is_empty() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    cx.session.active().map(|d| (d.id, d.revision)).hash(&mut hasher);
    glyph.name.hash(&mut hasher);
    layer.id.hash(&mut hasher);
    format!("{mode:?}").hash(&mut hasher);
    Some(ColorPicture { fills, key: hasher.finish(), text })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
