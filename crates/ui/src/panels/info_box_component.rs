//! The component, corner and cap parts of the Info box. Each shows under the selection part while its item is
//! selected; the models are in `app::models::components`.
//!
//! | Part | Control | Runs |
//! |---|---|---|
//! | component | base glyph name | the glyph picker for `component.setBaseGlyph` (`editView.chooseGlyph`, `baseGlyph`) |
//! | component | anchor menu, only with two or more anchors to choose from | `component.setAnchor` |
//! | component | Smart badge, only on a smart component | `smartComponent.showSettings` |
//! | component | arrow button | `component.editBaseGlyph` |
//! | component | X, Y, horizontal and vertical scale, slant, rotation | `component.setTransform` |
//! | corner | corner glyph name; arrow button | the picker for `cornerComponent.setGlyph`; `cornerComponent.editGlyph` |
//! | corner | horizontal and vertical scale; four alignment buttons | `cornerComponent.setScale`; `cornerComponent.setAlignment` |
//! | cap | cap glyph name; arrow button | the picker for `capComponent.setGlyph`; `cornerComponent.editGlyph` on the cap's hint |
//! | cap | ↔ and ↕ scale; four alignment buttons; Fit | `capComponent.setScale`; `capComponent.setAlignment`; `capComponent.setFit` |
//!
//! X and Y are greyed out while the component is aligned automatically (only X while it is aligned horizontally), and
//! every field while it is locked. With Fit on, the cap's horizontal scale is computed and greyed out.

use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Map, Value, json};

use super::widgets::{self, NumberEdit};
use crate::app::Msg;
use crate::app::models::components::{CapBox, ComponentBox, CornerBox, MODES};

craft::ui::words! {
    /// The words of the component, corner and cap parts of the Info box.
    pub struct ComponentBoxWords {
        component: "fontcraftComponentBox.component" = "Component";
        corner: "fontcraftComponentBox.corner" = "Corner Component";
        cap: "fontcraftComponentBox.cap" = "Cap Component";
        choose_base: "fontcraftComponentBox.chooseBase" = "Choose another glyph for this component";
        choose_corner: "fontcraftComponentBox.chooseCorner" = "Choose another corner glyph";
        choose_cap: "fontcraftComponentBox.chooseCap" = "Choose another cap glyph";
        /// The tooltip of a name whose glyph is missing.
        missing: "fontcraftComponentBox.missing" = "This glyph does not exist: choose another one";
        anchor: "fontcraftComponentBox.anchor" = "Anchor";
        smart: "fontcraftComponentBox.smart" = "Smart";
        smart_tip: "fontcraftComponentBox.smartTip" = "Smart Component Settings";
        /// The arrow button that edits the glyph a part names.
        edit: "fontcraftComponentBox.edit" = "↗";
        edit_base: "fontcraftComponentBox.editBase" = "Edit the base glyph";
        edit_corner: "fontcraftComponentBox.editCorner" = "Edit the corner glyph";
        edit_cap: "fontcraftComponentBox.editCap" = "Edit the cap glyph";
        x: "fontcraftComponentBox.x" = "X";
        y: "fontcraftComponentBox.y" = "Y";
        x_tip: "fontcraftComponentBox.xTip" = "Horizontal offset";
        y_tip: "fontcraftComponentBox.yTip" = "Vertical offset";
        scale_x: "fontcraftComponentBox.scaleX" = "↔%";
        scale_y: "fontcraftComponentBox.scaleY" = "↕%";
        scale_x_tip: "fontcraftComponentBox.scaleXTip" = "Horizontal scale in percent";
        scale_y_tip: "fontcraftComponentBox.scaleYTip" = "Vertical scale in percent";
        slant: "fontcraftComponentBox.slant" = "Slant";
        slant_tip: "fontcraftComponentBox.slantTip" = "Slant to the right, in degrees";
        rotation: "fontcraftComponentBox.rotation" = "Rotation";
        rotation_tip: "fontcraftComponentBox.rotationTip" = "Rotation counterclockwise, in degrees";
        left: "fontcraftComponentBox.left" = "←";
        both: "fontcraftComponentBox.both" = "↔";
        right: "fontcraftComponentBox.right" = "→";
        none: "fontcraftComponentBox.none" = "✕";
        left_tip: "fontcraftComponentBox.leftTip" = "Align to the outgoing stroke";
        both_tip: "fontcraftComponentBox.bothTip" = "Align to both strokes";
        right_tip: "fontcraftComponentBox.rightTip" = "Align to the incoming stroke";
        none_tip: "fontcraftComponentBox.noneTip" = "Do not align";
        fit: "fontcraftComponentBox.fit" = "Fit";
        fit_tip: "fontcraftComponentBox.fitTip" = "Size the cap to the width of the stroke end";
    }
}

