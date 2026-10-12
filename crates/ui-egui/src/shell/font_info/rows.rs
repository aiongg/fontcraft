//! The shared Font Info row: a General field or a custom parameter, with its checkbox (parameters),
//! its value editor ([`super::param_value`]), the minus button that Alt reveals, and row
//! selection (click, Cmd-click, Shift-click). Localizable fields show one sub-row per language.
//! The row draws and reports; the tab turns the report into a command (`font.info.set`,
//! `font.customParameters.set`, `font.info.removeField`…), so every change goes through
//! `app.run`.
//!
//! The tabs build on the sections here: [`general_section`] (the General heading with its plus
//! button and the added fields), [`parameters_section`] (the Custom Parameters heading and the
//! parameter rows) and [`builtin_field`] (one built-in field such as a master's name). Each
//! drawn row is recorded in [`super::FontInfoState::shown_rows`], which the window's selection,
//! Copy and Cmd+Delete act on, and every report becomes commands through [`action_commands`].
//!
//! Rows: `fontInfo.rows.plusAddsField`, `fontInfo.rows.altRevealsMinus`,
//! `fontInfo.rows.selection`, `fontInfo.rows.localizable` (M7.15).

use std::collections::HashMap;

use egui::{Align, Id, Layout, RichText, Sense};
use fontcraft_engine::fontinfo::rows::{Row, RowKind, builtin_row, rows_of};
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::fontinfo::{ParamKind, Scope, catalog, fields};
use fontcraft_model::plist::{Str, Value};
use fontcraft_model::{DEFAULT_LANGUAGE, Font, LANGUAGES};
use serde_json::{Map, Value as Json, json};

use super::param_value::{self, MAX_SHOWN_ENTRIES, plain_text, text_field};
use super::{FontInfoState, Picker};
use crate::dialogs::custom_parameter_picker::ParameterPicker;
use crate::dialogs::field_picker::FieldPicker;
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// The label column's width, in list columns of the theme.
const LABEL_COLUMNS: f32 = 4.0;
/// The language menu's width, in list columns of the theme.
const LANGUAGE_COLUMNS: f32 = 1.5;

/// What the user did to a row this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum RowAction {
    /// A new value from the editor.
    Edit(Value),
    /// The parameter's checkbox.
    SetEnabled(bool),
    /// The minus button.
    Remove,
    /// A click on the row: `extend` with Cmd, `range` with Shift.
    Select { extend: bool, range: bool },
    /// A new language for a localizable field.
    AddLanguage(String),
    /// The minus button of one language's sub-row.
    RemoveLanguage(String),
    /// New text for one language of a localizable field.
    EditLanguage { language: String, text: String },
    /// Another language picked in a sub-row's language menu.
    ChangeLanguage { from: String, to: String },
}

/// The font, or the masters or exports a tab's rows act on (several when several are selected;
/// the rows show the first one's values).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub scope: Scope,
    /// Indices into the font's masters or instances; ignored for the font.
    pub indices: Vec<usize>,
}

impl Target {
    pub fn font() -> Self {
        Self { scope: Scope::Font, indices: Vec::new() }
    }

    pub fn below_font(&self) -> bool {
        self.scope != Scope::Font
    }

    /// The master or instance whose values the rows show.
    pub fn first(&self) -> usize {
        self.indices.first().copied().unwrap_or(0)
    }

    /// `scope` and, below the font, every target as `targets`.
    pub fn params(&self) -> Map<String, Json> {
        let mut p = Map::new();
        p.insert("scope".into(), json!(self.scope.name()));
        if self.below_font() {
            p.insert("targets".into(), json!(self.indices));
        }
        p
    }

    /// `scope` and, below the font, one target as `target`: one set of parameters per target,
    /// for the commands that take a single one.
    pub fn each(&self) -> Vec<Map<String, Json>> {
        let one = |i: Option<usize>| {
            let mut p = Map::new();
            p.insert("scope".into(), json!(self.scope.name()));
            if let Some(i) = i {
                p.insert("target".into(), json!(i));
            }
            p
        };
        if self.below_font() { self.indices.iter().map(|i| one(Some(*i))).collect() } else { vec![one(None)] }
    }
}

