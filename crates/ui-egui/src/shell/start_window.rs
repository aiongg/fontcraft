//! The Start Window (`window.start`, `window.startWindow`): recent documents, and the script and
//! glyph-set chooser that creates a document (`file.newFromGlyphSets`). Also the recent-documents
//! list itself (`file.openRecent`, kept in the user settings under `file.recentDocuments`) and
//! Show in Finder (`file.showInFinder`).
//!
//! | Pointer | Does |
//! |---|---|
//! | double-click on a recent document | opens it (`file.openRecent`) |
//! | right-click on a recent document | Show in Finder (`file.showInFinder`) |
//! | Show window on startup | `settings.set` of `start.showAtLaunch` |
//! | plus button | shows the chooser in place of the recent documents |
//! | click on a script | shows its glyph sets on the right |
//! | No / Yes | leaves the script out of the new document or includes it |
//! | a glyph set's checkbox | takes the set out of the script's picks or adds it |
//! | Create Document | `file.newFromGlyphSets` with the included scripts and their checked sets |
//!
//! The chooser shows when there are no recent documents. Opening the window restores the last
//! picks, which `file.newFromGlyphSets` keeps in the user settings (`start.chooserPicks`).

use std::path::Path;

use egui::{Rect, Sense, Stroke, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use fontcraft_model::glyphdata::sets::{self, Sets};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// Most recent documents kept.
pub const MAX_RECENT: usize = 10;
/// Longest path kept in the list or accepted by the commands, in bytes.
pub const MAX_PATH_BYTES: usize = 4096;
/// The setting behind Show window on startup.
pub const SHOW_AT_LAUNCH: &str = "start.showAtLaunch";
/// The command Create Document runs.
const NEW_FROM_GLYPH_SETS: &str = "file.newFromGlyphSets";
/// The script included when no picks were made before.
const FIRST_SCRIPT: &str = "latin";
/// Longest search text, in characters.
const MAX_SEARCH_CHARS: usize = 64;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// The window's width, and the script list's, in `Tokens::list_column_unit`.
const WINDOW_UNITS: f32 = 16.0;
const SCRIPT_LIST_UNITS: f32 = 5.0;
/// Rows the lists show before they scroll.
const LIST_ROWS: f32 = 14.0;

pub const COMMANDS: &[UiCommand] = &[
    (
        "window.startWindow",
        "Start Window",
        &["Window"],
        "",
        "{open?: bool (default true), chooser?: bool} show or hide the Start Window: the recent documents, or with chooser the script and glyph-set chooser that creates a document (it also shows when there are no recent documents). Opening the window restores the last picks of file.newFromGlyphSets → {open, chooser, recent: [paths], picks: [{script, included, glyphSets}]}",
    ),
    (
        "file.openRecent",
        "Open Recent",
        &["File"],
        "",
        "{index?: number, path?: string, clear?: bool} open a recent document by its place in the list (0: the newest) or by its path in the list; clear: empty the list. With none of them, the Start Window shows the list. The list holds the last 10 documents opened or saved, newest first; files that no longer exist are left out → {title, path} | {cleared} | {open, chooser, recent, picks}",
    ),
    ("file.showInFinder", "Show in Finder", &[], "", "{path: string} show the file in the system's file manager → {path}"),
];

/// Shows a file in the system's file manager. The desktop app supplies one; without it (the
/// browser, MCP, tests) Show in Finder says it cannot.
pub type RevealFn = fn(&Path) -> Result<(), String>;

/// The platform's [`RevealFn`], if any. Not part of the window's state: two states compare equal
/// whatever it is.
#[derive(Clone, Copy, Default)]
pub struct Reveal(pub Option<RevealFn>);

impl PartialEq for Reveal {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Reveal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Reveal(platform)" } else { "Reveal(none)" })
    }
}

/// One script of the chooser: included in the new document or not, and its checked glyph sets
/// (ids, in the built-in order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPick {
    pub script: String,
    pub included: bool,
    pub glyph_sets: Vec<String>,
}

/// The window's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StartState {
    pub open: bool,
    /// The chooser shows in place of the recent documents (the plus button).
    pub chooser: bool,
    /// The script whose glyph sets show on the right.
    pub script: Option<String>,
    /// The chooser's search field.
    pub search: String,
    /// Every script with glyph sets, in the built-in order; filled when the window opens.
    pub picks: Vec<ScriptPick>,
    /// The recent document clicked last.
    pub selected_recent: Option<String>,
    /// Why the last Create Document failed.
    pub error: Option<String>,
    #[serde(skip)]
    pub reveal: Reveal,
    #[serde(skip)]
    pub laid_out: LaidOut,
}

