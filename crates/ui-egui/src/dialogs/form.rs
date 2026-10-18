//! The fields of a dialog: their kinds, how a value is checked, and how a field is drawn.
//!
//! A field edits one parameter of the dialog's command and is named after it: its `key` is the
//! parameter's key, the field's id for `ui.dialog.set` and `ui.click`.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{Editing, Event, OpenDialog};
use crate::icons::Icon;
use crate::panels::widgets::icon_button_response;
use crate::theme::Tokens;

/// Most fields one dialog may have (a schema read from the engine is capped by it).
pub const MAX_FIELDS: usize = 64;
/// Most choices one choice field may offer.
pub const MAX_CHOICES: usize = 256;
/// Longest text a number field takes.
pub const MAX_FIELD_TEXT: usize = 64;
/// Longest path a folder field takes.
pub const MAX_PATH_TEXT: usize = 4096;
/// Most rows an order field may list.
pub const MAX_ORDER_ROWS: usize = 4096;
/// Most rows a list field may hold.
pub const MAX_LIST_ROWS: usize = 65_536;
/// Most lines a text field shows at once.
pub const MAX_TEXT_LINES: usize = 32;
/// A folder field is this many number fields wide.
const PATH_WIDTH_FIELDS: f32 = 4.0;
/// A text field is this many number fields wide: room for a name or a short list of names.
const TEXT_WIDTH_FIELDS: f32 = 3.0;
/// A field of checkboxes is this many number fields wide…
const MULTI_WIDTH_FIELDS: f32 = 5.0;
/// …and scrolls past this many list rows.
const MULTI_ROWS: f32 = 12.0;

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
    /// On or off, as a lock button.
    Lock,
    /// Text of at most `max` characters; empty is allowed. `lines` above 1 makes a box of that
    /// many lines in which Return starts a new line.
    Text { max: usize, lines: usize },
    /// One word of a list, as a pop-up menu. `labels`, when given, holds the text the menu shows
    /// for each word, in the same order.
    Choice {
        choices: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        labels: Vec<String>,
    },
    /// Any words of a list, as checkboxes. The value lists the ticked words in the list's order.
    /// `labels` as for [`Kind::Choice`].
    Multi {
        choices: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        labels: Vec<String>,
    },
    /// A folder path, typed or picked with Choose… where the app has a folder chooser. An empty
    /// path is no folder: the field is left out of the parameters.
    Folder,
    /// An order of the rows `labels`: the value lists every row index once, the first row on top
    /// of the list. Rows are dragged into place.
    Order { labels: Vec<String> },
    /// A scrolling list of `rows`. With `select`, the value lists the selected rows in the list's
    /// order (click, Shift+click, Cmd+click, Cmd+A); without, the list only shows the rows and
    /// its value is all of them.
    List { rows: Vec<String>, select: bool },
    /// A smart filter's rule tree, `{all | any | none: [rule or group]}`, edited as rows (see
    /// [`super::rule_tree`]).
    Rules,
    /// Edits of the colour palettes `palettes` (`#RRGGBBAA`, one list per palette), drawn as the
    /// staged table (see [`super::color_palettes`]).
    Palettes { palettes: Vec<Vec<String>> },
    /// A colour `#RRGGBBAA`, drawn as a swatch that opens a colour picker with opacity.
    Color,
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
    /// The heading the field sits under; a heading row opens each run of fields with one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl Field {
    /// A number field.
    pub fn number(key: &str, label: &str, min: f64, max: f64, default: f64, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Number { min, max }, default: json!(default), doc: doc.into(), group: None }
    }

    /// A checkbox.
    pub fn bool(key: &str, label: &str, default: bool, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Bool, default: json!(default), doc: doc.into(), group: None }
    }

    /// On or off, drawn as a lock that is closed while on.
    pub fn lock(key: &str, label: &str, default: bool, doc: &str) -> Self {
        Self { kind: Kind::Lock, ..Self::bool(key, label, default, doc) }
    }

    /// The field under heading `group`.
    pub fn in_group(self, group: &str) -> Self {
        Self { group: Some(group.into()), ..self }
    }

    /// A text field of at most `max` characters.
    pub fn text(key: &str, label: &str, max: usize, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Text { max, lines: 1 }, default: json!(""), doc: doc.into(), group: None }
    }

    /// A box of `lines` lines of text of at most `max` characters.
    pub fn text_area(key: &str, label: &str, max: usize, lines: usize, doc: &str) -> Self {
        let lines = lines.clamp(1, MAX_TEXT_LINES);
        Self { kind: Kind::Text { max, lines }, ..Self::text(key, label, max, doc) }
    }

    /// A list that shows `rows` and is not edited; its value is all of them.
    pub fn rows(key: &str, label: &str, rows: Vec<String>, doc: &str) -> Self {
        let rows: Vec<String> = rows.into_iter().take(MAX_LIST_ROWS).collect();
        Self { key: key.into(), label: label.into(), default: json!(rows), kind: Kind::List { rows, select: false }, doc: doc.into(), group: None }
    }

    /// A list of `rows` to select from, the rows in `selected` selected at first.
    pub fn select_list(key: &str, label: &str, rows: Vec<String>, selected: &[String], doc: &str) -> Self {
        let rows: Vec<String> = rows.into_iter().take(MAX_LIST_ROWS).collect();
        let chosen: HashSet<&str> = selected.iter().map(String::as_str).collect();
        let default = json!(rows.iter().filter(|r| chosen.contains(r.as_str())).collect::<Vec<_>>());
        Self { key: key.into(), label: label.into(), default, kind: Kind::List { rows, select: true }, doc: doc.into(), group: None }
    }

    /// A rule tree, starting as `default`.
    pub fn rules(key: &str, label: &str, default: Value, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Rules, default, doc: doc.into(), group: None }
    }

    /// A text field of more than one line, where Return starts a new line.
    pub fn is_multiline(&self) -> bool {
        matches!(self.kind, Kind::Text { lines, .. } if lines > 1)
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
            group: None,
        }
    }

    /// Checkboxes for the `(word, label)` pairs; the words in `default` are ticked.
    pub fn multi(key: &str, label: &str, choices: &[(&str, &str)], default: &[&str], doc: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind: Kind::Multi {
                choices: choices.iter().map(|(w, _)| w.to_string()).collect(),
                labels: choices.iter().map(|(_, l)| l.to_string()).collect(),
            },
            default: json!(default),
            doc: doc.into(),
            group: None,
        }
    }

    /// A folder path, empty at first.
    pub fn folder(key: &str, label: &str, doc: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: Kind::Folder, default: json!(""), doc: doc.into(), group: None }
    }

    /// An order of the rows `labels`, at first as listed.
    pub fn order(key: &str, label: &str, labels: Vec<String>, doc: &str) -> Self {
        let default = json!((0..labels.len()).collect::<Vec<_>>());
        Self { key: key.into(), label: label.into(), kind: Kind::Order { labels }, default, doc: doc.into(), group: None }
    }

    /// A field that takes typed text (numbers, text, paths).
    pub fn is_text(&self) -> bool {
        matches!(self.kind, Kind::Number { .. } | Kind::Integer { .. } | Kind::Text { .. } | Kind::Folder)
    }

    /// The longest text the field takes.
    pub fn max_text(&self) -> usize {
        match self.kind {
            Kind::Folder => MAX_PATH_TEXT,
            Kind::Text { max, .. } => max,
            _ => MAX_FIELD_TEXT,
        }
    }

    /// A folder field with no folder in `value`.
    pub fn is_empty_folder(&self, value: &Value) -> bool {
        self.kind == Kind::Folder && value.as_str().is_none_or(str::is_empty)
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
            Kind::Bool | Kind::Lock => value.as_bool().map(Value::Bool).ok_or_else(|| format!("`{key}` must be true or false")),
            Kind::Text { max, .. } => match value.as_str() {
                Some(text) if text.chars().count() <= *max => Ok(json!(text)),
                Some(_) => Err(format!("`{key}` must be at most {max} characters")),
                None => Err(format!("`{key}` must be text")),
            },
            Kind::Choice { choices, .. } => match value.as_str() {
                Some(word) if choices.iter().any(|c| c == word) => Ok(json!(word)),
                _ => Err(format!("`{key}` must be one of {}", choices.join(", "))),
            },
            Kind::Multi { choices, .. } => {
                let bad = || format!("`{key}` must be a list of {}", choices.join(", "));
                let list = value.as_array().filter(|l| l.len() <= choices.len()).ok_or_else(bad)?;
                let words: Vec<&str> =
                    list.iter().map(|w| w.as_str().filter(|w| choices.iter().any(|c| c == w)).ok_or_else(bad)).collect::<Result<_, _>>()?;
                Ok(json!(choices.iter().filter(|c| words.contains(&c.as_str())).collect::<Vec<_>>()))
            }
            Kind::Folder => match value.as_str() {
                Some(path) if path.len() <= MAX_PATH_TEXT && !path.contains('\0') => Ok(json!(path.trim())),
                _ => Err(format!("`{key}` must be a folder path of at most {MAX_PATH_TEXT} bytes")),
            },
            Kind::Order { labels } => {
                let count = labels.len();
                let bad = || format!("`{key}` must list every row index from 0 to {} once", count.saturating_sub(1));
                let list = value.as_array().filter(|l| l.len() == count).ok_or_else(bad)?;
                let mut seen = vec![false; count];
                let mut order = Vec::with_capacity(count);
                for item in list {
                    let i = item.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(bad)?;
                    match seen.get_mut(i) {
                        Some(slot) if !*slot => *slot = true,
                        _ => return Err(bad()),
                    }
                    order.push(i);
                }
                Ok(json!(order))
            }
            Kind::List { rows, select: false } => {
                if value.as_array().is_some_and(|l| l.len() == rows.len() && l.iter().zip(rows).all(|(v, r)| v.as_str() == Some(r.as_str()))) {
                    Ok(json!(rows))
                } else {
                    Err(format!("`{key}` is a list that only shows its rows"))
                }
            }
            Kind::List { rows, select: true } => {
                let bad = || format!("`{key}` must be a list of rows of the list");
                let list = value.as_array().filter(|l| l.len() <= rows.len()).ok_or_else(bad)?;
                let known: HashSet<&str> = rows.iter().map(String::as_str).collect();
                let mut chosen = HashSet::with_capacity(list.len());
                for v in list {
                    chosen.insert(v.as_str().filter(|r| known.contains(r)).ok_or_else(bad)?);
                }
                Ok(json!(rows.iter().filter(|r| chosen.contains(r.as_str())).collect::<Vec<_>>()))
            }
            Kind::Rules => super::rule_tree::check(key, value),
            Kind::Palettes { palettes } => super::color_palettes::check(key, palettes, value),
            Kind::Color => super::master_color::check(key, value),
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
    Ok(Field { key, label, kind, default: p.get("default").cloned().unwrap_or(Value::Null), doc: text("doc"), group: None })
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
/// [`Event`]s; the dialog applies them after the frame's drawing. `can_choose`: the app has a
/// folder chooser, so folder fields show Choose….
pub fn draw(ui: &mut egui::Ui, dialog: &OpenDialog, tokens: &Tokens, can_choose: bool, events: &mut Vec<Event>) {
    egui::Grid::new("fontcraft-dialog-fields").num_columns(2).spacing([tokens.dialog_spacing, tokens.dialog_spacing]).show(ui, |ui| {
        let mut group: Option<&str> = None;
        let answered = dialog.third().map(|t| t.key);
        for field in &dialog.fields {
            // The buttons answer this field: its label, if any, says what they answer.
            if Some(field.key.as_str()) == answered {
                if !field.label.is_empty() {
                    ui.label(&field.label);
                    ui.end_row();
                }
                continue;
            }
            if field.group.as_deref() != group {
                group = field.group.as_deref();
                // A field outside any group follows the last group without a row of its own.
                if let Some(heading) = group {
                    ui.label(egui::RichText::new(heading).strong());
                    ui.end_row();
                }
            }
            let value = dialog.values.get(&field.key).cloned().unwrap_or(Value::Null);
            if group.is_some() {
                ui.horizontal(|ui| {
                    ui.add_space(tokens.dialog_spacing * 1.5);
                    ui.label(&field.label);
                });
            } else {
                ui.label(&field.label);
            }
            let response = match &field.kind {
                Kind::Number { .. } | Kind::Integer { .. } => text_field(ui, dialog, field, &value, tokens.dialog_field_width, events),
                Kind::Text { .. } => text_field(ui, dialog, field, &value, tokens.dialog_field_width * TEXT_WIDTH_FIELDS, events),
                Kind::List { rows, select } => list(ui, field, rows, *select, &value, tokens, events),
                Kind::Rules => super::rule_tree::draw(ui, field, &value, tokens, events),
                Kind::Palettes { palettes } => super::color_palettes::draw(ui, field, palettes, &value, tokens, events),
                Kind::Color => super::master_color::draw(ui, field, &value, tokens, events),
                Kind::Bool => {
                    let mut on = value.as_bool().unwrap_or(false);
                    let r = ui.add(egui::Checkbox::without_text(&mut on)).on_hover_text(&field.doc);
                    if r.changed() {
                        events.push(Event::Set(field.key.clone(), json!(on)));
                    }
                    r
                }
                Kind::Lock => {
                    let on = value.as_bool().unwrap_or(false);
                    let icon = if on { Icon::Locked } else { Icon::Unlocked };
                    let r = icon_button_response(ui, widget_id(&field.key), icon, &field.doc, true, on);
                    if r.clicked() {
                        events.push(Event::Set(field.key.clone(), json!(!on)));
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
                Kind::Multi { choices, labels } => {
                    let ticked: Vec<&str> = value.as_array().map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    // A block of fixed width that scrolls past a dozen rows, so a long list stays
                    // on screen and the grid row keeps the block's height.
                    ui.vertical(|ui| {
                        ui.set_width(tokens.dialog_field_width * MULTI_WIDTH_FIELDS);
                        let rows = tokens.list_row_height * MULTI_ROWS;
                        egui::ScrollArea::vertical().id_salt(widget_id(&field.key)).max_height(rows).min_scrolled_height(rows).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for choice in choices {
                                    let mut on = ticked.contains(&choice.as_str());
                                    let r = ui.checkbox(&mut on, choice_label(choices, labels, choice)).on_hover_text(&field.doc);
                                    if r.changed() {
                                        let next: Vec<&String> =
                                            choices.iter().filter(|c| if *c == choice { on } else { ticked.contains(&c.as_str()) }).collect();
                                        events.push(Event::Set(field.key.clone(), json!(next)));
                                    }
                                }
                            });
                        });
                    })
                    .response
                }
                Kind::Folder => {
                    ui.horizontal(|ui| {
                        let r = text_field(ui, dialog, field, &value, tokens.dialog_field_width * PATH_WIDTH_FIELDS, events);
                        if can_choose && ui.button("Choose…").clicked() {
                            events.push(Event::Choose(field.key.clone()));
                        }
                        r
                    })
                    .inner
                }
                Kind::Order { labels } => order_list(ui, field, labels, &value, events),
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

/// A list field: rows in a scrolling box, only the rows in view laid out. A selectable list
/// selects a row on click, the rows from the last clicked one with Shift, and adds or takes out
/// one with Cmd; Cmd+A is the dialog's ([`super::frame_keys`]).
fn list(ui: &mut egui::Ui, field: &Field, rows: &[String], select: bool, value: &Value, tokens: &Tokens, events: &mut Vec<Event>) -> egui::Response {
    let selected: HashSet<&str> =
        if select { value.as_array().map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default() } else { HashSet::new() };
    let frame = egui::Frame::NONE.stroke(egui::Stroke::new(1.0, tokens.text_dim)).inner_margin(egui::Margin::same(2));
    // One row under another: the grid's cell lays out left to right.
    let column = egui::Layout::top_down(egui::Align::Min);
    let response = frame.show(ui, |ui| ui.with_layout(column, |ui| list_rows(ui, field, rows, select, &selected, tokens)).inner).response;
    if let Some(at) = ui.ctx().data_mut(|d| d.remove_temp::<Vec<usize>>(widget_id(&field.key).with("picked"))) {
        let next: Vec<&String> = at.iter().filter_map(|i| rows.get(*i)).collect();
        events.push(Event::Set(field.key.clone(), json!(next)));
    }
    response.on_hover_text(&field.doc)
}

/// The rows of a list field; a click's new selection is left in the context's data as row
/// indexes, for [`list`] to send.
fn list_rows(ui: &mut egui::Ui, field: &Field, rows: &[String], select: bool, selected: &HashSet<&str>, tokens: &Tokens) {
    let id = widget_id(&field.key);
    let anchor_id = id.with("anchor");
    let row_height = tokens.list_row_height;
    let shown = if select { MULTI_ROWS } else { MULTI_ROWS / 2.0 };
    let height = row_height * shown.min(rows.len().max(1) as f32);
    ui.set_width(tokens.dialog_field_width * MULTI_WIDTH_FIELDS);
    let scroll = egui::ScrollArea::vertical().id_salt(id).max_height(height).min_scrolled_height(height).auto_shrink([false, true]);
    // Rows touch: a selection reads as one band.
    ui.spacing_mut().item_spacing.y = 0.0;
    scroll.show_rows(ui, row_height, rows.len(), |ui, range| {
        for at in range {
            let Some(row) = rows.get(at) else { continue };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_height), egui::Sense::hover());
            let response = ui.interact(rect, row_id(&field.key, at), if select { egui::Sense::click() } else { egui::Sense::hover() });
            let on = selected.contains(row.as_str());
            if on {
                ui.painter().rect_filled(rect, 0.0, tokens.row_selected);
            }
            let font = egui::TextStyle::Body.resolve(ui.style());
            let text = rect.left_center() + egui::vec2(tokens.dialog_spacing / 2.0, 0.0);
            ui.painter().with_clip_rect(rect).text(text, egui::Align2::LEFT_CENTER, row, font, tokens.text);
            if !select || !response.clicked() {
                continue;
            }
            let mods = ui.input(|i| i.modifiers);
            let anchor = ui.ctx().data(|d| d.get_temp::<usize>(anchor_id)).filter(|a| *a < rows.len());
            let next: Vec<usize> = match anchor {
                Some(a) if mods.shift => (a.min(at)..=a.max(at)).collect(),
                _ if mods.command => {
                    (0..rows.len()).filter(|i| if *i == at { !on } else { rows.get(*i).is_some_and(|r| selected.contains(r.as_str())) }).collect()
                }
                _ => vec![at],
            };
            ui.ctx().data_mut(|d| {
                if !mods.shift {
                    d.insert_temp(anchor_id, at);
                }
                d.insert_temp(id.with("picked"), next);
            });
        }
    });
}

/// The id of row `at` of the list field `key`.
pub fn row_id(key: &str, at: usize) -> egui::Id {
    widget_id(key).with(("row", at))
}

/// The rows of an order field, the first on top; a row dropped on another takes its place.
fn order_list(ui: &mut egui::Ui, field: &Field, labels: &[String], value: &Value, events: &mut Vec<Event>) -> egui::Response {
    let order: Vec<usize> =
        value.as_array().map(|l| l.iter().filter_map(Value::as_u64).filter_map(|n| usize::try_from(n).ok()).collect()).unwrap_or_default();
    ui.vertical(|ui| {
        for (at, &row) in order.iter().enumerate() {
            let text = labels.get(row).map_or("", String::as_str);
            let id = egui::Id::new(("fontcraft-dialog-order", field.key.as_str(), row));
            let response = ui.dnd_drag_source(id, at, |ui| ui.label(text)).response.on_hover_text(&field.doc);
            if let Some(from) = response.dnd_release_payload::<usize>()
                && let Some(next) = moved(&order, *from, at)
            {
                events.push(Event::Set(field.key.clone(), json!(next)));
            }
        }
    })
    .response
}

/// `order` with the entry at `from` moved to `to`; `None` when nothing moves.
pub fn moved(order: &[usize], from: usize, to: usize) -> Option<Vec<usize>> {
    if from == to || from >= order.len() || to >= order.len() {
        return None;
    }
    let mut next = order.to_vec();
    let row = next.remove(from);
    next.insert(to, row);
    Some(next)
}

/// A number, text or folder field. Its text is the value, or what is being typed while it has the
/// keyboard.
fn text_field(ui: &mut egui::Ui, dialog: &OpenDialog, field: &Field, value: &Value, width: f32, events: &mut Vec<Event>) -> egui::Response {
    let id = widget_id(&field.key);
    let editing: Option<&Editing> = dialog.editing.as_ref().filter(|e| e.field == field.key);
    let mut text = editing.map_or_else(|| format_value(value), |e| e.text.clone());
    // Sized, not just a desired width: the dialog's area takes its width from its widest row,
    // and a text field alone never widens it.
    let lines = match field.kind {
        Kind::Text { lines, .. } => lines.clamp(1, MAX_TEXT_LINES),
        _ => 1,
    };
    let edit = if lines > 1 { egui::TextEdit::multiline(&mut text).desired_rows(lines) } else { egui::TextEdit::singleline(&mut text) };
    let row = ui.text_style_height(&egui::TextStyle::Body).max(ui.spacing().interact_size.y);
    let height = if lines > 1 { row * lines as f32 } else { ui.spacing().interact_size.y };
    let response = ui.add_sized(egui::vec2(width, height), edit.id(id).char_limit(field.max_text()).desired_width(width));
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
