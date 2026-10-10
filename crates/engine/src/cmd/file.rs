//! File menu: new, open, save, close, export.
//!
//! Reading and writing files happens here only in native builds (through [`crate::io`]); on the
//! web the shell hands bytes to [`crate::Session::add_document`] and takes the compiled fonts from
//! `file.exportStatic` without a destination, as dataBase64. Building and naming the exported
//! fonts is shared with `font.compile` (`compile.rs`).

use base64::Engine as _;
use fontcraft_compile::{Flavour, Options};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use fontcraft_model::Font;
use serde_json::{Value, json};

use super::compile::{self, Dest, Pick, Report, Request, Sink};
use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.new",
            "New",
            ["File"],
            Some("Cmd+N"),
            "{familyName?: string} a new untitled font with one Regular master and the starting glyph set (.notdef, space, A–Z, a–z) → {title, glyphs}",
            always,
            new
        ),
        cmd!(
            "file.open",
            "Open…",
            ["File"],
            Some("Cmd+O"),
            "{path: string, face?: number} open a font as a new document, recognising it by content: a .fontcraft file, a .glyphs file (format versions 2 to 4), a .glyphspackage folder, a UFO folder (versions 2 and 3), a .designspace file with the UFOs it names (one document, a master per source), or a compiled font (TrueType, OpenType with CFF, a collection member chosen by face, WOFF, WOFF2). A compiled font opens view-only: save it with Save As to a source format. warnings lists what reading a compiled font left out → {title, path, format, version?, glyphs, masters, viewOnly, warnings}",
            always,
            open
        ),
        cmd!(
            "file.save",
            "Save",
            ["File"],
            Some("Cmd+S"),
            "{path?: string, force?: bool} write the document to its file in the format it was opened in (or to `path`, choosing the format from its extension). Refuses when the file changed on disk since it was read (for a designspace: the document or any of its UFOs), unless force is true. Single files are written atomically; a package or UFO rewrites only changed files; a designspace writes its UFOs, then the document → {path, format, version?, bytes, filesWritten, filesRemoved, losses}",
            has_doc,
            save
        ),
        cmd!(
            "file.saveAs",
            "Save As…",
            ["File"],
            Some("Cmd+Shift+S"),
            "{path: string, format?: \"fontcraft\"|\"glyphs\"|\"glyphspackage\"|\"ufo\"|\"designspace\", version?: 2|3|4, compress?: bool, force?: bool, smartComponents?: \"components\"|\"outlines\"} write the document to a new file and make that its file; format defaults to the path's extension, version to the one the document has (3 for a new document; .glyphs has 2 to 4, UFO 2 and 3). A single UFO holds one master; a designspace writes a UFO per master beside it. compress: gzip a .fontcraft file (default: as it was opened). smartComponents (UFO and designspace): keep smart components as components with their values in the glyph lib (default), or draw them as outlines interpolated from the smart glyph's pole layers; the document keeps its components. losses lists what the format cannot hold → {path, format, version?, bytes, filesWritten, filesRemoved, losses}",
            has_doc,
            save_as
        ),
        cmd!(
            "file.exportUfo",
            "Export UFO",
            [],
            None,
            "{destination: string, masters?: [string], productionNames?: bool, decomposeSmartStuff?: bool} write each chosen master (default: all) as its own UFO 3 into the destination folder, named Family-Style.ufo. The document keeps its file and format. productionNames: rename glyphs to production names (not supported yet). decomposeSmartStuff: draw smart components as outlines interpolated from the smart glyph's pole layers (default true); false keeps them as components with their values in the glyph lib → {files: [{path, master, filesWritten}], losses}",
            has_doc,
            export_ufo
        ),
        cmd!(
            "file.close",
            "Close",
            ["File"],
            Some("Cmd+W"),
            "{discard?: bool} close the active document; a document with unsaved changes needs discard: true → {closed: title}",
            has_doc,
            close
        ),
        cmd!(
            "file.exportStatic",
            "Export Static Fonts",
            [],
            None,
            "{instances?: [string], outlines?: \"truetype\"|\"cff\", containers?: [\"plain\"|\"woff\"|\"woff2\"], destination?: string, removeOverlap?: bool, autohint?: bool, timestamp?: number|\"now\"} compile the active document's exportable instances (or the named ones; a font without instances exports its first master) to static fonts with TrueType (.ttf) or CFF (.otf) outlines, in each container asked for (default plain; WOFF and WOFF2 are .woff and .woff2). Instances between masters are skipped and listed. destination: a folder, created when missing, where each file is named by the instance's fileName parameter, else its PostScript name (with the document's file name standing in for a missing family name); or one file path ending in .ttf, .otf, .woff or .woff2, which sets the outlines and container when they are not given and receives the first instance that can be built; without it the fonts are returned as dataBase64. Existing files are replaced; a failing export writes nothing. removeOverlap: merge overlapping contours in every glyph (default true; a glyph whose overlaps cannot be removed keeps them, with a warning). autohint: not supported yet. timestamp: seconds since the Unix epoch for the head table (default SOURCE_DATE_EPOCH, else now) → {fonts: [{instance, source, format, outlines, glyphs, warnings, files: [{name, container, bytes, path? | dataBase64?}]}], skipped: [{instance, source, reason}], warnings}; with one file also {path? | dataBase64?, bytes, glyphs, glyphOrder}",
            has_doc,
            export_static
        ),
        cmd!(
            "file.exportAll",
            "Export All",
            ["File"],
            Some("Cmd+Alt+E"),
            "{documents?: [string], outlines?: [\"truetype\"|\"cff\"], containers?: [\"plain\"|\"woff\"|\"woff2\"], destination: string, removeOverlap?: bool, autohint?: bool, timestamp?: number|\"now\"} export every exportable instance of every open document (or of the documents named by title or path) in each outline format and container asked for (default truetype, plain) into the destination folder, as file.exportStatic names them; with both outline formats, the files go into ttf and otf subfolders. Instances between masters are skipped and listed. removeOverlap: as in file.exportStatic (default true). A failing export writes nothing → {documents: [{title, fonts, skipped}], written, warnings}",
            has_doc,
            export_all
        ),
    ]
}

