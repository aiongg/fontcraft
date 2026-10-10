//! The bar above the glyphs, with the Grid View and List View buttons (`fontView.setViewMode`),
//! and the status area below them (`fontView.statusBar`): the selected, visible and total glyph
//! counts, and the cell size slider (`fontView.setCellSize`).

use serde_json::{Value, json};

use super::{MAX_CELL_SIZE, MIN_CELL_SIZE, ViewMode};
use crate::app::FontcraftApp;
use crate::icons::Icon;
use crate::panels::widgets;

type Pending = Vec<(&'static str, Value)>;

/// The counts as the status area prints them.
pub fn counts_text(selected: usize, visible: usize, total: usize) -> String {
    format!("{selected} selected · {visible} shown · {total} in font")
}

pub fn show(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let tokens = &app.tokens;
    let fv = &app.state.font_view;
    let has_font = app.session.font().is_ok();
    egui::Panel::top("fontView.bar").frame(egui::Frame::NONE.fill(tokens.panel).inner_margin(4.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (mode, icon, hover) in [(ViewMode::Grid, Icon::GridView, "Grid View"), (ViewMode::List, Icon::ListView, "List View")] {
                let id = widgets::id(format!("fontView.mode.{}", mode.as_str()));
                if widgets::icon_button(ui, id, icon, hover, has_font, fv.mode == mode) {
                    pending.push(("fontView.setViewMode", json!({"mode": mode.as_str()})));
                }
            }
        });
    });
    egui::Panel::bottom("fontView.status").frame(egui::Frame::NONE.fill(tokens.panel).inner_margin(4.0)).show(ui, |ui| {
        let cache = &app.font_view.cache;
        let counts = counts_text(app.state.selected_glyphs.len(), cache.visible.len(), cache.rows.len());
        ui.horizontal(|ui| {
            let row = ui.max_rect();
            if let Some(error) = &cache.error {
                ui.label(egui::RichText::new(error).color(tokens.error_text));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut size = fv.cell_size;
                let slider = egui::Slider::new(&mut size, MIN_CELL_SIZE..=MAX_CELL_SIZE).logarithmic(true).show_value(false);
                let response = ui.add_enabled(fv.mode == ViewMode::Grid && has_font, slider).on_hover_text("Cell Size");
                if response.changed() {
                    pending.push(("fontView.setCellSize", json!({"size": size})));
                }
            });
            ui.painter().text(row.center(), egui::Align2::CENTER_CENTER, counts, egui::FontId::proportional(tokens.font_size), tokens.text_dim);
        });
    });
}
