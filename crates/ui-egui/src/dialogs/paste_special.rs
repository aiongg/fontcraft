//! Edit › Paste Special: pastes copied glyphs (the `fontcraft-glyphs` text on the clipboard) over
//! glyphs of the font with `edit.pasteSpecial`, with a live preview. Three modes: Glyphs With
//! Same Name, Selected Glyphs and As Components (with All Masters); the parts apply to the first
//! two modes, All Masters to the third. The shell opens it with the clipboard text, the selected
//! glyphs as `targets` and the master shown as `master`.

use serde_json::{Map, Value};

use super::form::Field;
use super::{Dialog, Fields, Shaped};

pub const DIALOG: Dialog = Dialog {
    buttons: ["Paste", "Cancel"],
    ..Dialog::new("dialog.pasteSpecial", "Paste Special", "edit.pasteSpecial", Fields::Shaped(Shaped { fields, given, params, link }))
};

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
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

fn given(_: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(p.clone())
}

/// The parts go to the first two modes and All Masters to As Components only.
fn params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = values.clone();
    if m.get("mode").and_then(Value::as_str) == Some("asComponents") {
        m.remove("parts");
    } else {
        m.remove("allMasters");
    }
    m
}

fn link(_: &mut Map<String, Value>, _: &str) {}
