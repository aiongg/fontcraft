//! The Virtual Master dialog (M9.23): a location on every axis.
//!
//! Rows: `dialog.virtualMaster`.
//!
//! The dialog edits the value of one of the font's Virtual Master parameters through
//! `font.customParameters.set`, with a live preview: one number field per axis, keyed by the axis
//! name, holding the internal coordinate of the virtual master, entered as for a real master. It
//! opens from a parameter that exists; the parameter repeats, so `index` picks the row when the
//! font has several. The fields start from the row's entries, and an axis the value leaves out
//! starts at the first master's coordinate. OK writes an entry for every axis, in the order of
//! the axis names (see [`super::axis_location`]).
//!
//! `dialog.open {dialog: "dialog.virtualMaster", params: {index?: integer (which Virtual Master
//! parameter, from 0; needed when there are several), <axis name>?: number}}`.

use fontcraft_engine::Session;
use fontcraft_engine::model::axes::{VIRTUAL_MASTER, design_value};
use serde_json::{Map, Value, json};

use super::axis_location::{check_axes, entries, first_values, location_fields, location_value, no_link, rows};
use super::form::Field;
use super::{Dialog, Fields, Shaped};

pub const DIALOG: Dialog = Dialog::new(
    "dialog.virtualMaster",
    "Virtual Master",
    "font.customParameters.set",
    Fields::Shaped(Shaped { fields, given, params, link: no_link }),
);

fn fields(session: &Session) -> Vec<Field> {
    location_fields(session, "The virtual master's coordinate on this axis")
}

fn given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    const TITLE: &str = "Virtual Master";
    let font = session.font().map_err(|e| e.to_string())?;
    check_axes(font, TITLE)?;
    let stored = rows(&font.info.params, VIRTUAL_MASTER);
    let index = match given.get("index") {
        None if stored.len() <= 1 => 0,
        None => return Err(format!("{TITLE}: the font has {} Virtual Master parameters: pick one with `index`", stored.len())),
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| format!("{TITLE}: `index` is a whole number from 0, not {v}"))?,
    };
    let Some(value) = stored.get(index) else {
        return Err(match stored.len() {
            0 => format!("{TITLE}: the font has no Virtual Master parameter: add it first"),
            n => format!("{TITLE}: the font has {n} Virtual Master parameters; there is none with index {index}"),
        });
    };
    let mut out = given.clone();
    out.insert("scope".into(), json!("font"));
    if stored.len() > 1 {
        out.insert("index".into(), json!(index));
    } else {
        out.remove("index");
    }
    let origin = font.masters.first().map(|m| m.location.clone()).unwrap_or_default();
    first_values(font, &mut out, &entries(value), |i| design_value(&origin, i));
    Ok(out)
}

fn params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!(VIRTUAL_MASTER));
    m.insert("value".into(), location_value(values));
    m
}
