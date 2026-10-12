//! The URL scheme: `fontcraft://show/?path=…&glyph=…&production=…&layer=…`. Rows: `url.open`,
//! `url.show`, `url.show.*`, `url.safety`.
//!
//! URLs come from other apps and web pages, so they are untrusted. Only the `show` action is
//! accepted: it opens font files (or finds them already open) and resolves the glyph layers to
//! show; the shell's `url.show` adds the tabs. A URL never runs another command, writes a file or
//! loads a module. Parsing is strict and finishes before anything opens, so a malformed URL
//! changes nothing; within a well-formed URL, a file that cannot be opened and a glyph or layer
//! the font lacks are skipped with a warning.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use super::*;
use crate::params::Params;

/// The scheme FontCraft answers to.
pub const SCHEME: &str = "fontcraft";
/// The longest URL read, in bytes.
pub const MAX_URL_LEN: usize = 64 * 1024;
/// The most `path` parameters (files) in one URL.
pub const MAX_PATHS: usize = 64;
/// The most `glyph` and `production` parameters in one URL, over all its files.
pub const MAX_GLYPHS: usize = 10_000;
/// The most `layer` parameters after one glyph.
pub const MAX_LAYERS_PER_GLYPH: usize = 64;
/// The most warnings listed; the rest are counted.
const MAX_WARNINGS: usize = 100;
/// The most characters of an untrusted value quoted in a message.
const MAX_QUOTED: usize = 100;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "url.open",
        "Open URL",
        [],
        None,
        "{url?: string, files?: [{path: string, glyphs?: [{glyph?: name, production?: name, layers?: [id]}]}]} handle a fontcraft://show/?path=…&glyph=…&production=…&layer=… URL (or the same request as `files`): open each file given by its absolute path unless it is already open, and resolve the glyph layers to show in a new tab of each, in order. `glyph` names a glyph, `production` names it by its production name, `layer` (repeatable) picks a layer of the glyph before it by id; a glyph without `layer` shows its layer of the first master. Values are percent-decoded as UTF-8; `+` stays a plus sign. Only the show action is accepted: any other URL is refused and nothing opens. Files that cannot be opened and glyphs or layers the font lacks are skipped with a warning; the command fails when no file could be opened. The last file's document becomes the active one. Limits: 64 KiB, 64 files, 10,000 glyphs, 64 layers per glyph → {tabs: [{document, title, path, opened, glyphs: [{glyph, layer, master}]}], warnings}",
        always,
        open
    )]
}

/// What a URL asks for, before any file is touched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub files: Vec<FileRequest>,
    /// Parameters that were skipped, in URL order.
    pub warnings: Vec<String>,
}

/// One `path` and the glyphs that follow it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileRequest {
    pub path: String,
    pub glyphs: Vec<GlyphRequest>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GlyphRef {
    /// `glyph`: the glyph's name.
    Name(String),
    /// `production`: the glyph's production name.
    Production(String),
}

/// One glyph and the layers of it to show (empty: the first master's layer).
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRequest {
    pub glyph: GlyphRef,
    pub layers: Vec<String>,
}

/// Warnings with a cap, so a hostile URL cannot make an unbounded list.
#[derive(Default)]
struct Warnings {
    list: Vec<String>,
    dropped: usize,
}

