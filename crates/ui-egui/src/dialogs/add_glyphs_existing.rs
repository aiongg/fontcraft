//! Glyphs already in the font. Asked after `glyph.addGlyphs` with `onExisting: "ask"` found
//! names the font has (`existing`) and so added nothing: OK runs it again with the choice, Keep
//! Existing (only the new names are added) or Replace (those glyphs become new empty ones), with
//! a live preview; Cancel adds nothing at all. [`follow_up`] gives the parameters to open it with
//! from the request and the result.

use serde_json::{Map, Value, json};

use super::form::Field;
use super::{After, Dialog, Fields};

/// Most existing names the question names before "and N more".
const NAMED: usize = 8;

pub const DIALOG: Dialog = Dialog {
    follow_up: true,
    after: Some(After { command: "glyph.addGlyphs", params: follow_up }),
    buttons: ["Add Glyphs", "Cancel"],
    message: "Some of these glyphs are in the font already.",
    ..Dialog::new("dialog.addGlyphsExisting", "Glyphs Already in Font", "glyph.addGlyphs", Fields::Targeted(fields))
};

/// One pop-up menu whose label names the glyphs the font has (the `existing` target).
fn fields(_: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let names: Vec<&str> =
        targets.get("existing").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut label = names.iter().take(NAMED).copied().collect::<Vec<_>>().join(", ");
    if names.len() > NAMED {
        label.push_str(&format!(" and {} more", names.len() - NAMED));
    }
    let label = if label.is_empty() { "Glyphs in the font".to_string() } else { format!("In the font: {label}") };
    Ok(vec![Field::choice(
        "onExisting",
        &label,
        &[("keep", "Keep Existing"), ("replace", "Replace")],
        "keep",
        "keep adds only the names the font lacks; replace also makes each glyph the font has a new empty glyph in its place",
    )])
}

/// The parameters to open the question with after `glyph.addGlyphs` ran with `request` and
/// returned `result`: the request's text, the existing names, and Keep Existing. `None` unless
/// the request asked (`onExisting: "ask"`) and a name was in the font.
fn follow_up(request: &Value, result: &Value) -> Option<Map<String, Value>> {
    if request.get("onExisting").and_then(Value::as_str) != Some("ask") {
        return None;
    }
    let existing = result.get("existing")?.as_array()?;
    if existing.is_empty() {
        return None;
    }
    let mut m = Map::new();
    m.insert("text".into(), json!(request.get("text")?.as_str()?));
    m.insert("existing".into(), Value::Array(existing.clone()));
    m.insert("onExisting".into(), json!("keep"));
    Some(m)
}
