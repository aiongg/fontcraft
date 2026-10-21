//! Colour glyphs of a compiled font (M11.17): what Text Preview shows in place of a glyph's
//! outline, drawn from the font's own colour tables rather than from the editor's layers.
//!
//! Rows: `color.preview.textPreview`.
//!
//! | Table | Drawn |
//! |---|---|
//! | `COLR` (versions 0 and 1) with `CPAL` | through skrifa's colour painter: clips, solid fills, linear, radial and sweep gradients, transforms, and composite layers, with the palette [`ColorOptions::palette`] |
//! | `SVG ` | the glyph's document, inflated first when it is gzip-compressed, through the SVG drawing of `fontcraft_color` ([`svg_draw::drawing`]): filled outlines with solid, gradient or pattern paint, embedded PNG and JPEG images, clip paths, masks and filters |
//! | `sbix` | the PNG or JPEG image of the strike nearest the pixel size: the smallest strike at least that large, else the largest |
//! | `CBDT` with `CBLC` | the image of the strike nearest the pixel size, by the same rule: PNG data, 32-bit colour, and masks of 1 to 8 bits drawn in the text colour |
//!
//! A glyph in more than one table is drawn from `COLR`, else `SVG `, else `sbix`, else `CBDT`;
//! the first three are the order in which Edit View picks a glyph's colour layers. Palette index
//! 65,535, and a row the palette lacks, draw in [`ColorOptions::text_color`].
//!
//! Contract:
//! - Each glyph is rasterised once per palette, text colour and scale into a sprite whose pixel
//!   grid has the glyph origin on a pixel corner ([`GlyphRaster`]); [`composite`] lays sprites
//!   onto a page. Sprites are kept in a bounded cache ([`MAX_CACHE_GLYPHS`], [`MAX_CACHE_BYTES`]),
//!   the least recently used evicted.
//! - What a glyph draws is read once, whatever the scale: its `SVG ` document inflated, parsed
//!   and its images decoded, or the decoded image of one strike. These sources are kept in a
//!   second bounded cache ([`MAX_CACHE_GLYPHS`], [`MAX_SOURCE_BYTES`]).
//! - Work is counted in pixel operations. One glyph may take [`MIN_GLYPH_WORK`], or
//!   [`WORK_PER_PIXEL`] per pixel of its sprite when that is more; a glyph past its budget, a
//!   sprite larger than [`MAX_GLYPH_SIDE`] or [`MAX_GLYPH_PIXELS`], or data that cannot be read
//!   is [`Painted::Refused`], and the caller draws its outline. A [`FrameBudget`] spreads the
//!   first rasterisation of many glyphs over several frames: once it is spent, further glyphs
//!   are [`Painted::Deferred`]. Reading a source counts its bytes and decoded pixels.
//! - An `SVG ` document's user space has the glyph origin at its origin, y down, one unit per
//!   font unit. A document whose root gives a `viewBox`, `width` or `height` has its viewport
//!   (usvg's size: `width` and `height`, else the `viewBox`'s) fitted to the em square, which
//!   runs from the glyph origin `units_per_em` right and down, by the root's
//!   `preserveAspectRatio` (by default centred, the whole viewport shown), as the OpenType
//!   `SVG ` table defines.
//! - A compressed document inflates to at most `fontcraft_color::MAX_SVG_FILE_BYTES`; the images
//!   of one document decode to at most [`MAX_BITMAP_PIXELS`] pixels together, and an image that
//!   cannot be decoded or would pass that budget is left out. An image used twice is decoded once.
//! - In an `SVG ` document, a group with a clip path or a mask is drawn on a layer of its own,
//!   which each clip path's coverage and each mask's luminance (or alpha) multiplies before it
//!   is laid over what is below; a group whose clips are plain outlines and that has no mask is
//!   drawn under those clips directly, as are pattern tiles. Each layer, and each clip or mask
//!   drawn, counts the sprite's pixels against the glyph's work, so masks nested past
//!   [`MAX_LAYER_DEPTH`] layers, or clips past [`MAX_CLIP_DEPTH`], refuse the glyph.
//! - A group with filters is drawn on a layer of its own too, without the clips of enclosing
//!   groups; its filters run on that layer (module `filter`), then its clips, masks and opacity
//!   apply, and last the clips of enclosing groups. The sprite covers what the filters can
//!   paint: what the group draws grown by how far its blurs, offsets, dilations, convolutions
//!   and displacements reach, or the whole filter region when a primitive paints where its
//!   input is clear (a flood, a tile, a turbulence or a light), within the filter regions. Filters that can paint more than [`MAX_FILTER_PIXELS`] pixels, that cannot
//!   be drawn, or that would take the glyph past its work leave the group unfiltered, with a
//!   line in [`GlyphRaster::warnings`]; the glyph is not refused for them.
//! - A document whose root slices its viewport (`preserveAspectRatio` with `slice`) is clipped
//!   to the em square, the viewport it is fitted to, unless the root's `overflow` (its `style`
//!   property, else its attribute) is `visible` or `auto`.
//! - Not drawn: embedded images other than PNG and JPEG, `sbix`
//!   graphic types other than `png `, `jpg ` and `dupe`, and `EBDT` strikes.
//! - The `sbix` image's bottom left sits at the record's origin offsets from the glyph origin, a
//!   strike pixel being `units_per_em ÷ ppem` units, which is how the compiler places it. A
//!   `CBDT` image's top left sits at its bearings, in strike pixels right of and above the
//!   glyph origin.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_color::paint::{ColorLine, ColorRef, Paint as ColorPaint};
use fontcraft_color::svg_draw::{self, SvgGroup, SvgImage, SvgItem, SvgShape};
use fontcraft_color::svg_filter::SvgFilter;
use fontcraft_container::gzip::{GunzipError, MAGIC as GZIP_MAGIC, gunzip};
use fontcraft_geom::{Affine, BezPath, Point, Rect, Shape as _};
use fontcraft_model::Extend as ModelExtend;
use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapGlyph, BitmapStrikes, Origin};
use skrifa::color::{Brush, Color, ColorPainter, CompositeMode, Extend, Transform};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::{BoundingBox, Tag};
use skrifa::{FontRef, GlyphId, MetadataProvider, OutlineGlyphCollection};

use crate::{Image, MAX_SCALE, RenderError, Style, render_path};

/// Widest or tallest glyph sprite, in pixels.
pub const MAX_GLYPH_SIDE: u32 = 4_096;
/// Most pixels of one glyph sprite (each pushed layer holds this many again).
pub const MAX_GLYPH_PIXELS: u64 = 1 << 21;
/// Pixel operations any glyph may take.
pub const MIN_GLYPH_WORK: u64 = 32 << 20;
/// Pixel operations per pixel of its sprite a glyph may take, when that is more.
pub const WORK_PER_PIXEL: u64 = 32;
/// Pixel operations the first rasterisations of one frame may take together.
pub const MAX_FRAME_WORK: u64 = 16 << 20;
/// Most sprites kept.
pub const MAX_CACHE_GLYPHS: usize = 4_096;
/// Most bytes of sprites kept.
pub const MAX_CACHE_BYTES: usize = 64 << 20;
/// Deepest nesting of clips in one glyph.
pub const MAX_CLIP_DEPTH: usize = 32;
/// Deepest nesting of composite layers in one glyph.
pub const MAX_LAYER_DEPTH: usize = 8;
/// Largest image read from an `sbix` or `CBDT` record, and most pixels of the images of one `SVG ` document.
pub const MAX_BITMAP_PIXELS: u64 = 1 << 22;
/// Most outlines kept while one glyph is drawn.
const MAX_OUTLINES: usize = 4_096;
/// Most bytes of read colour sources kept: parsed SVG documents and decoded images.
pub const MAX_SOURCE_BYTES: usize = 64 << 20;
/// Most attributes read from an SVG document's root element.
const MAX_ROOT_ATTRIBUTES: usize = 64;
/// Samples per side for one sprite pixel when an image is drawn smaller than its pixels.
const MAX_BITMAP_SAMPLES: u32 = 4;
/// Most sprite pixels SVG filters may paint for one group; a group whose filters can paint more is drawn unfiltered.
pub const MAX_FILTER_PIXELS: u64 = 1 << 20;
/// Largest blur standard deviation in sprite pixels; a larger one blurs by this much.
pub const MAX_BLUR_SIGMA: f64 = 256.0;
/// Largest `feMorphology` radius in sprite pixels; a larger one counts as this.
pub const MAX_MORPHOLOGY_RADIUS: f64 = 256.0;

mod filter;

/// The tables a colour glyph is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorKind {
    Colr,
    Svg,
    Sbix,
    Cbdt,
}

/// How glyphs are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorOptions {
    /// Pixels per font unit.
    pub scale: f64,
    /// The `CPAL` palette; palette 0 when the font has no such palette.
    pub palette: u16,
    /// Straight RGBA of the text colour (palette index 65,535).
    pub text_color: [u8; 4],
}

/// One glyph drawn in colour.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRaster {
    /// Premultiplied RGBA.
    pub image: Image,
    /// Where the image's top left pixel lies, in pixels right of and below the glyph origin.
    pub left: i32,
    pub top: i32,
    pub kind: ColorKind,
    /// What of the glyph's colour drawing is left out or drawn plainer, one line for each
    /// reason.
    pub warnings: Vec<String>,
}

/// What [`ColorRenderer::glyph`] gives.
#[derive(Clone, Debug, PartialEq)]
pub enum Painted {
    /// The glyph's colour drawing.
    Color(Arc<GlyphRaster>),
    /// Not a colour glyph of this font: draw its outline.
    Plain,
    /// A colour glyph that is not drawn in colour, and why: draw its outline.
    Refused(Arc<str>),
    /// Not drawn in this frame, whose budget is spent: draw its outline and ask again.
    Deferred,
}

/// The pixel operations the first rasterisations of one frame may still take. The first glyph
/// always starts, so every frame makes progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameBudget {
    left: u64,
    started: bool,
    deferred: usize,
}

impl FrameBudget {
    pub fn new(work: u64) -> Self {
        Self { left: work, started: false, deferred: 0 }
    }

    /// A budget of [`MAX_FRAME_WORK`].
    pub fn frame() -> Self {
        Self::new(MAX_FRAME_WORK)
    }

    /// Glyphs deferred so far.
    pub fn deferred(&self) -> usize {
        self.deferred
    }

    fn may_start(&self) -> bool {
        !self.started || self.left > 0
    }

    fn spend(&mut self, work: u64) {
        self.started = true;
        self.left = self.left.saturating_sub(work);
    }
}

type CacheKey = (u16, u16, [u8; 4], u64);

struct Entry {
    painted: Painted,
    bytes: usize,
    used: u64,
}

/// The colour glyphs of one compiled font.
pub struct ColorRenderer {
    font: Arc<[u8]>,
    ids: HashMap<Arc<str>, u16>,
    kinds: HashMap<u16, Option<ColorKind>>,
    cache: HashMap<CacheKey, Entry>,
    bytes: usize,
    sources: Sources,
    tick: u64,
}

impl std::fmt::Debug for ColorRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorRenderer").field("glyphs", &self.ids.len()).field("cached", &self.cache.len()).finish_non_exhaustive()
    }
}

fn read_error(e: impl std::fmt::Display) -> RenderError {
    RenderError::Model(format!("the font cannot be read: {e}"))
}

impl ColorRenderer {
    /// A renderer for `font` (an sfnt), whose glyph names in glyph-id order are `glyph_order`
    /// (at most 65,536 are read).
    pub fn new(font: Arc<[u8]>, glyph_order: &[String]) -> Result<Self, RenderError> {
        if font.len() > crate::hinted::MAX_FONT_BYTES {
            return Err(RenderError::Model(format!("the font is larger than {} bytes", crate::hinted::MAX_FONT_BYTES)));
        }
        FontRef::new(&font).map_err(read_error)?;
        let mut ids = HashMap::with_capacity(glyph_order.len().min(usize::from(u16::MAX) + 1));
        for (gid, name) in (0u16..=u16::MAX).zip(glyph_order) {
            ids.entry(Arc::from(name.as_str())).or_insert(gid);
        }
        Ok(Self { font, ids, kinds: HashMap::new(), cache: HashMap::new(), bytes: 0, sources: Sources::default(), tick: 0 })
    }

    /// The font bytes.
    pub fn font(&self) -> &Arc<[u8]> {
        &self.font
    }

    pub fn glyph_id(&self, name: &str) -> Option<u16> {
        self.ids.get(name).copied()
    }

    /// The table glyph `gid` is drawn from; `None` when it is not a colour glyph.
    pub fn kind(&mut self, gid: u16) -> Option<ColorKind> {
        if let Some(k) = self.kinds.get(&gid) {
            return *k;
        }
        let font = self.font.clone();
        let kind = FontRef::new(&font).ok().and_then(|f| kind_of(&f, gid));
        self.kinds.insert(gid, kind);
        kind
    }

    /// Sprites kept, and their bytes.
    pub fn cached(&self) -> (usize, usize) {
        (self.cache.len(), self.bytes)
    }

    /// Glyph `gid` drawn with `options`, from the cache when it was drawn before.
    pub fn glyph(&mut self, gid: u16, options: &ColorOptions, budget: &mut FrameBudget) -> Painted {
        let key = (gid, options.palette, options.text_color, options.scale.to_bits());
        self.tick += 1;
        if let Some(e) = self.cache.get_mut(&key) {
            e.used = self.tick;
            return e.painted.clone();
        }
        let Some(kind) = self.kind(gid) else { return Painted::Plain };
        if !budget.may_start() {
            budget.deferred += 1;
            return Painted::Deferred;
        }
        let font = self.font.clone();
        let (painted, work) = match FontRef::new(&font) {
            Ok(f) => match draw(&f, gid, kind, options, &mut self.sources, self.tick) {
                Ok((raster, work)) => (Painted::Color(Arc::new(raster)), work),
                Err(Refusal { reason, work }) => (Painted::Refused(Arc::from(reason)), work),
            },
            Err(e) => (Painted::Refused(Arc::from(format!("the font cannot be read: {e}"))), 0),
        };
        budget.spend(work.max(1));
        self.insert(key, painted.clone());
        painted
    }

    fn insert(&mut self, key: CacheKey, painted: Painted) {
        let bytes = match &painted {
            Painted::Color(r) => r.image.pixels.len(),
            _ => 0,
        };
        if self.cache.len() >= MAX_CACHE_GLYPHS || self.bytes.saturating_add(bytes) > MAX_CACHE_BYTES {
            // The least recently used quarter goes, so eviction is not paid on every insert.
            let mut ticks: Vec<u64> = self.cache.values().map(|e| e.used).collect();
            ticks.sort_unstable();
            let cut = ticks.get(ticks.len() / 4).copied().unwrap_or(0);
            self.cache.retain(|_, e| e.used > cut);
            self.bytes = self.cache.values().map(|e| e.bytes).sum();
            while self.bytes.saturating_add(bytes) > MAX_CACHE_BYTES {
                let Some(oldest) = self.cache.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) else { break };
                if let Some(gone) = self.cache.remove(&oldest) {
                    self.bytes = self.bytes.saturating_sub(gone.bytes);
                }
            }
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.cache.insert(key, Entry { painted, bytes, used: self.tick });
    }
}