fn new(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.new", p)?;
    let family = p.opt_str("familyName")?.unwrap_or("Untitled");
    if family.trim().is_empty() {
        return Err(p.bad("`familyName` must not be empty"));
    }
    s.add_document(Font::new(family), None)?;
    let doc = s.doc()?;
    Ok(json!({ "title": doc.title, "glyphs": doc.font.glyphs.len() }))
}

#[cfg(not(target_arch = "wasm32"))]
fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.open", p)?;
    let path = std::path::PathBuf::from(p.str("path")?);
    let face = p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32);
    // A file that is already open is shown, not opened a second time (`file.openTwiceActivates`).
    if let Some(i) = super::session::find_open(s, &path) {
        s.set_active(i)?;
        let doc = s.doc()?;
        return Ok(
            json!({ "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "alreadyOpen": true, "glyphs": doc.font.glyphs.len() }),
        );
    }
    let (font, warnings) = crate::io::open(&path, face)?;
    // Every reader checks the font it returns.
    s.push_document(font, Some(path));
    let doc = s.doc()?;
    let save_format = crate::io::SaveFormat::of_source(doc.font.source.format);
    let format = match doc.font.source.format {
        fontcraft_model::SourceFormat::Binary => Some("binary"),
        fontcraft_model::SourceFormat::Ufoz => Some("ufoz"),
        _ => save_format.map(|f| f.name()),
    };
    let version = doc.font.source.glyphs_version.or(doc.font.source.ufo_version);
    Ok(json!({
        "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "format": format,
        "version": version, "glyphs": doc.font.glyphs.len(), "masters": doc.font.masters.len(), "viewOnly": save_format.is_none(), "warnings": warnings,
    }))
}

#[cfg(target_arch = "wasm32")]
fn open(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("opening a file by path in the browser; the web app passes the file's bytes".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_document(s: &mut Session, p: &Params, path: std::path::PathBuf, format: crate::io::SaveFormat, version: Option<u32>) -> Result<Value> {
    use crate::io::{self, SaveFormat};
    let outlines = smart_outlines(p)?;
    if outlines && !matches!(format, SaveFormat::Ufo | SaveFormat::Designspace) {
        return Err(p.bad("`smartComponents` applies to UFO and designspace files"));
    }
    let doc = s.doc()?;
    // Another program may have changed the file since it was read or last written here.
    if doc.path.as_deref() == Some(path.as_path()) && !p.bool_or("force", false)? {
        let now = io::document_stamp(&path, &doc.font);
        if now.is_some() && now != doc.stamp {
            return Err(p.bad(format!("{} changed on disk since it was opened; save elsewhere, or pass force: true to overwrite", path.display())));
        }
    }
    let mut font = Font::clone(&doc.font);
    // The version a format family is written in: the one asked for, else the one read, else the
    // newest that is not a draft (3 for both). Converting between versions is not supported yet.
    let pick = |have: Option<u32>, range: std::ops::RangeInclusive<u32>| -> Result<u32> {
        let want = version.or(have).unwrap_or(3);
        if !range.contains(&want) {
            return Err(p.bad(format!("`version` {want} does not exist for {}", format.name())));
        }
        if have.is_some_and(|h| h != want) {
            return Err(EngineError::Unsupported(format!("converting between format versions ({} to {want})", have.unwrap_or_default())));
        }
        Ok(want)
    };
    let version = match format {
        SaveFormat::Native => None,
        SaveFormat::Glyphs | SaveFormat::GlyphsPackage => {
            let v = pick(font.source.glyphs_version, 2..=4)?;
            font.source.glyphs_version = Some(v);
            Some(v)
        }
        SaveFormat::Ufo | SaveFormat::Designspace => {
            let v = pick(font.source.ufo_version, 2..=3)?;
            font.source.ufo_version = Some(v);
            Some(v)
        }
    };
    font.source.format = match format {
        SaveFormat::Native => font.source.format,
        SaveFormat::Glyphs => fontcraft_model::SourceFormat::Glyphs,
        SaveFormat::GlyphsPackage => fontcraft_model::SourceFormat::GlyphsPackage,
        SaveFormat::Ufo => fontcraft_model::SourceFormat::Ufo,
        SaveFormat::Designspace => fontcraft_model::SourceFormat::Designspace,
    };
    let (smart, smart_losses) = smart_choice(&font, outlines);
    let mut saved = io::save(smart.as_ref().unwrap_or(&font), &path, format)?;
    saved.losses.extend(smart_losses);
    let font = Arc::new(font);
    let doc = s.doc_mut()?;
    doc.title = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| doc.title.clone());
    doc.font = font.clone();
    doc.saved = font;
    doc.path = Some(path.clone());
    doc.stamp = io::document_stamp(&path, &doc.font);
    Ok(json!({
        "path": path.to_string_lossy(), "format": format.name(), "version": version, "bytes": saved.bytes,
        "filesWritten": saved.files_written, "filesRemoved": saved.files_removed, "losses": saved.losses,
    }))
}

/// `smartComponents`: whether a UFO or designspace write draws smart components as outlines.
#[cfg(not(target_arch = "wasm32"))]
fn smart_outlines(p: &Params) -> Result<bool> {
    match p.opt_str("smartComponents")? {
        None | Some("components") => Ok(false),
        Some("outlines") => Ok(true),
        Some(other) => Err(p.bad(format!("`smartComponents` must be \"components\" or \"outlines\", got \"{other}\""))),
    }
}

/// With `outlines`, the font to write with its smart components drawn as outlines, and what that
/// did, for the losses. The document keeps its components.
#[cfg(not(target_arch = "wasm32"))]
fn smart_choice(font: &Font, outlines: bool) -> (Option<Font>, Vec<Value>) {
    if !outlines {
        return (None, Vec::new());
    }
    let (drawn, report) = fontcraft_model::smart::draw_smart_components(font);
    let mut losses = Vec::new();
    if report.drawn > 0 {
        losses.push(json!({ "family": "glyphs", "entries": report.drawn, "what": "smart components written as outlines (the document keeps them as components)" }));
    }
    if !report.kept.is_empty() {
        losses.push(json!({ "family": "glyphs", "entries": report.kept.len(), "what": "smart components written as components because they cannot be drawn as outlines", "items": report.kept }));
    }
    (Some(drawn), losses)
}

#[cfg(not(target_arch = "wasm32"))]
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::io::SaveFormat;
    let p = Params::new("file.save", p)?;
    let (path, format) = match p.opt_str("path")? {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            let format = SaveFormat::of_path(&path).unwrap_or(SaveFormat::Native);
            (path, format)
        }
        None => {
            let doc = s.doc()?;
            let path = doc.path.clone().ok_or_else(|| p.bad("the document has never been saved: give a `path`"))?;
            let format = SaveFormat::of_source(doc.font.source.format)
                .ok_or_else(|| EngineError::Unsupported("saving in the format this document was opened from; use Save As".into()))?;
            (path, format)
        }
    };
    write_document(s, &p, path, format, None)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::io::SaveFormat;
    let p = Params::new("file.saveAs", p)?;
    let path = std::path::PathBuf::from(p.str("path")?);
    let format = match p.opt_str("format")? {
        Some(f) => SaveFormat::parse(f).ok_or_else(|| {
            p.bad(format!("`format` must be \"fontcraft\", \"glyphs\", \"glyphspackage\", \"ufo\" or \"designspace\", got \"{f}\""))
        })?,
        None => SaveFormat::of_path(&path)
            .ok_or_else(|| p.bad("give a `format`, or a path ending in .fontcraft, .glyphs, .glyphspackage, .ufo or .designspace"))?,
    };
    let version = p.opt_int("version", 2, 4)?.map(|v| v as u32);
    if let Some(compress) = p.get("compress") {
        let compress = compress.as_bool().ok_or_else(|| p.bad("`compress` must be true or false"))?;
        if format != SaveFormat::Native && compress {
            return Err(p.bad("only a .fontcraft file can be compressed"));
        }
        // Not an edit: how the file is stored, kept with the document.
        let doc = s.doc_mut()?;
        let mut font = Font::clone(&doc.font);
        font.source.compressed = compress;
        let font = Arc::new(font);
        let unchanged = !doc.modified();
        doc.font = font.clone();
        if unchanged {
            doc.saved = font;
        }
    }
    write_document(s, &p, path, format, version)
}

