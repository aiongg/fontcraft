//! The guide part of the Info box (M3.41, rows `guides.infoBoxFields`, `guides.angleLock`, `guides.measurementFlag`):
//! shown for one selected guide, local or global. The model is `app::models::guide_box::GuideBox`.
//!
//! | Control | Runs `guide.setProperties` with |
//! |---|---|
//! | X, Y | `x`, `y` (rounded to the grid by the command) |
//! | Angle, and the lock beside it | `angle`; `angleLocked` |
//! | Measured From: Left, Right or Both Sidebearings | `relativeTo` |
//! | Measurement | `measurement` |
//! | Name | `name` (empty clears it) |
//!
//! Each edit is one undo step. A locked guide, or a locked angle, makes the command refuse, and its message shows.
//! A global guide's glyph scope is edited in its own dialog (`dialogs::guide_scope`, through Info for Selection).

use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets;
use crate::app::Msg;
use crate::app::models::guide_box::GuideBox;

craft::ui::words! {
    /// The words of the guide part of the Info box.
    pub struct GuideBoxWords {
        local: "fontcraftGuideBox.local" = "Guide";
        global: "fontcraftGuideBox.global" = "Global Guide";
        x: "fontcraftGuideBox.x" = "Guide X";
        y: "fontcraftGuideBox.y" = "Guide Y";
        angle: "fontcraftGuideBox.angle" = "Angle";
        lock: "fontcraftGuideBox.lock" = "Lock Angle";
        relative: "fontcraftGuideBox.relativeTo" = "Measured From";
        left: "fontcraftGuideBox.left" = "Left Sidebearing";
        right: "fontcraftGuideBox.right" = "Right Sidebearing";
        both: "fontcraftGuideBox.both" = "Both Sidebearings";
        measurement: "fontcraftGuideBox.measurement" = "Measurement";
        name: "fontcraftGuideBox.name" = "Guide Name";
    }
}

/// Longest guide name the field takes, in characters.
const MAX_NAME_CHARS: usize = 256;

/// The part for `b`; what the person changed goes to `sent`.
pub fn view(ui: &mut Ui, b: &GuideBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<GuideBoxWords>();
    let set = |sent: &mut Vec<Msg>, extra: Value| sent.push(Msg::run("guide.setProperties", b.target.with(extra)));
    let title = if b.global { words.global.clone() } else { words.local.clone() };
    Section::new(title).id_salt("fontcraftGuideBox").show(ui, |ui, _| {
        let _ = Row::new().show(ui, |ui, _| {
            if let Some(x) = widgets::number(ui, words.x.clone(), Some(b.x)).filter(|v| v.is_finite()) {
                set(sent, json!({ "x": x }));
            }
            if let Some(y) = widgets::number(ui, words.y.clone(), Some(b.y)).filter(|v| v.is_finite()) {
                set(sent, json!({ "y": y }));
            }
        });
        let _ = Row::new().show(ui, |ui, _| {
            if let Some(angle) = widgets::number(ui, words.angle.clone(), Some(b.angle)).filter(|v| v.is_finite()) {
                set(sent, json!({ "angle": angle }));
            }
            let lock = ToggleButton::new(words.lock.clone(), Bind::given(b.angle_locked)).tooltip(words.lock.clone()).show(ui);
            if lock.events.committed().is_some() {
                set(sent, json!({ "angleLocked": !b.angle_locked }));
            }
        });
        let sides = [("left", &words.left), ("right", &words.right), ("both", &words.both)];
        let entries: Vec<Entry<String>> = sides.iter().map(|(v, l)| Entry::new((*v).to_string(), (*l).clone())).collect();
        let picked = widgets::labelled(ui, &words.relative, |ui| {
            Select::new(words.relative.clone(), Bind::given(b.relative_to.clone()), &entries).show(ui).events.committed().cloned()
        });
        if let Some(Some(side)) = picked
            && side != b.relative_to
        {
            set(sent, json!({ "relativeTo": side }));
        }
        let measure = ToggleButton::new(words.measurement.clone(), Bind::given(b.measurement)).tooltip(words.measurement.clone()).show(ui);
        if measure.events.committed().is_some() {
            set(sent, json!({ "measurement": !b.measurement }));
        }
        let committed = widgets::labelled(ui, &words.name, |ui| {
            TextField::new(words.name.clone(), Bind::given(b.name.clone()))
                .select_on_focus()
                .limit(MAX_NAME_CHARS)
                .show(ui)
                .events
                .committed()
                .cloned()
        });
        if let Some(Some(name)) = committed
            && name.trim() != b.name
        {
            set(sent, json!({ "name": name.trim() }));
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::{Value, json};

    use super::view;
    use crate::app::Msg;
    use crate::app::models::guide_box::GuideBox;
    use crate::app::models::palette::Targets;

    fn guide_box() -> GuideBox {
        GuideBox {
            target: Targets { json: json!({"glyph": "A", "layer": "m01", "guide": {"scope": "local", "index": 0}}).to_string() },
            x: 100.0,
            y: 200.0,
            angle: 90.0,
            relative_to: "left".into(),
            ..Default::default()
        }
    }

    fn fixture() -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let b = guide_box();
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &b, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.settle();
        (f, posted)
    }

    /// `guide.setProperties` on the guide, with `extra`: the command the part posts.
    fn expected(extra: Value) -> Msg {
        let mut params = json!({"glyph": "A", "layer": "m01", "guide": {"scope": "local", "index": 0}});
        if let (Some(p), Value::Object(e)) = (params.as_object_mut(), extra) {
            p.extend(e);
        }
        Msg::run("guide.setProperties", params)
    }

    fn type_into(label: &str, text: &str) -> Vec<Msg> {
        let (mut f, posted) = fixture();
        let field = f.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        f.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
        posted.borrow().clone()
    }

    fn click(label: &str) -> Vec<Msg> {
        let (mut f, posted) = fixture();
        let control = f.by_label(label).unwrap_or_else(|| panic!("the {label} control"));
        f.click(&control).settle();
        posted.borrow().clone()
    }

    /// Each field runs `guide.setProperties` on the guide with its one property.
    #[test]
    fn the_guide_fields_set_the_guide_properties() {
        assert_eq!(type_into("Guide X", "140"), vec![expected(json!({"x": 140.0}))]);
        assert_eq!(type_into("Angle", "45"), vec![expected(json!({"angle": 45.0}))]);
        assert_eq!(type_into("Guide Name", "stem"), vec![expected(json!({"name": "stem"}))]);
        assert_eq!(click("Lock Angle"), vec![expected(json!({"angleLocked": true}))]);
        assert_eq!(click("Measurement"), vec![expected(json!({"measurement": true}))]);

        let (mut f, posted) = fixture();
        let menu = f.by_label("Measured From").expect("the Measured From pop-up");
        f.click(&menu).settle();
        let right = f.by_label("Right Sidebearing").expect("Right Sidebearing in the pop-up");
        f.click(&right).settle();
        assert_eq!(*posted.borrow(), vec![expected(json!({"relativeTo": "right"}))]);
    }
}
