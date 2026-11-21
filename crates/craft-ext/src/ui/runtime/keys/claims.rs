//! Chords a dialog, a panel or a focused component claims for itself, so the window's shortcuts, the tools and the
//! focused control do not also take them: the arrows that move a dialog's selected point while no field has focus, or
//! Ctrl+Up that a focused list turns into a command of its own instead of a move.
//!
//! Meant for craft-ui-contract, `src/runtime/keys/claims.rs` and `src/runtime/focus/policy.rs`: there a focus scope (a
//! dialog's, a pane's) carries `claims` beside its `FocusPolicy`, held while focus is inside the scope and the focused
//! stop neither keeps the chord nor types with it, and the key stage routes a claimed press to the scope's container
//! as an intent; a component's `KeyUse::claims` takes a per-instance list the same way. craft holds claims for the
//! focused component only (`keys::claim`) or for the whole window whatever has focus (`keys::capture`, which would
//! take a focused text field's keys), so this stand-in builds the scope's rule on those two:
//!
//! - [`region_claims`]: a region (a modal dialog's body, a panel) claims chords while it shows. Each pass it renews a
//!   capture of the chords the focused stop does not hold itself, and none while a text or editor part has focus.
//! - [`focused_claims`]: a component claims chords while it is the focused component, past what its key use lists.
//!
//! Both return the claimed presses that arrived in this pass. A claim made in a pass holds for the next pass's presses
//! and lapses unless renewed, so the caller calls these every pass the claim should hold (a dialog's claims end when
//! it closes, a panel's when it hides).

use craft::keymap::Chord;
use craft::ui::runtime::{focus, keys};
use egui::{Context, Id};

/// Most presses of one chord read in a pass: a stuck key repeats, it does not flood.
pub const MAX_PRESSES: usize = 64;

/// The presses of `chords` claimed for `owner` in this pass, chord by chord in the order of `chords`.
fn pressed(ctx: &Context, owner: Id, chords: &[Chord]) -> Vec<Chord> {
    let mut out = Vec::new();
    for chord in chords {
        let mut presses = 0;
        while presses < MAX_PRESSES && keys::take_claimed(ctx, owner, *chord) {
            out.push(*chord);
            presses += 1;
        }
    }
    out
}

/// Whether a text or editor part has focus: it types with plain keys and moves its caret with the arrows, so a region
/// claims nothing from it.
fn typing(ctx: &Context) -> bool {
    focus::focused_rec(ctx).is_some_and(|rec| rec.stop.text)
}

/// The presses of `chords` the region `owner` claimed in this pass; and the claim, from the next pass on, of every
/// chord in `chords` that the focused stop does not hold (a field steps its number with the arrows, a list moves its
/// rows), none while a text part has focus. `owner` is any id stable across passes, unique to the region.
pub fn region_claims(ctx: &Context, owner: Id, chords: &[Chord]) -> Vec<Chord> {
    let got = pressed(ctx, owner, chords);
    if !typing(ctx) {
        for chord in chords {
            if !keys::focus_holds(ctx, *chord) {
                keys::capture(ctx, owner, *chord);
            }
        }
    }
    got
}

/// The presses of `chords` the component `owner` (its response's id) claimed in this pass; and the claim of `chords`
/// from the next pass on, which holds while that component is the focused one. A claimed chord reaches neither the
/// component's own key handling nor the window's shortcuts.
pub fn focused_claims(ctx: &Context, owner: Id, chords: &[Chord]) -> Vec<Chord> {
    let got = pressed(ctx, owner, chords);
    for chord in chords {
        keys::claim(ctx, owner, *chord);
    }
    got
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::behavior::keyed;
    use craft::keymap::{Key, Mods};
    use craft::ui::buttons::Button;
    use craft::ui::contract::{Bind, Show as _, ShowCollection as _};
    use craft::ui::list::{List, ListRow};
    use craft::ui::testing::Fixture;
    use craft::ui::text::TextField;
    use egui::Modifiers;

    use super::*;

    /// With nothing focused, or a button, the region takes Up; a focused text field keeps it.
    #[test]
    fn a_region_claims_what_the_focused_stop_leaves() {
        let up = Chord::plain(Key::Up);
        let got = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&got);
        let mut fixture = Fixture::new(move |ui| {
            let _ = TextField::new("Name", Bind::given(String::new())).show(ui);
            let _ = Button::new("Apply").show(ui);
            seen.borrow_mut().extend(region_claims(ui.ctx(), Id::unique("region"), &[up]));
        });
        fixture.settle();
        fixture.key(egui::Key::ArrowUp, Modifiers::NONE).settle();
        assert_eq!(*got.borrow(), vec![up], "nothing focused: the region takes Up");

        let button = fixture.by_label("Apply").expect("the button");
        fixture.click(&button).settle();
        fixture.key(egui::Key::ArrowUp, Modifiers::NONE).settle();
        assert_eq!(*got.borrow(), vec![up, up], "a button leaves the arrows to what is around it");

        let field = fixture.by_label("Name").expect("the field");
        fixture.click(&field).settle();
        fixture.key(egui::Key::ArrowUp, Modifiers::NONE).settle();
        assert_eq!(got.borrow().len(), 2, "a focused text field keeps its keys");
    }

    /// A focused list gets Alt+Up as a claimed chord, and nothing once it has lost the focus.
    #[test]
    fn a_focused_component_takes_its_claimed_chords() {
        let alt_up = Chord::new(Mods::ALT, Key::Up);
        let got = Rc::new(RefCell::new(Vec::new()));
        let id = Rc::new(RefCell::new(None));
        let (seen, list_id) = (Rc::clone(&got), Rc::clone(&id));
        let mut fixture = Fixture::new(move |ui| {
            let positions: Vec<usize> = vec![0, 1];
            let source = keyed(&positions, |at| *at);
            let out = List::<usize>::new("Entries")
                .header(false)
                .natural_height(true)
                .show_items(ui, &source, &mut |at, icx| ListRow::new(format!("entry {at}")).state(icx.row.clone()));
            seen.borrow_mut().extend(focused_claims(ui.ctx(), out.response.id, &[alt_up]));
            *list_id.borrow_mut() = Some(out.response.id);
        });
        fixture.settle();
        fixture.key(egui::Key::ArrowUp, Modifiers::ALT).settle();
        assert!(got.borrow().is_empty(), "a list without focus claims nothing");

        let list = id.borrow().expect("the list showed");
        fixture.ctx().memory_mut(|m| m.request_focus(list));
        fixture.settle();
        fixture.key(egui::Key::ArrowUp, Modifiers::ALT).settle();
        assert_eq!(*got.borrow(), vec![alt_up]);
    }
}
