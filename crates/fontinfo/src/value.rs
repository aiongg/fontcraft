//! Parameter values: the checks every add and set runs, the conversion between the JSON that
//! commands take and the property-list values the model stores, and typed readers.
//!
//! A value is checked against its parameter's kind ([`crate::ParamKind`]) and, where the catalogue
//! names one, the parameter's own syntax: here for Name Table Entry, the glyph lists, Filter and
//! versionString, in [`crate::syntax`] for the value lists and the other syntaxes and ranges. A
//! wrong value is refused and nothing changes. Values read from
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
use crate::glyphset::{GlyphList, ReencodeList, RenameList};
use crate::target::Scope;

/// Parameters whose value may carry a lowercase script name and a colon before it
/// (`greek: 705`), as `docs/spec/18-custom-parameters.md` › Per-script values lists them.
const PER_SCRIPT: &[&str] = &[
    "ascender",
    "capHeight",
    "descender",
    "italicAngle",
    "Default Layer Width",
    "CJK Guide",
    "CJK Grid",
    "CJK Grid Horizontal",
    "CJK Grid Vertical",
];

/// The language a plain text of a localized value is given.
const DEFAULT_LANGUAGE: &str = fontcraft_model::DEFAULT_LANGUAGE;

/// Longest language tag of a localized value, in bytes.
const MAX_LANGUAGE_BYTES: usize = 8;

/// `value` as `spec` stores it (numbers in text read as numbers, a single string for a list of
/// one…), or why it is refused.
pub fn check(spec: &ParamSpec, value: &Value) -> Result<Value, ValueError> {
    check_limits(spec.name, value)?;
    let wrong = || ValueError::WrongKind { name: spec.name.to_string(), expected: spec.kind.describe() };
    let checked = match spec.kind {
        ParamKind::Bool => Value::Bool(flag(value).ok_or_else(wrong)?),
        ParamKind::Number => number_value(spec, value).ok_or_else(wrong)??,
        ParamKind::String => match value {
            Value::String(_) | Value::Number(_) => value.clone(),
            _ => return Err(wrong()),
        },
        ParamKind::List => match value {
            Value::Array(_) => value.clone(),
            Value::String(s) => Value::Array(Array {
                items: s.as_str().lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Value::String(Str::new(l))).collect(),
                ..Array::default()
            }),
            Value::Number(_) => Value::Array(Array { items: vec![value.clone()], ..Array::default() }),
            _ => return Err(wrong()),
        },
        ParamKind::Dict => match value {
            Value::Dict(_) => value.clone(),
            _ => return Err(wrong()),
        },
        ParamKind::Localized => localized_value(spec.name, value)?,
    };
    syntax(spec, &checked)?;
    let refused = |message: String| ValueError::Syntax { name: spec.name.to_string(), message };
    let checked = crate::syntax::choices(spec, checked).map_err(refused)?;
    crate::syntax::check(spec, checked).map_err(refused)
}

/// A number parameter's value: `None` when it is not a number at all; an error when it is one
/// outside the parameter's range.
fn number_value(spec: &ParamSpec, value: &Value) -> Option<Result<Value, ValueError>> {
    let Some(n) = number(value) else {
        // `script: value` for the parameters that take one.
        let text = value.as_str()?;
        return (PER_SCRIPT.contains(&spec.name) && per_script(text).is_some()).then(|| Ok(Value::String(Str::new(text.trim()))));
    };
    let range = |min: f64, max: f64| ValueError::OutOfRange { name: spec.name.to_string(), min, max };
    if matches!(spec.name, "unitsPerEm" | "Scale to UPM") {
        let (min, max) = (f64::from(*crate::UPM_RANGE.start()), f64::from(*crate::UPM_RANGE.end()));
        if n.fract() != 0.0 || n < min || n > max {
            return Some(Err(range(min, max)));
        }
    } else if spec.units && n.abs() > crate::MAX_SCALED_COORDINATE {
        return Some(Err(range(-crate::MAX_SCALED_COORDINATE, crate::MAX_SCALED_COORDINATE)));
    }
    Some(Ok(number_of(n)))
}

