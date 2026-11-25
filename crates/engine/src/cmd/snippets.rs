//! Feature code snippets: the built-in list and the user's snippet folder (M8.16).
//!
//! Commands: `font.features.listSnippets`, `insertSnippet`, `showSnippetFolder`.
//!
//! The snippet folder is the session's ([`Session::snippets_dir`]): the desktop app sets it to
//! `snippets` in the folder it keeps its settings in. Headless sessions, tests and the web app
//! have none unless a command names one with `folder`. File names, formats and limits are
//! `fontcraft_features::snippets`'.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_features::snippets::{self, MAX_SNIPPET_FILE_BYTES, MAX_SNIPPET_FILES, Snippet as CodeSnippet};
use fontcraft_features::{EntryKind, MAX_ENTRIES, MAX_ENTRY_BYTES};
use fontcraft_model::{Features, Snippet as Entry};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most entries of the snippet folder looked at.
const MAX_FOLDER_ENTRIES: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.features.listSnippets",
            "List Code Snippets",
            [],
            None,
            "{folder?: path (default: the session's snippet folder, which the desktop app sets and a headless session lacks)} the snippets the Features tab's snippets menu offers: the built-in ones, then one group per user file FeatureSnippets<NAME>.plist or .toml in the folder, by file name. A file that cannot be read is left out with a warning; a missing folder, or none, has no files → {folder: path|null, builtIn: [{title, code}], groups: [{name, file, snippets: [{title, code}]}], warnings: [text]}",
            always,
            list
        ).files(const { &[FileParam::read("folder")] }),
        cmd!(
            "font.features.insertSnippet",
            "Insert Snippet",
            [],
            None,
            "{kind: \"prefix\"|\"class\"|\"feature\", name, index?, snippet: title, group?: user group name (default: the built-in snippets, then the user groups in menu order), position?: character offset in the entry's code (default: its end), end?: character offset (default: position), folder?: path} put the snippet's code in place of the characters from position to end, as choosing it from the snippets menu does over a selection. An automatic entry is refused → {kind, name, index, caret: character offset after the inserted code}",
            has_doc,
            insert
        ).files(const { &[FileParam::read("folder")] }),
        cmd!(
            "font.features.showSnippetFolder",
            "Show Snippet Folder",
            [],
            None,
            "{folder?: path (default: the session's snippet folder)} the folder user snippet files go in, created when missing; the app opens it in the file manager. Refused when there is no folder → {path, created: bool}",
            always,
            show_folder
        ).files(const { &[FileParam::write("folder")] }).writes_files(),
    ]
}

/// The folder the parameters name, else the session's ([`Session::snippets_dir`]).
fn folder_param(s: &Session, p: &Params) -> Result<Option<PathBuf>> {
    match p.opt_str("folder")? {
        Some("") => Err(p.bad("`folder` is empty")),
        Some(f) => Ok(Some(PathBuf::from(f))),
        None => Ok(s.snippets_dir().map(Path::to_path_buf)),
    }
}

/// One file's bytes, refused when it is not a regular file or is larger than a snippet file may be.
fn read_capped(path: &Path) -> std::result::Result<Vec<u8>, String> {
    let too_big = |n: u64| format!("{n} bytes, more than the {MAX_SNIPPET_FILE_BYTES} a snippet file may have");
    // Checked before opening: opening a named pipe would wait for a writer.
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    let limit = MAX_SNIPPET_FILE_BYTES as u64;
    if meta.len() > limit {
        return Err(too_big(meta.len()));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SNIPPET_FILE_BYTES {
        return Err(too_big(bytes.len() as u64));
    }
    Ok(bytes)
}

/// The snippet files of `dir` as (file name, bytes) in name order, and a message for each file
/// or problem left out. A missing folder has no files and no message.
fn read_folder(dir: &Path) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
    let mut warnings = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), warnings),
        Err(e) => {
            warnings.push(format!("{}: {e}", dir.display()));
            return (Vec::new(), warnings);
        }
    };
    let mut names = Vec::new();
    for (seen, entry) in entries.enumerate() {
        if seen >= MAX_FOLDER_ENTRIES {
            warnings.push(format!("{}: more than {MAX_FOLDER_ENTRIES} entries; the rest were not looked at", dir.display()));
            break;
        }
        let Ok(entry) = entry else { continue };
        let Ok(name) = entry.file_name().into_string() else { continue };
        if snippets::snippet_file(&name).is_some() {
            names.push(name);
        }
    }
    names.sort();
    if names.len() > MAX_SNIPPET_FILES {
        warnings.push(format!("{} snippet files found; only the first {MAX_SNIPPET_FILES} by name are read", names.len()));
        names.truncate(MAX_SNIPPET_FILES);
    }
    let mut files = Vec::new();
    for name in names {
        match read_capped(&dir.join(&name)) {
            Ok(bytes) => files.push((name, bytes)),
            Err(message) => warnings.push(format!("{name}: {message}")),
        }
    }
    (files, warnings)
}

