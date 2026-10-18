//! Component from Selection (`dialog.componentFromSelection`): Glyph › Component from Selection
//! and the canvas context menu ask for the name of the glyph that will hold the selected shapes.
//!
//! The name field starts with the suggestion `glyph.componentFromSelection` itself uses: `_part.`
//! and the edited glyph's name, with `.001`, `.002`… while that name is taken. A line above it
//! says that the change applies to every master layer. Nothing runs until OK, which runs the
//! command once; the app then puts the new glyph into the tab's text after the edited glyph and
//! edits it (`shell::components::after_run`). Cancel changes nothing.

use fontcraft_engine::model::MAX_NAME_LEN;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode};

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    message: "The change applies to all master layers.",
    ..Dialog::new("dialog.componentFromSelection", "Name of the Component Glyph", "glyph.componentFromSelection", Fields::Targeted(fields))
};

/// Most numbered names tried before the suggestion gives up and leaves the name to the person.
const MAX_SUFFIX: usize = 999;

/// The suggested name for a component made from `glyph`'s shapes in `font`.
pub fn suggestion(font: &fontcraft_engine::model::Font, glyph: &str) -> String {
    let base = format!("_part.{glyph}");
    if font.glyph(&base).is_none() {
        return base;
    }
    (1..=MAX_SUFFIX).map(|n| format!("{base}.{n:03}")).find(|name| font.glyph(name).is_none()).unwrap_or(base)
}

fn fields(session: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let glyph = targets.get("glyph").and_then(Value::as_str).ok_or("Component from Selection: select the shapes of a glyph first")?;
    let font = session.font().map_err(|e| e.to_string())?;
    if font.glyph(glyph).is_none() {
        return Err(format!("Component from Selection: there is no glyph `{}`", glyph.chars().take(64).collect::<String>()));
    }
    let name = suggestion(font, glyph);
    Ok(vec![Field {
        default: json!(name),
        ..Field::text(
            "name",
            "Name of the new glyph",
            MAX_NAME_LEN,
            "the glyph that receives the selected shapes; a name starting with an underscore makes a part that does not export",
        )
    }])
}
