//! General fields, settings, the note and the row clipboard: `font.info.listFields`, `.addField`, `.removeField`, `.setLocalizedValue`, `.removeLocalizedValue`, `.copyRows`, `.pasteRows`, `.stepVersion`, `.updateDate`, `font.settings.get/set`, `font.note.get/set`. `font.info.get/set` stay in `font.rs` and share this module's scope, target and field helpers.
//!
//! Every command takes its targets in its parameters: `scope` (`font`, `master`, `instance`) and,
//! below the font, `target` or `targets` (an id, a name or an index, through
//! `fontcraft_fontinfo::target`). Several targets change in one undo step; a refusal changes
//! nothing. Warnings (family name characters, units-per-em thresholds) come back in the result.

use std::sync::Arc;

use fontcraft_fontinfo::fields::{self, FieldSpec};
use fontcraft_fontinfo::rows::{self, PasteMode, Row, RowKind};
use fontcraft_fontinfo::{Scope, catalog, target, value as param_value};
use fontcraft_model::plist::{Dict, Str, Value as PlistValue};
use fontcraft_model::{DEFAULT_LANGUAGE, Extra, Font, Localized, Param, Params as ParamList, Properties, Property, PropertyValue, StyleClass};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, kind};

/// Longest font note, in bytes.
const MAX_NOTE_BYTES: usize = fontcraft_fontinfo::MAX_VALUE_BYTES;
/// Largest grid spacing and keyboard increment.
const MAX_GRID_OR_INCREMENT: f64 = 10_000.0;
/// Largest grid subdivision.
const MAX_SUBDIVISION: i64 = 1_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.info.listFields",
            "List Available Fields",
            [],
            None,
            "{scope: 'font'|'master'|'instance', query?: string, target?: id|name|index} the fields the plus button beside the General heading offers at `scope` (built-in fields are always there and not listed), filtered by `query` (part of the key or label, any case); present: whether `target` has the field, or without one whether any master or instance of the scope has it → {fields: [{key, label, localizable, present}]}",
            has_doc,
            list_fields
        ),
        cmd!(
            "font.info.addField",
            "Add Field",
            [],
            None,
            "{scope: 'font'|'master'|'instance', targets?: [id|name|index] (required below the font), key: string, value?: string, language?: string} add a General field to each target as one undo step. A localizable field gets one entry in `language` (default dflt, the default language); its value defaults to the built-in value it overrides (familyNames the Family Name, styleNames the Style Name), else empty text. A target that already has the field refuses the whole command → {key, targets}",
            has_doc,
            add_field
        ),
        CommandSpec {
            when: Some("panel:fontInfo"),
            ..cmd!(
                "font.info.removeField",
                "Remove Field",
                [],
                Some("Cmd+Delete"),
                "{scope: 'font'|'master'|'instance', targets?: [id|name|index] (required below the font), keys: [string]} remove added General fields from each target as one undo step; built-in fields (Family Name, Units per Em, Version, Creation Date, Style Name, Weight Class, Width Class, a master's name) cannot be removed, and a target without one of the fields refuses the whole command. Custom parameters are removed with font.customParameters.remove → {removed: [keys]}",
                has_doc,
                remove_field
            )
        },
        cmd!(
            "font.info.setLocalizedValue",
            "Set Localized Value",
            [],
            None,
            "{scope: 'font'|'master'|'instance', target?: id|name|index (required below the font), key: string, language: string, value: string} add or change one language of a localizable field, adding the field when the target lacks it. language: dflt (the default) or a language-system tag the name table maps to a Windows language (DEU, FRA…) → {key, language}",
            has_doc,
            set_localized
        ),
        cmd!(
            "font.info.removeLocalizedValue",
            "Remove Localized Value",
            [],
            None,
            "{scope: 'font'|'master'|'instance', target?: id|name|index (required below the font), key: string, language: string} remove one language of a localizable field; removing its last language removes the field → {key, language, fieldRemoved: bool}",
            has_doc,
            remove_localized
        ),
        cmd!(
            query "font.info.copyRows",
            "Copy Rows",
            [],
            None,
            "{scope: 'font'|'master'|'instance', target?: id|name|index (required below the font), keys?: [string]} the Font Info rows of one target as clipboard text: every added field and custom parameter in stored order, or the rows `keys` name (built-in fields such as familyName and unitsPerEm too; a parameter name gives every parameter of that name) in that order → {rows: [{scope, kind: 'field'|'param', key, value, enabled}], json: string, text: string (property list in the source format's spelling)}",
            has_doc,
            copy_rows
        ),
        cmd!(
            "font.info.pasteRows",
            "Paste Rows",
            [],
            None,
            "{scope: 'font'|'master'|'instance', targets?: [id|name|index] (required below the font), rows?: [row] | {fontcraftRows, rows}, text?: string (either clipboard form), mode?: 'replace'|'append'|'skip' (default replace)} paste rows into each target as one undo step. replace: a field or a parameter the target has takes the pasted value, except a parameter that may repeat, which is appended; append: parameters are added after the others; skip: rows the target already has are left out. A field cannot repeat, so append replaces it. A row the scope does not allow is skipped and listed; a value that fails its check refuses the whole paste → {added, replaced, skipped: [{target?, key, reason}]}",
            has_doc,
            paste_rows
        ),
        cmd!(
            "font.info.stepVersion",
            "Step Minor Version",
            [],
            None,
            "{delta?: integer from -999 to 999 (default 1)} add `delta` to the minor version; a step past 0 or 999 is refused → {versionMajor, versionMinor}",
            has_doc,
            step_version
        ),
        cmd!(
            "font.info.updateDate",
            "Update Creation Date",
            [],
            None,
            "{date?: ISO 8601 text (2026-10-11T12:00:00Z), the format's text (2026-10-11 12:00:00 +0000) or seconds since 1970} set the Creation Date to `date`, or to the current time; stored in UTC in the format's form → {date}",
            has_doc,
            update_date
        ),
        cmd!(
            query "font.settings.get",
            "Get Font Settings",
            [],
            None,
            "{keys?: [string]} the Other Settings values, with the defaults a font that does not set them gets → {gridSpacing, gridSubdivision, keyboardIncrementShift, keyboardIncrementCmd, useCustomNaming, disableAutomaticAlignment, keepAlternatesNextToBase, fileFormatVersion, fontType}, or only `keys`",
            has_doc,
            settings_get
        ),
        cmd!(
            "font.settings.set",
            "Set Font Settings",
            [],
            None,
            "{gridSpacing?: 0–10000 (0: no rounding), gridSubdivision?: integer 1–1000, keyboardIncrementShift?: above 0 to 10000, keyboardIncrementCmd?: above 0 to 10000, useCustomNaming?: bool, disableAutomaticAlignment?: bool, keepAlternatesNextToBase?: bool, fileFormatVersion?: 2|3|4, fontType?: 'default'|'variable'} change Other Settings as one undo step; no glyph is re-rounded or renamed → {changed: [keys]}",
            has_doc,
            settings_set
        ),
        cmd!(query "font.note.get", "Get Font Note", [], None, "{} the font note (the Notes tab; never exported) → {text}", has_doc, note_get),
        cmd!(
            "font.note.set",
            "Set Font Note",
            [],
            None,
            "{text: string (at most 4 MiB)} replace the font note, the text the note custom parameter holds in format version 2 files; never exported → {text}",
            has_doc,
            note_set
        ),
    ]
}

