//! Palette › Transformations: the origin row (`transform.setOrigin`), mirror, scale with its lock
//! (`transform.setScaleLock`), rotate, slant and align. The buttons act on the Edit View
//! selection, or on the whole layer when nothing is selected; in Font View on the selected
//! glyphs. The two buttons beside a field apply its value in opposite directions. Field values
//! go through `transform.setSettings`; each button passes them, and the origin, to its command.

use serde_json::{Map, Value, json};

use super::{METRICS, Pending, edit_targets, palette, widgets, with};
use crate::app::FontcraftApp;
use crate::shell::transform::OriginMode;

/// The targets of transform command `id`: the Edit View selection (the whole edited layer when it
/// is empty), else the glyphs selected in Font View.
fn targets(app: &FontcraftApp, id: &str) -> Option<Map<String, Value>> {
    if app.state.tab().is_some() {
        return edit_targets(app, id);
    }
    if app.state.selected_glyphs.is_empty() {
        return None;
    }
    let mut m = Map::new();
    m.insert("glyphs".into(), json!(app.state.selected_glyphs));
    Some(m)
}

fn metric_label(metric: &str) -> &'static str {
    match metric {
        "halfXHeight" => "Half x-height",
        "xHeight" => "x-height",
        "halfCapHeight" => "Half cap height",
        "capHeight" => "Cap height",
        _ => "Baseline",
    }
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let settings = app.state.transform.clone();
    let tokens = app.tokens.clone();
    let width = tokens.field_width;
    let any = targets(app, "transform.scale");
    let enabled = any.is_some();
    // Align needs one glyph's layer: Edit View only.
    let align = app.state.tab().is_some() && enabled;
    let origin = settings.origin_param();
    let press = |pending: &mut Pending, id: &'static str, extra: Value| {
        if let Some(t) = targets(app, id) {
            pending.push((id, with(t, extra)));
        }
    };

    ui.horizontal(|ui| {
        let active = (settings.mode == OriginMode::Box).then_some(settings.box_point.as_str());
        if let Some(point) = widgets::point_grid(ui, &tokens, "transform.origin", active) {
            pending.push(("transform.setOrigin", json!({"mode": "box", "boxPoint": point})));
        }
        let reference = settings.point;
        if widgets::button(
            ui,
            widgets::id("transform.reference"),
            "⌖",
            "Reference point",
            reference.is_some(),
            settings.mode == OriginMode::Reference,
        ) && let Some(p) = reference
        {
            pending.push(("transform.setOrigin", json!({"mode": "reference", "point": p})));
        }
        let selected = if settings.mode == OriginMode::Metrics { metric_label(&settings.metric) } else { "Metrics" };
        egui::ComboBox::from_id_salt(widgets::id("transform.metric")).selected_text(selected).show_ui(ui, |ui| {
            for metric in METRICS {
                let on = settings.mode == OriginMode::Metrics && settings.metric == metric;
                if ui.add(egui::Button::selectable(on, metric_label(metric))).clicked() {
                    pending.push(("transform.setOrigin", json!({"mode": "metrics", "metric": metric})));
                }
            }
        });
    });

    ui.horizontal(|ui| {
        if widgets::button(ui, widgets::id("transform.mirrorHorizontal"), "↔", "Mirror Horizontally", enabled, false) {
            press(pending, "transform.mirrorHorizontal", json!({ "origin": origin }));
        }
        if widgets::button(ui, widgets::id("transform.mirrorVertical"), "↕", "Mirror Vertically", enabled, false) {
            press(pending, "transform.mirrorVertical", json!({ "origin": origin }));
        }
    });

    let (sx, sy) = settings.scale_percent();
    let [scale_x, _] = settings.scale;
    ui.horizontal(|ui| {
        if widgets::button(ui, widgets::id("transform.reverseScale"), "÷", "Reverse Scale", enabled, false) {
            press(pending, "transform.reverseScale", json!({"x": sx, "y": sy, "origin": origin}));
        }
        ui.vertical(|ui| {
            if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("transform.scaleX"), Some(scale_x), width, false) {
                pending.push(("transform.setSettings", json!({ "scaleX": v })));
            }
            ui.add_enabled_ui(!settings.scale_locked, |ui| {
                if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("transform.scaleY"), Some(sy), width, false) {
                    pending.push(("transform.setSettings", json!({ "scaleY": v })));
                }
            });
        });
        let lock = if settings.scale_locked { "🔒" } else { "🔓" };
        if widgets::button(ui, widgets::id("transform.scaleLock"), lock, "Lock Scale Proportions", true, settings.scale_locked) {
            pending.push(("transform.setScaleLock", json!({ "locked": !settings.scale_locked })));
        }
        if widgets::button(ui, widgets::id("transform.scale"), "×", "Scale", enabled, false) {
            press(pending, "transform.scale", json!({"x": sx, "y": sy, "origin": origin}));
        }
    });

    ui.horizontal(|ui| {
        let angle = json!({"angle": settings.rotate_angle, "origin": origin});
        if widgets::button(ui, widgets::id("transform.rotateCounterclockwise"), "⟲", "Rotate Counterclockwise", enabled, false) {
            press(pending, "transform.rotateCounterclockwise", angle.clone());
        }
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("transform.rotate"), Some(settings.rotate_angle), width, false) {
            pending.push(("transform.setSettings", json!({ "rotate": v })));
        }
        if widgets::button(ui, widgets::id("transform.rotateClockwise"), "⟳", "Rotate Clockwise", enabled, false) {
            press(pending, "transform.rotateClockwise", angle);
        }
    });

    ui.horizontal_wrapped(|ui| {
        let angle = json!({"angle": settings.slant_angle, "origin": origin});
        if widgets::button(ui, widgets::id("transform.slantLeft"), "⇠", "Slant Left", enabled, false) {
            press(pending, "transform.slantLeft", angle.clone());
        }
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("transform.slant"), Some(settings.slant_angle), width, false) {
            pending.push(("transform.setSettings", json!({ "slant": v })));
        }
        if widgets::button(ui, widgets::id("transform.slantRight"), "⇢", "Slant Right", enabled, false) {
            press(pending, "transform.slantRight", angle.clone());
        }
        if widgets::button(ui, widgets::id("transform.slantDown"), "⇣", "Slant Down", enabled, false) {
            press(pending, "transform.slantDown", angle.clone());
        }
        if widgets::button(ui, widgets::id("transform.slantUp"), "⇡", "Slant Up", enabled, false) {
            press(pending, "transform.slantUp", angle);
        }
    });

    ui.horizontal(|ui| {
        for (id, label, hover) in [
            ("transform.alignLeft", "⇤", "Align Left"),
            ("transform.alignCenterHorizontal", "⇿", "Align Horizontal Centers"),
            ("transform.alignRight", "⇥", "Align Right"),
            ("transform.alignTop", "⤒", "Align Top"),
            ("transform.alignCenterVertical", "⇳", "Align Vertical Centers"),
            ("transform.alignBottom", "⤓", "Align Bottom"),
        ] {
            if widgets::button(ui, widgets::id(id), label, hover, align, false) {
                press(pending, id, json!({}));
            }
        }
    });

    if !enabled {
        palette::note(app, ui, "Select glyphs or open one in Edit View");
    }
}
