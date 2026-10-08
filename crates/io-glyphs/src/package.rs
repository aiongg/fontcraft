//! The package form, `.glyphspackage`: the same document split into files in a directory.
//!
//! - `fontinfo.plist`: everything except the glyphs.
//! - `order.plist`: the glyph names in glyph order. Unlike the other files it does not end in a
//!   line break.
//! - `glyphs/<name>.glyph`: one glyph each. The file name is the glyph name with `_` after every
//!   uppercase letter and a leading `.` turned into `_` (`A_.glyph`, `_notdef.glyph`); a glyph
//!   stored under another name keeps it in [`Glyph::file_name`](fontcraft_model::Glyph).
//! - `UIState.plist`: the edit tabs' display strings (`displayStrings`).
//! - Format version 4 only: `features/*.fea` with each prefix's, class's and feature's code (the
//!   entry in `fontinfo.plist` names its file in `file`), `kerning.plist` with the kerning tables,
//!   and `note.md` with the font note. Version 3 keeps these in `fontinfo.plist`.
//! - Anything else is kept in [`Font::files`] and written back unchanged.
//!
//! The reader moves the side files' contents into the root dictionary before reading it, so a
//! package and a single file of the same font give the same model; the writer moves them back.
//! Feature files are named `tag.fea` (`tag.2.fea` for the second feature with the same tag…),
//! `@Class.fea` and `_Prefix.fea`; a snippet stored under another name keeps it.
//!
//! The functions here work on file contents by relative path; reading and writing the directory
//! is the caller's job, so they also run in the browser.

use std::collections::BTreeMap;
use std::sync::Arc;

use fontcraft_model::{FileStore, Font, SourceFormat};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Dict, Layout, Str, Value};

use crate::{GlyphsError, Result, bad, read_root, take, write_root};

/// A package's files: relative path (with `/`) → contents.
pub type Files = BTreeMap<String, Vec<u8>>;

const FONTINFO: &str = "fontinfo.plist";
const ORDER: &str = "order.plist";
const GLYPHS_DIR: &str = "glyphs/";
const UI_STATE: &str = "UIState.plist";
const KERNING: &str = "kerning.plist";
const NOTE: &str = "note.md";
const FEATURES_DIR: &str = "features/";
const KERNING_KEYS: [&str; 3] = ["kerningLTR", "kerningRTL", "kerningVertical"];
/// The feature lists, with the key that names each item and the file-name rule's prefix.
const FEATURE_LISTS: [(&str, &str, &str); 3] = [("featurePrefixes", "name", "_"), ("classes", "name", "@"), ("features", "tag", "")];