// ---------- scopes, targets and fields (shared with `font.rs`) ----------

/// The `scope` parameter; the font when it is missing.
pub(super) fn scope_of(p: &Params) -> Result<Scope> {
    match p.opt_str("scope")? {
        None => Ok(Scope::Font),
        Some(name) => Scope::from_name(name).ok_or_else(|| p.bad(format!("`scope` is font, master or instance, got `{name}`"))),
    }
}

/// The masters or instances the `key` parameter (`target` or `targets`) names; `[0]` for the font.
pub(super) fn targets_of(p: &Params, font: &Font, scope: Scope, key: &str) -> Result<Vec<usize>> {
    if scope == Scope::Font {
        return Ok(vec![0]);
    }
    let value = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required at {} scope", scope.name())))?;
    let refs = target::parse_list(value).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    let found = match scope {
        Scope::Master => target::masters(font, &refs),
        _ => target::instances(font, &refs),
    };
    found.map_err(|e| p.bad(e.to_string()))
}

/// The one master or instance the `target` parameter names; 0 for the font.
pub(super) fn target_of(p: &Params, font: &Font, scope: Scope) -> Result<usize> {
    let all = targets_of(p, font, scope, "target")?;
    match all.as_slice() {
        [one] => Ok(*one),
        _ => Err(p.bad("`target` names one master or instance")),
    }
}

pub(super) fn properties(font: &Font, scope: Scope, i: usize) -> Option<&Properties> {
    match scope {
        Scope::Font => Some(&font.info.properties),
        Scope::Master => font.masters.get(i).map(|m| &m.properties),
        Scope::Instance => font.instances.get(i).map(|x| &x.properties),
    }
}

fn properties_mut(font: &mut Font, scope: Scope, i: usize) -> Option<&mut Properties> {
    match scope {
        Scope::Font => Some(&mut Arc::make_mut(&mut font.info).properties),
        Scope::Master => Arc::make_mut(&mut font.masters).get_mut(i).map(|m| &mut m.properties),
        Scope::Instance => Arc::make_mut(&mut font.instances).get_mut(i).map(|x| &mut x.properties),
    }
}

pub(super) fn params_of(font: &Font, scope: Scope, i: usize) -> Option<&ParamList> {
    match scope {
        Scope::Font => Some(&font.info.params),
        Scope::Master => font.masters.get(i).map(|m| &m.params),
        Scope::Instance => font.instances.get(i).map(|x| &x.params),
    }
}

fn params_mut(font: &mut Font, scope: Scope, i: usize) -> Option<&mut ParamList> {
    match scope {
        Scope::Font => Some(&mut Arc::make_mut(&mut font.info).params),
        Scope::Master => Arc::make_mut(&mut font.masters).get_mut(i).map(|m| &mut m.params),
        Scope::Instance => Arc::make_mut(&mut font.instances).get_mut(i).map(|x| &mut x.params),
    }
}

/// A property's value as commands report it: the text, or `[{language, value}]` in order.
pub(super) fn property_value_json(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Single(s) => json!(s),
        PropertyValue::Localized(v) => v.iter().map(|l| json!({"language": l.language, "value": l.value})).collect(),
    }
}

