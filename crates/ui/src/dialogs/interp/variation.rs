//! The interpolation dialogs of the Path, Glyph and Font menus.
//!
//! | Dialog | Command | Runs |
//! |---|---|---|
//! | `dialog.interpolateWithBackground` | `path.interpolateWithBackground`: the blend percentage | live preview |
//! | `dialog.adjustByVariation` | `glyph.replaceWithInstance`: the master, one coordinate per axis, relative or absolute | live preview |
//! | `dialog.setMasterLocations` | `font.setMasterLocations`: the axis, then a table (`40:100, 160:900`) or a scale and offset | on OK |
//! | `dialog.addVariation` | `font.addVariation`: the axis tag and name, the location, the master to copy, the new name | on OK |
//! | `dialog.mergeFontsToMasters` | `font.mergeFontsToMasters`: the open fonts to merge, the family name | on OK |
//!
//! Firing one of these commands from its menu without parameters opens its dialog.

use fontcraft_engine::Session;
use fontcraft_engine::interp::MAX_COORDINATE;
use fontcraft_engine::model::{Font, MAX_NAME_LEN};
use serde_json::{Map, Value, json};

use super::super::fields::{Field, Kind, MAX_CHOICES, MAX_FIELDS};
use super::super::spec::{DialogSpec, Fields, Shaped};
use super::no_link;

/// The engine's cap on the blend amount, in percent.
const MAX_AMOUNT: f64 = 100_000.0;
/// Most characters of a Set Master Locations table.
const MAX_TABLE: usize = 4096;
/// The registered axes Add Variation offers first, in order.
const REGISTERED: [&str; 5] = ["wght", "wdth", "opsz", "slnt", "ital"];

pub const INTERPOLATE_WITH_BACKGROUND: DialogSpec =
    DialogSpec::new("dialog.interpolateWithBackground", "Interpolate with Background", "path.interpolateWithBackground", Fields::Fixed(blend_fields));

pub const ADJUST_BY_VARIATION: DialogSpec = DialogSpec::new(
    "dialog.adjustByVariation",
    "Adjust by Variation",
    "glyph.replaceWithInstance",
    Fields::Shaped(Shaped { fields: adjust_fields, given: adjust_given, params: adjust_params, link: no_link }),
);

pub const SET_MASTER_LOCATIONS: DialogSpec = DialogSpec::new(
    "dialog.setMasterLocations",
    "Set Master Locations",
    "font.setMasterLocations",
    Fields::Shaped(Shaped { fields: locations_fields, given: locations_given, params: locations_params, link: no_link }),
)
.run_on_ok();

pub const ADD_VARIATION: DialogSpec = DialogSpec::new(
    "dialog.addVariation",
    "Add Variation",
    "font.addVariation",
    Fields::Shaped(Shaped { fields: variation_fields, given: variation_given, params: variation_params, link: no_link }),
)
.run_on_ok()
.buttons("Add", "");

pub const MERGE_FONTS: DialogSpec = DialogSpec::new(
    "dialog.mergeFontsToMasters",
    "Merge Fonts to Masters",
    "font.mergeFontsToMasters",
    Fields::Shaped(Shaped { fields: merge_fields, given: merge_given, params: merge_params, link: no_link }),
)
.run_on_ok()
.buttons("Merge", "");

fn blend_fields(_: &Session) -> Vec<Field> {
    vec![Field::number(
        "amount",
        "Percentage",
        -MAX_AMOUNT,
        MAX_AMOUNT,
        50.0,
        "0 keeps the foreground, 100 gives the background; below 0 or above 100 extrapolates",
    )]
}

/// A pop-up menu of `(word, label)` choices.
fn choice(key: &str, label: &str, list: Vec<(String, String)>, doc: &str) -> Field {
    let default = list.first().map_or(Value::Null, |(w, _)| json!(w));
    let (choices, labels) = list.into_iter().take(MAX_CHOICES).unzip();
    Field { key: key.into(), label: label.into(), kind: Kind::Choice { choices, labels }, default, doc: doc.into(), group: None, same_as: None }
}

