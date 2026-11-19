//! The fields of a dialog: their kinds, how a value is checked, the `filter.list` schema, and the craft row each
//! field shows as.
//!
//! A field edits one parameter of the dialog's command and is named after it: its `key` is the parameter's key, the
//! field's id for `ui.dialog.set` and `ui.dialog.click`. Values are JSON as the command takes them; the window's
//! rows hold them as `DialogFields`, and [`Field::to_field_value`] and [`Field::from_field_value`] convert.

use std::collections::HashSet;

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, Param, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;
use egui::{Response, Ui};
use serde_json::{Map, Value, json};

/// Most fields one dialog may have (a schema read from the engine is capped by it).
pub const MAX_FIELDS: usize = 64;
/// Most choices one choice field may offer.
pub const MAX_CHOICES: usize = 256;
/// Longest text a number field takes.
pub const MAX_FIELD_TEXT: usize = 64;
/// Longest path a folder field takes.
pub const MAX_PATH_TEXT: usize = 4096;
/// Most rows a list or order field may hold.
pub const MAX_LIST_ROWS: usize = 65_536;
/// Most lines a text field shows at once.
pub const MAX_TEXT_LINES: usize = 32;
/// Digits after the decimal mark a number field shows.
const NUMBER_DECIMALS: u32 = 2;

/// Checks a value for a [`Kind::Custom`] field: the value as the field stores it, or why it does not fit.
pub type CustomCheck = fn(&Field, &Value) -> Result<Value, String>;

/// Draws a [`Kind::Custom`] field's control cell over the row values; the field is the row's.
pub type CustomRowFn = fn(&mut Ui, &mut DialogFields, &mut CustomContext, &Field) -> Response;

/// A field kind a package defines: a rule tree, a palette table, a colour.
#[derive(Clone, Copy, Debug)]
pub struct Custom {
    /// The `kind` word `ui.dialog.inspect` shows.
    pub name: &'static str,
    pub check: CustomCheck,
    /// The control cell; `None` shows the value as text, set only over the control channel.
    pub row: Option<CustomRowFn>,
}

/// What a field holds.
#[derive(Clone, Debug)]
pub enum Kind {
    /// A number from `min` to `max`.
    Number { min: f64, max: f64 },
    /// A whole number from `min` to `max`.
    Integer { min: f64, max: f64 },
    /// A checkbox.
    Bool,
    /// On or off, as a lock button.
    Lock,
    /// Text of at most `max` characters; empty is allowed. `lines` above 1 makes a box of that many lines.
    Text { max: usize, lines: usize },
    /// One word of a list, as a pop-up menu. `labels`, when given, holds the text the menu shows for each word.
    Choice { choices: Vec<String>, labels: Vec<String> },
    /// Any words of a list, as checkboxes. The value lists the ticked words in the list's order.
    Multi { choices: Vec<String>, labels: Vec<String> },
    /// A folder path, typed or picked with the folder chooser. An empty path is no folder: the field is left out of
    /// the parameters.
    Folder,
    /// A file path, typed or picked with the file chooser: an existing file to read, or with `save` a file to write.
    /// `extensions` are the types the chooser offers (lowercase, without the dot; empty offers every file); a file to
    /// write gets the first when the person types none. An empty path is no file: the field is left out of the
    /// parameters.
    File { save: bool, extensions: Vec<String> },
    /// An order of the rows `labels`: the value lists every row index once, the first row on top.
    Order { labels: Vec<String> },
    /// A list of `rows`. With `select`, the value lists the selected rows in the list's order; without, the list only
    /// shows the rows and its value is all of them.
    List { rows: Vec<String>, select: bool },
    /// A kind a package defines, with what it needs to know (`data`).
    Custom { kind: Custom, data: Value },
}

