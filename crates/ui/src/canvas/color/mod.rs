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
//! What a glyph draws ([`content`]):
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

pub mod files;
pub mod knobs;
pub mod paint;
pub mod tokens;

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use craft::ui::contract::PlainData;
use fontcraft_engine::cmd::stroke::{layer_outline, shapes_outline};
use fontcraft_engine::color::layers::{self as kinds, ColorLayerKind};
use fontcraft_engine::color::{AssetStore, export};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::{Font, Glyph, Layer, Master, Shape, ShapeAttrs};
use fontcraft_render::color::artwork::{self, Drawable, Item, Leaf};

pub use self::files::{Bitmap, ColorFiles};
use crate::app::ViewState;
use crate::app::models::ProjectCx;

/// The Master Color parameter's dark-appearance counterpart.
pub const MASTER_COLOR_DARK: &str = "Master Color Dark";
/// The text colour (`*`) on the light canvas, straight RGBA.
pub const TEXT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Most glyphs of the edit view's text drawn in colour besides the edited one.
pub const MAX_TEXT_COLOR_GLYPHS: usize = 1000;

/// What one glyph draws in colour, in its font units.
#[derive(Clone, Debug)]
pub enum Content {
    /// Vector artwork, bottom first.
    Items(Vec<Item>),
    /// An iColor image: `place` maps the image's pixels (y up from its bottom left) to font units.
    Bitmap { bitmap: Bitmap, place: Affine },
}

impl Content {
    fn is_empty(&self) -> bool {
        match self {
            Content::Items(items) => artwork::bounds(items).is_none(),
            Content::Bitmap { .. } => false,
        }
    }
}

/// One glyph's colour drawing, with a key that changes whenever the drawing does: the key of its textures.
#[derive(PlainData, Clone, Debug)]
pub struct ColorArt {
    #[plain_data(assume = "artwork in font units: owned paths, numbers and shared decoded pixels")]
    pub content: Content,
    pub key: u64,
}

impl PartialEq for ColorArt {
    // Comparing the key spares comparing every path and pixel on each projection.
    fn eq(&self, other: &ColorArt) -> bool {
        self.key == other.key
    }
}

impl ColorArt {
    pub fn new(content: Content) -> ColorArt {
        let mut h = DefaultHasher::new();
        match &content {
            Content::Items(items) => {
                0u8.hash(&mut h);
                for item in items {
                    match item {
                        // A pattern's pixels are shared and never change once decoded: its pointer and size stand for them.
                        Item::Pattern { outline, pattern } => {
                            format!("{outline:?}").hash(&mut h);
                            (Arc::as_ptr(&pattern.image) as usize, pattern.image.width, pattern.image.height, pattern.unit.to_bits()).hash(&mut h);
                        }
                        other => format!("{other:?}").hash(&mut h),
                    }
                }
            }
            Content::Bitmap { bitmap, place } => {
                1u8.hash(&mut h);
                (Arc::as_ptr(&bitmap.image) as usize, bitmap.image.width, bitmap.image.height).hash(&mut h);
                place.as_coeffs().map(f64::to_bits).hash(&mut h);
            }
        }
        ColorArt { key: h.finish(), content }
    }
}

/// What the canvas draws in colour for the edited glyph.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct ColorPicture {
    pub art: ColorArt,
    /// The text colour (`*`), straight RGBA from 0 to 1: the plain fill of a glyph past the work budget.
    pub text: [f32; 4],
}

/// A glyph of the edit view's text in colour: its drawing and where its origin lies, in the edited glyph's font units.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct PlacedColor {
    pub at: [f64; 2],
    #[plain_data(assume = "a colour drawing shared between the glyphs of one name")]
    pub art: Arc<ColorArt>,
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
    Some(c.to_f64().map(|v| v as f32))
}

/// The masters of a layered font drawn with `shown` among them, bottom first (the first master on top); `None` when
/// `active` has no Master Color.
fn layered(font: &Font, active: &str, shown: impl Fn(&str) -> bool, dark: bool) -> Option<Mode> {
    let masters: Vec<(String, [f32; 4])> =
        font.masters.iter().rev().filter(|m| shown(&m.id)).filter_map(|m| master_color(m, dark).map(|c| (m.id.clone(), c))).collect();
    masters.iter().any(|(id, _)| id == active).then_some(Mode::Layered { masters })
}

