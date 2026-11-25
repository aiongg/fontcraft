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

use std::sync::Arc;

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
    UiCommand {
        id: COMPILE,
        label: "Compile Features in Background",
        menu: &[],
        shortcut: None,
        params: "{} compile the active document's feature code for the master the window shows without holding up the window, as the Features tab's Compile button does; the result is kept as font.features.compile keeps it",
    },
];

/// The shell command the Compile button posts.
pub const COMPILE: &str = "font.info.features.compile";

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
    /// The code editor's open completion question, which the projection answers with `font.features.complete`.
    pub completion: Option<CompletionAsk>,
    /// Where the caret goes in the entry's code field, in characters: after a snippet went in
    /// (`font.features.insertSnippet`'s `caret`). The code editor places it and drops it.
    pub caret: Option<(EntryKey, usize)>,
    /// The place of an error in an entry's stored code, as characters, start and end: after a click on the entry's
    /// error icon (`font.features.jumpToError`). The code editor selects it, scrolls to it and drops it.
    pub jump: Option<(EntryKey, usize, usize)>,
    /// The text the sidebar's filter field holds: the sidebar lists the entries whose name or code holds it.
    pub filter: String,
    /// The height of the notes field in points, once its divider was dragged.
    pub notes_height: Option<f32>,
    /// The code as typed in the code editor, while it differs from the entry's stored code; checked after a pause
    /// (`crate::shell::feature_work`).
    pub draft: Option<FeatureDraft>,
    /// The compile the Compile button asked for is under way.
    pub compiling: bool,
    /// The check of the last draft: its diagnostics, placed in the draft.
    pub checked: Option<CheckedDraft>,
    /// The name Alt-clicked in the code editor, which the projection previews with `font.features.previewName`.
    pub preview: Option<PreviewAsk>,
    /// The expanded-code view, while it is open in place of the entry.
    pub expanded: Option<ExpandedView>,
    /// The last expansion for that view (`crate::shell::feature_work`).
    pub expansion: Option<Expansion>,
    /// The code editor's find bar, while it is open; the projection lists its matches with `font.features.find`.
    pub find: Option<FindState>,
}

/// What the feature code editor's find bar holds.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FindState {
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Search every entry, not only the shown one.
    pub all_entries: bool,
    /// The query field takes the keyboard in the next pass (Cmd+F).
    pub focus: bool,
    /// After Next or Previous moved to another entry: select its first match (`true`) or its last (`false`) once its
    /// matches show.
    pub reveal: Option<bool>,
}

impl FindState {
    /// The query options as `font.features.find` and `font.features.replace` take them.
    pub fn params(&self) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("query".into(), json!(self.query));
        p.insert("matchCase".into(), json!(self.match_case));
        p.insert("wholeWord".into(), json!(self.whole_word));
        p.insert("regex".into(), json!(self.regex));
        p
    }
}

/// A name or `$` token Alt-clicked in an entry's code: its text and its characters in the code as shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewAsk {
    pub key: EntryKey,
    pub name: String,
    pub start: usize,
    pub end: usize,
    /// It is a token.
    pub token: bool,
}

/// The build the expanded-code view shows the code of.
#[derive(PlainData, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExpandTarget {
    /// The master with this id.
    Master(String),
    /// The instance at this index.
    Instance(usize),
    /// The variable font of the whole designspace.
    Variable,
}

impl ExpandTarget {
    /// The `target` parameter of `font.features.expandCode`.
    pub fn param(&self) -> Value {
        match self {
            ExpandTarget::Master(id) => json!({ "master": id }),
            ExpandTarget::Instance(index) => json!({ "instance": index }),
            ExpandTarget::Variable => json!({ "variable": true }),
        }
    }
}

/// What the expanded-code view is set to.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ExpandedView {
    /// The build shown; `None` is the current master.
    pub target: Option<ExpandTarget>,
    /// With the automatic code regenerated and the generated features, as an export compiles it.
    pub generated: bool,
    /// The text of the find field.
    pub query: String,
}

/// What an expansion reads: the document's id and revision, the build, and whether with the generated features.
pub type ExpandKey = (u64, u64, ExpandTarget, bool);

