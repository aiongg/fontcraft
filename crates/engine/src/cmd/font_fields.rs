//! General fields, settings, the note and the row clipboard: `font.info.listFields`, `.addField`, `.removeField`, `.setLocalizedValue`, `.removeLocalizedValue`, `.copyRows`, `.pasteRows`, `.stepVersion`, `.updateDate`, `font.settings.get/set`, `font.note.get/set`. `font.info.get/set` stay in `font.rs` and share this module's scope, target and field helpers.
//!
//! Every command takes its targets in its parameters: `scope` (`font`, `master`, `instance`) and,
//! below the font, `target` or `targets` (an id, a name or an index, through
//! `fontcraft_fontinfo::target`). Several targets change in one undo step; a refusal changes
//! nothing. Warnings (`fontcraft_fontinfo::warnings`: family and style names, units per em, weight
//! class) come back in the result.

use std::sync::Arc;

use fontcraft_fontinfo::fields::{self, FieldSpec};
use fontcraft_fontinfo::rows::{self, PasteMode, Row, RowKind};
use fontcraft_fontinfo::{Scope, catalog, target, value as param_value, warnings as warn};
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
            "{scope: 'font'|'master'|'instance', target?: id|name|index or targets?: [id|name|index] (one of them required below the font), key: string, language: string, value?: string, from?: string, onlyMissing?: bool} add or change one language of a localizable field on each target as one undo step, adding the field where a target lacks it. language: dflt (the default) or a language-system tag the name table maps to a Windows language (DEU, FRA…). from: the text of that language moves to `language`, in its place; a target without it is left alone, and without `value` each target keeps its own text. onlyMissing: targets that have the language are left alone. `value` is required without `from` → {key, language, targets: [index] (the changed ones)}",
            has_doc,
            set_localized
        ),
        cmd!(
            "font.info.removeLocalizedValue",
            "Remove Localized Value",
            [],
            None,
            "{scope: 'font'|'master'|'instance', target?: id|name|index or targets?: [id|name|index] (one of them required below the font), key: string, language: string} remove one language of a localizable field from each target as one undo step; removing its last language removes the field. Targets without it are left alone, and it is refused when none has it → {key, language, fieldRemoved: bool, targets: [index] (the changed ones)}",
            has_doc,
            remove_localized
        ),
        cmd!(
            query "font.info.copyRows",
            "Copy Rows",
            [],
            None,
            "{scope: 'font'|'master'|'instance', target?: id|name|index or targets?: [id|name|index] (one of them required below the font), keys?: [string], rows?: [{key: string, kind?: 'field'|'param', index?: integer}]} the Font Info rows of one target as clipboard text: every added field and custom parameter in stored order, the single rows `rows` names (index: which of the rows of that key and kind, from 0; with several targets each row comes from the first that holds it), or the rows `keys` name (built-in fields such as familyName and unitsPerEm too; a parameter name gives every parameter of that name) in that order → {rows: [{scope, kind: 'field'|'param', key, value, enabled}], json: string, text: string (property list in the source format's spelling)}",
            has_doc,
            copy_rows
        ),
        cmd!(
            "font.info.pasteRows",
            "Paste Rows",
            [],
            None,
            "{scope: 'font'|'master'|'instance', targets?: [id|name|index] (required below the font), rows?: [row] | {fontcraftRows, rows}, text?: string (either clipboard form), mode?: 'replace'|'append'|'skip' (default replace)} paste rows into each target as one undo step. replace: a field or a parameter the target has takes the pasted value, except a parameter that may repeat, which is appended; append: parameters are added after the others; skip: rows the target already has are left out. A field cannot repeat, so append replaces it. A parameter matches the target's row under any spelling of it; a name parameter (designers, familyNames…) is written as the name property, as font.customParameters.add does. A row the scope does not allow, and a row whose value fails its check, is skipped and listed with the reason; the other rows are pasted. A target already holding the most fields or parameters refuses the whole paste → {added, replaced, skipped: [{target?, key, reason}]}",
            has_doc,
            paste_rows
        ),
        cmd!(
            "font.info.removeRows",
            "Remove Rows",
            [],
            None,
            "{scope: 'font'|'master'|'instance', targets?: [id|name|index] (required below the font), rows: [{key: string, kind?: 'field'|'param', index?: integer}]} remove added fields and custom parameters from each target as one undo step; key is the row's as stored, index which of the rows of that key and kind, from 0. A target without a row keeps its others, a built-in field stays and is listed, and it is refused when no target has any of the rows → {removed: count, skipped: [{key, reason}]}",
            has_doc,
            remove_rows
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
            warnings.extend(warn::family_name(name));
            font.family_name = name.to_string();
        }
        (Scope::Font, "unitsPerEm") => {
            let upm = whole(text)?;
            warnings.extend(warn::units_per_em(upm));
            font.units_per_em = upm;
        }
        (Scope::Font, "versionMajor") => font.version_major = whole(text)?,
        (Scope::Font, "versionMinor") => font.version_minor = whole(text)?,
        (Scope::Font, "date") => {
            let date = fields::date_seconds(text).and_then(fields::date_text).ok_or_else(|| format!("`{key}` is not a date"))?;
            Arc::make_mut(&mut font.info).date = Some(date);
        }
        (Scope::Master, "name") => Arc::make_mut(&mut font.masters).get_mut(i).ok_or_else(missing)?.name = text.trim().to_string(),
        (Scope::Instance, "name") => {
            Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?.name = text.trim().to_string();
            let font: &Font = font;
            let inst = font.instances.get(i).ok_or_else(missing)?;
            warnings.extend(warn::style_name(font, inst));
        }
        (Scope::Instance, "weightClass") => {
            let n = whole(text)?;
            Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?.weight_class = Some(StyleClass::Number(f64::from(n)));
            warnings.extend(warn::weight_class(f64::from(n)));
        }
        (Scope::Instance, "widthClass") => {
            let n = whole(text)?;
            Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?.width_class = Some(StyleClass::Number(f64::from(n)));
        }
        _ => return Err(format!("`{key}` is not a field of the {}", scope.name())),
    }
    Ok(())
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

