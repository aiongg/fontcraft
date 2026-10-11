//! The instance parameters that change the feature code of an export (M8.3).
//!
//! Rows: `fontInfo.features.exportSpecific`, `Add Class`, `Add Feature`, `Add Prefix`,
//! `Remove Classes`, `Remove Features`, `Remove Prefixes`, `Replace Class`, `Replace Feature`,
//! `Replace Prefix`, `Optical Size` (the `size` feature).
//!
//! The values follow the parameters' documentation and glyphsLib's reading of them:
//!
//! | Parameter | Value | Effect |
//! |---|---|---|
//! | Add Class, Add Prefix, Add Feature | `name; code` | a new entry at the end of its list |
//! | Replace Class, Replace Prefix, Replace Feature | `name; code` | the code of the first entry of that name; nothing when there is none |
//! | Remove Classes, Remove Prefixes, Remove Features | a list of names | every entry of those names left out |
//! | Optical Size | `design; subfamily; start; end; name` | the code of the `size` feature |
//!
//! The name is everything before the first semicolon, trimmed; the code everything after it,
//! with the white space after the semicolon dropped. A class name may be written with or without
//! its `@`. A replaced entry is no longer automatic: its code is the parameter's.
//!
//! The `name; code` split follows glyphsLib's custom parameter builder
//! (`builder/custom_params.py`, Apache-2.0).

use std::collections::HashSet;

use fontcraft_model::plist::Value;
use fontcraft_model::{Features, Params, Snippet};

use crate::FeatureError;

pub const ADD_CLASS: &str = "Add Class";
pub const ADD_FEATURE: &str = "Add Feature";
pub const ADD_PREFIX: &str = "Add Prefix";
pub const REMOVE_CLASSES: &str = "Remove Classes";
pub const REMOVE_FEATURES: &str = "Remove Features";
pub const REMOVE_PREFIXES: &str = "Remove Prefixes";
pub const REPLACE_CLASS: &str = "Replace Class";
pub const REPLACE_FEATURE: &str = "Replace Feature";
pub const REPLACE_PREFIX: &str = "Replace Prefix";
pub const OPTICAL_SIZE: &str = "Optical Size";

/// Most names one Remove parameter may list.
pub const MAX_REMOVED_NAMES: usize = 100_000;

/// Which list of entries a parameter acts on.
#[derive(Clone, Copy)]
enum List {
    Prefixes,
    Classes,
    Features,
}

impl List {
    fn of(self, entries: &mut Features) -> &mut Vec<Snippet> {
        match self {
            List::Prefixes => &mut entries.prefixes,
            List::Classes => &mut entries.classes,
            List::Features => &mut entries.features,
        }
    }

    /// Whether `entry` is called `name` in this list: class names compare without their `@`.
    fn named(self, entry: &Snippet, name: &str) -> bool {
        match self {
            List::Classes => strip_at(&entry.name) == strip_at(name),
            List::Prefixes | List::Features => entry.name == name,
        }
    }
}

fn strip_at(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}

