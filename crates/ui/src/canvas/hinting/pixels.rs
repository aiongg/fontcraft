//! The Instructor's pixel backdrop and its instance and grid-fitted outlines.
//!
//! Rows: `hinting.tt.tool.instanceOutline`, `hinting.tt.tool.gridFittedOutline`, `hinting.tt.tool.pixelBackdrop`,
//! `hint.tt.showPixelPreview`, `hint.tt.setPixelSize`, `hint.tt.setRenderingIntent`.
//!
//! While the TrueType Instructor is active, the canvas draws behind the edited outline:
//!
//! | Layer | Drawn from | When |
//! |---|---|---|
//! | pixel backdrop, grey coverage | the grid-fitted outline rasterised at the pixel size | always |
//! | blue outline | the previewed instance, unhinted | the previewed instance is not at the edited master, and Show Pixel Preview is on |
//! | red outline | the previewed instance after its instructions ran, grid-fitted | the glyph has a glyph program, and Show Pixel Preview is on |
//!
//! The preview font is the previewed instance compiled as TrueType in memory through the export's compiler, holding
//! only the edited glyph, the glyphs its components and corner components use, and `.notdef`, without layout tables.
//! Its programs are the ones a full export writes. [`Cache`] keeps the last preview font and its previews per pixel
//! size and intent, so a projection that changes neither only copies them.

use std::collections::BTreeSet;
use std::sync::Arc;

use craft::ui::primitives::TextureCache;
use egui::{Color32, ColorImage, Pos2, Rect, Stroke, TextureOptions};
use fontcraft_engine::compile::{self, DataRef, Flavour, Options};
use fontcraft_engine::model::{Font, GlyphTable, MAX_COMPONENT_DEPTH, Shape as ModelShape, Target};
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_render::hinted::{HintedRenderer, Intent};

use super::{RenderingIntent, clamp_ppm};
use crate::canvas::layers::PaintCx;
use crate::canvas::paint::outline_shapes;

/// Pixel sizes and intents whose preview is kept for one preview font.
const MAX_CACHED_PREVIEWS: usize = 16;
/// Most glyphs the preview font takes from the source: the edited glyph and what it uses.
const MAX_PREVIEW_GLYPHS: usize = 1_000;

/// The position in the font's instance list of the instance `name` names: by name, or `#n`.
pub fn instance_index(font: &Font, name: &str) -> Option<usize> {
    match name.strip_prefix('#') {
        Some(n) => n.parse::<usize>().ok().filter(|&i| i < font.instances.len()),
        None => font.instances.iter().position(|i| i.name == name),
    }
}

/// What the preview shows: the font target, and whether it sits at the edited master (then the blue outline would lie
/// under the black one and is left out).
#[derive(Clone, Debug, PartialEq)]
pub struct Previewed {
    pub target: Target,
    pub at_master: bool,
}

/// The previewed `instance` (as `InstructorState::instance` holds it) for an edit of master `master`.
pub fn previewed(font: &Font, master: &str, instance: Option<&str>) -> Result<Previewed, String> {
    let Some(name) = instance else { return Ok(Previewed { target: Target::Master(master.to_string()), at_master: true }) };
    let index = instance_index(font, name).ok_or_else(|| format!("the font has no instance `{name}`"))?;
    let at_master = font.instances.get(index).and_then(|i| font.master_at(&i.location)).is_some_and(|m| m.id == master);
    Ok(Previewed { target: Target::Instance(index), at_master })
}

/// Pixel coverage at one size, placed in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    /// Coverage per pixel, rows from the top.
    pub alpha: Vec<u8>,
    /// The left edge, in pixels from the glyph origin.
    pub left: i32,
    /// The top edge, in pixels above the baseline.
    pub top: i32,
    /// Font units per pixel: units per em / pixel size.
    pub unit: f64,
}

impl Pixels {
    /// The area the pixels cover, in font units: (left, bottom) and (right, top).
    pub fn bounds(&self) -> (Point, Point) {
        let (left, top) = (f64::from(self.left), f64::from(self.top));
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        (Point::new(left * self.unit, (top - h) * self.unit), Point::new((left + w) * self.unit, top * self.unit))
    }
}

/// The Instructor's preview of the edited glyph at one pixel size and intent, in font units with y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixelPreview {
    /// The previewed instance before hinting; `None` when it sits at the edited master.
    pub instance: Option<BezPath>,
    /// The grid-fitted outline; `None` for a glyph without a glyph program.
    pub fitted: Option<BezPath>,
    pub pixels: Pixels,
    /// Why the glyph was drawn unhinted, when it was.
    pub message: Option<String>,
    /// Tells textures of different previews apart.
    pub key: u64,
}

