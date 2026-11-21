//! The Start Window's commands and state (`window.startWindow`): recent documents, and the script and glyph-set
//! chooser that creates a document (`file.newFromGlyphSets`). Also the recent-documents list (`file.openRecent`, kept
//! in the user settings under `file.recentDocuments`) and Show in Finder (`file.showInFinder`). The window itself is
//! `dialogs/start_window.rs`.
//!
//! The chooser shows when there are no recent documents. Opening the window restores the last picks, which
//! `file.newFromGlyphSets` keeps in the user settings (`start.chooserPicks`). File › New from Glyph Sets… from the
//! menu (no parameters) opens the chooser instead of repeating the last picks unseen.

use std::path::Path;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::params;
use crate::app::{FontCraftApp, Msg};

/// Longest path accepted by the commands, in bytes.
pub const MAX_PATH_BYTES: usize = fontcraft_engine::settings::MAX_RECENT_PATH_BYTES;
/// The setting behind Show window on startup.
pub const SHOW_AT_LAUNCH: &str = "start.showAtLaunch";
/// The command Create Document runs.
pub const NEW_FROM_GLYPH_SETS: &str = "file.newFromGlyphSets";
/// The script included when no picks were made before.
const FIRST_SCRIPT: &str = "latin";
/// Longest search text, in characters.
pub const MAX_SEARCH_CHARS: usize = 64;
/// Most scripts and glyph sets read from `glyphData.glyphSets`.
const MAX_SCRIPTS: usize = 256;
const MAX_SETS: usize = 256;

pub const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.startWindow",
        label: "Start Window",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true), chooser?: bool, script?: id, include?: bool, glyphSet?: id, on?: bool, search?: string, selected?: path} show or hide the Start Window: the recent documents, or with chooser the script and glyph-set chooser that creates a document (it also shows when there are no recent documents). Opening the window restores the last picks of file.newFromGlyphSets. script shows that script's glyph sets; include takes it into the new document or leaves it out; glyphSet with on checks or clears one of its glyph sets; search filters the script list; selected marks a recent document. ui.inspect reports the window as startWindow: {open, chooser, recent, picks, script, error}",
    },
    UiCommand {
        id: "file.openRecent",
        label: "Open Recent",
        menu: &["File"],
        shortcut: None,
        params: "{index?: number, path?: string, clear?: bool} open a recent document by its place in the list (0: the newest) or by its path in the list; clear: empty the list. With none of them, the Start Window shows the list. The list holds the last 10 documents opened or saved, newest first; files that no longer exist are left out",
    },
    UiCommand {
        id: "file.showInFinder",
        label: "Show in Finder",
        menu: &[],
        shortcut: None,
        params: "{path: string} show the file in the system's file manager",
    },
];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// One script of the chooser: included in the new document or not, and its checked glyph sets (ids, in the built-in
/// order).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptPick {
    pub script: String,
    pub included: bool,
    pub glyph_sets: Vec<String>,
}

/// A script of `glyphData.glyphSets`, with its glyph sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptSets {
    pub script: String,
    /// The script's readable name.
    pub name: String,
    pub sets: Vec<SetInfo>,
}

/// A glyph set of `glyphData.glyphSets`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetInfo {
    pub id: String,
    pub name: String,
    pub default: bool,
    pub glyphs: usize,
}

/// The window's state.
#[derive(Clone, Debug, Default)]
pub struct StartState {
    pub open: bool,
    /// The chooser shows in place of the recent documents.
    pub chooser: bool,
    /// The script whose glyph sets show on the right.
    pub script: Option<String>,
    /// The chooser's search field.
    pub search: String,
    /// Every script with glyph sets, in the built-in order; filled when the window opens.
    pub picks: Vec<ScriptPick>,
    /// What `glyphData.glyphSets` lists; read when the window opens.
    pub catalog: Vec<ScriptSets>,
    /// The recent document marked last.
    pub selected_recent: Option<String>,
    /// Why the last Create Document failed.
    pub error: Option<String>,
}