/// `entries` with the instance parameters of `params` applied, in parameter order, and the
/// `Optical Size` parameter's `size` feature. Disabled parameters are skipped. A malformed
/// parameter value is an error naming the parameter; the entries are untouched then.
pub fn apply_instance_params(entries: &Features, params: &Params) -> Result<Features, FeatureError> {
    let mut out = entries.clone();
    for param in params.0.iter().filter(|p| !p.disabled) {
        let name = param.name.as_str();
        match name {
            ADD_CLASS => add(&mut out, List::Classes, name, &param.value)?,
            ADD_PREFIX => add(&mut out, List::Prefixes, name, &param.value)?,
            ADD_FEATURE => add(&mut out, List::Features, name, &param.value)?,
            REPLACE_CLASS => replace(&mut out, List::Classes, name, &param.value)?,
            REPLACE_PREFIX => replace(&mut out, List::Prefixes, name, &param.value)?,
            REPLACE_FEATURE => replace(&mut out, List::Features, name, &param.value)?,
            REMOVE_CLASSES => remove(&mut out, List::Classes, name, &param.value)?,
            REMOVE_PREFIXES => remove(&mut out, List::Prefixes, name, &param.value)?,
            REMOVE_FEATURES => remove(&mut out, List::Features, name, &param.value)?,
            OPTICAL_SIZE => {
                let code = size_feature(&param.value)?;
                match out.features.iter_mut().find(|f| f.name == "size") {
                    Some(entry) => {
                        entry.code = code;
                        entry.automatic = false;
                    }
                    None => out.features.push(Snippet { name: "size".into(), code, ..Snippet::default() }),
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn malformed(param: &str, what: &str) -> FeatureError {
    FeatureError::Unsupported(format!("the `{param}` parameter {what}"))
}

/// The name and the code of a `name; code` value.
fn name_and_code<'a>(param: &str, value: &'a Value) -> Result<(&'a str, &'a str), FeatureError> {
    let text = value.as_str().ok_or_else(|| malformed(param, "needs a text value: a name, a semicolon and the code"))?;
    let (name, code) = text.split_once(';').ok_or_else(|| malformed(param, "needs a semicolon after the name"))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(malformed(param, "needs a name before the semicolon"));
    }
    Ok((name, code.trim_start()))
}

fn add(entries: &mut Features, list: List, param: &str, value: &Value) -> Result<(), FeatureError> {
    let (name, code) = name_and_code(param, value)?;
    let name = match list {
        List::Classes => strip_at(name),
        List::Prefixes | List::Features => name,
    };
    list.of(entries).push(Snippet { name: name.to_string(), code: code.to_string(), ..Snippet::default() });
    Ok(())
}

fn replace(entries: &mut Features, list: List, param: &str, value: &Value) -> Result<(), FeatureError> {
    let (name, code) = name_and_code(param, value)?;
    if let Some(entry) = list.of(entries).iter_mut().find(|e| list.named(e, name)) {
        entry.code = code.to_string();
        entry.automatic = false;
    }
    Ok(())
}

fn remove(entries: &mut Features, list: List, param: &str, value: &Value) -> Result<(), FeatureError> {
    let names = name_list(param, value)?;
    let key = |name: &str| -> String {
        match list {
            List::Classes => strip_at(name).to_string(),
            List::Prefixes | List::Features => name.to_string(),
        }
    };
    let names: HashSet<String> = names.iter().map(|n| key(n)).collect();
    list.of(entries).retain(|e| !names.contains(&key(&e.name)));
    Ok(())
}

/// The names of a list parameter: an array of names, or one text with the names separated by
/// commas or line breaks. Names are trimmed; empty ones are skipped.
fn name_list(param: &str, value: &Value) -> Result<Vec<String>, FeatureError> {
    let texts: Vec<&str> = match value {
        Value::Array(items) => items
            .items
            .iter()
            .map(|item| item.as_str().ok_or_else(|| malformed(param, "lists something that is not a name")))
            .collect::<Result<_, _>>()?,
        Value::String(text) => text.as_str().split([',', '\n', '\r']).collect(),
        _ => return Err(malformed(param, "needs a list of names")),
    };
    let names: Vec<String> = texts.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).map(str::to_string).collect();
    if names.len() > MAX_REMOVED_NAMES {
        return Err(malformed(param, &format!("lists more than {MAX_REMOVED_NAMES} names")));
    }
    Ok(names)
}

/// The `size` feature's code for an `Optical Size` value: `parameters` with the design size,
/// the subfamily and the range in decipoints, and the menu name for Windows and Mac.
fn size_feature(value: &Value) -> Result<String, FeatureError> {
    const WHAT: &str = "needs five values separated by semicolons: design size, subfamily, range start, range end (whole decipoints from 0 to 65535) and a menu name";
    let text = value.as_str().ok_or_else(|| malformed(OPTICAL_SIZE, WHAT))?;
    let mut parts = text.splitn(5, ';').map(str::trim);
    let mut number =
        || -> Result<u16, FeatureError> { parts.next().and_then(|p| p.parse::<u16>().ok()).ok_or_else(|| malformed(OPTICAL_SIZE, WHAT)) };
    let (design, subfamily, start, end) = (number()?, number()?, number()?, number()?);
    let name = parts.next().ok_or_else(|| malformed(OPTICAL_SIZE, WHAT))?;
    // The feature file specification: a subfamily of 0 has no range and no menu name.
    if subfamily == 0 {
        return Ok(format!("parameters {design} 0 0 0;"));
    }
    let mut code = format!("parameters {design} {subfamily} {start} {end};");
    if !name.is_empty() {
        code.push_str(&format!("\nsizemenuname 3 1 0x409 \"{}\";", windows_string(name)));
        if let Some(mac) = mac_string(name) {
            code.push_str(&format!("\nsizemenuname 1 0 0 \"{mac}\";"));
        }
    }
    Ok(code)
}

/// A string for a Windows name record: `"`, `\` and everything outside printable ASCII as
/// `\XXXX` UTF-16 code units.
fn windows_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\') {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\{unit:04x}"));
            }
        }
    }
    out
}

/// A string for a Mac name record (Mac Roman), or `None` when the text has characters outside
/// printable ASCII, which the Mac record is left out for.
fn mac_string(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\22"),
            '\\' => out.push_str("\\5c"),
            ' ' => out.push(' '),
            c if c.is_ascii_graphic() => out.push(c),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "params_tests.rs"]
mod tests;
