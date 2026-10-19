//! The cell grid (`fontView.grid`).
//!
//! Each cell draws the glyph's layer of Font View's master (`fontView.cell.followsMaster`), its
//! placed image and colour drawing ([`super::cell_images`]), its colour labels, its badges and,
//! from [`LABEL_MIN_CELL`] up, the name and the Unicode marker (`fontView.cell.*`). Only the
//! lines in view are laid out and painted. One response covers the whole grid and the pointer is
//! hit-tested against the layout, so a font of any size costs the same per frame.
//!
//! | Pointer | Does |
//! |---|---|
//! | click | selects the glyph alone |
//! | Shift+click | adds the glyphs from the last clicked one to this one |
//! | Cmd+click | adds or removes the glyph |
//! | click on the name of a selected glyph | renames it in place (`glyph.rename`) |
//! | double-click | opens the glyph in Edit View |
//! | drag | selects the cells the rectangle touches (with Shift or Cmd: adds them) |
//! | click on empty space | clears the selection |

use std::collections::HashSet;
use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextFormat, Vec2, pos2, vec2};
use fontcraft_model::{ColorLabel, Font, Glyph};
use fontcraft_spacing::{Problem, SyncCache};
use serde_json::{Value, json};

use super::cache::{FontViewCache, Row};
use super::cell_images::{self, ArtFrame, CellArt};
use super::columns::Column;
use super::images::{Cell, CellImages, Frame};
use super::{InlineEdit, LABEL_MIN_CELL};
use crate::app::FontcraftApp;
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// A marquee drag in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    /// Where it started, relative to the grid's top left.
    origin: Pos2,
    /// The selection it adds to (empty unless Shift or Cmd was held).
    base: Vec<String>,
    /// The cells it covered last frame, as display positions.
    covered: Vec<usize>,
}

/// One line of the grid: a section header or a row of cells.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    Header(usize),
    Cells { start: usize, end: usize },
}

/// The grid's lines with their tops, for one width and cell size.
struct Layout {
    lines: Vec<(f32, Line)>,
    height: f32,
    step: f32,
    cell: Vec2,
}

impl Layout {
    fn new(count: usize, sections: &[(usize, String)], width: f32, cell_side: f32, label: f32, tokens: &Tokens) -> Layout {
        let gap = tokens.cell_gap;
        let step = cell_side + gap;
        let columns = (((width + gap) / step).floor() as usize).max(1);
        let cell = vec2(cell_side, cell_side + label);
        let row_height = cell.y + gap;
        let mut lines = Vec::new();
        let mut y = 0.0;
        let mut at = 0;
        let mut next_section = sections.iter().enumerate().peekable();
        while at < count || next_section.peek().is_some_and(|(_, (s, _))| *s <= count) {
            while let Some((i, _)) = next_section.next_if(|(_, (s, _))| *s <= at) {
                lines.push((y, Line::Header(i)));
                y += tokens.section_header_height;
            }
            if at >= count {
                break;
            }
            let limit = next_section.peek().map_or(count, |(_, (s, _))| (*s).min(count));
            let end = limit.min(at + columns).max(at + 1);
            lines.push((y, Line::Cells { start: at, end }));
            y += row_height;
            at = end;
        }
        Layout { lines, height: y, step, cell }
    }

    /// The lines that overlap `top..bottom`.
    fn lines_in(&self, top: f32, bottom: f32) -> &[(f32, Line)] {
        let first = self.lines.partition_point(|(y, line)| y + self.line_height(*line) < top);
        let last = self.lines.partition_point(|(y, _)| *y <= bottom);
        self.lines.get(first..last.max(first)).unwrap_or_default()
    }

    fn line_height(&self, line: Line) -> f32 {
        match line {
            Line::Header(_) => 0.0,
            Line::Cells { .. } => self.cell.y,
        }
    }

    /// The cell rectangle of display position `at` on a line starting at `y`.
    fn cell_rect(&self, origin: Pos2, y: f32, start: usize, at: usize) -> Rect {
        let col = at.saturating_sub(start) as f32;
        Rect::from_min_size(origin + vec2(col * self.step, y), self.cell)
    }

