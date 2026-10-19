//! `font.subset` (M12.17–M12.19; `fontcraft_tables::subset`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. Compiled fonts with TrueType or CFF outlines are
//! subset with their layout, variation, colour, bitmap and math tables, CFF2 outlines included;
//! the tables the subsetter does not know are dropped (the result lists them).

use std::path::PathBuf;

use fontcraft_tables::sfnt::{parse_tag, tag_str};
use fontcraft_tables::subset::{SubsetOptions, subset};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most items one list parameter may hold, ranges counted by their size.
const MAX_LIST_ITEMS: usize = 1_114_112;

/// Parameters of the row that are not built yet: giving one is refused rather than ignored.
const NOT_BUILT: [&str; 14] = [
    "excludeGlyphs",
    "fd",
    "excludeFd",
    "random",
    "goadb",
    "exportFlags",
    "noSubsetTables",
    "obfuscateNames",
    "recalcBounds",
    "recalcAverageWidth",
    "recalcMaxContext",
    "canonicalOrder",
    "container",
    "prettySvg",
];

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.subset",
        "Subset Font",
        [],
        None,
        "{font: path, face?: int|name, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool, keep?: {glyphs?: [name]|\"*\", gids?: [int|\"a-b\"], unicodes?: [int|\"U+XXXX\"|\"XXXX-YYYY\"]|\"*\", text?: string}, ignoreMissingGlyphs?: bool, ignoreMissingUnicodes?: bool, retainGids?: bool, notdefGlyph?: bool, notdefOutline?: bool, recommendedGlyphs?: bool, bidiClosure?: bool, layoutClosure?: bool, layoutFeatures?: [tag]|\"*\"|{add?: [tag], remove?: [tag]}, layoutScripts?: [string]|\"*\", hinting?: bool, hintingTables?: [tag], desubroutinize?: bool, dropTables?: [tag], passthroughTables?: bool, legacyKern?: bool, nameIds?: [int]|\"*\", nameLanguages?: [int]|\"*\", nameLegacy?: bool, glyphNames?: bool, legacyCmap?: bool, symbolCmap?: bool, pruneUnicodeRanges?: bool, pruneCodepageRanges?: bool}",
        always,
        run_subset
    )
    .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
    .writes_files()]
}

