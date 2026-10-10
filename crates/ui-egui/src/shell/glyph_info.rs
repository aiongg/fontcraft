//! The Glyph Info window (`window.glyphInfo`): the glyph data as a table, searched and sorted by
//! `glyphData.search`, with Add to Font (`glyphInfo.addToFont`).
//!
//! | Pointer | Does |
//! |---|---|
//! | typing in the search field | searches again |
//! | header click | sorts by the column |
//! | row click | selects the row alone |
//! | Shift+click | selects the rows from the last clicked one to this one |
//! | Cmd+click | adds the row to the selection or takes it out |
//! | Add to Font | `glyphInfo.addToFont` with the selected names |
//!
//! The search runs again only when the query, the sort or the glyph data change. Only the rows in
//! view are laid out.

use std::sync::Arc;

use egui::{Align2, CornerRadius, FontId, Modifiers, Rect, Sense, Shape, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// Longest query, in characters: four bytes each stay within the search's byte limit.
const MAX_QUERY_CHARS: usize = 64;
/// Most rows one search shows (the search's own limit).
const MAX_ROWS: usize = 10_000;
/// Rows the table shows before it scrolls.
const VISIBLE_ROWS: f32 = 20.0;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;

pub const COMMANDS: &[UiCommand] = &[(
    "window.glyphInfo",
    "Glyph Info",
    &["Window"],
    "",
    "{open?: bool (default true), query?: string, sortBy?: \"name\" | \"unicode\" | \"category\" | \"subCategory\" | \"script\" | \"description\", select?: [names]} show or hide the Glyph Info window: the glyph data as a table, searched and sorted as glyphData.search does. query and sortBy set the search field and the sorted column; select sets the selected rows, which must be among the results → {open, query, sortBy, count, selected}",
)];

/// A column of the table. Its id is the `sortBy` value and the result field it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Column {
    Name,
    Unicode,
    Category,
    SubCategory,
    Script,
    Description,
}

impl Column {
    pub const ALL: [Column; 6] = [Column::Name, Column::Unicode, Column::Category, Column::SubCategory, Column::Script, Column::Description];

    pub fn id(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Unicode => "unicode",
            Column::Category => "category",
            Column::SubCategory => "subCategory",
            Column::Script => "script",
            Column::Description => "description",
        }
    }

    pub fn of_id(id: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Unicode => "Unicode",
            Column::Category => "Category",
            Column::SubCategory => "Subcategory",
            Column::Script => "Script",
            Column::Description => "Description",
        }
    }

    /// Width in `Tokens::list_column_unit`.
    fn width_units(self) -> f32 {
        match self {
            Column::Name => 4.0,
            Column::Unicode => 2.0,
            Column::Category | Column::Script => 2.2,
            Column::SubCategory => 2.6,
            Column::Description => 6.0,
        }
    }
}

/// The window's state: open or not, the search, the sort and the selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GlyphInfoState {
    pub open: bool,
    pub query: String,
    /// The sorted column; `None`: by name.
    pub sort_by: Option<Column>,
    /// Selected rows by name, in the order they were picked.
    pub selected: Vec<String>,
    /// The row Shift+click extends from.
    pub anchor: Option<String>,
    /// The rows of the last search.
    #[serde(skip)]
    pub results: Results,
    /// The rows laid out in the last frame with their rectangles, and the Add to Font button:
    /// tests click them.
    #[serde(skip)]
    pub laid_out: Vec<(String, Rect)>,
    #[serde(skip)]
    pub add_button: Option<Rect>,
}

/// One result of the search, as the table shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub name: String,
    pub unicode: String,
    pub category: String,
    pub sub_category: String,
    pub script: String,
    pub description: String,
}

impl Row {
    fn of(v: &Value) -> Option<Row> {
        let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let name = v.get("name").and_then(Value::as_str)?.to_string();
        Some(Row {
            name,
            unicode: text("unicode"),
            category: text("category"),
            sub_category: text("subCategory"),
            script: text("script"),
            description: text("description"),
        })
    }

