//! The Compare Family window (M9.24): one attribute of every glyph across the open documents of
//! a family, one column per document and one row per glyph, filtered and pinned.
//!
//! Rows: `edit.compareFamily`, `window.compareFamily`, `compareFamily.setAttribute`,
//! `compareFamily.setFilter`, `compareFamily.togglePin`, `compareFamily.update`,
//! `compareFamily.openGlyph`.
//!
//! | Input | Does |
//! |---|---|
//! | the attribute pop-up | `compareFamily.setAttribute` |
//! | the Filter menu | `compareFamily.setFilter` |
//! | Keep on Top | `compareFamily.togglePin` |
//! | Update | `compareFamily.update` |
//! | double-click a cell | `compareFamily.openGlyph` |
//!
//! The table is what `compareFamily.report` returns. It is read when the window opens, when the
//! attribute or the filter changes and on Update, and kept until then: editing a document does
//! not change it. Each column keeps the font it was read from, so the Glyphs attribute draws the
//! outlines that were compared. Only the rows in view are laid out.

use std::sync::Arc;

use egui::{Align2, FontId, Rect, Sense, Stroke, pos2, vec2};
use fontcraft_engine::cmd::encodings::resolve_documents;
use fontcraft_model::Font;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

/// The attributes the table can show: the `attribute` of `compareFamily.report`, and the label.
pub const ATTRIBUTES: [(&str, &str); 5] = [
    ("componentNames", "Component Names"),
    ("anchors", "Anchors"),
    ("leftKerningGroup", "Left Kerning Group"),
    ("rightKerningGroup", "Right Kerning Group"),
    ("glyphs", "Glyphs"),
];
/// The row filters: the `filter` of `compareFamily.report`, and the label.
pub const FILTERS: [(&str, &str); 3] = [("all", "All"), ("missingInOne", "Missing in One"), ("ignoreIfMissingInOne", "Ignore If Missing in One")];
/// Width of the glyph-name column and of each document's column, in points.
const NAME_WIDTH: f32 = 160.0;
const CELL_WIDTH: f32 = 160.0;
/// Rows a command returns; `compareFamily.report` pages through all of them.
const MAX_RETURNED_ROWS: usize = 100;
/// Height of a row of glyph images, in list rows.
const GLYPH_ROW_LINES: f32 = 3.0;
/// List rows the table shows before it scrolls.
const VISIBLE_ROWS: f32 = 24.0;

pub const COMMANDS: &[UiCommand] = &[
    (
        "edit.compareFamily",
        "Compare Family…",
        &["Edit"],
        "",
        "{} open the Compare Family window: one column per open document (its first master) and one row per glyph, showing the chosen attribute. The table is read now, when the attribute or filter changes and on Update, not after every edit → {open, attribute, filter, pinned, columns: [{document, title, master}], total, rows: [{glyph, cells, same}] (the first 100 as compareFamily.report gives them), more}",
    ),
    (
        "compareFamily.setAttribute",
        "Compare Family: Attribute",
        &[],
        "",
        "{attribute: \"componentNames\"|\"anchors\"|\"leftKerningGroup\"|\"rightKerningGroup\"|\"glyphs\"} show this glyph attribute in the Compare Family table and read the table again → the window as edit.compareFamily returns it",
    ),
    (
        "compareFamily.setFilter",
        "Compare Family: Filter",
        &[],
        "",
        "{filter: \"all\"|\"missingInOne\"|\"ignoreIfMissingInOne\"} all: glyphs with a value in some document; missingInOne: glyphs absent from at least one document; ignoreIfMissingInOne: glyphs present in every document. Reads the table again → the window as edit.compareFamily returns it",
    ),
    (
        "compareFamily.togglePin",
        "Compare Family: Keep on Top",
        &[],
        "",
        "{on?: bool (default: the opposite of now)} keep the Compare Family window above the document windows → {pinned}",
    ),
    (
        "compareFamily.update",
        "Update",
        &[],
        "",
        "{} read the Compare Family table again from the open documents → the window as edit.compareFamily returns it",
    ),
    (
        "compareFamily.openGlyph",
        "Open Glyph from Compare Family",
        &[],
        "",
        "{document: index|title|path, glyph: name} make the open document active and open the glyph in a new Edit View tab there, as a double-click on its cell does; refused when the document has no such glyph → {document, title, glyph, tab}",
    ),
];

/// One column of the table: the document it was read from and its font as it was then.
pub struct Column {
    pub document: u64,
    pub title: String,
    pub master: String,
    pub font: Arc<Font>,
}

