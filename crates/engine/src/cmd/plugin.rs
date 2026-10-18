//! Module commands: install from a file or bytes or an index entry, remove, list, run a filter,
//! module info, `app.info` and `app.loadExtensions`. Rows: `plugin.installFromFile`,
//! `pluginManager.install`, `pluginManager.remove`, `pluginManager.list`, `plugin.uninstall`,
//! `plugin.runFilter`, `plugin.info`, `app.launchWithoutPlugins`, `plugin.lockedPathsAndGlyphs`,
//! `app.info`. The registry, the folders and the switches are in `crate::extensions`; the host is
//! `fontcraft_plugins`.
//!
//! Installing validates the file first (a module is loaded: manifest, ABI version, no imports,
//! limits; a macro is parsed), then writes it into the Modules or Macros folder, then lists it:
//! a refused file writes nothing. A session without folders (headless, the web build) keeps
//! installed modules for the session only. Removing moves the file into the Removed folder
//! beside the two folders, so it can be taken back by hand.
//!
//! `plugin.runFilter` runs a filter module on every target layer before it changes anything:
//! each output is checked against the model's limits, then one edit applies them all as one
//! undo step. A failing module, a broken output or a locked glyph changes nothing. Locked and
//! hidden paths go to the module like the others and may change (`plugin.lockedPathsAndGlyphs`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_model::{Font, Layer};
use fontcraft_plugins::error::{LIMITS_TABLE, MAX_INSTALLED, MAX_MODULE_BYTES};
use fontcraft_plugins::layer_json::{FilterOutput, LayerInput, NodeRef};
use fontcraft_plugins::{Kind, Limits, ParamSpec, Plugin, sha256};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::extension_index::{Entry, EntryKind};
use crate::extensions::{self, MODULE_EXTENSION};
use crate::macros::{self, MACRO_EXTENSION, MAX_MACRO_BYTES, Macro};
use crate::params::{Params, TargetKind, Targets};

/// Most layer ids one `layers` list may hold.
const MAX_LAYER_IDS: usize = 1024;
/// Longest `pluginManager.list` query, in characters.
const MAX_QUERY_CHARS: usize = 1_000;
/// Longest file name given to an install, in bytes.
const MAX_FILE_NAME: usize = 255;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "plugin.installFromFile",
            "Install Plug-in from File",
            [],
            None,
            "{path?: path to a .wasm module or a .fcmacro macro, dataBase64?: the file's bytes in place of path, name?: its file name in the Modules or Macros folder, replace?: bool} validate a module (manifest, ABI version, no imports, limits) or a macro, copy it into the Modules or Macros folder and turn it on at once, with no relaunch. A module whose id is installed already is replaced only with replace: true, and keeps its file name unless `name` gives another. With only `name`, turn on a module file that is already in the Modules folder (one copied there by hand). Without a Modules folder, the module is installed for this session only → {kind: \"filter\" | \"reporter\" | \"check\" | \"macro\", id?, name?, version?, file?, path?, enabled: true, replaced: bool, warning?}",
            always,
            install_from_file
        )
        .files(const { &[FileParam::read("path")] })
        .writes_files(),
        cmd!(
            "pluginManager.install",
            "Install",
            [],
            None,
            "{id: string, version?: string, index?: path to a local extension index (default: the session's indexes), dataBase64?: the entry's file, downloaded by the desktop app} install the module or macro an index entry names: its file is read from the path the entry gives (beside a local index) or taken from dataBase64, checked against the entry's size and SHA-256, and installed as plugin.installFromFile does, replacing an installed copy. A file that does not match its entry writes nothing → as plugin.installFromFile",
            always,
            install_from_index
        )
        .files(const { &[FileParam::read("index")] })
        .writes_files(),
        cmd!(
            "pluginManager.remove",
            "Remove",
            [],
            None,
            "{id: module id | macro path or title} uninstall a module or forget a macro: its menu entries go, and its file moves out of its folder into the Removed folder beside it. Documents that name a removed filter module in an export parameter report it at export → {id, kind, movedTo?: path}",
            always,
            remove
        )
        .writes_files(),
        cmd!(
            query "pluginManager.list",
            "List Extensions",
            [],
            None,
            "{state?: \"all\" | \"installed\" | \"notInstalled\" (all), type?: \"filter\" | \"reporter\" | \"check\" | \"macro\", query?: text found in the id, title, vendor or description, any case} the extensions the session's indexes offer, the installed modules and the macros of the Macros folder (an id in several indexes: the first index wins) → {entries: [{id, kind, title, vendor, version, description, installed: bool, installedVersion?, update?: bool, enabled?, new?, file?, size?, sha256?, source: \"index\" | \"installed\" | \"folder\", index?}], count, safeMode}",
            always,
            list
        ),
        cmd!(
            "plugin.runFilter",
            "(filter module name)",
            [],
            None,
            "{id: module id, params?: object (the module's parameters; missing ones take their defaults), glyph?: name | glyphs?: [names], layer?: id | layers?: [ids] (default: every master layer), points?: [[shape, node]] (with glyph and one layer: the Edit View selection, handed to the module), selectionOnly?: bool (with points: only the paths holding a selected node take the module's result; the module must return the layer's shapes in their order)} run an installed, turned-on filter module on each target layer; every result is checked against the model's limits before one edit applies them all as one undo step named after the module. A locked glyph refuses; locked and hidden paths go to the module and may change. A module that fails changes nothing → {id, glyphs: [changed], layers: count, skipped: [{glyph, layer, reason}], log: [lines]}",
            has_doc,
            run_filter
        )
        .targets(&[TargetKind::Glyphs, TargetKind::Layer, TargetKind::Points]),
        cmd!(
            query "plugin.info",
            "Extension Info",
            [],
            None,
            "{id: module id} an installed module's manifest, parameters and defaults, size, SHA-256, switch and last error → {id, name, version, kind, author, description, capabilities, params: [{name, type, min?, max?, default, options?}], defaults, paramsDoc, size, sha256, enabled, new, file?, runs: bool, lastError?}",
            always,
            info
        ),
        cmd!(
            query "app.info",
            "App Info",
            ["Help"],
            None,
            "{} the app's version, start time and uptime, safe mode, the control channel's and the MCP server's state, the installed modules, the modules that failed to load, the extension folders, the command log's size and the module limits → {version, started?, uptimeSeconds?, safeMode, surfaces: {control?, mcp?}, modules: [{id, name, kind, version, author, enabled, new, file?, size, sha256}], failedModules: [{file, error}], modulesFolder?, macrosFolder?, macros: count, log: {entries}, limits: {name: value}}",
            always,
            app_info
        ),
        cmd!(
            "app.loadExtensions",
            "Load Extensions",
            ["Help"],
            None,
            "{} leave safe mode: the modules that are turned on run again, for this session → {safeMode: false, enabled: count}",
            in_safe_mode,
            load_extensions
        ),
    ]
}

