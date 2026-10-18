//! The Compare Fonts window (M9.24): two masters, from one open document or from two, side by
//! side, with Use Left and Use Right running their commands on the selected row.
//!
//! Rows: `edit.compareFonts`, `window.compareFonts`.
//!
//! | Input | Does |
//! |---|---|
//! | a document or master pop-up | `edit.compareFonts` with that side changed |
//! | click a grey heading | collapses or expands its category |
//! | click a row | selects it |
//! | Use Left, Use Right | `compareFonts.useLeft`, `compareFonts.useRight` on the selected row |
//!
//! The rows are what `compareFonts.diff` returns for the two sides. They are asked for again only
//! when a side or one of the two documents changes (kept in egui's memory, not in the saved
//! state), and only the rows in view are laid out.

use std::sync::Arc;

use egui::{Align2, FontId, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

/// The categories in the window's order: the `category` of `compareFonts.diff`, and the heading.
pub const CATEGORIES: [(&str, &str); 4] =
    [("fontInfo", "Font Info"), ("masterMetrics", "Master Metrics"), ("glyph", "Glyph Outlines"), ("kerning", "Kerning Pairs")];
/// Width of the row-name column and of each side's column, in points.
const LABEL_WIDTH: f32 = 200.0;
const VALUE_WIDTH: f32 = 240.0;
/// Characters of a value the window shows; a longer value ends in an ellipsis.
const MAX_SHOWN_CHARS: usize = 200;
/// Rows the list shows before it scrolls.
const VISIBLE_ROWS: f32 = 24.0;

pub const COMMANDS: &[UiCommand] = &[(
    "edit.compareFonts",
    "Compare Fonts…",
    &["Edit"],
    "",
    "{left?: {document?: index|title|path, master?: id|name}, right?: {document?, master?}} open the Compare Fonts window on two masters of one or two open documents. A side not given keeps the window's last one (the first time: the active document on the left, the next open document on the right, or the active document's second master when no other document is open); a document given without a master takes its first master → compareFonts.diff's result for the two sides, with open",
)];

/// One side of the window: an open document, by session id, and one of its masters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareSide {
    pub document: u64,
    /// A master id; empty for the document's first master.
    pub master: String,
}

/// The selected row, by the `category` and `key` `compareFonts.diff` gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SelectedRow {
    pub category: String,
    pub key: String,
}

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareFontsState {
    pub open: bool,
    pub left: Option<CompareSide>,
    pub right: Option<CompareSide>,
    /// The categories whose rows are hidden under their heading.
    pub collapsed: Vec<String>,
    pub selected: Option<SelectedRow>,
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "edit.compareFonts" => open(app, args),
        _ => Err(format!("unknown command `{id}`")),
    }
}

fn position(app: &FontcraftApp, document: u64) -> Option<usize> {
    app.session.documents().iter().position(|d| d.id == document)
}

/// `side` while its document is open; a master the document no longer has becomes its first.
fn still_open(app: &FontcraftApp, side: Option<&CompareSide>) -> Option<CompareSide> {
    let side = side?;
    let doc = app.session.documents().iter().find(|d| d.id == side.document)?;
    let master = if doc.font.masters.iter().any(|m| m.id == side.master) { side.master.clone() } else { String::new() };
    Some(CompareSide { document: side.document, master })
}

/// The sides the window first opens on.
fn default_sides(app: &FontcraftApp) -> Option<(CompareSide, CompareSide)> {
    let docs = app.session.documents();
    let active = app.session.active().or_else(|| docs.first())?;
    let left = CompareSide { document: active.id, master: String::new() };
    let right = match docs.iter().find(|d| d.id != active.id) {
        Some(other) => CompareSide { document: other.id, master: String::new() },
        None => CompareSide { document: active.id, master: active.font.masters.get(1).map(|m| m.id.clone()).unwrap_or_default() },
    };
    Some((left, right))
}

/// One side's parameters for the compare commands.
fn side_value(index: usize, side: &CompareSide) -> Value {
    if side.master.is_empty() { json!({ "document": index }) } else { json!({ "document": index, "master": side.master }) }
}

