//! The Dimensions notepad: measurements a designer notes per master (stems, bowls…). Nothing
//! else reads them (`dimensions.noEffectOnFont`). They live in the font's user data under the key
//! the reference's files use, as one dictionary of field key → number per master id.

use std::sync::Arc;

use fontcraft_model::plist::{Dict, Number, Str, Value as PValue};
use fontcraft_model::{Font, MAX_COORDINATE, MAX_NAME_LEN};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::Params;

/// The font user-data key that holds the values, as `.glyphs` files store them.
const KEY: &str = "GSDimensionPlugin.Dimensions";

/// The fields the Dimensions section shows per script. The reference does not document them;
/// these are the keys found in the corpus files.
const FIELDS: &[(&str, &[&str])] = &[("latin", &["HH", "HV", "OH", "OV", "nV", "nd", "oH", "oV", "tH"]), ("arabic", &["arAlef", "arBar"])];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "dimensions.setValue",
            "Set Dimension",
            [],
            None,
            "{master?: id (default: the first master), key: string (a field key such as \"nV\"), value?: number (absent or null clears the field)} note one measurement in a master's Dimensions; the numbers change nothing else in the font → {master, key, value}",
            has_doc,
            set_value
        ),
        cmd!(
            query "dimensions.getValues",
            "Get Dimensions",
            [],
            None,
            "{master?: id (default: the first master), script?: string, glyph?: name (its script, when `script` is not given)} the Dimensions values noted for a master; with a script whose fields are known, only those fields → {master, script?, fields?: [keys], values: {key: number}}",
            has_doc,
            get_values
        ),
    ]
}

fn master_id(p: &Params, font: &Font) -> Result<String> {
    Ok(match p.opt_str("master")? {
        Some(m) => font.master(m)?.id.clone(),
        None => font.default_master()?.id.clone(),
    })
}

fn remove_key(d: &mut Dict, key: &str) {
    if let Some(i) = d.entries.iter().position(|(k, _)| k.as_str() == key) {
        d.remove_at(i);
    }
}

/// A value as the file keeps it: a number, or text when the field held text before.
fn encode(p: &Params, v: f64, as_text: bool) -> Result<PValue> {
    let whole = v.fract() == 0.0 && v.abs() <= MAX_COORDINATE;
    if as_text {
        let text = if whole { format!("{}", v as i64) } else { format!("{v}") };
        return Ok(PValue::String(Str::new(text)));
    }
    if whole {
        return Ok(PValue::Number(Number::int(v as i64)));
    }
    Number::float(v).map(PValue::Number).ok_or_else(|| p.bad(format!("`value` {v} cannot be stored")))
}

fn decode(v: &PValue) -> Option<f64> {
    let n = match v {
        PValue::Number(n) => n.value().as_f64(),
        PValue::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// The dictionary under `key` in `d`: empty when absent, an error when something else is there.
fn sub_dict(p: &Params, d: &Dict, key: &str, what: &str) -> Result<Dict> {
    match d.get(key) {
        None => Ok(Dict::new()),
        Some(PValue::Dict(inner)) => Ok(inner.clone()),
        Some(_) => Err(p.bad(format!("{what} is not a dictionary in the font's user data; it is left as it is"))),
    }
}

fn set_value(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("dimensions.setValue", p)?;
    let key = p.str("key")?;
    if key.is_empty() || key.chars().count() > MAX_NAME_LEN || key.chars().any(char::is_control) {
        return Err(p.bad(format!("`key` must be 1 to {MAX_NAME_LEN} characters without control characters")));
    }
    let value = p.opt_f64("value")?;
    if let Some(v) = value
        && v.abs() > MAX_COORDINATE
    {
        return Err(p.bad(format!("`value` must be within ±{MAX_COORDINATE}, got {v}")));
    }
    let master = master_id(&p, s.font()?)?;
    s.edit("Set Dimension", |font| {
        let root = &mut Arc::make_mut(&mut font.info).user_data.0;
        let mut all = sub_dict(&p, root, KEY, &format!("`{KEY}`"))?;
        let mut values = sub_dict(&p, &all, &master, &format!("the entry of master `{master}`"))?;
        match value {
            Some(v) => {
                let as_text = matches!(values.get(key), Some(PValue::String(_)));
                values.insert(key, encode(&p, v, as_text)?);
            }
            None => remove_key(&mut values, key),
        }
        if values.is_empty() {
            remove_key(&mut all, &master);
        } else {
            all.insert(master.as_str(), PValue::Dict(values));
        }
        if all.is_empty() {
            remove_key(root, KEY);
        } else {
            root.insert(KEY, PValue::Dict(all));
        }
        Ok(())
    })?;
    Ok(json!({"master": master, "key": key, "value": value}))
}

fn get_values(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("dimensions.getValues", p)?;
    let font = s.font()?;
    let master = master_id(&p, font)?;
    let (script, explicit) = match (p.opt_str("script")?, p.opt_str("glyph")?) {
        (Some(sc), _) => (Some(sc.to_lowercase()), true),
        (None, Some(g)) => {
            let glyph = font.glyph(g).ok_or_else(|| p.bad(format!("there is no glyph `{g}`")))?;
            (glyph.info.script.as_deref().map(str::to_lowercase), false)
        }
        (None, None) => (None, false),
    };
    let fields = script.as_deref().and_then(|sc| FIELDS.iter().find(|(name, _)| *name == sc)).map(|(_, f)| *f);
    if let (Some(sc), None, true) = (&script, fields, explicit) {
        let known: Vec<&str> = FIELDS.iter().map(|(name, _)| *name).collect();
        return Err(p.bad(format!("no Dimensions fields are known for script `{sc}`; known: {}", known.join(", "))));
    }
    let mut values = Map::new();
    if let Some(PValue::Dict(all)) = font.info.user_data.get(KEY)
        && let Some(PValue::Dict(noted)) = all.get(&master)
    {
        for (k, v) in &noted.entries {
            if fields.is_none_or(|f| f.contains(&k.as_str()))
                && let Some(n) = decode(v)
            {
                values.insert(k.as_str().to_string(), json!(n));
            }
        }
    }
    let mut out = json!({"master": master, "values": values});
    if let Some(o) = out.as_object_mut() {
        if let Some(sc) = script {
            o.insert("script".into(), json!(sc));
        }
        if let Some(f) = fields {
            o.insert("fields".into(), json!(f));
        }
    }
    Ok(out)
}
