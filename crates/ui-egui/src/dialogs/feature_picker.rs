//! The Add Feature picker of Font Info › Features: the plus button beside the Features heading
//! opens it. Typing in its search field filters the registered feature tags by tag or name
//! (`font.features.listAvailable`); Up and Down move the highlight, and Return, a double-click or
//! Add adds the highlighted feature with `font.features.addFeature`. Tags the font already has
//! are listed dimmed and cannot be added. Cancel, Escape or a click outside closes it and changes
//! nothing.
//!
//! The picker keeps only view state; the Features tab fills its list with the query
//! ([`FeaturePicker::query_params`]) and runs what Add returns ([`FeaturePicker::command`]).
//!
//! Row: `fontInfo.featurePicker` (M8.17).

use egui::{Id, Key, Modifiers, RichText};
use serde_json::{Value, json};

use super::field_picker::{LIST_ROWS, Outcome, PICKER_COLUMNS, item};
use crate::panels::widgets;
use crate::theme::Tokens;

/// Most characters of the search text.
const MAX_QUERY_CHARS: usize = 100;
/// Most features listed.
const MAX_CHOICES: usize = 2_000;

/// One registered feature the picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureChoice {
    pub tag: String,
    pub name: String,
    /// The font has a feature with this tag already.
    pub in_font: bool,
    /// Update could write its code for this font.
    pub can_generate: bool,
}

/// The picker's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeaturePicker {
    pub query: String,
    /// The search text the list was made for; `None` before the first listing.
    pub listed: Option<String>,
    pub choices: Vec<FeatureChoice>,
    /// The highlighted choice, an index into `choices`.
    pub highlighted: Option<usize>,
    /// Why the list could not be made.
    pub error: Option<String>,
}

