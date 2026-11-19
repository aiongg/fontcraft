//! The dialogs that change the glyph set or paste into it:
//!
//! | Dialog | Command | Does |
//! |---|---|---|
//! | `dialog.addGlyphs` (Glyph › Add Glyphs…) | `glyph.addGlyphs` | a box of names, characters, uniXXXX names and ranges; Generate runs the command with `onExisting: "ask"` |
//! | `dialog.addGlyphsExisting` | `glyph.addGlyphs` | asked when that run found names the font has: Keep Existing, Replace or Cancel, previewed live |
//! | `dialog.removeGlyph` (Glyph › Remove Glyph) | `glyph.removeGlyph` | the confirmation; asks about components and metrics keys that use a removed glyph |
//! | `dialog.findAndReplaceGlyphNames` (Edit › Find › Find and Replace…) | `edit.findAndReplace` | renames the selected glyphs on Replace, all or none |
//! | `dialog.pasteSpecial` (Edit › Paste Special) | `edit.pasteSpecial` | pastes copied glyphs over glyphs of the font, previewed live |
//! | `dialog.pastedOutlineBounds` | `selection.move` | asked when `edit.paste` put the outline outside the glyph box: Move or Keep Position |

use std::collections::HashSet;

use fontcraft_engine::Session;
use fontcraft_model::{MAX_COORDINATE, MAX_NAME_LEN};
use serde_json::{Map, Value, json};

use super::fields::Field;
use super::spec::{After, DialogSpec, Fields, Shaped, Third};

/// Longest text the Add Glyphs box takes, in characters (the command takes at most 512 KB).
const MAX_ADD_TEXT: usize = 512 * 1024;
/// Lines the Add Glyphs box shows.
const ADD_TEXT_LINES: usize = 8;
/// Most glyph names a Remove Glyphs field's label names before "and N more".
const NAMED: usize = 5;

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new(
        "dialog.addGlyphs",
        "Add Glyphs",
        "glyph.addGlyphs",
        Fields::Shaped(Shaped { fields: add_fields, given: add_given, params: same, link: no_link }),
    )
    .run_on_ok()
    .buttons("Generate", "Cancel"),
    DialogSpec::new("dialog.addGlyphsExisting", "Glyphs Already in Font", "glyph.addGlyphs", Fields::Targeted(existing_fields))
        .after(After { command: "glyph.addGlyphs", params: existing_follow_up })
        .buttons("Keep Existing", "Cancel")
        .third(Third { id: "replace", caption: "Replace", key: "onExisting", ok: "keep", value: "replace", ok_caption: None })
        .message("Some of these glyphs are in the font already."),
    DialogSpec::new("dialog.removeGlyph", "Remove Glyphs", "glyph.removeGlyph", Fields::Targeted(remove_fields))
        .run_on_ok()
        .buttons("Remove", "Cancel")
        .third(Third { id: "remove", caption: "Remove", key: "components", ok: "decompose", value: "remove", ok_caption: Some("Decompose") })
        .message("The selected glyphs are removed from the font."),
    DialogSpec::new("dialog.findAndReplaceGlyphNames", "Find and Replace", "edit.findAndReplace", Fields::Fixed(find_fields))
        .run_on_ok()
        .buttons("Replace", "Cancel"),
    DialogSpec::new(
        "dialog.pasteSpecial",
        "Paste Special",
        "edit.pasteSpecial",
        Fields::Shaped(Shaped { fields: paste_fields, given: pass_through, params: paste_params, link: no_link }),
    )
    .buttons("Paste", "Cancel"),
    DialogSpec::new("dialog.pastedOutlineBounds", "Move Pasted Outline", "selection.move", Fields::Fixed(bounds_fields))
        .after(After { command: "edit.paste", params: bounds_follow_up })
        .buttons("Move", "Keep Position")
        .message("The pasted outline lies outside the glyph box."),
];

fn same(values: &Map<String, Value>) -> Map<String, Value> {
    values.clone()
}

fn no_link(_: &mut Map<String, Value>, _: &str) {}

fn pass_through(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(p.clone())
}

// ── Add Glyphs ──

