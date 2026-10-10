//! Smart Filter Editor (`dialog.smartFilterEditor`): a smart filter's name and rules, for
//! `fontView.addSmartFilter` ([`NEW`]) and `fontView.editFilter` ([`DIALOG`]). Nothing runs
//! until OK, which runs the command once; filters live in the user settings, so Cancel leaves
//! them as they were.
//!
//! The rules show as [`ROWS`] rows of property, comparison and value under one Match choice:
//! a glyph must match all the rows, any of them, or none. A row whose property is Nested Group
//! holds a group as rule-tree JSON in its value (`{"any": [rule…]}`). A filter with more
//! top-level rules than rows opens with the rest in the last row as a nested group, which
//! matches the same glyphs (all of all is all; any or none of the rest is the same with the rest
//! as one any-group).
//!
//! Changing a row's property keeps its comparison when the property takes it, else picks the
//! property's first comparison.
//!
//! The editor of an existing filter opens with the parameters in the command's shape:
//! `{filter, name, rules}`. The sidebar passes the stored name and rules; an agent passes them
//! too, or sets the fields.

use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode, Shaped};
use crate::shell::font_view::sidebar;

/// The dialog's row in the inventory, shared by the editor of a new filter and of a stored one.
pub const ID: &str = "dialog.smartFilterEditor";
/// Rule rows the dialog shows.
pub const ROWS: usize = 6;
/// Longest name the field takes, in characters (the engine counts at most 256 bytes).
const NAME_CHARS: usize = 256;
/// Longest value the field takes, in characters: room for a nested group's JSON.
const VALUE_CHARS: usize = 16_384;
/// The property of a row that holds a nested group.
const GROUP: &str = "group";

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

const MATCH: [(&str, &str); 3] = [("all", "All rows"), ("any", "Any row"), ("none", "No row")];

/// The rule properties with their labels; an empty property leaves the row out.
const PROPERTIES: [(&str, &str); 16] = [
    ("", "No Rule"),
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
    ("colorLabel", "Color Label"),
    ("layerColorLabel", "Layer Color Label"),
    ("custom", "Custom"),
    (GROUP, "Nested Group"),
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
    ("!=", "!="),
    ("<", "<"),
    ("<=", "<="),
    (">", ">"),
    (">=", ">="),
];

/// The comparisons each kind of property takes, the default first (as the engine checks them).
const TEXT_OPS: [&str; 7] = ["is", "isNot", "contains", "doesNotContain", "beginsWith", "endsWith", "matches"];
const NUMBER_OPS: [&str; 6] = ["==", "!=", "<", "<=", ">", ">="];
const CHOICE_OPS: [&str; 2] = ["is", "isNot"];
const NUMBER_PROPERTIES: [&str; 2] = ["countOfPaths", "countOfComponents"];
const BOOL_PROPERTIES: [&str; 4] = ["exportGlyph", "hasUnicode", "hasCustomGlyphInfo", "hasAnnotations"];
const COLOR_PROPERTIES: [&str; 2] = ["colorLabel", "layerColorLabel"];

fn ops_of(property: &str) -> &'static [&'static str] {
    match property {
        "glyphName" | "tags" | "script" => &TEXT_OPS,
        p if NUMBER_PROPERTIES.contains(&p) => &NUMBER_OPS,
        _ => &CHOICE_OPS,
    }
}

/// The key of a row's field: `rule1Property`, `rule1Op`, `rule1Value`.
fn key(row: usize, part: &str) -> String {
    format!("rule{row}{part}")
}

fn fields_with(name: &str, starter: bool) -> Vec<Field> {
    let mut out = vec![
        Field { default: json!(name), ..Field::text("name", "Name", NAME_CHARS, "The filter's name in the sidebar; unique within its folder") },
        Field::choice("match", "Match", &MATCH, "all", "Which rows a glyph must match: all of them, any of them, or none"),
    ];
    for row in 1..=ROWS {
        // A new filter starts with one rule every glyph matches, as the command gives it.
        let (property, op) = if row == 1 && starter { ("glyphName", "contains") } else { ("", "is") };
        out.push(Field::choice(
            &key(row, "Property"),
            &format!("Rule {row}"),
            &PROPERTIES,
            property,
            "What the rule inspects; No Rule leaves the row out",
        ));
        out.push(Field::choice(&key(row, "Op"), "Comparison", &OPS, op, "How the value compares; each property takes some of them"));
        out.push(Field::text(
            &key(row, "Value"),
            "Value",
            VALUE_CHARS,
            "Text to compare with; a number for the counts; yes or no for Export Glyph and the Has rules (empty is yes); a label number from 0 to 11, a colour name or empty (no label) for the colour rules; predicate text for Custom; rule-tree JSON for a nested group",
        ));
    }
    out
}

fn new_fields(s: &fontcraft_engine::Session) -> Vec<Field> {
    fields_with(&sidebar::unique_filter_name(s.settings(), "New Filter"), true)
}

fn edit_fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    fields_with("", false)
}

/// A rule's value as its field shows it.
fn value_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => (if *b { "yes" } else { "no" }).to_string(),
        Some(Value::Number(n)) => n.as_f64().map(super::form::format_number).unwrap_or_default(),
        Some(other) => other.to_string(),
    }
}

