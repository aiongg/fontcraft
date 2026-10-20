//! `font.transform` (M12.24; `fontcraft_tables::transform`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! A compiled font takes every option. A source takes `unitsPerEm` only, through the
//! document-level scaling of `font.info.scaleToUnitsPerEm` (one implementation per action);
//! its glyph transforms belong to `path.transformations` in the editor. `to` converts the
//! transformed font's outlines as `font.convert` does with its defaults. `glyphTable` reads a
//! per-glyph table: the listed glyphs take their own shifts and advances, then the font keeps
//! only them (a subset with the subsetter's defaults, glyph names kept) and the glyphs whose
//! output name differs are renamed. `fd` and `excludeFd` narrow the transformed glyphs of a CFF or
//! CFF2 font to some Font DICTs.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use fontcraft_tables::convert::{ConvertOptions, ConvertTarget, convert_with};
use fontcraft_tables::rename::{RenameOptions, rename_glyphs};
use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::subset::{SubsetOptions, subset};
use fontcraft_tables::transform::{
    GlyphKey, GlyphTableEntry, MAX_GLYPH_TABLE_BYTES, TransformOptions, TransformReport, Widths, glyph_cids, parse_glyph_table, settings_by_glyph,
    transform_with_report,
};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most names one glyph list may hold.
const MAX_GLYPH_NAMES: usize = 65_536;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.transform",
        "Transform Font",
        [],
        None,
        "{font: path, face?: int|name, matrix?: [a, b, c, d, e, f] (x' = a·x + c·y + e, y' = b·x + d·y + f), rotate?: degrees clockwise, translate?: {x, y} | [x, y], scale?: number | {x, y?} | [x, y], slant?: degrees, scaleEm?: bool, unitsPerEm?: 16–16384, widths?: \"auto\"|\"keep\"|\"transform\"|\"em\" (auto: transformed when the matrix's b is 0, else the em), glyphs?: [names], excludeGlyphs?: [names], fd?: [int], excludeFd?: [int] (CFF: only the glyphs of these Font DICTs, or not of these, are transformed), glyphTable?: path (lines of 'source output advance|None x y'; '#' comments; CIDs for a CID-keyed font; .notdef listed; only listed glyphs are transformed and kept, each with its own shift and advance, renamed to its output name), to?: \"ttf\"|\"otf\"|\"otf-cff2\" (convert the outlines afterwards, as font.convert does), output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. The named parts apply in the order scale, slant, rotate, translate; matrix excludes them. A compiled TrueType font: the matrix transforms the selected glyphs (transformed glyphs lose their instructions; composites keep their components), then unitsPerEm scales outlines, metrics, kerning, positioning, variation deltas and cvt and keeps instructions. A CFF or CFF2 font: the matrix transforms the selected glyphs and their stem hints (kept and turned for quarter turns, vertical ones dropped under a horizontal shear, all dropped at other angles), and unitsPerEm scales charstrings, hint values and metrics; the table is written without subroutines. A source takes unitsPerEm only. The input is never changed → {output, matrix, unitsPerEm: {from, to} | null, hintsKept, glyphsChanged, tablesChanged, tablesRemoved, warnings}",
        always,
        transform_font
    )
    .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
    .writes_files()]
}