/// One side's parameters for `compareFonts.diff`: what the command was given, the rest from `side`.
fn side_params(app: &FontcraftApp, given: Option<&Value>, side: &CompareSide) -> Value {
    let given = match given {
        None => None,
        Some(Value::Object(m)) => Some(m),
        // `compareFonts.diff` says what is wrong with it.
        Some(other) => return other.clone(),
    };
    let field = |name: &str| given.and_then(|m| m.get(name)).filter(|v| !v.is_null());
    let mut out = Map::new();
    let document = field("document");
    match document {
        Some(d) => {
            out.insert("document".into(), d.clone());
        }
        None => {
            if let Some(i) = position(app, side.document) {
                out.insert("document".into(), json!(i));
            }
        }
    }
    match field("master") {
        Some(m) => {
            out.insert("master".into(), m.clone());
        }
        None if document.is_none() && !side.master.is_empty() => {
            out.insert("master".into(), json!(side.master));
        }
        None => {}
    }
    Value::Object(out)
}

/// A side as `compareFonts.diff` resolved it (`{document: position, master: id}`).
fn resolved(app: &FontcraftApp, v: Option<&Value>) -> Option<CompareSide> {
    let v = v?;
    let index = usize::try_from(v.get("document")?.as_u64()?).ok()?;
    let doc = app.session.documents().get(index)?;
    Some(CompareSide { document: doc.id, master: v.get("master").and_then(Value::as_str).unwrap_or_default().to_string() })
}

fn open(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let (left_default, right_default) = default_sides(app).ok_or_else(|| args.err("no document is open"))?;
    let st = &app.state.panels.compare_fonts;
    let left = still_open(app, st.left.as_ref()).unwrap_or(left_default);
    let right = still_open(app, st.right.as_ref()).unwrap_or(right_default);
    let params = json!({
        "left": side_params(app, args.get("left"), &left),
        "right": side_params(app, args.get("right"), &right),
    });
    let mut out = app.session.query("compareFonts.diff", &params).map_err(|e| args.err(e))?;
    let (Some(left), Some(right)) = (resolved(app, out.get("left")), resolved(app, out.get("right"))) else {
        return Err(args.err("the compared documents are not open"));
    };
    let st = &mut app.state.panels.compare_fonts;
    if st.left.as_ref() != Some(&left) || st.right.as_ref() != Some(&right) {
        st.selected = None;
    }
    st.open = true;
    st.left = Some(left);
    st.right = Some(right);
    if let Some(o) = out.as_object_mut() {
        o.insert("open".into(), json!(true));
    }
    Ok(out)
}

// ---------- rows ----------

/// One row of the window: a `compareFonts.diff` row with its values as text.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub category: String,
    pub key: String,
    /// The row's name; a glyph row adds what differs.
    pub label: String,
    /// `None` where that side has no value.
    pub left: Option<String>,
    pub right: Option<String>,
    /// A kerning row's sides.
    pub first: Option<String>,
    pub second: Option<String>,
}

impl Row {
    fn from_diff(v: &Value) -> Option<Row> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let category = text("category")?;
        let key = text("key")?;
        let mut label = text("label").unwrap_or_else(|| key.clone());
        if let Some(d) = v.get("differences").and_then(Value::as_array) {
            let names: Vec<&str> = d.iter().filter_map(Value::as_str).collect();
            if !names.is_empty() {
                label = format!("{label} ({})", names.join(", "));
            }
        }
        Some(Row { category, key, label, left: shown(v.get("left")), right: shown(v.get("right")), first: text("first"), second: text("second") })
    }
}

/// The rows of two sides, with what they were read from.
pub struct Built {
    /// Each side's document id, its revision and master.
    key: [(u64, u64, String); 2],
    pub rows: Vec<Row>,
    pub truncated: bool,
    pub error: Option<String>,
}

fn stamp(app: &FontcraftApp, side: &CompareSide) -> (u64, u64, String) {
    let revision = app.session.documents().iter().find(|d| d.id == side.document).map_or(u64::MAX, |d| d.revision);
    (side.document, revision, side.master.clone())
}

/// Asks `compareFonts.diff` for the rows of the two sides.
pub fn build(app: &mut FontcraftApp, left: &CompareSide, right: &CompareSide) -> Built {
    let key = [stamp(app, left), stamp(app, right)];
    let (Some(l), Some(r)) = (position(app, left.document), position(app, right.document)) else {
        return Built { key, rows: Vec::new(), truncated: false, error: Some("A compared document was closed.".into()) };
    };
    let params = json!({ "left": side_value(l, left), "right": side_value(r, right) });
    match app.session.query("compareFonts.diff", &params) {
        Ok(out) => Built {
            key,
            rows: out.get("rows").and_then(Value::as_array).map(|rows| rows.iter().filter_map(Row::from_diff).collect()).unwrap_or_default(),
            truncated: out.get("truncated").and_then(Value::as_bool).unwrap_or(false),
            error: None,
        },
        Err(e) => Built { key, rows: Vec::new(), truncated: false, error: Some(e.to_string()) },
    }
}