/// The added fields of one target, in stored order.
pub(super) fn fields_json(props: &Properties, scope: Scope) -> Value {
    props
        .0
        .iter()
        .map(|p| {
            let spec = fields::field(scope, &p.key);
            json!({
                "key": p.key,
                "label": spec.map_or(p.key.as_str(), |f| f.label),
                "localizable": matches!(p.value, PropertyValue::Localized(_)),
                "value": property_value_json(&p.value),
            })
        })
        .collect()
}

/// The custom parameters of one target, in stored order.
pub(super) fn params_json(params: &ParamList) -> Value {
    params.0.iter().map(|p| json!({"name": p.name, "value": param_value::to_json(&p.value), "enabled": !p.disabled})).collect()
}

/// The key a property is stored under: the field's own spelling, or the key as given when the
/// target already holds a property of that name the field lists do not know (kept as read).
fn property_key(props: &Properties, scope: Scope, key: &str) -> Option<String> {
    match fields::field(scope, key) {
        Some(spec) if !spec.built_in => Some(spec.key.to_string()),
        Some(_) => None,
        None => props.get(key).map(|p| p.key.clone()),
    }
}

/// A built-in field's text from a command's JSON: numbers for the numeric fields, text for the
/// others.
fn builtin_text(spec: &FieldSpec, v: &Value) -> std::result::Result<String, String> {
    let numeric = matches!(spec.key, "unitsPerEm" | "versionMajor" | "versionMinor" | "weightClass" | "widthClass");
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) if numeric => Ok(n.to_string()),
        other => Err(format!("`{}` must be {}, got {}", spec.key, if numeric { "a whole number" } else { "a string" }, kind(other))),
    }
}

/// Writes one field of one target: a built-in field, or an added field (added when absent).
/// A text sets a localizable field's default language; `[{language, value}]` replaces all of
/// its languages. Warnings go to `warnings`.
pub(super) fn set_field(
    font: &mut Font,
    scope: Scope,
    i: usize,
    key: &str,
    v: &Value,
    warnings: &mut Vec<String>,
) -> std::result::Result<(), String> {
    let spec = fields::field(scope, key);
    if let Some(spec) = spec.filter(|s| s.built_in) {
        let text = builtin_text(spec, v)?;
        fields::check_text(spec, &text).map_err(|e| e.to_string())?;
        return set_builtin(font, scope, i, spec.key, &text, warnings);
    }
    let props = properties(font, scope, i).ok_or_else(|| format!("there is no {} #{i}", scope.name()))?;
    let Some(stored) = property_key(props, scope, key) else {
        return Err(format!("`{key}` is not a field of the {} (font.info.listFields lists them)", scope.name()));
    };
    let present = props.get(&stored);
    let localizable = spec.map_or_else(|| present.is_some_and(|p| matches!(p.value, PropertyValue::Localized(_))), |s| s.localizable);
    if present.is_none() && props.0.len() >= fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
        return Err(format!("the {} already has {} fields", scope.name(), fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE));
    }
    let new_value = match v {
        Value::String(_) | Value::Number(_) => {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if text.len() > fontcraft_fontinfo::MAX_FIELD_BYTES {
                return Err(format!("`{key}` is longer than {} bytes", fontcraft_fontinfo::MAX_FIELD_BYTES));
            }
            match present.map(|p| &p.value) {
                Some(PropertyValue::Localized(entries)) => {
                    let mut entries = entries.clone();
                    match entries.iter_mut().find(|l| l.language == DEFAULT_LANGUAGE) {
                        Some(l) => l.value = text,
                        None => entries.insert(0, Localized { language: DEFAULT_LANGUAGE.into(), value: text, extra: Extra::new() }),
                    }
                    PropertyValue::Localized(entries)
                }
                Some(PropertyValue::Single(_)) => PropertyValue::Single(text),
                None if localizable => {
                    PropertyValue::Localized(vec![Localized { language: DEFAULT_LANGUAGE.into(), value: text, extra: Extra::new() }])
                }
                None => PropertyValue::Single(text),
            }
        }
        Value::Array(entries) if localizable => {
            if entries.len() > fontcraft_fontinfo::MAX_LANGUAGES {
                return Err(format!("`{key}` has more than {} languages", fontcraft_fontinfo::MAX_LANGUAGES));
            }
            let mut dict = Dict::new();
            for e in entries {
                let (Some(language), Some(text)) = (e.get("language").and_then(Value::as_str), e.get("value").and_then(Value::as_str)) else {
                    return Err(format!("`{key}`: each entry is {{language, value}}"));
                };
                dict.entries.push((Str::new(language), PlistValue::String(Str::new(text))));
            }
            rows::property_from_value(&stored, &PlistValue::Dict(dict), true).map_err(|e| e.to_string())?
        }
        other => {
            let expected = if localizable { "a string or a list of {language, value}" } else { "a string" };
            return Err(format!("`{key}` must be {expected}, got {}", kind(other)));
        }
    };
    write_property(font, scope, i, &stored, new_value)
}

/// Replaces the property `key` of one target in place, or appends it.
fn write_property(font: &mut Font, scope: Scope, i: usize, key: &str, value: PropertyValue) -> std::result::Result<(), String> {
    let props = properties_mut(font, scope, i).ok_or_else(|| format!("there is no {} #{i}", scope.name()))?;
    match props.0.iter_mut().find(|p| p.key == key) {
        Some(p) => p.value = value,
        None => props.0.push(Property { key: key.to_string(), value, extra: Extra::new() }),
    }
    Ok(())
}

