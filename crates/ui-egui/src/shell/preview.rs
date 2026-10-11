//! The Preview Area under an Edit View tab's canvas, and the commands of both previews (the area
//! and the Preview Panel, [`super::preview_panel`]).
//!
//! A preview draws the tab's whole text filled, on one line without wrapping
//! (`preview.singleLine`), in the tab's master and picked layers, direction and kerning mode.
//! The strip reaches from the master's `Preview Descender` up to its `Preview Ascender`. The
//! edited glyph sits in the middle while Always Center Active Glyph is on; otherwise the text
//! keeps flush with the edge where it fits. Invert swaps text and ground, Flip turns the strip
//! upside down, Blur softens it, and the offset (dragging the text) shifts it sideways.
//! A double-click on a glyph makes it the edited glyph (`editView.setCursor`).
//!
//! The strip is one texture rasterised by the `render` crate, rebuilt only when what it shows
//! changes; the one-line layout is cached beside it in egui's memory.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_engine::Session;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{Font, Master, Params, metric};
use fontcraft_render::{Style, render_path};
use fontcraft_textlayout::{Direction, Layout, Options};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::composite::CompositeCache;
use crate::canvas::view;
use crate::icons::Icon;
use crate::panels::{Pending, widgets};
use crate::state::{Tab, WritingDirection};
use crate::theme::Tokens;

/// The master parameters that set the strip's upper and lower edge.
pub const PREVIEW_ASCENDER: &str = "Preview Ascender";
pub const PREVIEW_DESCENDER: &str = "Preview Descender";
/// The upper edge without `Preview Ascender`, in font units.
pub const DEFAULT_PREVIEW_ASCENDER: f64 = 1000.0;
/// The strongest blur, in screen points.
pub const MAX_BLUR: f64 = 10.0;
/// The tallest Preview Area, in screen points.
pub const MAX_AREA_HEIGHT: f64 = 4000.0;
/// The largest offset, in screen points either way.
pub const MAX_OFFSET: f64 = 1.0e6;
/// Largest absolute value read from `Preview Ascender`, `Preview Descender` and `winAscent`.
const MAX_EDGE: f64 = 65_535.0;
/// Box blur passes: three approximate a Gaussian.
const BLUR_PASSES: usize = 3;

/// One preview's options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewOptions {
    /// Light text on a dark ground (`preview.invert`).
    pub invert: bool,
    /// Upside down (`preview.flip`).
    pub flip: bool,
    /// Blur in screen points, 0 to [`MAX_BLUR`] (`preview.setBlur`).
    pub blur: f64,
    /// Keep the edited glyph in the middle (`preview.alwaysCenterActiveGlyph`).
    pub always_center: bool,
    /// The sideways shift in screen points (`preview.setOffset`).
    pub offset: f64,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self { invert: false, flip: false, blur: 0.0, always_center: true, offset: 0.0 }
    }
}

impl PreviewOptions {
    fn blur(&self) -> f64 {
        if self.blur.is_finite() { self.blur.clamp(0.0, MAX_BLUR) } else { 0.0 }
    }

    fn offset(&self) -> f64 {
        if self.offset.is_finite() { self.offset.clamp(-MAX_OFFSET, MAX_OFFSET) } else { 0.0 }
    }
}

/// Both previews' settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewState {
    /// The tabs that show the Preview Area, by id.
    pub area_tabs: Vec<u64>,
    /// The area's height in screen points; `None` for the theme's.
    pub area_height: Option<f64>,
    pub area: PreviewOptions,
    /// The Preview Panel is open (`window.previewPanel`).
    pub panel_open: bool,
    pub panel: PreviewOptions,
}

impl PreviewState {
    pub fn options(&self, target: Target) -> &PreviewOptions {
        match target {
            Target::Area => &self.area,
            Target::Panel => &self.panel,
        }
    }

    fn options_mut(&mut self, target: Target) -> &mut PreviewOptions {
        match target {
            Target::Area => &mut self.area,
            Target::Panel => &mut self.panel,
        }
    }

