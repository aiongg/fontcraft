//! The fields of a dialog: their kinds, how a value is checked, and how a field is drawn.
//!
//! A field edits one parameter of the dialog's command and is named after it: its `key` is the
//! parameter's key, the field's id for `ui.dialog.set` and `ui.click`.

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{Editing, Event, OpenDialog};
use crate::theme::Tokens;

/// Most fields one dialog may have (a schema read from the engine is capped by it).
pub const MAX_FIELDS: usize = 64;
/// Most choices one choice field may offer.
pub const MAX_CHOICES: usize = 256;
/// Longest text a number field takes.
pub const MAX_FIELD_TEXT: usize = 64;

/// What a field holds.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Kind {
    /// A number from `min` to `max`.
    Number { min: f64, max: f64 },
    /// A whole number from `min` to `max`.
    Integer { min: f64, max: f64 },
    /// A checkbox.
    Bool,
    /// One word of a list, as a pop-up menu. `labels`, when given, holds the text the menu shows
    /// for each word, in the same order.
    Choice {
        choices: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        labels: Vec<String>,
    },
}

/// One field: a parameter of the dialog's command.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Field {
    /// The parameter's key; also the field's id.
    pub key: String,
    pub label: String,
    #[serde(flatten)]
    pub kind: Kind,
    /// The value the dialog opens with.
    pub default: Value,
    /// One line for tooltips and agents.
    pub doc: String,
}

impl Field {
    /// A number field.
    pub fn number(key: &str, label: &str, min: f64, max: f64, default: f64, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Number { min, max }, default: json!(default), doc: doc.into() }
    }

    /// A checkbox.
    pub fn bool(key: &str, label: &str, default: bool, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Bool, default: json!(default), doc: doc.into() }
    }

    /// A pop-up menu of `(word, label)` pairs: the value is the word, the menu shows the label.
    pub fn choice(key: &str, label: &str, choices: &[(&str, &str)], default: &str, doc: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind: Kind::Choice {
                choices: choices.iter().map(|(w, _)| w.to_string()).collect(),
                labels: choices.iter().map(|(_, l)| l.to_string()).collect(),
            },
            default: json!(default),
            doc: doc.into(),
        }
    }

    /// A field that takes typed text (numbers).
    pub fn is_text(&self) -> bool {
        matches!(self.kind, Kind::Number { .. } | Kind::Integer { .. })
    }

    /// `value` as this field stores it, or why it does not fit. Number fields also take the text
    /// typed into them.
    pub fn check(&self, value: &Value) -> Result<Value, String> {
        let key = &self.key;
        match &self.kind {
            Kind::Number { min, max } | Kind::Integer { min, max } => {
                let n = match value {
                    Value::Number(n) => n.as_f64(),
                    Value::String(s) if s.len() <= MAX_FIELD_TEXT => s.trim().parse::<f64>().ok(),
                    _ => None,
                }
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("`{key}` must be a number"))?;
                let whole = matches!(self.kind, Kind::Integer { .. });
                if whole && n.fract() != 0.0 {
                    return Err(format!("`{key}` must be a whole number"));
                }
                if n < *min || n > *max {
                    return Err(format!("`{key}` must be from {} to {}", format_number(*min), format_number(*max)));
                }
                Ok(if whole { json!(n as i64) } else { json!(n) })
            }
            Kind::Bool => value.as_bool().map(Value::Bool).ok_or_else(|| format!("`{key}` must be true or false")),
            Kind::Choice { choices, .. } => match value.as_str() {
                Some(word) if choices.iter().any(|c| c == word) => Ok(json!(word)),
                _ => Err(format!("`{key}` must be one of {}", choices.join(", "))),
            },
        }
    }

    /// `value` one step up or down (`Up` and `Down` in a number field): by 1, or 10 with Shift.
    pub fn step(&self, value: &Value, up: bool, shift: bool) -> Option<Value> {
        let (Kind::Number { min, max } | Kind::Integer { min, max }) = self.kind else { return None };
        let size = if shift { 10.0 } else { 1.0 };
        let n = value.as_f64()? + if up { size } else { -size };
        Some(json!(n.clamp(min, max)))
    }
}

/// The fields of a filter, from its entry in `filter.list`.
pub fn filter_fields(entry: &Value) -> Result<Vec<Field>, String> {
    let params = entry.get("params").and_then(Value::as_array).ok_or("the filter's entry has no `params` list")?;
    if params.len() > MAX_FIELDS {
        return Err(format!("a dialog has at most {MAX_FIELDS} fields"));
    }
    params.iter().map(filter_field).collect()
}