#[cfg(target_arch = "wasm32")]
fn save(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("saving to a path in the browser; the web app downloads the document's bytes".into()))
}

#[cfg(target_arch = "wasm32")]
fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    save(s, p)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_ufo(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.exportUfo", p)?;
    let destination = std::path::PathBuf::from(p.str("destination")?);
    if p.bool_or("productionNames", false)? {
        return Err(EngineError::Unsupported("renaming glyphs to production names on export (needs the glyph database, ROADMAP.md M4)".into()));
    }
    let outlines = p.bool_or("decomposeSmartStuff", true)?;
    let font = s.font()?;
    let masters: Vec<String> = match p.get("masters") {
        None => font.masters.iter().map(|m| m.id.clone()).collect(),
        Some(Value::Array(ids)) => {
            let mut out = Vec::new();
            for id in ids {
                let id = id.as_str().ok_or_else(|| p.bad("`masters` must be a list of master ids"))?;
                font.master(id)?;
                out.push(id.to_string());
            }
            out
        }
        Some(_) => return Err(p.bad("`masters` must be a list of master ids")),
    };
    if destination.exists() && !destination.is_dir() {
        return Err(p.bad(format!("{} is a file: give a folder", destination.display())));
    }
    std::fs::create_dir_all(&destination).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", destination.display())))?;
    let (smart, smart_losses) = smart_choice(font, outlines);
    let mut exported = crate::io::export_ufos(smart.as_ref().unwrap_or(font), &masters, &destination)?;
    exported.losses.extend(smart_losses);
    let files: Vec<Value> =
        exported.files.iter().map(|(path, master, n)| json!({ "path": path.to_string_lossy(), "master": master, "filesWritten": n })).collect();
    Ok(json!({ "files": files, "losses": exported.losses }))
}

