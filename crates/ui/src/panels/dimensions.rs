//! Palette › Dimensions: the measurement notepad of the master the canvas shows, one field per key of the shown glyph's
//! script (`dimensions.getValues`). Typing a number notes it, emptying a field clears it (`dimensions.setValue`). The
//! numbers change nothing in the font.

use craft::ui::contract::{Show as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;
use serde_json::{Value, json};

use super::widgets::{self, NumberEdit};
use crate::app::Msg;
use crate::app::models::dimensions::DimensionsModel;

craft::ui::words! {
    /// The words of the Dimensions panel.
    pub struct DimensionsWords {
        no_master: "fontcraftDimensions.noMaster" = "No font master";
        empty: "fontcraftDimensions.empty" = "No measurements for this script";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<DimensionsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<DimensionsWords>();
    let model: &DimensionsModel = props.model;
    let Some(master) = &model.master else {
        Text::new(words.no_master.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    if model.fields.is_empty() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    for field in &model.fields {
        let value = match widgets::clearable_number(ui, Label::from(field.key.clone()), field.value) {
            Some(NumberEdit::Set(n)) => json!(n),
            Some(NumberEdit::Clear) => Value::Null,
            None => continue,
        };
        props.sent.push(Msg::run("dimensions.setValue", json!({ "master": master, "key": field.key, "value": value })));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
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
    use crate::app::models::dimensions::{DimensionField, DimensionsModel};
    use crate::app::{APP, FontCraftApp, Msg};

    /// Typing a number notes it in the shown master; emptying a field clears it.
    #[test]
    fn typed_and_emptied_fields_post_set_value() {
        let model = Tracked::new(DimensionsModel {
            master: Some("m01".into()),
            fields: vec![DimensionField { key: "HH".into(), value: Some(80.0) }, DimensionField { key: "nV".into(), value: None }],
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let nv = fixture.by_label("nV").expect("the nV field");
        fixture.click(&nv).type_text("92").key(Key::Enter, Modifiers::NONE).settle();
        let hh = fixture.by_label("HH").expect("the HH field");
        fixture.click(&hh).key(Key::A, Modifiers::COMMAND).key(Key::Backspace, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("dimensions.setValue", json!({"master": "m01", "key": "nV", "value": 92.0})),
                Msg::run("dimensions.setValue", json!({"master": "m01", "key": "HH", "value": null})),
            ]
        );
    }

    /// The panel shows in the default layout and reads the notepad of the master the canvas shows.
    #[test]
    fn the_model_reads_the_shown_master_notepad() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let master = app.session.font().unwrap().masters[0].id.clone();
        app.session.execute("dimensions.setValue", &json!({"master": master, "key": "nV", "value": 90})).unwrap();
        app.drain();
        let model = &*app.models.dimensions;
        assert_eq!(model.master.as_deref(), Some(master.as_str()));
        let nv = model.fields.iter().find(|f| f.key == "nV").expect("the nV field");
        assert_eq!(nv.value, Some(90.0));
    }
}
