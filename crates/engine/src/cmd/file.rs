//! File menu: new, open, save, close and the UFO export. The font exports are in
//! `compile_export.rs`.
//!
//! Reading and writing files happens here only in native builds (through [`crate::io`]); on the
//! web the shell hands bytes to [`crate::Session::add_document`].

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use fontcraft_model::Font;
use serde_json::{Value, json};

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
            "{path: string, face?: number, keepGlyphNames?: bool} open a font as a new document, recognising it by content: a .fontcraft file, a .glyphs file (format versions 2 to 4), a .glyphspackage folder, a UFO folder (versions 2 and 3), a .designspace file with the UFOs it names (one document, a master per source), or a compiled font (TrueType, OpenType with CFF, a collection member chosen by face, WOFF, WOFF2). A compiled font opens view-only: save it with Save As to a source format. A UFOZ is saved back as a UFOZ. GlyphData*.xml files beside the source and in an Info folder beside it add to the glyph data for this document, unless the font's Ignore custom GlyphData file parameter is on. A UFO or designspace has its composites aligned automatically as it opens, so their metrics follow their bases (aligned counts the glyphs that changed); with the user setting user.disableAutomaticAlignmentForImportedFiles on, the font's Disable Automatic Alignment option is switched on instead. A UFO, designspace or compiled font has its glyphs renamed to the nice names the glyph data gives their names (`uni0041` becomes `A`), keeping their places and code points (renamed counts them); a glyph whose name feature code or other text uses keeps it, with a warning. keepGlyphNames (default: the user setting user.keepGlyphNamesFromImportedFiles) or the font's Use Custom Naming option keeps the file's names. warnings lists what reading a compiled font left out and the glyph data files that could not be used. A .fontcraftproject file opens as a project, as project.open does, with its reply → {title, path, format, version?, glyphs, masters, viewOnly, aligned, renamed, warnings}",
            always,
            open
        ).files(const { &[FileParam::read("path")] }),
        cmd!(
            "file.save",
            "Save",
            ["File"],
            Some("Cmd+S"),
            "{path?: string, force?: bool, document?: number} write the document to its file in the format it was opened in (or to `path`, choosing the format from its extension). document: the session id of the document meant, which the app passes when it asks to overwrite; refused when another document is active, and left out of the command log. Refuses when the file changed on disk since it was read (for a designspace: the document or any of its UFOs), unless force is true. Single files (a UFOZ among them) are written atomically; a package or UFO folder rewrites only changed files; a designspace writes its UFOs, then the document → {path, format, version?, bytes, filesWritten, filesRemoved, losses}",
            has_doc,
            save
        ).files(const { &[FileParam::write("path")] }).writes_files(),
        cmd!(
            "file.saveAs",
            "Save As…",
            ["File"],
            Some("Cmd+Shift+S"),
            "{path: string, format?: \"fontcraft\"|\"glyphs\"|\"glyphspackage\"|\"ufo\"|\"ufoz\"|\"designspace\", version?: 2|3|4, compress?: bool, force?: bool, smartComponents?: \"components\"|\"outlines\"} write the document to a new file and make that its file; format defaults to the path's extension, version to the one the document has (3 for a new document; .glyphs has 2 to 4, UFO 2 and 3). A single UFO holds one master; a UFOZ is a UFO in a ZIP archive, under a folder named after the file; a designspace writes a UFO per master beside it. compress: gzip a .fontcraft file (default: as it was opened). smartComponents (UFO, UFOZ and designspace): keep smart components as components with their values in the glyph lib (default), or draw them as outlines interpolated from the smart glyph's pole layers; the document keeps its components. losses lists what the format cannot hold → {path, format, version?, bytes, filesWritten, filesRemoved, losses}",
            has_doc,
            save_as
        ).files(const { &[FileParam::write("path")] }).writes_files(),
        cmd!(
            "file.exportUfo",
            "Export UFO",
            [],
            None,
            "{destination: string, masters?: [string], productionNames?: bool, decomposeSmartStuff?: bool} write each chosen master (default: all) as its own UFO 3 into the destination folder, named Family-Style.ufo. The document keeps its file and format. productionNames: write each glyph under its production name (its own, else the glyph data's), components, kerning, groups and the glyph order following; the document keeps its names; a glyph whose production name is taken or breaks the naming rules, or whose name feature code or other text uses, keeps its name and is listed in losses. decomposeSmartStuff: draw smart components as outlines interpolated from the smart glyph's pole layers (default true); false keeps them as components with their values in the glyph lib → {files: [{path, master, filesWritten}], losses}",
            has_doc,
            export_ufo
        ).files(const { &[FileParam::write("destination")] }).writes_files(),
        cmd!(
            "file.openRecovered",
            "Open Recovered Document",
            [],
            None,
            "{path: string, original?: string, title?: string} open the copy of unsaved work at path as a document with unsaved changes. original is the file it was opened from or last saved to: Save goes there, and is refused until it is forced, since the file may hold something else by now. When original is open already, the copy opens without a file. The copy itself stays until the app removes it → {title, path, glyphs, document}",
            always,
            open_recovered
        ).files(const { &[FileParam::read("path"), FileParam::write("original")] }),
        cmd!(
            "file.close",
            "Close",
            ["File"],
            Some("Cmd+W"),
            "{discard?: bool} close the active document; a document with unsaved changes needs discard: true → {closed: title}",
            has_doc,
            close
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
    let text = p.str("path")?;
    let path = std::path::PathBuf::from(text);
    if crate::project::is_project_file(&path) {
        return super::project::open_project(s, &json!({ "path": text }));
    }
    let face = p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32);
    let keep_names = p.bool_or("keepGlyphNames", s.settings().get_bool(super::import_naming::KEEP_GLYPH_NAMES).unwrap_or(false))?;
    // A file that is already open is shown, not opened a second time (`file.openTwiceActivates`).
    if let Some(i) = super::shared::find_open(s, &path) {
        s.set_active(i)?;
        remember(s, &path);
        let doc = s.doc()?;
        return Ok(
            json!({ "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "alreadyOpen": true, "glyphs": doc.font.glyphs.len() }),
        );
    }
    let (font, mut warnings) = crate::io::open(&path, face)?;
    let font = crate::linked::resolve(&font, Some(&path), &mut warnings)?.unwrap_or(font);
    let (font, aligned) = super::shared::align_imported(s, font, &mut warnings);
    let glyph_data = crate::glyph_data::source_files(&path, &mut warnings);
    // Every reader checks the font it returns.
    s.push_document(font, Some(path.clone()));
    if !glyph_data.is_empty() {
        let files: Vec<(&str, &[u8])> = glyph_data.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        warnings.extend(s.set_source_glyph_data(&files)?);
    }
    // After the glyph data beside the source is in place, so its names count.
    let renamed = if keep_names { 0 } else { super::import_naming::name_imported(s, &mut warnings) };
    let doc = s.doc()?;
    let save_format = crate::io::SaveFormat::of_source(doc.font.source.format);
    let format = match doc.font.source.format {
        fontcraft_model::SourceFormat::Binary => Some("binary"),
        fontcraft_model::SourceFormat::Ufoz => Some("ufoz"),
        _ => save_format.map(|f| f.name()),
    };
    let version = doc.font.source.glyphs_version.or(doc.font.source.ufo_version);
    let reply = json!({
        "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "format": format,
        "version": version, "glyphs": doc.font.glyphs.len(), "masters": doc.font.masters.len(), "viewOnly": save_format.is_none(), "warnings": warnings,
        "aligned": aligned, "renamed": renamed,
    });
    remember(s, &path);
    Ok(reply)
}

/// Puts a document just opened or saved first in the recent documents. The list is a
/// convenience: a path it cannot keep leaves it as it was and does not fail the command.
#[cfg(not(target_arch = "wasm32"))]
fn remember(s: &mut Session, path: &std::path::Path) {
    if let Err(e) = s.settings_mut().remember_document(path) {
        log::warn!("recent documents: {e}");
    }
}

#[cfg(target_arch = "wasm32")]
fn open(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("opening a file by path in the browser; the web app passes the file's bytes".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_document(s: &mut Session, p: &Params, path: std::path::PathBuf, format: crate::io::SaveFormat, version: Option<u32>) -> Result<Value> {
    use crate::io::{self, SaveFormat};
    let outlines = smart_outlines(p)?;
    if outlines && !format.is_ufo() {
        return Err(p.bad("`smartComponents` applies to UFO, UFOZ and designspace files"));
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
    crate::assets::rebase_image_paths(&mut font, doc.path.as_deref(), &path);
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
        SaveFormat::Ufo | SaveFormat::Ufoz | SaveFormat::Designspace => {
            let v = pick(font.source.ufo_version, 2..=3)?;
            font.source.ufo_version = Some(v);
            Some(v)
        }
    };
    if format != SaveFormat::Native {
        font.source.format = format.source();
    }
    let (smart, smart_losses) = smart_choice(&font, outlines);
    let mut saved = io::save(smart.as_ref().unwrap_or(&font), &path, format)?;
    saved.losses.extend(smart_losses);
    let font = Arc::new(font);
    let doc = s.doc_mut()?;
    let (id, old_path) = (doc.id, doc.path.replace(path.clone()));
    doc.title = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| doc.title.clone());
    doc.font = font.clone();
    doc.saved = font;
    doc.stamp = io::document_stamp(&path, &doc.font);
    // Undo steps carry their own folder (`HistoryEntry::path`). The starting font of a running
    // batch or interaction becomes an undo step under the new folder, so it is rewritten here.
    let rebase = |font: &mut Arc<Font>| *font = crate::assets::rebased(Arc::clone(font), old_path.as_deref(), Some(&path));
    if let Some(batch) = s.batch.as_mut() {
        batch.before.iter_mut().filter(|(d, _)| *d == id).for_each(|(_, before)| rebase(before));
    }
    if let Some(it) = s.interaction.as_mut().filter(|it| it.doc == id) {
        rebase(&mut it.before);
    }
    remember(s, &path);
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
    if let Some(id) = p.opt_int("document", 0, i64::MAX)? {
        let active = s.doc()?;
        if i64::try_from(active.id).ok() != Some(id) {
            return Err(p.bad(format!("document {id} is not the active document ({}): show it and save again", active.title)));
        }
    }
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
            p.bad(format!("`format` must be \"fontcraft\", \"glyphs\", \"glyphspackage\", \"ufo\", \"ufoz\" or \"designspace\", got \"{f}\""))
        })?,
        None => SaveFormat::of_path(&path)
            .ok_or_else(|| p.bad("give a `format`, or a path ending in .fontcraft, .glyphs, .glyphspackage, .ufo, .ufoz or .designspace"))?,
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
    let production = p.bool_or("productionNames", false)?;
    let outlines = p.bool_or("decomposeSmartStuff", true)?;
    let data = s.glyph_data()?;
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
    let written = smart.as_ref().unwrap_or(font);
    let (named, name_losses) = if production { super::import_naming::production_copy(written, &data) } else { (written.clone(), Vec::new()) };
    let mut exported = crate::io::export_ufos(&named, &masters, &destination)?;
    exported.losses.extend(smart_losses);
    exported.losses.extend(name_losses);
    let files: Vec<Value> =
        exported.files.iter().map(|(path, master, n)| json!({ "path": path.to_string_lossy(), "master": master, "filesWritten": n })).collect();
    Ok(json!({ "files": files, "losses": exported.losses }))
}

