//! Metrics: the shown glyph's width and sidebearings, each a field that sets it.
//!
//! The pattern for every panel: props that hold everything the view reads, a plain `view` function, a view model
//! the app layer projects (`app::models::glyph`), and edits that leave as `Msg::Run` with an engine command.

use craft::ui::contract::{Bind, Show as _, ShowContainer as _, Tracked};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::glyph::GlyphMetrics;

craft::ui::words! {
    /// The words of the Metrics panel.
    pub struct MetricsWords {
        /// The section title.
        metrics: "fontcraftMetrics.title" = "Metrics";
        /// The advance width field.
        width: "fontcraftMetrics.width" = "Width";
        /// The left sidebearing field.
        left: "fontcraftMetrics.left" = "LSB";
        /// The right sidebearing field.
        right: "fontcraftMetrics.right" = "RSB";
    }
}

/// What the Metrics panel reads and sends. Every input is a field; nothing else is read.
pub struct Props<'a> {
    pub model: &'a Tracked<GlyphMetrics>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<MetricsWords>();
    let Some(glyph) = props.model.glyph.clone() else { return };
    let layer = props.model.layer.clone();
    let fields = [
        (words.width.clone(), props.model.width, "spacing.setWidth"),
        (words.left.clone(), props.model.left, "spacing.setLeftSidebearing"),
        (words.right.clone(), props.model.right, "spacing.setRightSidebearing"),
    ];
    let sent = &mut *props.sent;
    Section::new(words.metrics.clone()).show(ui, |ui, _| {
        for (label, value, command) in &fields {
            let (value, command) = (*value, *command);
            field(ui, label.clone(), value, |value| {
                let mut params = json!({ "glyphs": [glyph], "value": value });
                if let Some(id) = &layer
                    && let Some(map) = params.as_object_mut()
                {
                    map.insert("layer".into(), json!(id));
                }
                sent.push(Msg::run(command, params));
            });
        }
    });
}

/// A number field over `value` that calls `commit` with the value typed.
fn field(ui: &mut Ui, label: Label, value: Option<f64>, mut commit: impl FnMut(f64)) {
    let out = NumberField::new(label, Bind::given(value)).show(ui);
    if let Some(Some(value)) = out.edit.events.committed() {
        commit(*value);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::glyph::GlyphMetrics;

    /// Typing a width and pressing Enter posts one `spacing.setWidth` for the shown glyph's shown layer.
    #[test]
    fn a_typed_width_posts_set_width() {
        let model = Tracked::new(GlyphMetrics {
            glyph: Some("A".into()),
            layer: Some("m02".into()),
            width: Some(600.0),
            left: Some(20.0),
            right: Some(30.0),
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let width = fixture.by_label("Width").expect("the Width field");
        fixture.click(&width).key(Key::A, Modifiers::COMMAND).type_text("640").key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("spacing.setWidth", json!({ "glyphs": ["A"], "layer": "m02", "value": 640.0 }))]);
    }
}