/// The targets of a localized edit: `target` names one, `targets` several.
fn one_or_more(p: &Params, font: &Font, scope: Scope) -> Result<Vec<usize>> {
    match (p.get("target"), p.get("targets")) {
        (Some(_), Some(_)) => Err(p.bad("give `target` or `targets`, not both")),
        (None, Some(_)) => {
            let all = targets_of(p, font, scope, "targets")?;
            if all.is_empty() { Err(p.bad(format!("`targets` names no {}", scope.name()))) } else { Ok(all) }
        }
        _ => target_of(p, font, scope).map(|i| vec![i]),
    }
}

/// The languages of one target's localizable field `key`, and the key it is stored under; empty when the target lacks
/// the field.
fn languages_at(p: &Params, font: &Font, scope: Scope, i: usize, key: &str) -> Result<(String, Vec<Localized>)> {
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
    let entries = match present {
        Some(PropertyValue::Localized(entries)) => entries,
        Some(PropertyValue::Single(old)) => vec![Localized { language: DEFAULT_LANGUAGE.into(), value: old, extra: Extra::new() }],
        None => Vec::new(),
    };
    Ok((stored, entries))
}

fn set_localized(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.setLocalizedValue", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = one_or_more(&p, font, scope)?;
    let key = p.str("key")?;
    let language = language_of(&p, "language")?.ok_or_else(|| p.bad("`language` is required"))?;
    let from = p.opt_str("from")?.map(str::trim);
    let text = p.opt_str("value")?;
    if text.is_none() && from.is_none() {
        return Err(p.bad("`value` is required"));
    }
    if text.is_some_and(|t| t.len() > fontcraft_fontinfo::MAX_FIELD_BYTES) {
        return Err(p.bad(format!("`value` is longer than {} bytes", fontcraft_fontinfo::MAX_FIELD_BYTES)));
    }
    let only_missing = p.bool_or("onlyMissing", false)?;
    let mut plan: Vec<(usize, String, Vec<Localized>)> = Vec::with_capacity(targets.len());
    for &i in &targets {
        let (stored, mut entries) = languages_at(&p, font, scope, i, key)?;
        let there = entries.iter().position(|l| l.language == language);
        if only_missing && there.is_some() {
            continue;
        }
        let at = match from {
            // The text under `from` moves to `language`, taking the place of a text there.
            Some(from) => {
                let Some(old) = entries.iter().find(|l| l.language == from).map(|l| l.value.clone()) else { continue };
                if from != language {
                    entries.retain(|l| l.language != language);
                }
                if let Some(l) = entries.iter_mut().find(|l| l.language == from) {
                    l.language = language.clone();
                    l.value = text.map_or(old, str::to_string);
                }
                plan.push((i, stored, entries));
                continue;
            }
            None => there,
        };
        let value = text.unwrap_or_default().to_string();
        let languages = entries.len();
        match at.and_then(|at| entries.get_mut(at)) {
            Some(l) => l.value = value,
            None if languages >= fontcraft_fontinfo::MAX_LANGUAGES => {
                return Err(p.bad(format!("`{stored}` already has {} languages", fontcraft_fontinfo::MAX_LANGUAGES)));
            }
            None => entries.push(Localized { language: language.clone(), value, extra: Extra::new() }),
        }
        plan.push((i, stored, entries));
    }
    let Some(stored) = plan.first().map(|(_, stored, _)| stored.clone()) else {
        return Err(p.bad(match from {
            Some(from) => format!("no target has a `{from}` entry of `{key}`"),
            None => format!("every target already has a `{language}` entry of `{key}`"),
        }));
    };
    let changed: Vec<usize> = plan.iter().map(|(i, ..)| *i).collect();
    s.edit("Set Localized Value", |font| {
        for (i, stored, entries) in plan {
            write_property(font, scope, i, &stored, PropertyValue::Localized(entries)).map_err(|m| p.bad(m))?;
        }
        Ok(())
    })?;
    Ok(json!({ "key": stored, "language": language, "targets": changed }))
}

