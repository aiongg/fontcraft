//! The Masters tab: the master list (drag to reorder, multiple selection, the plus menu, minus), the General, Metrics & Alignment Zones, Stems and Custom Parameters sections, Multiple Values placeholders, the Axes Coordinates and the read-only Number Values.
//!
//! The list on the left shows each master with its icon ([`crate::master_icons`]). A click
//! selects one master, Cmd-click adds or drops one, Shift-click selects a range; the sections on
//! the right edit every selected master at once, and a field whose masters disagree shows
//! Multiple Values until a value is typed. Dragging a master onto another moves it to that place
//! (`font.master.reorder`). The plus button opens a menu (Add Master, Add Other Font,
//! Duplicate Selected); the minus button removes the selected masters.
//!
//! Every control queues an engine command: `font.info.set` (the name), `font.master.setIcon`
//! (through [`crate::dialogs::master_icon_picker`]), `font.master.setMetric`,
//! `font.metrics.add/set/remove` (through [`crate::dialogs::metric_editor`]),
//! `font.master.setStem`, `font.stems.add/set/remove`, `font.customParameters.*`,
//! `font.master.add/duplicate/remove/reorder`, `font.master.setAxisLocation` (one number field
//! per axis, sent as a location list that sets that axis alone, so the right axis changes even
//! when two share a tag). The tab keeps only view state: the selection in
//! [`super::FontInfoState`], the open icon picker and metric editor in egui's memory.
//!
//! Number Values are shown read-only until their commands exist.
//!
//! Rows: `fontInfo.masters`, `fontInfo.masterIconPicker`, `fontInfo.metricEditor`,
//! `fontInfo.masters.multipleValues` (M7.17); `master.axesValues`,
//! `font.master.setAxisLocation` (M9.27).

#[cfg(test)]
mod tests;

use egui::{Align2, Id, RichText, Sense};
use fontcraft_engine::fontinfo::{MAX_MASTERS, MAX_METRIC_DEFS, MAX_STEM_DEFS};
use fontcraft_model::axes::design_value;
use fontcraft_model::{Font, metric};
use serde_json::json;

use super::FontInfoState;
use super::param_value::MULTIPLE_VALUES;
use super::rows::{self, Target};
use crate::app::FontcraftApp;
use crate::dialogs::master_icon_picker::{self, IconPicker, Picked};
use crate::dialogs::metric_editor::{self, MetricEditor, Outcome};
use crate::master_icons;
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// The master list's width, in list columns of the theme.
const LIST_COLUMNS: f32 = 4.0;
/// The label column of the sections, in list columns of the theme (as the shared rows).
const LABEL_COLUMNS: f32 = 4.0;
/// Most axes the Axes Coordinates list.
const MAX_AXES_SHOWN: usize = 64;

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let FontcraftApp { session, state, tokens, .. } = app;
    let tokens: &Tokens = tokens;
    let Ok(font) = session.font() else { return };
    let fi = &mut state.panels.font_info;
    let Some(target) = fi.target(font) else {
        ui.label(RichText::new("The font has no master.").color(tokens.text_dim));
        return;
    };
    let ctx = ui.ctx().clone();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(tokens.list_column_unit * LIST_COLUMNS);
            master_list(ui, tokens, fi, font, &target, pending);
        });
        ui.separator();
        ui.vertical(|ui| {
            general(ui, tokens, fi, font, &target, pending);
            axes(ui, tokens, font, &target, pending);
            metrics(ui, tokens, font, &target, pending);
            stems(ui, tokens, font, &target, pending);
            rows::parameters_section(ui, tokens, fi, font, &target, pending);
            numbers(ui, tokens, font, &target);
        });
    });
    dialogs(&ctx, tokens, pending);
    crate::dialogs::ttf_zones::show(&ctx, tokens, font, font.masters.get(target.first()).map(|m| m.id.as_str()), pending);
}

// ── Ids ─────────────────────────────────────────────────────────────────────────────────────────

/// The id of master `at` in the list; dragging it onto another master moves it there.
pub fn master_id(at: usize) -> Id {
    widgets::id(("fontInfo.masters", "master", at))
}

/// The id of the plus button under the list.
pub fn plus_id() -> Id {
    widgets::id(("fontInfo.masters", "plus"))
}

/// The id of the plus button's menu.
pub fn menu_id() -> Id {
    widgets::id(("fontInfo.masters", "menu"))
}

/// The id of an item of the plus menu: `add`, `addOtherFont`, `duplicate`.
pub fn menu_item_id(item: &str) -> Id {
    widgets::id(("fontInfo.masters", "menu", item))
}

