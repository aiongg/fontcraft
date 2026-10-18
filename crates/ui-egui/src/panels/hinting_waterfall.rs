//! The Hinting Preview: a hinted waterfall of the edited glyphs from 8 to 80 PPM at three scales.
//!
//! While the TrueType Instructor is the active tool, the Preview Area of a tab that shows it
//! moves to the right edge of Edit View ([`area`]) and the Preview Panel shows the same
//! waterfall ([`draw`]). Each row is the tab's glyphs (at most [`MAX_GLYPHS`], around the edited
//! one) at one pixel size, built as TrueType in memory and grid-fitted by
//! `hint.tt.renderPreview`, then filled one image pixel per device pixel. The scale
//! (`hint.tt.setPreviewScale`) only changes how large those pixels are drawn: 1x puts one device
//! pixel on one screen pixel.
//!
//! The rows are cached in egui's memory and rebuilt when the document, the tab's text, master or
//! layers, or the colours change. While the pointer is down, or within [`REBUILD_INTERVAL`] of
//! the last build, the old rows stay up, so dragging a node does not compile the font on every
//! frame.
//!
//! Rows: `preview.ttHintingWaterfall`, `hint.tt.setPreviewScale`.

use std::hash::{Hash, Hasher};

use egui::{Color32, ColorImage, Rect, Sense, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_engine::Session;
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_render::{Image, Style, render_path};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::panels::{Pending, widgets};
use crate::shell::preview;
use crate::shell::{Args, UiCommand};
use crate::state::Tab;

/// The smallest and largest pixel size of the waterfall.
pub const FIRST_PPEM: u16 = 8;
pub const LAST_PPEM: u16 = 80;
/// The scales the pixels can be drawn at.
pub const SCALES: [u8; 3] = [1, 2, 4];
/// Most glyphs in one row.
pub const MAX_GLYPHS: usize = 32;
/// Widest and tallest row image, in device pixels; a longer line is cut at the right.
pub const MAX_ROW_PIXELS: f64 = 4096.0;
/// The tool whose activity turns the Preview Area into the waterfall.
pub const INSTRUCTOR_TOOL: &str = "trueTypeInstructor";
/// Shortest time between two builds while the font keeps changing, in seconds.
pub const REBUILD_INTERVAL: f64 = 0.25;
/// Sizes asked of `hint.tt.renderPreview` at once (its own limit).
const SIZES_PER_QUERY: usize = 64;
/// Most points read back for one row.
const MAX_ROW_POINTS: usize = 1_000_000;

/// The waterfall's own settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WaterfallState {
    /// Screen pixels per device pixel: 1, 2 or 4 (`hint.tt.setPreviewScale`).
    pub scale: u8,
}

impl Default for WaterfallState {
    fn default() -> Self {
        Self { scale: 1 }
    }
}

impl WaterfallState {
    /// The scale, or 1 when the stored one is not one of [`SCALES`].
    pub fn scale(&self) -> u8 {
        if SCALES.contains(&self.scale) { self.scale } else { 1 }
    }
}

pub const COMMANDS: &[UiCommand] = &[(
    "hint.tt.setPreviewScale",
    "Preview Scale",
    &[],
    "",
    "{scale: 1 | 2 | 4} draw the Hinting Preview's pixels at 1x (one screen pixel per device pixel), 2x or 4x → {scale}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "hint.tt.setPreviewScale" => {
            let n = args.number("scale")?.ok_or_else(|| args.err("give `scale`: 1, 2 or 4"))?;
            let Some(scale) = SCALES.iter().copied().find(|s| f64::from(*s) == n) else {
                return Err(args.err(format!("`scale` must be 1, 2 or 4, got {n}")));
            };
            app.state.panels.hinting_waterfall.scale = scale;
            Ok(json!({ "scale": scale }))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Whether the previews show the waterfall: the TrueType Instructor is the active tool.
pub fn active(app: &FontcraftApp) -> bool {
    app.tools.toolbox.active_id() == INSTRUCTOR_TOOL
}

/// The rendering intent the rows are grid-fitted for, as `hint.tt.renderPreview` names it.
/// Grayscale until the Instructor's intent choice (`hint.tt.setRenderingIntent`) has a state to
/// read.
fn intent(_app: &FontcraftApp) -> &'static str {
    "grayscale"
}

/// Draws the Preview Area as the waterfall at the right edge of Edit View, when the showing tab
/// has the Preview Area open and the TrueType Instructor is active. Returns whether it drew, so
/// the Preview Area under the canvas stays away. Call before the canvas.
pub fn area(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) -> bool {
    if !active(app) {
        return false;
    }
    let Some(tab) = app.state.tab().filter(|t| app.state.panels.preview.area_tabs.contains(&t.id)).cloned() else { return false };
    let width = app.tokens.preview_panel_size[0] * 0.5;
    let frame = egui::Frame::NONE.fill(app.tokens.app_bar);
    egui::Panel::right("edit_view_hinting_preview").resizable(true).default_size(width).frame(frame).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.allocate_rect(rect, Sense::hover());
        draw(app, ui, rect, &tab, pending);
    });
    true
}