/// Lays `raster` over `target` (premultiplied, source over) with the glyph origin at pixel
/// corner `(x, y)`.
pub fn composite(target: &mut Image, raster: &GlyphRaster, x: i64, y: i64) {
    let (tw, th) = (i64::from(target.width), i64::from(target.height));
    let (sw, sh) = (i64::from(raster.image.width), i64::from(raster.image.height));
    let (ox, oy) = (x.saturating_add(i64::from(raster.left)), y.saturating_add(i64::from(raster.top)));
    let (x0, y0) = (ox.max(0), oy.max(0));
    let (x1, y1) = (ox.saturating_add(sw).min(tw), oy.saturating_add(sh).min(th));
    for ty in y0..y1 {
        for tx in x0..x1 {
            let from = (((ty - oy) * sw + (tx - ox)) * 4) as usize;
            let to = ((ty * tw + tx) * 4) as usize;
            let Some(&[r, g, b, a]) = raster.image.pixels.get(from..from + 4).and_then(|s| <&[u8; 4]>::try_from(s).ok()) else { continue };
            if a == 0 {
                continue;
            }
            let Some(d) = target.pixels.get_mut(to..to + 4) else { continue };
            let keep = 255 - u16::from(a);
            for (d, s) in d.iter_mut().zip([r, g, b, a]) {
                *d = (u16::from(s) + (u16::from(*d) * keep + 127) / 255).min(255) as u8;
            }
        }
    }
}

// ── which table ────────────────────────────────────────────────────────────────────────────────

fn kind_of(font: &FontRef, gid: u16) -> Option<ColorKind> {
    let id = GlyphId::new(u32::from(gid));
    if font.color_glyphs().get(id).is_some() {
        return Some(ColorKind::Colr);
    }
    if font.svg().ok().and_then(|s| s.glyph_data(id)).is_some() {
        return Some(ColorKind::Svg);
    }
    if sbix_record(font, gid, f64::MAX).is_some() {
        return Some(ColorKind::Sbix);
    }
    cbdt_glyph(font, gid, f64::MAX).map(|_| ColorKind::Cbdt)
}

struct Refusal {
    reason: String,
    work: u64,
}

impl Refusal {
    fn new(reason: impl Into<String>, work: u64) -> Self {
        Self { reason: reason.into(), work }
    }
}

type Done = (GlyphRaster, u64);

fn draw(font: &FontRef, gid: u16, kind: ColorKind, options: &ColorOptions, sources: &mut Sources, tick: u64) -> Result<Done, Refusal> {
    if !(options.scale.is_finite() && options.scale > 0.0 && options.scale <= MAX_SCALE) {
        return Err(Refusal::new("the scale must be a positive number of at most 64 pixels per unit", 0));
    }
    if kind == ColorKind::Colr {
        return draw_colr(font, gid, options);
    }
    let ppem = options.scale * units_per_em(font);
    let Some(strike) = strike_of(font, gid, kind, ppem) else { return Err(Refusal::new("the glyph has no colour image", 0)) };
    let (source, read) = sources.get((gid, kind, strike), tick, || prepare(font, gid, kind, ppem));
    let source = source.map_err(|e| Refusal::new(e.to_string(), read))?;
    let drawn = match kind {
        ColorKind::Svg => draw_svg(font, &source, options),
        _ => draw_bitmaps(font, &source, kind, options),
    };
    match drawn {
        Ok((raster, work)) => Ok((raster, work.saturating_add(read))),
        Err(Refusal { reason, work }) => Err(Refusal { reason, work: work.saturating_add(read) }),
    }
}

// ── geometry ───────────────────────────────────────────────────────────────────────────────────

fn affine(t: Transform) -> Affine {
    Affine::new([t.xx, t.yx, t.xy, t.yy, t.dx, t.dy].map(f64::from))
}

fn finite_affine(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite())
}

fn finite_rect(r: Rect) -> bool {
    r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()
}

fn has_area(r: Rect) -> bool {
    finite_rect(r) && r.x1 > r.x0 && r.y1 > r.y0
}

fn rect_of(b: BoundingBox<f32>) -> Rect {
    Rect::new(f64::from(b.x_min), f64::from(b.y_min), f64::from(b.x_max), f64::from(b.y_max))
}

/// A box of pixels: `x0`, `y0` included, `x1`, `y1` not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PixBox {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl PixBox {
    const EMPTY: PixBox = PixBox { x0: 0, y0: 0, x1: 0, y1: 0 };

    /// The pixels `rect` covers under `t`, within `width × height`.
    fn of(rect: Rect, t: Affine, width: u32, height: u32) -> PixBox {
        let b = t.transform_rect_bbox(rect);
        if !finite_rect(b) {
            return Self::EMPTY;
        }
        let x0 = b.x0.floor().clamp(0.0, f64::from(width)) as u32;
        let y0 = b.y0.floor().clamp(0.0, f64::from(height)) as u32;
        let x1 = b.x1.ceil().clamp(0.0, f64::from(width)) as u32;
        let y1 = b.y1.ceil().clamp(0.0, f64::from(height)) as u32;
        PixBox { x0, y0, x1: x1.max(x0), y1: y1.max(y0) }
    }

    fn is_empty(self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    fn width(self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    fn area(self) -> u64 {
        u64::from(self.width()) * u64::from(self.y1.saturating_sub(self.y0))
    }

    fn intersect(self, o: PixBox) -> PixBox {
        let b = PixBox { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        if b.is_empty() { Self::EMPTY } else { b }
    }
}

/// The sprite for `rect` (font units, y up) at `scale`: its size, the affine from font units to
/// its pixels, and its top left relative to the glyph origin.
struct Sprite {
    width: u32,
    height: u32,
    /// Sprite pixels per font unit.
    scale: f64,
    base: Affine,
    left: i32,
    top: i32,
}

fn sprite(rect: Rect, scale: f64) -> Result<Sprite, Refusal> {
    if !has_area(rect) {
        return Err(Refusal::new("the colour glyph draws nothing", 0));
    }
    let left = (rect.x0 * scale).floor() - 1.0;
    let right = (rect.x1 * scale).ceil() + 1.0;
    let top = (-rect.y1 * scale).floor() - 1.0;
    let bottom = (-rect.y0 * scale).ceil() + 1.0;
    let (w, h) = (right - left, bottom - top);
    let side = f64::from(MAX_GLYPH_SIDE);
    let range = f64::from(i32::MAX / 2);
    if !(w.is_finite() && h.is_finite() && left.abs() < range && top.abs() < range) || w > side || h > side || w * h > MAX_GLYPH_PIXELS as f64 {
        return Err(Refusal::new(
            format!("the colour glyph would be {w} × {h} pixels; at most {MAX_GLYPH_SIDE} on a side and {MAX_GLYPH_PIXELS} in all"),
            0,
        ));
    }
    let base = Affine::translate((-left, -top)) * Affine::scale_non_uniform(scale, -scale);
    Ok(Sprite { width: w as u32, height: h as u32, scale, base, left: left as i32, top: top as i32 })
}

fn budget_for(sprite: &Sprite) -> u64 {
    MIN_GLYPH_WORK.max(WORK_PER_PIXEL.saturating_mul(u64::from(sprite.width) * u64::from(sprite.height)))
}

// ── outlines ───────────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct PathPen {
    path: BezPath,
    open: bool,
}

impl PathPen {
    fn point(x: f32, y: f32) -> Option<Point> {
        let p = Point::new(f64::from(x), f64::from(y));
        (p.x.is_finite() && p.y.is_finite()).then_some(p)
    }
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        if let Some(p) = Self::point(x, y) {
            self.path.move_to(p);
            self.open = true;
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if let (true, Some(p)) = (self.open, Self::point(x, y)) {
            self.path.line_to(p);
        }
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        if let (true, Some(a), Some(p)) = (self.open, Self::point(cx0, cy0), Self::point(x, y)) {
            self.path.quad_to(a, p);
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if let (true, Some(a), Some(b), Some(p)) = (self.open, Self::point(cx0, cy0), Self::point(cx1, cy1), Self::point(x, y)) {
            self.path.curve_to(a, b, p);
        }
    }

    fn close(&mut self) {
        if self.open {
            self.path.close_path();
        }
        self.open = false;
    }
}

/// The outlines of the glyphs a colour glyph clips by, read once per drawing.
struct Outlines<'a> {
    /// `None` for an SVG file drawn outside a font, which has no glyphs.
    glyphs: Option<OutlineGlyphCollection<'a>>,
    kept: HashMap<u32, Arc<BezPath>>,
}

impl<'a> Outlines<'a> {
    fn new(font: &FontRef<'a>) -> Self {
        Self { glyphs: Some(font.outline_glyphs()), kept: HashMap::new() }
    }

    fn none() -> Self {
        Self { glyphs: None, kept: HashMap::new() }
    }

    fn get(&mut self, gid: GlyphId) -> Arc<BezPath> {
        if let Some(p) = self.kept.get(&gid.to_u32()) {
            return p.clone();
        }
        let mut pen = PathPen::default();
        if let Some(glyph) = self.glyphs.as_ref().and_then(|g| g.get(gid))
            && glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).is_err()
        {
            pen.path = BezPath::new();
        }
        pen.close();
        let path = Arc::new(pen.path);
        if self.kept.len() < MAX_OUTLINES {
            self.kept.insert(gid.to_u32(), path.clone());
        }
        path
    }
}

// ── fills ──────────────────────────────────────────────────────────────────────────────────────

/// Gradient stops: offsets in order, straight RGBA from 0 to 1.
#[derive(Clone, Debug)]
struct Stops {
    at: Vec<(f64, [f32; 4])>,
    extend: ModelExtend,
}

/// A fill evaluated per pixel, in the coordinates of its transform.
#[derive(Clone, Debug)]
enum Leaf {
    Solid([f32; 4]),
    /// `t` is `(p − p0) · v`.
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
    /// Angles in degrees, clockwise from the positive x axis of a y-up space, as skrifa gives them.
    Sweep {
        center: Point,
        start: f64,
        end: f64,
        stops: Stops,
    },
}

fn finite_point(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn linear(p0: Point, p1: Point, stops: Stops) -> Option<Leaf> {
    if !(finite_point(p0) && finite_point(p1)) {
        return None;
    }
    let (vx, vy) = (p1.x - p0.x, p1.y - p0.y);
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return stops.at.last().map(|(_, c)| Leaf::Solid(*c));
    }
    Some(Leaf::Linear { p0, v: (vx / len2, vy / len2), stops })
}

fn extend_t(t: f64, mode: ModelExtend) -> f64 {
    match mode {
        ModelExtend::Pad => t.clamp(0.0, 1.0),
        ModelExtend::Repeat => t - t.floor(),
        ModelExtend::Reflect => {
            let m = t.rem_euclid(2.0);
            if m > 1.0 { 2.0 - m } else { m }
        }
    }
}

fn sample(stops: &Stops, t: f64) -> [f32; 4] {
    let t = if t.is_finite() { extend_t(t, stops.extend) } else { 0.0 };
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

impl Leaf {
    /// The colour at `p`; `None` where a radial gradient paints nothing.
    fn eval(&self, p: Point) -> Option<[f32; 4]> {
        match self {
            Leaf::Solid(c) => Some(*c),
            Leaf::Linear { p0, v, stops } => Some(sample(stops, (p.x - p0.x) * v.0 + (p.y - p0.y) * v.1)),
            Leaf::Radial { c0, r0, c1, r1, stops } => {
                // The largest t with |p − c(t)| = r(t) and r(t) ≥ 0, the circle moving from the
                // start circle to the end circle.
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
                let ccw = (p.y - center.y).atan2(p.x - center.x).to_degrees().rem_euclid(360.0);
                let clockwise = (360.0 - ccw).rem_euclid(360.0);
                Some(sample(stops, (clockwise - start) / span))
            }
        }
    }
}

/// Lays straight `color` at `coverage` over the premultiplied pixel at byte `at`.
fn blend(data: &mut [u8], at: usize, color: [f32; 4], coverage: f32) {
    let [r, g, b, a] = color;
    let alpha = (a * coverage).clamp(0.0, 1.0);
    if alpha.is_nan() || alpha <= 0.0 {
        return;
    }
    let Some(px) = data.get_mut(at..at + 4) else { return };
    let keep = 1.0 - alpha;
    for (d, s) in px.iter_mut().zip([r * alpha, g * alpha, b * alpha, alpha]) {
        let v = s * 255.0 + f32::from(*d) * keep;
        *d = v.round().clamp(0.0, 255.0) as u8;
    }
}

/// Lays premultiplied `color` (channels from 0 to 1) over the premultiplied pixel at byte `at`.
fn blend_premultiplied(data: &mut [u8], at: usize, color: [f64; 4]) {
    let [r, g, b, a] = color.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    if a <= 0.0 {
        return;
    }
    let Some(px) = data.get_mut(at..at + 4) else { return };
    let keep = 1.0 - a;
    for (d, s) in px.iter_mut().zip([r, g, b, a]) {
        *d = (s * 255.0 + f64::from(*d) * keep).round().clamp(0.0, 255.0) as u8;
    }
}

/// The unit square an image is placed from.
fn unit() -> Rect {
    Rect::new(0.0, 0.0, 1.0, 1.0)
}

// ── composite modes ────────────────────────────────────────────────────────────────────────────

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 && l - n > 1e-6 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 && x - l > 1e-6 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut order = [0usize, 1, 2];
    order.sort_by(|a, b| c.get(*a).unwrap_or(&0.0).total_cmp(c.get(*b).unwrap_or(&0.0)));
    let [lo, mid, hi] = order;
    let (cl, cm, ch) = (c.get(lo).copied().unwrap_or(0.0), c.get(mid).copied().unwrap_or(0.0), c.get(hi).copied().unwrap_or(0.0));
    let mut out = [0.0f32; 3];
    if ch > cl {
        if let Some(v) = out.get_mut(mid) {
            *v = (cm - cl) * s / (ch - cl);
        }
        if let Some(v) = out.get_mut(hi) {
            *v = s;
        }
    }
    out
}

