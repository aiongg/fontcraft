//! Font View's sidebar (`fontView.sidebar`): the search field, then All and the Categories,
//! Languages and Filters sections, each entry with its glyph count, or a grey present/expected
//! badge or a checkmark when the entry knows its full glyph set (`fontView.sidebar.missingBadge`).
//! The entries, counts and badges come from `fontView.queryGlyphs` with `sidebar: true`, read
//! again only when the document, the settings, the glyph data or Font View's master changed.
//!
//! A click selects one entry; Cmd-click adds or removes one; Shift-click selects the entries
//! from the last one selected in that section to the one clicked
//! (`fontView.selectSidebarEntries`). The selection is `FontViewState.query`'s `entries`, and the
//! engine combines them: categories add up, scripts and languages add up, filters narrow each
//! other, and sections narrow each other (`docs/spec/05-font-view.md` § 6.2).
//!
//! Selected glyphs that a new sidebar selection or search hides are deselected, so batch
//! commands never act on glyphs the view does not show.
//!
//! Filters are added, edited and removed through the actions menu at the sidebar's foot and the
//! entries' context menus, with the M4.5 filter commands; the filters live in the user settings.
//! Each gesture on a filter row runs one command:
//!
//! | Gesture | Runs |
//! |---|---|
//! | double-click a filter's or folder's name, or Return with one filter selected, then Return | `fontView.renameFilter` |
//! | Escape or a click away from the name field | nothing |
//! | double-click a smart or list filter's icon | its editor (`fontView.editFilter`) |
//! | drag a filter or folder: a line between rows, or a highlighted folder | `fontView.moveFilter` |
//!
//! A drop the engine would refuse (into the dragged folder or one of its folders, deeper than
//! folders nest, or beside an entry of the same name) shows no mark and runs nothing.
//! A renamed or moved entry stays selected.

use std::collections::HashSet;

use egui::{Align2, FontId, Pos2, Rect, Sense, Shape, Stroke, vec2};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::search::{self, SearchState};
use crate::app::FontcraftApp;
use crate::panels::widgets;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