fn in_safe_mode(s: &Session) -> std::result::Result<(), String> {
    if s.extensions().safe_mode { Ok(()) } else { Err("extensions are on already".into()) }
}

// ── Installing ──────────────────────────────────────────────────────────────────────────────────

/// Where an install's bytes came from.
struct Source<'a> {
    bytes: Vec<u8>,
    /// The file name the bytes had (a path's last part, an index entry's file).
    file_name: Option<&'a str>,
}

fn install_from_file(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("plugin.installFromFile", v)?;
    let replace = p.bool_or("replace", false)?;
    let name = p.opt_str("name")?;
    let source = match (p.opt_str("path")?, p.opt_str("dataBase64")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `path` or `dataBase64`, not both")),
        (Some(path), None) => {
            let bytes = read_capped(&p, Path::new(path))?;
            Source { bytes, file_name: Path::new(path).file_name().and_then(|n| n.to_str()) }
        }
        (None, Some(data)) => Source { bytes: decode_base64(&p, data)?, file_name: None },
        (None, None) => {
            let name = name.ok_or_else(|| p.bad("give `path`, `dataBase64`, or the `name` of a module file in the Modules folder"))?;
            return turn_on_folder_file(s, &p, name);
        }
    };
    install(s, &p, source, name, replace)
}

/// Installs a module or a macro from `source`, telling them apart by the WebAssembly header and
/// the file names.
fn install(s: &mut Session, p: &Params, source: Source, name: Option<&str>, replace: bool) -> Result<Value> {
    let named = name.or(source.file_name);
    if source.bytes.starts_with(b"\0asm") || named.is_some_and(extensions::is_module_file) {
        install_module(s, p, &source.bytes, name, source.file_name, replace)
    } else if named.is_some_and(macros::is_macro_file) || source.bytes.first() == Some(&b'{') {
        install_macro(s, p, &source.bytes, named, replace)
    } else {
        Err(p.bad("not a module (a .wasm file) or a macro (a .fcmacro file)"))
    }
}

/// Reads a file to install, refusing one over the module size limit without reading the rest.
fn read_capped(p: &Params, path: &Path) -> Result<Vec<u8>> {
    extensions::read_module_file(path).map_err(|e| p.bad(format!("{}: {e}", path.display())))
}

