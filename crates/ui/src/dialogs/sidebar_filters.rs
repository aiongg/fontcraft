//! The dialogs of Font View's sidebar: the Smart Filter Editor, the List Filter Editor and Missing Glyphs.
//!
//! | Dialog | Runs | Fields |
//! |---|---|---|
//! | `dialog.smartFilterEditor` | `fontView.editFilter` (with `filter`), `fontView.addSmartFilter` (new) | name, the rule tree ([`super::rule_tree`]) |
//! | `dialog.listFilterEditor` | `fontView.editFilter` (with `filter`), `fontView.addListFilter` (new) | name, glyph names |
//! | `dialog.missingGlyphs` | `fontView.generateMissingGlyphs` | the missing glyphs to add (`fontView.listMissingGlyphs`) |
//!
//! The filter editors run nothing until OK, which runs the command once; filters live in the user settings, so Cancel
//! leaves them as they were. The editor of a stored filter opens with its `filter` path and reads the stored name and
//! rules or names itself; values given with it take their place. `dialog.open {dialog}` opens the editor of a stored
//! filter, `dialog.open {command: fontView.addSmartFilter | addListFilter}` the editor of a new one.
//!
//! Missing Glyphs opens with the sidebar `entry` whose missing glyphs it lists, every one selected, so Generate right
//! away adds them all. Each change previews the new glyphs; Generate is one undo step and Cancel puts back the font the
//! dialog opened on.

use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use fontcraft_glyphquery::{Filter, RuleTree, default_filters, filter_at};
use serde_json::{Map, Value, json};

use super::fields::{Field, MAX_LIST_ROWS};
use super::rule_tree;
use super::spec::{DialogSpec, Fields, Shaped};
use crate::app::sidebar::{path_of, unique_name};

/// The dialog of a smart filter, new or stored.
pub const SMART: &str = "dialog.smartFilterEditor";
/// The dialog of a list filter, new or stored.
pub const LIST: &str = "dialog.listFilterEditor";
/// The dialog of a sidebar entry's missing glyphs.
pub const MISSING: &str = "dialog.missingGlyphs";

/// Longest name the field takes, in characters (the engine counts at most 256 bytes).
const NAME_CHARS: usize = 256;
/// Longest text of names the field takes, in characters.
const NAMES_CHARS: usize = 1 << 20;
/// Lines the names box shows.
const NAMES_LINES: usize = 10;

const NAME_DOC: &str = "The filter's name in the sidebar; unique within its folder";
const RULES_DOC: &str = "The rules a glyph must match: each group matches when all, any or none of its rows match";
const NAMES_DOC: &str = "The glyphs the filter shows, in this order, separated by spaces or line breaks; names the font lacks are allowed";

// The editor of a stored filter comes first: `dialog.open {dialog}` finds it by the id both share.
pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new(SMART, "Smart Filter", "fontView.editFilter", Fields::Shaped(Shaped { fields: edit_smart_fields, given, params, link }))
        .run_on_ok(),
    DialogSpec::new(SMART, "New Smart Filter", "fontView.addSmartFilter", Fields::Shaped(Shaped { fields: new_smart_fields, given, params, link }))
        .run_on_ok(),
    DialogSpec::new(LIST, "List Filter", "fontView.editFilter", Fields::Targeted(edit_list_fields)).run_on_ok(),
    DialogSpec::new(LIST, "New List Filter", "fontView.addListFilter", Fields::Targeted(new_list_fields)).run_on_ok(),
    DialogSpec::new(MISSING, "Missing Glyphs", "fontView.generateMissingGlyphs", Fields::Targeted(missing_fields)).buttons("Generate", ""),
];

// ── The stored filters ──

/// The Filters section as the engine reads it: the stored tree, else the built-in filters.
fn stored(s: &Session) -> Vec<Filter> {
    s.settings().reserved::<Vec<Filter>>(Reserved::Filters).ok().flatten().unwrap_or_else(default_filters)
}

