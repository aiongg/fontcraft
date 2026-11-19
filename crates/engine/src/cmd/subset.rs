//! `font.subset` (M12.17–M12.19; `fontcraft_tables::subset`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. Compiled fonts with TrueType or CFF outlines are
//! subset with their layout, variation, colour, bitmap and math tables, CFF2 outlines included;
//! the tables the subsetter does not know are dropped (the result lists them).

use std::path::PathBuf;

use fontcraft_tables::sfnt::{Tag, parse_tag, tag_str};
use fontcraft_tables::subset::{
    DEFAULT_DROP_TABLES, DEFAULT_HINTING_TABLES, DEFAULT_NAME_IDS, DEFAULT_NAME_LANGUAGES, DEFAULT_NO_SUBSET_TABLES, SubsetOptions, subset,
};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most items one list parameter may hold, ranges counted by their size.
const MAX_LIST_ITEMS: usize = 1_114_112;

/// Parameters of the row that are not built yet: giving one is refused rather than ignored.
const NOT_BUILT: [&str; 6] = ["excludeGlyphs", "fd", "excludeFd", "random", "goadb", "exportFlags"];

/// The container of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Plain,
    Woff,
    Woff2,
}

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.subset",
        "Subset Font",
        [],
        None,
        "{font: path, face?: int|name, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool, keep?: {glyphs?: [name]|\"*\", gids?: [int|\"a-b\"], unicodes?: [int|\"U+XXXX\"|\"XXXX-YYYY\"]|\"*\", text?: string}, ignoreMissingGlyphs?: bool, ignoreMissingUnicodes?: bool, retainGids?: bool, notdefGlyph?: bool, notdefOutline?: bool, recommendedGlyphs?: bool, bidiClosure?: bool, layoutClosure?: bool, layoutFeatures?: [tag]|\"*\"|{add?: [tag], remove?: [tag]}, layoutScripts?: [string]|\"*\"|{add?: [string], remove?: [string]}, hinting?: bool, hintingTables?: [tag]|{add?: [tag], remove?: [tag]}, desubroutinize?: bool, dropTables?: [tag]|{add?: [tag], remove?: [tag]}, passthroughTables?: bool, legacyKern?: bool, nameIds?: [int]|\"*\"|{add?: [int], remove?: [int]}, nameLanguages?: [int]|\"*\"|{add?: [int], remove?: [int]}, nameLegacy?: bool, glyphNames?: bool, legacyCmap?: bool, symbolCmap?: bool, pruneUnicodeRanges?: bool, pruneCodepageRanges?: bool, noSubsetTables?: [tag]|{add?: [tag], remove?: [tag]}, obfuscateNames?: bool, recalcBounds?: bool, timestamp?: \"keep\"|\"now\"|int, recalcAverageWidth?: bool, recalcMaxContext?: bool, canonicalOrder?: bool, container?: \"plain\"|\"woff\"|\"woff2\", zopfli?: bool, prettySvg?: bool} every list parameter takes a list, which replaces its default, or {add, remove}, which edits the default; dropTables defaults to JSTF DSIG EBDT EBLC EBSC PCLT LTSH Feat Glat Gloc Silf Sill, noSubsetTables to the tables copied as they are (avar BASE fvar gasp head hhea maxp vhea OS/2 loca name cvt fpgm prep VDMX DSIG CPAL MVAR cvar STAT: one left out is dropped unless passthroughTables, an unknown one added is kept as it is), hintingTables to cvt cvar fpgm prep hdmx VDMX. timestamp sets the modification time in seconds since 1970, now meaning SOURCE_DATE_EPOCH, else the clock (default keep); canonicalOrder writes the tables in the recommended order instead of the input's; container defaults to the input's (zopfli compression is not available); prettySvg writes subset SVG documents again with two-space indentation → {output, bytes, glyphCount, retained, glyphMap, tables, dropped, missingGlyphs, missingUnicodes, warnings}",
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
    let container = match p.opt_str("container")? {
        None => match file.kind {
            InputKind::Woff => Container::Woff,
            InputKind::Woff2 => Container::Woff2,
            _ => Container::Plain,
        },
        Some("plain" | "none") => Container::Plain,
        Some("woff") => Container::Woff,
        Some("woff2") => Container::Woff2,
        Some(other) => return Err(p.bad(format!("`container` is plain, woff or woff2, not `{other}`"))),
    };
    if p.bool_or("zopfli", false)? && container == Container::Woff {
        return Err(EngineError::Unsupported("zopfli compression for WOFF is not available; leave out zopfli to use zlib".into()));
    }
    let order = if p.bool_or("canonicalOrder", false)? { TableOrder::Recommended } else { TableOrder::Keep };
    let face = super::dehint::face_index(&p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    let result = subset(&font, &options)?;
    let sfnt = fontcraft_tables::write(&result.font, &WriteOptions { order })?;
    let bytes = match container {
        Container::Plain => sfnt,
        Container::Woff => {
            fontcraft_container::woff::encode(&sfnt, &extras(file.kind, &file.bytes)).map_err(|e| EngineError::Io(format!("WOFF: {e}")))?
        }
        Container::Woff2 => {
            let options =
                fontcraft_container::woff2::Options { extras: extras(file.kind, &file.bytes), ..fontcraft_container::woff2::Options::default() };
            fontcraft_container::woff2::encode(&sfnt, &options).map_err(|e| EngineError::Io(format!("WOFF2: {e}")))?
        }
    };

    let extension = match (container, file.kind) {
        (Container::Woff, _) => Some("woff"),
        (Container::Woff2, _) => Some("woff2"),
        (_, InputKind::TrueType | InputKind::Cff) => None,
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
        "warnings": result.warnings,
    }))
}

