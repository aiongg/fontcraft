//! File menu: the font exports, `file.exportStatic` and `file.exportAll`.
//!
//! On the web the shell takes the compiled fonts from `file.exportStatic` without a destination,
//! as dataBase64. Building and naming the exported fonts is shared with `font.compile`
//! (`compile.rs`).

use base64::Engine as _;
use fontcraft_compile::{Flavour, Options};
use serde_json::{Value, json};

use super::compile::{self, Dest, Pick, Report, Request, Sink};
use super::export_settings::{self, Tab};
use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.exportStatic",
            "Export Static Fonts",
            [],
            None,
            "{instances?: [string], outlines?: \"truetype\"|\"cff\", containers?: [\"plain\"|\"woff\"|\"woff2\"], destination?: string, removeOverlap?: bool, autohint?: bool, cbdt?: bool, timestamp?: number|\"now\", versionStamp?: bool, useDestination?: bool} compile the active document's exportable instances (or the named ones; a font without instances exports its first master) to static fonts with TrueType (.ttf) or CFF (.otf) outlines, in each container asked for (default plain; WOFF and WOFF2 are .woff and .woff2). Instances between masters are skipped and listed. A designspace without axes is refused when it states a location, or when it has two or more sources and instances to build. destination: a folder, created when missing, where each file is named by the instance's fileName parameter, else its PostScript name (with `Unnamed font` standing in for a missing family name in a .glyphs source, and the document's file name in any other), in the sub-folder an instance's Export Folder parameter names (`..` climbs out of the destination); or one file path ending in .ttf, .otf, .woff or .woff2, which sets the outlines and container when they are not given and receives the first instance that can be built; without it the fonts are returned as dataBase64. Existing files are replaced; a failing export writes nothing. removeOverlap: merge overlapping contours in every glyph (default true; a glyph whose overlaps cannot be removed keeps them, with a warning). autohint: with CFF outlines, run the PostScript autohinter on every glyph without manual PostScript hints, except those the Disable autohinting for glyphs parameter lists; with TrueType outlines, run the TrueType autohinter on every glyph with the instance's TTFAutohint options and control instructions, in place of the manual TrueType instructions, and then, when the TTFAutohint binary path parameter names a program (desktop only), that program on the finished font (if it fails, the built-in result is written with a warning) (an instance's Autohint parameter overrides this option; default false). cbdt: also write CBDT and CBLC bitmap tables from the iColor layers (default false). timestamp: seconds since the Unix epoch for the head table (default SOURCE_DATE_EPOCH, else now). versionStamp: append `;FontCraft <version>` to the version string (name ID 5) unless the font states its own (default true). A successful export keeps its outline format, containers, removeOverlap, autohint, cbdt and destination folder for the document as the Export dialog's OTF settings (file.exportSettings), which file.exportAll reuses; useDestination: whether the dialog's Export Destination box was ticked, kept with them (default true) → {fonts: [{instance, source, format, outlines, glyphs, warnings, files: [{name, container, bytes, path? | dataBase64?}]}], skipped: [{instance, source, reason}], warnings}; with one file also {path? | dataBase64?, bytes, glyphs, glyphOrder}",
            has_doc,
            export_static
        ).files(const { &[FileParam::write("destination")] }).writes_files(),
        cmd!(
            "file.exportAll",
            "Export All",
            ["File"],
            Some("Cmd+Alt+E"),
            "{documents?: [string], outlines?: [\"truetype\"|\"cff\"], containers?: [\"plain\"|\"woff\"|\"woff2\"], destination?: string, fallbackDestination?: string, removeOverlap?: bool, autohint?: bool, cbdt?: bool, timestamp?: number|\"now\", versionStamp?: bool} export every exportable instance of every open document (or of the documents named by title or path) without the Export dialog. What the parameters leave out comes, for each document, from the OTF settings of its last successful export (file.exportSettings: outline format, containers, removeOverlap, autohint, cbdt, and the folder while its Export Destination box was ticked), else from the defaults (truetype, plain, removeOverlap true, autohint and cbdt false); each document is written into its own folder, as file.exportStatic names the files; with both outline formats, the files go into ttf and otf subfolders. fallbackDestination: the folder for the documents with no folder from either (the desktop asks for it once); without it such a document is refused, naming it. Instances between masters are skipped and listed. A designspace without axes is refused when it states a location, or when it has two or more sources and instances to build. removeOverlap, autohint: as in file.exportStatic. Existing files are replaced; a failing export writes nothing → {documents: [{title, destination, fonts, skipped}], written, warnings}",
            has_doc,
            export_all
        ).files(const { &[FileParam::write("destination"), FileParam::write("fallbackDestination")] }).writes_files(),
    ]
}

