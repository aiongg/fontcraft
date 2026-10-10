//! Palette › Transformations: the origin row (`transform.setOrigin`), mirror, scale with its lock
//! (`transform.setScaleLock`), rotate, slant, align, and the booleans union, subtract and
//! intersect. The buttons act on the Edit View selection, or on the whole layer when nothing is
//! selected; in Font View on the selected glyphs. The booleans take the selected paths, a path
//! counting as selected when any of its nodes is. The two buttons beside a field apply its value
//! in opposite directions. Field values go through `transform.setSettings`; each button passes
//! them, and the origin, to its command.

use serde_json::{Map, Value, json};

use super::{METRICS, Pending, edit_targets, palette, stroke, widgets, with};
use crate::app::FontcraftApp;
use crate::icons::Icon;
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

/// The targets of boolean command `id`: in Edit View the selected paths (none: every path of the
/// layer), else the glyphs selected in Font View.
fn boolean_targets(app: &FontcraftApp, id: &str) -> Option<Map<String, Value>> {
    let mut t = targets(app, id)?;
    if app.state.tab().is_some() {
        t.remove("shapes");
        let paths = stroke::selected_paths(app);
        if !paths.is_empty() {
            t.insert("shapes".into(), json!(paths));
        }
    }
    Some(t)
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
        if widgets::icon_button(
            ui,
            widgets::id("transform.reference"),
            Icon::ReferencePoint,
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
        if widgets::icon_button(ui, widgets::id("transform.mirrorHorizontal"), Icon::MirrorHorizontal, "Mirror Horizontally", enabled, false) {
            press(pending, "transform.mirrorHorizontal", json!({ "origin": origin }));
        }
        if widgets::icon_button(ui, widgets::id("transform.mirrorVertical"), Icon::MirrorVertical, "Mirror Vertically", enabled, false) {
            press(pending, "transform.mirrorVertical", json!({ "origin": origin }));
        }
    });

    let (sx, sy) = settings.scale_percent();
    let [scale_x, _] = settings.scale;
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, widgets::id("transform.reverseScale"), Icon::ReverseScale, "Reverse Scale", enabled, false) {
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
        let lock = if settings.scale_locked { Icon::Locked } else { Icon::Unlocked };
        if widgets::icon_button(ui, widgets::id("transform.scaleLock"), lock, "Lock Scale Proportions", true, settings.scale_locked) {
            pending.push(("transform.setScaleLock", json!({ "locked": !settings.scale_locked })));
        }
        if widgets::icon_button(ui, widgets::id("transform.scale"), Icon::ScaleUp, "Scale", enabled, false) {
            press(pending, "transform.scale", json!({"x": sx, "y": sy, "origin": origin}));
        }
    });

    ui.horizontal(|ui| {
        let angle = json!({"angle": settings.rotate_angle, "origin": origin});
        if widgets::icon_button(
            ui,
            widgets::id("transform.rotateCounterclockwise"),
            Icon::RotateCounterclockwise,
            "Rotate Counterclockwise",
            enabled,
            false,
        ) {
            press(pending, "transform.rotateCounterclockwise", angle.clone());
        }
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("transform.rotate"), Some(settings.rotate_angle), width, false) {
            pending.push(("transform.setSettings", json!({ "rotate": v })));
        }
        if widgets::icon_button(ui, widgets::id("transform.rotateClockwise"), Icon::RotateClockwise, "Rotate Clockwise", enabled, false) {
            press(pending, "transform.rotateClockwise", angle);
        }
    });

    ui.horizontal_wrapped(|ui| {
        let angle = json!({"angle": settings.slant_angle, "origin": origin});
        if widgets::icon_button(ui, widgets::id("transform.slantLeft"), Icon::SlantLeft, "Slant Left", enabled, false) {
            press(pending, "transform.slantLeft", angle.clone());
        }
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("transform.slant"), Some(settings.slant_angle), width, false) {
            pending.push(("transform.setSettings", json!({ "slant": v })));
        }
        if widgets::icon_button(ui, widgets::id("transform.slantRight"), Icon::SlantRight, "Slant Right", enabled, false) {
            press(pending, "transform.slantRight", angle.clone());
        }
        if widgets::icon_button(ui, widgets::id("transform.slantDown"), Icon::SlantDown, "Slant Down", enabled, false) {
            press(pending, "transform.slantDown", angle.clone());
        }
        if widgets::icon_button(ui, widgets::id("transform.slantUp"), Icon::SlantUp, "Slant Up", enabled, false) {
            press(pending, "transform.slantUp", angle);
        }
    });

    ui.horizontal(|ui| {
        for (id, icon, hover) in [
            ("transform.alignLeft", Icon::AlignLeft, "Align Left"),
            ("transform.alignCenterHorizontal", Icon::AlignCenterHorizontal, "Align Horizontal Centers"),
            ("transform.alignRight", Icon::AlignRight, "Align Right"),
            ("transform.alignTop", Icon::AlignTop, "Align Top"),
            ("transform.alignCenterVertical", Icon::AlignCenterVertical, "Align Vertical Centers"),
            ("transform.alignBottom", Icon::AlignBottom, "Align Bottom"),
        ] {
            if widgets::icon_button(ui, widgets::id(id), icon, hover, align, false) {
                press(pending, id, json!({}));
            }
        }
    });

    ui.horizontal(|ui| {
        for (id, icon, hover) in [
            ("transform.booleanUnion", Icon::Union, "Union: merge the closed paths"),
            ("transform.booleanSubtract", Icon::Subtract, "Subtract the selected paths from the others"),
            ("transform.booleanIntersect", Icon::Intersect, "Intersect: keep where the selected paths overlap the others"),
        ] {
            if widgets::icon_button(ui, widgets::id(id), icon, hover, enabled, false)
                && let Some(t) = boolean_targets(app, id)
            {
                pending.push((id, Value::Object(t)));
            }
        }
    });

    if !enabled {
        palette::note(app, ui, "Select glyphs or open one in Edit View");
    }
}
