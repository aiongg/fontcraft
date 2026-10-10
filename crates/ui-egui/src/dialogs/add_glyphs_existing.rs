//! Glyphs already in the font. Asked after `glyph.addGlyphs` with `onExisting: "ask"` found
//! names the font has (`existing`) and so added nothing. The dialog lists those names; Keep
//! Existing runs the command again adding only the new names, Replace runs it making each of
//! those glyphs a new empty one, and Cancel adds nothing at all. The answer previews live
//! (Keep Existing until Replace is pressed). [`follow_up`] gives the parameters to open it with
//! from the request and the result.

use serde_json::{Map, Value, json};

use super::form::Field;
use super::{After, Dialog, Fields, Third};

pub const DIALOG: Dialog = Dialog {
    follow_up: true,
    after: Some(After { command: "glyph.addGlyphs", params: follow_up }),
    buttons: ["Keep Existing", "Cancel"],
    third: Some(Third { id: "replace", caption: "Replace", key: "onExisting", ok: "keep", value: "replace" }),
    message: "Some of these glyphs are in the font already.",
    ..Dialog::new("dialog.addGlyphsExisting", "Glyphs Already in Font", "glyph.addGlyphs", Fields::Targeted(fields))
};

/// The names the font has (the `existing` target) as a list, and the answer the buttons give.
fn fields(_: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let names: Vec<String> = targets
        .get("existing")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    Ok(vec![
        Field::rows("existing", "In the font", names, "The requested glyphs the font has already"),
        Field::choice(
            "onExisting",
            "",
            &[("keep", "Keep Existing"), ("replace", "Replace")],
            "keep",
            "keep adds only the names the font lacks; replace also makes each glyph the font has a new empty glyph in its place",
        ),
    ])
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
