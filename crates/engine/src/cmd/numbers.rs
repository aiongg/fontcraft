//! Number values: the font's named numbers and each master's values (M8.14).
//!
//! Commands: `font.numbers.list`, `add`, `rename`, `remove`, `font.master.setNumber`.
//!
//! The font holds the names ([`fontcraft_model::Font::numbers`]); each master holds its values in
//! the same order ([`fontcraft_model::Master::number_values`]), where a missing value counts as 0.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_model::{Font, MAX_COORDINATE, MAX_NAME_LEN, Master, NumberDef};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::Params;

/// Most named numbers one font may hold.
const MAX_NUMBERS: usize = 10_000;

/// The name `font.numbers.add` gives when none is passed (with a number appended while taken).
const NEW_NAME: &str = "value";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.numbers.list",
            "List Number Values",
            [],
            None,
            "{master?: id|name|index} the font's named numbers in order with every master's value (0 where a master has none) → {numbers: [{index, name, values: {masterId: number}, value?: the given master's value}]}",
            has_doc,
            list
        ),
        cmd!(
            "font.numbers.add",
            "Add Number Value",
            [],
            None,
            "{name?: string (letters, digits and `_`, not starting with a digit; a setting such as `.monospace` starts with `.`, and tokens do not read it), values?: {master id|name: number}} add a named number at the end; every master gets a value, 0 unless `values` gives one. Without a name it is called `value` (`value1`, `value2`… while taken); a name the font has is refused → {index, name, values: {masterId: number}}",
            has_doc,
            add
        ),
        cmd!(
            "font.numbers.rename",
            "Rename Number Value",
            [],
            None,
            "{number: index|name, name: string} rename a named number in all masters; tokens in feature code that use the old name are left as they are → {index, name}",
            has_doc,
            rename
        ),
        cmd!(
            "font.numbers.remove",
            "Remove Number Value",
            [],
            None,
            "{number: index|name} delete a named number and every master's value for it → {index, name}",
            has_doc,
            remove
        ),
        cmd!(
            "font.master.setNumber",
            "Set Master Number Value",
            [],
            None,
            "{masters: [id|name|index] (or one), number: index|name, value: number (finite, within ±1,000,000)} set the value the masters store for a named number → {number: {index, name}, masters: [ids], value}",
            has_doc,
            set_number
        ),
    ]
}

/// The master `v` names: its id, else its name, else its index.
fn master_index(p: &Params, font: &Font, key: &str, v: &Value) -> Result<usize> {
    match v {
        Value::String(s) => font
            .masters
            .iter()
            .position(|m| m.id == *s)
            .or_else(|| font.masters.iter().position(|m| m.name == *s))
            .ok_or_else(|| p.bad(format!("`{key}`: the font has no master `{s}`"))),
        Value::Number(n) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|&i| i < font.masters.len())
            .ok_or_else(|| p.bad(format!("`{key}`: the font has no master {n}"))),
        other => Err(p.bad(format!("`{key}` names a master by id, name or index, got {}", crate::params::kind(other)))),
    }
}

/// The named number `number` names: by index or by name.
fn number_index(p: &Params, font: &Font) -> Result<usize> {
    match p.get("number") {
        None => Err(p.bad("`number` is required")),
        Some(Value::String(name)) => {
            font.numbers.iter().position(|d| d.name == *name).ok_or_else(|| p.bad(format!("the font has no number value `{name}`")))
        }
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|&i| i < font.numbers.len())
            .ok_or_else(|| p.bad(format!("the font has no number value {n}"))),
        Some(other) => Err(p.bad(format!("`number` is an index or a name, got {}", crate::params::kind(other)))),
    }
}

/// `name` checked for a new or renamed number: valid, and not another number's (`except` is the
/// number being renamed).
fn check_name(p: &Params, font: &Font, name: &str, except: Option<usize>) -> Result<()> {
    // Tokens read `$name` up to the first character that is not a letter, digit or `_`. A
    // leading `.` marks a setting (`.monospace`), which no token reads.
    let bare = name.strip_prefix('.').unwrap_or(name);
    let first_ok = bare.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
    let rest_ok = bare.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !first_ok || !rest_ok || name.chars().count() > MAX_NAME_LEN {
        return Err(
            p.bad(format!("a number value name is 1 to {MAX_NAME_LEN} letters, digits and `_`, not starting with a digit, with an optional leading `.` for a setting, got `{name}`"))
        );
    }
    if font.numbers.iter().enumerate().any(|(i, d)| Some(i) != except && d.name == name) {
        return Err(p.bad(format!("the font already has a number value `{name}`")));
    }
    Ok(())
}

/// `value`, else `value1`, `value2`…: the first name no number of `font` has.
fn free_name(font: &Font) -> String {
    let taken: HashSet<&str> = font.numbers.iter().map(|d| d.name.as_str()).collect();
    if !taken.contains(NEW_NAME) {
        return NEW_NAME.to_string();
    }
    // Each taken name rules out at most one candidate, so one of these is free.
    (1..=taken.len().saturating_add(1))
        .map(|n| format!("{NEW_NAME}{n}"))
        .find(|name| !taken.contains(name.as_str()))
        .unwrap_or_else(|| format!("{NEW_NAME}{}", taken.len().saturating_add(1)))
}

