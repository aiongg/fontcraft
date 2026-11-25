//! The formats of `font.compile` fontmake calls `otf-cff2`, `ttf-interpolatable` and
//! `otf-interpolatable` (M2.9): a static CFF font converted to CFF2, and the masters written as
//! fonts whose outlines interpolate.
//!
//! Row: `compile.outputFormats`.

use fontcraft_compile::var::interpolatable::{self, MasterSource};
use fontcraft_compile::{Flavour, Options};
use fontcraft_model::{Font, Target};
use fontcraft_tables::convert::{ConvertOptions, ConvertTarget, convert_with};
use serde_json::{Value, json};

use super::compile::{Container, MAX_FONTS, Report, Request, Sink, format_name, outlines_name, stem};
use super::compile_select::{FolderKind, OutputLayout, by_format_folder, plan_path};
use crate::{EngineError, Result};

/// The format name of a static font of `flavour` in a request that writes CFF outlines as CFF2
/// (`cff2`) or not.
pub(super) fn static_format(flavour: Flavour, cff2: bool) -> &'static str {
    if cff2 && flavour == Flavour::Cff { "otf-cff2" } else { format_name(flavour) }
}

/// The format name of interpolatable masters.
pub(super) fn masters_format(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::TrueType => "ttf-interpolatable",
        Flavour::Cff => "otf-interpolatable",
    }
}

/// `bytes` (a compiled CFF font) with its outlines in `CFF2`, as `font.convert` converts them: the
/// hints kept, the glyph names in `post` format 2. Subroutinised again when the CFF table is.
/// `notes` receives what CFF2 cannot hold.
pub(super) fn to_cff2(bytes: &[u8], options: &Options, notes: &mut Vec<String>) -> Result<Vec<u8>> {
    let failed = |e: fontcraft_tables::TablesError| EngineError::Internal(format!("converting the compiled font to CFF2: {e}"));
    let font = fontcraft_tables::read(bytes, 0).map_err(failed)?;
    let subroutinize = font.get(*b"CFF ").is_some_and(fontcraft_compile::cff::has_subroutines);
    let wanted =
        ConvertOptions { subroutinize: Some(subroutinize), max_subrs: Some(options.cff.max_subrs), ..ConvertOptions::new(ConvertTarget::Cff2) };
    let converted = convert_with(&font, &wanted).map_err(failed)?;
    notes.extend(converted.left_behind.into_iter().map(|l| format!("left out of the CFF2 font: {l}")));
    notes.extend(converted.warnings);
    Ok(converted.bytes)
}