    /// The display position of the cell at `p` (relative to the grid's top left).
    fn hit(&self, p: Pos2) -> Option<usize> {
        let (y, line) = self.lines_in(p.y, p.y).iter().find(|(y, l)| p.y >= *y && p.y <= y + self.line_height(*l))?;
        let Line::Cells { start, end } = *line else { return None };
        if p.x < 0.0 {
            return None;
        }
        let col = (p.x / self.step).floor() as usize;
        let at = start + col;
        let rect = self.cell_rect(Pos2::ZERO, *y, start, at);
        (at < end && rect.contains(p)).then_some(at)
    }

    /// The display positions of the cells `r` touches (relative to the grid's top left).
    fn covered(&self, r: Rect) -> Vec<usize> {
        let mut out = Vec::new();
        for (y, line) in self.lines_in(r.min.y, r.max.y) {
            let Line::Cells { start, end } = *line else { continue };
            for at in start..end {
                if self.cell_rect(Pos2::ZERO, *y, start, at).intersects(r) {
                    out.push(at);
                }
            }
        }
        out
    }
}

/// What every cell of a frame shares.
struct CellPaint<'a> {
    font: &'a Font,
    doc: (u64, u64),
    master: &'a str,
    frame: Frame,
    tokens: &'a Tokens,
    labels: bool,
    display_unicode: bool,
    pixels_per_point: f32,
    /// The document's metrics keys that need attention (`Session::metrics_sync`).
    sync: Option<&'a SyncCache>,
    /// The glyphs whose layers cannot interpolate, while the font shows the indicators.
    incompatible: &'a HashSet<String>,
    /// Placed images and colour glyphs (`cell_images`).
    art: &'a ArtFrame,
}

/// Whether a metrics key of `glyph`, on any layer the scan covers, is out of sync
/// (`fontView.cell.badgeMetricsOutOfSync`). Unlinked keys and keys in error do not count.
pub fn metrics_out_of_sync(sync: &SyncCache, glyph: &str) -> bool {
    sync.glyph(glyph).iter().any(|f| matches!(f.problem, Problem::OutOfSync { .. }))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let FontcraftApp { session, state, tokens, font_view, .. } = app;
    let Some(doc) = session.active() else { return };
    let font = doc.font.clone();
    let doc_key = (doc.id, doc.revision);
    let assets = session.assets();
    let incompatible = if session.compat_indicators() { session.incompatible_glyphs() } else { HashSet::new() };
    // Scans again only the glyphs an edit touched and the glyphs whose keys read them.
    let sync = session.metrics_sync().ok();
    let fv = &state.font_view;
    let cache = &font_view.cache;
    let labels = fv.cell_size >= LABEL_MIN_CELL;
    let cell_side = fv.cell_size as f32;
    let label_height = if labels { tokens.cell_label_height } else { 0.0 };
    let width = ui.available_width();
    let layout = Layout::new(cache.visible.len(), cache.shown_sections(fv), width, cell_side, label_height, tokens);
    let selected: HashSet<&str> = state.selected_glyphs.iter().map(String::as_str).collect();
    let mods = ui.input(|i| i.modifiers);
    let art_frame = font_view.art.begin(ui.ctx(), assets, doc_key, &font, &cache.master, state.display.show_image, tokens);
    let paint = CellPaint {
        font: &font,
        doc: doc_key,
        master: &cache.master,
        frame: Frame::of(&font, &cache.master),
        tokens,
        labels,
        display_unicode: cache.display_unicode,
        pixels_per_point: ui.ctx().pixels_per_point(),
        sync,
        incompatible: &incompatible,
        art: &art_frame,
    };
    font_view.images.start_frame();
    let mut laid_out = Vec::new();
    let mut deferred = false;
    let mut edit = font_view.edit.take();
    let mut edit_shown = false;
    let images = &mut font_view.images;
    let art = &mut font_view.art;
    let drag = &mut font_view.drag;
    egui::ScrollArea::vertical().id_salt("fontView.grid").auto_shrink(false).show_viewport(ui, |ui, viewport| {
        let (rect, response) = ui.allocate_exact_size(vec2(width, layout.height.max(viewport.height())), Sense::click_and_drag());
        let origin = rect.min;
        let painter = ui.painter_at(ui.clip_rect());
        for &(y, line) in layout.lines_in(viewport.min.y, viewport.max.y) {
            match line {
                Line::Header(i) => {
                    let title = cache.sections.get(i).map_or("", |(_, t)| t.as_str());
                    let r = Rect::from_min_size(origin + vec2(0.0, y), vec2(width, tokens.section_header_height));
                    painter.text(r.left_center(), Align2::LEFT_CENTER, title, FontId::proportional(tokens.section_header_size), tokens.text);
                }
                Line::Cells { start, end } => {
                    for at in start..end {
                        let Some(row) = cache.visible_row(at) else { continue };
                        let r = layout.cell_rect(origin, y, start, at);
                        let is_selected = selected.contains(row.glyph.name.as_str());
                        laid_out.push((row.glyph.name.clone(), r));
                        deferred |= paint_cell(&painter, ui.ctx(), images, art, &paint, row, r, is_selected);
                        if let Some(e) = edit.as_mut().filter(|e| e.glyph == row.glyph.name && e.column == Column::Name) {
                            edit_shown = true;
                            if let Some(command) = name_field(ui, e, label_rect(r), row) {
                                pending.push(command);
                            }
                        }
                    }
                }
            }
        }
        pointer(ui, &response, origin, &layout, cache, &state.selected_glyphs, fv.anchor.as_deref(), mods, labels, drag, &mut edit, pending);
        if let Some(d) = drag.as_ref()
            && let Some(p) = response.interact_pointer_pos()
        {
            let r = Rect::from_two_pos(origin + d.origin.to_vec2(), p);
            painter.rect(r, CornerRadius::ZERO, tokens.marquee_fill, Stroke::new(1.0, tokens.marquee_stroke), StrokeKind::Inside);
        }
    });
    // A field scrolled out of view or finished (its glyph cleared) is dropped; a new one starts
    // next frame.
    font_view.edit = edit.filter(|e| !e.glyph.is_empty() && (edit_shown || e.fresh));
    cell_images::dropped(ui.ctx(), ui.max_rect(), &laid_out, &cache.master, pending);
    font_view.laid_out = laid_out;
    if deferred {
        ui.ctx().request_repaint();
    }
}