/// The separable blend function of `mode` for backdrop `cb` and source `cs` (unpremultiplied).
fn separable(mode: CompositeMode, cb: f32, cs: f32) -> f32 {
    let multiply = |b: f32, s: f32| b * s;
    let screen = |b: f32, s: f32| b + s - b * s;
    let hard_light = |b: f32, s: f32| if s <= 0.5 { multiply(b, 2.0 * s) } else { screen(b, 2.0 * s - 1.0) };
    match mode {
        CompositeMode::Multiply => multiply(cb, cs),
        CompositeMode::Screen => screen(cb, cs),
        CompositeMode::Overlay => hard_light(cs, cb),
        CompositeMode::Darken => cb.min(cs),
        CompositeMode::Lighten => cb.max(cs),
        CompositeMode::ColorDodge => {
            if cb <= 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        CompositeMode::ColorBurn => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        CompositeMode::HardLight => hard_light(cb, cs),
        CompositeMode::SoftLight => {
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
        CompositeMode::Difference => (cb - cs).abs(),
        CompositeMode::Exclusion => cb + cs - 2.0 * cb * cs,
        _ => cs,
    }
}

/// `s` (premultiplied, 0 to 1) composited onto `d` by `mode`.
fn composite_px(mode: CompositeMode, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    let (sa, da) = (s[3], d[3]);
    let pd = |fs: f32, fd: f32| [0, 1, 2, 3].map(|i| s.get(i).unwrap_or(&0.0) * fs + d.get(i).unwrap_or(&0.0) * fd);
    match mode {
        CompositeMode::Clear => [0.0; 4],
        CompositeMode::Src => s,
        CompositeMode::Dest => d,
        CompositeMode::SrcOver | CompositeMode::Unknown => pd(1.0, 1.0 - sa),
        CompositeMode::DestOver => pd(1.0 - da, 1.0),
        CompositeMode::SrcIn => pd(da, 0.0),
        CompositeMode::DestIn => pd(0.0, sa),
        CompositeMode::SrcOut => pd(1.0 - da, 0.0),
        CompositeMode::DestOut => pd(0.0, 1.0 - sa),
        CompositeMode::SrcAtop => pd(da, 1.0 - sa),
        CompositeMode::DestAtop => pd(1.0 - da, sa),
        CompositeMode::Xor => pd(1.0 - da, 1.0 - sa),
        CompositeMode::Plus => pd(1.0, 1.0).map(|v| v.min(1.0)),
        _ => {
            let un = |p: [f32; 4], a: f32| -> [f32; 3] { if a > 0.0 { [p[0] / a, p[1] / a, p[2] / a] } else { [0.0; 3] } };
            let (cs, cb) = (un(s, sa), un(d, da));
            let mixed: [f32; 3] = match mode {
                CompositeMode::HslHue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
                CompositeMode::HslSaturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
                CompositeMode::HslColor => set_lum(cs, lum(cb)),
                CompositeMode::HslLuminosity => set_lum(cb, lum(cs)),
                _ => [0, 1, 2].map(|i| separable(mode, cb.get(i).copied().unwrap_or(0.0), cs.get(i).copied().unwrap_or(0.0))),
            };
            let a = sa + da - sa * da;
            let c = [0, 1, 2].map(|i| {
                let (sc, dc) = (s.get(i).copied().unwrap_or(0.0), d.get(i).copied().unwrap_or(0.0));
                sc * (1.0 - da) + dc * (1.0 - sa) + sa * da * mixed.get(i).copied().unwrap_or(0.0)
            });
            [c[0], c[1], c[2], a]
        }
    }
}

fn composite_layer(top: &[u8], below: &mut [u8], mode: CompositeMode) {
    let to_f = |p: &[u8; 4]| -> [f32; 4] { p.map(|c| f32::from(c) / 255.0) };
    for (s, d) in top.as_chunks::<4>().0.iter().zip(below.as_chunks_mut::<4>().0) {
        if mode == CompositeMode::SrcOver && s[3] == 0 {
            continue;
        }
        let out = composite_px(mode, to_f(s), to_f(d));
        for (d, v) in d.iter_mut().zip(out) {
            *d = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

// ── the raster ─────────────────────────────────────────────────────────────────────────────────

const MASK: Style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };

/// A clip: coverage from 0 to 255 over its box, rows top to bottom.
struct Clip {
    bbox: PixBox,
    mask: Vec<u8>,
}

impl Clip {
    fn empty() -> Self {
        Self { bbox: PixBox::EMPTY, mask: Vec::new() }
    }

    fn coverage(&self, x: u32, y: u32) -> u8 {
        if x < self.bbox.x0 || y < self.bbox.y0 || x >= self.bbox.x1 || y >= self.bbox.y1 {
            return 0;
        }
        let i = (y - self.bbox.y0) as usize * self.bbox.width() as usize + (x - self.bbox.x0) as usize;
        self.mask.get(i).copied().unwrap_or(0)
    }
}

/// One glyph being drawn into a sprite.
struct Raster<'a, 'f> {
    width: u32,
    height: u32,
    /// Sprite pixels per font unit.
    scale: f64,
    base: Affine,
    transforms: Vec<Affine>,
    clips: Vec<Clip>,
    /// Premultiplied RGBA; the first is the sprite, the others pushed composite layers.
    layers: Vec<Vec<u8>>,
    outlines: &'a mut Outlines<'f>,
    colors: &'a [Color],
    text: [f32; 4],
    work: u64,
    budget: u64,
    refused: Option<String>,
    warnings: Vec<String>,
}

impl<'a, 'f> Raster<'a, 'f> {
    fn new(sprite: &Sprite, outlines: &'a mut Outlines<'f>, colors: &'a [Color], text: [u8; 4]) -> Self {
        let pixels = sprite.width as usize * sprite.height as usize * 4;
        Raster {
            width: sprite.width,
            height: sprite.height,
            scale: sprite.scale,
            base: sprite.base,
            transforms: Vec::new(),
            clips: Vec::new(),
            layers: vec![vec![0; pixels]],
            outlines,
            colors,
            text: text.map(|v| f32::from(v) / 255.0),
            work: 0,
            budget: budget_for(sprite),
            refused: None,
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, warning: String) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn full(&self) -> PixBox {
        PixBox { x0: 0, y0: 0, x1: self.width, y1: self.height }
    }

    fn area(&self) -> u64 {
        self.full().area()
    }

    fn ctm(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(self.base)
    }

    fn refuse(&mut self, reason: String) {
        self.refused.get_or_insert(reason);
    }

    /// Counts `n` pixel operations; false (and the glyph refused) past the budget.
    fn spend(&mut self, n: u64) -> bool {
        self.work = self.work.saturating_add(n);
        if self.work > self.budget {
            self.refuse(format!("the colour glyph takes more than {} pixel operations to draw", self.budget));
            return false;
        }
        true
    }

    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let base = if index == u16::MAX {
            self.text
        } else {
            self.colors.get(usize::from(index)).map_or(self.text, |c| [c.red(), c.green(), c.blue(), c.alpha()].map(|v| f32::from(v) / 255.0))
        };
        let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
        [base[0], base[1], base[2], base[3] * alpha]
    }

    fn stops(&self, stops: &[skrifa::color::ColorStop], extend: Extend) -> Option<Stops> {
        let mut at: Vec<(f64, [f32; 4])> =
            stops.iter().filter(|s| s.offset.is_finite()).map(|s| (f64::from(s.offset), self.color(s.palette_index, s.alpha))).collect();
        at.sort_by(|a, b| a.0.total_cmp(&b.0));
        let extend = match extend {
            Extend::Repeat => ModelExtend::Repeat,
            Extend::Reflect => ModelExtend::Reflect,
            _ => ModelExtend::Pad,
        };
        (!at.is_empty()).then_some(Stops { at, extend })
    }

    fn leaf(&self, brush: &Brush) -> Option<Leaf> {
        let pt = |p: skrifa::raw::types::Point<f32>| Point::new(f64::from(p.x), f64::from(p.y));
        match brush {
            Brush::Solid { palette_index, alpha } => Some(Leaf::Solid(self.color(*palette_index, *alpha))),
            Brush::LinearGradient { p0, p1, color_stops, extend } => linear(pt(*p0), pt(*p1), self.stops(color_stops, *extend)?),
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                let (c0, c1, r0, r1) = (pt(*c0), pt(*c1), f64::from(*r0), f64::from(*r1));
                let ok = finite_point(c0) && finite_point(c1) && r0.is_finite() && r1.is_finite();
                ok.then(|| self.stops(color_stops, *extend)).flatten().map(|stops| Leaf::Radial { c0, r0, c1, r1, stops })
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                let (center, start, end) = (pt(*c0), f64::from(*start_angle), f64::from(*end_angle));
                let ok = finite_point(center) && start.is_finite() && end.is_finite();
                ok.then(|| self.stops(color_stops, *extend)).flatten().map(|stops| Leaf::Sweep { center, start, end, stops })
            }
        }
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.push_clip_union(&[path]);
    }

    /// Pushes the clip covering the union of `paths`, within the current clip.
    fn push_clip_union(&mut self, paths: &[&BezPath]) {
        if self.clips.len() >= MAX_CLIP_DEPTH {
            self.refuse(format!("the colour glyph nests clips more than {MAX_CLIP_DEPTH} deep"));
        }
        let paths: Vec<&BezPath> = paths.iter().copied().filter(|p| !p.elements().is_empty()).collect();
        if self.refused.is_some() || paths.is_empty() {
            self.clips.push(Clip::empty());
            return;
        }
        let ctm = self.ctm();
        let (width, height) = (self.width, self.height);
        let parent = self.clips.last().map_or(self.full(), |c| c.bbox);
        let boxes: Vec<PixBox> = paths.iter().map(|p| PixBox::of(p.bounding_box(), ctm, width, height).intersect(parent)).collect();
        let bbox = boxes.iter().filter(|b| !b.is_empty()).fold(PixBox::EMPTY, |a, b| {
            if a.is_empty() { *b } else { PixBox { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) } }
        });
        let merge = if paths.len() > 1 { bbox.area() } else { 0 };
        let cost = boxes.iter().fold(merge, |n, b| n.saturating_add(b.area().saturating_mul(2)));
        if bbox.is_empty() || !self.spend(cost) {
            self.clips.push(Clip::empty());
            return;
        }
        let bw = bbox.width() as usize;
        let mut mask = vec![0u8; bbox.area() as usize];
        for (path, pb) in paths.iter().zip(&boxes) {
            if pb.is_empty() {
                continue;
            }
            let (pw, ph) = (pb.width(), pb.y1 - pb.y0);
            let shift = Affine::translate((-f64::from(pb.x0), -f64::from(pb.y0))) * ctm;
            let Ok(image) = render_path(path, shift, pw, ph, &MASK) else { continue };
            for (i, p) in image.pixels.as_chunks::<4>().0.iter().enumerate() {
                let (x, y) = ((pb.x0 - bbox.x0) as usize + i % pw as usize, (pb.y0 - bbox.y0) as usize + i / pw as usize);
                if let Some(m) = mask.get_mut(y * bw + x) {
                    // The union of coverages, as paths laid over each other.
                    let (a, b) = (u16::from(*m), u16::from(p[3]));
                    *m = (a + b - (a * b + 127) / 255) as u8;
                }
            }
        }
        if let Some(outer) = self.clips.last() {
            for (i, m) in mask.iter_mut().enumerate() {
                let (x, y) = (bbox.x0 + (i % bw) as u32, bbox.y0 + (i / bw) as u32);
                *m = ((u16::from(*m) * u16::from(outer.coverage(x, y)) + 127) / 255) as u8;
            }
        }
        self.clips.push(Clip { bbox, mask });
    }

    /// Draws SVG items in paint order over the current layer, within the current clip.
    fn draw_items(&mut self, items: &[Drawable]) {
        for item in items {
            if self.refused.is_some() {
                return;
            }
            match item {
                Drawable::Shape(shape) => {
                    let Some(leaf) = svg_leaf(&shape.paint) else { continue };
                    self.push_clip_path(&shape.outline);
                    self.fill_leaf(&leaf);
                    self.clips.pop();
                }
                Drawable::Image { bitmap, place, opacity } => self.draw_bitmap(bitmap, *place, *opacity),
                Drawable::Group(group) => self.draw_group(group),
            }
        }
    }

    /// A group whose clips are plain outlines and that has no mask is drawn under those clips
    /// directly; any other is drawn into a layer of its own, multiplied by the coverage of each
    /// clip and mask, and laid over the layer below.
    fn draw_group(&mut self, group: &DrawGroup) {
        let plain = group.filters.is_empty()
            && group.opacity >= 1.0
            && group.masks.is_empty()
            && group.clips.iter().all(|c| c.iter().all(|d| matches!(d, Drawable::Shape(_))));
        if plain {
            for clip in &group.clips {
                let paths: Vec<&BezPath> = clip.iter().filter_map(|d| if let Drawable::Shape(s) = d { Some(&s.outline) } else { None }).collect();
                self.push_clip_union(&paths);
            }
            self.draw_items(&group.items);
            for _ in &group.clips {
                self.clips.pop();
            }
            return;
        }
        let limit = if group.filters.is_empty() {
            None
        } else {
            match filtered_extent(&group.filters, extent(&group.items, self.scale), self.scale) {
                // The filters leave nothing.
                None => return,
                Some(r) if filter_fits(r, self.scale) => Some(PixBox::of(r, self.ctm(), self.width, self.height)),
                Some(r) => {
                    let pixels = r.area() * self.scale * self.scale;
                    self.warn(format!(
                        "an SVG filter that can paint {pixels:.0} pixels is drawn unfiltered; at most {MAX_FILTER_PIXELS} are filtered"
                    ));
                    None
                }
            }
        };
        if !self.open_layer() {
            return;
        }
        // A filter reads what its group draws before the clips of enclosing groups, which then
        // apply to its result.
        let outer = limit.map(|_| std::mem::take(&mut self.clips));
        self.draw_items(&group.items);
        if let Some(limit) = limit {
            self.apply_filters(&group.filters, limit);
        }
        for clip in &group.clips {
            if let Some(coverage) = self.coverage(clip, None, false) {
                self.mask_top(&coverage);
            }
        }
        for mask in &group.masks {
            if let Some(coverage) = self.coverage(&mask.items, Some(&mask.region), mask.luminance) {
                self.mask_top(&coverage);
            }
        }
        if group.opacity < 1.0 {
            self.fade_top(group.opacity);
        }
        if let Some(outer) = outer {
            self.clips = outer;
            self.clip_top();
        }
        self.close_layer();
    }

    /// Runs `filters` one after another on the top layer within `limit`, or, with a warning,
    /// none of them when one cannot be placed, has too large a region, or takes more work than
    /// the glyph has left.
    fn apply_filters(&mut self, filters: &[SvgFilter], limit: PixBox) {
        if self.refused.is_some() {
            return;
        }
        let mut plans = Vec::with_capacity(filters.len());
        let mut cost = 0u64;
        for f in filters {
            let to_pixels = self.ctm() * f.to_font;
            if !finite_affine(to_pixels) {
                return self.warn("an SVG filter cannot be placed on the glyph and is drawn unfiltered".into());
            }
            match filter::plan(f, to_pixels, self.width, self.height, limit) {
                Ok(plan) => {
                    cost = cost.saturating_add(plan.cost);
                    plans.push(plan);
                }
                Err(warning) => return self.warn(warning),
            }
        }
        if self.work.saturating_add(cost) > self.budget {
            return self.warn("an SVG filter takes more work than the glyph may take and is drawn unfiltered".into());
        }
        if !self.spend(cost) {
            return;
        }
        let width = self.width;
        if let Some(layer) = self.layers.last_mut() {
            for (f, plan) in filters.iter().zip(&plans) {
                filter::run(f, plan, layer, width);
            }
        }
    }

    /// Multiplies the top layer by the coverage of the current clip.
    fn clip_top(&mut self) {
        let area = self.area();
        if self.clips.is_empty() || self.refused.is_some() || !self.spend(area) {
            return;
        }
        let width = (self.width as usize).max(1);
        let (Some(clip), Some(top)) = (self.clips.last(), self.layers.last_mut()) else { return };
        for (i, px) in top.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let c = clip.coverage((i % width) as u32, (i / width) as u32);
            if c < 255 {
                for v in px.iter_mut() {
                    *v = ((u16::from(*v) * u16::from(c) + 127) / 255) as u8;
                }
            }
        }
    }

    /// Multiplies the top layer by `opacity`.
    fn fade_top(&mut self, opacity: f32) {
        let area = self.area();
        if self.refused.is_some() || !self.spend(area) {
            return;
        }
        let k = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.0 };
        let Some(top) = self.layers.last_mut() else { return };
        for v in top.iter_mut() {
            *v = (f32::from(*v) * k).round() as u8;
        }
    }

    /// Pushes a clear layer the size of the sprite; false (nothing pushed) when the glyph is
    /// refused, nests layers too deep or runs out of work.
    fn open_layer(&mut self) -> bool {
        if self.layers.len() > MAX_LAYER_DEPTH {
            self.refuse(format!("the colour glyph nests layers more than {MAX_LAYER_DEPTH} deep"));
        }
        let area = self.area();
        if self.refused.is_some() || !self.spend(area) {
            return false;
        }
        self.layers.push(vec![0; self.width as usize * self.height as usize * 4]);
        true
    }

    /// Lays the top layer over the one below it.
    fn close_layer(&mut self) {
        if self.layers.len() <= 1 {
            return;
        }
        let Some(top) = self.layers.pop() else { return };
        let area = self.area();
        if self.refused.is_some() || !self.spend(area) {
            return;
        }
        if let Some(below) = self.layers.last_mut() {
            composite_layer(&top, below, CompositeMode::SrcOver);
        }
    }

    /// The coverage of each sprite pixel by `items` drawn on a clear layer, outside the current
    /// clips and inside `region`: their luminance (from premultiplied colour, as resvg takes it)
    /// or their alpha.
    fn coverage(&mut self, items: &[Drawable], region: Option<&BezPath>, luminance: bool) -> Option<Vec<u8>> {
        let saved = std::mem::take(&mut self.clips);
        if let Some(region) = region {
            self.push_clip_path(region);
        }
        let opened = self.open_layer();
        if opened {
            self.draw_items(items);
        }
        let layer = if opened { self.layers.pop() } else { None };
        self.clips = saved;
        let area = self.area();
        if self.refused.is_some() || !self.spend(area) {
            return None;
        }
        let pixels = layer?;
        Some(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&[r, g, b, a]| {
                    if luminance {
                        (0.2125 * f32::from(r) + 0.7154 * f32::from(g) + 0.0721 * f32::from(b)).round().clamp(0.0, 255.0) as u8
                    } else {
                        a
                    }
                })
                .collect(),
        )
    }

    /// Multiplies the top layer by `coverage`, one value per pixel.
    fn mask_top(&mut self, coverage: &[u8]) {
        let area = self.area();
        if self.refused.is_some() || !self.spend(area) {
            return;
        }
        let Some(top) = self.layers.last_mut() else { return };
        for (px, c) in top.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
            for v in px.iter_mut() {
                *v = ((u16::from(*v) * u16::from(*c) + 127) / 255) as u8;
            }
        }
    }

    fn fill_leaf(&mut self, leaf: &Leaf) {
        if self.refused.is_some() {
            return;
        }
        let area = self.clips.last().map_or(self.full(), |c| c.bbox);
        if area.is_empty() || !self.spend(area.area()) {
            return;
        }
        let inverse = self.ctm().inverse();
        if !finite_affine(inverse) {
            return;
        }
        let width = self.width as usize;
        let clip = self.clips.last();
        let Some(layer) = self.layers.last_mut() else { return };
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let coverage = clip.map_or(255, |c| c.coverage(x, y));
                if coverage == 0 {
                    continue;
                }
                let Some(color) = leaf.eval(inverse * Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5)) else { continue };
                blend(layer, (y as usize * width + x as usize) * 4, color, f32::from(coverage) / 255.0);
            }
        }
    }

    /// Lays `bitmap`, placed by `place` from its unit square (y down from its top left) to font
    /// units, over the current layer within the current clip. Where the image is drawn smaller
    /// than its pixels, each sprite pixel averages up to [`MAX_BITMAP_SAMPLES`]² samples.
    fn draw_bitmap(&mut self, bitmap: &Bitmap, place: Affine, opacity: f32) {
        if self.refused.is_some() || bitmap.width == 0 || bitmap.height == 0 {
            return;
        }
        let (bw, bh) = (f64::from(bitmap.width), f64::from(bitmap.height));
        let to_pixels = self.ctm() * place * Affine::scale_non_uniform(1.0 / bw, 1.0 / bh);
        let to_image = to_pixels.inverse();
        if !finite_affine(to_pixels) || !finite_affine(to_image) {
            return;
        }
        let parent = self.clips.last().map_or(self.full(), |c| c.bbox);
        let area = PixBox::of(Rect::new(0.0, 0.0, bw, bh), to_pixels, self.width, self.height).intersect(parent);
        if area.is_empty() {
            return;
        }
        // Sprite pixels per image pixel, along the image's shorter side.
        let [a, b, c, d, _, _] = to_pixels.as_coeffs();
        let pixel = a.hypot(b).min(c.hypot(d));
        let n = if pixel >= 1.0 { 1 } else { ((1.0 / pixel).ceil().min(f64::from(MAX_BITMAP_SAMPLES)) as u32).max(1) };
        if !self.spend(area.area().saturating_mul(u64::from(n * n))) {
            return;
        }
        let tint = bitmap.mask.then_some(self.text);
        let opacity = if opacity.is_finite() { f64::from(opacity.clamp(0.0, 1.0)) } else { 0.0 };
        let (iw, ih) = (bitmap.width as usize, bitmap.height as usize);
        let width = self.width as usize;
        let k = 1.0 / f64::from(n * n);
        let step = 1.0 / f64::from(n);
        let clip = self.clips.last();
        let Some(layer) = self.layers.last_mut() else { return };
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let coverage = clip.map_or(255, |c| c.coverage(x, y));
                if coverage == 0 {
                    continue;
                }
                let mut sum = [0.0f64; 4];
                for j in 0..n {
                    for i in 0..n {
                        let q = to_image * Point::new(f64::from(x) + (f64::from(i) + 0.5) * step, f64::from(y) + (f64::from(j) + 0.5) * step);
                        if !(q.x >= 0.0 && q.y >= 0.0 && q.x < bw && q.y < bh) {
                            continue;
                        }
                        let from = ((q.y as usize).min(ih - 1) * iw + (q.x as usize).min(iw - 1)) * 4;
                        let Some(&[r, g, b, a]) = bitmap.pixels.get(from..from + 4).and_then(|s| <&[u8; 4]>::try_from(s).ok()) else { continue };
                        let a = f64::from(a) / 255.0;
                        let [r, g, b, a] = match tint {
                            Some([tr, tg, tb, ta]) => [f64::from(tr), f64::from(tg), f64::from(tb), f64::from(ta) * a],
                            None => [f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0, a],
                        };
                        for (s, v) in sum.iter_mut().zip([r * a, g * a, b * a, a]) {
                            *s += v * k;
                        }
                    }
                }
                if sum[3] <= 0.0 {
                    continue;
                }
                let scale = opacity * f64::from(coverage) / 255.0;
                blend_premultiplied(layer, (y as usize * width + x as usize) * 4, sum.map(|v| v * scale));
            }
        }
    }

    fn finish(mut self, left: i32, top: i32, kind: ColorKind) -> Result<Done, Refusal> {
        if let Some(reason) = self.refused.take() {
            return Err(Refusal::new(reason, self.work));
        }
        let pixels = self.layers.into_iter().next().unwrap_or_default();
        let image = Image::from_rgba_premultiplied(self.width, self.height, pixels).map_err(|e| Refusal::new(e.to_string(), self.work))?;
        Ok((GlyphRaster { image, left, top, kind, warnings: self.warnings }, self.work))
    }
}