    pub fn cell(&self, c: Column) -> &str {
        match c {
            Column::Name => &self.name,
            Column::Unicode => &self.unicode,
            Column::Category => &self.category,
            Column::SubCategory => &self.sub_category,
            Column::Script => &self.script,
            Column::Description => &self.description,
        }
    }
}

/// What a search was run for: the query, the sort and the glyph data it read.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    query: String,
    sort_by: Option<Column>,
    data: Option<usize>,
}

/// The rows of the last search and what they were searched for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Results {
    key: Option<Key>,
    pub rows: Vec<Row>,
    /// Why the search was refused.
    pub error: Option<String>,
}

impl Results {
    fn contains(&self, name: &str) -> bool {
        self.rows.iter().any(|r| r.name == name)
    }
}

/// The parameters of `glyphData.search` for this query and sort.
pub fn search_params(query: &str, sort_by: Option<Column>) -> Value {
    let mut p = Map::new();
    p.insert("query".into(), json!(query));
    p.insert("limit".into(), json!(MAX_ROWS));
    if let Some(c) = sort_by {
        p.insert("sortBy".into(), json!(c.id()));
    }
    Value::Object(p)
}

fn key(app: &FontcraftApp, query: &str, sort_by: Option<Column>) -> Key {
    // The glyph data is shared: a new value is a new pointer.
    let data = app.session.glyph_data().ok().map(|d| Arc::as_ptr(&d) as usize);
    Key { query: query.to_string(), sort_by, data }
}

/// Runs the search for `key`. A blank query lists the whole database, up to [`MAX_ROWS`].
fn search(app: &mut FontcraftApp, key: Key) -> Results {
    match app.session.execute("glyphData.search", &search_params(&key.query, key.sort_by)) {
        Ok(v) => {
            let rows = v.get("results").and_then(Value::as_array).map(|a| a.iter().filter_map(Row::of).collect()).unwrap_or_default();
            Results { key: Some(key), rows, error: None }
        }
        Err(e) => Results { key: Some(key), rows: Vec::new(), error: Some(e.to_string()) },
    }
}

/// Searches again when the query, the sort or the glyph data changed. The selection keeps only
/// the rows still shown.
pub fn refresh(app: &mut FontcraftApp) {
    let gi = &app.state.glyph_info;
    let k = key(app, &gi.query, gi.sort_by);
    if gi.results.key.as_ref() == Some(&k) {
        return;
    }
    let results = search(app, k);
    let gi = &mut app.state.glyph_info;
    gi.selected.retain(|n| results.contains(n));
    if gi.anchor.as_deref().is_some_and(|a| !results.contains(a)) {
        gi.anchor = None;
    }
    gi.results = results;
}

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "window.glyphInfo" {
        return Err(args.err("unknown command"));
    }
    let open = args.flag("open")?.unwrap_or(true);
    let query = match args.string("query")? {
        Some(q) if q.chars().count() > MAX_QUERY_CHARS => return Err(args.err(format!("`query` holds at most {MAX_QUERY_CHARS} characters"))),
        Some(q) => q.to_string(),
        None => app.state.glyph_info.query.clone(),
    };
    let sort_by = match args.string("sortBy")? {
        Some(s) => Some(Column::of_id(s).ok_or_else(|| {
            args.err(format!("`sortBy` must be name, unicode, category, subCategory, script or description, got \"{}\"", quoted(s)))
        })?),
        None => app.state.glyph_info.sort_by,
    };
    let select = args.strings("select", MAX_ROWS)?;
    // Everything is checked before anything changes.
    let k = key(app, &query, sort_by);
    let results = if app.state.glyph_info.results.key.as_ref() == Some(&k) { app.state.glyph_info.results.clone() } else { search(app, k) };
    if let Some(names) = &select
        && let Some(bad) = names.iter().find(|n| !results.contains(n))
    {
        return Err(args.err(format!("`{}` is not among the search results", quoted(bad))));
    }
    let gi = &mut app.state.glyph_info;
    gi.open = open;
    gi.query = query;
    gi.sort_by = sort_by;
    match select {
        Some(names) => {
            gi.anchor = names.last().cloned();
            gi.selected = names;
        }
        None => gi.selected.retain(|n| results.contains(n)),
    }
    gi.results = results;
    Ok(json!({
        "open": gi.open,
        "query": gi.query,
        "sortBy": gi.sort_by.map(Column::id),
        "count": gi.results.rows.len(),
        "selected": gi.selected,
    }))
}