/// The field `key` of one target without its `language`: the key it is stored under and the languages left.
fn without_language(p: &Params, font: &Font, scope: Scope, i: usize, key: &str, language: &str) -> Result<(String, Vec<Localized>)> {
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
    let rest = entries.iter().filter(|l| l.language != language).cloned().collect();
    Ok((stored, rest))
}

fn remove_localized(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.removeLocalizedValue", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = one_or_more(&p, font, scope)?;
    let key = p.str("key")?;
    let language = p.str("language")?.trim().to_string();
    let mut plan: Vec<(usize, String, Vec<Localized>)> = Vec::with_capacity(targets.len());
    let mut refused = None;
    for &i in &targets {
        match without_language(&p, font, scope, i, key, &language) {
            Ok((stored, rest)) => plan.push((i, stored, rest)),
            Err(e) => refused = Some(e),
        }
    }
    let Some(stored) = plan.first().map(|(_, stored, _)| stored.clone()) else {
        return Err(refused.unwrap_or_else(|| p.bad(format!("no target has a `{language}` entry of `{key}`"))));
    };
    let field_removed = plan.iter().any(|(_, _, rest)| rest.is_empty());
    let changed: Vec<usize> = plan.iter().map(|(i, ..)| *i).collect();
    s.edit("Remove Localized Value", |font| {
        for (i, stored, rest) in plan {
            if rest.is_empty() {
                if let Some(props) = properties_mut(font, scope, i) {
                    props.remove(&stored);
                }
            } else {
                write_property(font, scope, i, &stored, PropertyValue::Localized(rest)).map_err(|m| p.bad(m))?;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "key": stored, "language": language, "fieldRemoved": field_removed, "targets": changed }))
}

/// One row as `rows` names it: the key as stored, and which of the rows of that key and kind, from 0.
struct RowRef {
    /// Either kind when not given.
    kind: Option<RowKind>,
    key: String,
    index: usize,
}

impl RowRef {
    fn is(&self, kind: RowKind) -> bool {
        self.kind.is_none_or(|k| k == kind)
    }

    /// Where the row stands among the rows of `keys`.
    fn place<'k>(&self, keys: impl Iterator<Item = &'k str>) -> Option<usize> {
        keys.enumerate().filter(|(_, key)| *key == self.key.as_str()).nth(self.index).map(|(at, _)| at)
    }
}

