//! The Export dialog's kept settings: `file.exportSettings`. Rows: `file.export`, `file.exportAll`,
//! `dialog.exportOtf`, `dialog.exportVariable`, `export.destinationFolder`.
//!
//! After a successful `file.exportStatic` or `file.exportVariable` the settings it ran with are kept
//! for the document, one set per tab of the Export dialog (`otf`, `variable`), under the dialog's
//! field keys. A saved document keeps them in the user settings under
//! [`Reserved::ExportSettings`], keyed by its path, newest first and at most
//! [`MAX_KEPT_DOCUMENTS`]; an unsaved one keeps them in a session slot keyed by its document id
//! until it is saved and exported again. A failing export keeps what was there.
//!
//! The settings file is untrusted, so every kept set is checked when it is read; one that fails is
//! reported and not used.

use serde_json::{Map, Value, json};

use super::compile::{Container, Dest};
use super::*;
use crate::DocState;
use crate::params::Params;
use crate::settings::Reserved;

/// Most documents whose settings the user settings keep.
pub const MAX_KEPT_DOCUMENTS: usize = 64;
/// Most unsaved documents whose settings the session keeps.
const MAX_SESSION_DOCUMENTS: usize = 256;
/// Longest kept path, in bytes.
pub const MAX_PATH_BYTES: usize = 4096;

/// The tabs of the Export dialog whose settings are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// Static fonts (`file.exportStatic`).
    Otf,
    /// Variable fonts (`file.exportVariable`).
    Variable,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Otf, Tab::Variable];

    pub fn id(self) -> &'static str {
        match self {
            Tab::Otf => "otf",
            Tab::Variable => "variable",
        }
    }

    fn parse(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The outline formats the tab's command takes.
    fn outlines(self) -> &'static [&'static str] {
        match self {
            Tab::Otf => &["truetype", "cff"],
            Tab::Variable => &["truetype", "cff2"],
        }
    }

    /// The keys a kept set of this tab holds.
    fn keys(self) -> &'static [&'static str] {
        match self {
            Tab::Otf => &["outlines", "containers", "removeOverlap", "autohint", "cbdt", "destination", "useDestination"],
            Tab::Variable => &["outlines", "containers", "destination", "useDestination"],
        }
    }
}

/// One document's kept settings, by tab.
#[derive(Clone, Debug, Default, PartialEq)]
struct Kept {
    otf: Option<Map<String, Value>>,
    variable: Option<Map<String, Value>>,
}

impl Kept {
    fn tab(&self, tab: Tab) -> Option<&Map<String, Value>> {
        match tab {
            Tab::Otf => self.otf.as_ref(),
            Tab::Variable => self.variable.as_ref(),
        }
    }

    fn tab_mut(&mut self, tab: Tab) -> &mut Option<Map<String, Value>> {
        match tab {
            Tab::Otf => &mut self.otf,
            Tab::Variable => &mut self.variable,
        }
    }

    fn to_json(&self) -> Value {
        json!({ "otf": self.otf, "variable": self.variable })
    }
}

/// The kept settings of unsaved documents, by document id, in the order they were kept.
#[derive(Default)]
struct SessionKept(Vec<(u64, Kept)>);

/// A kept set of `tab`, every key checked, or why it cannot be used.
fn check_set(tab: Tab, set: &Map<String, Value>) -> std::result::Result<Map<String, Value>, String> {
    if let Some(key) = set.keys().find(|k| !tab.keys().contains(&k.as_str())) {
        return Err(format!("`{}` is not a setting of the {} tab", key.chars().take(64).collect::<String>(), tab.id()));
    }
    let mut out = Map::new();
    for (key, value) in set {
        let ok = match key.as_str() {
            "outlines" => value.as_str().is_some_and(|s| tab.outlines().contains(&s)),
            "containers" => value.as_array().is_some_and(|items| {
                let mut seen: Vec<&str> = Vec::new();
                !items.is_empty()
                    && items.len() <= 3
                    && items.iter().all(|v| match v.as_str() {
                        Some(c @ ("plain" | "woff" | "woff2")) if !seen.contains(&c) => {
                            seen.push(c);
                            true
                        }
                        _ => false,
                    })
            }),
            "destination" => value.as_str().is_some_and(|s| !s.trim().is_empty() && s.len() <= MAX_PATH_BYTES && !s.contains('\0')),
            _ => value.is_boolean(),
        };
        if !ok {
            return Err(format!("`{key}` of the {} tab holds a value the tab does not take", tab.id()));
        }
        out.insert(key.clone(), value.clone());
    }
    Ok(out)
}

