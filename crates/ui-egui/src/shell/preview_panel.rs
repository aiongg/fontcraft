//! The Preview Panel: a window with the same preview as the Preview Area, for the showing Edit
//! View tab. It stays blank while Font View shows and fills in as soon as a tab shows
//! (`preview.panelBlankWithoutEditTab`). Its commands live in [`super::preview`].

use egui::{Rect, Sense, pos2, vec2};
use serde_json::json;

use super::preview::{self, Target};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Draws the Preview Panel while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.preview.panel_open {
        return;
    }
    let tab = app.state.tab().cloned();
    let options = *app.state.panels.preview.options(Target::Panel);
    let ([w, h], row, ground) = (app.tokens.preview_panel_size, app.tokens.preview_controls_height, app.tokens.panel_darker);
    let mut open = true;
    egui::Window::new("Preview Panel")
        .id(egui::Id::new("window.previewPanel"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(w, h))
        .min_size(vec2(row * 4.0, row * 2.0))
        .show(&ui.ctx().clone(), |ui| {
            let area = ui.available_rect_before_wrap();
            let full = Rect::from_min_size(area.min, area.size().max(vec2(row * 4.0, row * 2.0)));
            ui.allocate_rect(full, Sense::hover());
            let controls = Rect::from_min_max(pos2(full.min.x, full.max.y - row), full.max);
            let strip = Rect::from_min_max(full.min, pos2(full.max.x, controls.min.y));
            match &tab {
                Some(tab) => preview::strip(app, ui, strip, Target::Panel, tab, pending),
                None => {
                    ui.painter_at(strip).rect_filled(strip, 0.0, ground);
                }
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| preview::controls(ui, Target::Panel, &options, pending));
        });
    if !open {
        pending.push(("window.previewPanel", json!({"on": false})));
    }
}