    /// The area's height: the one set, else the theme's, held between the theme's smallest and
    /// [`MAX_AREA_HEIGHT`].
    pub fn area_height(&self, tokens: &Tokens) -> f64 {
        let min = f64::from(tokens.preview_area_min_height);
        let h = self.area_height.filter(|h| h.is_finite()).unwrap_or(f64::from(tokens.preview_area_height));
        h.min(MAX_AREA_HEIGHT).max(min)
    }
}

/// Which preview a command changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Area,
    Panel,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Area => "area",
            Target::Panel => "panel",
        }
    }

    fn from_args(args: &Args) -> Result<Target, String> {
        match args.string("target")? {
            None | Some("area") => Ok(Target::Area),
            Some("panel") => Ok(Target::Panel),
            Some(other) => Err(args.err(format!("`target` must be \"area\" or \"panel\", got \"{other}\""))),
        }
    }
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.togglePreviewArea",
        "Preview",
        &[],
        "",
        "{tab?: id, on?: bool, height?: points} show or hide the Preview Area under the tab's canvas (omitted `on`: toggle); `height` sets the area's height, held to the smallest the theme allows and 4000 → {tab, on, height}",
    ),
    (
        "window.previewPanel",
        "Preview Panel",
        &["Window"],
        "",
        "{on?: bool} open or close the Preview Panel, a window that previews the showing Edit View tab and stays blank while Font View shows (omitted: toggle) → {on}",
    ),
    (
        "preview.invert",
        "Invert",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), on?: bool} light text on a dark ground instead of dark on light (omitted: toggle) → {target, on}",
    ),
    (
        "preview.flip",
        "Flip",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), on?: bool} draw the preview upside down (omitted: toggle) → {target, on}",
    ),
    (
        "preview.setBlur",
        "Blur",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), amount: points} blur the preview by `amount` screen points, held to 0..10; 0 is sharp → {target, amount}",
    ),
    (
        "preview.alwaysCenterActiveGlyph",
        "Always Center Active Glyph",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), on?: bool} on: the edited glyph stays in the middle of the preview; off: the text keeps flush with the left edge (right edge for right-to-left text) where it fits, and fills the width where it does not (omitted: toggle) → {target, on}",
    ),
    (
        "preview.setOffset",
        "Scroll Preview",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), x?: points, dx?: points} shift the preview line sideways to `x` screen points from where it would sit, or by `dx` more → {target, x}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "view.togglePreviewArea" => {
            let index = super::view::target_tab(app, args)?;
            let tab = app.state.tabs.get(index).map(|t| t.id).ok_or_else(|| args.err("no such tab"))?;
            let height = args.number("height")?;
            let ids: Vec<u64> = app.state.tabs.iter().map(|t| t.id).collect();
            let tokens = &app.tokens;
            let preview = &mut app.state.panels.preview;
            let on = args.toggle(preview.area_tabs.contains(&tab))?;
            // Closed tabs drop out, so the list never outgrows the tab strip.
            preview.area_tabs.retain(|t| *t != tab && ids.contains(t));
            if on {
                preview.area_tabs.push(tab);
            }
            if let Some(h) = height {
                preview.area_height = Some(h);
                preview.area_height = Some(preview.area_height(tokens));
            }
            Ok(json!({"tab": tab, "on": on, "height": preview.area_height(tokens)}))
        }
        "window.previewPanel" => {
            let preview = &mut app.state.panels.preview;
            preview.panel_open = args.toggle(preview.panel_open)?;
            Ok(json!({"on": preview.panel_open}))
        }
        "preview.invert" | "preview.flip" | "preview.alwaysCenterActiveGlyph" => {
            let target = Target::from_args(args)?;
            let options = app.state.panels.preview.options_mut(target);
            let slot = match id {
                "preview.invert" => &mut options.invert,
                "preview.flip" => &mut options.flip,
                _ => &mut options.always_center,
            };
            *slot = args.toggle(*slot)?;
            Ok(json!({"target": target.name(), "on": *slot}))
        }
        "preview.setBlur" => {
            let target = Target::from_args(args)?;
            let amount = args.number("amount")?.ok_or_else(|| args.err("give `amount` (screen points, 0 to 10)"))?.clamp(0.0, MAX_BLUR);
            app.state.panels.preview.options_mut(target).blur = amount;
            Ok(json!({"target": target.name(), "amount": amount}))
        }
        "preview.setOffset" => {
            let target = Target::from_args(args)?;
            let options = app.state.panels.preview.options_mut(target);
            let x = match (args.number("x")?, args.number("dx")?) {
                (Some(_), Some(_)) => return Err(args.err("give `x` or `dx`, not both")),
                (Some(x), None) => x,
                (None, Some(dx)) => options.offset() + dx,
                (None, None) => return Err(args.err("give `x` or `dx` (screen points)")),
            };
            options.offset = x.clamp(-MAX_OFFSET, MAX_OFFSET);
            Ok(json!({"target": target.name(), "x": options.offset}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// A master parameter as a finite number within ±[`MAX_EDGE`].
fn number_param(params: &Params, name: &str) -> Option<f64> {
    let v = match params.get(name)? {
        Plist::Number(n) => n.value().as_f64(),
        Plist::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (v.is_finite() && v.abs() <= MAX_EDGE).then_some(v)
}

/// The strip's lower and upper edge in font units for a master: `Preview Descender` (a distance
/// below the baseline, of either sign), else the master's or the font's `winAscent` (the
/// documented default, though `winDescent` is likely meant), else the master's descender; and `Preview Ascender`, else 1000. Falls back to the master's descender and
/// ascender when the edges would not enclose the baseline.
pub fn vertical_range(font: &Font, master: &Master) -> (f64, f64) {
    let ascender = number_param(&master.params, PREVIEW_ASCENDER).unwrap_or(DEFAULT_PREVIEW_ASCENDER);
    let descender = number_param(&master.params, PREVIEW_DESCENDER)
        .or_else(|| number_param(&master.params, "winAscent"))
        .or_else(|| number_param(&font.info.params, "winAscent"))
        .map(|d| -d.abs())
        .unwrap_or_else(|| font.metric_pos(master, metric::DESCENDER, -f64::from(font.units_per_em) * 0.2).min(0.0));
    if ascender > 0.0 && ascender > descender { (descender, ascender) } else { view::vertical_span(font, master) }
}

/// Where a preview draws the tab's line: font units to screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The strip.
    pub rect: Rect,
    /// Screen points per font unit.
    pub scale: f64,
    /// The screen x of the line's origin and the screen y of its baseline, before flipping.
    pub x0: f64,
    pub baseline: f64,
    pub flip: bool,
}

impl Placement {
    /// The screen point of a font point.
    pub fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        let p = pos2((self.x0 + x * self.scale) as f32, (self.baseline - y * self.scale) as f32);
        if self.flip { self.turn(p) } else { p }
    }

    /// The font point under a screen point.
    pub fn to_font(&self, p: Pos2) -> (f64, f64) {
        let p = if self.flip { self.turn(p) } else { p };
        ((f64::from(p.x) - self.x0) / self.scale, (self.baseline - f64::from(p.y)) / self.scale)
    }

    /// `p` turned half a circle about the strip's centre.
    fn turn(&self, p: Pos2) -> Pos2 {
        pos2(self.rect.min.x + self.rect.max.x - p.x, self.rect.min.y + self.rect.max.y - p.y)
    }
}

/// The line's extent along x in font units: from 0 rightwards, or ending at 0 for right-to-left
/// text.
fn extent(layout: &Layout) -> (f64, f64) {
    let length = layout.lines.first().map_or(0.0, |l| l.length);
    let length = if length.is_finite() { length.max(0.0) } else { 0.0 };
    if layout.direction == Direction::Rtl { (-length, 0.0) } else { (0.0, length) }
}

/// Where the line goes in `rect`: scaled so `range` (lower, upper edge) fills the height, and
/// placed along x by the options. `edited` is the item of the edited glyph. `None` for an empty
/// strip or a range that encloses nothing.
pub fn place(layout: &Layout, edited: Option<usize>, rect: Rect, range: (f64, f64), options: &PreviewOptions, margin: f64) -> Option<Placement> {
    let (low, high) = range;
    let span = high - low;
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    if !(span > 0.0 && w > 0.0 && h > 0.0 && span.is_finite()) {
        return None;
    }
    let scale = h / span;
    let (left, right) = extent(layout);
    let focus = edited.and_then(|i| layout.placed.get(i)).map_or((left + right) / 2.0, |p| p.x + p.advance / 2.0);
    let centred = w / 2.0 - focus * scale;
    let margin = if margin.is_finite() { margin.max(0.0) } else { 0.0 };
    let start = if options.always_center {
        centred
    } else if (right - left) * scale <= w - 2.0 * margin {
        if layout.direction == Direction::Rtl { w - margin - right * scale } else { margin - left * scale }
    } else {
        // Too long to fit: as near the middle as the edges allow, never leaving a gap at either.
        centred.max(w - margin - right * scale).min(margin - left * scale)
    };
    let x0 = f64::from(rect.min.x) + start + options.offset();
    let baseline = f64::from(rect.min.y) + high * scale;
    (x0.is_finite() && baseline.is_finite()).then_some(Placement { rect, scale, x0, baseline, flip: options.flip })
}

/// The item whose glyph is under screen point `p`, if any.
pub fn hit(layout: &Layout, tab: &Tab, placement: &Placement, p: Pos2) -> Option<usize> {
    let (x, _) = placement.to_font(p);
    layout
        .placed
        .iter()
        .enumerate()
        .position(|(i, placed)| placed.glyph.is_some() && tab.item_glyph(i).is_some() && x >= placed.x && x < placed.x + placed.advance)
}

/// The direction a preview lays its line out in: the tab's, with vertical text on a horizontal
/// line.
fn direction(tab: &Tab) -> Direction {
    match tab.direction {
        WritingDirection::Rtl => Direction::Rtl,
        WritingDirection::Ltr | WritingDirection::Ttb => Direction::Ltr,
    }
}

/// The one-line layout of a tab's text, in the tab's master with its direction and kerning
/// mode. `None` without a document or a master.
pub fn line_layout(session: &mut Session, tab: &Tab) -> Option<Layout> {
    let index = if tab.kerning.shows_kerning() { session.kern_index().ok() } else { None };
    let font = session.font().ok()?;
    let master = view::tab_master(font, tab)?.id.clone();
    let options = Options { direction: direction(tab), kerning: tab.kerning, single_line: true, index: index.as_deref(), layers: Some(&tab.layers), ..Default::default() };
    fontcraft_textlayout::layout(font, &master, &tab.text, &options).ok()
}

/// What a picture is drawn with.
pub struct Picture<'a> {
    pub font: &'a Font,
    pub tab: &'a Tab,
    pub layout: &'a Layout,
    pub placement: &'a Placement,
    /// Pixels per screen point.
    pub ppp: f32,
    /// The document's id and revision, for the composite cache.
    pub doc: (u64, u64),
    pub options: &'a PreviewOptions,
    /// Text and ground before inverting.
    pub ink: Color32,
    pub paper: Color32,
}

/// The strip as an image the size of its rect in pixels: the glyphs filled, inverted, turned
/// and blurred as the options say. `None` for an empty or oversized strip.
pub fn picture(p: &Picture, outlines: &mut CompositeCache) -> Option<ColorImage> {
    let rect = p.placement.rect;
    let ppp = if p.ppp.is_finite() && p.ppp > 0.0 { p.ppp } else { 1.0 };
    let (w, h) = ((rect.width() * ppp).round(), (rect.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let (width, height) = (w as u32, h as u32);
    let s = p.placement.scale;
    // Glyphs whose advance lies more than an em beyond the strip cannot reach into it.
    let reach = f64::from(p.font.units_per_em) * s;
    let (min_x, max_x) = (f64::from(rect.min.x) - reach, f64::from(rect.max.x) + reach);
    let mut filled = BezPath::new();
    for placed in &p.layout.placed {
        let Some(name) = placed.glyph.as_deref() else { continue };
        let (a, b) = (p.placement.x0 + placed.x * s, p.placement.x0 + (placed.x + placed.advance) * s);
        if b < min_x || a > max_x {
            continue;
        }
        let Some(layer) = view::shown_layer(p.font, p.tab, name) else { continue };
        let Some(composite) = outlines.get(p.doc, p.font, name, layer) else { continue };
        let mut outline = composite.outline.clone();
        outline.apply_affine(Affine::translate((placed.x, placed.y)));
        filled.extend(outline.elements().iter().copied());
    }
    let to_pixels = Affine::scale(f64::from(ppp))
        * Affine::translate((p.placement.x0 - f64::from(rect.min.x), p.placement.baseline - f64::from(rect.min.y)))
        * Affine::scale_non_uniform(s, -s);
    let (ink, paper) = if p.options.invert { (p.paper, p.ink) } else { (p.ink, p.paper) };
    let style = Style { ink: ink.to_srgba_unmultiplied(), background: paper.to_srgba_unmultiplied(), anti_alias: true };
    let image = render_path(&filled, to_pixels, width, height, &style).ok()?;
    let mut pixels = image.pixels;
    if p.options.flip {
        pixels = pixels.chunks_exact(4).rev().flatten().copied().collect();
    }
    let radius = (p.options.blur() * f64::from(ppp) / BLUR_PASSES as f64).ceil();
    if radius >= 1.0 {
        box_blur(&mut pixels, width as usize, height as usize, radius as usize);
    }
    Some(ColorImage::from_rgba_premultiplied([width as usize, height as usize], &pixels))
}

/// Blurs RGBA pixels in place with [`BLUR_PASSES`] box blurs of `radius` pixels each way; the
/// edges repeat.
fn box_blur(pixels: &mut [u8], width: usize, height: usize, radius: usize) {
    if width == 0 || height == 0 || radius == 0 || pixels.len() < width * height * 4 {
        return;
    }
    let mut line = Vec::new();
    for _ in 0..BLUR_PASSES {
        for y in 0..height {
            blur_run(pixels, y * width * 4, 4, width, radius, &mut line);
        }
        for x in 0..width {
            blur_run(pixels, x * 4, width * 4, height, radius, &mut line);
        }
    }
}

/// One box blur along `n` pixels starting at byte `start`, `stride` bytes apart.
fn blur_run(pixels: &mut [u8], start: usize, stride: usize, n: usize, radius: usize, line: &mut Vec<[u32; 4]>) {
    line.clear();
    for i in 0..n {
        let o = start + i * stride;
        let px = pixels.get(o..o + 4).unwrap_or(&[0; 4]);
        line.push([0, 1, 2, 3].map(|c| px.get(c).copied().map_or(0, u32::from)));
    }
    let last = n.saturating_sub(1);
    let at = |i: isize| line.get(i.clamp(0, last as isize) as usize).copied().unwrap_or([0; 4]);
    let r = radius as isize;
    let window = (2 * radius + 1) as u32;
    let mut sum = [0u32; 4];
    for k in -r..=r {
        let v = at(k);
        for c in 0..4 {
            if let (Some(s), Some(v)) = (sum.get_mut(c), v.get(c)) {
                *s += v;
            }
        }
    }
    for i in 0..n {
        let o = start + i * stride;
        if let Some(px) = pixels.get_mut(o..o + 4) {
            for (dst, s) in px.iter_mut().zip(sum) {
                *dst = (s / window).min(255) as u8;
            }
        }
        let (add, sub) = (at(i as isize + r + 1), at(i as isize - r));
        for c in 0..4 {
            if let (Some(s), Some(a), Some(b)) = (sum.get_mut(c), add.get(c), sub.get(c)) {
                *s = (*s + a).saturating_sub(*b);
            }
        }
    }
}

/// A preview's layout and texture, and what they were made from, kept in egui's memory.
#[derive(Clone, Default)]
struct Cache {
    layout_key: Option<u64>,
    layout: Option<Arc<Layout>>,
    picture_key: Option<u64>,
    texture: Option<TextureHandle>,
}

fn cache_id(target: Target) -> egui::Id {
    egui::Id::new(("fontcraft.preview", target))
}

/// The texture a preview drew last.
#[cfg(test)]
pub(crate) fn texture_id(ctx: &egui::Context, target: Target) -> Option<egui::TextureId> {
    ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).and_then(|c| c.texture.map(|t| t.id()))
}

fn hash(parts: impl Hash, floats: &[f64]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    for f in floats {
        f.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// The tab's one-line layout for `target`, from the cache while nothing it depends on changed.
fn cached_layout(app: &mut FontcraftApp, ctx: &egui::Context, target: Target, tab: &Tab) -> Option<Arc<Layout>> {
    let doc = app.session.active().map(|d| (d.id, d.revision))?;
    let key = hash((doc, tab.id, tab.text.revision(), &tab.master, &tab.layers, tab.direction, tab.kerning), &[]);
    let mut cache = ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).unwrap_or_default();
    if cache.layout_key == Some(key)
        && let Some(layout) = &cache.layout
    {
        return Some(layout.clone());
    }
    let layout = line_layout(&mut app.session, tab).map(Arc::new);
    cache.layout_key = Some(key);
    cache.layout.clone_from(&layout);
    ctx.data_mut(|d| d.insert_temp(cache_id(target), cache));
    layout
}

/// Draws a preview of `tab` in `rect` and takes its input: dragging shifts the line, a
/// double-click edits the glyph under the pointer, the context menu holds Always Center Active
/// Glyph.
pub(crate) fn strip(app: &mut FontcraftApp, ui: &mut egui::Ui, rect: Rect, target: Target, tab: &Tab, pending: &mut Pending) {
    let options = *app.state.panels.preview.options(target);
    let (ink, paper) = (app.tokens.preview_ink, app.tokens.preview_paper);
    let (ground, _) = if options.invert { (ink, paper) } else { (paper, ink) };
    ui.painter_at(rect).rect_filled(rect, 0.0, ground);
    let response = ui.interact(rect, widgets::id(("preview.strip", target.name())), Sense::click_and_drag());
    let ctx = ui.ctx().clone();
    let Some(layout) = cached_layout(app, &ctx, target, tab) else { return };
    let Ok(font) = app.session.font() else { return };
    let Some(master) = view::tab_master(font, tab) else { return };
    let range = vertical_range(font, master);
    let Some(placement) = place(&layout, tab.edited_index(), rect, range, &options, f64::from(app.tokens.preview_margin)) else { return };
    let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap_or_default();
    let ppp = ctx.pixels_per_point();
    let key = hash(
        (
            doc,
            tab.id,
            tab.text.revision(),
            &tab.master,
            &tab.layers,
            tab.direction,
            tab.kerning,
            options.invert,
            options.flip,
            ink.to_array(),
            paper.to_array(),
        ),
        &[
            placement.scale,
            placement.x0,
            placement.baseline,
            f64::from(rect.min.x),
            f64::from(rect.min.y),
            f64::from(rect.width()),
            f64::from(rect.height()),
            f64::from(ppp),
            options.blur(),
        ],
    );
    let mut cache = ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).unwrap_or_default();
    if cache.picture_key != Some(key) {
        let p = Picture { font, tab, layout: &layout, placement: &placement, ppp, doc, options: &options, ink, paper };
        cache.picture_key = Some(key);
        cache.texture = picture(&p, &mut app.fill_cache.outlines)
            .map(|image| ctx.load_texture(format!("preview-{}", target.name()), image, TextureOptions::LINEAR));
        ctx.data_mut(|d| d.insert_temp(cache_id(target), cache.clone()));
    }
    if let Some(texture) = &cache.texture {
        ui.painter_at(rect).image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        let dx = f64::from(response.drag_delta().x);
        if dx != 0.0 {
            pending.push(("preview.setOffset", json!({"target": target.name(), "dx": if options.flip { -dx } else { dx }})));
        }
    }
    if response.double_clicked()
        && let Some(at) = response.interact_pointer_pos()
        && let Some(index) = hit(&layout, tab, &placement, at)
    {
        pending.push(("editView.setCursor", json!({"tab": tab.id, "index": index})));
    }
    response.context_menu(|ui| {
        let mut on = options.always_center;
        if ui.checkbox(&mut on, "Always Center Active Glyph").clicked() {
            pending.push(("preview.alwaysCenterActiveGlyph", json!({"target": target.name(), "on": on})));
            ui.close();
        }
    });
}

/// A preview's controls row: Invert, Flip and the blur slider.
pub(crate) fn controls(ui: &mut egui::Ui, target: Target, options: &PreviewOptions, pending: &mut Pending) {
    ui.horizontal_centered(|ui| {
        let t = target.name();
        if widgets::button(ui, widgets::id(("preview.invert", t)), "Invert", "Light text on a dark ground", true, options.invert) {
            pending.push(("preview.invert", json!({ "target": t })));
        }
        if widgets::button(ui, widgets::id(("preview.flip", t)), "Flip", "Turn the preview upside down", true, options.flip) {
            pending.push(("preview.flip", json!({ "target": t })));
        }
        let mut amount = options.blur();
        let slider = egui::Slider::new(&mut amount, 0.0..=MAX_BLUR).show_value(false);
        if ui.add(slider).on_hover_text("Blur").changed() {
            pending.push(("preview.setBlur", json!({"target": t, "amount": amount})));
        }
    });
}

/// Draws the Preview Area under the showing tab's canvas while that tab has it open: the
/// separator (drag to change the height), the strip and the controls. Call before the canvas,
/// which takes the space left.
pub fn area(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab() else { return };
    if !app.state.panels.preview.area_tabs.contains(&tab.id) {
        return;
    }
    let tab = tab.clone();
    let height = app.state.panels.preview.area_height(&app.tokens);
    let options = *app.state.panels.preview.options(Target::Area);
    let (separator, row, fill, line) = (app.tokens.preview_separator, app.tokens.preview_controls_height, app.tokens.app_bar, app.tokens.border);
    let frame = egui::Frame::NONE.fill(fill);
    egui::Panel::bottom("edit_view_preview_area").resizable(false).exact_size(height as f32).frame(frame).show(ui, |ui| {
        let full = ui.max_rect();
        ui.allocate_rect(full, Sense::hover());
        let grab = Rect::from_min_size(full.min, vec2(full.width(), separator));
        let controls_rect = Rect::from_min_max(pos2(full.min.x, (full.max.y - row).max(grab.max.y)), full.max);
        let strip_rect = Rect::from_min_max(pos2(full.min.x, grab.max.y), pos2(full.max.x, controls_rect.min.y));
        let drag = ui.interact(grab, widgets::id("previewArea.separator"), Sense::drag());
        ui.painter().hline(grab.x_range(), grab.center().y, egui::Stroke::new(1.0, line));
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let dy = f64::from(drag.drag_delta().y);
        if drag.dragged() && dy != 0.0 {
            pending.push(("view.togglePreviewArea", json!({"tab": tab.id, "on": true, "height": height - dy})));
        }
        if strip_rect.height() > 0.0 {
            strip(app, ui, strip_rect, Target::Area, &tab, pending);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(controls_rect), |ui| controls(ui, Target::Area, &options, pending));
    });
}

/// The Preview button for the bottom bar: shows or hides the showing tab's Preview Area.
pub fn bottom_bar_button(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab() else { return };
    let on = app.state.panels.preview.area_tabs.contains(&tab.id);
    if widgets::icon_button(ui, widgets::id("bottomBar.preview"), Icon::Preview, "Show or hide the Preview Area", true, on) {
        pending.push(("view.togglePreviewArea", json!({ "tab": tab.id })));
    }
}