/// One row's pixels: the line at one size, one image pixel per device pixel, ink on paper.
#[derive(Clone, Debug, PartialEq)]
pub struct RowImage {
    pub ppem: u16,
    pub image: Image,
    /// Why a glyph of the row was drawn unhinted, when one was.
    pub message: Option<String>,
}

/// What the rows show: the instance (a master id), the glyphs by name, the intent, the vertical
/// extent in font units (descender, ascender) and the units per em.
pub struct RowsRequest<'a> {
    pub instance: &'a str,
    pub glyphs: &'a [String],
    pub intent: &'a str,
    pub range: (f64, f64),
    pub upm: f64,
    pub style: Style,
}

/// Renders the rows from [`FIRST_PPEM`] to [`LAST_PPEM`]. Fails with the engine's message when
/// the instance cannot be built or a glyph cannot be rendered.
pub fn render_rows(session: &mut Session, req: &RowsRequest) -> Result<Vec<RowImage>, String> {
    if req.glyphs.is_empty() {
        return Ok(Vec::new());
    }
    if !(req.upm.is_finite() && req.upm > 0.0) {
        return Err("the font's units per em are not usable".into());
    }
    let sizes: Vec<u16> = (FIRST_PPEM..=LAST_PPEM).collect();
    let mut rows = Vec::with_capacity(sizes.len());
    for chunk in sizes.chunks(SIZES_PER_QUERY) {
        let params = json!({"instance": req.instance, "glyphs": req.glyphs, "ppem": chunk, "intent": req.intent, "output": "outline"});
        let reply = session.query("hint.tt.renderPreview", &params).map_err(|e| e.to_string())?;
        let Some(found) = reply.get("sizes").and_then(Value::as_array) else { return Err("the hinted preview returned no sizes".into()) };
        for size in found {
            rows.push(row(size, req)?);
        }
    }
    Ok(rows)
}

/// One size of a `hint.tt.renderPreview` reply as a row image.
fn row(size: &Value, req: &RowsRequest) -> Result<RowImage, String> {
    let ppem = size.get("ppem").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or("a size without `ppem`")?;
    let glyphs = size.get("glyphs").and_then(Value::as_array).ok_or("a size without glyphs")?;
    let mut line = Line::default();
    let mut message = None;
    for g in glyphs {
        if message.is_none() && g.get("hinted").and_then(Value::as_bool) == Some(false) {
            let name = g.get("glyph").and_then(Value::as_str).unwrap_or("?");
            let why = g.get("message").and_then(Value::as_str).unwrap_or("drawn unhinted");
            message = Some(format!("{name} at {ppem} PPM: {why}"));
        }
        if let Some(contours) = g.get("contours").and_then(Value::as_array) {
            for contour in contours {
                if let Some(points) = contour.as_array() {
                    line.contour(points)?;
                }
            }
        }
        let advance = g.get("advance").and_then(Value::as_f64).filter(|a| a.is_finite()).unwrap_or(0.0);
        line.x += advance.max(0.0);
        if line.x > MAX_ROW_PIXELS {
            break;
        }
    }
    let scale = f64::from(ppem) / req.upm;
    let (descender, ascender) = req.range;
    let mut top = (ascender * scale).ceil().max(1.0);
    let mut bottom = (descender * scale).floor().min(0.0);
    let (mut left, mut right) = (0.0_f64, line.x.ceil().max(1.0));
    if let Some([x0, y0, x1, y1]) = line.bounds {
        (left, right) = (left.min(x0.floor()), right.max(x1.ceil()));
        (bottom, top) = (bottom.min(y0.floor()), top.max(y1.ceil()));
    }
    let right = right.min(left + MAX_ROW_PIXELS);
    let bottom = bottom.max(top - MAX_ROW_PIXELS);
    if ![left, right, bottom, top].iter().all(|v| v.is_finite()) {
        return Err(format!("the row at {ppem} PPM has no finite size"));
    }
    let (width, height) = ((right - left).max(1.0) as u32, (top - bottom).max(1.0) as u32);
    let transform = Affine::new([1.0, 0.0, 0.0, -1.0, -left, top]);
    let image = render_path(&line.path, transform, width, height, &req.style).map_err(|e| e.to_string())?;
    Ok(RowImage { ppem, image, message })
}