/// One row the window shows this frame, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownRow {
    pub scope: Scope,
    pub kind: RowKind,
    /// The property key or the parameter name.
    pub key: String,
    /// Among the parameter rows of this name, from 0; 0 for a field.
    pub occurrence: usize,
    /// A built-in field (Family Name, a master's name…): never removed.
    pub built_in: bool,
}

/// How [`row_with`] draws a row.
#[derive(Clone, Debug, PartialEq)]
pub struct RowOptions {
    /// The id the row's label and fields are made from.
    pub id: Id,
    pub label: String,
    /// The editor of a parameter, or of a built-in field that is not a text.
    pub kind: Option<ParamKind>,
    pub selected: bool,
    /// The targets hold different values: the field shows a placeholder.
    pub mixed: bool,
    /// One sub-row per language, with a language menu and a plus button.
    pub localizable: bool,
    /// The row has a minus button.
    pub removable: bool,
}

impl RowOptions {
    /// The options of `row` as its scope's field list and the parameter catalogue describe it.
    pub fn for_row(id: Id, row: &Row, selected: bool) -> Self {
        let field = (row.kind == RowKind::Field).then(|| fields::field(row.scope, &row.key)).flatten();
        let (label, kind, localizable, removable) = match row.kind {
            RowKind::Field => (
                field.map_or_else(|| row.key.clone(), |f| f.label.to_string()),
                field.filter(|f| f.built_in).and_then(|_| matches!(row.value, Value::Number(_)).then_some(ParamKind::Number)),
                field.map_or(matches!(row.value, Value::Dict(_)), |f| f.localizable),
                !field.is_some_and(|f| f.built_in),
            ),
            RowKind::Param => (row.key.clone(), Some(param_kind(&row.key, &row.value)), false, true),
        };
        Self { id, label, kind, selected, mixed: false, localizable, removable }
    }
}

/// The kind of a parameter's editor: the catalogue's, or for an unknown name the kind of its
/// value.
pub fn param_kind(name: &str, value: &Value) -> ParamKind {
    if let Some(spec) = catalog::lookup(name) {
        return spec.kind;
    }
    match value {
        Value::Bool(_) => ParamKind::Bool,
        Value::Number(_) => ParamKind::Number,
        Value::Array(_) => ParamKind::List,
        Value::Dict(_) => ParamKind::Dict,
        _ => ParamKind::String,
    }
}

/// The id of a row of the window, from what it shows.
pub fn row_id(scope: Scope, kind: RowKind, key: &str, occurrence: usize) -> Id {
    let kind = if kind == RowKind::Param { "param" } else { "field" };
    widgets::id(("fontInfo.row", scope.name(), kind, key, occurrence))
}

/// The id of the plus button of a section heading (`general`, `parameters`).
pub fn plus_id(scope: Scope, section: &str) -> Id {
    widgets::id(("fontInfo.plus", scope.name(), section))
}

/// Draws one row; `selected` highlights it. Returns what the user did, if anything.
pub fn row(ui: &mut egui::Ui, tokens: &Tokens, row: &Row, selected: bool) -> Option<RowAction> {
    let id = ui.id().with(("fontInfo.row", &row.key));
    let opts = RowOptions::for_row(id, row, selected);
    row_with(ui, tokens, row, &opts)
}

