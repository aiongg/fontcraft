//! Guide Scope (M3.41, row `guides.globalScopeFilter`): the rules that limit a global guide to the glyphs matching
//! them, built like a smart filter's (the rule-tree field of [`super::rule_tree`]). Info for Selection opens it on a
//! global guide selected alone in Edit View (`shell::sidebar`), with the guide's address and stored `filter`.
//!
//! OK compiles the rules with `RuleTree::compile`, writes the predicate with `Expr`'s `Display` and runs
//! `guide.setProperties {filter}` once. The starter rule (every glyph) writes an empty filter, which shows the guide in
//! every glyph. A stored filter opens as one Custom rule holding its text. Rules that do not compile set nothing; the
//! Predicate line says so.

use fontcraft_engine::Session;
use fontcraft_glyphquery::RuleTree;
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field};
use super::rule_tree;
use super::spec::{DialogSpec, Fields, Shaped};

/// The dialog's id, which `dialog.open` and Info for Selection open.
pub const DIALOG: &str = "dialog.guideScope";

const RULES: &str = "rules";
const PREDICATE: &str = "predicate";

const RULES_DOC: &str = "The glyphs the guide shows in: each group matches when all, any or none of its rows match";
const PREDICATE_DOC: &str = "The glyph predicate OK stores as the guide's filter, empty for every glyph";
/// Shown in the Predicate line while the rules do not compile.
const NOT_COMPILED: &str = "These rules do not make a glyph predicate: check the Custom and matches rules";
/// Longest predicate the line shows, in bytes.
const MAX_PREDICATE_BYTES: usize = 1 << 16;

pub const SPECS: &[DialogSpec] =
    &[DialogSpec::new(DIALOG, "Guide Scope", "guide.setProperties", Fields::Shaped(Shaped { fields, given, params, link }))
        .run_on_ok()
        .annotate(preview)];

/// The Predicate line: text the dialog sets, shown read only.
const PREDICATE_KIND: Custom = Custom { name: "predicate", check: predicate_check, row: None };

fn fields(_: &Session) -> Vec<Field> {
    vec![
        rule_tree::field(RULES, "Rules", RuleTree::starter().to_json(), RULES_DOC),
        Field::custom(PREDICATE, "Predicate", PREDICATE_KIND, Value::Null, json!(""), PREDICATE_DOC),
    ]
}

fn predicate_check(field: &Field, value: &Value) -> Result<Value, String> {
    match value {
        Value::String(s) if s.len() <= MAX_PREDICATE_BYTES => Ok(value.clone()),
        _ => Err(format!("`{}` is text the dialog fills in", field.key)),
    }
}

/// Field values from `{glyph, layer, side?, guide, filter?, rules?}`: given rules, else the stored filter as one Custom
/// rule, else the starter rule. The guide's address passes through as the targets.
fn given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = p.clone();
    let filter = out.remove("filter");
    out.remove(PREDICATE);
    if !out.contains_key(RULES) {
        let text = match &filter {
            None | Some(Value::Null) => "",
            Some(Value::String(s)) => s.trim(),
            Some(_) => return Err("`filter` must be text: a glyph predicate".into()),
        };
        out.insert(RULES.into(), rules_for(text));
    }
    Ok(out)
}

/// The rule tree a stored filter opens as.
fn rules_for(filter: &str) -> Value {
    if filter.is_empty() { RuleTree::starter().to_json() } else { json!({"all": [{"property": "custom", "op": "is", "value": filter}]}) }
}

/// The predicate `rules` stand for: empty for the starter rule.
fn predicate_of(rules: &Value) -> Result<String, String> {
    let mut rules = rules.clone();
    super::sidebar_filters::tidy(&mut rules);
    let tree = RuleTree::from_json(&rules).map_err(|e| e.to_string())?;
    if tree.to_json() == RuleTree::starter().to_json() {
        return Ok(String::new());
    }
    tree.compile().map(|e| e.to_string()).map_err(|e| e.to_string())
}

