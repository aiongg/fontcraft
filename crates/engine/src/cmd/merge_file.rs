//! `font.merge` (M12.23; `fontcraft_tables::merge`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! Each `fonts` item is a path, or `{path, face?, map?, glyphs?, excludeGlyphs?}`. A map file or
//! a glyph selection turns the run into a glyph merge (only the listed glyphs, font-wide values
//! from the first font); without either, whole fonts are merged with their layout. The options
//! for CID-keyed output, hint dictionaries, renaming and conversion are refused as not built.

use std::path::PathBuf;

use fontcraft_tables::merge::{Duplicates, LayoutMode, MergeOptions, merge_with_report, parse_glyph_map};
use fontcraft_tables::reorder::{glyph_code_points, glyph_names};
use fontcraft_tables::sfnt::parse_tag;
use fontcraft_tables::{SfntFont, TableOrder, WriteOptions};
use serde_json::{Value, json};

use super::file_io::{self, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most items one glyph selector may list.
const MAX_SELECTOR_ITEMS: usize = 65_536;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.merge",
        "Merge Fonts",
        [],
        None,
        "{fonts: [{path: path, face?: int|name, map?: path, glyphs?: selector, excludeGlyphs?: selector, fd?: [int], excludeFd?: [int]}], output: path, layout?: \"merge\"|\"base\"|\"drop\", duplicates?: \"first\"|\"last\"|\"error\"|\"rename\", dropTables?: [tag], importTtx?: path, cidInfo?: path, hintDictFrom?: path, psName?: string, languageGroup?: 0|1, to?: string}",
        always,
        merge_fonts
    )
    .files(const {
        &[
            FileParam::read("fonts"),
            FileParam::write("output"),
            FileParam::read("importTtx"),
            FileParam::read("cidInfo"),
            FileParam::read("hintDictFrom"),
        ]
    })
    .writes_files()]
}

/// Options of the reference tools that are not built.
const CFF_ONLY: [(&str, &str); 6] = [
    ("importTtx", "compiling an XML dump into the merged font"),
    ("cidInfo", "CID-keyed output"),
    ("hintDictFrom", "taking the hint dictionary from another font"),
    ("psName", "setting the PostScript name of a merged CFF font"),
    ("languageGroup", "setting the language group of a merged CFF font"),
    ("to", "converting the merged font"),
];

fn merge_fonts(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.merge", v)?;
    for (key, what) in CFF_ONLY {
        if p.get(key).is_some_and(|v| !v.is_null()) {
            return Err(EngineError::Unsupported(format!("`{key}`: {what} is not built yet")));
        }
    }
    p.str("output")?;
    let request = OutputRequest::from_params(&p)?;
    let layout = p.opt_str("layout")?.map(LayoutMode::parse).transpose().map_err(|e| p.bad(e.to_string()))?;
    let duplicates = p.opt_str("duplicates")?.map(Duplicates::parse).transpose().map_err(|e| p.bad(e.to_string()))?;
    let drop_tables = match p.opt_strings("dropTables")? {
        None => Vec::new(),
        Some(tags) => tags.iter().map(|t| parse_tag(t)).collect::<std::result::Result<_, _>>().map_err(|e| p.bad(e.to_string()))?,
    };
    let items = p.array("fonts")?;
    if items.len() > fontcraft_tables::limits::MAX_MERGE_FONTS {
        return Err(p.bad(format!("`fonts` lists more than {} fonts", fontcraft_tables::limits::MAX_MERGE_FONTS)));
    }

    let mut fonts: Vec<SfntFont> = Vec::with_capacity(items.len());
    let mut glyph_maps: Vec<Vec<(String, String)>> = Vec::with_capacity(items.len());
    let mut notes: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let (font, map) = read_item(&p, i, item, &mut notes)?;
        fonts.push(font);
        glyph_maps.push(map);
    }
    if glyph_maps.iter().all(Vec::is_empty) {
        glyph_maps.clear();
    }
    let options = MergeOptions { glyph_maps, drop_tables, layout, duplicates, timestamp: Some(super::file::default_timestamp()) };
    let result = merge_with_report(&fonts, &options)?;
    let bytes = fontcraft_tables::write(&result.font, &WriteOptions { order: TableOrder::Recommended })?;
    let first_input = item_path(&p, items.first().unwrap_or(&Value::Null))?;
    let path = file_io::output_path(&first_input, &request, ".merged", None)?;
    file_io::write_output(&path, &bytes, &request)?;
    let mut warnings = notes;
    warnings.extend(result.warnings);
    Ok(json!({
        "output": path.display().to_string(),
        "bytes": bytes.len(),
        "glyphCount": result.glyphs.len(),
        "glyphs": result.glyphs.iter().map(|g| json!({"name": g.name, "font": g.font, "source": g.source})).collect::<Vec<_>>(),
        "conflicts": result.conflicts,
        "warnings": warnings,
    }))
}

