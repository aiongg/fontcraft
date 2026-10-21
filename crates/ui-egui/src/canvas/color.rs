//! Colour glyphs in Edit View and in the previews: palette and full-colour layers through the
//! paint graph (solid fills, gradients, strokes), iColor images at their size, svg layers as
//! their file's outlines, and the masters of a layered font in their Master Color.
//!
//! Rows: `color.preview.editViewOtherGlyphs`, `color.preview.previewArea`,
//! `color.fullColor.transparentSwatch` (drawing), `color.sbix.sizeScalesImage` (drawing),
//! `color.layered.masterColor`, `color.layered.masterColorDark`, `color.layered.editLikeMasters`,
//! `color.layered.stackingOrder`, and the `Master Color` and `Master Color Dark` parameters.
//!
//! When Edit View draws in colour ([`mode`]):
//!
//! | The edited glyph's active layer | The tab's glyphs are drawn |
//! |---|---|
//! | a colour layer | each glyph with colour layers in that layer's master in colour, the edited glyph included, behind the edited outline; the others as usual |
//! | a master layer with a Master Color (a layered font) | every glyph filled in the Master Color of each shown master (the active one and those whose layer eye is on in the edited glyph), the first master on top; Master Color Dark on a dark canvas when the master has one |
//! | any other layer | as usual |
//!
//! A glyph drawn in colour takes the place of its ordinary fill ([`replaces_fill`]). Which kind
//! of colour layer a glyph shows: the edited glyph shows its active layer's kind (that very
//! layer for iColor and svg layers); every other glyph the kind of the active layer when it has
//! it, else vector layers, then svg, then iColor. Palette colours come from palette 0 of the
//! master's palettes (the font's when the master has none); the text colour (`*`) is black on a
//! light canvas and white on a dark one, and a preview's text colour in the previews. A colour with no opacity paints nothing, which is how a
//! removed fill shows.
//!
//! A component of a colour glyph on a full-colour layer (`color.export.colrGlyphComponent`) is
//! drawn as that glyph's colour layers, moved by the component's transform, as the compiled font
//! draws it; references inside them are followed too. Where the compile falls back to the
//! component's outline (a cycle, references nested more than the paint depth limit, a glyph
//! that does not export), the outline is filled in the default grey.
//!
//! The previews ([`preview_layer`]) draw a glyph in colour while one of its colour layers is the
//! layer the tab shows for it; the colour is drawn over the strip before Flip and Blur, and Invert
//! leaves it as it is.
//!
//! Drawing cost (`docs/plans/M11.md` § 5, an Edit View frame of 50 colour glyphs in 16 ms):
//! vector content is rasterised on the CPU into one texture per glyph, kept while the glyph,
//! the palettes, the mode and the zoom are unchanged, so scrolling and edits to other glyphs draw
//! textures only. A glyph's texture is clipped to the canvas with a margin when it is larger, and
//! a glyph wholly outside that is not rasterised. iColor images use the placed-image cache
//! ([`super::images`]); svg files are read once per file revision and placement ([`SvgCache`]).
//! An svg file with SVG filters is drawn as Text Preview draws an `SVG ` glyph
//! (`fontcraft_render::color::SvgFile`, M11.17e): with its filters, and with its clip paths,
//! masks, patterns and embedded images, clipped to the canvas like the other textures. When that
//! drawing is refused (a texture past `MAX_GLYPH_PIXELS`, or past its work budget) the file is
//! drawn from its outlines, and is not tried again while only the view moves; a group whose
//! filters would paint more than `MAX_FILTER_PIXELS` pixels of the texture is drawn unfiltered.
//! Either way a line at the canvas's bottom left says why. The previews draw the file the same
//! way, whole, from rasters [`SvgCache`] keeps, when their placement is an upright, uniform
//! scale, else from its outlines. One glyph's drawing has a work budget ([`MAX_PAINT_WORK`]); a glyph
//! that needs more is drawn as one plain fill of its outlines in the text colour. The fills of a
//! referenced glyph count towards the budget of the glyph that refers to it, and following
//! references visits at most [`MAX_PAINT_VISITS`] paint nodes per glyph.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::stroke::{layer_outline, shapes_outline};
use fontcraft_engine::color::layers::{self as kinds, ColorLayerKind};
use fontcraft_engine::color::paint::{self, ColorLine, ColorRef, Paint, ShapeRef};
use fontcraft_engine::color::svg_draw::{self, SvgShape};
use fontcraft_engine::color::{AssetStore, MAX_PAINT_DEPTH, Rgba, export, palette};
use fontcraft_engine::imagery;
use fontcraft_geom::{Affine, BezPath, Point, Shape as _};
use fontcraft_model::{Extend, Font, Glyph, Image, Layer, Master, Shape};
use fontcraft_render::color::{FileRaster, GlyphRaster, SvgFile};
use fontcraft_render::{Style, render_path};
use fontcraft_textlayout::Layout;

use super::Painting;
use super::images::draw_layer_image;
use crate::state::Tab;

/// Most glyph textures kept.
pub const MAX_SPRITES: usize = 2_048;
/// Most bytes of glyph textures kept.
pub const MAX_SPRITE_BYTES: usize = 256 << 20;
/// Pixels a clipped glyph texture reaches beyond the canvas on each side, so a short scroll
/// draws it again without rasterising.
pub const CLIP_MARGIN: f64 = 256.0;
/// Most filled outlines one glyph is drawn with.
pub const MAX_DRAWABLES: usize = 8_192;
/// Pixel operations (a fill's pixel box times its clips and its colour, summed over the fills)
/// one glyph may take to draw: at least this, or [`PAINT_WORK_PER_PIXEL`] per pixel of the
/// glyph's own box when that is more, up to [`MAX_GLYPH_PAINT_WORK`]. A glyph that needs more
/// is drawn as one plain fill of its outlines, so a file of many large layers cannot stall a
/// frame and no glyph is shown with layers missing.
pub const MAX_PAINT_WORK: u64 = 64 << 20;
/// Pixel operations one pixel of a glyph's box allows, for glyphs drawn large: room for a few
/// hundred layers that each cover the whole glyph.
pub const PAINT_WORK_PER_PIXEL: u64 = 256;
/// Most pixel operations one glyph may take to draw, however large it is drawn.
pub const MAX_GLYPH_PAINT_WORK: u64 = 1 << 30;
/// Most svg files whose outlines are kept.
pub const MAX_SVG_FILES: usize = 256;
/// Most bytes of svg files with filters kept read, their decoded images included.
pub const MAX_SVG_BYTES: usize = 64 << 20;
/// Most rasters of svg files with filters the previews keep.
pub const MAX_SVG_RASTERS: usize = 1_024;
/// Most bytes of rasters of svg files with filters the previews keep.
pub const MAX_SVG_RASTER_BYTES: usize = 64 << 20;
/// Steps per pixel at which the previews place a raster of an svg file with filters.
const RASTER_STEPS: f64 = 4.0;
/// The Master Color parameter's dark-appearance counterpart.
pub const MASTER_COLOR_DARK: &str = "Master Color Dark";

/// The look of colour drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorTokens {
    /// The text colour (`*`) of palette layers.
    pub text_color: Color32,
    /// Gradient knobs: the dot, its ring, its radius in screen points.
    pub knob: Color32,
    pub knob_outline: Color32,
    pub knob_radius: f32,
    pub knob_outline_width: f32,
    /// The line from a linear gradient's start knob to its end knob.
    pub knob_line: Color32,
    pub knob_line_width: f32,
    /// The line at the canvas's bottom left saying why an svg file is drawn without its filters.
    pub notice: Color32,
    pub notice_size: f32,
}

impl ColorTokens {
    pub fn dark() -> Self {
        Self {
            text_color: Color32::WHITE,
            knob: Color32::WHITE,
            knob_outline: Color32::from_rgb(0x20, 0x20, 0x20),
            knob_radius: 5.0,
            knob_outline_width: 1.5,
            knob_line: Color32::from_rgba_unmultiplied(0x80, 0x80, 0x80, 0xc0),
            knob_line_width: 1.0,
            notice: Color32::from_rgb(0xE8, 0xB0, 0x50),
            notice_size: 11.0,
        }
    }

    pub fn light() -> Self {
        Self { text_color: Color32::BLACK, notice: Color32::from_rgb(0x9A, 0x5A, 0x00), ..Self::dark() }
    }
}

// ── when and what ───────────────────────────────────────────────────────────────────────────────

/// How Edit View draws in colour.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// A colour layer of `master` is the active layer; `kind` is its kind.
    Layers { master: String, kind: ColorLayerKind },
    /// A layered font: these masters' layers, bottom first, each filled in its colour (straight
    /// RGBA from 0 to 1).
    Layered { masters: Vec<(String, [f32; 4])> },
}

/// The colour a master is drawn in: its Master Color Dark on a dark canvas when it has a readable
/// one, else its Master Color.
pub fn master_color(master: &Master, dark: bool) -> Option<[f32; 4]> {
    let dark_value = dark.then(|| master.params.get(MASTER_COLOR_DARK).and_then(export::master_color)).flatten();
    let c = dark_value.or_else(|| master.params.get(export::MASTER_COLOR).and_then(export::master_color))?;
    Some(straight(c))
}

fn straight(c: Rgba) -> [f32; 4] {
    c.to_f64().map(|v| v as f32)
}

/// How the tab draws in colour, from its edited glyph's active layer; `None` for the ordinary
/// drawing.
pub fn mode(font: &Font, tab: &Tab, dark: bool) -> Option<Mode> {
    let layer = super::view::active_layer(font, tab)?;
    if let Some(kind) = kinds::kind(layer) {
        return Some(Mode::Layers { master: layer.master.clone(), kind });
    }
    if !layer.is_master_layer() {
        return None;
    }
    let glyph = font.glyph(tab.edited_glyph()?)?;
    let shown = |id: &str| id == layer.master || glyph.layer(id).is_some_and(|l| l.info.visible);
    // The first master is drawn on top, so it comes last.
    let masters: Vec<(String, [f32; 4])> =
        font.masters.iter().rev().filter(|m| shown(&m.id)).filter_map(|m| master_color(m, dark).map(|c| (m.id.clone(), c))).collect();
    masters.iter().any(|(id, _)| *id == layer.master).then_some(Mode::Layered { masters })
}