/// What `font.features.expandCode` gave.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    pub key: ExpandKey,
    pub code: Arc<String>,
    /// An instance's feature parameters that cannot be applied.
    pub refused: Vec<String>,
    /// Why there is no code.
    pub problem: Option<String>,
}

/// The code as typed in an entry's code editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureDraft {
    pub key: EntryKey,
    pub code: String,
}

/// What `font.features.check` found in a draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckedDraft {
    pub draft: FeatureDraft,
    pub diagnostics: Vec<crate::app::models::font_features::DiagnosticModel>,
}

/// After a run of `id` the person asked for: an inserted snippet leaves the caret after itself; an error jumped to shows
/// its entry with the faulty code selected; a feature's specification opens in the browser; new code is compiled in the
/// background.
pub fn after_run(app: &mut crate::app::FontCraftApp, id: &str, reply: &Value) {
    crate::shell::feature_work::after_run(app, id);
    match id {
        "font.features.insertSnippet" => place_caret(app, reply),
        "font.features.jumpToError" => show_error(app, reply),
        "font.features.openSpec" => {
            if let Some(url) = reply.get("url").and_then(Value::as_str)
                && let Err(error) = app.opener.open_url(url)
            {
                app.status = error;
            }
        }
        _ => {}
    }
}

/// Shows the entry of `font.features.jumpToError`'s reply; the code editor then selects the range it names.
fn show_error(app: &mut FontCraftApp, reply: &Value) {
    let Some(key) = reply.get("entry").and_then(EntryKey::from_json) else { return };
    let Ok(font) = app.session.font() else { return };
    let Some(code) = key.snippet(&font.features).map(|e| e.code.as_str()) else { return };
    let byte = |at: usize| reply.get("range").and_then(|r| r.get(at)).and_then(Value::as_u64).and_then(|b| usize::try_from(b).ok());
    let jump = byte(0).zip(byte(1)).map(|(start, end)| {
        let chars = |byte: usize| code.char_indices().take_while(|(at, _)| *at < byte).count();
        (key.clone(), chars(start), chars(end.max(start)))
    });
    let view = &mut app.view.font_info.features;
    view.selected = vec![key];
    view.expanded = None;
    view.jump = jump;
    app.view_changed();
}

/// The caret goes after the inserted snippet.
fn place_caret(app: &mut FontCraftApp, reply: &Value) {
    let caret = reply.get("caret").and_then(Value::as_u64).and_then(|c| usize::try_from(c).ok());
    if let (Some(key), Some(caret)) = (EntryKey::from_json(reply), caret) {
        app.view.font_info.features.caret = Some((key, caret));
    }
}

/// What the code editor asks completions for: the entry, the code as typed and the caret's byte offset in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionAsk {
    pub key: EntryKey,
    pub code: String,
    pub position: usize,
}

/// The Exports tab's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportsView {
    /// Selected entries, by index into the font's instances.
    pub selected: Vec<usize>,
    /// What the last instance compile found (`crate::shell::feature_work`).
    pub problems: Option<InstanceProblems>,
    /// What the exports write, as last computed (`crate::shell::export_names`).
    pub names: Option<ExportedNames>,
}

/// The reply of `font.instance.names` for a document revision.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportedNames {
    pub doc: u64,
    pub revision: u64,
    pub reply: Arc<Value>,
}

/// What the compile of one instance's code found in its feature-code parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceProblems {
    /// The document, its revision and the instance's index the compile read.
    pub doc: u64,
    pub revision: u64,
    pub instance: usize,
    pub params: Vec<ParamProblem>,
}

/// One feature-code parameter of an instance as compiled: its name, its place among the parameters of that name, its
/// code, and the line and message of the first problem in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamProblem {
    pub key: String,
    pub occurrence: usize,
    pub text: String,
    pub problem: Option<(u64, String)>,
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
            app.view.font_info.features.expanded = None;
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
        COMPILE => crate::shell::feature_work::ask(app),
        other => Err(format!("no Font Info command `{other}`")),
    }
}