/// What the last frame laid out, with rectangles in the window: tests click them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaidOut {
    pub recent: Vec<(String, Rect)>,
    pub scripts: Vec<(String, Rect)>,
    pub sets: Vec<(String, Rect)>,
    pub no: Option<Rect>,
    pub yes: Option<Rect>,
    pub plus: Option<Rect>,
    pub create: Option<Rect>,
}

// ── Recent documents ──

/// The stored list, newest first, without entries no command would have written.
fn stored_recent(session: &Session) -> Vec<String> {
    let list = session.settings().reserved::<Vec<String>>(Reserved::RecentDocuments).ok().flatten().unwrap_or_default();
    list.into_iter().filter(|p| !p.is_empty() && p.len() <= MAX_PATH_BYTES).take(MAX_RECENT).collect()
}

/// The recent documents whose files still exist, newest first.
pub fn recent_documents(session: &Session) -> Vec<String> {
    stored_recent(session).into_iter().filter(|p| Path::new(p).exists()).collect()
}

/// Puts `path` first in the recent documents, as an absolute path, once, keeping at most
/// [`MAX_RECENT`].
pub fn remember(session: &mut Session, path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(format!("a path of {} bytes is not kept in the recent documents", path.len()));
    }
    let p = Path::new(path);
    let full = if p.is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_string(), |d| d.join(p).to_string_lossy().into_owned())
    };
    let mut list = stored_recent(session);
    list.retain(|p| *p != full);
    list.insert(0, full);
    list.truncate(MAX_RECENT);
    session.settings_mut().set_reserved(Reserved::RecentDocuments, &list).map_err(|e| e.to_string())
}

// ── The chooser's picks ──

/// A pick as `file.newFromGlyphSets` stores it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    script: String,
    #[serde(default)]
    glyph_sets: Option<Vec<String>>,
}

/// The sets a script takes when none are named: its default sets, else its first.
fn default_sets(data: &Sets, script: &str) -> Vec<String> {
    let of_script: Vec<&sets::GlyphSet> = data.glyph_sets.iter().filter(|g| g.script == script).collect();
    let defaults: Vec<String> = of_script.iter().filter(|g| g.default).map(|g| g.id.clone()).collect();
    if defaults.is_empty() { of_script.first().map(|g| vec![g.id.clone()]).unwrap_or_default() } else { defaults }
}

/// `ids` that are glyph sets of `script`, in the built-in order.
fn in_order(data: &Sets, script: &str, ids: &[String]) -> Vec<String> {
    data.glyph_sets.iter().filter(|g| g.script == script && ids.contains(&g.id)).map(|g| g.id.clone()).collect()
}