/// A finite number as a property-list number: an integer when it is whole.
fn number_of(n: f64) -> Value {
    // Whole numbers within ±2^53 convert to i64 exactly.
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::Number(Number::int(n as i64))
    } else {
        Number::float(n).map_or_else(|| Value::Number(Number::int(0)), Value::Number)
    }
}

/// `[script:] number`: the script (empty when none) and the number.
pub(crate) fn per_script(text: &str) -> Option<(&str, f64)> {
    let (script, n) = match text.split_once(':') {
        Some((s, n)) => (s.trim(), n),
        None => ("", text),
    };
    let ok_script = script.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    let n = n.trim().parse::<f64>().ok().filter(|n| n.is_finite())?;
    ok_script.then_some((script, n))
}

/// The parameter's own syntax, for the parameters the catalogue gives one.
fn syntax(spec: &ParamSpec, value: &Value) -> Result<(), ValueError> {
    let bad = |message: String| ValueError::Syntax { name: spec.name.to_string(), message };
    let glyph_set = |e: crate::PatternError| bad(e.to_string());
    match spec.name {
        "Name Table Entry" => {
            let text = value.as_str().ok_or_else(|| bad("the value is a text: `nameID; text`".into()))?;
            name_table_entry(text).map_err(bad)
        }
        "Prevent Name ID" => {
            let id = number(value).filter(|n| n.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(n));
            id.map(|_| ()).ok_or_else(|| bad("the value is one name ID, a whole number from 0 to 65535".into()))
        }
        "Remove Glyphs" | "Keep Glyphs" | "Export Glyphs" => GlyphList::parse(&list_lines(spec, value)?).map(|_| ()).map_err(glyph_set),
        "Rename Glyphs" => RenameList::parse(&list_lines(spec, value)?).map(|_| ()).map_err(glyph_set),
        "Reencode Glyphs" => ReencodeList::parse(&list_lines(spec, value)?).map(|_| ()).map_err(glyph_set),
        "Filter" | "PreFilter" => filter_string(value.as_str().unwrap_or_default()).map_err(bad),
        "versionString" => version_string(value.as_str().unwrap_or_default()).map_err(bad),
        name if PER_SCRIPT.contains(&name) && value.as_str().is_some() => {
            let text = value.as_str().unwrap_or_default();
            per_script(text).map(|_| ()).ok_or_else(|| bad(format!("`{text}` is not a number, or a lowercase script name, a colon and a number")))
        }
        _ => Ok(()),
    }
}

/// The text lines of a list value, or an error naming the parameter.
fn list_lines<'a>(spec: &ParamSpec, value: &'a Value) -> Result<Vec<&'a str>, ValueError> {
    lines(value).ok_or_else(|| ValueError::Syntax { name: spec.name.to_string(), message: "every entry is a text".into() })
}

/// A number of a Name Table Entry: decimal, octal with a leading 0, or hexadecimal with `0x`.
fn entry_number(token: &str) -> Option<u16> {
    if let Some(hex) = token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()
    } else if token.len() > 1
        && let Some(octal) = token.strip_prefix('0')
    {
        u16::from_str_radix(octal, 8).ok()
    } else {
        token.parse::<u16>().ok()
    }
}

/// `nameID; text`, `nameID platformID; text` or `nameID platformID encID langID; text`.
fn name_table_entry(text: &str) -> Result<(), String> {
    let (head, tail) = text.split_once(';').ok_or("the value is `nameID; text`, with a semicolon before the text")?;
    let tokens: Vec<&str> = head.split_whitespace().collect();
    if !matches!(tokens.len(), 1 | 2 | 4) {
        return Err("before the semicolon come the name ID, the name ID and platform, or all four of name ID, platform, encoding and language".into());
    }
    let mut numbers = Vec::with_capacity(tokens.len());
    for t in &tokens {
        numbers.push(entry_number(t).ok_or_else(|| format!("`{t}` is not a number from 0 to 65535"))?);
    }
    let id = numbers.first().copied().unwrap_or_default();
    if matches!(id, 1 | 2 | 3 | 5 | 6) {
        return Err(format!("name ID {id} is set from the font's own fields and cannot be given here"));
    }
    if let Some(platform) = numbers.get(1)
        && !matches!(platform, 1 | 3)
    {
        return Err(format!("the platform is 1 or 3, not {platform}"));
    }
    if tail.trim().is_empty() {
        return Err("the text after the semicolon is empty".into());
    }
    Ok(())
}

