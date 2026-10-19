//! The Space Center window: the fields, the preview and the matrix. State, tokens and the
//! commands are in `crate::shell::space_center`.
//!
//! | Input | Does |
//! |---|---|
//! | text, before and after fields | `window.spaceCenter {text}`, `{before}`, `{after}` on every change |
//! | size, line mode | `window.spaceCenter {size}`, `{lineMode}` |
//! | Cut-Off, suffix field | `view.setSpacingCutOff {on}`, `view.setPreferredSuffix {suffix}` |
//! | click a glyph of the text | selects it (`window.spaceCenter {select}`) and gives the preview the keyboard |
//! | drag a glyph's left or right edge, or its inside | the margin, or the outline inside the width, by the distance dragged; with Alt in every master |
//! | Left, Right; Alt with them; Ctrl+Cmd with them | the selected glyph's right margin, left margin, outline (steps 1, Shift 10, Shift+Cmd 100) |
//! | click a matrix cell | edits it: Return or leaving confirms, Escape cancels, Up and Down step the value, Tab and Alt-arrows move to the next cell |
//!
//! The text is laid out with `textlayout` and kerned; only glyph outlines are filled, in one
//! texture of the area in view, which the cut-off band shows again in the inverse colours.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Align2, ColorImage, Event, Key, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::Font;
use fontcraft_render::{Style, render_path};
use fontcraft_spacing::kerning::KernIndex;
use fontcraft_textlayout::{KerningMode, Layout, Options, Text};
use serde_json::json;

use crate::app::FontcraftApp;
use crate::canvas::composite::CompositeCache;
use crate::panels::{Pending, widgets};
use crate::shell::space_center::{
    self as sc, Column, Context, Drag, Editing, LineMode, MAX_CELL_CHARS, MAX_GLYPHS_SHOWN, Resolved, Row, SpaceCenterState,
};
use crate::theme::Tokens;

/// Matrix columns drawn at most; the rest are reached through `window.spaceCenter {select}`.
const MAX_MATRIX_COLUMNS: usize = 1_000;

/// What the window keeps between frames: the resolved text, its layout and matrix values, the
/// drawn texture, a drag in progress, and what the last frame drew (tests read it).
#[derive(Default)]
pub struct Cache {
    key: Option<u64>,
    resolved: Option<Arc<Resolved>>,
    layout: Option<Arc<Layout>>,
    /// Width, left and right of each column.
    values: Arc<Vec<[Option<f64>; 3]>>,
    error: Option<String>,
    texture_key: Option<u64>,
    texture: Option<TextureHandle>,
    outlines: CompositeCache,
    drag: Option<(Drag, usize, f32)>,
    /// Why the last matrix entry was refused.
    pub matrix_error: Option<String>,
    pub drawn: Drawn,
}

// The cache is rebuilt from the state, so copies start empty and all caches compare equal.
impl Clone for Cache {
    fn clone(&self) -> Self {
        Cache::default()
    }
}

impl PartialEq for Cache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache").field("drawn", &self.drawn).finish_non_exhaustive()
    }
}

/// What the last frame drew.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawn {
    /// Glyphs drawn and their origins on the screen, in text order.
    pub glyphs: Vec<(Arc<str>, Pos2)>,
    /// The cut-off band of each line drawn, while it is on.
    pub bands: Vec<Rect>,
    /// The screen y of the first line's baseline, and points per font unit.
    pub baseline: Option<f32>,
    pub scale: f32,
    /// The matrix columns' glyphs and their width, left and right values.
    pub columns: Vec<String>,
    pub values: Vec<[Option<f64>; 3]>,
    /// The selected glyph's box.
    pub selected: Option<Rect>,
    pub lines: usize,
    pub master: Option<String>,
    pub message: Option<String>,
}

/// The id of the window.
pub fn window_id() -> egui::Id {
    egui::Id::new("fontcraft.spaceCenter")
}

/// The id of the preview area, which takes the keyboard for the arrow keys.
pub fn area_id() -> egui::Id {
    widgets::id(("spaceCenter", "area"))
}

