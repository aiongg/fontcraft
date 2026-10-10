//! The Edit View bottom bar: zoom out, the zoom field (screen points per 1000 units) and zoom in,
//! at the right (`view.zoomOut`, `view.setZoom`, `view.zoomIn`). The features menu, the preview
//! button, the writing direction and kerning mode buttons have no commands yet and are not drawn.

use serde_json::json;

use super::{Pending, widgets};
use crate::app::FontcraftApp;

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab() else { return };
    let (tab, zoom) = (tab.id, tab.view.scale * 1000.0);
    let tokens = &app.tokens;
    let frame = egui::Frame::NONE.fill(tokens.app_bar).inner_margin(tokens.bottom_bar_margin);
    egui::Panel::bottom("edit_view_bottom_bar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, widgets::id("bottomBar.zoomIn"), "+", "Zoom In", true, false) {
                    pending.push(("view.zoomIn", json!({ "tab": tab })));
                }
                if let Some(widgets::NumberEdit::Set(size)) =
                    widgets::number(ui, widgets::id("bottomBar.zoom"), Some(zoom.round()), tokens.field_width, false)
                {
                    pending.push(("view.setZoom", json!({"tab": tab, "size": size})));
                }
                if widgets::button(ui, widgets::id("bottomBar.zoomOut"), "−", "Zoom Out", true, false) {
                    pending.push(("view.zoomOut", json!({ "tab": tab })));
                }
            });
        });
    });
}