pub type Pending = Vec<(&'static str, Value)>;

/// Most sidebar entries one selection may hold (`fontView.queryGlyphs` takes no more).
pub const MAX_ENTRIES: usize = 256;
/// Most parts of a sidebar path.
pub const MAX_PATH_SEGMENTS: usize = 16;
/// Longest part of a sidebar path, in bytes.
const MAX_SEGMENT_BYTES: usize = 256;
/// Most filters and folders the Filters section holds (the engine's limit).
const MAX_FILTERS: usize = 1024;
/// Deepest nesting the sidebar draws (folders and groups nest at most 8 deep in the engine).
const MAX_DEPTH: usize = 24;
/// Longest name the rename field takes, in bytes (the engine's limit).
const NAME_BYTES: usize = fontcraft_glyphquery::MAX_FILTER_NAME_BYTES;
/// The scripts the Languages section lists until the user chooses others (the engine's
/// default list).
pub const DEFAULT_SCRIPTS: [&str; 3] = ["latin", "greek", "cyrillic"];

pub const COMMANDS: &[UiCommand] = &[
    (
        "fontView.selectSidebarEntries",
        "Select Sidebar Entries",
        &[],
        "",
        "{entries: [path], mode?: \"replace\" (default) | \"toggle\" | \"range\"} select the sidebar entries that filter Font View; a path is text such as all, categories/Letter/Uppercase, languages/latin/de or filters/Folder/Name, or a list of names. replace: these entries only; toggle (Cmd-click): add each entry not selected, remove each selected one; range (Shift-click): one entry, and every entry of its section from the last one selected there to it. Selecting all shows every glyph. Selected glyphs the new view hides are deselected → {entries, search, count}",
    ),
    (
        "fontView.setSearch",
        "Search Glyphs",
        &[],
        "",
        "{query: string, fields?: [\"name\" | \"unicode\" | \"note\"], matchCase?: bool, regex?: bool} the search term that narrows the glyphs the sidebar selection shows, matched against the name, the code point in hexadecimal, the character and the note (or only the fields named); an empty query removes it. Options left out keep their value. A regular expression must match a whole field. Selected glyphs the new view hides are deselected → {entries, search, count}",
    ),
    (
        "fontView.focusSearch",
        "Find (Search Glyphs)",
        &[],
        "",
        "{} give the keyboard to Font View's search field; in Edit View it does nothing → {focused: bool}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "fontView.selectSidebarEntries" => select_entries(app, args),
        "fontView.setSearch" => search::set(app, args),
        "fontView.focusSearch" => Ok(search::focus(app)),
        other => Err(format!("unknown command `{other}`")),
    }
}

// ── Paths ───────────────────────────────────────────────────────────────────────────────────────

/// A sidebar path from JSON: `"filters/Folder/Name"` or `["filters", "Folder", "Name"]`.
pub fn path_of(v: &Value) -> Option<Vec<String>> {
    let parts: Vec<String> = match v {
        Value::String(s) => s.split('/').map(str::to_string).collect(),
        Value::Array(items) if items.len() <= MAX_PATH_SEGMENTS => items.iter().map(|i| i.as_str().map(str::to_string)).collect::<Option<_>>()?,
        _ => return None,
    };
    let fits = !parts.is_empty() && parts.len() <= MAX_PATH_SEGMENTS && parts.iter().all(|p| !p.is_empty() && p.len() <= MAX_SEGMENT_BYTES);
    fits.then_some(parts)
}

/// A path as the query stores it: text, or a list when a part holds `/`.
pub fn path_value(path: &[String]) -> Value {
    if path.iter().any(|p| p.contains('/')) { json!(path) } else { json!(path.join("/")) }
}

fn is_all(path: &[String]) -> bool {
    path.len() == 1 && path.first().is_some_and(|p| p == "all")
}

/// The sidebar entries selected now (none: All).
pub fn current_entries(app: &FontcraftApp) -> Vec<Vec<String>> {
    app.state
        .font_view
        .query
        .as_ref()
        .and_then(|q| q.get("entries"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(path_of).collect())
        .unwrap_or_default()
}

// ── The stored filters ──────────────────────────────────────────────────────────────────────────

/// The Filters section as the user settings store it (`[{kind, name, rules | glyphs |
/// children}]`).
pub fn stored_filters(settings: &fontcraft_engine::settings::UserSettings) -> Vec<Value> {
    settings.get("fontView.filters").ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// The stored filter at `path` (`filters/…` or the path inside the section).
pub fn stored_filter(settings: &fontcraft_engine::settings::UserSettings, path: &[String]) -> Option<Value> {
    let inside = match path.split_first() {
        Some((first, rest)) if first == "filters" => rest,
        _ => path,
    };
    let mut level = stored_filters(settings);
    let mut found = None;
    for name in inside {
        let item = level.iter().find(|f| f.get("name").and_then(Value::as_str) == Some(name.as_str()))?.clone();
        level = item.get("children").and_then(Value::as_array).cloned().unwrap_or_default();
        found = Some(item);
    }
    found
}

/// `base`, or `base 2`, `base 3`… : the first name no top-level filter has.
pub fn unique_filter_name(settings: &fontcraft_engine::settings::UserSettings, base: &str) -> String {
    let taken: HashSet<String> = stored_filters(settings).iter().filter_map(|f| f.get("name").and_then(Value::as_str)).map(str::to_string).collect();
    (1..=MAX_FILTERS + 1)
        .map(|n| if n == 1 { base.to_string() } else { format!("{base} {n}") })
        .find(|n| !taken.contains(n))
        .unwrap_or_else(|| base.to_string())
}

// ── Selecting ───────────────────────────────────────────────────────────────────────────────────

fn select_entries(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let list = args.get("entries").and_then(Value::as_array).ok_or_else(|| args.err("`entries` must be a list of sidebar paths"))?;
    if list.len() > MAX_ENTRIES {
        return Err(args.err(format!("`entries` holds {} paths; at most {MAX_ENTRIES} are allowed", list.len())));
    }
    let given: Vec<Vec<String>> = list
        .iter()
        .map(|v| path_of(v).ok_or_else(|| args.err(format!("`{v}` is not a sidebar path: text such as categories/Letter, or a list of names"))))
        .collect::<Result<_, _>>()?;
    let current = current_entries(app);
    let next = match args.string("mode")?.unwrap_or("replace") {
        "replace" => given.clone(),
        "toggle" => {
            let mut next = current;
            for path in &given {
                match next.iter().position(|c| c == path) {
                    Some(i) => {
                        next.remove(i);
                    }
                    None => next.push(path.clone()),
                }
            }
            next
        }
        "range" => {
            let [clicked] = given.as_slice() else { return Err(args.err("`range` takes one entry: the one Shift-clicked")) };
            range(app, &current, clicked).map_err(|e| args.err(e))?
        }
        other => return Err(args.err(format!("`mode` must be \"replace\", \"toggle\" or \"range\", got \"{other}\""))),
    };
    // All is the absence of a selection: choosing it drops the rest.
    let mut next = if given.iter().any(|p| is_all(p)) { Vec::new() } else { next };
    next.retain(|p| !is_all(p));
    let mut seen = HashSet::new();
    next.retain(|p| seen.insert(p.clone()));
    if next.len() > MAX_ENTRIES {
        return Err(args.err(format!("at most {MAX_ENTRIES} sidebar entries can be selected at once")));
    }
    let search = SearchState::of(app.state.font_view.query.as_ref());
    apply(app, next, &search).map_err(|e| args.err(e))
}

/// Shift-click on `clicked`: the entries of its section from the last one selected there to it,
/// in the order the sidebar lists them, with the other sections' entries kept. Without a selected
/// entry in that section, the clicked one is added.
fn range(app: &mut FontcraftApp, current: &[Vec<String>], clicked: &[String]) -> Result<Vec<Vec<String>>, String> {
    let section = clicked.first().cloned().unwrap_or_default();
    let mut next = current.to_vec();
    let Some(anchor) = current.iter().rev().find(|c| c.first() == Some(&section)).cloned() else {
        next.push(clicked.to_vec());
        return Ok(next);
    };
    let order = section_order(app, &section)?;
    let (Some(a), Some(b)) = (order.iter().position(|p| *p == anchor), order.iter().position(|p| p == clicked)) else {
        next.push(clicked.to_vec());
        return Ok(next);
    };
    let span = order.get(a.min(b)..=a.max(b)).unwrap_or_default();
    next.retain(|c| c.first() != Some(&section));
    next.extend(span.iter().filter(|p| **p != anchor).cloned());
    // The anchor goes last, so it stays the anchor for the next Shift-click.
    next.push(anchor);
    Ok(next)
}

/// Every entry of a section in the order the sidebar lists them, children after their parent.
fn section_order(app: &mut FontcraftApp, section: &str) -> Result<Vec<Vec<String>>, String> {
    let mut params = Map::new();
    params.insert("sidebar".into(), json!(true));
    if let Some(m) = super::master(app) {
        params.insert("master".into(), json!(m));
    }
    let result = app.session.execute("fontView.queryGlyphs", &Value::Object(params)).map_err(|e| e.to_string())?;
    let items: Vec<Item> = result.get("sidebar").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let mut out = Vec::new();
    if let Some(top) = items.iter().find(|i| i.path.len() == 1 && i.path.first().is_some_and(|p| p == section)) {
        flatten(&top.children, &mut out, 0);
    }
    Ok(out)
}

fn flatten(items: &[Item], out: &mut Vec<Vec<String>>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for item in items {
        out.push(item.path.clone());
        flatten(&item.children, out, depth + 1);
    }
}

/// Puts `entries` and `search` into Font View's query once the engine has accepted them, and
/// deselects the glyphs the view no longer shows. A refused query changes nothing.
pub(crate) fn apply(app: &mut FontcraftApp, entries: Vec<Vec<String>>, search: &SearchState) -> Result<Value, String> {
    let mut q = Map::new();
    if !entries.is_empty() {
        q.insert("entries".into(), Value::Array(entries.iter().map(|p| path_value(p)).collect()));
    }
    if let Some(s) = search.to_json() {
        q.insert("search".into(), s);
    }
    let query = (!q.is_empty()).then(|| Value::Object(q.clone()));
    let count = match &query {
        None => app.session.font().map(|f| f.glyphs.len()).unwrap_or(0),
        Some(_) => {
            let mut params = q.clone();
            if let Some(m) = super::master(app) {
                params.insert("master".into(), json!(m));
            }
            let result = app.session.execute("fontView.queryGlyphs", &Value::Object(params)).map_err(|e| e.to_string())?;
            let shown: HashSet<&str> =
                result.get("glyphs").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            app.state.selected_glyphs.retain(|n| shown.contains(n.as_str()));
            shown.len()
        }
    };
    app.state.font_view.query = query;
    let entries: Vec<Value> = entries.iter().map(|p| path_value(p)).collect();
    Ok(json!({"entries": entries, "search": search.to_json(), "count": count}))
}

// ── The sidebar's entries ───────────────────────────────────────────────────────────────────────

/// One entry as `fontView.queryGlyphs` lists it with `sidebar: true`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub path: Vec<String>,
    pub label: String,
    /// `all`, `section`, `category`, `subcategory`, `case`, `script`, `language`, `group`,
    /// `smart`, `list` or `folder`.
    pub kind: String,
    pub count: usize,
    pub expected: Option<usize>,
    pub present: Option<usize>,
    pub children: Vec<Item>,
    pub problem: Option<String>,
}

impl Item {
    /// The entry knows its full set and the font lacks some of it.
    pub fn lacks_glyphs(&self) -> bool {
        matches!((self.present, self.expected), (Some(p), Some(e)) if p < e)
    }
}

/// What the entries were read from.
#[derive(Clone, PartialEq)]
struct Key {
    doc: (u64, u64),
    settings: u64,
    master: String,
    /// The glyph data and the user's `Groups*.plist` entries, which change without a new
    /// document or settings.
    glyph_data: u64,
}

/// A row the last frame laid out: its entry, its rectangle, its disclosure triangle's and its
/// kind icon's, and how deep it is nested (a section is 0).
#[derive(Clone, Debug)]
pub struct Row {
    pub path: Vec<String>,
    pub rect: Rect,
    pub chevron: Option<Rect>,
    pub icon: Option<Rect>,
    pub depth: usize,
}

/// The name field of the filter being renamed in place.
#[derive(Clone, Debug)]
pub struct Renaming {
    pub path: Vec<String>,
    pub text: String,
    /// The field has been drawn once, so a frame without the keyboard means it lost it.
    started: bool,
}

/// Where a dragged filter would land: `index` among the entries of the folder `parent` (a path
/// inside the Filters section; empty for the top level), as `fontView.moveFilter` takes them.
#[derive(Clone, Debug, PartialEq)]
pub struct DropTarget {
    pub parent: Vec<String>,
    pub index: usize,
    pub mark: Mark,
}

/// What the sidebar draws for a drop target.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A line at `y`, indented as a row `depth` deep.
    Line { y: f32, depth: usize },
    /// The folder row the filter would go into.
    Into(Rect),
}

/// The sidebar's state that is not data.
#[derive(Default)]
pub struct SidebarUi {
    key: Option<Key>,
    pub items: Vec<Item>,
    /// Why the entries could not be read.
    pub error: Option<String>,
    /// `fontView.focusSearch` asked for the keyboard; the field takes it when next drawn.
    pub focus_search: bool,
    /// The search field's text while it has the keyboard.
    pub typed: Option<String>,
    /// The rows of the last frame, for tests and clicks over the control channel.
    pub laid_out: Vec<Row>,
    /// The part of the rows' scroll area the last frame showed: rows scrolled out of it take
    /// no presses or drops.
    pub rows_clip: Option<Rect>,
    /// The filter whose name is being edited in place.
    pub renaming: Option<Renaming>,
    /// The filter row being dragged.
    pub dragging: Option<Vec<String>>,
    /// Where the dragged filter would land, as the last frame showed it.
    pub drop: Option<DropTarget>,
    /// The last pointer press was on a sidebar row, so Return renames the selected filter.
    pub has_keyboard: bool,
    /// Entries renamed or moved by the commands this sidebar ran last frame (old path, new
    /// path), so the selection follows them.
    follow: Vec<(Vec<String>, Vec<String>)>,
}

/// The id of the rename field.
pub fn rename_id() -> egui::Id {
    widgets::id("fontView.sidebar.rename")
}

/// Whether Font View's sidebar has the keyboard: the last pointer press was on one of its rows.
pub fn has_keyboard(app: &FontcraftApp) -> bool {
    app.font_view.sidebar.has_keyboard
}

/// Reads the entries again when something they depend on changed, and deselects entries that
/// no longer exist (a removed or renamed filter).
pub fn refresh(app: &mut FontcraftApp) {
    let Some(doc) = app.session.active() else {
        app.font_view.sidebar.items.clear();
        app.font_view.sidebar.key = None;
        return;
    };
    let doc = (doc.id, doc.revision);
    let master = super::master(app).unwrap_or_default();
    let key = Key { doc, settings: app.session.settings().revision(), master: master.clone(), glyph_data: app.session.glyph_data_revision() };
    let follow = std::mem::take(&mut app.font_view.sidebar.follow);
    if app.font_view.sidebar.key.as_ref() == Some(&key) {
        return;
    }
    let mut params = Map::new();
    params.insert("sidebar".into(), json!(true));
    if !master.is_empty() {
        params.insert("master".into(), json!(master));
    }
    let sb = &mut app.font_view.sidebar;
    match app.session.execute("fontView.queryGlyphs", &Value::Object(params)) {
        Ok(result) => {
            sb.items = result.get("sidebar").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            sb.error = None;
        }
        Err(e) => {
            sb.items.clear();
            sb.error = Some(e.to_string());
        }
    }
    sb.key = Some(key);
    if sb.error.is_none() {
        let mut listed = Vec::new();
        flatten(&sb.items, &mut listed, 0);
        // A rename or move the engine refused leaves its old path listed, so it is not followed.
        let follow: Vec<_> = follow.into_iter().filter(|(old, new)| !listed.contains(old) && listed.contains(new)).collect();
        let entries = current_entries(app);
        let mut kept: Vec<Vec<String>> = entries.iter().map(|e| followed(e, &follow)).filter(|e| listed.contains(e)).collect();
        let mut seen = HashSet::new();
        kept.retain(|e| seen.insert(e.clone()));
        if kept != entries
            && let Some(Value::Object(q)) = app.state.font_view.query.as_mut()
        {
            if kept.is_empty() {
                q.remove("entries");
            } else {
                q.insert("entries".into(), Value::Array(kept.iter().map(|p| path_value(p)).collect()));
            }
            if q.is_empty() {
                app.state.font_view.query = None;
            }
        }
    }
}

/// `entry` with the path of a renamed or moved filter or folder replaced by its new one.
fn followed(entry: &[String], follow: &[(Vec<String>, Vec<String>)]) -> Vec<String> {
    for (old, new) in follow {
        if let Some(rest) = entry.strip_prefix(old.as_slice()) {
            let mut out = new.clone();
            out.extend(rest.iter().cloned());
            return out;
        }
    }
    entry.to_vec()
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// What drawing the rows needs besides the rows.
struct Cx<'a> {
    tokens: &'a Tokens,
    selected: &'a [Vec<String>],
    mods: egui::Modifiers,
    pending: &'a mut Pending,
    laid_out: &'a mut Vec<Row>,
    renaming: &'a mut Option<Renaming>,
    /// Entries renamed or moved by the commands queued this frame (old path, new path).
    follow: &'a mut Vec<(Vec<String>, Vec<String>)>,
    /// A filter row whose drag began this frame.
    drag_started: Option<Vec<String>>,
    /// The dragged row was let go this frame.
    drag_stopped: bool,
}

/// Draws the sidebar at Font View's left: the search field, the entries, and the actions menu.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if app.session.font().is_err() {
        return;
    }
    refresh(app);
    let frame = egui::Frame::NONE.fill(app.tokens.panel).inner_margin(app.tokens.palette_margin);
    let width = app.tokens.palette_width * super::inspector::COLUMN_WIDTH;
    egui::Panel::left("fontView.sidebar").frame(frame).default_size(width).show(ui, |ui| {
        // The inspector takes the column's bottom; the lists fill the rest.
        super::inspector::show_in(app, ui, pending);
        search::field(app, ui, pending);
        ui.separator();
        let selected = current_entries(app);
        let items = std::mem::take(&mut app.font_view.sidebar.items);
        keyboard(app, ui, &items, &selected);
        let mut renaming = app.font_view.sidebar.renaming.take();
        let mut dragging = app.font_view.sidebar.dragging.take();
        let mut target = None;
        let mut follow = Vec::new();
        let mut laid_out = Vec::new();
        let mut rows_clip = None;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            actions(app, ui, &items, &selected, pending);
            ui.separator();
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                if let Some(e) = &app.font_view.sidebar.error {
                    ui.colored_label(app.tokens.error_text, e);
                }
                let mods = ui.input(|i| i.modifiers);
                let mut cx = Cx {
                    tokens: &app.tokens,
                    selected: &selected,
                    mods,
                    pending,
                    laid_out: &mut laid_out,
                    renaming: &mut renaming,
                    follow: &mut follow,
                    drag_started: None,
                    drag_stopped: false,
                };
                egui::ScrollArea::vertical().id_salt("fontView.sidebar.rows").auto_shrink([false, false]).show(ui, |ui| {
                    for item in &items {
                        row(ui, &mut cx, item, 0);
                    }
                    rows_clip = Some(ui.clip_rect());
                    if let Some(source) = cx.drag_started.take() {
                        dragging = Some(source);
                    }
                    let Some(source) = dragging.clone() else { return };
                    let at = ui.ctx().pointer_latest_pos().filter(|p| over_rows(ui.ctx(), ui.layer_id(), Some(ui.clip_rect()), *p));
                    target = at.and_then(|p| drop_target(&items, cx.laid_out, &source, p));
                    if let Some(d) = &target {
                        paint_mark(ui, cx.tokens, cx.laid_out, d);
                    }
                    if cx.drag_stopped {
                        if let Some(d) = &target {
                            let entry = moved(cx.pending, &source, d);
                            cx.follow.push(entry);
                        }
                        dragging = None;
                        target = None;
                    } else if ui.ctx().dragged_id().is_none() {
                        dragging = None;
                        target = None;
                    }
                });
            });
        });
        // A rename whose row is not drawn (its folder closed) ends, so Return works again.
        if renaming.as_ref().is_some_and(|r| !laid_out.iter().any(|row| row.path == r.path)) {
            renaming = None;
        }
        let sb = &mut app.font_view.sidebar;
        sb.items = items;
        sb.laid_out = laid_out;
        sb.rows_clip = rows_clip;
        sb.renaming = renaming;
        sb.dragging = dragging;
        sb.drop = target;
        sb.follow = follow;
    });
}