/// `fonts` (the results of static fonts, or their plans) with the format names of CFF2 fonts, when
/// the request writes CFF outlines as CFF2.
pub(super) fn name_cff2(fonts: &mut [Value]) {
    for font in fonts.iter_mut() {
        let Value::Object(m) = font else { continue };
        if m.get("format").and_then(Value::as_str) != Some(format_name(Flavour::Cff)) {
            continue;
        }
        m.insert("format".into(), json!(static_format(Flavour::Cff, true)));
        if m.contains_key("outlines") {
            m.insert("outlines".into(), json!("cff2"));
        }
        if let Some(Value::Array(tables)) = m.get_mut("tables") {
            for t in tables.iter_mut().filter(|t| t.as_str() == Some("CFF ")) {
                *t = json!("CFF2");
            }
            // As the tables are listed for every other font: in the file's order, by tag.
            tables.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
}

/// The file a master font of `stem` is written to, relative to the destination folder.
fn file_name(req: &Request, stem: &str, flavour: Flavour, container: Container) -> String {
    let ext = container.extension(flavour);
    match req.naming.layout {
        OutputLayout::ByFormat => format!("{}/{stem}.{ext}", by_format_folder(FolderKind::Interpolatable, format_name(flavour))),
        OutputLayout::Flat if req.naming.split_masters => format!("{}/{stem}.{ext}", masters_format(flavour)),
        OutputLayout::Flat => format!("{stem}.{ext}"),
    }
}

/// How one master font is named: its label in the result and its file name stem. A sparse font
/// has its master's, with the location as fontmake appends it.
fn names(font: &Font, title: &str, source: &MasterSource, req: &Request) -> Result<(String, String, &'static str)> {
    let target = Target::Master(source.master.clone());
    let resolved = fontcraft_compile::drawing::resolve(font, &target)?;
    let label = font.masters.iter().find(|m| m.id == source.master).map(|m| m.name.clone()).unwrap_or_default();
    let stem = stem(font, &resolved, &req.options, title, req.naming.postscript_names);
    Ok(match &source.intermediate {
        None => (label, stem, "master"),
        Some(at) => (format!("{label} {at}"), format!("{stem}-{at}"), "intermediate"),
    })
}

/// The master fonts a request for interpolatable masters writes in each of `flavours`; refuses
/// more than [`MAX_FONTS`].
pub(super) fn sources(font: &Font, flavours: &[Flavour]) -> Result<Vec<MasterSource>> {
    let sources = interpolatable::master_sources(font)?;
    let n = sources.len().saturating_mul(flavours.len());
    if n > MAX_FONTS {
        return Err(EngineError::Unsupported(format!("building {n} master fonts in one command (at most {MAX_FONTS})")));
    }
    Ok(sources)
}

/// The plan's description of the master fonts a request builds.
pub(super) fn plan(font: &Font, title: &str, flavours: &[Flavour], req: &Request) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    if flavours.is_empty() {
        return Ok(out);
    }
    let sources = sources(font, flavours)?;
    for &flavour in flavours {
        for source in &sources {
            let (label, stem, kind) = names(font, title, source, req)?;
            let files: Vec<Value> = req
                .containers
                .iter()
                .map(|&c| {
                    let name = file_name(req, &stem, flavour, c);
                    match plan_path(&req.dest, &name) {
                        Some(path) => json!({ "name": name, "container": c.name(), "path": path.to_string_lossy() }),
                        None => json!({ "name": name, "container": c.name() }),
                    }
                })
                .collect();
            out.push(json!({
                "instance": label, "source": kind, "format": masters_format(flavour), "removeOverlap": false, "files": files,
            }));
        }
    }
    Ok(out)
}

/// Builds the masters of `font` (its linked sources already read) in each of `flavours` into
/// `sink`, adding their results to `fonts`.
pub(super) fn build(
    font: &Font,
    title: &str,
    flavours: &[Flavour],
    req: &Request,
    sink: &mut Sink,
    report: &mut Report,
    fonts: &mut Vec<Value>,
) -> Result<()> {
    if flavours.is_empty() {
        return Ok(());
    }
    sources(font, flavours)?;
    for &flavour in flavours {
        for built in interpolatable::compile_masters(font, flavour, &req.options)? {
            let (label, stem, kind) = names(font, title, &built.source, req)?;
            if report.feature_code.is_none() {
                report.feature_code.clone_from(&built.compiled.feature_code);
            }
            let mut files = Vec::with_capacity(req.containers.len());
            for &container in &req.containers {
                let data = container.wrap(&built.compiled.bytes)?;
                let file = sink.put(file_name(req, &stem, flavour, container), container, data, &mut report.warnings)?;
                let mut v = json!({ "name": file.name, "container": container.name(), "bytes": file.bytes });
                if let (Some(path), Value::Object(m)) = (&file.path, &mut v) {
                    m.insert("path".into(), json!(path.to_string_lossy()));
                }
                files.push(v);
            }
            fonts.push(json!({
                "instance": label, "source": kind, "format": masters_format(flavour), "outlines": outlines_name(flavour),
                "glyphs": built.compiled.glyph_order.len(), "warnings": built.compiled.warnings, "files": files,
            }));
        }
    }
    Ok(())
}
