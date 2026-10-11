//! Font Info rows and their clipboard text.
//!
//! A row is one General field or one custom parameter of the font, a master or an instance.
//! Copy writes the selected rows as JSON (what FontCraft pastes) and as property-list text (what
//! the reference app's clipboard holds); paste reads either. Pasting replaces rows with the same
//! key, appends, or skips them ([`PasteMode`]). At most [`crate::MAX_ROWS`] rows and
//! [`crate::MAX_ROWS_BYTES`] of text.
//!
//! Rows: `font.info.copyRows`, `font.info.pasteRows` (M7.3); the Font Info window's Copy and
//! Paste (M7.15).

use fontcraft_model::{DEFAULT_LANGUAGE, Extra, Font, Localized, Params, Properties, PropertyValue, StyleClass};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Dict, Num, Number, Str, Value};
use serde_json::{Value as Json, json};

use crate::error::{RowsError, ValueError};
use crate::target::Scope;

/// One row of a Font Info tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The scope the row was copied from (a paste may go to another).
    pub scope: Scope,
    pub kind: RowKind,
    /// The property key or the parameter name.
    pub key: String,
    /// A parameter's value; a field's text, or for a localizable field a dictionary from language
    /// tag to text.
    pub value: Value,
    /// A parameter's checkbox; always true for a field.
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Field,
    Param,
}

/// What a paste does with a row whose key the target already has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasteMode {
    /// The pasted row takes the place of the first row with its key.
    #[default]
    Replace,
    /// The pasted row is added after the others.
    Append,
    /// The pasted row is left out.
    Skip,
}

impl PasteMode {
    /// `replace`, `append` or `skip`.
    pub fn from_name(name: &str) -> Option<PasteMode> {
        match name {
            "replace" => Some(PasteMode::Replace),
            "append" => Some(PasteMode::Append),
            "skip" => Some(PasteMode::Skip),
            _ => None,
        }
    }
}

/// The rows of one target: the font (`index` is ignored), or the master or instance at `index`.
/// Fields first, then parameters, each in stored order. An index past the end gives no rows.
pub fn rows_of(font: &Font, scope: Scope, index: usize) -> Vec<Row> {
    let Some((properties, params)) = parts(font, scope, index) else { return Vec::new() };
    let fields = properties.0.iter().map(|p| Row { scope, kind: RowKind::Field, key: p.key.clone(), value: property_value(&p.value), enabled: true });
    let params = params.0.iter().map(|p| Row { scope, kind: RowKind::Param, key: p.name.clone(), value: p.value.clone(), enabled: !p.disabled });
    fields.chain(params).collect()
}

/// The properties and parameters of one target.
fn parts(font: &Font, scope: Scope, index: usize) -> Option<(&Properties, &Params)> {
    match scope {
        Scope::Font => Some((&font.info.properties, &font.info.params)),
        Scope::Master => font.masters.get(index).map(|m| (&m.properties, &m.params)),
        Scope::Instance => font.instances.get(index).map(|i| (&i.properties, &i.params)),
    }
}

/// The row of a built-in field (`familyName`, `unitsPerEm`, a master's or instance's `name`…), or
/// `None` when the target has no such field or does not set it (a missing date or class).
pub fn builtin_row(font: &Font, scope: Scope, index: usize, key: &str) -> Option<Row> {
    let value = builtin_value(font, scope, index, key)?;
    Some(Row { scope, kind: RowKind::Field, key: key.to_string(), value, enabled: true })
}

/// The value of a built-in field, as [`builtin_row`] gives it.
pub fn builtin_value(font: &Font, scope: Scope, index: usize, key: &str) -> Option<Value> {
    let text = |s: &str| Some(Value::String(Str::new(s)));
    let class = |c: &Option<StyleClass>| match c.as_ref()? {
        StyleClass::Number(n) => number(*n).map(Value::Number),
        StyleClass::Name(s) => text(s),
    };
    match scope {
        Scope::Font => match key {
            "familyName" => text(&font.family_name),
            "unitsPerEm" => Some(Value::Number(Number::int(i64::from(font.units_per_em)))),
            "versionMajor" => Some(Value::Number(Number::int(i64::from(font.version_major)))),
            "versionMinor" => Some(Value::Number(Number::int(i64::from(font.version_minor)))),
            "date" => text(font.info.date.as_deref()?),
            _ => None,
        },
        Scope::Master => match key {
            "name" => text(&font.masters.get(index)?.name),
            _ => None,
        },
        Scope::Instance => {
            let instance = font.instances.get(index)?;
            match key {
                "name" => text(&instance.name),
                "weightClass" => class(&instance.weight_class),
                "widthClass" => class(&instance.width_class),
                _ => None,
            }
        }
    }
}

