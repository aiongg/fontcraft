//! The modifiers held when a collection asked for its context menu, so the menu can show alternates (an item that
//! acts on the layer instead of the glyph while Alt is held).
//!
//! Meant for craft-ui-contract, `src/contract/collection/events.rs`: a `mods: Mods` field on `ContextMenuRequest`,
//! filled by the collection from the press or key that made the request, as `ContextRequest::mods` is on the canvas's
//! `ToolHost`. The request's fields are craft's, so this stand-in reads the modifiers of the pass instead. A request
//! is handed out in the pass of the press or key that made it, so these are the modifiers held at that press.

use craft::keymap::Mods;
use craft::ui::contract::ContextMenuRequest;
use craft::ui::runtime::windows::os_of;
use egui::Context;

/// The modifiers held in this pass, folded for the shell's platform (Ctrl is the command modifier off macOS).
pub fn held_mods(ctx: &Context) -> Mods {
    let os = os_of(ctx);
    ctx.input(|input| Mods::from_egui(&input.modifiers, os))
}

/// The modifiers a context menu request was made with.
pub trait RequestMods {
    /// The modifiers held at the press or key that made the request. Read it in the pass the request arrived in.
    fn mods(&self, ctx: &Context) -> Mods;
}

impl<K> RequestMods for ContextMenuRequest<K> {
    fn mods(&self, ctx: &Context) -> Mods {
        held_mods(ctx)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};

    use super::*;

    /// The modifiers of the pass a key arrives in are the ones held with it.
    #[test]
    fn the_modifiers_held_in_the_pass_are_read() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| out.borrow_mut().push(held_mods(ui.ctx())));
        fixture.settle();
        assert!(seen.borrow().iter().all(|mods| !mods.alt), "nothing is held at first");
        fixture.key(Key::F10, Modifiers::ALT).settle();
        assert!(seen.borrow().iter().any(|mods| mods.alt && !mods.shift), "{:?}", seen.borrow());
    }
}