#[cfg(target_arch = "wasm32")]
fn export_ufo(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("writing UFO folders in the browser".into()))
}

/// Longest title a recovered document takes, in characters.
#[cfg(not(target_arch = "wasm32"))]
const MAX_RECOVERED_TITLE: usize = 256;

#[cfg(not(target_arch = "wasm32"))]
fn open_recovered(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.openRecovered", p)?;
    let copy = std::path::PathBuf::from(p.str("path")?);
    let original = p.opt_str("original")?.filter(|text| !text.is_empty()).map(std::path::PathBuf::from);
    let title = p.opt_str("title")?.map(|t| t.chars().filter(|c| !c.is_control()).take(MAX_RECOVERED_TITLE).collect::<String>());
    // The native reader checks the font it returns; anything else in the recovery folder is refused.
    let bytes = crate::io::read_file(&copy)?;
    let font = fontcraft_format::load(&bytes)?;
    let free = original.filter(|path| super::shared::find_open(s, path).is_none());
    s.push_recovered(font, free, title.filter(|t| !t.trim().is_empty()));
    let doc = s.doc()?;
    Ok(json!({
        "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "glyphs": doc.font.glyphs.len(), "document": doc.id,
    }))
}

#[cfg(target_arch = "wasm32")]
fn open_recovered(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("opening a file by path in the browser".into()))
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
