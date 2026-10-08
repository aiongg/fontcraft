//! Documents on disk: recognising a source, reading it, writing it back (native builds only; the
//! web app passes bytes instead).
//!
//! A source is recognised by its content, never by its file name: a folder with `metainfo.plist`
//! is a UFO and one with `fontinfo.plist` a `.glyphspackage`, a file that starts with an sfnt,
//! collection or WOFF signature is a compiled font, a JSON file whose `format` is `fontcraft` is a
//! native file, XML whose root is `<designspace>` a designspace (opened with the UFOs it names,
//! relative to its folder), and other text that starts with `{` is a `.glyphs` file. Every read is bounded ([`MAX_PACKAGE_FILES`],
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
    Ufo,
    Designspace,
}

impl SaveFormat {
    pub fn name(self) -> &'static str {
        match self {
            SaveFormat::Native => "fontcraft",
            SaveFormat::Glyphs => "glyphs",
            SaveFormat::GlyphsPackage => "glyphspackage",
            SaveFormat::Ufo => "ufo",
            SaveFormat::Designspace => "designspace",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fontcraft" => Some(SaveFormat::Native),
            "glyphs" => Some(SaveFormat::Glyphs),
            "glyphspackage" => Some(SaveFormat::GlyphsPackage),
            "ufo" => Some(SaveFormat::Ufo),
            "designspace" => Some(SaveFormat::Designspace),
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
            SourceFormat::Ufo => Some(SaveFormat::Ufo),
            SourceFormat::Designspace => Some(SaveFormat::Designspace),
            SourceFormat::Binary => None,
        }
    }

    /// The format families whose kept data this format writes back.
    fn families(self) -> &'static [Family] {
        match self {
            SaveFormat::Native => &[Family::Native],
            SaveFormat::Glyphs | SaveFormat::GlyphsPackage => &[Family::Glyphs],
            SaveFormat::Ufo => &[Family::Ufo],
            SaveFormat::Designspace => &[Family::Ufo, Family::Designspace],
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

impl Stamp {
    fn add(&mut self, other: Stamp) {
        self.files += other.files;
        self.bytes += other.bytes;
        self.modified = self.modified.max(other.modified);
    }
}

/// The stamp of a document's files: the file or folder at `path`, and for a designspace also
/// every UFO it names.
pub fn document_stamp(path: &Path, font: &Font) -> Option<Stamp> {
    let mut s = stamp(path)?;
    if font.source.format == SourceFormat::Designspace {
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut seen = Vec::new();
        for m in font.masters.iter() {
            if let Some(f) = m.extra.get(Family::Designspace, "filename").and_then(fontcraft_model::plist::Value::as_str)
                && !seen.contains(&f)
            {
                seen.push(f);
                if let Some(u) = stamp(&dir.join(f)) {
                    s.add(u);
                }
            }
        }
    }
    Some(s)
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

/// Reads the source at `path`, recognising its format by content, with what reading it left out
/// (compiled fonts only). `face` chooses a member of a font collection.
pub fn open(path: &Path, face: Option<u32>) -> Result<(Font, Vec<String>)> {
    let meta = std::fs::metadata(path).map_err(|e| io_err("open", path, e))?;
    if meta.is_dir() {
        if path.join("metainfo.plist").is_file() {
            let files = read_folder(path)?;
            return Ok((fontcraft_io_ufo::ufo::read(&files)?, Vec::new()));
        }
        if path.join("fontinfo.plist").is_file() {
            let files = read_folder(path)?;
            return Ok((fontcraft_io_glyphs::read_package(&files)?, Vec::new()));
        }
        return Err(EngineError::Io(format!("{} is a folder but not a font source", path.display())));
    }
    let bytes = read_file(path)?;
    if fontcraft_format::is_compressed(&bytes) {
        return Ok((fontcraft_format::load(&bytes)?, Vec::new()));
    }
    if fontcraft_decompile::sniff(&bytes).is_some() {
        let d = fontcraft_decompile::decompile(&bytes, face)?;
        return Ok((d.font, d.warnings));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| EngineError::Io(format!("{} is not a font source: it is not UTF-8 text", path.display())))?;
    let body = text.trim_start_matches('\u{feff}').trim_start();
    if body.starts_with('<') {
        if !is_designspace(body) {
            return Err(EngineError::Io(format!("{} is XML but not a designspace", path.display())));
        }
        return Ok((open_designspace(path, text)?, Vec::new()));
    }
    if !body.starts_with('{') {
        return Err(EngineError::Io(format!("{} is not a font source this build can read", path.display())));
    }
    if is_native(body) {
        return Ok((fontcraft_format::load(&bytes)?, Vec::new()));
    }
    Ok((fontcraft_io_glyphs::read(text)?, Vec::new()))
}

/// Whether XML text has `<designspace` as its root element: the first element after the
/// declaration, comments and document type.
fn is_designspace(body: &str) -> bool {
    let mut rest = body;
    loop {
        rest = rest.trim_start();
        let skip = if rest.starts_with("<?") {
            rest.find("?>").map(|i| i + 2)
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if rest.starts_with("<!") {
            rest.find('>').map(|i| i + 1)
        } else {
            return rest.strip_prefix("<designspace").is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/'));
        };
        match skip.and_then(|i| rest.get(i..)) {
            Some(r) => rest = r,
            None => return false,
        }
    }
}

/// A designspace and the UFOs it names, read relative to its folder, within the same bounds as
/// one package.
fn open_designspace(path: &Path, text: &str) -> Result<Font> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut ufos = BTreeMap::new();
    let mut total = text.len();
    for rel in fontcraft_io_ufo::designspace::ufo_paths(text)? {
        let ufo_dir = dir.join(&rel);
        if !ufo_dir.join("metainfo.plist").is_file() {
            return Err(EngineError::Io(format!("{}: the source `{rel}` is not a UFO folder", path.display())));
        }
        let files = read_folder(&ufo_dir)?;
        total += files.values().map(Vec::len).sum::<usize>();
        if total > fontcraft_format::MAX_BYTES {
            return Err(fontcraft_format::FormatError::TooLarge(total).into());
        }
        ufos.insert(rel, files);
    }
    Ok(fontcraft_io_ufo::designspace::read(text, &ufos)?)
}

/// Whether text that starts with `{` is a native file: JSON whose first key is a quoted string
/// followed by `:` and which names the `fontcraft` format near the start.
fn is_native(body: &str) -> bool {
    let after = body.trim_start_matches('{').trim_start();
    let json_key = after.starts_with('"')
        && after.split_once('"').and_then(|(_, rest)| rest.split_once('"')).is_some_and(|(_, rest)| rest.trim_start().starts_with(':'));
    json_key && body.get(..body.len().min(4096)).unwrap_or(body).contains("\"fontcraft\"")
}

/// What saving could not keep: extra entries and kept shapes of other format families.
fn losses(font: &Font, targets: &[Family]) -> Vec<Value> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut count = |x: &fontcraft_model::Extra| {
        for e in x.iter().filter(|e| !targets.contains(&e.family)) {
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
    let mut out: Vec<Value> =
        counts.into_iter().map(|(family, entries)| json!({ "family": family, "entries": entries, "what": "keys read from that format, which this one has no place for" })).collect();
    let mut shapes: BTreeMap<&'static str, usize> = BTreeMap::new();
    for g in font.glyphs.iter() {
        for l in &g.layers {
            for s in &l.shapes {
                if let Shape::Raw(r) = s
                    && !targets.contains(&r.family)
                {
                    *shapes.entry(family_name(r.family)).or_default() += 1;
                }
            }
        }
    }
    for (family, n) in shapes {
        out.push(json!({ "family": family, "entries": n, "what": "shapes read from that format that the model keeps as written, left out" }));
    }
    out
}

/// Files beside another format's document (a package's UI state, feature files), which a UFO does
/// not take.
fn side_file_losses(font: &Font) -> Option<Value> {
    let foreign = !matches!(font.source.format, SourceFormat::Ufo | SourceFormat::Designspace);
    (foreign && !font.files.is_empty()).then(|| {
        let names: Vec<&str> = font.files.0.keys().map(String::as_str).collect();
        json!({ "family": "glyphspackage", "entries": names.len(), "what": "files beside the glyphs in the package", "files": names })
    })
}

/// Layers a UFO has no place for yet: non-master layers that did not come from a UFO layer.
fn ufo_layer_losses(font: &Font) -> Option<Value> {
    let n: usize = font
        .glyphs
        .iter()
        .flat_map(|g| &g.layers)
        .filter(|l| !l.is_master_layer() && l.extra.get(Family::Ufo, "layer").is_none() && !font.masters.iter().any(|m| m.id == l.id))
        .count();
    (n > 0).then(|| json!({ "family": "glyphs", "entries": n, "what": "layers other than master layers (backup, intermediate, alternate, colour), not written to UFO yet" }))
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
    let mut losses = if format == SaveFormat::Native { Vec::new() } else { losses(font, format.families()) };
    if matches!(format, SaveFormat::Ufo | SaveFormat::Designspace) {
        losses.extend(ufo_layer_losses(font));
        losses.extend(side_file_losses(font));
    }
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
        SaveFormat::Ufo => {
            let files = fontcraft_io_ufo::ufo::write(font)?;
            let (written, removed) = write_ufo(path, &files)?;
            Ok(Saved { bytes: files.values().map(Vec::len).sum(), files_written: written, files_removed: removed, losses })
        }
        SaveFormat::Designspace => {
            let out = fontcraft_io_ufo::designspace::write(font)?;
            let dir = path.parent().unwrap_or(Path::new("."));
            let (mut written, mut removed, mut bytes) = (0, 0, out.document.len());
            for (rel, files) in &out.ufos {
                let ufo_dir = safe_join_relative(dir, rel)?;
                let (w, r) = write_ufo(&ufo_dir, files)?;
                written += w;
                removed += r;
                bytes += files.values().map(Vec::len).sum::<usize>();
            }
            // The document last, so a crash before it leaves the old document pointing at UFOs
            // that are each complete.
            if std::fs::read(path).ok().as_deref() != Some(out.document.as_bytes()) {
                fontcraft_format::write_atomic(path, out.document.as_bytes())?;
                written += 1;
            }
            Ok(Saved { bytes, files_written: written, files_removed: removed, losses })
        }
    }
}

/// What [`export_ufos`] wrote: each UFO's path, master and number of files written.
pub struct Exported {
    pub files: Vec<(PathBuf, String, usize)>,
    pub losses: Vec<Value>,
}

/// Writes each of `masters` as its own UFO 3 into the folder `destination`.
pub fn export_ufos(font: &Font, masters: &[String], destination: &Path) -> Result<Exported> {
    let mut font = font.clone();
    font.source.ufo_version = Some(3);
    let mut losses = losses(&font, &[Family::Ufo]);
    losses.extend(ufo_layer_losses(&font));
    losses.extend(side_file_losses(&font));
    let mut taken = Vec::new();
    let mut files = Vec::new();
    for id in masters {
        let master = font.master(id)?;
        let name = fontcraft_io_ufo::designspace::ufo_file_name(&font, master, &taken);
        taken.push(name.clone());
        let ufo = fontcraft_io_ufo::ufo::write_one(&font, id)?;
        let path = destination.join(&name);
        let (written, _) = write_ufo(&path, &ufo)?;
        files.push((path, id.clone(), written));
    }
    Ok(Exported { files, losses })
}

/// Writes a UFO's files into `path`: only files whose bytes differ, the index files
/// (`contents.plist`, `layercontents.plist`, `metainfo.plist`) last; then removes glyph files,
/// layer index files and standard files the document no longer has. Refuses to write into a
/// folder that is neither empty nor a UFO.
fn write_ufo(path: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(usize, usize)> {
    if path.exists() && !path.is_dir() {
        return Err(EngineError::Io(format!("cannot write a UFO to {}: it is a file", path.display())));
    }
    if path.is_dir() && !path.join("metainfo.plist").is_file() && std::fs::read_dir(path).map_err(|e| io_err("read", path, e))?.next().is_some() {
        return Err(EngineError::Io(format!("cannot write a UFO to {}: the folder is not empty and not a UFO", path.display())));
    }
    std::fs::create_dir_all(path).map_err(|e| io_err("create", path, e))?;
    let on_disk = read_folder(path)?;
    let is_index = |rel: &str| rel.ends_with("/contents.plist") || rel == "layercontents.plist" || rel == "metainfo.plist";
    let mut written = 0;
    for index_pass in [false, true] {
        for (rel, bytes) in files.iter().filter(|(rel, _)| is_index(rel) == index_pass) {
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
    }
    const STANDARD: [&str; 5] = ["groups.plist", "kerning.plist", "lib.plist", "features.fea", "layercontents.plist"];
    let stale = |rel: &str| {
        let in_layer = rel.starts_with("glyphs") && rel.split('/').count() == 2;
        let layer_file = rel.ends_with(".glif") || rel.ends_with("/contents.plist") || rel.ends_with("/layerinfo.plist");
        in_layer && layer_file || STANDARD.contains(&rel)
    };
    let mut removed = 0;
    for rel in on_disk.keys().filter(|r| stale(r) && !files.contains_key(*r)) {
        let target = safe_join(path, rel)?;
        std::fs::remove_file(&target).map_err(|e| io_err("remove", &target, e))?;
        removed += 1;
        // A layer folder left empty goes too.
        if let Some(parent) = target.parent()
            && parent != path
            && std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none())
        {
            std::fs::remove_dir(parent).map_err(|e| io_err("remove", parent, e))?;
        }
    }
    Ok((written, removed))
}

/// A designspace's `filename` relative to its folder. `..` is allowed, as designspace files use
/// it for UFOs beside their folder; absolute paths and backslashes are not.
fn safe_join_relative(dir: &Path, rel: &str) -> Result<PathBuf> {
    let ok =
        !rel.is_empty() && !rel.starts_with('/') && !rel.contains('\\') && !rel.contains(':') && rel.split('/').all(|p| !p.is_empty() && p != ".");
    if !ok {
        return Err(EngineError::Io(format!("refusing to write the UFO `{rel}`: not a relative path")));
    }
    Ok(dir.join(rel))
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
    fn designspace_documents_are_recognised() {
        assert!(is_designspace("<?xml version='1.0' encoding='UTF-8'?>\n<!-- c -->\n<designspace format=\"5.0\">"));
        assert!(is_designspace("<designspace>"));
        assert!(!is_designspace("<?xml version='1.0'?>\n<plist version=\"1.0\">"));
        assert!(!is_designspace("<designspacex>"));
        assert!(!is_designspace("<?xml never closed"));
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