fn decode_base64(p: &Params, data: &str) -> Result<Vec<u8>> {
    // Four characters hold three bytes; anything longer would decode past the module limit.
    if data.len() > MAX_MODULE_BYTES / 3 * 4 + 4 {
        return Err(p.bad(format!("`dataBase64` holds more than the module limit of {} MiB", MAX_MODULE_BYTES >> 20)));
    }
    base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| p.bad(format!("`dataBase64` is not base64: {e}")))
}

/// Checks a file name for a folder and gives it the extension `ext` when it has another.
fn folder_file_name(p: &Params, name: &str, ext: &str) -> Result<String> {
    let bad = name.trim().is_empty()
        || name.len() > MAX_FILE_NAME
        || name.starts_with('.')
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    if bad {
        return Err(p.bad(format!("{name:?} cannot be a file name: give a name without folders or the characters / \\ : * ? \" < > |")));
    }
    let has_ext = Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    Ok(if has_ext { name.to_string() } else { format!("{name}.{ext}") })
}

fn install_module(s: &mut Session, p: &Params, bytes: &[u8], name: Option<&str>, source_name: Option<&str>, replace: bool) -> Result<Value> {
    let plugin = Plugin::load(bytes, Limits::default()).map_err(|e| p.bad(e.to_string()))?;
    let id = plugin.id().to_string();
    let hash = sha256::hex(bytes);
    let ext = s.extensions();
    let existing = ext.registry.get(&id).map(|m| (m.file.clone(), m.sha256.clone(), m.plugin.manifest().version.clone()));
    // The same module again: it is only turned on.
    if let Some((Some(file), sha, _)) = &existing
        && *sha == hash
        && name.is_none_or(|n| folder_file_name(p, n, MODULE_EXTENSION).is_ok_and(|n| n == *file))
    {
        let file = file.clone();
        s.extensions_mut().set_module_enabled(&id, true).map_err(|e| p.bad(e))?;
        let warning = save_switches(s);
        return Ok(installed_json(plugin.manifest(), Some(&file), None, false, warning));
    }
    if let Some((_, _, version)) = &existing
        && !replace
    {
        return Err(p.bad(format!("module {id:?} (version {version:?}) is installed already: pass replace: true to replace it")));
    }
    if existing.is_none() && ext.registry.len() >= MAX_INSTALLED {
        return Err(p.bad(format!("too many modules are installed (limit {MAX_INSTALLED}); remove one first")));
    }
    let Some(dir) = ext.modules_dir.clone() else {
        // No Modules folder: installed for this session.
        let replaced = existing.is_some();
        let manifest = plugin.manifest().clone();
        let ext = s.extensions_mut();
        ext.registry.install(plugin, None).map_err(|e| p.bad(e.to_string()))?;
        ext.registry.set_file(&id, None, hash).map_err(|e| p.bad(e.to_string()))?;
        return Ok(installed_json(&manifest, None, None, replaced, None));
    };
    let old_file = existing.as_ref().and_then(|(file, _, _)| file.clone());
    let file = match (name, &old_file, source_name) {
        (Some(n), _, _) => folder_file_name(p, n, MODULE_EXTENSION)?,
        (None, Some(old), _) => old.clone(),
        (None, None, Some(n)) => folder_file_name(p, n, MODULE_EXTENSION)?,
        (None, None, None) => folder_file_name(p, &id, MODULE_EXTENSION)?,
    };
    // A file of that name holding another module, or one that failed to load, is replaced only
    // with replace: true.
    let taken = match ext.registry.by_file(&file) {
        Some(m) => m.plugin.id() != id,
        None => old_file.as_deref() != Some(file.as_str()) && dir.join(&file).exists(),
    };
    if taken && !replace {
        return Err(p.bad(format!("{} exists: pass replace: true to overwrite it", dir.join(&file).display())));
    }
    let other = ext.registry.by_file(&file).filter(|m| m.plugin.id() != id).map(|m| m.plugin.id().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Io(format!("{}: {e}", dir.display())))?;
    let target = dir.join(&file);
    extensions::write_file(&target, bytes).map_err(EngineError::Io)?;
    let manifest = plugin.manifest().clone();
    let ext = s.extensions_mut();
    // The module the overwritten file held is gone with it.
    if let Some(other) = other {
        ext.forget_module(&other);
    }
    ext.registry.install(plugin, Some(file.clone())).map_err(|e| p.bad(e.to_string()))?;
    ext.registry.set_file(&id, Some(file.clone()), hash).map_err(|e| p.bad(e.to_string()))?;
    ext.module_switches.insert(file.clone(), true);
    let mut warning = None;
    if let Some(old) = old_file.filter(|old| *old != file) {
        ext.module_switches.remove(&old);
        if let Err(e) = extensions::move_out_of_folder(&dir, &old) {
            warning = Some(format!("the replaced module's file stays in the Modules folder: {e}"));
        }
    }
    let warning = save_switches(s).or(warning);
    Ok(installed_json(&manifest, Some(&file), Some(&target), existing.is_some(), warning))
}

