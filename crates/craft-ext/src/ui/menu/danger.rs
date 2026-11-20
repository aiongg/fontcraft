//! The danger tone on a menu entry: the mark of a row whose command destroys something (Delete…, Remove…).
//!
//! Meant for craft-ui-contract, `src/contract/entry.rs` (a `danger` flag on `Entry`, as `DialogRole::Destructive` is
//! on a dialog button) and craft-ui-widgets, `src/menu/item.rs` (a `MenuItemStyleKey` tone whose style reads the
//! label, icon and mark in `StyleCx::tone(Tone::Bad)` while the row rests, and the row's own inks when it is active
//! or disabled). The row's inks are private to craft, so this stand-in shows the tone on the row's icon only: the
//! entry's own icon, or the `trash` icon of craft's set, in the bad status ink while the row is enabled. The label
//! keeps the normal ink, and an active row keeps the danger ink on the row fill. Dialog buttons already have the tone
//! in craft (`DialogRole::Destructive`, `Variant::DANGER`); only menu rows need this piece.

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Env;
use craft::ui::style::StyleCx;
use egui::Ui;

/// The icon a danger row without one of its own shows: craft's Delete command icon.
pub const DANGER_ICON: IconId = IconId::new("trash");

/// The icon a danger row shows: `own`, else [`DANGER_ICON`], in the bad status ink while `enabled`, in the row's own
/// ink when disabled.
pub fn danger_icon(ui: &Ui, enabled: bool, own: Option<&IconSpec<'static>>) -> IconSpec<'static> {
    let icon = own.cloned().unwrap_or_else(|| IconSpec::svg(DANGER_ICON));
    if !enabled {
        return icon;
    }
    let env = Env::of(ui);
    let cx = StyleCx::of_env(ui.ctx(), &env);
    icon.color(cx.tone(Tone::Bad))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::overlay::Anchor;
    use craft::ui::contract::{Bind, Entry, ShowContainer as _};
    use craft::ui::menu::Menu;
    use craft::ui::primitives::has_icon;
    use craft::ui::testing::Fixture;
    use egui::pos2;

    use super::*;
    use crate::ui::menu::{MarkedEntry, entry_rows};

    #[test]
    fn the_danger_icon_is_in_the_set_and_takes_the_bad_ink_only_while_enabled() {
        assert!(has_icon(&DANGER_ICON));
        let seen = Rc::new(RefCell::new(None));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let env = Env::of(ui);
            let bad = StyleCx::of_env(ui.ctx(), &env).tone(Tone::Bad);
            *out.borrow_mut() = Some((danger_icon(ui, true, None).color, danger_icon(ui, false, None).color, bad));
        });
        fixture.settle();
        let (enabled, disabled, bad) = seen.borrow().expect("the pass ran");
        assert_eq!(enabled, Some(bad));
        assert_eq!(disabled, None);
    }

    /// A danger entry shows among plain ones and runs like them.
    #[test]
    fn a_danger_entry_shows_and_runs() {
        let picked = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&picked);
        let mut fixture = Fixture::new(move |ui| {
            let mut open = true;
            let opener = ui.response();
            let entries = [MarkedEntry::new(Entry::new("copy", "Copy")), MarkedEntry::new(Entry::new("remove", "Remove Glyph")).danger(true)];
            let shown = Menu::context("Glyph Menu", &opener)
                .anchor(Anchor::Point(pos2(20.0, 20.0)))
                .open(Bind::bound(&mut open))
                .show(ui, |ui, _| entry_rows(ui, &entries));
            if let Some(at) = shown.inner.flatten() {
                *seen.borrow_mut() = entries.get(at).and_then(MarkedEntry::value).copied();
            }
        });
        fixture.settle();
        let row = fixture.by_label("Remove Glyph").expect("the danger row");
        fixture.click(&row).settle();
        assert_eq!(*picked.borrow(), Some("remove"));
    }
}