/// Whether `glyph` has something to draw in `mode`.
fn has_color(glyph: &Glyph, mode: &Mode) -> bool {
    match mode {
        Mode::Layers { master, .. } => glyph.layers.iter().any(|l| l.master == *master && kinds::export_kind(l).is_some()),
        Mode::Layered { .. } => true,
    }
}

/// Whether Edit View draws glyph `name` of `tab` in colour, in place of its ordinary fill or
/// outline.
pub fn replaces_fill(font: &Font, tab: &Tab, name: &str) -> bool {
    mode(font, tab, false).is_some_and(|m| font.glyph(name).is_some_and(|g| has_color(g, &m)))
}

/// The layer a preview draws glyph `name` of `tab` in colour with: the layer the tab shows for
/// it, while that is a colour layer (`color.preview.previewArea`).
pub fn preview_layer<'f>(font: &'f Font, tab: &Tab, name: &str) -> Option<&'f Layer> {
    super::view::shown_layer(font, tab, name).filter(|l| kinds::kind(l).is_some())
}

/// Whether Edit View draws the image of `layer` as its colour content rather than as a placed
/// image: iColor and svg layers.
pub fn draws_layer_image(layer: &Layer) -> bool {
    matches!(kinds::kind(layer), Some(ColorLayerKind::IColor(_) | ColorLayerKind::Svg))
}

// ── content ─────────────────────────────────────────────────────────────────────────────────────

/// A colour stop line with colours resolved: offsets in order, straight RGBA.
#[derive(Clone, Debug, PartialEq)]
struct Stops {
    at: Vec<(f64, [f32; 4])>,
    extend: Extend,
}

/// What fills an outline, in font units of the glyph.
#[derive(Clone, Debug, PartialEq)]
enum Leaf {
    Solid([f32; 4]),
    /// `t = (p − p0) · v`: `v` is the gradient vector divided by its squared length.
    Linear {
        p0: Point,
        v: (f64, f64),
        stops: Stops,
    },
    Radial {
        c0: Point,
        r0: f64,
        c1: Point,
        r1: f64,
        stops: Stops,
    },
    Sweep {
        center: Point,
        start: f64,
        end: f64,
        stops: Stops,
    },
    /// `leaf` of a referenced colour glyph, read in that glyph's font units: `to_leaf` maps the
    /// drawn glyph's font units to them. Never nested: references compose their transforms.
    Moved {
        to_leaf: Affine,
        leaf: Box<Leaf>,
    },
}

/// One fill: `leaf` within every outline of `clips`.
#[derive(Clone, Debug, PartialEq)]
struct Drawable {
    clips: Vec<BezPath>,
    leaf: Leaf,
}

/// What a glyph draws in colour.
#[derive(Clone, Debug)]
enum Content {
    Vector(Vec<Drawable>),
    /// An iColor layer's image, drawn whole.
    Bitmap(Image),
    /// An svg file with filters, read as [`SvgCache`] keeps it under `key`; `plain` is its
    /// outlines, drawn when the file itself is not.
    Filtered {
        file: Arc<SvgFile>,
        key: SvgKey,
        plain: Vec<Drawable>,
    },
}

/// Where colours and files come from.
pub struct Sources<'a> {
    pub assets: &'a dyn AssetStore,
    pub svgs: &'a mut SvgCache,
    /// The app's file revision (`super::images::ImageUi::revision`).
    pub revision: u64,
    /// The text colour.
    pub text: Color32,
}

/// An svg file as Edit View keeps it: its outlines, and the file read for drawing when it has
/// filters.
type SvgEntry = (Arc<Vec<SvgShape>>, Option<Arc<SvgFile>>);

/// An svg file's stored path, file revision and placement.
type SvgKey = (Arc<str>, u64, [u64; 6]);

/// A raster of an svg file with filters: the file, its pixels per font unit and its shift in
/// steps of [`RASTER_STEPS`].
type RasterKey = (SvgKey, u64, [u8; 2]);

/// The svg files read, by [`SvgKey`], and the rasters the previews draw of those with filters.
#[derive(Default)]
pub struct SvgCache {
    entries: HashMap<SvgKey, SvgEntry>,
    /// Bytes the entries' files with filters keep.
    bytes: usize,
    /// `None` where the file was not drawn.
    rasters: HashMap<RasterKey, Option<Arc<GlyphRaster>>>,
    raster_bytes: usize,
}

impl SvgCache {
    /// The outlines of the svg file at `path` placed by `transform`, empty when it cannot be
    /// read, and the file when it has filters that can be drawn.
    fn get(&mut self, assets: &dyn AssetStore, path: &str, revision: u64, transform: Affine) -> (SvgEntry, SvgKey) {
        let key: SvgKey = (Arc::from(path), revision, transform.as_coeffs().map(f64::to_bits));
        if let Some(found) = self.entries.get(&key) {
            return (found.clone(), key);
        }
        let bytes = assets.read(path).ok();
        let shapes = bytes.as_ref().and_then(|b| svg_draw::shapes(b, transform).ok()).unwrap_or_default();
        // Only a file that names a filter is read a second time, for its groups.
        let filtered = bytes
            .filter(|b| b.windows(6).any(|w| w == b"filter"))
            .and_then(|b| SvgFile::read(&b, transform).ok())
            .filter(SvgFile::has_filters)
            .map(Arc::new);
        let size = filtered.as_ref().map_or(0, |f| f.bytes());
        if self.entries.len() >= MAX_SVG_FILES || self.bytes.saturating_add(size) > MAX_SVG_BYTES {
            self.entries.clear();
            self.bytes = 0;
        }
        // A file larger than the whole limit is drawn from its outlines.
        let filtered = filtered.filter(|_| size <= MAX_SVG_BYTES);
        self.bytes += filtered.as_ref().map_or(0, |_| size);
        let entry = (Arc::new(shapes), filtered);
        self.entries.insert(key.clone(), entry.clone());
        (entry, key)
    }

    /// The raster of `file`, kept under `key`, at `scale` pixels per font unit, shifted by
    /// `steps` ÷ [`RASTER_STEPS`] pixels right and down; `None` when it is not drawn.
    fn raster(&mut self, key: &SvgKey, file: &SvgFile, scale: f64, steps: [u8; 2]) -> Option<Arc<GlyphRaster>> {
        let k = (key.clone(), scale.to_bits(), steps);
        if let Some(found) = self.rasters.get(&k) {
            return found.clone();
        }
        let shift = (f64::from(steps[0]) / RASTER_STEPS, f64::from(steps[1]) / RASTER_STEPS);
        let raster = file.draw(scale, shift, None).ok().and_then(|f| f.raster).map(Arc::new);
        let size = raster.as_ref().map_or(0, |r| r.image.pixels.len());
        if self.rasters.len() >= MAX_SVG_RASTERS || self.raster_bytes.saturating_add(size) > MAX_SVG_RASTER_BYTES {
            self.rasters.clear();
            self.raster_bytes = 0;
        }
        if size <= MAX_SVG_RASTER_BYTES {
            self.raster_bytes += size;
            self.rasters.insert(k, raster.clone());
        }
        raster
    }

    pub fn clear(&mut self) {
        *self = SvgCache::default();
    }
}

fn clamp01(v: f64) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
}

fn color32_straight(c: Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    [r, g, b, a].map(|v| f32::from(v) / 255.0)
}

/// `c` resolved against palette `palette`; a row the palette lacks is drawn in the text colour.
fn resolve(c: ColorRef, palette: &[Rgba], text: [f32; 4]) -> [f32; 4] {
    let with = |[r, g, b, a]: [f32; 4], alpha: f64| [r, g, b, a * clamp01(alpha)];
    match c {
        ColorRef::Palette { index, alpha } => with(palette.get(usize::from(index)).map_or(text, |row| straight(*row)), alpha),
        ColorRef::Foreground { alpha } => with(text, alpha),
        ColorRef::Rgba(v) => v.map(clamp01),
    }
}