/// Clicks, double-clicks and marquee drags on the grid.
#[allow(clippy::too_many_arguments)]
fn pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    origin: Pos2,
    layout: &Layout,
    cache: &FontViewCache,
    selected: &[String],
    anchor: Option<&str>,
    mods: egui::Modifiers,
    labels: bool,
    drag: &mut Option<Drag>,
    edit: &mut Option<InlineEdit>,
    pending: &mut Pending,
) {
    let name_at = |at: usize| cache.visible_row(at).map(|r| r.glyph.name.clone());
    let adding = mods.shift || mods.command;
    if response.drag_started()
        && let Some(start) = ui.input(|i| i.pointer.press_origin())
    {
        *drag = Some(Drag { origin: (start - origin).to_pos2(), base: if adding { selected.to_vec() } else { Vec::new() }, covered: Vec::new() });
    }
    if response.dragged()
        && let Some(d) = drag.as_mut()
        && let Some(p) = response.interact_pointer_pos()
    {
        let covered = layout.covered(Rect::from_two_pos(d.origin, (p - origin).to_pos2()));
        if covered != d.covered {
            let mut glyphs = d.base.clone();
            let mut seen: HashSet<String> = glyphs.iter().cloned().collect();
            glyphs.extend(covered.iter().filter_map(|&at| name_at(at)).filter(|n| seen.insert(n.clone())));
            pending.push(("fontView.setSelection", json!({"glyphs": glyphs})));
            d.covered = covered;
        }
        // Near the top or bottom edge the grid scrolls along.
        let clip = ui.clip_rect();
        let margin = layout.cell.y / 2.0;
        if p.y < clip.min.y + margin {
            ui.scroll_with_delta(vec2(0.0, margin / 4.0));
        } else if p.y > clip.max.y - margin {
            ui.scroll_with_delta(vec2(0.0, -margin / 4.0));
        }
    }
    if response.drag_stopped() {
        *drag = None;
    }
    let Some(p) = response.interact_pointer_pos() else { return };
    let local = (p - origin).to_pos2();
    let hit = layout.hit(local);
    if response.double_clicked() {
        if let Some(name) = hit.and_then(name_at) {
            *edit = None;
            pending.push(("view.editSelectedGlyphs", json!({"glyphs": [name]})));
        }
        return;
    }
    if !response.clicked() {
        return;
    }
    let Some(at) = hit else {
        if !adding {
            pending.push(("fontView.setSelection", json!({"glyphs": []})));
        }
        return;
    };
    let Some(row) = cache.visible_row(at) else { return };
    let name = row.glyph.name.clone();
    let is_selected = selected.contains(&name);
    if mods.shift {
        let from = anchor.and_then(|a| cache.visible.iter().position(|&i| cache.rows.get(i).is_some_and(|r| r.glyph.name == a))).unwrap_or(at);
        // The anchor goes last, so it stays the anchor for the next Shift+click.
        let range: Vec<String> = if from <= at { (from..=at).rev().filter_map(name_at).collect() } else { (at..=from).filter_map(name_at).collect() };
        pending.push(("fontView.setSelection", json!({"glyphs": range, "mode": "add"})));
    } else if mods.command {
        pending.push(("fontView.setSelection", json!({"glyphs": [name], "mode": if is_selected { "remove" } else { "add" }})));
    } else if labels && is_selected && selected.len() == 1 && on_label(layout, local, at) {
        *edit = Some(InlineEdit { glyph: name.clone(), column: Column::Name, text: name, fresh: true });
    } else {
        pending.push(("fontView.setSelection", json!({"glyphs": [name]})));
    }
}