/// One field: a parameter of the dialog's command.
#[derive(Clone, Debug)]
pub struct Field {
    /// The parameter's key; also the field's id.
    pub key: String,
    pub label: String,
    pub kind: Kind,
    /// The value the dialog opens with.
    pub default: Value,
    /// One line for tooltips and agents.
    pub doc: String,
    /// The heading the field sits under; a heading row opens each run of fields with one.
    pub group: Option<String>,
}

impl Field {
    fn of(key: &str, label: &str, kind: Kind, default: Value, doc: &str) -> Self {
        Field { key: key.into(), label: label.into(), kind, default, doc: doc.into(), group: None }
    }

    /// A number field.
    pub fn number(key: &str, label: &str, min: f64, max: f64, default: f64, doc: &str) -> Self {
        Self::of(key, label, Kind::Number { min, max }, json!(default), doc)
    }

    /// A whole-number field.
    pub fn integer(key: &str, label: &str, min: i64, max: i64, default: i64, doc: &str) -> Self {
        Self::of(key, label, Kind::Integer { min: min as f64, max: max as f64 }, json!(default), doc)
    }

    /// A checkbox.
    pub fn bool(key: &str, label: &str, default: bool, doc: &str) -> Self {
        Self::of(key, label, Kind::Bool, json!(default), doc)
    }

    /// On or off, drawn as a lock that is closed while on.
    pub fn lock(key: &str, label: &str, default: bool, doc: &str) -> Self {
        Self::of(key, label, Kind::Lock, json!(default), doc)
    }

    /// A text field of at most `max` characters.
    pub fn text(key: &str, label: &str, max: usize, doc: &str) -> Self {
        Self::of(key, label, Kind::Text { max, lines: 1 }, json!(""), doc)
    }

    /// A box of `lines` lines of text of at most `max` characters.
    pub fn text_area(key: &str, label: &str, max: usize, lines: usize, doc: &str) -> Self {
        Self::of(key, label, Kind::Text { max, lines: lines.clamp(1, MAX_TEXT_LINES) }, json!(""), doc)
    }

    /// A pop-up menu of `(word, label)` pairs: the value is the word, the menu shows the label.
    pub fn choice(key: &str, label: &str, choices: &[(&str, &str)], default: &str, doc: &str) -> Self {
        let (choices, labels) = split(choices);
        Self::of(key, label, Kind::Choice { choices, labels }, json!(default), doc)
    }

    /// Checkboxes for the `(word, label)` pairs; the words in `default` are ticked.
    pub fn multi(key: &str, label: &str, choices: &[(&str, &str)], default: &[&str], doc: &str) -> Self {
        let (choices, labels) = split(choices);
        Self::of(key, label, Kind::Multi { choices, labels }, json!(default), doc)
    }

    /// A folder path, empty at first.
    pub fn folder(key: &str, label: &str, doc: &str) -> Self {
        Self::of(key, label, Kind::Folder, json!(""), doc)
    }

    /// A path of an existing file to read, empty at first.
    pub fn open_file(key: &str, label: &str, extensions: &[&str], doc: &str) -> Self {
        Self::of(key, label, Kind::File { save: false, extensions: extensions.iter().map(|e| e.to_string()).collect() }, json!(""), doc)
    }

    /// A path of a file to write, empty at first.
    pub fn save_file(key: &str, label: &str, extensions: &[&str], doc: &str) -> Self {
        Self::of(key, label, Kind::File { save: true, extensions: extensions.iter().map(|e| e.to_string()).collect() }, json!(""), doc)
    }

    /// An order of the rows `labels`, at first as listed.
    pub fn order(key: &str, label: &str, labels: Vec<String>, doc: &str) -> Self {
        let labels: Vec<String> = labels.into_iter().take(MAX_LIST_ROWS).collect();
        let default = json!((0..labels.len()).collect::<Vec<_>>());
        Self::of(key, label, Kind::Order { labels }, default, doc)
    }

    /// A list that shows `rows` and is not edited; its value is all of them.
    pub fn rows(key: &str, label: &str, rows: Vec<String>, doc: &str) -> Self {
        let rows: Vec<String> = rows.into_iter().take(MAX_LIST_ROWS).collect();
        Self::of(key, label, Kind::List { rows: rows.clone(), select: false }, json!(rows), doc)
    }