/// Return with the sidebar holding the keyboard and one filter or folder selected opens its name
/// for editing, if its row is drawn. The press that last went down decides whether the sidebar
/// holds the keyboard. The question about unsaved changes, the dialogs and the windows are drawn
/// after Font View, so Return is left to them whenever one of them is showing or has the keyboard.
fn keyboard(app: &mut FontcraftApp, ui: &egui::Ui, items: &[Item], selected: &[Vec<String>]) {
    let ctx = ui.ctx();
    let layer = ui.layer_id();
    let elsewhere = app.close_prompt.is_open()
        || app.dialog.is_some()
        || crate::shell::font_info::has_keyboard(app)
        || ctx.egui_wants_keyboard_input()
        || !ctx.memory(|m| m.is_above_modal_layer(layer));
    let sb = &mut app.font_view.sidebar;
    if let Some(at) = ui.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None }) {
        sb.has_keyboard = over_rows(ctx, layer, sb.rows_clip, at) && sb.laid_out.iter().any(|r| r.rect.contains(at));
    }
    if !sb.has_keyboard || sb.renaming.is_some() || elsewhere {
        return;
    }
    let [only] = selected else { return };
    let Some(item) = find_item(items, only, 0).filter(|i| is_filter(i)) else { return };
    if !sb.laid_out.iter().any(|r| r.path == item.path) {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        sb.renaming = Some(Renaming { path: item.path.clone(), text: item.label.clone(), started: false });
    }
}