/// The selection after a click on `name` with `mods`, among `order` (the rows as shown).
pub fn click(selected: &mut Vec<String>, anchor: &mut Option<String>, order: &[&str], name: &str, mods: Modifiers) {
    let at = order.iter().position(|n| *n == name);
    let from = anchor.as_deref().and_then(|a| order.iter().position(|n| *n == a));
    match (mods.shift, at, from) {
        (true, Some(at), Some(from)) => {
            let (lo, hi) = if from <= at { (from, at) } else { (at, from) };
            let range: Vec<String> = order.get(lo..=hi).unwrap_or_default().iter().map(|n| n.to_string()).collect();
            if mods.command {
                for n in range {
                    if !selected.contains(&n) {
                        selected.push(n);
                    }
                }
            } else {
                *selected = range;
            }
            // The anchor stays, so the next Shift+click measures from the same row.
        }
        _ if mods.command => {
            if let Some(i) = selected.iter().position(|n| n == name) {
                selected.remove(i);
            } else {
                selected.push(name.to_string());
            }
            *anchor = Some(name.to_string());
        }
        _ => {
            *selected = vec![name.to_string()];
            *anchor = Some(name.to_string());
        }
    }
}

/// The columns with their left edges and widths, relative to the table's left.
fn placed(tokens: &Tokens) -> Vec<(Column, f32, f32)> {
    let mut x = 0.0;
    Column::ALL
        .iter()
        .map(|&c| {
            let w = c.width_units() * tokens.list_column_unit;
            let out = (c, x, w);
            x += w;
            out
        })
        .collect()
}

/// What the person did in the window this frame.
enum Event {
    Query(String),
    Sort(Column),
    Click(String, Modifiers),
    Close,
}

