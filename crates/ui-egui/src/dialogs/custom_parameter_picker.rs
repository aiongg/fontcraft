//! The Add Custom Parameter picker of the Font Info window: the plus button beside a Custom
//! Parameters heading opens it. A search field narrows the list of known parameters allowed at
//! the tab's scope (`font.customParameters.describe`, which matches names, other spellings and
//! descriptions in any case); the list is in alphabetical order. A click selects one parameter
//! and shows its description under the list; Cmd-click or Shift-click adds or removes one more.
//! Add runs `font.customParameters.add` for each selected parameter, in list order, with its
//! kind's empty value. Cancel, Escape or a click outside closes it and changes nothing.
//!
//! The picker keeps only view state; the Font Info window runs the search query when the text
//! changes ([`ParameterPicker::needs_listing`]) and runs what Add returns.
//!
//! Row: `fontInfo.customParameterPicker` (M7.15).

use egui::{Id, RichText};
use serde_json::{Value, json};

pub use super::field_picker::Outcome;
use super::field_picker::{LIST_ROWS, PICKER_COLUMNS, item};
use crate::panels::widgets;
use crate::shell::font_info::rows::Target;
use crate::theme::Tokens;

/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 256;

/// One parameter the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamChoice {
    pub name: String,
    /// One line on what it does.
    pub summary: String,
    /// `bool`, `number`, `string`, `list`, `dict` or `localized`.
    pub kind: String,
    /// It sets a General field rather than a parameter row.
    pub property: bool,
}

/// The picker's state.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterPicker {
    /// The font, masters or exports the parameters are added to.
    pub target: Target,
    /// The search text.
    pub query: String,
    /// The search text the list was made for; `None` before the first listing.
    pub listed: Option<String>,
    pub choices: Vec<ParamChoice>,
    /// Selected parameter names.
    pub selected: Vec<String>,
    /// The parameter whose description shows: the last one clicked.
    pub described: Option<String>,
    /// Why the list could not be made.
    pub error: Option<String>,
}

impl ParameterPicker {
    pub fn new(target: Target) -> Self {
        Self { target, query: String::new(), listed: None, choices: Vec::new(), selected: Vec::new(), described: None, error: None }
    }

    /// Whether the list is older than the search text.
    pub fn needs_listing(&self) -> bool {
        self.listed.as_deref() != Some(self.query.as_str())
    }

    /// The parameters of the `font.customParameters.describe` query that fills the list.
    pub fn query_params(&self) -> Value {
        json!({"scope": self.target.scope.name(), "query": self.query})
    }