    /// A list of `rows` to select from, the rows in `selected` selected at first.
    pub fn select_list(key: &str, label: &str, rows: Vec<String>, selected: &[String], doc: &str) -> Self {
        let rows: Vec<String> = rows.into_iter().take(MAX_LIST_ROWS).collect();
        let chosen: HashSet<&str> = selected.iter().map(String::as_str).collect();
        let default = json!(rows.iter().filter(|r| chosen.contains(r.as_str())).collect::<Vec<_>>());
        Self::of(key, label, Kind::List { rows, select: true }, default, doc)
    }

    /// A field of a package's own kind, starting as `default`.
    pub fn custom(key: &str, label: &str, kind: Custom, data: Value, default: Value, doc: &str) -> Self {
        Self::of(key, label, Kind::Custom { kind, data }, default, doc)
    }

    /// The field under heading `group`.
    pub fn in_group(self, group: &str) -> Self {
        Field { group: Some(group.into()), ..self }
    }

    /// A field that takes typed text (numbers, text, paths): `ui.dialog.click` gives it the keyboard.
    pub fn is_text(&self) -> bool {
        matches!(self.kind, Kind::Number { .. } | Kind::Integer { .. } | Kind::Text { .. } | Kind::Folder | Kind::File { .. })
    }

    /// A text field of more than one line, where Return starts a new line.
    pub fn is_multiline(&self) -> bool {
        matches!(self.kind, Kind::Text { lines, .. } if lines > 1)
    }

    /// The longest text the field takes.
    pub fn max_text(&self) -> usize {
        match self.kind {
            Kind::Folder | Kind::File { .. } => MAX_PATH_TEXT,
            Kind::Text { max, .. } => max,
            _ => MAX_FIELD_TEXT,
        }
    }

    /// A folder or file field with no path in `value`: it is left out of the parameters.
    pub fn is_empty_path(&self, value: &Value) -> bool {
        matches!(self.kind, Kind::Folder | Kind::File { .. }) && value.as_str().is_none_or(str::is_empty)
    }

    /// Where the output goes, still empty: a folder or a file to write with no path. OK in the window asks the chooser
    /// for it.
    pub fn needs_destination(&self, value: &Value) -> bool {
        matches!(self.kind, Kind::Folder | Kind::File { save: true, .. }) && self.is_empty_path(value)
    }