/// A number as a property list holds it: an integer when it is whole.
fn number(n: f64) -> Option<Number> {
    if n.fract() == 0.0 && n.abs() < 9.0e15 { Some(Number::int(n as i64)) } else { Number::float(n) }
}

/// A property's value as a row holds it: the text, or a dictionary from language to text.
pub fn property_value(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Single(s) => Value::String(Str::new(s.as_str())),
        PropertyValue::Localized(entries) => {
            let mut dict = Dict::new();
            for l in entries {
                dict.entries.push((Str::new(l.language.as_str()), Value::String(Str::new(l.value.as_str()))));
            }
            Value::Dict(dict)
        }
    }
}

/// A row's field value as a property: a text, a number (written as text), or a dictionary from
/// language to text. A localizable field keeps one text per language (a plain text becomes the
/// default language's); any other field keeps the default language's text, or the first. Each
/// language must be one [`crate::fields::is_language`] accepts; the limits apply.
pub fn property_from_value(key: &str, value: &Value, localizable: bool) -> Result<PropertyValue, ValueError> {
    let checked = |t: &str| -> Result<String, ValueError> {
        if t.len() > crate::MAX_FIELD_BYTES {
            return Err(ValueError::LimitExceeded { what: "bytes in one field", limit: crate::MAX_FIELD_BYTES });
        }
        Ok(t.to_string())
    };
    let plain = match value {
        Value::String(s) => Some(checked(s.as_str())?),
        Value::Number(n) => Some(match n.value() {
            Num::Int(i) => i.to_string(),
            Num::Float(f) => f.to_string(),
        }),
        Value::Dict(_) => None,
        _ => return Err(ValueError::WrongKind { name: key.to_string(), expected: "a text or one text per language" }),
    };
    if let Some(text) = plain {
        return Ok(if localizable {
            PropertyValue::Localized(vec![Localized { language: DEFAULT_LANGUAGE.into(), value: text, extra: Extra::new() }])
        } else {
            PropertyValue::Single(text)
        });
    }
    let Value::Dict(dict) = value else { return Err(ValueError::WrongKind { name: key.to_string(), expected: "a text" }) };
    if dict.entries.is_empty() {
        return Err(ValueError::Syntax { name: key.to_string(), message: "holds no language".into() });
    }
    if dict.entries.len() > crate::MAX_LANGUAGES {
        return Err(ValueError::LimitExceeded { what: "languages in one field", limit: crate::MAX_LANGUAGES });
    }
    let mut entries: Vec<Localized> = Vec::with_capacity(dict.entries.len());
    for (language, text) in &dict.entries {
        let language = language.as_str().trim();
        if !crate::fields::is_language(language) {
            return Err(ValueError::Syntax { name: key.to_string(), message: format!("`{language}` is not a language this field can hold") });
        }
        if entries.iter().any(|l| l.language == language) {
            return Err(ValueError::Syntax { name: key.to_string(), message: format!("`{language}` is given twice") });
        }
        let Some(text) = text.as_str() else {
            return Err(ValueError::WrongKind { name: key.to_string(), expected: "a text for each language" });
        };
        entries.push(Localized { language: language.to_string(), value: checked(text)?, extra: Extra::new() });
    }
    if localizable {
        return Ok(PropertyValue::Localized(entries));
    }
    let first = entries.iter().find(|l| l.language == DEFAULT_LANGUAGE).or(entries.first()).map(|l| l.value.clone());
    Ok(PropertyValue::Single(first.unwrap_or_default()))
}

