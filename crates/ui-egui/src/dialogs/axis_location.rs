//! The Axis Location dialog (M9.23): the external coordinates of a master or instance.
//!
//! Rows: `dialog.axisLocation`.
//!
//! The dialog edits the value of a master's or an instance's Axis Location parameter through
//! `font.customParameters.set`, with a live preview: one number field per axis, keyed by the axis
//! name, holding the coordinate the master or instance stands for in a variable font. It opens
//! from a parameter that exists (the parameter list adds it first); the fields start from its
//! entries, and an axis the value leaves out starts at the target's internal coordinate. OK writes
//! an entry for every axis. The entries follow the order of the axis names, the order the field
//! values are kept in; every reader finds an entry by its axis name.
//!
//! `dialog.open {dialog: "dialog.axisLocation", params: {scope?: "master" (default) | "instance",
//! targets?: id|name|index (needed when there is more than one), <axis name>?: number}}`.
//!
//! [`location_fields`], [`location_value`] and [`entries`] are shared with the Virtual Master
//! dialog, which edits a value of the same shape.

use fontcraft_engine::Session;
use fontcraft_engine::fontinfo::target;
use fontcraft_engine::interp::MAX_COORDINATE;
use fontcraft_engine::model::axes::{AXIS_LOCATION, design_value};
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::{Font, Params};
use serde_json::{Map, Value, json};

use super::form::{Field, MAX_FIELDS};
use super::{Dialog, Fields, Shaped};

pub const DIALOG: Dialog =
    Dialog::new("dialog.axisLocation", "Axis Location", "font.customParameters.set", Fields::Shaped(Shaped { fields, given, params, link: no_link }));

/// Parameters of `font.customParameters.set` that the dialog passes through, so no axis may
/// have one of these names.
const RESERVED: [&str; 6] = ["scope", "targets", "name", "index", "value", "enabled"];

/// One number field per axis of the active font, keyed and labelled by the axis name.
pub(super) fn location_fields(session: &Session, doc: &str) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    font.axes
        .iter()
        .take(MAX_FIELDS)
        .map(|a| Field::number(&a.name, &a.name, -MAX_COORDINATE, MAX_COORDINATE, 0.0, &format!("{doc} ({})", a.tag)))
        .collect()
}

fn fields(session: &Session) -> Vec<Field> {
    location_fields(session, "The external coordinate on this axis")
}

/// Refuses a font the dialog cannot show: no axes, more axes than fields, or an axis named like
/// one of the command's own parameters.
pub(super) fn check_axes(font: &Font, title: &str) -> Result<(), String> {
    if font.axes.is_empty() {
        return Err(format!("{title}: the font has no axes"));
    }
    if font.axes.len() > MAX_FIELDS {
        return Err(format!("{title}: the font has {} axes; the dialog shows at most {MAX_FIELDS}", font.axes.len()));
    }
    if let Some(a) = font.axes.iter().find(|a| RESERVED.contains(&a.name.as_str())) {
        return Err(format!("{title}: an axis is named `{}`, which the dialog uses for its own parameters; rename the axis first", a.name));
    }
    Ok(())
}

/// The `(axis name, coordinate)` entries of a stored Axis Location or Virtual Master value; a
/// value that is not a list of `{Axis, Location}` gives none, so the dialog starts over.
pub(super) fn entries(value: &Plist) -> Vec<(String, f64)> {
    let Some(list) = value.as_array() else { return Vec::new() };
    list.items
        .iter()
        .filter_map(|item| {
            let d = item.as_dict()?;
            let axis = d.get("Axis")?.as_str()?;
            let location = match d.get("Location")? {
                Plist::Number(n) => n.value().as_f64(),
                Plist::String(s) => s.as_str().trim().parse::<f64>().ok()?,
                _ => return None,
            };
            (location.is_finite() && location.abs() <= MAX_COORDINATE).then(|| (axis.to_string(), location))
        })
        .collect()
}

/// The parameter value the fields give: one `{Axis, Location}` entry per field.
pub(super) fn location_value(values: &Map<String, Value>) -> Value {
    Value::Array(values.iter().map(|(axis, location)| json!({"Axis": axis, "Location": location})).collect())
}

/// The fields' first values: those given, else the stored entry of the axis, else `fallback(i)`.
pub(super) fn first_values(font: &Font, given: &mut Map<String, Value>, stored: &[(String, f64)], fallback: impl Fn(usize) -> f64) {
    for (i, axis) in font.axes.iter().enumerate() {
        if given.contains_key(&axis.name) {
            continue;
        }
        let v = stored.iter().find(|(name, _)| *name == axis.name).map_or_else(|| fallback(i), |(_, v)| *v);
        given.insert(axis.name.clone(), json!(v));
    }
}

/// The rows called `name` in `params`, switched-off ones included, as `font.customParameters.set`
/// counts them.
pub(super) fn rows<'a>(params: &'a Params, name: &str) -> Vec<&'a Plist> {
    params.0.iter().filter(|p| p.name == name).map(|p| &p.value).collect()
}

fn given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    const TITLE: &str = "Axis Location";
    let font = session.font().map_err(|e| e.to_string())?;
    check_axes(font, TITLE)?;
    let scope = match given.get("scope") {
        None => "master",
        Some(Value::String(s)) if s == "master" || s == "instance" => s.as_str(),
        Some(other) => return Err(format!("{TITLE}: `scope` is \"master\" or \"instance\", not {other}")),
    };
    let count = if scope == "master" { font.masters.len() } else { font.instances.len() };
    let index = match given.get("targets") {
        Some(raw) => {
            let r = target::parse(raw).map_err(|e| format!("{TITLE}: `targets` names one {scope}: {e}"))?;
            if scope == "master" { target::master(font, &r) } else { target::instance(font, &r) }.map_err(|e| format!("{TITLE}: {e}"))?
        }
        None if count == 1 => 0,
        None if count == 0 => return Err(format!("{TITLE}: the font has no {scope}")),
        None => return Err(format!("{TITLE}: the font has {count} {scope}s: name one with `targets`")),
    };
    let (params, location, what) = if scope == "master" {
        let m = font.masters.get(index).ok_or_else(|| format!("{TITLE}: there is no master #{index}"))?;
        (&m.params, &m.location, format!("the master `{}`", m.name))
    } else {
        let i = font.instances.get(index).ok_or_else(|| format!("{TITLE}: there is no instance #{index}"))?;
        (&i.params, &i.location, format!("the instance `{}`", target::instance_name(i)))
    };
    let stored = rows(params, AXIS_LOCATION);
    let Some(first) = stored.first() else {
        return Err(format!("{TITLE}: {what} has no Axis Location parameter: add it first"));
    };
    let mut out = given.clone();
    out.insert("scope".into(), json!(scope));
    out.insert("targets".into(), given.get("targets").cloned().unwrap_or_else(|| json!(index)));
    if stored.len() > 1 {
        // The parameter takes one value; the first row is the one that counts.
        out.insert("index".into(), json!(0));
    }
    first_values(font, &mut out, &entries(first), |i| design_value(location, i));
    Ok(out)
}

fn params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!(AXIS_LOCATION));
    m.insert("value".into(), location_value(values));
    m
}

pub(super) fn no_link(_: &mut Map<String, Value>, _: &str) {}
