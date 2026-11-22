//! Strike Properties (row `dialog.strikeProperties`): the dialog over `bitmap.setStrikeProperties` that the Bitmap
//! Glyph Editor's Strike Properties button opens.
//!
//! It edits one strike, named by the `strike` parameter (`{ppem, depth}`); opened without one it takes the strike the
//! Bitmap Glyph Editor shows, else the font's first. The fields open on that strike's name, size in points,
//! resolution, ascent, descent and file properties. The properties are one per line, a name and its value: a whole
//! number, or text (quotes around it are dropped). Nothing changes while the dialog is open; OK runs the command once,
//! one undo step, and a value the command refuses keeps the dialog open with the message.

use fontcraft_engine::Session;
use fontcraft_engine::model::strike::{BitmapStrike, MAX_STRIKE_COORDINATE, MAX_STRIKE_PROPERTIES, StrikeValue};
use serde_json::{Map, Value, json};

use super::fields::Field;
use super::spec::{DialogSpec, Fields, Shaped};
use crate::app::FontCraftApp;

/// Longest strike name the field takes, in characters.
const MAX_NAME: usize = 1_024;
/// Longest property list the field takes, in characters.
const MAX_PROPERTIES_TEXT: usize = 262_144;
/// Highest resolution the fields take, in dots per inch.
const MAX_RESOLUTION: i64 = 100_000;

pub const SPECS: &[DialogSpec] = &[DialogSpec::new(
    "dialog.strikeProperties",
    "Strike Properties",
    "bitmap.setStrikeProperties",
    Fields::Shaped(Shaped { fields, given, params, link }),
)
.run_on_ok()
.from_window(window)];

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
        Field::text_area(
            "properties",
            "Properties",
            MAX_PROPERTIES_TEXT,
            8,
            "the BDF properties, one per line: a name, a space and the value (a whole number, or text)",
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
    let wanted = p.get("strike").ok_or("the font has no bitmap strikes")?;
    let ppem = wanted.get("ppem").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or("`strike.ppem` must be a whole number")?;
    let depth = wanted.get("depth").and_then(Value::as_u64).and_then(|d| u8::try_from(d).ok());
    let strike = font
        .bitmap_strikes
        .iter()
        .filter(|s| s.ppem == ppem && depth.is_none_or(|d| s.depth == d))
        .min_by_key(|s| s.depth)
        .ok_or_else(|| format!("the font has no strike of {ppem} pixels"))?;
    let mut out = values(strike);
    out.insert("strike".into(), json!({"ppem": strike.ppem, "depth": strike.depth}));
    for (key, value) in p {
        if key != "strike" {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(out)
}

/// The fields' values for `strike`.
fn values(strike: &BitmapStrike) -> Map<String, Value> {
    let properties: Vec<String> = strike
        .properties
        .iter()
        .map(|p| match &p.value {
            StrikeValue::Integer(n) => format!("{} {n}", p.name),
            StrikeValue::Text(t) => format!("{} \"{t}\"", p.name),
        })
        .collect();
    let mut out = Map::new();
    out.insert("name".into(), json!(strike.name));
    out.insert("pointSize".into(), json!(strike.point_size));
    let [rx, ry] = strike.resolution;
    out.insert("resolutionX".into(), json!(rx));
    out.insert("resolutionY".into(), json!(ry));
    out.insert("ascent".into(), json!(strike.ascent));
    out.insert("descent".into(), json!(strike.descent));
    out.insert("properties".into(), json!(properties.join("\n")));
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
    let text = v.get("properties").and_then(Value::as_str).unwrap_or("");
    out.insert("properties".into(), Value::Array(parse_properties(text)));
    out
}

/// The property lines as `bitmap.setStrikeProperties` takes them: `{name, value}`, the value a number when the text
/// after the name is a whole number. Empty lines are skipped; lines past the strike's property limit are dropped.
pub fn parse_properties(text: &str) -> Vec<Value> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_STRIKE_PROPERTIES)
        .map(|line| {
            let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();
            let value = match rest.parse::<i64>() {
                Ok(n) => json!(n),
                Err(_) => {
                    let unquoted = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')).unwrap_or(rest);
                    json!(unquoted)
                }
            };
            json!({"name": name, "value": value})
        })
        .collect()
}

fn link(_: &mut Map<String, Value>, _: &str) {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use serde_json::json;

    use super::parse_properties;
    use crate::app::{APP, FontCraftApp};

    /// Property lines become typed values: whole numbers as numbers, the rest as text without its quotes.
    #[test]
    fn property_lines_become_typed_values() {
        let parsed = parse_properties("POINT_SIZE 120\n\nFOUNDRY \"Synthetic\"\nCOPYRIGHT made here\n");
        assert_eq!(
            parsed,
            vec![
                json!({"name": "POINT_SIZE", "value": 120}),
                json!({"name": "FOUNDRY", "value": "Synthetic"}),
                json!({"name": "COPYRIGHT", "value": "made here"}),
            ]
        );
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
        crate::dialogs::form::set(&mut app, "properties", &json!("FOUNDRY \"Synthetic\"")).unwrap();
        crate::dialogs::form::confirm(&mut app).unwrap();
        let font = app.session.font().unwrap();
        let strike = font.bitmap_strikes.iter().find(|s| s.ppem == 16).unwrap();
        assert_eq!((strike.ascent, strike.descent, strike.properties.len()), (13, 4, 1));
        assert_eq!(*font.bitmap_strikes.iter().find(|s| s.ppem == 12).unwrap(), other, "the other strike is left alone");
    }
}
