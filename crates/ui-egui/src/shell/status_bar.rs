//! The status bar: the document title, the last command's result, and the zoom value.

use crate::app::FontcraftApp;

pub fn show(app: &FontcraftApp, ui: &mut egui::Ui) {
    let tokens = &app.tokens;
    let frame = egui::Frame::NONE.fill(tokens.panel_darker).inner_margin(4.0);
    let title = app.session.active().map(|d| format!("{}{}", d.title, if d.modified() { " •" } else { "" })).unwrap_or_else(|| "No document".into());
    egui::Panel::bottom("status_bar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title).color(tokens.text));
            ui.separator();
            ui.label(egui::RichText::new(&app.state.status).color(tokens.text_dim));
            if let Some(tab) = app.state.tab() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{:.0}", tab.view.scale * 1000.0)).color(tokens.text_dim));
                });
            }
        });
    });
}