fn item_path(p: &Params<'_>, item: &Value) -> Result<PathBuf> {
    match item {
        Value::String(s) => Ok(PathBuf::from(s)),
        Value::Object(o) => o.get("path").and_then(Value::as_str).map(PathBuf::from).ok_or_else(|| p.bad("each `fonts` item needs a `path`")),
        _ => Err(p.bad("each `fonts` item is a path or {path, …}")),
    }
}

/// One input font and its glyph map (empty: every glyph under its own name).
fn read_item(p: &Params<'_>, index: usize, item: &Value, notes: &mut Vec<String>) -> Result<(SfntFont, Vec<(String, String)>)> {
    let path = item_path(p, item)?;
    let file = file_io::read_input(&path)?;
    if !file.kind.is_font() {
        return Err(EngineError::Unsupported(format!("merging {}: give compiled fonts", path.display())));
    }
    let empty = Value::Null;
    let item_params = Params::new("font.merge", if item.is_object() { item } else { &empty })?;
    for key in ["fd", "excludeFd"] {
        if item_params.get(key).is_some_and(|v| !v.is_null()) {
            return Err(EngineError::Unsupported(format!("`{key}` (font {}): selecting CID font dictionaries is not built yet", index + 1)));
        }
    }
    let face = super::dehint::face_index(&item_params, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    let names = glyph_names(&font)?;

    let mut map: Vec<(String, String)> = Vec::new();
    if let Some(map_path) = item_params.opt_str("map")? {
        let text = file_io::read_input(&PathBuf::from(map_path))?;
        let text = String::from_utf8(text.bytes).map_err(|_| p.bad(format!("the glyph map {map_path} is not UTF-8 text")))?;
        let parsed = parse_glyph_map(&text).map_err(|e| p.bad(format!("{map_path}: {e}")))?;
        if parsed.ps_name.is_some() || parsed.language_group.is_some() {
            notes.push(format!("the PostScript name and language group in {map_path} apply to CFF output only and are ignored"));
        }
        map = parsed.entries;
    }
    let include = selector_items(&item_params, "glyphs")?;
    let exclude = selector_items(&item_params, "excludeGlyphs")?;
    if include.is_some() || exclude.is_some() {
        let code_points = glyph_code_points(&font)?;
        let resolve = |items: &[String]| -> Result<Vec<bool>> {
            let mut set = vec![false; names.len()];
            for item in items {
                select(item, &names, &code_points, &mut set).map_err(|m| p.bad(format!("font {}: {m}", index + 1)))?;
            }
            Ok(set)
        };
        let mut chosen = match &include {
            Some(items) => resolve(items)?,
            None => vec![true; names.len()],
        };
        if let Some(items) = &exclude {
            for (c, x) in chosen.iter_mut().zip(resolve(items)?) {
                *c &= !x;
            }
        }
        let is_chosen = |name: &str| names.iter().position(|n| n == name).and_then(|g| chosen.get(g).copied()).unwrap_or(false);
        if map.is_empty() {
            map = names.iter().filter(|n| is_chosen(n)).map(|n| (n.clone(), n.clone())).collect();
        } else {
            map.retain(|(_, source)| is_chosen(source));
        }
        if map.is_empty() {
            return Err(p.bad(format!("font {}: the selection takes no glyph", index + 1)));
        }
    }
    Ok((font, map))
}

/// A selector given as a list of strings or as one comma- or space-separated string.
fn selector_items(p: &Params<'_>, key: &str) -> Result<Option<Vec<String>>> {
    let items: Vec<String> = match p.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s.split([',', ' ']).filter(|w| !w.is_empty()).map(str::to_string).collect(),
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| p.bad(format!("`{key}` items must be strings")))).collect::<Result<_>>()?
        }
        Some(_) => return Err(p.bad(format!("`{key}` must be a string or a list of strings"))),
    };
    if items.len() > MAX_SELECTOR_ITEMS {
        return Err(p.bad(format!("`{key}` lists more than {MAX_SELECTOR_ITEMS} items")));
    }
    Ok(Some(items))
}