/// `{filter}` from the rules; nothing when they do not compile.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(Ok(filter)) = v.get(RULES).map(predicate_of) {
        out.insert("filter".into(), json!(filter));
    }
    out
}

/// The Predicate line from the command's parameters.
fn preview(_: &Session, params: &Value) -> Map<String, Value> {
    let shown = match params.get("filter").and_then(Value::as_str) {
        Some(text) => text.chars().take(MAX_PREDICATE_BYTES / 4).collect(),
        None => NOT_COMPILED.to_string(),
    };
    let mut out = Map::new();
    out.insert(PREDICATE.into(), json!(shown));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::model::LayerSide;
    use fontcraft_tools::selection::{GuideRef, GuideScope};
    use serde_json::{Value, json};

    use super::{DIALOG, given, params};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::canvas::overlays::guides::out_of_scope;
    use crate::canvas::tools::Target;
    use crate::dialogs::form;

    /// Glyphs A and B, one global guide; A in Edit View with the guide selected.
    fn app() -> (FontCraftApp, String) {
        let mut s = Session::with_document();
        let mut run = |id: &str, params: Value| {
            s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        };
        run("glyph.newGlyph", json!({"name": "A"}));
        run("glyph.newGlyph", json!({"name": "B"}));
        run("guide.addGlobal", json!({"y": 300}));
        let master = s.font().unwrap().masters[0].id.clone();
        let mut app = FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        app.tools.selection.guides.insert(GuideRef { scope: GuideScope::Global, index: 0 });
        (app, master)
    }

    fn hidden_in(app: &FontCraftApp, glyph: &str, layer: &str) -> Vec<usize> {
        let target = Target { glyph: glyph.into(), layer: layer.into(), side: LayerSide::Foreground };
        out_of_scope(&app.session, app.session.font().unwrap(), &target)
    }

    /// Info for Selection on the global guide opens the rules; OK with a glyph-name rule hides the guide in B.
    #[test]
    fn the_scope_dialog_hides_the_guide_outside_its_rules() {
        let (mut app, master) = app();
        app.run_now(Msg::Shell(CommandRef::with_params("edit.infoForSelection", json!({}))), false).unwrap();
        assert_eq!(app.dialogs.open.as_ref().map(|d| d.id.as_str()), Some(DIALOG), "Info for Selection opens the guide's rules");
        form::set(&mut app, "rules", &json!({"all": [{"property": "glyphName", "op": "is", "value": "A"}]})).unwrap();
        form::confirm(&mut app).unwrap();
        let filter = app.session.font().unwrap().masters[0].guides[0].filter.clone().unwrap_or_default();
        assert!(!filter.is_empty(), "OK stores the compiled rules as the filter");
        assert_eq!(hidden_in(&app, "B", &master), vec![0], "the guide is hidden in B");
        assert!(hidden_in(&app, "A", &master).is_empty(), "and shown in A");
    }

    /// A stored filter opens as one Custom rule and is written back as the same predicate; the starter rule clears it.
    #[test]
    fn a_stored_filter_opens_as_a_custom_rule() {
        let rule = json!({"all": [{"property": "glyphName", "op": "is", "value": "A"}]});
        let stored = fontcraft_glyphquery::RuleTree::from_json(&rule).unwrap().compile().unwrap().to_string();
        let mut p = serde_json::Map::new();
        p.insert("guide".into(), json!({"scope": "global", "index": 0}));
        p.insert("filter".into(), json!(stored));
        let values = given(&Session::new(), &p).unwrap();
        assert_eq!(values["rules"], json!({"all": [{"property": "custom", "op": "is", "value": stored}]}));
        assert!(params(&values).get("filter").and_then(Value::as_str).is_some_and(|f| !f.is_empty()), "{:?}", params(&values));
        p.remove("filter");
        let values = given(&Session::new(), &p).unwrap();
        assert_eq!(params(&values).get("filter"), Some(&json!("")), "the starter rule writes an empty filter");
    }
}
