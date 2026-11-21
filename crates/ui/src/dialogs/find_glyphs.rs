//! Find Glyphs (`editView.findGlyphs`): the pop-up of Edit › Find › Find… (`edit.find`) that searches the font with
//! `glyph.search` while the person types and inserts the chosen glyphs into the edit view's text at its cursor
//! (`editView.insertText`).
//!
//! | In the pop-up | Does |
//! |---|---|
//! | typing in the field | searches again; the first result is selected |
//! | the mode menu | searches by Name, Unicode or All |
//! | click, Shift+click, Cmd+click | selects one result, a range, or adds and removes one |
//! | Insert, Return | inserts the selected results, in list order, and closes the pop-up |
//! | Cancel, Escape | closes the pop-up |

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::contract::{Bind, ColumnSpec, Entry, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};

craft::ui::words! {
    /// The words of the Find Glyphs pop-up.
    pub struct FindGlyphsWords {
        title: "fontcraftFindGlyphs.title" = "Find Glyphs";
        insert: "fontcraftFindGlyphs.insert" = "Insert";
        search: "fontcraftFindGlyphs.search" = "Search";
        mode: "fontcraftFindGlyphs.mode" = "Search by";
        by_name: "fontcraftFindGlyphs.name" = "Name";
        by_unicode: "fontcraftFindGlyphs.unicode" = "Unicode";
        by_all: "fontcraftFindGlyphs.all" = "All";
        results: "fontcraftFindGlyphs.results" = "Results";
        glyph: "fontcraftFindGlyphs.glyph" = "Glyph";
        code: "fontcraftFindGlyphs.code" = "Unicode";
        nothing: "fontcraftFindGlyphs.nothing" = "No glyph matches.";
    }
}

/// Longest query, in characters: four bytes each stay within the search's byte limit.
const MAX_QUERY_CHARS: usize = 64;
/// Most results one search lists (the search's own limit).
const MAX_RESULTS: usize = 1000;
/// Rows the list shows before it scrolls: the dialog body scrolls too, so the list must be capped.
const LIST_ROWS: u16 = 14;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// The search modes, as `glyph.search` names them.
const MODES: [&str; 3] = ["name", "unicode", "all"];

const NAME: ColumnKey = ColumnKey::new("name");
const CODE: ColumnKey = ColumnKey::new("unicode");

/// One result as the pop-up lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    /// The first code point in hexadecimal, or empty.
    pub unicode: String,
}

/// What a search was run for: the query, the mode and the document's id and revision.
type SearchKey = (String, String, Option<(u64, u64)>);

/// The pop-up's state, part of the edit view's text state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindGlyphs {
    pub open: bool,
    pub query: String,
    /// `name`, `unicode` or `all`; empty is `name`.
    pub mode: String,
    /// Selected results by name.
    pub selected: Vec<String>,
    pub results: Vec<Found>,
    /// How many glyphs matched, also beyond the listed ones.
    pub total: usize,
    key: Option<SearchKey>,
}

impl FindGlyphs {
    pub fn mode(&self) -> &str {
        if self.mode.is_empty() { "name" } else { &self.mode }
    }

    /// The selected results in list order.
    pub fn selected_in_order(&self) -> Vec<String> {
        self.results.iter().filter(|r| self.selected.contains(&r.name)).map(|r| r.name.clone()).collect()
    }