/// The chooser's picks as the last `file.newFromGlyphSets` left them: every script with glyph
/// sets, the ones it named included with their sets. A script left out shows its default sets,
/// which it takes when it is switched on. Without last picks, Latin is included.
pub fn last_picks(session: &Session) -> Vec<ScriptPick> {
    let Ok(data) = sets::builtin() else { return Vec::new() };
    let stored: Option<Vec<Stored>> = session.settings().reserved::<Vec<Stored>>(Reserved::ChooserPicks).ok().flatten();
    data.glyph_set_scripts()
        .into_iter()
        .map(|script| {
            let defaults = default_sets(data, script);
            let (included, glyph_sets) = match &stored {
                None => (script == FIRST_SCRIPT, defaults),
                Some(list) => match list.iter().find(|p| p.script == script) {
                    Some(p) => (true, p.glyph_sets.as_deref().map_or(defaults, |ids| in_order(data, script, ids))),
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

/// Includes `script` in the new document or leaves it out (the No / Yes toggle).
pub fn set_included(st: &mut StartState, script: &str, on: bool) {
    if let Some(p) = st.picks.iter_mut().find(|p| p.script == script) {
        p.included = on;
    }
}

/// Checks or clears one glyph set of `script` (its checkbox).
pub fn set_glyph_set(st: &mut StartState, script: &str, set: &str, on: bool) {
    let Ok(data) = sets::builtin() else { return };
    let Some(p) = st.picks.iter_mut().find(|p| p.script == script) else { return };
    let mut ids: Vec<String> = p.glyph_sets.iter().filter(|id| *id != set).cloned().collect();
    if on {
        ids.push(set.to_string());
    }
    p.glyph_sets = in_order(data, script, &ids);
}

/// A script id as the chooser lists it.
pub fn script_label(script: &str) -> String {
    let mut chars = script.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

// ── Commands ──

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

/// Opens or closes the window. Opening it restores the last picks.
fn open_window(app: &mut FontcraftApp, open: bool, chooser: Option<bool>) {
    let st = &mut app.state.start;
    if open && !st.open {
        st.picks = last_picks(&app.session);
        st.script = st.picks.iter().find(|p| p.included).or(st.picks.first()).map(|p| p.script.clone());
        st.search.clear();
        st.error = None;
        st.chooser = false;
    }
    st.open = open;
    if let Some(c) = chooser {
        st.chooser = c;
    }
}

fn window_json(app: &FontcraftApp) -> Value {
    let st = &app.state.start;
    let recent = recent_documents(&app.session);
    json!({ "open": st.open, "chooser": st.chooser || recent.is_empty(), "recent": recent, "picks": st.picks })
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.startWindow" => {
            let open = args.flag("open")?.unwrap_or(true);
            let chooser = args.flag("chooser")?;
            open_window(app, open, chooser);
            Ok(window_json(app))
        }
        "file.openRecent" => open_recent(app, args),
        "file.showInFinder" => show_in_finder(app, args),
        _ => Err(args.err("unknown command")),
    }
}

fn open_recent(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let index = args.index("index")?;
    let path = args.string("path")?;
    if args.flag("clear")? == Some(true) {
        if index.is_some() || path.is_some() {
            return Err(args.err("give `clear` alone"));
        }
        let cleared = stored_recent(&app.session).len();
        app.session.settings_mut().clear_reserved(Reserved::RecentDocuments);
        return Ok(json!({ "cleared": cleared }));
    }
    let list = recent_documents(&app.session);
    let path = match (index, path) {
        (Some(_), Some(_)) => return Err(args.err("give `index` or `path`, not both")),
        (Some(_), None) if list.is_empty() => return Err(args.err("there are no recent documents")),
        (Some(i), None) => usize::try_from(i)
            .ok()
            .and_then(|i| list.get(i))
            .cloned()
            .ok_or_else(|| args.err(format!("`index` must be below {}, the number of recent documents", list.len())))?,
        (None, Some(p)) => list.iter().find(|l| *l == p).cloned().ok_or_else(|| args.err(format!("`{}` is not a recent document", quoted(p))))?,
        (None, None) => {
            open_window(app, true, Some(false));
            return Ok(window_json(app));
        }
    };
    let opened = app.run("file.open", &json!({ "path": path }))?;
    Ok(json!({ "title": opened.get("title"), "path": opened.get("path") }))
}

fn show_in_finder(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let path = args.string("path")?.ok_or_else(|| args.err("give `path`"))?;
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(args.err(format!("`path` must be from 1 to {MAX_PATH_BYTES} bytes")));
    }
    if !Path::new(path).exists() {
        return Err(args.err(format!("`{}` does not exist", quoted(path))));
    }
    let reveal = app.state.start.reveal.0.ok_or_else(|| args.err("this window cannot show files in the file manager"))?;
    reveal(Path::new(path)).map_err(|e| args.err(e))?;
    Ok(json!({ "path": path }))
}

/// After every command: a document opened or saved goes first in the recent documents; a
/// document opened or created closes the window; a failed Create Document says why in it.
pub fn after_run(app: &mut FontcraftApp, id: &str, result: &Result<Value, String>) {
    match result {
        Ok(r) => {
            if matches!(id, "file.open" | "file.save" | "file.saveAs")
                && let Some(path) = r.get("path").and_then(Value::as_str)
                && let Err(e) = remember(&mut app.session, path)
            {
                log::warn!("recent documents: {e}");
            }
            let opened = match id {
                "file.open" | "file.new" | NEW_FROM_GLYPH_SETS => true,
                "file.openRecent" => r.get("path").is_some(),
                _ => false,
            };
            let st = &mut app.state.start;
            if opened && st.open {
                st.open = false;
                st.chooser = false;
                st.error = None;
            }
        }
        Err(e) => {
            let st = &mut app.state.start;
            if id == NEW_FROM_GLYPH_SETS && st.open {
                st.error = Some(e.clone());
            }
        }
    }
}

/// File › New from Glyph Sets… from the menu (no parameters) opens the chooser, preselected with
/// the last picks, instead of repeating them headless. Returns whether it did.
pub fn opens_chooser(app: &mut FontcraftApp, id: &str, params: &Value) -> bool {
    let empty = params.is_null() || params.as_object().is_some_and(Map::is_empty);
    if id != NEW_FROM_GLYPH_SETS || !empty {
        return false;
    }
    open_window(app, true, Some(true));
    true
}

// ── Drawing ──

/// What the person did in the window this frame.
enum Event {
    Close,
    Plus,
    Back,
    SelectRecent(String),
    OpenRecent(String),
    Reveal(String),
    ShowAtLaunch(bool),
    Search(String),
    Focus(String),
    Include(String, bool),
    GlyphSet(String, String, bool),
    Create,
}

fn apply(st: &mut StartState, e: Event, pending: &mut Pending) {
    match e {
        Event::Close => st.open = false,
        Event::Plus => {
            st.chooser = true;
            st.error = None;
        }
        Event::Back => st.chooser = false,
        Event::SelectRecent(p) => st.selected_recent = Some(p),
        Event::OpenRecent(p) => pending.push(("file.openRecent", json!({ "path": p }))),
        Event::Reveal(p) => pending.push(("file.showInFinder", json!({ "path": p }))),
        Event::ShowAtLaunch(on) => pending.push(("settings.set", json!({ "id": SHOW_AT_LAUNCH, "value": on }))),
        Event::Search(s) => st.search = s,
        Event::Focus(s) => st.script = Some(s),
        Event::Include(s, on) => set_included(st, &s, on),
        Event::GlyphSet(script, set, on) => set_glyph_set(st, &script, &set, on),
        Event::Create => {
            st.error = None;
            pending.push((NEW_FROM_GLYPH_SETS, json!({ "scripts": picks_param(&st.picks) })));
        }
    }
}

/// Draws the window while it is open. Its buttons queue their commands in `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.start.open {
        return;
    }
    let recent = recent_documents(&app.session);
    let show_at_launch = app.session.settings().get_bool(SHOW_AT_LAUNCH).unwrap_or(true);
    let FontcraftApp { state, tokens, .. } = app;
    let st = &mut state.start;
    let chooser = st.chooser || recent.is_empty();
    let mut events: Vec<Event> = Vec::new();
    let mut laid = LaidOut::default();
    let mut open = true;
    let width = tokens.list_column_unit * WINDOW_UNITS;
    egui::Window::new("FontCraft").id(egui::Id::new("window.start")).open(&mut open).collapsible(false).resizable(false).default_width(width).show(
        ui.ctx(),
        |ui| {
            ui.set_width(width);
            if chooser {
                chooser_ui(ui, st, tokens, !recent.is_empty(), &mut events, &mut laid);
            } else {
                recent_ui(ui, st, &recent, show_at_launch, tokens, &mut events, &mut laid);
            }
        },
    );
    if !open {
        events.push(Event::Close);
    }
    for e in events {
        apply(st, e, pending);
    }
    st.laid_out = laid;
}

/// The recent documents on the left; version, website and the plus button on the right.
fn recent_ui(
    ui: &mut egui::Ui,
    st: &StartState,
    recent: &[String],
    show_at_launch: bool,
    tokens: &Tokens,
    events: &mut Vec<Event>,
    laid: &mut LaidOut,
) {
    let list_width = tokens.list_column_unit * (WINDOW_UNITS - SCRIPT_LIST_UNITS);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(list_width);
            ui.label(egui::RichText::new("Recent Documents").strong());
            egui::ScrollArea::vertical().id_salt("start.recent").max_height(tokens.list_row_height * LIST_ROWS).show(ui, |ui| {
                for path in recent {
                    let p = Path::new(path);
                    let name = p.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                    let folder = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                    let selected = st.selected_recent.as_deref() == Some(path.as_str());
                    let response = ui.selectable_label(selected, name).on_hover_text(folder);
                    laid.recent.push((path.clone(), response.rect));
                    if response.double_clicked() {
                        events.push(Event::OpenRecent(path.clone()));
                    } else if response.clicked() {
                        events.push(Event::SelectRecent(path.clone()));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Show in Finder").clicked() {
                            events.push(Event::Reveal(path.clone()));
                            ui.close();
                        }
                    });
                }
            });
        });
        column_rule(ui, tokens);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("FontCraft").strong().size(tokens.dialog_title_size));
            ui.label(egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(tokens.text_dim));
            ui.hyperlink_to("Project website", env!("CARGO_PKG_REPOSITORY"));
            ui.add_space(tokens.dialog_spacing);
            let mut on = show_at_launch;
            if ui.checkbox(&mut on, "Show window on startup").changed() {
                events.push(Event::ShowAtLaunch(on));
            }
            ui.add_space(tokens.dialog_spacing);
            let plus = ui.button("+").on_hover_text("New document from glyph sets");
            laid.plus = Some(plus.rect);
            if plus.clicked() {
                events.push(Event::Plus);
            }
        });
    });
}

