//! List View (`fontView.list`): one row per visible glyph, one column per property
//! ([`super::columns`]).
//!
//! | Pointer | Does |
//! |---|---|
//! | header click | sorts by the column; again: reverses (`fontView.sortByColumn`) |
//! | header drag onto another header | moves the column there (`fontView.setColumns`) |
//! | header right-click | shows or hides columns (`fontView.setColumns`) |
//! | row click, Shift+click, Cmd+click | selects as in the grid |
//! | click on an editable cell of the only selected row, or double-click it | edits it in place |
//! | click on an Exports cell | flips the flag (`glyph.setExport`) |
//! | double-click on a cell that cannot be edited | opens the glyph in Edit View |
//!
//! Only the rows in view are laid out.

use std::collections::HashSet;

use egui::text::{LayoutJob, TextWrapping};
use egui::{CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextFormat, pos2, vec2};
use fontcraft_model::ColorLabel;
use serde_json::{Value, json};

use super::cache::Row;
use super::columns::{CellCx, Column, Editing};
use super::images::{Cell, CellImages, Frame};
use super::{FontViewState, InlineEdit};
use crate::app::FontcraftApp;
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// The columns shown with their left edges and widths, relative to the table's left.
pub(crate) fn placed(columns: &[Column], tokens: &Tokens) -> Vec<(Column, f32, f32)> {
    let mut x = 0.0;
    columns
        .iter()
        .map(|&c| {
            let w = c.width_units() * tokens.list_column_unit;
            let out = (c, x, w);
            x += w;
            out
        })
        .collect()
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let FontcraftApp { session, state, tokens, font_view, .. } = app;
    let Some(doc) = session.active() else { return };
    let font = doc.font.clone();
    let fv = &state.font_view;
    let cache = &font_view.cache;
    let shown = fv.shown_columns();
    let columns = placed(&shown, tokens);
    let total_width = columns.last().map_or(0.0, |(_, x, w)| x + w);
    let selected: HashSet<&str> = state.selected_glyphs.iter().map(String::as_str).collect();
    let mods = ui.input(|i| i.modifiers);
    let cx = CellCx { font: &font, master: &cache.master };
    font_view.list_images.start_frame();
    let mut outlines = Outlines {
        ctx: ui.ctx().clone(),
        images: &mut font_view.list_images,
        doc: (doc.id, doc.revision),
        frame: Frame::of(&font, &cache.master),
        pixels_per_point: ui.ctx().pixels_per_point(),
        deferred: false,
    };
    let mut edit = font_view.edit.take();
    let mut edit_shown = false;
    let mut laid_out = Vec::new();
    egui::ScrollArea::horizontal().id_salt("fontView.list.h").auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.vertical(|ui| {
            header(ui, fv, &columns, total_width, tokens, pending);
            let height = tokens.list_row_height;
            egui::ScrollArea::vertical().id_salt("fontView.list.v").auto_shrink(false).show_rows(ui, height, cache.visible.len(), |ui, range| {
                for at in range {
                    let Some(row) = cache.visible_row(at) else { continue };
                    let (rect, response) = ui.allocate_exact_size(vec2(total_width.max(ui.available_width()), height), Sense::click());
                    laid_out.push((row.glyph.name.clone(), rect));
                    let is_selected = selected.contains(row.glyph.name.as_str());
                    let painter = ui.painter_at(rect);
                    let fill = if is_selected {
                        tokens.row_selected
                    } else if at % 2 == 1 {
                        tokens.panel
                    } else {
                        tokens.panel_darker
                    };
                    painter.rect_filled(rect, CornerRadius::ZERO, fill);
                    for &(column, x, w) in &columns {
                        let cell = Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w, height));
                        if let Some(e) = edit.as_mut().filter(|e| e.glyph == row.glyph.name && e.column == column) {
                            edit_shown = true;
                            if let Some(command) = cell_field(ui, e, cell, row, cx.master) {
                                pending.push(command);
                            }
                            continue;
                        }
                        paint_cell(&painter, tokens, &cx, &mut outlines, row, column, cell, is_selected);
                    }
                    let hit = response
                        .interact_pointer_pos()
                        .and_then(|p| columns.iter().find(|(_, x, w)| p.x >= rect.min.x + x && p.x < rect.min.x + x + w))
                        .map(|c| c.0);
                    row_pointer(&response, row, at, hit, &cx, cache, &state.selected_glyphs, fv.anchor.as_deref(), mods, &mut edit, pending);
                }
            });
        });
    });
    if outlines.deferred {
        ui.ctx().request_repaint();
    }
    font_view.edit = edit.filter(|e| !e.glyph.is_empty() && (edit_shown || e.fresh));
    font_view.laid_out = laid_out;
}

/// What a Char cell needs to draw a glyph's outline in place of a character.
struct Outlines<'a> {
    ctx: egui::Context,
    images: &'a mut CellImages,
    doc: (u64, u64),
    frame: Frame,
    pixels_per_point: f32,
    /// An outline waits for the next frame.
    deferred: bool,
}