fn stops(line: &ColorLine, palette: &[Rgba], text: [f32; 4]) -> Option<Stops> {
    let mut at: Vec<(f64, [f32; 4])> =
        line.stops.iter().filter(|s| s.offset.is_finite()).map(|s| (s.offset, resolve(s.color, palette, text))).collect();
    at.sort_by(|a, b| a.0.total_cmp(&b.0));
    (!at.is_empty()).then_some(Stops { at, extend: line.extend })
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The fill a leaf paint gives; `None` for a node that is not a leaf or cannot be drawn.
fn leaf(paint: &Paint, palette: &[Rgba], text: [f32; 4]) -> Option<Leaf> {
    match paint {
        Paint::Solid(c) => Some(Leaf::Solid(resolve(*c, palette, text))),
        Paint::Linear { p0, p1, p2, line } => {
            let stops = stops(line, palette, text)?;
            if !(finite(*p0) && finite(*p1) && finite(*p2)) {
                return None;
            }
            // Colour is constant along lines parallel to p0–p2: p1 projected onto the normal of
            // p0–p2 through p0 gives the gradient vector, as COLR version 1 defines it.
            let normal = (-(p2.y - p0.y), p2.x - p0.x);
            let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
            let nn = normal.0 * normal.0 + normal.1 * normal.1;
            let (vx, vy) = if nn > 1e-12 {
                let k = (dx * normal.0 + dy * normal.1) / nn;
                (normal.0 * k, normal.1 * k)
            } else {
                (dx, dy)
            };
            let len2 = vx * vx + vy * vy;
            if len2 <= 1e-12 {
                return stops.at.last().map(|(_, c)| Leaf::Solid(*c));
            }
            Some(Leaf::Linear { p0: *p0, v: (vx / len2, vy / len2), stops })
        }
        Paint::Radial { c0, r0, c1, r1, line } => {
            let ok = finite(*c0) && finite(*c1) && r0.is_finite() && r1.is_finite();
            ok.then(|| stops(line, palette, text)).flatten().map(|stops| Leaf::Radial { c0: *c0, r0: r0.abs(), c1: *c1, r1: r1.abs(), stops })
        }
        Paint::Sweep { center, start, end, line } => {
            let ok = finite(*center) && start.is_finite() && end.is_finite();
            ok.then(|| stops(line, palette, text)).flatten().map(|stops| Leaf::Sweep { center: *center, start: *start, end: *end, stops })
        }
        Paint::Layers(_) | Paint::Glyph { .. } | Paint::ColrGlyph { .. } => None,
    }
}

fn concat(parts: Vec<BezPath>) -> BezPath {
    parts.into_iter().fold(BezPath::new(), |mut all, p| {
        all.extend(p.elements().iter().copied());
        all
    })
}

/// The outline of the shapes `shape` names, as the COLR writer draws its layer glyph: on a
/// full-colour layer the fill without strokes, or the stroke alone, expanded.
fn shape_outline(font: &Font, glyph: &Glyph, shape: &ShapeRef) -> Option<BezPath> {
    let layer = glyph.layers.iter().find(|l| l.id == shape.layer)?;
    let mut shapes: Vec<Shape> = shape.shapes.iter().filter_map(|&i| layer.shapes.get(i).cloned()).collect();
    if layer.info.attrs.full_color {
        if shape.stroke.is_some() {
            shapes.retain(|s| matches!(s, Shape::Path(_)));
        }
        for s in &mut shapes {
            if let Shape::Path(p) = s {
                if shape.stroke.is_some() {
                    p.info.attrs.fill = false;
                } else {
                    p.info.attrs.stroke_width = None;
                }
            }
        }
    } else if shape.stroke.is_some() {
        return None;
    }
    shapes_outline(font, &shapes, &layer.master).ok().map(concat)
}

/// Most paint nodes one glyph's drawing visits, references followed included, so that
/// references fanning out to other references cannot multiply the work without bound.
pub const MAX_PAINT_VISITS: usize = 65_536;

/// Turns a glyph's paint graph into fills, following references to other colour glyphs
/// ([`Paint::ColrGlyph`]) as the compiled font draws them.
struct Collect<'f> {
    font: &'f Font,
    paints: paint::MasterPaints<'f>,
    /// The paint graphs of the glyphs referenced so far with their [`paint::nesting`], walked
    /// once per glyph however often it is referenced; `None` for a glyph without one.
    graphs: HashMap<String, Option<(Arc<Paint>, usize)>>,
    palette: &'f [Rgba],
    text: [f32; 4],
    out: Vec<Drawable>,
    /// The glyphs being drawn: the drawn glyph, then each glyph a reference is following.
    glyphs: Vec<String>,
    visits: usize,
    /// How often [`paint::nesting`] walked a graph.
    #[cfg(test)]
    nesting_walks: usize,
}

impl Collect<'_> {
    /// The fills of `paint` of `glyph`, bottom first, each within the outlines of the glyph
    /// nodes above it. `place` maps `glyph`'s font units to the drawn glyph's.
    fn collect(&mut self, glyph: &Glyph, paint: &Paint, clips: &mut Vec<BezPath>, place: Affine, depth: usize) {
        if depth > MAX_PAINT_DEPTH || self.out.len() >= MAX_DRAWABLES || self.visits >= MAX_PAINT_VISITS {
            return;
        }
        self.visits += 1;
        match paint {
            Paint::Layers(list) => {
                for p in list {
                    self.collect(glyph, p, clips, place, depth + 1);
                }
            }
            Paint::Glyph { shape, paint } => {
                let Some(outline) = shape_outline(self.font, glyph, shape) else { return };
                clips.push(placed(outline, place));
                match leaf(paint, self.palette, self.text) {
                    Some(l) => self.push(clips, l, place),
                    None => self.collect(glyph, paint, clips, place, depth + 1),
                }
                clips.pop();
            }
            Paint::ColrGlyph { glyph: name, transform, shape } => {
                let font = self.font;
                let to = place * *transform;
                let target = self.graph(name, depth).filter(|_| invertible(to)).zip(font.glyph(name));
                if let Some((graph, target)) = target {
                    self.glyphs.push(name.clone());
                    self.collect(target, &graph, clips, to, depth + 1);
                    self.glyphs.pop();
                    return;
                }
                let Some(outline) = shape_outline(self.font, glyph, shape) else { return };
                clips.push(placed(outline, place));
                if let Some(l) = leaf(&paint::outline_fill(), self.palette, self.text) {
                    self.push(clips, l, place);
                }
                clips.pop();
            }
            other => {
                if !clips.is_empty()
                    && let Some(l) = leaf(other, self.palette, self.text)
                {
                    self.push(clips, l, place);
                }
            }
        }
    }

    fn push(&mut self, clips: &[BezPath], leaf: Leaf, place: Affine) {
        let leaf = if place == Affine::IDENTITY { leaf } else { Leaf::Moved { to_leaf: place.inverse(), leaf: Box::new(leaf) } };
        self.out.push(Drawable { clips: clips.to_vec(), leaf });
    }

    /// The paint graph a reference to `name` at `depth` draws, or `None` when it is drawn as
    /// its component's outline, as the compile falls back: the glyph has no graph, is already
    /// being drawn (a cycle), references nest more than [`MAX_PAINT_DEPTH`] deep, or its graph
    /// would nest past [`MAX_PAINT_DEPTH`].
    fn graph(&mut self, name: &str, depth: usize) -> Option<Arc<Paint>> {
        if self.glyphs.len() > MAX_PAINT_DEPTH || self.glyphs.iter().any(|g| g == name) {
            return None;
        }
        let (graph, nesting) = match self.graphs.get(name) {
            Some(known) => known.clone(),
            None => {
                let font = self.font;
                let made = font.glyph(name).and_then(|g| self.paints.paint(g, &mut Vec::new()).ok().flatten()).map(|graph| {
                    #[cfg(test)]
                    {
                        self.nesting_walks += 1;
                    }
                    let nesting = paint::nesting(&graph);
                    (Arc::new(graph), nesting)
                });
                self.graphs.insert(name.to_string(), made.clone());
                made
            }
        }?;
        ((depth + 1).saturating_add(nesting) <= MAX_PAINT_DEPTH).then_some(graph)
    }
}

/// `outline` moved by `place`.
fn placed(mut outline: BezPath, place: Affine) -> BezPath {
    if place != Affine::IDENTITY {
        outline.apply_affine(place);
    }
    outline
}

/// Whether `a` maps the plane onto itself with finite numbers, so its inverse exists.
fn invertible(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite()) && a.determinant().abs() > 1e-12
}

/// Palette 0 of `master`'s palettes (the font's when the master has none).
fn palette0(font: &Font, master: &str) -> Vec<Rgba> {
    palette::read(font, Some(master)).ok().and_then(|p| p.palette(0).map(<[Rgba]>::to_vec)).unwrap_or_default()
}

fn vector(font: &Font, glyph: &Glyph, master: &str, text: [f32; 4]) -> Option<Content> {
    let palettes = palette::read(font, Some(master)).unwrap_or_default();
    let palette = palettes.palette(0).map(<[Rgba]>::to_vec).unwrap_or_default();
    let mut paints = paint::MasterPaints::new(font, master);
    let graph = paints.paint(glyph, &mut Vec::new()).ok()??;
    let mut c = Collect {
        font,
        paints,
        graphs: HashMap::new(),
        palette: &palette,
        text,
        out: Vec::new(),
        glyphs: vec![glyph.name.clone()],
        visits: 0,
        #[cfg(test)]
        nesting_walks: 0,
    };
    c.collect(glyph, &graph, &mut Vec::new(), Affine::IDENTITY, 0);
    Some(Content::Vector(c.out))
}

fn svg(layer: &Layer, src: &mut Sources) -> Option<Content> {
    let image = layer.info.image.as_ref()?;
    let path = image.path.as_deref()?;
    let place = image.transform.affine();
    let ((shapes, file), key) = src.svgs.get(src.assets, path, src.revision, place);
    let text = color32_straight(src.text);
    let out = shapes
        .iter()
        .take(MAX_DRAWABLES)
        .filter_map(|s| leaf(&s.paint, &[], text).map(|l| Drawable { clips: vec![s.outline.clone()], leaf: l }))
        .collect();
    Some(match file {
        Some(file) => Content::Filtered { file, key, plain: out },
        None => Content::Vector(out),
    })
}

fn bitmap(layer: &Layer) -> Option<Content> {
    let image = layer.info.image.as_ref().filter(|i| i.path.is_some())?;
    // The image is the glyph: drawn whole, not at a tracing image's opacity.
    Some(Content::Bitmap(Image { alpha: Some(100.0), ..image.clone() }))
}

/// The colour layers of `glyph` in `master` that are not intermediate layers, in file order.
fn static_layers<'g>(glyph: &'g Glyph, master: &str) -> Vec<(&'g Layer, ColorLayerKind)> {
    kinds::color_layers(glyph, master).into_iter().filter(|(l, _)| l.info.attrs.coordinates.is_empty()).collect()
}

/// What `glyph` draws in `mode`. `exact` is the edited glyph's active layer, drawn itself when it
/// is an iColor or svg layer.
fn content(font: &Font, glyph: &Glyph, mode: &Mode, exact: Option<&Layer>, src: &mut Sources) -> Option<Content> {
    let text = color32_straight(src.text);
    let (master, kind) = match mode {
        Mode::Layered { masters } => {
            let out = masters
                .iter()
                .filter_map(|(id, c)| {
                    let layer = glyph.layer(id)?;
                    let outline = layer_outline(font, layer).ok().map(concat)?;
                    (!outline.elements().is_empty()).then(|| Drawable { clips: vec![outline], leaf: Leaf::Solid(*c) })
                })
                .collect();
            return Some(Content::Vector(out));
        }
        Mode::Layers { master, kind } => (master.as_str(), *kind),
    };
    if let Some(layer) = exact {
        return match kinds::kind(layer)? {
            ColorLayerKind::IColor(_) => bitmap(layer),
            ColorLayerKind::Svg => svg(layer, src),
            ColorLayerKind::Palette(_) | ColorLayerKind::FullColor => vector(font, glyph, master, text),
        };
    }
    let order = match kind {
        ColorLayerKind::IColor(_) => [Pick::IColor, Pick::Vector, Pick::Svg],
        ColorLayerKind::Svg => [Pick::Svg, Pick::Vector, Pick::IColor],
        ColorLayerKind::Palette(_) | ColorLayerKind::FullColor => [Pick::Vector, Pick::Svg, Pick::IColor],
    };
    order.into_iter().find_map(|pick| by_kind(font, glyph, master, kind, pick, text, src))
}