/// The names of `glyph` and of every glyph its layers use as components or corner components, and `.notdef`.
fn glyph_closure(font: &Font, glyph: &str) -> Result<BTreeSet<String>, String> {
    let mut wanted = BTreeSet::new();
    let mut frontier = vec![(glyph.to_string(), 0usize)];
    if font.glyph(".notdef").is_some() {
        frontier.push((".notdef".to_string(), 0));
    }
    while let Some((name, depth)) = frontier.pop() {
        if depth > MAX_COMPONENT_DEPTH || wanted.contains(&name) {
            continue;
        }
        let Some(g) = font.glyph(&name) else { continue };
        if wanted.len() >= MAX_PREVIEW_GLYPHS {
            return Err(format!("`{glyph}` uses more than {MAX_PREVIEW_GLYPHS} glyphs"));
        }
        for layer in &g.layers {
            for shape in &layer.shapes {
                if let ModelShape::Component(c) = shape {
                    frontier.push((c.glyph.clone(), depth + 1));
                }
            }
            for hint in &layer.info.hints {
                if let Ok(Some(corner)) = fontcraft_engine::model::corner::corner_hint(hint) {
                    frontier.push((corner.name, depth + 1));
                }
            }
        }
        wanted.insert(name);
    }
    Ok(wanted)
}

/// A preview font with its renderers: one unhinted, one per intent asked for.
struct Built {
    bytes: Arc<[u8]>,
    gid: u32,
    units_per_em: f64,
    plain: HintedRenderer,
    hinted: Vec<(RenderingIntent, HintedRenderer)>,
}

impl Built {
    /// `font` at `target` compiled as TrueType with only `glyph` and the glyphs it uses.
    fn new(font: &Font, target: &Target, glyph: &str, options: &Options) -> Result<Built, String> {
        if font.glyph(glyph).is_none() {
            return Err(format!("the font has no glyph `{glyph}`"));
        }
        let wanted = glyph_closure(font, glyph)?;
        let mut subset = font.clone();
        subset.glyphs = GlyphTable::from_glyphs(font.glyphs.iter().filter(|g| wanted.contains(&g.name)).cloned().collect());
        let options = Options { skip_features: true, ..options.clone() };
        let compiled = compile::compile(&subset, target, Flavour::TrueType, &options).map_err(|e| e.to_string())?;
        let gid = compiled
            .glyph_order
            .iter()
            .position(|g| g == glyph)
            .and_then(|i| u32::try_from(i).ok())
            .ok_or_else(|| format!("`{glyph}` is not in the exported font"))?;
        let bytes: Arc<[u8]> = compiled.bytes.into();
        let plain = HintedRenderer::new(bytes.clone(), Intent::Grayscale).map_err(|e| e.to_string())?.without_hinting();
        Ok(Built { bytes, gid, units_per_em: f64::from(font.units_per_em.max(1)), plain, hinted: Vec::new() })
    }

    /// The renderer of `intent`; it keeps the font's hinting state per pixel size.
    fn renderer(&mut self, intent: RenderingIntent) -> Result<HintedRenderer, String> {
        if let Some((_, r)) = self.hinted.iter().find(|(i, _)| *i == intent) {
            return Ok(r.clone());
        }
        let r = HintedRenderer::new(self.bytes.clone(), intent.hinted()).map_err(|e| e.to_string())?;
        self.hinted.push((intent, r.clone()));
        Ok(r)
    }

    fn render(&mut self, ppm: u16, intent: RenderingIntent, at_master: bool) -> Result<PixelPreview, String> {
        let unit = self.units_per_em / f64::from(ppm.max(1));
        let to_units = Affine::scale(unit);
        let gid = self.gid;
        let hinted = self.renderer(intent)?;
        let fitted = hinted.outline(gid, ppm).map_err(|e| e.to_string())?;
        let bitmap = hinted.pixels(gid, ppm).map_err(|e| e.to_string())?;
        let instance = if at_master { None } else { Some(to_units * self.plain.outline(gid, ppm).map_err(|e| e.to_string())?.path) };
        // Black ink: the coverage is the alpha.
        let alpha = bitmap.image.pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let pixels = Pixels { width: bitmap.image.width, height: bitmap.image.height, alpha, left: bitmap.left, top: bitmap.top, unit };
        let fitted_path = hinted.has_instructions(gid).then(|| to_units * fitted.path);
        Ok(PixelPreview { instance, fitted: fitted_path, pixels, message: fitted.message, key: 0 })
    }
}

/// What a preview font is built from: the document id and revision, the glyph, the edited master, the target.
type FontKey = ((u64, u64), String, String, Target);

