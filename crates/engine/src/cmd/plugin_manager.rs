//! The Extension Manager's engine commands: reading the user's extension indexes with their
//! updates, and turning a module on or off. Rows: `pluginManager.reloadIndexes`,
//! `plugin.setEnabled`, `pluginManager.repositories`, `pluginManager.automaticUpdates`,
//! `addons.alternatePluginRepos`, `addons.repo.*`. The index format is in
//! `crate::extension_index`; installing and listing are in `super::plugin`.
//!
//! The engine reads local indexes itself and takes HTTPS ones as bytes the desktop app fetched
//! (`fetched`, by position in the list), so no URL with credentials appears in the parameters.
//! An index that cannot be read is reported and left out; the others still load. After loading,
//! every installed module that an index lists in another version is offered as an update; from
//! an index whose updates are automatic, an update whose file is local is installed at once
//! (outside safe mode), and one on the network is marked for the desktop app to download and
//! install with `pluginManager.install`. An update keeps the module's switch.

use std::io::Read as _;
use std::path::Path;

use base64::Engine as _;
use serde_json::{Value, json};

use super::*;
use crate::extension_index::{EntryKind, Index, IndexSource, Location, MAX_INDEX_BYTES, MAX_INDEXES, SOURCES_SETTING, parse_sources, redact};
use crate::extensions::{self, LoadedIndex};
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "pluginManager.reloadIndexes",
            "Reload Extension Indexes",
            [],
            None,
            "{indexes?: [line] (default: the lines of the addons.alternatePluginRepos setting; each a full path or an https:// URL, starting with \"manual \" when its updates are only offered), fetched?: [base64 | null] (the HTTPS indexes the desktop app downloaded, by position in the list), automaticUpdates?: bool (true: install the updates of indexes whose line is not manual)} read every index in order, replacing the ones the session knows; an index that cannot be read is reported and left out. Then offer each installed module that an index lists in another version (the first index listing an id wins) and install the automatic updates whose files are local, keeping each module's switch; in safe mode nothing is installed → {indexes: [{source, entries?, automaticUpdates, error?}], updates: [{id, kind, version, installedVersion, index, automatic, network, installed}], failed: [{id, error}]}",
            always,
            reload_indexes
        )
        .files(const { &[FileParam::read("indexes")] })
        .writes_files(),
        cmd!(
            "plugin.setEnabled",
            "Turn Module On or Off",
            [],
            None,
            "{id: module id, enabled: bool} turn an installed module on or off; the switch is kept for its file in the Modules folder. A module copied in by hand stays off until turned on; in safe mode a module turned on runs only after Help › Load Extensions → {id, enabled, runs: bool, warning?}",
            always,
            set_enabled
        )
        .writes_files(),
    ]
}

/// Largest base64 text of one fetched index.
const MAX_FETCHED_CHARS: usize = MAX_INDEX_BYTES / 3 * 4 + 4;

fn reload_indexes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("pluginManager.reloadIndexes", v)?;
    let sources = match p.opt_strings("indexes")? {
        Some(lines) if lines.len() > MAX_INDEXES => return Err(p.bad(format!("at most {MAX_INDEXES} indexes"))),
        Some(lines) => parse_sources(&lines.join("\n")),
        None => {
            let text = s.settings().get(SOURCES_SETTING).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            parse_sources(&text)
        }
    };
    let fetched = p.opt_array("fetched")?.map(Vec::as_slice).unwrap_or_default();
    if fetched.len() > MAX_INDEXES {
        return Err(p.bad(format!("`fetched` holds at most {MAX_INDEXES} indexes")));
    }
    let automatic = p.bool_or("automaticUpdates", true)?;

    let mut report = Vec::new();
    let mut loaded: Vec<(IndexSource, LoadedIndex)> = Vec::new();
    for (i, source) in sources.into_iter().enumerate() {
        let shown = redact(&source.location);
        match read_index(&source.location, fetched.get(i)) {
            Ok(index) => {
                report.push(json!({"source": shown, "entries": index.entries.len(), "automaticUpdates": source.automatic_updates}));
                // A file:// line is kept as its path, which entry files are found beside.
                let location = match Location::of_source(&source.location) {
                    Ok(Location::Local(path)) => path.display().to_string(),
                    _ => source.location.clone(),
                };
                loaded.push((source, LoadedIndex { source: location, index }));
            }
            Err(error) => report.push(json!({"source": shown, "automaticUpdates": source.automatic_updates, "error": error})),
        }
    }
    s.extensions_mut().indexes = loaded.iter().map(|(_, l)| l.clone()).collect();

    let mut updates = offered_updates(s, &loaded);
    let mut failed = Vec::new();
    if automatic && !s.extensions().safe_mode {
        for u in updates.iter_mut().filter(|u| u.automatic && !u.network) {
            match install_update(s, &u.id, &u.version) {
                Ok(()) => u.installed = true,
                Err(e) => failed.push(json!({"id": u.id, "error": e})),
            }
        }
    }
    let updates: Vec<Value> = updates
        .iter()
        .map(|u| {
            json!({
                "id": u.id, "kind": u.kind.name(), "version": u.version, "installedVersion": u.installed_version,
                "index": redact(&u.index), "automatic": u.automatic && automatic, "network": u.network, "installed": u.installed,
            })
        })
        .collect();
    Ok(json!({"indexes": report, "updates": updates, "failed": failed}))
}