impl FeaturePicker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The list is missing or older than the search text.
    pub fn needs_listing(&self) -> bool {
        self.listed.as_deref() != Some(self.query.as_str())
    }

    /// The parameters of the `font.features.listAvailable` query that fills the list.
    pub fn query_params(&self) -> Value {
        if self.query.trim().is_empty() { json!({}) } else { json!({"query": self.query.trim()}) }
    }

    /// Takes the query's result; the highlight goes to the first feature that can be added.
    pub fn fill(&mut self, result: &Value) {
        let features = result.get("features").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        self.choices = features
            .iter()
            .take(MAX_CHOICES)
            .filter_map(|f| {
                Some(FeatureChoice {
                    tag: f.get("tag")?.as_str()?.to_string(),
                    name: f.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    in_font: f.get("inFont").and_then(Value::as_bool).unwrap_or(false),
                    can_generate: f.get("canGenerate").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        self.highlighted = self.choices.iter().position(|c| !c.in_font);
        self.listed = Some(self.query.clone());
        self.error = None;
    }

    /// Takes a refused query: the message, and no list.
    pub fn fail(&mut self, message: String) {
        self.choices.clear();
        self.highlighted = None;
        self.error = Some(message);
        self.listed = Some(self.query.clone());
    }

    /// Moves the highlight `by` addable choices down (negative: up), stopping at the ends.
    pub fn move_by(&mut self, by: i32) {
        let addable: Vec<usize> = self.choices.iter().enumerate().filter(|(_, c)| !c.in_font).map(|(i, _)| i).collect();
        let Some(last) = addable.len().checked_sub(1) else {
            self.highlighted = None;
            return;
        };
        let at = self.highlighted.and_then(|h| addable.iter().position(|i| *i == h));
        let next = match at {
            None => 0,
            Some(at) => {
                let step = usize::try_from(by.unsigned_abs()).unwrap_or(usize::MAX);
                if by < 0 { at.saturating_sub(step) } else { at.saturating_add(step).min(last) }
            }
        };
        self.highlighted = addable.get(next).copied();
    }

    /// What Add runs: `font.features.addFeature` for the highlighted feature, generated
    /// automatically when Update could write its code.
    pub fn command(&self) -> Option<(&'static str, Value)> {
        let c = self.choices.get(self.highlighted?).filter(|c| !c.in_font)?;
        Some(("font.features.addFeature", json!({"tag": c.tag, "automatic": c.can_generate})))
    }
}

/// The id of the picker's modal area.
pub fn modal_id() -> Id {
    widgets::id(("fontInfo.featurePicker", "modal"))
}

/// The id of the search field.
pub fn search_id() -> Id {
    widgets::id(("fontInfo.featurePicker", "search"))
}

/// The id of the list item of `tag`.
pub fn item_id(tag: &str) -> Id {
    widgets::id(("fontInfo.featurePicker", "item", tag))
}

pub fn add_id() -> Id {
    widgets::id(("fontInfo.featurePicker", "add"))
}

pub fn cancel_id() -> Id {
    widgets::id(("fontInfo.featurePicker", "cancel"))
}

/// Draws the picker over everything else. Returns how it was closed, if it was.
pub fn show(ctx: &egui::Context, tokens: &Tokens, picker: &mut FeaturePicker) -> Option<Outcome> {
    let mut outcome = None;
    let (down, up, enter) = ctx.input_mut(|i| {
        (
            i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown),
            i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    let moved = i32::try_from(down).unwrap_or(i32::MAX).saturating_sub(i32::try_from(up).unwrap_or(i32::MAX));
    if moved != 0 {
        picker.move_by(moved);
    }
    let modal = egui::Modal::new(modal_id()).show(ctx, |ui| {
        ui.set_width(tokens.list_column_unit * PICKER_COLUMNS);
        ui.label(RichText::new("Add Feature").strong().size(tokens.dialog_title_size));
        ui.add_space(tokens.dialog_spacing);
        let search = ui.add(
            egui::TextEdit::singleline(&mut picker.query)
                .id(search_id())
                .char_limit(MAX_QUERY_CHARS)
                .hint_text("Search by tag or name")
                .desired_width(f32::INFINITY),
        );
        if !ui.memory(|m| m.focused().is_some()) {
            search.request_focus();
        }
        ui.add_space(tokens.dialog_spacing);
        let mut clicked = None;
        let mut double = false;
        if let Some(error) = &picker.error {
            ui.label(RichText::new(error.as_str()).color(tokens.error_text));
        } else if picker.listed.is_none() {
            ui.label(RichText::new("Loading…").color(tokens.text_dim));
        } else if picker.choices.is_empty() {
            ui.label(RichText::new("No registered feature matches.").color(tokens.text_dim));
        } else {
            egui::ScrollArea::vertical().id_salt("fontInfo.featurePicker.list").max_height(tokens.list_row_height * LIST_ROWS).show(ui, |ui| {
                for (i, c) in picker.choices.iter().enumerate() {
                    let text = if c.in_font { format!("{}  {}  (in the font)", c.tag, c.name) } else { format!("{}  {}", c.tag, c.name) };
                    let response = if c.in_font {
                        ui.label(RichText::new(text).color(tokens.text_dim))
                    } else {
                        item(ui, tokens, item_id(&c.tag), &text, picker.highlighted == Some(i))
                    };
                    if moved != 0 && picker.highlighted == Some(i) {
                        response.scroll_to_me(None);
                    }
                    if !c.in_font && response.clicked() {
                        clicked = Some(i);
                    }
                    if !c.in_font && response.double_clicked() {
                        clicked = Some(i);
                        double = true;
                    }
                }
            });
        }
        if let Some(i) = clicked {
            picker.highlighted = Some(i);
            if double {
                outcome = Some(Outcome::Add);
            }
        }
        ui.add_space(tokens.dialog_spacing);
        ui.horizontal(|ui| {
            if widgets::button(ui, cancel_id(), "Cancel", "Close without adding", true, false) {
                outcome = Some(Outcome::Cancel);
            }
            if widgets::button(ui, add_id(), "Add", "Add the highlighted feature", picker.command().is_some(), false) {
                outcome = Some(Outcome::Add);
            }
        });
    });
    if outcome.is_none() && enter && picker.command().is_some() {
        outcome = Some(Outcome::Add);
    }
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Cancel);
    }
    outcome
}