/// How the canvas draws `glyph` in colour while it shows `layer`; `None` for the ordinary drawing.
pub fn mode(font: &Font, glyph: &Glyph, layer: &Layer, dark: bool) -> Option<Mode> {
    if let Some(kind) = kinds::kind(layer) {
        return Some(Mode::Layers { master: layer.master.clone(), kind });
    }
    if !layer.is_master_layer() {
        return None;
    }
    layered(font, &layer.master, |id| id == layer.master || glyph.layer(id).is_some_and(|l| l.info.visible), dark)
}

/// How Font View cells draw in colour on `master`: a layered font's masters when it has a Master Color, else each
/// glyph's colour layers in that master, vector layers first.
pub fn cell_mode(font: &Font, master: &str, dark: bool) -> Mode {
    layered(font, master, |_| true, dark).unwrap_or_else(|| Mode::Layers { master: master.to_string(), kind: ColorLayerKind::FullColor })
}

/// Whether `glyph` has something to draw in `mode`.
pub fn has_color(glyph: &Glyph, mode: &Mode) -> bool {
    match mode {
        Mode::Layers { master, .. } => glyph.layers.iter().any(|l| l.master == *master && kinds::export_kind(l).is_some()),
        Mode::Layered { .. } => true,
    }
}

/// Where outlines, files and the text colour come from.
pub struct Sources<'a> {
    pub assets: &'a dyn AssetStore,
    pub files: &'a mut ColorFiles,
    pub text: [f32; 4],
}

fn concat(parts: Vec<BezPath>) -> BezPath {
    parts.into_iter().fold(BezPath::new(), |mut all, p| {
        all.extend(p.elements().iter().copied());
        all
    })
}

/// The artwork of `glyph`'s vector colour layers in `master`.
fn vector(font: &Font, glyph: &Glyph, master: &str, src: &mut Sources<'_>) -> Option<Content> {
    let outline = |f: &Font, shapes: &[Shape], m: &str| shapes_outline(f, shapes, m).ok().map(concat);
    let assets = src.assets;
    let files = &mut *src.files;
    let mut pattern = |attrs: &ShapeAttrs| files.pattern(assets, attrs);
    let mut art = artwork::Sources { outline: &outline, pattern: &mut pattern, text: src.text };
    artwork::collect(font, glyph, master, &mut art).map(Content::Items)
}

/// An svg layer's file as outlines in their paints.
fn svg(layer: &Layer, src: &mut Sources<'_>) -> Option<Content> {
    let image = layer.info.image.as_ref()?;
    let path = image.path.as_deref()?;
    let shapes = src.files.svg(src.assets, path, image.transform.affine());
    let items = shapes
        .iter()
        .take(artwork::MAX_DRAWABLES)
        .filter_map(|s| artwork::leaf(&s.paint, &[], src.text).map(|leaf| Item::Fill(Drawable { clips: vec![s.outline.clone()], leaf })))
        .collect();
    Some(Content::Items(items))
}

/// An iColor layer's image, drawn whole (not at a tracing image's opacity).
fn bitmap(layer: &Layer, src: &mut Sources<'_>) -> Option<Content> {
    let image = layer.info.image.as_ref()?;
    let path = image.path.as_deref()?;
    let bitmap = src.files.bitmap(src.assets, path)?;
    let place = image.transform.affine() * Affine::scale_non_uniform(bitmap.step.0, bitmap.step.1);
    Some(Content::Bitmap { bitmap, place })
}

/// The colour layers of `glyph` in `master` that are not intermediate layers, in file order.
fn static_layers<'g>(glyph: &'g Glyph, master: &str) -> Vec<(&'g Layer, ColorLayerKind)> {
    kinds::color_layers(glyph, master).into_iter().filter(|(l, _)| l.info.attrs.coordinates.is_empty()).collect()
}