/// The component part.
pub fn component_part(ui: &mut Ui, b: &ComponentBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ComponentBoxWords>();
    let target = json!({"glyph": b.glyph, "layer": b.layer, "component": b.component});
    Section::new(words.component.clone()).show(ui, |ui, _| {
        let _ = Row::new().show(ui, |ui, _| {
            let name = if b.selected > 1 { format!("{} (+{})", b.base, b.selected - 1) } else { b.base.clone() };
            if Button::new(name).tooltip(words.choose_base.clone()).show(ui).clicked() {
                sent.push(shell("editView.chooseGlyph", with(&target, json!({"purpose": "baseGlyph"}))));
            }
            if b.anchors.len() > 1 {
                let entries: Vec<Entry<String>> = b.anchors.iter().map(|a| Entry::new(a.clone(), a.clone())).collect();
                let shown = b.anchor.clone().unwrap_or_default();
                if let Some(anchor) = Select::new(words.anchor.clone(), Bind::given(shown.clone()), &entries).show(ui).events.committed()
                    && *anchor != shown
                {
                    sent.push(Msg::run("component.setAnchor", with(&target, json!({ "anchor": anchor }))));
                }
            }
            if b.smart && Button::new(words.smart.clone()).selected(true).tooltip(words.smart_tip.clone()).show(ui).clicked() {
                let params = json!({"glyph": b.glyph, "layer": b.layer, "components": [b.component]});
                sent.push(shell("smartComponent.showSettings", params));
            }
            if Button::new(words.edit.clone()).tooltip(words.edit_base.clone()).id_salt("editBase").show(ui).clicked() {
                sent.push(shell("component.editBaseGlyph", json!({ "component": b.component })));
            }
        });
        let v = b.values;
        let editable = !b.locked && v.is_some();
        let fields: [(&Label, &Label, &str, Option<f64>, bool); 6] = [
            (&words.x, &words.x_tip, "x", Some(b.x), b.x_free),
            (&words.y, &words.y_tip, "y", Some(b.y), b.y_free),
            (&words.scale_x, &words.scale_x_tip, "scaleX", v.map(|v| v.scale_x), editable),
            (&words.scale_y, &words.scale_y_tip, "scaleY", v.map(|v| v.scale_y), editable),
            (&words.slant, &words.slant_tip, "slant", v.map(|v| v.slant), editable),
            (&words.rotation, &words.rotation_tip, "rotation", v.map(|v| v.rotation), editable),
        ];
        let _ = Row::new().show(ui, |ui, _| {
            for (label, tip, key, value, enabled) in fields {
                if let Some(n) = field(ui, label, tip, value, enabled) {
                    sent.push(Msg::run("component.setTransform", with(&target, keyed(key, n))));
                }
            }
        });
    });
}

/// The corner part.
pub fn corner_part(ui: &mut Ui, b: &CornerBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ComponentBoxWords>();
    let target = json!({"glyph": b.glyph, "layer": b.layer, "corners": [b.corner]});
    Section::new(words.corner.clone()).show(ui, |ui, _| {
        let names = Names {
            glyph: &b.glyph,
            layer: &b.layer,
            name: &b.name,
            exists: b.exists,
            choose: &words.choose_corner,
            edit: &words.edit_corner,
            purpose: "cornerGlyph",
            hint: b.corner,
        };
        name_row(ui, &words, &names, &target, sent);
        let _ = Row::new().show(ui, |ui, _| {
            scale_fields(ui, &words, b.scale, [true, true], "cornerComponent.setScale", &target, sent);
            mode_buttons(ui, &words, b.mode.as_deref(), "cornerComponent.setAlignment", &target, sent);
        });
    });
}

