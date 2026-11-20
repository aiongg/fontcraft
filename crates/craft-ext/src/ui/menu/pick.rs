//! [`Menu::pick`] over marked entries, so a menu whose entries carry the mixed mark or the danger tone picks a value
//! as one over plain entries does.
//!
//! Meant for craft-ui-widgets, `src/menu/popup.rs`: once `Entry` carries the mixed state and the danger flag,
//! `Menu::pick` covers these entries and this goes.

use craft::ui::contract::ShowContainer as _;
use craft::ui::menu::Menu;
use craft::ui::runtime::overlay::CloseReason;
use egui::{Response, Ui};

use super::{MarkedEntry, entry_rows};

/// What [`PickMarked::pick_marked`] returns: the response, how the menu closed and the value of the entry that ran,
/// as [`craft::ui::menu::MenuPick`].
#[derive(Debug)]
pub struct MarkedPick<T> {
    /// The menu's response.
    pub response: Response,
    /// How the menu closed in this pass.
    pub closed: Option<CloseReason>,
    /// The value of the entry that ran this pass.
    pub picked: Option<T>,
}

/// [`Menu::pick`] over [`MarkedEntry`] rows.
pub trait PickMarked {
    /// Shows the menu with one row per entry (`entry_rows`) and returns the value of the entry that ran.
    fn pick_marked<T: Clone>(self, ui: &mut Ui, entries: &[MarkedEntry<T>]) -> MarkedPick<T>;
}

impl PickMarked for Menu<'_> {
    fn pick_marked<T: Clone>(self, ui: &mut Ui, entries: &[MarkedEntry<T>]) -> MarkedPick<T> {
        let shown = self.show(ui, |ui, _| entry_rows(ui, entries));
        let picked = shown.inner.flatten().and_then(|at| entries.get(at)).and_then(MarkedEntry::value).cloned();
        MarkedPick { response: shown.output.response, closed: shown.output.closed, picked }
    }
}
