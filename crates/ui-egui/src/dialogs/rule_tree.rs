//! The rule-tree control of the Smart Filter Editor: a smart filter's rules as rows, nested
//! groups as indented rows under their own Match row.
//!
//! | Control | Does |
//! |---|---|
//! | Match pop-up of a group | the glyph must match all, any or none of the rows under it |
//! | `+` on a group | adds a rule at the end of the group |
//! | `…` on a group | adds a nested group at the end of the group |
//! | `−` on a nested group | removes the group and its rows |
//! | property, comparison and value of a rule | edit the rule |
//! | `+` on a rule | adds a rule after it |
//! | `…` on a rule | adds a nested group after it |
//! | `−` on a rule | removes it |
//!
//! The field's value is the tree in the shape `fontView.addSmartFilter` takes,
//! `{all | any | none: [rule or group]}` with rules `{property, op, value}`. Each change makes a
//! new tree; the engine checks the rules themselves when the command runs.

use fontcraft_glyphquery::{MAX_GROUP_DEPTH, MAX_RULES};
use serde_json::{Map, Value, json};

use super::Event;
use super::form::{Field, format_number, widget_id};
use crate::theme::Tokens;

/// Most bytes of a rule tree, as JSON, the field takes.
const MAX_TREE_BYTES: usize = 1 << 20;
/// Longest value text, in characters (the engine takes at most 1024 bytes).
const VALUE_CHARS: usize = 1024;

/// The group kinds with the labels the Match pop-up shows.
pub const MATCH: [(&str, &str); 3] = [("all", "All"), ("any", "Any"), ("none", "None")];

/// The rule properties with their labels, in the pop-up's order.
pub const PROPERTIES: [(&str, &str); 17] = [
    ("glyphName", "Glyph Name"),
    ("countOfPaths", "Count of Paths"),
    ("countOfComponents", "Count of Components"),
    ("tags", "Tags"),
    ("script", "Script"),
    ("category", "Category & Subcategory"),
    ("case", "Case"),
    ("exportGlyph", "Export Glyph"),
    ("hasUnicode", "Has Unicode"),
    ("hasCustomGlyphInfo", "Has Custom Glyph Info"),
    ("hasAnnotations", "Has Annotations"),
    ("hasMetricsKeys", "Has Metrics Keys"),
    ("metricsKeysOutOfSync", "Metrics Keys Out of Sync"),
    ("hasKerningGroups", "Has Kerning Groups"),
    ("colorLabel", "Color Label"),
    ("layerColorLabel", "Layer Color Label"),
    ("custom", "Custom"),
];

const OPS: [(&str, &str); 13] = [
    ("is", "is"),
    ("isNot", "is not"),
    ("contains", "contains"),
    ("doesNotContain", "does not contain"),
    ("beginsWith", "begins with"),
    ("endsWith", "ends with"),
    ("matches", "matches"),
    ("==", "="),
    ("!=", "≠"),
    ("<", "<"),
    ("<=", "≤"),
    (">", ">"),
    (">=", "≥"),
];

/// The comparisons each kind of property takes, the default first (as the engine checks them).
const TEXT_OPS: [&str; 7] = ["is", "isNot", "contains", "doesNotContain", "beginsWith", "endsWith", "matches"];
const NUMBER_OPS: [&str; 6] = ["==", "!=", "<", "<=", ">", ">="];
const CHOICE_OPS: [&str; 2] = ["is", "isNot"];

const CASES: [(&str, &str); 6] =
    [("upper", "Uppercase"), ("lower", "Lowercase"), ("smallCaps", "Small Caps"), ("minor", "Minor"), ("other", "Other"), ("noCase", "No Case")];

/// The colour labels by number.
const COLORS: [&str; 12] =
    ["Red", "Orange", "Brown", "Yellow", "Light Green", "Dark Green", "Cyan", "Blue", "Purple", "Pink", "Light Gray", "Dark Gray"];

/// What a rule's value is, which decides its control.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Text,
    Number,
    Bool,
    Case,
    Color,
}