/// The last preview font and its previews: compiling runs the export's outline preparation and the hinting programs,
/// too slow for every projection.
#[derive(Default)]
pub struct Cache {
    key: Option<FontKey>,
    built: Option<Result<Built, String>>,
    previews: Vec<((u16, RenderingIntent), Result<Arc<PixelPreview>, String>)>,
    /// Counts preview fonts, for texture keys.
    builds: u64,
}

impl Cache {
    /// The preview of `glyph` in an edit of `master` at `ppm` in `intent`, for the previewed `instance`. `doc` is the
    /// document's id and revision.
    #[allow(clippy::too_many_arguments)]
    pub fn preview(
        &mut self,
        font: &Font,
        options: impl FnOnce() -> Options,
        doc: (u64, u64),
        master: &str,
        glyph: &str,
        instance: Option<&str>,
        ppm: u16,
        intent: RenderingIntent,
    ) -> Result<Arc<PixelPreview>, String> {
        let previewed = previewed(font, master, instance)?;
        let ppm = clamp_ppm(i64::from(ppm));
        let key: FontKey = (doc, glyph.to_string(), master.to_string(), previewed.target.clone());
        if self.key.as_ref() != Some(&key) {
            self.built = Some(Built::new(font, &previewed.target, glyph, &options()));
            self.key = Some(key);
            self.previews.clear();
            self.builds = self.builds.wrapping_add(1);
        }
        if let Some((_, found)) = self.previews.iter().find(|(k, _)| *k == (ppm, intent)) {
            return found.clone();
        }
        let made = match self.built.as_mut() {
            Some(Ok(built)) => built.render(ppm, intent, previewed.at_master).map(|mut p| {
                p.key = self.builds.wrapping_mul(1 << 20) ^ (u64::from(ppm) << 2) ^ intent as u64;
                Arc::new(p)
            }),
            Some(Err(e)) => Err(e.clone()),
            None => Err("the preview font was not kept".to_string()),
        };
        if self.previews.len() >= MAX_CACHED_PREVIEWS {
            drop(self.previews.remove(0));
        }
        self.previews.push(((ppm, intent), made.clone()));
        made
    }
}

/// The compile options a preview uses: the session's glyph data.
pub fn options(session: &fontcraft_engine::Session) -> Options {
    Options { glyph_data: DataRef(session.glyph_data().ok()), ..Options::default() }
}

/// The pixel backdrop, behind the outline.
pub fn backdrop(cx: &PaintCx<'_>) {
    let Some(preview) = preview(cx) else { return };
    let p = &preview.pixels;
    if p.width == 0 || p.height == 0 || p.alpha.len() != (p.width as usize).saturating_mul(p.height as usize) {
        return;
    }
    let (min, max) = p.bounds();
    let (a, b) = (cx.camera.to_screen(Point::new(min.x, max.y)), cx.camera.to_screen(Point::new(max.x, min.y)));
    let rect = Rect::from_two_pos(a, b);
    if !rect.is_finite() || !rect.intersects(cx.rect) {
        return;
    }
    let ink = cx.hinting.tokens.tt_pixel;
    let image = || {
        let rgba: Vec<u8> = p
            .alpha
            .iter()
            .flat_map(|&a| Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), ((u16::from(a) * u16::from(ink.a())) / 255) as u8).to_array())
            .collect();
        ColorImage::from_rgba_premultiplied([p.width as usize, p.height as usize], &rgba)
    };
    let texture = TextureCache::texture_with(cx.painter.ctx(), ("fontcraftTtPixels", preview.key), TextureOptions::NEAREST, image);
    cx.painter.image(texture, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
}

/// The blue and red outlines, behind the edited outline.
pub fn outlines(cx: &PaintCx<'_>) {
    cx.painter.extend(outline_shapes_of(cx));
}

/// The shapes of [`outlines`].
pub fn outline_shapes_of(cx: &PaintCx<'_>) -> Vec<egui::Shape> {
    let Some(preview) = preview(cx) else { return Vec::new() };
    if !cx.hinting.view.instructor.pixel_preview {
        return Vec::new();
    }
    let t = cx.hinting.tokens;
    let mut out = Vec::new();
    if let Some(path) = &preview.instance {
        out.extend(outline_shapes(cx, path, Stroke::new(t.tt_preview_outline_width, t.tt_instance_outline)));
    }
    if let Some(path) = &preview.fitted {
        out.extend(outline_shapes(cx, path, Stroke::new(t.tt_preview_outline_width, t.tt_fitted_outline)));
    }
    out
}

/// The preview the layers draw: with the Instructor active, on the foreground.
fn preview<'a>(cx: &'a PaintCx<'_>) -> Option<&'a PixelPreview> {
    let tt = cx.hinting.model.tt.as_ref().filter(|tt| tt.instructor && !cx.edit_background)?;
    tt.preview.as_deref()
}