/// The version, metadata and private data of a WOFF or WOFF2 input, which a WOFF or WOFF2
/// output keeps; none for other inputs or when they cannot be read.
fn extras(kind: InputKind, bytes: &[u8]) -> fontcraft_container::Extras {
    let none = fontcraft_container::Extras::default();
    match kind {
        InputKind::Woff => fontcraft_container::woff::decode(bytes).map_or(none, |u| fontcraft_container::Extras {
            version: Some(u.version),
            metadata: u.metadata,
            private: u.private,
        }),
        InputKind::Woff2 => {
            let Ok(dir) = fontcraft_container::woff2::directory(bytes) else { return none };
            let Ok((metadata, private)) = fontcraft_container::woff2::extras(bytes, &dir) else { return none };
            fontcraft_container::Extras { version: Some(dir.version), metadata, private }
        }
        _ => none,
    }
}

/// A list parameter: a list replaces `default`, `{add, remove}` edits it. Items compare as
/// text; `None` when the parameter is absent.
fn edited_list(p: &Params<'_>, key: &str, default: &[String]) -> Result<Option<Vec<Value>>> {
    let items = |v: Option<&Value>, what: &str| -> Result<Vec<Value>> {
        match v {
            None => Ok(Vec::new()),
            Some(Value::Array(a)) => Ok(a.clone()),
            Some(_) => Err(p.bad(format!("`{what}` must be a list"))),
        }
    };
    match p.get(key) {
        None => Ok(None),
        Some(Value::Object(edit)) => {
            for k in edit.keys() {
                if k != "add" && k != "remove" {
                    return Err(p.bad(format!("`{key}.{k}` is not a parameter; give add or remove")));
                }
            }
            let remove = items(edit.get("remove"), &format!("{key}.remove"))?;
            let mut list: Vec<Value> = default.iter().map(|d| json!(d)).collect();
            for v in items(edit.get("add"), &format!("{key}.add"))? {
                if !list.contains(&v) {
                    list.push(v);
                }
            }
            list.retain(|v| !remove.contains(v));
            Ok(Some(list))
        }
        v => Ok(Some(items(v, key)?)),
    }
}