/// A row's property, comparison and value for one node of a rule tree.
fn row_of(node: &Value) -> (String, String, String) {
    match node.get("property").and_then(Value::as_str) {
        Some(p) => (
            p.to_string(),
            node.get("op").and_then(Value::as_str).unwrap_or(ops_of(p).first().copied().unwrap_or("is")).to_string(),
            value_text(node.get("value")),
        ),
        None => (GROUP.to_string(), "is".to_string(), node.to_string()),
    }
}

/// The group key and the nodes of a rule tree.
fn group_of(tree: &Value) -> Option<(&str, &[Value])> {
    let m = tree.as_object().filter(|m| m.len() == 1)?;
    let (kind, list) = m.iter().next()?;
    let kind = MATCH.iter().map(|(k, _)| *k).find(|k| *k == kind.as_str())?;
    Some((kind, list.as_array()?.as_slice()))
}

/// Field values from `{filter?, folder?, name?, rules?}`; the filter and folder pass through.
fn given(p: &Map<String, Value>) -> Map<String, Value> {
    let mut out: Map<String, Value> = p.iter().filter(|(k, _)| k.as_str() != "rules").map(|(k, v)| (k.clone(), v.clone())).collect();
    let Some((kind, nodes)) = p.get("rules").and_then(group_of) else { return out };
    out.insert("match".into(), json!(kind));
    let mut rows: Vec<(String, String, String)> = Vec::new();
    if nodes.len() > ROWS {
        rows.extend(nodes.get(..ROWS - 1).unwrap_or_default().iter().map(row_of));
        let mut rest = Map::new();
        let inner = if kind == "all" { "all" } else { "any" };
        rest.insert(inner.into(), Value::Array(nodes.get(ROWS - 1..).unwrap_or_default().to_vec()));
        rows.push((GROUP.to_string(), "is".to_string(), Value::Object(rest).to_string()));
    } else {
        rows.extend(nodes.iter().map(row_of));
    }
    for row in 1..=ROWS {
        let (property, op, value) = rows.get(row - 1).cloned().unwrap_or_else(|| (String::new(), "is".into(), String::new()));
        out.insert(key(row, "Property"), json!(property));
        out.insert(key(row, "Op"), json!(op));
        out.insert(key(row, "Value"), json!(value));
    }
    out
}

/// A rule's value from its field's text; `None` leaves `value` out (yes, or no label).
fn value_of(property: &str, text: &str) -> Option<Value> {
    let t = text.trim();
    if NUMBER_PROPERTIES.contains(&property) {
        return Some(t.parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| json!(text), |n| json!(n)));
    }
    if BOOL_PROPERTIES.contains(&property) {
        return match t.to_lowercase().as_str() {
            "" => None,
            "yes" | "true" => Some(json!(true)),
            "no" | "false" => Some(json!(false)),
            _ => Some(json!(text)),
        };
    }
    if COLOR_PROPERTIES.contains(&property) {
        return match t {
            "" => None,
            _ => Some(t.parse::<i64>().map_or_else(|_| json!(t), |n| json!(n))),
        };
    }
    Some(json!(text))
}

/// `{name, rules}` from the field values.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut rules = Vec::new();
    for row in 1..=ROWS {
        let property = text(&key(row, "Property"));
        let value = text(&key(row, "Value"));
        match property.as_str() {
            "" => {}
            // Text that does not read as JSON goes to the engine as it is, which says why it
            // is not a group.
            GROUP => rules.push(serde_json::from_str::<Value>(&value).unwrap_or(Value::String(value))),
            p => {
                let mut rule = Map::new();
                rule.insert("property".into(), json!(p));
                rule.insert("op".into(), json!(text(&key(row, "Op"))));
                if let Some(val) = value_of(p, &value) {
                    rule.insert("value".into(), val);
                }
                rules.push(Value::Object(rule));
            }
        }
    }
    let kind = Some(text("match")).filter(|k| MATCH.iter().any(|(m, _)| m == k)).unwrap_or_else(|| "all".into());
    let mut tree = Map::new();
    tree.insert(kind, Value::Array(rules));
    let mut out = Map::new();
    out.insert("name".into(), json!(text("name")));
    out.insert("rules".into(), Value::Object(tree));
    out
}

/// A new property keeps the row's comparison when it takes it, else gets its first one.
fn link(v: &mut Map<String, Value>, changed: &str) {
    let Some(row) = changed.strip_prefix("rule").and_then(|r| r.strip_suffix("Property")) else { return };
    let property = v.get(changed).and_then(Value::as_str).unwrap_or_default().to_string();
    let op_key = format!("rule{row}Op");
    let op = v.get(&op_key).and_then(Value::as_str).unwrap_or_default();
    let ops = ops_of(&property);
    if property != GROUP && !property.is_empty() && !ops.contains(&op) {
        v.insert(op_key, json!(ops.first().copied().unwrap_or("is")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_longer_than_the_rows_keeps_its_rest_in_a_nested_group() {
        let rule = |n: usize| json!({"property": "glyphName", "op": "contains", "value": format!("{n}")});
        let nodes: Vec<Value> = (0..ROWS + 2).map(rule).collect();
        let mut p = Map::new();
        p.insert("rules".into(), json!({"none": nodes}));
        let values = given(&p);
        let last = values.get(&key(ROWS, "Value")).and_then(Value::as_str).unwrap();
        assert_eq!(values.get(&key(ROWS, "Property")), Some(&json!(GROUP)));
        assert_eq!(serde_json::from_str::<Value>(last).unwrap(), json!({"any": nodes[ROWS - 1..]}));
        let tree = params(&values)["rules"].clone();
        assert_eq!(tree["none"].as_array().unwrap().len(), ROWS);
    }
}