fn cell_id(column: usize, row: Row) -> egui::Id {
    widgets::id(("spaceCenter", "cell", column, row))
}

/// Where the text sits on the screen: a font-unit point (x, y) of the layout is at
/// (`ax` + x·`scale`, `ay` − y·`scale`).
#[derive(Clone, Copy)]
struct Placement {
    ax: f32,
    ay: f32,
    scale: f32,
}

impl Placement {
    fn screen(self, x: f64, y: f64) -> Pos2 {
        pos2(self.ax + x as f32 * self.scale, self.ay - y as f32 * self.scale)
    }

    fn font(self, p: Pos2) -> (f64, f64) {
        let s = f64::from(self.scale.max(f32::MIN_POSITIVE));
        (f64::from(p.x - self.ax) / s, f64::from(self.ay - p.y) / s)
    }

    fn affine(self) -> Affine {
        let s = f64::from(self.scale);
        Affine::new([s, 0.0, 0.0, -s, f64::from(self.ax), f64::from(self.ay)])
    }
}

/// A matrix value as a cell shows it: whole numbers without decimals, others to two places.
pub fn format_value(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r}") }
}

/// The text item under a font-unit point of the layout: a glyph whose advance box holds it.
pub fn item_at(layout: &Layout, x: f64, y: f64) -> Option<usize> {
    let band = (layout.ascender - y) / layout.line_height;
    if !band.is_finite() || band < 0.0 {
        return None;
    }
    let line = layout.lines.get(band.floor() as usize)?;
    line.items.clone().find(|&i| layout.placed.get(i).is_some_and(|p| p.glyph.is_some() && x >= p.x && x < p.x + p.advance))
}

/// The matrix values of one glyph of the line.
pub type MatrixRow = [Option<f64>; 3];

/// Lays the window's text out for `master` at `line_width` font units, with the matrix values.
pub fn lay_out(
    font: &Font,
    master: &str,
    state: &SpaceCenterState,
    cx: &Context<'_>,
    line_width: f64,
    index: Option<&KernIndex>,
) -> Result<(Resolved, Layout, Vec<MatrixRow>), String> {
    let resolved = sc::resolve(font, master, state, cx);
    let mut text = Text::with_limit(MAX_GLYPHS_SHOWN).map_err(|e| e.to_string())?;
    text.insert(0, resolved.items.clone()).map_err(|e| e.to_string())?;
    let options = Options {
        kerning: KerningMode::Kerning,
        line_width,
        single_line: state.line_mode == LineMode::Single,
        index,
        layers: Some(&resolved.layers),
        ..Options::default()
    };
    let layout = fontcraft_textlayout::layout(font, master, &text, &options).map_err(|e| e.to_string())?;
    let values = resolved.columns.iter().map(|c| Row::ALL.map(|row| sc::value(font, &c.glyph, &c.layer, row))).collect();
    Ok((resolved, layout, values))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.space_center.open {
        return;
    }
    let master = sc::shown_master(app);
    let index = app.session.kern_index().ok();
    let doc = app.session.active().map(|d| (d.id, d.revision));
    let (current, selection) = sc::context(app);
    let FontcraftApp { session, state, tokens, .. } = app;
    let font = session.font().ok();
    let st = &mut state.panels.space_center;
    let mut open = true;
    let ctx = ui.ctx().clone();
    let shown = egui::Window::new("Space Center")
        .id(window_id())
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.text_preview_width, tokens.text_preview_height))
        .pivot(Align2::CENTER_TOP)
        .default_pos(pos2(ctx.content_rect().center().x, tokens.dialog_top))
        .show(&ctx, |ui| {
            fields(ui, st, tokens, pending);
            ui.add_space(tokens.dialog_spacing);
            match (font, master.as_deref(), doc) {
                (Some(font), Some(master), Some(doc)) => {
                    let cx = Context { current: current.as_deref(), selection: &selection };
                    preview(ui, st, tokens, font, master, doc, &cx, index.as_deref(), pending);
                    matrix(ui, st, tokens, font, master, pending);
                }
                _ => {
                    ui.label(egui::RichText::new("No font is open").color(tokens.text_dim));
                    st.cache.drawn = Drawn { message: Some("No font is open".into()), ..Drawn::default() };
                }
            }
        });
    // A press inside the window gives it the keyboard; a press anywhere else takes it away.
    if let (Some(shown), Some(at)) = (shown, ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten())) {
        st.keyboard = shown.response.rect.contains(at);
    }
    if !open {
        pending.push(("window.spaceCenter", json!({ "open": false })));
    }
}