/// The cap part (M6.25): the cap glyph's name and arrow button, ↔ and ↕ scale, four alignment buttons and Fit.
pub fn cap_part(ui: &mut Ui, b: &CapBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ComponentBoxWords>();
    let target = json!({"glyph": b.glyph, "layer": b.layer, "caps": [b.cap]});
    Section::new(words.cap.clone()).show(ui, |ui, _| {
        let names = Names {
            glyph: &b.glyph,
            layer: &b.layer,
            name: &b.name,
            exists: b.exists,
            choose: &words.choose_cap,
            edit: &words.edit_cap,
            purpose: "capGlyph",
            hint: b.cap,
        };
        name_row(ui, &words, &names, &target, sent);
        let _ = Row::new().show(ui, |ui, _| {
            scale_fields(ui, &words, b.scale, [!b.fit, true], "capComponent.setScale", &target, sent);
            let fit = Checkbox::new(Bind::given(b.fit)).label(words.fit.clone()).tooltip(words.fit_tip.clone()).show(ui);
            if let Some(on) = fit.events.committed()
                && *on != b.fit
            {
                sent.push(Msg::run("capComponent.setFit", with(&target, json!({ "fit": on }))));
            }
        });
        let _ = Row::new().show(ui, |ui, _| mode_buttons(ui, &words, b.mode.as_deref(), "capComponent.setAlignment", &target, sent));
    });
}

/// What the name row of a corner or cap shows.
struct Names<'a> {
    glyph: &'a str,
    layer: &'a str,
    name: &'a str,
    exists: bool,
    choose: &'a Label,
    edit: &'a Label,
    /// The glyph picker's purpose.
    purpose: &'a str,
    /// The hint index `cornerComponent.editGlyph` takes.
    hint: usize,
}

fn name_row(ui: &mut Ui, words: &ComponentBoxWords, n: &Names<'_>, target: &Value, sent: &mut Vec<Msg>) {
    let _ = Row::new().show(ui, |ui, _| {
        let tip = if n.exists { n.choose.clone() } else { words.missing.clone() };
        if Button::new(n.name.to_string()).tooltip(tip).show(ui).clicked() {
            sent.push(shell("editView.chooseGlyph", with(target, json!({ "purpose": n.purpose }))));
        }
        if Button::new(words.edit.clone()).tooltip(n.edit.clone()).enabled(n.exists).id_salt("editGlyph").show(ui).clicked() && n.exists {
            sent.push(shell("cornerComponent.editGlyph", json!({"glyph": n.glyph, "layer": n.layer, "corner": n.hint})));
        }
    });
}

/// The horizontal and vertical scale fields of a corner or cap, in percent.
fn scale_fields(ui: &mut Ui, words: &ComponentBoxWords, scale: [f64; 2], enabled: [bool; 2], command: &str, target: &Value, sent: &mut Vec<Msg>) {
    let [sx, sy] = scale;
    let [ex, ey] = enabled;
    for (label, tip, key, value, on) in
        [(&words.scale_x, &words.scale_x_tip, "scaleX", sx, ex), (&words.scale_y, &words.scale_y_tip, "scaleY", sy, ey)]
    {
        if let Some(n) = field(ui, label, tip, Some(value), on) {
            sent.push(Msg::run(command, with(target, keyed(key, n))));
        }
    }
}

/// The four alignment buttons; the current mode's is pressed.
fn mode_buttons(ui: &mut Ui, words: &ComponentBoxWords, current: Option<&str>, command: &str, target: &Value, sent: &mut Vec<Msg>) {
    let faces = [(&words.left, &words.left_tip), (&words.both, &words.both_tip), (&words.right, &words.right_tip), (&words.none, &words.none_tip)];
    for (mode, (face, tip)) in MODES.iter().zip(faces) {
        let on = current == Some(*mode);
        if Button::new(face.clone()).tooltip(tip.clone()).selected(on).id_salt(*mode).show(ui).clicked() && !on {
            sent.push(Msg::run(command, with(target, json!({ "mode": mode }))));
        }
    }
}

/// A number field that commits only while `enabled`.
fn field(ui: &mut Ui, label: &Label, tip: &Label, value: Option<f64>, enabled: bool) -> Option<f64> {
    let shown = NumberField::new(label.clone(), Bind::given(value)).tooltip(tip.clone()).enabled(enabled);
    match widgets::clearable(ui, shown, value) {
        Some(NumberEdit::Set(n)) if enabled => Some(n),
        _ => None,
    }
}