/// Whether a press or drop at `at` reaches the sidebar's rows: inside the part of the scroll area
/// shown (`clip`), and not over a window, menu or popup drawn above the sidebar.
fn over_rows(ctx: &egui::Context, layer: egui::LayerId, clip: Option<Rect>, at: Pos2) -> bool {
    clip.is_some_and(|c| c.contains(at)) && ctx.layer_id_at(at).is_none_or(|l| l == layer)
}

/// How many folder levels `item` holds, itself included (a filter is 1).
fn folder_depth(item: &Item, depth: usize) -> usize {
    if depth > MAX_DEPTH || item.kind != "folder" {
        return 1;
    }
    1 + item.children.iter().map(|c| folder_depth(c, depth + 1)).max().unwrap_or(0)
}

/// A smart filter, list filter or folder of the Filters section.
fn is_filter(item: &Item) -> bool {
    matches!(item.kind.as_str(), "smart" | "list" | "folder")
}

/// Queues the `fontView.moveFilter` of a drop; returns the old and new path of the entry.
fn moved(pending: &mut Pending, source: &[String], drop: &DropTarget) -> (Vec<String>, Vec<String>) {
    // The sidebar's form of the paths, with the section first, so a folder named `filters` is
    // not taken for the section.
    let mut parent = vec!["filters".to_string()];
    parent.extend(drop.parent.iter().cloned());
    let mut params = Map::new();
    params.insert("filter".into(), path_value(source));
    if !drop.parent.is_empty() {
        params.insert("parent".into(), path_value(&parent));
    }
    params.insert("index".into(), json!(drop.index));
    pending.push(("fontView.moveFilter", Value::Object(params)));
    let mut new = parent;
    new.extend(source.last().cloned());
    (source.to_vec(), new)
}

