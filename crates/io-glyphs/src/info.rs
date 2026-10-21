//! Custom parameters, properties, user data and settings, as every format version writes them.
//!
//! - `customParameters`: a list of `{name, value, disabled?}`.
//! - `properties` (version 3 and later): a list of `{key, value}` or `{key, values}`, the values
//!   being `{language, value}`.
//! - `userData`: a dictionary.
//! - `settings` (version 3 and later; top-level keys in version 2).
//!
//! A list or dictionary becomes model fields only when all of it reads cleanly and the writer
//! would spell it the same way; otherwise it stays in `Extra` as written. Empty containers stay in
//! `Extra` too, since the model does not tell an empty one from none.

use fontcraft_model::{Localized, Param, Params, Properties, Property, PropertyValue, Settings, UserData};
use fontcraft_plist::{Dict, Num, Str, Value};

use crate::{Result, as_bool01, finish, int, into_extra, list, num, string, take};

/// A dictionary's entries as model `Extra`, or `None` if `v` is not a dictionary.
fn dict(v: &Value) -> Option<Dict> {
    v.as_dict().cloned()
}

/// A string the writer would spell the same way.
fn plain_str(v: Value) -> Option<String> {
    match v {
        Value::String(s) if s.spelling().is_none() => Some(s.as_str().to_string()),
        _ => None,
    }
}

/// A name's text, whatever its spelling. One the writer would spell otherwise (quotes it does
/// not need) is kept beside it in `extra` under `key`, and written again while the text is the
/// same ([`spelled`]).
fn text_value(d: &mut Dict, key: &str) -> Option<String> {
    let v = take(d, key)?;
    let Value::String(s) = &v else { return None };
    let text = s.as_str().to_string();
    if s.spelling().is_some() {
        d.entries.push((key.into(), v));
    }
    Some(text)
}

/// `text` as a value, in the spelling `extra` kept for `key` while it is still that text.
fn spelled(text: &str, extra: &fontcraft_model::Extra, key: &str) -> Value {
    match extra.get(fontcraft_model::Family::Glyphs, key) {
        Some(v @ Value::String(s)) if s.as_str() == text => v.clone(),
        _ => string(text),
    }
}

fn read_param(v: &Value) -> Option<Param> {
    let mut d = dict(v)?;
    let name = plain_str(take(&mut d, "name")?)?;
    let value = take(&mut d, "value")?;
    let disabled = match take(&mut d, "disabled") {
        Some(x) => as_bool01(&x, "disabled").ok()?,
        None => false,
    };
    Some(Param { name, value, disabled, extra: into_extra(d) })
}

/// `customParameters`, when every entry reads as a parameter.
pub(crate) fn take_params(d: &mut Dict) -> Params {
    let Some(Value::Array(a)) = d.get("customParameters") else { return Params::default() };
    if a.items.is_empty() {
        return Params::default();
    }
    match a.items.iter().map(read_param).collect::<Option<Vec<_>>>() {
        Some(p) => {
            take(d, "customParameters");
            Params(p)
        }
        None => Params::default(),
    }
}

pub(crate) fn write_param(p: &Param) -> Value {
    let value = if p.name == fontcraft_model::axes::AXIS_MAPPINGS { number_keys(&p.value) } else { p.value.clone() };
    let mut e = vec![("name".into(), string(&p.name)), ("value".into(), value)];
    if p.disabled {
        e.push(("disabled".into(), int(1)));
    }
    Value::Dict(finish(e, &p.extra))
}

/// An `Axis Mappings` value with the user coordinates of each axis's map spelled as the
/// reference app writes them: as numbers, without quotes. A key with a spelling of its own keeps it.
fn number_keys(v: &Value) -> Value {
    let Value::Dict(axes) = v else { return v.clone() };
    let mut axes = axes.clone();
    for (_, map) in &mut axes.entries {
        if let Value::Dict(m) = map {
            for (k, _) in &mut m.entries {
                if k.spelling().is_none()
                    && let Some(bare) = Str::number_key(k.as_str())
                {
                    *k = bare;
                }
            }
        }
    }
    Value::Dict(axes)
}

pub(crate) fn push_params(e: &mut Vec<(Str, Value)>, p: &Params) {
    if !p.is_empty() {
        e.push(("customParameters".into(), list(p.0.iter().map(write_param).collect())));
    }
}

pub(crate) fn read_localized(v: &Value) -> Option<Localized> {
    let mut d = dict(v)?;
    let language = plain_str(take(&mut d, "language")?)?;
    let value = text_value(&mut d, "value")?;
    Some(Localized { language, value, extra: into_extra(d) })
}

