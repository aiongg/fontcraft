//! Measurements: the design values of the shown master. Each metric shows its height and its overshoot zone, each stem
//! its width; typing a number writes it (`font.master.setMetric`, `font.master.setStem`). Recalculate measures them again
//! from the master's outlines (`font.recalculateMeasurements`).

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::glyph_sections::MeasurementsModel;

craft::ui::words! {
    /// The words of the Measurements panel.
    pub struct MeasurementsWords {
        no_master: "fontcraftMeasurements.noMaster" = "No font master";
        empty: "fontcraftMeasurements.empty" = "No metrics or stems defined";
        /// A metric without a name or label.
        metric: "fontcraftMeasurements.metric" = "Metric";
        /// A stem without a name.
        stem: "fontcraftMeasurements.stem" = "Stem";
        /// After a metric's name: its overshoot field.
        zone: "fontcraftMeasurements.zone" = "zone";
        recalculate: "fontcraftMeasurements.recalculate" = "Recalculate";
        recalculate_tip: "fontcraftMeasurements.recalculateTip" = "Measure these values again from the master's outlines";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<MeasurementsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

/// A number field over `value` that gives the number committed.
fn number(ui: &mut Ui, label: impl Into<Label>, value: Option<f64>) -> Option<f64> {
    NumberField::new(label, Bind::given(value)).show(ui).edit.events.committed().copied().flatten()
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<MeasurementsWords>();
    let model: &MeasurementsModel = props.model;
    let Some(master) = &model.master else {
        Text::new(words.no_master.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let sent = &mut *props.sent;
    for m in &model.metrics {
        let name = if m.label.is_empty() { words.metric.as_str().to_string() } else { m.label.clone() };
        let _ = Row::new().show(ui, |ui, _| {
            if let Some(n) = number(ui, name.clone(), m.position) {
                sent.push(Msg::run("font.master.setMetric", json!({ "masters": [master], "metric": m.index, "position": n })));
            }
            if m.has_zone
                && let Some(n) = number(ui, format!("{name} {}", words.zone.as_str()), m.zone)
            {
                sent.push(Msg::run("font.master.setMetric", json!({ "masters": [master], "metric": m.index, "zone": n })));
            }
        });
    }
    for s in &model.stems {
        let name = if s.name.is_empty() { words.stem.as_str() } else { s.name.as_str() };
        let mark = if s.horizontal { "↕" } else { "↔" };
        if let Some(n) = number(ui, format!("{mark} {name}"), s.value) {
            sent.push(Msg::run("font.master.setStem", json!({ "masters": [master], "stem": s.index, "value": n })));
        }
    }
    if model.metrics.is_empty() && model.stems.is_empty() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
    }
    if Button::new(words.recalculate.clone()).tooltip(words.recalculate_tip.clone()).show(ui).clicked() {
        sent.push(Msg::run("font.recalculateMeasurements", json!({ "masters": [master] })));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::shell::dock::PanelId;

    /// The panel lists the shown master's metrics; typing a height posts `font.master.setMetric` for that master.
    #[test]
    fn a_typed_metric_height_posts_set_metric() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::TogglePanel(PanelId::Measurements)));
        app.drain();
        let model = Tracked::new((*app.models.measurements).clone());
        let master = model.master.clone().expect("a master");
        let first = model.metrics.iter().find(|m| m.has_zone && !m.label.is_empty()).cloned().expect("a metric with a zone");
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let field = fixture.by_label(&first.label).expect("the metric's field");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("612").key(Key::Enter, Modifiers::NONE).settle();
        let expected = Msg::run("font.master.setMetric", json!({ "masters": [master], "metric": first.index, "position": 612.0 }));
        assert_eq!(*posted.borrow(), vec![expected]);
    }
}
