//! Reading and writing the interpolation parameters in the model's parameter lists (M9.2).
//!
//! Rows: parameters `Axis Location`, `Axis Mappings`, `Virtual Master`, `Disable Masters`,
//! `Variable Font Origin`, `Variation Font Origin`, `Ignore out of bounds instances`,
//! `Enforce Compatibility Check`.
//!
//! Contract: M9's commands write these parameters through the setters here, so the values they
//! write have the shape the reference's format gives them; M7's generic custom-parameter
//! commands edit the same values. Readers refuse a value they cannot read with
//! [`InterpError::BadParameter`]; the setters return a new parameter list and leave the input
//! unchanged. An empty value removes the parameter.

use fontcraft_model::axes::{number_value, whole};
use fontcraft_model::plist::{Array, Dict, Str, Value};
use fontcraft_model::{Font, Instance, Param, Params};

use crate::{InterpError, MAX_AXES, MAX_MAPPING_POINTS, MAX_SOURCES, MAX_VIRTUAL_MASTERS};

pub use fontcraft_model::axes::{AXIS_LOCATION, AXIS_MAPPINGS, ORIGIN, VIRTUAL_MASTER};

/// The variable font setting parameter that lists the masters left out of that font, by name.
pub const DISABLE_MASTERS: &str = "Disable Masters";
/// The older name of [`ORIGIN`], which names the master by name.
pub const ORIGIN_BY_NAME: &str = "Variation Font Origin";
/// The font parameter that leaves instances outside the axis ranges out of the export.
pub const IGNORE_OUT_OF_BOUNDS: &str = "Ignore out of bounds instances";
/// The font parameter that widens the compatibility check to every master (on) or hides the
/// indicators (off).
pub const ENFORCE_COMPATIBILITY_CHECK: &str = "Enforce Compatibility Check";

/// One entry of an Axis Location parameter: an axis, by name, and the external coordinate there.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisLocationEntry {
    pub axis: String,
    pub location: f64,
}

/// A Virtual Master parameter: one coordinate per axis, by axis name.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMasterEntry {
    pub coordinates: Vec<(String, f64)>,
}

fn bad(name: &str, reason: impl Into<String>) -> InterpError {
    InterpError::BadParameter { name: name.to_string(), reason: reason.into() }
}

/// A finite number from a parameter value that may be written as text (`"600"`).
fn number_of(v: &Value) -> Option<f64> {
    let n = match v {
        Value::Number(n) => n.value().as_f64(),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && n.abs() <= crate::MAX_COORDINATE).then_some(n)
}

/// A number as a parameter value, refusing one that is not usable.
fn number(v: f64, what: impl FnOnce() -> String) -> Result<Value, InterpError> {
    if !(v.is_finite() && v.abs() <= crate::MAX_COORDINATE) {
        return Err(InterpError::BadNumber { what: what() });
    }
    number_value(v).ok_or_else(|| InterpError::BadNumber { what: format!("{v}") })
}

/// The text of a number as a dictionary key: a whole number without a fraction.
fn key_text(v: f64) -> String {
    whole(v).map_or_else(|| format!("{v}"), |i| i.to_string())
}

/// A list of `{Axis, Location}` dictionaries, the shape of Axis Location and Virtual Master.
fn read_locations(name: &str, value: &Value) -> Result<Vec<(String, f64)>, InterpError> {
    let list = value.as_array().ok_or_else(|| bad(name, "it is not a list"))?;
    if list.items.len() > MAX_AXES {
        return Err(InterpError::LimitExceeded { what: "axis coordinates in one parameter", limit: MAX_AXES });
    }
    list.items
        .iter()
        .map(|item| {
            let d = item.as_dict().ok_or_else(|| bad(name, "an entry is not an axis and a location"))?;
            let axis = d.get("Axis").and_then(Value::as_str).ok_or_else(|| bad(name, "an entry names no axis"))?;
            let location =
                d.get("Location").and_then(number_of).ok_or_else(|| bad(name, format!("the location of axis `{axis}` is not a usable number")))?;
            Ok((axis.to_string(), location))
        })
        .collect()
}

