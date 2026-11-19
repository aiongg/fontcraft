//! The rule-tree field of the Smart Filter Editor ([`RULES`]): a smart filter's rules as rows, nested groups as
//! indented rows under their own Match row.
//!
//! | Control | Does |
//! |---|---|
//! | Match pop-up of a group | the glyph must match all, any or none of the rows under it |
//! | `+` on a group | adds a rule at the end of the group |
//! | `…` on a group, or `+` with Alt held | adds a nested group at the end of the group |
//! | `−` on a nested group | removes the group and its rows |
//! | property, comparison and value of a rule | edit the rule |
//! | `+` on a rule | adds a rule after it |
//! | `…` on a rule, or `+` with Alt held | adds a nested group after it |
//! | `−` on a rule | removes it |
//!
//! The field's value is the tree in the shape `fontView.addSmartFilter` takes, `{all | any | none: [rule or group]}`
//! with rules `{property, op, value}`. Each change makes a new tree; the engine checks the rules themselves when the
//! command runs. Over the control channel the whole tree is set with `ui.dialog.set`.

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::{Response, Ui};
use fontcraft_glyphquery::{MAX_GROUP_DEPTH, MAX_RULES};
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field, format_number};

craft::ui::words! {
    /// The words of the rule rows.
    pub struct RuleTreeWords {
        /// The text before a group's Match pop-up, and the pop-up's name.
        match_word: "fontcraftRuleTree.match" = "Match";
        /// The text after the top group's Match pop-up.
        following: "fontcraftRuleTree.following" = "of the following rules";
        /// The text after a nested group's Match pop-up.
        these: "fontcraftRuleTree.these" = "of these rules";
        add_rule: "fontcraftRuleTree.addRule" = "Add a rule";
        add_group: "fontcraftRuleTree.addGroup" = "Add a nested group";
        remove_group: "fontcraftRuleTree.removeGroup" = "Remove this group";
        add_rule_after: "fontcraftRuleTree.addRuleAfter" = "Add a rule after this one";
        add_group_after: "fontcraftRuleTree.addGroupAfter" = "Add a nested group after this rule";
        remove_rule: "fontcraftRuleTree.removeRule" = "Remove this rule";
        /// The names of a rule's three controls.
        property: "fontcraftRuleTree.property" = "Property";
        comparison: "fontcraftRuleTree.comparison" = "Comparison";
        value: "fontcraftRuleTree.value" = "Value";
        /// A value that is no rule group.
        not_a_group: "fontcraftRuleTree.notAGroup" = "Not a rule group";
    }
}

/// The field kind of a rule tree.
pub const RULES: Custom = Custom { name: "rules", check, row: Some(row) };

/// A rule-tree field starting as `tree`.
pub fn field(key: &str, label: &str, tree: Value, doc: &str) -> Field {
    Field::custom(key, label, RULES, Value::Null, tree, doc)
}

/// Most bytes of a rule tree, as JSON, the field takes.
const MAX_TREE_BYTES: usize = 1 << 20;
/// Longest value text, in characters (the engine takes at most 1024 bytes).
const VALUE_CHARS: usize = 1024;

/// The group kinds with the labels the Match pop-up shows.
pub const MATCH: [(&str, &str); 3] = [("all", "All"), ("any", "Any"), ("none", "None")];