/// The outline format named by `outlines` (one of them, or a list under `many`).
fn flavour(p: &Params, name: &str) -> Result<Flavour> {
    match name {
        "truetype" => Ok(Flavour::TrueType),
        "cff" => Ok(Flavour::Cff),
        other => Err(p.bad(format!("`outlines` must be \"truetype\" or \"cff\", got \"{other}\""))),
    }
}

/// The compile options of the export commands: `timestamp`, `removeOverlap` (default true),
/// `autohint` and `cbdt`.
fn export_options(p: &Params) -> Result<Options> {
    Ok(Options {
        hinting: fontcraft_compile::HintingOptions { autohint: p.bool_or("autohint", false)?, strip: false },
        timestamp: compile::timestamp(p)?,
        remove_overlap: p.bool_or("removeOverlap", true)?,
        cbdt: p.bool_or("cbdt", false)?,
        version_stamp: p.bool_or("versionStamp", true)?,
        ..Default::default()
    })
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
    let switches = [("removeOverlap", options.remove_overlap), ("autohint", options.hinting.autohint), ("cbdt", options.cbdt)];
    let kept = export_settings::ran_with(&p, Tab::Otf, compile::outlines_name(flavour), &containers, &switches, &dest)?;
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
    let req = Request {
        flavours: vec![flavour],
        containers,
        options,
        dest,
        first_only,
        autohinter: crate::external_autohint::trusted(s),
        naming: Default::default(),
        instance_outputs: true,
    };
    compile::check_size(&req, jobs.len())?;
    let mut report = Report::default();
    compile::check_glyph_names(font)?;
    let mut sink = Sink::new(&req.dest, true, s.export_outside())?;
    compile::build(font, &s.doc()?.title, s.doc()?.path.as_deref(), s.glyph_data()?, &jobs, &req, &mut sink, &mut report)?;
    if report.fonts.is_empty() {
        return Err(compile::nothing_built(&report));
    }
    sink.commit()?;
    if let Some(warning) = export_settings::remember(s, Tab::Otf, kept) {
        report.warnings.push(warning);
    }
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

/// The keys of `file.exportAll` that a document's kept OTF settings stand in for.
const KEPT_KEYS: [&str; 6] = ["outlines", "containers", "removeOverlap", "autohint", "cbdt", "destination"];

/// `file.exportAll`'s parameters for one document: what the call gives, else the document's kept OTF
/// settings (the kept folder only while its Export Destination box was ticked).
fn document_params(given: &Value, kept: &serde_json::Map<String, Value>) -> Value {
    let mut out = serde_json::Map::new();
    for key in KEPT_KEYS {
        let Some(value) = kept.get(key) else { continue };
        match key {
            "outlines" => {
                out.insert(key.into(), json!([value]));
            }
            "destination" if kept.get("useDestination") == Some(&Value::Bool(false)) => {}
            _ => {
                out.insert(key.into(), value.clone());
            }
        }
    }
    if let Value::Object(given) = given {
        for (key, value) in given.iter().filter(|(k, v)| !v.is_null() && k.as_str() != "fallbackDestination") {
            out.insert(key.clone(), value.clone());
        }
        if !out.contains_key("destination")
            && let Some(fallback) = given.get("fallbackDestination").filter(|v| !v.is_null())
        {
            out.insert("destination".into(), fallback.clone());
        }
    }
    Value::Object(out)
}

/// One document's request of `file.exportAll`, read from its [`document_params`].
fn document_request(p: &Params, title: &str, timestamp: i64, autohinter: Option<String>) -> Result<Request> {
    let dest = Dest::read(p)?;
    match &dest {
        // A kept folder comes from the settings file, not from the caller's parameters, so the automation roots are
        // checked here as well.
        Dest::Folder(folder) => crate::file_access::check_write(folder).map_err(|e| p.bad(format!("`{title}`: {e}")))?,
        Dest::Memory => {
            return Err(p.bad(format!(
                "`{title}` has no export folder: give a `destination` folder, or export it once from the Export dialog with its Export Destination ticked"
            )));
        }
        Dest::File(_) => return Err(p.bad("`destination` must be a folder: Export All writes several files")),
    }
    let mut flavours = Vec::new();
    for name in p.opt_strings("outlines")?.unwrap_or_else(|| vec!["truetype"]) {
        let f = flavour(p, name)?;
        if !flavours.contains(&f) {
            flavours.push(f);
        }
    }
    if flavours.is_empty() {
        return Err(p.bad("`outlines` names no outline format"));
    }
    let containers = compile::containers(p, "containers", &dest)?;
    let mut options = export_options(p)?;
    options.timestamp = timestamp;
    Ok(Request { flavours, containers, options, dest, first_only: false, autohinter, naming: Default::default(), instance_outputs: true })
}

fn export_all(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.exportAll", v)?;
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
    if let Dest::File(_) = Dest::read(&p)? {
        return Err(p.bad("`destination` must be a folder: Export All writes several files"));
    }
    let timestamp = compile::timestamp(&p)?;
    let autohinter = crate::external_autohint::trusted(s);
    let mut given = v.clone();
    if let Value::Object(m) = &mut given {
        m.remove("documents");
    }
    let mut plans: Vec<(usize, Request, Vec<compile::Job>)> = Vec::with_capacity(chosen.len());
    let mut fonts = 0usize;
    for &i in &chosen {
        let doc = docs.get(i).ok_or(EngineError::NoDocument)?;
        let merged = document_params(&given, &export_settings::kept_otf(s, doc));
        let dp = Params::new("file.exportAll", &merged)?;
        let req = document_request(&dp, &doc.title, timestamp, autohinter.clone())?;
        compile::check_glyph_names(&doc.font)?;
        let jobs = compile::jobs(&doc.font, &Pick::Exportable, false, &p)?;
        compile::check_size(&req, jobs.len())?;
        fonts = fonts.saturating_add(jobs.len().saturating_mul(req.flavours.len()));
        plans.push((i, req, jobs));
    }
    if fonts > compile::MAX_FONTS {
        return Err(EngineError::Unsupported(format!(
            "building {fonts} fonts in one command (at most {}); choose fewer documents or formats",
            compile::MAX_FONTS
        )));
    }
    // Documents that share a folder share a sink, so their files are checked against each other.
    let outside = s.export_outside();
    let mut sinks: Vec<(std::path::PathBuf, Sink)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut reports = Vec::with_capacity(plans.len());
    for (i, req, jobs) in &plans {
        let doc = docs.get(*i).ok_or(EngineError::NoDocument)?;
        let Dest::Folder(folder) = &req.dest else { return Err(EngineError::Internal("Export All without a folder".into())) };
        let folder = compile::absolute(folder);
        let at = match sinks.iter().position(|(f, _)| *f == folder) {
            Some(at) => at,
            None => {
                sinks.push((folder.clone(), Sink::new(&req.dest, false, outside)?));
                sinks.len() - 1
            }
        };
        let (_, sink) = sinks.get_mut(at).ok_or_else(|| EngineError::Internal("Export All lost a sink".into()))?;
        let mut report = Report::default();
        compile::build(&doc.font, &doc.title, doc.path.as_deref(), s.glyph_data_of(doc)?, jobs, req, sink, &mut report)?;
        warnings.append(&mut report.warnings);
        reports.push((doc.title.clone(), folder, report));
    }
    let written: usize = reports.iter().map(|(_, _, r)| r.file_count()).sum();
    if written == 0 {
        let skipped = Report { skipped: reports.into_iter().flat_map(|(_, _, r)| r.skipped).collect(), ..Default::default() };
        return Err(compile::nothing_built(&skipped));
    }
    let mut sinks: Vec<Sink> = sinks.into_iter().map(|(_, sink)| sink).collect();
    match sinks.len() {
        1 => sinks.pop().map_or(Ok(()), Sink::commit)?,
        _ => compile::commit_all(sinks)?,
    }
    let documents: Vec<Value> = reports
        .iter()
        .map(|(title, folder, r)| json!({ "title": title, "destination": folder.to_string_lossy(), "fonts": r.fonts_json(false), "skipped": r.skipped }))
        .collect();
    Ok(json!({ "documents": documents, "written": written, "warnings": warnings }))
}