/// Which kind of colour layer a glyph is drawn with.
#[derive(Clone, Copy)]
enum Pick {
    Vector,
    Svg,
    IColor,
}

/// The content of `glyph`'s layers of one kind in `master`; `kind` is the active layer's. Of
/// several iColor layers, the one of the active layer's size, else the largest.
fn by_kind(font: &Font, glyph: &Glyph, master: &str, kind: ColorLayerKind, pick: Pick, text: [f32; 4], src: &mut Sources) -> Option<Content> {
    match pick {
        Pick::Vector => vector(font, glyph, master, text),
        Pick::Svg => static_layers(glyph, master).into_iter().filter(|(_, k)| *k == ColorLayerKind::Svg).find_map(|(l, _)| svg(l, src)),
        Pick::IColor => {
            let sized: Vec<(&Layer, u16)> = static_layers(glyph, master)
                .into_iter()
                .filter(|(l, _)| l.info.image.as_ref().is_some_and(|i| i.path.is_some()))
                .filter_map(|(l, k)| if let ColorLayerKind::IColor(size) = k { Some((l, size)) } else { None })
                .collect();
            let same = match kind {
                ColorLayerKind::IColor(n) => sized.iter().find(|(_, size)| *size == n),
                _ => None,
            };
            same.or_else(|| sized.iter().max_by_key(|(_, size)| *size)).and_then(|(l, _)| bitmap(l))
        }
    }
}

// ── rasterising ─────────────────────────────────────────────────────────────────────────────────

/// Premultiplied RGBA pixels, rows top to bottom.
pub struct Pixels<'a> {
    pub data: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

const MASK: Style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };

/// `t` within 0 to 1 as the extend mode says.
fn extend(t: f64, mode: Extend) -> f64 {
    match mode {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let m = t.rem_euclid(2.0);
            if m > 1.0 { 2.0 - m } else { m }
        }
    }
}

fn sample(stops: &Stops, t: f64) -> [f32; 4] {
    let t = if t.is_finite() { extend(t, stops.extend) } else { 0.0 };
    let (Some(first), Some(last)) = (stops.at.first(), stops.at.last()) else { return [0.0; 4] };
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = stops.at.partition_point(|(o, _)| *o <= t);
    match (stops.at.get(i.wrapping_sub(1)), stops.at.get(i)) {
        (Some((o0, c0)), Some((o1, c1))) if o1 > o0 => {
            let k = ((t - o0) / (o1 - o0)) as f32;
            let mut out = [0.0; 4];
            for ((o, a), b) in out.iter_mut().zip(c0).zip(c1) {
                *o = a + (b - a) * k;
            }
            out
        }
        (Some((_, c)), _) | (_, Some((_, c))) => *c,
        _ => last.1,
    }
}

/// The colour of `leaf` at font point `p`; `None` where a radial gradient paints nothing.
fn eval(leaf: &Leaf, p: Point) -> Option<[f32; 4]> {
    match leaf {
        Leaf::Solid(c) => Some(*c),
        Leaf::Linear { p0, v, stops } => Some(sample(stops, (p.x - p0.x) * v.0 + (p.y - p0.y) * v.1)),
        Leaf::Radial { c0, r0, c1, r1, stops } => {
            // The largest t with |p − c(t)| = r(t) and r(t) ≥ 0, c and r moving from the start
            // circle to the end circle.
            let (cdx, cdy, dr) = (c1.x - c0.x, c1.y - c0.y, r1 - r0);
            let (pdx, pdy) = (p.x - c0.x, p.y - c0.y);
            let a = cdx * cdx + cdy * cdy - dr * dr;
            let b = pdx * cdx + pdy * cdy + r0 * dr;
            let c = pdx * pdx + pdy * pdy - r0 * r0;
            let r = |t: f64| r0 + t * dr;
            let t = if a.abs() < 1e-9 {
                (b.abs() > 1e-12).then(|| c / (2.0 * b)).filter(|t| r(*t) >= 0.0)
            } else {
                let disc = b * b - a * c;
                if disc < 0.0 {
                    None
                } else {
                    let s = disc.sqrt();
                    let (t1, t2) = ((b + s) / a, (b - s) / a);
                    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
                    [hi, lo].into_iter().find(|t| r(*t) >= 0.0)
                }
            }?;
            Some(sample(stops, t))
        }
        Leaf::Sweep { center, start, end, stops } => {
            let span = end - start;
            if span.abs() < 1e-9 {
                return stops.at.last().map(|(_, c)| *c);
            }
            let angle = (p.y - center.y).atan2(p.x - center.x).to_degrees().rem_euclid(360.0);
            Some(sample(stops, (angle - start) / span))
        }
        Leaf::Moved { to_leaf, leaf } => match leaf.as_ref() {
            Leaf::Moved { .. } => None,
            inner => eval(inner, *to_leaf * p),
        },
    }
}

/// Lays `color` (straight) at `coverage` over the premultiplied pixel at `at`.
fn blend(data: &mut [u8], at: usize, color: [f32; 4], coverage: f32) {
    let [r, g, b, a] = color;
    let alpha = (a * coverage).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let Some(px) = data.get_mut(at..at + 4) else { return };
    let keep = 1.0 - alpha;
    for (d, s) in px.iter_mut().zip([r * alpha, g * alpha, b * alpha, alpha]) {
        let v = s * 255.0 + f32::from(*d) * keep;
        *d = v.round().clamp(0.0, 255.0) as u8;
    }
}

/// The pixel box of `rect` under `t` within `width × height`; `None` when it is empty.
fn pixel_box(rect: fontcraft_geom::Rect, t: Affine, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let b = t.transform_rect_bbox(rect);
    if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
        return None;
    }
    let x0 = b.x0.floor().clamp(0.0, f64::from(width)) as u32;
    let y0 = b.y0.floor().clamp(0.0, f64::from(height)) as u32;
    let x1 = b.x1.ceil().clamp(0.0, f64::from(width)) as u32;
    let y1 = b.y1.ceil().clamp(0.0, f64::from(height)) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Paints `drawables` into `px`; `to_pixels` maps the glyph's font units to pixels. A glyph past
/// the work budget ([`MAX_PAINT_WORK`]) is painted as one fill of its outlines in `plain`.
fn paint_drawables(px: &mut Pixels, drawables: &[Drawable], to_pixels: Affine, plain: [f32; 4]) {
    paint_drawables_within(px, drawables, to_pixels, MAX_PAINT_WORK, plain);
}

/// What painting `drawables` into a `width × height` image takes.
struct Work {
    /// Each drawable's pixel box, `None` when it paints nothing.
    boxes: Vec<Option<(u32, u32, u32, u32)>>,
    /// The box of them all.
    all: Option<(u32, u32, u32, u32)>,
    /// Pixel operations needed.
    cost: u64,
    /// Pixel operations allowed, with at least `least`.
    budget: u64,
}

fn work(drawables: &[Drawable], to_pixels: Affine, width: u32, height: u32, least: u64) -> Work {
    let boxes: Vec<Option<(u32, u32, u32, u32)>> = drawables
        .iter()
        .map(|d| d.clips.first().filter(|f| !f.elements().is_empty()).and_then(|f| pixel_box(f.bounding_box(), to_pixels, width, height)))
        .collect();
    let all = boxes.iter().flatten().copied().reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));
    let area = all.map_or(0, |(x0, y0, x1, y1)| u64::from(x1 - x0) * u64::from(y1 - y0));
    let budget = least.max(PAINT_WORK_PER_PIXEL.saturating_mul(area).min(MAX_GLYPH_PAINT_WORK));
    let cost = drawables.iter().zip(&boxes).fold(0u64, |sum, (d, b)| {
        let Some((x0, y0, x1, y1)) = *b else { return sum };
        let clips = u64::try_from(d.clips.len()).unwrap_or(u64::MAX);
        sum.saturating_add((u64::from(x1 - x0) * u64::from(y1 - y0)).saturating_mul(clips.saturating_add(1)))
    });
    Work { boxes, all, cost, budget }
}

/// [`paint_drawables`] with at least `least` pixel operations to spend. Answers whether the
/// drawables were painted in colour; when they would cost more than the budget, nothing of
/// them is painted and their outlines are filled in `plain` instead.
fn paint_drawables_within(px: &mut Pixels, drawables: &[Drawable], to_pixels: Affine, least: u64, plain: [f32; 4]) -> bool {
    let inverse = to_pixels.inverse();
    if !inverse.as_coeffs().iter().all(|v| v.is_finite()) {
        return true;
    }
    let work = work(drawables, to_pixels, px.width, px.height, least);
    if work.cost > work.budget {
        paint_plain(px, drawables, to_pixels, &work, plain);
        return false;
    }
    for (d, b) in drawables.iter().zip(&work.boxes) {
        let Some((x0, y0, x1, y1)) = *b else { continue };
        let (w, h) = (x1 - x0, y1 - y0);
        let shift = Affine::translate((-f64::from(x0), -f64::from(y0))) * to_pixels;
        let mut coverage = vec![1.0f32; w as usize * h as usize];
        let mut drawn = true;
        for clip in &d.clips {
            let Ok(mask) = render_path(clip, shift, w, h, &MASK) else {
                drawn = false;
                break;
            };
            for (c, p) in coverage.iter_mut().zip(mask.pixels.as_chunks::<4>().0) {
                *c *= f32::from(p[3]) / 255.0;
            }
        }
        if !drawn {
            continue;
        }
        for (i, cov) in coverage.iter().enumerate() {
            if *cov <= 0.0 {
                continue;
            }
            let (col, row) = (i as u32 % w, i as u32 / w);
            let (x, y) = (x0 + col, y0 + row);
            let Some(color) = eval(&d.leaf, inverse * Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5)) else { continue };
            let at = (y as usize * px.width as usize + x as usize) * 4;
            blend(px.data, at, color, *cov);
        }
    }
    true
}