/// Draws one row as `opts` says. Returns what the user did, if anything.
pub fn row_with(ui: &mut egui::Ui, tokens: &Tokens, row: &Row, opts: &RowOptions) -> Option<RowAction> {
    let mut action = None;
    let mods = ui.input(|i| i.modifiers);
    ui.horizontal_top(|ui| {
        if row.kind == RowKind::Param {
            let mut on = row.enabled;
            let _ = ui.push_id(opts.id.with("enabled"), |ui| ui.checkbox(&mut on, "")).inner.on_hover_text("Use this parameter");
            if on != row.enabled {
                action = Some(RowAction::SetEnabled(on));
            }
        }
        if label(ui, tokens, opts, row.enabled).clicked() {
            action = Some(RowAction::Select { extend: mods.command, range: mods.shift });
        }
        if opts.localizable {
            if let Some(a) = languages(ui, tokens, row, opts, mods.alt) {
                action = Some(a);
            }
            return;
        }
        let edited = match opts.kind {
            Some(kind) => param_value::editor_with_id(ui, tokens, opts.id.with("value"), kind, &row.value, opts.mixed),
            None => {
                let text = plain_text(&row.value).unwrap_or_default();
                let width = param_value::field_width(ui, tokens);
                text_field(ui, opts.id.with("value"), if opts.mixed { "" } else { &text }, width, opts.mixed).map(|t| Value::String(Str::new(t)))
            }
        };
        if let Some(v) = edited {
            action = Some(RowAction::Edit(v));
        }
        if opts.removable && widgets::flat_button(ui, opts.id.with("minus"), "−", "Remove this row") {
            action = Some(RowAction::Remove);
        }
    });
    action
}

/// The row's label, which takes the clicks that select the row.
fn label(ui: &mut egui::Ui, tokens: &Tokens, opts: &RowOptions, enabled: bool) -> egui::Response {
    let size = egui::vec2(tokens.list_column_unit * LABEL_COLUMNS, tokens.list_row_height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, opts.id, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if opts.selected {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        }
        let color = match (opts.selected, enabled) {
            (true, _) => tokens.selected_text,
            (false, true) => tokens.text,
            (false, false) => tokens.text_dim,
        };
        let font = egui::TextStyle::Body.resolve(ui.style());
        let at = egui::pos2(rect.right() - tokens.dialog_spacing, rect.center().y);
        painter.with_clip_rect(rect).text(at, egui::Align2::RIGHT_CENTER, &opts.label, font, color);
    }
    response.on_hover_text(opts.label.as_str())
}

/// The languages of a localizable field's value: a dictionary from language to text, or one text
/// for the default language.
pub fn languages_of(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Dict(d) => d.entries.iter().map(|(k, v)| (k.as_str().to_string(), plain_text(v).unwrap_or_default())).collect(),
        other => vec![(DEFAULT_LANGUAGE.to_string(), plain_text(other).unwrap_or_default())],
    }
}

/// The languages a localizable field may take: the default language, then the language-system
/// tags the name table knows.
fn all_languages() -> impl Iterator<Item = &'static str> {
    std::iter::once(DEFAULT_LANGUAGE).chain(LANGUAGES.iter().map(|(tag, _)| *tag))
}

/// One sub-row per language: the language menu, the text, and the plus button, which Alt turns
/// into the minus button.
fn languages(ui: &mut egui::Ui, tokens: &Tokens, row: &Row, opts: &RowOptions, alt: bool) -> Option<RowAction> {
    let entries = languages_of(&row.value);
    let next_free = all_languages().find(|l| !entries.iter().any(|(e, _)| e == l));
    let mut action = None;
    ui.vertical(|ui| {
        for (at, (language, text)) in entries.iter().enumerate().take(MAX_SHOWN_ENTRIES) {
            ui.horizontal(|ui| {
                let mut chosen = language.clone();
                egui::ComboBox::from_id_salt(opts.id.with(("language", at)))
                    .selected_text(language.as_str())
                    .width(tokens.list_column_unit * LANGUAGE_COLUMNS)
                    .show_ui(ui, |ui| {
                        for tag in all_languages().filter(|l| *l == language || !entries.iter().any(|(e, _)| e == l)) {
                            ui.selectable_value(&mut chosen, tag.to_string(), tag);
                        }
                    });
                if chosen != *language {
                    action = Some(RowAction::ChangeLanguage { from: language.clone(), to: chosen });
                }
                let width = param_value::field_width(ui, tokens);
                if let Some(new) = text_field(ui, opts.id.with(("text", at)), if opts.mixed { "" } else { text }, width, opts.mixed) {
                    action = Some(RowAction::EditLanguage { language: language.clone(), text: new });
                }
                if alt {
                    if widgets::flat_button(ui, opts.id.with(("minus", at)), "−", "Remove this language") {
                        action = Some(RowAction::RemoveLanguage(language.clone()));
                    }
                } else if let Some(free) = next_free
                    && widgets::flat_button(ui, opts.id.with(("plus", at)), "+", "Add a language (hold Alt to remove one)")
                {
                    action = Some(RowAction::AddLanguage(free.to_string()));
                }
            });
        }
    });
    action
}