/// A list of table tags (`edited_list`).
fn tag_list(p: &Params<'_>, key: &str, default: &[Tag]) -> Result<Option<Vec<Tag>>> {
    let default: Vec<String> = default.iter().map(|t| tag_str(*t)).collect();
    let Some(items) = edited_list(p, key, &default)? else { return Ok(None) };
    let mut tags = Vec::with_capacity(items.len());
    for item in &items {
        let text = item.as_str().ok_or_else(|| p.bad(format!("`{key}` items must be table tags")))?;
        let tag = parse_tag(text).map_err(|e| p.bad(e.to_string()))?;
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    Ok(Some(tags))
}

/// A list of name or language IDs: `"*"`, a list, or `{add, remove}` on `default`, the IDs
/// compared by value.
fn id_list(p: &Params<'_>, key: &str, default: &[u16]) -> Result<Option<IdList>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if s == "*" => Ok(Some(IdList::All)),
        Some(Value::Object(edit)) => {
            for k in edit.keys() {
                if k != "add" && k != "remove" {
                    return Err(p.bad(format!("`{key}.{k}` is not a parameter; give add or remove")));
                }
            }
            let part = |k: &str| edit.get(k).map(|v| u16_list(p, &format!("{key}.{k}"), v)).transpose().map(Option::unwrap_or_default);
            let (add, remove) = (part("add")?, part("remove")?);
            let mut list = default.to_vec();
            list.extend(add.into_iter().filter(|a| !default.contains(a)));
            list.retain(|v| !remove.contains(v));
            Ok(Some(IdList::Some(list)))
        }
        Some(v) => Ok(Some(IdList::Some(u16_list(p, key, v)?))),
    }
}

enum IdList {
    All,
    Some(Vec<u16>),
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
    if !matches!(p.get("layoutScripts"), Some(Value::String(s)) if s == "*")
        && let Some(scripts) = edited_list(p, "layoutScripts", &["*".to_string()])?
    {
        let scripts: Vec<String> = scripts
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| p.bad("`layoutScripts` items must be strings")))
            .collect::<Result<_>>()?;
        o.layout_scripts = if scripts.iter().any(|s| s == "*") { None } else { Some(scripts) };
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
    o.obfuscate_names = p.bool_or("obfuscateNames", o.obfuscate_names)?;
    o.recalc_bounds = p.bool_or("recalcBounds", o.recalc_bounds)?;
    o.recalc_average_width = p.bool_or("recalcAverageWidth", o.recalc_average_width)?;
    o.recalc_max_context = p.bool_or("recalcMaxContext", o.recalc_max_context)?;
    o.pretty_svg = p.bool_or("prettySvg", o.pretty_svg)?;
    match p.get("timestamp") {
        None => {}
        Some(Value::String(s)) if s == "keep" => {}
        Some(Value::String(s)) if s == "now" => o.recalc_timestamp = Some(super::shared::default_timestamp()),
        // Years 1904 to 9999, which is what the head table's date can sensibly hold.
        Some(_) => o.recalc_timestamp = p.opt_int("timestamp", -2_082_844_800, 253_402_300_799)?,
    }
    if let Some(tags) = tag_list(p, "hintingTables", &DEFAULT_HINTING_TABLES)? {
        o.hinting_tables = tags;
    }
    if let Some(tags) = tag_list(p, "dropTables", &DEFAULT_DROP_TABLES)? {
        o.drop_tables = tags;
    }
    if let Some(tags) = tag_list(p, "noSubsetTables", &DEFAULT_NO_SUBSET_TABLES)? {
        o.no_subset_tables = tags;
    }
    match id_list(p, "nameIds", &DEFAULT_NAME_IDS)? {
        None => {}
        Some(IdList::All) => o.all_name_ids = true,
        Some(IdList::Some(ids)) => o.name_ids = Some(ids),
    }
    match id_list(p, "nameLanguages", &DEFAULT_NAME_LANGUAGES)? {
        None => {}
        Some(IdList::All) => o.all_name_languages = true,
        Some(IdList::Some(ids)) => o.name_languages = Some(ids),
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