fn value_kind(property: &str) -> ValueKind {
    match property {
        "countOfPaths" | "countOfComponents" => ValueKind::Number,
        "exportGlyph" | "hasUnicode" | "hasCustomGlyphInfo" | "hasAnnotations" | "hasMetricsKeys" | "metricsKeysOutOfSync" | "hasKerningGroups" => {
            ValueKind::Bool
        }
        "case" => ValueKind::Case,
        "colorLabel" | "layerColorLabel" => ValueKind::Color,
        _ => ValueKind::Text,
    }
}

pub fn ops_of(property: &str) -> &'static [&'static str] {
    match property {
        "glyphName" | "tags" | "script" => &TEXT_OPS,
        "countOfPaths" | "countOfComponents" => &NUMBER_OPS,
        _ => &CHOICE_OPS,
    }
}

/// The value a rule takes when its property changes to `property`; `None` leaves it out.
fn default_value(property: &str) -> Option<Value> {
    match (property, value_kind(property)) {
        ("category", _) => Some(json!("Letter")),
        (_, ValueKind::Text) => Some(json!("")),
        (_, ValueKind::Number) => Some(json!(0)),
        (_, ValueKind::Bool) => Some(json!(true)),
        (_, ValueKind::Case) => Some(json!("upper")),
        (_, ValueKind::Color) => Some(json!(0)),
    }
}

/// The rule a `+` adds: one every glyph matches.
fn starter_rule() -> Value {
    json!({"property": "glyphName", "op": "contains", "value": ""})
}

/// The kind and the nodes of a group.
pub fn group_of(tree: &Value) -> Option<(&str, &Vec<Value>)> {
    let m = tree.as_object().filter(|m| m.len() == 1)?;
    let (kind, list) = m.iter().next()?;
    let kind = MATCH.iter().map(|(k, _)| *k).find(|k| *k == kind.as_str())?;
    Some((kind, list.as_array()?))
}

// ── Checking ──

/// `value` when it is a rule tree within the limits, else why not. The rules' values are
/// checked by the engine when the command runs.
pub fn check(key: &str, value: &Value) -> Result<Value, String> {
    let bytes = serde_json::to_string(value).map_or(usize::MAX, |s| s.len());
    if bytes > MAX_TREE_BYTES {
        return Err(format!("`{key}` is {bytes} bytes long; at most {MAX_TREE_BYTES} are allowed"));
    }
    let mut count = 0;
    walk(value, 1, &mut count).map_err(|e| format!("`{key}`: {e}"))?;
    Ok(value.clone())
}

fn walk(group: &Value, depth: usize, count: &mut usize) -> Result<(), String> {
    if depth > MAX_GROUP_DEPTH {
        return Err(format!("rule groups are nested more than {MAX_GROUP_DEPTH} levels deep"));
    }
    let (_, nodes) = group_of(group).ok_or("a rule group is an object with one key, `all`, `any` or `none`, holding a list of rules")?;
    for node in nodes {
        *count += 1;
        if *count > MAX_RULES {
            return Err(format!("a rule tree holds at most {MAX_RULES} rules and groups"));
        }
        let m = node.as_object().ok_or("each rule is an object {property, op, value} or a nested group")?;
        match m.get("property") {
            Some(Value::String(_)) => {}
            Some(_) => return Err("a rule's `property` is text".into()),
            None => walk(node, depth + 1, count)?,
        }
    }
    Ok(())
}

fn node_count(group: &Value) -> usize {
    group_of(group).map_or(0, |(_, nodes)| nodes.iter().map(|n| 1 + if n.get("property").is_some() { 0 } else { node_count(n) }).sum())
}

// ── Editing ──

/// A change to the tree. A path lists node indexes from the top group; a group's path is empty
/// for the top group.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Match(Vec<usize>, &'static str),
    Property(Vec<usize>, &'static str),
    Op(Vec<usize>, &'static str),
    Value(Vec<usize>, Option<Value>),
    Remove(Vec<usize>),
    AddRule(Vec<usize>, usize),
    AddGroup(Vec<usize>, usize),
}

fn list_mut(group: &mut Value) -> Option<&mut Vec<Value>> {
    group.as_object_mut()?.values_mut().next()?.as_array_mut()
}

fn group_mut<'a>(tree: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let mut at = tree;
    for i in path {
        at = list_mut(at)?.get_mut(*i)?;
    }
    Some(at)
}

