//! The command palette (`app.commandPalette`) over the window, and the console's clipboard output.
//!
//! The palette lists the commands of the menus with their menu path and shortcut. A pick closes it and fires the
//! command as a menu row does, so it reaches the app through the same catalog route; a disabled command is listed
//! and does not run.

use craft::ui::contract::{Bind, BodyCx};
use craft::ui::menu::search::{SearchIndex, SearchWords, Synonyms};
use craft::ui::patterns::panels::CommandPalette;
use craft::ui::runtime::commands::{ClipItem, Commands as _};
use craft::ui::runtime::env::Env;
use egui::Ui;

use crate::app::FontCraftApp;

craft::ui::words! {
    /// The words of the command palette.
    pub struct PaletteWords {
        /// The palette's name.
        title: "fontcraftPalette.title" = "Commands";
    }
}

/// Shows the palette while it is open and hands text the console copied to the clipboard.
pub fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut FontCraftApp) {
    let surfaces = &mut app.view.commands;
    if let Some(text) = surfaces.console.copy_out.take() {
        cx.clipboard().put(&[ClipItem::Text(text.into())]);
    }
    if !surfaces.palette_open {
        return;
    }
    let env = Env::of(ui);
    let words = env.words::<PaletteWords>();
    let entries = &surfaces.palette;
    let index = surfaces.palette_index.get_or_insert_with(|| {
        let view = env.view();
        SearchIndex::new(&view, entries, &Synonyms::of(&view.words::<SearchWords>()))
    });
    let out = CommandPalette::new(words.title.clone(), entries, index, Bind::given(true)).show(ui);
    if out.closed.is_some() {
        surfaces.palette_open = false;
    }
}
