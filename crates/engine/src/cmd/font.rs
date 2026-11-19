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
            "{font?: path, face?: number, layer?: master id (default: the first master), glyphs?: selector, excludeGlyphs?: selector, columns?: [\"gid\"|\"name\"|\"unicodes\"|\"advance\"|\"bbox\"|\"kind\"], names?: \"final\"|\"source\", goadb?: path} the glyphs of the active document, or of the font file at `font` without opening it, in glyph order. A selector is a string or list of strings of comma-separated items: names, glyph ids, U+XXXX code points, ranges A..B (or N-M between two ids) and *; excluded glyphs other than .notdef are left out. names \"source\" (the default when goadb is given) shows the source name the GOADB gives each final name. Without columns each glyph is {gid, name, unicodes: [hex], export, advance, shapes, bounds: [x0, y0, x1, y1] | null}; with columns it holds gid and the columns asked for (bbox as bounds; kind is empty, simple or composite) → {glyphs: [...]}",
            always,
            list_glyphs
        ).files(const { &[FileParam::read("font"), FileParam::read("goadb")] }),
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

/// The columns `font.listGlyphs` can give, in the order they are listed.
const LIST_COLUMNS: [&str; 8] = ["gid", "name", "cid", "unicodes", "advance", "bbox", "fd", "kind"];

/// Most items the selectors of one `font.listGlyphs` call may hold.
const MAX_SELECTOR_ITEMS: usize = 100_000;

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
    let columns = match p.opt_strings("columns")? {
        None => None,
        Some(cols) => {
            for c in &cols {
                if !LIST_COLUMNS.contains(c) {
                    return Err(p.bad(format!("unknown column `{c}` (columns: {})", LIST_COLUMNS.join(", "))));
                }
                if matches!(*c, "cid" | "fd") {
                    return Err(crate::EngineError::Unsupported(format!("the `{c}` column of the glyph list")));
                }
            }
            Some(cols)
        }
    };
    let source_names = source_names(&p)?;
    let chosen = chosen_glyphs(&p, font)?;
    let wants = |c: &str| columns.as_ref().is_none_or(|cols| cols.iter().any(|x| *x == c));
    let glyphs: Vec<Value> = font
        .glyphs
        .iter()
        .enumerate()
        .filter(|(gid, _)| chosen.as_ref().is_none_or(|set| set.contains(gid)))
        .map(|(gid, g)| {
            let l = g.layer(&layer);
            let mut row = Map::new();
            row.insert("gid".into(), json!(gid));
            if wants("name") {
                let name = source_names.as_ref().and_then(|m| m.get(g.name.as_str())).map_or(g.name.as_str(), String::as_str);
                row.insert("name".into(), json!(name));
            }
            if wants("unicodes") {
                row.insert("unicodes".into(), json!(g.unicodes.iter().map(|u| format!("{u:04X}")).collect::<Vec<_>>()));
            }
            if columns.is_none() {
                row.insert("export".into(), json!(g.export));
            }
            if wants("advance") {
                row.insert("advance".into(), json!(l.map(|l| l.width)));
            }
            if columns.is_none() {
                row.insert("shapes".into(), json!(l.map_or(0, |l| l.shapes.len())));
            }
            if wants("bbox") {
                // A glyph whose outline cannot be resolved still lists; `bounds` is null.
                let bounds = font.outline(&g.name, &layer).ok().filter(|o| !o.elements().is_empty()).map(|o| {
                    let b = o.bounding_box();
                    json!([b.x0, b.y0, b.x1, b.y1])
                });
                row.insert("bounds".into(), json!(bounds));
            }
            if columns.as_ref().is_some_and(|cols| cols.contains(&"kind")) {
                let shapes = l.map(|l| l.shapes.as_slice()).unwrap_or_default();
                let kind = if shapes.iter().any(|s| matches!(s, fontcraft_model::Shape::Component(_))) {
                    "composite"
                } else if shapes.iter().any(|s| matches!(s, fontcraft_model::Shape::Path(_))) {
                    "simple"
                } else {
                    "empty"
                };
                row.insert("kind".into(), json!(kind));
            }
            Value::Object(row)
        })
        .collect();
    Ok(json!({ "glyphs": glyphs }))
}

/// The source name of each final name, read from `goadb`, when the names asked for are the
/// source names.
fn source_names(p: &Params) -> Result<Option<std::collections::HashMap<String, String>>> {
    let goadb = p.opt_str("goadb")?;
    let names = p.opt_str("names")?.unwrap_or(if goadb.is_some() { "source" } else { "final" });
    match (names, goadb) {
        ("final", _) => Ok(None),
        ("source", Some(path)) => {
            let table = read_goadb(p, path)?;
            Ok(Some(table.entries.into_iter().map(|e| (e.final_name, e.source_name)).collect()))
        }
        ("source", None) => Err(p.bad("`names` \"source\" needs a `goadb` to map the names back")),
        (other, _) => Err(p.bad(format!("`names` must be \"final\" or \"source\", got \"{other}\""))),
    }
}