/// `tree` with `edit` made; `None` when the edit does not apply (a stale path, a limit reached).
pub fn apply(tree: &Value, edit: Edit) -> Option<Value> {
    let mut t = tree.clone();
    match edit {
        Edit::Match(path, kind) => {
            let group = group_mut(&mut t, &path)?;
            let nodes = std::mem::take(list_mut(group)?);
            let mut m = Map::new();
            m.insert(kind.to_string(), Value::Array(nodes));
            *group = Value::Object(m);
        }
        Edit::Property(path, property) => {
            let rule = group_mut(&mut t, &path)?.as_object_mut()?;
            let old = rule.get("property").and_then(Value::as_str).unwrap_or_default().to_string();
            rule.insert("property".into(), json!(property));
            let op = rule.get("op").and_then(Value::as_str).unwrap_or_default();
            if !ops_of(property).contains(&op) {
                rule.insert("op".into(), json!(ops_of(property).first().copied().unwrap_or("is")));
            }
            if value_kind(&old) != value_kind(property) || (old == "category") != (property == "category") {
                match default_value(property) {
                    Some(v) => rule.insert("value".into(), v),
                    None => rule.remove("value"),
                };
            }
        }
        Edit::Op(path, op) => {
            group_mut(&mut t, &path)?.as_object_mut()?.insert("op".into(), json!(op));
        }
        Edit::Value(path, value) => {
            let rule = group_mut(&mut t, &path)?.as_object_mut()?;
            match value {
                Some(v) => rule.insert("value".into(), v),
                None => rule.remove("value"),
            };
        }
        Edit::Remove(path) => {
            let (last, parent) = path.split_last()?;
            let nodes = list_mut(group_mut(&mut t, parent)?)?;
            if *last >= nodes.len() {
                return None;
            }
            nodes.remove(*last);
        }
        Edit::AddRule(path, at) => insert(&mut t, &path, at, starter_rule())?,
        Edit::AddGroup(path, at) => {
            // The new group sits one level below the group at `path`, which is itself
            // `path.len() + 1` levels deep.
            if path.len() + 2 > MAX_GROUP_DEPTH {
                return None;
            }
            insert(&mut t, &path, at, json!({"any": [starter_rule()]}))?;
        }
    }
    Some(t)
}

/// Puts `node` at `at` in the group at `path`, unless the tree is full.
fn insert(tree: &mut Value, path: &[usize], at: usize, node: Value) -> Option<()> {
    if node_count(tree) >= MAX_RULES {
        return None;
    }
    let nodes = list_mut(group_mut(tree, path)?)?;
    nodes.insert(at.min(nodes.len()), node);
    Some(())
}

// ── Drawing ──

/// Draws the tree; what the person changes becomes one [`Event::Set`] of the new tree.
pub fn draw(ui: &mut egui::Ui, field: &Field, value: &Value, tokens: &Tokens, events: &mut Vec<Event>) -> egui::Response {
    let mut edits = Vec::new();
    let response = ui.vertical(|ui| group_ui(ui, field, value, &[], 0, tokens, &mut edits)).response;
    let mut tree = value.clone();
    let mut changed = false;
    for edit in edits {
        if let Some(next) = apply(&tree, edit) {
            tree = next;
            changed = true;
        }
    }
    if changed {
        events.push(Event::Set(field.key.clone(), tree));
    }
    response
}

/// The id of a control of the row at `path`: `match`, `add`, `addGroup` and `remove` of a
/// group; `property`, `op`, `value`, `remove`, `add` and `addGroup` of a rule.
pub fn control_id(key: &str, path: &[usize], part: &str) -> egui::Id {
    widget_id(key).with((path, part))
}

