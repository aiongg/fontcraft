//! Strike Properties (row `dialog.strikeProperties`): the dialog over `bitmap.setStrikeProperties` that the Bitmap
//! Glyph Editor's Strike Properties button opens. Set Bitmap Width (row `dialog.bitmapWidth`): the dialog over
//! `bitmap.setPixels` with `advance` that the canvas menu's Set Width opens on its glyph and strike.
//!
//! Strike Properties edits one strike, named by the `strike` parameter (`{ppem, depth}`); opened without one it takes the strike the
//! Bitmap Glyph Editor shows, else the font's first. The fields open on that strike's name, size in points,
//! resolution, ascent, descent and file properties. The properties are a table, one row a property: its name, its value
//! and Remove; Add Property adds a row. A value is a whole number, or text: what reads as a whole number is one, and
//! quotes around a value make it text. The `properties` field holds the list as OK sends it, `[{name, value}]`. Nothing
//! changes while the dialog is open; OK runs the command once, one undo step, and a value the command refuses (a name
//! given twice) keeps the dialog open with the message.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::{Response, Ui};
use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use fontcraft_engine::model::strike::{BitmapStrike, MAX_STRIKE_COORDINATE, MAX_STRIKE_PROPERTIES, MAX_STRIKE_STRING, StrikeProperty, StrikeValue};
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field};
use super::spec::{DialogSpec, Fields, Shaped};
use crate::app::FontCraftApp;

/// Longest strike name the field takes, in characters.
const MAX_NAME: usize = 1_024;
/// Most property rows the table shows; further properties are kept as they are.
const MAX_SHOWN_PROPERTIES: usize = 500;
/// Highest resolution the fields take, in dots per inch.
const MAX_RESOLUTION: i64 = 100_000;

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new(
        "dialog.strikeProperties",
        "Strike Properties",
        "bitmap.setStrikeProperties",
        Fields::Shaped(Shaped { fields, given, params, link }),
    )
    .run_on_ok()
    .from_window(window),
    DialogSpec::new(
        "dialog.bitmapWidth",
        "Set Bitmap Width",
        "bitmap.setPixels",
        Fields::Shaped(Shaped { fields: width_fields, given: width_given, params: width_params, link }),
    )
    .run_on_ok()
    .from_window(width_window),
];

craft::ui::words! {
    /// The words of the Strike Properties table.
    pub struct StrikeWords {
        name: "fontcraftStrikeProperties.name" = "Property {n}";
        value: "fontcraftStrikeProperties.value" = "Value {n}";
        remove: "fontcraftStrikeProperties.remove" = "Remove";
        add: "fontcraftStrikeProperties.add" = "Add Property";
        none: "fontcraftStrikeProperties.none" = "The strike has no properties";
        more: "fontcraftStrikeProperties.more" = "more";
    }
}

/// The property table.
pub const PROPERTIES_KIND: Custom = Custom { name: "strikeProperties", check, row: Some(row) };

fn fields(_: &Session) -> Vec<Field> {
    let c = f64::from(MAX_STRIKE_COORDINATE);
    let ci = i64::from(MAX_STRIKE_COORDINATE);
    vec![
        Field::text("name", "Name", MAX_NAME, "the strike's name, the font descriptor a BDF file starts with"),
        Field::number("pointSize", "Point Size", 0.0, c, 0.0, "the size in points the strike is drawn for"),
        Field::integer("resolutionX", "Horizontal Resolution", 0, MAX_RESOLUTION, 0, "dots per inch across"),
        Field::integer("resolutionY", "Vertical Resolution", 0, MAX_RESOLUTION, 0, "dots per inch up"),
        Field::integer("ascent", "Ascent", -ci, ci, 0, "pixels above the baseline"),
        Field::integer("descent", "Descent", -ci, ci, 0, "pixels below the baseline"),
        Field::custom(
            "properties",
            "Properties",
            PROPERTIES_KIND,
            Value::Null,
            json!([]),
            "the BDF properties: [{name, value (a whole number, or text)}]",
        ),
    ]
}

/// The strike the window offers: the one the Bitmap Glyph Editor shows, else the font's first.
fn window(app: &FontCraftApp) -> Map<String, Value> {
    let mut out = Map::new();
    let Ok(font) = app.session.font() else { return out };
    let strike = crate::app::models::bitmap_editor::shown(font, &app.view).map(|(_, s)| s).or_else(|| font.bitmap_strikes.first());
    if let Some(s) = strike {
        out.insert("strike".into(), json!({"ppem": s.ppem, "depth": s.depth}));
    }
    out
}

