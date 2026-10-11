//! Hinted rendering: grid-fitted outlines and pixel images of a compiled font at a pixel size,
//! with its TrueType programs run by skrifa's interpreter or its CFF hints applied by skrifa's
//! PostScript hinter (owner decision M10 § 7, question 4 (a): FreeType's behaviour, not
//! Windows').
//!
//! Execution bounds, checked in skrifa 0.48's source: the TrueType interpreter stops a program
//! after 1,000,000 instructions (`MAX_RUN_INSTRUCTIONS`), also counts instructions skipped in
//! untaken branches against that budget, limits loop calls and the call stack, and reports
//! `ExceededExecutionBudget` instead of running on. That is the cap
//! ([`MAX_INSTRUCTIONS_PER_GLYPH`]); skrifa gives no way to lower it. A glyph whose program
//! fails or exceeds it is drawn unhinted and [`HintedOutline::message`] says why. A font whose
//! `fpgm` or `prep` fails is drawn unhinted at that size, with the same kind of message.
//!
//! Intents map to skrifa's hinting targets as follows. Each one is told apart by the bits the
//! TrueType `GETINFO` instruction returns, which is how a font's programs see the intent.
//!
//! | Intent | skrifa target | Pixels |
//! |---|---|---|
//! | Grayscale | smooth, normal mode, symmetric rendering | anti-aliased in both directions |
//! | GDI ClearType | smooth, LCD mode, no symmetric rendering | 6 samples across each pixel, one sample (the pixel centre) down it |
//! | DirectWrite | smooth, LCD mode, symmetric rendering | 6 samples across each pixel, anti-aliased down it |
//!
//! Pixels are grey coverage, not coloured subpixels: the backdrop is only indicative
//! (`hinting.tt.tool.pixelBackdrop`).
//!
//! Rows: `hint.tt.renderPreview`, `hint.ps.renderPreview`, `hinting.tt.tool.gridFittedOutline`
//! and `hinting.tt.tool.pixelBackdrop` (engine side), `hint.tt.setRenderingIntent` (the mapping).

use std::sync::{Arc, Mutex, PoisonError};

use fontcraft_geom::{Affine, BezPath, PathEl, Point};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph;
use skrifa::raw::tables::loca::LocaGlyph;
use skrifa::{FontRef, GlyphId, MetadataProvider, OutlineGlyphCollection};

use crate::{Image, MAX_LINE_GLYPHS, RenderError, Style};

/// The instructions skrifa runs for one program before it gives up (its own constant).
pub const MAX_INSTRUCTIONS_PER_GLYPH: usize = 1_000_000;
/// Pixel sizes accepted.
pub const MIN_PPEM: u16 = 1;
pub const MAX_PPEM: u16 = 400;
/// Largest font accepted, in bytes.
pub const MAX_FONT_BYTES: usize = 1 << 28;
/// Horizontal samples per pixel in the two ClearType intents.
pub const CLEARTYPE_SAMPLES: u32 = 6;
/// Pixel sizes whose hinting state a renderer keeps; the oldest is dropped beyond this.
const MAX_CACHED_SIZES: usize = 64;
/// Largest pixel coordinate a bitmap may reach, in either direction.
const MAX_PIXEL_COORD: f64 = 1.0e6;

/// The rendering a preview imitates (the Instructor's intent pop-up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Grayscale anti-aliasing (skrifa's smooth normal target).
    #[default]
    Grayscale,
    /// GDI ClearType: horizontal subpixels, no vertical oversampling; the intent in which the
    /// GDI-only rounding mode rounds.
    GdiClearType,
    /// DirectWrite ClearType: horizontal subpixels with symmetric smoothing.
    DirectWrite,
}

impl Intent {
    /// The skrifa hinting target that imitates this intent.
    fn target(self) -> Target {
        let (mode, symmetric_rendering) = match self {
            Intent::Grayscale => (SmoothMode::Normal, true),
            Intent::GdiClearType => (SmoothMode::Lcd, false),
            Intent::DirectWrite => (SmoothMode::Lcd, true),
        };
        Target::Smooth { mode, symmetric_rendering, preserve_linear_metrics: false }
    }

    /// Horizontal samples per pixel, and whether coverage is anti-aliased (both directions).
    fn sampling(self) -> (u32, bool) {
        match self {
            Intent::Grayscale => (1, true),
            Intent::GdiClearType => (CLEARTYPE_SAMPLES, false),
            Intent::DirectWrite => (CLEARTYPE_SAMPLES, true),
        }
    }
}

/// A grid-fitted outline in pixels at the requested size, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintedOutline {
    pub path: BezPath,
    /// The hinted advance width, in pixels.
    pub advance: f64,
    /// Whether hinting ran; `false` with a message when the glyph was drawn unhinted.
    pub hinted: bool,
    pub message: Option<String>,
}