/// Writes a built-in field from text [`fields::check_text`] accepted.
fn set_builtin(font: &mut Font, scope: Scope, i: usize, key: &str, text: &str, warnings: &mut Vec<String>) -> std::result::Result<(), String> {
    let whole = |t: &str| -> std::result::Result<u16, String> {
        t.trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(n))
            .map(|n| n as u16)
            .ok_or_else(|| format!("`{key}` must be a whole number"))
    };
    let missing = || format!("there is no {} #{i}", scope.name());
    match (scope, key) {
        (Scope::Font, "familyName") => {
            let name = text.trim();
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
                warnings.push(
                    "the family name should hold only ASCII letters, digits and spaces: other characters can stop the fonts from exporting or installing (add a familyNames field for other names)"
                        .into(),
                );
            }
            font.family_name = name.to_string();
        }
        (Scope::Font, "unitsPerEm") => {
            let upm = whole(text)?;
            warnings.extend(upm_warnings(upm));
            font.units_per_em = upm;
        }
        (Scope::Font, "versionMajor") => font.version_major = whole(text)?,
        (Scope::Font, "versionMinor") => font.version_minor = whole(text)?,
        (Scope::Font, "date") => {
            let date = fields::date_seconds(text).and_then(fields::date_text).ok_or_else(|| format!("`{key}` is not a date"))?;
            Arc::make_mut(&mut font.info).date = Some(date);
        }
        (Scope::Master, "name") => Arc::make_mut(&mut font.masters).get_mut(i).ok_or_else(missing)?.name = text.trim().to_string(),
        (Scope::Instance, "name") => Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?.name = text.trim().to_string(),
        (Scope::Instance, "weightClass") => {
            let n = whole(text)?;
            Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?.weight_class = Some(StyleClass::Number(f64::from(n)));
        }
        (Scope::Instance, "widthClass") => {
            let n = whole(text)?;
            Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?.width_class = Some(StyleClass::Number(f64::from(n)));
        }
        _ => return Err(format!("`{key}` is not a field of the {}", scope.name())),
    }
    Ok(())
}

/// The handbook's cautions about large units per em (`fontInfo.unitsPerEm.limits`).
pub(super) fn upm_warnings(upm: u16) -> Vec<String> {
    let mut out = Vec::new();
    if upm > 3000 {
        out.push("some applications have trouble with more than 3000 units per em".to_string());
    }
    if upm > 4096 {
        out.push("glyph widths of fonts with CFF outlines can be unreliable above 4096 units per em".to_string());
    }
    if upm > 5000 {
        out.push("some design applications have trouble with more than 5000 units per em".to_string());
    }
    out
}

// ---------- the field commands ----------

fn list_fields(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.listFields", p)?;
    let font = s.font()?;
    let scope = scope_of(&p)?;
    let on: Vec<usize> = match p.get("target") {
        Some(_) => vec![target_of(&p, font, scope)?],
        None => match scope {
            Scope::Font => vec![0],
            Scope::Master => (0..font.masters.len()).collect(),
            Scope::Instance => (0..font.instances.len()).collect(),
        },
    };
    let query = p.opt_str("query")?.map(str::to_lowercase).unwrap_or_default();
    let list: Vec<Value> = fields::fields(scope)
        .iter()
        .filter(|f| !f.built_in)
        .filter(|f| query.is_empty() || f.key.to_lowercase().contains(&query) || f.label.to_lowercase().contains(&query))
        .map(|f| {
            let present = on.iter().any(|i| properties(font, scope, *i).is_some_and(|props| props.get(f.key).is_some()));
            json!({"key": f.key, "label": f.label, "localizable": f.localizable, "present": present})
        })
        .collect();
    Ok(json!({ "fields": list }))
}

/// The added field `key` names at `scope`, or a refusal that says why it cannot be added.
fn addable(p: &Params, scope: Scope, key: &str) -> Result<&'static FieldSpec> {
    match fields::field(scope, key) {
        Some(f) if !f.built_in => Ok(f),
        Some(f) => Err(p.bad(format!("`{}` is a built-in field: change it with font.info.set", f.key))),
        None => Err(p.bad(format!("`{key}` is not a field the {} offers (font.info.listFields lists them)", scope.name()))),
    }
}

fn language_of(p: &Params, key: &str) -> Result<Option<String>> {
    let Some(language) = p.opt_str(key)? else { return Ok(None) };
    let language = language.trim();
    if !fields::is_language(language) {
        return Err(p.bad(format!("`{language}` is not a language: dflt or a language-system tag such as DEU or FRA")));
    }
    Ok(Some(language.to_string()))
}