/// A row's outlines in pixels, y up, from the line's origin.
#[derive(Default)]
struct Line {
    path: BezPath,
    /// The pen position: the advances so far.
    x: f64,
    points: usize,
    bounds: Option<[f64; 4]>,
}

impl Line {
    /// Adds one contour of `[x, y, onCurve]` points at the pen position. One off-curve point
    /// between on-curve points is a quadratic segment, two are a cubic one.
    fn contour(&mut self, points: &[Value]) -> Result<(), String> {
        self.points = self.points.saturating_add(points.len());
        if self.points > MAX_ROW_POINTS {
            return Err(format!("a row has more than {MAX_ROW_POINTS} points"));
        }
        let mut read = points.iter().filter_map(|p| {
            let p = p.as_array()?;
            let (x, y) = (p.first()?.as_f64()?, p.get(1)?.as_f64()?);
            let on = p.get(2).and_then(Value::as_bool).unwrap_or(true);
            (x.is_finite() && y.is_finite()).then(|| (Point::new(x + self.x, y), on))
        });
        let Some((start, true)) = read.next() else { return Ok(()) };
        let mut path = BezPath::new();
        let mut seen = vec![start];
        path.move_to(start);
        let mut off: Vec<Point> = Vec::with_capacity(2);
        for (p, on) in read {
            seen.push(p);
            if !on {
                if off.len() == 2 {
                    return Ok(());
                }
                off.push(p);
                continue;
            }
            match off.as_slice() {
                [] => path.line_to(p),
                [a] => path.quad_to(*a, p),
                [a, b] => path.curve_to(*a, *b, p),
                _ => return Ok(()),
            }
            off.clear();
        }
        match off.as_slice() {
            [a] => path.quad_to(*a, start),
            [a, b] => path.curve_to(*a, *b, start),
            _ => {}
        }
        path.close_path();
        self.path.extend(path.elements().iter().copied());
        for p in seen {
            let [x0, y0, x1, y1] = self.bounds.unwrap_or([p.x, p.y, p.x, p.y]);
            self.bounds = Some([x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)]);
        }
        Ok(())
    }
}

/// A row on screen: its texture and its size in device pixels.
#[derive(Clone)]
struct Row {
    ppem: u16,
    texture: TextureHandle,
    size: [u32; 2],
    warning: Option<String>,
}

#[derive(Clone, Default)]
struct Cache {
    key: Option<u64>,
    /// `egui::InputState::time` of the last build.
    built_at: f64,
    rows: Vec<Row>,
    message: Option<String>,
}

fn cache_id() -> egui::Id {
    egui::Id::new("fontcraft.hintingWaterfall")
}

/// The textures of the rows drawn last, in size order.
#[cfg(test)]
pub(crate) fn texture_ids(ctx: &egui::Context) -> Vec<egui::TextureId> {
    ctx.data(|d| d.get_temp::<Cache>(cache_id())).map(|c| c.rows.iter().map(|r| r.texture.id()).collect()).unwrap_or_default()
}

