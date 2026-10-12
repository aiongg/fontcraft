//! The font comparison table (`font.compareInTable`, Window › Compare Fonts in List): one row per
//! glyph and one column per chosen open font for a property such as the advance width, so the
//! fonts of a family can be read across a row. Rows whose values differ are drawn in the accent
//! colour; Only Differences hides the others.
//!
//! The table comes from `fontcraft_engine::cmd::encodings::compare_documents` and is rebuilt only
//! when a compared document changes or another property is chosen (kept in egui's memory, not in
//! the saved state).

use std::sync::Arc;

use fontcraft_engine::cmd::encodings::{COMPARE_PROPERTIES, CompareRow, compare_table, resolve_documents};
use fontcraft_engine::model::Font;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};

/// Width of the glyph-name column and of each font's column, in points.
const NAME_WIDTH: f32 = 160.0;
const VALUE_WIDTH: f32 = 110.0;

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareTableState {
    pub open: bool,
    /// The compared documents, by session id, in column order.
    pub docs: Vec<u64>,
    /// A [`COMPARE_PROPERTIES`] id.
    pub property: String,
    pub only_differences: bool,
}

impl Default for CompareTableState {
    fn default() -> Self {
        Self { open: false, docs: Vec::new(), property: "width".into(), only_differences: false }
    }
}

pub const COMMANDS: &[UiCommand] = &[(
    "font.compareInTable",
    "Compare Fonts in List",
    &["Window"],
    "",
    "{fonts?: [fontRef] (positions in the open-document list, titles or paths; default every open document), property?: \"width\" | \"lsb\" | \"rsb\" | \"nodes\" | \"paths\" | \"components\" | \"anchors\" | \"unicodes\" (default the last one shown, else width)} opens a table with one row per glyph and one column per font showing the property in each font's first master → {fonts: [titles], property, rows: [{glyph, values: [value or null per font], differ}]}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "font.compareInTable" {
        return Err(format!("unknown command `{id}`"));
    }
    let docs = resolve_documents(&app.session, args.get("fonts")).map_err(|e| args.err(e))?;
    if docs.is_empty() {
        return Err(args.err("no document is open"));
    }
    let property = args.string("property")?.map(str::to_string).unwrap_or_else(|| app.state.panels.compare_table.property.clone());
    let table = fontcraft_engine::cmd::encodings::compare_documents(&app.session, &docs, &property).map_err(|e| args.err(e))?;
    let st = &mut app.state.panels.compare_table;
    st.open = true;
    st.docs = docs;
    st.property = property;
    Ok(table)
}

/// The table as last built, with what it was built from.
struct Built {
    /// Each compared document's id and revision, and the property.
    key: (Vec<(u64, u64)>, String),
    titles: Vec<String>,
    rows: Vec<CompareRow>,
    error: Option<String>,
}