fn add_field(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.addField", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = targets_of(&p, font, scope, "targets")?;
    let spec = addable(&p, scope, p.str("key")?)?;
    let language = language_of(&p, "language")?;
    if language.is_some() && !spec.localizable {
        return Err(p.bad(format!("`{}` holds one text for all languages: leave `language` out", spec.key)));
    }
    let given = match p.get("value") {
        None => None,
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(other) => return Err(p.bad(format!("`value` must be a string, got {}", kind(other)))),
    };
    if given.as_ref().is_some_and(|t| t.len() > fontcraft_fontinfo::MAX_FIELD_BYTES) {
        return Err(p.bad(format!("`value` is longer than {} bytes", fontcraft_fontinfo::MAX_FIELD_BYTES)));
    }
    let language = language.unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    let count = targets.len();
    s.edit("Add Field", |font| {
        for &i in &targets {
            let start = given.clone().unwrap_or_else(|| match fields::overrides(spec.key) {
                Some("familyName") => font.family_name.clone(),
                Some("name") if scope == Scope::Instance => font.instances.get(i).map(|x| target::instance_name(x).to_string()).unwrap_or_default(),
                _ => String::new(),
            });
            let props = properties_mut(font, scope, i).ok_or_else(|| p.bad(format!("there is no {} #{i}", scope.name())))?;
            if props.get(spec.key).is_some() {
                return Err(p.bad(format!(
                    "the {} already has `{}`: change it with font.info.set or font.info.setLocalizedValue",
                    scope.name(),
                    spec.key
                )));
            }
            if props.0.len() >= fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
                return Err(p.bad(format!("the {} already has {} fields", scope.name(), fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE)));
            }
            let value = if spec.localizable {
                PropertyValue::Localized(vec![Localized { language: language.clone(), value: start, extra: Extra::new() }])
            } else {
                PropertyValue::Single(start)
            };
            props.0.push(Property { key: spec.key.to_string(), value, extra: Extra::new() });
        }
        Ok(())
    })?;
    Ok(json!({ "key": spec.key, "targets": count }))
}

fn remove_field(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.removeField", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = targets_of(&p, font, scope, "targets")?;
    let keys = p.strings("keys")?;
    if keys.is_empty() {
        return Err(p.bad("`keys` is empty"));
    }
    if keys.len() > fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
        return Err(p.bad(format!("more than {} keys", fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE)));
    }
    for key in &keys {
        if let Some(f) = fields::field(scope, key).filter(|f| f.built_in) {
            return Err(p.bad(format!("`{}` is a built-in field and cannot be removed", f.key)));
        }
    }
    s.edit("Remove Field", |font| {
        for &i in &targets {
            for key in &keys {
                let props = properties_mut(font, scope, i).ok_or_else(|| p.bad(format!("there is no {} #{i}", scope.name())))?;
                let stored = fields::field(scope, key).map_or(*key, |f| f.key);
                if !props.remove(stored) && !props.remove(key) {
                    return Err(p.bad(format!("the {} has no field `{key}`", scope.name())));
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "removed": keys }))
}

fn set_localized(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.setLocalizedValue", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let i = target_of(&p, font, scope)?;
    let key = p.str("key")?;
    let language = language_of(&p, "language")?.ok_or_else(|| p.bad("`language` is required"))?;
    let text = p.str("value")?;
    if text.len() > fontcraft_fontinfo::MAX_FIELD_BYTES {
        return Err(p.bad(format!("`value` is longer than {} bytes", fontcraft_fontinfo::MAX_FIELD_BYTES)));
    }
    let props = properties(font, scope, i).ok_or_else(|| p.bad(format!("there is no {} #{i}", scope.name())))?;
    let spec = fields::field(scope, key);
    if spec.is_some_and(|f| f.built_in) {
        return Err(p.bad(format!("`{key}` is a built-in field: change it with font.info.set")));
    }
    let stored = property_key(props, scope, key).ok_or_else(|| p.bad(format!("`{key}` is not a field of the {}", scope.name())))?;
    let present = props.get(&stored).map(|x| x.value.clone());
    let localizable = spec.map_or(matches!(present, Some(PropertyValue::Localized(_))), |f| f.localizable);
    if !localizable {
        return Err(p.bad(format!("`{stored}` holds one text for all languages: change it with font.info.set")));
    }
    if present.is_none() && props.0.len() >= fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
        return Err(p.bad(format!("the {} already has {} fields", scope.name(), fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE)));
    }
    let mut entries = match present {
        Some(PropertyValue::Localized(entries)) => entries,
        Some(PropertyValue::Single(old)) => vec![Localized { language: DEFAULT_LANGUAGE.into(), value: old, extra: Extra::new() }],
        None => Vec::new(),
    };
    match entries.iter().position(|l| l.language == language) {
        Some(at) => {
            if let Some(l) = entries.get_mut(at) {
                l.value = text.to_string();
            }
        }
        None if entries.len() >= fontcraft_fontinfo::MAX_LANGUAGES => {
            return Err(p.bad(format!("`{stored}` already has {} languages", fontcraft_fontinfo::MAX_LANGUAGES)));
        }
        None => entries.push(Localized { language: language.clone(), value: text.to_string(), extra: Extra::new() }),
    }
    s.edit("Set Localized Value", |font| write_property(font, scope, i, &stored, PropertyValue::Localized(entries)).map_err(|m| p.bad(m)))?;
    Ok(json!({ "key": stored, "language": language }))
}