fn keyed(key: &str, n: f64) -> Value {
    let mut m = Map::new();
    m.insert(key.into(), json!(n));
    Value::Object(m)
}

/// `target` with the keys of `extra` added.
fn with(target: &Value, extra: Value) -> Value {
    let mut m = target.as_object().cloned().unwrap_or_default();
    if let Value::Object(extra) = extra {
        m.extend(extra);
    }
    Value::Object(m)
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers, Ui};
    use serde_json::json;

    use super::{cap_part, component_part, corner_part};
    use crate::app::Msg;
    use crate::app::models::components::{CapBox, ComponentBox, ComponentValues, CornerBox};

    fn fixture(show: impl Fn(&mut Ui, &mut Vec<Msg>) + 'static) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        (fixture, posted)
    }

    fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
        let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
    }

    fn component() -> ComponentBox {
        ComponentBox {
            glyph: "Aacute".into(),
            layer: "m01".into(),
            component: 1,
            base: "acutecomb".into(),
            values: Some(ComponentValues { x: 100.0, y: 0.0, scale_x: 100.0, scale_y: 100.0, slant: 0.0, rotation: 0.0 }),
            x: 100.0,
            y: 0.0,
            x_free: true,
            y_free: true,
            smart: true,
            selected: 1,
            ..Default::default()
        }
    }

    /// The component's fields set its transform; the Smart badge opens the settings of that component.
    #[test]
    fn component_fields_set_the_transform() {
        let b = component();
        let (mut f, posted) = fixture(move |ui, sent| component_part(ui, &b, sent));
        type_into(&mut f, "↔%", "50");
        let smart = f.by_label("Smart").expect("the Smart badge");
        f.click(&smart).settle();
        let posted = posted.borrow();
        assert_eq!(
            posted.first(),
            Some(&Msg::run("component.setTransform", json!({"glyph": "Aacute", "layer": "m01", "component": 1, "scaleX": 50.0})))
        );
        assert_eq!(posted.get(1).and_then(Msg::command_id), Some("smartComponent.showSettings"));
    }

    /// A component aligned automatically refuses X: the field is greyed out and posts nothing.
    #[test]
    fn an_aligned_component_keeps_its_x() {
        let b = ComponentBox { x_free: false, ..component() };
        let (mut f, posted) = fixture(move |ui, sent| component_part(ui, &b, sent));
        type_into(&mut f, "X", "300");
        assert!(posted.borrow().iter().all(|m| m.command_id() != Some("component.setTransform")));
    }

    /// A corner's alignment button and scale field post their commands for that corner.
    #[test]
    fn corner_controls_post_for_the_corner() {
        let b = CornerBox {
            glyph: "H".into(),
            layer: "m01".into(),
            corner: 2,
            name: "_corner.serif".into(),
            scale: [100.0, 100.0],
            mode: Some("left".into()),
            exists: true,
        };
        let (mut f, posted) = fixture(move |ui, sent| corner_part(ui, &b, sent));
        let right = f.by_label("→").expect("the right alignment button");
        f.click(&right).settle();
        type_into(&mut f, "↕%", "-100");
        let target = json!({"glyph": "H", "layer": "m01", "corners": [2]});
        let with = |extra: serde_json::Value| super::with(&target, extra);
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("cornerComponent.setAlignment", with(json!({"mode": "right"}))),
                Msg::run("cornerComponent.setScale", with(json!({"scaleY": -100.0})))
            ]
        );
    }

    /// The cap's Fit checkbox posts `capComponent.setFit`; with Fit on the horizontal scale is computed and refuses edits.
    #[test]
    fn cap_fit_greys_the_horizontal_scale() {
        let b = CapBox {
            glyph: "l".into(),
            layer: "m01".into(),
            cap: 0,
            name: "_cap.flag".into(),
            scale: [80.0, 100.0],
            mode: Some("both".into()),
            fit: true,
            exists: true,
        };
        let (mut f, posted) = fixture(move |ui, sent| cap_part(ui, &b, sent));
        type_into(&mut f, "↔%", "50");
        let fit = f.by_label("Fit").expect("the Fit checkbox");
        f.click(&fit).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("capComponent.setFit", json!({"glyph": "l", "layer": "m01", "caps": [0], "fit": false}))]);
    }
}