/// Levels of nesting the property column absorbs, so the comparison and value columns of
/// rules at those levels line up.
const ALIGNED_LEVELS: f32 = 3.0;

fn indent_width(tokens: &Tokens, depth: usize) -> f32 {
    tokens.dialog_spacing * 2.0 * depth as f32
}

fn indent(ui: &mut egui::Ui, tokens: &Tokens, depth: usize) {
    if depth > 0 {
        ui.add_space(indent_width(tokens, depth));
    }
}

fn label_of<'a>(list: &'a [(&'a str, &'a str)], word: &'a str) -> &'a str {
    list.iter().find(|(w, _)| *w == word).map_or(word, |(_, l)| l)
}

/// A pop-up of `(word, label)` pairs; the word picked, if one was.
fn popup(ui: &mut egui::Ui, id: egui::Id, width: f32, choices: &[(&'static str, &'static str)], current: &str) -> Option<&'static str> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(id).width(width).selected_text(label_of(choices, current)).show_ui(ui, |ui| {
        for (word, label) in choices {
            if ui.selectable_label(*word == current, *label).clicked() && *word != current {
                picked = Some(*word);
            }
        }
    });
    picked
}

fn small(ui: &mut egui::Ui, id: egui::Id, text: &str, hint: &str) -> bool {
    crate::panels::widgets::button(ui, id, text, hint, true, false)
}

fn group_ui(ui: &mut egui::Ui, field: &Field, group: &Value, path: &[usize], depth: usize, tokens: &Tokens, edits: &mut Vec<Edit>) {
    let Some((kind, nodes)) = group_of(group) else {
        ui.colored_label(tokens.error_text, "not a rule group");
        return;
    };
    ui.horizontal(|ui| {
        indent(ui, tokens, depth);
        ui.label("Match");
        if let Some(k) = popup(ui, control_id(&field.key, path, "match"), tokens.dialog_field_width, &MATCH, kind) {
            edits.push(Edit::Match(path.to_vec(), k));
        }
        ui.label(if depth == 0 { "of the following rules" } else { "of these rules" });
        if small(ui, control_id(&field.key, path, "add"), "+", "Add a rule") {
            edits.push(Edit::AddRule(path.to_vec(), nodes.len()));
        }
        if small(ui, control_id(&field.key, path, "addGroup"), "…", "Add a nested group") {
            edits.push(Edit::AddGroup(path.to_vec(), nodes.len()));
        }
        if depth > 0 && small(ui, control_id(&field.key, path, "remove"), "−", "Remove this group") {
            edits.push(Edit::Remove(path.to_vec()));
        }
    });
    for (i, node) in nodes.iter().enumerate() {
        let mut at = path.to_vec();
        at.push(i);
        if node.get("property").is_some() {
            rule_ui(ui, field, node, &at, depth + 1, tokens, edits);
        } else if depth + 1 < MAX_GROUP_DEPTH {
            group_ui(ui, field, node, &at, depth + 1, tokens, edits);
        }
    }
}

/// A rule's value as its text field shows it.
fn value_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.as_f64().map(format_number).unwrap_or_default(),
        Some(other) => other.to_string(),
    }
}

/// Typed text as a rule's value: a number for the count rules once it reads as one.
fn typed_value(kind: ValueKind, text: &str) -> Value {
    match kind {
        ValueKind::Number => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| json!(text), |n| json!(n)),
        _ => json!(text),
    }
}

/// The colour label a rule's value names: its number, or `None` for no label.
fn color_of(v: Option<&Value>) -> Option<usize> {
    match v {
        Some(Value::Number(n)) => n.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < COLORS.len()),
        Some(Value::String(s)) => {
            let key: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase().replace("grey", "gray");
            COLORS.iter().position(|c| c.replace(' ', "").to_lowercase() == key)
        }
        _ => None,
    }
}

