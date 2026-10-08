//! Documents on disk: recognising a source, reading it, writing it back (native builds only; the
//! web app passes bytes instead).
//!
//! A source is recognised by its content, never by its file name: a folder with `fontinfo.plist`
//! is a `.glyphspackage`, a JSON file whose `format` is `fontcraft` is a native file, and other
//! text that starts with `{` is a `.glyphs` file. Every read is bounded ([`MAX_PACKAGE_FILES`],
//! [`fontcraft_format::MAX_BYTES`] in all) and follows no symlinked folders, so a hostile folder
//! can neither exhaust memory nor loop.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use fontcraft_model::{Family, Font, Shape, SourceFormat};
use serde_json::{Value, json};

use crate::{EngineError, Result};

/// Most files read from one package.
pub const MAX_PACKAGE_FILES: usize = 200_000;

/// The formats a document can be saved in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormat {
    Native,
    Glyphs,
    GlyphsPackage,
}

impl SaveFormat {
    pub fn name(self) -> &'static str {
        match self {
            SaveFormat::Native => "fontcraft",
            SaveFormat::Glyphs => "glyphs",
            SaveFormat::GlyphsPackage => "glyphspackage",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fontcraft" => Some(SaveFormat::Native),
            "glyphs" => Some(SaveFormat::Glyphs),
            "glyphspackage" => Some(SaveFormat::GlyphsPackage),
            _ => None,
        }
    }

    /// The format a path's extension names.
    pub fn of_path(path: &Path) -> Option<Self> {
        Self::parse(path.extension()?.to_str()?)
    }

    /// The format a document read from `source` is saved in by default.
    pub fn of_source(source: SourceFormat) -> Option<Self> {
        match source {
            SourceFormat::Native => Some(SaveFormat::Native),
            SourceFormat::Glyphs => Some(SaveFormat::Glyphs),
            SourceFormat::GlyphsPackage => Some(SaveFormat::GlyphsPackage),
            SourceFormat::Ufo | SourceFormat::Designspace | SourceFormat::Binary => None,
        }
    }

    fn family(self) -> Family {
        match self {
            SaveFormat::Native => Family::Native,
            SaveFormat::Glyphs | SaveFormat::GlyphsPackage => Family::Glyphs,
        }
    }
}

/// What a file or folder looked like when it was read or written, to notice that something else
/// changed it since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    files: usize,
    bytes: u64,
    modified: Option<SystemTime>,
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> EngineError {
    EngineError::Io(format!("cannot {what} {}: {e}", path.display()))
}

/// The stamp of a file, or of every file in a folder; `None` when there is nothing there.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        return Some(Stamp { files: 1, bytes: meta.len(), modified: meta.modified().ok() });
    }
    let mut s = Stamp { files: 0, bytes: 0, modified: None };
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else { continue };
            if m.is_dir() {
                stack.push(entry.path());
            } else if m.is_file() {
                s.files += 1;
                s.bytes += m.len();
                s.modified = s.modified.max(m.modified().ok());
            }
            if s.files > MAX_PACKAGE_FILES {
                return Some(s);
            }
        }
    }
    Some(s)
}

/// A regular file's bytes, at most [`fontcraft_format::MAX_BYTES`].
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| io_err("open", path, e))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("cannot open {}: it is not a regular file", path.display())));
    }
    let limit = fontcraft_format::MAX_BYTES as u64;
    let mut bytes = Vec::new();
    // At most one byte past the limit, whatever the file's size claims to be.
    std::fs::File::open(path).map_err(|e| io_err("open", path, e))?.take(limit + 1).read_to_end(&mut bytes).map_err(|e| io_err("read", path, e))?;
    if bytes.len() as u64 > limit {
        return Err(fontcraft_format::FormatError::TooLarge(bytes.len()).into());
    }
    Ok(bytes)
}