/// The recent documents whose files still exist, newest first (the engine keeps the list).
pub fn recent_documents(session: &Session) -> Vec<String> {
    session.settings().recent_documents()
}

/// A pick as `file.newFromGlyphSets` stores it: the script and, when named, its glyph sets.
fn stored_picks(session: &Session) -> Option<Vec<(String, Option<Vec<String>>)>> {
    let list: Vec<Value> = session.settings().reserved::<Vec<Value>>(Reserved::ChooserPicks).ok().flatten()?;
    Some(
        list.iter()
            .take(MAX_SCRIPTS)
            .filter_map(|p| {
                let script = p.get("script")?.as_str()?.to_string();
                let sets = p
                    .get("glyphSets")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().take(MAX_SETS).filter_map(|v| v.as_str().map(str::to_string)).collect());
                Some((script, sets))
            })
            .collect(),
    )
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The glyph sets `glyphData.glyphSets` lists, by script. Empty when the query fails.
fn catalog(session: &mut Session) -> Vec<ScriptSets> {
    let listed = session.query("glyphData.glyphSets", &json!({})).ok();
    let scripts = listed.as_ref().and_then(|v| v.get("scripts")).and_then(Value::as_array).cloned().unwrap_or_default();
    scripts
        .iter()
        .take(MAX_SCRIPTS)
        .map(|s| ScriptSets {
            script: text(s, "script"),
            name: text(s, "name"),
            sets: s
                .get("sets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_SETS)
                .map(|g| SetInfo {
                    id: text(g, "id"),
                    name: text(g, "name"),
                    default: g.get("default").and_then(Value::as_bool).unwrap_or(false),
                    glyphs: g.get("glyphs").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0),
                })
                .collect(),
        })
        .collect()
}

/// A pick as `ui.inspect` reports it.
fn pick_json(p: &ScriptPick) -> Value {
    json!({"script": p.script, "included": p.included, "glyphSets": p.glyph_sets})
}

fn sets_of<'c>(catalog: &'c [ScriptSets], script: &str) -> &'c [SetInfo] {
    catalog.iter().find(|s| s.script == script).map_or(&[], |s| s.sets.as_slice())
}

/// The sets a script takes when none are named: its default sets, else its first.
fn default_sets(catalog: &[ScriptSets], script: &str) -> Vec<String> {
    let of_script = sets_of(catalog, script);
    let defaults: Vec<String> = of_script.iter().filter(|g| g.default).map(|g| g.id.clone()).collect();
    if defaults.is_empty() { of_script.first().map(|g| vec![g.id.clone()]).unwrap_or_default() } else { defaults }
}

/// `ids` that are glyph sets of `script`, in the built-in order.
fn in_order(catalog: &[ScriptSets], script: &str, ids: &[String]) -> Vec<String> {
    sets_of(catalog, script).iter().filter(|g| ids.contains(&g.id)).map(|g| g.id.clone()).collect()
}

/// The chooser's picks as the last `file.newFromGlyphSets` left them: every script with glyph sets, the ones it named
/// included with their sets. A script left out shows its default sets, which it takes when it is switched on. Without
/// last picks, Latin is included.
pub fn last_picks(session: &Session, catalog: &[ScriptSets]) -> Vec<ScriptPick> {
    let stored = stored_picks(session);
    catalog
        .iter()
        .map(|entry| {
            let script = entry.script.as_str();
            let defaults = default_sets(catalog, script);
            let (included, glyph_sets) = match &stored {
                None => (script == FIRST_SCRIPT, defaults),
                Some(list) => match list.iter().find(|(s, _)| s == script) {
                    Some((_, sets)) => (true, sets.as_deref().map_or(defaults, |ids| in_order(catalog, script, ids))),
                    None => (false, defaults),
                },
            };
            ScriptPick { script: script.to_string(), included, glyph_sets }
        })
        .collect()
}