/// The commands a row action runs on `target`. `occurrence` picks one of several parameter rows
/// of the same name. Selection runs nothing.
pub fn action_commands(target: &Target, row: &Row, occurrence: usize, action: &RowAction) -> Vec<(&'static str, Json)> {
    let param = |extra: Vec<(&'static str, Json)>| {
        let mut all = vec![("name", json!(row.key)), ("index", json!(occurrence))];
        all.extend(extra);
        with(target.params(), all)
    };
    let each = |id: &'static str, extra: Vec<(&'static str, Json)>| -> Vec<(&'static str, Json)> {
        let mut all = vec![("key", json!(row.key))];
        all.extend(extra);
        target.each().into_iter().map(|p| (id, with(p, all.clone()))).collect()
    };
    match (action, row.kind) {
        (RowAction::Select { .. }, _) | (RowAction::SetEnabled(_), RowKind::Field) => Vec::new(),
        (RowAction::Edit(v), RowKind::Param) => vec![("font.customParameters.set", param(vec![("value", fv::to_json(v))]))],
        (RowAction::Edit(v), RowKind::Field) => {
            let mut values = Map::new();
            values.insert(row.key.clone(), field_json(v));
            vec![("font.info.set", with(target.params(), vec![("values", Json::Object(values))]))]
        }
        (RowAction::SetEnabled(on), RowKind::Param) => vec![("font.customParameters.set", param(vec![("enabled", json!(on))]))],
        (RowAction::Remove, RowKind::Param) => vec![("font.customParameters.remove", param(Vec::new()))],
        (RowAction::Remove, RowKind::Field) => vec![("font.info.removeField", with(target.params(), vec![("keys", json!([row.key]))]))],
        (RowAction::AddLanguage(language), _) => each("font.info.setLocalizedValue", vec![("language", json!(language)), ("value", json!(""))]),
        (RowAction::RemoveLanguage(language), _) => each("font.info.removeLocalizedValue", vec![("language", json!(language))]),
        (RowAction::EditLanguage { language, text }, _) => {
            each("font.info.setLocalizedValue", vec![("language", json!(language)), ("value", json!(text))])
        }
        (RowAction::ChangeLanguage { from, to }, _) => {
            let text = languages_of(&row.value).into_iter().find(|(l, _)| l == from).map(|(_, t)| t).unwrap_or_default();
            let mut out = each("font.info.setLocalizedValue", vec![("language", json!(to)), ("value", json!(text))]);
            out.extend(each("font.info.removeLocalizedValue", vec![("language", json!(from))]));
            out
        }
    }
}

/// `p` with the `extra` entries added, as a JSON object.
fn with(mut p: Map<String, Json>, extra: Vec<(&'static str, Json)>) -> Json {
    for (k, v) in extra {
        p.insert(k.to_string(), v);
    }
    Json::Object(p)
}

/// A field's new value as `font.info.set` takes it: a number stays a number, anything else is
/// its text.
fn field_json(value: &Value) -> Json {
    match value {
        Value::Number(_) | Value::Bool(_) => fv::to_json(value),
        other => plain_text(other).map_or_else(|| fv::to_json(other), Json::String),
    }
}

/// A bold section heading with the plus button at its right. True when the plus was clicked.
pub fn heading(ui: &mut egui::Ui, tokens: &Tokens, plus: Id, title: &str, hover: &str) -> bool {
    let mut clicked = false;
    ui.add_space(tokens.dialog_spacing);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(tokens.section_header_size));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| clicked = widgets::flat_button(ui, plus, "+", hover));
    });
    clicked
}