/// The header: labels, the sort arrow, sorting, dragging and the show/hide menu.
fn header(ui: &mut egui::Ui, fv: &FontViewState, columns: &[(Column, f32, f32)], width: f32, tokens: &Tokens, pending: &mut Pending) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), tokens.list_header_height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, tokens.app_bar);
    for &(column, x, w) in columns {
        let cell = Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w, rect.height()));
        let response = ui.interact(cell, egui::Id::new(("fontView.list.header", column.id())), Sense::click_and_drag());
        painter.line_segment([cell.right_top(), cell.right_bottom()], Stroke::new(1.0, tokens.border));
        let sorted = fv.sort.filter(|s| s.column == column);
        let text_rect = cell.shrink2(vec2(tokens.cell_inset, 0.0));
        let room = text_rect.width() - if sorted.is_some() { tokens.badge_size } else { 0.0 };
        text(&painter, text_rect, column.label(), FontId::proportional(tokens.cell_label_size + 1.0), tokens.text, room.max(1.0), false);
        if let Some(sort) = sorted {
            let c = pos2(cell.max.x - tokens.badge_size / 2.0 - tokens.cell_inset, cell.center().y);
            let h = tokens.badge_size / 3.0;
            let points = if sort.ascending {
                vec![c + vec2(-h, h / 2.0), c + vec2(h, h / 2.0), c + vec2(0.0, -h)]
            } else {
                vec![c + vec2(-h, -h / 2.0), c + vec2(h, -h / 2.0), c + vec2(0.0, h)]
            };
            painter.add(Shape::convex_polygon(points, tokens.text, Stroke::NONE));
        }
        if response.clicked() {
            pending.push(("fontView.sortByColumn", json!({"column": column.id()})));
        }
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            let ghost = Rect::from_center_size(pos2(p.x, cell.center().y), cell.size());
            ui.painter().rect(ghost, CornerRadius::ZERO, tokens.marquee_fill, Stroke::new(1.0, tokens.marquee_stroke), StrokeKind::Inside);
        }
        if response.drag_stopped()
            && let Some(p) = response.interact_pointer_pos()
            && let Some(&(target, _, _)) = columns.iter().find(|(_, x, w)| p.x >= rect.min.x + x && p.x < rect.min.x + x + w)
            && target != column
        {
            let mut order: Vec<Column> = columns.iter().map(|c| c.0).filter(|c| *c != column).collect();
            let at = order.iter().position(|c| *c == target).map_or(order.len(), |i| if p.x > cell.center().x { i + 1 } else { i });
            order.insert(at.min(order.len()), column);
            pending.push(("fontView.setColumns", json!({"order": order.iter().map(|c| c.id()).collect::<Vec<_>>()})));
        }
        response.context_menu(|ui| {
            let shown: Vec<Column> = fv.shown_columns();
            for c in Column::ALL {
                let mut on = shown.contains(&c);
                if ui.checkbox(&mut on, c.label()).changed() {
                    let visible: Vec<&str> = Column::ALL.iter().filter(|x| if **x == c { on } else { shown.contains(x) }).map(|x| x.id()).collect();
                    if !visible.is_empty() {
                        pending.push(("fontView.setColumns", json!({"visible": visible})));
                    }
                }
            }
        });
    }
}

/// Clicks on a row: selection, inline edits, opening the glyph.
#[allow(clippy::too_many_arguments)]
fn row_pointer(
    response: &egui::Response,
    row: &Row,
    at: usize,
    column: Option<Column>,
    cx: &CellCx,
    cache: &super::cache::FontViewCache,
    selected: &[String],
    anchor: Option<&str>,
    mods: egui::Modifiers,
    edit: &mut Option<InlineEdit>,
    pending: &mut Pending,
) {
    let name = row.glyph.name.clone();
    let editing = column.map_or(Editing::No, Column::editing);
    let start_edit = |edit: &mut Option<InlineEdit>, column: Column| {
        *edit = Some(InlineEdit { glyph: name.clone(), column, text: column.edit_text(cx, row), fresh: true });
    };
    if response.double_clicked() {
        match column {
            Some(c) if editing == Editing::Text => start_edit(edit, c),
            Some(_) if editing == Editing::Toggle => {}
            _ => pending.push(("view.editSelectedGlyphs", json!({"glyphs": [name]}))),
        }
        return;
    }
    if !response.clicked() {
        return;
    }
    let is_selected = selected.contains(&name);
    if mods.shift {
        let from = anchor.and_then(|a| cache.visible.iter().position(|&i| cache.rows.get(i).is_some_and(|r| r.glyph.name == a))).unwrap_or(at);
        let name_at = |i: usize| cache.visible_row(i).map(|r| r.glyph.name.clone());
        // The anchor goes last, so it stays the anchor for the next Shift+click.
        let range: Vec<String> = if from <= at { (from..=at).rev().filter_map(name_at).collect() } else { (at..=from).filter_map(name_at).collect() };
        pending.push(("fontView.setSelection", json!({"glyphs": range, "mode": "add"})));
        return;
    }
    if mods.command {
        pending.push(("fontView.setSelection", json!({"glyphs": [name], "mode": if is_selected { "remove" } else { "add" }})));
        return;
    }
    match (column, editing) {
        (Some(c), Editing::Toggle) => {
            if let Some(command) = c.edit_command(row, "") {
                pending.push(command);
            }
        }
        (Some(c), Editing::Text) if is_selected && selected.len() == 1 => start_edit(edit, c),
        _ => {}
    }
    if !(is_selected && selected.len() == 1) {
        pending.push(("fontView.setSelection", json!({"glyphs": [name]})));
    }
}