fn build(app: &FontcraftApp, key: (Vec<(u64, u64)>, String)) -> Built {
    let docs: Vec<_> = key.0.iter().filter_map(|(id, _)| app.session.documents().iter().find(|d| d.id == *id)).collect();
    let fonts: Vec<&Font> = docs.iter().map(|d| d.font.as_ref()).collect();
    let titles = docs.iter().map(|d| d.title.clone()).collect();
    let (rows, error) = match compare_table(&fonts, &key.1) {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    Built { key, titles, rows, error }
}

/// A value as the table shows it: whole numbers without decimals, nothing for a missing value.
fn shown(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{f:.0}"),
            Some(f) => format!("{f:.2}"),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.compare_table.open {
        return;
    }
    let st = app.state.panels.compare_table.clone();
    let revisions: Vec<(u64, u64)> =
        st.docs.iter().filter_map(|id| app.session.documents().iter().find(|d| d.id == *id).map(|d| (d.id, d.revision))).collect();
    let key = (revisions, st.property.clone());
    let memory_id = egui::Id::new("font.compareInTable.table");
    let cached: Option<Arc<Built>> = ui.ctx().data(|d| d.get_temp::<Arc<Built>>(memory_id));
    let built = match cached {
        Some(b) if b.key == key => b,
        _ => {
            let b = Arc::new(build(app, key));
            ui.ctx().data_mut(|d| d.insert_temp(memory_id, b.clone()));
            b
        }
    };
    let tokens = &app.tokens;
    let positions: Vec<Value> = st.docs.iter().filter_map(|id| app.session.documents().iter().position(|d| d.id == *id)).map(|i| json!(i)).collect();
    let mut open = true;
    egui::Window::new("Compare Fonts in List")
        .id(egui::Id::new("font.compareInTable"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(NAME_WIDTH + VALUE_WIDTH * built.titles.len().max(1) as f32 + 2.0 * tokens.dialog_spacing)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                let caption = COMPARE_PROPERTIES.iter().find(|(id, _)| *id == st.property).map_or(st.property.as_str(), |(_, c)| *c);
                egui::ComboBox::from_id_salt("font.compareInTable.property").selected_text(caption).show_ui(ui, |ui| {
                    for (id, label) in COMPARE_PROPERTIES {
                        if ui.selectable_label(st.property == id, label).clicked() && st.property != id {
                            pending.push(("font.compareInTable", json!({"fonts": positions, "property": id})));
                        }
                    }
                });
                let mut only = st.only_differences;
                if ui.checkbox(&mut only, "Only Differences").changed() {
                    app.state.panels.compare_table.only_differences = only;
                }
            });
            ui.add_space(tokens.dialog_spacing);
            if let Some(e) = &built.error {
                ui.colored_label(tokens.error_text, e);
                return;
            }
            if built.titles.is_empty() {
                ui.colored_label(tokens.text_dim, "The compared fonts are closed.");
                return;
            }
            let row_height = tokens.list_row_height;
            ui.horizontal(|ui| {
                ui.add_sized([NAME_WIDTH, row_height], egui::Label::new(egui::RichText::new("Glyph").strong()));
                for title in &built.titles {
                    ui.add_sized([VALUE_WIDTH, row_height], egui::Label::new(egui::RichText::new(title).strong()).truncate());
                }
            });
            ui.separator();
            let rows: Vec<&CompareRow> = built.rows.iter().filter(|r| !st.only_differences || r.differ).collect();
            egui::ScrollArea::both().auto_shrink([false, false]).show_rows(ui, row_height, rows.len(), |ui, range| {
                for row in rows.get(range).unwrap_or_default() {
                    let colour = if row.differ { tokens.accent } else { tokens.text };
                    ui.horizontal(|ui| {
                        ui.add_sized([NAME_WIDTH, row_height], egui::Label::new(egui::RichText::new(&row.glyph).color(tokens.text)).truncate());
                        for v in &row.values {
                            let text = if v.is_null() {
                                egui::RichText::new("–").color(tokens.text_dim)
                            } else {
                                egui::RichText::new(shown(v)).color(colour)
                            };
                            ui.add_sized([VALUE_WIDTH, row_height], egui::Label::new(text));
                        }
                    });
                }
            });
        });
    if !open {
        app.state.panels.compare_table.open = false;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn compare_in_table_opens_with_a_width_column_per_font() {
        let mut app = FontcraftApp::new(fontcraft_engine::Session::with_document());
        let mut other = Font::new("Other");
        other.glyph_mut("A").unwrap().layers[0].width = 640.0;
        app.session.add_document(other, None).unwrap();
        let r = app.run("font.compareInTable", &json!({"fonts": [0, 1]})).unwrap();
        assert_eq!(r["property"], "width");
        assert_eq!(r["fonts"].as_array().unwrap().len(), 2);
        let a = r["rows"].as_array().unwrap().iter().find(|row| row["glyph"] == "A").unwrap();
        assert_eq!(a["values"], json!([600.0, 640.0]));
        assert!(app.state.panels.compare_table.open);

        let r = app.run("font.compareInTable", &json!({"property": "nodes"})).unwrap();
        assert_eq!(r["property"], "nodes");
        assert!(app.run("font.compareInTable", &json!({"property": "colour"})).is_err());
        assert_eq!(app.state.panels.compare_table.property, "nodes", "a refused command changes nothing");

        // The window draws in a headless frame.
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let _ = ctx.run_ui(input, |ui| app.ui(ui));
    }
}