/// What the rows depend on.
fn key(app: &FontcraftApp, tab: &Tab) -> u64 {
    let doc = app.session.active().map(|d| (d.id, d.revision));
    let colours = (app.tokens.preview_ink.to_array(), app.tokens.preview_paper.to_array());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (doc, tab.id, tab.text.revision(), tab.edited_index(), &tab.master, &tab.layers, tab.direction, tab.kerning, intent(app), colours)
        .hash(&mut hasher);
    hasher.finish()
}

/// The glyphs of the tab's line that the font exports, at most [`MAX_GLYPHS`] around the
/// edited one.
fn glyph_names(app: &mut FontcraftApp, tab: &Tab) -> Vec<String> {
    let Some(layout) = preview::line_layout(&mut app.session, tab) else { return Vec::new() };
    let Ok(font) = app.session.font() else { return Vec::new() };
    let names: Vec<(usize, String)> = layout
        .placed
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.glyph.as_deref().filter(|g| font.glyph(g).is_some_and(|g| g.export)).map(|g| (i, g.to_string())))
        .collect();
    let edited = tab.edited_index().and_then(|e| names.iter().position(|(i, _)| *i >= e)).unwrap_or(0);
    let start = edited.saturating_sub(MAX_GLYPHS / 2).min(names.len().saturating_sub(MAX_GLYPHS));
    names.into_iter().skip(start).take(MAX_GLYPHS).map(|(_, g)| g).collect()
}

/// Builds the rows of `tab` from scratch.
fn build(app: &mut FontcraftApp, ctx: &egui::Context, tab: &Tab) -> (Vec<Row>, Option<String>) {
    let glyphs = glyph_names(app, tab);
    let Ok(font) = app.session.font() else { return (Vec::new(), None) };
    let Some(master) = view::tab_master(font, tab) else { return (Vec::new(), None) };
    let (instance, range, upm) = (master.id.clone(), preview::vertical_range(font, master), f64::from(font.units_per_em));
    let style = Style { ink: app.tokens.preview_ink.to_array(), background: app.tokens.preview_paper.to_array(), anti_alias: true };
    let req = RowsRequest { instance: &instance, glyphs: &glyphs, intent: intent(app), range, upm, style };
    match render_rows(&mut app.session, &req) {
        Ok(images) => {
            let rows = images
                .into_iter()
                .map(|r| {
                    let size = [r.image.width as usize, r.image.height as usize];
                    let color = ColorImage::from_rgba_premultiplied(size, &r.image.pixels);
                    let texture = ctx.load_texture(format!("hinting-waterfall-{}", r.ppem), color, TextureOptions::NEAREST);
                    Row { ppem: r.ppem, texture, size: [r.image.width, r.image.height], warning: r.message }
                })
                .collect();
            (rows, None)
        }
        Err(e) => (Vec::new(), Some(format!("The Hinting Preview cannot be drawn: {e}"))),
    }
}

/// The rows of `tab`, rebuilt when what they show changed, unless the pointer is down or the
/// last build was moments ago.
fn cached_rows(app: &mut FontcraftApp, ctx: &egui::Context, tab: &Tab) -> Cache {
    let key = key(app, tab);
    let mut cache = ctx.data(|d| d.get_temp::<Cache>(cache_id())).unwrap_or_default();
    if cache.key == Some(key) {
        return cache;
    }
    let (now, down) = ctx.input(|i| (i.time, i.pointer.any_down()));
    let shown = cache.key.is_some();
    let wait = REBUILD_INTERVAL - (now - cache.built_at);
    if shown && (down || wait > 0.0) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(0.0, REBUILD_INTERVAL).max(0.02)));
        return cache;
    }
    let (rows, message) = build(app, ctx, tab);
    cache = Cache { key: Some(key), built_at: now, rows, message };
    ctx.data_mut(|d| d.insert_temp(cache_id(), cache.clone()));
    cache
}