/// The command Use Left (`from_left`) or Use Right runs on the selected row, with its
/// parameters; `None` while no listed row is selected or a compared document is closed.
pub fn use_command(app: &FontcraftApp, rows: &[Row], from_left: bool) -> Option<(&'static str, Value)> {
    let st = &app.state.panels.compare_fonts;
    let selected = st.selected.as_ref()?;
    let row = rows.iter().find(|r| r.category == selected.category && r.key == selected.key)?;
    let (left, right) = (st.left.as_ref()?, st.right.as_ref()?);
    let mut target = Map::new();
    target.insert("category".into(), json!(row.category));
    target.insert("key".into(), json!(row.key));
    if let (Some(first), Some(second)) = (&row.first, &row.second) {
        target.insert("first".into(), json!(first));
        target.insert("second".into(), json!(second));
    }
    let params = json!({
        "left": side_value(position(app, left.document)?, left),
        "right": side_value(position(app, right.document)?, right),
        "row": Value::Object(target),
    });
    Some((if from_left { "compareFonts.useLeft" } else { "compareFonts.useRight" }, params))
}

/// A value as the window shows it; `None` for no value.
fn shown(v: Option<&Value>) -> Option<String> {
    let text = match v? {
        Value::Null => return None,
        Value::String(s) => s.clone(),
        Value::Object(o) if o.contains_key("paths") => glyph_summary(o),
        other => number_text(other),
    };
    Some(clip(text))
}