/// A pixel box: left, top, right, bottom.
type PixelBox = (u32, u32, u32, u32);

/// Fills the outer outline of every drawable in `color` within the box `work.all`.
fn paint_plain(px: &mut Pixels, drawables: &[Drawable], to_pixels: Affine, work: &Work, color: [f32; 4]) {
    let Some(all) = work.all else { return };
    let coverage = match layer_coverage(drawables, &work.boxes, to_pixels, all, work.budget) {
        Some(c) => c,
        None => {
            let Some(c) = joined_coverage(drawables, to_pixels, all) else { return };
            c
        }
    };
    let (x0, y0, x1, _) = all;
    let w = x1.saturating_sub(x0).max(1);
    for (i, cov) in coverage.iter().enumerate() {
        if *cov <= 0.0 {
            continue;
        }
        let (x, y) = (x0 + i as u32 % w, y0 + i as u32 / w);
        blend(px.data, (y as usize * px.width as usize + x as usize) * 4, color, *cov);
    }
}

/// The coverage of the box `all`, row by row, by the outer outlines of `drawables`: each
/// outline is rendered on its own box (`boxes`) and a pixel takes the most any one gives it, so
/// an outline cannot cut a hole in another whatever their windings. `None` when that costs more
/// than `budget` pixel operations.
fn layer_coverage(drawables: &[Drawable], boxes: &[Option<PixelBox>], to_pixels: Affine, all: PixelBox, budget: u64) -> Option<Vec<f32>> {
    let area = |(x0, y0, x1, y1): PixelBox| u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0));
    let cost = boxes.iter().flatten().fold(area(all), |sum, &b| sum.saturating_add(area(b)));
    if cost > budget {
        return None;
    }
    let (ax0, ay0, ax1, ay1) = all;
    let aw = ax1.saturating_sub(ax0);
    let mut coverage = vec![0.0f32; aw as usize * ay1.saturating_sub(ay0) as usize];
    for (d, b) in drawables.iter().zip(boxes) {
        let (Some(outline), Some((x0, y0, x1, y1))) = (d.clips.first(), *b) else { continue };
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        if w == 0 || h == 0 {
            continue;
        }
        let shift = Affine::translate((-f64::from(x0), -f64::from(y0))) * to_pixels;
        let Ok(mask) = render_path(outline, shift, w, h, &MASK) else { continue };
        for (i, p) in mask.pixels.as_chunks::<4>().0.iter().enumerate() {
            if p[3] == 0 {
                continue;
            }
            let col = x0.saturating_sub(ax0) + i as u32 % w;
            let row = y0.saturating_sub(ay0) + i as u32 / w;
            if let Some(c) = coverage.get_mut(row as usize * aw as usize + col as usize) {
                *c = c.max(f32::from(p[3]) / 255.0);
            }
        }
    }
    Some(coverage)
}

/// The coverage of the box `all`, row by row, by the outer outlines of `drawables` joined as one
/// path: one pass over the glyph's pixels whatever the number of drawables. Each outline is
/// turned to one winding first, so layers wound in opposite directions add up under the nonzero
/// rule; a layer that holds parts of both windings can still cancel another where they overlap.
fn joined_coverage(drawables: &[Drawable], to_pixels: Affine, all: PixelBox) -> Option<Vec<f32>> {
    let (x0, y0, x1, y1) = all;
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let outline =
        concat(drawables.iter().filter_map(|d| d.clips.first()).map(|c| if c.area() < 0.0 { c.reverse_subpaths() } else { c.clone() }).collect());
    let shift = Affine::translate((-f64::from(x0), -f64::from(y0))) * to_pixels;
    let mask = render_path(&outline, shift, w, h, &MASK).ok()?;
    Some(mask.pixels.as_chunks::<4>().0.iter().map(|p| f32::from(p[3]) / 255.0).collect())
}

/// Paints the bitmap of `image` (straight RGBA, rows top to bottom) into `px`: `place` maps the
/// image's own space (pixels, y up from its bottom left) to pixels. Nearest-pixel sampling.
fn paint_bitmap(px: &mut Pixels, decoded: &imagery::Rgba8, place: Affine) {
    let inverse = place.inverse();
    let (iw, ih) = (decoded.width, decoded.height);
    if iw == 0 || ih == 0 || !inverse.as_coeffs().iter().all(|v| v.is_finite()) {
        return;
    }
    let image_rect = fontcraft_geom::Rect::new(0.0, 0.0, f64::from(iw), f64::from(ih));
    let Some((x0, y0, x1, y1)) = pixel_box(image_rect, place, px.width, px.height) else { return };
    for y in y0..y1 {
        for x in x0..x1 {
            let q = inverse * Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
            if !(q.x >= 0.0 && q.y >= 0.0 && q.x < f64::from(iw) && q.y < f64::from(ih)) {
                continue;
            }
            let (sx, sy) = (q.x as usize, ih as usize - 1 - (q.y as usize).min(ih as usize - 1));
            let from = (sy * iw as usize + sx) * 4;
            let Some(&[r, g, b, a]) = decoded.pixels.get(from..from + 4).and_then(|s| <&[u8; 4]>::try_from(s).ok()) else { continue };
            let c = [r, g, b, a].map(|v| f32::from(v) / 255.0);
            blend(px.data, (y as usize * px.width as usize + x as usize) * 4, c, 1.0);
        }
    }
}

/// Paints glyph `name` of a preview in colour with its colour layer `layer`, into `px`:
/// `to_pixels` maps the glyph's font units, at its place on the line, to pixels.
pub fn paint_preview_glyph(font: &Font, name: &str, layer: &Layer, src: &mut Sources, px: &mut Pixels, to_pixels: Affine) {
    let Some(glyph) = font.glyph(name) else { return };
    let Some(kind) = kinds::kind(layer) else { return };
    let mode = Mode::Layers { master: layer.master.clone(), kind };
    match content(font, glyph, &mode, Some(layer), src) {
        Some(Content::Vector(list)) => paint_drawables(px, &list, to_pixels, color32_straight(src.text)),
        Some(Content::Filtered { file, key, plain }) => {
            if !paint_svg_file(px, src.svgs, &key, &file, to_pixels) {
                paint_drawables(px, &plain, to_pixels, color32_straight(src.text));
            }
        }
        Some(Content::Bitmap(image)) => {
            let Some(path) = image.path.as_deref() else { return };
            let Ok(bytes) = src.assets.read(path) else { return };
            let Ok(decoded) = imagery::decode_rgba(&bytes, imagery::MAX_PIXELS) else { return };
            paint_bitmap(px, &decoded, to_pixels * image.transform.affine());
        }
        None => {}
    }
}

/// The colour drawing of `glyph` in `mode` as an image of `width × height` pixels, `to_pixels`
/// mapping its font units to pixels; for tests and exports of the canvas. iColor images are left
/// out (they are textures of the image cache).
#[allow(clippy::too_many_arguments)]
pub fn glyph_image(
    font: &Font,
    glyph: &str,
    mode: &Mode,
    exact: Option<&Layer>,
    src: &mut Sources,
    width: u32,
    height: u32,
    to_pixels: Affine,
) -> Option<ColorImage> {
    let g = font.glyph(glyph)?;
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let mut px = Pixels { data: &mut data, width, height };
    match content(font, g, mode, exact, src)? {
        Content::Vector(list) => paint_drawables(&mut px, &list, to_pixels, color32_straight(src.text)),
        Content::Filtered { file, key, plain } => {
            if !paint_svg_file(&mut px, src.svgs, &key, &file, to_pixels) {
                paint_drawables(&mut px, &plain, to_pixels, color32_straight(src.text));
            }
        }
        Content::Bitmap(_) => return None,
    }
    Some(ColorImage::from_rgba_premultiplied([width as usize, height as usize], &data))
}

/// The pixels per font unit and the pixel of the glyph origin of `to_pixels` when it is an
/// upright uniform scale with y flipped, the placement an svg file's raster can be laid with.
fn upright(to_pixels: Affine) -> Option<(f64, f64, f64)> {
    let [a, b, c, d, e, f] = to_pixels.as_coeffs();
    let upright = b == 0.0 && c == 0.0 && a > 0.0 && (a + d).abs() <= a * 1e-9;
    (upright && [a, e, f].iter().all(|v| v.is_finite())).then_some((a, e, f))
}

/// Lays `raster` (premultiplied) over `px` with the glyph origin at pixel corner `(x, y)`.
fn lay(px: &mut Pixels, raster: &GlyphRaster, x: i64, y: i64) {
    let image = &raster.image;
    for row in 0..image.height {
        let ty = y + i64::from(raster.top) + i64::from(row);
        if ty < 0 || ty >= i64::from(px.height) {
            continue;
        }
        for col in 0..image.width {
            let tx = x + i64::from(raster.left) + i64::from(col);
            if tx < 0 || tx >= i64::from(px.width) {
                continue;
            }
            let Some([r, g, b, a]) = image.pixel(col, row) else { continue };
            if a == 0 {
                continue;
            }
            let at = (ty as usize * px.width as usize + tx as usize) * 4;
            let keep = 255 - u32::from(a);
            if let Some(d) = px.data.get_mut(at..at + 4) {
                for (d, s) in d.iter_mut().zip([r, g, b, a]) {
                    *d = (u32::from(s) + (u32::from(*d) * keep + 127) / 255).min(255) as u8;
                }
            }
        }
    }
}