/// Rows with children open by default only for the sections.
fn open_by_default(item: &Item) -> bool {
    item.kind == "section"
}

fn open_id(path: &[String]) -> egui::Id {
    widgets::id(("fontView.sidebar.open", path))
}

/// One entry and, while it is open, its children.
fn row(ui: &mut egui::Ui, cx: &mut Cx, item: &Item, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let tokens = cx.tokens;
    let height = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let filter = is_filter(item);
    let sense = if filter { Sense::click_and_drag() } else { Sense::click() };
    let response = ui.interact(rect, row_id(&item.path), sense);
    let response = match &item.problem {
        Some(problem) => response.on_hover_text(problem),
        None => response,
    };
    let section = item.kind == "section";
    let selected = !section && (cx.selected.contains(&item.path) || (item.kind == "all" && cx.selected.is_empty()));
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, tokens.radius, tokens.row_selected);
    } else if response.hovered() && !section {
        painter.rect_filled(rect, tokens.radius, tokens.hover);
    }
    let open_key = open_id(&item.path);
    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_key)).unwrap_or_else(|| open_by_default(item));
    let left = rect.left() + depth as f32 * tokens.layers_indent;
    let chevron = (!item.children.is_empty()).then(|| Rect::from_min_size(Pos2::new(left, rect.top()), vec2(height, height)));
    if let Some(c) = chevron {
        let color = if response.hovered() { tokens.text } else { tokens.text_dim };
        painter.add(triangle(c.center(), height * 0.2, open, color));
    }
    let mut x = left + height;
    let mut icon = None;
    if let Some(shape) = kind_icon(&item.kind, Pos2::new(x + height * 0.4, rect.center().y), height * 0.3, tokens.text_dim) {
        painter.extend(shape);
        icon = Some(Rect::from_min_size(Pos2::new(x, rect.top()), vec2(height, height)));
        x += height;
    }
    let text_color = match (&item.problem, selected) {
        (Some(_), _) => tokens.error_text,
        (None, true) => tokens.selected_text,
        (None, false) => tokens.text,
    };
    let font = if section { FontId::proportional(tokens.font_size) } else { egui::TextStyle::Button.resolve(ui.style()) };
    let label_end = badge(&painter, tokens, item, rect, section);
    let clip = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(label_end, rect.bottom()));
    if cx.renaming.as_ref().is_some_and(|r| r.path == item.path) {
        rename_field(ui, cx, clip);
    } else {
        ui.painter_at(clip).text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, &item.label, font, text_color);
    }
    cx.laid_out.push(Row { path: item.path.clone(), rect, chevron, icon, depth });

    if section && item.path.first().is_some_and(|p| p == "languages") {
        let plus = Rect::from_min_size(Pos2::new(rect.right() - height, rect.top()), vec2(height, height));
        let r = ui.interact(plus, widgets::id("fontView.sidebar.languagesList"), Sense::click()).on_hover_text("Edit Languages List");
        let color = if r.hovered() { tokens.text } else { tokens.text_dim };
        let c = plus.center();
        let s = height * 0.22;
        painter.line_segment([c - vec2(s, 0.0), c + vec2(s, 0.0)], Stroke::new(1.5, color));
        painter.line_segment([c - vec2(0.0, s), c + vec2(0.0, s)], Stroke::new(1.5, color));
        if r.clicked() {
            cx.pending.push(("dialog.open", json!({"dialog": "dialog.languagesList"})));
        }
    }

    let on = |part: Option<Rect>| response.interact_pointer_pos().is_some_and(|p| part.is_some_and(|r| r.contains(p)));
    if response.clicked() {
        let on_chevron = on(chevron);
        if on_chevron || section {
            ui.ctx().data_mut(|d| d.insert_temp(open_key, !open));
        } else {
            let mode = if cx.mods.command {
                "toggle"
            } else if cx.mods.shift {
                "range"
            } else {
                "replace"
            };
            cx.pending.push(("fontView.selectSidebarEntries", json!({"entries": [path_value(&item.path)], "mode": mode})));
        }
    }
    // A double-click on a filter's icon opens its editor; on the rest of the row, its name.
    if response.double_clicked() && filter && !on(chevron) {
        match editor(item) {
            Some(open_editor) if on(icon) => cx.pending.push(open_editor),
            _ => *cx.renaming = Some(Renaming { path: item.path.clone(), text: item.label.clone(), started: false }),
        }
    }
    if filter {
        if response.drag_started_by(egui::PointerButton::Primary) {
            cx.drag_started = Some(item.path.clone());
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            cx.drag_stopped = true;
        }
    }
    if !section && item.kind != "all" {
        response.context_menu(|ui| context_menu(ui, cx, item));
    }
    if open {
        for child in &item.children {
            row(ui, cx, child, depth + 1);
        }
    }
}

