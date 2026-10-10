//! Glyph › Add Glyphs…: a box of glyph names, characters, uniXXXX names and ranges, separated
//! by spaces or line breaks.
//! Generate runs `glyph.addGlyphs` with `onExisting: "ask"`: with no name in the font already it
//! adds them all; otherwise it changes nothing and `dialog.addGlyphsExisting` asks what to do with
//! the names the font has.

use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Mode, Shaped};

/// Longest text the field takes, in characters (the command takes at most 512 KB).
const MAX_TEXT: usize = 512 * 1024;
/// Lines the field shows.
const TEXT_LINES: usize = 8;

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Generate", "Cancel"],
    ..Dialog::new("dialog.addGlyphs", "Add Glyphs", "glyph.addGlyphs", Fields::Shaped(Shaped { fields, given, params, link }))
};

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field::text_area(
        "text",
        "Glyphs",
        MAX_TEXT,
        TEXT_LINES,
        "Glyph names, characters, uniXXXX or uXXXXX names, and ranges such as a:z, separated by spaces or line breaks",
    )]
}

/// The given parameters, asking about names the font has already: `onExisting` is a target of
/// the dialog, so the selection never becomes one.
fn given(_: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut m = p.clone();
    m.insert("onExisting".into(), json!("ask"));
    Ok(m)
}

fn params(values: &Map<String, Value>) -> Map<String, Value> {
    values.clone()
}

fn link(_: &mut Map<String, Value>, _: &str) {}
