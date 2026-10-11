//! Find Glyphs (`editView.findGlyphs`): the pop-up of Edit › Find › Find… that searches the
//! font with `glyph.search` while the person types and inserts the chosen glyphs into the text
//! of an Edit View tab at its cursor (`editView.insertText`).
//!
//! | In the pop-up | Does |
//! |---|---|
//! | typing in the field | searches again; the first result is selected |
//! | the mode menu | searches by Name, Unicode or All |
//! | click | selects the result alone |
//! | Shift+click | selects the results from the last clicked one to this one |
//! | Cmd+click | adds the result to the selection or takes it out |
//! | Up, Down | selects the result above or below |
//! | Return | inserts the selected results, in list order, and closes the pop-up |
//! | Escape | closes the pop-up |
//!
//! In Font View, `edit.find` gives the keyboard to Font View's search field instead.

use egui::{Modifiers, Rect, Sense, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

type Pending = Vec<(&'static str, Value)>;

/// Longest query, in characters: four bytes each stay within the search's byte limit.
const MAX_QUERY_CHARS: usize = 64;
/// Most results one search lists (the search's own limit).
const MAX_RESULTS: usize = 1000;
/// Results the pop-up shows before it scrolls.
const VISIBLE_ROWS: f32 = 14.0;
/// Width of the pop-up, in points.
const WIDTH: f32 = 320.0;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// The search modes, as `glyph.search` names them, with their menu labels.
const MODES: [(&str, &str); 3] = [("name", "Name"), ("unicode", "Unicode"), ("all", "All")];

pub const COMMANDS: &[UiCommand] = &[(
    "edit.find",
    "Find…",
    &["Edit", "Find"],
    "Cmd+F",
    "{tab?: id, query?: string, mode?: \"name\" | \"unicode\" | \"all\", select?: [names], insert?: bool, open?: bool (default true)} in Edit View, show the Find Glyphs pop-up for the tab and search the font as glyph.search does (query and mode left out keep their value; a new search selects its first result); select picks results; insert puts the selected results, in list order, into the tab's text at the cursor and closes the pop-up. In Font View, give the keyboard to Font View's search field → {open, tab, query, mode, results: [names], total, selected} | with insert, the tab's text as editView.getText gives it, with inserted: [names]",
)];

/// One result as the pop-up lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub name: String,
    /// The first code point in hexadecimal, or empty.
    pub unicode: String,
}

/// What a search was run for: the query, the mode and the font it read.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    query: String,
    mode: String,
    /// The document's id and revision: every change, and every other document, gives a new one.
    font: Option<(u64, u64)>,
}

/// The pop-up's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FindGlyphsState {
    pub open: bool,
    /// The tab the results go into.
    pub tab: Option<u64>,
    pub query: String,
    /// `name`, `unicode` or `all`; empty is `name`.
    pub mode: String,
    /// Selected results by name, in the order they were picked.
    pub selected: Vec<String>,
    /// The result Shift+click extends from.
    pub anchor: Option<String>,
    #[serde(skip)]
    key: Option<Key>,
    #[serde(skip)]
    pub results: Vec<Found>,
    /// How many glyphs matched, also beyond the listed ones.
    #[serde(skip)]
    pub total: usize,
    /// Why the last search was refused.
    #[serde(skip)]
    pub error: Option<String>,
    /// The pop-up opened this frame: the field takes the keyboard.
    #[serde(skip)]
    pub fresh: bool,
    /// The results laid out in the last frame with their rectangles: tests click them.
    #[serde(skip)]
    pub laid_out: Vec<(String, Rect)>,
}

impl FindGlyphsState {
    pub fn mode(&self) -> &str {
        if self.mode.is_empty() { "name" } else { &self.mode }
    }

    fn contains(&self, name: &str) -> bool {
        self.results.iter().any(|r| r.name == name)
    }

    /// The selected results in list order.
    pub fn selected_in_order(&self) -> Vec<String> {
        self.results.iter().filter(|r| self.selected.contains(&r.name)).map(|r| r.name.clone()).collect()
    }
}

/// The id of the search field, for focus.
pub fn field_id() -> egui::Id {
    egui::Id::new("editView.findGlyphs.query")
}

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn key(app: &FontcraftApp, query: &str, mode: &str) -> Key {
    let font = app.session.active().map(|d| (d.id, d.revision));
    Key { query: query.to_string(), mode: mode.to_string(), font }
}