/// A glyph's or a line's pixels, placed relative to the origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintedBitmap {
    /// Black ink whose alpha is the coverage, on a transparent background, one image pixel per
    /// device pixel.
    pub image: Image,
    /// The x of the image's left edge, in pixels from the origin.
    pub left: i32,
    /// The y of the image's top edge, in pixels above the baseline.
    pub top: i32,
}

/// A line of glyphs: its pixels and each glyph's outline at its own origin.
#[derive(Clone, Debug, PartialEq)]
pub struct HintedLine {
    pub bitmap: HintedBitmap,
    pub glyphs: Vec<HintedOutline>,
}

type Cached = (u16, Result<Arc<HintingInstance>, String>);

/// Renders the glyphs of one compiled font, hinted.
#[derive(Clone)]
pub struct HintedRenderer {
    font: Arc<[u8]>,
    intent: Intent,
    hinting: bool,
    /// The hinting state per pixel size (the TrueType `fpgm` and `prep` run once per size).
    /// Clones share it: they render the same font in the same intent.
    instances: Arc<Mutex<Vec<Cached>>>,
}

impl std::fmt::Debug for HintedRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HintedRenderer")
            .field("font_bytes", &self.font.len())
            .field("intent", &self.intent)
            .field("hinting", &self.hinting)
            .finish_non_exhaustive()
    }
}

impl HintedRenderer {
    /// A renderer for `font` (a TrueType or CFF sfnt) in `intent`. The font must parse.
    pub fn new(font: Arc<[u8]>, intent: Intent) -> Result<Self, RenderError> {
        if font.len() > MAX_FONT_BYTES {
            return Err(RenderError::Model(format!("the font is larger than {MAX_FONT_BYTES} bytes")));
        }
        {
            let parsed = FontRef::new(&font).map_err(|e| RenderError::Model(format!("the font cannot be read: {e}")))?;
            if parsed.outline_glyphs().format().is_none() {
                return Err(RenderError::Model("the font has no glyph outlines (no glyf, CFF or CFF2 table)".into()));
            }
        }
        Ok(Self { font, intent, hinting: true, instances: Arc::new(Mutex::new(Vec::new())) })
    }

    /// The same renderer with hinting off: outlines scaled to the pixel size only, for
    /// comparison with the hinted ones.
    pub fn without_hinting(mut self) -> Self {
        self.hinting = false;
        self
    }

    /// Whether the renderer applies hints.
    pub fn hinting(&self) -> bool {
        self.hinting
    }

    pub fn intent(&self) -> Intent {
        self.intent
    }

    /// The font bytes the renderer was made with.
    pub fn font(&self) -> &Arc<[u8]> {
        &self.font
    }

