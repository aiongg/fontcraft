//! The value editor of each parameter kind ([`ParamKind`]): a checkbox (Bool), a number field
//! (Number), a text field (String), a multi-line list, one line per item (List), a key and value
//! table (Dict), and a language table (Localized). The editor returns a new value; the row
//! hands it to a command, which checks it (`fontcraft_fontinfo::value::check`).
//!
//! A one-line field commits on Return or when it loses the keyboard (the panel field rules); the
//! list commits when it loses the keyboard, as Return starts a new line. A value the editor of
//! its kind cannot show (a list of dictionaries, a nested dictionary entry) is shown as a short
//! read-only summary.
//!
//! Rows: `fontInfo.customParameterPicker`, `fontInfo.rows.localizable` (M7.15).

use egui::{Id, RichText};
use fontcraft_engine::fontinfo::ParamKind;
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_model::plist::{Array, Dict, Num, Number, Str, Value};

use crate::panels::widgets::{self, Look, NumberEdit};
use crate::theme::Tokens;

/// Most dictionary entries or languages one editor draws; the rest are counted.
pub const MAX_SHOWN_ENTRIES: usize = 256;
/// Most lines the list editor grows to before it scrolls.
const MAX_LIST_ROWS: usize = 8;
/// The text a field shows when the selected targets hold different values.
pub const MULTIPLE_VALUES: &str = "Multiple Values";

/// Draws the editor of a value of `kind`. Returns the new value when the user changed it (on
/// Enter or when the field loses focus, not on every keystroke).
pub fn editor(ui: &mut egui::Ui, tokens: &Tokens, kind: ParamKind, value: &Value) -> Option<Value> {
    let id = ui.id().with("fontInfo.value");
    editor_with_id(ui, tokens, id, kind, value, false)
}

/// [`editor`] with the id its fields are made from. `mixed`: the targets hold different values,
/// so the field shows [`MULTIPLE_VALUES`] instead of `value`.
pub fn editor_with_id(ui: &mut egui::Ui, tokens: &Tokens, id: Id, kind: ParamKind, value: &Value, mixed: bool) -> Option<Value> {
    let width = field_width(ui, tokens);
    match kind {
        ParamKind::Bool => {
            let mut on = !mixed && fv::flag(value).unwrap_or(false);
            let before = on;
            ui.push_id(id, |ui| ui.checkbox(&mut on, ""));
            (on != before).then_some(Value::Bool(on))
        }
        ParamKind::Number => match widgets::number(ui, id, if mixed { None } else { fv::number(value) }, width, false)? {
            NumberEdit::Set(n) => number_value(n).map(Value::Number),
            NumberEdit::Clear => None,
        },
        ParamKind::String => match plain_text(value) {
            Some(text) => text_field(ui, id, if mixed { "" } else { &text }, width, mixed).map(|t| Value::String(Str::new(t))),
            None => {
                summary(ui, tokens, value);
                None
            }
        },
        ParamKind::List => match fv::lines(value) {
            Some(lines) => {
                let shown = if mixed { String::new() } else { lines.join("\n") };
                multiline(ui, id, &shown, width).map(|t| Value::String(Str::new(t)))
            }
            None => {
                summary(ui, tokens, value);
                None
            }
        },
        ParamKind::Dict => dict_editor(ui, tokens, id, value, width, mixed),
        ParamKind::Localized => localized_editor(ui, tokens, id, value, width, mixed),
    }
}

/// The width of a value field: what is left of the row after the row's buttons.
pub fn field_width(ui: &egui::Ui, tokens: &Tokens) -> f32 {
    (ui.available_width() - 2.0 * tokens.list_row_height).max(tokens.field_width)
}

/// A number as a property list holds it: an integer when it is whole.
pub fn number_value(n: f64) -> Option<Number> {
    if !n.is_finite() {
        return None;
    }
    if n.fract() == 0.0 && n.abs() < 9.0e15 { Some(Number::int(n as i64)) } else { Number::float(n) }
}

/// A text or a number as one line of text; `None` for other values.
pub fn plain_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.as_str().to_string()),
        Value::Number(n) => Some(match n.value() {
            Num::Int(i) => i.to_string(),
            Num::Float(f) => widgets::format_number(f),
        }),
        Value::Bool(b) => Some(if *b { "1".into() } else { "0".into() }),
        _ => None,
    }
}

/// One line of text under the field rules; `mixed` shows the placeholder.
pub fn text_field(ui: &mut egui::Ui, id: Id, shown: &str, width: f32, mixed: bool) -> Option<String> {
    let look = Look { color: None, placeholder: mixed.then(|| RichText::new(MULTIPLE_VALUES)) };
    widgets::line(ui, id, shown, width, look, &|_, _| None).0
}

