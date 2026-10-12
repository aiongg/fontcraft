//! Custom parameters at the font, master and instance scopes: `font.customParameters.list`, `.describe`, `.add`, `.set` and `.remove`, checked through `fontcraft_fontinfo::catalog` and `fontcraft_fontinfo::value`; several targets in one undo step.
//!
//! Rules (`docs/plans/M7.md` § 3, rows in `spec/inventory/06-font-info.toml` and
//! `18-custom-parameters.toml`):
//! - A known parameter's value is checked against its kind and syntax and stored normalised; a
//!   parameter whose entry names its scopes cannot be added elsewhere (rows read from files stay editable). A name the catalogue does not
//!   know is kept as given, within the size limits, and listed as unknown.
//! - A name property (`designers`, `versionString`…) added here is written to the target's
//!   properties, not to its parameter list. Masters hold no name properties.
//! - Another spelling of a known parameter (`openTypeOS2TypoAscender`) is stored under the
//!   catalogue's name, and `set` and `remove` find rows by either spelling.
//! - A name may repeat; for a parameter that takes one value the first enabled row counts and
//!   `list` marks the others as duplicates. `index` picks one of the rows with the same name.
//! - Every change to several targets is one undo step; a refusal changes nothing.

use std::sync::Arc;

use fontcraft_fontinfo::catalog::{self, Effect, ParamSpec};
use fontcraft_fontinfo::value as fv;
use fontcraft_fontinfo::{Scope, target};
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{Extra, Font, Localized, Param, Params, Properties, PropertyValue};
use serde_json::{Value, json};

use super::*;
use crate::params::Params as Args;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.customParameters.list",
            "List Custom Parameters",
            [],
            None,
            "{scope: \"font\"|\"master\"|\"instance\", target?: id|name|index (needed when the font has more than one master or instance)} the custom parameters of the font, of one master or of one export, in row order → {scope, target?: {index, id, name}, parameters: [{row, name, index (among the rows of this name), value, enabled, known, canonical? (the catalogue's spelling when the row uses another), kind?, duplicate? (another row of a parameter that takes one value counts instead)}]}",
            has_doc,
            list
        ),
        cmd!(
            query "font.customParameters.describe",
            "Describe Custom Parameters",
            [],
            None,
            "{scope?: \"font\"|\"master\"|\"instance\", query?: string} search the catalogue of known custom parameters: names and other spellings first, then descriptions, any case; with scope, only the parameters allowed there → {count, parameters: [{name, aliases, kind: \"bool\"|\"number\"|\"string\"|\"list\"|\"dict\"|\"localized\", scopes: [scope], scopesStated, effect, milestone, units, repeatable, property (a name property: written to the General fields), summary}]}",
            always,
            describe
        ),
        cmd!(
            "font.customParameters.add",
            "Add Custom Parameter",
            [],
            None,
            "{scope: \"font\"|\"master\"|\"instance\", targets?: id|name|index or a list of them (needed when the font has more than one master or instance), name: string, value?: any} add a parameter row at the end of each target's list, as one undo step. A known parameter's value is checked against its kind and syntax and stored normalised (numbers in text read as numbers, a text for a list is split into lines); without value it gets its kind's empty value. Another spelling is stored under the catalogue's name. A name property (designers, versionString…) is written to the target's properties instead and refused on masters and where it is already set. An unknown name is kept as given and needs a value → {scope, targets, name, property, value}",
            has_doc,
            add
        ),
        cmd!(
            "font.customParameters.set",
            "Set Custom Parameter",
            [],
            None,
            "{scope: \"font\"|\"master\"|\"instance\", targets?: id|name|index or a list, name: string, index?: integer (which of the rows called name, from 0; needed when there are several), value?: any, enabled?: bool} change the value and/or the checkbox of an existing row on each target, as one undo step; the value is checked as for add. A name property that is not a row changes the property's value → {scope, targets, name, property, value?, enabled?}",
            has_doc,
            set
        ),
        cmd!(
            "font.customParameters.remove",
            "Remove Custom Parameter",
            [],
            None,
            "{scope: \"font\"|\"master\"|\"instance\", targets?: id|name|index or a list, name: string, index?: integer (which of the rows called name, from 0; needed when there are several)} delete one row from each target, as one undo step; a name property that is not a row is removed from the properties → {scope, targets, name, property}",
            has_doc,
            remove
        ),
    ]
}