/// The id of the minus button under the list.
pub fn minus_id() -> Id {
    widgets::id(("fontInfo.masters", "minus"))
}

/// The id of the icon button of the General section.
pub fn icon_id() -> Id {
    widgets::id(("fontInfo.masters", "icon"))
}

/// The id of the coordinate field of axis `at`.
pub fn axis_id(at: usize) -> Id {
    widgets::id(("fontInfo.masters", "axis", at))
}

/// The id of the plus button beside the Metrics heading.
pub fn metrics_plus_id() -> Id {
    widgets::id(("fontInfo.masters", "metrics", "plus"))
}

/// The id of a part of metric `at`'s row: `name`, `position`, `zone`, `minus`.
pub fn metric_id(at: usize, part: &str) -> Id {
    widgets::id(("fontInfo.masters", "metric", at, part))
}

/// The id of the plus button beside the Stems heading.
pub fn stems_plus_id() -> Id {
    widgets::id(("fontInfo.masters", "stems", "plus"))
}

/// The id of a part of stem `at`'s row: `orientation`, `name`, `value`, `actions`, `minus`.
pub fn stem_id(at: usize, part: &str) -> Id {
    widgets::id(("fontInfo.masters", "stem", at, part))
}

/// Where egui's memory keeps the open metric editor, the open icon picker, and the master a
/// Shift-click selects from.
fn editor_memory() -> Id {
    widgets::id(("fontInfo.masters", "metricEditor"))
}

fn icon_picker_memory() -> Id {
    widgets::id(("fontInfo.masters", "iconPicker"))
}

fn anchor_memory() -> Id {
    widgets::id(("fontInfo.masters", "anchor"))
}

/// The open metric editor, if any.
pub fn open_editor(ctx: &egui::Context) -> Option<MetricEditor> {
    ctx.data(|d| d.get_temp::<MetricEditor>(editor_memory()))
}

/// The open icon picker, if any.
pub fn open_icon_picker(ctx: &egui::Context) -> Option<IconPicker> {
    ctx.data(|d| d.get_temp::<IconPicker>(icon_picker_memory()))
}

// ── Shared values ───────────────────────────────────────────────────────────────────────────────

/// The value every target shares, or `None` when they differ (the field then shows Multiple
/// Values).
fn shared(mut values: impl Iterator<Item = f64>) -> Option<f64> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

/// The position and zone of metric `at` the targets share: a missing value reads as zero.
pub fn metric_values(font: &Font, target: &Target, at: usize) -> (Option<f64>, Option<f64>) {
    let value = |i: &usize| font.masters.get(*i).and_then(|m| m.metric_values.get(at)).copied().unwrap_or_default();
    (shared(target.indices.iter().map(|i| value(i).pos)), shared(target.indices.iter().map(|i| value(i).over)))
}

/// The width of stem `at` the targets share.
pub fn stem_value(font: &Font, target: &Target, at: usize) -> Option<f64> {
    shared(target.indices.iter().map(|i| font.masters.get(*i).and_then(|m| m.stem_values.get(at)).copied().unwrap_or_default()))
}

/// The coordinate on axis `at` the targets share; a short location reads the model's default.
pub fn location(font: &Font, target: &Target, at: usize) -> Option<f64> {
    shared(target.indices.iter().filter_map(|i| font.masters.get(*i)).map(|m| design_value(&m.location, at)))
}

/// The number value `at` the targets share.
fn number_value(font: &Font, target: &Target, at: usize) -> Option<f64> {
    shared(target.indices.iter().map(|i| font.masters.get(*i).and_then(|m| m.number_values.get(at)).copied().unwrap_or_default()))
}

// ── The master list ─────────────────────────────────────────────────────────────────────────────

/// A click on master `at`: alone it selects only that master, with `extend` it adds or drops
/// it, with `range` it selects from the master clicked last. The rows' selection is cleared.
pub fn select_master(ctx: &egui::Context, fi: &mut FontInfoState, target: &Target, count: usize, at: usize, extend: bool, range: bool) {
    let anchor = ctx.data(|d| d.get_temp::<usize>(anchor_memory())).filter(|a| *a < count);
    let mut selected = target.indices.clone();
    match (range, anchor) {
        (true, Some(anchor)) => selected.extend(anchor.min(at)..=anchor.max(at)),
        _ if extend => {
            match selected.iter().position(|i| *i == at) {
                Some(found) => {
                    selected.remove(found);
                }
                None => selected.push(at),
            }
            ctx.data_mut(|d| d.insert_temp(anchor_memory(), at));
        }
        _ => {
            selected = vec![at];
            ctx.data_mut(|d| d.insert_temp(anchor_memory(), at));
        }
    }
    selected.retain(|i| *i < count);
    selected.sort_unstable();
    selected.dedup();
    fi.selected_masters = selected;
    fi.clear_selection();
}