    /// `value` as this field stores it, or why it does not fit. Number fields also take the text typed into them.
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
            Kind::File { .. } => match value.as_str() {
                Some(path) if path.len() <= MAX_PATH_TEXT && !path.contains('\0') => Ok(json!(path.trim())),
                _ => Err(format!("`{key}` must be a file path of at most {MAX_PATH_TEXT} bytes")),
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
            Kind::Custom { kind, .. } => (kind.check)(self, value),
        }
    }

    /// `value` one step up or down (`Up` and `Down` in a number field): by 1, or 10 with Shift.
    pub fn step(&self, value: &Value, up: bool, shift: bool) -> Option<Value> {
        let (Kind::Number { min, max } | Kind::Integer { min, max }) = self.kind else { return None };
        let size = if shift { 10.0 } else { 1.0 };
        let n = value.as_f64()? + if up { size } else { -size };
        self.check(&json!(n.clamp(min, max))).ok()
    }

    /// The field as `ui.dialog.inspect` lists it, without its value.
    pub fn describe(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.key));
        m.insert("key".into(), json!(self.key));
        m.insert("label".into(), json!(self.label));
        let kind = match &self.kind {
            Kind::Number { min, max } | Kind::Integer { min, max } => {
                m.insert("min".into(), json!(min));
                m.insert("max".into(), json!(max));
                if matches!(self.kind, Kind::Integer { .. }) { "integer" } else { "number" }
            }
            Kind::Bool => "bool",
            Kind::Lock => "lock",
            Kind::Text { max, lines } => {
                m.insert("max".into(), json!(max));
                m.insert("lines".into(), json!(lines));
                "text"
            }
            Kind::Choice { choices, labels } | Kind::Multi { choices, labels } => {
                m.insert("choices".into(), json!(choices));
                if !labels.is_empty() {
                    m.insert("labels".into(), json!(labels));
                }
                if matches!(self.kind, Kind::Multi { .. }) { "multi" } else { "choice" }
            }
            Kind::Folder => "folder",
            Kind::File { save, extensions } => {
                m.insert("extensions".into(), json!(extensions));
                if *save { "saveFile" } else { "openFile" }
            }
            Kind::Order { labels } => {
                m.insert("labels".into(), json!(labels));
                "order"
            }
            Kind::List { rows, select } => {
                m.insert("rows".into(), json!(rows));
                m.insert("select".into(), json!(select));
                "list"
            }
            Kind::Custom { kind, data } => {
                if let Value::Object(extra) = data {
                    m.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
                kind.name
            }
        };
        m.insert("kind".into(), json!(kind));
        m.insert("default".into(), self.default.clone());
        m.insert("doc".into(), json!(self.doc));
        if let Some(group) = &self.group {
            m.insert("group".into(), json!(group));
        }
        m
    }

    /// The craft row the field shows as.
    pub fn param(&self) -> Param {
        let label = Label::from(self.label.clone());
        let param = match &self.kind {
            Kind::Number { min, max } => {
                let default = self.default.as_f64().unwrap_or(0.0);
                Param::new(self.key.clone(), label, ParamKind::number(bound(*min), bound(*max), default, 1.0, NUMBER_DECIMALS))
            }
            Kind::Integer { min, max } => {
                let default = self.default.as_i64().unwrap_or(0);
                Param::new(self.key.clone(), label, ParamKind::int(bound(*min).map(|n| n as i64), bound(*max).map(|n| n as i64), default))
            }
            Kind::Bool | Kind::Lock => Param::new(self.key.clone(), label, ParamKind::bool(self.default.as_bool().unwrap_or(false))),
            Kind::Text { lines, .. } => {
                let param = Param::new(self.key.clone(), label, ParamKind::text(self.default.as_str().unwrap_or_default()));
                if *lines > 1 { param.multiline(*lines) } else { param }
            }
            Kind::Choice { choices, labels } => {
                let options =
                    choices.iter().enumerate().map(|(i, word)| (word.clone(), Label::from(choice_label(labels, i, word).to_string()))).collect();
                Param::new(self.key.clone(), label, ParamKind::choice(options, self.default.as_str().map(str::to_string)))
            }
            Kind::Folder => Param::new(self.key.clone(), label, ParamKind::text("")).browse(PathKind::Folder),
            Kind::File { save: true, .. } => Param::new(self.key.clone(), label, ParamKind::text("")).browse(PathKind::SaveFile),
            Kind::File { save: false, extensions } => {
                let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
                Param::new(self.key.clone(), label, ParamKind::text("")).browse(PathKind::open_file(&extensions))
            }
            Kind::Multi { .. } | Kind::List { .. } | Kind::Order { .. } | Kind::Custom { .. } => {
                let field = self.clone();
                Param::custom(self.key.clone(), label, move |ui, fields, cx| custom_row(ui, fields, cx, &field))
                    .opens_with(FieldValue::Json(self.default.clone()))
            }
        };
        if self.doc.is_empty() { param } else { param.tooltip(Label::from(self.doc.clone())) }
    }

    /// `value` as the window's row holds it.
    pub fn to_field_value(&self, value: &Value) -> FieldValue {
        match &self.kind {
            Kind::Number { .. } => FieldValue::Number(value.as_f64().unwrap_or(0.0)),
            Kind::Integer { .. } => FieldValue::Int(value.as_i64().unwrap_or(0)),
            Kind::Bool | Kind::Lock => FieldValue::Bool(value.as_bool().unwrap_or(false)),
            Kind::Text { .. } | Kind::Choice { .. } | Kind::Folder | Kind::File { .. } => {
                FieldValue::Text(value.as_str().unwrap_or_default().to_string())
            }
            Kind::Multi { .. } | Kind::List { .. } | Kind::Order { .. } | Kind::Custom { .. } => FieldValue::Json(value.clone()),
        }
    }

    /// What the window's row holds, as the JSON value the field checks; `None` for a value of no field kind.
    pub fn from_field_value(&self, value: &FieldValue) -> Option<Value> {
        Some(match value {
            FieldValue::Bool(b) => json!(b),
            FieldValue::Int(n) => json!(n),
            FieldValue::Number(n) => json!(n),
            FieldValue::Text(t) => json!(t),
            FieldValue::Json(v) => v.clone(),
            _ => return None,
        })
    }
}

