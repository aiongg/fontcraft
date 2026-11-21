//! Up and Down step the number a text field holds: a field that takes a number or something else (a metrics key, a
//! version written with leading zeros) steps like a number field while its text is a number.
//!
//! Meant for craft-ui-widgets, `src/text/text_field.rs`: there it is a `steps(ArrowSteps)` prop that sets
//! `TextOptions::steps_value`, so the text part keeps the arrows, and turns the arrow intents into a stepped value as
//! `NumberField` does. `TextField` has no such prop, and its single-line text part leaves the arrows alone, so this
//! stand-in takes Up and Down from the pass's input while the field's text has focus, right after the field has shown.
//! `NumberField` steps by its step and ten steps with Shift; the huge step (Cmd+Shift) is this piece's only.

use craft::ui::contract::EditResponse;
use egui::{Key, Modifiers, Response, Ui};

/// How far one arrow press moves the number: plain, with Shift, with Cmd+Shift (Ctrl+Shift off macOS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowSteps {
    pub step: f64,
    pub large: f64,
    pub huge: f64,
}

impl ArrowSteps {
    /// 1, 10 with Shift, 100 with Cmd+Shift: the steps of a field in font units.
    pub const UNITS: ArrowSteps = ArrowSteps { step: 1.0, large: 10.0, huge: 100.0 };
}

impl Default for ArrowSteps {
    fn default() -> Self {
        ArrowSteps::UNITS
    }
}

/// Most presses of one arrow and modifier counted in a pass: a stuck key repeats, it does not jump.
const MAX_PRESSES: usize = 64;

/// The number `text` holds moved by `delta`; `None` when the text is no finite number.
pub fn stepped(text: &str, delta: f64) -> Option<f64> {
    let now = text.trim().parse::<f64>().ok().filter(|n| n.is_finite())?;
    Some(now + delta).filter(|n| n.is_finite())
}

/// The arrow steps a field took this pass.
pub trait StepKeys {
    /// While the field's text has focus, takes the pass's Up and Down presses (so nothing else acts on them) and
    /// returns how far they move the number: up is positive. `None` without focus or presses.
    fn arrow_steps(&self, ui: &Ui, steps: ArrowSteps) -> Option<f64>;
}

fn take(ui: &Ui, field: egui::Id, steps: ArrowSteps) -> Option<f64> {
    if !super::text_focused(ui.ctx(), field) {
        return None;
    }
    let delta = ui.input_mut(|input| {
        let mut delta = 0.0;
        for (key, sign) in [(Key::ArrowUp, 1.0), (Key::ArrowDown, -1.0)] {
            // The most modifiers first: a plain pattern also matches presses with Shift held.
            for (mods, by) in [(Modifiers::COMMAND | Modifiers::SHIFT, steps.huge), (Modifiers::SHIFT, steps.large), (Modifiers::NONE, steps.step)] {
                let presses = input.count_and_consume_key(mods, key).min(MAX_PRESSES);
                delta += sign * by * presses as f64;
            }
        }
        delta
    });
    (delta != 0.0 && delta.is_finite()).then_some(delta)
}

impl StepKeys for Response {
    fn arrow_steps(&self, ui: &Ui, steps: ArrowSteps) -> Option<f64> {
        take(ui, self.id, steps)
    }
}

impl<T> StepKeys for EditResponse<T> {
    fn arrow_steps(&self, ui: &Ui, steps: ArrowSteps) -> Option<f64> {
        take(ui, self.response.id, steps)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::{Bind, Show as _};
    use craft::ui::testing::Fixture;
    use craft::ui::text::TextField;

    use super::*;

    #[test]
    fn only_a_number_steps() {
        assert_eq!(stepped(" 40 ", 10.0), Some(50.0));
        assert_eq!(stepped("=H", 1.0), None);
        assert_eq!(stepped("inf", 1.0), None);
    }

    /// Up steps by one, Shift+Down by ten, and only while the field has focus.
    #[test]
    fn arrows_step_the_focused_field() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&steps);
        let mut fixture = Fixture::new(move |ui| {
            let out = TextField::new("Width", Bind::given("40".to_string())).show(ui);
            if let Some(delta) = out.arrow_steps(ui, ArrowSteps::UNITS) {
                seen.borrow_mut().push(delta);
            }
        });
        fixture.settle();
        fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
        assert!(steps.borrow().is_empty(), "a field without focus takes no arrows");
        let field = fixture.by_label("Width").expect("the field");
        fixture.click(&field).key(Key::ArrowUp, Modifiers::NONE).key(Key::ArrowDown, Modifiers::SHIFT).settle();
        assert_eq!(*steps.borrow(), vec![1.0, -10.0]);
    }
}