/// Paints the svg file `file` (kept in `svgs` under `key`) into `px` through `to_pixels` with its
/// filters, from a raster `svgs` keeps; false when it is not drawn that way (a placement that is
/// not upright, or a refused drawing). The raster is shifted to the glyph origin's pixel
/// position within a quarter pixel.
fn paint_svg_file(px: &mut Pixels, svgs: &mut SvgCache, key: &SvgKey, file: &SvgFile, to_pixels: Affine) -> bool {
    let Some((scale, ox, oy)) = upright(to_pixels) else { return false };
    let step = |v: f64| {
        let q = (v.clamp(-1e12, 1e12) * RASTER_STEPS).round();
        let whole = (q / RASTER_STEPS).floor();
        (whole as i64, (q - whole * RASTER_STEPS).clamp(0.0, RASTER_STEPS - 1.0) as u8)
    };
    let ((x, sx), (y, sy)) = (step(ox), step(oy));
    let Some(raster) = svgs.raster(key, file, scale, [sx, sy]) else { return false };
    lay(px, &raster, x, y);
    true
}

// ── Edit View ───────────────────────────────────────────────────────────────────────────────────

/// A glyph's colour drawing as Edit View keeps it.
enum Drawn {
    /// A texture of the glyph's pixels: `bounds` the whole drawing and `region` the part
    /// rasterised, in pixels from the glyph's origin (x right, y down); `None` without pixels.
    Sprite {
        texture: Option<TextureHandle>,
        bounds: [i64; 4],
        region: [i64; 4],
        bytes: usize,
        /// Rasterised whole (`region` holds `bounds`), so scrolling never makes it again.
        whole: bool,
    },
    Bitmap(Image),
}

struct Sprite {
    glyph: Arc<Glyph>,
    doc: (u64, u64),
    /// Its layers hold components, whose outlines depend on other glyphs.
    components: bool,
    stamp: u64,
    /// Pixels per font unit.
    scale: u64,
    drawn: Drawn,
    used: u64,
    /// Its svg file with filters was refused at this scale: a scroll redraws only its outlines.
    refused: bool,
    /// Why its svg file is drawn without its filters, or without some of them.
    note: Option<String>,
}

impl Sprite {
    fn bytes(&self) -> usize {
        match &self.drawn {
            Drawn::Sprite { bytes, .. } => *bytes,
            Drawn::Bitmap(_) => 0,
        }
    }
}

/// Colour drawing state: the glyph textures, the svg files read, the gradient knob drag.
#[derive(Default)]
pub struct ColorUi {
    sprites: HashMap<(String, Option<String>), Sprite>,
    pub svgs: SvgCache,
    /// The palette stamp of a document revision and master.
    palette_stamp: Option<((u64, u64), String, u64)>,
    doc: Option<u64>,
    frame: u64,
    /// Glyph drawings made, and svg files with filters drawn (a file wholly outside the view
    /// draws nothing and is not counted), since the start.
    made: u64,
    file_draws: u64,
    pub knobs: super::gradient_knobs::KnobUi,
}

impl ColorUi {
    /// Glyph textures held, for tests.
    pub fn sprites(&self) -> usize {
        self.sprites.len()
    }

    /// Bytes of the glyph textures held, for tests.
    pub fn sprite_bytes(&self) -> usize {
        self.sprites.values().map(Sprite::bytes).sum()
    }

    /// Glyph drawings made, and svg files with filters drawn, since the start; for tests.
    pub fn made(&self) -> (u64, u64) {
        (self.made, self.file_draws)
    }

    fn palette_stamp(&mut self, font: &Font, doc: (u64, u64), master: &str) -> u64 {
        if let Some((d, m, s)) = &self.palette_stamp
            && *d == doc
            && m == master
        {
            return *s;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        palette0(font, master).hash(&mut h);
        let s = h.finish();
        self.palette_stamp = Some((doc, master.to_string(), s));
        s
    }

    /// Drops the sprites not drawn this frame, least recently drawn first, until the rest are
    /// within [`MAX_SPRITES`] and [`MAX_SPRITE_BYTES`] or none of them is left. This frame's are
    /// kept: [`draw`] makes no more of them once their bytes pass the limit.
    fn evict(&mut self) {
        let mut count = self.sprites.len();
        let mut total: usize = self.sprites.values().map(Sprite::bytes).sum();
        if count <= MAX_SPRITES && total <= MAX_SPRITE_BYTES {
            return;
        }
        let frame = self.frame;
        let mut old: Vec<(u64, (String, Option<String>), usize)> =
            self.sprites.iter().filter(|(_, s)| s.used != frame).map(|(k, s)| (s.used, k.clone(), s.bytes())).collect();
        old.sort_by_key(|o| o.0);
        for (_, key, bytes) in old {
            if count <= MAX_SPRITES && total <= MAX_SPRITE_BYTES {
                break;
            }
            self.sprites.remove(&key);
            count -= 1;
            total = total.saturating_sub(bytes);
        }
    }
}

/// What [`draw`] reads besides the painting.
pub struct Scene<'a> {
    pub session: &'a Session,
    pub tab: &'a Tab,
    pub font: &'a Font,
    pub layout: Option<&'a Layout>,
    /// Where the edited glyph sits in the text.
    pub origin: [f64; 2],
    /// The document's id and revision.
    pub doc: (u64, u64),
}