/// Turns on the module file `name` of the Modules folder, installing it from there when the
/// folder has not been read since it arrived.
fn turn_on_folder_file(s: &mut Session, p: &Params, name: &str) -> Result<Value> {
    let dir = s.extensions().modules_dir.clone().ok_or_else(|| p.bad("this session has no Modules folder"))?;
    if macros::is_macro_file(name) {
        return Err(p.bad("macros in the Macros folder need no turning on: they are listed in the Script menu"));
    }
    let file = folder_file_name(p, name, MODULE_EXTENSION)?;
    if let Some(m) = s.extensions().registry.by_file(&file) {
        let (id, manifest) = (m.plugin.id().to_string(), m.plugin.manifest().clone());
        s.extensions_mut().set_module_enabled(&id, true).map_err(|e| p.bad(e))?;
        let warning = save_switches(s);
        return Ok(installed_json(&manifest, Some(&file), Some(&dir.join(&file)), false, warning));
    }
    let bytes = read_capped(p, &dir.join(&file))?;
    install_module(s, p, &bytes, Some(&file), None, true)
}

fn install_macro(s: &mut Session, p: &Params, bytes: &[u8], name: Option<&str>, replace: bool) -> Result<Value> {
    if bytes.len() > MAX_MACRO_BYTES {
        return Err(p.bad(format!("a macro file holds at most {} MiB", MAX_MACRO_BYTES >> 20)));
    }
    let m = Macro::parse(bytes).map_err(|e| p.bad(e.to_string()))?;
    let dir = s.extensions().macros_dir.clone().ok_or_else(|| p.bad("this session has no Macros folder"))?;
    let name = name.ok_or_else(|| p.bad("give the macro's file `name`"))?;
    let file = folder_file_name(p, name, MACRO_EXTENSION)?;
    let target = dir.join(&file);
    let replaced = target.exists();
    if replaced && !replace {
        return Err(p.bad(format!("{} exists: pass replace: true to overwrite it", target.display())));
    }
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Io(format!("{}: {e}", dir.display())))?;
    extensions::write_file(&target, bytes).map_err(EngineError::Io)?;
    let title = m.title.clone();
    s.extensions_mut().put_macro(file.clone(), m);
    Ok(json!({"kind": "macro", "id": file, "name": title, "file": file, "path": target.display().to_string(), "enabled": true, "replaced": replaced}))
}

/// Writes the module switches to the Modules folder; the problem, if any, as a warning.
fn save_switches(s: &Session) -> Option<String> {
    let ext = s.extensions();
    let dir = ext.modules_dir.as_ref()?;
    extensions::save_switches(dir, &ext.module_switches).err().map(|e| format!("the module switches were not saved: {e}"))
}

fn installed_json(m: &fontcraft_plugins::Manifest, file: Option<&str>, path: Option<&Path>, replaced: bool, warning: Option<String>) -> Value {
    let mut out = json!({
        "kind": m.kind.name(), "id": m.id, "name": m.name, "version": m.version,
        "file": file, "path": path.map(|p| p.display().to_string()), "enabled": true, "replaced": replaced,
    });
    if let (Some(w), Some(o)) = (warning, out.as_object_mut()) {
        o.insert("warning".into(), json!(w));
    }
    out
}

// ── Index entries ───────────────────────────────────────────────────────────────────────────────

pub(crate) fn install_from_index(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("pluginManager.install", v)?;
    let id = p.str("id")?;
    let version = p.opt_str("version")?;
    let (entry, source) = match p.opt_str("index")? {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|e| p.bad(format!("{path}: {e}")))?;
            let index = crate::extension_index::Index::parse(&bytes).map_err(|e| p.bad(format!("{path}: {e}")))?;
            let entry = find_entry(std::slice::from_ref(&index), id, version).cloned();
            (entry, Some(path.to_string()))
        }
        None => {
            let ext = s.extensions();
            let found =
                ext.indexes.iter().find_map(|l| find_entry(std::slice::from_ref(&l.index), id, version).map(|e| (e.clone(), l.source.clone())));
            match found {
                Some((entry, source)) => (Some(entry), Some(source)),
                None => (None, None),
            }
        }
    };
    let entry = entry.ok_or_else(|| match version {
        Some(version) => p.bad(format!("no extension index offers {id:?} version {version:?}")),
        None => p.bad(format!("no extension index offers {id:?}")),
    })?;
    let bytes = match p.opt_str("dataBase64")? {
        Some(data) => decode_base64(&p, data)?,
        None => read_entry_file(&p, &entry, source.as_deref())?,
    };
    verify_download(&entry, &bytes).map_err(|e| p.bad(e))?;
    let file_name = entry.url.rsplit(['/', '\\']).next().filter(|n| !n.is_empty() && !n.contains(['?', '#']));
    if entry.kind == EntryKind::Macro {
        // Named after the entry's id, so the listing and Remove find it by that id.
        let name = format!("{}.{MACRO_EXTENSION}", entry.id);
        return install_macro(s, &p, &bytes, Some(&name), true);
    }
    let plugin_kind = Plugin::load(&bytes, Limits::default()).map_err(|e| p.bad(e.to_string()))?.kind();
    if entry_kind_name(entry.kind) != plugin_kind.name() {
        return Err(p.bad(format!(
            "the index lists {:?} as a {} module, but the file is a {} module",
            entry.id,
            entry_kind_name(entry.kind),
            plugin_kind.name()
        )));
    }
    install_module(s, &p, &bytes, None, file_name.filter(|n| extensions::is_module_file(n)), true)
}