fn master_list(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    let count = font.masters.len();
    let side = tokens.toolbar_button;
    let width = ui.available_width();
    let mods = ui.input(|i| i.modifiers);
    let ctx = ui.ctx().clone();
    for (at, master) in font.masters.iter().enumerate().take(MAX_MASTERS) {
        let selected = target.indices.contains(&at);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, side), Sense::hover());
        let response = ui.interact(rect, master_id(at), Sense::click_and_drag());
        response.dnd_set_drag_payload(at);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let dragged = response.dragged();
            if selected || dragged {
                painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
            }
            if response.dnd_hover_payload::<usize>().is_some_and(|from| *from != at) {
                painter.hline(rect.x_range(), rect.top(), egui::Stroke::new(tokens.selection_width, tokens.selected_text));
            }
            let color = if selected { tokens.selected_text } else { tokens.text };
            let icon = egui::Rect::from_min_size(rect.min, egui::vec2(side, side));
            master_icons::paint_master(painter, icon, font, master, color);
            let text = egui::pos2(icon.right() + tokens.dialog_spacing, rect.center().y);
            let font_id = egui::TextStyle::Body.resolve(ui.style());
            painter.with_clip_rect(rect).text(text, Align2::LEFT_CENTER, master.name.as_str(), font_id, color);
        }
        if let Some(from) = response.dnd_release_payload::<usize>()
            && *from != at
            && *from < count
        {
            pending.push(("font.master.reorder", json!({"master": *from, "toIndex": at})));
            fi.selected_masters = vec![at];
            fi.clear_selection();
        } else if response.on_hover_text(master.name.as_str()).clicked() {
            select_master(&ctx, fi, target, count, at, mods.command, mods.shift);
        }
    }
    ui.add_space(tokens.dialog_spacing);
    ui.horizontal(|ui| {
        let plus = plain_button(ui, plus_id(), "+", "Add a master");
        egui::Popup::menu(&plus).id(menu_id()).show(|ui| {
            if widgets::button(ui, menu_item_id("add"), "Add Master", "Add an empty master with the default metrics", true, false) {
                pending.push(("font.master.add", json!({})));
                fi.selected_masters = vec![count];
                fi.clear_selection();
            }
            if widgets::button(ui, menu_item_id("addOtherFont"), "Add Other Font…", "Copy masters of the open fonts into this font", true, false) {
                pending.push(("dialog.open", json!({"dialog": "fontInfo.addOtherFont"})));
            }
            if widgets::button(
                ui,
                menu_item_id("duplicate"),
                "Duplicate Selected",
                "Copy the selected masters with their layers and kerning",
                true,
                false,
            ) {
                pending.push(("font.master.duplicate", json!({"masters": target.indices})));
            }
        });
        let removable = target.indices.len() < count;
        if widgets::button(ui, minus_id(), "−", "Remove the selected masters", removable, false) {
            pending.push(("font.master.remove", json!({"masters": target.indices})));
            fi.selected_masters.clear();
            fi.clear_selection();
        }
    });
}

/// A small push button with a stable id that gives its response (for the menu it opens).
fn plain_button(ui: &mut egui::Ui, id: Id, label: &str, hover: &str) -> egui::Response {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font_id, visuals.text_color());
    }
    response.on_hover_text(hover)
}

// ── Sections ────────────────────────────────────────────────────────────────────────────────────

/// A bold section heading without a plus button.
fn plain_heading(ui: &mut egui::Ui, tokens: &Tokens, title: &str) {
    ui.add_space(tokens.dialog_spacing);
    ui.label(RichText::new(title).strong().size(tokens.section_header_size));
}