/// A text field of several lines. The typed text is kept while the field has the keyboard and
/// committed when it loses it, if it differs from `shown`.
pub fn multiline(ui: &mut egui::Ui, id: Id, shown: &str, width: f32) -> Option<String> {
    let kept = ui.data_mut(|d| d.get_temp::<String>(id));
    let mut text = kept.unwrap_or_else(|| shown.to_string());
    let rows = text.lines().count().clamp(1, MAX_LIST_ROWS);
    let response = ui.add(egui::TextEdit::multiline(&mut text).id(id).desired_width(width).desired_rows(rows));
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        return (text != shown).then_some(text);
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    None
}

/// A value no editor shows, as a greyed one-line summary.
pub(crate) fn summary(ui: &mut egui::Ui, tokens: &Tokens, value: &Value) {
    let text = match value {
        Value::Array(a) => format!("{} items", a.items.len()),
        Value::Dict(d) => format!("{} entries", d.entries.len()),
        Value::Data(_) => "data".to_string(),
        other => plain_text(other).unwrap_or_default(),
    };
    ui.label(RichText::new(text).color(tokens.text_dim));
}

/// One text field per entry whose value is a text or a number; the new dictionary when one
/// changed.
fn dict_editor(ui: &mut egui::Ui, tokens: &Tokens, id: Id, value: &Value, width: f32, mixed: bool) -> Option<Value> {
    let Value::Dict(dict) = value else {
        summary(ui, tokens, value);
        return None;
    };
    if mixed {
        ui.label(RichText::new(MULTIPLE_VALUES).color(tokens.text_dim));
        return None;
    }
    let mut changed = None;
    ui.vertical(|ui| {
        for (at, (key, entry)) in dict.entries.iter().enumerate().take(MAX_SHOWN_ENTRIES) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(key.as_str()).color(tokens.text_dim));
                let field = (width - tokens.list_column_unit).max(tokens.field_width);
                match plain_text(entry) {
                    Some(text) => {
                        if let Some(new) = text_field(ui, id.with(("entry", at)), &text, field, false) {
                            let typed = match entry {
                                Value::Number(_) => new.trim().parse::<f64>().ok().and_then(number_value).map(Value::Number),
                                _ => None,
                            };
                            changed = Some((at, typed.unwrap_or_else(|| Value::String(Str::new(new)))));
                        }
                    }
                    None => summary(ui, tokens, entry),
                }
            });
        }
        more(ui, tokens, dict.entries.len());
    });
    let (at, new) = changed?;
    let mut out = dict.clone();
    if let Some(slot) = out.entries.get_mut(at) {
        slot.1 = new;
    }
    Some(Value::Dict(out))
}

/// One text field per language; the new list of `{language, value}` entries when one changed.
fn localized_editor(ui: &mut egui::Ui, tokens: &Tokens, id: Id, value: &Value, width: f32, mixed: bool) -> Option<Value> {
    let Some(entries) = fv::localized(value) else {
        summary(ui, tokens, value);
        return None;
    };
    if mixed {
        ui.label(RichText::new(MULTIPLE_VALUES).color(tokens.text_dim));
        return None;
    }
    let mut changed = None;
    ui.vertical(|ui| {
        for (at, (language, text)) in entries.iter().enumerate().take(MAX_SHOWN_ENTRIES) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(language).color(tokens.text_dim));
                let field = (width - tokens.list_column_unit).max(tokens.field_width);
                if let Some(new) = text_field(ui, id.with(("language", at)), text, field, false) {
                    changed = Some((at, new));
                }
            });
        }
        more(ui, tokens, entries.len());
    });
    let (at, new) = changed?;
    let items = entries
        .iter()
        .enumerate()
        .map(|(i, (language, text))| {
            let mut d = Dict::new();
            d.entries.push((Str::new("language"), Value::String(Str::new(language.as_str()))));
            let text = if i == at { new.as_str() } else { text.as_str() };
            d.entries.push((Str::new("value"), Value::String(Str::new(text))));
            Value::Dict(d)
        })
        .collect();
    Some(Value::Array(Array { items, ..Default::default() }))
}

/// "and N more" under a table cut at [`MAX_SHOWN_ENTRIES`].
fn more(ui: &mut egui::Ui, tokens: &Tokens, total: usize) {
    if total > MAX_SHOWN_ENTRIES {
        ui.label(RichText::new(format!("and {} more", total - MAX_SHOWN_ENTRIES)).color(tokens.text_dim));
    }
}