/// Whether `p` (relative to the grid) is on the name line of the cell at display position `at`.
fn on_label(layout: &Layout, p: Pos2, at: usize) -> bool {
    layout.lines_in(p.y, p.y).iter().any(|(y, line)| match *line {
        Line::Cells { start, end } if (start..end).contains(&at) => p.y >= y + layout.cell.x,
        _ => false,
    })
}

/// The name line under a cell.
fn label_rect(cell: Rect) -> Rect {
    Rect::from_min_max(pos2(cell.min.x, cell.min.y + cell.width()), cell.max)
}

/// The rename field over a cell's name. Returns the command to run when the edit is committed.
fn name_field(ui: &mut egui::Ui, edit: &mut InlineEdit, rect: Rect, row: &Row) -> Option<(&'static str, Value)> {
    let id = egui::Id::new(("fontView.rename", &edit.glyph));
    let response = ui.put(rect, egui::TextEdit::singleline(&mut edit.text).id(id).font(egui::TextStyle::Small).horizontal_align(egui::Align::Center));
    if edit.fresh {
        response.request_focus();
        edit.fresh = false;
        return None;
    }
    if !response.lost_focus() {
        return None;
    }
    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let text = edit.text.trim().to_string();
    edit.glyph.clear();
    if cancelled || text.is_empty() || text == row.glyph.name {
        return None;
    }
    Column::Name.edit_command(row, &text)
}