/// Draws the waterfall of `tab` in `rect`: one row per size with its size at the left, dragged
/// up and down to scroll; the context menu picks the scale.
pub fn draw(app: &mut FontcraftApp, ui: &mut egui::Ui, rect: Rect, tab: &Tab, pending: &mut Pending) {
    ui.painter_at(rect).rect_filled(rect, 0.0, app.tokens.preview_paper);
    let ctx = ui.ctx().clone();
    let cache = cached_rows(app, &ctx, tab);
    let scale = app.state.panels.hinting_waterfall.scale();
    let ppp = ctx.pixels_per_point().max(0.01);
    let unit = f32::from(scale) / ppp;
    let t = &app.tokens;
    let (margin, gap, label_width, text_size) = (t.preview_margin, t.preview_margin * 0.5, t.font_size * 2.5, t.font_size);
    let (label, warn, ink) = (t.text_dim, t.audit_warn, t.preview_ink);
    let widest = cache.rows.iter().map(|r| r.size[0] as f32 * unit).fold(0.0_f32, f32::max);
    let tall = cache.rows.iter().map(|r| r.size[1] as f32 * unit + gap).sum::<f32>();
    let content = vec2((widest + label_width + 2.0 * margin).max(rect.width()), (tall + 2.0 * margin).max(rect.height()));
    let snap = |v: f32| (v * ppp).round() / ppp;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        egui::ScrollArea::both()
            .id_salt("hintingWaterfall.scroll")
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource::ALL)
            .show(ui, |ui| {
                let (area, response) = ui.allocate_exact_size(content, Sense::click());
                let painter = ui.painter().clone();
                let clip = ui.clip_rect();
                let mut y = area.min.y + margin;
                for row in &cache.rows {
                    let size = vec2(row.size[0] as f32 * unit, row.size[1] as f32 * unit);
                    let at = Rect::from_min_size(pos2(snap(area.min.x + margin + label_width), snap(y)), size);
                    y += size.y + gap;
                    if !at.intersects(clip) {
                        continue;
                    }
                    painter.image(row.texture.id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    let colour = if row.warning.is_some() { warn } else { label };
                    let text = painter.text(
                        pos2(at.min.x - margin * 0.5, at.center().y),
                        egui::Align2::RIGHT_CENTER,
                        row.ppem.to_string(),
                        egui::FontId::proportional(text_size),
                        colour,
                    );
                    if let Some(why) = &row.warning {
                        ui.interact(text, widgets::id(("hintingWaterfall.warning", row.ppem)), Sense::hover()).on_hover_text(why);
                    }
                }
                if let Some(message) = &cache.message {
                    painter.text(area.min + vec2(margin, margin), egui::Align2::LEFT_TOP, message, egui::FontId::proportional(text_size), ink);
                }
                response.context_menu(|ui| {
                    for s in SCALES {
                        if ui.selectable_label(s == scale, format!("{s}×")).clicked() {
                            pending.push(("hint.tt.setPreviewScale", json!({ "scale": s })));
                            ui.close();
                        }
                    }
                });
            });
    });
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Shape};

    use super::*;

    /// A (a rectangle 50 to 550 by 0 to 700) and V (100 to 500), and a tab holding them.
    fn waterfall_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 50, "y": 0, "width": 500, "height": 700})).unwrap();
        app.run("path.addRectangle", &json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
        app.run("view.newTab", &json!({"glyphs": ["A", "V"]})).unwrap();
        app
    }

    /// Draws the waterfall of the showing tab in a 1280 × 800 window and returns the meshes'
    /// texture ids and rects.
    fn frame(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<(egui::TextureId, Rect)> {
        let tab = app.state.tab().cloned().unwrap();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut pending = Pending::new();
        let output = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            draw(app, ui, rect, &tab, &mut pending);
        });
        let mut out = Vec::new();
        let mut shapes: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        while let Some(shape) = shapes.pop() {
            match shape {
                Shape::Vec(list) => shapes.extend(list),
                Shape::Mesh(m) => {
                    let rect = Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
                    out.push((m.texture_id, rect));
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn hint_tt_set_preview_scale_takes_1_2_or_4_and_refuses_the_rest() {
        let mut app = FontcraftApp::default();
        assert_eq!(app.state.panels.hinting_waterfall.scale(), 1);
        assert_eq!(app.run("hint.tt.setPreviewScale", &json!({"scale": 2})).unwrap(), json!({"scale": 2}));
        assert_eq!(app.state.panels.hinting_waterfall.scale(), 2);
        for bad in [json!({"scale": 3}), json!({"scale": 2.5}), json!({}), json!({"scale": "4"})] {
            assert!(app.run("hint.tt.setPreviewScale", &bad).is_err(), "{bad} was accepted");
            assert_eq!(app.state.panels.hinting_waterfall.scale(), 2, "a refused {bad} changed the scale");
        }
        app.run("hint.tt.setPreviewScale", &json!({"scale": 4})).unwrap();
        assert_eq!(app.state.panels.hinting_waterfall.scale(), 4);
        // A stored scale that is not one of the three draws at 1x.
        assert_eq!(WaterfallState { scale: 3 }.scale(), 1);
    }

    #[test]
    fn preview_tt_hinting_waterfall_renders_every_size_from_8_to_80_ppm() {
        let mut app = waterfall_app();
        let font = app.session.font().unwrap();
        let master = font.masters.first().unwrap().id.clone();
        let upm = f64::from(font.units_per_em);
        let glyphs = vec!["A".to_string(), "V".to_string()];
        let style = Style { ink: [0, 0, 0, 255], background: [255, 255, 255, 255], anti_alias: true };
        let req = RowsRequest { instance: &master, glyphs: &glyphs, intent: "grayscale", range: (-200.0, 800.0), upm, style };
        let rows = render_rows(&mut app.session, &req).unwrap();
        let sizes: Vec<u16> = rows.iter().map(|r| r.ppem).collect();
        assert_eq!(sizes, (8..=80).collect::<Vec<u16>>());
        for r in &rows {
            let s = f64::from(r.ppem) / upm;
            // Two glyphs 600 units wide, and the rows reach from the descender to the ascender.
            assert!(f64::from(r.image.width) >= (1200.0 * s).floor(), "row {} is {} pixels wide", r.ppem, r.image.width);
            assert!(f64::from(r.image.height) >= (1000.0 * s).floor(), "row {} is {} pixels tall", r.ppem, r.image.height);
        }
        // At 80 PPM the middle of A (300, 350) is ink and the gap between A and V (575, 350) is
        // paper. The image's left edge is the origin and its top edge 64 pixels above the
        // baseline (800 units at 80 PPM).
        let row = rows.last().unwrap();
        let y = 64 - 28;
        assert!(row.image.pixel(24, y).unwrap()[0] < 64, "A is not filled at 80 PPM");
        assert!(row.image.pixel(46, y).unwrap()[0] > 192, "the space between A and V is filled at 80 PPM");
    }

    #[test]
    fn preview_tt_hinting_waterfall_draws_cached_rows_at_each_scale() {
        let mut app = waterfall_app();
        let ctx = egui::Context::default();
        let first = frame(&ctx, &mut app);
        let textures = texture_ids(&ctx);
        assert_eq!(textures.len(), 73, "one row per size from 8 to 80 PPM");
        let row8 = textures.first().copied().unwrap();
        let width_at = |meshes: &[(egui::TextureId, Rect)]| meshes.iter().find(|(t, _)| *t == row8).map(|(_, r)| r.width()).unwrap();
        let one = width_at(&first);
        for scale in [2, 4] {
            app.run("hint.tt.setPreviewScale", &json!({ "scale": scale })).unwrap();
            let meshes = frame(&ctx, &mut app);
            assert_eq!(texture_ids(&ctx), textures, "changing the scale rebuilt the rows");
            let w = width_at(&meshes);
            assert!((w - one * scale as f32).abs() < 0.5, "at {scale}x row 8 is {w} points wide, at 1x {one}");
        }
        // An edit rebuilds the rows once the pause between builds is over.
        app.run("path.addRectangle", &json!({"glyph": "V", "x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        let input = egui::RawInput { time: Some(REBUILD_INTERVAL * 4.0), ..Default::default() };
        let _ = ctx.run_ui(input, |_| {});
        frame(&ctx, &mut app);
        assert_ne!(texture_ids(&ctx), textures, "an edit did not rebuild the rows");
    }
}