/// A filter name, then semicolon-separated arguments; an `include:` or `exclude:` argument with
/// glyph names comes last.
fn filter_string(text: &str) -> Result<(), String> {
    let mut parts = text.split(';').map(str::trim);
    let name = parts.next().unwrap_or_default();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(format!("`{name}` is not a filter name: the value starts with one, then its arguments after semicolons"));
    }
    let args: Vec<&str> = parts.collect();
    for (i, arg) in args.iter().enumerate() {
        let limit = arg.strip_prefix("include:").or_else(|| arg.strip_prefix("exclude:"));
        if let Some(names) = limit {
            if i + 1 != args.len() {
                return Err("`include:` or `exclude:` comes last".into());
            }
            if names.split([',', ' ']).all(|n| n.trim().is_empty()) {
                return Err(format!("`{arg}` names no glyph"));
            }
        }
    }
    Ok(())
}

/// `Version ` and a number (`1.000`, digits each below 65535) or a template with `%d`, then
/// anything after a semicolon or a space.
fn version_string(text: &str) -> Result<(), String> {
    let rest = text.strip_prefix("Version ").ok_or("the value begins with `Version ` and a number")?;
    let number = rest.split([';', ' ']).next().unwrap_or_default();
    if number.contains('%') {
        return if number.chars().all(|c| c.is_ascii_digit() || matches!(c, '%' | 'd' | '.')) {
            Ok(())
        } else {
            Err(format!("`{number}` is not a version template such as `%d.%03d`"))
        };
    }
    let mut parts = number.split('.');
    let whole = |p: Option<&str>| {
        p.filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).and_then(|p| p.parse::<u32>().ok()).filter(|n| *n < 65_535)
    };
    match (whole(parts.next()), whole(parts.next()), parts.next()) {
        (Some(_), Some(_), None) => Ok(()),
        _ => Err(format!("`{number}` is not a version number: digits, a period, digits, each below 65535")),
    }
}

/// A localized value as stored: a list of `{language, value}` dictionaries. Takes that list, a
/// dictionary from language to text, or a plain text (the default language).
fn localized_value(name: &str, value: &Value) -> Result<Value, ValueError> {
    let pairs: Vec<(String, String)> = match value {
        Value::Dict(d) => {
            let mut out = Vec::with_capacity(d.len());
            for (k, v) in &d.entries {
                let text = v.as_str().ok_or_else(|| ValueError::WrongKind { name: name.to_string(), expected: ParamKind::Localized.describe() })?;
                out.push((k.as_str().to_string(), text.to_string()));
            }
            out
        }
        _ => localized(value).ok_or_else(|| ValueError::WrongKind { name: name.to_string(), expected: ParamKind::Localized.describe() })?,
    };
    if pairs.len() > crate::MAX_LANGUAGES {
        return Err(ValueError::LimitExceeded { what: "languages", limit: crate::MAX_LANGUAGES });
    }
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::with_capacity(pairs.len());
    for (language, text) in pairs {
        let tag_ok = !language.is_empty() && language.len() <= MAX_LANGUAGE_BYTES && language.chars().all(|c| c.is_ascii_graphic());
        if !tag_ok {
            return Err(ValueError::Syntax { name: name.to_string(), message: format!("`{language}` is not a language tag") });
        }
        if !seen.insert(language.clone()) {
            return Err(ValueError::Syntax { name: name.to_string(), message: format!("the language `{language}` is given twice") });
        }
        if text.len() > crate::MAX_FIELD_BYTES {
            return Err(ValueError::LimitExceeded { what: "bytes in one language's text", limit: crate::MAX_FIELD_BYTES });
        }
        let mut d = Dict::new();
        d.insert("language", Value::String(Str::new(language)));
        d.insert("value", Value::String(Str::new(text)));
        items.push(Value::Dict(d));
    }
    Ok(Value::Array(Array { items, ..Array::default() }))
}