impl Warnings {
    fn push(&mut self, message: String) {
        if self.list.len() < MAX_WARNINGS {
            self.list.push(message);
        } else {
            self.dropped += 1;
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.dropped > 0 {
            self.list.push(format!("{} more warnings were left out", self.dropped));
        }
        self.list
    }
}

/// An untrusted value cut for quoting in a message.
fn quoted(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(MAX_QUOTED).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

/// Parses a `fontcraft://show/?…` URL. Anything but the show action, a malformed URL and a URL
/// over a limit are refused with a message; unknown parameters, and `glyph` or `layer` with
/// nothing to attach to, are skipped with a warning.
pub fn parse_url(url: &str) -> std::result::Result<Request, String> {
    if url.len() > MAX_URL_LEN {
        return Err(format!("the URL is longer than {} KiB", MAX_URL_LEN / 1024));
    }
    if url.bytes().any(|b| !b.is_ascii_graphic()) {
        return Err("the URL holds a space, a control character or a character outside ASCII: percent-encode it".into());
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return Err(format!("`{}` is not a URL", quoted(url)));
    };
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return Err(format!("only {SCHEME}:// URLs are accepted, not `{}:`", quoted(scheme)));
    }
    let Some(rest) = rest.strip_prefix("//") else {
        return Err(format!("a {SCHEME} URL starts with {SCHEME}://show/"));
    };
    if rest.contains('#') {
        return Err("a URL with a fragment (#) is refused".into());
    }
    let (before_query, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (action, path) = before_query.split_once('/').unwrap_or((before_query, ""));
    if !action.eq_ignore_ascii_case("show") || !path.is_empty() {
        return Err(format!("a URL can only open files and show glyphs ({SCHEME}://show/?path=…); `{}` is refused", quoted(before_query)));
    }
    let mut files: Vec<FileRequest> = Vec::new();
    let mut warnings = Warnings::default();
    let mut glyphs = 0usize;
    // Whether a `layer` now has a glyph to attach to: false after a path or a skipped glyph.
    let mut glyph_open = false;
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        let key = decode(key)?;
        let value = decode(value)?;
        match key.as_str() {
            "path" => {
                if files.len() >= MAX_PATHS {
                    return Err(format!("the URL names more than {MAX_PATHS} files"));
                }
                files.push(FileRequest { path: value, glyphs: Vec::new() });
                glyph_open = false;
            }
            "glyph" | "production" => {
                glyphs += 1;
                if glyphs > MAX_GLYPHS {
                    return Err(format!("the URL names more than {MAX_GLYPHS} glyphs"));
                }
                glyph_open = false;
                let Some(file) = files.last_mut() else {
                    warnings.push(format!("`{key}={}` comes before any `path` and is skipped", quoted(&value)));
                    continue;
                };
                if value.is_empty() {
                    warnings.push(format!("a `{key}` with no value is skipped"));
                    continue;
                }
                let glyph = if key == "glyph" { GlyphRef::Name(value) } else { GlyphRef::Production(value) };
                file.glyphs.push(GlyphRequest { glyph, layers: Vec::new() });
                glyph_open = true;
            }
            "layer" => {
                let current = files.last_mut().and_then(|f| f.glyphs.last_mut()).filter(|_| glyph_open);
                let Some(glyph) = current else {
                    warnings.push(format!("`layer={}` follows no glyph and is skipped", quoted(&value)));
                    continue;
                };
                if glyph.layers.len() >= MAX_LAYERS_PER_GLYPH {
                    return Err(format!("the URL gives more than {MAX_LAYERS_PER_GLYPH} layers for one glyph"));
                }
                if value.is_empty() {
                    warnings.push("a `layer` with no value is skipped".into());
                    continue;
                }
                glyph.layers.push(value);
            }
            _ => warnings.push(format!("the parameter `{}` is not known and is skipped", quoted(&key))),
        }
    }
    Ok(Request { files, warnings: warnings.finish() })
}

/// Percent-decodes one query key or value as UTF-8. A `%` not followed by two hexadecimal
/// digits, bytes that are not UTF-8 and decoded control characters refuse the URL. `+` is kept.
fn decode(text: &str) -> std::result::Result<String, String> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let digits = (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex));
            let (Some(high), Some(low)) = digits else {
                return Err("the URL has a `%` that is not followed by two hexadecimal digits".into());
            };
            out.push((high << 4) | low);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).map_err(|_| "the URL has a percent-encoded value that is not UTF-8".to_string())?;
    if decoded.chars().any(char::is_control) {
        return Err("the URL has a percent-encoded control character".into());
    }
    Ok(decoded)
}

