//! `font.transform` (M12.24; `fontcraft_tables::transform`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! A compiled TrueType font takes every option; a CFF or CFF2 font takes `unitsPerEm`. A source
//! takes `unitsPerEm` only, through the
//! document-level scaling of `font.info.scaleToUnitsPerEm` (one implementation per action);
//! its glyph transforms belong to `path.transformations` in the editor. The per-glyph table,
//! the FD selectors and a change of format are refused for now.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::transform::{TransformOptions, TransformReport, Widths, transform_with_report};
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
        "{font: path, face?: int|name, matrix?: [a, b, c, d, e, f] (x' = a·x + c·y + e, y' = b·x + d·y + f), rotate?: degrees clockwise, translate?: {x, y} | [x, y], scale?: number | {x, y?} | [x, y], slant?: degrees, scaleEm?: bool, unitsPerEm?: 16–16384, widths?: \"auto\"|\"keep\"|\"transform\"|\"em\" (auto: transformed when the matrix's b is 0, else the em), glyphs?: [names], excludeGlyphs?: [names], output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. The named parts apply in the order scale, slant, rotate, translate; matrix excludes them. A compiled TrueType font: the matrix transforms the selected glyphs (transformed glyphs lose their instructions; composites keep their components), then unitsPerEm scales outlines, metrics, kerning, positioning, variation deltas and cvt and keeps instructions. A CFF or CFF2 font takes unitsPerEm: charstrings, hint values and metrics scale, and the table is written without subroutines. A source takes unitsPerEm only. The input is never changed → {output, matrix, unitsPerEm: {from, to} | null, hintsKept, glyphsChanged, tablesChanged, tablesRemoved, warnings}",
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
    for (key, why) in [
        ("glyphTable", "the per-glyph table"),
        ("fd", "selecting font dictionaries"),
        ("excludeFd", "selecting font dictionaries"),
        ("to", "a change of format (convert the result with font.convert)"),
    ] {
        if p.get(key).is_some() {
            return Err(EngineError::Unsupported(format!("{why} in font.transform")));
        }
    }
    let mut options = options(&p)?;
    if input.is_dir() {
        return transform_source(&p, &input, &options, &request);
    }
    let file = file_io::read_input(&input)?;
    match file.kind {
        InputKind::Other => return transform_source(&p, &input, &options, &request),
        InputKind::Ttx => return Err(EngineError::Unsupported("transforming TTX files: compile the TTX first".into())),
        _ => {}
    }
    let face = super::dehint::face_index(&p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    options.glyphs = selection(&p, &font)?;
    let (out, report) = transform_with_report(&font, &options)?;
    let bytes = fontcraft_tables::write(&out, &WriteOptions { order: TableOrder::Keep })?;
    let extension = match file.kind {
        InputKind::TrueType | InputKind::Cff => None,
        _ if out.is_cff() => Some("otf"),
        _ => Some("ttf"),
    };
    let path = file_io::output_path(&input, &request, ".transformed", extension)?;
    file_io::write_output(&path, &bytes, &request)?;
    Ok(result(&path, &report))
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
        units_per_em,
    })
}

fn names(p: &Params<'_>, key: &str) -> Result<Option<Vec<String>>> {
    let Some(list) = p.opt_strings(key)? else { return Ok(None) };
    if list.len() > MAX_GLYPH_NAMES {
        return Err(p.bad(format!("`{key}` lists more than {MAX_GLYPH_NAMES} names")));
    }
    Ok(Some(list.into_iter().map(str::to_string).collect()))
}

/// The glyph ids `glyphs` and `excludeGlyphs` name; `None` when neither is given.
fn selection(p: &Params<'_>, sfnt: &fontcraft_tables::SfntFont) -> Result<Option<BTreeSet<u16>>> {
    let include = names(p, "glyphs")?;
    let exclude = names(p, "excludeGlyphs")?;
    if include.is_none() && exclude.is_none() {
        return Ok(None);
    }
    // A WOFF, WOFF2 or collection input is looked at as the plain font of the chosen face.
    let bytes = fontcraft_tables::write(sfnt, &WriteOptions::default())?;
    let font = skrifa::FontRef::new(&bytes).map_err(|e| EngineError::Io(format!("the font cannot be read: {e}")))?;
    let glyph_names = skrifa::GlyphNames::new(&font);
    let mut by_name = std::collections::HashMap::new();
    for (gid, name) in glyph_names.iter() {
        if let Ok(g) = u16::try_from(gid.to_u32()) {
            by_name.entry(name.as_str().to_string()).or_insert(g);
        }
    }
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

#[cfg(not(target_arch = "wasm32"))]
fn transform_source(p: &Params<'_>, input: &Path, o: &TransformOptions, request: &OutputRequest) -> Result<Value> {
    use crate::io::SaveFormat;
    let named = o.matrix.is_some() || o.scale.is_some() || o.slant.is_some() || o.rotate.is_some() || o.translate.is_some() || o.scale_em;
    if named || p.get("glyphs").is_some() || p.get("excludeGlyphs").is_some() {
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