/// The font's masters as `(id, name)`.
fn masters(font: &Font) -> Vec<(String, String)> {
    font.masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect()
}

/// Refuses a font without masters or axes, or with more axes than the dialog has fields for.
fn check(session: &Session, title: &str, spare: usize) -> Result<(), String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if font.masters.is_empty() {
        return Err(format!("{title}: the font has no master"));
    }
    if font.axes.is_empty() {
        return Err(format!("{title}: the font has no axes"));
    }
    if font.axes.len() + spare > MAX_FIELDS {
        return Err(format!("{title}: the font has {} axes; the dialog shows at most {}", font.axes.len(), MAX_FIELDS - spare));
    }
    Ok(())
}

// ── Adjust by Variation ─────────────────────────────────────────────────────────────────────────

/// The keys of the fields that are not axes.
const ADJUST_KEYS: [&str; 2] = ["master", "relative"];

fn adjust_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    let mut out = vec![
        choice("master", "Master", masters(font), "The master whose layers take the interpolated outlines"),
        Field::bool("relative", "Relative", true, "On: each coordinate is an offset from the master's own; off: the location itself"),
    ];
    out.extend(
        font.axes
            .iter()
            .take(MAX_FIELDS - ADJUST_KEYS.len())
            .map(|a| Field::number(&a.tag, &a.name, -MAX_COORDINATE, MAX_COORDINATE, 0.0, &format!("The coordinate on this axis ({})", a.tag))),
    );
    out
}

fn adjust_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    check(session, "Adjust by Variation", ADJUST_KEYS.len())?;
    let mut out = given.clone();
    // The command's `location` comes in as one field per axis.
    if let Some(Value::Object(location)) = out.remove("location") {
        out.extend(location);
    }
    Ok(out)
}

fn adjust_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    let mut location = Map::new();
    for (k, v) in values {
        if ADJUST_KEYS.contains(&k.as_str()) {
            m.insert(k.clone(), v.clone());
        } else {
            location.insert(k.clone(), v.clone());
        }
    }
    m.insert("location".into(), Value::Object(location));
    m
}

// ── Set Master Locations ────────────────────────────────────────────────────────────────────────

fn locations_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    let axes = font.axes.iter().map(|a| (a.tag.clone(), format!("{} ({})", a.name, a.tag))).collect();
    vec![
        choice("axis", "Axis", axes, "The axis whose coordinates change"),
        Field::text("table", "Table", MAX_TABLE, "Old and new coordinates as `old:new` pairs (40:100, 160:900); empty uses the scale and offset"),
        Field::number("scale", "Scale", -MAX_COORDINATE, MAX_COORDINATE, 1.0, "Each coordinate is multiplied by this, then the offset is added"),
        Field::number("offset", "Offset", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "Added to each coordinate after the scale"),
    ]
}

fn locations_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    check(session, "Set Master Locations", 3)?;
    let mut out = given.clone();
    // A `map` given in the command's shape shows as the table.
    if let Some(map) = out.remove("map").and_then(|m| m.as_array().cloned()) {
        let pairs: Vec<String> =
            map.iter().filter_map(|e| Some(format!("{}:{}", fmt(e.get("from")?.as_f64()?), fmt(e.get("to")?.as_f64()?)))).collect();
        out.insert("table".into(), json!(pairs.join(", ")));
    }
    Ok(out)
}

fn fmt(n: f64) -> String {
    super::super::fields::format_number(n)
}

/// The table's `{from, to}` entries; `None` when an entry does not read as two numbers.
pub fn table_map(text: &str) -> Option<Vec<Value>> {
    text.split([',', '\n', ';'])
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|e| {
            let (from, to) = e.split_once(':')?;
            let (from, to) = (from.trim().parse::<f64>().ok()?, to.trim().parse::<f64>().ok()?);
            (from.is_finite() && to.is_finite()).then(|| json!({"from": from, "to": to}))
        })
        .collect()
}

