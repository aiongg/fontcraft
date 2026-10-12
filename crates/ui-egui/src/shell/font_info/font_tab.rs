//! The Font tab: the General heading with its plus button, Family Name with its character
//! warning, Units per Em with the scale button, Version with the stepper, Creation Date with the
//! update button, the added General fields, the read-only Axes and the font's Custom Parameters.
//!
//! Every control queues an engine command: `font.info.set` for the fields,
//! `font.info.scaleToUnitsPerEm` for the scale prompt, `font.info.stepVersion` for the stepper,
//! `font.info.updateDate` for the update button, and the shared rows' commands for the added
//! fields and the parameters. The scale prompt is view state kept in egui's memory: Cancel
//! closes it and runs nothing.
//!
//! Rows: `fontInfo.font`, `fontInfo.familyName.allowedCharacters` (the warning under the field)
//! (M7.16).

use egui::{Id, Label, RichText};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_model::Font;
use serde_json::json;

use super::rows::{self, ShownRow, Target, builtin_field, heading, plus_id};
use super::{FontInfoState, Picker};
use crate::app::FontcraftApp;
use crate::dialogs::field_picker::FieldPicker;
use crate::panels::Pending;
use crate::panels::widgets::{self, Look, NumberEdit};
use crate::theme::Tokens;

/// The label column's width, in list columns of the theme (the shared rows' width).
const LABEL_COLUMNS: f32 = 4.0;
/// Most axes the read-only Axes section lists.
const MAX_AXES_SHOWN: usize = 256;
/// The family name warning, under the field.
pub const FAMILY_NAME_WARNING: &str = "Only ASCII letters, digits and spaces are safe in a family name: other characters can keep the fonts from exporting or installing. Add a Family Names field for names in other scripts or languages.";

/// The scale prompt the double-arrow button opens: the units per em to scale the font to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScalePrompt {
    pub units_per_em: u16,
}

/// The id of the double-arrow button beside Units per Em.
pub fn scale_button_id() -> Id {
    widgets::id(("fontInfo.font", "scale"))
}

/// The id of the scale prompt's number field.
pub fn scale_field_id() -> Id {
    widgets::id(("fontInfo.font", "scale.field"))
}

/// The key the open scale prompt is kept under in egui's memory.
fn scale_prompt_key() -> Id {
    widgets::id(("fontInfo.font", "scale.prompt"))
}

/// The id of the scale prompt's Scale button.
pub fn scale_ok_id() -> Id {
    widgets::id(("fontInfo.font", "scale.ok"))
}

/// The id of the scale prompt's Cancel button.
pub fn scale_cancel_id() -> Id {
    widgets::id(("fontInfo.font", "scale.cancel"))
}

/// The id of the Version row's label.
pub fn version_label_id() -> Id {
    widgets::id(("fontInfo.font", "version"))
}

/// The id of the major version's field.
pub fn version_major_id() -> Id {
    widgets::id(("fontInfo.font", "version.major"))
}

/// The id of the minor version's field.
pub fn version_minor_id() -> Id {
    widgets::id(("fontInfo.font", "version.minor"))
}

/// The id of the stepper button that raises (`up`) or lowers the minor version.
pub fn version_step_id(up: bool) -> Id {
    widgets::id(("fontInfo.font", "version.step", up))
}

/// The id of the Creation Date row's label.
pub fn date_label_id() -> Id {
    widgets::id(("fontInfo.font", "date"))
}

/// The id of the Creation Date's field.
pub fn date_field_id() -> Id {
    widgets::id(("fontInfo.font", "date.field"))
}

/// The id of the Creation Date's update button.
pub fn date_update_id() -> Id {
    widgets::id(("fontInfo.font", "date.update"))
}

/// Whether `name` holds a character other than an ASCII letter, a digit or a space, the test
/// `font.info.set` warns on.
pub fn family_name_needs_warning(name: &str) -> bool {
    !name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')
}

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let FontcraftApp { session, state, tokens, .. } = app;
    let Ok(font) = session.font() else { return };
    let fi = &mut state.panels.font_info;
    let target = Target::font();
    if heading(ui, tokens, plus_id(Scope::Font, "general"), "General", "Add a field") {
        fi.picker = Some(Picker::Field(FieldPicker::new(target.clone())));
    }
    builtin_field(ui, tokens, fi, font, &target, "familyName", pending);
    if family_name_needs_warning(&font.family_name) {
        under_label(ui, tokens, |ui| {
            ui.add(Label::new(RichText::new(FAMILY_NAME_WARNING).color(tokens.audit_warn)).wrap());
        });
    }
    units_per_em(ui, tokens, fi, font, &target, pending);
    version(ui, tokens, fi, font, pending);
    date(ui, tokens, fi, font, pending);
    added_fields(ui, tokens, fi, font, &target, pending);
    axes(ui, tokens, font);
    rows::parameters_section(ui, tokens, fi, font, &target, pending);
}