fn transform_font(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.transform", v)?;
    let input = PathBuf::from(p.str("font")?);
    let request = OutputRequest::from_params(&p)?;
    let mut options = options(&p)?;
    let to = target(&p)?;
    let source_to = || p.bad("`to` converts compiled fonts; convert a source with font.convert");
    let table = read_glyph_table(&p)?;
    if input.is_dir() {
        if to.is_some() {
            return Err(source_to());
        }
        return transform_source(&p, &input, &options, &request);
    }
    let file = file_io::read_input(&input)?;
    match file.kind {
        InputKind::Other if to.is_some() => return Err(source_to()),
        InputKind::Other => return transform_source(&p, &input, &options, &request),
        InputKind::Ttx => return Err(EngineError::Unsupported("transforming TTX files: compile the TTX first".into())),
        _ => {}
    }
    let face = super::dehint::face_index(&p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    options.glyphs = selection(&p, &font)?;
    if let Some(entries) = &table {
        options.per_glyph = Some(table_settings(&p, &font, entries)?);
    }
    let (mut out, mut report) = transform_with_report(&font, &options)?;
    if let Some(entries) = &table {
        out = keep_listed(&p, out, &options, entries, &mut report)?;
    }
    let mut converted = false;
    if let Some((target, outlines)) = to
        && outline_tag(&out) != outlines
    {
        let c = convert_with(&out, &ConvertOptions::new(target))?;
        out = c.font.ok_or_else(|| EngineError::Internal("the conversion gave no font".into()))?;
        report.warnings.extend(c.left_behind.into_iter().map(|l| format!("left behind by the conversion: {l}")));
        converted = true;
    }
    let bytes = fontcraft_tables::write(&out, &WriteOptions { order: TableOrder::Keep })?;
    let extension = match file.kind {
        InputKind::TrueType | InputKind::Cff if !converted => None,
        _ if out.is_cff() => Some("otf"),
        _ => Some("ttf"),
    };
    let path = file_io::output_path(&input, &request, ".transformed", extension)?;
    file_io::write_output(&path, &bytes, &request)?;
    Ok(result(&path, &report))
}

/// The table holding a font's outlines.
fn outline_tag(font: &fontcraft_tables::SfntFont) -> [u8; 4] {
    if font.get(*b"CFF2").is_some() {
        *b"CFF2"
    } else if font.get(*b"CFF ").is_some() {
        *b"CFF "
    } else {
        *b"glyf"
    }
}

/// The conversion `to` asks for, with the table that will hold the outlines. TrueType curves
/// keep within one font unit, `font.convert`'s default.
fn target(p: &Params<'_>) -> Result<Option<(ConvertTarget, [u8; 4])>> {
    Ok(match p.opt_str("to")? {
        None => None,
        Some("ttf") => Some((ConvertTarget::TrueType { max_error: 1.0 }, *b"glyf")),
        Some("otf") => Some((ConvertTarget::Cff, *b"CFF ")),
        Some("otf-cff2") => Some((ConvertTarget::Cff2, *b"CFF2")),
        Some(other) => return Err(p.bad(format!("`to` must be ttf, otf or otf-cff2, not `{other}`"))),
    })
}

fn result(path: &Path, r: &TransformReport) -> Value {
    json!({
        "output": path.display().to_string(),
        "matrix": r.matrix,
        "unitsPerEm": r.units_per_em.map(|(from, to)| json!({"from": from, "to": to})),
        "hintsKept": r.hints_kept,
        "glyphsChanged": r.glyphs_changed,
        "tablesChanged": r.tables_changed.iter().map(|t| tag_str(*t)).collect::<Vec<_>>(),
        "tablesRemoved": r.tables_removed.iter().map(|t| tag_str(*t)).collect::<Vec<_>>(),
        "warnings": r.warnings,
    })
}

/// A pair given as `[x, y]`, `{x, y}` or, when `single` is allowed, one number for both.
fn pair(p: &Params<'_>, key: &str, single: bool) -> Result<Option<(f64, f64)>> {
    let finite = |v: &Value| v.as_f64().filter(|f| f.is_finite());
    let bad = || p.bad(format!("`{key}` must be {}[x, y] or {{x, y}} with finite numbers", if single { "a number, " } else { "" }));
    match p.get(key) {
        None => Ok(None),
        Some(v @ Value::Number(_)) if single => finite(v).map(|f| Some((f, f))).ok_or_else(bad),
        Some(Value::Array(a)) => match a.as_slice() {
            [x, y] => Ok(Some((finite(x).ok_or_else(bad)?, finite(y).ok_or_else(bad)?))),
            _ => Err(bad()),
        },
        Some(Value::Object(o)) => {
            let x = o.get("x").and_then(finite).ok_or_else(bad)?;
            let y = match o.get("y") {
                None if single => x,
                None => return Err(bad()),
                Some(y) => finite(y).ok_or_else(bad)?,
            };
            Ok(Some((x, y)))
        }
        Some(_) => Err(bad()),
    }
}

fn options(p: &Params<'_>) -> Result<TransformOptions> {
    let matrix = match p.opt_array("matrix")? {
        None => None,
        Some(a) => {
            let values: Vec<f64> = a.iter().filter_map(|v| v.as_f64().filter(|f| f.is_finite())).collect();
            let m: [f64; 6] = values.try_into().map_err(|_| p.bad("`matrix` must hold six finite numbers"))?;
            Some(m)
        }
    };
    let finite = |key: &str| -> Result<Option<f64>> {
        match p.opt_f64(key)? {
            Some(v) if !v.is_finite() => Err(p.bad(format!("`{key}` must be a finite number"))),
            other => Ok(other),
        }
    };
    let widths = match p.opt_str("widths")? {
        None => Widths::Auto,
        Some(text) => Widths::parse(text).map_err(|e| p.bad(e.to_string()))?,
    };
    let units_per_em = p
        .opt_int("unitsPerEm", i64::from(fontcraft_tables::limits::MIN_UNITS_PER_EM), i64::from(fontcraft_tables::limits::MAX_UNITS_PER_EM))?
        .and_then(|v| u16::try_from(v).ok());
    Ok(TransformOptions {
        matrix,
        scale: pair(p, "scale", true)?,
        slant: finite("slant")?,
        rotate: finite("rotate")?,
        translate: pair(p, "translate", false)?,
        scale_em: p.bool_or("scaleEm", false)?,
        widths,
        glyphs: None,
        per_glyph: None,
        fds: fd_list(p, "fd")?,
        exclude_fds: fd_list(p, "excludeFd")?,
        units_per_em,
    })
}

/// A list of Font DICT indexes.
fn fd_list(p: &Params<'_>, key: &str) -> Result<Option<BTreeSet<u16>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_GLYPH_NAMES {
        return Err(p.bad(format!("`{key}` lists more than {MAX_GLYPH_NAMES} Font DICTs")));
    }
    list.iter()
        .map(|v| v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or_else(|| p.bad(format!("`{key}` must list Font DICT indexes from 0 to 65,535"))))
        .collect::<Result<BTreeSet<u16>>>()
        .map(Some)
}

