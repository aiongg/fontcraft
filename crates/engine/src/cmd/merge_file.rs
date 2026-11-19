//! `font.merge` (M12.23; `fontcraft_tables::merge`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! Each `fonts` item is a path, or `{path, face?, map?, glyphs?, excludeGlyphs?, fd?,
//! excludeFd?}`: a compiled font, a Type 1 font or a bare CFF table. A map file or a glyph
//! selection turns the run into a glyph merge (only the listed glyphs, font-wide values from the
//! first font); without either, whole fonts are merged with their layout. `cidInfo`, numeric map
//! names or a CID-keyed first font make a CFF result CID-keyed. `importTtx` is compiled into the
//! result; `to` (or a font program as the first font) hands the result to the converter.

use std::path::{Path, PathBuf};

use fontcraft_tables::convert::{PsSource, read_postscript};
use fontcraft_tables::merge::{
    Duplicates, LayoutMode, MapHeader, MergeOptions, glyph_font_dicts, merge_with_report, parse_cid_info, parse_glyph_map,
};
use fontcraft_tables::reorder::{glyph_code_points, glyph_names};
use fontcraft_tables::sfnt::parse_tag;
use fontcraft_tables::ttx::{ImportOptions, import_tree};
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

/// The `to` values the result can be converted to.
const TARGETS: [&str; 5] = ["ttf", "otf", "otf-cff2", "cff", "cff2"];
/// Longest PostScript name.
const MAX_PS_NAME: usize = 127;
/// Most Font DICTs a CID-keyed CFF font has (an FDSelect entry is one byte).
const MAX_FONT_DICTS: usize = 256;