// ---------- scope and targets ----------

fn scope(p: &Args) -> Result<Scope> {
    let name = p.str("scope")?;
    Scope::from_name(name).ok_or_else(|| p.bad(format!("`scope` is font, master or instance, not `{name}`")))
}

fn count(font: &Font, scope: Scope) -> usize {
    match scope {
        Scope::Font => 1,
        Scope::Master => font.masters.len(),
        Scope::Instance => font.instances.len(),
    }
}

/// The indexes `key` names at `scope` (`[0]` for the font). Without `key`, the only master or
/// instance; a font with several needs one named.
fn targets(p: &Args, font: &Font, scope: Scope, key: &str) -> Result<Vec<usize>> {
    if scope == Scope::Font {
        return Ok(vec![0]);
    }
    let Some(v) = p.get(key) else {
        return match count(font, scope) {
            1 => Ok(vec![0]),
            0 => Err(p.bad(format!("the font has no {}", scope.name()))),
            n => Err(p.bad(format!("the font has {n} {}s: give `{key}`", scope.name()))),
        };
    };
    let refs = target::parse_list(v).map_err(|e| p.bad(e.to_string()))?;
    let found = match scope {
        Scope::Master => target::masters(font, &refs),
        _ => target::instances(font, &refs),
    };
    let found = found.map_err(|e| p.bad(e.to_string()))?;
    if found.is_empty() {
        return Err(p.bad(format!("`{key}` names no {}", scope.name())));
    }
    Ok(found)
}

/// `{index, id, name}` of a master or instance, for results.
fn describe_target(font: &Font, scope: Scope, i: usize) -> Value {
    match scope {
        Scope::Font => json!({"index": 0, "id": null, "name": font.family_name}),
        Scope::Master => font.masters.get(i).map_or(Value::Null, |m| json!({"index": i, "id": m.id, "name": m.name})),
        Scope::Instance => font.instances.get(i).map_or(Value::Null, |m| json!({"index": i, "id": m.id, "name": m.name})),
    }
}

/// The parameters and properties of one target.
fn view(font: &Font, scope: Scope, i: usize) -> Option<(&Params, &Properties)> {
    match scope {
        Scope::Font => Some((&font.info.params, &font.info.properties)),
        Scope::Master => font.masters.get(i).map(|m| (&m.params, &m.properties)),
        Scope::Instance => font.instances.get(i).map(|m| (&m.params, &m.properties)),
    }
}

/// The parameters and properties of one target, to change.
fn slot<'f>(p: &Args, font: &'f mut Font, scope: Scope, i: usize) -> Result<(&'f mut Params, &'f mut Properties)> {
    let missing = || p.bad(format!("there is no {} #{i}", scope.name()));
    match scope {
        Scope::Font => {
            let info = Arc::make_mut(&mut font.info);
            Ok((&mut info.params, &mut info.properties))
        }
        Scope::Master => {
            let m = Arc::make_mut(&mut font.masters).get_mut(i).ok_or_else(missing)?;
            Ok((&mut m.params, &mut m.properties))
        }
        Scope::Instance => {
            let m = Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(missing)?;
            Ok((&mut m.params, &mut m.properties))
        }
    }
}

fn where_(scope: Scope, font: &Font, i: usize) -> String {
    match scope {
        Scope::Font => "the font".into(),
        Scope::Master => font.masters.get(i).map_or_else(|| format!("master #{i}"), |m| format!("master `{}`", m.name)),
        Scope::Instance => font.instances.get(i).map_or_else(|| format!("instance #{i}"), |m| format!("instance `{}`", m.name)),
    }
}

// ---------- names and rows ----------

/// Whether a row called `row` is the parameter `name`: the same spelling, or two spellings of one
/// known parameter.
pub(super) fn same(row: &str, name: &str) -> bool {
    row == name || catalog::canonical(row).is_some_and(|c| Some(c) == catalog::canonical(name))
}