/// Whole numbers without decimals; other values as JSON.
fn number_text(v: &Value) -> String {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{f:.0}"),
            Some(f) => format!("{f:.2}"),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A glyph row's side: width, paths, nodes, components and anchors.
fn glyph_summary(o: &Map<String, Value>) -> String {
    let n = |k: &str| o.get(k).map(number_text).unwrap_or_default();
    let mut parts = vec![format!("width {}", n("width")), format!("{} paths", n("paths")), format!("{} nodes", n("nodes"))];
    let list = |k: &str| o.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let components: Vec<&str> = list("components").iter().filter_map(Value::as_str).collect();
    if !components.is_empty() {
        parts.push(format!("components {}", components.join(" ")));
    }
    let anchors: Vec<String> = list("anchors")
        .iter()
        .map(|a| {
            let name = a.get("name").and_then(Value::as_str).unwrap_or_default();
            let at = |k: &str| a.get(k).map(number_text).unwrap_or_default();
            format!("{name} {} {}", at("x"), at("y"))
        })
        .collect();
    if !anchors.is_empty() {
        parts.push(format!("anchors {}", anchors.join("; ")));
    }
    parts.join(", ")
}

fn clip(s: String) -> String {
    if s.chars().count() <= MAX_SHOWN_CHARS {
        return s;
    }
    let mut out: String = s.chars().take(MAX_SHOWN_CHARS).collect();
    out.push('…');
    out
}

/// A line of the list: a category heading (category, row count) or a row.
#[derive(Clone, Copy)]
enum Line {
    Heading(usize, usize),
    Row(usize),
}

/// The list's lines: each heading, then its rows unless it is collapsed.
fn lines(rows: &[Row], collapsed: &[String]) -> Vec<Line> {
    let mut by_category: [Vec<usize>; 4] = Default::default();
    for (i, r) in rows.iter().enumerate() {
        if let Some(c) = CATEGORIES.iter().position(|(id, _)| *id == r.category)
            && let Some(list) = by_category.get_mut(c)
        {
            list.push(i);
        }
    }
    let mut out = Vec::with_capacity(rows.len() + CATEGORIES.len());
    for (c, ((id, _), list)) in CATEGORIES.iter().zip(by_category).enumerate() {
        out.push(Line::Heading(c, list.len()));
        if !collapsed.iter().any(|x| x == id) {
            out.extend(list.into_iter().map(Line::Row));
        }
    }
    out
}

// ---------- drawing ----------

/// An open document as the pop-ups list it: id, title, masters (id, name).
type DocInfo = (u64, String, Vec<(String, String)>);

/// `{name: value}`.
fn one(name: &str, value: Value) -> Value {
    let mut m = Map::new();
    m.insert(name.into(), value);
    Value::Object(m)
}

/// The document and master pop-ups of one side (`name`: left or right).
fn side_pickers(ui: &mut egui::Ui, tokens: &Tokens, docs: &[DocInfo], name: &'static str, side: &CompareSide, pending: &mut Pending) {
    let width = VALUE_WIDTH - tokens.dialog_spacing;
    let current = docs.iter().enumerate().find(|(_, d)| d.0 == side.document);
    egui::ComboBox::from_id_salt(("window.compareFonts.document", name))
        .width(width)
        .selected_text(current.map_or("", |(_, d)| d.1.as_str()))
        .show_ui(ui, |ui| {
            for (i, (id, title, _)) in docs.iter().enumerate() {
                if ui.selectable_label(*id == side.document, title).clicked() && *id != side.document {
                    pending.push(("edit.compareFonts", one(name, json!({ "document": i }))));
                }
            }
        });
    let masters: &[(String, String)] = current.map(|(_, d)| d.2.as_slice()).unwrap_or_default();
    let shown_master = if side.master.is_empty() { masters.first() } else { masters.iter().find(|m| m.0 == side.master) };
    egui::ComboBox::from_id_salt(("window.compareFonts.master", name)).width(width).selected_text(shown_master.map_or("", |m| m.1.as_str())).show_ui(
        ui,
        |ui| {
            let Some((index, _)) = current else { return };
            for (id, label) in masters {
                let is = shown_master.is_some_and(|m| m.0 == *id);
                if ui.selectable_label(is, label).clicked() && !is {
                    pending.push(("edit.compareFonts", one(name, json!({ "document": index, "master": id }))));
                }
            }
        },
    );
}

/// A grey category heading with its disclosure triangle; true when clicked.
fn heading(ui: &mut egui::Ui, tokens: &Tokens, title: &str, count: usize, expanded: bool) -> bool {
    let h = tokens.list_row_height;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, tokens.panel_darker);
    let tri = Rect::from_center_size(pos2(rect.left() + h * 0.5, rect.center().y), vec2(h * 0.4, h * 0.4));
    let points = if expanded {
        vec![tri.left_top(), tri.right_top(), tri.center_bottom()]
    } else {
        vec![tri.left_top(), tri.right_center(), tri.left_bottom()]
    };
    p.add(egui::Shape::convex_polygon(points, tokens.text_dim, Stroke::NONE));
    p.text(
        pos2(rect.left() + h, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{title} ({count})"),
        FontId::proportional(tokens.font_size),
        tokens.text_dim,
    );
    resp.clicked()
}

/// One row: its name and the two sides' values; true when clicked.
fn row_line(ui: &mut egui::Ui, tokens: &Tokens, row: &Row, selected: bool) -> bool {
    let h = tokens.list_row_height;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let p = ui.painter_at(rect);
    if selected {
        p.rect_filled(rect, 0.0, tokens.row_selected);
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, tokens.hover);
    }
    let colour = if selected { tokens.selected_text } else { tokens.text };
    let font = FontId::proportional(tokens.font_size);
    let columns = [
        (rect.left() + h, LABEL_WIDTH - h, Some(&row.label)),
        (rect.left() + LABEL_WIDTH, VALUE_WIDTH, row.left.as_ref()),
        (rect.left() + LABEL_WIDTH + VALUE_WIDTH, VALUE_WIDTH, row.right.as_ref()),
    ];
    for (x, w, text) in columns {
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2((w - tokens.dialog_spacing).max(0.0), h));
        let (text, c) = match text {
            Some(t) => (t.as_str(), colour),
            None => ("–", tokens.text_dim),
        };
        p.with_clip_rect(cell.intersect(rect)).text(pos2(cell.left(), cell.center().y), Align2::LEFT_CENTER, text, font.clone(), c);
    }
    let clicked = resp.clicked();
    resp.on_hover_text(format!("{}\nLeft: {}\nRight: {}", row.label, row.left.as_deref().unwrap_or("–"), row.right.as_deref().unwrap_or("–")));
    clicked
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let st = app.state.panels.compare_fonts.clone();
    if !st.open {
        return;
    }
    let (Some(left), Some(right)) = (st.left.clone(), st.right.clone()) else {
        app.state.panels.compare_fonts.open = false;
        return;
    };
    let key = [stamp(app, &left), stamp(app, &right)];
    let memory_id = egui::Id::new("window.compareFonts.rows");
    let cached: Option<Arc<Built>> = ui.ctx().data(|d| d.get_temp::<Arc<Built>>(memory_id));
    let built = match cached {
        Some(b) if b.key == key => b,
        _ => {
            let b = Arc::new(build(app, &left, &right));
            ui.ctx().data_mut(|d| d.insert_temp(memory_id, b.clone()));
            b
        }
    };
    let use_left = use_command(app, &built.rows, true);
    let use_right = use_command(app, &built.rows, false);
    let docs: Vec<DocInfo> = app
        .session
        .documents()
        .iter()
        .map(|d| (d.id, d.title.clone(), d.font.masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect()))
        .collect();
    let lines = lines(&built.rows, &st.collapsed);
    let tokens = &app.tokens;
    let mut open = true;
    let mut clicked: Option<SelectedRow> = None;
    let mut toggled: Option<&'static str> = None;
    egui::Window::new("Compare Fonts")
        .id(egui::Id::new("window.compareFonts"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(LABEL_WIDTH + 2.0 * VALUE_WIDTH + 2.0 * tokens.dialog_spacing)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(LABEL_WIDTH - ui.spacing().item_spacing.x);
                for (name, side) in [("left", &left), ("right", &right)] {
                    ui.vertical(|ui| {
                        ui.set_width(VALUE_WIDTH);
                        side_pickers(ui, tokens, &docs, name, side, pending);
                    });
                }
            });
            ui.separator();
            if let Some(e) = &built.error {
                ui.colored_label(tokens.error_text, e);
            } else {
                let row_height = tokens.list_row_height;
                egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(row_height * VISIBLE_ROWS).show_rows(
                    ui,
                    row_height,
                    lines.len(),
                    |ui, range| {
                        for line in lines.get(range).unwrap_or_default() {
                            match *line {
                                Line::Heading(c, count) => {
                                    let Some((id, title)) = CATEGORIES.get(c) else { continue };
                                    if heading(ui, tokens, title, count, !st.collapsed.iter().any(|x| x == id)) {
                                        toggled = Some(*id);
                                    }
                                }
                                Line::Row(i) => {
                                    let Some(row) = built.rows.get(i) else { continue };
                                    let selected = st.selected.as_ref().is_some_and(|s| s.category == row.category && s.key == row.key);
                                    if row_line(ui, tokens, row, selected) {
                                        clicked = Some(SelectedRow { category: row.category.clone(), key: row.key.clone() });
                                    }
                                }
                            }
                        }
                    },
                );
                if built.rows.is_empty() {
                    ui.colored_label(tokens.text_dim, "The two masters do not differ.");
                } else if built.truncated {
                    ui.colored_label(tokens.text_dim, format!("Only the first {} differences are listed.", built.rows.len()));
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(use_left.is_some(), egui::Button::new("Use Left")).clicked()
                    && let Some(c) = use_left.clone()
                {
                    pending.push(c);
                }
                if ui.add_enabled(use_right.is_some(), egui::Button::new("Use Right")).clicked()
                    && let Some(c) = use_right.clone()
                {
                    pending.push(c);
                }
            });
        });
    let st = &mut app.state.panels.compare_fonts;
    if !open {
        st.open = false;
    }
    if let Some(row) = clicked {
        st.selected = Some(row);
    }
    if let Some(category) = toggled {
        match st.collapsed.iter().position(|c| c == category) {
            Some(i) => {
                st.collapsed.remove(i);
            }
            None => st.collapsed.push(category.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_engine::Session;
    use fontcraft_engine::model::{Font, Layer, Master};

    use super::*;

    /// "Untitled" (A 600 wide) and "Other" (A 640 wide, x-height 520), Untitled active.
    fn two_documents() -> FontcraftApp {
        let mut app = FontcraftApp::new(Session::with_document());
        let mut other = Font::new("Other");
        other.glyph_mut("A").unwrap().layer_mut("m01").unwrap().width = 640.0;
        other.set_metric_pos("m01", "x-height", 520.0).unwrap();
        app.session.add_document(other, None).unwrap();
        app.run("document.activate", &json!({"index": 0})).unwrap();
        app
    }

    fn draw(app: &mut FontcraftApp) {
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }

    #[test]
    fn edit_compare_fonts_shows_the_diff_and_use_left_equals_compare_fonts_use_left() {
        let mut app = two_documents();
        let out = app.run("edit.compareFonts", &json!({})).unwrap();
        assert_eq!(out["open"], true);
        assert_eq!((out["left"]["document"].clone(), out["right"]["document"].clone()), (json!(0), json!(1)));
        let diff = app.session.query("compareFonts.diff", &json!({"left": {"document": 0}, "right": {"document": 1}})).unwrap();
        assert_eq!(out["rows"], diff["rows"], "the window lists what compareFonts.diff returns");

        let st = app.state.panels.compare_fonts.clone();
        let built = build(&mut app, st.left.as_ref().unwrap(), st.right.as_ref().unwrap());
        assert_eq!(built.rows.len(), diff["rows"].as_array().unwrap().len());
        let a = built.rows.iter().find(|r| r.category == "glyph" && r.key == "A").unwrap();
        assert_eq!(a.label, "A (width)");
        assert!(a.left.as_deref().unwrap().starts_with("width 600"), "{a:?}");
        assert!(use_command(&app, &built.rows, true).is_none(), "nothing is selected");

        // Use Left on the selected row runs compareFonts.useLeft with the window's sides.
        app.state.panels.compare_fonts.selected = Some(SelectedRow { category: "glyph".into(), key: "A".into() });
        let (id, params) = use_command(&app, &built.rows, true).unwrap();
        assert_eq!(id, "compareFonts.useLeft");
        app.run(id, &params).unwrap();
        let mut twin = two_documents();
        twin.run("compareFonts.useLeft", &json!({"left": {"document": 0}, "right": {"document": 1}, "row": {"category": "glyph", "key": "A"}}))
            .unwrap();
        let width = |app: &FontcraftApp| app.session.documents()[1].font.glyph("A").unwrap().layer("m01").unwrap().width;
        assert_eq!(width(&app), 600.0);
        assert_eq!(width(&app), width(&twin));

        // Use Right on the x-height writes the right master's value to the left document.
        let built = build(&mut app, st.left.as_ref().unwrap(), st.right.as_ref().unwrap());
        assert!(!built.rows.iter().any(|r| r.category == "glyph" && r.key == "A"), "the difference is gone");
        assert!(use_command(&app, &built.rows, true).is_none(), "the selected row is no longer listed");
        let metric = built.rows.iter().find(|r| r.category == "masterMetrics").unwrap().clone();
        app.state.panels.compare_fonts.selected = Some(SelectedRow { category: metric.category.clone(), key: metric.key.clone() });
        let (id, params) = use_command(&app, &built.rows, false).unwrap();
        assert_eq!(id, "compareFonts.useRight");
        app.run(id, &params).unwrap();
        let after = app
            .session
            .query("compareFonts.diff", &json!({"left": {"document": 0}, "right": {"document": 1}, "categories": ["masterMetrics"]}))
            .unwrap();
        assert_eq!(after["rows"], json!([]));

        // A refused side changes nothing.
        let before = app.state.panels.compare_fonts.clone();
        assert!(app.run("edit.compareFonts", &json!({"right": {"master": "Nonesuch"}})).is_err());
        assert_eq!(app.state.panels.compare_fonts, before);

        // Collapsing a category and drawing.
        app.state.panels.compare_fonts.collapsed.push("fontInfo".into());
        draw(&mut app);
        assert!(app.state.panels.compare_fonts.open);
    }

    #[test]
    fn edit_compare_fonts_with_one_document_compares_its_first_two_masters() {
        let mut f = Font::new("Two");
        Arc::make_mut(&mut f.masters).push(Master { id: "m02".into(), name: "Bold".into(), ..Default::default() });
        f.glyph_mut("A").unwrap().layers.push(Layer::new("m02", 700.0));
        let mut session = Session::new();
        session.add_document(f, None).unwrap();
        let mut app = FontcraftApp::new(session);
        let out = app.run("edit.compareFonts", &json!({})).unwrap();
        assert_eq!(out["left"]["master"], "m01");
        assert_eq!(out["right"]["master"], "m02");
        assert_eq!(out["right"]["document"], 0);
        let a = out["rows"].as_array().unwrap().iter().find(|r| r["category"] == "glyph" && r["key"] == "A").unwrap();
        assert_eq!(a["differences"], json!(["width"]));

        // A side given by master name keeps the other side.
        let out = app.run("edit.compareFonts", &json!({"left": {"master": "Bold"}})).unwrap();
        assert_eq!((out["left"]["master"].clone(), out["right"]["master"].clone()), (json!("m02"), json!("m02")));
        assert_eq!(out["rows"].as_array().unwrap().iter().filter(|r| r["category"] == "glyph").count(), 0);
        draw(&mut app);
    }
}
