//! Parameter values: the checks every add and set runs, the conversion between the JSON that
//! commands take and the property-list values the model stores, and typed readers.
//!
//! A value is checked against its parameter's kind ([`crate::ParamKind`]) and, where the catalogue
//! names one, the parameter's own syntax (Name Table Entry, Remove Glyphs, Rename Glyphs, Reencode
//! Glyphs, Filter, versionString). A wrong value is refused and nothing changes. Values read from
//! files are not checked when the file opens; export reports the ones it cannot use
//! (`docs/plans/M7.md` § 3 › Value checks). Every value, known parameter or not, stays within
//! [`crate::MAX_VALUE_BYTES`], [`crate::MAX_VALUE_ITEMS`] and [`crate::MAX_VALUE_DEPTH`].
//!
//! Rows: `font.customParameters.add`, `font.customParameters.set`, `customParameters.unknownKept`
//! (M7.2).

use fontcraft_plist::{Array, Dict, Number, Str, Value};
use serde_json::Value as Json;

use crate::catalog::{ParamKind, ParamSpec};
use crate::error::ValueError;
use crate::target::Scope;

/// `value` as `spec` stores it (numbers in text read as numbers, a single string for a list of
/// one…), or why it is refused.
pub fn check(spec: &ParamSpec, value: &Value) -> Result<Value, ValueError> {
    // M7.2: the kind, the parameter syntaxes, the limits.
    let _ = spec;
    Ok(value.clone())
}

/// Refuses `spec` at a scope its entry does not allow.
pub fn check_scope(spec: &ParamSpec, scope: Scope) -> Result<(), ValueError> {
    if spec.scopes.allows(scope) { Ok(()) } else { Err(ValueError::WrongScope { name: spec.name.to_string(), scope: scope.name() }) }
}

/// The limits alone, for a parameter the catalogue does not know (kept as given).
pub fn check_limits(name: &str, value: &Value) -> Result<(), ValueError> {
    // M7.2: bytes, items, depth.
    let _ = (name, value);
    Ok(())
}

/// The value a newly added parameter of `kind` gets.
pub fn empty(kind: ParamKind) -> Value {
    match kind {
        ParamKind::Bool => Value::Bool(false),
        ParamKind::Number => Value::Number(Number::int(0)),
        ParamKind::String => Value::String(Str::new("")),
        ParamKind::List | ParamKind::Localized => Value::Array(Array::default()),
        ParamKind::Dict => Value::Dict(Dict::new()),
    }
}

/// A command's JSON value as a property-list value. `name` is the parameter, for messages. JSON
/// `null` and numbers that are not finite are refused; the limits apply.
pub fn from_json(name: &str, json: &Json) -> Result<Value, ValueError> {
    // M7.2: every JSON kind, with the limits.
    let _ = (name, json);
    Err(ValueError::NotYetImplemented("reading parameter values from JSON"))
}

/// A property-list value as JSON (data as base64 text, dates as their text).
pub fn to_json(value: &Value) -> Json {
    // M7.2: every value kind.
    let _ = value;
    Json::Null
}

/// A yes-or-no value (a boolean, or a number, also in text, that is on unless 0).
pub fn flag(value: &Value) -> Option<bool> {
    fontcraft_model::param_flag(value)
}

/// A finite number, or a text that reads as one.
pub fn number(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Number(n) => n.value().as_f64(),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// A text value.
pub fn text(value: &Value) -> Option<&str> {
    value.as_str()
}

/// The lines of a list value: each string item, or the lines of a single text. `None` for any
/// other value or a list holding something other than text.
pub fn lines(value: &Value) -> Option<Vec<&str>> {
    match value {
        Value::Array(a) => a.items.iter().map(Value::as_str).collect(),
        Value::String(s) => Some(s.as_str().lines().collect()),
        _ => None,
    }
}

/// A localized value as (language tag, text) pairs, in order.
pub fn localized(value: &Value) -> Option<Vec<(String, String)>> {
    // M7.2: the list-of-dictionaries form and a plain text (the default language).
    let _ = value;
    None
}