fn remove_localized(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.removeLocalizedValue", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let i = target_of(&p, font, scope)?;
    let key = p.str("key")?;
    let language = p.str("language")?.trim().to_string();
    let props = properties(font, scope, i).ok_or_else(|| p.bad(format!("there is no {} #{i}", scope.name())))?;
    let stored = property_key(props, scope, key).ok_or_else(|| p.bad(format!("`{key}` is not a field of the {}", scope.name())))?;
    let entries = match props.get(&stored).map(|x| &x.value) {
        Some(PropertyValue::Localized(entries)) => entries,
        Some(PropertyValue::Single(_)) => return Err(p.bad(format!("`{stored}` holds one text for all languages"))),
        None => return Err(p.bad(format!("the {} has no field `{stored}`", scope.name()))),
    };
    if !entries.iter().any(|l| l.language == language) {
        return Err(p.bad(format!("`{stored}` has no `{language}` entry")));
    }
    let rest: Vec<Localized> = entries.iter().filter(|l| l.language != language).cloned().collect();
    let field_removed = rest.is_empty();
    s.edit("Remove Localized Value", |font| {
        if field_removed {
            if let Some(props) = properties_mut(font, scope, i) {
                props.remove(&stored);
            }
            Ok(())
        } else {
            write_property(font, scope, i, &stored, PropertyValue::Localized(rest)).map_err(|m| p.bad(m))
        }
    })?;
    Ok(json!({ "key": stored, "language": language, "fieldRemoved": field_removed }))
}

// ---------- the row clipboard ----------

fn copy_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.copyRows", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let i = target_of(&p, font, scope)?;
    let rows: Vec<Row> = match p.opt_strings("keys")? {
        None => rows::rows_of(font, scope, i),
        Some(keys) => {
            if keys.len() > fontcraft_fontinfo::MAX_ROWS {
                return Err(p.bad(format!("more than {} keys", fontcraft_fontinfo::MAX_ROWS)));
            }
            let all = rows::rows_of(font, scope, i);
            let mut out = Vec::new();
            for key in keys {
                if let Some(f) = fields::field(scope, key).filter(|f| f.built_in) {
                    out.push(rows::builtin_row(font, scope, i, f.key).ok_or_else(|| p.bad(format!("`{}` is not set", f.key)))?);
                    continue;
                }
                let field = fields::field(scope, key).map_or(key, |f| f.key);
                let param = catalog::canonical(key).unwrap_or(key);
                let found: Vec<Row> = all
                    .iter()
                    .filter(|r| match r.kind {
                        RowKind::Field => r.key == field || r.key == key,
                        RowKind::Param => r.key == param || r.key == key,
                    })
                    .cloned()
                    .collect();
                if found.is_empty() {
                    return Err(p.bad(format!("the {} has no field or parameter `{key}`", scope.name())));
                }
                out.extend(found);
            }
            out
        }
    };
    if rows.len() > fontcraft_fontinfo::MAX_ROWS {
        return Err(p.bad(format!("more than {} rows", fontcraft_fontinfo::MAX_ROWS)));
    }
    Ok(json!({
        "rows": rows.iter().map(rows::row_json).collect::<Vec<_>>(),
        "json": rows::to_json(&rows),
        "text": rows::to_plist_text(&rows),
    }))
}

/// What one paste did.
#[derive(Default)]
struct PasteReport {
    added: usize,
    replaced: usize,
    skipped: Vec<Value>,
}

impl PasteReport {
    fn skip(&mut self, scope: Scope, i: usize, key: &str, reason: String) {
        self.skipped.push(if scope == Scope::Font {
            json!({"key": key, "reason": reason})
        } else {
            json!({"target": i, "key": key, "reason": reason})
        });
    }
}

fn paste_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.pasteRows", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = targets_of(&p, font, scope, "targets")?;
    let rows = match (p.get("rows"), p.opt_str("text")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `rows` or `text`, not both")),
        (Some(json), None) => rows::from_json_value(json),
        (None, Some(text)) => rows::from_text(text),
        (None, None) => return Err(p.bad("`rows` or `text` is required")),
    }
    .map_err(|e| p.bad(e.to_string()))?;
    let mode = match p.opt_str("mode")? {
        None => PasteMode::default(),
        Some(m) => PasteMode::from_name(m).ok_or_else(|| p.bad(format!("`mode` is replace, append or skip, got `{m}`")))?,
    };
    let report = s.edit("Paste Rows", |font| {
        let mut report = PasteReport::default();
        for &i in &targets {
            for row in &rows {
                paste_row(font, scope, i, row, mode, &mut report).map_err(|m| p.bad(format!("row `{}`: {m}", row.key)))?;
            }
        }
        Ok(report)
    })?;
    Ok(json!({ "added": report.added, "replaced": report.replaced, "skipped": report.skipped }))
}