/// The fields of the strike `p` names, which passes through as the command's target.
fn given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let strike = wanted_strike(font, p)?;
    let mut out = values(strike);
    out.insert("strike".into(), json!({"ppem": strike.ppem, "depth": strike.depth}));
    for (key, value) in p {
        if key != "strike" {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(out)
}

/// The strike `p` names: without a depth, the lowest depth of that size.
fn wanted_strike<'f>(font: &'f Font, p: &Map<String, Value>) -> Result<&'f BitmapStrike, String> {
    let wanted = p.get("strike").ok_or("the font has no bitmap strikes")?;
    let ppem = wanted.get("ppem").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or("`strike.ppem` must be a whole number")?;
    let depth = wanted.get("depth").and_then(Value::as_u64).and_then(|d| u8::try_from(d).ok());
    font.bitmap_strikes
        .iter()
        .filter(|s| s.ppem == ppem && depth.is_none_or(|d| s.depth == d))
        .min_by_key(|s| s.depth)
        .ok_or_else(|| format!("the font has no strike of {ppem} pixels"))
}

/// The fields' values for `strike`.
fn values(strike: &BitmapStrike) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("name".into(), json!(strike.name));
    out.insert("pointSize".into(), json!(strike.point_size));
    let [rx, ry] = strike.resolution;
    out.insert("resolutionX".into(), json!(rx));
    out.insert("resolutionY".into(), json!(ry));
    out.insert("ascent".into(), json!(strike.ascent));
    out.insert("descent".into(), json!(strike.descent));
    out.insert("properties".into(), to_value(&strike.properties));
    out
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for key in ["name", "pointSize", "ascent", "descent"] {
        if let Some(value) = v.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    let x = v.get("resolutionX").and_then(Value::as_i64).unwrap_or(0);
    let y = v.get("resolutionY").and_then(Value::as_i64).unwrap_or(0);
    out.insert("resolution".into(), json!([x, y]));
    let properties = v.get("properties").map(properties).and_then(Result::ok).unwrap_or_default();
    out.insert("properties".into(), to_value(&properties));
    out
}

/// The list a `properties` value holds.
fn properties(value: &Value) -> Result<Vec<StrikeProperty>, String> {
    let list = value.as_array().ok_or("the properties are not a list")?;
    if list.len() > MAX_STRIKE_PROPERTIES {
        return Err(format!("a strike has at most {MAX_STRIKE_PROPERTIES} properties"));
    }
    let one = |v: &Value| {
        let name = v.get("name").and_then(Value::as_str).ok_or("`name` must be text")?.to_string();
        let value = match v.get("value") {
            Some(Value::String(t)) if t.len() <= MAX_STRIKE_STRING => StrikeValue::Text(t.clone()),
            Some(Value::String(_)) => return Err("the value is too long"),
            Some(n) => StrikeValue::Integer(n.as_i64().ok_or("`value` must be text or a whole number")?),
            None => return Err("`value` is required"),
        };
        Ok(StrikeProperty { name, value })
    };
    list.iter().enumerate().map(|(i, v)| one(v).map_err(|e| format!("property {}: {e}", i + 1))).collect()
}

fn to_value(list: &[StrikeProperty]) -> Value {
    let one = |p: &StrikeProperty| match &p.value {
        StrikeValue::Integer(n) => json!({"name": p.name, "value": n}),
        StrikeValue::Text(t) => json!({"name": p.name, "value": t}),
    };
    Value::Array(list.iter().map(one).collect())
}

fn check(field: &Field, value: &Value) -> Result<Value, String> {
    properties(value).map(|list| to_value(&list)).map_err(|e| format!("`{}`: {e}", field.key))
}

/// A property's value as its cell shows it: text that reads as a whole number or stands in quotes gets quotes.
fn cell_text(value: &StrikeValue) -> String {
    match value {
        StrikeValue::Integer(n) => n.to_string(),
        StrikeValue::Text(t) if parse_cell(t) != StrikeValue::Text(t.clone()) => format!("\"{t}\""),
        StrikeValue::Text(t) => t.clone(),
    }
}

/// The value a cell's text stands for: a whole number, text in quotes, or the text itself.
pub fn parse_cell(text: &str) -> StrikeValue {
    let text = text.trim();
    match text.parse::<i64>() {
        Ok(n) => StrikeValue::Integer(n),
        Err(_) => StrikeValue::Text(text.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(text).to_string()),
    }
}

/// One edit the table made.
enum Edit {
    Name(usize, String),
    Value(usize, StrikeValue),
    Remove(usize),
    Add,
}