fn filter_field(p: &Value) -> Result<Field, String> {
    let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let key = text("key");
    if key.is_empty() {
        return Err("a filter parameter has no `key`".into());
    }
    let bound = |name: &str, fallback: f64| p.get(name).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(fallback);
    let kind = match p.get("kind").and_then(Value::as_str) {
        Some("number") => Kind::Number { min: bound("min", f64::MIN), max: bound("max", f64::MAX) },
        Some("integer") => Kind::Integer { min: bound("min", f64::MIN), max: bound("max", f64::MAX) },
        Some("bool") => Kind::Bool,
        Some("choice") => {
            let list = p.get("choices").and_then(Value::as_array).ok_or_else(|| format!("`{key}` has no choices"))?;
            if list.is_empty() || list.len() > MAX_CHOICES {
                return Err(format!("`{key}` must have 1 to {MAX_CHOICES} choices"));
            }
            Kind::Choice { choices: list.iter().filter_map(Value::as_str).map(str::to_string).collect(), labels: Vec::new() }
        }
        other => return Err(format!("`{key}` has an unknown kind {other:?}")),
    };
    let label = Some(text("label")).filter(|l| !l.is_empty()).unwrap_or_else(|| key.clone());
    Ok(Field { key, label, kind, default: p.get("default").cloned().unwrap_or(Value::Null), doc: text("doc") })
}

/// The values a dialog opens with: each field's default, unless `given` sets it.
pub fn initial_values(fields: &[Field], given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut values = Map::new();
    for f in fields {
        let value = match given.get(&f.key).filter(|v| !v.is_null()) {
            Some(v) => f.check(v)?,
            None => f.default.clone(),
        };
        values.insert(f.key.clone(), value);
    }
    Ok(values)
}

/// A number as a field shows it: no trailing `.0`.
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// A value as its field shows it.
pub fn format_value(value: &Value) -> String {
    match value {
        Value::Number(n) => n.as_f64().map(format_number).unwrap_or_default(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// The egui id of a field's widget.
pub fn widget_id(key: &str) -> egui::Id {
    egui::Id::new(("fontcraft-dialog-field", key))
}

/// Draws the fields as a two-column grid of labels and widgets. What the person does becomes
/// [`Event`]s; the dialog applies them after the frame's drawing.
pub fn draw(ui: &mut egui::Ui, dialog: &OpenDialog, tokens: &Tokens, events: &mut Vec<Event>) {
    egui::Grid::new("fontcraft-dialog-fields").num_columns(2).spacing([tokens.dialog_spacing, tokens.dialog_spacing]).show(ui, |ui| {
        for field in &dialog.fields {
            let value = dialog.values.get(&field.key).cloned().unwrap_or(Value::Null);
            ui.label(&field.label);
            let response = match &field.kind {
                Kind::Number { .. } | Kind::Integer { .. } => text_field(ui, dialog, field, &value, tokens, events),
                Kind::Bool => {
                    let mut on = value.as_bool().unwrap_or(false);
                    let r = ui.add(egui::Checkbox::without_text(&mut on)).on_hover_text(&field.doc);
                    if r.changed() {
                        events.push(Event::Set(field.key.clone(), json!(on)));
                    }
                    r
                }
                Kind::Choice { choices, labels } => {
                    let current = value.as_str().unwrap_or_default();
                    egui::ComboBox::from_id_salt(widget_id(&field.key))
                        .selected_text(choice_label(choices, labels, current))
                        .show_ui(ui, |ui| {
                            for choice in choices {
                                if ui.selectable_label(choice == current, choice_label(choices, labels, choice)).clicked() {
                                    events.push(Event::Set(field.key.clone(), json!(choice)));
                                }
                            }
                        })
                        .response
                }
            };
            // Tab or a click moved the keyboard to a widget that takes no text: the field
            // being edited is done.
            if !field.is_text() && response.has_focus() && dialog.editing.is_some() {
                events.push(Event::CommitEdit);
            }
            ui.end_row();
        }
    });
}

/// The text a pop-up menu shows for `word`: its label, else the word.
fn choice_label<'a>(choices: &[String], labels: &'a [String], word: &'a str) -> &'a str {
    choices.iter().position(|c| c == word).and_then(|i| labels.get(i)).map_or(word, String::as_str)
}

/// A number field. Its text is the value, or what is being typed while it has the keyboard.
fn text_field(ui: &mut egui::Ui, dialog: &OpenDialog, field: &Field, value: &Value, tokens: &Tokens, events: &mut Vec<Event>) -> egui::Response {
    let id = widget_id(&field.key);
    let editing: Option<&Editing> = dialog.editing.as_ref().filter(|e| e.field == field.key);
    let mut text = editing.map_or_else(|| format_value(value), |e| e.text.clone());
    let response = ui.add(egui::TextEdit::singleline(&mut text).id(id).char_limit(MAX_FIELD_TEXT).desired_width(tokens.dialog_field_width));
    let response = response.on_hover_text(&field.doc);
    if response.has_focus() {
        if dialog.blur {
            response.surrender_focus();
        } else if editing.is_none() {
            events.push(Event::StartEdit(field.key.clone(), text));
        } else if response.changed() {
            events.push(Event::Text(text));
        }
    } else if response.lost_focus() {
        if editing.is_some() {
            events.push(Event::CommitEdit);
        }
    } else if editing.is_some() && !dialog.blur {
        // The keyboard came here without the mouse (Tab over the control channel, `ui.click`).
        response.request_focus();
    }
    response
}