fn row_id(path: &[String]) -> egui::Id {
    widgets::id(("fontView.sidebar.row", path))
}

/// The name field over a filter's label: Return runs `fontView.renameFilter` with the new name;
/// Escape, a click away or an unchanged name run nothing.
fn rename_field(ui: &mut egui::Ui, cx: &mut Cx, clip: Rect) {
    let Some(r) = cx.renaming.as_mut() else { return };
    let id = rename_id();
    if !r.started {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let edit = egui::TextEdit::singleline(&mut r.text).id(id).margin(egui::Margin::ZERO).font(egui::TextStyle::Button);
    let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
    let mut output = edit.show(&mut ui.new_child(egui::UiBuilder::new().max_rect(clip).layout(layout)));
    // The whole name starts selected, so typing replaces it.
    if !r.started && output.response.response.has_focus() {
        output.state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all(&output.galley)));
        output.state.store(ui.ctx(), id);
    }
    // The engine counts a name's bytes; a longer one loses its last characters.
    if r.text.len() > NAME_BYTES {
        let end = (0..=NAME_BYTES).rev().find(|i| r.text.is_char_boundary(*i)).unwrap_or(0);
        r.text.truncate(end);
    }
    let field = output.response.response;
    let committed = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let done = field.lost_focus() || (r.started && !field.has_focus());
    r.started = true;
    if committed {
        let old = r.path.last().map(String::as_str).unwrap_or_default();
        let name = r.text.trim();
        if !name.is_empty() && name != old {
            let mut new = r.path.clone();
            new.pop();
            new.push(name.to_string());
            cx.pending.push(("fontView.renameFilter", json!({"filter": path_value(&r.path), "name": name})));
            cx.follow.push((r.path.clone(), new));
        }
    }
    if done {
        *cx.renaming = None;
    }
}