/// The rule properties with their labels, in the pop-up's order.
pub const PROPERTIES: [(&str, &str); 27] = [
    ("glyphName", "Glyph Name"),
    ("countOfPaths", "Count of Paths"),
    ("countOfComponents", "Count of Components"),
    ("tags", "Tags"),
    ("script", "Script"),
    ("category", "Category & Subcategory"),
    ("case", "Case"),
    ("exportGlyph", "Export Glyph"),
    ("hasUnicode", "Has Unicode"),
    ("hasComponents", "Has Components"),
    ("hasHints", "Has Hints"),
    ("hasPostScriptHints", "Has PostScript Hints"),
    ("hasTrueTypeHints", "Has TrueType Hints"),
    ("hasCorners", "Has Corners"),
    ("hasAnchors", "Has Anchors"),
    ("containsComponent", "Contains Component"),
    ("hasCustomGlyphInfo", "Has Custom Glyph Info"),
    ("hasAnnotations", "Has Annotations"),
    ("isAutoAligned", "Is Auto-Aligned"),
    ("hasMetricsKeys", "Has Metrics Keys"),
    ("metricsKeysOutOfSync", "Metrics Keys Out of Sync"),
    ("hasKerningGroups", "Has Kerning Groups"),
    ("masterCompatible", "Master Compatible"),
    ("hasSpecialLayers", "Has Special Layers"),
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

const YES_NO: [(&str, &str); 2] = [("yes", "Yes"), ("no", "No")];

const CASES: [(&str, &str); 6] =
    [("upper", "Uppercase"), ("lower", "Lowercase"), ("smallCaps", "Small Caps"), ("minor", "Minor"), ("other", "Other"), ("noCase", "No Case")];

/// The colour labels by number.
const COLORS: [&str; 12] =
    ["Red", "Orange", "Brown", "Yellow", "Light Green", "Dark Green", "Cyan", "Blue", "Purple", "Pink", "Light Gray", "Dark Gray"];

/// The words of the colour pop-up: no label, then the label numbers.
const COLOR_WORDS: [&str; 13] = ["none", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];

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
        "exportGlyph"
        | "hasUnicode"
        | "hasComponents"
        | "hasHints"
        | "hasPostScriptHints"
        | "hasTrueTypeHints"
        | "hasCorners"
        | "hasAnchors"
        | "hasCustomGlyphInfo"
        | "hasAnnotations"
        | "isAutoAligned"
        | "hasMetricsKeys"
        | "metricsKeysOutOfSync"
        | "hasKerningGroups"
        | "masterCompatible"
        | "hasSpecialLayers" => ValueKind::Bool,
        "case" => ValueKind::Case,
        "colorLabel" | "layerColorLabel" => ValueKind::Color,
        _ => ValueKind::Text,
    }
}

/// The comparisons `property` takes, the default first.
pub fn ops_of(property: &str) -> &'static [&'static str] {
    match property {
        "glyphName" | "tags" | "script" => &TEXT_OPS,
        "countOfPaths" | "countOfComponents" => &NUMBER_OPS,
        _ => &CHOICE_OPS,
    }
}

/// The value a rule takes when its property changes to `property`.
fn default_value(property: &str) -> Value {
    match (property, value_kind(property)) {
        ("category", _) => json!("Letter"),
        (_, ValueKind::Text) => json!(""),
        (_, ValueKind::Number | ValueKind::Color) => json!(0),
        (_, ValueKind::Bool) => json!(true),
        (_, ValueKind::Case) => json!("upper"),
    }
}

/// The rule a `+` adds: one every glyph matches.
fn starter_rule() -> Value {
    json!({"property": "glyphName", "op": "contains", "value": ""})
}

/// The kind and the nodes of a group.
pub fn group_of(tree: &Value) -> Option<(&'static str, &Vec<Value>)> {
    let m = tree.as_object().filter(|m| m.len() == 1)?;
    let (kind, list) = m.iter().next()?;
    let kind = MATCH.iter().map(|(k, _)| *k).find(|k| *k == kind.as_str())?;
    Some((kind, list.as_array()?))
}

// ── Checking ──