/// The label column of a section row, right-aligned; `id` makes it clickable.
fn label_cell(ui: &mut egui::Ui, tokens: &Tokens, text: &str, id: Option<Id>) -> Option<egui::Response> {
    let size = egui::vec2(tokens.list_column_unit * LABEL_COLUMNS, tokens.list_row_height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = id.map(|id| ui.interact(rect, id, Sense::click()));
    if ui.is_rect_visible(rect) {
        let hovered = response.as_ref().is_some_and(egui::Response::hovered);
        let color = if hovered { tokens.selected_text } else { tokens.text };
        if hovered {
            ui.painter().rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        }
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let at = egui::pos2(rect.right() - tokens.dialog_spacing, rect.center().y);
        ui.painter().with_clip_rect(rect).text(at, Align2::RIGHT_CENTER, text, font_id, color);
    }
    response
}

/// A number field showing `value`, or Multiple Values when it is `None`. The committed number.
pub(super) fn number_field(ui: &mut egui::Ui, id: Id, value: Option<f64>, width: f32) -> Option<f64> {
    let shown = value.map(widgets::format_number).unwrap_or_default();
    let look = widgets::Look { color: None, placeholder: value.is_none().then(|| RichText::new(MULTIPLE_VALUES)) };
    let parse = |text: &str| text.trim().parse::<f64>().ok().filter(|n| n.is_finite());
    let step = |text: &str, delta: f64| parse(text).or(value).map(|v| widgets::format_number(v + delta));
    let text = widgets::line(ui, id, &shown, width, look, &step).0?;
    parse(&text)
}

/// A value shown read-only: the number, or Multiple Values.
fn read_only(ui: &mut egui::Ui, tokens: &Tokens, value: Option<f64>) {
    match value {
        Some(v) => ui.label(widgets::format_number(v)),
        None => ui.label(RichText::new(MULTIPLE_VALUES).color(tokens.text_dim)),
    };
}

/// General: the icon and the name.
fn general(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    plain_heading(ui, tokens, "General");
    rows::builtin_field(ui, tokens, fi, font, target, "name", pending);
    let Some(first) = font.masters.get(target.first()) else { return };
    let mixed = target.indices.iter().filter_map(|i| font.masters.get(*i)).any(|m| m.icon != first.icon);
    ui.horizontal(|ui| {
        label_cell(ui, tokens, "Icon", None);
        let side = tokens.toolbar_button;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
        let response = ui.interact(rect, icon_id(), Sense::click());
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);
            ui.painter().rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
            master_icons::paint_master(ui.painter(), rect, font, first, tokens.text);
        }
        if mixed {
            ui.label(RichText::new(MULTIPLE_VALUES).color(tokens.text_dim));
        }
        if response.on_hover_text("Choose the master's icon").clicked() {
            let current = if mixed { None } else { first.icon.clone() };
            let picker = IconPicker::new(target.indices.clone(), current);
            ui.ctx().data_mut(|d| d.insert_temp(icon_picker_memory(), picker));
        }
    });
}

/// Axes Coordinates: one number field per axis; a typed value goes to every selected master.
fn axes(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, target: &Target, pending: &mut Pending) {
    if font.axes.is_empty() {
        return;
    }
    plain_heading(ui, tokens, "Axes Coordinates");
    for (at, axis) in font.axes.iter().enumerate().take(MAX_AXES_SHOWN) {
        ui.horizontal(|ui| {
            label_cell(ui, tokens, &axis.name, None);
            if let Some(v) = number_field(ui, axis_id(at), location(font, target, at), tokens.field_width) {
                let location = super::exports_tab::location_list(at, v);
                pending.push(("font.master.setAxisLocation", json!({"masters": target.indices, "location": location})));
            }
        });
    }
}

/// Metrics & Alignment Zones: per metric its clickable name, position, zone and minus.
fn metrics(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, target: &Target, pending: &mut Pending) {
    if rows::heading(ui, tokens, metrics_plus_id(), "Metrics & Alignment Zones", "Add a metric") {
        let editor = MetricEditor::new_metric(font);
        ui.ctx().data_mut(|d| d.insert_temp(editor_memory(), editor));
    }
    for (at, def) in font.metrics.iter().enumerate().take(MAX_METRIC_DEFS) {
        ui.horizontal(|ui| {
            let label = def.name.clone().unwrap_or_else(|| metric_editor::category_label(&def.kind).to_string());
            let hover = match &def.filter {
                Some(scope) => format!("Edit the category, name and scope (scope: {scope})"),
                None => "Edit the category, name and scope".to_string(),
            };
            if let Some(r) = label_cell(ui, tokens, &label, Some(metric_id(at, "name")))
                && r.on_hover_text(hover).clicked()
                && let Some(editor) = MetricEditor::metric(font, at)
            {
                ui.ctx().data_mut(|d| d.insert_temp(editor_memory(), editor));
            }
            let (position, zone) = metric_values(font, target, at);
            if let Some(v) = number_field(ui, metric_id(at, "position"), position, tokens.field_width) {
                pending.push(("font.master.setMetric", json!({"masters": target.indices, "metric": at, "position": v})));
            }
            if def.kind != metric::ITALIC_ANGLE
                && let Some(v) = number_field(ui, metric_id(at, "zone"), zone, tokens.field_width)
            {
                pending.push(("font.master.setMetric", json!({"masters": target.indices, "metric": at, "zone": v})));
            }
            if widgets::flat_button(ui, metric_id(at, "minus"), "−", "Remove this metric from every master") {
                pending.push(("font.metrics.remove", json!({"metric": at})));
            }
        });
    }
}

