//! Palette › Fit Curve: the two range fields (`fitCurve.setRange`), minus, the eight step buttons
//! (`fitCurve.applyStep`), plus, and balance, all on the curve segments of the Edit View
//! selection. A marker under the steps shows the selected handles' length
//! (`fitCurve.getHandleLength`).

use fontcraft_engine::cmd::fit_curve;
use fontcraft_model::Font;
use serde_json::{Value, json};

use super::{Pending, edit_targets, palette, widgets, with};
use crate::app::FontcraftApp;

/// The range the font sets, else the default range of the `fitCurve` commands.
pub fn font_range(font: Option<&Font>) -> [f64; 2] {
    let (min, max) = font.and_then(fit_curve::font_range).unwrap_or(fit_curve::DEFAULT_RANGE);
    [min, max]
}

/// The range fields as shown: set with `fitCurve.setRange`, else the font's.
pub fn range(app: &FontcraftApp) -> [f64; 2] {
    app.state.panels.fit_curve_range.unwrap_or_else(|| font_range(app.session.font().ok()))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let [min, max] = range(app);
    let targets = edit_targets(app, "fitCurve.applyStep").filter(|t| t.contains_key("points"));
    let current = targets
        .clone()
        .and_then(|t| app.session.execute("fitCurve.getHandleLength", &Value::Object(t)).ok())
        .and_then(|r| r.get("percent").and_then(Value::as_f64));
    let enabled = targets.is_some();
    let run = |pending: &mut Pending, id: &'static str, extra: Value| {
        if let Some(t) = &targets {
            pending.push((id, with(t.clone(), extra)));
        }
    };
    let range = json!({"min": min, "max": max});
    let tokens = app.tokens.clone();
    ui.horizontal(|ui| {
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("fitCurve.min"), Some(min), tokens.field_width, false) {
            pending.push(("fitCurve.setRange", json!({ "min": v })));
        }
        if widgets::button(ui, widgets::id("fitCurve.balance"), "=", "Balance Handles", enabled, false) {
            run(pending, "fitCurve.balanceHandles", json!({}));
        }
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("fitCurve.max"), Some(max), tokens.field_width, false) {
            pending.push(("fitCurve.setRange", json!({ "max": v })));
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tokens.fit_curve_gap;
        if widgets::button(ui, widgets::id("fitCurve.decrease"), "−", "Decrease Handle Length", enabled, false) {
            run(pending, "fitCurve.decrease", range.clone());
        }
        let mut steps: Option<(f32, f32)> = None;
        for step in 1..=8u8 {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(tokens.fit_curve_step, tokens.fit_curve_step), egui::Sense::hover());
            let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
            let percent = min + f64::from(step - 1) * (max - min) / 7.0;
            let response = ui
                .interact(rect, widgets::id(("fitCurve.step", step)), sense)
                .on_hover_text(format!("Fit Curve: {}%", widgets::format_number(percent)));
            let color = if !enabled {
                tokens.text_dim
            } else if response.hovered() {
                tokens.accent
            } else {
                tokens.text
            };
            ui.painter().circle_stroke(rect.center(), tokens.fit_curve_step * 0.35, egui::Stroke::new(1.5, color));
            steps = Some((steps.map_or(rect.center().x, |s| s.0), rect.center().x));
            if enabled && response.clicked() {
                run(pending, "fitCurve.applyStep", json!({"step": step, "min": min, "max": max}));
            }
        }
        if widgets::button(ui, widgets::id("fitCurve.increase"), "+", "Increase Handle Length", enabled, false) {
            run(pending, "fitCurve.increase", range.clone());
        }
        // The grey marker: the selected handles' length on the scale of the eight steps.
        if let (Some((first, last)), Some(p)) = (steps, current)
            && max > min
        {
            let t = ((p - min) / (max - min)).clamp(0.0, 1.0) as f32;
            let x = first + t * (last - first);
            let y = ui.min_rect().bottom() + tokens.fit_curve_step * 0.2;
            let s = tokens.fit_curve_step * 0.18;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![egui::pos2(x, y - s), egui::pos2(x - s, y + s), egui::pos2(x + s, y + s)],
                tokens.text_dim,
                egui::Stroke::NONE,
            ));
        }
    });
    ui.add_space(tokens.fit_curve_step * 0.4);
    if !enabled {
        palette::note(app, ui, "Select curve segments in Edit View");
    }
}