fn names(p: &Params<'_>, key: &str) -> Result<Option<Vec<String>>> {
    let Some(list) = p.opt_strings(key)? else { return Ok(None) };
    if list.len() > MAX_GLYPH_NAMES {
        return Err(p.bad(format!("`{key}` lists more than {MAX_GLYPH_NAMES} names")));
    }
    Ok(Some(list.into_iter().map(str::to_string).collect()))
}

/// Each glyph name of a font with its glyph id (the first glyph when names repeat).
fn glyph_ids_by_name(sfnt: &fontcraft_tables::SfntFont) -> Result<HashMap<String, u16>> {
    // A WOFF, WOFF2 or collection input is looked at as the plain font of the chosen face.
    let bytes = fontcraft_tables::write(sfnt, &WriteOptions::default())?;
    let font = skrifa::FontRef::new(&bytes).map_err(|e| EngineError::Io(format!("the font cannot be read: {e}")))?;
    let glyph_names = skrifa::GlyphNames::new(&font);
    let mut by_name = HashMap::new();
    for (gid, name) in glyph_names.iter() {
        if let Ok(g) = u16::try_from(gid.to_u32()) {
            by_name.entry(name.as_str().to_string()).or_insert(g);
        }
    }
    Ok(by_name)
}

/// The glyph ids `glyphs` and `excludeGlyphs` name; `None` when neither is given.
fn selection(p: &Params<'_>, sfnt: &fontcraft_tables::SfntFont) -> Result<Option<BTreeSet<u16>>> {
    let include = names(p, "glyphs")?;
    let exclude = names(p, "excludeGlyphs")?;
    if include.is_none() && exclude.is_none() {
        return Ok(None);
    }
    let by_name = glyph_ids_by_name(sfnt)?;
    let resolve = |list: &[String]| -> Result<BTreeSet<u16>> {
        list.iter().map(|n| by_name.get(n).copied().ok_or_else(|| p.bad(format!("the font has no glyph named `{n}`")))).collect()
    };
    let mut set = match &include {
        Some(list) => resolve(list)?,
        None => by_name.values().copied().collect(),
    };
    if let Some(list) = &exclude {
        for g in resolve(list)? {
            set.remove(&g);
        }
    }
    Ok(Some(set))
}

/// The lines of the glyph table file `glyphTable` names, when it names one.
fn read_glyph_table(p: &Params<'_>) -> Result<Option<Vec<GlyphTableEntry>>> {
    let Some(path) = p.opt_str("glyphTable")? else { return Ok(None) };
    let file = file_io::read_input_up_to(Path::new(path), MAX_GLYPH_TABLE_BYTES)?;
    if file.bytes.len() > MAX_GLYPH_TABLE_BYTES {
        return Err(p.bad(format!("the glyph table is larger than {MAX_GLYPH_TABLE_BYTES} bytes")));
    }
    let text = String::from_utf8(file.bytes).map_err(|_| p.bad("the glyph table is not UTF-8 text"))?;
    let entries = parse_glyph_table(&text).map_err(|e| p.bad(e.to_string()))?;
    if entries.is_empty() {
        return Err(p.bad("the glyph table lists no glyphs"));
    }
    Ok(Some(entries))
}

/// The glyph table's settings by glyph id; glyph 0 (`.notdef`) must be listed.
fn table_settings(
    p: &Params<'_>,
    font: &fontcraft_tables::SfntFont,
    entries: &[GlyphTableEntry],
) -> Result<BTreeMap<u16, fontcraft_tables::transform::GlyphSettings>> {
    let by_name = glyph_ids_by_name(font)?;
    let by_cid: HashMap<u16, u16> = match glyph_cids(font)? {
        Some(cids) => cids.iter().enumerate().rev().filter_map(|(g, c)| u16::try_from(g).ok().map(|g| (*c, g))).collect(),
        None => HashMap::new(),
    };
    if entries.iter().any(|e| matches!(e.source, GlyphKey::Cid(_))) && by_cid.is_empty() {
        return Err(p.bad("the glyph table names CIDs, which only a CID-keyed CFF font has"));
    }
    let settings = settings_by_glyph(entries, |key| match key {
        GlyphKey::Name(n) => by_name.get(n).copied(),
        GlyphKey::Cid(c) => by_cid.get(c).copied(),
    })
    .map_err(|e| p.bad(e.to_string()))?;
    if !settings.contains_key(&0) {
        return Err(p.bad("the glyph table must list .notdef (glyph 0)"));
    }
    Ok(settings)
}

