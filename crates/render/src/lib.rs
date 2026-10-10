//! FontCraft rasterising (L3): glyph layers and lines of glyphs to RGBA images, and PNG.
//!
//! Shapes are filled by the nonzero rule with anti-aliasing (tiny-skia). Only closed contours
//! are filled: an open path has no inside, and compiled fonts leave it out. The same input
//! always gives the same bytes, so tests and agents can compare renders.
//!
//! | Function | Draws |
//! |---|---|
//! | [`render_path`] | one Bézier path through an affine transform, into an image of a given size |
//! | [`render_line`] | a line of glyphs of one master, side by side by their advance widths |
//! | [`render_layer`] | one glyph layer, by layer id |
//!
//! [`Image`] also wraps pixels from elsewhere (a window screenshot) so they can be written as PNG.
#![forbid(unsafe_code)]

use fontcraft_geom::{Affine, BezPath, PathEl, Point};
use fontcraft_model::{Font, Glyph, Layer, Shape, metric};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// Widest or tallest image, in pixels.
pub const MAX_SIDE: u32 = 16_384;
/// Most pixels one image may have (4096 × 4096).
pub const MAX_PIXELS: u64 = 1 << 24;
/// Most glyphs one line may hold.
pub const MAX_LINE_GLYPHS: usize = 10_000;
/// Largest scale, in pixels per font unit.
pub const MAX_SCALE: f64 = 64.0;
/// Em size used when neither a size nor a scale is given, in pixels.
pub const DEFAULT_EM_PIXELS: f64 = 256.0;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RenderError {
    #[error("the image would be {width} × {height} pixels; at most {MAX_SIDE} on a side and {MAX_PIXELS} in all")]
    TooLarge { width: u64, height: u64 },
    #[error("the image is empty (0 pixels wide or high)")]
    Empty,
    #[error("{0} must be a positive number")]
    BadNumber(&'static str),
    #[error("a line holds at most {MAX_LINE_GLYPHS} glyphs")]
    TooManyGlyphs,
    #[error("no glyphs to render")]
    NoGlyphs,
    #[error("the pixel data does not match the size")]
    BadPixels,
    #[error("{0}")]
    Model(String),
    #[error("cannot encode the PNG: {0}")]
    Png(String),
}

/// An RGBA image, 8 bits per channel, colour premultiplied by alpha, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    /// Wraps premultiplied RGBA pixels (`width × height × 4` bytes).
    pub fn from_rgba_premultiplied(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, RenderError> {
        check_size(u64::from(width), u64::from(height))?;
        if pixels.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err(RenderError::BadPixels);
        }
        Ok(Self { width, height, pixels })
    }

    /// The pixel at `(x, y)`, as premultiplied RGBA.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = ((u64::from(y) * u64::from(self.width) + u64::from(x)) * 4) as usize;
        let p = self.pixels.get(at..at + 4)?;
        Some([*p.first()?, *p.get(1)?, *p.get(2)?, *p.get(3)?])
    }

    /// The image as a PNG file.
    pub fn png(&self) -> Result<Vec<u8>, RenderError> {
        let size = tiny_skia::IntSize::from_wh(self.width, self.height).ok_or(RenderError::Empty)?;
        let pixmap = Pixmap::from_vec(self.pixels.clone(), size).ok_or(RenderError::BadPixels)?;
        pixmap.encode_png().map_err(|e| RenderError::Png(e.to_string()))
    }
}

/// Colours and anti-aliasing of a render. Colours are straight (not premultiplied) RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub ink: [u8; 4],
    pub background: [u8; 4],
    pub anti_alias: bool,
}

impl Default for Style {
    /// Black on white, anti-aliased.
    fn default() -> Self {
        Self { ink: [0, 0, 0, 255], background: [255, 255, 255, 255], anti_alias: true }
    }
}

fn check_size(width: u64, height: u64) -> Result<(), RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::Empty);
    }
    if width > u64::from(MAX_SIDE) || height > u64::from(MAX_SIDE) || width * height > MAX_PIXELS {
        return Err(RenderError::TooLarge { width, height });
    }
    Ok(())
}

fn blank(width: u32, height: u32, style: &Style) -> Result<Pixmap, RenderError> {
    check_size(u64::from(width), u64::from(height))?;
    let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::Empty)?;
    let [r, g, b, a] = style.background;
    pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    Ok(pixmap)
}