fn check_value(p: &Params, key: &str, v: f64) -> Result<f64> {
    if !v.is_finite() || v.abs() > MAX_COORDINATE {
        return Err(p.bad(format!("`{key}` must be a number within ±{MAX_COORDINATE}, got {v}")));
    }
    Ok(v)
}

fn value_of(master: &Master, index: usize) -> f64 {
    master.number_values.get(index).copied().unwrap_or(0.0)
}

fn values_json(font: &Font, index: usize) -> Value {
    let map: Map<String, Value> = font.masters.iter().map(|m| (m.id.clone(), json!(value_of(m, index)))).collect();
    Value::Object(map)
}

fn list(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.numbers.list", raw)?;
    let font = s.font()?;
    let master = match p.get("master") {
        None => None,
        Some(v) => Some(master_index(&p, font, "master", v)?),
    };
    let numbers: Vec<Value> = font
        .numbers
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut entry = json!({"index": i, "name": d.name, "values": values_json(font, i)});
            if let (Some(m), Value::Object(map)) = (master.and_then(|m| font.masters.get(m)), &mut entry) {
                map.insert("value".into(), json!(value_of(m, i)));
            }
            entry
        })
        .collect();
    Ok(json!({ "numbers": numbers }))
}

fn add(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.numbers.add", raw)?;
    let font = s.font()?;
    if font.numbers.len() >= MAX_NUMBERS {
        return Err(p.bad(format!("the font already has {MAX_NUMBERS} number values")));
    }
    let name = match p.opt_str("name")? {
        Some(name) => {
            check_name(&p, font, name, None)?;
            name.to_string()
        }
        None => free_name(font),
    };
    let mut values = vec![0.0; font.masters.len()];
    if let Some(given) = p.opt_object("values")? {
        for (key, v) in given {
            let m = master_index(&p, font, "values", &Value::String(key.clone()))?;
            let n = v.as_f64().ok_or_else(|| p.bad(format!("`values.{key}` must be a number, got {}", crate::params::kind(v))))?;
            if let Some(slot) = values.get_mut(m) {
                *slot = check_value(&p, "values", n)?;
            }
        }
    }
    let index = font.numbers.len();
    s.edit("Add Number Value", |font| {
        Arc::make_mut(&mut font.numbers).push(NumberDef { name: name.clone(), ..NumberDef::default() });
        for (m, v) in Arc::make_mut(&mut font.masters).iter_mut().zip(&values) {
            m.number_values.resize(index, 0.0);
            m.number_values.push(*v);
        }
        Ok(())
    })?;
    Ok(json!({ "index": index, "name": name, "values": values_json(s.font()?, index) }))
}

fn rename(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.numbers.rename", raw)?;
    let font = s.font()?;
    let index = number_index(&p, font)?;
    let name = p.str("name")?.to_string();
    check_name(&p, font, &name, Some(index))?;
    s.edit("Rename Number Value", |font| {
        if let Some(d) = Arc::make_mut(&mut font.numbers).get_mut(index) {
            d.name = name.clone();
        }
        Ok(())
    })?;
    Ok(json!({ "index": index, "name": name }))
}

fn remove(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.numbers.remove", raw)?;
    let font = s.font()?;
    let index = number_index(&p, font)?;
    let name = font.numbers.get(index).map(|d| d.name.clone()).unwrap_or_default();
    s.edit("Remove Number Value", |font| {
        let numbers = Arc::make_mut(&mut font.numbers);
        if index < numbers.len() {
            numbers.remove(index);
        }
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            if index < m.number_values.len() {
                m.number_values.remove(index);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "index": index, "name": name }))
}

fn set_number(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.master.setNumber", raw)?;
    let font = s.font()?;
    let index = number_index(&p, font)?;
    let value = check_value(&p, "value", p.f64("value")?)?;
    let given: Vec<&Value> = match p.get("masters") {
        None => return Err(p.bad("`masters` is required")),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(one) => vec![one],
    };
    if given.is_empty() {
        return Err(p.bad("`masters` is empty"));
    }
    if given.len() > font.masters.len() {
        return Err(p.bad(format!("`masters` names {} masters; the font has {}", given.len(), font.masters.len())));
    }
    let mut masters = Vec::new();
    for v in given {
        let m = master_index(&p, font, "masters", v)?;
        if !masters.contains(&m) {
            masters.push(m);
        }
    }
    let ids: Vec<String> = masters.iter().filter_map(|&m| font.masters.get(m).map(|m| m.id.clone())).collect();
    let name = font.numbers.get(index).map(|d| d.name.clone()).unwrap_or_default();
    s.edit("Set Master Number Value", |font| {
        let all = Arc::make_mut(&mut font.masters);
        for &m in &masters {
            if let Some(master) = all.get_mut(m) {
                if master.number_values.len() <= index {
                    master.number_values.resize(index + 1, 0.0);
                }
                if let Some(slot) = master.number_values.get_mut(index) {
                    *slot = value;
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "number": {"index": index, "name": name}, "masters": ids, "value": value }))
}