fn fields(ui: &mut egui::Ui, st: &SpaceCenterState, tokens: &Tokens, pending: &mut Pending) {
    ui.horizontal(|ui| {
        let mut text = st.text.clone();
        let edit = egui::TextEdit::singleline(&mut text)
            .id(widgets::id(("spaceCenter", "text")))
            .char_limit(sc::MAX_TEXT_CHARS)
            .hint_text("Text")
            .desired_width(tokens.dialog_field_width * 4.0);
        if ui.add(edit).changed() {
            pending.push(("window.spaceCenter", json!({ "text": text })));
        }
        for (key, value, hint) in [("before", &st.before, "Before"), ("after", &st.after, "After")] {
            let mut v = value.clone();
            let edit = egui::TextEdit::singleline(&mut v)
                .id(widgets::id(("spaceCenter", key)))
                .char_limit(sc::MAX_TEXT_CHARS)
                .hint_text(hint)
                .desired_width(tokens.field_width);
            if ui.add(edit).changed() {
                pending.push(("window.spaceCenter", json!({ key: v })));
            }
        }
        let mut size = st.size;
        if ui.add(egui::DragValue::new(&mut size).range(sc::SIZE_RANGE).suffix(" pt")).changed() {
            pending.push(("window.spaceCenter", json!({ "size": size })));
        }
        for mode in [LineMode::Multi, LineMode::Single] {
            let label = if mode == LineMode::Multi { "Lines" } else { "One Line" };
            if ui.selectable_label(st.line_mode == mode, label).clicked() && st.line_mode != mode {
                pending.push(("window.spaceCenter", json!({ "lineMode": mode.name() })));
            }
        }
        let mut cut = st.cut_off.on;
        if ui.checkbox(&mut cut, "Cut-Off").changed() {
            pending.push(("view.setSpacingCutOff", json!({ "on": cut })));
        }
        let mut suffix = st.suffix.as_deref().unwrap_or("").trim_start_matches('.').to_string();
        let edit = egui::TextEdit::singleline(&mut suffix)
            .id(widgets::id(("spaceCenter", "suffix")))
            .char_limit(MAX_CELL_CHARS)
            .hint_text("Suffix")
            .desired_width(tokens.field_width);
        if ui.add(edit).changed() {
            pending.push(("view.setPreferredSuffix", json!({ "suffix": suffix })));
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn preview(
    ui: &mut egui::Ui,
    st: &mut SpaceCenterState,
    tokens: &Tokens,
    font: &Font,
    master: &str,
    doc: (u64, u64),
    cx: &Context<'_>,
    index: Option<&KernIndex>,
    pending: &mut Pending,
) {
    let pad = tokens.text_preview_padding;
    let avail = ui.available_size();
    let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    let matrix_h = row_h * 5.0 + ui.spacing().scroll.bar_width + tokens.dialog_spacing;
    let size = vec2(avail.x.max(pad * 8.0), (avail.y - matrix_h).max(pad * 8.0));
    let upm = f64::from(font.units_per_em.max(1));
    let scale = (st.size / upm) as f32;
    // The line length, in whole steps of 8 points so that resizing re-lays out only now and then.
    let span = (((size.x - 2.0 * pad) / 8.0).floor() * 8.0).max(8.0);
    let line_width =
        (f64::from(span) / f64::from(scale)).clamp(*fontcraft_textlayout::LENGTH_RANGE.start(), *fontcraft_textlayout::LENGTH_RANGE.end());

    let mut h = std::collections::hash_map::DefaultHasher::new();
    (doc, master, &st.text, &st.before, &st.after, &st.suffix, st.line_mode, line_width.to_bits(), cx.current, cx.selection).hash(&mut h);
    let key = h.finish();
    if st.cache.key != Some(key) {
        st.cache.key = Some(key);
        match lay_out(font, master, st, cx, line_width, index) {
            Ok((resolved, layout, values)) => {
                st.cache.error = resolved.message.clone();
                st.cache.resolved = Some(Arc::new(resolved));
                st.cache.layout = Some(Arc::new(layout));
                st.cache.values = Arc::new(values);
            }
            Err(e) => {
                st.cache.resolved = None;
                st.cache.layout = None;
                st.cache.values = Arc::default();
                st.cache.error = Some(e);
            }
        }
    }
    let mut drawn = Drawn { scale, master: Some(master.to_string()), message: st.cache.error.clone(), ..Drawn::default() };
    let (Some(layout), Some(resolved)) = (st.cache.layout.clone(), st.cache.resolved.clone()) else {
        if let Some(e) = &st.cache.error {
            ui.label(egui::RichText::new(e).color(tokens.error_text));
        }
        st.cache.drawn = drawn;
        return;
    };
    drawn.lines = layout.lines.len();
    drawn.columns = resolved.columns.iter().map(|c| c.glyph.clone()).collect();
    drawn.values = st.cache.values.to_vec();
    let lh = (layout.line_height as f32) * scale;
    let length = layout.lines.iter().map(|l| l.length).fold(0.0, f64::max) as f32 * scale;
    let content = vec2((length + 2.0 * pad).max(size.x), (pad * 2.0 + lh * layout.lines.len().max(1) as f32).max(size.y));
    let ppp = ui.ctx().pixels_per_point();
    let (bottom, top) = sc::band(font, master, &st.cut_off);
    let selected = st.selected.and_then(|i| resolved.columns.get(i)).map(|c| c.item);

    let scroll = egui::ScrollArea::both().id_salt(widgets::id(("spaceCenter", "scroll"))).auto_shrink([false, false]).max_height(size.y);
    scroll.show_viewport(ui, |ui, viewport| {
        ui.set_width(content.x);
        ui.set_height(content.y);
        let origin = ui.max_rect().min;
        let visible = viewport.translate(origin.to_vec2());
        let response = ui.interact(visible, area_id(), Sense::click_and_drag());
        let painter = ui.painter_at(visible);
        painter.rect_filled(visible, 0.0, tokens.canvas);
        let place = Placement { ax: origin.x + pad, ay: origin.y + pad + layout.ascender as f32 * scale, scale };
        drawn.baseline = layout.lines.first().map(|l| place.screen(0.0, l.baseline).y);
        let advance_box = |i: usize| -> Option<Rect> {
            let p = layout.placed.get(i)?;
            let line = layout.lines.get(p.line)?;
            Some(Rect::from_two_pos(
                place.screen(p.x, line.baseline + layout.ascender),
                place.screen(p.x + p.advance, line.baseline + layout.ascender - layout.line_height),
            ))
        };
        // The selected glyph's box, behind everything else.
        if let Some(r) = selected.and_then(advance_box) {
            painter.rect_filled(r, 0.0, tokens.text_selection);
            drawn.selected = Some(r);
        }
        // The glyphs' edges on each baseline, so the margins can be seen and grabbed.
        let edge = Stroke::new(tokens.metric_line_width, tokens.metric_line);
        for column in resolved.columns.iter() {
            let (Some(p), Some(r)) = (layout.placed.get(column.item), advance_box(column.item)) else { continue };
            if !visible.intersects(r) || p.glyph.is_none() {
                continue;
            }
            for x in [r.min.x, r.max.x] {
                painter.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], edge);
            }
        }
        let mut outline = BezPath::new();
        for p in layout.placed.iter() {
            let Some(name) = p.glyph.as_deref() else { continue };
            let Some(glyph) = font.glyph(name) else { continue };
            let layer_id = resolved.layers.get(name).map_or(master, String::as_str);
            let Some(layer) = glyph.layers.iter().find(|l| l.id == layer_id).or_else(|| glyph.layer(master)) else { continue };
            let at = place.screen(p.x, p.y);
            if at.x > visible.max.x + scale * upm as f32 || at.x < visible.min.x - scale * 2.0 * upm as f32 {
                continue;
            }
            if let Some(composite) = st.cache.outlines.get(doc, font, name, layer) {
                let mut g = composite.outline.clone();
                g.apply_affine(Affine::translate((p.x, p.y)));
                outline.extend(g.elements().iter().copied());
            }
            drawn.glyphs.push((Arc::from(name), at));
        }
        let texture = fill(ui, &mut st.cache, &outline, place, visible, ppp, key);
        if let Some(texture) = &texture {
            painter.image(texture.id(), visible, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tokens.glyph_fill);
        }
        // The cut-off band: glyph colour behind, the outlines in the canvas colour on top.
        if st.cut_off.on {
            for line in layout.lines.iter() {
                let (y0, y1) = (place.screen(0.0, line.baseline + top).y, place.screen(0.0, line.baseline + bottom).y);
                let band = Rect::from_min_max(pos2(visible.min.x, y0), pos2(visible.max.x, y1));
                if !band.intersects(visible) {
                    continue;
                }
                let band = band.intersect(visible);
                painter.rect_filled(band, 0.0, tokens.glyph_fill);
                if let Some(texture) = &texture {
                    painter.with_clip_rect(band).image(texture.id(), visible, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tokens.canvas);
                }
                drawn.bands.push(band);
            }
        }
        pointer(ui, &response, st, tokens, font, &layout, &resolved, place, pending);
    });
    keys(ui, st, &resolved, pending);
    st.cache.drawn = drawn;
}

/// Clicks select a glyph of the text; drags change a margin or move the outline.
#[allow(clippy::too_many_arguments)]
fn pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    st: &mut SpaceCenterState,
    tokens: &Tokens,
    font: &Font,
    layout: &Layout,
    resolved: &Resolved,
    place: Placement,
    pending: &mut Pending,
) {
    let column_at = |at: Pos2| -> Option<(usize, usize)> {
        let (x, y) = place.font(at);
        let item = item_at(layout, x, y)?;
        resolved.columns.iter().position(|c| c.item == item).map(|c| (c, item))
    };
    if response.clicked() || response.drag_started() {
        response.request_focus();
        st.editing = None;
        let hit = response.interact_pointer_pos().and_then(column_at);
        let select = hit.map(|(c, _)| c);
        if select != st.selected {
            pending.push(("window.spaceCenter", json!({ "select": select })));
        }
        if response.drag_started()
            && let (Some((column, item)), Some(at)) = (hit, response.interact_pointer_pos())
            && let Some(p) = layout.placed.get(item)
        {
            let grip = tokens.selection_ring_radius.max(1.0);
            let (left, right) = (place.screen(p.x, 0.0).x, place.screen(p.x + p.advance, 0.0).x);
            let drag = if (at.x - left).abs() <= grip {
                Drag::Left
            } else if (at.x - right).abs() <= grip {
                Drag::Right
            } else {
                Drag::Outline
            };
            st.cache.drag = Some((drag, column, at.x));
        }
    }
    // Where a drag in progress has got to.
    if st.cache.drag.is_some()
        && let Some(at) = response.interact_pointer_pos()
    {
        let painter = ui.painter_at(response.rect);
        painter.line_segment(
            [pos2(at.x, response.rect.min.y), pos2(at.x, response.rect.max.y)],
            Stroke::new(tokens.metric_line_width, tokens.metric_highlight),
        );
    }
    if response.drag_stopped()
        && let Some((drag, column, start)) = st.cache.drag.take()
        && let Some(at) = response.interact_pointer_pos().or_else(|| ui.ctx().pointer_latest_pos())
        && let Some(col) = resolved.columns.get(column)
    {
        let dx = f64::from(at.x - start) / f64::from(place.scale.max(f32::MIN_POSITIVE));
        let every = ui.input(|i| i.modifiers.alt);
        let layers = if every { master_layers(font, col) } else { vec![col.layer.clone()] };
        pending.extend(sc::drag_commands(drag, &col.glyph, &layers, dx));
    }
}

/// The glyph's master layers, for edits made in every master.
fn master_layers(font: &Font, col: &Column) -> Vec<String> {
    let Some(glyph) = font.glyph(&col.glyph) else { return Vec::new() };
    font.masters.iter().filter_map(|m| glyph.layer(&m.id).map(|l| l.id.clone())).collect()
}

/// The arrow keys while the preview has the keyboard and a glyph is selected.
fn keys(ui: &egui::Ui, st: &SpaceCenterState, resolved: &Resolved, pending: &mut Pending) {
    if st.editing.is_some() || !ui.memory(|m| m.has_focus(area_id())) {
        return;
    }
    ui.memory_mut(|m| m.set_focus_lock_filter(area_id(), egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() }));
    let Some(col) = st.selected.and_then(|i| resolved.columns.get(i)) else { return };
    let events = ui.input(|i| i.events.clone());
    for event in events {
        let Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
        let Some((id, amount)) = sc::key_command(key, modifiers) else { continue };
        ui.input_mut(|i| i.consume_key(modifiers, key));
        pending.push((id, json!({"glyphs": [col.glyph], "layer": col.layer, "amount": amount})));
    }
}