/// The first entry with `id` (and `version`, when given) in `indexes`.
fn find_entry<'a>(indexes: &'a [crate::extension_index::Index], id: &str, version: Option<&str>) -> Option<&'a Entry> {
    indexes.iter().flat_map(|i| &i.entries).find(|e| e.id == id && version.is_none_or(|v| e.version == v))
}

/// Reads the file of a local index entry: a path, absolute or beside the index file.
fn read_entry_file(p: &Params, entry: &Entry, index: Option<&str>) -> Result<Vec<u8>> {
    let url = entry.url.as_str();
    if url.starts_with("https://") || url.starts_with("http://") || index.is_some_and(|i| i.starts_with("https://")) {
        return Err(p.bad(format!("{:?} is on the network: the desktop app downloads it and passes it as dataBase64", entry.id)));
    }
    let path = PathBuf::from(url.strip_prefix("file://").unwrap_or(url));
    let path = match index.map(Path::new).and_then(Path::parent) {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    };
    read_capped(p, &path)
}

/// Checks downloaded bytes against the entry's size and SHA-256.
fn verify_download(entry: &Entry, bytes: &[u8]) -> std::result::Result<(), String> {
    let mismatch = |what: String| format!("the file of {:?} does not match its index entry ({what}); nothing was installed", entry.id);
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != entry.size {
        return Err(mismatch(format!("{} bytes, the entry says {}", bytes.len(), entry.size)));
    }
    let sum = sha256::hex(bytes);
    if !sum.eq_ignore_ascii_case(entry.sha256.trim()) {
        return Err(mismatch(format!("SHA-256 {sum}, the entry says {}", entry.sha256.chars().take(64).collect::<String>())));
    }
    Ok(())
}

fn entry_kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Filter => "filter",
        EntryKind::Reporter => "reporter",
        EntryKind::Check => "check",
        EntryKind::Macro => "macro",
    }
}

// ── Removing ────────────────────────────────────────────────────────────────────────────────────

fn remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("pluginManager.remove", v)?;
    let id = p.str("id")?;
    let ext = s.extensions();
    if let Some(m) = ext.registry.get(id) {
        let kind = m.plugin.kind().name();
        let moved = match (&m.file, &ext.modules_dir) {
            (Some(file), Some(dir)) if dir.join(file).exists() => Some(extensions::move_out_of_folder(dir, file).map_err(EngineError::Io)?),
            _ => None,
        };
        s.extensions_mut().forget_module(id);
        let mut out = json!({"id": id, "kind": kind, "movedTo": moved.map(|p| p.display().to_string())});
        if let (Some(w), Some(o)) = (save_switches(s), out.as_object_mut()) {
            o.insert("warning".into(), json!(w));
        }
        return Ok(out);
    }
    let Some(path) = ext.find_macro(id).map(|(path, _)| path.to_string()) else {
        return Err(p.bad(format!("no module or macro {id:?} is installed")));
    };
    let moved = match &ext.macros_dir {
        Some(dir) if dir.join(&path).exists() => Some(extensions::move_out_of_folder(dir, &path).map_err(EngineError::Io)?),
        _ => None,
    };
    s.extensions_mut().forget_macro(&path);
    Ok(json!({"id": path, "kind": "macro", "movedTo": moved.map(|p| p.display().to_string())}))
}

// ── Listing and info ────────────────────────────────────────────────────────────────────────────