/// Draws one cell. Returns true when its outline, image or colour drawing waits for the next
/// frame.
#[allow(clippy::too_many_arguments)]
fn paint_cell(
    painter: &egui::Painter,
    ctx: &egui::Context,
    images: &mut CellImages,
    cell_art: &mut CellArt,
    c: &CellPaint,
    row: &Row,
    rect: Rect,
    selected: bool,
) -> bool {
    let t = c.tokens;
    let glyph = &row.glyph;
    let art = Rect::from_min_size(rect.min, Vec2::splat(rect.width()));
    let layer_color = glyph.layer(c.master).and_then(|l| l.info.color);
    painter.rect_filled(rect, CornerRadius::from(t.radius), t.canvas);
    paint_colors(painter, t, rect, glyph.info.color, layer_color);
    if selected && c.labels {
        painter.rect_filled(label_rect(rect), CornerRadius::ZERO, t.row_selected);
    }
    let mut deferred = false;
    let inner = art.shrink(t.cell_inset);
    let side = (inner.width() * c.pixels_per_point).round().max(1.0) as u32;
    let drawn = cell_art.paint(c.art, painter, c.font, glyph, inner);
    deferred |= drawn.deferred;
    if !drawn.colour {
        match images.get(ctx, c.doc, c.font, glyph, c.master, c.frame, side) {
            Cell::Ready(Some(texture)) => {
                painter.image(texture, inner, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), t.glyph_fill);
            }
            Cell::Ready(None) => {}
            Cell::Later => deferred = true,
        }
    }
    let out_of_sync = c.sync.is_some_and(|s| metrics_out_of_sync(s, &glyph.name));
    paint_badges(painter, t, art, glyph, c.master, row.props.has_custom_info, out_of_sync);
    if c.incompatible.contains(&glyph.name) {
        paint_incompatible(painter, t, rect);
    }
    if c.labels {
        paint_label(painter, c, label_rect(rect), glyph, selected);
    }
    if selected {
        painter.rect_stroke(rect, CornerRadius::from(t.radius), Stroke::new(t.cell_selection_width, t.accent), StrokeKind::Inside);
    }
    deferred
}

/// The colour of a label as a cell tint.
fn tint(t: &Tokens, label: ColorLabel) -> Color32 {
    let base = match label {
        ColorLabel::Index(i) => t.color_labels.get(usize::from(i)).copied().unwrap_or(t.unicode_marker),
        ColorLabel::Rgba([r, g, b, _]) => {
            let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            Color32::from_rgb(byte(r), byte(g), byte(b))
        }
    };
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), t.color_label_alpha)
}

/// Colour labels (`fontView.cell.colorGlyphOnly`, `colorLayerOnly`, `colorGlyphAndLayer`): the
/// glyph colour fills the cell; a layer colour alone fills it except a cut-out at the top
/// left; with both, the layer colour covers the right half.
fn paint_colors(painter: &egui::Painter, t: &Tokens, rect: Rect, glyph: Option<ColorLabel>, layer: Option<ColorLabel>) {
    match (glyph, layer) {
        (Some(g), None) => {
            painter.rect_filled(rect, CornerRadius::from(t.radius), tint(t, g));
        }
        (None, Some(l)) => {
            let cut = rect.width() * t.cell_cutout;
            let points = vec![rect.min + vec2(cut, 0.0), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.min + vec2(0.0, cut)];
            painter.add(Shape::convex_polygon(points, tint(t, l), Stroke::NONE));
        }
        (Some(g), Some(l)) => {
            painter.rect_filled(rect, CornerRadius::from(t.radius), tint(t, g));
            painter.rect_filled(Rect::from_min_max(pos2(rect.center().x, rect.min.y), rect.max), CornerRadius::ZERO, tint(t, l));
        }
        (None, None) => {}
    }
}

/// The badges (`fontView.cell.badge*`): annotation note top left, custom glyph info bottom
/// left, not exported top centre, out-of-sync metrics keys top right.
fn paint_badges(painter: &egui::Painter, t: &Tokens, art: Rect, glyph: &Arc<Glyph>, master: &str, custom: bool, out_of_sync: bool) {
    let s = t.badge_size;
    let pad = t.cell_inset / 2.0;
    if glyph.layer(master).is_some_and(|l| !l.info.annotations.is_empty()) {
        let r = Rect::from_min_size(art.min + Vec2::splat(pad), Vec2::splat(s));
        painter.rect_filled(r, CornerRadius::same(1), t.badge_note);
        let fold = s * 0.35;
        painter.add(Shape::convex_polygon(
            vec![r.right_bottom() - vec2(fold, 0.0), r.right_bottom() - vec2(0.0, fold), r.right_bottom()],
            t.canvas,
            Stroke::NONE,
        ));
    }
    if custom {
        let c = pos2(art.min.x + pad + s / 2.0, art.max.y - pad - s / 2.0);
        painter.circle_filled(c, s / 2.0, t.badge_info);
        painter.text(c, Align2::CENTER_CENTER, "i", FontId::proportional(s * 0.85), t.badge_info_text);
    }
    if !glyph.export {
        let c = pos2(art.center().x, art.min.y + pad + s / 2.0);
        let r = s / 2.0 - 0.75;
        let stroke = Stroke::new(1.5, t.badge_non_export);
        painter.circle_stroke(c, r, stroke);
        let d = r * std::f32::consts::FRAC_1_SQRT_2;
        painter.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
    }
    if out_of_sync {
        let r = Rect::from_min_size(pos2(art.max.x - pad - s, art.min.y + pad), Vec2::splat(s));
        painter.add(Shape::convex_polygon(vec![r.center_top(), r.right_bottom(), r.left_bottom()], t.font_view_badge_out_of_sync, Stroke::NONE));
        let mark = Stroke::new(1.25, t.font_view_badge_out_of_sync_mark);
        let x = r.center().x;
        painter.line_segment([pos2(x, r.top() + s * 0.35), pos2(x, r.top() + s * 0.7)], mark);
        painter.circle_filled(pos2(x, r.top() + s * 0.84), 0.75, t.font_view_badge_out_of_sync_mark);
    }
}