/// The line between the window's two columns, as high as a list. A plain separator in a row
/// takes the window's whole height, and the window grows by it every frame.
fn column_rule(ui: &mut egui::Ui, tokens: &Tokens) {
    let height = tokens.list_row_height * LIST_ROWS;
    ui.add_sized(vec2(ui.spacing().item_spacing.x, height), egui::Separator::default().vertical());
}

/// The chooser: the script list on the left, the clicked script's toggle and glyph sets on the
/// right, the search field at the top right, Create Document at the lower right.
fn chooser_ui(ui: &mut egui::Ui, st: &StartState, tokens: &Tokens, has_recent: bool, events: &mut Vec<Event>, laid: &mut LaidOut) {
    let Ok(data) = sets::builtin() else {
        ui.colored_label(tokens.error_text, "The glyph sets cannot be read");
        return;
    };
    ui.horizontal(|ui| {
        if has_recent && ui.button("Recent Documents").clicked() {
            events.push(Event::Back);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut search = st.search.clone();
            let field = egui::TextEdit::singleline(&mut search)
                .hint_text("Search scripts")
                .char_limit(MAX_SEARCH_CHARS)
                .desired_width(tokens.list_column_unit * SCRIPT_LIST_UNITS);
            if ui.add(field).changed() {
                events.push(Event::Search(search));
            }
        });
    });
    ui.add_space(tokens.dialog_spacing);
    let needle = st.search.trim().to_lowercase();
    let height = tokens.list_row_height * LIST_ROWS;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(tokens.list_column_unit * SCRIPT_LIST_UNITS);
            egui::ScrollArea::vertical().id_salt("start.scripts").max_height(height).show(ui, |ui| {
                for pick in st.picks.iter().filter(|p| needle.is_empty() || script_label(&p.script).to_lowercase().contains(&needle)) {
                    ui.horizontal(|ui| {
                        let size = tokens.badge_size;
                        let (dot, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                        let radius = size / 2.0 - 1.0;
                        if pick.included {
                            ui.painter().circle_filled(dot.center(), radius, tokens.accent);
                        } else {
                            ui.painter().circle_stroke(dot.center(), radius, Stroke::new(1.0, tokens.text_dim));
                        }
                        let focused = st.script.as_deref() == Some(pick.script.as_str());
                        let response = ui.selectable_label(focused, script_label(&pick.script));
                        laid.scripts.push((pick.script.clone(), response.rect));
                        if response.clicked() {
                            events.push(Event::Focus(pick.script.clone()));
                        }
                    });
                }
            });
        });
        column_rule(ui, tokens);
        ui.vertical(|ui| {
            let Some(pick) = st.script.as_deref().and_then(|s| st.picks.iter().find(|p| p.script == s)) else {
                ui.label("Click a script to see its glyph sets");
                return;
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(script_label(&pick.script)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Right to left: Yes is placed first so it ends up on the right.
                    let yes = ui.selectable_label(pick.included, "Yes");
                    let no = ui.selectable_label(!pick.included, "No");
                    laid.yes = Some(yes.rect);
                    laid.no = Some(no.rect);
                    if yes.clicked() && !pick.included {
                        events.push(Event::Include(pick.script.clone(), true));
                    }
                    if no.clicked() && pick.included {
                        events.push(Event::Include(pick.script.clone(), false));
                    }
                });
            });
            ui.add_space(tokens.dialog_spacing);
            for set in data.glyph_sets.iter().filter(|g| g.script == pick.script) {
                let mut on = pick.glyph_sets.contains(&set.id);
                let label = format!("{} ({} glyphs)", set.name, set.glyphs.len());
                let response = ui.add_enabled(pick.included, egui::Checkbox::new(&mut on, label));
                laid.sets.push((set.id.clone(), response.rect));
                if response.changed() {
                    events.push(Event::GlyphSet(pick.script.clone(), set.id.clone(), on));
                }
            }
        });
    });
    ui.add_space(tokens.dialog_spacing);
    if let Some(e) = &st.error {
        ui.colored_label(tokens.error_text, e);
    }
    // One row high: a right-to-left layout straight in the window would centre the button in
    // the window's whole height and grow the window every frame.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let any = st.picks.iter().any(|p| p.included && !p.glyph_sets.is_empty());
            let create = ui.add_enabled(any, egui::Button::new("Create Document"));
            laid.create = Some(create.rect);
            if create.clicked() {
                events.push(Event::Create);
            }
        });
    });
}