fn apply(mut list: Vec<StrikeProperty>, edit: Edit) -> Vec<StrikeProperty> {
    match edit {
        Edit::Name(i, name) => {
            if let Some(p) = list.get_mut(i) {
                p.name = name.trim().to_string();
            }
        }
        Edit::Value(i, value) => {
            if let Some(p) = list.get_mut(i) {
                p.value = value;
            }
        }
        Edit::Remove(i) => {
            if i < list.len() {
                list.remove(i);
            }
        }
        Edit::Add => {
            // One of these names is free: there are more of them than properties.
            let free = (1..=list.len().saturating_add(1)).map(|n| format!("PROPERTY_{n}")).find(|n| !list.iter().any(|p| p.name == *n));
            if let Some(name) = free.filter(|_| list.len() < MAX_STRIKE_PROPERTIES) {
                list.push(StrikeProperty { name, value: StrikeValue::Text(String::new()) });
            }
        }
    }
    list
}

fn row(ui: &mut Ui, rows: &mut DialogFields, _: &mut CustomContext, field: &Field) -> Response {
    let words = Env::of(ui).words::<StrikeWords>();
    let list = match rows.get(&field.key) {
        Some(FieldValue::Json(v)) => properties(v).unwrap_or_default(),
        _ => Vec::new(),
    };
    let numbered = |label: &Label, i: usize| Label::from(label.as_str().replace("{n}", &(i + 1).to_string()));
    let mut edit = None;
    let shown = Stack::new().show(ui, |ui, _| {
        if list.is_empty() {
            let _ = Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        }
        for (i, p) in list.iter().take(MAX_SHOWN_PROPERTIES).enumerate() {
            let _ = Row::new().show(ui, |ui, _| {
                let name = TextField::new(numbered(&words.name, i), Bind::given(p.name.clone())).show(ui);
                if let Some(text) = name.events.committed()
                    && text.trim() != p.name
                {
                    edit = Some(Edit::Name(i, text.clone()));
                }
                let value = TextField::new(numbered(&words.value, i), Bind::given(cell_text(&p.value))).show(ui);
                if let Some(text) = value.events.committed() {
                    let typed = parse_cell(text);
                    if typed != p.value {
                        edit = Some(Edit::Value(i, typed));
                    }
                }
                if Button::new(words.remove.clone()).show(ui).clicked() {
                    edit = Some(Edit::Remove(i));
                }
            });
        }
        if list.len() > MAX_SHOWN_PROPERTIES {
            let hidden = list.len().saturating_sub(MAX_SHOWN_PROPERTIES);
            let _ = Text::new(format!("{hidden} {}", words.more.as_str())).shade(Shade::Muted).show(ui);
        }
        if list.len() < MAX_STRIKE_PROPERTIES && Button::new(words.add.clone()).show(ui).clicked() {
            edit = Some(Edit::Add);
        }
    });
    if let Some(edit) = edit {
        rows.set(field.key.clone(), FieldValue::Json(to_value(&apply(list, edit))));
    }
    shown.output
}

fn width_fields(_: &Session) -> Vec<Field> {
    let c = i64::from(MAX_STRIKE_COORDINATE);
    vec![Field::integer("advance", "Width", -c, c, 0, "the bitmap's advance in pixels")]
}

/// The glyph and strike the window offers: the ones the Bitmap Glyph Editor shows.
fn width_window(app: &FontCraftApp) -> Map<String, Value> {
    let mut out = Map::new();
    let Ok(font) = app.session.font() else { return out };
    if let Some((glyph, strike)) = crate::app::models::bitmap_editor::shown(font, &app.view) {
        out.insert("glyph".into(), json!(glyph.name));
        out.insert("strike".into(), json!({"ppem": strike.ppem, "depth": strike.depth}));
    }
    out
}

/// The advance the glyph's bitmap has in the strike; the glyph and the strike pass through as the command's targets.
fn width_given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let name = p.get("glyph").and_then(Value::as_str).ok_or("`glyph` is required")?;
    let strike = wanted_strike(font, p)?;
    let mut out = Map::new();
    if let Some(bitmap) = strike.glyph(name) {
        out.insert("advance".into(), json!(bitmap.advance));
    }
    out.extend(p.clone());
    out.insert("strike".into(), json!({"ppem": strike.ppem, "depth": strike.depth}));
    Ok(out)
}

fn width_params(v: &Map<String, Value>) -> Map<String, Value> {
    v.get("advance").map(|a| Map::from_iter([("advance".to_string(), a.clone())])).unwrap_or_default()
}