/// One row: the glyph and its cells as `compareFamily.report` gives them.
pub struct FamilyRow {
    pub glyph: String,
    pub cells: Vec<Value>,
    pub same: bool,
}

/// The table as last read.
pub struct Table {
    pub attribute: String,
    pub columns: Vec<Column>,
    /// Glyphs the filter keeps; `rows` holds at most the report's limit of them.
    pub total: usize,
    pub rows: Vec<FamilyRow>,
}

/// The table in the window's state: shared, compared by identity.
#[derive(Clone)]
pub struct Snapshot(pub Arc<Table>);

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot").field("columns", &self.0.columns.len()).field("rows", &self.0.rows.len()).finish()
    }
}

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareFamilyState {
    pub open: bool,
    /// An [`ATTRIBUTES`] id.
    pub attribute: String,
    /// A [`FILTERS`] id.
    pub filter: String,
    /// The window stays above the document windows.
    pub pinned: bool,
    #[serde(skip)]
    pub table: Option<Snapshot>,
}

impl Default for CompareFamilyState {
    fn default() -> Self {
        Self { open: false, attribute: "componentNames".into(), filter: "all".into(), pinned: false, table: None }
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let st = &app.state.panels.compare_family;
    let (attribute, filter) = (st.attribute.clone(), st.filter.clone());
    match id {
        "edit.compareFamily" => reload(app, args, attribute, filter, true),
        "compareFamily.update" => reload(app, args, attribute, filter, false),
        "compareFamily.setAttribute" => {
            let a = args.string("attribute")?.ok_or_else(|| args.err("`attribute` is required"))?;
            if !ATTRIBUTES.iter().any(|(id, _)| *id == a) {
                return Err(args.err(format!("unknown attribute `{}`: use {}", clip(a), ids(&ATTRIBUTES))));
            }
            reload(app, args, a.to_string(), filter, false)
        }
        "compareFamily.setFilter" => {
            let f = args.string("filter")?.ok_or_else(|| args.err("`filter` is required"))?;
            if !FILTERS.iter().any(|(id, _)| *id == f) {
                return Err(args.err(format!("unknown filter `{}`: use {}", clip(f), ids(&FILTERS))));
            }
            reload(app, args, attribute, f.to_string(), false)
        }
        "compareFamily.togglePin" => {
            let on = args.toggle(app.state.panels.compare_family.pinned)?;
            app.state.panels.compare_family.pinned = on;
            Ok(json!({ "pinned": on }))
        }
        "compareFamily.openGlyph" => open_glyph(app, args),
        _ => Err(format!("unknown command `{id}`")),
    }
}

fn ids(list: &[(&str, &str)]) -> String {
    list.iter().map(|(id, _)| *id).collect::<Vec<_>>().join(", ")
}

/// At most 64 characters of untrusted text, for a message.
fn clip(s: &str) -> String {
    s.chars().take(64).collect()
}

fn position(app: &FontcraftApp, document: u64) -> Option<usize> {
    app.session.documents().iter().position(|d| d.id == document)
}

/// Reads the table with `compareFamily.report`.
fn read(app: &mut FontcraftApp, attribute: &str, filter: &str) -> Result<Table, String> {
    let mut out = app.session.query("compareFamily.report", &json!({ "attribute": attribute, "filter": filter })).map_err(|e| e.to_string())?;
    let docs = app.session.documents();
    let columns = out
        .get("columns")
        .and_then(Value::as_array)
        .map(|cols| {
            cols.iter()
                .filter_map(|c| {
                    let doc = docs.get(usize::try_from(c.get("document")?.as_u64()?).ok()?)?;
                    let master = c.get("master").and_then(Value::as_str).unwrap_or_default().to_string();
                    Some(Column { document: doc.id, title: doc.title.clone(), master, font: doc.font.clone() })
                })
                .collect()
        })
        .unwrap_or_default();
    let total = out.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let rows = match out.get_mut("rows").map(Value::take) {
        Some(Value::Array(rows)) => rows
            .into_iter()
            .filter_map(|mut r| {
                let glyph = r.get("glyph")?.as_str()?.to_string();
                let same = r.get("same").and_then(Value::as_bool).unwrap_or(true);
                let Some(Value::Array(cells)) = r.get_mut("cells").map(Value::take) else { return None };
                Some(FamilyRow { glyph, cells, same })
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(Table { attribute: attribute.to_string(), columns, total, rows })
}

/// Reads the table for `attribute` and `filter` and keeps it with them; `open` also opens the
/// window. A refused read changes nothing.
fn reload(app: &mut FontcraftApp, args: &Args, attribute: String, filter: String, open: bool) -> Result<Value, String> {
    let table = read(app, &attribute, &filter).map_err(|e| args.err(e))?;
    let st = &mut app.state.panels.compare_family;
    st.attribute = attribute;
    st.filter = filter;
    st.table = Some(Snapshot(Arc::new(table)));
    if open {
        st.open = true;
    }
    Ok(window_json(app))
}

/// The window as the commands return it.
fn window_json(app: &FontcraftApp) -> Value {
    let st = &app.state.panels.compare_family;
    let (columns, total, rows) = match &st.table {
        Some(Snapshot(t)) => (
            t.columns.iter().map(|c| json!({ "document": position(app, c.document), "title": c.title, "master": c.master })).collect::<Vec<_>>(),
            t.total,
            t.rows.iter().take(MAX_RETURNED_ROWS).map(|r| json!({ "glyph": r.glyph, "cells": r.cells, "same": r.same })).collect::<Vec<_>>(),
        ),
        None => (Vec::new(), 0, Vec::new()),
    };
    let more = total > rows.len();
    json!({
        "open": st.open,
        "attribute": st.attribute,
        "filter": st.filter,
        "pinned": st.pinned,
        "columns": columns,
        "total": total,
        "rows": rows,
        "more": more,
    })
}

fn open_glyph(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let document = args.get("document").ok_or_else(|| args.err("`document` is required: an open document's position, title or path"))?;
    let glyph = args.string("glyph")?.ok_or_else(|| args.err("`glyph` is required"))?;
    let found = resolve_documents(&app.session, Some(&Value::Array(vec![document.clone()]))).map_err(|e| args.err(e))?;
    let index = found.first().and_then(|id| position(app, *id)).ok_or_else(|| args.err("no such open document"))?;
    let doc = app.session.documents().get(index).ok_or_else(|| args.err("no such open document"))?;
    if doc.font.glyph(glyph).is_none() {
        return Err(args.err(format!("`{}` has no glyph `{}`", doc.title, clip(glyph))));
    }
    let title = doc.title.clone();
    let before = app.session.active().and_then(|d| position(app, d.id));
    app.run("document.activate", &json!({ "index": index }))?;
    match app.run("view.newTab", &json!({ "glyphs": [glyph] })) {
        Ok(tab) => Ok(json!({ "document": index, "title": title, "glyph": glyph, "tab": tab })),
        Err(e) => {
            // Leave the active document as it was.
            if let Some(back) = before
                && let Err(again) = app.run("document.activate", &json!({ "index": back }))
            {
                log::warn!("compareFamily.openGlyph: {again}");
            }
            Err(e)
        }
    }
}

// ---------- drawing ----------

static NULL: Value = Value::Null;

fn is_missing(cell: &Value) -> bool {
    cell.get("missing").and_then(Value::as_bool).unwrap_or(false)
}

/// A text cell's text: names joined by commas; `None` for a glyph image.
fn cell_text(cell: &Value) -> Option<String> {
    match cell {
        Value::Null => Some("--".into()),
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => Some(items.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).collect::<Vec<_>>().join(", ")),
        Value::Object(_) => None,
        other => Some(other.to_string()),
    }
}

/// Draws one cell; returns its text for the tooltip.
fn draw_cell(p: &egui::Painter, rect: Rect, tokens: &Tokens, cell: &Value, column: &Column, row: &FamilyRow, glyphs: bool) -> Option<String> {
    let font = FontId::proportional(tokens.font_size);
    let at = pos2(rect.left() + tokens.cell_inset, rect.center().y);
    if is_missing(cell) {
        if glyphs {
            let r = rect.width().min(rect.height()) * 0.3;
            let stroke = Stroke::new(tokens.outline_width.max(1.5), tokens.error_text);
            p.circle_stroke(rect.center(), r, stroke);
            let d = vec2(r, -r) * std::f32::consts::FRAC_1_SQRT_2;
            p.line_segment([rect.center() - d, rect.center() + d], stroke);
        } else {
            p.text(at, Align2::LEFT_CENTER, "(missing glyph)", font, tokens.text_dim);
        }
        return None;
    }
    match cell_text(cell) {
        Some(text) => {
            let colour = if matches!(cell, Value::Null) {
                tokens.text_dim
            } else if row.same {
                tokens.text
            } else {
                tokens.accent
            };
            p.text(at, Align2::LEFT_CENTER, &text, font, colour);
            Some(text)
        }
        None => {
            crate::master_icons::paint_glyph(p, rect.shrink(tokens.cell_inset), &column.font, &row.glyph, &column.master, tokens.glyph_fill);
            let width = cell.get("width").and_then(Value::as_f64).unwrap_or(0.0);
            Some(format!("width {width}"))
        }
    }
}

fn draw_table(ui: &mut egui::Ui, tokens: &Tokens, t: &Table, positions: &[Option<usize>], pending: &mut Pending) {
    let line = tokens.list_row_height;
    let glyphs = t.attribute == "glyphs";
    let row_height = if glyphs { line * GLYPH_ROW_LINES } else { line };
    ui.horizontal(|ui| {
        ui.add_sized([NAME_WIDTH, line], egui::Label::new(egui::RichText::new("Glyph").strong()).truncate());
        for c in &t.columns {
            ui.add_sized([CELL_WIDTH, line], egui::Label::new(egui::RichText::new(&c.title).strong()).truncate());
        }
    });
    ui.separator();
    if t.rows.is_empty() {
        ui.colored_label(tokens.text_dim, "No glyph matches the filter.");
        return;
    }
    egui::ScrollArea::both().auto_shrink([false, true]).max_height(line * VISIBLE_ROWS).show_rows(ui, row_height, t.rows.len(), |ui, range| {
        for row in t.rows.get(range).unwrap_or_default() {
            ui.horizontal(|ui| {
                ui.add_sized([NAME_WIDTH, row_height], egui::Label::new(egui::RichText::new(&row.glyph).color(tokens.text)).truncate());
                for (i, (column, pos)) in t.columns.iter().zip(positions).enumerate() {
                    let cell = row.cells.get(i).unwrap_or(&NULL);
                    let (rect, resp) = ui.allocate_exact_size(vec2(CELL_WIDTH, row_height), Sense::click());
                    let text = draw_cell(&ui.painter_at(rect), rect, tokens, cell, column, row, glyphs);
                    if resp.double_clicked()
                        && !is_missing(cell)
                        && let Some(pos) = pos
                    {
                        pending.push(("compareFamily.openGlyph", json!({ "document": pos, "glyph": row.glyph })));
                    }
                    if let Some(text) = text {
                        resp.on_hover_text(text);
                    }
                }
            });
        }
    });
    if t.total > t.rows.len() {
        ui.colored_label(tokens.text_dim, format!("{} of {} glyphs are listed.", t.rows.len(), t.total));
    }
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let st = app.state.panels.compare_family.clone();
    if !st.open {
        return;
    }
    let positions: Vec<Option<usize>> =
        st.table.as_ref().map(|Snapshot(t)| t.columns.iter().map(|c| position(app, c.document)).collect()).unwrap_or_default();
    let tokens = &app.tokens;
    let columns = st.table.as_ref().map_or(1, |Snapshot(t)| t.columns.len().max(1));
    let mut open = true;
    egui::Window::new("Compare Family")
        .id(egui::Id::new("window.compareFamily"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .order(if st.pinned { egui::Order::Foreground } else { egui::Order::Middle })
        .default_width(NAME_WIDTH + CELL_WIDTH * columns as f32 + 2.0 * tokens.dialog_spacing)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                let caption = ATTRIBUTES.iter().find(|(id, _)| *id == st.attribute).map_or(st.attribute.as_str(), |(_, c)| *c);
                egui::ComboBox::from_id_salt("window.compareFamily.attribute").selected_text(caption).show_ui(ui, |ui| {
                    for (id, label) in ATTRIBUTES {
                        if ui.selectable_label(st.attribute == id, label).clicked() && st.attribute != id {
                            pending.push(("compareFamily.setAttribute", json!({ "attribute": id })));
                        }
                    }
                });
                let filter = FILTERS.iter().find(|(id, _)| *id == st.filter).map_or(st.filter.as_str(), |(_, c)| *c);
                ui.menu_button(format!("Filter: {filter}"), |ui| {
                    for (id, label) in FILTERS {
                        if ui.selectable_label(st.filter == id, label).clicked() {
                            if st.filter != id {
                                pending.push(("compareFamily.setFilter", json!({ "filter": id })));
                            }
                            ui.close();
                        }
                    }
                });
                if ui.selectable_label(st.pinned, "Keep on Top").on_hover_text("Keep this window above the document windows").clicked() {
                    pending.push(("compareFamily.togglePin", json!({})));
                }
                if ui.button("Update").on_hover_text("Read the glyphs of the open documents again").clicked() {
                    pending.push(("compareFamily.update", json!({})));
                }
            });
            ui.add_space(tokens.dialog_spacing);
            match &st.table {
                Some(Snapshot(t)) => draw_table(ui, tokens, t, &positions, pending),
                None => {
                    ui.colored_label(tokens.text_dim, "Press Update to read the open documents.");
                }
            }
        });
    if !open {
        app.state.panels.compare_family.open = false;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use fontcraft_engine::Session;
    use fontcraft_engine::model::{Component, Shape};

    use super::*;

    /// "Untitled" and "Other", which adds Aacute (a component of A); Untitled active.
    fn two_documents() -> FontcraftApp {
        let mut app = FontcraftApp::new(Session::with_document());
        let mut other = Font::new("Other");
        other.add_glyph("Aacute", vec![0xC1], 600.0).unwrap();
        other.glyph_mut("Aacute").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        app.session.add_document(other, None).unwrap();
        app.run("document.activate", &json!({"index": 0})).unwrap();
        app
    }

    fn report(app: &mut FontcraftApp, attribute: &str, filter: &str) -> Value {
        app.session.query("compareFamily.report", &json!({"attribute": attribute, "filter": filter})).unwrap()
    }

    fn draw(app: &mut FontcraftApp) {
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }

    #[test]
    fn edit_compare_family_shows_compare_family_report_and_set_attribute_set_filter_and_update_read_it_again() {
        let mut app = two_documents();
        let out = app.run("edit.compareFamily", &json!({})).unwrap();
        assert_eq!((out["open"].clone(), out["attribute"].clone(), out["filter"].clone()), (json!(true), json!("componentNames"), json!("all")));
        let expected = report(&mut app, "componentNames", "all");
        assert_eq!(out["rows"], expected["rows"], "the window lists what compareFamily.report returns");
        assert_eq!(out["total"], expected["total"]);
        assert_eq!(out["columns"].as_array().unwrap().len(), 2);
        assert_eq!(out["rows"], json!([{"glyph": "Aacute", "cells": [{"missing": true}, ["A"]], "same": false}]));
        draw(&mut app);

        let out = app.run("compareFamily.setAttribute", &json!({"attribute": "glyphs"})).unwrap();
        assert_eq!(out["attribute"], "glyphs");
        assert_eq!(out["total"], report(&mut app, "glyphs", "all")["total"]);
        draw(&mut app);
        assert!(app.run("compareFamily.setAttribute", &json!({"attribute": "colour"})).is_err());
        assert_eq!(app.state.panels.compare_family.attribute, "glyphs", "a refused command changes nothing");

        let out = app.run("compareFamily.setFilter", &json!({"filter": "missingInOne"})).unwrap();
        assert_eq!(out["rows"], report(&mut app, "glyphs", "missingInOne")["rows"]);
        assert_eq!(out["total"], 1);
        assert!(app.run("compareFamily.setFilter", &json!({"filter": "some"})).is_err());

        // Editing a document leaves the table as it was until Update.
        app.run("glyph.newGlyph", &json!({"name": "Q.alt"})).unwrap();
        let shown =
            |app: &FontcraftApp| app.state.panels.compare_family.table.as_ref().unwrap().0.rows.iter().map(|r| r.glyph.clone()).collect::<Vec<_>>();
        assert_eq!(shown(&app), ["Aacute"]);
        let out = app.run("compareFamily.update", &json!({})).unwrap();
        assert_eq!(out["total"], 2);
        assert_eq!(shown(&app), ["Q.alt", "Aacute"], "the first document's glyphs come first");
        draw(&mut app);
    }

    #[test]
    fn compare_family_toggle_pin_and_open_glyph_act_on_the_window_and_the_document() {
        let mut app = two_documents();
        app.run("edit.compareFamily", &json!({})).unwrap();
        assert_eq!(app.run("compareFamily.togglePin", &json!({})).unwrap()["pinned"], true);
        assert!(app.state.panels.compare_family.pinned);
        draw(&mut app);
        assert_eq!(app.run("compareFamily.togglePin", &json!({"on": false})).unwrap()["pinned"], false);

        // A glyph the document lacks is refused and the active document stays.
        let active = app.session.active().unwrap().id;
        assert!(app.run("compareFamily.openGlyph", &json!({"document": 0, "glyph": "Aacute"})).is_err());
        assert!(app.run("compareFamily.openGlyph", &json!({"document": 7, "glyph": "A"})).is_err());
        assert_eq!(app.session.active().unwrap().id, active);
        assert!(app.state.tab().is_none());

        let out = app.run("compareFamily.openGlyph", &json!({"document": "Untitled 2", "glyph": "Aacute"})).unwrap();
        assert_eq!(out["document"], 1);
        assert_eq!(app.session.active().unwrap().id, app.session.documents()[1].id);
        assert_eq!(app.state.editing(), Some("Aacute"));
    }
}