/// Runs `glyph.search` for `key`: the results, how many matched, and the refusal.
fn search(app: &mut FontcraftApp, key: &Key) -> (Vec<Found>, usize, Option<String>) {
    let params = json!({ "query": key.query, "mode": key.mode, "limit": MAX_RESULTS });
    match app.session.execute("glyph.search", &params) {
        Ok(v) => {
            let found = v
                .get("results")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|r| {
                            let name = r.get("name").and_then(Value::as_str)?.to_string();
                            let unicode = r.pointer("/unicodes/0").and_then(Value::as_str).unwrap_or_default().to_string();
                            Some(Found { name, unicode })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let total = v.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or_default();
            (found, total, None)
        }
        Err(e) => (Vec::new(), 0, Some(e.to_string())),
    }
}

/// Searches again when the query, the mode or the font changed; a new search selects its first
/// result.
pub fn refresh(app: &mut FontcraftApp) {
    let st = &app.state.panels.find_glyphs;
    let k = key(app, &st.query, st.mode());
    if st.key.as_ref() == Some(&k) {
        return;
    }
    let (results, total, error) = search(app, &k);
    let st = &mut app.state.panels.find_glyphs;
    st.selected = results.first().map(|r| vec![r.name.clone()]).unwrap_or_default();
    st.anchor = st.selected.first().cloned();
    (st.results, st.total, st.error, st.key) = (results, total, error, Some(k));
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "edit.find" {
        return Err(args.err("unknown command"));
    }
    if args.get("tab").is_none() && app.state.tab().is_none() {
        return app.run("fontView.focusSearch", &json!({}));
    }
    let index = crate::shell::view::target_tab(app, args)?;
    let tab = app.state.tabs.get(index).map(|t| t.id).ok_or_else(|| args.err("no such tab"))?;
    let was_open = app.state.panels.find_glyphs.open;
    let query = match args.string("query")? {
        Some(q) if q.chars().count() > MAX_QUERY_CHARS => return Err(args.err(format!("`query` holds at most {MAX_QUERY_CHARS} characters"))),
        Some(q) => q.to_string(),
        // A pop-up that opens starts with an empty field.
        None if !was_open => String::new(),
        None => app.state.panels.find_glyphs.query.clone(),
    };
    let mode = match args.string("mode")? {
        Some(m) => MODES
            .iter()
            .find(|(id, _)| *id == m)
            .map(|(id, _)| (*id).to_string())
            .ok_or_else(|| args.err(format!("`mode` must be name, unicode or all, got \"{}\"", quoted(m))))?,
        None => app.state.panels.find_glyphs.mode().to_string(),
    };
    let select = args.strings("select", MAX_RESULTS)?;
    let insert = args.flag("insert")?.unwrap_or(false);
    let open = args.flag("open")?.unwrap_or(true);
    // Everything is checked on a copy before anything changes.
    let mut next = app.state.panels.find_glyphs.clone();
    let k = key(app, &query, &mode);
    if next.key.as_ref() != Some(&k) {
        let (results, total, error) = search(app, &k);
        if let Some(e) = error {
            return Err(args.err(e));
        }
        next.selected = results.first().map(|r| vec![r.name.clone()]).unwrap_or_default();
        next.anchor = next.selected.first().cloned();
        (next.results, next.total, next.error, next.key) = (results, total, None, Some(k));
    }
    if let Some(names) = select {
        if let Some(bad) = names.iter().find(|n| !next.contains(n)) {
            return Err(args.err(format!("`{}` is not among the results", quoted(bad))));
        }
        next.anchor = names.last().cloned();
        next.selected = names;
    }
    (next.query, next.mode, next.tab) = (query, mode, Some(tab));
    if !was_open && open {
        next.fresh = true;
    }
    if insert {
        let names = next.selected_in_order();
        if names.is_empty() {
            return Err(args.err("no result is selected"));
        }
        let mut reply = app.run("editView.insertText", &json!({ "tab": tab, "glyphs": names }))?;
        next.open = false;
        app.state.panels.find_glyphs = next;
        if let Some(map) = reply.as_object_mut() {
            map.insert("inserted".into(), json!(names));
        }
        return Ok(reply);
    }
    next.open = open;
    let st = &mut app.state.panels.find_glyphs;
    *st = next;
    Ok(json!({
        "open": st.open,
        "tab": tab,
        "query": st.query,
        "mode": st.mode(),
        "results": st.results.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        "total": st.total,
        "selected": st.selected_in_order(),
    }))
}

/// What the person did in the pop-up this frame.
enum Event {
    Query(String),
    Mode(&'static str),
    Click(String, Modifiers),
    Step(bool),
    Insert,
    Close,
}

/// Draws the pop-up while it is open and its tab is showing. Return queues the insertion in
/// `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let showing = app.state.tab().map(|t| t.id);
    let st = &app.state.panels.find_glyphs;
    if !st.open || st.tab.is_none() || st.tab != showing {
        return;
    }
    refresh(app);
    let FontcraftApp { state, tokens, .. } = app;
    let st = &mut state.panels.find_glyphs;
    let mods = ui.input(|i| i.modifiers);
    let mut events = Vec::new();
    let mut laid_out = Vec::new();
    let mut open = true;
    egui::Window::new("Find Glyphs")
        .id(egui::Id::new("editView.findGlyphs"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(WIDTH)
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                let label = MODES.iter().find(|(id, _)| *id == st.mode()).map_or("Name", |(_, l)| *l);
                ui.menu_button(label, |ui| {
                    for (id, l) in MODES {
                        if ui.selectable_label(st.mode() == id, l).clicked() {
                            events.push(Event::Mode(id));
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Search by glyph name, by Unicode value, or by both");
                let mut query = st.query.clone();
                let field = egui::TextEdit::singleline(&mut query)
                    .id(field_id())
                    .hint_text("Name, character or code point")
                    .char_limit(MAX_QUERY_CHARS)
                    .desired_width(ui.available_width());
                let response = ui.add(field);
                // Up and Down step through the results; without this they move the focus.
                let filter = egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() };
                ui.memory_mut(|m| m.set_focus_lock_filter(field_id(), filter));
                if st.fresh {
                    response.request_focus();
                    st.fresh = false;
                }
                if response.changed() {
                    events.push(Event::Query(query));
                }
                let (enter, escape, up, down) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                    )
                });
                let focused = response.has_focus() || response.lost_focus();
                if focused && enter {
                    events.push(Event::Insert);
                } else if focused && escape {
                    events.push(Event::Close);
                } else if response.has_focus() && (up || down) {
                    events.push(Event::Step(down));
                }
            });
            ui.add_space(tokens.dialog_spacing);
            let height = tokens.list_row_height;
            egui::ScrollArea::vertical().id_salt("findGlyphs.v").max_height(height * VISIBLE_ROWS).auto_shrink([false, true]).show_rows(
                ui,
                height,
                st.results.len(),
                |ui, range| {
                    for at in range {
                        let Some(row) = st.results.get(at) else { continue };
                        let selected = st.selected.contains(&row.name);
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
                        laid_out.push((row.name.clone(), rect));
                        let fill = if selected { tokens.row_selected } else { tokens.panel };
                        ui.painter_at(rect).rect_filled(rect, egui::CornerRadius::ZERO, fill);
                        let color = if selected { tokens.selected_text } else { tokens.text };
                        let inner = rect.shrink2(vec2(tokens.cell_inset, 0.0));
                        let font = egui::FontId::proportional(tokens.font_size);
                        ui.painter_at(inner).text(inner.left_center(), egui::Align2::LEFT_CENTER, &row.name, font.clone(), color);
                        if !row.unicode.is_empty() {
                            let dim = if selected { tokens.selected_text } else { tokens.text_dim };
                            ui.painter_at(inner).text(inner.right_center(), egui::Align2::RIGHT_CENTER, &row.unicode, font, dim);
                        }
                        if response.clicked() {
                            events.push(Event::Click(row.name.clone(), mods));
                        }
                    }
                },
            );
            let note = match &st.error {
                Some(e) => e.clone(),
                None if st.query.trim().is_empty() => String::new(),
                None if st.total > st.results.len() => format!("The first {} of {} matches", st.results.len(), st.total),
                None => format!("{} matches", st.total),
            };
            if !note.is_empty() {
                let color = if st.error.is_some() { tokens.error_text } else { tokens.text_dim };
                ui.colored_label(color, note);
            }
        });
    if !open {
        events.push(Event::Close);
    }
    let order: Vec<String> = st.results.iter().map(|r| r.name.clone()).collect();
    for e in events {
        match e {
            Event::Query(q) => st.query = q,
            Event::Mode(m) => st.mode = m.to_string(),
            Event::Click(name, mods) => {
                let order: Vec<&str> = order.iter().map(String::as_str).collect();
                crate::shell::glyph_info::click(&mut st.selected, &mut st.anchor, &order, &name, mods);
                ui.ctx().memory_mut(|m| m.request_focus(field_id()));
            }
            Event::Step(down) => {
                let at = st.selected.last().and_then(|s| order.iter().position(|n| n == s));
                let to = match (at, down) {
                    (Some(i), true) => (i + 1).min(order.len().saturating_sub(1)),
                    (Some(i), false) => i.saturating_sub(1),
                    (None, _) => 0,
                };
                if let Some(name) = order.get(to) {
                    st.selected = vec![name.clone()];
                    st.anchor = Some(name.clone());
                }
            }
            Event::Insert => {
                if let Some(tab) = st.tab {
                    pending.push(("edit.find", json!({ "tab": tab, "insert": true })));
                }
            }
            Event::Close => st.open = false,
        }
    }
    st.laid_out = laid_out;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_document_opened_after_another_closed_is_searched_again() {
        let mut app = FontcraftApp::default();
        let mut font = fontcraft_model::Font::new("First");
        font.add_glyph("zzz", vec![], 500.0).unwrap();
        app.session.add_document(font, None).unwrap();
        app.state.panels.find_glyphs.query = "zzz".into();
        refresh(&mut app);
        assert_eq!(app.state.panels.find_glyphs.total, 1);
        // The next font is often allocated where the closed one was.
        app.session.execute("file.close", &json!({"discard": true})).unwrap();
        app.session.add_document(fontcraft_model::Font::new("Second"), None).unwrap();
        refresh(&mut app);
        assert_eq!(app.state.panels.find_glyphs.total, 0, "the second document has no zzz");
    }
}