    fn font_ref(&self) -> Result<FontRef<'_>, RenderError> {
        FontRef::new(&self.font).map_err(|e| RenderError::Model(format!("the font cannot be read: {e}")))
    }

    /// Whether glyph id `glyph` has a TrueType glyph program of at least one byte. Always
    /// `false` for a CFF font.
    pub fn has_instructions(&self, glyph: u32) -> bool {
        let Ok(font) = self.font_ref() else { return false };
        let (Ok(loca), Ok(glyf)) = (font.loca(None), font.glyf()) else { return false };
        match loca.get(GlyphId::new(glyph), &glyf).and_then(LocaGlyph::into_glyph) {
            Some(Glyph::Simple(s)) => !s.instructions().is_empty(),
            Some(Glyph::Composite(c)) => c.instructions().is_some_and(|i| !i.is_empty()),
            None => false,
        }
    }

    /// The hinting state at `ppem`, made on first use. `Err` holds why the font's programs
    /// could not be set up at that size.
    fn instance(&self, outlines: &OutlineGlyphCollection, ppem: u16) -> Result<Arc<HintingInstance>, String> {
        let mut cache = self.instances.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, found)) = cache.iter().find(|(size, _)| *size == ppem) {
            return found.clone();
        }
        let options = HintingOptions { engine: Engine::Interpreter, target: self.intent.target() };
        let made = HintingInstance::new(outlines, Size::new(f32::from(ppem)), LocationRef::default(), options)
            .map(Arc::new)
            .map_err(|e| format!("the font's hinting programs failed at {ppem} PPM ({e}); drawn unhinted"));
        if cache.len() >= MAX_CACHED_SIZES {
            drop(cache.remove(0));
        }
        cache.push((ppem, made.clone()));
        made
    }

    /// The grid-fitted outline of glyph id `glyph` at `ppem` pixels per em.
    pub fn outline(&self, glyph: u32, ppem: u16) -> Result<HintedOutline, RenderError> {
        check_ppem(ppem)?;
        let font = self.font_ref()?;
        let outlines = font.outline_glyphs();
        let gid = GlyphId::new(glyph);
        let drawn = outlines.get(gid).ok_or_else(|| RenderError::Model(format!("the font has no glyph with id {glyph}")))?;
        let size = Size::new(f32::from(ppem));
        let fallback_advance = || {
            let advance = font.glyph_metrics(size, LocationRef::default()).advance_width(gid).map(f64::from).unwrap_or(0.0);
            if advance.is_finite() { advance } else { 0.0 }
        };
        let mut message = None;
        if self.hinting {
            match self.instance(&outlines, ppem) {
                Ok(instance) => {
                    let mut pen = Pen::default();
                    match drawn.draw(DrawSettings::hinted(&instance, false), &mut pen) {
                        Ok(metrics) => {
                            let enabled = instance.is_enabled();
                            let advance = metrics.advance_width.map(f64::from).filter(|a| a.is_finite()).unwrap_or_else(fallback_advance).round();
                            let message = (!enabled).then(|| format!("the font's programs turn hinting off at {ppem} PPM")).or(pen.message);
                            return Ok(HintedOutline { path: pen.path, advance, hinted: enabled, message });
                        }
                        Err(e) => message = Some(format!("hinting glyph {glyph} failed ({e}); drawn unhinted")),
                    }
                }
                Err(why) => message = Some(why),
            }
        }
        // A hinted draw that failed may have reached the pen: start again.
        let mut pen = Pen::default();
        let metrics = drawn
            .draw(DrawSettings::unhinted(size, LocationRef::default()), &mut pen)
            .map_err(|e| RenderError::Model(format!("glyph {glyph} cannot be drawn: {e}")))?;
        let advance = metrics.advance_width.map(f64::from).filter(|a| a.is_finite()).unwrap_or_else(fallback_advance);
        Ok(HintedOutline { path: pen.path, advance, hinted: false, message: message.or(pen.message) })
    }

    /// The glyph's pixels at `ppem`, one image pixel per device pixel, anti-aliased as the
    /// intent does it. The image spans the outline's pixels; [`Self::pixels`] says where it
    /// sits.
    pub fn bitmap(&self, glyph: u32, ppem: u16) -> Result<Image, RenderError> {
        Ok(self.pixels(glyph, ppem)?.image)
    }

    /// The glyph's pixels at `ppem` and where they sit. A glyph without contours gives one
    /// transparent pixel at the origin.
    pub fn pixels(&self, glyph: u32, ppem: u16) -> Result<HintedBitmap, RenderError> {
        let outline = self.outline(glyph, ppem)?;
        let Some(b) = Bounds::of(&outline.path) else { return empty_bitmap() };
        self.raster(&outline.path, b)
    }

    /// `glyphs` side by side at `ppem`, each placed by the advance of the one before (hinted
    /// advances are whole pixels). The image spans the outlines, the advances and the font's
    /// ascent and descent at that size.
    pub fn line(&self, glyphs: &[u32], ppem: u16) -> Result<HintedLine, RenderError> {
        if glyphs.is_empty() {
            return Err(RenderError::NoGlyphs);
        }
        if glyphs.len() > MAX_LINE_GLYPHS {
            return Err(RenderError::TooManyGlyphs);
        }
        check_ppem(ppem)?;
        let mut path = BezPath::new();
        let mut outlines = Vec::with_capacity(glyphs.len());
        let mut x = 0.0_f64;
        for &glyph in glyphs {
            let outline = self.outline(glyph, ppem)?;
            let placed = Affine::translate((x, 0.0)) * &outline.path;
            path.extend(placed.elements().iter().copied());
            x += outline.advance.max(0.0);
            if !x.is_finite() || x > MAX_PIXEL_COORD {
                return Err(RenderError::Model(format!("the line is wider than {MAX_PIXEL_COORD} pixels")));
            }
            outlines.push(outline);
        }
        let font = self.font_ref()?;
        let metrics = font.metrics(Size::new(f32::from(ppem)), LocationRef::default());
        let (ascent, descent) = (f64::from(metrics.ascent), f64::from(metrics.descent));
        let mut b = Bounds { x0: 0.0, y0: 0.0, x1: x.max(1.0), y1: 1.0 };
        if ascent.is_finite() && descent.is_finite() {
            b.include(Point::new(0.0, ascent));
            b.include(Point::new(0.0, descent));
        }
        if let Some(ink) = Bounds::of(&path) {
            b.include(Point::new(ink.x0, ink.y0));
            b.include(Point::new(ink.x1, ink.y1));
        }
        Ok(HintedLine { bitmap: self.raster(&path, b)?, glyphs: outlines })
    }

    /// Fills `path` over the whole pixels that hold `b`.
    fn raster(&self, path: &BezPath, b: Bounds) -> Result<HintedBitmap, RenderError> {
        let too_large = |w: f64, h: f64| RenderError::TooLarge { width: w.max(0.0) as u64, height: h.max(0.0) as u64 };
        let (left, right, bottom, top) = (b.x0.floor(), b.x1.ceil(), b.y0.floor(), b.y1.ceil());
        if [left, right, bottom, top].iter().any(|v| !v.is_finite() || v.abs() > MAX_PIXEL_COORD) {
            return Err(too_large(right - left, top - bottom));
        }
        // At least one pixel each way, for an outline flat in one direction.
        let (width, height) = ((right - left).max(1.0), (top - bottom).max(1.0));
        let (w, h) = (width as u32, height as u32);
        let (samples, anti_alias) = self.intent.sampling();
        let wide = w.checked_mul(samples).ok_or_else(|| too_large(width, height))?;
        let style = Style { ink: [0, 0, 0, 255], background: [0, 0, 0, 0], anti_alias };
        let mut pixmap = crate::blank(wide, h, &style)?;
        let s = f64::from(samples);
        let transform = Affine::new([s, 0.0, 0.0, -1.0, -left * s, top]);
        crate::fill(&mut pixmap, path, transform, &style);
        let data = pixmap.data();
        let (wide, n) = (wide as usize, samples as usize);
        let mut pixels = Vec::with_capacity((w as usize) * (h as usize) * 4);
        for row in 0..h as usize {
            for col in 0..w as usize {
                let start = row * wide + col * n;
                let sum: usize = (start..start + n).map(|i| usize::from(data.get(i * 4 + 3).copied().unwrap_or(0))).sum();
                let alpha = ((sum + n / 2) / n).min(255) as u8;
                pixels.extend_from_slice(&[0, 0, 0, alpha]);
            }
        }
        let image = Image::from_rgba_premultiplied(w, h, pixels)?;
        Ok(HintedBitmap { image, left: left as i32, top: top as i32 })
    }
}