fn read_property(v: &Value) -> Option<Property> {
    let mut d = dict(v)?;
    let key = plain_str(take(&mut d, "key")?)?;
    let value = match (take(&mut d, "value"), take(&mut d, "values")) {
        (Some(v), None) => {
            d.entries.push(("value".into(), v));
            PropertyValue::Single(text_value(&mut d, "value")?)
        }
        (None, Some(Value::Array(a))) => PropertyValue::Localized(a.items.iter().map(read_localized).collect::<Option<_>>()?),
        _ => return None,
    };
    Some(Property { key, value, extra: into_extra(d) })
}

/// `properties`, when every entry reads as a property.
pub(crate) fn take_properties(d: &mut Dict) -> Properties {
    let Some(Value::Array(a)) = d.get("properties") else { return Properties::default() };
    if a.items.is_empty() {
        return Properties::default();
    }
    match a.items.iter().map(read_property).collect::<Option<Vec<_>>>() {
        Some(p) => {
            take(d, "properties");
            Properties(p)
        }
        None => Properties::default(),
    }
}

pub(crate) fn write_localized(l: &Localized) -> Value {
    Value::Dict(finish(vec![("language".into(), string(&l.language)), ("value".into(), spelled(&l.value, &l.extra, "value"))], &l.extra))
}

fn write_property(p: &Property) -> Value {
    let mut e = vec![("key".into(), string(&p.key))];
    match &p.value {
        PropertyValue::Single(s) => e.push(("value".into(), spelled(s, &p.extra, "value"))),
        PropertyValue::Localized(values) => e.push(("values".into(), list(values.iter().map(write_localized).collect()))),
    }
    // A single value's kept spelling is stale once the value is localized: the reader takes a
    // property with both keys for neither.
    let localized = matches!(p.value, PropertyValue::Localized(_));
    Value::Dict(finish(e, &crate::without_key(&p.extra, "value", localized)))
}

pub(crate) fn push_properties(e: &mut Vec<(Str, Value)>, p: &Properties) {
    if !p.is_empty() {
        e.push(("properties".into(), list(p.0.iter().map(write_property).collect())));
    }
}

/// `userData`, when it is a dictionary with something in it.
pub(crate) fn take_user_data(d: &mut Dict) -> UserData {
    match d.get("userData") {
        Some(Value::Dict(u)) if !u.is_empty() => match take(d, "userData") {
            Some(Value::Dict(u)) => UserData(u),
            _ => UserData::default(),
        },
        _ => UserData::default(),
    }
}

pub(crate) fn push_user_data(e: &mut Vec<(Str, Value)>, u: &UserData) {
    if !u.is_empty() {
        e.push(("userData".into(), Value::Dict(u.0.clone())));
    }
}

/// The settings keys: (key, kind) where kind is `b` for a 0/1 flag, `n` for a number, `s` for a
/// string.
const SETTINGS: [(&str, char); 11] = [
    ("disablesAutomaticAlignment", 'b'),
    ("disablesNiceNames", 'b'),
    ("gridLength", 'n'),
    ("gridSubDivision", 'n'),
    ("keyboardIncrement", 'n'),
    ("keyboardIncrementBig", 'n'),
    ("keyboardIncrementHuge", 'n'),
    ("keepAlternatesTogether", 'b'),
    ("previewRemoveOverlap", 'b'),
    ("snapToObjects", 'b'),
    ("fontType", 's'),
];

fn bool_slot<'a>(s: &'a mut Settings, key: &str) -> Option<&'a mut Option<bool>> {
    Some(match key {
        "disablesAutomaticAlignment" => &mut s.disables_automatic_alignment,
        "disablesNiceNames" => &mut s.disables_nice_names,
        "keepAlternatesTogether" => &mut s.keep_alternates_together,
        "previewRemoveOverlap" => &mut s.preview_remove_overlap,
        "snapToObjects" => &mut s.snap_to_objects,
        _ => return None,
    })
}

fn number_slot<'a>(s: &'a mut Settings, key: &str) -> Option<&'a mut Option<f64>> {
    Some(match key {
        "gridLength" => &mut s.grid_length,
        "gridSubDivision" => &mut s.grid_sub_division,
        "keyboardIncrement" => &mut s.keyboard_increment,
        "keyboardIncrementBig" => &mut s.keyboard_increment_big,
        "keyboardIncrementHuge" => &mut s.keyboard_increment_huge,
        _ => return None,
    })
}