fn paste_row(font: &mut Font, scope: Scope, i: usize, row: &Row, mode: PasteMode, report: &mut PasteReport) -> std::result::Result<(), String> {
    match row.kind {
        RowKind::Field => {
            let Some(spec) = fields::field(scope, &row.key) else {
                report.skip(scope, i, &row.key, format!("not a field of the {}", scope.name()));
                return Ok(());
            };
            if spec.built_in {
                if mode == PasteMode::Skip {
                    report.skip(scope, i, &row.key, "already there".into());
                    return Ok(());
                }
                let json = match &row.value {
                    PlistValue::String(t) => json!(t.as_str()),
                    PlistValue::Number(n) => json!(n.value().as_f64()),
                    _ => return Err(format!("`{}` takes a text or a number", spec.key)),
                };
                let mut ignored = Vec::new();
                set_field(font, scope, i, spec.key, &json, &mut ignored)?;
                report.replaced += 1;
                return Ok(());
            }
            let value = rows::property_from_value(spec.key, &row.value, spec.localizable).map_err(|e| e.to_string())?;
            let props = properties(font, scope, i).ok_or_else(|| format!("there is no {} #{i}", scope.name()))?;
            let present = props.get(spec.key).is_some();
            if present && mode == PasteMode::Skip {
                report.skip(scope, i, &row.key, "already there".into());
                return Ok(());
            }
            if !present && props.0.len() >= fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
                return Err(format!("the {} already has {} fields", scope.name(), fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE));
            }
            write_property(font, scope, i, spec.key, value)?;
            if present {
                report.replaced += 1;
            } else {
                report.added += 1;
            }
        }
        RowKind::Param => {
            let spec = catalog::lookup(&row.key);
            let (name, value, repeatable) = match spec {
                Some(spec) => {
                    if param_value::check_scope(spec, scope).is_err() {
                        report.skip(scope, i, &row.key, format!("cannot be set on the {}", scope.name()));
                        return Ok(());
                    }
                    (spec.name.to_string(), param_value::check(spec, &row.value).map_err(|e| e.to_string())?, spec.repeatable)
                }
                None => {
                    param_value::check_limits(&row.key, &row.value).map_err(|e| e.to_string())?;
                    (row.key.clone(), row.value.clone(), false)
                }
            };
            let list = params_mut(font, scope, i).ok_or_else(|| format!("there is no {} #{i}", scope.name()))?;
            let existing = list.0.iter().position(|q| q.name == name);
            match (existing, mode) {
                (Some(_), PasteMode::Skip) => report.skip(scope, i, &row.key, "already there".into()),
                (Some(at), PasteMode::Replace) if !repeatable => {
                    if let Some(q) = list.0.get_mut(at) {
                        q.value = value;
                        q.disabled = !row.enabled;
                    }
                    report.replaced += 1;
                }
                _ => {
                    if list.0.len() >= fontcraft_fontinfo::MAX_PARAMS_PER_SCOPE {
                        return Err(format!("the {} already has {} custom parameters", scope.name(), fontcraft_fontinfo::MAX_PARAMS_PER_SCOPE));
                    }
                    let mut param = Param::new(name, value);
                    param.disabled = !row.enabled;
                    list.0.push(param);
                    report.added += 1;
                }
            }
        }
    }
    Ok(())
}

// ---------- version and date ----------

fn step_version(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.stepVersion", p)?;
    let delta = p.opt_int("delta", -999, 999)?.unwrap_or(1);
    let font = s.font()?;
    let next = i64::from(font.version_minor) + delta;
    let max = i64::from(fontcraft_fontinfo::MAX_VERSION_MINOR);
    if !(0..=max).contains(&next) {
        return Err(p.bad(format!("the minor version is {} and stays from 0 to {max}: a step of {delta} would make it {next}", font.version_minor)));
    }
    let minor = u16::try_from(next).map_err(|_| p.bad("the minor version is out of range"))?;
    s.edit("Step Minor Version", |font| {
        font.version_minor = minor;
        Ok(())
    })?;
    let font = s.font()?;
    Ok(json!({ "versionMajor": font.version_major, "versionMinor": font.version_minor }))
}

fn update_date(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.updateDate", p)?;
    let seconds = match p.get("date") {
        None => now(s).ok_or_else(|| p.bad("the current time is not known: give `date`"))?,
        Some(Value::String(t)) => fields::date_seconds(t).ok_or_else(|| p.bad(format!("`date` is not a date: `{t}`")))?,
        Some(Value::Number(_)) => p.opt_int("date", 0, 253_402_300_799)?.unwrap_or(0),
        Some(other) => return Err(p.bad(format!("`date` must be a text or a number, got {}", kind(other)))),
    };
    let text = fields::date_text(seconds).ok_or_else(|| p.bad("`date` must be from 1970 to 9999"))?;
    s.edit("Update Creation Date", |font| {
        Arc::make_mut(&mut font.info).date = Some(text.clone());
        Ok(())
    })?;
    Ok(json!({ "date": text }))
}