/// The user's snippets in `folder`, and the warnings of reading them.
fn user_snippets(folder: Option<&Path>) -> (Vec<CodeSnippet>, Vec<String>) {
    let Some(dir) = folder else { return (Vec::new(), Vec::new()) };
    let (files, mut warnings) = read_folder(dir);
    let named: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let user = snippets::read_user(&named);
    warnings.extend(user.warnings);
    (user.snippets, warnings)
}

fn snippet_json(s: &CodeSnippet) -> Value {
    json!({ "title": s.name, "code": s.code })
}

fn list(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.listSnippets", raw)?;
    let folder = folder_param(s, &p)?;
    let (user, warnings) = user_snippets(folder.as_deref());
    // One group per file, in the order read_user gives (file-name order).
    let mut groups: Vec<(String, String, Vec<Value>)> = Vec::new();
    for s in &user {
        let snippets::SnippetSource::User { file } = &s.source else { continue };
        match groups.last_mut() {
            Some((_, last, items)) if last == file => items.push(snippet_json(s)),
            _ => groups.push((s.source.group().unwrap_or_default().to_string(), file.clone(), vec![snippet_json(s)])),
        }
    }
    let groups: Vec<Value> = groups.into_iter().map(|(name, file, items)| json!({ "name": name, "file": file, "snippets": items })).collect();
    let builtin: Vec<Value> = snippets::builtin().iter().map(snippet_json).collect();
    Ok(json!({
        "folder": folder.map(|f| f.to_string_lossy().into_owned()),
        "builtIn": builtin,
        "groups": groups,
        "warnings": warnings,
    }))
}

fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Prefix => "prefix",
        EntryKind::Class => "class",
        EntryKind::Feature => "feature",
    }
}

fn group_mut(f: &mut Features, kind: EntryKind) -> &mut Vec<Entry> {
    match kind {
        EntryKind::Prefix => &mut f.prefixes,
        EntryKind::Class => &mut f.classes,
        EntryKind::Feature => &mut f.features,
    }
}

/// The entry the parameters `kind`, `name` and `index?` name, as (kind, index).
fn entry_param(p: &Params, f: &Features) -> Result<(EntryKind, usize)> {
    let kind = match p.str("kind")? {
        "prefix" => EntryKind::Prefix,
        "class" => EntryKind::Class,
        "feature" => EntryKind::Feature,
        other => return Err(p.bad(format!("`kind` must be \"prefix\", \"class\" or \"feature\", got \"{other}\""))),
    };
    let raw_name = p.str("name")?;
    let name = if kind == EntryKind::Class { raw_name.strip_prefix('@').unwrap_or(raw_name) } else { raw_name };
    let entries = match kind {
        EntryKind::Prefix => &f.prefixes,
        EntryKind::Class => &f.classes,
        EntryKind::Feature => &f.features,
    };
    let what = kind_name(kind);
    let index = match p.opt_int("index", 0, MAX_ENTRIES as i64)? {
        Some(i) => {
            let i = i as usize;
            let at = entries.get(i).ok_or_else(|| p.bad(format!("the font has no {what} at index {i}")))?;
            if at.name != name {
                return Err(p.bad(format!("the {what} at index {i} is `{}`, not `{name}`", at.name)));
            }
            i
        }
        None => entries.iter().position(|e| e.name == name).ok_or_else(|| p.bad(format!("the font has no {what} `{name}`")))?,
    };
    Ok((kind, index))
}

