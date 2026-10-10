//! Glyph › Remove Glyph: the confirmation before `glyph.removeGlyph`. Each button runs the
//! command once; Cancel removes nothing.
//!
//! The dialog asks the command first (`confirm: false`, which changes nothing) for the answers
//! it needs. When a removed glyph is used as a component, the default button Decompose turns
//! those components into outlines first, so the glyphs that use it keep their shapes, and Remove
//! deletes the components. When metrics keys point at a removed glyph, a pop-up menu asks
//! whether to dissolve or keep them.

use std::collections::HashSet;

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode, Third};

/// Most glyph names a field's label names before "and N more".
const NAMED: usize = 5;

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Remove", "Cancel"],
    third: Some(Third { id: "remove", caption: "Remove", key: "components", ok: "decompose", value: "remove", ok_caption: Some("Decompose") }),
    message: "The selected glyphs are removed from the font.",
    ..Dialog::new("dialog.removeGlyph", "Remove Glyphs", "glyph.removeGlyph", Fields::Targeted(fields))
};

fn fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let glyphs = match (targets.get("glyphs"), targets.get("glyph")) {
        (Some(list), _) => list.clone(),
        (None, Some(one)) => json!([one]),
        (None, None) => return Err("Remove Glyphs: select the glyphs to remove first".into()),
    };
    let report = session.execute("glyph.removeGlyph", &json!({"glyphs": glyphs, "confirm": false})).map_err(|e| e.to_string())?;
    let needs: Vec<&str> = report.get("needs").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut fields = Vec::new();
    if needs.contains(&"components") {
        let users = glyph_names(&report, "usedAsComponent");
        fields.push(Field::choice(
            "components",
            &format!("Used as a component in {users}"),
            &[("decompose", "Decompose them"), ("remove", "Remove those components")],
            "decompose",
            "decompose first turns every component that uses a removed glyph into outlines; remove deletes those components",
        ));
    }
    if needs.contains(&"metricsKeys") {
        let users = glyph_names(&report, "metricsKeys");
        fields.push(Field::choice(
            "metricsKeys",
            &format!("Metrics keys of {users} point at them"),
            &[("dissolve", "Dissolve those keys"), ("keep", "Keep them")],
            "dissolve",
            "dissolve clears the metrics keys that name a removed glyph; keep leaves them pointing at a glyph that is gone",
        ));
    }
    Ok(fields)
}

/// The distinct `glyph` values of the report's list `key`, in order.
fn distinct_glyphs<'a>(report: &'a Value, key: &str) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    let mut names: Vec<&str> = Vec::new();
    for entry in report.get(key).and_then(Value::as_array).into_iter().flatten() {
        if let Some(name) = entry.get("glyph").and_then(Value::as_str)
            && seen.insert(name)
        {
            names.push(name);
        }
    }
    names
}

/// The distinct `glyph` values of the report's list `key`, in order, as a short phrase.
fn glyph_names(report: &Value, key: &str) -> String {
    let names = distinct_glyphs(report, key);
    let shown = names.iter().take(NAMED).copied().collect::<Vec<_>>().join(", ");
    if names.len() > NAMED { format!("{shown} and {} more", names.len() - NAMED) } else { shown }
}