fn add_fields(_: &Session) -> Vec<Field> {
    vec![Field::text_area(
        "text",
        "Glyphs",
        MAX_ADD_TEXT,
        ADD_TEXT_LINES,
        "Glyph names, characters, uniXXXX or uXXXXX names, and ranges such as a:z, separated by spaces or line breaks",
    )]
}

/// The given parameters, asking about names the font has already: `onExisting` is a target of the dialog, so the
/// selection never becomes one.
fn add_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut m = p.clone();
    m.insert("onExisting".into(), json!("ask"));
    Ok(m)
}

/// The names the font has (the `existing` target) as a list, and the answer the buttons give.
fn existing_fields(_: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
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

/// The parameters to open the question with after `glyph.addGlyphs` ran with `request` and returned `result`: the
/// request's text, the existing names, and Keep Existing. `None` unless the request asked (`onExisting: "ask"`) and a
/// name was in the font.
fn existing_follow_up(request: &Value, result: &Value) -> Option<Map<String, Value>> {
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

// ── Remove Glyphs ──

/// Asks the command first (`confirm: false`, which changes nothing) for the questions it needs: what happens to the
/// components and the metrics keys that use a removed glyph.
fn remove_fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
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

// ── Find and Replace ──

fn find_fields(_: &Session) -> Vec<Field> {
    vec![
        Field::text("find", "Find", MAX_NAME_LEN, "The text replaced in each name; empty appends Replace to every name"),
        Field::text("replace", "Replace", MAX_NAME_LEN, "What takes its place; with Regex, \\1 to \\9 name the expression's groups"),
        Field::bool("matchCase", "Match Case", true, "Find matches only the same capitals and small letters"),
        Field::bool("regex", "Regex", false, "Find is a regular expression"),
    ]
}

// ── Paste Special ──

fn paste_fields(_: &Session) -> Vec<Field> {
    vec![
        Field::choice(
            "mode",
            "Paste Into",
            &[("sameName", "Glyphs With Same Name"), ("selected", "Selected Glyphs"), ("asComponents", "As Components")],
            "sameName",
            "sameName overwrites the glyphs of the same names and adds the copied glyphs the font lacks; selected overwrites the selected glyphs in order, keeping their names; asComponents adds each copied glyph as a component to the selected glyphs",
        ),
        Field::multi(
            "parts",
            "Parts",
            &[
                ("all", "All Data"),
                ("activeLayer", "Content of Active Layer"),
                ("kerningGroups", "Kerning Groups"),
                ("metricsKeys", "Metric Keys"),
                ("lsb", "LSB"),
                ("rsb", "RSB"),
                ("width", "Width"),
            ],
            &["all"],
            "What each copied glyph transfers in the first two modes; with RSB and Width both ticked, RSB wins",
        ),
        Field::bool("allMasters", "All Masters", false, "As Components: add the components on every master, not only the one shown"),
    ]
}

/// The parts go to the first two modes and All Masters to As Components only.
fn paste_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = values.clone();
    if m.get("mode").and_then(Value::as_str) == Some("asComponents") {
        m.remove("parts");
    } else {
        m.remove("allMasters");
    }
    m
}

// ── Pasted outline outside the glyph ──

fn bounds_fields(_: &Session) -> Vec<Field> {
    let max = 2.0 * MAX_COORDINATE;
    vec![
        Field::number("dx", "Move X", -max, max, 0.0, "Horizontal move in font units that brings the pasted outline into the glyph box"),
        Field::number("dy", "Move Y", -max, max, 0.0, "Vertical move in font units that brings the pasted outline into the glyph box"),
    ]
}

/// The parameters to open the question with after `edit.paste` returned `result`: its `selection` and its
/// `correction` as `dx`, `dy`, which put the pasted box's bottom left at the origin. `None` unless the paste is out of
/// bounds.
pub fn bounds_follow_up(_request: &Value, result: &Value) -> Option<Map<String, Value>> {
    if result.get("outOfBounds").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let mut m = result.get("selection")?.as_object()?.clone();
    let correction = result.get("correction")?.as_array()?;
    let (dx, dy) = (correction.first()?.as_f64()?, correction.get(1)?.as_f64()?);
    m.insert("dx".into(), json!(dx));
    m.insert("dy".into(), json!(dy));
    Some(m)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests;