/// The red top-left corner of a glyph whose layers cannot interpolate
/// (`fontView.cell.badgeIncompatible`).
fn paint_incompatible(painter: &egui::Painter, t: &Tokens, rect: Rect) {
    let s = t.badge_size;
    painter.add(Shape::convex_polygon(vec![rect.min, rect.min + vec2(s, 0.0), rect.min + vec2(0.0, s)], t.compat_bad, Stroke::NONE));
}

/// CJK scripts, whose cells show the character itself as the Unicode marker
/// (`fontView.cell.unicodeIndicator`): Han, Hiragana, Katakana, Hangul and Bopomofo.
pub fn is_cjk(cp: u32) -> bool {
    matches!(cp,
        0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x3040..=0x30FF | 0x3100..=0x312F | 0x3130..=0x318F | 0x31A0..=0x31BF | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// What sits beside the name: the code point printed, the character, or a marker.
#[derive(Debug, PartialEq)]
pub enum UnicodeMark {
    None,
    Value(String),
    Char(char),
    Marker,
}

pub fn unicode_mark(glyph: &Glyph, display_value: bool, has_glyph: impl FnOnce(char) -> bool) -> UnicodeMark {
    let Some(&cp) = glyph.unicodes.first() else { return UnicodeMark::None };
    if display_value {
        return UnicodeMark::Value(format!("{cp:04X}"));
    }
    match char::from_u32(cp) {
        Some(ch) if is_cjk(cp) && has_glyph(ch) => UnicodeMark::Char(ch),
        _ => UnicodeMark::Marker,
    }
}

/// The name, truncated to fit, and the Unicode mark at the right.
fn paint_label(painter: &egui::Painter, c: &CellPaint, rect: Rect, glyph: &Glyph, selected: bool) {
    let t = c.tokens;
    let color = if selected { t.selected_text } else { t.cell_label };
    let value_font = FontId::proportional(t.cell_value_size);
    let label_font = FontId::proportional(t.cell_label_size);
    let mark = unicode_mark(glyph, c.display_unicode, |ch| painter.ctx().fonts_mut(|f| f.has_glyph(&label_font, ch)));
    let right = rect.max.x - t.cell_inset;
    let mark_width = match &mark {
        UnicodeMark::None => 0.0,
        UnicodeMark::Marker => {
            let s = t.unicode_marker_size;
            let r = Rect::from_center_size(pos2(right - s / 2.0, rect.center().y), Vec2::splat(s));
            painter.rect_filled(r, CornerRadius::same(1), t.unicode_marker);
            s + 2.0
        }
        UnicodeMark::Char(ch) => {
            let r = painter.text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, ch.to_string(), label_font.clone(), color);
            r.width() + 2.0
        }
        UnicodeMark::Value(v) => {
            let r = painter.text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, v, value_font, t.unicode_marker);
            r.width() + 2.0
        }
    };
    let room = (rect.width() - 2.0 * t.cell_inset - 2.0 * mark_width).max(1.0);
    let mut job = LayoutJob::single_section(glyph.name.clone(), TextFormat::simple(label_font, color));
    job.wrap = TextWrapping::truncate_at_width(room);
    let galley = painter.layout_job(job);
    let at = pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(at, galley, color);
}