/// The file names the rule gives the items of one feature list: `prefix` + name, and for a name
/// seen before, `.2`, `.3`… before the extension.
fn feature_file_names<'a>(names: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    names
        .map(|n| {
            let count = seen.entry(n).or_insert(0);
            *count += 1;
            if *count == 1 { format!("{prefix}{n}.fea") } else { format!("{prefix}{n}.{count}.fea") }
        })
        .collect()
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

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
    let mut consumed: Vec<String> = Vec::new();
    // Display strings, when the UI state holds nothing else.
    if files.contains_key(UI_STATE)
        && let Value::Dict(ui) = openstep::parse(text(files, UI_STATE)?).map_err(|e| bad(UI_STATE, e.to_string()))?.root
        && let [(key, strings @ Value::Array(_))] = ui.entries.as_slice()
        && key.as_str() == "displayStrings"
        && root.get("DisplayStrings").is_none()
    {
        root.entries.push(("DisplayStrings".into(), strings.clone()));
        consumed.push(UI_STATE.into());
    }
    if files.contains_key(KERNING) {
        let Value::Dict(k) = openstep::parse(text(files, KERNING)?).map_err(|e| bad(KERNING, e.to_string()))?.root else {
            return Err(bad(KERNING, "the top level is not a dictionary"));
        };
        if k.entries.iter().all(|(key, _)| KERNING_KEYS.contains(&key.as_str()) && root.get(key.as_str()).is_none()) {
            root.entries.extend(k.entries);
            consumed.push(KERNING.into());
        }
    }
    if files.contains_key(NOTE) && root.get("note").is_none() {
        root.entries.push(("note".into(), Value::String(Str::new(text(files, NOTE)?))));
        consumed.push(NOTE.into());
    }
    // Feature code kept in files: put it where a single file has it, and remember each file name.
    let mut feature_files: Vec<Vec<Option<String>>> = Vec::new();
    for (list_key, _, _) in FEATURE_LISTS {
        let mut names = Vec::new();
        if let Some(Value::Array(items)) = root.get_mut(list_key) {
            for item in &mut items.items {
                let file = match item {
                    Value::Dict(d) if d.get("code").is_none() => d.get("file").and_then(Value::as_str).map(str::to_string),
                    _ => None,
                };
                if let (Some(file), Value::Dict(d)) = (&file, &mut *item) {
                    let path = format!("{FEATURES_DIR}{file}");
                    if is_plain_file_name(file) && files.contains_key(&path) {
                        let code = text(files, &path)?.to_string();
                        take(d, "file");
                        d.entries.push(("code".into(), Value::String(Str::new(code))));
                        consumed.push(path);
                        names.push(Some(file.clone()));
                        continue;
                    }
                }
                names.push(None);
            }
        }
        feature_files.push(names);
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
    root.entries.push((
        "glyphs".into(),
        Value::Array(Array { items: ordered.into_iter().map(|(_, v)| v).collect(), layout: Layout::Multiline, ..Default::default() }),
    ));
    let mut font = read_root(root)?;
    font.source.format = SourceFormat::GlyphsPackage;
    for (i, stem) in stems.iter().enumerate() {
        if let Some(mut g) = font.glyphs.get_mut_at(i)
            && glyph_file_stem(&g.name) != *stem
        {
            g.file_name = Some(stem.clone());
        }
    }
    // A feature file whose name the rule would not give is remembered on its snippet.
    {
        let features = Arc::make_mut(&mut font.features);
        let lists = [&mut features.prefixes, &mut features.classes, &mut features.features];
        for ((list, (_, _, prefix)), read_names) in lists.into_iter().zip(FEATURE_LISTS).zip(&feature_files) {
            let rule = feature_file_names(list.iter().map(|s| s.name.as_str()), prefix);
            for ((snippet, rule_name), read_name) in list.iter_mut().zip(rule).zip(read_names) {
                if let Some(read_name) = read_name
                    && *read_name != rule_name
                {
                    snippet.file_name = Some(read_name.clone());
                }
            }
        }
    }
    let side: BTreeMap<String, Arc<[u8]>> = files
        .iter()
        .filter(|(p, _)| !consumed.contains(p))
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
    if let Some(strings) = take(&mut root, "DisplayStrings") {
        let mut ui = Dict::new();
        ui.entries.push(("displayStrings".into(), strings));
        out.insert(UI_STATE.into(), write(Value::Dict(ui), dialect));
    }
    if font.source.glyphs_version.unwrap_or(3) >= 4 {
        if let Some(Value::String(note)) = take(&mut root, "note") {
            out.insert(NOTE.into(), note.as_str().as_bytes().to_vec());
        }
        let mut kerning = Dict::new();
        for key in KERNING_KEYS {
            if let Some(table) = take(&mut root, key) {
                kerning.entries.push((key.into(), table));
            }
        }
        if !kerning.is_empty() {
            out.insert(KERNING.into(), write(Value::Dict(kerning), dialect));
        }
        let lists = [&font.features.prefixes, &font.features.classes, &font.features.features];
        for (snippets, (list_key, _, prefix)) in lists.into_iter().zip(FEATURE_LISTS) {
            let rule = feature_file_names(snippets.iter().map(|s| s.name.as_str()), prefix);
            let Some(Value::Array(items)) = root.get_mut(list_key) else { continue };
            for ((item, snippet), rule_name) in items.items.iter_mut().zip(snippets.iter()).zip(rule) {
                let Value::Dict(d) = item else { continue };
                let Some(Value::String(code)) = take(d, "code") else { continue };
                let name = snippet.file_name.clone().unwrap_or(rule_name);
                if !is_plain_file_name(&name) {
                    return Err(GlyphsError::Unsupported(format!("`{}`: its name cannot be a feature file name", snippet.name)));
                }
                d.entries.push(("file".into(), Value::String(Str::new(name.as_str()))));
                d.entries.sort_by(|a, b| a.0.as_str().as_bytes().cmp(b.0.as_str().as_bytes()));
                out.insert(format!("{FEATURES_DIR}{name}"), code.as_str().as_bytes().to_vec());
            }
        }
    }
    out.insert(FONTINFO.into(), write(Value::Dict(root), dialect));
    let names: Vec<Value> = font.glyphs.iter().map(|g| Value::String(Str::new(g.name.as_str()))).collect();
    let mut order = openstep::Document::new(Value::Array(Array { items: names, layout: Layout::Multiline, ..Default::default() }));
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