/// The field over a cell being edited. Returns the command to run when the edit is committed.
fn cell_field(ui: &mut egui::Ui, edit: &mut InlineEdit, rect: Rect, row: &Row, master: &str) -> Option<(&'static str, Value)> {
    let id = egui::Id::new(("fontView.listEdit", &edit.glyph, edit.column.id()));
    let response = ui.put(rect, egui::TextEdit::singleline(&mut edit.text).id(id).font(egui::TextStyle::Small));
    if edit.fresh {
        response.request_focus();
        edit.fresh = false;
        return None;
    }
    if !response.lost_focus() {
        return None;
    }
    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let text = edit.text.clone();
    let column = edit.column;
    edit.glyph.clear();
    if cancelled || (column == Column::Name && (text.trim().is_empty() || text.trim() == row.glyph.name)) {
        return None;
    }
    column.edit_command_in(Some(master), row, &text)
}

/// Draws one cell.
#[allow(clippy::too_many_arguments)]
fn paint_cell(painter: &egui::Painter, t: &Tokens, cx: &CellCx, outlines: &mut Outlines, row: &Row, column: Column, cell: Rect, selected: bool) {
    let inner = cell.shrink2(vec2(t.cell_inset, 0.0));
    let color = if selected {
        t.selected_text
    } else if column.is_derived(row) {
        t.text_dim
    } else {
        t.text
    };
    match column {
        Column::Export => {
            let s = t.badge_size;
            let r = Rect::from_center_size(inner.center(), vec2(s, s));
            painter.rect_stroke(r, CornerRadius::same(2), Stroke::new(1.0, color), StrokeKind::Inside);
            if row.glyph.export {
                let p = |x: f32, y: f32| r.min + vec2(x * s, y * s);
                painter.add(Shape::line(vec![p(0.2, 0.5), p(0.42, 0.75), p(0.8, 0.25)], Stroke::new(1.5, color)));
            }
        }
        Column::Color => {
            let mut room = inner.width();
            if let Some(label) = row.glyph.info.color {
                let s = t.badge_size;
                let swatch = Rect::from_min_size(pos2(inner.min.x, inner.center().y - s / 2.0), vec2(s, s));
                let fill = match label {
                    ColorLabel::Index(i) => t.color_labels.get(usize::from(i)).copied().unwrap_or(t.unicode_marker),
                    ColorLabel::Rgba([r, g, b, _]) => {
                        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                        egui::Color32::from_rgb(byte(r), byte(g), byte(b))
                    }
                };
                painter.rect_filled(swatch, CornerRadius::same(2), fill);
                room -= s + 4.0;
                let r = Rect::from_min_max(pos2(swatch.max.x + 4.0, inner.min.y), inner.max);
                text(painter, r, &column.text(cx, row), FontId::proportional(t.cell_label_size + 1.0), color, room.max(1.0), false);
            }
        }
        Column::Char => {
            let font = FontId::proportional(t.list_char_size);
            let value = column.text(cx, row);
            if value.chars().all(|c| painter.ctx().fonts_mut(|f| f.has_glyph(&font, c))) {
                text(painter, inner, &value, font, color, inner.width(), false);
            } else {
                // The interface font would show an empty box: the glyph's own outline stands in.
                let side = cell.height().min(inner.width()).max(1.0);
                let square = Rect::from_min_size(pos2(inner.min.x, cell.center().y - side / 2.0), vec2(side, side));
                let pixels = (side * outlines.pixels_per_point).round().max(1.0) as u32;
                match outlines.images.get(&outlines.ctx, outlines.doc, cx.font, &row.glyph, cx.master, outlines.frame, pixels) {
                    Cell::Ready(Some(texture)) => {
                        painter.image(texture, square, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), color);
                    }
                    Cell::Ready(None) => {}
                    Cell::Later => outlines.deferred = true,
                }
            }
        }
        _ => {
            let value = column.text(cx, row);
            text(painter, inner, &value, FontId::proportional(t.cell_label_size + 1.0), color, inner.width(), column.is_numeric());
        }
    }
}

/// One line of text in `rect`, truncated to `room`, vertically centred.
fn text(painter: &egui::Painter, rect: Rect, s: &str, font: FontId, color: egui::Color32, room: f32, right: bool) {
    if s.is_empty() {
        return;
    }
    let mut job = LayoutJob::single_section(s.to_string(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(room.max(1.0));
    let galley = painter.layout_job(job);
    let y = rect.center().y - galley.size().y / 2.0;
    let x = if right { rect.max.x - galley.size().x } else { rect.min.x };
    painter.galley(Pos2::new(x, y), galley, color);
}
