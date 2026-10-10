//! Glyph › Remove Glyph: the confirmation before `glyph.removeGlyph`. Remove runs the command
//! once; Cancel removes nothing.
//!
//! The dialog asks the command first (`confirm: false`, which changes nothing) and adds a
//! pop-up menu for each answer it needs: what happens to components that use a removed glyph
//! (remove them, or decompose them into outlines), and to metrics keys that point at one
//! (dissolve or keep them).

use std::collections::HashSet;

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode};

/// Most glyph names a field's label names before "and N more".
const NAMED: usize = 5;

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Remove", "Cancel"],
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
            &[("remove", "Remove those components"), ("decompose", "Decompose them")],
            "decompose",
            "remove deletes every component that uses a removed glyph; decompose first turns those components into outlines",
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

/// The distinct `glyph` values of the report's list `key`, in order, as a short phrase.
fn glyph_names(report: &Value, key: &str) -> String {
    let mut seen = HashSet::new();
    let mut names: Vec<&str> = Vec::new();
    for entry in report.get(key).and_then(Value::as_array).into_iter().flatten() {
        if let Some(name) = entry.get("glyph").and_then(Value::as_str)
            && seen.insert(name)
        {
            names.push(name);
        }
    }
    let shown = names.iter().take(NAMED).copied().collect::<Vec<_>>().join(", ");
    if names.len() > NAMED { format!("{shown} and {} more", names.len() - NAMED) } else { shown }
}