/// The `scripts` parameter of `file.newFromGlyphSets` for these picks.
pub fn picks_param(picks: &[ScriptPick]) -> Value {
    Value::Array(picks.iter().filter(|p| p.included).map(|p| json!({"script": p.script, "glyphSets": p.glyph_sets})).collect())
}

/// A script's readable name, as the chooser lists it.
pub fn script_label(st: &StartState, script: &str) -> String {
    st.catalog.iter().find(|s| s.script == script).map_or_else(|| script.to_string(), |s| s.name.clone())
}

/// The glyph sets of `script`.
pub fn sets(st: &StartState, script: &str) -> Vec<SetInfo> {
    sets_of(&st.catalog, script).to_vec()
}

/// Opens or closes the window. Opening it restores the last picks.
fn open_window(app: &mut FontCraftApp, open: bool, chooser: Option<bool>) {
    if open && !app.view.start.open {
        let catalog = catalog(&mut app.session);
        let st = &mut app.view.start;
        st.picks = last_picks(&app.session, &catalog);
        st.catalog = catalog;
        st.script = st.picks.iter().find(|p| p.included).or(st.picks.first()).map(|p| p.script.clone());
        st.search.clear();
        st.error = None;
        st.chooser = false;
    }
    let st = &mut app.view.start;
    st.open = open;
    if let Some(c) = chooser {
        st.chooser = c;
    }
}

/// `ui.inspect`'s `startWindow`.
pub fn inspect(app: &FontCraftApp, out: &mut Map<String, Value>) {
    let st = &app.view.start;
    let recent = recent_documents(&app.session);
    out.insert(
        "startWindow".into(),
        json!({
            "open": st.open,
            "chooser": st.chooser || recent.is_empty(),
            "recent": recent,
            "picks": st.picks.iter().map(pick_json).collect::<Vec<_>>(),
            "script": st.script,
            "error": st.error,
        }),
    );
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "window.startWindow" => start_window(app, p),
        "file.openRecent" => open_recent(app, p),
        "file.showInFinder" => show_in_finder(app, p),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn start_window(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let open = params::flag(p, "open")?.unwrap_or(true);
    let chooser = params::flag(p, "chooser")?;
    let script = params::string(p, "script")?;
    let include = params::flag(p, "include")?;
    let glyph_set = params::string(p, "glyphSet")?;
    let on = params::flag(p, "on")?;
    let search = params::string(p, "search")?;
    let selected = params::string(p, "selected")?;
    open_window(app, open, chooser);
    let st = &mut app.view.start;
    if let Some(script) = script {
        if !st.picks.iter().any(|s| s.script == script) {
            return Err(format!("`{}` is not a script of the chooser", params::quoted(script)));
        }
        st.script = Some(script.to_string());
    }
    let focused = st.script.clone().unwrap_or_default();
    if let Some(on) = include
        && let Some(pick) = st.picks.iter_mut().find(|p| p.script == focused)
    {
        pick.included = on;
    }
    if let Some(set) = glyph_set {
        let on = on.ok_or("give `on` with `glyphSet`")?;
        if !sets_of(&st.catalog, &focused).iter().any(|g| g.id == set) {
            return Err(format!("`{}` is not a glyph set of `{focused}`", params::quoted(set)));
        }
        if let Some(pick) = st.picks.iter_mut().find(|p| p.script == focused) {
            let mut ids: Vec<String> = pick.glyph_sets.iter().filter(|id| *id != set).cloned().collect();
            if on {
                ids.push(set.to_string());
            }
            pick.glyph_sets = in_order(&st.catalog, &focused, &ids);
        }
    }
    if let Some(search) = search {
        st.search = search.chars().take(MAX_SEARCH_CHARS).collect();
    }
    if let Some(path) = selected {
        st.selected_recent = Some(path.chars().take(MAX_PATH_BYTES).collect());
    }
    Ok(())
}

fn open_recent(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let index = p.get("index").filter(|v| !v.is_null()).map(|v| v.as_u64().ok_or("`index` must be a whole number from 0")).transpose()?;
    let path = params::string(p, "path")?;
    if params::flag(p, "clear")? == Some(true) {
        if index.is_some() || path.is_some() {
            return Err("give `clear` alone".into());
        }
        app.session.settings_mut().clear_reserved(Reserved::RecentDocuments);
        return Ok(());
    }
    let list = recent_documents(&app.session);
    let path = match (index, path) {
        (Some(_), Some(_)) => return Err("give `index` or `path`, not both".into()),
        (Some(_), None) if list.is_empty() => return Err("there are no recent documents".into()),
        (Some(i), None) => usize::try_from(i)
            .ok()
            .and_then(|i| list.get(i))
            .cloned()
            .ok_or_else(|| format!("`index` must be below {}, the number of recent documents", list.len()))?,
        (None, Some(p)) => list.iter().find(|l| *l == p).cloned().ok_or_else(|| format!("`{}` is not a recent document", params::quoted(p)))?,
        (None, None) => {
            open_window(app, true, Some(false));
            return Ok(());
        }
    };
    app.apply_msg(Msg::run("file.open", json!({ "path": path })))
}

fn show_in_finder(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let path = params::string(p, "path")?.ok_or("give `path`")?;
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(format!("`path` must be from 1 to {MAX_PATH_BYTES} bytes"));
    }
    // The opener refuses a path that does not exist and says so when this build has no file manager (tests, the
    // browser build).
    app.opener.reveal(Path::new(path))
}