/// Draws the tab's glyphs in colour as [`mode`] says, behind the edited outline. A glyph's
/// texture covers the canvas and [`CLIP_MARGIN`] around it; one wholly outside that is not
/// rasterised. Once this frame's textures pass [`MAX_SPRITE_BYTES`], further glyphs are left
/// out of the frame rather than evicting textures it draws.
pub fn draw(c: &Painting, ctx: &egui::Context, ui: &mut ColorUi, images: &mut super::images::ImageUi, s: &Scene) {
    if ui.doc != Some(s.doc.0) {
        ui.sprites.clear();
        ui.svgs.clear();
        ui.doc = Some(s.doc.0);
    }
    let tokens = &c.tokens.color;
    let Some(mode) = mode(s.font, s.tab, c.tokens.dark) else { return };
    let Some(layout) = s.layout else { return };
    ui.frame = ui.frame.wrapping_add(1);
    let ppp = f64::from(ctx.pixels_per_point());
    let scale = c.view.scale() * ppp;
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    let master_stamp = match &mode {
        Mode::Layers { master, .. } => ui.palette_stamp(s.font, s.doc, master),
        Mode::Layered { .. } => 0,
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (format!("{mode:?}"), master_stamp, tokens.text_color.to_array(), images.revision).hash(&mut h);
    let stamp = h.finish();
    let to_screen = Affine::translate(c.view.origin.to_vec2()) * Affine::scale_non_uniform(c.view.scale(), -c.view.scale());
    let edited_index = s.tab.edited_index();
    let active = super::view::active_layer(s.font, s.tab);
    let exact = active.filter(|l| draws_layer_image(l));
    let assets = s.session.assets();
    let mut src = Sources { assets: assets.as_ref(), svgs: &mut ui.svgs, revision: images.revision, text: tokens.text_color };
    let canvas = [f64::from(c.rect.min.x) * ppp, f64::from(c.rect.min.y) * ppp, f64::from(c.rect.max.x) * ppp, f64::from(c.rect.max.y) * ppp];
    let frame = ui.frame;
    let plain = color32_straight(tokens.text_color);
    let mut fresh = false;
    // Bytes of the textures this frame draws.
    let mut frame_bytes = 0usize;
    let mut notes: Vec<(&str, String)> = Vec::new();
    for (i, placed) in layout.placed.iter().enumerate() {
        let glyphs = placed
            .glyph
            .as_deref()
            .map(|g| (g, placed.x, placed.y, Some(i) == edited_index))
            .into_iter()
            .chain(placed.extra.iter().map(|(g, x, y)| (g.as_ref(), *x, *y, false)));
        for (name, x, y, edited) in glyphs {
            let Some(glyph) = s.font.glyph(name) else { continue };
            if !has_color(glyph, &mode) {
                continue;
            }
            let place = to_screen * Affine::translate((x - s.origin[0], y - s.origin[1]));
            let exact = if edited { exact } else { None };
            let key = (name.to_string(), exact.map(|l| l.id.clone()));
            let o = place * Point::ZERO;
            let (ox, oy) = ((o.x * ppp).round(), (o.y * ppp).round());
            if !(ox.is_finite() && oy.is_finite()) {
                continue;
            }
            // The canvas, and the part a texture holds, in pixels from the glyph's origin.
            let inner = [canvas[0] - ox, canvas[1] - oy, canvas[2] - ox, canvas[3] - oy];
            let view = [inner[0] - CLIP_MARGIN, inner[1] - CLIP_MARGIN, inner[2] + CLIP_MARGIN, inner[3] + CLIP_MARGIN];
            let old = ui.sprites.get(&key).filter(|e| {
                e.stamp == stamp
                    && e.scale == scale.to_bits()
                    && (e.doc == s.doc || (e.doc.0 == s.doc.0 && Arc::ptr_eq(&e.glyph, glyph) && !e.components))
            });
            let valid = old.is_some_and(|e| match &e.drawn {
                Drawn::Sprite { bounds, region, whole, .. } => *whole || covers(*region, clip(*bounds, inner)),
                Drawn::Bitmap(_) => true,
            });
            if !valid {
                if frame_bytes > MAX_SPRITE_BYTES {
                    continue;
                }
                // Moved only: a file refused at this scale is not drawn again.
                let refused = old.filter(|e| e.refused).map(|e| e.note.clone());
                let mut note = None;
                let mut file_refused = false;
                let drawn = match content(s.font, glyph, &mode, exact, &mut src) {
                    Some(Content::Bitmap(image)) => Drawn::Bitmap(image),
                    Some(Content::Vector(list)) => rasterise(ctx, &list, scale, view, name, plain),
                    Some(Content::Filtered { plain: outlines, .. }) if refused.is_some() => {
                        file_refused = true;
                        note = refused.flatten();
                        rasterise(ctx, &outlines, scale, view, name, plain)
                    }
                    Some(Content::Filtered { file, plain: outlines, .. }) => {
                        let result = file.draw(scale, (0.0, 0.0), Some(view));
                        if !matches!(result, Ok(FileRaster { raster: None, .. })) {
                            ui.file_draws += 1;
                        }
                        match result {
                            Ok(f) => {
                                note = f.raster.as_ref().and_then(|r| r.warnings.first().cloned());
                                file_sprite(ctx, f, name)
                            }
                            Err(reason) => {
                                file_refused = true;
                                note = Some(format!("drawn from its outlines without its filters: {reason}"));
                                rasterise(ctx, &outlines, scale, view, name, plain)
                            }
                        }
                    }
                    None => Drawn::Sprite { texture: None, bounds: [0; 4], region: [0; 4], bytes: 0, whole: false },
                };
                ui.made += 1;
                let components = glyph.layers.iter().any(|l| l.shapes.iter().any(|s| matches!(s, Shape::Component(_))));
                let sprite = Sprite {
                    glyph: glyph.clone(),
                    doc: s.doc,
                    components,
                    stamp,
                    scale: scale.to_bits(),
                    drawn,
                    used: frame,
                    refused: file_refused,
                    note,
                };
                frame_bytes = frame_bytes.saturating_add(sprite.bytes());
                ui.sprites.insert(key.clone(), sprite);
                fresh = true;
            }
            let Some(entry) = ui.sprites.get_mut(&key) else { continue };
            if entry.used != frame {
                frame_bytes = frame_bytes.saturating_add(entry.bytes());
            }
            entry.used = frame;
            entry.doc = s.doc;
            match &entry.drawn {
                Drawn::Sprite { texture, region, bounds, .. } => {
                    if let Some(note) = entry.note.as_ref().filter(|_| clip(*bounds, inner) != [0; 4])
                        && !notes.iter().any(|(n, _)| *n == name)
                    {
                        notes.push((name, note.clone()));
                    }
                    if let Some(texture) = texture {
                        let [x0, y0, x1, y1] = region.map(|v| v as f64);
                        let min = pos2(((ox + x0) / ppp) as f32, ((oy + y0) / ppp) as f32);
                        let rect = Rect::from_min_size(min, vec2(((x1 - x0) / ppp) as f32, ((y1 - y0) / ppp) as f32));
                        c.painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
                Drawn::Bitmap(image) => {
                    draw_layer_image(c, &mut images.cache, ctx, src.assets, image, place, images.revision);
                }
            }
        }
    }
    if fresh {
        ui.evict();
    }
    if let Some((name, note)) = notes.first() {
        let more = match notes.len() - 1 {
            0 => String::new(),
            1 => " (and 1 more glyph)".to_string(),
            n => format!(" (and {n} more glyphs)"),
        };
        let at = c.rect.left_bottom() + vec2(8.0, -8.0);
        c.painter.text(at, egui::Align2::LEFT_BOTTOM, format!("{name}: {note}{more}"), egui::FontId::proportional(tokens.notice_size), tokens.notice);
    }
}

/// Whether `region` holds `part`, both `[x0, y0, x1, y1]`; an empty part is always held.
fn covers(region: [i64; 4], part: [i64; 4]) -> bool {
    part == [0; 4] || (region[0] <= part[0] && region[1] <= part[1] && region[2] >= part[2] && region[3] >= part[3])
}

/// `bounds` within `view`, both `[x0, y0, x1, y1]`; all zero when they do not meet.
fn clip(bounds: [i64; 4], view: [f64; 4]) -> [i64; 4] {
    let v = [view[0].floor(), view[1].floor(), view[2].ceil(), view[3].ceil()].map(|f| f.clamp(-1.0e12, 1.0e12) as i64);
    let out = [bounds[0].max(v[0]), bounds[1].max(v[1]), bounds[2].min(v[2]), bounds[3].min(v[3])];
    if out[2] > out[0] && out[3] > out[1] { out } else { [0; 4] }
}

/// The drawables at `scale` pixels per font unit as a texture of their part within `view`
/// (pixels from the glyph's origin); past the work budget, a fill of their outlines in `plain`.
fn rasterise(ctx: &egui::Context, list: &[Drawable], scale: f64, view: [f64; 4], name: &str, plain: [f32; 4]) -> Drawn {
    let to_px = Affine::scale_non_uniform(scale, -scale);
    let mut bounds: Option<fontcraft_geom::Rect> = None;
    for d in list {
        if let Some(first) = d.clips.first().filter(|p| !p.elements().is_empty()) {
            let b = to_px.transform_rect_bbox(first.bounding_box());
            bounds = Some(bounds.map_or(b, |o| o.union(b)));
        }
    }
    let empty = Drawn::Sprite { texture: None, bounds: [0; 4], region: [0; 4], bytes: 0, whole: false };
    let Some(b) = bounds.filter(|b| b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) else { return empty };
    let bounds = [b.x0.floor() - 1.0, b.y0.floor() - 1.0, b.x1.ceil() + 1.0, b.y1.ceil() + 1.0].map(|v| v.clamp(-1.0e12, 1.0e12) as i64);
    let region = clip(bounds, view);
    let (w, h) = (region[2] - region[0], region[3] - region[1]);
    let (Ok(width), Ok(height)) = (u32::try_from(w), u32::try_from(h)) else {
        return Drawn::Sprite { texture: None, bounds, region, bytes: 0, whole: false };
    };
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) * 4 > MAX_SPRITE_BYTES as u64 {
        return Drawn::Sprite { texture: None, bounds, region, bytes: 0, whole: false };
    }
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let to_pixels = Affine::translate((-(region[0] as f64), -(region[1] as f64))) * to_px;
    paint_drawables(&mut Pixels { data: &mut data, width, height }, list, to_pixels, plain);
    let bytes = data.len();
    let image = ColorImage::from_rgba_premultiplied([width as usize, height as usize], &data);
    let texture = ctx.load_texture(format!("color-glyph:{name}"), image, TextureOptions::LINEAR);
    Drawn::Sprite { texture: Some(texture), bounds, region, bytes, whole: false }
}

/// An svg file's raster within the view as a texture: its pixels from the glyph's origin, x
/// right and y down.
fn file_sprite(ctx: &egui::Context, f: FileRaster, name: &str) -> Drawn {
    let bounds = f.bounds;
    let Some(raster) = f.raster.filter(|r| r.image.width > 0 && r.image.height > 0) else {
        return Drawn::Sprite { texture: None, bounds, region: [0; 4], bytes: 0, whole: bounds == [0; 4] };
    };
    let (w, h) = (raster.image.width, raster.image.height);
    let (left, top) = (i64::from(raster.left), i64::from(raster.top));
    let region = [left, top, left + i64::from(w), top + i64::from(h)];
    let image = ColorImage::from_rgba_premultiplied([w as usize, h as usize], &raster.image.pixels);
    let texture = ctx.load_texture(format!("color-glyph:{name}"), image, TextureOptions::LINEAR);
    Drawn::Sprite { texture: Some(texture), bounds, region, bytes: raster.image.pixels.len(), whole: covers(region, bounds) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_engine::color::paint::{ColorLine, ColorStop};

    fn line(a: [f64; 4], b: [f64; 4]) -> ColorLine {
        ColorLine {
            stops: vec![ColorStop { offset: 0.0, color: ColorRef::Rgba(a) }, ColorStop { offset: 1.0, color: ColorRef::Rgba(b) }],
            extend: Extend::Pad,
        }
    }

    #[test]
    fn gradients_follow_their_geometry_and_extend_mode() {
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        // Left to right over 0..100, isolines vertical.
        let p = Paint::Linear { p0: Point::new(0.0, 0.0), p1: Point::new(100.0, 0.0), p2: Point::new(0.0, -100.0), line: line(red, blue) };
        let l = leaf(&p, &[], [0.0; 4]).unwrap();
        assert_eq!(eval(&l, Point::new(-50.0, 30.0)), Some([1.0, 0.0, 0.0, 1.0]), "padded before the start");
        let mid = eval(&l, Point::new(50.0, 999.0)).unwrap();
        assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[2] - 0.5).abs() < 1e-6, "{mid:?}");
        // A rotated p2 turns the isolines: p1 projects onto the normal of p0–p2.
        let skew = Paint::Linear { p0: Point::new(0.0, 0.0), p1: Point::new(100.0, 100.0), p2: Point::new(0.0, -100.0), line: line(red, blue) };
        let l = leaf(&skew, &[], [0.0; 4]).unwrap();
        assert_eq!(eval(&l, Point::new(100.0, 0.0)), Some([0.0f32, 0.0, 1.0, 1.0]), "x alone decides the colour");
        let mut repeat = line(red, blue);
        repeat.extend = Extend::Repeat;
        let r = Paint::Radial { c0: Point::new(0.0, 0.0), r0: 0.0, c1: Point::new(0.0, 0.0), r1: 10.0, line: repeat };
        let l = leaf(&r, &[], [0.0; 4]).unwrap();
        assert_eq!(eval(&l, Point::new(0.0, 0.0)), Some([1.0f32, 0.0, 0.0, 1.0]), "the centre is the first stop");
        let again = eval(&l, Point::new(12.5, 0.0)).unwrap();
        assert!((again[2] - 0.25).abs() < 1e-5, "repeats past the end circle: {again:?}");
        let sweep = Paint::Sweep { center: Point::new(0.0, 0.0), start: 0.0, end: 360.0, line: line(red, blue) };
        let l = leaf(&sweep, &[], [0.0; 4]).unwrap();
        let quarter = eval(&l, Point::new(0.0, 10.0)).unwrap();
        assert!((quarter[2] - 0.25).abs() < 1e-5, "counter-clockwise from the x axis: {quarter:?}");
    }

    #[test]
    fn painting_stops_at_the_work_budget() {
        let square = |size: f64| {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((size, 0.0));
            p.line_to((size, size));
            p.line_to((0.0, size));
            p.close_path();
            p
        };
        let red = Drawable { clips: vec![square(64.0)], leaf: Leaf::Solid([1.0, 0.0, 0.0, 1.0]) };
        let blue = Drawable { clips: vec![square(64.0)], leaf: Leaf::Solid([0.0, 0.0, 1.0, 1.0]) };
        // Each fill costs 64 × 64 × 2 operations; 1,000 of them are far past a budget of 100 fills.
        let mut list: Vec<Drawable> = (0..1_000).map(|_| Drawable { clips: red.clips.clone(), leaf: Leaf::Solid([1.0, 0.0, 0.0, 1.0]) }).collect();
        list.push(blue);
        let green = [0.0, 1.0, 0.0, 1.0];
        let mut data = vec![0u8; 64 * 64 * 4];
        let all = paint_drawables_within(&mut Pixels { data: &mut data, width: 64, height: 64 }, &list, Affine::IDENTITY, 100 * 64 * 64 * 2, green);
        assert!(!all, "the budget runs out");
        assert_eq!(data[..4], [0, 255, 0, 255], "past the budget the outlines are filled in the plain colour, no layer at all");
        assert!(data.chunks(4).all(|p| p == &[0, 255, 0, 255][..]), "the whole outline is filled");
        let mut data = vec![0u8; 64 * 64 * 4];
        assert!(paint_drawables_within(
            &mut Pixels { data: &mut data, width: 64, height: 64 },
            &list[990..],
            Affine::IDENTITY,
            100 * 64 * 64 * 2,
            green
        ));
        assert_eq!(data[..4], [0, 0, 255, 255], "a glyph within the budget is painted whole");
    }

    #[test]
    fn the_plain_fill_has_no_hole_where_layers_wind_oppositely() {
        let mut square = BezPath::new();
        square.move_to((0.0, 0.0));
        square.line_to((64.0, 0.0));
        square.line_to((64.0, 64.0));
        square.line_to((0.0, 64.0));
        square.close_path();
        let reversed = square.reverse_subpaths();
        // Half the layers wound each way: joined as they are, their windings sum to zero.
        let list: Vec<Drawable> = (0..1_000)
            .map(|i| Drawable { clips: vec![if i % 2 == 0 { square.clone() } else { reversed.clone() }], leaf: Leaf::Solid([1.0, 0.0, 0.0, 1.0]) })
            .collect();
        let green = [0.0, 1.0, 0.0, 1.0];
        let mut data = vec![0u8; 64 * 64 * 4];
        let all = paint_drawables_within(&mut Pixels { data: &mut data, width: 64, height: 64 }, &list, Affine::IDENTITY, 100 * 64 * 64 * 2, green);
        assert!(!all, "the budget runs out");
        assert!(data.chunks(4).all(|p| p == &[0, 255, 0, 255][..]), "the overlap is filled solid");
    }

    #[test]
    fn the_plain_fill_has_no_hole_where_a_layer_holds_both_windings() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
            let mut p = BezPath::new();
            p.move_to((x0, y0));
            p.line_to((x1, y0));
            p.line_to((x1, y1));
            p.line_to((x0, y1));
            p.close_path();
            p
        };
        // One layer: a large square and, apart from it, a small one wound the other way; nonzero
        // fills both. The other layer covers the small square, wound like the large one.
        let mut mixed = rect(0.0, 0.0, 100.0, 100.0);
        mixed.extend(rect(200.0, 20.0, 220.0, 40.0).reverse_subpaths().elements().iter().copied());
        let red = Leaf::Solid([1.0, 0.0, 0.0, 1.0]);
        let list = vec![Drawable { clips: vec![mixed], leaf: red.clone() }, Drawable { clips: vec![rect(190.0, 10.0, 230.0, 50.0)], leaf: red }];
        let (width, height) = (240u32, 110u32);
        let work = work(&list, Affine::IDENTITY, width, height, 0);
        let all = work.all.unwrap();
        let at = |x: u32, y: u32| ((y - all.1) * (all.2 - all.0) + (x - all.0)) as usize;
        let joined = joined_coverage(&list, Affine::IDENTITY, all).unwrap();
        assert!(joined[at(210, 30)] < 0.5, "joined as one path, the windings cancel there");
        let green = [0.0, 1.0, 0.0, 1.0];
        let mut data = vec![0u8; (width * height * 4) as usize];
        paint_plain(&mut Pixels { data: &mut data, width, height }, &list, Affine::IDENTITY, &work, green);
        let pixel = |x: u32, y: u32| data[((y * width + x) * 4) as usize..][..4].to_vec();
        assert_eq!(pixel(210, 30), [0, 255, 0, 255], "the overlap is filled");
        assert_eq!(pixel(195, 15), [0, 255, 0, 255], "the covering layer is filled");
        assert_eq!(pixel(50, 50), [0, 255, 0, 255], "the large square is filled");
        assert_eq!(pixel(150, 50), [0, 0, 0, 0], "between them nothing is");
    }

    #[test]
    fn a_glyph_of_twenty_full_layers_fits_the_budget_on_a_large_canvas() {
        let square = |size: f64| {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((size, 0.0));
            p.line_to((size, size));
            p.line_to((0.0, size));
            p.close_path();
            p
        };
        // Twenty layers that each cover the whole glyph, drawn at 4096 × 4096 pixels.
        let list: Vec<Drawable> = (0..20).map(|_| Drawable { clips: vec![square(4096.0)], leaf: Leaf::Solid([1.0, 0.0, 0.0, 1.0]) }).collect();
        let w = work(&list, Affine::IDENTITY, 4096, 4096, MAX_PAINT_WORK);
        assert_eq!(w.cost, 20 * 4096 * 4096 * 2);
        assert!(w.cost <= w.budget, "every layer is drawn: {} of {}", w.cost, w.budget);
        // The same glyph painted small draws its top layer, not a plain fill.
        let mut list = list;
        for d in &mut list {
            d.clips = vec![square(32.0)];
        }
        if let Some(top) = list.last_mut() {
            top.leaf = Leaf::Solid([0.0, 0.0, 1.0, 1.0]);
        }
        let mut data = vec![0u8; 32 * 32 * 4];
        assert!(paint_drawables_within(
            &mut Pixels { data: &mut data, width: 32, height: 32 },
            &list,
            Affine::IDENTITY,
            MAX_PAINT_WORK,
            [0.0, 1.0, 0.0, 1.0]
        ));
        assert_eq!(data[..4], [0, 0, 255, 255]);
    }

    #[test]
    fn palette_rows_resolve_with_their_alpha_and_missing_rows_take_the_text_colour() {
        let palette = [Rgba::new(255, 0, 0, 255)];
        let text = [1.0, 1.0, 1.0, 1.0];
        assert_eq!(resolve(ColorRef::Palette { index: 0, alpha: 0.5 }, &palette, text), [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(resolve(ColorRef::Palette { index: 7, alpha: 1.0 }, &palette, text), text);
        assert_eq!(resolve(ColorRef::Foreground { alpha: 1.0 }, &palette, text), text);
        let mut data = vec![0u8; 4];
        blend(&mut data, 0, [1.0, 0.0, 0.0, 0.0], 1.0);
        assert_eq!(data, [0, 0, 0, 0], "a colour with no opacity paints nothing");
    }

    fn collector<'f>(font: &'f Font, master: &'f str) -> Collect<'f> {
        Collect {
            font,
            paints: paint::MasterPaints::new(font, master),
            graphs: HashMap::new(),
            palette: &[],
            text: [0.0, 0.0, 0.0, 1.0],
            out: Vec::new(),
            glyphs: vec!["top".to_string()],
            visits: 0,
            nesting_walks: 0,
        }
    }

    #[test]
    fn references_fanning_out_stop_at_the_visit_limit() {
        // `g0` holds two references to `g1`, `g1` two to `g2`, and so on down to `g20`, a paint
        // outside any glyph, which draws nothing: followed in full, one reference to `g0` visits
        // 2^20 of them.
        const LEVELS: usize = 20;
        let mut font = Font::new("Fan Out");
        let master = font.masters[0].id.clone();
        for name in std::iter::once("top".to_string()).chain((0..=LEVELS).map(|k| format!("g{k}"))) {
            font.add_glyph(&name, Vec::new(), 600.0).unwrap();
        }
        let shape = ShapeRef { layer: master.clone(), shapes: vec![0], stroke: None };
        let reference = |k: usize| Paint::ColrGlyph { glyph: format!("g{k}"), transform: Affine::IDENTITY, shape: shape.clone() };
        let mut c = collector(&font, &master);
        for k in 0..=LEVELS {
            let graph = if k == LEVELS {
                Paint::Solid(ColorRef::Rgba([1.0, 0.0, 0.0, 1.0]))
            } else {
                Paint::Layers(vec![reference(k + 1), reference(k + 1)])
            };
            let nesting = paint::nesting(&graph);
            c.graphs.insert(format!("g{k}"), Some((Arc::new(graph), nesting)));
        }
        let top = font.glyph("top").unwrap();
        c.collect(top, &reference(0), &mut Vec::new(), Affine::IDENTITY, 0);
        assert_eq!(c.visits, MAX_PAINT_VISITS);
        assert!(c.out.is_empty());
    }

    #[test]
    fn many_references_to_one_colour_glyph_walk_its_graph_once() {
        use fontcraft_model::{Color, LayerAttrs, Path};

        // `dot`'s full-colour layer is a red square; the drawn paint refers to `dot` 5000 times.
        let mut font = Font::new("References");
        let master = font.masters[0].id.clone();
        font.add_glyph("top", Vec::new(), 600.0).unwrap();
        font.add_glyph("dot", Vec::new(), 600.0).unwrap();
        {
            let mut g = font.glyph_mut("dot").unwrap();
            let mut red = Path::rect(0.0, 0.0, 100.0, 100.0);
            red.info.attrs.fill_color = Some(Color::Rgba([1.0, 0.0, 0.0, 1.0]));
            let mut l = Layer::new(master.as_str(), 600.0);
            l.id = "dot-full".into();
            l.info.attrs = kinds::with_kind(&LayerAttrs::default(), Some(ColorLayerKind::FullColor));
            l.shapes = vec![Shape::Path(red)];
            g.layers.push(l);
        }
        let shape = ShapeRef { layer: master.clone(), shapes: vec![0], stroke: None };
        let references =
            Paint::Layers((0..5000).map(|_| Paint::ColrGlyph { glyph: "dot".into(), transform: Affine::IDENTITY, shape: shape.clone() }).collect());
        let mut c = collector(&font, &master);
        let top = font.glyph("top").unwrap();
        c.collect(top, &references, &mut Vec::new(), Affine::IDENTITY, 0);
        assert_eq!(c.nesting_walks, 1);
        assert_eq!(c.out.len(), 5000);
    }
}