/// Reads one index: a local file, or the base64 the desktop app fetched for an HTTPS one.
fn read_index(location: &str, fetched: Option<&Value>) -> std::result::Result<Index, String> {
    let bytes = match Location::of_source(location)? {
        Location::Local(path) => read_index_file(&path)?,
        Location::Https(_) => match fetched {
            Some(Value::String(data)) if data.len() > MAX_FETCHED_CHARS => {
                return Err(format!("the index is larger than {} MiB", MAX_INDEX_BYTES >> 20));
            }
            Some(Value::String(data)) => {
                base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| format!("the fetched index is not base64: {e}"))?
            }
            _ => return Err("not fetched yet: the desktop app downloads HTTPS indexes".into()),
        },
    };
    Index::parse(&bytes).map_err(|e| e.to_string())
}

/// Reads an index file, refusing one over [`MAX_INDEX_BYTES`] without reading the rest.
fn read_index_file(path: &Path) -> std::result::Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_INDEX_BYTES).unwrap_or(u64::MAX).saturating_add(1);
    file.take(limit).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(format!("{}: the index is larger than {} MiB", path.display(), MAX_INDEX_BYTES >> 20));
    }
    Ok(bytes)
}

/// An installed module that an index lists in another version.
struct Update {
    id: String,
    kind: EntryKind,
    version: String,
    installed_version: String,
    /// The index's location (with credentials; redacted in results).
    index: String,
    automatic: bool,
    /// The entry's file is fetched over HTTPS by the desktop app.
    network: bool,
    installed: bool,
}

fn offered_updates(s: &Session, loaded: &[(IndexSource, LoadedIndex)]) -> Vec<Update> {
    let ext = s.extensions();
    let mut seen = std::collections::BTreeSet::new();
    let mut updates = Vec::new();
    for (source, l) in loaded {
        let Ok(location) = Location::of_source(&l.source) else { continue };
        for e in &l.index.entries {
            // The first index listing an id wins, as in the listing.
            if !seen.insert(e.id.as_str()) || e.kind == EntryKind::Macro {
                continue;
            }
            let Some(m) = ext.registry.get(&e.id) else { continue };
            let installed_version = &m.plugin.manifest().version;
            if *installed_version == e.version {
                continue;
            }
            updates.push(Update {
                id: e.id.clone(),
                kind: e.kind,
                version: e.version.clone(),
                installed_version: installed_version.clone(),
                index: l.source.clone(),
                automatic: source.automatic_updates,
                network: matches!(location.resolve(&e.url), Location::Https(_)),
                installed: false,
            });
        }
    }
    updates
}

/// Installs version `version` of the module `id` from the session's indexes as
/// `pluginManager.install` does, then puts back the switch it had.
fn install_update(s: &mut Session, id: &str, version: &str) -> std::result::Result<(), String> {
    let was_on = s.extensions().registry.get(id).map(|m| m.enabled);
    super::plugin::install_from_index(s, &json!({"id": id, "version": version})).map_err(|e| e.to_string())?;
    if was_on == Some(false) {
        s.extensions_mut().set_module_enabled(id, false)?;
        save_switches(s).map_or(Ok(()), Err)?;
    }
    Ok(())
}

/// Writes the module switches to the Modules folder; the problem, if any.
fn save_switches(s: &Session) -> Option<String> {
    let ext = s.extensions();
    let dir = ext.modules_dir.as_ref()?;
    extensions::save_switches(dir, &ext.module_switches).err().map(|e| format!("the module switches were not saved: {e}"))
}

fn set_enabled(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("plugin.setEnabled", v)?;
    let id = p.str("id")?;
    let enabled = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| p.bad("`enabled` must be true or false"))?;
    s.extensions_mut().set_module_enabled(id, enabled).map_err(|e| p.bad(e))?;
    let mut out = json!({"id": id, "enabled": enabled, "runs": enabled && s.extensions().modules_run()});
    if let (Some(w), Some(o)) = (save_switches(s), out.as_object_mut()) {
        o.insert("warning".into(), json!(w));
    }
    Ok(out)
}