/// Draws the window while it is open. The Add to Font button queues its command in `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.glyph_info.open {
        return;
    }
    refresh(app);
    let has_doc = app.session.active().is_some();
    let FontcraftApp { state, tokens, .. } = app;
    let gi = &mut state.glyph_info;
    let columns = placed(tokens);
    let width = columns.last().map_or(0.0, |(_, x, w)| x + w);
    let mods = ui.input(|i| i.modifiers);
    let mut events: Vec<Event> = Vec::new();
    let mut laid_out = Vec::new();
    let mut add_button = None;
    let mut open = true;
    egui::Window::new("Glyph Info")
        .id(egui::Id::new("window.glyphInfo"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(width, tokens.list_row_height * VISIBLE_ROWS))
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            let mut query = gi.query.clone();
            let field = egui::TextEdit::singleline(&mut query)
                .hint_text("Search name, character or code point")
                .char_limit(MAX_QUERY_CHARS)
                .desired_width(width);
            if ui.add(field).changed() {
                events.push(Event::Query(query));
            }
            ui.add_space(tokens.dialog_spacing);
            egui::ScrollArea::horizontal().id_salt("glyphInfo.h").show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.vertical(|ui| {
                    header(ui, gi.sort_by, &columns, width, tokens, &mut events);
                    let height = tokens.list_row_height;
                    let selected: Vec<&str> = gi.selected.iter().map(String::as_str).collect();
                    egui::ScrollArea::vertical().id_salt("glyphInfo.v").max_height(height * VISIBLE_ROWS).auto_shrink([false, false]).show_rows(
                        ui,
                        height,
                        gi.results.rows.len(),
                        |ui, range| {
                            for at in range {
                                let Some(row) = gi.results.rows.get(at) else { continue };
                                let (rect, response) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), height), Sense::click());
                                laid_out.push((row.name.clone(), rect));
                                let is_selected = selected.contains(&row.name.as_str());
                                let fill = if is_selected {
                                    tokens.row_selected
                                } else if at % 2 == 1 {
                                    tokens.panel
                                } else {
                                    tokens.panel_darker
                                };
                                ui.painter_at(rect).rect_filled(rect, CornerRadius::ZERO, fill);
                                let color = if is_selected { tokens.selected_text } else { tokens.text };
                                for &(column, x, w) in &columns {
                                    let cell = Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w, height));
                                    let text = cell.shrink2(vec2(tokens.cell_inset, 0.0));
                                    ui.painter_at(text).text(
                                        text.left_center(),
                                        Align2::LEFT_CENTER,
                                        row.cell(column),
                                        FontId::proportional(tokens.font_size),
                                        color,
                                    );
                                }
                                if response.clicked() {
                                    events.push(Event::Click(row.name.clone(), mods));
                                }
                            }
                        },
                    );
                });
            });
            ui.add_space(tokens.dialog_spacing);
            ui.horizontal(|ui| {
                let note = match &gi.results.error {
                    Some(e) => e.clone(),
                    None if gi.results.rows.len() >= MAX_ROWS && gi.query.trim().is_empty() => {
                        format!("The first {MAX_ROWS} records; search to find the others, {} selected", gi.selected.len())
                    }
                    None if gi.results.rows.len() >= MAX_ROWS => format!("The first {MAX_ROWS} matches, {} selected", gi.selected.len()),
                    None => format!("{} matches, {} selected", gi.results.rows.len(), gi.selected.len()),
                };
                let color = if gi.results.error.is_some() { tokens.error_text } else { tokens.text_dim };
                ui.colored_label(color, note);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enabled = has_doc && !gi.selected.is_empty();
                    let button = ui.add_enabled(enabled, egui::Button::new("Add to Font"));
                    let button = if has_doc { button } else { button.on_disabled_hover_text("Open a font first") };
                    add_button = Some(button.rect);
                    if button.clicked() {
                        pending.push(("glyphInfo.addToFont", json!({ "names": gi.selected })));
                    }
                });
            });
        });
    if !open {
        events.push(Event::Close);
    }
    let order: Vec<String> = gi.results.rows.iter().map(|r| r.name.clone()).collect();
    for e in events {
        match e {
            Event::Query(q) => gi.query = q,
            Event::Sort(c) => gi.sort_by = Some(c),
            Event::Click(name, mods) => {
                let order: Vec<&str> = order.iter().map(String::as_str).collect();
                click(&mut gi.selected, &mut gi.anchor, &order, &name, mods);
            }
            Event::Close => gi.open = false,
        }
    }
    gi.laid_out = laid_out;
    gi.add_button = add_button;
}

/// The header: labels, the sort mark, and sorting on click.
fn header(ui: &mut egui::Ui, sort_by: Option<Column>, columns: &[(Column, f32, f32)], width: f32, tokens: &Tokens, events: &mut Vec<Event>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(ui.available_width()), tokens.list_header_height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, tokens.app_bar);
    let sorted = sort_by.unwrap_or(Column::Name);
    for &(column, x, w) in columns {
        let cell = Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w, rect.height()));
        let response = ui.interact(cell, egui::Id::new(("glyphInfo.header", column.id())), Sense::click());
        painter.line_segment([cell.right_top(), cell.right_bottom()], Stroke::new(1.0, tokens.border));
        let text = cell.shrink2(vec2(tokens.cell_inset, 0.0));
        ui.painter_at(text).text(
            text.left_center(),
            Align2::LEFT_CENTER,
            column.label(),
            FontId::proportional(tokens.cell_label_size + 1.0),
            tokens.text,
        );
        if column == sorted {
            // A small downward triangle at the right edge: the rows ascend from the top.
            let s = tokens.badge_size / 2.0;
            let c = pos2(cell.right() - tokens.cell_inset - s, cell.center().y);
            let points = vec![pos2(c.x - s, c.y - s / 2.0), pos2(c.x + s, c.y - s / 2.0), pos2(c.x, c.y + s / 2.0)];
            painter.add(Shape::convex_polygon(points, tokens.text_dim, Stroke::NONE));
        }
        if response.clicked() {
            events.push(Event::Sort(column));
        }
    }
}