/// The value of an Axis Location or Virtual Master parameter.
fn write_locations<'a>(name: &str, entries: impl Iterator<Item = (&'a str, f64)>) -> Result<Value, InterpError> {
    let items = entries
        .map(|(axis, location)| {
            let mut d = Dict::new();
            d.insert("Axis", Value::String(Str::new(axis)));
            d.insert("Location", number(location, || format!("the {axis} location of the {name} parameter"))?);
            Ok(Value::Dict(d))
        })
        .collect::<Result<Vec<_>, InterpError>>()?;
    if items.len() > MAX_AXES {
        return Err(InterpError::LimitExceeded { what: "axis coordinates in one parameter", limit: MAX_AXES });
    }
    Ok(Value::Array(Array { items, ..Default::default() }))
}

/// The Axis Location of a master or instance; `None` when the parameter is absent.
pub fn axis_location(params: &Params) -> Result<Option<Vec<AxisLocationEntry>>, InterpError> {
    let Some(value) = params.get(AXIS_LOCATION) else { return Ok(None) };
    let entries = read_locations(AXIS_LOCATION, value)?;
    Ok(Some(entries.into_iter().map(|(axis, location)| AxisLocationEntry { axis, location }).collect()))
}

/// `params` with its Axis Location set to `entries` (removed when empty).
pub fn set_axis_location(params: &Params, entries: &[AxisLocationEntry]) -> Result<Params, InterpError> {
    let mut out = params.clone();
    if entries.is_empty() {
        out.remove(AXIS_LOCATION);
        return Ok(out);
    }
    let value = write_locations(AXIS_LOCATION, entries.iter().map(|e| (e.axis.as_str(), e.location)))?;
    out.set(AXIS_LOCATION, value);
    Ok(out)
}

/// One axis's mapping: the axis tag and its `(external, internal)` points.
pub type AxisMapping = (String, Vec<(f64, f64)>);