/// Refuses `spec` at a scope its entry does not allow.
pub fn check_scope(spec: &ParamSpec, scope: Scope) -> Result<(), ValueError> {
    if spec.scopes.allows(scope) { Ok(()) } else { Err(ValueError::WrongScope { name: spec.name.to_string(), scope: scope.name() }) }
}

/// The limits alone, for a parameter the catalogue does not know (kept as given).
pub fn check_limits(_name: &str, value: &Value) -> Result<(), ValueError> {
    // Iterative, so a deep value cannot exhaust the stack.
    let mut bytes = 0usize;
    let mut stack: Vec<(&Value, usize)> = vec![(value, 1)];
    while let Some((v, depth)) = stack.pop() {
        if depth > crate::MAX_VALUE_DEPTH {
            return Err(ValueError::LimitExceeded { what: "levels of nesting", limit: crate::MAX_VALUE_DEPTH });
        }
        match v {
            Value::Dict(d) => {
                if d.len() > crate::MAX_VALUE_ITEMS {
                    return Err(ValueError::LimitExceeded { what: "entries in one dictionary", limit: crate::MAX_VALUE_ITEMS });
                }
                for (k, item) in &d.entries {
                    bytes = bytes.saturating_add(k.as_str().len());
                    stack.push((item, depth + 1));
                }
            }
            Value::Array(a) => {
                if a.items.len() > crate::MAX_VALUE_ITEMS {
                    return Err(ValueError::LimitExceeded { what: "items in one list", limit: crate::MAX_VALUE_ITEMS });
                }
                stack.extend(a.items.iter().map(|item| (item, depth + 1)));
            }
            Value::String(s) | Value::Date(s) => bytes = bytes.saturating_add(s.as_str().len()),
            Value::Data(d) => bytes = bytes.saturating_add(d.bytes().len()),
            Value::Number(_) | Value::Bool(_) => {}
        }
        if bytes > crate::MAX_VALUE_BYTES {
            return Err(ValueError::LimitExceeded { what: "bytes of text in one value", limit: crate::MAX_VALUE_BYTES });
        }
    }
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
    let value = json_value(name, json, 1)?;
    check_limits(name, &value)?;
    Ok(value)
}

fn json_value(name: &str, json: &Json, depth: usize) -> Result<Value, ValueError> {
    if depth > crate::MAX_VALUE_DEPTH {
        return Err(ValueError::LimitExceeded { what: "levels of nesting", limit: crate::MAX_VALUE_DEPTH });
    }
    Ok(match json {
        Json::Null => return Err(ValueError::NotAValue(format!("`{name}`: null is not a parameter value"))),
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Number(Number::int(i)),
            None => Value::Number(n.as_f64().and_then(Number::float).ok_or_else(|| ValueError::NotFinite { name: name.to_string() })?),
        },
        Json::String(s) => {
            if s.len() > crate::MAX_VALUE_BYTES {
                return Err(ValueError::LimitExceeded { what: "bytes of text in one value", limit: crate::MAX_VALUE_BYTES });
            }
            Value::String(Str::new(s.as_str()))
        }
        Json::Array(items) => {
            if items.len() > crate::MAX_VALUE_ITEMS {
                return Err(ValueError::LimitExceeded { what: "items in one list", limit: crate::MAX_VALUE_ITEMS });
            }
            let items = items.iter().map(|v| json_value(name, v, depth + 1)).collect::<Result<Vec<_>, _>>()?;
            Value::Array(Array { items, ..Array::default() })
        }
        Json::Object(map) => {
            if map.len() > crate::MAX_VALUE_ITEMS {
                return Err(ValueError::LimitExceeded { what: "entries in one dictionary", limit: crate::MAX_VALUE_ITEMS });
            }
            let mut d = Dict::new();
            for (k, v) in map {
                d.insert(k.as_str(), json_value(name, v, depth + 1)?);
            }
            Value::Dict(d)
        }
    })
}