#[cfg(target_arch = "wasm32")]
fn export_ufo(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("writing UFO folders in the browser".into()))
}

fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.close", p)?;
    let doc = s.doc()?;
    if doc.modified() && !p.bool_or("discard", false)? {
        return Err(p.bad(format!("`{}` has unsaved changes: save it first, or pass discard: true", doc.title)));
    }
    let title = doc.title.clone();
    s.close_active()?;
    Ok(json!({ "closed": title }))
}

/// The head-table time when the caller gives none: `SOURCE_DATE_EPOCH` for reproducible builds,
/// else the clock.
pub(super) fn default_timestamp() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(t) = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.trim().parse::<i64>().ok()) {
            return t;
        }
    }
    clock()
}

/// Seconds since the Unix epoch (0 on the web, where the shell passes the time).
pub(super) fn clock() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// The outline format named by `outlines` (one of them, or a list under `many`).
fn flavour(p: &Params, name: &str) -> Result<Flavour> {
    match name {
        "truetype" => Ok(Flavour::TrueType),
        "cff" => Ok(Flavour::Cff),
        other => Err(p.bad(format!("`outlines` must be \"truetype\" or \"cff\", got \"{other}\""))),
    }
}

/// The compile options of the export commands: `timestamp`, `removeOverlap` (default true) and
/// `autohint`.
fn export_options(p: &Params) -> Result<Options> {
    if p.bool_or("autohint", false)? {
        return Err(EngineError::Unsupported("`autohint`: hinting on export (ROADMAP.md, M10)".into()));
    }
    Ok(Options { timestamp: compile::timestamp(p)?, remove_overlap: p.bool_or("removeOverlap", true)?, ..Default::default() })
}

