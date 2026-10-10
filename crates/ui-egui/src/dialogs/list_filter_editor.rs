//! List Filter Editor (`dialog.listFilterEditor`): a list filter's name and glyph names, for
//! `fontView.addListFilter` ([`NEW`]) and `fontView.editFilter` ([`DIALOG`]). Nothing runs
//! until OK, which runs the command once; filters live in the user settings, so Cancel leaves
//! them as they were.
//!
//! The names are one text, separated by spaces or line breaks, as the commands take them. A new
//! list filter opens with the glyphs selected in Font View; the editor of a stored one opens
//! with its name and names, read from the settings by its `filter` path.

use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode};
use crate::shell::font_view::sidebar;

/// The dialog's row in the inventory, shared by the editor of a new filter and of a stored one.
pub const ID: &str = "dialog.listFilterEditor";
/// Longest name the field takes, in characters (the engine counts at most 256 bytes).
const NAME_CHARS: usize = 256;
/// Longest text of names the field takes, in characters.
const NAMES_CHARS: usize = 1 << 20;

/// The editor of a stored list filter (`fontView.editFilter`).
pub const DIALOG: Dialog = Dialog { mode: Mode::RunOnOk, ..Dialog::new(ID, "List Filter", "fontView.editFilter", Fields::Targeted(edit_fields)) };

/// The editor of a new list filter (`fontView.addListFilter`).
pub const NEW: Dialog = Dialog { mode: Mode::RunOnOk, ..Dialog::new(ID, "New List Filter", "fontView.addListFilter", Fields::Targeted(new_fields)) };

fn fields(name: &str, names: &str) -> Vec<Field> {
    vec![
        Field { default: json!(name), ..Field::text("name", "Name", NAME_CHARS, "The filter's name in the sidebar; unique within its folder") },
        Field {
            default: json!(names),
            ..Field::text(
                "glyphs",
                "Glyph Names",
                NAMES_CHARS,
                "The glyphs the filter shows, in this order, separated by spaces or line breaks; names the font lacks are allowed",
            )
        },
    ]
}

/// The names of a list of strings, one text.
fn joined(v: Option<&Value>) -> String {
    v.and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

fn edit_fields(s: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let path = targets
        .get("filter")
        .and_then(sidebar::path_of)
        .ok_or("give `filter`: the path of the list filter to edit, such as filters/Name; fontView.addListFilter makes a new one")?;
    let shown = path.join("/");
    let stored = sidebar::stored_filter(s.settings(), &path).ok_or_else(|| format!("there is no filter `{shown}`"))?;
    if stored.get("kind").and_then(Value::as_str) != Some("list") {
        return Err(format!("`{shown}` is not a list filter"));
    }
    Ok(fields(stored.get("name").and_then(Value::as_str).unwrap_or_default(), &joined(stored.get("glyphs"))))
}

fn new_fields(s: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    Ok(fields(&sidebar::unique_filter_name(s.settings(), "New List"), &joined(targets.get("glyphs"))))
}