/// `value` when it is a rule tree within the limits, else why not. The rules' values are checked by the engine when
/// the command runs.
fn check(field: &Field, value: &Value) -> Result<Value, String> {
    let key = &field.key;
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

/// The rules and groups of a tree that [`check`] took (so its depth is capped).
fn node_count(group: &Value) -> usize {
    group_of(group).map_or(0, |(_, nodes)| nodes.iter().map(|n| 1 + if n.get("property").is_some() { 0 } else { node_count(n) }).sum())
}

// ── Editing ──

/// A change to the tree. A path lists node indexes from the top group; the top group's path is empty.
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
                rule.insert("value".into(), default_value(property));
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
            // The new group sits one level below the group at `path`, which is itself `path.len() + 1` levels deep.
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

// ── The rows ──

/// The field's control cell: the tree as rows. What the person changes is set into the row values as the new tree.
fn row(ui: &mut Ui, fields: &mut DialogFields, _: &mut CustomContext, field: &Field) -> Response {
    let words = Env::of(ui).words::<RuleTreeWords>();
    let tree = match fields.get(&field.key) {
        Some(FieldValue::Json(v)) => v.clone(),
        _ => field.default.clone(),
    };
    let mut edits = Vec::new();
    let shown = Stack::new().id_salt(("fontcraftRuleTree", field.key.as_str())).show(ui, |ui, _| {
        group_rows(ui, &words, &field.key, &tree, &[], 0, &mut edits);
    });
    let mut next = tree.clone();
    for edit in edits {
        if let Some(t) = apply(&next, edit) {
            next = t;
        }
    }
    if next != tree {
        fields.set(field.key.clone(), FieldValue::Json(next));
    }
    shown.output
}

fn entries(list: &[(&str, &str)]) -> Vec<Entry<String>> {
    list.iter().map(|(word, label)| Entry::new((*word).to_string(), Label::from((*label).to_string()))).collect()
}

/// A pop-up of `(word, label)` pairs showing `current`; the word picked, if it is another.
fn popup(ui: &mut Ui, label: &Label, salt: (&str, &[usize], &str), list: &[(&'static str, &'static str)], current: &str) -> Option<&'static str> {
    let picked = Select::new(label.clone(), Bind::given(current.to_string()), &entries(list)).id_salt(salt).show(ui).events.committed().cloned()?;
    list.iter().map(|(w, _)| *w).find(|w| *w == picked && *w != current)
}

fn button(ui: &mut Ui, icon: &'static str, label: &Label, salt: (&str, &[usize], &str)) -> Option<craft::keymap::Mods> {
    let out = IconButton::new(IconId::new(icon)).label(label.clone()).tooltip(label.clone()).id_salt(salt).show(ui);
    out.clicked().then_some(out.mods)
}

fn depth_u8(depth: usize) -> u8 {
    u8::try_from(depth).unwrap_or(u8::MAX)
}