/// The transformed font with only the glyphs the table lists, renamed as it says.
fn keep_listed(
    p: &Params<'_>,
    font: fontcraft_tables::SfntFont,
    options: &TransformOptions,
    entries: &[GlyphTableEntry],
    report: &mut TransformReport,
) -> Result<fontcraft_tables::SfntFont> {
    let listed: Vec<u32> = options.per_glyph.iter().flat_map(|t| t.keys()).map(|g| u32::from(*g)).collect();
    let count = font.get(*b"maxp").and_then(|m| m.get(4..6)).map_or(0, |b| usize::from(u16::from_be_bytes([b[0], b[1]])));
    let mut font = font;
    if listed.len() < count {
        let options = SubsetOptions {
            gids: listed,
            layout_closure: false,
            bidi_closure: false,
            notdef_outline: true,
            glyph_names: true,
            all_name_ids: true,
            all_name_languages: true,
            legacy_kern: true,
            ..SubsetOptions::default()
        };
        let result = subset(&font, &options)?;
        report
            .warnings
            .extend(result.dropped.iter().map(|d| format!("the subset to the listed glyphs drops {}: {}", tag_str(d.tag).trim_end(), d.reason)));
        font = result.font;
    }
    let mut map = Vec::new();
    for e in entries {
        match (&e.source, &e.output) {
            (GlyphKey::Name(from), GlyphKey::Name(to)) if from != to => map.push((from.clone(), to.clone())),
            (GlyphKey::Cid(from), GlyphKey::Cid(to)) if from != to => {
                return Err(EngineError::Unsupported(format!("giving glyph CID {from} the CID {to} in a glyph table")));
            }
            (GlyphKey::Name(_), GlyphKey::Cid(_)) | (GlyphKey::Cid(_), GlyphKey::Name(_)) => {
                return Err(p.bad("a glyph table line names its source and its output glyph differently"));
            }
            _ => {}
        }
    }
    if map.is_empty() {
        return Ok(font);
    }
    let renamed = rename_glyphs(font, &RenameOptions { map, ..RenameOptions::default() })?;
    Ok(renamed.font)
}

#[cfg(not(target_arch = "wasm32"))]
fn transform_source(p: &Params<'_>, input: &Path, o: &TransformOptions, request: &OutputRequest) -> Result<Value> {
    use crate::io::SaveFormat;
    let named = o.matrix.is_some() || o.scale.is_some() || o.slant.is_some() || o.rotate.is_some() || o.translate.is_some() || o.scale_em;
    let narrowed = ["glyphs", "excludeGlyphs", "glyphTable", "fd", "excludeFd"].iter().any(|k| p.get(k).is_some());
    if named || narrowed {
        return Err(EngineError::Unsupported("transforming the glyphs of a source in font.transform: use path.transformations on its glyphs".into()));
    }
    let upm = o.units_per_em.ok_or_else(|| p.bad("a source takes `unitsPerEm`"))?;
    let (font, _) = crate::io::open(input, None)?;
    if font.source.format == fontcraft_model::SourceFormat::Binary {
        return Err(EngineError::Unsupported("this kind of compiled font".into()));
    }
    let previous = font.units_per_em;
    let scaled = fontcraft_fontinfo::scale::scale_font(&font, upm).map_err(|e| p.bad(e.to_string()))?;
    let output = file_io::output_path(input, request, ".transformed", None)?;
    let format = SaveFormat::of_path(&output)
        .or_else(|| SaveFormat::of_source(font.source.format))
        .ok_or_else(|| EngineError::Io(format!("cannot tell which format to save {} in", output.display())))?;
    if request.no_clobber && !request.in_place && output.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", output.display())));
    }
    crate::io::save(&scaled, &output, format)?;
    let report = TransformReport {
        units_per_em: (previous != upm).then_some((previous, upm)),
        hints_kept: true,
        glyphs_changed: if previous != upm { scaled.glyphs.len() } else { 0 },
        ..TransformReport::default()
    };
    Ok(result(&output, &report))
}

#[cfg(target_arch = "wasm32")]
fn transform_source(_: &Params<'_>, _: &Path, _: &TransformOptions, _: &OutputRequest) -> Result<Value> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}

#[cfg(test)]
#[path = "transform_file_tests.rs"]
mod tests;