fn merge_fonts(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.merge", v)?;
    p.str("output")?;
    let request = OutputRequest::from_params(&p)?;
    let layout = p.opt_str("layout")?.map(LayoutMode::parse).transpose().map_err(|e| p.bad(e.to_string()))?;
    let duplicates = p.opt_str("duplicates")?.map(Duplicates::parse).transpose().map_err(|e| p.bad(e.to_string()))?;
    let drop_tables = match p.opt_strings("dropTables")? {
        None => Vec::new(),
        Some(tags) => tags.iter().map(|t| parse_tag(t)).collect::<std::result::Result<_, _>>().map_err(|e| p.bad(e.to_string()))?,
    };
    let ps_name = p.opt_str("psName")?.map(str::to_string);
    if ps_name.as_ref().is_some_and(|n| n.is_empty() || n.len() > MAX_PS_NAME || !n.bytes().all(|b| b.is_ascii_graphic())) {
        return Err(p.bad(format!("`psName` must be printable ASCII without spaces, at most {MAX_PS_NAME} bytes")));
    }
    let language_group = p.opt_int("languageGroup", 0, 1)?.and_then(|g| u8::try_from(g).ok());
    let to = p.opt_str("to")?;
    if let Some(t) = to
        && !TARGETS.contains(&t)
    {
        return Err(p.bad(format!("`to` must be one of {}, not `{t}`", TARGETS.join(", "))));
    }
    let cid_info = match p.opt_str("cidInfo")? {
        None => None,
        Some(path) => {
            let text = String::from_utf8(file_io::read_input(Path::new(path))?.bytes)
                .map_err(|_| p.bad(format!("the cidfontinfo file {path} is not UTF-8 text")))?;
            Some(parse_cid_info(&text).map_err(|e| p.bad(format!("{path}: {e}")))?)
        }
    };
    let mut notes: Vec<String> = Vec::new();
    let hint_dict_from = match p.opt_str("hintDictFrom")? {
        None => None,
        Some(path) => Some(read_font(Path::new(path), None, "the hint font", &mut notes)?.0),
    };
    let items = p.array("fonts")?;
    if items.len() > fontcraft_tables::limits::MAX_MERGE_FONTS {
        return Err(p.bad(format!("`fonts` lists more than {} fonts", fontcraft_tables::limits::MAX_MERGE_FONTS)));
    }

    let mut fonts: Vec<SfntFont> = Vec::with_capacity(items.len());
    let mut glyph_maps: Vec<Vec<(String, String)>> = Vec::with_capacity(items.len());
    let mut headers: Vec<MapHeader> = Vec::with_capacity(items.len());
    let mut first_program: Option<PsSource> = None;
    for (i, item) in items.iter().enumerate() {
        let read = read_item(&p, i, item, &mut notes)?;
        if i == 0 {
            first_program = read.program;
        }
        fonts.push(read.font);
        glyph_maps.push(read.map);
        headers.push(read.header);
    }
    if glyph_maps.iter().all(Vec::is_empty) {
        glyph_maps.clear();
    }
    let options = MergeOptions {
        glyph_maps,
        drop_tables,
        layout,
        duplicates,
        timestamp: Some(super::file::default_timestamp()),
        headers,
        cid_info,
        ps_name,
        language_group,
        hint_dict_from,
    };
    let result = merge_with_report(&fonts, &options)?;
    let mut font = result.font;
    if let Some(path) = p.opt_str("importTtx")? {
        let root = super::tables_import::read_dump(Path::new(path))?;
        font = import_tree(font, &root, &ImportOptions::default())?;
    }
    // A font program in, a font program out: Type 1 is read, not written, so it becomes CFF.
    let target = to.or(first_program.map(|_| "cff"));
    if to.is_none() && first_program == Some(PsSource::Type1) {
        notes.push("the first font is a Type 1 font, which is read, not written: the result is a bare CFF font program".into());
    }
    let (bytes, left_behind) = match target {
        Some(t) => super::convert_file::convert_to(&font, t)?,
        None => (fontcraft_tables::write(&font, &WriteOptions { order: TableOrder::Recommended })?, Vec::new()),
    };
    let extension = target.map(|t| match t {
        "ttf" => "ttf",
        "cff" => "cff",
        "cff2" => "cff2",
        _ => "otf",
    });
    let first_input = item_path(&p, items.first().unwrap_or(&Value::Null))?;
    let path = file_io::output_path(&first_input, &request, ".merged", extension)?;
    file_io::write_output(&path, &bytes, &request)?;
    let mut warnings = notes;
    warnings.extend(result.warnings);
    warnings.extend(left_behind);
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

/// A compiled font (face `face` of a collection), or a Type 1 font or bare CFF table wrapped in
/// an sfnt, with the kind of font program it was.
fn read_font(path: &Path, face: Option<&Params<'_>>, what: &str, notes: &mut Vec<String>) -> Result<(SfntFont, Option<PsSource>)> {
    let file = file_io::read_input(path)?;
    if file.kind.is_font() {
        let index = match face {
            Some(p) => super::dehint::face_index(p, &file.bytes)?,
            None => 0,
        };
        return Ok((fontcraft_tables::read(&file.bytes, index)?, None));
    }
    let Some(program) = PsSource::sniff(&file.bytes) else {
        return Err(EngineError::Unsupported(format!("merging {}: give compiled fonts, Type 1 fonts or bare CFF tables", path.display())));
    };
    if face.is_some_and(|p| p.get("face").is_some()) {
        return Err(EngineError::Unsupported(format!("`face` ({what}): {} holds one font", path.display())));
    }
    let input = read_postscript(&file.bytes)?;
    notes.extend(input.notes.iter().map(|n| format!("{what}: {n}")));
    Ok((input.font, Some(program)))
}

/// One `fonts` item read.
struct Item {
    font: SfntFont,
    /// Empty: every glyph under its own name.
    map: Vec<(String, String)>,
    header: MapHeader,
    /// The kind of font program, for a Type 1 font or bare CFF table.
    program: Option<PsSource>,
}

/// One input font with its glyph map and map header.
fn read_item(p: &Params<'_>, index: usize, item: &Value, notes: &mut Vec<String>) -> Result<Item> {
    let path = item_path(p, item)?;
    let empty = Value::Null;
    let item_params = Params::new("font.merge", if item.is_object() { item } else { &empty })?;
    let (font, program) = read_font(&path, Some(&item_params), &format!("font {}", index + 1), notes)?;
    let names = glyph_names(&font)?;
    let font_dicts = glyph_font_dicts(&font)?;
    // A CID-keyed font's glyphs are also found by their CIDs.
    let find = |name: &str| -> Option<usize> {
        names.iter().position(|n| n == name).or_else(|| {
            let cid = name.parse::<u16>().ok().filter(|_| font_dicts.is_some())?;
            let canonical = if cid == 0 { ".notdef".to_string() } else { format!("cid{cid:05}") };
            names.iter().position(|n| *n == canonical)
        })
    };

    let mut map: Vec<(String, String)> = Vec::new();
    let mut header = MapHeader::default();
    if let Some(map_path) = item_params.opt_str("map")? {
        let text = file_io::read_input(&PathBuf::from(map_path))?;
        let text = String::from_utf8(text.bytes).map_err(|_| p.bad(format!("the glyph map {map_path} is not UTF-8 text")))?;
        let parsed = parse_glyph_map(&text).map_err(|e| p.bad(format!("{map_path}: {e}")))?;
        header = MapHeader { ps_name: parsed.ps_name, language_group: parsed.language_group };
        map = parsed.entries;
    }
    let include = selector_items(&item_params, "glyphs")?;
    let exclude = selector_items(&item_params, "excludeGlyphs")?;
    let fd = font_dict_list(&item_params, "fd")?;
    let exclude_fd = font_dict_list(&item_params, "excludeFd")?;
    if include.is_none() && exclude.is_none() && fd.is_none() && exclude_fd.is_none() {
        return Ok(Item { font, map, header, program });
    }
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
    if fd.is_some() || exclude_fd.is_some() {
        let Some(dicts) = &font_dicts else {
            return Err(p.bad(format!("font {}: `fd` and `excludeFd` select Font DICTs of a CID-keyed font; this font is name-keyed", index + 1)));
        };
        for (c, d) in chosen.iter_mut().zip(dicts) {
            let d = u64::from(*d);
            *c &= fd.as_ref().is_none_or(|list| list.contains(&d)) && !exclude_fd.as_ref().is_some_and(|list| list.contains(&d));
        }
    }
    let is_chosen = |name: &str| find(name).and_then(|g| chosen.get(g).copied()).unwrap_or(false);
    if map.is_empty() {
        map = names.iter().filter(|n| is_chosen(n)).map(|n| (n.clone(), n.clone())).collect();
    } else {
        map.retain(|(_, source)| is_chosen(source));
    }
    if map.is_empty() {
        return Err(p.bad(format!("font {}: the selection takes no glyph", index + 1)));
    }
    Ok(Item { font, map, header, program })
}

/// A list of Font DICT indices.
fn font_dict_list(p: &Params<'_>, key: &str) -> Result<Option<Vec<u64>>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => {
            if a.len() > MAX_FONT_DICTS {
                return Err(p.bad(format!("`{key}` lists more than {MAX_FONT_DICTS} Font DICTs")));
            }
            a.iter().map(|v| v.as_u64().ok_or_else(|| p.bad(format!("`{key}` items must be Font DICT numbers")))).collect::<Result<_>>().map(Some)
        }
        Some(_) => Err(p.bad(format!("`{key}` must be a list of Font DICT numbers"))),
    }
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
        flavoured_bytes(family, glyphs, Flavour::TrueType)
    }

    fn flavoured_bytes(family: &str, glyphs: &[(&str, u32, f64)], flavour: Flavour) -> Vec<u8> {
        let mut f = Font::new(family);
        for (name, cp, width) in glyphs {
            if f.glyph(name).is_none() {
                f.add_glyph(name, vec![*cp], *width).unwrap();
            }
            let mut g = f.glyph_mut(name).unwrap();
            g.layers[0].width = *width;
            g.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 700.0)));
        }
        compile(&f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap().bytes
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
                &json!({"fonts": [a.display().to_string(), b.display().to_string()], "output": out.display().to_string(), "psName": "X-Regular"}),
            )
            .unwrap_err();
        assert!(e.to_string().contains("CFF"), "{e}");
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

    #[test]
    fn cff_fonts_merge_into_a_cid_keyed_font_program() {
        let dir = std::env::temp_dir().join(format!("fontcraft-merge-cid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("A.otf"), dir.join("B.otf"));
        std::fs::write(&a, flavoured_bytes("Cid A", &[("A", 0x41, 600.0)], Flavour::Cff)).unwrap();
        std::fs::write(&b, flavoured_bytes("Cid B", &[("B", 0x42, 500.0)], Flavour::Cff)).unwrap();
        let (map_a, map_b, info) = (dir.join("a.map"), dir.join("b.map"), dir.join("cidfontinfo"));
        std::fs::write(&map_a, "mergefonts\n0 .notdef\n1 A\n").unwrap();
        std::fs::write(&map_b, "mergefonts PartB 1\n2 B\n").unwrap();
        std::fs::write(
            &info,
            "FontName (Merged-CID)\nFullName (Merged CID)\nFamilyName (Merged)\nWeight (Regular)\nversion (1.000)\n\
             Registry (Adobe)\nOrdering (Identity)\nSupplement 0\nAdobeCopyright (Nobody)\n",
        )
        .unwrap();
        let out = dir.join("merged.cff");
        let mut session = Session::new();
        let result = session
            .execute(
                "font.merge",
                &json!({
                    "fonts": [{"path": a.display().to_string(), "map": map_a.display().to_string()}, {"path": b.display().to_string(), "map": map_b.display().to_string()}],
                    "output": out.display().to_string(),
                    "cidInfo": info.display().to_string(),
                    "to": "cff",
                }),
            )
            .unwrap();
        assert_eq!(result["glyphCount"], json!(3));
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(bytes.first(), Some(&1), "a bare CFF table (major version 1)");
        assert!(bytes.windows(b"PartB".len()).any(|w| w == b"PartB"), "the second map's header names its Font DICT");

        // fd selects Font DICTs of a CID-keyed font only.
        let e = session
            .execute(
                "font.merge",
                &json!({"fonts": [{"path": a.display().to_string(), "fd": [0]}], "output": dir.join("x.otf").display().to_string()}),
            )
            .unwrap_err();
        assert!(e.to_string().contains("name-keyed"), "{e}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