fn group_rows(ui: &mut Ui, words: &RuleTreeWords, key: &str, group: &Value, path: &[usize], depth: usize, edits: &mut Vec<Edit>) {
    let Some((kind, nodes)) = group_of(group) else {
        Text::new(words.not_a_group.clone()).show(ui);
        return;
    };
    let _ = Row::new().indent(depth_u8(depth)).id_salt((key, path, "group")).show(ui, |ui, _| {
        Text::new(words.match_word.clone()).show(ui);
        if let Some(k) = popup(ui, &words.match_word, (key, path, "match"), &MATCH, kind) {
            edits.push(Edit::Match(path.to_vec(), k));
        }
        Text::new(if depth == 0 { words.following.clone() } else { words.these.clone() }).show(ui);
        if let Some(mods) = button(ui, "plus", &words.add_rule, (key, path, "add")) {
            edits.push(if mods.alt { Edit::AddGroup(path.to_vec(), nodes.len()) } else { Edit::AddRule(path.to_vec(), nodes.len()) });
        }
        if button(ui, "more", &words.add_group, (key, path, "addGroup")).is_some() {
            edits.push(Edit::AddGroup(path.to_vec(), nodes.len()));
        }
        if depth > 0 && button(ui, "minus", &words.remove_group, (key, path, "remove")).is_some() {
            edits.push(Edit::Remove(path.to_vec()));
        }
    });
    for (i, node) in nodes.iter().enumerate() {
        let mut at = path.to_vec();
        at.push(i);
        if node.get("property").is_some() {
            rule_row(ui, words, key, node, &at, depth + 1, edits);
        } else if depth + 1 < MAX_GROUP_DEPTH {
            group_rows(ui, words, key, node, &at, depth + 1, edits);
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

fn label_of<'a>(list: &[(&'a str, &'a str)], word: &str) -> &'a str {
    list.iter().find(|(w, _)| *w == word).map_or("", |(_, l)| l)
}

fn rule_row(ui: &mut Ui, words: &RuleTreeWords, key: &str, rule: &Value, path: &[usize], depth: usize, edits: &mut Vec<Edit>) {
    let property = rule.get("property").and_then(Value::as_str).unwrap_or_default();
    let ops = ops_of(property);
    let op = rule.get("op").and_then(Value::as_str).or(ops.first().copied()).unwrap_or("is");
    let value = rule.get("value");
    let _ = Row::new().indent(depth_u8(depth)).id_salt((key, path, "rule")).show(ui, |ui, _| {
        if let Some(p) = popup(ui, &words.property, (key, path, "property"), &PROPERTIES, property) {
            edits.push(Edit::Property(path.to_vec(), p));
        }
        let op_choices: Vec<(&'static str, &'static str)> = ops.iter().map(|o| (*o, label_of(&OPS, o))).collect();
        if let Some(o) = popup(ui, &words.comparison, (key, path, "op"), &op_choices, op) {
            edits.push(Edit::Op(path.to_vec(), o));
        }
        let kind = value_kind(property);
        let salt = (key, path, "value");
        match kind {
            ValueKind::Text | ValueKind::Number => {
                let shown = value_text(value);
                let out = TextField::new(words.value.clone(), Bind::given(shown.clone())).limit(VALUE_CHARS).id_salt(salt).show(ui);
                if let Some(text) = out.events.committed()
                    && *text != shown
                {
                    edits.push(Edit::Value(path.to_vec(), Some(typed_value(kind, text))));
                }
            }
            ValueKind::Bool => {
                let on = value.and_then(Value::as_bool).unwrap_or(true);
                if let Some(w) = popup(ui, &words.value, salt, &YES_NO, if on { "yes" } else { "no" }) {
                    edits.push(Edit::Value(path.to_vec(), Some(json!(w == "yes"))));
                }
            }
            ValueKind::Case => {
                let current = value.and_then(Value::as_str).unwrap_or_default();
                if let Some(c) = popup(ui, &words.value, salt, &CASES, current) {
                    edits.push(Edit::Value(path.to_vec(), Some(json!(c))));
                }
            }
            ValueKind::Color => {
                let choices: Vec<(&'static str, &'static str)> = COLOR_WORDS
                    .iter()
                    .enumerate()
                    .map(|(i, w)| (*w, if i == 0 { "No Label" } else { COLORS.get(i - 1).copied().unwrap_or_default() }))
                    .collect();
                let current = color_of(value).and_then(|i| COLOR_WORDS.get(i + 1)).copied().unwrap_or("none");
                if let Some(w) = popup(ui, &words.value, salt, &choices, current) {
                    edits.push(Edit::Value(path.to_vec(), w.parse::<u64>().ok().map(|n| json!(n))));
                }
            }
        }
        let (last, parent) = path.split_last().map_or((0, &[][..]), |(l, p)| (*l, p));
        if button(ui, "minus", &words.remove_rule, (key, path, "remove")).is_some() {
            edits.push(Edit::Remove(path.to_vec()));
        }
        if let Some(mods) = button(ui, "plus", &words.add_rule_after, (key, path, "add")) {
            edits.push(if mods.alt { Edit::AddGroup(parent.to_vec(), last + 1) } else { Edit::AddRule(parent.to_vec(), last + 1) });
        }
        if button(ui, "more", &words.add_group_after, (key, path, "addGroup")).is_some() {
            edits.push(Edit::AddGroup(parent.to_vec(), last + 1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edits keep the tree in the command's shape: a property change resets the comparison and value it no
    /// longer fits, groups nest no deeper than the engine takes, and stale paths change nothing.
    #[test]
    fn edits_keep_the_tree_in_the_commands_shape() {
        let tree = json!({"all": [starter_rule()]});
        let counted = apply(&tree, Edit::Property(vec![0], "countOfPaths")).unwrap();
        assert_eq!(counted, json!({"all": [{"property": "countOfPaths", "op": "==", "value": 0}]}));
        let any = apply(&counted, Edit::Match(vec![], "any")).unwrap();
        assert_eq!(group_of(&any).map(|(k, n)| (k, n.len())), Some(("any", 1)));
        assert_eq!(apply(&tree, Edit::Remove(vec![3])), None, "a stale path");

        let mut deep = tree.clone();
        let mut path = Vec::new();
        while let Some(next) = apply(&deep, Edit::AddGroup(path.clone(), 0)) {
            deep = next;
            path.push(0);
        }
        assert_eq!(path.len() + 1, MAX_GROUP_DEPTH, "groups stop at the engine's depth");
        let field = field("rules", "Rules", Value::Null, "");
        assert!(check(&field, &deep).is_ok());
        assert!(check(&field, &json!({"some": []})).is_err());
    }
}
