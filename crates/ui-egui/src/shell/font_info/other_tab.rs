//! The Other Settings tab: Grid Spacing, Subdivision, the Shift and Command keyboard increments,
//! Use Custom Naming, Disable Automatic Alignment, Keep Alternates Next to Base Glyph, File
//! Format Version and Font Type. It shows what `font.settings.get` answers (the defaults for a
//! font that does not set a value), and every change queues `font.settings.set` with that one
//! key.
//!
//! Rows: `fontInfo.other` (M7.16).

use egui::{Align2, Id, RichText, Sense, StrokeKind};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::panels::widgets::{self, NumberEdit};
use crate::theme::Tokens;

/// The label column's width, in list columns of the theme (the shared rows' width).
const LABEL_COLUMNS: f32 = 4.0;

/// The number fields: key, label, hover text.
const NUMBERS: [(&str, &str, &str); 2] = [
    ("gridSpacing", "Grid Spacing", "The grid points snap to, in font units; 0 keeps coordinates unrounded"),
    ("gridSubdivision", "Subdivision", "Divides the grid for finer steps"),
];

/// The keyboard increments: key, label, hover text.
const INCREMENTS: [(&str, &str, &str); 2] = [
    ("keyboardIncrementShift", "Shift", "How far arrow keys move or step values with Shift held"),
    ("keyboardIncrementCmd", "Command", "How far arrow keys move or step values with Command held"),
];

/// The checkboxes: key, label.
const FLAGS: [(&str, &str); 3] = [
    ("useCustomNaming", "Use Custom Naming"),
    ("disableAutomaticAlignment", "Disable Automatic Alignment"),
    ("keepAlternatesNextToBase", "Keep Alternates Next to Base Glyph"),
];

/// The File Format Version menu: value, label.
const FORMATS: [(i64, &str); 3] = [(2, "Version 2"), (3, "Version 3"), (4, "Version 4")];

/// The Font Type menu: value, label.
const FONT_TYPES: [(&str, &str); 2] = [("default", "Default"), ("variable", "Variable")];

/// The id of the control of the setting `key` (a `font.settings.set` key).
pub fn control_id(key: &str) -> Id {
    widgets::id(("fontInfo.other", key))
}

/// The command a new value of the setting `key` queues.
pub fn set_command(key: &str, value: Value) -> (&'static str, Value) {
    let mut values = serde_json::Map::new();
    values.insert(key.to_string(), value);
    ("font.settings.set", Value::Object(values))
}

/// A number as `font.settings.set` takes it: whole numbers as integers.
fn number_json(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1.0e15 { json!(n as i64) } else { json!(n) }
}

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Ok(settings) = app.session.execute("font.settings.get", &json!({})) else { return };
    let tokens = &app.tokens;
    let number = |key: &str| settings.get(key).and_then(Value::as_f64);
    ui.add_space(tokens.dialog_spacing);
    for (key, label, hover) in NUMBERS {
        row(ui, tokens, label, |ui| {
            if let Some(NumberEdit::Set(n)) = widgets::number(ui, control_id(key), number(key), tokens.field_width, false) {
                pending.push(set_command(key, number_json(n)));
            }
            ui.label(RichText::new(hover).color(tokens.text_dim));
        });
    }
    row(ui, tokens, "Keyboard Increments", |ui| {
        for (key, label, hover) in INCREMENTS {
            ui.label(label).on_hover_text(hover);
            if let Some(NumberEdit::Set(n)) = widgets::number(ui, control_id(key), number(key), tokens.field_width, false) {
                pending.push(set_command(key, number_json(n)));
            }
        }
    });
    for (key, label) in FLAGS {
        row(ui, tokens, "", |ui| {
            let on = settings.get(key).and_then(Value::as_bool).unwrap_or(false);
            if let Some(next) = checkbox(ui, control_id(key), on, label) {
                pending.push(set_command(key, json!(next)));
            }
        });
    }
    row(ui, tokens, "File Format Version", |ui| {
        let current = settings.get("fileFormatVersion").and_then(Value::as_i64).unwrap_or(3);
        let shown = FORMATS.iter().find(|(v, _)| *v == current).map_or_else(|| format!("Version {current}"), |(_, l)| (*l).to_string());
        let mut chosen = current;
        egui::ComboBox::from_id_salt(control_id("fileFormatVersion")).selected_text(shown).show_ui(ui, |ui| {
            for (value, label) in FORMATS {
                ui.selectable_value(&mut chosen, value, label);
            }
        });
        if chosen != current {
            pending.push(set_command("fileFormatVersion", json!(chosen)));
        }
    });
    row(ui, tokens, "Font Type", |ui| {
        let current = settings.get("fontType").and_then(Value::as_str).unwrap_or("default").to_string();
        let shown = FONT_TYPES.iter().find(|(v, _)| *v == current).map_or(current.as_str(), |(_, l)| *l).to_string();
        let mut chosen = current.clone();
        egui::ComboBox::from_id_salt(control_id("fontType")).selected_text(shown).show_ui(ui, |ui| {
            for (value, label) in FONT_TYPES {
                ui.selectable_value(&mut chosen, value.to_string(), label);
            }
        });
        if chosen != current {
            pending.push(set_command("fontType", json!(chosen)));
        }
    });
}

/// One row: the label right-aligned in the label column, then what `add` draws.
fn row(ui: &mut egui::Ui, tokens: &Tokens, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let size = egui::vec2(tokens.list_column_unit * LABEL_COLUMNS, tokens.list_row_height);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        if !label.is_empty() && ui.is_rect_visible(rect) {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let at = egui::pos2(rect.right() - tokens.dialog_spacing, rect.center().y);
            ui.painter().with_clip_rect(rect).text(at, Align2::RIGHT_CENTER, label, font, tokens.text);
        }
        add(ui);
    });
}

/// A checkbox with a stable id, drawn as egui draws its own. The new state when it was clicked.
fn checkbox(ui: &mut egui::Ui, id: Id, on: bool, label: &str) -> Option<bool> {
    let side = ui.spacing().icon_width;
    let gap = ui.spacing().icon_spacing;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, ui.visuals().text_color());
    let size = egui::vec2(side + gap + galley.size().x, ui.spacing().interact_size.y.max(galley.size().y));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        let square = egui::Rect::from_center_size(egui::pos2(rect.left() + side / 2.0, rect.center().y), egui::vec2(side, side));
        painter.rect(square.expand(visuals.expansion), visuals.corner_radius, visuals.bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        if on {
            let inner = square.shrink(side / 4.0);
            let tick = vec![
                egui::pos2(inner.left(), inner.center().y),
                egui::pos2(inner.center().x, inner.bottom()),
                egui::pos2(inner.right(), inner.top()),
            ];
            painter.add(egui::Shape::line(tick, visuals.fg_stroke));
        }
        let at = egui::pos2(square.right() + gap, rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, visuals.text_color());
    }
    response.clicked().then_some(!on)
}

#[cfg(test)]
#[path = "other_tab_tests.rs"]
mod tests;