    /// Searches when the query, the mode or the font changed; a new search selects its first result.
    fn refresh(&mut self, session: &mut Session) -> Result<(), String> {
        let key = (self.query.clone(), self.mode().to_string(), session.active().map(|d| (d.id, d.revision)));
        if self.key.as_ref() == Some(&key) {
            return Ok(());
        }
        let found = session.query("glyph.search", &json!({"query": key.0, "mode": key.1, "limit": MAX_RESULTS})).map_err(|e| e.to_string())?;
        self.results = found
            .get("results")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .take(MAX_RESULTS)
                    .filter_map(|r| {
                        let name = r.get("name").and_then(Value::as_str)?.to_string();
                        let unicode = r.pointer("/unicodes/0").and_then(Value::as_str).unwrap_or_default().to_string();
                        Some(Found { name, unicode })
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.total = found.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or_default();
        self.selected = self.results.first().map(|r| vec![r.name.clone()]).unwrap_or_default();
        self.key = Some(key);
        Ok(())
    }
}

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "edit.find",
    label: "Find…",
    menu: &["Edit", "Find"],
    shortcut: Some("Cmd+F"),
    params: "{query?: string, mode?: \"name\" | \"unicode\" | \"all\", select?: [names], insert?: bool, open?: bool (default true)} show the Find Glyphs pop-up and search the font as glyph.search does (query and mode left out keep their value; a pop-up that opens starts with an empty query; a new search selects its first result); select picks results; insert puts the selected results, in list order, into the edit view's text at the cursor and closes the pop-up. Without parameters while Font View shows, it gives the keyboard to Font View's search field instead (fontView.focusSearch)",
}];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let empty = Map::new();
    let p = command.params.as_object().unwrap_or(&empty);
    if p.is_empty() && app.font_view_shows() {
        // In Font View, Find searches the glyphs shown there.
        return crate::shell::font_view::focus_search(app);
    }
    let string = |key: &str| -> Result<Option<&str>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    };
    let flag = |key: &str| -> Result<Option<bool>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("`{key}` must be true or false")),
        }
    };
    // Everything is worked out on a copy before anything changes.
    let mut next = app.view.text.find.clone();
    let was_open = next.open;
    next.query = match string("query")? {
        Some(q) if q.chars().count() > MAX_QUERY_CHARS => return Err(format!("`query` holds at most {MAX_QUERY_CHARS} characters")),
        Some(q) => q.to_string(),
        None if !was_open => String::new(),
        None => next.query,
    };
    if let Some(m) = string("mode")? {
        let found = MODES.iter().find(|id| **id == m).ok_or_else(|| format!("`mode` must be name, unicode or all, got \"{}\"", quoted(m)))?;
        next.mode = (*found).to_string();
    }
    next.refresh(&mut app.session)?;
    match p.get("select") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            let mut names = Vec::new();
            for name in list.iter().take(MAX_RESULTS) {
                let name = name.as_str().ok_or("`select` must be a list of glyph names")?;
                if !next.results.iter().any(|r| r.name == name) {
                    return Err(format!("`{}` is not among the results", quoted(name)));
                }
                names.push(name.to_string());
            }
            next.selected = names;
        }
        Some(_) => return Err("`select` must be a list of glyph names".into()),
    }
    if flag("insert")?.unwrap_or(false) {
        let names = next.selected_in_order();
        if names.is_empty() {
            return Err("no result is selected".into());
        }
        crate::shell::commands::run(app, &CommandRef::with_params("editView.insertText", json!({ "glyphs": names })))?;
        next.open = false;
    } else {
        next.open = flag("open")?.unwrap_or(true);
    }
    app.view.text.find = next;
    Ok(())
}

/// `ui.inspect`'s `editText.findGlyphs`: `{open, query, mode, results: [names], total, selected}`.
pub fn reply(app: &FontCraftApp) -> Value {
    let st = &app.view.text.find;
    json!({
        "open": st.open,
        "query": st.query,
        "mode": st.mode(),
        "results": st.results.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        "total": st.total,
        "selected": st.selected_in_order(),
    })
}

/// The pop-up on the dialog host: a copy of the state it shows, refreshed every pass.
pub struct FindGlyphsDialog {
    id: DialogId,
    pub state: FindGlyphs,
    pub selection: Selection<String>,
    open: bool,
    /// What the person did since [`run_dialog`] last took it.
    picked: Vec<Msg>,
}

fn find(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("edit.find", params))
}