fn rule_ui(ui: &mut egui::Ui, field: &Field, rule: &Value, path: &[usize], depth: usize, tokens: &Tokens, edits: &mut Vec<Edit>) {
    let property = rule.get("property").and_then(Value::as_str).unwrap_or_default();
    let ops = ops_of(property);
    let op = rule.get("op").and_then(Value::as_str).or(ops.first().copied()).unwrap_or("is");
    let value = rule.get("value");
    let width = tokens.dialog_field_width;
    ui.horizontal(|ui| {
        indent(ui, tokens, depth);
        // The property column narrows as the rule is nested deeper, keeping the next columns in line.
        let column = width * 2.2 + indent_width(tokens, 1) * ALIGNED_LEVELS;
        let property_width = (column - indent_width(tokens, depth)).max(width * 2.2);
        if let Some(p) = popup(ui, control_id(&field.key, path, "property"), property_width, &PROPERTIES, property) {
            edits.push(Edit::Property(path.to_vec(), p));
        }
        let op_choices: Vec<(&'static str, &'static str)> = ops.iter().map(|o| (*o, label_of(&OPS, o))).collect();
        if let Some(o) = popup(ui, control_id(&field.key, path, "op"), width * 1.3, &op_choices, op) {
            edits.push(Edit::Op(path.to_vec(), o));
        }
        let kind = value_kind(property);
        let value_id = control_id(&field.key, path, "value");
        match kind {
            ValueKind::Text | ValueKind::Number => {
                let shown = value_text(value);
                let focused = ui.memory(|m| m.has_focus(value_id));
                // While the field has the keyboard it shows what is typed: `1.` stays `1.`.
                let mut text = if focused { ui.data(|d| d.get_temp::<String>(value_id)).unwrap_or_else(|| shown.clone()) } else { shown };
                let response = ui.add(egui::TextEdit::singleline(&mut text).id(value_id).char_limit(VALUE_CHARS).desired_width(width * 1.8));
                if response.changed() {
                    edits.push(Edit::Value(path.to_vec(), Some(typed_value(kind, &text))));
                }
                if response.has_focus() {
                    ui.data_mut(|d| d.insert_temp(value_id, text));
                } else {
                    ui.data_mut(|d| d.remove::<String>(value_id));
                }
            }
            ValueKind::Bool => {
                let on = value.and_then(Value::as_bool).unwrap_or(true);
                if let Some(w) = popup(ui, value_id, width * 1.8, &[("yes", "Yes"), ("no", "No")], if on { "yes" } else { "no" }) {
                    edits.push(Edit::Value(path.to_vec(), Some(json!(w == "yes"))));
                }
            }
            ValueKind::Case => {
                let current = value.and_then(Value::as_str).unwrap_or_default();
                if let Some(c) = popup(ui, value_id, width * 1.8, &CASES, current) {
                    edits.push(Edit::Value(path.to_vec(), Some(json!(c))));
                }
            }
            ValueKind::Color => {
                const WORDS: [&str; 13] = ["none", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];
                let choices: Vec<(&'static str, &'static str)> =
                    WORDS.iter().enumerate().map(|(i, w)| (*w, if i == 0 { "No Label" } else { COLORS.get(i - 1).copied().unwrap_or("") })).collect();
                let current = color_of(value).and_then(|i| WORDS.get(i + 1)).copied().unwrap_or("none");
                if let Some(w) = popup(ui, value_id, width * 1.8, &choices, current) {
                    edits.push(Edit::Value(path.to_vec(), w.parse::<u64>().ok().map(|n| json!(n))));
                }
            }
        }
        let (last, parent) = path.split_last().map_or((0, &[][..]), |(l, p)| (*l, p));
        if small(ui, control_id(&field.key, path, "remove"), "−", "Remove this rule") {
            edits.push(Edit::Remove(path.to_vec()));
        }
        if small(ui, control_id(&field.key, path, "add"), "+", "Add a rule after this one") {
            edits.push(Edit::AddRule(parent.to_vec(), last + 1));
        }
        if small(ui, control_id(&field.key, path, "addGroup"), "…", "Add a nested group after this rule") {
            edits.push(Edit::AddGroup(parent.to_vec(), last + 1));
        }
    });
}
