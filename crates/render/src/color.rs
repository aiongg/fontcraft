//! Colour glyphs of a compiled font (M11.17): what Text Preview shows in place of a glyph's
//! outline, drawn from the font's own colour tables rather than from the editor's layers.
//!
//! Rows: `color.preview.textPreview`.
//!
//! | Table | Drawn |
//! |---|---|
//! | `COLR` (versions 0 and 1) with `CPAL` | through skrifa's colour painter: clips, solid fills, linear, radial and sweep gradients, transforms, and composite layers, with the palette [`ColorOptions::palette`] |
//! | `SVG ` | the glyph's document through the SVG drawing of `fontcraft_color` ([`svg_draw`]): filled outlines with solid or gradient paint |
//! | `sbix` | the PNG or JPEG image of the strike nearest the pixel size: the smallest strike at least that large, else the largest |
//!
//! A glyph in more than one table is drawn from `COLR`, else `SVG `, else `sbix`, the order in
//! which Edit View picks a glyph's colour layers. Palette index 65,535, and a row the palette
//! lacks, draw in [`ColorOptions::text_color`].
//!
//! Contract:
//! - Each glyph is rasterised once per palette, text colour and scale into a sprite whose pixel
//!   grid has the glyph origin on a pixel corner ([`GlyphRaster`]); [`composite`] lays sprites
//!   onto a page. Sprites are kept in a bounded cache ([`MAX_CACHE_GLYPHS`], [`MAX_CACHE_BYTES`]),
//!   the least recently used evicted.
//! - Work is counted in pixel operations. One glyph may take [`MIN_GLYPH_WORK`], or
//!   [`WORK_PER_PIXEL`] per pixel of its sprite when that is more; a glyph past its budget, a
//!   sprite larger than [`MAX_GLYPH_SIDE`] or [`MAX_GLYPH_PIXELS`], or data that cannot be read
//!   is [`Painted::Refused`], and the caller draws its outline. A [`FrameBudget`] spreads the
//!   first rasterisation of many glyphs over several frames: once it is spent, further glyphs
//!   are [`Painted::Deferred`].
//! - Not drawn: compressed SVG documents, documents that give their own width, height or
//!   viewBox (the compiler writes none), embedded images, patterns, clip paths, masks and
//!   filters inside SVG documents (as Edit View), `sbix` graphic types other than `png `, `jpg `
//!   and `dupe`, and `CBDT`.
//! - The `sbix` image's bottom left sits at the record's origin offsets from the glyph origin, a
//!   strike pixel being `units_per_em ÷ ppem` units, which is how the compiler places it.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_color::paint::{ColorLine, ColorRef, Paint as ColorPaint};
use fontcraft_color::svg_draw;
use fontcraft_geom::{Affine, BezPath, Point, Rect, Shape as _};
use fontcraft_model::Extend as ModelExtend;
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
/// Largest image read from an `sbix` record.
pub const MAX_BITMAP_PIXELS: u64 = 1 << 22;
/// Most outlines kept while one glyph is drawn.
const MAX_OUTLINES: usize = 4_096;
/// The size usvg gives a document without width, height or viewBox.
const SVG_DEFAULT_SIDE: f64 = 100.0;
/// Samples per side for one sprite pixel when an `sbix` image is drawn smaller than its pixels.
const MAX_BITMAP_SAMPLES: u32 = 4;

