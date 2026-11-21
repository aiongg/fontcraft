//! Edit View tabs: Font View is the fixed first tab of the document area's strip, Edit View tabs follow
//! (`tabs.windowHoldsFontViewPlusEditTabs`).
//!
//! The edit-view fields of [`ViewState`] (the shown glyph, the canvas view, the text, the master and the layers
//! picked, Edit Background, whether the Preview Area shows) and the canvas selection in [`FontCraftApp::tools`] are the state of one tab: the *live*
//! tab, which is the showing tab or, while Font View shows, the tab shown last. Every other tab keeps its own copy in
//! [`EditTab::state`]; showing a tab swaps the two ([`show`]). Projections, the canvas and the tools read the live
//! fields only.
//!
//! Tab commands name tabs by their position in the strip, counting from 1 for Font View, as Cmd+Alt+1 to Cmd+Alt+9
//! do.
//!
//! Each open document has its own tab set. [`follow_document`] keeps [`EditTabs`] on the active document's set: when
//! another document becomes active, the live tab's state goes back into its tab, the whole set is put away for its
//! document ([`EditTabs`]'s `others`), and the new document's set comes back with its live tab made live again. A
//! document shown for the first time opens with one Edit View tab; a closed document's set is dropped with it.

use std::collections::BTreeMap;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::Font;
use fontcraft_textlayout::{Item, MAX_ITEMS, Text};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use crate::app::{FontCraftApp, ViewState};
use crate::canvas::text::TabText;
use crate::canvas::tools::ParkedSelection;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most Edit View tabs open at once.
pub const MAX_TABS: usize = 1_000;
/// Most closed tabs kept for Reopen Last Closed Tab; the oldest is dropped first.
pub const MAX_CLOSED_TABS: usize = 32;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "view.newTab",
        label: "New Tab",
        menu: &["View"],
        shortcut: Some("Cmd+T"),
        params: "{glyphs?: [names], text?: string} open a new Edit View tab with these glyphs (default: the glyph the showing tab edits, or Font View's selection while Font View shows); `text` names them by their characters or as /name",
    },
    UiCommand {
        id: "view.showTab",
        label: "Show Tab",
        menu: &[],
        shortcut: None,
        params: "{index: integer} show the tab at this position (1 is Font View)",
    },
    UiCommand {
        id: "view.showNextTab",
        label: "Show Next Tab",
        menu: &["View", "Navigation"],
        shortcut: Some("Ctrl+Tab"),
        params: "{} show the tab to the right (after the last: Font View)",
    },
    UiCommand {
        id: "view.showPreviousTab",
        label: "Show Previous Tab",
        menu: &["View", "Navigation"],
        shortcut: Some("Ctrl+Shift+Tab"),
        params: "{} show the tab to the left (before Font View: the last tab)",
    },
    UiCommand {
        id: "view.closeTab",
        label: "Close Tab",
        menu: &["View"],
        shortcut: Some("Cmd+Shift+W"),
        params: "{index?: integer} close the tab at this position (default: the showing tab; Font View cannot be closed)",
    },
    UiCommand {
        id: "view.closeOtherTabs",
        label: "Close Other Tabs",
        menu: &[],
        shortcut: None,
        params: "{keep?: integer} close every Edit View tab but the one at this position (default: the showing tab)",
    },
    UiCommand {
        id: "view.reopenLastClosedTab",
        label: "Reopen Last Closed Tab",
        menu: &["View"],
        shortcut: Some("Cmd+Alt+Shift+W"),
        params: "{} bring back the most recently closed tab where it was, with its view and selection",
    },
];

/// What an Edit View tab shows, kept while another tab is live.
pub struct TabState {
    /// The shown glyph; `None` shows the font's first glyph.
    pub glyph: Option<String>,
    pub canvas: CanvasView,
    pub text: TabText,
    /// The master the tab draws in (`LayersView::master`).
    pub master: Option<String>,
    /// Glyphs shown in another layer than their layer of `master` (`LayersView::layers`).
    pub layers: BTreeMap<String, String>,
    /// The Layers panel's selected rows (`LayersView::selection`).
    pub layer_selection: Vec<String>,
    pub edit_background: bool,
    /// The Preview Area shows under the canvas (`PreviewState::area_open`); all tabs share its height.
    pub preview_area: bool,
    pub selection: ParkedSelection,
}

impl Default for TabState {
    fn default() -> Self {
        TabState {
            glyph: None,
            canvas: fresh_canvas(),
            text: TabText::default(),
            master: None,
            layers: BTreeMap::new(),
            layer_selection: Vec::new(),
            edit_background: false,
            preview_area: false,
            selection: ParkedSelection::default(),
        }
    }
}