/// The label column's width.
fn label_width(tokens: &Tokens) -> f32 {
    tokens.list_column_unit * LABEL_COLUMNS
}

/// Draws `add` in the value column, under the row above.
fn under_label(ui: &mut egui::Ui, tokens: &Tokens, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_space(label_width(tokens) + ui.spacing().item_spacing.x);
        ui.vertical(add);
    });
}

/// A row label like the shared rows draw: right-aligned in the label column, highlighted when
/// `selected`; its clicks select the row.
fn row_label(ui: &mut egui::Ui, tokens: &Tokens, id: Id, text: &str, selected: bool) -> egui::Response {
    rows::label_cell(ui, tokens, Some(id), text, selected, true)
}

/// Records the built-in field `key` as a shown row; its index.
fn record(fi: &mut FontInfoState, key: &str) -> usize {
    let index = fi.shown_rows.len();
    fi.shown_rows.push(ShownRow { scope: Scope::Font, kind: RowKind::Field, key: key.to_string(), occurrence: 0, built_in: true });
    index
}

/// Units per Em: the shared row, the double-arrow button, and the scale prompt it opens.
fn units_per_em(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    let prompt_id = scale_prompt_key();
    ui.horizontal_top(|ui| {
        builtin_field(ui, tokens, fi, font, target, "unitsPerEm", pending);
        if widgets::flat_button(ui, scale_button_id(), "↔", "Scale the whole font to another units per em") {
            let prompt = ScalePrompt { units_per_em: font.units_per_em };
            ui.data_mut(|d| d.insert_temp(prompt_id, prompt));
        }
    });
    let Some(mut prompt) = ui.data(|d| d.get_temp::<ScalePrompt>(prompt_id)) else { return };
    let mut close = false;
    under_label(ui, tokens, |ui| {
        ui.horizontal(|ui| {
            ui.label("Scale to");
            if let Some(NumberEdit::Set(n)) = widgets::number(ui, scale_field_id(), Some(f64::from(prompt.units_per_em)), tokens.field_width, false)
                && n.fract() == 0.0
                && (0.0..=f64::from(u16::MAX)).contains(&n)
            {
                prompt.units_per_em = n as u16;
            }
            ui.label("units per em");
            if widgets::button(ui, scale_ok_id(), "Scale", "Scale outlines, metrics and kerning to the new units per em", true, false) {
                pending.push(("font.info.scaleToUnitsPerEm", json!({"unitsPerEm": prompt.units_per_em})));
                close = true;
            }
            if widgets::button(ui, scale_cancel_id(), "Cancel", "Keep the font as it is", true, false) {
                close = true;
            }
        });
    });
    ui.data_mut(|d| {
        if close {
            d.remove::<ScalePrompt>(prompt_id);
        } else {
            d.insert_temp(prompt_id, prompt);
        }
    });
}