impl ColorPainter for Raster<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        let next = self.ctm() * affine(transform);
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let path = self.outlines.get(glyph_id);
        self.push_clip_path(&path);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let path = rect_of(clip_box).to_path(0.1);
        self.push_clip_path(&path);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        if let Some(leaf) = self.leaf(&brush) {
            self.fill_leaf(&leaf);
        }
    }

    fn push_layer(&mut self, _composite_mode: CompositeMode) {
        if self.layers.len() > MAX_LAYER_DEPTH {
            self.refuse(format!("the colour glyph nests layers more than {MAX_LAYER_DEPTH} deep"));
        }
        let area = self.area();
        if self.refused.is_some() || !self.spend(area) {
            self.layers.push(Vec::new());
            return;
        }
        self.layers.push(vec![0; self.width as usize * self.height as usize * 4]);
    }

    fn pop_layer_with_mode(&mut self, composite_mode: CompositeMode) {
        if self.layers.len() <= 1 {
            return;
        }
        let Some(top) = self.layers.pop() else { return };
        let area = self.area();
        if self.refused.is_some() || top.is_empty() || !self.spend(area) {
            return;
        }
        if let Some(below) = self.layers.last_mut() {
            composite_layer(&top, below, composite_mode);
        }
    }
}

/// The area a colour glyph paints, in font units: the union of the clip boxes its fills are
/// made in.
struct Bounds<'a, 'f> {
    outlines: &'a mut Outlines<'f>,
    transforms: Vec<Affine>,
    clips: Vec<Rect>,
    total: Option<Rect>,
    unbounded: bool,
}

impl<'a, 'f> Bounds<'a, 'f> {
    fn new(outlines: &'a mut Outlines<'f>) -> Self {
        Self { outlines, transforms: Vec::new(), clips: Vec::new(), total: None, unbounded: false }
    }

    fn ctm(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(Affine::IDENTITY)
    }

    fn push_rect(&mut self, r: Rect) {
        let r = match self.clips.last() {
            Some(p) => p.intersect(r),
            None => r,
        };
        self.clips.push(r);
    }
}

impl ColorPainter for Bounds<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        let next = self.ctm() * affine(transform);
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let path = self.outlines.get(glyph_id);
        let r = if path.elements().is_empty() { Rect::ZERO } else { self.ctm().transform_rect_bbox(path.bounding_box()) };
        self.push_rect(r);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let r = self.ctm().transform_rect_bbox(rect_of(clip_box));
        self.push_rect(r);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, _brush: Brush<'_>) {
        match self.clips.last() {
            Some(r) if has_area(*r) => self.total = Some(self.total.map_or(*r, |t| t.union(*r))),
            Some(_) => {}
            None => self.unbounded = true,
        }
    }

    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}

/// The glyph's em box (advance by descender to ascender), for fills no clip bounds.
fn em_box(font: &FontRef, gid: u16) -> Rect {
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let advance = font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(GlyphId::new(u32::from(gid))).unwrap_or(0.0);
    let upm = f64::from(metrics.units_per_em.max(1));
    let (lo, hi) = (f64::from(metrics.descent), f64::from(metrics.ascent));
    let (lo, hi) = if hi > lo { (lo, hi) } else { (-0.2 * upm, 0.8 * upm) };
    Rect::new(0.0, lo, f64::from(advance).max(1.0), hi)
}

fn palette_colors(font: &FontRef, index: u16) -> Vec<Color> {
    let palettes = font.color_palettes();
    let palette = palettes.get(index).or_else(|| palettes.get(0));
    palette.map(|p| p.colors().to_vec()).unwrap_or_default()
}

fn draw_colr(font: &FontRef, gid: u16, options: &ColorOptions) -> Result<Done, Refusal> {
    let glyphs = font.color_glyphs();
    let Some(glyph) = glyphs.get(GlyphId::new(u32::from(gid))) else {
        return Err(Refusal::new("the glyph has no COLR record", 0));
    };
    let mut outlines = Outlines::new(font);
    let mut bounds = Bounds::new(&mut outlines);
    glyph.paint(LocationRef::default(), &mut bounds).map_err(|e| Refusal::new(format!("the COLR glyph cannot be drawn: {e}"), 0))?;
    let rect = match (bounds.total, bounds.unbounded) {
        (Some(t), true) => t.union(em_box(font, gid)),
        (None, true) => em_box(font, gid),
        (Some(t), false) => t,
        (None, false) => Rect::ZERO,
    };
    if !has_area(rect) {
        return blank(ColorKind::Colr);
    }
    let sprite = sprite(rect, options.scale)?;
    let colors = palette_colors(font, options.palette);
    let mut raster = Raster::new(&sprite, &mut outlines, &colors, options.text_color);
    if let Err(e) = glyph.paint(LocationRef::default(), &mut raster) {
        raster.refuse(format!("the COLR glyph cannot be drawn: {e}"));
    }
    raster.finish(sprite.left, sprite.top, ColorKind::Colr)
}

/// A colour glyph that paints nothing.
fn blank(kind: ColorKind) -> Result<Done, Refusal> {
    let image = Image::from_rgba_premultiplied(1, 1, vec![0; 4]).map_err(|e| Refusal::new(e.to_string(), 0))?;
    Ok((GlyphRaster { image, left: 0, top: 0, kind, warnings: Vec::new() }, 1))
}

// ── colour sources ─────────────────────────────────────────────────────────────────────────────

/// A decoded image: straight RGBA, rows top to bottom.
#[derive(Debug)]
struct Bitmap {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    /// A mask (`CBDT` bit depths 1 to 8): the alpha is coverage of the text colour.
    mask: bool,
}

impl Bitmap {
    fn decode(data: &[u8], max_pixels: u64) -> Result<Self, String> {
        let image = fontcraft_imagery::decode_rgba(data, max_pixels).map_err(|e| e.to_string())?;
        Ok(Bitmap { width: image.width, height: image.height, pixels: image.pixels, mask: false })
    }

    fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// What a colour glyph draws, ready to rasterise at any scale, in font units, y up.
enum Drawable {
    Shape(SvgShape),
    /// An image placed by an affine from its unit square (y down from its top left).
    Image {
        bitmap: Arc<Bitmap>,
        place: Affine,
        opacity: f32,
    },
    Group(Box<DrawGroup>),
}

/// An [`svg_draw::SvgGroup`] with its images decoded.
struct DrawGroup {
    items: Vec<Drawable>,
    filters: Vec<SvgFilter>,
    opacity: f32,
    clips: Vec<Vec<Drawable>>,
    masks: Vec<DrawMask>,
}

struct DrawMask {
    luminance: bool,
    region: BezPath,
    items: Vec<Drawable>,
}

/// The bytes `items` keep.
fn drawable_bytes(items: &[Drawable]) -> usize {
    items
        .iter()
        .map(|d| match d {
            Drawable::Shape(s) => 64 + 32 * s.outline.elements().len(),
            Drawable::Image { bitmap, .. } => 64 + bitmap.pixels.len(),
            Drawable::Group(g) => {
                let masks: usize = g.masks.iter().map(|m| 64 + 32 * m.region.elements().len() + drawable_bytes(&m.items)).sum();
                let filters: usize = g.filters.iter().map(|f| 128 + 160 * f.primitives.len()).sum();
                64 + drawable_bytes(&g.items) + g.clips.iter().map(|c| drawable_bytes(c)).sum::<usize>() + masks + filters
            }
        })
        .sum()
}

/// The area `items` may paint at `scale` sprite pixels per font unit, font units: a group's
/// within its clips and mask regions, a filtered group's where its filters can paint
/// ([`filtered_extent`]), or its unfiltered items' when that is past the filter limit.
fn extent(items: &[Drawable], scale: f64) -> Option<Rect> {
    items
        .iter()
        .filter_map(|d| match d {
            Drawable::Shape(s) => Some(s.outline.bounding_box()),
            Drawable::Image { place, .. } => Some(place.transform_rect_bbox(unit())),
            Drawable::Group(g) => {
                let content = extent(&g.items, scale);
                let mut r = if g.filters.is_empty() {
                    content?
                } else {
                    match filtered_extent(&g.filters, content, scale)? {
                        r if filter_fits(r, scale) => r,
                        _ => content?,
                    }
                };
                for clip in &g.clips {
                    r = r.intersect(extent(clip, scale)?);
                }
                for mask in &g.masks {
                    r = r.intersect(mask.region.bounding_box());
                }
                Some(r)
            }
        })
        .filter(|r| has_area(*r))
        .reduce(|a, b| a.union(b))
}

/// The box, font units, that `filters` run one after another can paint when what they read lies
/// within `content`: within each filter's region, and within `content` grown by how far the
/// filters move and spread it ([`filter::reach`]), unless a primitive paints where its inputs
/// are clear. `None` when that is nothing.
fn filtered_extent(filters: &[SvgFilter], content: Option<Rect>, scale: f64) -> Option<Rect> {
    let mut r = content;
    for f in filters {
        let region = f.to_font.transform_rect_bbox(f.region);
        if !finite_rect(region) {
            return None;
        }
        r = match filter::reach(f, scale) {
            None => Some(region),
            Some((x, y)) => r.map(|c| c.inflate(x, y).intersect(region)),
        }
        .filter(|r| has_area(*r));
    }
    r
}

/// Whether filters that can paint `r` (font units) are run at `scale`: it covers at most
/// [`MAX_FILTER_PIXELS`] sprite pixels.
fn filter_fits(r: Rect, scale: f64) -> bool {
    let pixels = r.area() * scale * scale;
    pixels.is_finite() && pixels <= MAX_FILTER_PIXELS as f64
}

/// A glyph's colour source read once: its inflated and parsed `SVG ` document, or the decoded
/// image of one `sbix` or `CBDT` strike.
struct Source {
    items: Vec<Drawable>,
    /// What of the source is not drawn as it should be.
    warnings: Vec<String>,
}

impl Source {
    fn bytes(&self) -> usize {
        drawable_bytes(&self.items)
    }
}

type Prepared = Result<Arc<Source>, Arc<str>>;

/// A source: glyph, table, and the strike's pixels per em (0 for `SVG `).
type SourceKey = (u16, ColorKind, u16);

struct SourceEntry {
    source: Prepared,
    bytes: usize,
    used: u64,
}

/// Sources read so far, the least recently used evicted past [`MAX_CACHE_GLYPHS`] entries or
/// [`MAX_SOURCE_BYTES`].
#[derive(Default)]
struct Sources {
    map: HashMap<SourceKey, SourceEntry>,
    bytes: usize,
}

impl Sources {
    /// The source for `key`, read by `prepare` when it is not kept, and the work reading took.
    fn get(&mut self, key: SourceKey, tick: u64, prepare: impl FnOnce() -> (Prepared, u64)) -> (Prepared, u64) {
        if let Some(e) = self.map.get_mut(&key) {
            e.used = tick;
            return (e.source.clone(), 0);
        }
        let (source, work) = prepare();
        let bytes = source.as_ref().map_or(0, |s| s.bytes());
        if self.map.len() >= MAX_CACHE_GLYPHS || self.bytes.saturating_add(bytes) > MAX_SOURCE_BYTES {
            let mut ticks: Vec<u64> = self.map.values().map(|e| e.used).collect();
            ticks.sort_unstable();
            let cut = ticks.get(ticks.len() / 4).copied().unwrap_or(0);
            self.map.retain(|_, e| e.used > cut);
            self.bytes = self.map.values().map(|e| e.bytes).sum();
            while self.bytes.saturating_add(bytes) > MAX_SOURCE_BYTES {
                let Some(oldest) = self.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) else { break };
                if let Some(gone) = self.map.remove(&oldest) {
                    self.bytes = self.bytes.saturating_sub(gone.bytes);
                }
            }
        }
        // A source larger than the whole cache is drawn but not kept.
        if bytes <= MAX_SOURCE_BYTES {
            self.bytes = self.bytes.saturating_add(bytes);
            self.map.insert(key, SourceEntry { source: source.clone(), bytes, used: tick });
        }
        (source, work)
    }
}

fn units_per_em(font: &FontRef) -> f64 {
    f64::from(font.head().map(|h| h.units_per_em()).unwrap_or(1000).max(1))
}

/// The strike glyph `gid` of `kind` is drawn from at `ppem` pixels per em (0 for `SVG `).
fn strike_of(font: &FontRef, gid: u16, kind: ColorKind, ppem: f64) -> Option<u16> {
    match kind {
        ColorKind::Colr | ColorKind::Svg => Some(0),
        ColorKind::Sbix => sbix_record(font, gid, ppem).map(|r| r.0),
        ColorKind::Cbdt => cbdt_glyph(font, gid, ppem).map(|(size, _)| size),
    }
}

fn prepare(font: &FontRef, gid: u16, kind: ColorKind, ppem: f64) -> (Prepared, u64) {
    let (result, work) = match kind {
        ColorKind::Colr => (Err("COLR glyphs are painted, not read".to_string()), 0),
        ColorKind::Svg => prepare_svg(font, gid),
        ColorKind::Sbix => prepare_sbix(font, gid, ppem),
        ColorKind::Cbdt => prepare_cbdt(font, gid, ppem),
    };
    (result.map(Arc::new).map_err(Arc::from), work)
}

// ── SVG documents ──────────────────────────────────────────────────────────────────────────────

/// The attributes of the document's root element, as written (at most [`MAX_ROOT_ATTRIBUTES`]).
fn root_attributes(doc: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let Some(start) =
        doc.windows(5).position(|w| w.starts_with(b"<svg") && w.get(4).is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/'))
    else {
        return out;
    };
    let mut at = start + 4;
    let byte = |i: usize| doc.get(i).copied();
    while out.len() < MAX_ROOT_ATTRIBUTES {
        while byte(at).is_some_and(|b| b.is_ascii_whitespace()) {
            at += 1;
        }
        match byte(at) {
            None | Some(b'>') | Some(b'/') => break,
            _ => {}
        }
        let name_start = at;
        while byte(at).is_some_and(|b| !b.is_ascii_whitespace() && b != b'=' && b != b'>' && b != b'/') {
            at += 1;
        }
        let name = doc.get(name_start..at).unwrap_or_default();
        while byte(at).is_some_and(|b| b.is_ascii_whitespace()) {
            at += 1;
        }
        if byte(at) != Some(b'=') {
            if name.is_empty() {
                break;
            }
            continue;
        }
        at += 1;
        while byte(at).is_some_and(|b| b.is_ascii_whitespace()) {
            at += 1;
        }
        let Some(quote) = byte(at).filter(|b| *b == b'"' || *b == b'\'') else { break };
        let value_start = at + 1;
        let Some(len) = doc.get(value_start..).and_then(|rest| rest.iter().position(|b| *b == quote)) else { break };
        out.push((name, doc.get(value_start..value_start + len).unwrap_or_default()));
        at = value_start + len + 1;
    }
    out
}

/// How a document that gives its own size is fitted to the em square: the alignment along x
/// and y (0, ½ or 1) and the scaling, from the root's `preserveAspectRatio`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    align: Option<(f64, f64)>,
    slice: bool,
}

/// The fit of a document whose root gives a `viewBox`, `width` or `height`; `None` for one
/// that gives none, whose user units are font units.
fn root_fit(doc: &[u8]) -> Option<Fit> {
    let attributes = root_attributes(doc);
    if !attributes.iter().any(|(n, _)| matches!(*n, b"viewBox" | b"width" | b"height")) {
        return None;
    }
    let mut fit = Fit { align: Some((0.5, 0.5)), slice: false };
    let Some((_, par)) = attributes.iter().find(|(n, _)| *n == b"preserveAspectRatio") else { return Some(fit) };
    let text = String::from_utf8_lossy(par);
    let mut words = text.split_ascii_whitespace().filter(|w| *w != "defer");
    let along = |w: &str| match w {
        "Min" => Some(0.0),
        "Mid" => Some(0.5),
        "Max" => Some(1.0),
        _ => None,
    };
    match words.next() {
        Some("none") => fit.align = None,
        Some(a) if a.len() == 8 && a.starts_with('x') && a.get(4..5) == Some("Y") => {
            if let (Some(x), Some(y)) = (a.get(1..4).and_then(along), a.get(5..8).and_then(along)) {
                fit.align = Some((x, y));
            }
        }
        _ => {}
    }
    fit.slice = words.next() == Some("slice");
    Some(fit)
}

/// The affine from the viewport as `svg_draw` places it (y up, its bottom left at the origin,
/// `w × h`) to font units: the document's user space with the glyph origin at its origin, y
/// down, and a document that gives its own size fitted to the em square, as the OpenType `SVG `
/// table defines.
fn svg_place(fit: Option<Fit>, upem: f64, w: f64, h: f64) -> Option<Affine> {
    let (a, d, e, f) = match fit {
        None => (1.0, 1.0, 0.0, 0.0),
        Some(fit) => {
            if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
                return None;
            }
            let (sx, sy) = (upem / w, upem / h);
            match fit.align {
                None => (sx, sy, 0.0, 0.0),
                Some((ax, ay)) => {
                    let s = if fit.slice { sx.max(sy) } else { sx.min(sy) };
                    (s, s, ax * (upem - s * w), ay * (upem - s * h))
                }
            }
        }
    };
    // A viewport point (x, y) y down arrives as (x, h − y); it goes to (a·x + e, −(d·y + f)).
    let place = Affine::new([a, 0.0, 0.0, d, e, -d * h - f]);
    finite_affine(place).then_some(place)
}

/// Whether the document's root clips to its viewport: its `overflow`, from its `style` when that
/// sets one, else from the attribute, is not `visible` or `auto`.
fn root_clips(doc: &[u8]) -> bool {
    let attributes = root_attributes(doc);
    let styled = attributes
        .iter()
        .filter(|(n, _)| *n == b"style")
        .flat_map(|(_, v)| v.split(|b| *b == b';'))
        .filter_map(|declaration| {
            let (name, value) = declaration.split_at(declaration.iter().position(|b| *b == b':')?);
            name.trim_ascii().eq_ignore_ascii_case(b"overflow").then(|| value.get(1..).unwrap_or_default().trim_ascii())
        })
        .next_back();
    let overflow = styled.or_else(|| attributes.iter().find(|(n, _)| *n == b"overflow").map(|(_, v)| v.trim_ascii()));
    !overflow.is_some_and(|v| v.eq_ignore_ascii_case(b"visible") || v.eq_ignore_ascii_case(b"auto"))
}

fn prepare_svg(font: &FontRef, gid: u16) -> (Result<Source, String>, u64) {
    let Some(stored) = font.svg().ok().and_then(|s| s.glyph_data(GlyphId::new(u32::from(gid)))) else {
        return (Err("the glyph has no SVG document".into()), 0);
    };
    let mut work = stored.len() as u64;
    let doc: Cow<[u8]> = if stored.starts_with(&GZIP_MAGIC) {
        match gunzip(stored, fontcraft_color::MAX_SVG_FILE_BYTES) {
            Ok(plain) => Cow::Owned(plain),
            Err(GunzipError::TooLarge) => {
                let limit = fontcraft_color::MAX_SVG_FILE_BYTES;
                return (Err(format!("the compressed SVG document inflates to more than {limit} bytes")), work);
            }
            Err(GunzipError::Invalid(what)) => return (Err(format!("the compressed SVG document cannot be read: {what}")), work),
        }
    } else {
        Cow::Borrowed(stored)
    };
    work = work.saturating_add(doc.len() as u64);
    let upem = units_per_em(font);
    let fit = root_fit(&doc);
    let mut unplaced = false;
    let place = |w: f64, h: f64| {
        svg_place(fit, upem, w, h).unwrap_or_else(|| {
            unplaced = true;
            Affine::IDENTITY
        })
    };
    let drawing = match svg_draw::drawing(&doc, place) {
        Ok(drawing) => drawing,
        Err(e) => return (Err(format!("the SVG document cannot be drawn: {e}")), work),
    };
    if unplaced {
        return (Err("the SVG document's width, height or viewBox is empty".into()), work);
    }
    let mut images = Images { decoded: HashMap::new(), pixels_left: MAX_BITMAP_PIXELS, work };
    let mut items = images.drawables(drawing.items);
    if fit.is_some_and(|f| f.slice) && root_clips(&doc) {
        // The em square, font units: the viewport the sliced document overflows.
        let em = Rect::new(0.0, -upem, upem, 0.0);
        let mut outline = BezPath::new();
        outline.move_to((em.x0, em.y0));
        outline.line_to((em.x1, em.y0));
        outline.line_to((em.x1, em.y1));
        outline.line_to((em.x0, em.y1));
        outline.close_path();
        let clip = Drawable::Shape(SvgShape { outline, paint: ColorPaint::Solid(ColorRef::Rgba([0.0, 0.0, 0.0, 1.0])) });
        items = vec![Drawable::Group(Box::new(DrawGroup { items, filters: Vec::new(), opacity: 1.0, clips: vec![vec![clip]], masks: Vec::new() }))];
    }
    (Ok(Source { items, warnings: drawing.warnings }), images.work)
}

/// The embedded images of one document, decoded once each within [`MAX_BITMAP_PIXELS`].
struct Images {
    decoded: HashMap<*const Vec<u8>, Option<Arc<Bitmap>>>,
    pixels_left: u64,
    work: u64,
}

impl Images {
    fn drawables(&mut self, items: Vec<SvgItem>) -> Vec<Drawable> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item {
                SvgItem::Shape(s) => out.push(Drawable::Shape(s)),
                SvgItem::Image(SvgImage { data, place, opacity }) => {
                    // An image used more than once is decoded once; one that cannot be read, or
                    // that would pass the glyph's pixel budget, is left out.
                    let bitmap = self
                        .decoded
                        .entry(Arc::as_ptr(&data))
                        .or_insert_with(|| {
                            let b = Bitmap::decode(&data, self.pixels_left).ok()?;
                            self.pixels_left = self.pixels_left.saturating_sub(b.pixel_count());
                            self.work = self.work.saturating_add(b.pixel_count());
                            Some(Arc::new(b))
                        })
                        .clone();
                    if let Some(bitmap) = bitmap
                        && finite_affine(place)
                    {
                        out.push(Drawable::Image { bitmap, place, opacity: opacity as f32 });
                    }
                }
                SvgItem::Group(SvgGroup { items, filters, opacity, clips, masks }) => {
                    let items = self.drawables(items);
                    let clips = clips.into_iter().map(|c| self.drawables(c)).collect();
                    let masks =
                        masks.into_iter().map(|m| DrawMask { luminance: m.luminance, region: m.region, items: self.drawables(m.items) }).collect();
                    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) as f32 } else { 0.0 };
                    out.push(Drawable::Group(Box::new(DrawGroup { items, filters, opacity, clips, masks })));
                }
            }
        }
        out
    }
}