/// Stems: per stem its orientation, name, width, actions (scope) and minus.
fn stems(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, target: &Target, pending: &mut Pending) {
    if rows::heading(ui, tokens, stems_plus_id(), "Stems", "Add a stem") {
        pending.push(("font.stems.add", json!({"orientation": "vertical"})));
    }
    for (at, def) in font.stems.iter().enumerate().take(MAX_STEM_DEFS) {
        ui.horizontal(|ui| {
            if orientation_button(ui, tokens, stem_id(at, "orientation"), def.horizontal) {
                let other = if def.horizontal { "vertical" } else { "horizontal" };
                pending.push(("font.stems.set", json!({"stem": at, "orientation": other})));
            }
            if let Some(name) = widgets::text(ui, stem_id(at, "name"), &def.name, tokens.list_column_unit * (LABEL_COLUMNS - 1.0)) {
                pending.push(("font.stems.set", json!({"stem": at, "name": name.trim()})));
            }
            if let Some(v) = number_field(ui, stem_id(at, "value"), stem_value(font, target, at), tokens.field_width) {
                pending.push(("font.master.setStem", json!({"masters": target.indices, "stem": at, "value": v})));
            }
            let hover = match &def.filter {
                Some(scope) => format!("Limit the stem to some glyphs (scope: {scope})"),
                None => "Limit the stem to some glyphs".to_string(),
            };
            if widgets::flat_button(ui, stem_id(at, "actions"), "…", &hover)
                && let Some(editor) = MetricEditor::stem(font, at)
            {
                ui.ctx().data_mut(|d| d.insert_temp(editor_memory(), editor));
            }
            if widgets::flat_button(ui, stem_id(at, "minus"), "−", "Remove this stem from every master") {
                pending.push(("font.stems.remove", json!({"stem": at})));
            }
        });
    }
}

/// A square button drawing a stem's orientation: an upright bar measured left to right
/// (vertical) or a flat bar measured top to bottom (horizontal). True when clicked.
fn orientation_button(ui: &mut egui::Ui, tokens: &Tokens, id: Id, horizontal: bool) -> bool {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let bar = egui::vec2(side * 0.25, side * 0.6);
        let bar = if horizontal { egui::vec2(bar.y, bar.x) } else { bar };
        painter.rect_filled(egui::Rect::from_center_size(rect.center(), bar), 0.0, tokens.text);
    }
    let hover = if horizontal {
        "Horizontal stem (measured top to bottom); click for vertical"
    } else {
        "Vertical stem (measured left to right); click for horizontal"
    };
    response.on_hover_text(hover).clicked()
}

/// Number Values, read-only.
fn numbers(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, target: &Target) {
    plain_heading(ui, tokens, "Number Values");
    if font.numbers.is_empty() {
        ui.label(RichText::new("None").color(tokens.text_dim));
    }
    for (at, number) in font.numbers.iter().enumerate() {
        ui.horizontal(|ui| {
            label_cell(ui, tokens, &number.name, None);
            read_only(ui, tokens, number_value(font, target, at));
        });
    }
}

// ── Dialogs ─────────────────────────────────────────────────────────────────────────────────────

/// Draws the open icon picker and metric editor; what they pick is queued.
fn dialogs(ctx: &egui::Context, tokens: &Tokens, pending: &mut Pending) {
    if let Some(mut picker) = open_icon_picker(ctx) {
        match master_icon_picker::show(ctx, tokens, &mut picker) {
            Some(picked) => {
                if picked != Picked::Cancel
                    && let Some(command) = picker.command(&picked)
                {
                    pending.push(command);
                }
                ctx.data_mut(|d| {
                    d.remove::<IconPicker>(icon_picker_memory());
                });
            }
            None => ctx.data_mut(|d| {
                d.insert_temp(icon_picker_memory(), picker);
            }),
        }
    }
    if let Some(mut editor) = open_editor(ctx) {
        match metric_editor::show(ctx, tokens, &mut editor) {
            Some(outcome) => {
                if outcome == Outcome::Add {
                    pending.extend(editor.commands());
                }
                ctx.data_mut(|d| {
                    d.remove::<MetricEditor>(editor_memory());
                });
            }
            None => ctx.data_mut(|d| {
                d.insert_temp(editor_memory(), editor);
            }),
        }
    }
}
