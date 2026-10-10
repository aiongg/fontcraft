//! Palette › Fit Curve: the two range fields (`fitCurve.setRange`), minus, the eight step buttons
//! (`fitCurve.applyStep`), plus, and balance, all on the curve segments of the Edit View
//! selection; in Font View on those of every selected glyph, as it last had them selected in
//! Edit View. A marker under the steps shows the selected handles' length
//! (`fitCurve.getHandleLength`).

use fontcraft_engine::cmd::fit_curve;
use fontcraft_model::{Font, Shape};
use serde_json::{Map, Value, json};

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

/// In Font View, the targets of each selected glyph: its layer of Font View's master with the
/// nodes it last had selected in Edit View that the layer still has (`fontView.batch.palette`).
pub fn font_view_targets(app: &FontcraftApp) -> Vec<Map<String, Value>> {
    let (Ok(font), Some(master)) = (app.session.font(), crate::shell::font_view::master(app)) else { return Vec::new() };
    let kept = &app.state.font_view.node_selections;
    app.state
        .selected_glyphs
        .iter()
        .filter_map(|name| {
            let remembered = kept.get(name)?;
            let layer = font.glyph(name)?.layer(&master)?;
            let points: Vec<Value> = remembered
                .points
                .iter()
                .filter(|&&(shape, node)| match layer.shapes.get(shape) {
                    Some(Shape::Path(p)) => node < p.nodes.len(),
                    _ => false,
                })
                .map(|&(shape, node)| json!([shape, node]))
                .collect();
            if points.is_empty() {
                return None;
            }
            let mut m = Map::new();
            m.insert("glyph".into(), json!(name));
            m.insert("layer".into(), json!(master));
            m.insert("points".into(), Value::Array(points));
            Some(m)
        })
        .collect()
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let [min, max] = range(app);
    let targets = edit_targets(app, "fitCurve.applyStep").filter(|t| t.contains_key("points"));
    let current = targets
        .clone()
        .and_then(|t| app.session.execute("fitCurve.getHandleLength", &Value::Object(t)).ok())
        .and_then(|r| r.get("percent").and_then(Value::as_f64));
    let batch = if app.state.tab().is_none() { font_view_targets(app) } else { Vec::new() };
    let enabled = targets.is_some() || !batch.is_empty();
    // In Font View every selected glyph is one step of a batch, so the whole change is one undo
    // step.
    let run = |pending: &mut Pending, id: &'static str, extra: Value| {
        if let Some(t) = &targets {
            pending.push((id, with(t.clone(), extra)));
        } else if !batch.is_empty() {
            let label = fontcraft_engine::find_command(id).map_or(id, |c| c.label);
            let steps: Vec<Value> = batch.iter().map(|t| json!({"command": id, "params": with(t.clone(), extra.clone())})).collect();
            pending.push(("command.batch", json!({"label": label, "steps": steps})));
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
        let hint = if app.state.tab().is_some() { "Select curve segments" } else { "Select glyphs whose curve segments were selected in Edit View" };
        palette::note(app, ui, hint);
    }
}
