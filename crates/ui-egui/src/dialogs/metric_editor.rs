//! The metric editor of the Masters tab: a click on a metric's name opens it to edit the metric's
//! category, name and glyph scope; the plus beside the Metrics heading opens it to add a metric;
//! a stem's actions button opens it to edit the stem's glyph scope. The scope is a glyph
//! predicate, as smart filters write them (`script == "greek"`).
//!
//! OK (Add for a new metric) runs one command with what changed: `font.metrics.set`,
//! `font.metrics.add` or `font.stems.set`; an emptied name or scope removes it. Cancel, Escape
//! or a click outside closes it and changes nothing. The editor keeps only view state.
//!
//! Row: `fontInfo.metricEditor` (M7.17).

use egui::{Id, RichText};
use fontcraft_model::{Font, metric};
use serde_json::{Map, Value, json};

pub use super::field_picker::Outcome;
use super::field_picker::PICKER_COLUMNS;
use crate::panels::widgets;
use crate::theme::Tokens;

/// Longest name or scope the fields take, in characters.
pub const MAX_TEXT_CHARS: usize = 1024;

/// The categories a metric may take, in menu order: the format's spelling and the label shown.
/// The empty category is a metric known only by its name.
pub const CATEGORIES: [(&str, &str); 10] = [
    (metric::ASCENDER, "Ascender"),
    (metric::CAP_HEIGHT, "Cap Height"),
    ("slant height", "Slant Height"),
    (metric::X_HEIGHT, "x-Height"),
    ("midHeight", "Mid Height"),
    ("bodyHeight", "Body Height"),
    (metric::DESCENDER, "Descender"),
    (metric::BASELINE, "Baseline"),
    (metric::ITALIC_ANGLE, "Italic Angle"),
    ("", "None (named only)"),
];

/// The label of a category; a category the menu lacks shows as stored.
pub fn category_label(kind: &str) -> &str {
    CATEGORIES.iter().find(|(k, _)| k.eq_ignore_ascii_case(kind)).map_or(kind, |(_, l)| l)
}

/// What the editor edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// A metric the plus button adds.
    NewMetric,
    /// The font's metric definition at this index.
    Metric(usize),
    /// The font's stem definition at this index: only its scope.
    Stem(usize),
}

/// The editor's state.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricEditor {
    pub subject: Subject,
    /// The category in the format's spelling; empty for a metric known only by its name.
    pub category: String,
    pub name: String,
    pub scope: String,
    /// Category, name and scope as the editor opened with them.
    pub opened: (String, String, String),
}

impl MetricEditor {
    fn with(subject: Subject, category: String, name: String, scope: String) -> Self {
        let opened = (category.clone(), name.clone(), scope.clone());
        Self { subject, category, name, scope, opened }
    }

    /// The editor of the font's metric `index`; `None` when there is no such metric.
    pub fn metric(font: &Font, index: usize) -> Option<Self> {
        let def = font.metrics.get(index)?;
        Some(Self::with(Subject::Metric(index), def.kind.clone(), def.name.clone().unwrap_or_default(), def.filter.clone().unwrap_or_default()))
    }

    /// The editor of a new metric: the first category the font has no plain metric of, else
    /// the first category.
    pub fn new_metric(font: &Font) -> Self {
        let plain = |kind: &str| font.metrics.iter().any(|m| m.kind == kind && m.name.is_none() && m.filter.is_none());
        let category = CATEGORIES.iter().map(|(k, _)| *k).find(|k| !k.is_empty() && !plain(*k)).unwrap_or(metric::ASCENDER);
        let mut editor = Self::with(Subject::NewMetric, category.to_string(), String::new(), String::new());
        // A new metric sends everything, so nothing counts as unchanged.
        editor.opened = (String::new(), String::new(), String::new());
        editor
    }

    /// The scope editor of the font's stem `index`; `None` when there is no such stem.
    pub fn stem(font: &Font, index: usize) -> Option<Self> {
        let def = font.stems.get(index)?;
        Some(Self::with(Subject::Stem(index), String::new(), def.name.clone(), def.filter.clone().unwrap_or_default()))
    }

    /// The window's title.
    pub fn title(&self) -> &'static str {
        match self.subject {
            Subject::NewMetric => "Add Metric",
            Subject::Metric(_) => "Metric",
            Subject::Stem(_) => "Stem Scope",
        }
    }

    /// What OK runs: nothing when nothing changed.
    pub fn commands(&self) -> Vec<(&'static str, Value)> {
        let (category, name, scope) = (self.category.trim(), self.name.trim(), self.scope.trim());
        let mut p = Map::new();
        let id = match self.subject {
            Subject::NewMetric => {
                p.insert("category".into(), json!(category));
                if !name.is_empty() {
                    p.insert("name".into(), json!(name));
                }
                if !scope.is_empty() {
                    p.insert("scope".into(), json!(scope));
                }
                return vec![("font.metrics.add", Value::Object(p))];
            }
            Subject::Metric(index) => {
                p.insert("metric".into(), json!(index));
                if category != self.opened.0 {
                    p.insert("category".into(), json!(category));
                }
                if name != self.opened.1 {
                    p.insert("name".into(), json!(name));
                }
                "font.metrics.set"
            }
            Subject::Stem(index) => {
                p.insert("stem".into(), json!(index));
                "font.stems.set"
            }
        };
        if scope != self.opened.2 {
            p.insert("scope".into(), json!(scope));
        }
        if p.len() == 1 { Vec::new() } else { vec![(id, Value::Object(p))] }
    }
}