/// A canvas view the canvas fits to the document the first time it draws.
fn fresh_canvas() -> CanvasView {
    CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)
}

/// One Edit View tab.
pub struct EditTab {
    /// Stable for the session, unlike the tab's position in the strip; from 1.
    pub id: u64,
    /// The tab's state while it is not the live tab; a placeholder while it is.
    pub state: TabState,
}

/// The tab set of an open document that is not active. The live tab's state is in its tab; `master` is Font View's
/// master when no tab was live.
struct DocumentTabs {
    tabs: Vec<EditTab>,
    active: Option<u64>,
    live: Option<u64>,
    closed: Vec<(usize, EditTab)>,
    master: Option<String>,
}

/// The Edit View tabs of the active document, and the tab sets of the other open documents.
pub struct EditTabs {
    /// In strip order, after Font View.
    pub tabs: Vec<EditTab>,
    /// The showing tab's id; `None` while Font View shows.
    pub active: Option<u64>,
    /// The tab whose state the edit-view fields of [`ViewState`] hold; `None` after it closed with Font View showing.
    live: Option<u64>,
    /// Closed tabs and the index each had, the last closed at the end.
    pub closed: Vec<(usize, EditTab)>,
    /// Tab ids count across documents, so an id names one tab in the window.
    next_id: u64,
    /// The document the tabs above belong to; `None` until a document shows.
    document: Option<u64>,
    /// The tab sets of the other open documents, by document id.
    others: BTreeMap<u64, DocumentTabs>,
}

impl Default for EditTabs {
    /// A window opens with one Edit View tab, showing the font's first glyph.
    fn default() -> Self {
        EditTabs {
            tabs: vec![EditTab { id: 1, state: TabState::default() }],
            active: Some(1),
            live: Some(1),
            closed: Vec::new(),
            next_id: 2,
            document: None,
            others: BTreeMap::new(),
        }
    }
}

impl EditTabs {
    /// Whether Font View is the showing tab.
    pub fn font_view_shows(&self) -> bool {
        self.active.is_none()
    }

    /// The index into [`EditTabs::tabs`] of the tab `id`.
    pub fn index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The tab whose state the edit-view fields hold.
    pub fn live(&self) -> Option<u64> {
        self.live
    }

    /// The showing tab's position: 1 for Font View.
    pub fn position(&self) -> usize {
        self.active.and_then(|id| self.index(id)).map_or(1, |i| i + 2)
    }

    /// How many positions the strip has: Font View and every Edit View tab.
    pub fn positions(&self) -> usize {
        self.tabs.len() + 1
    }