/// The session's clock, else the system's.
fn now(s: &Session) -> Option<i64> {
    match &s.clock {
        Some(clock) => clock(),
        None => system_now(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn system_now() -> Option<i64> {
    crate::clock::system_clock()()
}

#[cfg(target_arch = "wasm32")]
fn system_now() -> Option<i64> {
    None
}

// ---------- Other Settings ----------

const SETTING_KEYS: [&str; 9] = [
    "gridSpacing",
    "gridSubdivision",
    "keyboardIncrementShift",
    "keyboardIncrementCmd",
    "useCustomNaming",
    "disableAutomaticAlignment",
    "keepAlternatesNextToBase",
    "fileFormatVersion",
    "fontType",
];

fn settings_json(font: &Font) -> Value {
    let st = &font.info.settings;
    json!({
        "gridSpacing": st.grid_length.unwrap_or(1.0),
        "gridSubdivision": st.grid_sub_division.unwrap_or(1.0),
        "keyboardIncrementShift": st.keyboard_increment_big.unwrap_or(10.0),
        "keyboardIncrementCmd": st.keyboard_increment_huge.unwrap_or(100.0),
        "useCustomNaming": st.disables_nice_names.unwrap_or(false),
        "disableAutomaticAlignment": st.disables_automatic_alignment.unwrap_or(false),
        "keepAlternatesNextToBase": st.keep_alternates_together.unwrap_or(false),
        "fileFormatVersion": font.source.glyphs_version.unwrap_or(3),
        "fontType": st.font_type.as_deref().unwrap_or("default"),
    })
}

fn settings_get(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.settings.get", p)?;
    let all = settings_json(s.font()?);
    let Some(keys) = p.opt_strings("keys")? else { return Ok(all) };
    let mut out = serde_json::Map::new();
    for key in keys {
        let value = all.get(key).ok_or_else(|| p.bad(format!("unknown key `{key}` (known: {})", SETTING_KEYS.join(", "))))?;
        out.insert(key.to_string(), value.clone());
    }
    Ok(Value::Object(out))
}

/// Stores `value` unless it is what the font already has, stated or by default.
fn put<T: PartialEq + Copy>(slot: &mut Option<T>, value: T, default: T) {
    if slot.unwrap_or(default) != value {
        *slot = Some(value);
    }
}

fn settings_set(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.settings.set", raw)?;
    if let Some(unknown) = raw.as_object().and_then(|given| given.keys().find(|k| !SETTING_KEYS.contains(&k.as_str()))) {
        return Err(p.bad(format!("unknown key `{unknown}` (known: {})", SETTING_KEYS.join(", "))));
    }
    let changed: Vec<&str> = SETTING_KEYS.iter().copied().filter(|k| p.get(k).is_some()).collect();
    if changed.is_empty() {
        return Err(p.bad(format!("give at least one of {}", SETTING_KEYS.join(", "))));
    }
    let number = |key: &str, min: f64, min_open: bool| -> Result<Option<f64>> {
        let Some(n) = p.opt_f64(key)? else { return Ok(None) };
        if !(min..=MAX_GRID_OR_INCREMENT).contains(&n) || (min_open && n <= min) {
            let low = if min_open { format!("above {min}") } else { format!("{min}") };
            return Err(p.bad(format!("`{key}` must be from {low} to {MAX_GRID_OR_INCREMENT}, got {n}")));
        }
        Ok(Some(n))
    };
    let grid = number("gridSpacing", 0.0, false)?;
    let subdivision = p.opt_int("gridSubdivision", 1, MAX_SUBDIVISION)?;
    let shift = number("keyboardIncrementShift", 0.0, true)?;
    let cmd = number("keyboardIncrementCmd", 0.0, true)?;
    let flag = |key: &str| -> Result<Option<bool>> { if p.get(key).is_some() { p.bool_or(key, false).map(Some) } else { Ok(None) } };
    let (custom, alignment, alternates) = (flag("useCustomNaming")?, flag("disableAutomaticAlignment")?, flag("keepAlternatesNextToBase")?);
    let version = match p.opt_int("fileFormatVersion", 2, 4)? {
        None => None,
        Some(v) => Some(u32::try_from(v).map_err(|_| p.bad("`fileFormatVersion` is 2, 3 or 4"))?),
    };
    let font_type = match p.opt_str("fontType")? {
        None => None,
        Some(t @ ("default" | "variable")) => Some(t),
        Some(other) => return Err(p.bad(format!("`fontType` is default or variable, got `{other}`"))),
    };
    s.edit("Set Font Settings", |font| {
        if let Some(v) = version
            && font.source.glyphs_version.unwrap_or(3) != v
        {
            font.source.glyphs_version = Some(v);
        }
        let st = &mut Arc::make_mut(&mut font.info).settings;
        if let Some(v) = grid {
            put(&mut st.grid_length, v, 1.0);
        }
        if let Some(v) = subdivision {
            put(&mut st.grid_sub_division, v as f64, 1.0);
        }
        if let Some(v) = shift {
            put(&mut st.keyboard_increment_big, v, 10.0);
        }
        if let Some(v) = cmd {
            put(&mut st.keyboard_increment_huge, v, 100.0);
        }
        if let Some(v) = custom {
            put(&mut st.disables_nice_names, v, false);
        }
        if let Some(v) = alignment {
            put(&mut st.disables_automatic_alignment, v, false);
        }
        if let Some(v) = alternates {
            put(&mut st.keep_alternates_together, v, false);
        }
        match font_type {
            Some("default") if st.font_type.is_some() => st.font_type = None,
            Some("variable") if st.font_type.as_deref() != Some("variable") => st.font_type = Some("variable".into()),
            _ => {}
        }
        Ok(())
    })?;
    Ok(json!({ "changed": changed }))
}

// ---------- the note ----------

fn note_get(s: &mut Session, p: &Value) -> Result<Value> {
    Params::new("font.note.get", p)?;
    Ok(json!({ "text": s.font()?.note }))
}

fn note_set(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.note.set", p)?;
    let text = p.str("text")?;
    if text.len() > MAX_NOTE_BYTES {
        return Err(p.bad(format!("`text` is longer than {MAX_NOTE_BYTES} bytes")));
    }
    s.edit("Set Font Note", |font| {
        font.note = text.to_string();
        Ok(())
    })?;
    Ok(json!({ "text": text }))
}