fn split(pairs: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
    pairs.iter().map(|(w, l)| (w.to_string(), l.to_string())).unzip()
}

/// A bound read from a schema: the open ends (`f64::MIN`, `f64::MAX`) are no bound.
fn bound(n: f64) -> Option<f64> {
    (n.is_finite() && n > f64::MIN && n < f64::MAX).then_some(n)
}

fn choice_label<'a>(labels: &'a [String], at: usize, word: &'a str) -> &'a str {
    labels.get(at).map_or(word, String::as_str)
}

/// The control cell of a field that has no craft row of its own: checkboxes for words and selectable rows, the rows
/// as text for an order or a list that only shows, a package's own row for its kind.
fn custom_row(ui: &mut Ui, fields: &mut DialogFields, cx: &mut CustomContext, field: &Field) -> Response {
    let current = match fields.get(&field.key) {
        Some(FieldValue::Json(v)) => v.clone(),
        Some(FieldValue::List(items)) => Value::Array(items.iter().filter_map(|i| field.from_field_value(i)).collect()),
        _ => field.default.clone(),
    };
    let words: Vec<(String, String)> = match &field.kind {
        Kind::Multi { choices, labels } => choices.iter().enumerate().map(|(i, w)| (w.clone(), choice_label(labels, i, w).to_string())).collect(),
        Kind::List { rows, select: true } => rows.iter().map(|r| (r.clone(), r.clone())).collect(),
        Kind::Custom { kind, .. } => {
            if let Some(row) = kind.row {
                return row(ui, fields, cx, field);
            }
            return Text::new(Label::from(current.to_string())).show(ui);
        }
        Kind::Order { labels } => {
            let order: Vec<usize> =
                current.as_array().map(|l| l.iter().filter_map(|i| i.as_u64().and_then(|n| usize::try_from(n).ok())).collect()).unwrap_or_default();
            let lines: Vec<&str> = order.iter().filter_map(|i| labels.get(*i).map(String::as_str)).collect();
            return Text::new(Label::from(lines.join("\n"))).show(ui);
        }
        Kind::List { rows, .. } => return Text::new(Label::from(rows.join("\n"))).show(ui),
        _ => return Text::new(Label::Empty).show(ui),
    };
    let ticked: Vec<String> = current.as_array().map(|l| l.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let mut next = ticked.clone();
    let shown = Stack::new().show(ui, |ui, _| {
        for (at, (word, label)) in words.iter().enumerate() {
            let was = ticked.contains(word);
            let mut on = was;
            let _ = Checkbox::new(Bind::bound(&mut on)).label(Label::from(label.clone())).id_salt((field.key.as_str(), at)).show(ui);
            if on != was {
                next = words.iter().filter(|(w, _)| if w == word { on } else { next.contains(w) }).map(|(w, _)| w.clone()).collect();
            }
        }
    });
    if next != ticked {
        fields.set(field.key.clone(), FieldValue::Json(json!(next)));
    }
    shown.output
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