fn svg_leaf(paint: &ColorPaint) -> Option<Leaf> {
    let rgba = |c: &ColorRef| -> [f32; 4] {
        match c {
            ColorRef::Rgba(v) => v.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) as f32 } else { 0.0 }),
            // svg_draw gives RGBA colours only; the others draw nothing.
            ColorRef::Palette { .. } | ColorRef::Foreground { .. } => [0.0; 4],
        }
    };
    let stops = |line: &ColorLine| -> Option<Stops> {
        let mut at: Vec<(f64, [f32; 4])> = line.stops.iter().filter(|s| s.offset.is_finite()).map(|s| (s.offset, rgba(&s.color))).collect();
        at.sort_by(|a, b| a.0.total_cmp(&b.0));
        (!at.is_empty()).then_some(Stops { at, extend: line.extend })
    };
    match paint {
        ColorPaint::Solid(c) => Some(Leaf::Solid(rgba(c))),
        ColorPaint::Linear { p0, p1, p2, line } => {
            if !(finite_point(*p0) && finite_point(*p1) && finite_point(*p2)) {
                return None;
            }
            // Colour is constant along lines parallel to p0–p2, as COLR version 1 defines it.
            let normal = (-(p2.y - p0.y), p2.x - p0.x);
            let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
            let nn = normal.0 * normal.0 + normal.1 * normal.1;
            let end = if nn > 1e-12 {
                let k = (dx * normal.0 + dy * normal.1) / nn;
                Point::new(p0.x + normal.0 * k, p0.y + normal.1 * k)
            } else {
                *p1
            };
            linear(*p0, end, stops(line)?)
        }
        ColorPaint::Radial { c0, r0, c1, r1, line } => {
            let ok = finite_point(*c0) && finite_point(*c1) && r0.is_finite() && r1.is_finite();
            ok.then(|| stops(line)).flatten().map(|stops| Leaf::Radial { c0: *c0, r0: r0.abs(), c1: *c1, r1: r1.abs(), stops })
        }
        ColorPaint::Sweep { .. } | ColorPaint::Layers(_) | ColorPaint::Glyph { .. } | ColorPaint::ColrGlyph { .. } => None,
    }
}

fn draw_svg(font: &FontRef, source: &Source, options: &ColorOptions) -> Result<Done, Refusal> {
    paint_svg(&mut Outlines::new(font), source, options)
}

fn paint_svg(outlines: &mut Outlines, source: &Source, options: &ColorOptions) -> Result<Done, Refusal> {
    let Some(rect) = extent(&source.items, options.scale) else { return blank(ColorKind::Svg) };
    let sprite = sprite(rect, options.scale)?;
    let mut raster = Raster::new(&sprite, outlines, &[], options.text_color);
    raster.warnings.clone_from(&source.warnings);
    raster.draw_items(&source.items);
    raster.finish(sprite.left, sprite.top, ColorKind::Svg)
}

/// An SVG file drawn in colour outside a font (M11.17e), as Edit View draws an svg layer whose
/// file has filters: `svg` placed by `place` (from the file's viewport, y up with its bottom left
/// at the origin, to font units, as [`svg_draw::drawing`] takes it) at `scale` pixels per font
/// unit, with what an `SVG ` table's glyph is drawn with: paints, images, clip paths, masks,
/// patterns and filters, the same limits and the same warnings. `Err` with the reason when it is
/// not drawn.
pub fn svg_file(svg: &[u8], place: Affine, scale: f64) -> Result<GlyphRaster, String> {
    if !(scale.is_finite() && scale > 0.0 && scale <= MAX_SCALE) {
        return Err("the scale must be a positive number of at most 64 pixels per unit".into());
    }
    if !finite_affine(place) {
        return Err("the SVG file cannot be placed".into());
    }
    let drawing = svg_draw::drawing(svg, |_, _| place).map_err(|e| format!("the SVG file cannot be drawn: {e}"))?;
    let mut images = Images { decoded: HashMap::new(), pixels_left: MAX_BITMAP_PIXELS, work: 0 };
    let source = Source { items: images.drawables(drawing.items), warnings: drawing.warnings };
    let options = ColorOptions { scale, palette: 0, text_color: [0, 0, 0, 255] };
    paint_svg(&mut Outlines::none(), &source, &options).map(|(raster, _)| raster).map_err(|r| r.reason)
}

/// The `sbix` record drawn for glyph `gid` at `ppem` pixels per em: the strike's size, the
/// origin offsets, the graphic type and the data, `dupe` records followed once.
fn sbix_record<'a>(font: &FontRef<'a>, gid: u16, ppem: f64) -> Option<(u16, i16, i16, Tag, &'a [u8])> {
    let sbix = font.sbix().ok()?;
    let id = GlyphId::new(u32::from(gid));
    let mut best: Option<(u16, i16, i16, Tag, &'a [u8])> = None;
    for strike in sbix.strikes().iter().take(fontcraft_color::MAX_STRIKES) {
        let Ok(strike) = strike else { continue };
        let Ok(Some(mut data)) = strike.glyph_data(id) else { continue };
        if data.graphic_type() == Tag::new(b"dupe") {
            let target = data.data().get(..2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_be_bytes);
            let Some(Ok(Some(other))) = target.map(|t| strike.glyph_data(GlyphId::new(u32::from(t)))) else { continue };
            data = other;
        }
        let size = strike.ppem();
        if size == 0 {
            continue;
        }
        let record = (size, data.origin_offset_x(), data.origin_offset_y(), data.graphic_type(), data.data());
        if nearer(size, best.map(|b| b.0), ppem) {
            best = Some(record);
        }
    }
    best
}

/// Whether a strike of `size` pixels per em is a better match for `ppem` than `best`: the
/// smallest strike at least that large, else the largest.
fn nearer(size: u16, best: Option<u16>, ppem: f64) -> bool {
    match best {
        None => true,
        Some(b) => {
            let (s, b) = (f64::from(size), f64::from(b));
            (s >= ppem && (b < ppem || s < b)) || (b < ppem && s > b)
        }
    }
}

fn prepare_sbix(font: &FontRef, gid: u16, ppem: f64) -> (Result<Source, String>, u64) {
    let upm = units_per_em(font);
    let Some((size, ox, oy, tag, data)) = sbix_record(font, gid, ppem) else {
        return (Err("the glyph has no sbix image".into()), 0);
    };
    if tag != Tag::new(b"png ") && tag != Tag::new(b"jpg ") {
        return (Err(format!("sbix images of type `{tag}` are not drawn")), 0);
    }
    let bitmap = match Bitmap::decode(data, MAX_BITMAP_PIXELS) {
        Ok(b) => b,
        Err(e) => return (Err(format!("the sbix image cannot be read: {e}")), data.len() as u64),
    };
    let unit = upm / f64::from(size);
    let (w, h) = (f64::from(bitmap.width), f64::from(bitmap.height));
    let place = Affine::new([w * unit, 0.0, 0.0, -h * unit, f64::from(ox) * unit, (f64::from(oy) + h) * unit]);
    let work = bitmap.pixel_count().saturating_add(data.len() as u64);
    (Ok(Source { items: vec![Drawable::Image { bitmap: Arc::new(bitmap), place, opacity: 1.0 }], warnings: Vec::new() }), work)
}

// ── CBDT ───────────────────────────────────────────────────────────────────────────────────────

/// The `CBDT` glyph drawn for `gid` at `ppem` pixels per em, by the rule `sbix` follows, and its
/// strike's pixels per em.
fn cbdt_glyph<'a>(font: &FontRef<'a>, gid: u16, ppem: f64) -> Option<(u16, BitmapGlyph<'a>)> {
    let strikes = BitmapStrikes::with_format(font, BitmapFormat::Cbdt)?;
    let id = GlyphId::new(u32::from(gid));
    let mut best: Option<(u16, BitmapGlyph<'a>)> = None;
    for strike in strikes.iter().take(fontcraft_color::MAX_STRIKES) {
        let size = strike.ppem();
        if !(size >= 1.0 && size <= f32::from(u16::MAX)) {
            continue;
        }
        let size = size as u16;
        let Some(glyph) = strike.get(id) else { continue };
        if nearer(size, best.as_ref().map(|b| b.0), ppem) {
            best = Some((size, glyph));
        }
    }
    best
}

fn prepare_cbdt(font: &FontRef, gid: u16, ppem: f64) -> (Result<Source, String>, u64) {
    let upm = units_per_em(font);
    let Some((_, glyph)) = cbdt_glyph(font, gid, ppem) else {
        return (Err("the glyph has no CBDT image".into()), 0);
    };
    let (gw, gh) = (glyph.width, glyph.height);
    let declared = u64::from(gw) * u64::from(gh);
    let bitmap = match glyph.data {
        BitmapData::Png(data) => Bitmap::decode(data, MAX_BITMAP_PIXELS).map_err(|e| format!("the CBDT image cannot be read: {e}")),
        _ if declared > MAX_BITMAP_PIXELS => Err(format!("the CBDT image is {gw} × {gh} pixels; at most {MAX_BITMAP_PIXELS} are drawn")),
        BitmapData::Bgra(data) => {
            let n = declared as usize * 4;
            match data.get(..n) {
                Some(bgra) => {
                    // Premultiplied BGRA to straight RGBA.
                    let pixels = bgra
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|&[b, g, r, a]| {
                            let straight = |c: u8| if a == 0 { 0 } else { ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8 };
                            [straight(r), straight(g), straight(b), a]
                        })
                        .collect();
                    Ok(Bitmap { width: gw, height: gh, pixels, mask: false })
                }
                None => Err("the CBDT image data is cut short".to_string()),
            }
        }
        BitmapData::Mask(mask) => match mask.decode(gw, gh) {
            Ok(alpha) => Ok(Bitmap { width: gw, height: gh, pixels: alpha.into_iter().flat_map(|a| [255, 255, 255, a]).collect(), mask: true }),
            Err(e) => Err(format!("the CBDT mask cannot be read: {e:?}")),
        },
    };
    let bitmap = match bitmap {
        Ok(b) => b,
        Err(e) => return (Err(e), declared),
    };
    if bitmap.width == 0 || bitmap.height == 0 {
        return (Ok(Source { items: Vec::new(), warnings: Vec::new() }), 0);
    }
    // The bearings put the image's top left that many strike pixels right of and above the
    // glyph origin; a strike pixel is `units_per_em ÷ ppem` units.
    let ux = upm / f64::from(glyph.ppem_x.max(1.0));
    let uy = upm / f64::from(glyph.ppem_y.max(1.0));
    let (w, h) = (f64::from(bitmap.width), f64::from(bitmap.height));
    let (bx, by) = (f64::from(glyph.inner_bearing_x), f64::from(glyph.inner_bearing_y));
    let place = match glyph.placement_origin {
        Origin::TopLeft => Affine::new([w * ux, 0.0, 0.0, -h * uy, bx * ux, by * uy]),
        Origin::BottomLeft => Affine::new([w * ux, 0.0, 0.0, -h * uy, bx * ux, (by + h) * uy]),
    };
    let work = bitmap.pixel_count();
    (Ok(Source { items: vec![Drawable::Image { bitmap: Arc::new(bitmap), place, opacity: 1.0 }], warnings: Vec::new() }), work)
}