/// Where the filter at `source` (a sidebar path) lands when let go at `at`, over the rows the
/// frame laid out. `None` outside the Filters section, and for a drop the engine refuses (into
/// `source` itself or a folder inside it, deeper than folders nest, or beside an entry of the
/// same name) or onto its own place.
///
/// Over a folder's middle half the filter goes into the folder, at its end; over the top half
/// of a row it goes before the row, over the bottom half after it, or first into an open folder
/// with entries showing. Over the section's heading it goes first, below the last row last.
pub fn drop_target(items: &[Item], rows: &[Row], source: &[String], at: Pos2) -> Option<DropTarget> {
    let rows: Vec<&Row> = rows.iter().filter(|r| r.path.first().is_some_and(|p| p == "filters")).collect();
    let heading = rows.first().filter(|r| r.path.len() == 1)?;
    if at.y < heading.rect.top() || at.x < heading.rect.left() || at.x > heading.rect.right() {
        return None;
    }
    let section = items.iter().find(|i| i.path == heading.path)?;
    let moving = source.get(1..).filter(|p| !p.is_empty())?;
    let (from_name, from_parent) = moving.split_last()?;
    let siblings = |parent: &[String]| -> Option<Vec<String>> {
        let level = if parent.is_empty() {
            &section.children
        } else {
            let mut path = heading.path.clone();
            path.extend(parent.iter().cloned());
            &find_item(items, &path, 0)?.children
        };
        Some(level.iter().filter_map(|i| i.path.last().cloned()).collect())
    };
    let over = rows.iter().position(|r| at.y >= r.rect.top() && at.y < r.rect.bottom());
    let (parent, index, mark) = match over.and_then(|i| rows.get(i).map(|r| (i, *r))) {
        None => {
            let last = rows.last()?;
            if at.y < last.rect.bottom() {
                return None;
            }
            (Vec::new(), section.children.len(), Mark::Line { y: last.rect.bottom(), depth: 1 })
        }
        Some((_, r)) if r.path.len() == 1 => (Vec::new(), 0, Mark::Line { y: r.rect.bottom(), depth: 1 }),
        Some((i, r)) => {
            let inside = r.path.get(1..).unwrap_or_default();
            let (name, parent) = inside.split_last()?;
            let item = find_item(items, &r.path, 0)?;
            let folder = item.kind == "folder";
            let open = rows.get(i + 1).is_some_and(|next| next.path.len() > r.path.len() && next.path.starts_with(&r.path));
            let place = siblings(parent)?.iter().position(|n| n == name)?;
            let part = (at.y - r.rect.top()) / r.rect.height().max(1.0);
            if folder && (0.25..0.75).contains(&part) {
                (inside.to_vec(), item.children.len(), Mark::Into(r.rect))
            } else if part < 0.5 {
                (parent.to_vec(), place, Mark::Line { y: r.rect.top(), depth: r.depth })
            } else if folder && open {
                (inside.to_vec(), 0, Mark::Line { y: r.rect.bottom(), depth: r.depth + 1 })
            } else {
                (parent.to_vec(), place + 1, Mark::Line { y: r.rect.bottom(), depth: r.depth })
            }
        }
    };
    if parent.starts_with(moving) {
        return None;
    }
    let moved_item = find_item(items, source, 0)?;
    if parent.len() + folder_depth(moved_item, 0) > fontcraft_glyphquery::MAX_FOLDER_DEPTH {
        return None;
    }
    if parent.as_slice() != from_parent && siblings(&parent)?.iter().any(|n| n == from_name) {
        return None;
    }
    let mut index = index;
    if parent.as_slice() == from_parent {
        let from = siblings(from_parent)?.iter().position(|n| n == from_name)?;
        if from < index {
            index -= 1;
        }
        if from == index {
            return None;
        }
    }
    Some(DropTarget { parent, index, mark })
}

/// Draws a drop target: a line between rows, or a frame around the folder.
fn paint_mark(ui: &egui::Ui, tokens: &Tokens, rows: &[Row], drop: &DropTarget) {
    let stroke = Stroke::new(2.0, tokens.accent);
    let Some(heading) = rows.iter().find(|r| r.path.len() == 1 && r.path.first().is_some_and(|p| p == "filters")) else { return };
    match &drop.mark {
        Mark::Line { y, depth } => {
            let left = heading.rect.left() + *depth as f32 * tokens.layers_indent;
            ui.painter().line_segment([Pos2::new(left, *y), Pos2::new(heading.rect.right(), *y)], stroke);
        }
        Mark::Into(rect) => {
            ui.painter().rect_stroke(*rect, tokens.radius, stroke, egui::StrokeKind::Inside);
        }
    }
}

/// The disclosure triangle: pointing down while open.
fn triangle(c: Pos2, s: f32, open: bool, color: egui::Color32) -> Shape {
    let points = if open {
        vec![c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6), c + vec2(0.0, s * 0.8)]
    } else {
        vec![c + vec2(-s * 0.6, -s), c + vec2(-s * 0.6, s), c + vec2(s * 0.8, 0.0)]
    };
    Shape::convex_polygon(points, color, Stroke::NONE)
}

/// The small mark before a filter's name: a ring for a smart filter, lines for a list filter, a
/// tabbed box for a folder.
fn kind_icon(kind: &str, c: Pos2, s: f32, color: egui::Color32) -> Option<Vec<Shape>> {
    let stroke = Stroke::new(1.0, color);
    Some(match kind {
        "smart" => vec![Shape::circle_stroke(c, s * 0.8, stroke), Shape::circle_filled(c, s * 0.3, color)],
        "list" => (-1..=1).map(|i| Shape::line_segment([c + vec2(-s, i as f32 * s * 0.6), c + vec2(s, i as f32 * s * 0.6)], stroke)).collect(),
        "folder" => vec![
            Shape::rect_stroke(Rect::from_center_size(c + vec2(0.0, s * 0.15), vec2(s * 2.0, s * 1.4)), 1.0, stroke, egui::StrokeKind::Inside),
            Shape::line_segment([c + vec2(-s, -s * 0.75), c + vec2(-s * 0.2, -s * 0.75)], stroke),
        ],
        _ => return None,
    })
}