impl StackDialog for FindGlyphsDialog {
    /// What the person did in the pass the pop-up closed.
    type Result = Vec<Msg>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Vec<Msg>>> {
        let words = Env::of(ui).words::<FindGlyphsWords>();
        let FindGlyphsDialog { state, selection, open, picked, .. } = self;
        let modes = [
            Entry::new("name".to_string(), words.by_name.clone()),
            Entry::new("unicode".to_string(), words.by_unicode.clone()),
            Entry::new("all".to_string(), words.by_all.clone()),
        ];
        let shown = FormDialog::new(words.title.clone(), Bind::bound(selection)).opened(Bind::bound(open)).ok_label(words.insert.clone()).show(
            ui,
            |ui, _, selection| {
                let mut mode = state.mode().to_string();
                Select::new(words.mode.clone(), Bind::bound(&mut mode), &modes).show(ui);
                if mode != state.mode() {
                    picked.push(find(json!({ "mode": mode })));
                }
                let mut query = state.query.clone();
                TextField::search(words.search.clone(), Bind::bound(&mut query)).placeholder(words.search.clone()).show(ui);
                if query != state.query {
                    picked.push(find(json!({ "query": query })));
                }
                if state.results.is_empty() {
                    Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
                    return false;
                }
                let columns = vec![
                    ColumnSpec::new(NAME, words.glyph.clone(), ColumnWidth::Content),
                    ColumnSpec::new(CODE, words.code.clone(), ColumnWidth::Content),
                ];
                let names: Vec<String> = state.results.iter().map(|r| r.name.clone()).collect();
                let source = keyed(&names, |n| n.clone()).with_text(|n| Cow::Owned(n.clone()));
                let before = selection.items.clone();
                let _ = Table::<String>::new(words.results.clone(), columns)
                    .select(SelectMode::Multi)
                    .selection(Bind::bound(selection))
                    .max_rows(LIST_ROWS)
                    .show_items(ui, &source, &mut |name, icx| {
                        let unicode = state.results.iter().find(|r| r.name == *name).map(|r| r.unicode.clone()).unwrap_or_default();
                        ListRow::new(name.clone()).cells(icx.cells()).cell(CODE, unicode)
                    });
                if selection.items != before {
                    picked.push(find(json!({ "select": selection.items })));
                }
                !selection.items.is_empty()
            },
        );
        if let Some(selection) = shown.accepted {
            picked.push(find(json!({"select": selection.items, "insert": true})));
            return Some(DialogResult::Ok(std::mem::take(picked)));
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }
}

/// Opens the pop-up when the text state asks for it, keeps it in step with the state, posts what the person did, and
/// marks the state closed when the pop-up closes.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    if app.view.text.find.open
        && let Err(e) = app.view.text.find.refresh(&mut app.session)
    {
        app.status = e;
    }
    let st = app.view.text.find.clone();
    match host.get_mut::<FindGlyphsDialog>() {
        None if st.open => {
            let selection = Selection::new(st.selected.clone(), st.selected.first().cloned(), st.selected.first().cloned());
            host.open(FindGlyphsDialog { id: dialog_id("editView.findGlyphs"), state: st, selection, open: true, picked: Vec::new() });
        }
        Some(dialog) => {
            app.sent.append(&mut dialog.picked);
            if dialog.state.selected != st.selected {
                dialog.selection = Selection::new(st.selected.clone(), st.selected.first().cloned(), st.selected.first().cloned());
            }
            dialog.open = st.open;
            dialog.state = st;
        }
        None => {}
    }
    if let Some(result) = host.take::<FindGlyphsDialog>() {
        if let DialogResult::Ok(mut msgs) = result {
            app.sent.append(&mut msgs);
        }
        if !app.sent.iter().any(|m| matches!(m, Msg::Shell(c) if c.params.get("insert").is_some())) {
            app.view.text.find.open = false;
        }
    }
}