/// Every regular file in a folder, by path relative to it with `/`. Symlinks are not followed.
pub fn read_folder(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut total = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| io_err("read the folder", &dir, e))? {
            let entry = entry.map_err(|e| io_err("read the folder", &dir, e))?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|e| io_err("read", &path, e))?;
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            if out.len() >= MAX_PACKAGE_FILES {
                return Err(EngineError::Io(format!("{} holds more than {MAX_PACKAGE_FILES} files", root.display())));
            }
            let rel = path.strip_prefix(root).map_err(|e| EngineError::Internal(e.to_string()))?;
            let rel = rel.to_str().ok_or_else(|| EngineError::Io(format!("{}: a file name is not UTF-8", path.display())))?.replace('\\', "/");
            let bytes = read_file(&path)?;
            total += bytes.len();
            if total > fontcraft_format::MAX_BYTES {
                return Err(fontcraft_format::FormatError::TooLarge(total).into());
            }
            out.insert(rel, bytes);
        }
    }
    Ok(out)
}

/// Reads the source at `path`, recognising its format by content.
pub fn open(path: &Path) -> Result<Font> {
    let meta = std::fs::metadata(path).map_err(|e| io_err("open", path, e))?;
    if meta.is_dir() {
        if path.join("fontinfo.plist").is_file() {
            let files = read_folder(path)?;
            return Ok(fontcraft_io_glyphs::read_package(&files)?);
        }
        if path.join("metainfo.plist").is_file() {
            return Err(EngineError::Unsupported("opening UFO sources (ROADMAP.md, M1)".into()));
        }
        return Err(EngineError::Io(format!("{} is a folder but not a font source", path.display())));
    }
    let bytes = read_file(path)?;
    let head = bytes.get(..4).unwrap_or(&bytes);
    if matches!(head, [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf" | b"wOFF" | b"wOF2") {
        return Err(EngineError::Unsupported("opening compiled fonts (ROADMAP.md, M1)".into()));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| EngineError::Io(format!("{} is not a font source: it is not UTF-8 text", path.display())))?;
    let body = text.trim_start_matches('\u{feff}').trim_start();
    if body.starts_with("<?xml") || body.starts_with('<') {
        return Err(EngineError::Unsupported("opening designspace and other XML sources (ROADMAP.md, M1)".into()));
    }
    if !body.starts_with('{') {
        return Err(EngineError::Io(format!("{} is not a font source this build can read", path.display())));
    }
    if is_native(body) {
        return Ok(fontcraft_format::load(&bytes)?);
    }
    Ok(fontcraft_io_glyphs::read(text)?)
}

/// Whether text that starts with `{` is a native file: JSON whose first key is a quoted string
/// followed by `:` and which names the `fontcraft` format near the start.
fn is_native(body: &str) -> bool {
    let after = body.trim_start_matches('{').trim_start();
    let json_key = after.starts_with('"')
        && after.split_once('"').and_then(|(_, rest)| rest.split_once('"')).is_some_and(|(_, rest)| rest.trim_start().starts_with(':'));
    json_key && body.get(..body.len().min(4096)).unwrap_or(body).contains("\"fontcraft\"")
}

/// What saving could not keep: extra entries of other format families.
fn losses(font: &Font, target: Family) -> Vec<Value> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut count = |x: &fontcraft_model::Extra| {
        for e in x.iter().filter(|e| e.family != target) {
            *counts.entry(family_name(e.family)).or_default() += 1;
        }
    };
    count(&font.extra);
    font.metrics.iter().for_each(|m| count(&m.extra));
    font.masters.iter().for_each(|m| count(&m.extra));
    for g in font.glyphs.iter() {
        count(&g.extra);
        for l in &g.layers {
            count(&l.extra);
            l.anchors.iter().for_each(|a| count(&a.extra));
            for s in &l.shapes {
                match s {
                    Shape::Path(p) => {
                        count(&p.extra);
                        p.nodes.iter().for_each(|n| count(&n.extra));
                    }
                    Shape::Component(c) => count(&c.extra),
                    Shape::Raw(_) => {}
                }
            }
        }
    }
    counts.into_iter().map(|(family, entries)| json!({ "family": family, "entries": entries, "what": "keys read from that format, which this one has no place for" })).collect()
}

