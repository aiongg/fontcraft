//! File menu: new, open, save, close, export.
//!
//! Reading and writing files happens here only in native builds (through [`crate::io`]); on the
//! web the shell hands bytes to [`crate::Session::add_document`] and takes the compiled font from
//! [`export_ttf`].

use base64::Engine as _;
use fontcraft_compile::{Compiled, Options};
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
            "{path: string} open a font source as a new document: a .fontcraft file, a .glyphs file (format versions 2 to 4) or a .glyphspackage folder, recognised by content → {title, path, format, version?, glyphs}",
            always,
            open
        ),
        cmd!(
            "file.save",
            "Save",
            ["File"],
            Some("Cmd+S"),
            "{path?: string, force?: bool} write the document to its file in the format it was opened in (or to `path`, choosing the format from its extension). Refuses when the file changed on disk since it was read, unless force is true. Single files are written atomically; a package rewrites only changed files → {path, format, version?, bytes, filesWritten, filesRemoved, losses}",
            has_doc,
            save
        ),
        cmd!(
            "file.saveAs",
            "Save As…",
            ["File"],
            Some("Cmd+Shift+S"),
            "{path: string, format?: \"fontcraft\"|\"glyphs\"|\"glyphspackage\", version?: 2|3|4, force?: bool} write the document to a new file and make that its file; format defaults to the path's extension, version to the one the document has (3 for a new document). losses lists what the format cannot hold → {path, format, version?, bytes, filesWritten, filesRemoved, losses}",
            has_doc,
            save_as
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
            "{destination?: string, outlines?: \"truetype\", timestamp?: number} compile the first master to a static TrueType font. destination: a .ttf path, or a folder (the file is named Family-Style.ttf); without it the font is returned as dataBase64. timestamp: seconds since the Unix epoch for the head table (default: SOURCE_DATE_EPOCH, else now) → {path? | dataBase64?, bytes, glyphs, glyphOrder, warnings}",
            has_doc,
            export_static
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
    let font = crate::io::open(&path)?;
    // Every reader checks the font it returns.
    s.push_document(font, Some(path));
    let doc = s.doc()?;
    let format = crate::io::SaveFormat::of_source(doc.font.source.format).map(|f| f.name());
    Ok(json!({
        "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "format": format,
        "version": doc.font.source.glyphs_version, "glyphs": doc.font.glyphs.len(),
    }))
}

#[cfg(target_arch = "wasm32")]
fn open(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("opening a file by path in the browser; the web app passes the file's bytes".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_document(s: &mut Session, p: &Params, path: std::path::PathBuf, format: crate::io::SaveFormat, version: Option<u32>) -> Result<Value> {
    use crate::io::{self, SaveFormat};
    let doc = s.doc()?;
    // Another program may have changed the file since it was read or last written here.
    if doc.path.as_deref() == Some(path.as_path()) && !p.bool_or("force", false)? {
        let now = io::stamp(&path);
        if now.is_some() && now != doc.stamp {
            return Err(p.bad(format!("{} changed on disk since it was opened; save elsewhere, or pass force: true to overwrite", path.display())));
        }
    }
    let mut font = Font::clone(&doc.font);
    let version = match format {
        SaveFormat::Native => None,
        SaveFormat::Glyphs | SaveFormat::GlyphsPackage => {
            let have = font.source.glyphs_version;
            let want = version.or(have).unwrap_or(3);
            if have.is_some_and(|h| h != want) {
                return Err(EngineError::Unsupported(format!("converting between format versions ({} to {want})", have.unwrap_or_default())));
            }
            Some(want)
        }
    };
    font.source.format = match format {
        SaveFormat::Native => font.source.format,
        SaveFormat::Glyphs => fontcraft_model::SourceFormat::Glyphs,
        SaveFormat::GlyphsPackage => fontcraft_model::SourceFormat::GlyphsPackage,
    };
    if format != SaveFormat::Native {
        font.source.glyphs_version = version;
    }
    let saved = io::save(&font, &path, format)?;
    let font = Arc::new(font);
    let doc = s.doc_mut()?;
    doc.title = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| doc.title.clone());
    doc.font = font.clone();
    doc.saved = font;
    doc.path = Some(path.clone());
    doc.stamp = io::stamp(&path);
    Ok(json!({
        "path": path.to_string_lossy(), "format": format.name(), "version": version, "bytes": saved.bytes,
        "filesWritten": saved.files_written, "filesRemoved": saved.files_removed, "losses": saved.losses,
    }))
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
        Some(f) => {
            SaveFormat::parse(f).ok_or_else(|| p.bad(format!("`format` must be \"fontcraft\", \"glyphs\" or \"glyphspackage\", got \"{f}\"")))?
        }
        None => SaveFormat::of_path(&path).ok_or_else(|| p.bad("give a `format`, or a path ending in .fontcraft, .glyphs or .glyphspackage"))?,
    };
    let version = p.opt_int("version", 2, 4)?.map(|v| v as u32);
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
/// else the clock (0 on the web, where the shell passes the time).
fn default_timestamp() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(t) = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.trim().parse::<i64>().ok()) {
            return t;
        }
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// Compiles the active document to a static TrueType font.
pub fn export_ttf(s: &Session, timestamp: i64) -> Result<Compiled> {
    Ok(fontcraft_compile::compile_ttf(s.font()?, &Options { timestamp, ..Default::default() })?)
}

fn export_static(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.exportStatic", p)?;
    match p.opt_str("outlines")?.unwrap_or("truetype") {
        "truetype" => {}
        "cff" => return Err(EngineError::Unsupported("CFF outlines (ROADMAP.md, M2)".into())),
        other => return Err(p.bad(format!("`outlines` must be \"truetype\" or \"cff\", got \"{other}\""))),
    }
    // Years 1904 to 9999, which is what the head table's date can sensibly hold.
    let timestamp = p.opt_int("timestamp", -2_082_844_800, 253_402_300_799)?.unwrap_or_else(default_timestamp);
    let destination = p.opt_str("destination")?;
    let out = export_ttf(s, timestamp)?;
    let (key, value) = match destination {
        None => ("dataBase64", base64::engine::general_purpose::STANDARD.encode(&out.bytes)),
        Some(dest) => ("path", write_export(s, dest, &out.bytes)?),
    };
    Ok(json!({ key: value, "bytes": out.bytes.len(), "glyphs": out.glyph_order.len(), "glyphOrder": out.glyph_order, "warnings": out.warnings }))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_export(s: &Session, destination: &str, bytes: &[u8]) -> Result<String> {
    let mut path = std::path::PathBuf::from(destination);
    if path.is_dir() {
        let font = s.font()?;
        let style = font.default_master().map(|m| m.name.clone())?;
        let name: String = format!("{}-{style}", font.family_name).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        path.push(format!("{}.ttf", if name.is_empty() { "font" } else { &name }));
    }
    fontcraft_format::write_atomic(&path, bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(target_arch = "wasm32")]
fn write_export(_: &Session, _: &str, _: &[u8]) -> Result<String> {
    Err(EngineError::Unsupported("writing to a path in the browser; omit `destination` to get the font as dataBase64".into()))
}