/// The rows the `rows` parameter names.
fn row_refs(p: &Params) -> Result<Vec<RowRef>> {
    let list = p.array("rows")?;
    if list.is_empty() {
        return Err(p.bad("`rows` is empty"));
    }
    if list.len() > fontcraft_fontinfo::MAX_ROWS {
        return Err(p.bad(format!("more than {} rows", fontcraft_fontinfo::MAX_ROWS)));
    }
    let read = |v: &Value| -> Result<RowRef> {
        let key = v.get("key").and_then(Value::as_str).ok_or_else(|| p.bad("each of `rows` has a `key`"))?;
        let kind = match v.get("kind").and_then(Value::as_str) {
            None => None,
            Some("field") => Some(RowKind::Field),
            Some("param") => Some(RowKind::Param),
            Some(other) => return Err(p.bad(format!("a row's `kind` is field or param, got `{other}`"))),
        };
        let index = match v.get("index") {
            None | Some(Value::Null) => 0,
            Some(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad("a row's `index` is a whole number from 0"))?,
        };
        Ok(RowRef { kind, key: key.to_string(), index })
    };
    list.iter().map(read).collect()
}

fn remove_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.removeRows", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = targets_of(&p, font, scope, "targets")?;
    let refs = row_refs(&p)?;
    let mut skipped: Vec<Value> = Vec::new();
    let mut wanted: Vec<&RowRef> = Vec::new();
    for r in &refs {
        if r.is(RowKind::Field) && fields::field(scope, &r.key).is_some_and(|f| f.built_in) {
            skipped.push(json!({"key": r.key, "reason": "a built-in field stays"}));
        } else {
            wanted.push(r);
        }
    }
    // Per target the places of its rows: fields, then parameters, each in stored order.
    let mut plan: Vec<(usize, Vec<usize>, Vec<usize>)> = Vec::with_capacity(targets.len());
    for &i in &targets {
        let (Some(props), Some(params)) = (properties(font, scope, i), params_of(font, scope, i)) else {
            return Err(p.bad(format!("there is no {} #{i}", scope.name())));
        };
        let (mut field_places, mut param_places) = (Vec::new(), Vec::new());
        for r in &wanted {
            let field = if r.is(RowKind::Field) { r.place(props.0.iter().map(|x| x.key.as_str())) } else { None };
            let param = if field.is_none() && r.is(RowKind::Param) { r.place(params.0.iter().map(|x| x.name.as_str())) } else { None };
            field_places.extend(field);
            param_places.extend(param);
        }
        for places in [&mut field_places, &mut param_places] {
            places.sort_unstable();
            places.dedup();
        }
        if !field_places.is_empty() || !param_places.is_empty() {
            plan.push((i, field_places, param_places));
        }
    }
    if plan.is_empty() {
        return Err(p.bad(if wanted.is_empty() { "built-in fields cannot be removed".to_string() } else { "no target has the rows".to_string() }));
    }
    let removed: usize = plan.iter().map(|(_, f, q)| f.len().saturating_add(q.len())).sum();
    s.edit("Remove Rows", |font| {
        for (i, field_places, param_places) in &plan {
            if !field_places.is_empty()
                && let Some(props) = properties_mut(font, scope, *i)
            {
                let mut at = 0usize;
                props.0.retain(|_| {
                    let keep = field_places.binary_search(&at).is_err();
                    at = at.saturating_add(1);
                    keep
                });
            }
            if !param_places.is_empty()
                && let Some(params) = params_mut(font, scope, *i)
            {
                let mut at = 0usize;
                params.0.retain(|_| {
                    let keep = param_places.binary_search(&at).is_err();
                    at = at.saturating_add(1);
                    keep
                });
            }
        }
        Ok(())
    })?;
    Ok(json!({ "removed": removed, "skipped": skipped }))
}

// ---------- the row clipboard ----------