fn link(_: &mut Map<String, Value>, _: &str) {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use serde_json::json;

    use fontcraft_engine::model::strike::{StrikeProperty, StrikeValue};

    use super::{Edit, apply, cell_text, parse_cell, to_value};
    use crate::app::{APP, FontCraftApp};

    /// A cell's text becomes a typed value and shows as text that becomes the same value again.
    #[test]
    fn cells_hold_typed_values() {
        let text = |t: &str| StrikeValue::Text(t.into());
        for (typed, value) in [("120", StrikeValue::Integer(120)), ("\"120\"", text("120")), ("made here", text("made here")), ("", text(""))] {
            assert_eq!(parse_cell(typed), value, "{typed}");
            assert_eq!(parse_cell(&cell_text(&value)), value, "{typed}");
        }
        assert_eq!(parse_cell(&cell_text(&text("\"quoted\""))), text("\"quoted\""));
    }

    /// Add Property takes a free name; Remove takes the row out.
    #[test]
    fn rows_are_added_and_removed() {
        let first = StrikeProperty { name: "PROPERTY_1".into(), value: StrikeValue::Integer(3) };
        let list = apply(vec![first.clone()], Edit::Add);
        assert_eq!(to_value(&list), json!([{"name": "PROPERTY_1", "value": 3}, {"name": "PROPERTY_2", "value": ""}]));
        let list = apply(list, Edit::Value(1, StrikeValue::Integer(7)));
        assert_eq!(to_value(&apply(list, Edit::Remove(0))), json!([{"name": "PROPERTY_2", "value": 7}]));
    }

    /// Set Bitmap Width opens on the bitmap's advance and sets it in a font without outlines.
    #[test]
    fn the_width_dialog_sets_the_advance() {
        let mut app = FontCraftApp::with_edit_tab(fontcraft_engine::Session::with_document(), Arc::new(Host::builder(APP).build()));
        let names: Vec<String> = app.session.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect();
        if fontcraft_engine::cmd::bitmap_edit::has_outlines(app.session.font().unwrap()) {
            app.session.execute("glyph.removeGlyph", &json!({"glyphs": names})).unwrap();
            app.session.execute("glyph.newGlyph", &json!({"name": "dot"})).unwrap();
        }
        let glyph = app.session.font().unwrap().glyphs.iter().next().unwrap().name.clone();
        app.session.execute("bitmap.setStrikes", &json!({"add": [{"ppem": 12}]})).unwrap();
        let strike = json!({"ppem": 12, "depth": 1});
        app.session.execute("bitmap.setPixels", &json!({"glyph": glyph, "strike": strike, "advance": 7})).unwrap();
        let opened = json!({"dialog": "dialog.bitmapWidth", "params": {"glyph": glyph, "strike": strike}});
        crate::dialogs::form::open_command(&mut app, &opened).unwrap();
        assert!(crate::dialogs::form::inspect(&app).to_string().contains('7'));
        crate::dialogs::form::set(&mut app, "advance", &json!(9)).unwrap();
        crate::dialogs::form::confirm(&mut app).unwrap();
        let font = app.session.font().unwrap();
        assert_eq!(font.bitmap_strikes.first().unwrap().glyph(&glyph).unwrap().advance, 9);
    }

    /// The dialog opens on the strike's values and its OK sets them on that strike in one step.
    #[test]
    fn the_dialog_edits_the_strike_it_opened_on() {
        let mut app = FontCraftApp::with_edit_tab(fontcraft_engine::Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.session.execute("bitmap.setStrikes", &json!({"add": [{"ppem": 12}, {"ppem": 16}]})).unwrap();
        app.session.execute("bitmap.setStrikeProperties", &json!({"strike": {"ppem": 16}, "ascent": 13})).unwrap();
        let other = app.session.font().unwrap().bitmap_strikes.iter().find(|s| s.ppem == 12).unwrap().clone();
        let opened = json!({"dialog": "dialog.strikeProperties", "params": {"strike": {"ppem": 16}}});
        crate::dialogs::form::open_command(&mut app, &opened).unwrap();
        let inspected = crate::dialogs::form::inspect(&app);
        assert!(inspected.to_string().contains("13"), "{inspected}");
        crate::dialogs::form::set(&mut app, "descent", &json!(4)).unwrap();
        crate::dialogs::form::set(&mut app, "properties", &json!([{"name": "FOUNDRY", "value": "Synthetic"}])).unwrap();
        crate::dialogs::form::confirm(&mut app).unwrap();
        let font = app.session.font().unwrap();
        let strike = font.bitmap_strikes.iter().find(|s| s.ppem == 16).unwrap();
        assert_eq!((strike.ascent, strike.descent, strike.properties.len()), (13, 4, 1));
        assert_eq!(*font.bitmap_strikes.iter().find(|s| s.ppem == 12).unwrap(), other, "the other strike is left alone");
    }
}