/// Reads the `files` form of a request, with the URL's limits.
fn parse_files(p: &Params, files: &[Value]) -> Result<Request> {
    if files.len() > MAX_PATHS {
        return Err(p.bad(format!("`files` names more than {MAX_PATHS} files")));
    }
    let text = |v: Option<&Value>, what: &str| -> Result<Option<String>> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.chars().any(char::is_control) => Err(p.bad(format!("{what} holds a control character"))),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(p.bad(format!("{what} must be a string"))),
        }
    };
    let mut out = Vec::with_capacity(files.len());
    let mut glyph_count = 0usize;
    for file in files {
        let Some(obj) = file.as_object() else {
            return Err(p.bad("each entry of `files` must be an object {path, glyphs}"));
        };
        let path = text(obj.get("path"), "`path`")?.ok_or_else(|| p.bad("each entry of `files` needs a `path`"))?;
        let entries = match obj.get("glyphs") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(a)) => a.as_slice(),
            Some(_) => return Err(p.bad("`glyphs` must be a list")),
        };
        glyph_count = glyph_count.saturating_add(entries.len());
        if glyph_count > MAX_GLYPHS {
            return Err(p.bad(format!("`files` names more than {MAX_GLYPHS} glyphs")));
        }
        let mut glyphs = Vec::with_capacity(entries.len());
        for entry in entries {
            let Some(g) = entry.as_object() else {
                return Err(p.bad("each glyph must be an object {glyph | production, layers?}"));
            };
            let glyph = match (text(g.get("glyph"), "`glyph`")?, text(g.get("production"), "`production`")?) {
                (Some(name), None) => GlyphRef::Name(name),
                (None, Some(name)) => GlyphRef::Production(name),
                _ => return Err(p.bad("each glyph needs `glyph` or `production`, not both")),
            };
            let layers = match g.get("layers") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) if a.len() > MAX_LAYERS_PER_GLYPH => {
                    return Err(p.bad(format!("more than {MAX_LAYERS_PER_GLYPH} layers for one glyph")));
                }
                Some(Value::Array(a)) => {
                    let mut ids = Vec::with_capacity(a.len());
                    for id in a {
                        ids.push(text(Some(id), "a layer id")?.ok_or_else(|| p.bad("a layer id must be a string"))?);
                    }
                    ids
                }
                Some(_) => return Err(p.bad("`layers` must be a list of layer ids")),
            };
            glyphs.push(GlyphRequest { glyph, layers });
        }
        out.push(FileRequest { path, glyphs });
    }
    Ok(Request { files: out, warnings: Vec::new() })
}

fn open(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("url.open", v)?;
    let request = match (p.opt_str("url")?, p.opt_array("files")?) {
        (Some(url), None) => parse_url(url).map_err(|e| p.bad(e))?,
        (None, Some(files)) => parse_files(&p, files)?,
        _ => return Err(p.bad("give `url` or `files`")),
    };
    if request.files.is_empty() {
        return Err(p.bad("the URL names no file: give one with `path`"));
    }
    let mut warnings = Warnings::default();
    for w in request.warnings {
        warnings.push(w);
    }
    let mut tabs = Vec::new();
    let mut last = None;
    for file in &request.files {
        let Some((index, opened)) = open_file(s, &file.path, &mut warnings) else { continue };
        s.set_active(index)?;
        let glyphs = resolve(s, file, &mut warnings)?;
        let doc = s.doc()?;
        tabs.push(json!({
            "document": doc.id, "title": doc.title, "path": doc.path.as_ref().map(|p| p.to_string_lossy()),
            "opened": opened, "glyphs": glyphs,
        }));
        last = Some(index);
    }
    let warnings = warnings.finish();
    let Some(last) = last else {
        return Err(p.bad(format!("no file could be opened: {}", warnings.join("; "))));
    };
    s.set_active(last)?;
    Ok(json!({ "tabs": tabs, "warnings": warnings }))
}

