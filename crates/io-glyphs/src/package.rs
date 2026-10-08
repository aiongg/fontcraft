//! The package form, `.glyphspackage`: the same document split into files in a directory.
//!
//! - `fontinfo.plist`: everything except the glyphs.
//! - `order.plist`: the glyph names in glyph order. Unlike the other files it does not end in a
//!   line break.
//! - `glyphs/<name>.glyph`: one glyph each. The file name is the glyph name with `_` after every
//!   uppercase letter and a leading `.` turned into `_` (`A_.glyph`, `_notdef.glyph`); a glyph
//!   stored under another name keeps it in [`Glyph::file_name`](fontcraft_model::Glyph).
//! - Everything else (`UIState.plist`, and in format version 4 `features/*.fea`, `kerning.plist`,
//!   `note.md`) is kept in [`Font::files`] and written back unchanged.
//!
//! The functions here work on file contents by relative path; reading and writing the directory
//! is the caller's job, so they also run in the browser.

use std::collections::BTreeMap;
use std::sync::Arc;

use fontcraft_model::{FileStore, Font, SourceFormat};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Layout, Str, Value};

use crate::{GlyphsError, Result, bad, read_root, take, write_root};

/// A package's files: relative path (with `/`) → contents.
pub type Files = BTreeMap<String, Vec<u8>>;

const FONTINFO: &str = "fontinfo.plist";
const ORDER: &str = "order.plist";
const GLYPHS_DIR: &str = "glyphs/";

/// The file stem the reference gives a glyph's file.
pub fn glyph_file_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        match c {
            '.' if i == 0 => out.push('_'),
            c if c.is_ascii_uppercase() => {
                out.push(c);
                out.push('_');
            }
            c => out.push(c),
        }
    }
    out
}

fn text<'a>(files: &'a Files, path: &str) -> Result<&'a str> {
    let bytes = files.get(path).ok_or_else(|| bad(path, "the package has no such file"))?;
    std::str::from_utf8(bytes).map_err(|_| bad(path, "not UTF-8 text"))
}

/// Reads a package from its files.
pub fn read_package(files: &Files) -> Result<Font> {
    let info = openstep::parse(text(files, FONTINFO)?)?;
    let Value::Dict(mut root) = info.root else { return Err(bad(FONTINFO, "the top level is not a dictionary")) };
    if root.get("glyphs").is_some() {
        return Err(bad(FONTINFO, "glyphs belong in the glyphs folder"));
    }
    let order: Vec<String> = match files.get(ORDER) {
        Some(_) => match openstep::parse(text(files, ORDER)?)?.root {
            Value::Array(a) => a.items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => return Err(bad(ORDER, "expected a list of glyph names")),
        },
        None => Vec::new(),
    };
    // Each glyph file, by glyph name.
    let mut by_name: BTreeMap<String, (String, Value)> = BTreeMap::new();
    let mut stray: Vec<(String, String)> = Vec::new();
    for path in files.keys().filter(|p| p.starts_with(GLYPHS_DIR) && p.ends_with(".glyph")) {
        let stem = path.get(GLYPHS_DIR.len()..path.len() - ".glyph".len()).unwrap_or_default();
        if stem.contains('/') {
            continue;
        }
        let doc = openstep::parse(text(files, path)?).map_err(|e| bad(path, e.to_string()))?;
        let name =
            doc.root.as_dict().and_then(|d| d.get("glyphname")).and_then(Value::as_str).ok_or_else(|| bad(path, "a glyph file has no glyphname"))?;
        let name = name.to_string();
        stray.push((stem.to_string(), name.clone()));
        if by_name.insert(name.clone(), (stem.to_string(), doc.root)).is_some() {
            return Err(bad(path, format!("two glyph files hold `{name}`")));
        }
    }
    // Glyph order: as `order.plist` says, then any glyph it leaves out, by file name.
    let mut ordered: Vec<(String, Value)> = Vec::with_capacity(by_name.len());
    for name in &order {
        if let Some(entry) = by_name.remove(name) {
            ordered.push(entry);
        }
    }
    stray.sort();
    for (_, name) in stray {
        if let Some(entry) = by_name.remove(&name) {
            ordered.push(entry);
        }
    }
    let stems: Vec<String> = ordered.iter().map(|(s, _)| s.clone()).collect();
    root.entries.push(("glyphs".into(), Value::Array(Array { items: ordered.into_iter().map(|(_, v)| v).collect(), layout: Layout::Multiline })));
    let mut font = read_root(root)?;
    font.source.format = SourceFormat::GlyphsPackage;
    for (i, stem) in stems.iter().enumerate() {
        if let Some(mut g) = font.glyphs.get_mut_at(i)
            && glyph_file_stem(&g.name) != *stem
        {
            g.file_name = Some(stem.clone());
        }
    }
    let side: BTreeMap<String, Arc<[u8]>> = files
        .iter()
        .filter(|(p, _)| p.as_str() != FONTINFO && p.as_str() != ORDER && !(p.starts_with(GLYPHS_DIR) && p.ends_with(".glyph")))
        .map(|(p, b)| (p.clone(), Arc::from(b.as_slice())))
        .collect();
    font.files = Arc::new(FileStore(side));
    font.check()?;
    Ok(font)
}

/// Writes a document as a package's files.
pub fn write_package(font: &Font) -> Result<Files> {
    let (mut root, dialect) = write_root(font)?;
    let glyphs = match take(&mut root, "glyphs") {
        Some(Value::Array(a)) => a.items,
        _ => Vec::new(),
    };
    let write = |v: Value, d: Dialect| openstep::write(&openstep::Document::new(v), d).into_bytes();
    let mut out = Files::new();
    out.insert(FONTINFO.into(), write(Value::Dict(root), dialect));
    let names: Vec<Value> = font.glyphs.iter().map(|g| Value::String(Str::new(g.name.as_str()))).collect();
    let mut order = openstep::Document::new(Value::Array(Array { items: names, layout: Layout::Multiline }));
    order.trailing = String::new();
    out.insert(ORDER.into(), openstep::write(&order, dialect).into_bytes());
    for (g, value) in font.glyphs.iter().zip(glyphs) {
        let stem = g.file_name.clone().unwrap_or_else(|| glyph_file_stem(&g.name));
        if stem.is_empty() || stem.contains(['/', '\\', '\0']) || stem == ".." {
            return Err(GlyphsError::Unsupported(format!("glyph `{}`: its name cannot be a file name in a package", g.name)));
        }
        let path = format!("{GLYPHS_DIR}{stem}.glyph");
        if out.insert(path.clone(), write(value, dialect)).is_some() {
            return Err(GlyphsError::Unsupported(format!("two glyphs would be written to {path}")));
        }
    }
    for (path, bytes) in &font.files.0 {
        out.entry(path.clone()).or_insert_with(|| bytes.to_vec());
    }
    Ok(out)
}