    /// Takes the query's result, sorted by name in any case. The selection keeps only names the
    /// new list holds.
    pub fn fill(&mut self, result: &Value) {
        let found = result.get("parameters").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut choices: Vec<ParamChoice> = found
            .iter()
            .filter_map(|p| {
                Some(ParamChoice {
                    name: p.get("name")?.as_str()?.to_string(),
                    summary: p.get("summary").and_then(Value::as_str).unwrap_or_default().to_string(),
                    kind: p.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
                    property: p.get("property").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
        self.selected.retain(|s| choices.iter().any(|c| c.name == *s));
        self.choices = choices;
        self.listed = Some(self.query.clone());
        self.error = None;
    }

    /// A click on `name`: alone it selects only that parameter; with `extend` it adds or removes
    /// it. Either way its description shows.
    pub fn click(&mut self, name: &str, extend: bool) {
        if !extend {
            self.selected = vec![name.to_string()];
        } else if let Some(at) = self.selected.iter().position(|n| n == name) {
            self.selected.remove(at);
        } else {
            self.selected.push(name.to_string());
        }
        self.described = Some(name.to_string());
    }

    /// What Add runs: `font.customParameters.add` for each selected parameter, in list order.
    pub fn commands(&self) -> Vec<(&'static str, Value)> {
        self.choices
            .iter()
            .filter(|c| self.selected.contains(&c.name))
            .map(|c| {
                let mut p = self.target.params();
                p.insert("name".into(), json!(c.name));
                ("font.customParameters.add", Value::Object(p))
            })
            .collect()
    }
}

/// The id of the picker's modal area.
pub fn modal_id() -> Id {
    widgets::id(("fontInfo.customParameterPicker", "modal"))
}

/// The id of the search field.
pub fn search_id() -> Id {
    widgets::id(("fontInfo.customParameterPicker", "search"))
}

/// The id of the list item of `name`.
pub fn item_id(name: &str) -> Id {
    widgets::id(("fontInfo.customParameterPicker", "item", name))
}

/// The id of the Add button.
pub fn add_id() -> Id {
    widgets::id(("fontInfo.customParameterPicker", "add"))
}

/// The id of the Cancel button.
pub fn cancel_id() -> Id {
    widgets::id(("fontInfo.customParameterPicker", "cancel"))
}

/// Draws the picker over everything else. Returns how it was closed, if it was.
pub fn show(ctx: &egui::Context, tokens: &Tokens, picker: &mut ParameterPicker) -> Option<Outcome> {
    let mut outcome = None;
    let modal = egui::Modal::new(modal_id()).show(ctx, |ui| {
        ui.set_width(tokens.list_column_unit * PICKER_COLUMNS);
        ui.label(RichText::new("Add Custom Parameter").strong().size(tokens.dialog_title_size));
        ui.add_space(tokens.dialog_spacing);
        let search = ui.add(
            egui::TextEdit::singleline(&mut picker.query)
                .id(search_id())
                .hint_text("Search")
                .char_limit(MAX_QUERY_CHARS)
                .desired_width(f32::INFINITY),
        );
        if picker.listed.is_none() {
            search.request_focus();
        }
        ui.add_space(tokens.dialog_spacing);
        let extend = ui.input(|i| i.modifiers.command || i.modifiers.shift);
        let mut clicked = None;
        if let Some(error) = &picker.error {
            ui.label(RichText::new(error.as_str()).color(tokens.error_text));
        } else if picker.choices.is_empty() {
            let text = if picker.listed.is_some() { "No parameter matches." } else { "Loading…" };
            ui.label(RichText::new(text).color(tokens.text_dim));
        } else {
            egui::ScrollArea::vertical().id_salt("fontInfo.customParameterPicker.list").max_height(tokens.list_row_height * LIST_ROWS).show(
                ui,
                |ui| {
                    for c in &picker.choices {
                        if item(ui, tokens, item_id(&c.name), &c.name, picker.selected.contains(&c.name)).clicked() {
                            clicked = Some(c.name.clone());
                        }
                    }
                },
            );
        }
        if let Some(name) = clicked {
            picker.click(&name, extend);
        }
        ui.add_space(tokens.dialog_spacing);
        let described = picker.described.as_ref().and_then(|n| picker.choices.iter().find(|c| c.name == *n));
        match described {
            Some(c) => {
                ui.label(RichText::new(c.name.as_str()).strong());
                let kind = if c.property { format!("{}, sets a General field", c.kind) } else { c.kind.clone() };
                ui.label(RichText::new(kind).color(tokens.text_dim));
                ui.label(c.summary.as_str());
            }
            None => {
                ui.label(RichText::new("Click a parameter to read what it does.").color(tokens.text_dim));
            }
        }
        ui.add_space(tokens.dialog_spacing);
        ui.horizontal(|ui| {
            if widgets::button(ui, cancel_id(), "Cancel", "Close without adding", true, false) {
                outcome = Some(Outcome::Cancel);
            }
            if widgets::button(ui, add_id(), "Add", "Add the selected parameters", !picker.selected.is_empty(), false) {
                outcome = Some(Outcome::Add);
            }
        });
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Cancel);
    }
    outcome
}