/// Version of the JSON row format.
const JSON_VERSION: u64 = 1;
/// Longest key (property key or parameter name) of one row, in bytes.
const MAX_KEY_BYTES: usize = 1_024;

/// The rows as JSON text: `{"fontcraftRows": 1, "rows": [{scope, kind, key, value, enabled}]}`.
pub fn to_json(rows: &[Row]) -> String {
    let rows: Vec<Json> = rows.iter().take(crate::MAX_ROWS).map(row_json).collect();
    json!({"fontcraftRows": JSON_VERSION, "rows": rows}).to_string()
}

/// One row as JSON. A field's value is its text, its number, or `[{language, value}]` in order;
/// a parameter's value goes through [`crate::value::to_json`].
pub fn row_json(row: &Row) -> Json {
    let value = match row.kind {
        RowKind::Param => crate::value::to_json(&row.value),
        RowKind::Field => field_json(&row.value),
    };
    let kind = match row.kind {
        RowKind::Field => "field",
        RowKind::Param => "param",
    };
    json!({"scope": row.scope.name(), "kind": kind, "key": row.key, "value": value, "enabled": row.enabled})
}

fn field_json(value: &Value) -> Json {
    match value {
        Value::String(s) => Json::String(s.as_str().to_string()),
        Value::Number(n) => match n.value() {
            Num::Int(i) => Json::from(i),
            Num::Float(f) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
        },
        Value::Dict(d) => {
            Json::Array(d.entries.iter().map(|(k, v)| json!({"language": k.as_str(), "value": v.as_str().unwrap_or_default()})).collect())
        }
        _ => Json::Null,
    }
}

/// Reads [`to_json`]'s text.
pub fn from_json(text: &str) -> Result<Vec<Row>, RowsError> {
    if text.len() > crate::MAX_ROWS_BYTES {
        return Err(RowsError::LimitExceeded { what: "bytes of rows", limit: crate::MAX_ROWS_BYTES });
    }
    let json: Json = serde_json::from_str(text).map_err(|e| RowsError::NotRows(e.to_string()))?;
    from_json_value(&json)
}

/// Reads rows from JSON: [`to_json`]'s object, or its `rows` list alone.
pub fn from_json_value(json: &Json) -> Result<Vec<Row>, RowsError> {
    let list = match json {
        Json::Array(list) => list,
        Json::Object(map) => match (map.get("fontcraftRows").and_then(Json::as_u64), map.get("rows")) {
            (Some(JSON_VERSION), Some(Json::Array(list))) => list,
            (Some(v), Some(_)) if v != JSON_VERSION => return Err(RowsError::NotRows(format!("row format version {v} is not known"))),
            _ => return Err(RowsError::NotRows("expected `fontcraftRows` and a `rows` list".into())),
        },
        _ => return Err(RowsError::NotRows("expected a list of rows".into())),
    };
    if list.len() > crate::MAX_ROWS {
        return Err(RowsError::LimitExceeded { what: "rows", limit: crate::MAX_ROWS });
    }
    list.iter().map(row_from_json).collect()
}

fn row_from_json(json: &Json) -> Result<Row, RowsError> {
    let bad = |m: &str| RowsError::NotRows(m.to_string());
    let Json::Object(map) = json else { return Err(bad("a row is an object")) };
    let scope = match map.get("scope") {
        None | Some(Json::Null) => Scope::Font,
        Some(v) => v.as_str().and_then(Scope::from_name).ok_or_else(|| bad("a row's `scope` is font, master or instance"))?,
    };
    let kind = match map.get("kind").and_then(Json::as_str) {
        Some("field") => RowKind::Field,
        Some("param") => RowKind::Param,
        _ => return Err(bad("a row's `kind` is field or param")),
    };
    let key = row_key(map.get("key").and_then(Json::as_str))?;
    let enabled = match map.get("enabled") {
        None | Some(Json::Null) => true,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(bad("a row's `enabled` is true or false")),
    };
    let json_value = map.get("value").unwrap_or(&Json::Null);
    let value = match kind {
        RowKind::Param => {
            let value = crate::value::from_json(&key, json_value)?;
            crate::value::check_limits(&key, &value)?;
            value
        }
        RowKind::Field => field_from_json(&key, json_value)?,
    };
    Ok(Row { scope, kind, key, value, enabled })
}