/// The closed contours of `path`, in order; open ones are dropped.
pub fn closed_contours(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut contour: Vec<PathEl> = Vec::new();
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            contour.clear();
        }
        contour.push(*el);
        if matches!(el, PathEl::ClosePath) {
            out.extend(contour.drain(..));
        }
    }
    out
}

/// Fills the closed contours of `path` into `pixmap`. Elements with a coordinate that is not
/// finite after the transform are skipped with the rest of their contour.
fn fill(pixmap: &mut Pixmap, path: &BezPath, transform: Affine, style: &Style) {
    let path = closed_contours(path);
    let mut builder = PathBuilder::new();
    let mut broken = false;
    let f = |p: Point| {
        let q = transform * p;
        (q.x.is_finite() && q.y.is_finite()).then_some((q.x as f32, q.y as f32))
    };
    for el in path.elements() {
        if broken && !matches!(el, PathEl::MoveTo(_)) {
            continue;
        }
        broken = false;
        let ok = match *el {
            PathEl::MoveTo(p) => f(p).map(|(x, y)| builder.move_to(x, y)).is_some(),
            PathEl::LineTo(p) => f(p).map(|(x, y)| builder.line_to(x, y)).is_some(),
            PathEl::QuadTo(a, p) => match (f(a), f(p)) {
                (Some(a), Some(p)) => {
                    builder.quad_to(a.0, a.1, p.0, p.1);
                    true
                }
                _ => false,
            },
            PathEl::CurveTo(a, b, p) => match (f(a), f(b), f(p)) {
                (Some(a), Some(b), Some(p)) => {
                    builder.cubic_to(a.0, a.1, b.0, b.1, p.0, p.1);
                    true
                }
                _ => false,
            },
            PathEl::ClosePath => {
                builder.close();
                true
            }
        };
        if !ok {
            builder.close();
            broken = true;
        }
    }
    // `None`: nothing to draw (an empty path or one without area).
    let Some(path) = builder.finish() else { return };
    let mut paint = Paint::default();
    let [r, g, b, a] = style.ink;
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = style.anti_alias;
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// Fills the closed contours of `path`, mapped to pixels by `transform`, into a new
/// `width × height` image.
pub fn render_path(path: &BezPath, transform: Affine, width: u32, height: u32, style: &Style) -> Result<Image, RenderError> {
    let mut pixmap = blank(width, height, style)?;
    fill(&mut pixmap, path, transform, style);
    Ok(Image { width, height, pixels: pixmap.take() })
}

/// How large a line is drawn and where.
#[derive(Clone, Debug, PartialEq)]
pub struct LineOptions {
    /// The master whose layers are drawn (default: the first master).
    pub master: Option<String>,
    /// Pixels per em. Ignored when `scale` is set.
    pub size: Option<f64>,
    /// Pixels per font unit.
    pub scale: Option<f64>,
    /// Empty pixels around the glyphs.
    pub padding: u32,
    pub style: Style,
}

impl Default for LineOptions {
    fn default() -> Self {
        Self { master: None, size: None, scale: None, padding: 8, style: Style::default() }
    }
}

impl LineOptions {
    fn scale(&self, upm: f64) -> Result<f64, RenderError> {
        let positive = |v: f64, what| if v.is_finite() && v > 0.0 { Ok(v) } else { Err(RenderError::BadNumber(what)) };
        let scale = match (self.scale, self.size) {
            (Some(s), _) => positive(s, "scale")?,
            (None, Some(size)) => positive(size, "size")? / upm.max(1.0),
            (None, None) => DEFAULT_EM_PIXELS / upm.max(1.0),
        };
        if scale > MAX_SCALE {
            return Err(RenderError::BadNumber("scale (at most 64 pixels per unit)"));
        }
        Ok(scale)
    }
}

/// The vertical span drawn for a master: descender to ascender, in font units.
fn span(font: &Font, master: &fontcraft_model::Master) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    if ascender > descender { (descender, ascender) } else { (-upm * 0.2, upm * 0.8) }
}