/// The kept settings in the user settings, newest first, and a line for each entry that cannot
/// be read.
fn stored(s: &Session) -> (Vec<(String, Kept)>, Vec<String>) {
    let value = match s.settings().reserved::<Value>(Reserved::ExportSettings) {
        Ok(v) => v.unwrap_or(Value::Null),
        Err(e) => return (Vec::new(), vec![e.to_string()]),
    };
    let items = match value {
        Value::Null => return (Vec::new(), Vec::new()),
        Value::Array(items) => items,
        _ => return (Vec::new(), vec!["the kept export settings are not a list; none is used".into()]),
    };
    let mut problems = Vec::new();
    if items.len() > MAX_KEPT_DOCUMENTS {
        problems.push(format!("export settings are kept for {} documents; only the first {MAX_KEPT_DOCUMENTS} are used", items.len()));
    }
    let mut out: Vec<(String, Kept)> = Vec::new();
    for (i, item) in items.iter().take(MAX_KEPT_DOCUMENTS).enumerate() {
        let checked = (|| {
            let path = item.get("path").and_then(Value::as_str).ok_or("it has no path")?;
            if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
                return Err("its path cannot be a file's".to_string());
            }
            if out.iter().any(|(p, _)| p == path) {
                return Err("its path is kept twice".to_string());
            }
            let mut kept = Kept::default();
            for tab in Tab::ALL {
                match item.get(tab.id()) {
                    None | Some(Value::Null) => {}
                    Some(Value::Object(set)) => *kept.tab_mut(tab) = Some(check_set(tab, set)?),
                    Some(_) => return Err(format!("its {} settings are not an object", tab.id())),
                }
            }
            Ok((path.to_string(), kept))
        })();
        match checked {
            Ok(entry) => out.push(entry),
            Err(e) => problems.push(format!("kept export settings {}: {e}; they are not used", i + 1)),
        }
    }
    (out, problems)
}

fn path_key(doc: &DocState) -> Option<String> {
    doc.path.as_deref().map(|p| p.to_string_lossy().into_owned()).filter(|p| p.len() <= MAX_PATH_BYTES)
}

/// The settings kept for `doc`: by its path, else for its session.
fn kept_for(s: &Session, doc: &DocState) -> Kept {
    if let Some(path) = path_key(doc)
        && let Some((_, kept)) = stored(s).0.into_iter().find(|(p, _)| *p == path)
    {
        return kept;
    }
    s.slot::<SessionKept>().and_then(|k| k.0.iter().find(|(id, _)| *id == doc.id)).map(|(_, kept)| kept.clone()).unwrap_or_default()
}

/// The `tab` settings kept for the active document, in the dialog's field keys; empty when none
/// are kept or there is no document.
pub fn kept_for_active(s: &Session, tab: Tab) -> Map<String, Value> {
    s.active().and_then(|doc| kept_for(s, doc).tab(tab).cloned()).unwrap_or_default()
}

/// Whether some open document has no export folder kept with its Export Destination box ticked, so that
/// `file.exportAll` without a `destination` would refuse it.
pub fn export_all_needs_folder(s: &Session) -> bool {
    s.documents().iter().any(|doc| {
        let kept = kept_otf(s, doc);
        let ticked = kept.get("useDestination") != Some(&Value::Bool(false));
        !(ticked && kept.get("destination").and_then(Value::as_str).is_some_and(|d| !d.trim().is_empty()))
    })
}

/// The OTF settings kept for `doc`: what `file.exportAll` uses for what its parameters leave out.
pub(super) fn kept_otf(s: &Session, doc: &DocState) -> Map<String, Value> {
    kept_for(s, doc).otf.unwrap_or_default()
}

/// The settings `file.exportStatic` or `file.exportVariable` ran with, in the dialog's field keys:
/// the outline format and containers it used, the given switches, the destination folder (a
/// one-file or in-memory export keeps the folder kept before) and `useDestination` (default true).
pub(super) fn ran_with(
    p: &Params,
    tab: Tab,
    outlines: &str,
    containers: &[Container],
    switches: &[(&str, bool)],
    dest: &Dest,
) -> Result<Map<String, Value>> {
    let mut set = Map::new();
    set.insert("outlines".into(), json!(outlines));
    set.insert("containers".into(), json!(containers.iter().map(|c| c.name()).collect::<Vec<_>>()));
    for (key, value) in switches {
        set.insert((*key).into(), json!(value));
    }
    if let Dest::Folder(path) = dest {
        let text = super::compile::absolute(path).to_string_lossy().into_owned();
        if text.len() <= MAX_PATH_BYTES {
            set.insert("destination".into(), json!(text));
        }
    }
    set.insert("useDestination".into(), json!(p.bool_or("useDestination", true)?));
    set.retain(|k, _| tab.keys().contains(&k.as_str()));
    Ok(set)
}

