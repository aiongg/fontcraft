//! Font Info and whole-font queries.

use fontcraft_fontinfo::fields as fontinfo_fields;
use fontcraft_fontinfo::{Scope, target};
use fontcraft_geom::Shape as _;
use fontcraft_model::{StyleClass, metric};
use serde_json::{Map, Value, json};

use super::font_fields::{fields_json, params_json, params_of, properties, property_value_json, scope_of, set_field, target_of, targets_of};
use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.info.get",
            "Get Font Info",
            [],
            None,
            "{scope?: 'font'|'master'|'instance' (default font), target?: id|name|index (required below the font), keys?: [string]} the Font Info of the font, one master or one export: its built-in fields, its added fields with every language (fields: [{key, label, localizable, value: text | [{language, value}]}]) and its custom parameters (customParameters: [{name, value, enabled}]); `keys` picks top-level entries or added fields by key. Font: {familyName, unitsPerEm, versionMajor, versionMinor, date, masters: [{id, name, ascender, capHeight, xHeight, descender}], glyphCount, fields, customParameters}; master: {id, name, fields, customParameters}; instance: {id, name, weightClass, widthClass, exports, fields, customParameters}",
            has_doc,
            info_get
        ),
        cmd!(
            "font.info.set",
            "Set Font Info",
            [],
            None,
            "{scope?: 'font'|'master'|'instance' (default font), targets?: [id|name|index] (required below the font), values: {key: value}} write built-in and added fields as one undo step, the same values to every target. Font: familyName (at most 255 bytes), unitsPerEm (16–16384; outlines are not scaled), versionMajor (0–32767), versionMinor (0–999), date (ISO 8601 or the format's text, stored in UTC); master: name; instance: name, weightClass (1–1000), widthClass (1–9). An added field takes a text (a localizable field's default language) or [{language, value}] (all its languages), and is added when the target lacks it → {changed: [keys], warnings: [text]} (a family name with characters other than ASCII letters, digits and spaces; more than 3000, 4096 or 5000 units per em)",
            has_doc,
            info_set
        ),
        cmd!(
            query "font.listGlyphs",
            "List Glyphs",
            [],
            None,
            "{font?: path, face?: number, layer?: master id (default: the first master)} the glyphs of the active document, or of the font file at `font` without opening it, in glyph order → {glyphs: [{gid, name, unicodes: [hex], export, advance, shapes, bounds: [x0, y0, x1, y1] | null}]}",
            always,
            list_glyphs
        ),
    ]
}

fn info_get(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.get", p)?;
    let font = s.font()?;
    let scope = scope_of(&p)?;
    let i = target_of(&p, font, scope)?;
    let missing = || p.bad(format!("there is no {} #{i}", scope.name()));
    let props = properties(font, scope, i).ok_or_else(missing)?;
    let fields = fields_json(props, scope);
    let params = params_json(params_of(font, scope, i).ok_or_else(missing)?);
    let class = |c: &Option<StyleClass>| match c {
        Some(StyleClass::Number(n)) => json!(n),
        Some(StyleClass::Name(name)) => json!(name),
        None => Value::Null,
    };
    let all = match scope {
        Scope::Font => json!({
            "familyName": font.family_name,
            "unitsPerEm": font.units_per_em,
            "versionMajor": font.version_major,
            "versionMinor": font.version_minor,
            "date": font.info.date,
            "masters": font.masters.iter().map(|m| json!({
                "id": m.id, "name": m.name,
                "ascender": font.metric_pos(m, metric::ASCENDER, 0.0), "capHeight": font.metric_pos(m, metric::CAP_HEIGHT, 0.0),
                "xHeight": font.metric_pos(m, metric::X_HEIGHT, 0.0), "descender": font.metric_pos(m, metric::DESCENDER, 0.0),
            })).collect::<Vec<_>>(),
            "glyphCount": font.glyphs.len(),
            "fields": fields,
            "customParameters": params,
        }),
        Scope::Master => {
            let m = font.masters.get(i).ok_or_else(missing)?;
            json!({"id": m.id, "name": m.name, "fields": fields, "customParameters": params})
        }
        Scope::Instance => {
            let x = font.instances.get(i).ok_or_else(missing)?;
            json!({
                "id": x.id, "name": target::instance_name(x), "weightClass": class(&x.weight_class), "widthClass": class(&x.width_class),
                "exports": x.exports, "fields": fields, "customParameters": params,
            })
        }
    };
    let Some(keys) = p.opt_strings("keys")? else { return Ok(all) };
    let mut out = Map::new();
    for key in keys {
        let value = match all.get(key) {
            Some(v) => v.clone(),
            None => {
                let stored = fontinfo_fields::field(scope, key).map_or(key, |f| f.key);
                let prop = props.get(stored).or_else(|| props.get(key)).ok_or_else(|| p.bad(format!("unknown key `{key}`")))?;
                property_value_json(&prop.value)
            }
        };
        out.insert(key.to_string(), value);
    }
    Ok(Value::Object(out))
}

fn info_set(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.set", p)?;
    let scope = scope_of(&p)?;
    let values = p.opt_object("values")?.ok_or_else(|| p.bad("`values` is required"))?;
    if values.is_empty() {
        return Err(p.bad("`values` is empty"));
    }
    let targets = targets_of(&p, s.font()?, scope, "targets")?;
    let mut warnings = s.edit("Set Font Info", |font| {
        let mut warnings = Vec::new();
        for &i in &targets {
            for (key, v) in values {
                set_field(font, scope, i, key, v, &mut warnings).map_err(|m| p.bad(m))?;
            }
        }
        Ok(warnings)
    })?;
    warnings.dedup();
    Ok(json!({ "changed": values.keys().collect::<Vec<_>>(), "warnings": warnings }))
}

fn list_glyphs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.listGlyphs", p)?;
    let from_file;
    let font = match p.opt_str("font")? {
        Some(path) => {
            from_file = open_file(path, p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32))?;
            &from_file
        }
        None => s.font()?,
    };
    let layer = match p.opt_str("layer")? {
        Some(id) => font.master(id)?.id.clone(),
        None => font.default_master()?.id.clone(),
    };
    let glyphs: Vec<Value> = font
        .glyphs
        .iter()
        .enumerate()
        .map(|(gid, g)| {
            let l = g.layer(&layer);
            // A glyph whose outline cannot be resolved still lists; `bounds` is null.
            let bounds = font.outline(&g.name, &layer).ok().filter(|o| !o.elements().is_empty()).map(|o| {
                let b = o.bounding_box();
                json!([b.x0, b.y0, b.x1, b.y1])
            });
            json!({
                "gid": gid,
                "name": g.name,
                "unicodes": g.unicodes.iter().map(|u| format!("{u:04X}")).collect::<Vec<_>>(),
                "export": g.export,
                "advance": l.map(|l| l.width),
                "shapes": l.map_or(0, |l| l.shapes.len()),
                "bounds": bounds,
            })
        })
        .collect();
    Ok(json!({ "glyphs": glyphs }))
}

#[cfg(not(target_arch = "wasm32"))]
fn open_file(path: &str, face: Option<u32>) -> Result<fontcraft_model::Font> {
    crate::io::open(std::path::Path::new(path), face).map(|(font, _)| font)
}

#[cfg(target_arch = "wasm32")]
fn open_file(_: &str, _: Option<u32>) -> Result<fontcraft_model::Font> {
    Err(crate::EngineError::Unsupported("reading a file by path in the browser".into()))
}