/// What `glyph` draws in `mode`. `exact` is the edited glyph's shown layer, drawn itself when it is a colour layer.
pub fn content(font: &Font, glyph: &Glyph, mode: &Mode, exact: Option<&Layer>, src: &mut Sources<'_>) -> Option<Content> {
    let (master, kind) = match mode {
        Mode::Layered { masters } => {
            let items = masters
                .iter()
                .filter_map(|(id, c)| {
                    let layer = glyph.layer(id)?;
                    let outline = layer_outline(font, layer).ok().map(concat)?;
                    (!outline.elements().is_empty()).then(|| Item::Fill(Drawable { clips: vec![outline], leaf: Leaf::Solid(*c) }))
                })
                .collect();
            return Some(Content::Items(items));
        }
        Mode::Layers { master, kind } => (master.as_str(), *kind),
    };
    if let Some(layer) = exact {
        return match kinds::kind(layer)? {
            ColorLayerKind::IColor(_) => bitmap(layer, src),
            ColorLayerKind::Svg => svg(layer, src),
            ColorLayerKind::Palette(_) | ColorLayerKind::FullColor => vector(font, glyph, master, src),
        };
    }
    let svg_layer =
        |src: &mut Sources<'_>| static_layers(glyph, master).into_iter().filter(|(_, k)| *k == ColorLayerKind::Svg).find_map(|(l, _)| svg(l, src));
    let icolor = |src: &mut Sources<'_>| {
        let sized: Vec<(&Layer, u16)> = static_layers(glyph, master)
            .into_iter()
            .filter(|(l, _)| l.info.image.as_ref().is_some_and(|i| i.path.is_some()))
            .filter_map(|(l, k)| if let ColorLayerKind::IColor(size) = k { Some((l, size)) } else { None })
            .collect();
        let same = match kind {
            ColorLayerKind::IColor(n) => sized.iter().find(|(_, size)| *size == n),
            _ => None,
        };
        same.or_else(|| sized.iter().max_by_key(|(_, size)| *size)).and_then(|(l, _)| bitmap(l, src))
    };
    match kind {
        ColorLayerKind::IColor(_) => icolor(src).or_else(|| vector(font, glyph, master, src)).or_else(|| svg_layer(src)),
        ColorLayerKind::Svg => svg_layer(src).or_else(|| vector(font, glyph, master, src)).or_else(|| icolor(src)),
        ColorLayerKind::Palette(_) | ColorLayerKind::FullColor => vector(font, glyph, master, src).or_else(|| svg_layer(src)).or_else(|| icolor(src)),
    }
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
    let mut src = Sources { assets: assets.as_ref(), files, text: TEXT };
    let content = content(font, glyph, &mode, Some(layer), &mut src).filter(|c| !c.is_empty())?;
    Some(ColorPicture { art: ColorArt::new(content), text: TEXT })
}

/// The glyphs of the edit view's text drawn in colour, from `(name, origin)` of each glyph besides the edited one, in
/// the edited glyph's font units. Each name's drawing is found once.
pub fn text_colors(session: &fontcraft_engine::Session, view: &ViewState, files: &mut ColorFiles, glyphs: &[(&str, [f64; 2])]) -> Vec<PlacedColor> {
    files.follow(session.active().map(|d| d.id));
    let Ok(font) = session.font() else { return Vec::new() };
    let Some(edited) = crate::shell::masters::shown_glyph(font, view) else { return Vec::new() };
    let Some(layer) = crate::shell::masters::shown_layer(font, &view.layers, edited) else { return Vec::new() };
    let Some(mode) = mode(font, edited, layer, false) else { return Vec::new() };
    let assets = session.assets();
    let mut src = Sources { assets: assets.as_ref(), files, text: TEXT };
    let mut found: HashMap<&str, Option<Arc<ColorArt>>> = HashMap::new();
    let mut out = Vec::new();
    for &(name, at) in glyphs {
        if out.len() >= MAX_TEXT_COLOR_GLYPHS {
            break;
        }
        let art = found.entry(name).or_insert_with(|| {
            let glyph = font.glyph(name).filter(|g| has_color(g, &mode))?;
            content(font, glyph, &mode, None, &mut src).filter(|c| !c.is_empty()).map(|c| Arc::new(ColorArt::new(c)))
        });
        if let Some(art) = art {
            out.push(PlacedColor { at, art: Arc::clone(art) });
        }
    }
    out
}

/// A Font View cell's colour drawing of `glyph` in `mode` (see [`cell_mode`]); `None` when it has none.
pub fn cell_color(font: &Font, glyph: &Glyph, mode: &Mode, src: &mut Sources<'_>) -> Option<Arc<ColorArt>> {
    if !has_color(glyph, mode) {
        return None;
    }
    content(font, glyph, mode, None, src).filter(|c| !c.is_empty()).map(|c| Arc::new(ColorArt::new(c)))
}

/// Changes whenever what colour drawings read besides the glyphs does: the palettes and the Master Colors.
pub fn stamp(font: &Font, master: &str) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{:?}", fontcraft_engine::color::palette::read(font, Some(master)).ok()).hash(&mut h);
    for m in font.masters.iter() {
        format!("{:?}{:?}", m.params.get(export::MASTER_COLOR), m.params.get(MASTER_COLOR_DARK)).hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