    /// The id of the tab at `position`; `None` for Font View.
    fn id_at(&self, position: usize) -> Option<u64> {
        position.checked_sub(2).and_then(|i| self.tabs.get(i)).map(|t| t.id)
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = command.params.as_object().cloned().unwrap_or_default();
    let positions = app.view.tabs.positions();
    let current = app.view.tabs.position();
    match command.id.as_str() {
        "view.newTab" => new_tab(app, &p),
        "view.showTab" => {
            let at = position(&p, "index", positions)?.ok_or("`index` (a tab position, 1 for Font View) is required")?;
            let to = app.view.tabs.id_at(at);
            show(app, to);
            Ok(())
        }
        "view.showNextTab" | "view.showPreviousTab" => {
            // `current` is at least 1 and at most `positions`, so neither sum underflows.
            let next = if command.id == "view.showNextTab" { current % positions + 1 } else { (current + positions - 2) % positions + 1 };
            let to = app.view.tabs.id_at(next);
            show(app, to);
            Ok(())
        }
        "view.closeTab" => {
            let at = position(&p, "index", positions)?.unwrap_or(current);
            let index = at.checked_sub(2).ok_or("Font View cannot be closed")?;
            close(app, index);
            Ok(())
        }
        "view.closeOtherTabs" => {
            let keep = position(&p, "keep", positions)?.unwrap_or(current);
            let keep = app.view.tabs.id_at(keep);
            show(app, keep);
            let mut at = app.view.tabs.tabs.len();
            while at > 0 {
                at -= 1;
                if app.view.tabs.tabs.get(at).map(|t| t.id) != keep {
                    close(app, at);
                }
            }
            Ok(())
        }
        "view.reopenLastClosedTab" => {
            let tabs = &mut app.view.tabs;
            let (at, tab) = tabs.closed.pop().ok_or("no tab was closed")?;
            if tabs.tabs.len() >= MAX_TABS {
                tabs.closed.push((at, tab));
                return Err(format!("at most {MAX_TABS} tabs can be open"));
            }
            let id = tab.id;
            let at = at.min(tabs.tabs.len());
            tabs.tabs.insert(at, tab);
            show(app, Some(id));
            Ok(())
        }
        other => Err(format!("no tab command `{other}`")),
    }
}

/// A tab position from the parameter `key`: 1 to `positions`.
fn position(p: &Map<String, Value>, key: &str, positions: usize) -> Result<Option<usize>, String> {
    let Some(value) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let n = value.as_u64().ok_or_else(|| format!("`{key}` must be a tab position (1 is Font View)"))?;
    usize::try_from(n)
        .ok()
        .filter(|i| (1..=positions).contains(i))
        .map(Some)
        .ok_or_else(|| format!("there is no tab at position {n} (1 is Font View; {positions} positions now)"))
}

/// `view.newTab`: the glyphs the parameters name, else the showing tab's edited glyph, else Font View's selection.
fn new_tab(app: &mut FontCraftApp, p: &Map<String, Value>) -> Result<(), String> {
    let text = {
        let font = app.session.font().map_err(|e| e.to_string())?;
        let items = match crate::canvas::text::commands::items(font, p)? {
            Some(items) => items,
            None => {
                let names: Vec<String> = if app.view.tabs.font_view_shows() {
                    app.view.font_view.selected.iter().take(MAX_ITEMS).cloned().collect()
                } else {
                    crate::canvas::tools::edited_glyph(font, &app.view).map(str::to_string).into_iter().collect()
                };
                names.iter().map(|name| crate::canvas::text::commands::glyph_item(font, name)).collect()
            }
        };
        if !items.iter().any(|item| matches!(item, Item::Glyph { .. })) {
            return Err("no glyph is selected".into());
        }
        TabText::of(items)?
    };
    open(app, text)
}

/// Opens `glyphs` for editing (Font View's double-click, Return and Cmd+Down): an open tab whose text is exactly
/// these glyphs shows, else a new tab opens with them.
pub(crate) fn edit_glyphs(app: &mut FontCraftApp, glyphs: &[String]) -> Result<(), String> {
    let text = glyph_text(app, glyphs)?;
    let view = &app.view;
    let holding = view.tabs.tabs.iter().find(|t| holds(if view.tabs.live == Some(t.id) { &view.text.text } else { &t.state.text.text }, glyphs));
    match holding.map(|t| t.id) {
        Some(id) => {
            show(app, Some(id));
            Ok(())
        }
        None => open(app, text),
    }
}

/// Opens `glyphs` in a new tab (`url.show`).
pub(crate) fn open_glyphs(app: &mut FontCraftApp, glyphs: &[String]) -> Result<(), String> {
    let text = glyph_text(app, glyphs)?;
    open(app, text)
}

/// A tab text of `glyphs`, each a glyph of the font.
fn glyph_text(app: &FontCraftApp, glyphs: &[String]) -> Result<TabText, String> {
    if glyphs.is_empty() {
        return Err("no glyph is selected".into());
    }
    if glyphs.len() > MAX_ITEMS {
        return Err(format!("at most {MAX_ITEMS} glyphs open in one tab"));
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    if let Some(missing) = glyphs.iter().find(|g| font.glyph(g).is_none()) {
        return Err(format!("no glyph named `{missing}`"));
    }
    TabText::of(glyphs.iter().map(|g| crate::canvas::text::commands::glyph_item(font, g)).collect())
}

/// Shows `glyph` in the edit view: in the showing tab, or while Font View shows, in the tab shown last; in a new tab
/// when none is open.
pub(crate) fn show_glyph(app: &mut FontCraftApp, glyph: String) -> Result<(), String> {
    match app.view.tabs.active.or(app.view.tabs.live) {
        Some(id) => {
            show(app, Some(id));
            app.view.glyph = Some(glyph);
            Ok(())
        }
        None => edit_glyphs(app, &[glyph]),
    }
}

/// Whether `text` is exactly `glyphs`, with nothing else.
fn holds(text: &Text, glyphs: &[String]) -> bool {
    text.len() == glyphs.len() && text.items().iter().zip(glyphs).all(|(item, g)| matches!(item, Item::Glyph { name, .. } if **name == **g))
}

/// Adds a tab with `text` at the end of the strip and shows it. It draws in the master of the tab it was opened from.
fn open(app: &mut FontCraftApp, text: TabText) -> Result<(), String> {
    if app.view.tabs.tabs.len() >= MAX_TABS {
        return Err(format!("at most {MAX_TABS} tabs can be open"));
    }
    let glyph = text.text.current_glyph().map(str::to_string).or_else(|| {
        text.text.items().iter().find_map(|item| match item {
            Item::Glyph { name, .. } => Some(name.to_string()),
            _ => None,
        })
    });
    let state = TabState { glyph, text, master: app.view.layers.master.clone(), ..TabState::default() };
    let tabs = &mut app.view.tabs;
    let id = tabs.next_id;
    tabs.next_id = tabs.next_id.saturating_add(1);
    tabs.tabs.push(EditTab { id, state });
    show(app, Some(id));
    Ok(())
}

/// Shows the tab `to` (`None`: Font View).
pub(crate) fn show(app: &mut FontCraftApp, to: Option<u64>) {
    let to = to.filter(|id| app.view.tabs.index(*id).is_some());
    if let Some(id) = to {
        make_live(app, id);
    }
    app.view.tabs.active = to;
    app.view_changed();
}

/// Makes tab `id`'s state the live one; the state live until now goes back to its tab.
fn make_live(app: &mut FontCraftApp, id: u64) {
    if app.view.tabs.live == Some(id) {
        return;
    }
    let Some(incoming) = app.view.tabs.tabs.iter_mut().find(|t| t.id == id).map(|t| std::mem::take(&mut t.state)) else { return };
    let outgoing = take_live(app);
    let live = app.view.tabs.live;
    if let Some(tab) = app.view.tabs.tabs.iter_mut().find(|t| Some(t.id) == live) {
        tab.state = outgoing;
    }
    put_live(app, incoming);
    app.view.tabs.live = Some(id);
}

/// Keeps the tabs on the active document's set (module doc): the set shown until now goes back to its document while
/// that document is open and is dropped with it otherwise. Returns whether another document's set shows now.
pub(crate) fn follow_document(app: &mut FontCraftApp) -> bool {
    let active = app.session.active().map(|d| d.id);
    let open: Vec<u64> = app.session.documents().iter().map(|d| d.id).collect();
    app.view.tabs.others.retain(|id, _| open.contains(id));
    let shown = app.view.tabs.document;
    match (shown, active) {
        (Some(shown), Some(now)) if shown == now => return false,
        // The window's first tab set becomes the first document's.
        (None, _) => {
            app.view.tabs.document = active;
            return false;
        }
        _ => {}
    }
    // The live tab's state goes back into its tab, so the set is whole while it is put away.
    let state = take_live(app);
    let tabs = &mut app.view.tabs;
    let live = tabs.live.and_then(|id| tabs.tabs.iter_mut().find(|t| t.id == id));
    let master = match live {
        Some(tab) => {
            tab.state = state;
            None
        }
        None => state.master,
    };
    let kept = DocumentTabs {
        tabs: std::mem::take(&mut tabs.tabs),
        active: tabs.active.take(),
        live: tabs.live.take(),
        closed: std::mem::take(&mut tabs.closed),
        master,
    };
    if let Some(shown) = shown
        && open.contains(&shown)
    {
        tabs.others.insert(shown, kept);
    }
    let back = active.and_then(|now| tabs.others.remove(&now)).unwrap_or_else(|| {
        let id = tabs.next_id;
        tabs.next_id = tabs.next_id.saturating_add(1);
        DocumentTabs { tabs: vec![EditTab { id, state: TabState::default() }], active: Some(id), live: Some(id), closed: Vec::new(), master: None }
    });
    tabs.tabs = back.tabs;
    tabs.active = back.active.filter(|id| tabs.tabs.iter().any(|t| t.id == *id));
    tabs.closed = back.closed;
    tabs.document = active;
    let incoming = back.live.and_then(|id| tabs.tabs.iter_mut().find(|t| t.id == id)).map(|t| (t.id, std::mem::take(&mut t.state)));
    match incoming {
        Some((id, state)) => {
            tabs.live = Some(id);
            put_live(app, state);
        }
        None => {
            put_live(app, TabState::default());
            app.view.layers.master = back.master;
        }
    }
    app.view_changed();
    true
}

/// Takes the edit-view fields out of the view state, leaving fresh ones.
fn take_live(app: &mut FontCraftApp) -> TabState {
    let view = &mut app.view;
    TabState {
        glyph: view.glyph.take(),
        canvas: std::mem::replace(&mut view.canvas, fresh_canvas()),
        text: view.text.park(),
        master: view.layers.master.take(),
        layers: std::mem::take(&mut view.layers.layers),
        layer_selection: std::mem::take(&mut view.layers.selection),
        edit_background: std::mem::take(&mut view.edit_background),
        preview_area: std::mem::take(&mut view.preview.area_open),
        selection: app.tools.park(),
    }
}

fn put_live(app: &mut FontCraftApp, state: TabState) {
    let view = &mut app.view;
    view.glyph = state.glyph;
    view.canvas = state.canvas;
    view.text.restore(state.text);
    view.layers.master = state.master;
    view.layers.layers = state.layers;
    view.layers.selection = state.layer_selection;
    view.edit_background = state.edit_background;
    view.preview.area_open = state.preview_area;
    app.tools.restore(state.selection);
}

/// Closes the tab at index `at` of [`EditTabs::tabs`] and keeps it for Reopen Last Closed Tab. When it was showing,
/// the tab to its left shows, or Font View when it was the first.
fn close(app: &mut FontCraftApp, at: usize) {
    let Some(id) = app.view.tabs.tabs.get(at).map(|t| t.id) else { return };
    // Font View keeps drawing in the master of the tab shown last.
    let mut master = app.view.layers.master.clone();
    if app.view.tabs.live == Some(id) {
        let state = take_live(app);
        master = state.master.clone();
        app.view.tabs.live = None;
        if let Some(tab) = app.view.tabs.tabs.get_mut(at) {
            tab.state = state;
        }
    }
    let tabs = &mut app.view.tabs;
    if at >= tabs.tabs.len() {
        return;
    }
    let tab = tabs.tabs.remove(at);
    tabs.closed.push((at, tab));
    if tabs.closed.len() > MAX_CLOSED_TABS {
        tabs.closed.remove(0);
    }
    if tabs.active == Some(id) {
        let next = at.checked_sub(1).and_then(|i| tabs.tabs.get(i)).map(|t| t.id);
        show(app, next);
    }
    if app.view.tabs.live.is_none() {
        app.view.layers.master = master;
    }
    app.view_changed();
}

/// The glyphs tab `tab` shows, in order.
pub fn tab_glyphs(font: Option<&Font>, view: &ViewState, tab: &EditTab) -> Vec<String> {
    if view.tabs.live == Some(tab.id) {
        let shown = font.and_then(|f| crate::canvas::tools::edited_glyph(f, view)).map(str::to_string).or_else(|| view.glyph.clone());
        return crate::canvas::text::shown_glyphs(&view.text.text, shown.as_deref());
    }
    let shown = parked_glyph(font, &tab.state);
    tab.state.text.shown_glyphs(shown.as_deref())
}

/// The glyph a tab that is not live shows: its own while the font has it, else the font's first.
fn parked_glyph(font: Option<&Font>, state: &TabState) -> Option<String> {
    let Some(font) = font else { return state.glyph.clone() };
    match &state.glyph {
        Some(name) if font.glyphs.get(name).is_some() => Some(name.clone()),
        _ => font.glyphs.first().map(|g| g.name.to_string()),
    }
}

/// `ui.inspect`'s `tabs` (each `{id, position, glyphs, edited, master, zoom, center, selection}`), `activeTab` (an
/// index into `tabs`; `null` while Font View shows) and `closedTabs` (how many Reopen Last Closed Tab can bring back).
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let font = app.session.font().ok();
    let view = &app.view;
    let master_of = |id: Option<&String>| -> Option<String> {
        let font = font?;
        id.and_then(|id| font.masters.iter().find(|m| &m.id == id)).or_else(|| font.masters.first()).map(|m| m.id.clone())
    };
    let tabs: Vec<Value> = view
        .tabs
        .tabs
        .iter()
        .zip(2usize..)
        .map(|(tab, position)| {
            let live = view.tabs.live == Some(tab.id);
            let (edited, canvas, master, selection) = if live {
                let edited = font.and_then(|f| crate::canvas::tools::edited_glyph(f, view)).map(str::to_string);
                (edited, &view.canvas, master_of(view.layers.master.as_ref()), serde_json::to_value(&app.tools.selection))
            } else {
                let s = &tab.state;
                (parked_glyph(font, s), &s.canvas, master_of(s.master.as_ref()), serde_json::to_value(s.selection.selection()))
            };
            json!({
                "id": tab.id,
                "position": position,
                "glyphs": tab_glyphs(font, view, tab),
                "edited": edited,
                "master": master,
                "zoom": canvas.zoom,
                "center": [canvas.center.x, canvas.center.y],
                "selection": selection.unwrap_or(Value::Null),
            })
        })
        .collect();
    state.insert("tabs".into(), Value::Array(tabs));
    state.insert("activeTab".into(), json!(view.tabs.active.and_then(|id| view.tabs.index(id))));
    state.insert("closedTabs".into(), json!(view.tabs.closed.len()));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