/// Version: the major and minor parts, separated by a period, with the stepper of the minor
/// part. The row stands for both built-in fields, so selecting it selects both.
fn version(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, pending: &mut Pending) {
    let major = record(fi, "versionMajor");
    let minor = record(fi, "versionMinor");
    let selected = fi.selected_rows.contains(&major);
    let mods = ui.input(|i| i.modifiers);
    let set = |key: &str, value: serde_json::Value| ("font.info.set", json!({"scope": "font", "values": {key: value}}));
    ui.horizontal_top(|ui| {
        if row_label(ui, tokens, version_label_id(), "Version", selected).clicked() {
            fi.select(major, mods.command, mods.shift);
            match (fi.selected_rows.contains(&major), fi.selected_rows.contains(&minor)) {
                (true, false) => fi.selected_rows.push(minor),
                (false, true) => fi.selected_rows.retain(|i| *i != minor),
                _ => {}
            }
        }
        let width = tokens.field_width;
        if let Some(NumberEdit::Set(n)) = widgets::number(ui, version_major_id(), Some(f64::from(font.version_major)), width, false) {
            // A fraction goes to the command as it is, which refuses it with a message.
            let value = if n.fract() == 0.0 && n.abs() <= f64::from(u16::MAX) { json!(n as i64) } else { json!(n) };
            pending.push(set("versionMajor", value));
        }
        ui.label(".");
        let shown = format!("{:03}", font.version_minor);
        // Arrows in the field step the minor version as the stepper does.
        let step = |text: &str, delta: f64| {
            let now = text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).unwrap_or(f64::from(font.version_minor));
            Some(format!("{:03}", (now + delta).round().max(0.0) as i64))
        };
        if let Some(text) = widgets::line(ui, version_minor_id(), &shown, width, Look::default(), &step).0 {
            // A text that is no number goes to the command, which refuses it with a message.
            let value = text.trim().parse::<u64>().map_or_else(|_| json!(text.trim()), |n| json!(n));
            pending.push(set("versionMinor", value));
        }
        if widgets::flat_button(ui, version_step_id(true), "+", "Raise the minor version by one") {
            pending.push(("font.info.stepVersion", json!({"delta": 1})));
        }
        if widgets::flat_button(ui, version_step_id(false), "−", "Lower the minor version by one") {
            pending.push(("font.info.stepVersion", json!({"delta": -1})));
        }
    });
}

/// Creation Date: the date as the file stores it, editable, with the update button that sets it
/// to now.
fn date(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, pending: &mut Pending) {
    let shown = font.info.date.clone().unwrap_or_default();
    // A font without a date has no row to copy.
    let index = font.info.date.is_some().then(|| record(fi, "date"));
    let selected = index.is_some_and(|i| fi.selected_rows.contains(&i));
    let mods = ui.input(|i| i.modifiers);
    ui.horizontal_top(|ui| {
        if row_label(ui, tokens, date_label_id(), "Creation Date", selected).clicked()
            && let Some(i) = index
        {
            fi.select(i, mods.command, mods.shift);
        }
        let width = (ui.available_width() - tokens.list_column_unit * 2.0).max(tokens.field_width);
        let look = Look { color: None, placeholder: Some(RichText::new("No date")) };
        if let Some(text) = widgets::line(ui, date_field_id(), &shown, width, look, &|_, _| None).0 {
            pending.push(("font.info.set", json!({"scope": "font", "values": {"date": text.trim()}})));
        }
        if widgets::button(ui, date_update_id(), "Update", "Set the creation date to now", true, false) {
            pending.push(("font.info.updateDate", json!({})));
        }
    });
}

/// The added General fields of the font, in stored order, with the shared row.
fn added_fields(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    rows::section_rows(ui, tokens, fi, font, target, RowKind::Field, pending);
}

/// The font's axes, read-only: name, tag and the Hidden checkbox.
fn axes(ui: &mut egui::Ui, tokens: &Tokens, font: &Font) {
    ui.add_space(tokens.dialog_spacing);
    ui.label(RichText::new("Axes").strong().size(tokens.section_header_size));
    if font.axes.is_empty() {
        under_label(ui, tokens, |ui| {
            ui.label(RichText::new("None").color(tokens.text_dim));
        });
    }
    for (at, axis) in font.axes.iter().enumerate().take(MAX_AXES_SHOWN) {
        ui.horizontal_top(|ui| {
            let size = egui::vec2(label_width(tokens), tokens.list_row_height);
            ui.add_sized(size, Label::new(RichText::new(axis.name.as_str()).color(tokens.text)).truncate());
            ui.label(RichText::new(axis.tag.as_str()).monospace().color(tokens.text));
            let mut hidden = axis.hidden;
            ui.push_id(("fontInfo.axis", at), |ui| ui.add_enabled(false, egui::Checkbox::new(&mut hidden, "Hidden")));
        });
    }
    if font.axes.len() > MAX_AXES_SHOWN {
        under_label(ui, tokens, |ui| {
            ui.label(RichText::new(format!("and {} more", font.axes.len() - MAX_AXES_SHOWN)).color(tokens.text_dim));
        });
    }
}

#[cfg(test)]
#[path = "font_tab_tests.rs"]
pub(crate) mod tests;