/// Draws one outline per entry, each `advance` units after the last, on a canvas from the
/// master's descender to its ascender.
fn render_outlines(font: &Font, master: &fontcraft_model::Master, items: &[(BezPath, f64)], options: &LineOptions) -> Result<Image, RenderError> {
    let scale = options.scale(f64::from(font.units_per_em))?;
    let (descender, ascender) = span(font, master);
    let advance: f64 = items.iter().map(|(_, w)| if w.is_finite() { w.max(0.0) } else { 0.0 }).sum();
    let pad = f64::from(options.padding.min(MAX_SIDE));
    let (w, h) = ((advance * scale + 2.0 * pad).ceil(), ((ascender - descender) * scale + 2.0 * pad).ceil());
    if !(w.is_finite() && h.is_finite()) || w > f64::from(MAX_SIDE) || h > f64::from(MAX_SIDE) {
        return Err(RenderError::TooLarge { width: w.max(0.0) as u64, height: h.max(0.0) as u64 });
    }
    let (width, height) = ((w as u32).max(1), (h as u32).max(1));
    let mut pixmap = blank(width, height, &options.style)?;
    let mut x = 0.0;
    for (outline, width) in items {
        let at = Affine::translate((pad + x * scale, pad + ascender * scale)) * Affine::scale_non_uniform(scale, -scale);
        fill(&mut pixmap, outline, at, &options.style);
        x += if width.is_finite() { width.max(0.0) } else { 0.0 };
    }
    Ok(Image { width, height, pixels: pixmap.take() })
}

fn pick_master<'a>(font: &'a Font, master: Option<&str>) -> Result<&'a fontcraft_model::Master, RenderError> {
    match master {
        Some(id) => font.master(id).map_err(|e| RenderError::Model(e.to_string())),
        None => font.masters.first().ok_or_else(|| RenderError::Model("the font has no master".into())),
    }
}

/// Draws `glyphs` side by side on one line, each by its advance width, with components resolved.
/// The image spans the master's descender to its ascender, plus the padding.
pub fn render_line(font: &Font, glyphs: &[&str], options: &LineOptions) -> Result<Image, RenderError> {
    if glyphs.is_empty() {
        return Err(RenderError::NoGlyphs);
    }
    if glyphs.len() > MAX_LINE_GLYPHS {
        return Err(RenderError::TooManyGlyphs);
    }
    let master = pick_master(font, options.master.as_deref())?;
    let mut items = Vec::with_capacity(glyphs.len());
    for name in glyphs {
        let glyph = font.glyph(name).ok_or_else(|| RenderError::Model(format!("no glyph named `{name}`")))?;
        let outline = font.outline_of(glyph, &master.id).map_err(|e| RenderError::Model(e.to_string()))?;
        items.push((outline, glyph.layer(&master.id).map_or(0.0, |l| l.width)));
    }
    render_outlines(font, master, &items, options)
}

/// The outline of one layer: its paths, and its components resolved on the layer's master.
pub fn layer_outline(font: &Font, layer: &Layer) -> Result<BezPath, RenderError> {
    let mut out = BezPath::new();
    for shape in &layer.shapes {
        match shape {
            Shape::Path(p) => {
                let bez = p.to_bez().map_err(RenderError::Model)?;
                out.extend(bez.elements().iter().copied());
            }
            Shape::Component(c) => {
                let mut inner = font.outline(&c.glyph, &layer.master).map_err(|e| RenderError::Model(e.to_string()))?;
                inner.apply_affine(c.affine());
                out.extend(inner.elements().iter().copied());
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Draws one layer of `glyph`: the layer with id `layer`, or the first master's layer.
pub fn render_layer(font: &Font, glyph: &Glyph, layer: Option<&str>, options: &LineOptions) -> Result<Image, RenderError> {
    let layer = match layer {
        Some(id) => {
            glyph.layers.iter().find(|l| l.id == id).ok_or_else(|| RenderError::Model(format!("glyph `{}` has no layer `{id}`", glyph.name)))?
        }
        None => {
            let master = pick_master(font, None)?;
            glyph.layer(&master.id).ok_or_else(|| RenderError::Model(format!("glyph `{}` has no layer for the first master", glyph.name)))?
        }
    };
    let master = pick_master(font, Some(&layer.master)).or_else(|_| pick_master(font, None))?;
    let outline = layer_outline(font, layer)?;
    render_outlines(font, master, &[(outline, layer.width)], options)
}

#[cfg(test)]
mod tests;
