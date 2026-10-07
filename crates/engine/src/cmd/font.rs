//! Font Info and whole-font queries.

use fontcraft_geom::Shape as _;
use fontcraft_model::{UPM_RANGE, metric};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{Params, kind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.info.get",
            "Get Font Info",
            [],
            None,
            "{keys?: [string]} the font-wide fields (familyName, unitsPerEm, versionMajor, versionMinor, masters, glyphCount), or only `keys` → {…fields}",
            has_doc,
            info_get
        ),
        cmd!(
            "font.info.set",
            "Set Font Info",
            [],
            None,
            "{values: {familyName?: string (at most 255 bytes), unitsPerEm?: 16–16384, versionMajor?: 0–32767, versionMinor?: 0–999}} change font-wide fields as one undo step; outlines are not scaled when unitsPerEm changes → {changed: [keys]}",
            has_doc,
            info_set
        ),
        cmd!(
            query "font.listGlyphs",
            "List Glyphs",
            [],
            None,
            "{layer?: master id (default: the first master)} the glyphs in glyph order → {glyphs: [{gid, name, unicodes: [hex], export, advance, shapes, bounds: [x0, y0, x1, y1] | null}]}",
            has_doc,
            list_glyphs
        ),
    ]
}

fn info_get(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.get", p)?;
    let font = s.font()?;
    let all = json!({
        "familyName": font.family_name,
        "unitsPerEm": font.units_per_em,
        "versionMajor": font.version_major,
        "versionMinor": font.version_minor,
        "masters": font.masters.iter().map(|m| json!({
            "id": m.id, "name": m.name,
            "ascender": font.metric_pos(m, metric::ASCENDER, 0.0), "capHeight": font.metric_pos(m, metric::CAP_HEIGHT, 0.0),
            "xHeight": font.metric_pos(m, metric::X_HEIGHT, 0.0), "descender": font.metric_pos(m, metric::DESCENDER, 0.0),
        })).collect::<Vec<_>>(),
        "glyphCount": font.glyphs.len(),
    });
    let Some(keys) = p.opt_strings("keys")? else { return Ok(all) };
    let mut out = Map::new();
    for key in keys {
        let value = all.get(key).ok_or_else(|| p.bad(format!("unknown key `{key}`")))?;
        out.insert(key.to_string(), value.clone());
    }
    Ok(Value::Object(out))
}

fn info_set(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.info.set", p)?;
    let values = p.opt_object("values")?.ok_or_else(|| p.bad("`values` is required"))?;
    if values.is_empty() {
        return Err(p.bad("`values` is empty"));
    }
    let whole = |key: &str, v: &Value, min: u32, max: u32| -> Result<u16> {
        let n = v.as_f64().filter(|n| n.fract() == 0.0 && *n >= f64::from(min) && *n <= f64::from(max));
        n.map(|n| n as u16).ok_or_else(|| p.bad(format!("`{key}` must be a whole number from {min} to {max}, got {v}")))
    };
    s.edit("Set Font Info", |font| {
        for (key, v) in values {
            match key.as_str() {
                "familyName" => match v.as_str().map(str::trim).filter(|n| !n.is_empty()) {
                    Some(name) => font.family_name = name.to_string(),
                    None => return Err(p.bad(format!("`familyName` must be a non-empty string, got {}", kind(v)))),
                },
                "unitsPerEm" => font.units_per_em = whole(key, v, u32::from(*UPM_RANGE.start()), u32::from(*UPM_RANGE.end()))?,
                "versionMajor" => font.version_major = whole(key, v, 0, 32_767)?,
                "versionMinor" => font.version_minor = whole(key, v, 0, 999)?,
                other => return Err(p.bad(format!("unknown key `{other}` (known: familyName, unitsPerEm, versionMajor, versionMinor)"))),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "changed": values.keys().collect::<Vec<_>>() }))
}

fn list_glyphs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.listGlyphs", p)?;
    let font = s.font()?;
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