fn export_static(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.exportStatic", p)?;
    let font = s.font()?;
    let dest = Dest::read(&p)?;
    let flavour = match p.opt_str("outlines")? {
        Some(name) => flavour(&p, name)?,
        None if dest.extension() == Some("otf") => Flavour::Cff,
        None => Flavour::TrueType,
    };
    let containers = compile::containers(&p, "containers", &dest)?;
    compile::check_file_destination(&p, &dest, &[flavour], &containers)?;
    let options = export_options(&p)?;
    let pick = Pick::read(&p, "instances")?;
    let jobs = compile::jobs(font, &pick, false, &p)?;
    if jobs.is_empty() {
        return Err(p.bad("`instances` names no instance"));
    }
    let named_several = matches!(pick, Pick::Named(_)) && jobs.len() > 1;
    if matches!(dest, Dest::File(_)) && named_several {
        return Err(p.bad("`destination` is one file, but several instances are named: give a folder"));
    }
    let first_only = matches!(dest, Dest::File(_));
    let req = Request { flavours: vec![flavour], containers, options, dest, first_only };
    compile::check_size(&req, jobs.len())?;
    let mut report = Report::default();
    let mut sink = Sink::new(&req.dest, true)?;
    compile::build(font, &s.doc()?.title, &jobs, &req, &mut sink, &mut report)?;
    if report.fonts.is_empty() {
        return Err(compile::nothing_built(&report));
    }
    sink.commit()?;
    let mut result = json!({ "fonts": report.fonts_json(true), "skipped": report.skipped, "warnings": report.warnings });
    // One file: its path or data, size and glyph order at the top, as callers that export a
    // single font read them.
    if let (Some(font), Value::Object(m)) = (report.fonts.first().filter(|_| report.file_count() == 1), &mut result) {
        if let Some(file) = font.files.first() {
            match (&file.path, &file.data) {
                (Some(path), _) => m.insert("path".into(), json!(path.to_string_lossy())),
                (None, Some(data)) => m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(data))),
                (None, None) => None,
            };
            m.insert("bytes".into(), json!(file.bytes));
        }
        m.insert("glyphs".into(), json!(font.glyph_order.len()));
        m.insert("glyphOrder".into(), json!(font.glyph_order));
    }
    Ok(result)
}