/// A glyph drawn from a bitmap source (`sbix` or `CBDT`).
fn draw_bitmaps(font: &FontRef, source: &Source, kind: ColorKind, options: &ColorOptions) -> Result<Done, Refusal> {
    let rect = source
        .items
        .iter()
        .filter_map(|d| match d {
            Drawable::Image { place, .. } => Some(place.transform_rect_bbox(unit())),
            Drawable::Shape(_) | Drawable::Group(_) => None,
        })
        .filter(|r| has_area(*r) && finite_rect(*r))
        .reduce(|a, b| a.union(b));
    let Some(rect) = rect else { return blank(kind) };
    let sprite = sprite(rect, options.scale)?;
    let mut outlines = Outlines::new(font);
    let mut raster = Raster::new(&sprite, &mut outlines, &[], options.text_color);
    for item in &source.items {
        if let Drawable::Image { bitmap, place, opacity } = item {
            raster.draw_bitmap(bitmap, *place, *opacity);
        }
    }
    raster.finish(sprite.left, sprite.top, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(w: u32, h: u32, rgba: [u8; 4]) -> GlyphRaster {
        GlyphRaster {
            image: Image::from_rgba_premultiplied(w, h, rgba.repeat((w * h) as usize)).unwrap(),
            left: -1,
            top: -2,
            kind: ColorKind::Colr,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn a_sprite_lands_at_its_offset_from_the_origin_and_is_clipped_to_the_page() {
        let mut page = Image::from_rgba_premultiplied(4, 4, vec![0; 64]).unwrap();
        composite(&mut page, &raster(2, 2, [255, 0, 0, 255]), 1, 2);
        assert_eq!(page.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(page.pixel(1, 1), Some([255, 0, 0, 255]));
        assert_eq!(page.pixel(2, 2), Some([0, 0, 0, 0]));
        // Half-transparent green over the red: source over, premultiplied.
        composite(&mut page, &raster(2, 2, [0, 128, 0, 128]), 1, 2);
        assert_eq!(page.pixel(0, 0), Some([127, 128, 0, 255]));
        // Far outside: nothing, and no panic.
        composite(&mut page, &raster(2, 2, [0, 0, 255, 255]), i64::MAX, i64::MIN);
        composite(&mut page, &raster(2, 2, [0, 0, 255, 255]), -100, 100);
        assert_eq!(page.pixel(3, 3), Some([0, 0, 0, 0]));
    }

    #[test]
    fn bytes_that_are_not_a_font_are_refused() {
        assert!(ColorRenderer::new(Arc::from(&b"not a font"[..]), &[]).is_err());
    }

    #[test]
    fn composite_modes_follow_the_compositing_formulas() {
        let s = [0.5, 0.0, 0.0, 0.5];
        let d = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(composite_px(CompositeMode::SrcOver, s, d), [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(composite_px(CompositeMode::DestIn, s, d), [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(composite_px(CompositeMode::Clear, s, d), [0.0; 4]);
        let m = composite_px(CompositeMode::Multiply, [1.0, 1.0, 1.0, 1.0], [0.25, 0.5, 0.75, 1.0]);
        assert!(m.iter().zip([0.25, 0.5, 0.75, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6), "{m:?}");
    }

    #[test]
    fn a_sweep_runs_clockwise_from_its_start_angle() {
        let stops = Stops { at: vec![(0.0, [0.0; 4]), (1.0, [1.0; 4])], extend: ModelExtend::Pad };
        let leaf = Leaf::Sweep { center: Point::ORIGIN, start: 0.0, end: 360.0, stops };
        // A quarter turn clockwise in y-up space lies below the centre.
        let below = leaf.eval(Point::new(0.0, -1.0)).unwrap();
        assert!((below[0] - 0.25).abs() < 1e-3, "{below:?}");
    }

    // ── fonts built in code ──

    /// An sfnt of `tables` with a 1000-unit em and `glyphs` glyphs.
    fn sfnt(glyphs: u16, extra: Vec<([u8; 4], Vec<u8>)>) -> Arc<[u8]> {
        let mut head = vec![0u8; 54];
        head[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
        maxp.extend(glyphs.to_be_bytes());
        let mut tables = vec![(*b"head", head), (*b"maxp", maxp)];
        tables.extend(extra);
        tables.sort_by_key(|t| t.0);
        let mut out = 0x0001_0000u32.to_be_bytes().to_vec();
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0u8; 6]);
        let mut offset = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, mut data) in tables {
            out.extend(tag);
            out.extend(0u32.to_be_bytes());
            out.extend((offset as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            data.resize(data.len().div_ceil(4) * 4, 0);
            offset += data.len();
            body.extend(data);
        }
        out.extend(body);
        Arc::from(out)
    }

    /// An `SVG ` table with document `doc` for glyph 1.
    fn svg_table(doc: &[u8]) -> ([u8; 4], Vec<u8>) {
        let mut t = vec![0, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1];
        t.extend(14u32.to_be_bytes());
        t.extend((doc.len() as u32).to_be_bytes());
        t.extend(doc);
        (*b"SVG ", t)
    }

    fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        fontcraft_imagery::to_png(&fontcraft_imagery::Rgba8 { width, height, pixels: rgba.repeat((width * height) as usize) }).unwrap()
    }

    fn base64(data: &[u8]) -> String {
        const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ABC[((n >> (18 - 6 * i)) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// Glyph 1 drawn at `scale`.
    fn glyph1(font: Arc<[u8]>, scale: f64) -> Arc<GlyphRaster> {
        let mut r = ColorRenderer::new(font, &["g0".to_string(), "g1".to_string()]).unwrap();
        let options = ColorOptions { scale, palette: 0, text_color: [0, 0, 0, 255] };
        match r.glyph(1, &options, &mut FrameBudget::frame()) {
            Painted::Color(raster) => raster,
            other => panic!("glyph 1 is not drawn in colour: {other:?}"),
        }
    }

    /// The pixels with ink, as a box in pixels right of and above the glyph origin.
    fn ink(raster: &GlyphRaster) -> (i32, i32, i32, i32) {
        let (mut x0, mut x1, mut y0, mut y1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for py in 0..raster.image.height {
            for px in 0..raster.image.width {
                if raster.image.pixel(px, py).unwrap()[3] > 128 {
                    let (x, y_down) = (raster.left + px as i32, raster.top + py as i32);
                    (x0, x1, y0, y1) = (x0.min(x), x1.max(x + 1), y0.min(-y_down - 1), y1.max(-y_down));
                }
            }
        }
        (x0, x1, y0, y1)
    }

    /// The colour at pixel `(x, y)` right of and above the glyph origin.
    fn colour_at(raster: &GlyphRaster, x: i32, y: i32) -> [u8; 4] {
        let (px, py) = (x - raster.left, -y - 1 - raster.top);
        raster.image.pixel(px as u32, py as u32).unwrap()
    }

    /// The alpha at pixel `(x, y)` right of and above the glyph origin; 0 outside the sprite.
    fn alpha_at(raster: &GlyphRaster, x: i32, y: i32) -> u8 {
        let (px, py) = (x - raster.left, -y - 1 - raster.top);
        let inside = px >= 0 && py >= 0;
        if inside { raster.image.pixel(px as u32, py as u32).map_or(0, |p| p[3]) } else { 0 }
    }

    #[test]
    fn a_compressed_svg_document_is_inflated_and_its_view_box_fitted_to_the_em() {
        // The viewBox's top left lands on the glyph origin, a tenth of the em square per user
        // unit; the rect sits above the box, so above the baseline.
        let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 100 100 100"><rect id="glyph1" x="10" y="50" width="20" height="50" fill="#0000FF"/></svg>"##;
        let font = sfnt(2, vec![svg_table(&fontcraft_container::gzip::gzip(doc.as_bytes()))]);
        let raster = glyph1(font, 0.1);
        assert_eq!(raster.kind, ColorKind::Svg);
        assert_eq!(ink(&raster), (10, 30, 0, 50), "x 100 to 300 units, y 0 to 500");
        assert_eq!(colour_at(&raster, 20, 25), [0, 0, 255, 255]);

        // width and height without a viewBox: a 2000-unit viewport is the 1000-unit em.
        let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="2000"><rect x="200" y="-800" width="400" height="800" fill="#00FF00"/></svg>"##;
        assert_eq!(ink(&glyph1(sfnt(2, vec![svg_table(doc.as_bytes())]), 0.1)), (10, 30, 0, 40));

        // A damaged stream is refused, and the glyph drawn as its outline.
        let mut broken = fontcraft_container::gzip::gzip(doc.as_bytes());
        let n = broken.len();
        broken[n - 6] ^= 0xFF;
        let mut r = ColorRenderer::new(sfnt(2, vec![svg_table(&broken)]), &[]).unwrap();
        let options = ColorOptions { scale: 0.1, palette: 0, text_color: [0, 0, 0, 255] };
        assert!(matches!(r.glyph(1, &options, &mut FrameBudget::frame()), Painted::Refused(_)));
    }

    #[test]
    fn an_svg_document_draws_its_embedded_png_images() {
        let data = base64(&png(2, 2, [0, 255, 0, 255]));
        let doc = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" version="1.1" viewBox="0 1000 1000 1000"><image id="glyph1" x="100" y="800" width="200" height="200" xlink:href="data:image/png;base64,{data}"/></svg>"#
        );
        let raster = glyph1(sfnt(2, vec![svg_table(doc.as_bytes())]), 0.1);
        assert_eq!(ink(&raster), (10, 30, 0, 20), "x 100 to 300 units, y 0 to 200");
        assert_eq!(colour_at(&raster, 20, 10), [0, 255, 0, 255]);
    }

    /// Glyph 1 of a font whose `SVG ` table holds `body` in a 1000-unit viewBox, whose top left
    /// is the glyph origin: user y −500 lies 500 units above the baseline.
    fn svg_glyph(body: &str) -> Arc<GlyphRaster> {
        let doc = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" version="1.1" viewBox="0 0 1000 1000">{body}</svg>"#
        );
        glyph1(sfnt(2, vec![svg_table(doc.as_bytes())]), 0.1)
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    #[test]
    fn svg_clip_paths_keep_what_lies_inside_them() {
        let raster = svg_glyph(
            r##"<defs><clipPath id="c"><rect x="0" y="-500" width="250" height="500"/></clipPath></defs>
            <rect x="0" y="-500" width="500" height="500" fill="#FF0000" clip-path="url(#c)"/>"##,
        );
        assert_eq!(ink(&raster), (0, 25, 0, 50), "the left half of the square");
        assert_eq!(colour_at(&raster, 10, 25), [255, 0, 0, 255]);

        // A clip path with a clip path of its own: both apply.
        let raster = svg_glyph(
            r##"<defs><clipPath id="d"><rect x="0" y="-500" width="500" height="250"/></clipPath>
            <clipPath id="c" clip-path="url(#d)"><rect x="0" y="-500" width="250" height="500"/></clipPath></defs>
            <rect x="0" y="-500" width="500" height="500" fill="#FF0000" clip-path="url(#c)"/>"##,
        );
        assert_eq!(ink(&raster), (0, 25, 25, 50), "the top left quarter");
    }

    #[test]
    fn svg_masks_multiply_by_their_luminance_or_alpha() {
        // Left half white, right half 50 % grey: the blue square is whole on the left, half
        // transparent on the right.
        let raster = svg_glyph(
            r##"<defs><mask id="m" maskUnits="userSpaceOnUse" x="0" y="-500" width="500" height="500">
              <rect x="0" y="-500" width="250" height="500" fill="#FFFFFF"/>
              <rect x="250" y="-500" width="250" height="500" fill="#808080"/></mask></defs>
            <rect x="0" y="-500" width="500" height="500" fill="#0000FF" mask="url(#m)"/>"##,
        );
        assert_eq!(colour_at(&raster, 10, 25), [0, 0, 255, 255]);
        assert!(near(colour_at(&raster, 40, 25), [0, 0, 128, 128]), "{:?}", colour_at(&raster, 40, 25));

        // An alpha mask of black: luminance would hide everything.
        let raster = svg_glyph(
            r##"<defs><mask id="m" mask-type="alpha" maskUnits="userSpaceOnUse" x="0" y="-500" width="500" height="500">
              <rect x="0" y="-500" width="250" height="500" fill="#000000"/>
              <rect x="250" y="-500" width="250" height="500" fill="#000000" fill-opacity="0.5"/></mask></defs>
            <rect x="0" y="-500" width="500" height="500" fill="#0000FF" mask="url(#m)"/>"##,
        );
        assert_eq!(colour_at(&raster, 10, 25), [0, 0, 255, 255]);
        assert!(near(colour_at(&raster, 40, 25), [0, 0, 128, 128]), "{:?}", colour_at(&raster, 40, 25));

        // The mask's region cuts it: nothing shows below user y −250.
        let raster = svg_glyph(
            r##"<defs><mask id="m" maskUnits="userSpaceOnUse" x="0" y="-500" width="500" height="250">
              <rect x="0" y="-500" width="500" height="500" fill="#FFFFFF"/></mask></defs>
            <rect x="0" y="-500" width="500" height="500" fill="#0000FF" mask="url(#m)"/>"##,
        );
        assert_eq!(ink(&raster), (0, 50, 25, 50));
    }

    #[test]
    fn svg_patterns_tile_their_content_over_the_shape() {
        // 200-unit tiles, each red in its top left quarter, over a 400-unit square.
        let raster = svg_glyph(
            r##"<defs><pattern id="p" patternUnits="userSpaceOnUse" x="0" y="0" width="200" height="200">
              <rect x="0" y="0" width="100" height="100" fill="#FF0000"/></pattern></defs>
            <rect x="0" y="-400" width="400" height="400" fill="url(#p)"/>"##,
        );
        for (x, y) in [(5, 35), (25, 35), (5, 15), (25, 15)] {
            assert_eq!(colour_at(&raster, x, y), [255, 0, 0, 255], "red at ({x}, {y})");
        }
        for (x, y) in [(15, 35), (5, 25), (15, 15), (25, 25)] {
            assert_eq!(colour_at(&raster, x, y)[3], 0, "clear at ({x}, {y})");
        }
    }

    #[test]
    fn a_sliced_svg_image_is_clipped_to_its_box() {
        // A square image in a 200 × 100 box, scaled to cover it: 200 × 200 centred, of which
        // the box's middle 100 units show.
        let data = base64(&png(2, 2, [0, 255, 0, 255]));
        let raster = svg_glyph(&format!(
            r#"<image x="100" y="-300" width="200" height="100" preserveAspectRatio="xMidYMid slice" xlink:href="data:image/png;base64,{data}"/>"#
        ));
        assert_eq!(ink(&raster), (10, 30, 20, 30));
        assert_eq!(colour_at(&raster, 20, 25), [0, 255, 0, 255]);
    }

    /// `body` filtered by `filter` (the primitives of a filter whose region is the document's
    /// top left 500-unit square, above the baseline).
    fn filtered(filter: &str, body: &str) -> Arc<GlyphRaster> {
        svg_glyph(&format!(
            r#"<defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="-500" width="500" height="500">{filter}</filter></defs><g filter="url(#f)">{body}</g>"#
        ))
    }

    #[test]
    fn svg_filters_blur_move_and_recolour_what_they_filter() {
        // A 10-pixel red square blurred by 3 pixels: its middle stays nearly opaque, and ink
        // reaches past its edges.
        let square = r##"<rect x="200" y="-300" width="100" height="100" fill="#FF0000"/>"##;
        let raster = filtered(r#"<feGaussianBlur stdDeviation="30"/>"#, square);
        let mid = colour_at(&raster, 25, 25);
        assert!(mid[3] > 160 && mid[0] == mid[3] && mid[1] == 0 && mid[2] == 0, "{mid:?}");
        assert!(colour_at(&raster, 32, 25)[3] > 0, "the blur spreads 2 pixels beyond the edge");
        assert!(colour_at(&raster, 25, 18)[3] > 0);
        assert_eq!(alpha_at(&raster, 45, 25), 0, "but not 15");

        // A shadow of the square's alpha moved 10 pixels right and 10 down, under the square.
        let square = r##"<rect x="100" y="-400" width="100" height="100" fill="#FF0000"/>"##;
        for filter in [
            r#"<feOffset in="SourceAlpha" dx="100" dy="100" result="s"/><feMerge><feMergeNode in="s"/><feMergeNode in="SourceGraphic"/></feMerge>"#,
            r##"<feDropShadow dx="100" dy="100" stdDeviation="0" flood-color="#000000"/>"##,
        ] {
            let raster = filtered(filter, square);
            assert_eq!(colour_at(&raster, 15, 35), [255, 0, 0, 255], "{filter}");
            assert_eq!(colour_at(&raster, 25, 25), [0, 0, 0, 255], "the shadow, {filter}");
            assert_eq!((colour_at(&raster, 15, 25)[3], colour_at(&raster, 25, 35)[3]), (0, 0), "{filter}");
        }

        // A colour matrix swapping red and blue, and one taking the saturation away.
        let raster = filtered(r#"<feColorMatrix type="matrix" values="0 0 1 0 0  0 1 0 0 0  1 0 0 0 0  0 0 0 1 0"/>"#, square);
        assert_eq!(colour_at(&raster, 15, 35), [0, 0, 255, 255]);
        let raster = filtered(r#"<feColorMatrix type="saturate" values="0"/>"#, square);
        assert!(near(colour_at(&raster, 15, 35), [127, 127, 127, 255]), "{:?}", colour_at(&raster, 15, 35));
        assert!(raster.warnings.is_empty(), "{:?}", raster.warnings);
    }

    #[test]
    fn svg_filters_that_cannot_be_drawn_leave_the_element_unfiltered() {
        let square = r##"<rect x="200" y="-300" width="100" height="100" fill="#FF0000"/>"##;
        // A convolution kernel past the limit on a side.
        let kernel = vec!["1"; 17 * 17].join(" ");
        let raster = filtered(&format!(r#"<feConvolveMatrix order="17" kernelMatrix="{kernel}"/>"#), square);
        assert_eq!(colour_at(&raster, 25, 25), [255, 0, 0, 255]);
        assert_eq!(raster.warnings.len(), 1, "{:?}", raster.warnings);
        assert!(raster.warnings[0].contains("kernel"), "{:?}", raster.warnings);

        // A blur in a region of 1,200 × 1,200 sprite pixels runs over the part its square can
        // reach.
        let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 1000 1000"><defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="-1000" width="1000" height="1000"><feGaussianBlur stdDeviation="50"/></filter></defs><rect x="200" y="-300" width="100" height="100" fill="#FF0000" filter="url(#f)"/></svg>"##;
        let raster = glyph1(sfnt(2, vec![svg_table(doc.as_bytes())]), 1.2);
        let mid = colour_at(&raster, 300, 300);
        assert!(mid[3] > 64 && mid[0] == mid[3] && mid[1] == 0, "{mid:?}");
        assert!(alpha_at(&raster, 362, 300) > 0, "blurred");
        assert!(raster.warnings.is_empty(), "{:?}", raster.warnings);

        // A flood over that region would paint past the filter limit: drawn unfiltered.
        let flood = r##"<feFlood flood-color="#0000FF" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>"##;
        let flooded = doc.replace(r#"<feGaussianBlur stdDeviation="50"/>"#, flood);
        let raster = glyph1(sfnt(2, vec![svg_table(flooded.as_bytes())]), 1.2);
        assert_eq!(colour_at(&raster, 300, 300), [255, 0, 0, 255]);
        assert_eq!(alpha_at(&raster, 362, 300), 0, "no flood");
        assert!(raster.warnings.iter().any(|w| w.contains("can paint")), "{:?}", raster.warnings);

        // Regions far larger than any sprite: the blur still runs where the square reaches, and
        // the flood leaves the square unfiltered rather than hiding the glyph.
        let huge = |doc: &str| doc.replace(r#"width="1000" height="1000""#, r#"width="100000000" height="100000000""#);
        let raster = glyph1(sfnt(2, vec![svg_table(huge(doc).as_bytes())]), 0.1);
        let mid = colour_at(&raster, 25, 25);
        assert!(mid[3] > 0 && mid[0] == mid[3] && mid[1] == 0, "{mid:?}");
        assert!(alpha_at(&raster, 31, 25) > 0, "blurred");
        let raster = glyph1(sfnt(2, vec![svg_table(huge(&flooded).as_bytes())]), 0.1);
        assert_eq!(colour_at(&raster, 25, 25), [255, 0, 0, 255]);
        assert!(raster.warnings.iter().any(|w| w.contains("can paint")), "{:?}", raster.warnings);

        // A drop shadow over a million pixels takes more work than the glyph may: drawn
        // unfiltered rather than refused.
        let shadow = r##"<feGaussianBlur in="SourceAlpha" stdDeviation="20"/><feOffset dx="30" dy="30" result="o"/><feFlood flood-color="#000000" flood-opacity="0.5"/><feComposite in2="o" operator="in" result="s"/><feMerge><feMergeNode in="s"/><feMergeNode in="SourceGraphic"/></feMerge>"##;
        let shadowed = doc.replace(r#"<feGaussianBlur stdDeviation="50"/>"#, shadow);
        let raster = glyph1(sfnt(2, vec![svg_table(shadowed.as_bytes())]), 1.0);
        assert_eq!(colour_at(&raster, 250, 250), [255, 0, 0, 255]);
        assert_eq!(alpha_at(&raster, 320, 180), 0, "no shadow");
        assert!(raster.warnings.iter().any(|w| w.contains("more work")), "{:?}", raster.warnings);
    }

    /// `filtered` with `color-interpolation-filters="sRGB"`, so colours keep their values.
    fn filtered_srgb(filter: &str, body: &str) -> Arc<GlyphRaster> {
        svg_glyph(&format!(
            r#"<defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="-500" width="500" height="500" color-interpolation-filters="sRGB">{filter}</filter></defs><g filter="url(#f)">{body}</g>"#
        ))
    }

    #[test]
    fn svg_filters_transfer_convolve_dilate_erode_displace_and_tile() {
        // A red square over pixels 10 to 20 right of the origin and 30 to 40 above it.
        let square = r##"<rect x="100" y="-400" width="100" height="100" fill="#FF0000"/>"##;
        fn clean(r: &GlyphRaster) {
            assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        }

        // Red inverted by a table, green set by a linear function; clear pixels stay clear.
        let r = filtered_srgb(
            r#"<feComponentTransfer><feFuncR type="table" tableValues="1 0"/><feFuncG type="linear" slope="0" intercept="1"/></feComponentTransfer>"#,
            square,
        );
        assert_eq!(colour_at(&r, 15, 35), [0, 255, 0, 255]);
        assert_eq!(alpha_at(&r, 25, 35), 0);
        clean(&r);

        // A kernel whose one weight is in its bottom right corner moves the image a pixel right
        // and a pixel down.
        let r = filtered(r#"<feConvolveMatrix order="3" kernelMatrix="0 0 0 0 0 0 0 0 1"/>"#, square);
        assert_eq!(colour_at(&r, 20, 35), [255, 0, 0, 255]);
        assert_eq!(colour_at(&r, 15, 29), [255, 0, 0, 255]);
        assert_eq!((alpha_at(&r, 10, 35), alpha_at(&r, 15, 39)), (0, 0));
        clean(&r);

        // Dilated by 2 pixels: 8 to 22; eroded by 3: 13 to 17.
        let r = filtered(r#"<feMorphology operator="dilate" radius="20"/>"#, square);
        assert_eq!((colour_at(&r, 8, 35), colour_at(&r, 21, 35)), ([255, 0, 0, 255], [255, 0, 0, 255]));
        assert_eq!((alpha_at(&r, 7, 35), alpha_at(&r, 22, 35)), (0, 0));
        let r = filtered(r#"<feMorphology operator="erode" radius="30"/>"#, square);
        assert_eq!(colour_at(&r, 15, 35), [255, 0, 0, 255]);
        assert_eq!((alpha_at(&r, 12, 35), alpha_at(&r, 17, 35)), (0, 0));
        clean(&r);

        // A map whose red is full reads each pixel from half the scale (5 pixels) to its right,
        // so the square moves 5 pixels left; green at the middle leaves y as it is.
        let r = filtered_srgb(
            r#"<feFlood flood-color="rgb(255,128,0)" result="m"/><feDisplacementMap in="SourceGraphic" in2="m" scale="100" xChannelSelector="R" yChannelSelector="G"/>"#,
            square,
        );
        assert_eq!((colour_at(&r, 5, 35), colour_at(&r, 14, 35)), ([255, 0, 0, 255], [255, 0, 0, 255]));
        assert_eq!((alpha_at(&r, 4, 35), alpha_at(&r, 15, 35)), (0, 0));
        clean(&r);

        // A 5-pixel square in the top left of a 10-pixel tile, repeated over the region.
        let small = r##"<rect x="100" y="-400" width="50" height="50" fill="#FF0000"/>"##;
        let r = filtered(r#"<feOffset dx="0" dy="0" x="100" y="-400" width="100" height="100" result="t"/><feTile in="t"/>"#, small);
        for (x, y) in [(12, 37), (22, 37), (2, 37), (12, 27), (42, 7)] {
            assert_eq!(colour_at(&r, x, y), [255, 0, 0, 255], "({x}, {y})");
        }
        assert_eq!((alpha_at(&r, 17, 37), alpha_at(&r, 12, 32)), (0, 0));
        clean(&r);

        // A dilation by a billion units dilates by the limit, which fills the region.
        let r = filtered(r#"<feMorphology operator="dilate" radius="1000000000"/>"#, square);
        assert_eq!((colour_at(&r, 0, 0), colour_at(&r, 49, 49)), ([255, 0, 0, 255], [255, 0, 0, 255]));
        clean(&r);
    }

    #[test]
    fn svg_filters_make_noise_and_light() {
        let square = r##"<rect x="100" y="-400" width="100" height="100" fill="#FF0000"/>"##;
        // Turbulence fills the region, the same way each time, and nothing outside it.
        let noise = r#"<feTurbulence baseFrequency="0.05" numOctaves="2" seed="4" type="fractalNoise"/>"#;
        let a = filtered(noise, square);
        let b = filtered(noise, square);
        assert_eq!(a.image.pixels, b.image.pixels);
        assert!(a.warnings.is_empty(), "{:?}", a.warnings);
        let (x0, x1, y0, y1) = ink(&a);
        assert!(x0 >= 0 && x1 <= 50 && y0 >= 0 && y1 <= 50, "{:?}", ink(&a));
        assert!((0..50).any(|x| alpha_at(&a, x, 5) != alpha_at(&a, 25, 5)), "the noise varies");
        // A billion octaves run as ten, in time.
        let many = filtered(r#"<feTurbulence baseFrequency="0.05" numOctaves="1000000000"/>"#, square);
        assert!(many.warnings.is_empty(), "{:?}", many.warnings);

        // A light straight above a flat surface: white and opaque over the whole region.
        let r = filtered_srgb(
            r##"<feDiffuseLighting lighting-color="#FFFFFF"><feDistantLight azimuth="0" elevation="90"/></feDiffuseLighting>"##,
            square,
        );
        assert_eq!((colour_at(&r, 40, 10), colour_at(&r, 2, 47)), ([255; 4], [255; 4]));

        // A point light over the middle shines brightest there; a spot light aimed away leaves
        // the middle dark.
        let r = filtered_srgb(
            r##"<feSpecularLighting lighting-color="#00FF00" specularExponent="40"><fePointLight x="250" y="-250" z="1000"/></feSpecularLighting>"##,
            square,
        );
        let mid = colour_at(&r, 25, 25);
        assert!(mid[0] == 0 && mid[1] > 250 && mid[3] == mid[1], "{mid:?}");
        assert!(alpha_at(&r, 1, 1) < mid[3] - 50, "{}", alpha_at(&r, 1, 1));
        let r = filtered_srgb(
            r##"<feSpecularLighting lighting-color="#00FF00"><feSpotLight x="250" y="-250" z="1000" pointsAtX="5000" pointsAtY="-250" pointsAtZ="0" limitingConeAngle="10"/></feSpecularLighting><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge>"##,
            square,
        );
        assert_eq!(alpha_at(&r, 25, 25), 0);
        assert_eq!(colour_at(&r, 15, 35), [255, 0, 0, 255]);
    }

    #[test]
    fn an_svg_file_outside_a_font_draws_its_filters() {
        // The file's bottom left at the origin, a pixel per unit; a 20-unit square moved 30 right.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="100" height="100"><feOffset dx="30" dy="0"/></filter><rect x="0" y="80" width="20" height="20" fill="#0000FF" filter="url(#f)"/></svg>"##;
        let r = svg_file(svg, Affine::IDENTITY, 1.0).unwrap();
        assert_eq!(ink(&r), (30, 50, 0, 20));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(svg_file(svg, Affine::IDENTITY, f64::NAN).is_err());
        assert!(svg_file(b"<svg", Affine::IDENTITY, 1.0).is_err());
    }

    #[test]
    fn enclosing_clips_apply_to_what_a_filter_paints() {
        // A flood over a 500-unit region inside a group clipped to the left 250 units, beside a
        // green bar that makes the sprite 500 units wide.
        let raster = svg_glyph(
            r##"<defs><clipPath id="c"><rect x="0" y="-500" width="250" height="500"/></clipPath>
            <filter id="f" filterUnits="userSpaceOnUse" x="0" y="-500" width="500" height="500"><feFlood flood-color="#0000FF"/></filter></defs>
            <g clip-path="url(#c)"><rect x="0" y="-300" width="100" height="100" fill="#FF0000" filter="url(#f)"/></g>
            <rect x="450" y="-100" width="50" height="100" fill="#00FF00"/>"##,
        );
        assert_eq!(colour_at(&raster, 10, 10), [0, 0, 255, 255]);
        assert_eq!(colour_at(&raster, 24, 45), [0, 0, 255, 255]);
        assert_eq!(colour_at(&raster, 47, 5), [0, 255, 0, 255]);
        assert_eq!((alpha_at(&raster, 30, 25), alpha_at(&raster, 40, 45)), (0, 0), "nothing right of the clip");

        // A sliced document's shadow moved past the em square's right edge stays within the em.
        let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 1000 500" preserveAspectRatio="xMidYMid slice"><defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="1000" height="500"><feOffset dx="100" dy="0"/></filter></defs><rect x="600" y="100" width="100" height="100" fill="#00FF00" filter="url(#f)"/></svg>"##;
        let raster = glyph1(sfnt(2, vec![svg_table(doc.as_bytes())]), 0.1);
        assert_eq!(ink(&raster), (90, 100, -40, -20));
    }

    #[test]
    fn a_sliced_svg_document_is_clipped_to_the_em_square() {
        // A 1000 × 500 viewBox sliced into the em: scaled 2×, its middle 1000 units wide show.
        let doc = |overflow: &str| {
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 1000 500" preserveAspectRatio="xMidYMid slice"{overflow}><rect width="1000" height="500" fill="#00FF00"/></svg>"##
            )
        };
        let clipped = glyph1(sfnt(2, vec![svg_table(doc("").as_bytes())]), 0.1);
        assert_eq!(ink(&clipped), (0, 100, -100, 0));
        let visible = glyph1(sfnt(2, vec![svg_table(doc(r#" overflow="visible""#).as_bytes())]), 0.1);
        assert_eq!(ink(&visible), (-50, 150, -100, 0));
        // The style sets overflow too, over the attribute.
        let styled = glyph1(sfnt(2, vec![svg_table(doc(r#" overflow="hidden" style="stroke: none; overflow : Visible""#).as_bytes())]), 0.1);
        assert_eq!(ink(&styled), (-50, 150, -100, 0));
    }

    #[test]
    fn svg_masks_nested_past_the_layer_limit_draw_the_outline() {
        let mut defs = String::from(r##"<mask id="m9"><rect x="0" y="-500" width="500" height="500" fill="#FFFFFF"/></mask>"##);
        for k in (0..9).rev() {
            let next = k + 1;
            defs.push_str(&format!(
                r##"<mask id="m{k}"><rect x="0" y="-500" width="500" height="500" fill="#FFFFFF" mask="url(#m{next})"/></mask>"##
            ));
        }
        let doc = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 1000 1000"><defs>{defs}</defs><rect x="0" y="-500" width="500" height="500" fill="#0000FF" mask="url(#m0)"/></svg>"##
        );
        let mut r = ColorRenderer::new(sfnt(2, vec![svg_table(doc.as_bytes())]), &[]).unwrap();
        let options = ColorOptions { scale: 0.1, palette: 0, text_color: [0, 0, 0, 255] };
        let Painted::Refused(reason) = r.glyph(1, &options, &mut FrameBudget::frame()) else { panic!("drawn past the layer limit") };
        assert!(reason.contains("layers"), "{reason}");

        // Two masks deep is drawn.
        let doc = doc.replace(r#"mask="url(#m2)""#, "");
        let mut r = ColorRenderer::new(sfnt(2, vec![svg_table(doc.as_bytes())]), &[]).unwrap();
        assert!(matches!(r.glyph(1, &options, &mut FrameBudget::frame()), Painted::Color(_)));
    }

    /// `CBLC` and `CBDT` with one strike of `ppem` holding glyph 1 as `png` (format 17), its top
    /// left `bearing` strike pixels right of and above the origin.
    fn cbdt_tables(ppem: u8, png: &[u8], size: (u8, u8), bearing: (i8, i8)) -> Vec<([u8; 4], Vec<u8>)> {
        let mut cblc = vec![0, 3, 0, 0, 0, 0, 0, 1];
        cblc.extend(56u32.to_be_bytes());
        cblc.extend(24u32.to_be_bytes());
        cblc.extend(1u32.to_be_bytes());
        cblc.extend(0u32.to_be_bytes());
        cblc.extend([0u8; 24]);
        cblc.extend([0, 1, 0, 1, ppem, ppem, 32, 1]);
        cblc.extend([0, 1, 0, 1, 0, 0, 0, 8]);
        let record = 5 + 4 + png.len() as u32;
        cblc.extend([0, 1, 0, 17, 0, 0, 0, 4]);
        cblc.extend(0u32.to_be_bytes());
        cblc.extend(record.to_be_bytes());
        let mut cbdt = vec![0, 3, 0, 0, size.1, size.0, bearing.0 as u8, bearing.1 as u8, size.0];
        cbdt.extend((png.len() as u32).to_be_bytes());
        cbdt.extend(png);
        vec![(*b"CBLC", cblc), (*b"CBDT", cbdt)]
    }

    #[test]
    fn a_cbdt_glyph_is_drawn_from_its_strike_scaled_to_the_text_size() {
        let font = sfnt(2, cbdt_tables(100, &png(20, 10, [255, 0, 0, 255]), (20, 10), (5, 30)));
        // 100 pixels per em: one image pixel per sprite pixel, the top left at (5, 30).
        let raster = glyph1(font.clone(), 0.1);
        assert_eq!(raster.kind, ColorKind::Cbdt);
        assert_eq!(ink(&raster), (5, 25, 20, 30));
        assert_eq!(colour_at(&raster, 10, 25), [255, 0, 0, 255]);
        // 200 pixels per em: the only strike, twice as large.
        assert_eq!(ink(&glyph1(font, 0.2)), (10, 50, 40, 60));
    }
}
