//! Smart Filter Editor (`dialog.smartFilterEditor`): a smart filter's name and rules, for
//! `fontView.addSmartFilter` ([`NEW`]) and `fontView.editFilter` ([`DIALOG`]). Nothing runs
//! until OK, which runs the command once; filters live in the user settings, so Cancel leaves
//! them as they were.
//!
//! The rules are a rule tree drawn as rows ([`super::rule_tree`]): a Match row per group (all,
//! any or none), a row per rule with its property, comparison and value, nested groups indented
//! under their own Match row, and `+`, `…` and `−` buttons to add rules, add groups and remove
//! either.
//!
//! The editor of a stored filter opens with its `filter` path and reads the stored name and
//! rules itself; `name` and `rules` given with it take their place.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode, Shaped};
use crate::shell::font_view::sidebar;

/// The dialog's row in the inventory, shared by the editor of a new filter and of a stored one.
pub const ID: &str = "dialog.smartFilterEditor";
/// Longest name the field takes, in characters (the engine counts at most 256 bytes).
const NAME_CHARS: usize = 256;

/// The editor of a stored smart filter (`fontView.editFilter`).
pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    ..Dialog::new(ID, "Smart Filter", "fontView.editFilter", Fields::Shaped(Shaped { fields: edit_fields, given, params, link }))
};

/// The editor of a new smart filter (`fontView.addSmartFilter`).
pub const NEW: Dialog = Dialog {
    mode: Mode::RunOnOk,
    ..Dialog::new(ID, "New Smart Filter", "fontView.addSmartFilter", Fields::Shaped(Shaped { fields: new_fields, given, params, link }))
};

const RULES_DOC: &str = "The rules a glyph must match: each group matches when all, any or none of its rows match";

fn fields_with(name: &str, rules: Value) -> Vec<Field> {
    vec![
        Field { default: json!(name), ..Field::text("name", "Name", NAME_CHARS, "The filter's name in the sidebar; unique within its folder") },
        Field::rules("rules", "Rules", rules, RULES_DOC),
    ]
}

fn new_fields(s: &Session) -> Vec<Field> {
    // A new filter starts with one rule every glyph matches, as the command gives it.
    fields_with(&sidebar::unique_filter_name(s.settings(), "New Filter"), fontcraft_glyphquery::RuleTree::starter().to_json())
}

fn edit_fields(_: &Session) -> Vec<Field> {
    fields_with("", json!({"all": []}))
}

/// Field values from `{filter?, folder?, name?, rules?}`: a stored filter's name and rules
/// unless they are given; the filter and folder pass through.
fn given(s: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = p.clone();
    let Some(filter) = p.get("filter") else { return Ok(out) };
    let path = sidebar::path_of(filter).ok_or("`filter` is the path of a smart filter, such as filters/Name")?;
    let shown = path.join("/");
    let stored = sidebar::stored_filter(s.settings(), &path).ok_or_else(|| format!("there is no filter `{shown}`"))?;
    if stored.get("kind").and_then(Value::as_str) != Some("smart") {
        return Err(format!("`{shown}` is not a smart filter"));
    }
    for key in ["name", "rules"] {
        if !out.contains_key(key)
            && let Some(v) = stored.get(key)
        {
            out.insert(key.into(), v.clone());
        }
    }
    Ok(out)
}

/// A count rule's value typed as text that reads as a number becomes the number.
fn tidy(node: &mut Value) {
    if let Some(m) = node.as_object_mut() {
        if m.contains_key("property") {
            let counts = matches!(m.get("property").and_then(Value::as_str), Some("countOfPaths" | "countOfComponents"));
            if counts && let Some(n) = m.get("value").and_then(Value::as_str).and_then(|t| t.trim().parse::<f64>().ok()).filter(|n| n.is_finite()) {
                m.insert("value".into(), json!(n));
            }
            return;
        }
        for list in m.values_mut() {
            if let Some(nodes) = list.as_array_mut() {
                nodes.iter_mut().for_each(tidy);
            }
        }
    }
}

/// `{name, rules}` from the field values.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = v.clone();
    if let Some(rules) = out.get_mut("rules") {
        tidy(rules);
    }
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}