fn empty_bitmap() -> Result<HintedBitmap, RenderError> {
    Ok(HintedBitmap { image: Image::from_rgba_premultiplied(1, 1, vec![0; 4])?, left: 0, top: 0 })
}

fn check_ppem(ppem: u16) -> Result<(), RenderError> {
    if (MIN_PPEM..=MAX_PPEM).contains(&ppem) { Ok(()) } else { Err(RenderError::BadNumber("the pixel size")) }
}

/// The box around a path's points, control points included (a little larger than the ink of a
/// curve, which only adds empty pixels).
#[derive(Clone, Copy, Debug)]
struct Bounds {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Bounds {
    fn of(path: &BezPath) -> Option<Self> {
        let mut out: Option<Self> = None;
        for el in path.elements() {
            let points = match *el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => [Some(p), None, None],
                PathEl::QuadTo(a, p) => [Some(a), Some(p), None],
                PathEl::CurveTo(a, b, p) => [Some(a), Some(b), Some(p)],
                PathEl::ClosePath => [None; 3],
            };
            for p in points.into_iter().flatten().filter(|p| p.x.is_finite() && p.y.is_finite()) {
                match out.as_mut() {
                    Some(b) => b.include(p),
                    None => out = Some(Self { x0: p.x, y0: p.y, x1: p.x, y1: p.y }),
                }
            }
        }
        out
    }

    fn include(&mut self, p: Point) {
        self.x0 = self.x0.min(p.x);
        self.y0 = self.y0.min(p.y);
        self.x1 = self.x1.max(p.x);
        self.y1 = self.y1.max(p.y);
    }
}

/// Collects skrifa's drawing into a path. A contour with a point that is not finite is left
/// out from that point on.
#[derive(Default)]
struct Pen {
    path: BezPath,
    message: Option<String>,
    /// A contour has been started and nothing in it was dropped (kurbo needs a move first).
    open: bool,
}

impl Pen {
    fn point(&mut self, x: f32, y: f32) -> Option<Point> {
        let p = Point::new(f64::from(x), f64::from(y));
        if p.x.is_finite() && p.y.is_finite() {
            Some(p)
        } else {
            self.message.get_or_insert_with(|| "the outline has points that are not finite numbers; they are left out".into());
            self.open = false;
            None
        }
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.path.close_path();
        }
        self.open = false;
        if let Some(p) = self.point(x, y) {
            self.path.move_to(p);
            self.open = true;
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if let (true, Some(p)) = (self.open, self.point(x, y)) {
            self.path.line_to(p);
        }
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        if let (true, Some(a), Some(p)) = (self.open, self.point(cx0, cy0), self.point(x, y)) {
            self.path.quad_to(a, p);
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if let (true, Some(a), Some(b), Some(p)) = (self.open, self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y)) {
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