/// The positions of the rows called `name` (any spelling).
fn rows_of(params: &Params, name: &str) -> Vec<usize> {
    params.0.iter().enumerate().filter(|(_, p)| same(&p.name, name)).map(|(i, _)| i).collect()
}

/// The one row `name` and `index` pick, or why none.
fn pick(p: &Args, params: &Params, name: &str, at: &str) -> Result<usize> {
    let rows = rows_of(params, name);
    match p.opt_int("index", 0, i64::from(u32::MAX))? {
        Some(n) => {
            rows.get(n as usize).copied().ok_or_else(|| p.bad(format!("{at} has {} row(s) called `{name}`; `index` {n} is past them", rows.len())))
        }
        None => match rows.as_slice() {
            [one] => Ok(*one),
            [] => Err(p.bad(format!("{at} has no `{name}` parameter"))),
            many => Err(p.bad(format!("{at} has {} rows called `{name}`: give `index` (0 to {})", many.len(), many.len() - 1))),
        },
    }
}

/// The value to store for `name`: checked against its entry, or only against the limits.
fn value_for(p: &Args, spec: Option<&ParamSpec>, name: &str, json: Option<&Value>) -> Result<Plist> {
    let bad = |e: fontcraft_fontinfo::ValueError| p.bad(e.to_string());
    match (spec, json) {
        (Some(spec), Some(j)) => fv::check(spec, &fv::from_json(spec.name, j).map_err(bad)?).map_err(bad),
        (Some(spec), None) => Ok(fv::empty(spec.kind)),
        (None, Some(j)) => fv::from_json(name, j).map_err(bad),
        (None, None) => Err(p.bad(format!("`{name}` is not a known parameter: give its `value`"))),
    }
}

/// A checked value as a property's value.
fn property_value(p: &Args, name: &str, localized: bool, value: &Plist) -> Result<PropertyValue> {
    if localized {
        let pairs = fv::localized(value).ok_or_else(|| p.bad(format!("`{name}` takes a text per language")))?;
        let pairs = if pairs.is_empty() { vec![(fontcraft_model::DEFAULT_LANGUAGE.to_string(), String::new())] } else { pairs };
        return Ok(PropertyValue::Localized(pairs.into_iter().map(|(language, value)| Localized { language, value, extra: Extra::new() }).collect()));
    }
    let text = match value {
        Plist::String(s) => s.as_str().to_string(),
        Plist::Number(_) => fv::number(value).map(|n| n.to_string()).unwrap_or_default(),
        _ => return Err(p.bad(format!("`{name}` takes a text"))),
    };
    if text.len() > fontcraft_fontinfo::MAX_FIELD_BYTES {
        return Err(p.bad(format!("`{name}` holds at most {} bytes", fontcraft_fontinfo::MAX_FIELD_BYTES)));
    }
    Ok(PropertyValue::Single(text))
}

fn property_json(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Single(s) => json!(s),
        PropertyValue::Localized(v) => Value::Array(v.iter().map(|l| json!({"language": l.language, "value": l.value})).collect()),
    }
}

fn kind_name(spec: &ParamSpec) -> &'static str {
    use fontcraft_fontinfo::ParamKind as K;
    match spec.kind {
        K::Bool => "bool",
        K::Number => "number",
        K::String => "string",
        K::List => "list",
        K::Dict => "dict",
        K::Localized => "localized",
    }
}

fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::TableValue => "table-value",
        Effect::ExportFilter => "export-filter",
        Effect::Naming => "naming",
        Effect::Interpolation => "interpolation",
        Effect::Hinting => "hinting",
        Effect::Features => "features",
        Effect::Ui => "ui",
        Effect::Other => "other",
    }
}