fn list(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("pluginManager.list", v)?;
    let state = p.opt_str("state")?.unwrap_or("all");
    if !matches!(state, "all" | "installed" | "notInstalled") {
        return Err(p.bad(format!("`state` is \"all\", \"installed\" or \"notInstalled\", not {state:?}")));
    }
    let kind = p.opt_str("type")?;
    if let Some(k) = kind
        && !matches!(k, "filter" | "reporter" | "check" | "macro")
    {
        return Err(p.bad(format!("`type` is \"filter\", \"reporter\", \"check\" or \"macro\", not {k:?}")));
    }
    let query = p.opt_str("query")?.map(|q| q.chars().take(MAX_QUERY_CHARS).collect::<String>().to_lowercase());
    let ext = s.extensions();
    let mut entries = Vec::new();
    let mut ids = BTreeSet::new();
    // Macros of the Macros folder that an index entry stands for, so they are listed once.
    let mut covered = BTreeSet::new();
    for loaded in &ext.indexes {
        for e in &loaded.index.entries {
            if !ids.insert(e.id.clone()) {
                continue;
            }
            let mut row = json!({
                "id": e.id, "kind": entry_kind_name(e.kind), "title": e.title("en"), "vendor": e.vendor, "version": e.version,
                "description": e.descriptions.get("en").cloned().unwrap_or_default(), "size": e.size, "sha256": e.sha256,
                "source": "index", "index": crate::extension_index::redact(&loaded.source),
            });
            let installed = if e.kind == EntryKind::Macro {
                ext.find_macro(&e.id).map(|(path, _)| {
                    covered.insert(path.to_string());
                    None
                })
            } else {
                ext.registry.get(&e.id).map(Some)
            };
            if let Some(o) = row.as_object_mut() {
                o.insert("installed".into(), json!(installed.is_some()));
                if let Some(Some(m)) = installed {
                    let version = &m.plugin.manifest().version;
                    o.insert("installedVersion".into(), json!(version));
                    o.insert("update".into(), json!(*version != e.version));
                    o.insert("enabled".into(), json!(m.enabled));
                    o.insert("new".into(), json!(m.new));
                    o.insert("file".into(), json!(m.file));
                }
            }
            entries.push(row);
        }
    }
    for m in ext.registry.list().iter().filter(|m| !ids.contains(m.plugin.id())) {
        let man = m.plugin.manifest();
        entries.push(json!({
            "id": man.id, "kind": man.kind.name(), "title": man.name, "vendor": man.author, "version": man.version,
            "description": man.description, "installed": true, "enabled": m.enabled, "new": m.new, "file": m.file,
            "size": m.plugin.size(), "sha256": m.sha256, "source": "installed",
        }));
    }
    for e in ext.macros.iter().filter(|e| e.error.is_none() && !ids.contains(&e.path) && !covered.contains(&e.path)) {
        entries.push(json!({
            "id": e.path, "kind": "macro", "title": e.title, "vendor": "", "version": "", "description": e.description,
            "installed": true, "file": e.path, "source": "folder",
        }));
    }
    let text = |row: &Value, key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default().to_lowercase();
    entries.retain(|row| {
        let installed = row.get("installed").and_then(Value::as_bool).unwrap_or(false);
        let state_ok = match state {
            "installed" => installed,
            "notInstalled" => !installed,
            _ => true,
        };
        let kind_ok = kind.is_none_or(|k| row.get("kind").and_then(Value::as_str) == Some(k));
        let query_ok = query.as_deref().is_none_or(|q| ["id", "title", "vendor", "description"].iter().any(|key| text(row, key).contains(q)));
        state_ok && kind_ok && query_ok
    });
    Ok(json!({"count": entries.len(), "entries": entries, "safeMode": ext.safe_mode}))
}

fn info(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("plugin.info", v)?;
    let id = p.str("id")?;
    let ext = s.extensions();
    let m = ext.registry.get(id).ok_or_else(|| p.bad(format!("no module {id:?} is installed")))?;
    let man = m.plugin.manifest();
    let params: Vec<Value> = man.params.iter().map(|(name, spec)| param_json(name, spec)).collect();
    Ok(json!({
        "id": man.id, "name": man.name, "version": man.version, "kind": man.kind.name(), "author": man.author,
        "description": man.description, "capabilities": man.capabilities, "params": params, "defaults": man.defaults(),
        "paramsDoc": man.params_doc(), "size": m.plugin.size(), "sha256": m.sha256, "enabled": m.enabled, "new": m.new,
        "file": m.file, "runs": ext.runnable(id, man.kind).is_ok(), "lastError": ext.module_errors.get(id),
    }))
}