fn row_key(key: Option<&str>) -> Result<String, RowsError> {
    match key {
        Some(k) if !k.trim().is_empty() && k.len() <= MAX_KEY_BYTES => Ok(k.to_string()),
        _ => Err(RowsError::NotRows(format!("a row's `key` is a non-empty text of at most {MAX_KEY_BYTES} bytes"))),
    }
}

fn field_from_json(key: &str, json: &Json) -> Result<Value, RowsError> {
    let wrong = || RowsError::Value(ValueError::WrongKind { name: key.to_string(), expected: "a text, a number or a list of {language, value}" });
    match json {
        Json::String(s) => Ok(Value::String(Str::new(s.as_str()))),
        Json::Number(n) => n.as_f64().and_then(number).map(Value::Number).ok_or_else(wrong),
        Json::Array(entries) => {
            if entries.len() > crate::MAX_LANGUAGES {
                return Err(RowsError::LimitExceeded { what: "languages in one field", limit: crate::MAX_LANGUAGES });
            }
            let mut dict = Dict::new();
            for e in entries {
                let (Some(language), Some(text)) = (e.get("language").and_then(Json::as_str), e.get("value").and_then(Json::as_str)) else {
                    return Err(wrong());
                };
                dict.entries.push((Str::new(language), Value::String(Str::new(text))));
            }
            Ok(Value::Dict(dict))
        }
        _ => Err(wrong()),
    }
}

/// The rows as property-list text: parameters as `{name = …; value = …;}` dictionaries, fields as
/// `{key = …; value = …;}`, in one list.
pub fn to_plist_text(rows: &[Row]) -> String {
    let items: Vec<Value> = rows.iter().take(crate::MAX_ROWS).map(row_plist).collect();
    let mut out = String::new();
    openstep::write_value(&mut out, &Value::Array(Array { items, ..Default::default() }), Dialect::Glyphs3);
    out
}

/// One row in the format's spelling: a custom parameter's `{disabled = 1; name; value}`, or a
/// property's `{key; value}` or `{key; values = ({language; value}, …)}`.
fn row_plist(row: &Row) -> Value {
    let mut d = Dict::new();
    let text = |s: &str| Value::String(Str::new(s));
    match row.kind {
        RowKind::Param => {
            if !row.enabled {
                d.insert("disabled", Value::Number(Number::int(1)));
            }
            d.insert("name", text(&row.key));
            d.insert("value", row.value.clone());
        }
        RowKind::Field => {
            d.insert("key", text(&row.key));
            match &row.value {
                Value::Dict(languages) => {
                    let items = languages
                        .entries
                        .iter()
                        .map(|(language, value)| {
                            let mut e = Dict::new();
                            e.insert("language", Value::String(language.clone()));
                            e.insert("value", value.clone());
                            Value::Dict(e)
                        })
                        .collect();
                    d.insert("values", Value::Array(Array { items, ..Default::default() }));
                }
                other => d.insert("value", other.clone()),
            }
        }
    }
    Value::Dict(d)
}

/// Reads property-list rows (ours, or the reference app's clipboard text).
pub fn from_plist_text(text: &str) -> Result<Vec<Row>, RowsError> {
    if text.len() > crate::MAX_ROWS_BYTES {
        return Err(RowsError::LimitExceeded { what: "bytes of rows", limit: crate::MAX_ROWS_BYTES });
    }
    let root = openstep::parse_value(text).map_err(|e| RowsError::NotRows(e.to_string()))?;
    let items: Vec<&Value> = match &root {
        Value::Array(a) => a.items.iter().collect(),
        dict @ Value::Dict(_) => vec![dict],
        _ => return Err(RowsError::NotRows("expected a list of rows".into())),
    };
    if items.len() > crate::MAX_ROWS {
        return Err(RowsError::LimitExceeded { what: "rows", limit: crate::MAX_ROWS });
    }
    items.into_iter().map(row_from_plist).collect()
}