/// Draws the count, the present/expected badge or the checkmark at the row's right; returns
/// where the label must end.
fn badge(painter: &egui::Painter, tokens: &Tokens, item: &Item, rect: Rect, section: bool) -> f32 {
    let height = rect.height();
    // The Languages heading keeps its right end for the plus button.
    if section {
        return rect.right() - height;
    }
    let font = FontId::proportional(tokens.font_size * 0.85);
    let gap = f32::from(tokens.radius);
    let right = Pos2::new(rect.right() - gap * 2.0, rect.center().y);
    match (item.present, item.expected) {
        (Some(p), Some(e)) if p >= e => {
            let s = height * 0.18;
            let c = right - vec2(s * 1.5, 0.0);
            let points = vec![c + vec2(-s, 0.0), c + vec2(-s * 0.3, s * 0.8), c + vec2(s * 1.1, -s * 0.9)];
            painter.add(Shape::line(points, Stroke::new(1.5, tokens.text_dim)));
            c.x - s * 2.0
        }
        (Some(p), Some(e)) => {
            let galley = painter.layout_no_wrap(format!("{p}/{e}"), font, tokens.text_dim);
            let pill =
                Rect::from_min_size(right - vec2(galley.size().x + height * 0.5, height * 0.35), vec2(galley.size().x + height * 0.5, height * 0.7));
            painter.rect_filled(pill, height * 0.35, tokens.panel_darker);
            painter.galley(pill.center() - galley.size() / 2.0, galley, tokens.text_dim);
            pill.left() - gap
        }
        _ => {
            let galley = painter.layout_no_wrap(item.count.to_string(), font, tokens.text_dim);
            let at = right - vec2(galley.size().x, galley.size().y / 2.0);
            let left = at.x;
            painter.galley(at, galley, tokens.text_dim);
            left - gap
        }
    }
}

/// The `dialog.open` that edits a smart or list filter; each editor reads the stored filter by
/// its path.
fn editor(item: &Item) -> Option<(&'static str, Value)> {
    let path = path_value(&item.path);
    let dialog = match item.kind.as_str() {
        "smart" => "dialog.smartFilterEditor",
        "list" => "dialog.listFilterEditor",
        _ => return None,
    };
    Some(("dialog.open", json!({"dialog": dialog, "params": {"filter": path}})))
}

fn context_menu(ui: &mut egui::Ui, cx: &mut Cx, item: &Item) {
    let path = path_value(&item.path);
    if ui.add_enabled(item.lacks_glyphs(), egui::Button::new("Show Missing Glyphs…")).clicked() {
        cx.pending.push(("dialog.open", json!({"dialog": "dialog.missingGlyphs", "params": {"entry": path}})));
        ui.close();
    }
    if !matches!(item.kind.as_str(), "smart" | "list" | "folder") {
        return;
    }
    ui.separator();
    if let Some(open_editor) = editor(item)
        && ui.button("Edit Filter…").clicked()
    {
        cx.pending.push(open_editor);
        ui.close();
    }
    if ui.button("Remove Filter").clicked() {
        cx.pending.push(("fontView.removeFilter", json!({"filter": path})));
        ui.close();
    }
}

/// The filter the actions menu edits or removes: the last selected entry of the Filters section.
fn chosen_filter<'a>(items: &'a [Item], selected: &[Vec<String>]) -> Option<&'a Item> {
    let path = selected.iter().rev().find(|p| p.first().is_some_and(|s| s == "filters"))?;
    find_item(items, path, 0)
}

fn find_item<'a>(items: &'a [Item], path: &[String], depth: usize) -> Option<&'a Item> {
    if depth > MAX_DEPTH {
        return None;
    }
    items.iter().find_map(|i| if i.path == path { Some(i) } else { find_item(&i.children, path, depth + 1) })
}

/// The actions menu at the sidebar's foot.
fn actions(app: &FontcraftApp, ui: &mut egui::Ui, items: &[Item], selected: &[Vec<String>], pending: &mut Pending) {
    let settings = app.session.settings();
    let chosen = chosen_filter(items, selected);
    ui.menu_button("Actions", |ui| {
        if ui.button("Add Smart Filter…").clicked() {
            pending.push(("dialog.open", json!({"command": "fontView.addSmartFilter"})));
            ui.close();
        }
        if ui.button("Add List Filter…").clicked() {
            pending.push(("dialog.open", json!({"command": "fontView.addListFilter"})));
            ui.close();
        }
        if ui.button("Add Folder").clicked() {
            pending.push(("fontView.addFilterFolder", json!({"name": unique_filter_name(settings, "New Folder")})));
            ui.close();
        }
        ui.separator();
        let edit = chosen.and_then(editor);
        if ui.add_enabled(edit.is_some(), egui::Button::new("Edit Filter…")).clicked()
            && let Some(open_editor) = edit
        {
            pending.push(open_editor);
            ui.close();
        }
        if ui.add_enabled(chosen.is_some(), egui::Button::new("Remove Filter")).clicked()
            && let Some(item) = chosen
        {
            pending.push(("fontView.removeFilter", json!({"filter": path_value(&item.path)})));
            ui.close();
        }
        ui.separator();
        if ui.button("Edit Languages List…").clicked() {
            pending.push(("dialog.open", json!({"dialog": "dialog.languagesList"})));
            ui.close();
        }
    });
}
