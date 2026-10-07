//! File menu: new, open, save, close, export.
//!
//! Reading and writing files happens here only in native builds; on the web the shell hands bytes
//! to [`crate::Session::add_document`] and takes them from [`save_bytes`] / [`export_ttf`].

use base64::Engine as _;
use fontcraft_compile::{Compiled, Options};
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
            "{path: string} open a .fontcraft file as a new document (other source formats: see ROADMAP.md, M1) → {title, path, glyphs}",
            always,
            open
        ),
        cmd!(
            "file.save",
            "Save",
            ["File"],
            Some("Cmd+S"),
            "{path?: string} write the document to its file, or to `path` and remember it; written atomically → {path, bytes}",
            has_doc,
            save
        ),
        cmd!(
            "file.saveAs",
            "Save As…",
            ["File"],
            Some("Cmd+Shift+S"),
            "{path: string} write the document to a new file and make that its file → {path, bytes}",
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
    use std::io::Read as _;
    let path = std::path::PathBuf::from(p.str("path")?);
    let io = |what: &str, e: std::io::Error| EngineError::Io(format!("cannot {what} {}: {e}", path.display()));
    // Only regular files: a pipe or a device would block or never end.
    let meta = std::fs::metadata(&path).map_err(|e| io("open", e))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("cannot open {}: it is not a regular file", path.display())));
    }
    let limit = fontcraft_format::MAX_BYTES as u64;
    let mut bytes = Vec::new();
    // Read at most one byte past the limit, whatever the file's size claims to be.
    std::fs::File::open(&path).map_err(|e| io("open", e))?.take(limit + 1).read_to_end(&mut bytes).map_err(|e| io("read", e))?;
    if bytes.len() as u64 > limit {
        return Err(fontcraft_format::FormatError::TooLarge(bytes.len()).into());
    }
    let font = fontcraft_format::load(&bytes)?;
    // `load` has checked the font.
    s.push_document(font, Some(path));
    let doc = s.doc()?;
    Ok(json!({ "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()), "glyphs": doc.font.glyphs.len() }))
}

#[cfg(target_arch = "wasm32")]
fn open(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("opening a file by path in the browser; the web app passes the file's bytes".into()))
}

/// The active document as `.fontcraft` bytes.
// On the web only the shell calls this, so the engine alone sees it as unused there.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn save_bytes(s: &Session) -> Result<Vec<u8>> {
    Ok(fontcraft_format::save(s.font()?)?)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_document(s: &mut Session, path: std::path::PathBuf) -> Result<Value> {
    let bytes = save_bytes(s)?;
    fontcraft_format::write_atomic(&path, &bytes)?;
    let doc = s.doc_mut()?;
    doc.title = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| doc.title.clone());
    doc.saved = doc.font.clone();
    doc.path = Some(path.clone());
    Ok(json!({ "path": path.to_string_lossy(), "bytes": bytes.len() }))
}

#[cfg(not(target_arch = "wasm32"))]
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.save", p)?;
    let path = match p.opt_str("path")? {
        Some(path) => std::path::PathBuf::from(path),
        None => s.doc()?.path.clone().ok_or_else(|| p.bad("the document has never been saved: give a `path`"))?,
    };
    write_document(s, path)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.saveAs", p)?;
    write_document(s, std::path::PathBuf::from(p.str("path")?))
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