/// The General heading, whose plus button opens the field picker, and the added fields of
/// `target`.
pub fn general_section(ui: &mut egui::Ui, tokens: &Tokens, state: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    if heading(ui, tokens, plus_id(target.scope, "general"), "General", "Add a field") {
        state.picker = Some(Picker::Field(FieldPicker::new(target.clone())));
    }
    section_rows(ui, tokens, state, font, target, RowKind::Field, pending);
}

/// The Custom Parameters heading, whose plus button opens the parameter picker, and the
/// parameters of `target`.
pub fn parameters_section(ui: &mut egui::Ui, tokens: &Tokens, state: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    if heading(ui, tokens, plus_id(target.scope, "parameters"), "Custom Parameters", "Add a custom parameter") {
        state.picker = Some(Picker::Parameter(ParameterPicker::new(target.clone())));
    }
    section_rows(ui, tokens, state, font, target, RowKind::Param, pending);
}

/// One built-in field of `target` (`familyName`, a master's `name`, an instance's
/// `weightClass`…) as a row; nothing when the target does not set it.
pub fn builtin_field(ui: &mut egui::Ui, tokens: &Tokens, state: &mut FontInfoState, font: &Font, target: &Target, key: &str, pending: &mut Pending) {
    let Some(row) = builtin_row(font, target.scope, target.first(), key) else { return };
    let mixed = target.indices.iter().skip(1).any(|i| builtin_row(font, target.scope, *i, key).is_none_or(|r| r.value != row.value));
    draw(ui, tokens, state, target, &row, 0, mixed, pending);
}

/// The rows of `kind` of `target`, in stored order.
fn section_rows(ui: &mut egui::Ui, tokens: &Tokens, state: &mut FontInfoState, font: &Font, target: &Target, kind: RowKind, pending: &mut Pending) {
    let rows: Vec<Row> = rows_of(font, target.scope, target.first()).into_iter().filter(|r| r.kind == kind).collect();
    // The other targets' values by (key, occurrence), to mark the rows they differ in.
    let others: Vec<HashMap<(String, usize), Value>> = if target.below_font() {
        target.indices.iter().skip(1).map(|i| keyed(rows_of(font, target.scope, *i).into_iter().filter(|r| r.kind == kind))).collect()
    } else {
        Vec::new()
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    for row in &rows {
        let count = seen.entry(row.key.clone()).or_insert(0);
        let occurrence = *count;
        *count = count.saturating_add(1);
        let lookup = (row.key.clone(), occurrence);
        let mixed = others.iter().any(|o| o.get(&lookup).is_none_or(|v| *v != row.value));
        draw(ui, tokens, state, target, row, occurrence, mixed, pending);
    }
}

/// Rows by (key, occurrence among the rows of that key).
fn keyed(rows: impl Iterator<Item = Row>) -> HashMap<(String, usize), Value> {
    let mut out = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let count = seen.entry(row.key.clone()).or_insert(0);
        out.insert((row.key, *count), row.value);
        *count = count.saturating_add(1);
    }
    out
}

/// Draws one row of a section, records it as shown, and turns what the user did into
/// selection or commands.
#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &mut FontInfoState,
    target: &Target,
    row: &Row,
    occurrence: usize,
    mixed: bool,
    pending: &mut Pending,
) {
    let index = state.shown_rows.len();
    let id = row_id(target.scope, row.kind, &row.key, occurrence);
    let mut opts = RowOptions::for_row(id, row, state.selected_rows.contains(&index));
    opts.mixed = mixed;
    state.shown_rows.push(ShownRow { scope: target.scope, kind: row.kind, key: row.key.clone(), occurrence, built_in: !opts.removable });
    match row_with(ui, tokens, row, &opts) {
        Some(RowAction::Select { extend, range }) => state.select(index, extend, range),
        Some(action) => pending.extend(action_commands(target, row, occurrence, &action)),
        None => {}
    }
}