/// Keeps `set` as the active document's `tab` settings, after its export succeeded. A destination
/// it leaves out stays as kept. Returns a warning when the user settings refuse the value.
pub(super) fn remember(s: &mut Session, tab: Tab, mut set: Map<String, Value>) -> Option<String> {
    let doc = s.doc().ok()?;
    let (id, path) = (doc.id, path_key(doc));
    let before = kept_for(s, doc);
    if !set.contains_key("destination")
        && let Some(folder) = before.tab(tab).and_then(|b| b.get("destination"))
    {
        set.insert("destination".into(), folder.clone());
    }
    match path {
        Some(path) => {
            let (mut list, _) = stored(s);
            let mut kept = list.iter().position(|(p, _)| *p == path).map(|i| list.remove(i).1).unwrap_or(before);
            *kept.tab_mut(tab) = Some(set);
            list.insert(0, (path, kept));
            list.truncate(MAX_KEPT_DOCUMENTS);
            let items: Vec<Value> = list
                .iter()
                .map(|(path, kept)| {
                    let mut v = kept.to_json();
                    if let Value::Object(m) = &mut v {
                        m.insert("path".into(), json!(path));
                        m.retain(|_, v| !v.is_null());
                    }
                    v
                })
                .collect();
            match s.settings_mut().set_reserved(Reserved::ExportSettings, &items) {
                Ok(()) => {
                    s.slot_mut::<SessionKept>().0.retain(|(d, _)| *d != id);
                    None
                }
                Err(e) => Some(format!("the export settings were not kept: {e}")),
            }
        }
        None => {
            let slot = s.slot_mut::<SessionKept>();
            let mut kept = slot.0.iter().position(|(d, _)| *d == id).map(|i| slot.0.remove(i).1).unwrap_or_default();
            *kept.tab_mut(tab) = Some(set);
            slot.0.push((id, kept));
            if slot.0.len() > MAX_SESSION_DOCUMENTS {
                slot.0.remove(0);
            }
            None
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "file.exportSettings",
        "Export Settings",
        [],
        None,
        "{document?: title or path (default: the active document), tab?: \"otf\"|\"variable\"} the Export dialog's settings kept from the document's last successful export on each tab (file.exportStatic for otf, file.exportVariable for variable), in the dialog's field keys: otf {outlines, containers, removeOverlap, autohint, cbdt, destination?, useDestination}, variable {outlines, containers, destination?, useDestination}; null for a tab never exported from. A saved document's settings are kept in the user settings by its path (at most 64 documents, newest first), an unsaved one's for the session. problems lists kept settings that cannot be read, which are not used → {document, otf, variable, problems} | {document, tab, settings, problems}",
        has_doc,
        export_settings
    )]
}

fn export_settings(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.exportSettings", v)?;
    let doc = match p.opt_str("document")? {
        None => s.doc()?,
        Some(name) => s
            .documents()
            .iter()
            .find(|d| d.title == name || d.path.as_deref().is_some_and(|path| path == std::path::Path::new(name)))
            .ok_or_else(|| p.bad(format!("no open document is called or saved as `{name}`")))?,
    };
    let kept = kept_for(s, doc);
    let problems = stored(s).1;
    match p.opt_str("tab")? {
        None => Ok(json!({ "document": doc.title, "otf": kept.otf, "variable": kept.variable, "problems": problems })),
        Some(id) => {
            let tab = Tab::parse(id).ok_or_else(|| p.bad(format!("`tab` is \"otf\" or \"variable\", got \"{id}\"")))?;
            Ok(json!({ "document": doc.title, "tab": tab.id(), "settings": kept.tab(tab), "problems": problems }))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn kept_sets_from_the_settings_file_are_checked() {
        let good = json!({"outlines": "cff", "containers": ["plain", "woff2"], "removeOverlap": false, "autohint": true, "cbdt": false, "destination": "/tmp/out", "useDestination": true});
        assert!(check_set(Tab::Otf, good.as_object().unwrap()).is_ok());
        for bad in [
            json!({"outlines": "cff2"}),
            json!({"containers": []}),
            json!({"containers": ["plain", "plain"]}),
            json!({"containers": ["svg"]}),
            json!({"destination": ""}),
            json!({"destination": "a\u{0}b"}),
            json!({"removeOverlap": "yes"}),
            json!({"script": "rm -rf"}),
        ] {
            assert!(check_set(Tab::Otf, bad.as_object().unwrap()).is_err(), "{bad}");
        }
        // The variable tab takes CFF2 and none of the static switches.
        assert!(check_set(Tab::Variable, json!({"outlines": "cff2"}).as_object().unwrap()).is_ok());
        assert!(check_set(Tab::Variable, json!({"autohint": true}).as_object().unwrap()).is_err());
    }
}
