//! Palette › Measurements (`palette.measurements`): the design values of the active master (Font
//! View: the first). Each metric shows its height and its overshoot (the zone), each stem
//! definition its width; typing a number writes it (`font.master.setMetric`,
//! `font.master.setStem`). Recalculate measures them again from the master's outlines
//! (`font.recalculateMeasurements`).

use serde_json::{Value, json};

use super::{Pending, palette, widgets};
use crate::app::FontcraftApp;

/// Most metric and stem rows the section lists; a font may define 256 of each.
const MAX_ROWS: usize = 64;

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let master = {
        let Ok(font) = app.session.font() else {
            palette::note(app, ui, "No font master");
            return;
        };
        let active = app.state.tab().and_then(|tab| crate::canvas::view::active_master(font, tab)).or_else(|| font.masters.first());
        let Some(m) = active else {
            palette::note(app, ui, "No font master");
            return;
        };
        m.id.clone()
    };
    let metrics = app.session.query("font.metrics.list", &json!({"master": master})).ok();
    let stems = app.session.query("font.stems.list", &json!({"master": master})).ok();
    let rows = |v: &Option<Value>, key: &str| -> Vec<Value> {
        v.as_ref().and_then(|v| v.get(key)).and_then(Value::as_array).map(|a| a.iter().take(MAX_ROWS).cloned().collect()).unwrap_or_default()
    };
    let (metrics, stems) = (rows(&metrics, "metrics"), rows(&stems, "stems"));
    let width = app.tokens.field_width;
    egui::Grid::new("measurements").num_columns(3).show(ui, |ui| {
        for m in &metrics {
            let Some(index) = m.get("index").and_then(Value::as_u64) else { continue };
            let label = m.get("name").and_then(Value::as_str).or_else(|| m.get("label").and_then(Value::as_str)).unwrap_or("Metric");
            ui.label(label);
            let position = m.get("position").and_then(Value::as_f64);
            if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, widgets::id(("measurements.position", index)), position, width, false) {
                pending.push(("font.master.setMetric", json!({"masters": [master], "metric": index, "position": n})));
            }
            // Italic Angle has no zone.
            match m.get("zone").and_then(Value::as_f64) {
                Some(zone) => {
                    if let Some(widgets::NumberEdit::Set(n)) =
                        widgets::number(ui, widgets::id(("measurements.zone", index)), Some(zone), width, false)
                    {
                        pending.push(("font.master.setMetric", json!({"masters": [master], "metric": index, "zone": n})));
                    }
                }
                None => {
                    ui.label("");
                }
            }
            ui.end_row();
        }
        for s in &stems {
            let Some(index) = s.get("index").and_then(Value::as_u64) else { continue };
            let name = s.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or("Stem");
            let mark = if s.get("orientation").and_then(Value::as_str) == Some("horizontal") { "↕" } else { "↔" };
            ui.label(format!("{mark} {name}"));
            let value = s.get("value").and_then(Value::as_f64);
            if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, widgets::id(("measurements.stem", index)), value, width, false) {
                pending.push(("font.master.setStem", json!({"masters": [master], "stem": index, "value": n})));
            }
            ui.label("");
            ui.end_row();
        }
    });
    if metrics.is_empty() && stems.is_empty() {
        palette::note(app, ui, "No metrics or stems defined");
    }
    if widgets::button(
        ui,
        widgets::id("measurements.recalculate"),
        "Recalculate",
        "Measure these values again from the master's outlines",
        true,
        false,
    ) {
        pending.push(("font.recalculateMeasurements", json!({"masters": [master]})));
    }
}
