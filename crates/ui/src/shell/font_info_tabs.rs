//! The view state of the Features and Exports tabs of Font Info, and the shell commands that change it.
//!
//! | Command | Does |
//! |---|---|
//! | `font.info.features.select {entries: [{kind, name, index}]}` | selects sidebar entries of the Features tab; the last one shows in the editor |
//! | `font.info.exports.select {instances: [index]}` | selects entries of the Exports list; the rows at the right edit all of them |
//! | `font.info.features.paste` | asks the window for the clipboard's text and pastes it with `font.features.pasteEntries` |
//!
//! The selection is view state: the projection resolves it against the font (an entry that moved or was renamed
//! keeps its place in the selection), so a stale selection never names an entry the font lacks.

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::{Features, Snippet};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;
use crate::app::clipboard::PasteKind;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "font.info.features.select",
        label: "Select Feature Entries",
        menu: &[],
        shortcut: None,
        params: "{entries: [{kind: 'prefix'|'class'|'feature', name, index}]} select entries of the Features tab's sidebar; the last one shows in the code editor. An empty list selects the first entry",
    },
    UiCommand {
        id: "font.info.exports.select",
        label: "Select Exports",
        menu: &[],
        shortcut: None,
        params: "{instances: [index]} select entries of the Exports tab's list; the fields at the right edit every selected entry. An empty list selects the first entry",
    },
    UiCommand {
        id: PASTE_ENTRIES,
        label: "Paste Feature Entries",
        menu: &[],
        shortcut: None,
        params: "{} paste the clipboard's text into the Features tab's sidebar: the entries it holds go after the font's, as one undo step",
    },
];

/// The shell command the sidebar's Paste posts: the clipboard's text, once the window hands it over, goes to
/// `font.features.pasteEntries`.
pub const PASTE_ENTRIES: &str = "font.info.features.paste";

/// Most entries one selection names.
const MAX_SELECTED: usize = 4096;

/// A group of the Features sidebar.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EntryKind {
    #[default]
    Prefix,
    Class,
    Feature,
}

impl EntryKind {
    /// The groups in sidebar order.
    pub const ALL: [EntryKind; 3] = [EntryKind::Prefix, EntryKind::Class, EntryKind::Feature];

    /// The `kind` parameter of the `font.features.*` commands.
    pub fn name(self) -> &'static str {
        match self {
            EntryKind::Prefix => "prefix",
            EntryKind::Class => "class",
            EntryKind::Feature => "feature",
        }
    }

    pub fn from_name(name: &str) -> Option<EntryKind> {
        EntryKind::ALL.into_iter().find(|k| k.name() == name)
    }

    pub fn group(self, f: &Features) -> &[Snippet] {
        match self {
            EntryKind::Prefix => &f.prefixes,
            EntryKind::Class => &f.classes,
            EntryKind::Feature => &f.features,
        }
    }
}

/// One sidebar entry: its group, its name and its place in the group.
#[derive(PlainData, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntryKey {
    pub kind: EntryKind,
    pub name: String,
    pub index: usize,
}

impl EntryKey {
    /// `{kind, name, index}`, as every entry command takes it.
    pub fn params(&self) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("kind".into(), json!(self.kind.name()));
        p.insert("name".into(), json!(self.name));
        p.insert("index".into(), json!(self.index));
        p
    }

    /// [`Self::params`] with `extra` added.
    pub fn with(&self, extra: &[(&str, Value)]) -> Value {
        let mut p = self.params();
        for (k, v) in extra {
            p.insert((*k).to_string(), v.clone());
        }
        Value::Object(p)
    }

    /// The entry, when the font has it at this place under this name.
    pub fn snippet<'a>(&self, f: &'a Features) -> Option<&'a Snippet> {
        self.kind.group(f).get(self.index).filter(|e| e.name == self.name)
    }

    /// The key of this entry in the font as it is now: at its place, else the first entry of its name (it moved),
    /// else the entry now at its place (it was renamed). `None` when the group has neither.
    pub fn resolve(&self, f: &Features) -> Option<EntryKey> {
        let group = self.kind.group(f);
        if self.snippet(f).is_some() {
            return Some(self.clone());
        }
        if let Some(index) = group.iter().position(|e| e.name == self.name) {
            return Some(EntryKey { index, ..self.clone() });
        }
        group.get(self.index).map(|e| EntryKey { name: e.name.clone(), ..self.clone() })
    }

    pub fn from_json(v: &Value) -> Option<EntryKey> {
        Some(EntryKey {
            kind: EntryKind::from_name(v.get("kind")?.as_str()?)?,
            name: v.get("name")?.as_str()?.to_string(),
            index: usize::try_from(v.get("index")?.as_u64()?).ok()?,
        })
    }
}

/// The Features tab's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeaturesView {
    /// Selected entries; the last is the one the editor shows.
    pub selected: Vec<EntryKey>,
}

/// The Exports tab's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportsView {
    /// Selected entries, by index into the font's instances.
    pub selected: Vec<usize>,
}

/// The `font.info.features.select` message that selects `entries`.
pub fn select_entries(entries: &[EntryKey]) -> CommandRef {
    let list: Vec<Value> = entries.iter().map(|k| Value::Object(k.params())).collect();
    CommandRef::with_params("font.info.features.select", json!({ "entries": list }))
}

/// The `font.info.exports.select` message that selects `instances`.
pub fn select_exports(instances: &[usize]) -> CommandRef {
    CommandRef::with_params("font.info.exports.select", json!({ "instances": instances }))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    app.session.font().map_err(|e| e.to_string())?;
    let params = &command.params;
    match command.id.as_str() {
        "font.info.features.select" => {
            let list = params.get("entries").and_then(Value::as_array).ok_or("`entries` is required")?;
            let mut selected = Vec::new();
            for entry in list.iter().take(MAX_SELECTED) {
                let key = EntryKey::from_json(entry).ok_or("each entry needs `kind` (prefix, class or feature), `name` and `index`")?;
                if !selected.contains(&key) {
                    selected.push(key);
                }
            }
            app.view.font_info.features.selected = selected;
            Ok(())
        }
        "font.info.exports.select" => {
            let list = params.get("instances").and_then(Value::as_array).ok_or("`instances` is required")?;
            let mut selected: Vec<usize> = list
                .iter()
                .take(MAX_SELECTED)
                .map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`instances` holds indices"))
                .collect::<Result<_, _>>()?;
            selected.sort_unstable();
            selected.dedup();
            app.view.font_info.exports.selected = selected;
            Ok(())
        }
        PASTE_ENTRIES => {
            app.clipboard.request = Some(PasteKind::FeatureEntries);
            Ok(())
        }
        other => Err(format!("no Font Info command `{other}`")),
    }
}