fn param_json(name: &str, spec: &ParamSpec) -> Value {
    let default = spec.default_value();
    match spec {
        ParamSpec::Number { min, max, .. } => json!({"name": name, "type": "number", "min": min, "max": max, "default": default}),
        ParamSpec::Int { min, max, .. } => json!({"name": name, "type": "int", "min": min, "max": max, "default": default}),
        ParamSpec::Bool { .. } => json!({"name": name, "type": "bool", "default": default}),
        ParamSpec::Choice { options, .. } => json!({"name": name, "type": "choice", "options": options, "default": default}),
    }
}

fn app_info(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("app.info", v)?;
    let now = s.now();
    let ext = s.extensions();
    let uptime = match (ext.started_at, now) {
        (Some(start), Some(now)) => Some(now.saturating_sub(start).max(0)),
        _ => None,
    };
    let limits: Map<String, Value> = LIMITS_TABLE.iter().map(|(name, value)| ((*name).to_string(), json!(value))).collect();
    let failed: Vec<Value> = ext.module_failures.iter().map(|(file, error)| json!({"file": file, "error": error})).collect();
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "started": ext.started_at,
        "uptimeSeconds": uptime,
        "safeMode": ext.safe_mode,
        "surfaces": ext.surfaces,
        "modules": ext.modules_json(),
        "failedModules": failed,
        "modulesFolder": ext.modules_dir.as_ref().map(|d| d.display().to_string()),
        "macrosFolder": ext.macros_dir.as_ref().map(|d| d.display().to_string()),
        "macros": ext.macros.len(),
        "log": {"entries": ext.log.len()},
        "limits": limits,
    }))
}

fn load_extensions(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("app.loadExtensions", v)?;
    let ext = s.extensions_mut();
    ext.leave_safe_mode();
    let enabled = ext.registry.list().iter().filter(|m| m.enabled).count();
    Ok(json!({"safeMode": false, "enabled": enabled}))
}

// ── Running a filter ────────────────────────────────────────────────────────────────────────────

/// What a filter run computed before anything changes.
struct Computed {
    /// `(glyph, layer id, new layer)` for each target layer.
    layers: Vec<(String, String, Layer)>,
    skipped: Vec<Value>,
    log: Vec<String>,
}

/// The layers a run addresses.
struct Scope {
    glyphs: Vec<String>,
    /// `None`: every master layer.
    layers: Option<Vec<String>>,
    single: bool,
    selection: Vec<NodeRef>,
    selection_only: bool,
}

fn run_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("plugin.runFilter", v)?;
    let id = p.str("id")?;
    let plugin = s.extensions().runnable(id, Kind::Filter).map_err(|e| p.bad(e))?;
    let given = p.get("params").cloned().unwrap_or(Value::Null);
    let params = plugin.manifest().validate_params(&given).map_err(|e| p.bad(e.to_string()))?;
    let params = serde_json::to_vec(&params).map_err(|e| EngineError::Internal(e.to_string()))?;
    let scope = scope(&p, s.font()?)?;
    let computed = compute(s.font()?, &plugin, &params, &scope);
    let computed = match computed {
        Ok(c) => c,
        Err(Failure::Locked(glyph)) => return Err(EngineError::Locked(glyph)),
        Err(Failure::Params(message)) => return Err(p.bad(message)),
        Err(Failure::Module(message)) => {
            s.extensions_mut().note_module_error(id, &message);
            return Err(p.bad(message));
        }
    };
    let label = plugin.manifest().name.clone();
    let mut glyphs: Vec<String> = Vec::new();
    let count = computed.layers.len();
    s.edit(&label, |font| {
        for (glyph, layer_id, new) in computed.layers {
            let mut g = font.glyph_mut(&glyph)?;
            if let Some(layer) = g.layers.iter_mut().find(|l| l.id == layer_id) {
                *layer = new;
            }
            if glyphs.last() != Some(&glyph) {
                glyphs.push(glyph);
            }
        }
        Ok(())
    })?;
    let ext = s.extensions_mut();
    for line in &computed.log {
        ext.log.push_module_line(id, line);
    }
    Ok(json!({"id": id, "glyphs": glyphs, "layers": count, "skipped": computed.skipped, "log": computed.log}))
}

/// Why a filter run stopped before changing anything.
enum Failure {
    Locked(String),
    Params(String),
    Module(String),
}