fn locations_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("axis".into(), values.get("axis").cloned().unwrap_or(Value::Null));
    let table = values.get("table").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if table.is_empty() {
        for key in ["scale", "offset"] {
            m.insert(key.into(), values.get(key).cloned().unwrap_or(Value::Null));
        }
    } else {
        // A table that does not read goes to the command as typed, which refuses it with the reason.
        m.insert("map".into(), table_map(table).map_or_else(|| json!(table), Value::Array));
    }
    m
}

// ── Add Variation ───────────────────────────────────────────────────────────────────────────────

fn variation_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    let free = REGISTERED.iter().find(|t| !font.axes.iter().any(|a| a.tag == **t)).copied().unwrap_or("wdth");
    let mut tag = Field::text("tag", "Axis Tag", 4, "The axis of the new master: an axis of the font, or a new one added at 0 for the other masters");
    tag.default = json!(free);
    vec![
        tag,
        Field::text("axisName", "Axis Name", MAX_NAME_LEN, "The new axis's name; empty takes the registered name of the tag"),
        Field::number("location", "Location", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "The new master's coordinate on the axis"),
        choice("from", "Copy of", masters(font), "The master the new one copies, with its layers and kerning"),
        Field::text("name", "Master Name", MAX_NAME_LEN, "The new master's name; empty names it after the original, the axis and the location"),
    ]
}

fn variation_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if font.masters.is_empty() {
        return Err("Add Variation: the font has no master".into());
    }
    let mut out = given.clone();
    // An axis given in the command's shape fills the tag and name fields.
    match out.remove("axis") {
        Some(Value::String(tag)) => {
            out.insert("tag".into(), json!(tag));
        }
        Some(Value::Object(axis)) => {
            if let Some(tag) = axis.get("tag") {
                out.insert("tag".into(), tag.clone());
            }
            if let Some(name) = axis.get("name") {
                out.insert("axisName".into(), name.clone());
            }
        }
        _ => {}
    }
    Ok(out)
}

fn variation_params(values: &Map<String, Value>) -> Map<String, Value> {
    let text = |k: &str| values.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let (tag, axis_name, name) = (text("tag"), text("axisName"), text("name"));
    let mut m = Map::new();
    m.insert("axis".into(), if axis_name.is_empty() { json!(tag) } else { json!({"tag": tag, "name": axis_name}) });
    m.insert("location".into(), values.get("location").cloned().unwrap_or(Value::Null));
    if let Some(from) = values.get("from").filter(|v| !v.is_null()) {
        m.insert("from".into(), from.clone());
    }
    if !name.is_empty() {
        m.insert("name".into(), json!(name));
    }
    m
}

// ── Merge Fonts to Masters ──────────────────────────────────────────────────────────────────────

/// The open documents as `(reference, label)`: the path a document was opened from, else its title.
fn documents(session: &Session) -> Vec<(String, String)> {
    session
        .documents()
        .iter()
        .take(MAX_CHOICES)
        .map(|d| (d.path.as_ref().map_or_else(|| d.title.clone(), |p| p.display().to_string()), d.title.clone()))
        .collect()
}

fn merge_fields(session: &Session) -> Vec<Field> {
    let (choices, labels): (Vec<String>, Vec<String>) = documents(session).into_iter().unzip();
    vec![
        Field {
            key: "fonts".into(),
            label: "Fonts".into(),
            default: json!(choices),
            kind: Kind::Multi { choices, labels },
            doc: "The static fonts to merge, one master each; each needs one master and the same units per em".into(),
            group: None,
            same_as: None,
        },
        Field::text("family", "Family Name", MAX_NAME_LEN, "The merged font's family name; empty keeps the first font's"),
    ]
}

fn merge_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if session.documents().len() < 2 {
        return Err("Merge Fonts to Masters: open at least two fonts".into());
    }
    Ok(given.clone())
}

fn merge_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("fonts".into(), values.get("fonts").cloned().unwrap_or_else(|| json!([])));
    if let Some(family) = values.get("family").and_then(Value::as_str).map(str::trim).filter(|f| !f.is_empty()) {
        m.insert("family".into(), json!(family));
    }
    m
}
