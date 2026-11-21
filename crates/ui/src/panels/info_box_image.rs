//! The Info box of a selected placed image (`infoBox.image`), in place of the selection part while the edited side's
//! image is selected (`canvas::images`).
//!
//! | Control | Runs |
//! |---|---|
//! | X, Y: the image's position (its bottom-left corner before rotation) | `image.setTransform` with `x` or `y` |
//! | W, H: the whole image's size in font units | `image.setTransform` with `width` or `height` |
//! | rotation in degrees, counter-clockwise | `image.setTransform` with `rotation` |
//! | the arrow | `image.revealFile`: the file in the system's file manager |
//! | the lock | `image.lock`; a locked image cannot be selected, so the box closes |
//!
//! The values are what `image.getInfo` reports (`app::models::image_box`). W and H need the file's pixel size and are left out
//! without it; a sheared image shows X and Y only. Under the fields, why the file cannot be drawn, when it cannot.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::json;

use super::widgets;
use crate::app::Msg;
pub use crate::app::models::image_box::ImageBox;

craft::ui::words! {
    /// The words of the image Info box.
    pub struct ImageBoxWords {
        title: "fontcraftInfoBoxImage.title" = "Image";
        x: "fontcraftInfoBoxImage.x" = "X";
        y: "fontcraftInfoBoxImage.y" = "Y";
        width: "fontcraftInfoBoxImage.width" = "W";
        height: "fontcraftInfoBoxImage.height" = "H";
        /// The rotation field, counter-clockwise in degrees.
        rotation: "fontcraftInfoBoxImage.rotation" = "⟲";
        /// The button that shows the image's file in the file manager.
        reveal: "fontcraftInfoBoxImage.reveal" = "→";
        reveal_tip: "fontcraftInfoBoxImage.revealTip" = "Reveal Image File";
        lock: "fontcraftInfoBoxImage.lock" = "Lock Image";
    }
}

/// The box of `b`; what the person changed goes to `sent`.
pub fn view(ui: &mut Ui, b: &ImageBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ImageBoxWords>();
    Section::new(words.title.clone()).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let fields = [(&words.x, "x", b.x), (&words.y, "y", b.y), (&words.width, "width", b.width), (&words.height, "height", b.height)];
                for (label, key, value) in fields {
                    // Without the file's pixel size the image has no displayed size to edit.
                    if value.is_none() && key != "x" && key != "y" {
                        continue;
                    }
                    if let Some(v) = widgets::number(ui, label.clone(), value) {
                        sent.push(Msg::run("image.setTransform", b.targets(json!({ key: v }))));
                    }
                }
            });
            Row::new().show(ui, |ui, _| {
                if b.rotation.is_some()
                    && let Some(v) = widgets::number(ui, words.rotation.clone(), b.rotation)
                {
                    sent.push(Msg::run("image.setTransform", b.targets(json!({ "rotation": v }))));
                }
                if Button::new(words.reveal.clone()).tooltip(words.reveal_tip.clone()).show(ui).clicked() {
                    sent.push(Msg::Shell(CommandRef::with_params("image.revealFile", b.targets(json!({})))));
                }
                if Button::new(words.lock.clone()).tooltip(words.lock.clone()).show(ui).clicked() {
                    sent.push(Msg::run("image.lock", b.targets(json!({}))));
                }
            });
            if let Some(problem) = &b.problem {
                Text::new(problem.clone()).tone(Tone::Warn).show(ui);
            }
        });
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};

    use serde_json::Value;

    use super::*;

    fn fixture(b: ImageBox) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &b, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        (fixture, posted)
    }

    #[test]
    fn typed_values_set_the_transform_and_the_buttons_reveal_and_lock() {
        let b = ImageBox {
            glyph: "A".into(),
            layer: "m01".into(),
            side: Some("background".into()),
            x: Some(10.0),
            y: Some(0.0),
            width: Some(100.0),
            height: Some(50.0),
            rotation: Some(0.0),
            problem: None,
        };
        let (mut f, posted) = fixture(b);
        let width = f.by_label("W").expect("the W field");
        f.click(&width).key(Key::A, Modifiers::COMMAND).type_text("200").key(Key::Enter, Modifiers::NONE).settle();
        let reveal = f.by_label("→").expect("the reveal button");
        f.click(&reveal).settle();
        let lock = f.by_label("Lock Image").expect("the lock button");
        f.click(&lock).settle();
        let target = json!({"glyph": "A", "layer": "m01", "side": "background"});
        let with = |extra: Value| {
            let mut t = target.clone();
            t.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            t
        };
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("image.setTransform", with(json!({"width": 200.0}))),
                Msg::Shell(CommandRef::with_params("image.revealFile", target.clone())),
                Msg::run("image.lock", target.clone()),
            ]
        );

        let (f, _) = fixture(ImageBox { width: None, height: None, problem: Some("the file cannot be found".into()), ..ImageBox::default() });
        assert!(f.by_label("W").is_none(), "no size without the file's pixel size");
        assert!(f.by_label("the file cannot be found").is_some());
    }
}