/// The id of the editor's modal area.
pub fn modal_id() -> Id {
    widgets::id(("fontInfo.metricEditor", "modal"))
}

/// The id of the name field.
pub fn name_id() -> Id {
    widgets::id(("fontInfo.metricEditor", "name"))
}

/// The id of the scope field.
pub fn scope_id() -> Id {
    widgets::id(("fontInfo.metricEditor", "scope"))
}

/// The id of the category menu.
pub fn category_id() -> Id {
    widgets::id(("fontInfo.metricEditor", "category"))
}

/// The id of the OK (or Add) button.
pub fn ok_id() -> Id {
    widgets::id(("fontInfo.metricEditor", "ok"))
}

/// The id of the Cancel button.
pub fn cancel_id() -> Id {
    widgets::id(("fontInfo.metricEditor", "cancel"))
}

/// Draws the editor over everything else. Returns how it was closed, if it was.
pub fn show(ctx: &egui::Context, tokens: &Tokens, editor: &mut MetricEditor) -> Option<Outcome> {
    let mut outcome = None;
    let modal = egui::Modal::new(modal_id()).show(ctx, |ui| {
        ui.set_width(tokens.list_column_unit * PICKER_COLUMNS);
        ui.label(RichText::new(editor.title()).strong().size(tokens.dialog_title_size));
        ui.add_space(tokens.dialog_spacing);
        egui::Grid::new(widgets::id(("fontInfo.metricEditor", "grid"))).num_columns(2).spacing([tokens.dialog_spacing, tokens.dialog_spacing]).show(
            ui,
            |ui| {
                if matches!(editor.subject, Subject::Stem(_)) {
                    ui.label("Stem");
                    ui.label(RichText::new(editor.name.as_str()).strong());
                    ui.end_row();
                } else {
                    ui.label("Category");
                    egui::ComboBox::from_id_salt(category_id())
                        .selected_text(category_label(&editor.category).to_string())
                        .width(tokens.dialog_field_width)
                        .show_ui(ui, |ui| {
                            for (kind, label) in CATEGORIES {
                                ui.selectable_value(&mut editor.category, kind.to_string(), label);
                            }
                        });
                    ui.end_row();
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.name)
                            .id(name_id())
                            .hint_text("Optional")
                            .char_limit(MAX_TEXT_CHARS)
                            .desired_width(tokens.dialog_field_width),
                    );
                    ui.end_row();
                }
                ui.label("Scope");
                ui.add(
                    egui::TextEdit::singleline(&mut editor.scope)
                        .id(scope_id())
                        .hint_text("All glyphs")
                        .char_limit(MAX_TEXT_CHARS)
                        .desired_width(tokens.dialog_field_width),
                )
                .on_hover_text("A glyph predicate, as smart filters write them, for example script == \"greek\"");
                ui.end_row();
            },
        );
        ui.add_space(tokens.dialog_spacing);
        ui.horizontal(|ui| {
            if widgets::button(ui, cancel_id(), "Cancel", "Close without changing anything", true, false) {
                outcome = Some(Outcome::Cancel);
            }
            let (label, hover) = if editor.subject == Subject::NewMetric { ("Add", "Add the metric") } else { ("OK", "Apply the changes") };
            if widgets::button(ui, ok_id(), label, hover, true, false) {
                outcome = Some(Outcome::Add);
            }
        });
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Cancel);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metric_editor_sends_only_what_changed() {
        let font = fontcraft_engine::Session::with_document().font().unwrap().clone();
        let at = font.metrics.iter().position(|m| m.kind == metric::X_HEIGHT).unwrap();
        let mut e = MetricEditor::metric(&font, at).unwrap();
        assert_eq!(e.category, metric::X_HEIGHT);
        assert!(e.commands().is_empty(), "OK without a change runs nothing");
        e.name = "small".into();
        e.scope = "script == \"greek\"".into();
        assert_eq!(e.commands(), vec![("font.metrics.set", json!({"metric": at, "name": "small", "scope": "script == \"greek\""}))]);
        let new = MetricEditor::new_metric(&font);
        assert!(!font.metrics.iter().any(|m| m.kind == new.category && m.name.is_none()), "a new metric starts at a category the font lacks");
        assert_eq!(category_label("x-height"), "x-Height");
        assert_eq!(category_label("Alef Height"), "Alef Height");
    }
}
