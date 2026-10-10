//! The Palette sidebar: the built-in sections in order, each under a title row whose chevron
//! collapses it (`palette.toggleSection`), then the contextual area, which shows only while
//! Show Info is on (`view.showInfoAffectsPalette`).

use serde_json::json;

use super::{Pending, SECTIONS, dimensions, fit_curve, layers, transformations, widgets};
use crate::app::FontcraftApp;

/// Draws one section's contents.
type Draw = fn(&mut FontcraftApp, &mut egui::Ui, &mut Pending);

fn drawer(section: &str) -> Option<Draw> {
    match section {
        "dimensions" => Some(dimensions::show),
        "fitCurve" => Some(fit_curve::show),
        "layers" => Some(layers::show),
        "transformations" => Some(transformations::show),
        _ => None,
    }
}

/// The contextual sections, shown at the foot of the Palette while Show Info is on (id, title,
/// contents). Stroke joins them when `path.setStroke` exists.
const CONTEXTUAL: &[(&str, &str, Draw)] = &[];

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let frame = egui::Frame::NONE.fill(app.tokens.panel).inner_margin(app.tokens.palette_margin);
    let width = app.tokens.palette_width;
    egui::Panel::right("palette").frame(frame).default_size(width).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (id, title) in SECTIONS {
                let expanded = !app.state.panels.collapsed.iter().any(|c| c == id);
                if widgets::section_header(ui, &app.tokens, id, title, expanded) {
                    pending.push(("palette.toggleSection", json!({"section": id, "expanded": !expanded})));
                }
                if expanded && let Some(draw) = drawer(id) {
                    ui.push_id(id, |ui| draw(app, ui, pending));
                }
                ui.separator();
            }
            if app.state.display.info {
                for (id, title, draw) in CONTEXTUAL {
                    ui.label(egui::RichText::new(*title).strong());
                    ui.push_id(*id, |ui| draw(app, ui, pending));
                }
            }
        });
    });
}

/// A dimmed note in a section, for "nothing to show".
pub fn note(app: &FontcraftApp, ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(app.tokens.text_dim));
}
