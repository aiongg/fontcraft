//! Palette › Dimensions: the measurement notepad of the active master (Font View: the first), one field per key of the
//! current glyph's script (`dimensions.getValues`). Typing a number notes it, emptying a field
//! clears it (`dimensions.setValue`). The values change nothing in the font.

use serde_json::{Map, Value, json};

use super::{Pending, current_glyph, palette, widgets};
use crate::app::FontcraftApp;

/// Most fields the section lists; a font can note any number of keys.
const MAX_FIELDS: usize = 64;

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let mut query = Map::new();
    if let Some(glyph) = current_glyph(app) {
        query.insert("glyph".into(), json!(glyph));
    }
    if let (Ok(font), Some(tab)) = (app.session.font(), app.state.tab())
        && let Some(master) = crate::canvas::view::active_master(font, tab)
    {
        query.insert("master".into(), json!(master.id));
    }
    let Ok(result) = app.session.execute("dimensions.getValues", &Value::Object(query)) else {
        palette::note(app, ui, "No font master");
        return;
    };
    let master = result.get("master").cloned().unwrap_or(Value::Null);
    let empty = Map::new();
    let values = result.get("values").and_then(Value::as_object).unwrap_or(&empty);
    let keys: Vec<String> = match result.get("fields").and_then(Value::as_array) {
        Some(fields) => fields.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => values.keys().cloned().collect(),
    };
    if keys.is_empty() {
        palette::note(app, ui, "No measurements for this script");
        return;
    }
    let width = app.tokens.field_width;
    egui::Grid::new("dimensions").num_columns(2).show(ui, |ui| {
        for key in keys.iter().take(MAX_FIELDS) {
            ui.label(key);
            let value = values.get(key).and_then(Value::as_f64);
            match widgets::number(ui, widgets::id(("dimensions", key)), value, width, true) {
                Some(widgets::NumberEdit::Set(n)) => pending.push(("dimensions.setValue", json!({"master": master, "key": key, "value": n}))),
                Some(widgets::NumberEdit::Clear) => pending.push(("dimensions.setValue", json!({"master": master, "key": key, "value": null}))),
                None => {}
            }
            ui.end_row();
        }
    });
}