// ---------- commands ----------

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Args::new("font.customParameters.list", p)?;
    let scope = scope(&p)?;
    let font = s.font()?;
    let i = targets(&p, font, scope, "target")?.first().copied().unwrap_or(0);
    let (params, _) = view(font, scope, i).ok_or_else(|| p.bad(format!("there is no {} #{i}", scope.name())))?;
    // The canonical names whose first enabled row has been seen, for the duplicate mark.
    let mut counted: Vec<&str> = Vec::new();
    let rows: Vec<Value> = params
        .0
        .iter()
        .enumerate()
        .map(|(row, param)| {
            let spec = catalog::lookup(&param.name);
            let index = params.0.iter().take(row).filter(|q| same(&q.name, &param.name)).count();
            let mut out = json!({
                "row": row,
                "name": param.name,
                "index": index,
                "value": fv::to_json(&param.value),
                "enabled": !param.disabled,
                "known": spec.is_some(),
            });
            if let (Some(spec), Some(obj)) = (spec, out.as_object_mut()) {
                if spec.name != param.name {
                    obj.insert("canonical".into(), json!(spec.name));
                }
                obj.insert("kind".into(), json!(kind_name(spec)));
                if !spec.repeatable && !param.disabled {
                    let seen = counted.contains(&spec.name);
                    obj.insert("duplicate".into(), json!(seen));
                    if !seen {
                        counted.push(spec.name);
                    }
                }
            }
            out
        })
        .collect();
    let mut out = json!({"scope": scope.name(), "parameters": rows});
    if scope != Scope::Font
        && let Some(obj) = out.as_object_mut()
    {
        obj.insert("target".into(), describe_target(font, scope, i));
    }
    Ok(out)
}