/// The stored filter at the sidebar path `filter` (`filters/Folder/Name`); `kind` names the filter kind in the message.
fn stored_filter(s: &Session, filter: &Value, kind: &str) -> Result<Filter, String> {
    let mut path = path_of(filter).ok_or_else(|| format!("`filter` is the path of a {kind} filter, such as filters/Name"))?;
    let shown = path.join("/");
    if path.len() > 1 && path.first().is_some_and(|p| p == "filters") {
        path.remove(0);
    }
    let items = stored(s);
    filter_at(&items, &path).cloned().map_err(|_| format!("there is no filter `{shown}`"))
}

/// `base`, or `base 2`, `base 3`…: the first name no top-level filter has.
fn unique_filter_name(s: &Session, base: &str) -> String {
    let taken: Vec<String> = stored(s).iter().map(|f| f.name().to_string()).collect();
    unique_name(&taken, base)
}

fn name_field(name: &str) -> Field {
    Field { default: json!(name), ..Field::text("name", "Name", NAME_CHARS, NAME_DOC) }
}

// ── Smart filters ──

fn smart_fields(name: &str, rules: Value) -> Vec<Field> {
    vec![name_field(name), rule_tree::field("rules", "Rules", rules, RULES_DOC)]
}

fn new_smart_fields(s: &Session) -> Vec<Field> {
    // A new filter starts with one rule every glyph matches, as the command gives it.
    smart_fields(&unique_filter_name(s, "New Filter"), RuleTree::starter().to_json())
}

fn edit_smart_fields(_: &Session) -> Vec<Field> {
    smart_fields("", json!({"all": []}))
}

/// Field values from `{filter?, folder?, name?, rules?}`: a stored filter's name and rules unless they are given; the
/// filter and folder pass through.
fn given(s: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = p.clone();
    let Some(filter) = p.get("filter") else { return Ok(out) };
    let Filter::Smart { name, rules } = stored_filter(s, filter, "smart")? else {
        return Err(format!("`{}` is not a smart filter", path_of(filter).unwrap_or_default().join("/")));
    };
    out.entry("name").or_insert_with(|| json!(name));
    out.entry("rules").or_insert_with(|| rules.to_json());
    Ok(out)
}

/// A count rule's value typed as text that reads as a number becomes the number.
fn tidy(node: &mut Value) {
    let Some(m) = node.as_object_mut() else { return };
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

/// `{name, rules}` from the field values. The rule field's check capped the tree's depth, so `tidy` recurses within it.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = v.clone();
    if let Some(rules) = out.get_mut("rules") {
        tidy(rules);
    }
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

// ── List filters ──

fn list_fields(name: &str, names: &str) -> Vec<Field> {
    vec![name_field(name), Field { default: json!(names), ..Field::text_area("glyphs", "Glyph Names", NAMES_CHARS, NAMES_LINES, NAMES_DOC) }]
}

/// Glyph names as one text: a list joined by spaces, or the text itself.
fn joined(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(l)) => l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

fn edit_list_fields(s: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let filter = targets
        .get("filter")
        .ok_or("give `filter`: the path of the list filter to edit, such as filters/Name; fontView.addListFilter makes a new one")?;
    let Filter::List { name, glyphs } = stored_filter(s, filter, "list")? else {
        return Err(format!("`{}` is not a list filter", path_of(filter).unwrap_or_default().join("/")));
    };
    Ok(list_fields(&name, &glyphs.join(" ")))
}

fn new_list_fields(s: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    Ok(list_fields(&unique_filter_name(s, "New List"), &joined(targets.get("glyphs"))))
}

// ── Missing glyphs ──

fn missing_fields(s: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let entry = targets.get("entry").cloned().ok_or("give `entry`: the sidebar entry whose missing glyphs to list, such as languages/latin/de")?;
    let listed = s.execute("fontView.listMissingGlyphs", &json!({"entry": entry})).map_err(|e| e.to_string())?;
    let names: Vec<String> = listed
        .get("missing")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).take(MAX_LIST_ROWS).map(str::to_string).collect())
        .unwrap_or_default();
    if names.is_empty() {
        return Err(format!("the font has every glyph `{}` expects", entry.as_str().map_or_else(|| entry.to_string(), str::to_string)));
    }
    Ok(vec![Field::select_list("glyphs", "Glyphs", names.clone(), &names, "The missing glyphs to add")])
}

#[cfg(test)]
#[path = "sidebar_filters_tests.rs"]
mod tests;