/// A property-list value as JSON (data as base64 text, dates as their text).
pub fn to_json(value: &Value) -> Json {
    json_of(value, 1)
}

fn json_of(value: &Value, depth: usize) -> Json {
    if depth > crate::MAX_VALUE_DEPTH {
        return Json::Null;
    }
    match value {
        Value::Dict(d) => {
            let mut map = serde_json::Map::new();
            for (k, v) in &d.entries {
                // A repeated key reads as its first value, as `Dict::get` does.
                map.entry(k.as_str()).or_insert_with(|| json_of(v, depth + 1));
            }
            Json::Object(map)
        }
        Value::Array(a) => Json::Array(a.items.iter().map(|v| json_of(v, depth + 1)).collect()),
        Value::String(s) | Value::Date(s) => Json::String(s.as_str().to_string()),
        Value::Number(n) => match n.value() {
            fontcraft_plist::Num::Int(i) => Json::from(i),
            fontcraft_plist::Num::Float(f) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
        },
        Value::Data(d) => Json::String(base64(d.bytes())),
        Value::Bool(b) => Json::Bool(*b),
    }
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let sym = |i: u32| ALPHABET.get((i & 63) as usize).map_or('=', |c| char::from(*c));
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(sym(n >> 18));
        out.push(sym(n >> 12));
        out.push(if chunk.len() > 1 { sym(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(n) } else { '=' });
    }
    out
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

/// A localized value as (language tag, text) pairs, in order: a list of `{language, value}`
/// dictionaries, or a plain text (the default language).
pub fn localized(value: &Value) -> Option<Vec<(String, String)>> {
    match value {
        Value::String(s) => Some(vec![(DEFAULT_LANGUAGE.to_string(), s.as_str().to_string())]),
        Value::Array(a) => a
            .items
            .iter()
            .map(|item| {
                let d = item.as_dict()?;
                Some((d.get("language")?.as_str()?.to_string(), d.get("value")?.as_str()?.to_string()))
            })
            .collect(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Effect, Scopes};
    use serde_json::json;

    fn spec(name: &'static str, kind: ParamKind) -> ParamSpec {
        ParamSpec {
            name,
            aliases: &[],
            kind,
            scopes: Scopes::ANY,
            effect: Effect::Other,
            milestone: "M7",
            units: false,
            repeatable: false,
            choices: &[],
            names: crate::catalog::Names::Nothing,
            summary: "",
        }
    }

    fn check_json(spec: &ParamSpec, j: Json) -> Result<Json, ValueError> {
        check(spec, &from_json(spec.name, &j)?).map(|v| to_json(&v))
    }

    #[test]
    fn values_are_checked_and_normalised_per_kind() {
        let b = spec("Use Typo Metrics", ParamKind::Bool);
        assert_eq!(check_json(&b, json!(true)), Ok(json!(true)));
        assert_eq!(check_json(&b, json!(0)), Ok(json!(false)));
        assert!(matches!(check_json(&b, json!(["x"])), Err(ValueError::WrongKind { .. })));

        let mut n = spec("typoAscender", ParamKind::Number);
        assert_eq!(check_json(&n, json!("800")), Ok(json!(800)));
        assert_eq!(check_json(&n, json!(12.5)), Ok(json!(12.5)));
        assert!(matches!(check_json(&n, json!("tall")), Err(ValueError::WrongKind { .. })));
        n.units = true;
        assert!(matches!(check_json(&n, json!(40_000)), Err(ValueError::OutOfRange { .. })));
        let upm = spec("Scale to UPM", ParamKind::Number);
        assert!(matches!(check_json(&upm, json!(8)), Err(ValueError::OutOfRange { .. })));
        assert_eq!(check_json(&spec("Default Layer Width", ParamKind::Number), json!("hebrew: 460")), Ok(json!("hebrew: 460")));

        let l = spec("Disable Masters", ParamKind::List);
        assert_eq!(check_json(&l, json!("Bold\n\nLight")), Ok(json!(["Bold", "Light"])));
        assert!(matches!(check_json(&l, json!({"a": 1})), Err(ValueError::WrongKind { .. })));
        assert!(matches!(check_json(&spec("gasp Table", ParamKind::Dict), json!(8)), Err(ValueError::WrongKind { .. })));

        let loc = spec("designers", ParamKind::Localized);
        assert_eq!(check_json(&loc, json!("Ann")), Ok(json!([{"language": "dflt", "value": "Ann"}])));
        assert_eq!(check_json(&loc, json!({"dflt": "Ann", "DEU": "Anna"})).map(|j| j.as_array().map(Vec::len)), Ok(Some(2)));
        assert!(check_json(&loc, json!([{"language": "dflt", "value": "A"}, {"language": "dflt", "value": "B"}])).is_err());
        assert_eq!(localized(&check(&loc, &Value::String(Str::new("Ann"))).unwrap()), Some(vec![("dflt".into(), "Ann".into())]));
    }

    #[test]
    fn parameter_syntaxes() {
        let nte = spec("Name Table Entry", ParamKind::String);
        for good in ["25; Prefix", "16 3; Family", "16 3 1 0x409; Family", "16 1 0 010; Family"] {
            assert!(check_json(&nte, json!(good)).is_ok(), "{good}");
        }
        for bad in ["25 Prefix", "1; Name", "16 2; Text", "16 3 1; Text", "16; ", "x; Text"] {
            assert!(matches!(check_json(&nte, json!(bad)), Err(ValueError::Syntax { .. })), "{bad}");
        }
        let f = spec("Filter", ParamKind::String);
        assert!(check_json(&f, json!("RoundCorner; 20; 1; include: a, b")).is_ok());
        assert!(check_json(&f, json!("; 20")).is_err());
        assert!(check_json(&f, json!("RemoveOverlap; exclude: x; 20")).is_err());
        let v = spec("versionString", ParamKind::String);
        for good in ["Version 1.000", "Version 2.010;beta", "Version %d.%03d"] {
            assert!(check_json(&v, json!(good)).is_ok(), "{good}");
        }
        for bad in ["1.000", "Version one", "Version 1", "Version 70000.0"] {
            assert!(check_json(&v, json!(bad)).is_err(), "{bad}");
        }
        assert!(check_json(&spec("Prevent Name ID", ParamKind::String), json!("16")).is_ok());
        assert!(check_json(&spec("Prevent Name ID", ParamKind::String), json!("x")).is_err());
        assert!(check_json(&spec("ascender", ParamKind::String), json!("greek: 705")).is_ok());
        assert!(check_json(&spec("ascender", ParamKind::String), json!("Greek 705")).is_err());
    }

    #[test]
    fn hostile_values_are_refused_without_crashing() {
        let big = "x".repeat(crate::MAX_VALUE_BYTES + 1);
        assert!(matches!(from_json("note", &json!(big)), Err(ValueError::LimitExceeded { .. })));
        let many: Vec<Json> = (0..100_000).map(|i| json!(i)).collect();
        assert!(matches!(from_json("glyphOrder", &Json::Array(many)), Err(ValueError::LimitExceeded { .. })));
        assert!(matches!(from_json("x", &json!([1, null])), Err(ValueError::NotAValue(_))));
        // A value nested deeper than the limit, built in code (JSON text could not hold it).
        let mut deep = Value::Array(Array::default());
        for _ in 0..crate::MAX_VALUE_DEPTH + 5 {
            deep = Value::Array(Array { items: vec![deep], ..Array::default() });
        }
        assert!(matches!(check_limits("x", &deep), Err(ValueError::LimitExceeded { .. })));
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