fn copy_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.copyRows", p)?;
    let scope = scope_of(&p)?;
    let font = s.font()?;
    let targets = one_or_more(&p, font, scope)?;
    let by_ref = match (p.get("rows"), p.get("keys")) {
        (Some(_), Some(_)) => return Err(p.bad("give `rows` or `keys`, not both")),
        (Some(_), None) => Some(row_refs(&p)?),
        _ => None,
    };
    let i = match targets.as_slice() {
        [one, ..] if targets.len() == 1 || by_ref.is_some() => *one,
        _ => return Err(p.bad("several targets need `rows`")),
    };
    let rows: Vec<Row> = match p.opt_strings("keys")? {
        None if by_ref.is_some() => {
            // Each row comes from the first target that holds it.
            let all: Vec<(usize, Vec<Row>)> = targets.iter().map(|at| (*at, rows::rows_of(font, scope, *at))).collect();
            let mut out = Vec::new();
            for r in by_ref.iter().flatten() {
                let built_in = fields::field(scope, &r.key).filter(|f| f.built_in && r.is(RowKind::Field));
                let row = all.iter().find_map(|(at, held)| match built_in {
                    Some(f) => rows::builtin_row(font, scope, *at, f.key),
                    None => held.iter().filter(|row| r.is(row.kind) && row.key == r.key).nth(r.index).cloned(),
                });
                out.push(row.ok_or_else(|| p.bad(format!("the {} has no row `{}` #{}", scope.name(), r.key, r.index)))?);
            }
            out
        }
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
                    _ => {
                        report.skip(scope, i, &row.key, format!("`{}` takes a text or a number", spec.key));
                        return Ok(());
                    }
                };
                let mut ignored = Vec::new();
                // A built-in field checks its value before it writes anything.
                match set_field(font, scope, i, spec.key, &json, &mut ignored) {
                    Ok(()) => report.replaced += 1,
                    Err(reason) => report.skip(scope, i, &row.key, reason),
                }
                return Ok(());
            }
            let value = match rows::property_from_value(spec.key, &row.value, spec.localizable) {
                Ok(v) => v,
                Err(e) => {
                    report.skip(scope, i, &row.key, e.to_string());
                    return Ok(());
                }
            };
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
            // A name parameter the target holds no row of is its name property, as
            // font.customParameters.add and .set store it.
            if let Some((spec, localized)) = spec.and_then(|spec| catalog::property(spec).map(|l| (spec, l))) {
                let has_row = params_of(font, scope, i).is_some_and(|list| list.0.iter().any(|q| super::custom_params::same(&q.name, spec.name)));
                if !has_row {
                    if scope == Scope::Master {
                        report.skip(scope, i, &row.key, "a name property, which masters do not hold".into());
                        return Ok(());
                    }
                    let value = match rows::property_from_value(spec.name, &row.value, localized) {
                        Ok(v) => v,
                        Err(e) => {
                            report.skip(scope, i, &row.key, e.to_string());
                            return Ok(());
                        }
                    };
                    let props = properties(font, scope, i).ok_or_else(|| format!("there is no {} #{i}", scope.name()))?;
                    let present = props.get(spec.name).is_some();
                    if present && mode == PasteMode::Skip {
                        report.skip(scope, i, &row.key, "already there".into());
                        return Ok(());
                    }
                    if !present && props.0.len() >= fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
                        return Err(format!("the {} already has {} fields", scope.name(), fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE));
                    }
                    write_property(font, scope, i, spec.name, value)?;
                    if present {
                        report.replaced += 1;
                    } else {
                        report.added += 1;
                    }
                    return Ok(());
                }
            }
            // A value its check refuses (one stored before the check existed, say) is left out
            // and listed, so it does not keep the other rows out.
            let checked = match spec {
                Some(spec) => {
                    if param_value::check_scope(spec, scope).is_err() {
                        report.skip(scope, i, &row.key, format!("cannot be set on the {}", scope.name()));
                        return Ok(());
                    }
                    param_value::check(spec, &row.value).map(|v| (spec.name.to_string(), v, spec.repeatable)).map_err(|e| e.to_string())
                }
                None => {
                    param_value::check_limits(&row.key, &row.value).map(|()| (row.key.clone(), row.value.clone(), false)).map_err(|e| e.to_string())
                }
            };
            let (name, value, repeatable) = match checked {
                Ok(c) => c,
                Err(reason) => {
                    report.skip(scope, i, &row.key, reason);
                    return Ok(());
                }
            };
            let list = params_mut(font, scope, i).ok_or_else(|| format!("there is no {} #{i}", scope.name()))?;
            let existing = list.0.iter().position(|q| super::custom_params::same(&q.name, &name));
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
