//! Status definitions and glyph status: the font's agreed list of development statuses (each a
//! code, a name, a colour and at most one default) and the status each glyph is in.
//!
//! | Data | Where it is kept |
//! |---|---|
//! | the definitions | the font's user data, [`fc_keys::STATUS_DEFINITIONS`]: an array of `{code, color, isDefault?, name}` |
//! | a glyph's status | the glyph's user data, [`fc_keys::GLYPH_STATUS`]: the code, an integer |
//!
//! `.glyphs` and UFO both keep these keys. A glyph without a stored status is in the default
//! status, when one is defined. Redefining the list keeps every glyph's stored code, even one the
//! new list no longer defines; such a glyph shows no tint until the code is defined again.

use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Num, Number, Str, Value as PValue};
use fontcraft_model::{Font, MAX_NAME_LEN, fc_keys};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{Params, TargetKind, kind};

/// Most status definitions a font holds.
pub const MAX_STATUS_DEFINITIONS: usize = 64;
/// Largest status code.
pub const MAX_STATUS_CODE: i64 = 1_000_000;

const GLYPHS: &[TargetKind] = &[TargetKind::Glyphs];

/// One status definition.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusDef {
    pub code: i64,
    pub name: String,
    /// Red, green, blue and alpha, each from 0 to 1, in sRGB.
    pub color: [f64; 4],
    pub is_default: bool,
}

impl StatusDef {
    fn to_json(&self) -> Value {
        json!({"code": self.code, "name": self.name, "color": self.color, "isDefault": self.is_default})
    }

    /// The stored form, with keys in sorted order so writers that sort keys keep it unchanged.
    fn to_plist(&self) -> PValue {
        let mut d = Dict::new();
        d.insert("code", PValue::Number(Number::int(self.code)));
        let items = self.color.iter().filter_map(|c| Number::float(*c)).map(PValue::Number).collect();
        d.insert("color", PValue::Array(Array { items, ..Default::default() }));
        // A number, not a boolean: the `.glyphs` text format has no booleans and reads `1` back
        // as a number.
        if self.is_default {
            d.insert("isDefault", PValue::Number(Number::int(1)));
        }
        d.insert("name", PValue::String(Str::new(self.name.as_str())));
        PValue::Dict(d)
    }

    /// Reads one stored definition; `None` when it is malformed.
    fn from_plist(v: &PValue) -> Option<Self> {
        let d = v.as_dict()?;
        let code = whole(d.get("code")?).filter(|c| (0..=MAX_STATUS_CODE).contains(c))?;
        let name = d.get("name")?.as_str()?.trim();
        if name.is_empty() || name.len() > MAX_NAME_LEN {
            return None;
        }
        let parts: Vec<f64> = d.get("color")?.as_array()?.items.iter().take(5).map(|c| c.as_num().map(Num::as_f64)).collect::<Option<_>>()?;
        let color = rgba(&parts)?;
        let is_default = match d.get("isDefault") {
            Some(PValue::Bool(b)) => *b,
            Some(other) => other.as_num().is_some_and(|n| n.as_f64() != 0.0),
            None => false,
        };
        Some(Self { code, name: name.to_string(), color, is_default })
    }
}

/// A whole number from a plist value: an integer, an integral float or digits as text.
fn whole(v: &PValue) -> Option<i64> {
    match v {
        PValue::Number(n) => match n.value() {
            Num::Int(i) => Some(i),
            Num::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() <= MAX_STATUS_CODE as f64 => Some(f as i64),
            Num::Float(_) => None,
        },
        PValue::String(s) => s.as_str().trim().parse().ok(),
        _ => None,
    }
}

/// Three or four components from 0 to 1 as RGBA; a missing alpha is 1.
fn rgba(parts: &[f64]) -> Option<[f64; 4]> {
    if !(3..=4).contains(&parts.len()) || !parts.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)) {
        return None;
    }
    let at = |i: usize| parts.get(i).copied();
    Some([at(0)?, at(1)?, at(2)?, at(3).unwrap_or(1.0)])
}

/// The font's status definitions in stored order. Malformed entries are skipped, and so are
/// entries past [`MAX_STATUS_DEFINITIONS`] and repeated codes.
pub fn definitions(font: &Font) -> Vec<StatusDef> {
    let Some(PValue::Array(a)) = font.info.user_data.get(fc_keys::STATUS_DEFINITIONS) else { return Vec::new() };
    let mut out: Vec<StatusDef> = Vec::new();
    for def in a.items.iter().filter_map(StatusDef::from_plist) {
        if out.len() >= MAX_STATUS_DEFINITIONS {
            break;
        }
        if !out.iter().any(|d| d.code == def.code) {
            out.push(def);
        }
    }
    out
}