fn export_all(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.exportAll", p)?;
    let docs = s.documents();
    let chosen: Vec<usize> = match p.opt_strings("documents")? {
        None => (0..docs.len()).collect(),
        Some(names) => {
            let mut out = Vec::new();
            for name in names {
                let i = docs
                    .iter()
                    .position(|d| d.title == name || d.path.as_deref().is_some_and(|path| path == std::path::Path::new(name)))
                    .ok_or_else(|| p.bad(format!("no open document is called or saved as `{name}`")))?;
                if !out.contains(&i) {
                    out.push(i);
                }
            }
            out
        }
    };
    if chosen.is_empty() {
        return Err(EngineError::NoDocument);
    }
    let dest = Dest::read(&p)?;
    match dest {
        Dest::Folder(_) => {}
        Dest::Memory => return Err(p.bad("give a `destination` folder")),
        Dest::File(_) => return Err(p.bad("`destination` must be a folder: Export All writes several files")),
    }
    let mut flavours = Vec::new();
    for name in p.opt_strings("outlines")?.unwrap_or_else(|| vec!["truetype"]) {
        let f = flavour(&p, name)?;
        if !flavours.contains(&f) {
            flavours.push(f);
        }
    }
    if flavours.is_empty() {
        return Err(p.bad("`outlines` names no outline format"));
    }
    let containers = compile::containers(&p, "containers", &dest)?;
    let options = export_options(&p)?;
    let req = Request { flavours, containers, options, dest, first_only: false };
    let mut jobs = Vec::with_capacity(chosen.len());
    for &i in &chosen {
        let doc = docs.get(i).ok_or(EngineError::NoDocument)?;
        jobs.push(compile::jobs(&doc.font, &Pick::Exportable, false, &p)?);
    }
    compile::check_size(&req, jobs.iter().map(Vec::len).sum())?;
    let mut sink = Sink::new(&req.dest, false)?;
    let mut warnings: Vec<String> = Vec::new();
    let mut reports = Vec::with_capacity(chosen.len());
    for (&i, jobs) in chosen.iter().zip(&jobs) {
        let doc = docs.get(i).ok_or(EngineError::NoDocument)?;
        let mut report = Report::default();
        compile::build(&doc.font, &doc.title, jobs, &req, &mut sink, &mut report)?;
        warnings.append(&mut report.warnings);
        reports.push((doc.title.clone(), report));
    }
    let written: usize = reports.iter().map(|(_, r)| r.file_count()).sum();
    if written == 0 {
        let skipped = Report { skipped: reports.into_iter().flat_map(|(_, r)| r.skipped).collect(), ..Default::default() };
        return Err(compile::nothing_built(&skipped));
    }
    sink.commit()?;
    let documents: Vec<Value> =
        reports.iter().map(|(title, r)| json!({ "title": title, "fonts": r.fonts_json(false), "skipped": r.skipped })).collect();
    Ok(json!({ "documents": documents, "written": written, "warnings": warnings }))
}
