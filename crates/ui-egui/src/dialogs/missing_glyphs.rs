//! Missing Glyphs (`dialog.missingGlyphs`): the glyphs a sidebar entry expects that the font
//! lacks (`fontView.listMissingGlyphs`), as a list to select from (click, Shift+click,
//! Cmd+click, Cmd+A); Generate adds the selected ones (`fontView.generateMissingGlyphs`) as one
//! undo step. It opens from the context menu of a sidebar entry that knows its full set and
//! lacks some of it.
//!
//! Every missing glyph starts selected, so Generate right away adds them all. Each change
//! previews the new glyphs; Cancel puts back the font the dialog opened on. The list holds at
//! most [`MAX_LIST_ROWS`] names; the command without `glyphs` adds every missing glyph.

use serde_json::{Map, Value, json};

use super::form::{Field, MAX_LIST_ROWS};
use super::{Dialog, Fields};

pub const DIALOG: Dialog = Dialog {
    buttons: ["Generate", "Cancel"],
    ..Dialog::new("dialog.missingGlyphs", "Missing Glyphs", "fontView.generateMissingGlyphs", Fields::Targeted(fields))
};

fn fields(s: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
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