fn family_name(f: Family) -> &'static str {
    match f {
        Family::Glyphs => "glyphs",
        Family::Ufo => "ufo",
        Family::Designspace => "designspace",
        Family::Binary => "binary",
        Family::Native => "fontcraft",
    }
}

/// The result of a save.
pub struct Saved {
    pub bytes: usize,
    pub files_written: usize,
    pub files_removed: usize,
    pub losses: Vec<Value>,
}

/// Writes `font` to `path` in `format`. A single file is written atomically. A package writes
/// only the files whose bytes differ from what is on disk, and removes glyph files the document
/// no longer has.
pub fn save(font: &Font, path: &Path, format: SaveFormat) -> Result<Saved> {
    let mut losses = if format == SaveFormat::Native { Vec::new() } else { losses(font, format.family()) };
    // A single file has no place for a package's side files (UI state, version 4 feature files).
    if format == SaveFormat::Glyphs && !font.files.is_empty() {
        let names: Vec<&str> = font.files.0.keys().map(String::as_str).collect();
        losses.push(json!({ "family": "glyphspackage", "entries": names.len(), "what": "files beside the glyphs in the package", "files": names }));
    }
    match format {
        SaveFormat::Native | SaveFormat::Glyphs => {
            let bytes = if format == SaveFormat::Native { fontcraft_format::save(font)? } else { fontcraft_io_glyphs::write(font)?.into_bytes() };
            fontcraft_format::write_atomic(path, &bytes)?;
            Ok(Saved { bytes: bytes.len(), files_written: 1, files_removed: 0, losses })
        }
        SaveFormat::GlyphsPackage => {
            let files = fontcraft_io_glyphs::write_package(font)?;
            if path.exists() && !path.is_dir() {
                return Err(EngineError::Io(format!("cannot write a package to {}: it is a file", path.display())));
            }
            std::fs::create_dir_all(path).map_err(|e| io_err("create", path, e))?;
            let on_disk = read_folder(path)?;
            let mut written = 0;
            for (rel, bytes) in &files {
                if on_disk.get(rel) == Some(bytes) {
                    continue;
                }
                let target = safe_join(path, rel)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_err("create", parent, e))?;
                }
                fontcraft_format::write_atomic(&target, bytes)?;
                written += 1;
            }
            let mut removed = 0;
            for rel in on_disk.keys().filter(|r| r.starts_with("glyphs/") && r.ends_with(".glyph") && !files.contains_key(*r)) {
                let target = safe_join(path, rel)?;
                std::fs::remove_file(&target).map_err(|e| io_err("remove", &target, e))?;
                removed += 1;
            }
            Ok(Saved { bytes: files.values().map(Vec::len).sum(), files_written: written, files_removed: removed, losses })
        }
    }
}

/// `root/rel`, refusing a relative path that would leave `root`.
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let ok = !rel.is_empty()
        && !rel.starts_with('/')
        && rel.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\\'));
    if !ok {
        return Err(EngineError::Io(format!("refusing to write `{rel}` outside the package")));
    }
    Ok(root.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_files_are_told_from_glyphs_files() {
        assert!(is_native("{\n  \"format\": \"fontcraft\",\n  \"version\": 2"));
        assert!(!is_native("{\n.appVersion = \"3343\";\n.formatVersion = 3;"));
        assert!(!is_native("{\n\"Some Key\" = \"fontcraft\";"));
    }

    #[test]
    fn package_paths_cannot_escape() {
        let root = Path::new("/tmp/pkg");
        assert!(safe_join(root, "glyphs/A_.glyph").is_ok());
        for bad in ["", "/etc/passwd", "../x", "glyphs/../../x", "a//b", "a\\b", "./x"] {
            assert!(safe_join(root, bad).is_err(), "{bad}");
        }
    }
}