/// A number the writer would spell the same way.
fn plain_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) if n.spelling().is_none() => Some(n.value().as_f64()),
        _ => None,
    }
}

/// Moves the settings keys out of `d` (the `settings` dictionary, or in version 2 the root) into
/// `s`; a key whose value the writer would spell otherwise stays.
pub(crate) fn take_settings_keys(d: &mut Dict, s: &mut Settings) {
    for (key, kind) in SETTINGS {
        let Some(v) = d.get(key) else { continue };
        let typed = match kind {
            'b' => match v.as_num() {
                Some(Num::Int(0)) if plain_number(v).is_some() => bool_slot(s, key).map(|slot| *slot = Some(false)),
                Some(Num::Int(1)) if plain_number(v).is_some() => bool_slot(s, key).map(|slot| *slot = Some(true)),
                _ => None,
            },
            'n' => plain_number(v).and_then(|n| number_slot(s, key).map(|slot| *slot = Some(n))),
            _ => match v {
                Value::String(t) if t.spelling().is_none() => {
                    s.font_type = Some(t.as_str().to_string());
                    Some(())
                }
                _ => None,
            },
        };
        if typed.is_some() {
            take(d, key);
        }
    }
}

/// Version 2's top-level settings keys that glyphsLib also reads at the top level of a version 3
/// or 4 file, unless a `settings` dictionary follows them there (it reads that dictionary as
/// replacing them all). It does not read `gridSubDivision` there.
pub(crate) const V2_SETTINGS: [&str; 5] =
    ["disablesAutomaticAlignment", "disablesNiceNames", "gridLength", "keepAlternatesTogether", "keyboardIncrement"];

/// Takes the version 2 settings keys in `found` into `s`; the keys taken.
pub(crate) fn take_v2_settings(mut found: Dict, s: &mut Settings) -> Vec<String> {
    let before: Vec<String> = found.entries.iter().map(|(k, _)| k.as_str().to_string()).collect();
    take_settings_keys(&mut found, s);
    before.into_iter().filter(|k| found.get(k).is_none()).collect()
}

/// Clears the setting `key` in `s` when `value`, read at the top level, gives the same setting;
/// whether it did.
pub(crate) fn clear_if_same(s: &mut Settings, key: &str, value: &Value) -> bool {
    let mut theirs = Settings::default();
    take_settings_keys(&mut Dict { entries: vec![(key.into(), value.clone())], ..Default::default() }, &mut theirs);
    if let Some(slot) = bool_slot(s, key) {
        let other = bool_slot(&mut theirs, key).and_then(|v| *v);
        if slot.is_some() && *slot == other {
            *slot = None;
            return true;
        }
        return false;
    }
    if let Some(slot) = number_slot(s, key) {
        let other = number_slot(&mut theirs, key).and_then(|v| *v);
        if slot.is_some() && *slot == other {
            *slot = None;
            return true;
        }
    }
    false
}

/// The `settings` dictionary of version 3 and later, when it has anything in it.
pub(crate) fn take_settings(d: &mut Dict) -> Settings {
    let mut s = Settings::default();
    let Some(Value::Dict(inner)) = d.get("settings") else { return s };
    if inner.is_empty() {
        return s;
    }
    let Some(Value::Dict(mut inner)) = take(d, "settings") else { return s };
    take_settings_keys(&mut inner, &mut s);
    s.extra = into_extra(inner);
    s
}

/// The settings as entries (`settings` dictionary contents, or version 2 root keys).
pub(crate) fn settings_entries(s: &Settings) -> Result<Vec<(Str, Value)>> {
    let mut e: Vec<(Str, Value)> = Vec::new();
    let mut copy = s.clone();
    for (key, kind) in SETTINGS {
        match kind {
            'b' => {
                if let Some(Some(b)) = bool_slot(&mut copy, key).map(|v| *v) {
                    e.push((key.into(), int(i64::from(b))));
                }
            }
            'n' => {
                if let Some(Some(n)) = number_slot(&mut copy, key).map(|v| *v) {
                    e.push((key.into(), num(n, key)?));
                }
            }
            _ => {
                if let Some(t) = &s.font_type {
                    e.push((key.into(), string(t)));
                }
            }
        }
    }
    Ok(e)
}

pub(crate) fn push_settings(e: &mut Vec<(Str, Value)>, s: &Settings) -> Result<()> {
    if !s.is_empty() {
        e.push(("settings".into(), Value::Dict(finish(settings_entries(s)?, &s.extra))));
    }
    Ok(())
}