fn row_from_plist(item: &Value) -> Result<Row, RowsError> {
    let bad = |m: &str| RowsError::NotRows(m.to_string());
    let Some(d) = item.as_dict() else { return Err(bad("a row is a dictionary")) };
    if let Some(name) = d.get("name") {
        let key = row_key(name.as_str())?;
        let value = d.get("value").cloned().ok_or_else(|| bad("a parameter row has a `value`"))?;
        crate::value::check_limits(&key, &value)?;
        let enabled = !d.get("disabled").and_then(fontcraft_model::param_flag).unwrap_or(false);
        return Ok(Row { scope: Scope::Font, kind: RowKind::Param, key, value, enabled });
    }
    let key = row_key(d.get("key").and_then(Value::as_str))?;
    let value = match (d.get("values"), d.get("value")) {
        (Some(Value::Array(entries)), _) => {
            if entries.items.len() > crate::MAX_LANGUAGES {
                return Err(RowsError::LimitExceeded { what: "languages in one field", limit: crate::MAX_LANGUAGES });
            }
            let mut dict = Dict::new();
            for e in &entries.items {
                let language = e.as_dict().and_then(|e| e.get("language")).and_then(Value::as_str);
                let text = e.as_dict().and_then(|e| e.get("value")).and_then(Value::as_str);
                let (Some(language), Some(text)) = (language, text) else { return Err(bad("a localized value has `language` and `value`")) };
                dict.entries.push((Str::new(language), Value::String(Str::new(text))));
            }
            Value::Dict(dict)
        }
        (_, Some(v @ (Value::String(_) | Value::Number(_)))) => v.clone(),
        _ => return Err(bad("a field row has a text `value` or a `values` list")),
    };
    Ok(Row { scope: Scope::Font, kind: RowKind::Field, key, value, enabled: true })
}

/// Reads either form: JSON when the text starts with `{"`, else a property list.
pub fn from_text(text: &str) -> Result<Vec<Row>, RowsError> {
    if text.len() > crate::MAX_ROWS_BYTES {
        return Err(RowsError::LimitExceeded { what: "bytes of rows", limit: crate::MAX_ROWS_BYTES });
    }
    if text.trim_start().starts_with("{\"") { from_json(text) } else { from_plist_text(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_rows_round_trip_through_the_property_list_form() {
        let mut font = Font::new("Rows");
        let info = std::sync::Arc::make_mut(&mut font.info);
        info.properties.set("designerURL", "https://example.com", false);
        info.properties.0.push(fontcraft_model::Property {
            key: "familyNames".into(),
            value: PropertyValue::Localized(vec![
                Localized { language: "dflt".into(), value: "Rows".into(), extra: Extra::new() },
                Localized { language: "DEU".into(), value: "Reihen".into(), extra: Extra::new() },
            ]),
            extra: Extra::new(),
        });
        let mut rows = rows_of(&font, Scope::Font, 0);
        rows.push(builtin_row(&font, Scope::Font, 0, "unitsPerEm").unwrap());
        let text = to_plist_text(&rows);
        let back = from_text(&text).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back, rows);
        let localized = property_from_value("familyNames", &back[1].value, true).unwrap();
        assert_eq!(localized, font.info.properties.get("familyNames").unwrap().value);
        // A localizable value pasted into a single-text field keeps the default language's text.
        assert_eq!(property_from_value("x", &back[1].value, false).unwrap(), PropertyValue::Single("Rows".into()));
        assert!(from_text("(1, 2)").is_err());
        assert!(from_text("not rows at all").is_err());
    }

    #[test]
    fn field_rows_round_trip_through_json() {
        let rows = vec![
            Row { scope: Scope::Instance, kind: RowKind::Field, key: "vendorID".into(), value: Value::String(Str::new("ABCD")), enabled: true },
            Row { scope: Scope::Font, kind: RowKind::Field, key: "unitsPerEm".into(), value: Value::Number(Number::int(2048)), enabled: true },
        ];
        assert_eq!(from_text(&to_json(&rows)).unwrap(), rows);
        assert!(from_json(r#"{"fontcraftRows": 9, "rows": []}"#).is_err());
        assert!(from_json(r#"{"fontcraftRows": 1, "rows": [{"kind": "field", "key": "", "value": "x"}]}"#).is_err());
    }
}