/// Reads `glyph` or `glyphs`, `layer` or `layers`, `points` and `selectionOnly`.
fn scope(p: &Params, font: &Font) -> Result<Scope> {
    let (glyphs, single) = match (p.get("glyph"), p.get("glyphs")) {
        (Some(_), Some(_)) => return Err(p.bad("give `glyph` or `glyphs`, not both")),
        (Some(_), None) => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            (vec![name.to_string()], true)
        }
        (None, _) => {
            let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyph` or `glyphs` is required"))?;
            if glyphs.is_empty() {
                return Err(p.bad("`glyphs` is empty"));
            }
            (glyphs, false)
        }
    };
    let layers = match (p.get("layer"), p.get("layers")) {
        (Some(_), Some(_)) => return Err(p.bad("give `layer` or `layers`, not both")),
        (Some(_), None) => Some(vec![p.str("layer")?.to_string()]),
        (None, Some(Value::Array(a))) if a.len() > MAX_LAYER_IDS => {
            return Err(p.bad(format!("`layers` holds {} ids; at most {MAX_LAYER_IDS} are allowed", a.len())));
        }
        (None, Some(_)) => {
            let mut seen = BTreeSet::new();
            let ids: Vec<String> = p.strings("layers")?.into_iter().filter(|id| seen.insert(*id)).map(str::to_string).collect();
            if ids.is_empty() {
                return Err(p.bad("`layers` is empty"));
            }
            Some(ids)
        }
        (None, None) => None,
    };
    let selection_only = p.bool_or("selectionOnly", false)?;
    let selection = if p.get("points").is_some() {
        if !single || layers.as_ref().is_none_or(|l| l.len() != 1) {
            return Err(p.bad("`points` name nodes in one layer: give `glyph` and one `layer`"));
        }
        let t = Targets::parse(p)?;
        t.resolve(font)?;
        t.points.iter().map(|&(shape, node)| NodeRef { shape, node }).collect()
    } else {
        Vec::new()
    };
    if selection_only && selection.is_empty() {
        return Err(p.bad("`selectionOnly` needs the selected `points`"));
    }
    Ok(Scope { glyphs, layers, single, selection, selection_only })
}

/// Runs the module on every target layer and checks each result; changes nothing.
fn compute(font: &Font, plugin: &Arc<Plugin>, params: &[u8], scope: &Scope) -> std::result::Result<Computed, Failure> {
    let mut out = Computed { layers: Vec::new(), skipped: Vec::new(), log: Vec::new() };
    let id = plugin.id();
    for name in &scope.glyphs {
        let glyph = font.glyph(name).ok_or_else(|| Failure::Params(format!("there is no glyph `{name}`")))?;
        if glyph.info.locked {
            return Err(Failure::Locked(name.clone()));
        }
        let targets: Vec<&Layer> = match &scope.layers {
            Some(ids) => {
                for missing in ids.iter().filter(|id| !glyph.layers.iter().any(|l| l.id == **id)) {
                    if scope.single {
                        return Err(Failure::Params(format!("glyph `{name}` has no layer `{missing}`")));
                    }
                    out.skipped.push(json!({"glyph": name, "layer": missing, "reason": "the glyph has no such layer"}));
                }
                glyph.layers.iter().filter(|l| ids.contains(&l.id)).collect()
            }
            None => glyph.layers.iter().filter(|l| l.is_master_layer()).collect(),
        };
        for layer in targets {
            let failed = |e: String| Failure::Module(format!("module {id:?} on glyph `{name}`, layer `{}`: {e}", layer.id));
            let mut input = LayerInput::from_layer(font, glyph, layer).map_err(|e| failed(e.to_string()))?;
            if !scope.selection.is_empty() {
                input = input.with_selection(scope.selection.clone());
            }
            let bytes = input.to_json().map_err(|e| failed(e.to_string()))?;
            let result = plugin.call(&bytes, params).and_then(|o| FilterOutput::parse(&o)).map_err(|e| failed(e.to_string()))?;
            let new =
                if scope.selection_only { keep_selected(&result, layer, &scope.selection) } else { result.apply(layer).map_err(|e| e.to_string()) };
            let new = new.map_err(failed)?;
            out.log.extend(result.log);
            out.layers.push((name.clone(), layer.id.clone(), new));
        }
    }
    Ok(out)
}

/// `layer` with only the paths that hold a selected node taken from the module's result; the
/// result must list the layer's shapes in their order.
fn keep_selected(result: &FilterOutput, layer: &Layer, selection: &[NodeRef]) -> std::result::Result<Layer, String> {
    if result.shapes.len() != layer.shapes.len() {
        return Err(format!(
            "with selectionOnly the module must return the layer's {} shapes in their order; it returned {}",
            layer.shapes.len(),
            result.shapes.len()
        ));
    }
    let selected: BTreeSet<usize> = selection.iter().map(|r| r.shape).collect();
    let shapes = layer
        .shapes
        .iter()
        .zip(&result.shapes)
        .enumerate()
        .map(|(i, (old, new))| if selected.contains(&i) { new.clone() } else { old.clone() })
        .collect();
    FilterOutput { shapes, anchors: None, width: None, log: Vec::new() }.apply(layer).map_err(|e| e.to_string())
}