/// The status code stored on a glyph, if any.
pub fn stored_status(font: &Font, glyph: &str) -> Option<i64> {
    font.glyph(glyph)?.info.user_data.get(fc_keys::GLYPH_STATUS).and_then(whole)
}

/// The definition a glyph is in: its stored status, or the default status when it has none. A
/// stored code the font does not define gives `None`.
pub fn effective_status(font: &Font, glyph: &str) -> Option<StatusDef> {
    font.glyph(glyph)?;
    let defs = definitions(font);
    match stored_status(font, glyph) {
        Some(code) => defs.into_iter().find(|d| d.code == code),
        None => defs.into_iter().find(|d| d.is_default),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.setStatusDefinitions",
            "Status Definitions…",
            ["Font"],
            None,
            "{statuses: [{code: whole number 0..1000000, name: string, color: [r, g, b] | [r, g, b, a] (each 0..1), isDefault?: bool}] (at most 64; codes and names unique; at most one default; an empty list removes the definitions)} replaces the font's list of development statuses. Glyph statuses stay as stored → {statuses: [{code, name, color, isDefault}], undefined: number (glyphs whose stored code the list does not define)}",
            has_doc,
            set_definitions
        ),
        cmd!(
            "glyph.setStatus",
            "Set Status",
            ["Glyph"],
            None,
            "{glyphs: [names], status: code | name | null} puts the glyphs in one of the font's defined statuses, by code or by name; null clears the stored status (the glyph then shows the default status, if one is defined) → {glyphs: [names], status: code | null}",
            has_doc,
            set_status
        )
        .targets(GLYPHS),
    ]
}

/// Parses one entry of `statuses`.
fn parse_definition(p: &Params, i: usize, v: &Value) -> Result<StatusDef> {
    let at = |m: &str| p.bad(format!("`statuses[{i}]`: {m}"));
    let Value::Object(o) = v else { return Err(at(&format!("must be an object, got {}", kind(v)))) };
    let code = match o.get("code") {
        Some(Value::Number(n)) => n.as_i64().filter(|c| (0..=MAX_STATUS_CODE).contains(c)),
        _ => None,
    }
    .ok_or_else(|| at(&format!("`code` must be a whole number from 0 to {MAX_STATUS_CODE}")))?;
    let name = match o.get("name") {
        Some(Value::String(s)) => s.trim(),
        _ => return Err(at("`name` must be a string")),
    };
    if name.is_empty() || name.len() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(at(&format!("`name` must be 1 to {MAX_NAME_LEN} bytes without control characters")));
    }
    let bad_color = || at("`color` must be [r, g, b] or [r, g, b, a] with each component from 0 to 1");
    let parts: Vec<f64> = match o.get("color") {
        Some(Value::Array(a)) if a.len() <= 4 => a.iter().map(Value::as_f64).collect::<Option<_>>().ok_or_else(bad_color)?,
        _ => return Err(bad_color()),
    };
    let color = rgba(&parts).ok_or_else(bad_color)?;
    let is_default = match o.get("isDefault") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(at(&format!("`isDefault` must be true or false, got {}", kind(other)))),
    };
    Ok(StatusDef { code, name: name.to_string(), color, is_default })
}