/// The glyph ids `glyphs` names, less those `excludeGlyphs` names (never .notdef); `None` when
/// neither is given.
fn chosen_glyphs(p: &Params, font: &fontcraft_model::Font) -> Result<Option<std::collections::BTreeSet<usize>>> {
    let include = selector_items(p, "glyphs")?;
    let exclude = selector_items(p, "excludeGlyphs")?;
    if include.is_none() && exclude.is_none() {
        return Ok(None);
    }
    let by_name: std::collections::HashMap<&str, usize> = font.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let mut by_code: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (i, g) in font.glyphs.iter().enumerate() {
        for &u in &g.unicodes {
            by_code.entry(u).or_insert(i);
        }
    }
    let count = font.glyphs.len();
    let one = |key: &str, item: &str| -> Result<usize> {
        if let Some(&gid) = by_name.get(item) {
            return Ok(gid);
        }
        if let Ok(gid) = item.parse::<usize>() {
            return if gid < count { Ok(gid) } else { Err(p.bad(format!("`{key}`: there is no glyph {gid} (the font has {count})"))) };
        }
        if let Some(hex) = item.strip_prefix("U+").or_else(|| item.strip_prefix("u+")) {
            let code = u32::from_str_radix(hex, 16).map_err(|_| p.bad(format!("`{key}`: `{item}` is not a code point")))?;
            return by_code.get(&code).copied().ok_or_else(|| p.bad(format!("`{key}`: no glyph has the code point {item}")));
        }
        if item.starts_with('/') || item.starts_with("re:") {
            return Err(p.bad(format!("`{key}`: `{item}`: CID and pattern items are not read by this listing")));
        }
        Err(p.bad(format!("`{key}`: there is no glyph `{item}`")))
    };
    // Items become id ranges that are merged before the set is filled, so repeated or
    // overlapping ranges cost no more than the glyph count.
    let resolve = |key: &str, items: &[String]| -> Result<std::collections::BTreeSet<usize>> {
        let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(items.len());
        for item in items {
            if item == "*" {
                if count > 0 {
                    ranges.push((0, count - 1));
                }
                continue;
            }
            let range = item.split_once("..").or_else(|| {
                item.split_once('-')
                    .filter(|(a, b)| a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit()) && !a.is_empty() && !b.is_empty())
            });
            match range {
                Some((a, b)) if !by_name.contains_key(item.as_str()) => {
                    let (first, last) = (one(key, a)?, one(key, b)?);
                    if first > last {
                        return Err(p.bad(format!("`{key}`: the range `{item}` runs backwards")));
                    }
                    ranges.push((first, last));
                }
                _ => {
                    let gid = one(key, item)?;
                    ranges.push((gid, gid));
                }
            }
        }
        ranges.sort_unstable();
        let mut set = std::collections::BTreeSet::new();
        let mut next = 0;
        for (first, last) in ranges {
            let start = first.max(next);
            if start <= last {
                set.extend(start..=last);
                next = last + 1;
            }
        }
        Ok(set)
    };
    let mut chosen = match &include {
        Some(items) => resolve("glyphs", items)?,
        None => (0..count).collect(),
    };
    if let Some(items) = &exclude {
        for gid in resolve("excludeGlyphs", items)? {
            if gid != 0 {
                chosen.remove(&gid);
            }
        }
    }
    Ok(Some(chosen))
}

/// The items of the selector `key`: one string or a list of strings, each split at commas.
fn selector_items(p: &Params, key: &str) -> Result<Option<Vec<String>>> {
    let Some(strings) = p.opt_strings(key)? else { return Ok(None) };
    let mut items = Vec::new();
    for s in strings {
        for item in s.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            if items.len() >= MAX_SELECTOR_ITEMS {
                return Err(p.bad(format!("`{key}` holds more than {MAX_SELECTOR_ITEMS} items")));
            }
            items.push(item.to_string());
        }
    }
    Ok(Some(items))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_goadb(p: &Params, path: &str) -> Result<fontcraft_tables::goadb::Goadb> {
    let bytes = crate::io::read_file(std::path::Path::new(path))?;
    let text = String::from_utf8(bytes).map_err(|_| p.bad(format!("`goadb`: {path} is not UTF-8 text")))?;
    fontcraft_tables::goadb::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).map_err(|e| p.bad(format!("`goadb`: {e}")))
}

#[cfg(target_arch = "wasm32")]
fn read_goadb(_: &Params, _: &str) -> Result<fontcraft_tables::goadb::Goadb> {
    Err(crate::EngineError::Unsupported("reading a file by path in the browser".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn open_file(path: &str, face: Option<u32>) -> Result<fontcraft_model::Font> {
    crate::io::open(std::path::Path::new(path), face).map(|(font, _)| font)
}

#[cfg(target_arch = "wasm32")]
fn open_file(_: &str, _: Option<u32>) -> Result<fontcraft_model::Font> {
    Err(crate::EngineError::Unsupported("reading a file by path in the browser".into()))
}
