//! The Add Field picker of the Font Info window: the plus button beside a General heading opens
//! it. It lists the fields `font.info.listFields` offers for the tab's scope that the targets do
//! not have yet; a click selects one field, Cmd-click or Shift-click adds or removes one more, and
//! Add runs `font.info.addField` for each selected field, in list order. Cancel, Escape or a click
//! outside closes it and changes nothing.
//!
//! The picker keeps only view state; the Font Info window fills its list with the query
//! ([`FieldPicker::query`]) and runs what Add returns ([`FieldPicker::commands`]).
//!
//! Row: `fontInfo.fieldPicker` (M7.15).

use egui::{Id, RichText, Sense};
use serde_json::{Map, Value, json};

use crate::panels::widgets;
use crate::shell::font_info::rows::Target;
use crate::theme::Tokens;

/// The picker's width, in list columns of the theme.
pub(crate) const PICKER_COLUMNS: f32 = 9.0;
/// The rows the list shows before it scrolls.
pub(crate) const LIST_ROWS: f32 = 14.0;

/// How the person closed a picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Add: run the picker's commands.
    Add,
    Cancel,
}

/// One field the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldChoice {
    pub key: String,
    pub label: String,
    pub localizable: bool,
}

/// The picker's state.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldPicker {
    /// The font, masters or exports the fields are added to.
    pub target: Target,
    /// The fields offered; `None` until the list query has run.
    pub choices: Option<Vec<FieldChoice>>,
    /// Selected field keys.
    pub selected: Vec<String>,
    /// Why the list could not be made.
    pub error: Option<String>,
}

impl FieldPicker {
    pub fn new(target: Target) -> Self {
        Self { target, choices: None, selected: Vec::new(), error: None }
    }

    /// The parameters of the `font.info.listFields` query that fills the list. With one target
    /// the query marks what that target has; with several, what any of them has.
    pub fn query(&self) -> Value {
        let mut p = Map::new();
        p.insert("scope".into(), json!(self.target.scope.name()));
        if let [one] = self.target.indices.as_slice()
            && self.target.below_font()
        {
            p.insert("target".into(), json!(one));
        }
        Value::Object(p)
    }

    /// Takes the query's result: every field the targets do not have, in the query's order.
    pub fn fill(&mut self, result: &Value) {
        let fields = result.get("fields").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let choices = fields
            .iter()
            .filter(|f| !f.get("present").and_then(Value::as_bool).unwrap_or(false))
            .filter_map(|f| {
                Some(FieldChoice {
                    key: f.get("key")?.as_str()?.to_string(),
                    label: f.get("label").and_then(Value::as_str).unwrap_or_default().to_string(),
                    localizable: f.get("localizable").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        self.choices = Some(choices);
    }

    /// A click on `key`: alone it selects only that field; with `extend` it adds or removes it.
    pub fn click(&mut self, key: &str, extend: bool) {
        if !extend {
            self.selected = vec![key.to_string()];
        } else if let Some(at) = self.selected.iter().position(|k| k == key) {
            self.selected.remove(at);
        } else {
            self.selected.push(key.to_string());
        }
    }

    /// What Add runs: `font.info.addField` for each selected field, in list order.
    pub fn commands(&self) -> Vec<(&'static str, Value)> {
        let choices = self.choices.as_deref().unwrap_or_default();
        choices
            .iter()
            .filter(|c| self.selected.contains(&c.key))
            .map(|c| {
                let mut p = self.target.params();
                p.insert("key".into(), json!(c.key));
                ("font.info.addField", Value::Object(p))
            })
            .collect()
    }
}

/// The id of the picker's modal area.
pub fn modal_id() -> Id {
    widgets::id(("fontInfo.fieldPicker", "modal"))
}

/// The id of the list item of `key`.
pub fn item_id(key: &str) -> Id {
    widgets::id(("fontInfo.fieldPicker", "item", key))
}

/// The id of the Add button.
pub fn add_id() -> Id {
    widgets::id(("fontInfo.fieldPicker", "add"))
}

/// The id of the Cancel button.
pub fn cancel_id() -> Id {
    widgets::id(("fontInfo.fieldPicker", "cancel"))
}

/// Draws the picker over everything else. Returns how it was closed, if it was.
pub fn show(ctx: &egui::Context, tokens: &Tokens, picker: &mut FieldPicker) -> Option<Outcome> {
    let mut outcome = None;
    let modal = egui::Modal::new(modal_id()).show(ctx, |ui| {
        ui.set_width(tokens.list_column_unit * PICKER_COLUMNS);
        ui.label(RichText::new("Add Field").strong().size(tokens.dialog_title_size));
        ui.add_space(tokens.dialog_spacing);
        let extend = ui.input(|i| i.modifiers.command || i.modifiers.shift);
        let mut clicked = None;
        match (&picker.choices, &picker.error) {
            (_, Some(error)) => {
                ui.label(RichText::new(error.as_str()).color(tokens.error_text));
            }
            (None, None) => {
                ui.label(RichText::new("Loading…").color(tokens.text_dim));
            }
            (Some(choices), None) if choices.is_empty() => {
                ui.label(RichText::new("Every field is already there.").color(tokens.text_dim));
            }
            (Some(choices), None) => {
                egui::ScrollArea::vertical().id_salt("fontInfo.fieldPicker.list").max_height(tokens.list_row_height * LIST_ROWS).show(ui, |ui| {
                    for c in choices {
                        let text = if c.localizable { format!("{}  ({}, localizable)", c.label, c.key) } else { format!("{}  ({})", c.label, c.key) };
                        if item(ui, tokens, item_id(&c.key), &text, picker.selected.contains(&c.key)).clicked() {
                            clicked = Some(c.key.clone());
                        }
                    }
                });
            }
        }
        if let Some(key) = clicked {
            picker.click(&key, extend);
        }
        ui.add_space(tokens.dialog_spacing);
        ui.horizontal(|ui| {
            if widgets::button(ui, cancel_id(), "Cancel", "Close without adding", true, false) {
                outcome = Some(Outcome::Cancel);
            }
            if widgets::button(ui, add_id(), "Add", "Add the selected fields", !picker.selected.is_empty(), false) {
                outcome = Some(Outcome::Add);
            }
        });
        if !picker.selected.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            outcome = Some(Outcome::Add);
        }
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Cancel);
    }
    outcome
}

/// One line of a picker's list with a stable id; `selected` fills it with the selection colour.
pub fn item(ui: &mut egui::Ui, tokens: &Tokens, id: Id, text: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, tokens.list_row_height), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        } else if response.hovered() {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.hover);
        }
        let color = if selected { tokens.selected_text } else { tokens.text };
        let font = egui::TextStyle::Body.resolve(ui.style());
        let at = egui::pos2(rect.left() + tokens.dialog_spacing, rect.center().y);
        painter.with_clip_rect(rect).text(at, egui::Align2::LEFT_CENTER, text, font, color);
    }
    response
}