/// Marks the glyphs one selector item names: `*`, a glyph id (`gid12`), a code point (`U+0041`)
/// or a glyph name.
fn select(item: &str, names: &[String], code_points: &[Vec<u32>], set: &mut [bool]) -> std::result::Result<(), String> {
    if item == "*" {
        set.iter_mut().for_each(|s| *s = true);
        return Ok(());
    }
    if let Some(at) = names.iter().position(|n| n == item) {
        if let Some(s) = set.get_mut(at) {
            *s = true;
        }
        return Ok(());
    }
    if let Some(hex) = item.strip_prefix("U+").or_else(|| item.strip_prefix("u+")) {
        let cp = u32::from_str_radix(hex, 16).map_err(|_| format!("`{item}` is not a code point"))?;
        let at = code_points.iter().position(|list| list.contains(&cp)).ok_or_else(|| format!("no glyph is mapped to {item}"))?;
        if let Some(s) = set.get_mut(at) {
            *s = true;
        }
        return Ok(());
    }
    if let Some(g) = item.strip_prefix("gid").and_then(|t| t.parse::<usize>().ok()) {
        let s = set.get_mut(g).ok_or_else(|| format!("glyph {g} does not exist: the font has {} glyphs", names.len()))?;
        *s = true;
        return Ok(());
    }
    Err(format!("no glyph is named `{item}`"))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use fontcraft_compile::{Flavour, Options, compile};
    use fontcraft_model::{Font, Path, Shape};

    fn font_bytes(family: &str, glyphs: &[(&str, u32, f64)]) -> Vec<u8> {
        let mut f = Font::new(family);
        for (name, cp, width) in glyphs {
            if f.glyph(name).is_none() {
                f.add_glyph(name, vec![*cp], *width).unwrap();
            }
            let mut g = f.glyph_mut(name).unwrap();
            g.layers[0].width = *width;
            g.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 700.0)));
        }
        compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap().bytes
    }

    #[test]
    fn font_merge_writes_one_font_and_leaves_the_inputs() {
        let dir = std::env::temp_dir().join(format!("fontcraft-merge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("A.ttf"), dir.join("B.ttf"));
        let (bytes_a, bytes_b) = (font_bytes("Merge A", &[("A", 0x41, 600.0)]), font_bytes("Merge B", &[("B", 0x42, 500.0)]));
        std::fs::write(&a, &bytes_a).unwrap();
        std::fs::write(&b, &bytes_b).unwrap();
        let out = dir.join("AB.ttf");

        let mut session = Session::new();
        let result = session
            .execute("font.merge", &json!({"fonts": [a.display().to_string(), b.display().to_string()], "output": out.display().to_string()}))
            .unwrap();
        assert_eq!(result["output"], json!(out.display().to_string()));
        let names: Vec<&str> = result["glyphs"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"A") && names.contains(&"B") && names.contains(&".notdef.1"), "{names:?}");
        let merged = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        assert_eq!(glyph_names(&merged).unwrap().len(), names.len());
        assert_eq!(std::fs::read(&a).unwrap(), bytes_a, "the inputs are never changed");

        // A map file selects and renames; .notdef comes from the first font, which has no map.
        let map = dir.join("b.map");
        std::fs::write(&map, "mergefonts\nB.alt B\n").unwrap();
        let out2 = dir.join("AB2.ttf");
        let result = session
            .execute(
                "font.merge",
                &json!({"fonts": [a.display().to_string(), {"path": b.display().to_string(), "map": map.display().to_string()}], "output": out2.display().to_string()}),
            )
            .unwrap();
        assert_eq!(result["glyphs"].as_array().unwrap().last().unwrap()["name"], json!("B.alt"));

        let e = session
            .execute(
                "font.merge",
                &json!({"fonts": [a.display().to_string(), b.display().to_string()], "output": out.display().to_string(), "cidInfo": "x"}),
            )
            .unwrap_err();
        assert!(e.to_string().contains("cidInfo"), "{e}");
        let e = session
            .execute(
                "font.merge",
                &json!({"fonts": [a.display().to_string(), b.display().to_string()], "output": out.display().to_string(), "layout": "all"}),
            )
            .unwrap_err();
        assert!(e.to_string().contains("layout"), "{e}");
        let e = session
            .execute("font.merge", &json!({"fonts": [{"path": b.display().to_string(), "glyphs": "nosuch"}], "output": out.display().to_string()}))
            .unwrap_err();
        assert!(e.to_string().contains("nosuch"), "{e}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