fn set_definitions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.setStatusDefinitions", v)?;
    let list = p.array("statuses")?;
    if list.len() > MAX_STATUS_DEFINITIONS {
        return Err(p.bad(format!("`statuses` holds {} entries; at most {MAX_STATUS_DEFINITIONS} are allowed", list.len())));
    }
    let mut defs: Vec<StatusDef> = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let def = parse_definition(&p, i, item)?;
        if defs.iter().any(|d| d.code == def.code) {
            return Err(p.bad(format!("the code {} is used twice", def.code)));
        }
        if defs.iter().any(|d| d.name == def.name) {
            return Err(p.bad(format!("the name \"{}\" is used twice", def.name)));
        }
        defs.push(def);
    }
    if defs.iter().filter(|d| d.is_default).count() > 1 {
        return Err(p.bad("at most one status may be the default"));
    }
    s.edit("Status Definitions", |font| {
        let root = &mut Arc::make_mut(&mut font.info).user_data.0;
        if defs.is_empty() {
            if let Some(i) = root.entries.iter().position(|(k, _)| k.as_str() == fc_keys::STATUS_DEFINITIONS) {
                root.remove_at(i);
            }
        } else {
            let items = defs.iter().map(StatusDef::to_plist).collect();
            root.insert(fc_keys::STATUS_DEFINITIONS, PValue::Array(Array { items, ..Default::default() }));
        }
        Ok(())
    })?;
    let font = s.font()?;
    let undefined = font
        .glyphs
        .iter()
        .filter(|g| g.info.user_data.get(fc_keys::GLYPH_STATUS).and_then(whole).is_some_and(|c| !defs.iter().any(|d| d.code == c)))
        .count();
    Ok(json!({"statuses": defs.iter().map(StatusDef::to_json).collect::<Vec<_>>(), "undefined": undefined}))
}

/// The code `status` names: a defined code or a defined name. `None` for null (clear).
fn requested_status(p: &Params, raw: &Value, defs: &[StatusDef]) -> Result<Option<i64>> {
    let none_defined = || p.bad("the font has no status definitions; define them with Status Definitions first");
    match raw.get("status") {
        None => Err(p.bad("`status` is required (null clears it)")),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            if defs.is_empty() {
                return Err(none_defined());
            }
            let code = n.as_i64().filter(|c| defs.iter().any(|d| d.code == *c));
            let known: Vec<String> = defs.iter().map(|d| format!("{} ({})", d.code, d.name)).collect();
            code.map(Some).ok_or_else(|| p.bad(format!("the status {n} is not defined; defined: {}", known.join(", "))))
        }
        Some(Value::String(name)) => {
            if defs.is_empty() {
                return Err(none_defined());
            }
            let wanted = name.trim();
            let found = defs.iter().find(|d| d.name == wanted).or_else(|| defs.iter().find(|d| d.name.to_lowercase() == wanted.to_lowercase()));
            let known: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            found.map(|d| Some(d.code)).ok_or_else(|| p.bad(format!("there is no status named \"{wanted}\"; defined: {}", known.join(", "))))
        }
        Some(other) => Err(p.bad(format!("`status` must be a code, a name or null, got {}", kind(other)))),
    }
}

fn set_status(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setStatus", v)?;
    let font = s.font()?;
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let status = requested_status(&p, v, &definitions(font))?;
    s.edit("Set Status", |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            let current = g.info.user_data.get(fc_keys::GLYPH_STATUS).and_then(whole);
            if current == status && (status.is_some() || g.info.user_data.get(fc_keys::GLYPH_STATUS).is_none()) {
                continue;
            }
            let data = &mut g.info.user_data.0;
            match status {
                Some(code) => data.insert(fc_keys::GLYPH_STATUS, PValue::Number(Number::int(code))),
                None => {
                    if let Some(i) = data.entries.iter().position(|(k, _)| k.as_str() == fc_keys::GLYPH_STATUS) {
                        data.remove_at(i);
                    }
                }
            }
            g.info.tidy();
        }
        Ok(())
    })?;
    let mut out = Map::new();
    out.insert("glyphs".into(), json!(names));
    out.insert("status".into(), json!(status));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn stored_definitions_read_back_leniently() {
        let def = StatusDef { code: 3, name: "Review".into(), color: [1.0, 0.5, 0.0, 1.0], is_default: true };
        assert_eq!(StatusDef::from_plist(&def.to_plist()), Some(def.clone()));
        // A `.glyphs` file may give the code back as text and the flag as a number.
        let mut d = Dict::new();
        d.insert("code", PValue::String(Str::new("3")));
        d.insert(
            "color",
            PValue::Array(Array {
                items: vec![PValue::Number(Number::int(1)), PValue::Number(Number::float(0.5).unwrap()), PValue::Number(Number::int(0))],
                ..Default::default()
            }),
        );
        d.insert("isDefault", PValue::Number(Number::int(1)));
        d.insert("name", PValue::String(Str::new("Review")));
        assert_eq!(StatusDef::from_plist(&PValue::Dict(d)), Some(def));
        let mut bad = Dict::new();
        bad.insert("code", PValue::Number(Number::int(-1)));
        bad.insert("name", PValue::String(Str::new("x")));
        assert_eq!(StatusDef::from_plist(&PValue::Dict(bad)), None);
    }
}