/// The code of the snippet the parameters name.
fn snippet_code(s: &Session, p: &Params) -> Result<String> {
    let title = p.str("snippet")?;
    let group = p.opt_str("group")?;
    if group.is_none()
        && let Some(s) = snippets::builtin().into_iter().find(|s| s.name == title)
    {
        return Ok(s.code);
    }
    let (user, _) = user_snippets(folder_param(s, p)?.as_deref());
    user.into_iter().find(|s| s.name == title && group.is_none_or(|g| s.source.group() == Some(g))).map(|s| s.code).ok_or_else(|| match group {
        Some(g) => p.bad(format!("the snippet group `{g}` has no snippet `{title}`")),
        None => p.bad(format!("no snippet is titled `{title}`")),
    })
}

fn insert(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.insertSnippet", raw)?;
    let (kind, index) = entry_param(&p, &s.font()?.features)?;
    let code = snippet_code(s, &p)?;
    let features = &s.font()?.features;
    let entries = match kind {
        EntryKind::Prefix => &features.prefixes,
        EntryKind::Class => &features.classes,
        EntryKind::Feature => &features.features,
    };
    let e = entries.get(index).ok_or_else(|| p.bad("the entry is gone"))?;
    if e.automatic {
        return Err(p.bad(format!(
            "the {} `{}` is generated automatically: turn off Generate Feature Automatically to edit its code",
            kind_name(kind),
            e.name
        )));
    }
    let name = e.name.clone();
    let old = &e.code;
    let count = old.chars().count();
    let limit = MAX_ENTRY_BYTES as i64;
    let start = p.opt_int("position", 0, limit)?.map_or(count, |v| v as usize);
    let end = p.opt_int("end", 0, limit)?.map_or(start, |v| v as usize);
    if start > count || end > count {
        return Err(p.bad(format!("`position` and `end` must lie within the code's {count} characters")));
    }
    if end < start {
        return Err(p.bad("`end` must not come before `position`"));
    }
    let byte = |chars: usize| old.char_indices().map(|(b, _)| b).chain(std::iter::once(old.len())).nth(chars);
    let (Some(head), Some(tail)) = (byte(start).and_then(|b| old.get(..b)), byte(end).and_then(|b| old.get(b..))) else {
        return Err(p.bad("`position` and `end` must lie within the code"));
    };
    let new_len = head.len().saturating_add(code.len()).saturating_add(tail.len());
    if new_len > MAX_ENTRY_BYTES {
        return Err(p.bad(format!("the code would have {new_len} bytes, more than the {MAX_ENTRY_BYTES} one entry may hold")));
    }
    let new_code = [head, code.as_str(), tail].concat();
    let caret = start.saturating_add(code.chars().count());
    let command = p.command();
    s.edit("Insert Snippet", |font| {
        let e = group_mut(Arc::make_mut(&mut font.features), kind)
            .get_mut(index)
            .ok_or_else(|| EngineError::BadParams { command: command.into(), message: "the entry is gone".into() })?;
        e.code = new_code;
        Ok(())
    })?;
    Ok(json!({ "kind": kind_name(kind), "name": name, "index": index, "caret": caret }))
}

fn show_folder(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.showSnippetFolder", raw)?;
    let Some(dir) = folder_param(s, &p)? else {
        return Err(EngineError::Unsupported("a snippet folder: this session has no settings folder (pass `folder`)".into()));
    };
    let existed = dir.is_dir();
    if !existed {
        fontcraft_format::output::create_dir_all(&dir).map_err(|e| EngineError::Io(format!("{}: {e}", dir.display())))?;
    }
    Ok(json!({ "path": dir.to_string_lossy(), "created": !existed }))
}