fn describe(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Args::new("font.customParameters.describe", p)?;
    let scope = match p.opt_str("scope")? {
        Some(name) => Some(Scope::from_name(name).ok_or_else(|| p.bad(format!("`scope` is font, master or instance, not `{name}`")))?),
        None => None,
    };
    let query = p.opt_str("query")?.unwrap_or_default();
    let found: Vec<Value> = catalog::search(query, scope)
        .into_iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "aliases": spec.aliases,
                "kind": kind_name(spec),
                "scopes": Scope::ALL.iter().filter(|s| spec.scopes.allows(**s)).map(|s| s.name()).collect::<Vec<_>>(),
                "scopesStated": !spec.scopes.is_any(),
                "effect": effect_name(spec.effect),
                "milestone": spec.milestone,
                "units": spec.units,
                "repeatable": spec.repeatable,
                "property": catalog::property(spec).is_some(),
                "summary": spec.summary,
            })
        })
        .collect();
    Ok(json!({"count": found.len(), "parameters": found}))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Args::new("font.customParameters.add", p)?;
    let scope = scope(&p)?;
    let given = p.str("name")?;
    if given.trim().is_empty() {
        return Err(p.bad("`name` is empty"));
    }
    let spec = catalog::lookup(given);
    let name = spec.map_or(given, |s| s.name);
    let value = value_for(&p, spec, name, p.get("value"))?;
    let property = spec.and_then(catalog::property);
    if property.is_some() && scope == Scope::Master {
        return Err(p.bad(format!("`{name}` is a name property, which masters do not hold")));
    }
    if let (Some(spec), None) = (spec, property) {
        fv::check_scope(spec, scope).map_err(|e| p.bad(e.to_string()))?;
    }
    let prop_value = property.map(|localized| property_value(&p, name, localized, &value)).transpose()?;
    let indexes = targets(&p, s.font()?, scope, "targets")?;
    s.edit("Add Custom Parameter", |font| {
        for &i in &indexes {
            let at = where_(scope, font, i);
            let (params, properties) = slot(&p, font, scope, i)?;
            match &prop_value {
                Some(pv) => {
                    if properties.get(name).is_some() {
                        return Err(p.bad(format!("{at} already has `{name}`: change it with font.customParameters.set")));
                    }
                    if properties.0.len() >= fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE {
                        return Err(p.bad(format!("{at} holds at most {} properties", fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE)));
                    }
                    properties.0.push(fontcraft_model::Property { key: name.to_string(), value: pv.clone(), extra: Extra::new() });
                }
                None => {
                    if params.0.len() >= fontcraft_fontinfo::MAX_PARAMS_PER_SCOPE {
                        return Err(p.bad(format!("{at} holds at most {} custom parameters", fontcraft_fontinfo::MAX_PARAMS_PER_SCOPE)));
                    }
                    params.0.push(Param::new(name, value.clone()));
                }
            }
        }
        Ok(())
    })?;
    let font = s.font()?;
    Ok(json!({
        "scope": scope.name(),
        "targets": indexes.iter().map(|i| describe_target(font, scope, *i)).collect::<Vec<_>>(),
        "name": name,
        "property": property.is_some(),
        "value": prop_value.as_ref().map_or_else(|| fv::to_json(&value), property_json),
    }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Args::new("font.customParameters.set", p)?;
    let scope = scope(&p)?;
    let name = p.str("name")?;
    let json_value = p.get("value");
    let enabled = match p.get("enabled") {
        None => None,
        Some(_) => Some(p.bool_or("enabled", true)?),
    };
    if json_value.is_none() && enabled.is_none() {
        return Err(p.bad("give `value`, `enabled` or both"));
    }
    let spec = catalog::lookup(name);
    let value = json_value.map(|j| value_for(&p, spec, name, Some(j))).transpose()?;
    let indexes = targets(&p, s.font()?, scope, "targets")?;
    let mut property = false;
    s.edit("Set Custom Parameter", |font| {
        for &i in &indexes {
            let at = where_(scope, font, i);
            let (params, properties) = slot(&p, font, scope, i)?;
            let as_property = spec.and_then(catalog::property).filter(|_| rows_of(params, name).is_empty());
            if let (Some(localized), Some(spec)) = (as_property, spec) {
                property = true;
                if enabled == Some(false) {
                    return Err(p.bad(format!("`{}` is a name property, which has no checkbox", spec.name)));
                }
                let Some(value) = &value else { continue };
                let pv = property_value(&p, spec.name, localized, value)?;
                let entry = properties
                    .0
                    .iter_mut()
                    .find(|q| q.key == spec.name)
                    .ok_or_else(|| p.bad(format!("{at} has no `{}`: add it first", spec.name)))?;
                entry.value = pv;
                continue;
            }
            let row = pick(&p, params, name, &at)?;
            let param = params.0.get_mut(row).ok_or_else(|| p.bad(format!("{at} has no `{name}` parameter")))?;
            if let Some(v) = &value {
                param.value = v.clone();
            }
            if let Some(on) = enabled {
                param.disabled = !on;
            }
        }
        Ok(())
    })?;
    let font = s.font()?;
    let mut out = json!({
        "scope": scope.name(),
        "targets": indexes.iter().map(|i| describe_target(font, scope, *i)).collect::<Vec<_>>(),
        "name": spec.map_or(name, |s| s.name),
        "property": property,
    });
    if let Some(obj) = out.as_object_mut() {
        if let Some(v) = &value {
            obj.insert("value".into(), fv::to_json(v));
        }
        if let Some(on) = enabled {
            obj.insert("enabled".into(), json!(on));
        }
    }
    Ok(out)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Args::new("font.customParameters.remove", p)?;
    let scope = scope(&p)?;
    let name = p.str("name")?;
    let spec = catalog::lookup(name);
    let indexes = targets(&p, s.font()?, scope, "targets")?;
    let mut property = false;
    s.edit("Remove Custom Parameter", |font| {
        for &i in &indexes {
            let at = where_(scope, font, i);
            let (params, properties) = slot(&p, font, scope, i)?;
            let as_property = spec.filter(|sp| catalog::property(sp).is_some() && rows_of(params, name).is_empty());
            if let Some(spec) = as_property {
                property = true;
                if !properties.remove(spec.name) {
                    return Err(p.bad(format!("{at} has no `{}`", spec.name)));
                }
                continue;
            }
            let row = pick(&p, params, name, &at)?;
            if row < params.0.len() {
                params.0.remove(row);
            }
        }
        Ok(())
    })?;
    let font = s.font()?;
    Ok(json!({
        "scope": scope.name(),
        "targets": indexes.iter().map(|i| describe_target(font, scope, *i)).collect::<Vec<_>>(),
        "name": spec.map_or(name, |s| s.name),
        "property": property,
    }))
}