/// Finds the document of `path` among the open ones, or opens it. Returns its position and
/// whether this opened it; `None` (with a warning) when the path is refused or cannot be opened.
fn open_file(s: &mut Session, path: &str, warnings: &mut Warnings) -> Option<(usize, bool)> {
    if path.is_empty() {
        warnings.push("a `path` with no value is skipped, with the glyphs after it".into());
        return None;
    }
    // Network shares (`//server/…`, `\\server\…`) are not local files.
    if path.starts_with("//") || path.starts_with("\\\\") {
        warnings.push(format!("`{}` is a network path; only local files open from a URL", quoted(path)));
        return None;
    }
    let file = Path::new(path);
    if !file.is_absolute() {
        warnings.push(format!("`{}` is not an absolute path and is skipped", quoted(path)));
        return None;
    }
    if let Some(index) = super::session::find_open(s, file) {
        return Some((index, false));
    }
    // A device or a pipe would never finish reading: only files and folders (packages) open.
    match std::fs::metadata(file) {
        Ok(meta) if meta.is_file() || meta.is_dir() => {}
        Ok(_) => {
            warnings.push(format!("`{}` is not a file and is skipped", quoted(path)));
            return None;
        }
        Err(e) => {
            warnings.push(format!("`{}` cannot be opened: {e}", quoted(path)));
            return None;
        }
    }
    let before = s.documents().len();
    match s.execute("file.open", &json!({ "path": path })) {
        Ok(reply) => {
            let title = reply.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
            for w in reply.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                warnings.push(format!("{title}: {w}"));
            }
            let id = s.active().map(|d| d.id)?;
            Some((s.position_of(id)?, s.documents().len() > before))
        }
        Err(e) => {
            // file.open adds the document before its last step; a failure after that leaves it.
            if s.documents().len() > before {
                let _ = s.close_active();
            }
            warnings.push(format!("`{}` cannot be opened: {e}", quoted(path)));
            None
        }
    }
}

/// The layers of the active document that `file` asks to show, in order, as
/// `{glyph, layer, master}`. Glyphs and layers the font lacks are skipped with a warning.
fn resolve(s: &mut Session, file: &FileRequest, warnings: &mut Warnings) -> Result<Vec<Value>> {
    let (font, title) = {
        let doc = s.doc()?;
        (doc.font.clone(), doc.title.clone())
    };
    if file.glyphs.is_empty() {
        return Ok(Vec::new());
    }
    let Some(main) = font.masters.first() else {
        warnings.push(format!("{title}: the font has no master, so no glyph is shown"));
        return Ok(Vec::new());
    };
    let production: HashMap<String, &str> = if file.glyphs.iter().any(|g| matches!(g.glyph, GlyphRef::Production(_))) {
        let props = s.effective_props()?;
        let mut map = HashMap::new();
        for g in font.glyphs.as_arcs() {
            let name = props.get(g).and_then(|p| p.production_name.value.clone()).unwrap_or_else(|| g.name.clone());
            map.entry(name).or_insert(g.name.as_str());
        }
        map
    } else {
        HashMap::new()
    };
    let mut out = Vec::new();
    for request in &file.glyphs {
        let glyph = match &request.glyph {
            GlyphRef::Name(name) => font.glyph(name).ok_or_else(|| format!("{title}: no glyph named `{}`; skipped", quoted(name))),
            GlyphRef::Production(name) => production
                .get(name)
                .and_then(|n| font.glyph(n))
                .ok_or_else(|| format!("{title}: no glyph has the production name `{}`; skipped", quoted(name))),
        };
        let glyph = match glyph {
            Ok(g) => g,
            Err(message) => {
                warnings.push(message);
                continue;
            }
        };
        if request.layers.is_empty() {
            match glyph.layer(&main.id) {
                Some(layer) => out.push(json!({ "glyph": glyph.name, "layer": layer.id, "master": layer.master })),
                None => warnings.push(format!("{title}: `{}` has no layer for the first master; skipped", glyph.name)),
            }
            continue;
        }
        for id in &request.layers {
            match glyph.layers.iter().find(|l| l.id == *id) {
                Some(layer) => out.push(json!({ "glyph": glyph.name, "layer": layer.id, "master": layer.master })),
                None => warnings.push(format!("{title}: `{}` has no layer `{}`; skipped", glyph.name, quoted(id))),
            }
        }
    }
    Ok(out)
}
