//! Palette › Smart Component (`palette.smartComponent`) and the body of the Smart Component Settings dialog: one control
//! per property of the selected smart components' smart glyphs.
//!
//! | Property | Control |
//! |---|---|
//! | range other than 0 to 1 | a slider from minimum to maximum and a number field taking any value |
//! | range 0 to 1 | a checkbox (`smart.zeroOneRangeIsToggle`) |
//!
//! Every control writes through `smartComponent.setValue`; with several components one `command.batch` sets them all
//! as one undo step. The slider posts when the drag ends, so a drag is one undo step.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::Env;
use craft::ui::value_drag::Slider;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets;
use crate::app::Msg;
use crate::app::models::components::{SmartModel, SmartRow};

craft::ui::words! {
    /// The words of the Smart Component controls.
    pub struct SmartComponentWords {
        /// Nothing to show.
        nothing: "fontcraftSmartComponent.nothing" = "Select a smart component";
        /// The undo label of a change to several components.
        label: "fontcraftSmartComponent.label" = "Set Smart Component Value";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<SmartModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    super::widgets::scrolling(ui, |ui| view(&mut props, ui));
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &SmartModel = props.model;
    controls(ui, model, props.sent);
}

/// The controls for `model`; the dialog shows the same ones.
pub fn controls(ui: &mut Ui, model: &SmartModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<SmartComponentWords>();
    if model.components.is_empty() {
        Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    for row in &model.rows {
        if let Some(value) = row_control(ui, row) {
            sent.push(value_command(model, row, value, words.label.as_str()));
        }
    }
}

/// One property's control; the value committed.
fn row_control(ui: &mut Ui, row: &SmartRow) -> Option<f64> {
    if row.toggle {
        let on = row.value.map(|v| v >= 0.5);
        let out = Checkbox::new(Bind::given(on.unwrap_or(false))).label(row.name.clone()).show(ui);
        return out.events.committed().map(|on| if *on { 1.0 } else { 0.0 });
    }
    let (lo, hi) = if row.min <= row.max { (row.min, row.max) } else { (row.max, row.min) };
    let mut picked = None;
    let _ = Row::new().show(ui, |ui, _| {
        let shown = row.value.unwrap_or(lo).clamp(lo, hi);
        let slid = Slider::new(Bind::given(shown), lo..=hi).label(row.name.clone()).show(ui);
        if let Some(v) = slid.events.committed().filter(|v| v.is_finite() && Some(**v) != row.value) {
            picked = Some(*v);
        }
        if let Some(v) = widgets::number(ui, row.name.clone().into(), row.value) {
            picked = Some(v);
        }
    });
    picked
}

/// The command that sets `row`'s property to `value` on the components that have it: one `smartComponent.setValue`, or
/// a `command.batch` of them for several components.
pub fn value_command(model: &SmartModel, row: &SmartRow, value: f64, label: &str) -> Msg {
    let step = |component: usize| {
        let mut params = json!({"glyph": model.glyph, "layer": model.layer, "component": component, "property": row.name, "value": value});
        if model.background
            && let Some(m) = params.as_object_mut()
        {
            m.insert("side".into(), json!("background"));
        }
        params
    };
    match row.components.as_slice() {
        [only] => Msg::run("smartComponent.setValue", step(*only)),
        many => {
            let steps: Vec<Value> = many.iter().map(|&i| json!({"command": "smartComponent.setValue", "params": step(i)})).collect();
            Msg::run("command.batch", json!({"label": label, "steps": steps}))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::components::{SmartModel, SmartRow};

    /// A 0-to-1 property is a checkbox; ticking it sets 1 on every selected component that has it, as one batch.
    #[test]
    fn a_toggle_property_sets_every_component() {
        let model = Tracked::new(SmartModel {
            glyph: "a".into(),
            layer: "m01".into(),
            components: vec![0, 2],
            rows: vec![SmartRow { name: "Shoulder".into(), min: 0.0, max: 1.0, toggle: true, value: Some(0.0), components: vec![0, 2] }],
            ..Default::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let check = fixture.by_label("Shoulder").expect("the Shoulder checkbox");
        fixture.click(&check).settle();
        let step = |c: usize| json!({"command": "smartComponent.setValue", "params": {"glyph": "a", "layer": "m01", "component": c, "property": "Shoulder", "value": 1.0}});
        assert_eq!(*posted.borrow(), vec![Msg::run("command.batch", json!({"label": "Set Smart Component Value", "steps": [step(0), step(2)]}))]);
    }
}