fn run_subset(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.subset", v)?;
    for key in NOT_BUILT {
        if p.get(key).is_some() {
            return Err(EngineError::Unsupported(format!("`{key}` of font.subset is not built yet")));
        }
    }
    let input = PathBuf::from(p.str("font")?);
    let request = OutputRequest::from_params(&p)?;
    let options = options(&p)?;

    let file = file_io::read_input(&input)?;
    if !file.kind.is_font() {
        return Err(EngineError::Unsupported(format!("subsetting {}: give a compiled font (subsetting sources is not built yet)", input.display())));
    }
    let face = super::dehint::face_index(&p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    let result = subset(&font, &options)?;
    let bytes = fontcraft_tables::write(&result.font, &WriteOptions { order: TableOrder::Keep })?;

    let extension = match file.kind {
        InputKind::TrueType | InputKind::Cff => None,
        _ if result.font.is_cff() => Some("otf"),
        _ => Some("ttf"),
    };
    let path = file_io::output_path(&input, &request, ".subset", extension)?;
    file_io::write_output(&path, &bytes, &request)?;

    let tables: Vec<Value> = font
        .tags()
        .into_iter()
        .map(|t| json!({"tag": tag_str(t), "before": font.get(t).map_or(0, <[u8]>::len), "after": result.font.get(t).map_or(0, <[u8]>::len)}))
        .collect();
    Ok(json!({
        "output": path.display().to_string(),
        "bytes": bytes.len(),
        "glyphCount": result.retained.len(),
        "retained": result.retained,
        "glyphMap": result.glyph_map.iter().map(|(o, n)| json!([o, n])).collect::<Vec<_>>(),
        "tables": tables,
        "dropped": result.dropped.iter().map(|d| json!({"tag": tag_str(d.tag), "reason": d.reason})).collect::<Vec<_>>(),
        "missingGlyphs": result.missing_glyphs,
        "missingUnicodes": result.missing_unicodes.iter().map(|u| format!("U+{u:04X}")).collect::<Vec<_>>(),
    }))
}

/// The subsetter options the parameters ask for; defaults follow the reference subsetter.
fn options(p: &Params<'_>) -> Result<SubsetOptions> {
    let mut o = SubsetOptions::default();
    if let Some(keep) = p.opt_object("keep")? {
        read_keep(p, keep, &mut o)?;
    }
    o.ignore_missing_glyphs = p.bool_or("ignoreMissingGlyphs", o.ignore_missing_glyphs)?;
    o.ignore_missing_unicodes = p.bool_or("ignoreMissingUnicodes", o.ignore_missing_unicodes)?;
    o.retain_gids = p.bool_or("retainGids", o.retain_gids)?;
    o.notdef_glyph = p.bool_or("notdefGlyph", o.notdef_glyph)?;
    o.notdef_outline = p.bool_or("notdefOutline", o.notdef_outline)?;
    o.recommended_glyphs = p.bool_or("recommendedGlyphs", o.recommended_glyphs)?;
    o.bidi_closure = p.bool_or("bidiClosure", o.bidi_closure)?;
    o.layout_closure = p.bool_or("layoutClosure", o.layout_closure)?;
    o.layout_features = layout_features(p)?;
    match p.get("layoutScripts") {
        None => {}
        Some(Value::String(s)) if s == "*" => o.layout_scripts = None,
        Some(_) => o.layout_scripts = p.opt_strings("layoutScripts")?.map(|v| v.into_iter().map(str::to_string).collect()),
    }
    o.hinting = p.bool_or("hinting", o.hinting)?;
    o.desubroutinize = p.bool_or("desubroutinize", o.desubroutinize)?;
    o.passthrough_tables = p.bool_or("passthroughTables", o.passthrough_tables)?;
    o.legacy_kern = p.bool_or("legacyKern", o.legacy_kern)?;
    o.name_legacy = p.bool_or("nameLegacy", o.name_legacy)?;
    o.glyph_names = p.bool_or("glyphNames", o.glyph_names)?;
    o.legacy_cmap = p.bool_or("legacyCmap", o.legacy_cmap)?;
    o.symbol_cmap = p.bool_or("symbolCmap", o.symbol_cmap)?;
    o.prune_unicode_ranges = p.bool_or("pruneUnicodeRanges", o.prune_unicode_ranges)?;
    o.prune_codepage_ranges = p.bool_or("pruneCodepageRanges", o.prune_codepage_ranges)?;
    if let Some(tags) = p.opt_strings("hintingTables")? {
        o.hinting_tables = tags.iter().map(|t| parse_tag(t)).collect::<std::result::Result<_, _>>().map_err(|e| p.bad(e.to_string()))?;
    }
    if let Some(tags) = p.opt_strings("dropTables")? {
        o.drop_tables = tags.iter().map(|t| parse_tag(t)).collect::<std::result::Result<_, _>>().map_err(|e| p.bad(e.to_string()))?;
    }
    match p.get("nameIds") {
        None => {}
        Some(Value::String(s)) if s == "*" => o.all_name_ids = true,
        Some(v) => o.name_ids = Some(u16_list(p, "nameIds", v)?),
    }
    match p.get("nameLanguages") {
        None => {}
        Some(Value::String(s)) if s == "*" => o.name_languages = Some(Vec::new()),
        Some(v) => o.name_languages = Some(u16_list(p, "nameLanguages", v)?),
    }
    Ok(o)
}

/// `layoutFeatures`: a list, `"*"` for all, or `{add, remove}` applied to the default list.
fn layout_features(p: &Params<'_>) -> Result<Option<Vec<String>>> {
    let tags = |v: Option<&Value>, key: &str| -> Result<Vec<String>> {
        match v {
            None => Ok(Vec::new()),
            Some(Value::Array(a)) => {
                a.iter().map(|t| t.as_str().map(str::to_string).ok_or_else(|| p.bad(format!("`{key}` items must be tags")))).collect()
            }
            Some(_) => Err(p.bad(format!("`{key}` must be a list of tags"))),
        }
    };
    match p.get("layoutFeatures") {
        None => Ok(None),
        Some(Value::String(s)) if s == "*" => Ok(Some(vec!["*".to_string()])),
        Some(Value::Object(edit)) => {
            for key in edit.keys() {
                if key != "add" && key != "remove" {
                    return Err(p.bad(format!("`layoutFeatures.{key}` is not a parameter; give add or remove")));
                }
            }
            let remove = tags(edit.get("remove"), "layoutFeatures.remove")?;
            let mut list: Vec<String> = fontcraft_tables::subset::layout::DEFAULT_LAYOUT_FEATURES.iter().map(|t| t.to_string()).collect();
            list.extend(tags(edit.get("add"), "layoutFeatures.add")?);
            list.retain(|t| !remove.contains(t));
            Ok(Some(list))
        }
        v => Ok(Some(tags(v, "layoutFeatures")?)),
    }
}

fn read_keep(p: &Params<'_>, keep: &Map<String, Value>, o: &mut SubsetOptions) -> Result<()> {
    for key in keep.keys() {
        if !matches!(key.as_str(), "glyphs" | "gids" | "unicodes" | "text") {
            return Err(EngineError::Unsupported(format!("`keep.{key}` of font.subset is not built yet")));
        }
    }
    match keep.get("glyphs") {
        None => {}
        Some(Value::String(s)) if s == "*" => o.all_glyphs = true,
        Some(Value::String(s)) => o.glyphs = s.split([',', ' ']).filter(|w| !w.is_empty()).map(str::to_string).collect(),
        Some(Value::Array(a)) => {
            for item in a {
                let name = item.as_str().ok_or_else(|| p.bad("`keep.glyphs` items must be glyph names"))?;
                if name == "*" {
                    o.all_glyphs = true;
                } else {
                    o.glyphs.push(name.to_string());
                }
            }
        }
        Some(_) => return Err(p.bad("`keep.glyphs` must be a list of glyph names or \"*\"")),
    }
    if let Some(v) = keep.get("gids") {
        o.gids = number_list(p, "keep.gids", v, 10)?;
    }
    match keep.get("unicodes") {
        None => {}
        Some(Value::String(s)) if s == "*" => o.all_unicodes = true,
        Some(v) => o.unicodes = number_list(p, "keep.unicodes", v, 16)?,
    }
    if let Some(v) = keep.get("text") {
        o.text = v.as_str().ok_or_else(|| p.bad("`keep.text` must be a string"))?.to_string();
    }
    Ok(())
}

/// A list of numbers and ranges: JSON integers, or strings in base `radix` (`U+` and `0x`
/// prefixes allowed for base 16) with `a-b` ranges, given as a list or one comma- or
/// space-separated string.
fn number_list(p: &Params<'_>, key: &str, v: &Value, radix: u32) -> Result<Vec<u32>> {
    let mut words: Vec<String> = Vec::new();
    let mut out: Vec<u32> = Vec::new();
    match v {
        Value::String(s) => words.extend(s.split([',', ' ']).filter(|w| !w.is_empty()).map(str::to_string)),
        Value::Array(a) => {
            for item in a {
                match item {
                    Value::Number(n) => {
                        out.push(n.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| p.bad(format!("`{key}` holds a number out of range")))?)
                    }
                    Value::String(s) => words.push(s.clone()),
                    _ => return Err(p.bad(format!("`{key}` items must be numbers or strings"))),
                }
            }
        }
        _ => return Err(p.bad(format!("`{key}` must be a list or a string"))),
    }
    let number = |w: &str| -> Result<u32> {
        let w = w.trim();
        let digits =
            if radix == 16 { w.trim_start_matches("U+").trim_start_matches("u+").trim_start_matches("0x").trim_start_matches("0X") } else { w };
        u32::from_str_radix(digits, radix).map_err(|_| p.bad(format!("`{w}` in `{key}` is not a number")))
    };
    for w in &words {
        match w.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (number(a)?, number(b)?);
                if a > b {
                    return Err(p.bad(format!("the range `{w}` in `{key}` runs backwards")));
                }
                if out.len() + (b - a) as usize >= MAX_LIST_ITEMS {
                    return Err(p.bad(format!("`{key}` lists more than {MAX_LIST_ITEMS} items")));
                }
                out.extend(a..=b);
            }
            None => out.push(number(w)?),
        }
        if out.len() > MAX_LIST_ITEMS {
            return Err(p.bad(format!("`{key}` lists more than {MAX_LIST_ITEMS} items")));
        }
    }
    Ok(out)
}

fn u16_list(p: &Params<'_>, key: &str, v: &Value) -> Result<Vec<u16>> {
    number_list(p, key, v, 10)?.into_iter().map(|n| u16::try_from(n).map_err(|_| p.bad(format!("`{key}` holds {n}, past 65535")))).collect()
}