/// Fills `outline` in white into a texture of the area in view; the caller tints it.
fn fill(ui: &egui::Ui, cache: &mut Cache, outline: &BezPath, place: Placement, clip: Rect, ppp: f32, key: u64) -> Option<TextureHandle> {
    if outline.elements().is_empty() {
        return None;
    }
    let (w, h) = ((clip.width() * ppp).round(), (clip.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    for v in [place.ax, place.ay, place.scale, clip.min.x, clip.min.y, w, h, ppp] {
        v.to_bits().hash(&mut hasher);
    }
    let tkey = hasher.finish();
    if cache.texture_key != Some(tkey) {
        cache.texture_key = Some(tkey);
        let to_pixels = Affine::scale(f64::from(ppp)) * Affine::translate((-f64::from(clip.min.x), -f64::from(clip.min.y))) * place.affine();
        let style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };
        cache.texture = render_path(outline, to_pixels, w as u32, h as u32, &style).ok().map(|image| {
            let colour = ColorImage::from_rgba_premultiplied([w as usize, h as usize], &image.pixels);
            ui.ctx().load_texture("space-center-fill", colour, TextureOptions::LINEAR)
        });
    }
    cache.texture.clone()
}

/// The matrix: a column per glyph of the text, rows for the width and the margins.
fn matrix(ui: &mut egui::Ui, st: &mut SpaceCenterState, tokens: &Tokens, font: &Font, master: &str, pending: &mut Pending) {
    let Some(resolved) = st.cache.resolved.clone() else { return };
    let values = st.cache.values.clone();
    let shown = resolved.columns.len().min(MAX_MATRIX_COLUMNS);
    egui::ScrollArea::horizontal().id_salt(widgets::id(("spaceCenter", "matrix", "scroll"))).show(ui, |ui| {
        egui::Grid::new(widgets::id(("spaceCenter", "matrix"))).striped(true).min_col_width(tokens.field_width * 0.6).show(ui, |ui| {
            ui.label(egui::RichText::new("Glyph").color(tokens.text_dim));
            for (i, col) in resolved.columns.iter().take(shown).enumerate() {
                if ui.selectable_label(st.selected == Some(i), &col.glyph).clicked() {
                    pending.push(("window.spaceCenter", json!({ "select": i })));
                }
            }
            ui.end_row();
            for row in Row::ALL {
                ui.label(egui::RichText::new(row.label()).color(tokens.text_dim));
                for (i, col) in resolved.columns.iter().take(shown).enumerate() {
                    let value = values.get(i).and_then(|v| v.get(row.index()).copied().flatten());
                    cell(ui, st, tokens, font, master, &resolved, &values, i, col, row, value, pending);
                }
                ui.end_row();
            }
        });
    });
    if let Some(e) = &st.cache.matrix_error {
        ui.label(egui::RichText::new(e).color(tokens.error_text));
    }
}

fn shown_value(value: Option<f64>) -> String {
    value.map_or_else(String::new, format_value)
}

#[allow(clippy::too_many_arguments)]
fn cell(
    ui: &mut egui::Ui,
    st: &mut SpaceCenterState,
    tokens: &Tokens,
    font: &Font,
    master: &str,
    resolved: &Resolved,
    values: &[[Option<f64>; 3]],
    column: usize,
    col: &Column,
    row: Row,
    value: Option<f64>,
    pending: &mut Pending,
) {
    let editing_here = st.editing.as_ref().is_some_and(|e| e.column == column && e.row == row);
    if !editing_here {
        let text = value.map_or_else(|| "–".to_string(), format_value);
        if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
            let shown = shown_value(value);
            st.editing = Some(Editing { column, row, buffer: shown.clone(), original: shown, focused: false });
            st.cache.matrix_error = None;
        }
        return;
    }
    let id = cell_id(column, row);
    let focused = ui.memory(|m| m.has_focus(id));
    // Keys the cell handles itself, taken before the text field sees them.
    let mut confirm = false;
    let mut cancel = false;
    let mut moved: Option<(usize, Row)> = None;
    let mut stepped: Option<f64> = None;
    if focused {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            let Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
            match key {
                Key::Enter => confirm = true,
                Key::Escape => cancel = true,
                Key::ArrowUp | Key::ArrowDown if !modifiers.alt => {
                    stepped = Some(if key == Key::ArrowUp { sc::step(modifiers) } else { -sc::step(modifiers) });
                }
                _ => match sc::neighbour(column, row, key, modifiers, resolved.columns.len().min(MAX_MATRIX_COLUMNS)) {
                    Some(to) => moved = Some(to),
                    None => continue,
                },
            }
            ui.input_mut(|i| i.consume_key(modifiers, key));
        }
    }
    let Some(editing) = st.editing.as_mut() else { return };
    let response = ui.add(egui::TextEdit::singleline(&mut editing.buffer).id(id).char_limit(MAX_CELL_CHARS).desired_width(tokens.field_width * 0.6));
    // After the text field, which sets its own filter: the cell keeps the arrows, Tab and
    // Escape, so they reach the handling above on the next frame instead of moving focus.
    if response.has_focus() {
        let all = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: true, escape: true };
        ui.memory_mut(|m| m.set_focus_lock_filter(id, all));
    }
    if !editing.focused {
        response.request_focus();
        editing.focused = true;
    }
    if cancel {
        st.editing = None;
        return;
    }
    if let Some(delta) = stepped {
        let base = sc::entered_value(font, master, row, &editing.buffer).ok().or(value);
        if let Some(base) = base {
            let next = base + delta;
            match sc::matrix_command(font, master, col, row, &format_value(next)) {
                Ok(command) => {
                    pending.push(command);
                    editing.buffer = format_value(next);
                    editing.original = editing.buffer.clone();
                    st.cache.matrix_error = None;
                }
                Err(e) => st.cache.matrix_error = Some(e),
            }
        }
        return;
    }
    let leaving = response.lost_focus() && !confirm && moved.is_none();
    if confirm || moved.is_some() || leaving {
        let typed = editing.buffer.clone();
        let unchanged = typed.trim() == editing.original.trim();
        st.editing = None;
        if !unchanged && !typed.trim().is_empty() {
            match sc::matrix_command(font, master, col, row, &typed) {
                Ok(command) => {
                    pending.push(command);
                    st.cache.matrix_error = None;
                }
                Err(e) => {
                    st.cache.matrix_error = Some(e);
                    return;
                }
            }
        }
        if let Some((c, r)) = moved {
            let v = values.get(c).and_then(|v| v.get(r.index()).copied().flatten());
            let shown = shown_value(v);
            st.editing = Some(Editing { column: c, row: r, buffer: shown.clone(), original: shown, focused: false });
        }
    }
}