/// After an engine command ran: a document opened or created closes the window.
pub fn after_run(app: &mut FontCraftApp, id: &str) {
    let st = &mut app.view.start;
    if st.open && matches!(id, "file.open" | "file.new" | NEW_FROM_GLYPH_SETS) {
        st.open = false;
        st.chooser = false;
        st.error = None;
    }
}

/// After an engine command failed: a failed Create Document says why in the window.
pub fn after_error(app: &mut FontCraftApp, id: &str, error: &str) {
    let st = &mut app.view.start;
    if id == NEW_FROM_GLYPH_SETS && st.open {
        st.error = Some(error.to_string());
    }
}

/// File › New from Glyph Sets… fired without parameters opens the chooser with the last picks, and returns true.
pub fn fired(app: &mut FontCraftApp, command: &CommandRef) -> bool {
    let empty = command.params.is_null() || command.params.as_object().is_some_and(Map::is_empty);
    if command.id != NEW_FROM_GLYPH_SETS || !empty {
        return false;
    }
    open_window(app, true, Some(true));
    app.view_changed();
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg};

    /// New from Glyph Sets from the menu opens the chooser with Latin included; Create Document makes a document and
    /// closes the window.
    #[test]
    fn the_chooser_opens_from_the_menu_and_creates_a_document() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        assert!(super::fired(&mut app, &CommandRef::with_params(super::NEW_FROM_GLYPH_SETS, json!({}))));
        let st = &app.view.start;
        assert!(st.open && st.chooser);
        assert!(st.picks.iter().any(|p| p.script == "latin" && p.included && !p.glyph_sets.is_empty()), "{:?}", st.picks);
        let scripts = super::picks_param(&st.picks);
        let documents = app.session.documents().len();
        app.run_now(Msg::run(super::NEW_FROM_GLYPH_SETS, json!({ "scripts": scripts })), false).unwrap();
        assert_eq!(app.session.documents().len(), documents + 1);
        assert!(!app.view.start.open, "a new document closes the window");

        let err = app.run_now(Msg::Shell(CommandRef::with_params("window.startWindow", json!({"script": "no such script"}))), false).unwrap_err();
        assert!(err.contains("not a script"), "{err}");
    }
}
