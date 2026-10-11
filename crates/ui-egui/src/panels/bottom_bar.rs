//! The Edit View bottom bar, at the right: the writing direction buttons
//! (`editView.setWritingDirection`), the kerning mode button (`kerning.setMode`, a click goes to
//! the next mode), then zoom out, the zoom field (screen points per 1000 units) and zoom in
//! (`view.zoomOut`, `view.setZoom`, `view.zoomIn`). The features menu and the preview button
//! have no commands yet and are not drawn.

use fontcraft_textlayout::KerningMode;
use serde_json::json;

use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::icons::Icon;
use crate::state::WritingDirection;

/// The direction buttons, in the order they are shown.
const DIRECTIONS: [(WritingDirection, Icon, &str); 3] = [
    (WritingDirection::Ltr, Icon::LeftToRight, "Left to Right"),
    (WritingDirection::Rtl, Icon::RightToLeft, "Right to Left"),
    (WritingDirection::Ttb, Icon::TopToBottom, "Top to Bottom"),
];

fn kerning_button(mode: KerningMode) -> (Icon, &'static str) {
    match mode {
        KerningMode::None => (Icon::KerningOff, "No kerning (click for kerning)"),
        KerningMode::Kerning => (Icon::KerningOn, "Kerning (click for locked kerning)"),
        KerningMode::Locked => (Icon::KerningLocked, "Locked kerning (click for no kerning)"),
    }
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab() else { return };
    let (tab, zoom, direction, kerning) = (tab.id, tab.view.scale * 1000.0, tab.direction, tab.kerning);
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
                ui.separator();
                let (icon, hover) = kerning_button(kerning);
                if widgets::icon_button(ui, widgets::id("bottomBar.kerningMode"), icon, hover, true, false) {
                    pending.push(("kerning.setMode", json!({ "tab": tab })));
                }
                ui.separator();
                for (d, icon, hover) in DIRECTIONS.iter().rev() {
                    if widgets::icon_button(ui, widgets::id(("bottomBar.direction", d.name())), *icon, hover, true, *d == direction) {
                        pending.push(("editView.setWritingDirection", json!({"tab": tab, "direction": d.name()})));
                    }
                }
            });
        });
    });
}