/// The entries of an Axis Mappings value, in order; later points with the same external value
/// replace earlier ones, and each axis's points are sorted by external value.
fn read_mappings(value: &Value) -> Result<Vec<AxisMapping>, InterpError> {
    let d = value.as_dict().ok_or_else(|| bad(AXIS_MAPPINGS, "it is not a dictionary of axes"))?;
    if d.entries.len() > MAX_AXES {
        return Err(InterpError::LimitExceeded { what: "axes in the axis mappings", limit: MAX_AXES });
    }
    let mut out: Vec<AxisMapping> = Vec::with_capacity(d.entries.len());
    for (tag, points) in &d.entries {
        let tag = tag.as_str();
        let inner = points.as_dict().ok_or_else(|| bad(AXIS_MAPPINGS, format!("the entry of axis `{tag}` is not a dictionary")))?;
        if inner.entries.len() > MAX_MAPPING_POINTS {
            return Err(InterpError::LimitExceeded { what: "points in an axis mapping", limit: MAX_MAPPING_POINTS });
        }
        let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(inner.entries.len());
        for (k, v) in &inner.entries {
            let unusable = || bad(AXIS_MAPPINGS, format!("a point of axis `{tag}` is not a pair of usable numbers"));
            let external =
                k.as_str().trim().parse::<f64>().ok().filter(|x| x.is_finite() && x.abs() <= crate::MAX_COORDINATE).ok_or_else(unusable)?;
            let internal = number_of(v).ok_or_else(unusable)?;
            match pairs.iter_mut().find(|p| p.0 == external) {
                Some(p) => p.1 = internal,
                None => pairs.push((external, internal)),
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.push((tag.to_string(), pairs));
    }
    Ok(out)
}

/// The font's Axis Mappings, as (axis tag, `(external, internal)` points sorted by external
/// value), in the parameter's order.
pub fn axis_mappings(font: &Font) -> Result<Vec<AxisMapping>, InterpError> {
    match font.info.params.get(AXIS_MAPPINGS) {
        Some(value) => read_mappings(value),
        None => Ok(Vec::new()),
    }
}

/// `params` (a font's) with axis `tag`'s mapping set to `points`; other axes are kept. Empty
/// `points` remove the axis's entry, and the parameter when no entry is left. Refuses points
/// that [`crate::mapping::check_points`] refuses.
pub fn set_axis_mappings(params: &Params, tag: &str, points: &[(f64, f64)]) -> Result<Params, InterpError> {
    crate::mapping::check_points(tag, points)?;
    let mut d = match params.get(AXIS_MAPPINGS) {
        Some(value) => {
            // Read first, so a value that cannot be read is refused rather than overwritten.
            read_mappings(value)?;
            value.as_dict().cloned().unwrap_or_default()
        }
        None => Dict::new(),
    };
    let at = d.entries.iter().position(|(k, _)| k.as_str() == tag);
    if points.is_empty() {
        if let Some(i) = at {
            d.remove_at(i);
        }
    } else {
        let mut inner = Dict::new();
        for (external, internal) in points {
            inner.insert(key_text(*external), number(*internal, || format!("the mapping of axis `{tag}`"))?);
        }
        match at.and_then(|i| d.entries.get_mut(i)) {
            Some(slot) => slot.1 = Value::Dict(inner),
            None => {
                if d.entries.len() >= MAX_AXES {
                    return Err(InterpError::LimitExceeded { what: "axes in the axis mappings", limit: MAX_AXES });
                }
                d.insert(tag, Value::Dict(inner));
            }
        }
    }
    let mut out = params.clone();
    if d.entries.is_empty() {
        out.remove(AXIS_MAPPINGS);
    } else {
        out.set(AXIS_MAPPINGS, Value::Dict(d));
    }
    Ok(out)
}

/// The font's Virtual Master parameters, in order.
pub fn virtual_masters(font: &Font) -> Result<Vec<VirtualMasterEntry>, InterpError> {
    let mut out = Vec::new();
    for value in font.info.params.all(VIRTUAL_MASTER) {
        if out.len() >= MAX_VIRTUAL_MASTERS {
            return Err(InterpError::LimitExceeded { what: "virtual masters", limit: MAX_VIRTUAL_MASTERS });
        }
        out.push(VirtualMasterEntry { coordinates: read_locations(VIRTUAL_MASTER, value)? });
    }
    Ok(out)
}

/// `params` (a font's) with the Virtual Master parameters replaced by `entries`. The new ones
/// take the place of the first one replaced; switched-off ones are kept.
pub fn set_virtual_masters(params: &Params, entries: &[VirtualMasterEntry]) -> Result<Params, InterpError> {
    if entries.len() > MAX_VIRTUAL_MASTERS {
        return Err(InterpError::LimitExceeded { what: "virtual masters", limit: MAX_VIRTUAL_MASTERS });
    }
    let new = entries
        .iter()
        .map(|e| Ok(Param::new(VIRTUAL_MASTER, write_locations(VIRTUAL_MASTER, e.coordinates.iter().map(|(a, v)| (a.as_str(), *v)))?)))
        .collect::<Result<Vec<_>, InterpError>>()?;
    let replaced = |p: &Param| p.name == VIRTUAL_MASTER && !p.disabled;
    let mut out = params.clone();
    // Only kept parameters come before the first replaced one, so its index stays valid.
    let at = out.0.iter().position(replaced).unwrap_or(out.0.len());
    out.0.retain(|p| !replaced(p));
    let at = at.min(out.0.len());
    out.0.splice(at..at, new);
    Ok(out)
}

/// The names of the masters a variable font setting disables. The value is a list of names, or
/// one text of names separated by commas.
pub fn disabled_masters(setting: &Instance) -> Vec<String> {
    let Some(value) = setting.params.get(DISABLE_MASTERS) else { return Vec::new() };
    let names: Vec<&str> = match value {
        Value::Array(a) => a.items.iter().filter_map(Value::as_str).collect(),
        Value::String(s) => s.as_str().split(',').collect(),
        _ => Vec::new(),
    };
    names.into_iter().map(str::trim).filter(|n| !n.is_empty()).take(MAX_SOURCES).map(str::to_string).collect()
}

/// `params` (a variable font setting's) with Disable Masters set to `names` (removed when empty).
pub fn set_disabled_masters(params: &Params, names: &[String]) -> Params {
    let mut out = params.clone();
    let items: Vec<Value> = names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()).take(MAX_SOURCES).map(|n| Value::String(Str::new(n))).collect();
    if items.is_empty() {
        out.remove(DISABLE_MASTERS);
    } else {
        out.set(DISABLE_MASTERS, Value::Array(Array { items, ..Default::default() }));
    }
    out
}

/// The Enforce Compatibility Check setting: `Some(true)` widens the check to every master,
/// `Some(false)` hides the indicators, `None` is the default scope.
pub fn enforce_compatibility(font: &Font) -> Option<bool> {
    // M9.3
    let _ = font;
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designspace::tests::{edit_params, font};
    use std::sync::Arc;

    #[test]
    fn interpolation_parameters_round_trip() {
        let entries = [AxisLocationEntry { axis: "Weight".into(), location: 400.0 }, AxisLocationEntry { axis: "Width".into(), location: 87.5 }];
        let p = set_axis_location(&Params::default(), &entries).unwrap();
        assert_eq!(axis_location(&p).unwrap().unwrap(), entries);
        // Whole numbers are written as integers, as the format does.
        let first =
            p.get(AXIS_LOCATION).and_then(Value::as_array).and_then(|a| a.items.first()).and_then(Value::as_dict).and_then(|d| d.get("Location"));
        assert_eq!(first.and_then(Value::as_num), Some(fontcraft_model::plist::Num::Int(400)));
        assert!(set_axis_location(&p, &[]).unwrap().get(AXIS_LOCATION).is_none());
        assert!(set_axis_location(&p, &[AxisLocationEntry { axis: "Weight".into(), location: f64::NAN }]).is_err());

        let mut f = font(&[("Weight", "wght"), ("Width", "wdth")], &[("Regular", &[400.0, 100.0])]);
        edit_params(&mut f, |p| set_axis_mappings(p, "wght", &[(100.0, 40.0), (900.0, 160.5)]).unwrap());
        edit_params(&mut f, |p| set_axis_mappings(p, "wdth", &[(75.0, 60.0), (100.0, 100.0)]).unwrap());
        edit_params(&mut f, |p| set_axis_mappings(p, "wght", &[(100.0, 40.0), (400.0, 90.0), (900.0, 160.5)]).unwrap());
        let read = axis_mappings(&f).unwrap();
        assert_eq!(
            read,
            [("wght".to_string(), vec![(100.0, 40.0), (400.0, 90.0), (900.0, 160.5)]), ("wdth".to_string(), vec![(75.0, 60.0), (100.0, 100.0)])]
        );
        // The model reads the same map.
        assert_eq!(fontcraft_model::axes::mapping_parameter(&f, "wght").unwrap(), [(100.0, 40.0), (400.0, 90.0), (900.0, 160.5)]);
        edit_params(&mut f, |p| set_axis_mappings(p, "wght", &[]).unwrap());
        assert_eq!(axis_mappings(&f).unwrap().len(), 1);

        let vm = |cap: f64| VirtualMasterEntry { coordinates: vec![("Cap Height".into(), cap)] };
        let mut off = Param::new(VIRTUAL_MASTER, Value::Array(Array::default()));
        off.disabled = true;
        let p = Params(vec![Param::new("glyphOrder", Value::Array(Array::default())), off]);
        let p = set_virtual_masters(&p, &[vm(600.0), vm(800.0)]).unwrap();
        let p = set_virtual_masters(&p, &[vm(500.0)]).unwrap();
        assert_eq!(
            p.0.iter().map(|q| (q.name.as_str(), q.disabled)).collect::<Vec<_>>(),
            [("glyphOrder", false), (VIRTUAL_MASTER, true), (VIRTUAL_MASTER, false)]
        );
        let mut g = font(&[], &[]);
        Arc::make_mut(&mut g.info).params = p;
        assert_eq!(virtual_masters(&g).unwrap(), [vm(500.0)]);
    }

    #[test]
    fn disable_masters_reads_a_list_or_a_text() {
        let setting = |p: Params| Instance { kind: Some("variable".into()), params: p, ..Default::default() };
        let names = vec!["Light".to_string(), "Bold Italic".to_string()];
        let p = set_disabled_masters(&Params::default(), &names);
        assert!(p.get(DISABLE_MASTERS).and_then(Value::as_array).is_some());
        assert_eq!(disabled_masters(&setting(p.clone())), names);
        let mut text = Params::default();
        text.set(DISABLE_MASTERS, Value::String(Str::new("Light, Bold Italic, ")));
        assert_eq!(disabled_masters(&setting(text)), names);
        assert!(set_disabled_masters(&p, &[]).get(DISABLE_MASTERS).is_none());
    }

    #[test]
    fn unreadable_parameters_are_refused() {
        let mut p = Params::default();
        p.set(AXIS_LOCATION, Value::String(Str::new("Weight")));
        assert!(matches!(axis_location(&p), Err(InterpError::BadParameter { .. })));
        let mut f = font(&[("Weight", "wght")], &[("Regular", &[400.0])]);
        let mut d = Dict::new();
        d.insert("wght", Value::String(Str::new("400")));
        Arc::make_mut(&mut f.info).params.set(AXIS_MAPPINGS, Value::Dict(d));
        assert!(matches!(axis_mappings(&f), Err(InterpError::BadParameter { .. })));
        assert!(set_axis_mappings(&f.info.params, "wght", &[(1.0, 1.0)]).is_err(), "an unreadable value is not overwritten");
    }
}