/// The tables a colour glyph is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorKind {
    Colr,
    Svg,
    Sbix,
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
        Ok(Self { font, ids, kinds: HashMap::new(), cache: HashMap::new(), bytes: 0, tick: 0 })
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
            Ok(f) => match draw(&f, gid, kind, options) {
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
    sbix_record(font, gid, f64::MAX).map(|_| ColorKind::Sbix)
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

fn draw(font: &FontRef, gid: u16, kind: ColorKind, options: &ColorOptions) -> Result<Done, Refusal> {
    if !(options.scale.is_finite() && options.scale > 0.0 && options.scale <= MAX_SCALE) {
        return Err(Refusal::new("the scale must be a positive number of at most 64 pixels per unit", 0));
    }
    match kind {
        ColorKind::Colr => draw_colr(font, gid, options),
        ColorKind::Svg => draw_svg(font, gid, options),
        ColorKind::Sbix => draw_sbix(font, gid, options),
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
    Ok(Sprite { width: w as u32, height: h as u32, base, left: left as i32, top: top as i32 })
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
    glyphs: OutlineGlyphCollection<'a>,
    kept: HashMap<u32, Arc<BezPath>>,
}

impl<'a> Outlines<'a> {
    fn new(font: &FontRef<'a>) -> Self {
        Self { glyphs: font.outline_glyphs(), kept: HashMap::new() }
    }

    fn get(&mut self, gid: GlyphId) -> Arc<BezPath> {
        if let Some(p) = self.kept.get(&gid.to_u32()) {
            return p.clone();
        }
        let mut pen = PathPen::default();
        if let Some(glyph) = self.glyphs.get(gid)
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
}

impl<'a, 'f> Raster<'a, 'f> {
    fn new(sprite: &Sprite, outlines: &'a mut Outlines<'f>, colors: &'a [Color], text: [u8; 4]) -> Self {
        let pixels = sprite.width as usize * sprite.height as usize * 4;
        Raster {
            width: sprite.width,
            height: sprite.height,
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
        if self.clips.len() >= MAX_CLIP_DEPTH {
            self.refuse(format!("the colour glyph nests clips more than {MAX_CLIP_DEPTH} deep"));
        }
        if self.refused.is_some() || path.elements().is_empty() {
            self.clips.push(Clip::empty());
            return;
        }
        let ctm = self.ctm();
        let parent = self.clips.last().map_or(self.full(), |c| c.bbox);
        let bbox = PixBox::of(path.bounding_box(), ctm, self.width, self.height).intersect(parent);
        if bbox.is_empty() || !self.spend(bbox.area().saturating_mul(2)) {
            self.clips.push(Clip::empty());
            return;
        }
        let (bw, bh) = (bbox.width(), bbox.y1 - bbox.y0);
        let shift = Affine::translate((-f64::from(bbox.x0), -f64::from(bbox.y0))) * ctm;
        let Ok(image) = render_path(path, shift, bw, bh, &MASK) else {
            self.clips.push(Clip::empty());
            return;
        };
        let mut mask: Vec<u8> = image.pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        if let Some(outer) = self.clips.last() {
            for (i, m) in mask.iter_mut().enumerate() {
                let (x, y) = (bbox.x0 + (i % bw as usize) as u32, bbox.y0 + (i / bw as usize) as u32);
                *m = ((u16::from(*m) * u16::from(outer.coverage(x, y)) + 127) / 255) as u8;
            }
        }
        self.clips.push(Clip { bbox, mask });
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

    fn finish(mut self, left: i32, top: i32, kind: ColorKind) -> Result<Done, Refusal> {
        if let Some(reason) = self.refused.take() {
            return Err(Refusal::new(reason, self.work));
        }
        let pixels = self.layers.into_iter().next().unwrap_or_default();
        let image = Image::from_rgba_premultiplied(self.width, self.height, pixels).map_err(|e| Refusal::new(e.to_string(), self.work))?;
        Ok((GlyphRaster { image, left, top, kind }, self.work))
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
    Ok((GlyphRaster { image, left: 0, top: 0, kind }, 1))
}

/// Whether the root element of `doc` gives its own size.
fn sized_root(doc: &[u8]) -> bool {
    let Some(start) = doc.windows(4).position(|w| w == b"<svg") else { return true };
    let rest = doc.get(start..).unwrap_or_default();
    let head = rest.get(..rest.iter().position(|b| *b == b'>').unwrap_or(rest.len())).unwrap_or_default();
    [&b"viewBox"[..], b" width", b" height", b"\twidth", b"\theight", b"\nwidth", b"\nheight"].iter().any(|k| head.windows(k.len()).any(|w| w == *k))
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
        ColorPaint::Sweep { .. } | ColorPaint::Layers(_) | ColorPaint::Glyph { .. } => None,
    }
}

fn draw_svg(font: &FontRef, gid: u16, options: &ColorOptions) -> Result<Done, Refusal> {
    let Some(doc) = font.svg().ok().and_then(|s| s.glyph_data(GlyphId::new(u32::from(gid)))) else {
        return Err(Refusal::new("the glyph has no SVG document", 0));
    };
    if doc.starts_with(&[0x1f, 0x8b]) {
        return Err(Refusal::new("compressed SVG documents are not drawn", 0));
    }
    if sized_root(doc) {
        return Err(Refusal::new("SVG documents with their own width, height or viewBox are not drawn", 0));
    }
    // svg_draw reads a file's user space y down from its top left; a document's user space has
    // the glyph origin at its origin, so the file's default height moves it back.
    let shapes = svg_draw::shapes(doc, Affine::translate((0.0, -SVG_DEFAULT_SIDE)))
        .map_err(|e| Refusal::new(format!("the SVG document cannot be drawn: {e}"), 0))?;
    let rect = shapes.iter().map(|s| s.outline.bounding_box()).filter(|r| has_area(*r)).reduce(|a, b| a.union(b));
    let Some(rect) = rect else { return blank(ColorKind::Svg) };
    let sprite = sprite(rect, options.scale)?;
    let mut outlines = Outlines::new(font);
    let mut raster = Raster::new(&sprite, &mut outlines, &[], options.text_color);
    for shape in &shapes {
        let Some(leaf) = svg_leaf(&shape.paint) else { continue };
        raster.push_clip_path(&shape.outline);
        raster.fill_leaf(&leaf);
        raster.clips.pop();
        if raster.refused.is_some() {
            break;
        }
    }
    raster.finish(sprite.left, sprite.top, ColorKind::Svg)
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
        let better = match best {
            None => true,
            Some((b, ..)) => {
                let (s, b) = (f64::from(size), f64::from(b));
                (s >= ppem && (b < ppem || s < b)) || (b < ppem && s > b)
            }
        };
        if better {
            best = Some(record);
        }
    }
    best
}

fn draw_sbix(font: &FontRef, gid: u16, options: &ColorOptions) -> Result<Done, Refusal> {
    let upm = f64::from(font.head().map(|h| h.units_per_em()).unwrap_or(1000).max(1));
    let Some((ppem, ox, oy, tag, data)) = sbix_record(font, gid, options.scale * upm) else {
        return Err(Refusal::new("the glyph has no sbix image", 0));
    };
    if tag != Tag::new(b"png ") && tag != Tag::new(b"jpg ") {
        return Err(Refusal::new(format!("sbix images of type `{tag}` are not drawn"), 0));
    }
    let image =
        fontcraft_imagery::decode_rgba(data, MAX_BITMAP_PIXELS).map_err(|e| Refusal::new(format!("the sbix image cannot be read: {e}"), 0))?;
    if image.width == 0 || image.height == 0 {
        return blank(ColorKind::Sbix);
    }
    let unit = upm / f64::from(ppem);
    let place = Affine::translate((f64::from(ox) * unit, f64::from(oy) * unit)) * Affine::scale(unit);
    let rect = place.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(image.width), f64::from(image.height)));
    let sprite = sprite(rect, options.scale)?;
    let mut outlines = Outlines::new(font);
    let mut raster = Raster::new(&sprite, &mut outlines, &[], options.text_color);
    // Sprite pixels to image pixels (y up from the image's bottom left).
    let to_image = (sprite.base * place).inverse();
    let pixel = options.scale * unit;
    let n = if pixel >= 1.0 { 1 } else { ((1.0 / pixel).ceil() as u32).clamp(1, MAX_BITMAP_SAMPLES) };
    let area = PixBox::of(rect, sprite.base, sprite.width, sprite.height);
    if !finite_affine(to_image) || !raster.spend(area.area().saturating_mul(u64::from(n * n))) {
        return raster.finish(sprite.left, sprite.top, ColorKind::Sbix);
    }
    let (iw, ih) = (image.width as usize, image.height as usize);
    let width = sprite.width as usize;
    let k = 1.0 / f64::from(n * n);
    let Some(layer) = raster.layers.first_mut() else { return Err(Refusal::new("no layer", 0)) };
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let mut sum = [0.0f64; 4];
            for j in 0..n {
                for i in 0..n {
                    let q =
                        to_image * Point::new(f64::from(x) + (f64::from(i) + 0.5) / f64::from(n), f64::from(y) + (f64::from(j) + 0.5) / f64::from(n));
                    if !(q.x >= 0.0 && q.y >= 0.0 && q.x < iw as f64 && q.y < ih as f64) {
                        continue;
                    }
                    let (sx, sy) = (q.x as usize, ih - 1 - (q.y as usize).min(ih - 1));
                    let from = (sy * iw + sx) * 4;
                    let Some(&[r, g, b, a]) = image.pixels.get(from..from + 4).and_then(|s| <&[u8; 4]>::try_from(s).ok()) else { continue };
                    let a = f64::from(a) / 255.0;
                    for (s, v) in sum.iter_mut().zip([f64::from(r) / 255.0 * a, f64::from(g) / 255.0 * a, f64::from(b) / 255.0 * a, a]) {
                        *s += v * k;
                    }
                }
            }
            if sum[3] <= 0.0 {
                continue;
            }
            let at = (y as usize * width + x as usize) * 4;
            if let Some(px) = layer.get_mut(at..at + 4) {
                for (d, v) in px.iter_mut().zip(sum) {
                    *d = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    raster.finish(sprite.left, sprite.top, ColorKind::Sbix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(w: u32, h: u32, rgba: [u8; 4]) -> GlyphRaster {
        GlyphRaster { image: Image::from_rgba_premultiplied(w, h, rgba.repeat((w * h) as usize)).unwrap(), left: -1, top: -2, kind: ColorKind::Colr }
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
}
