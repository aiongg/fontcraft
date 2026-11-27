//! The window: title bar with the menu bar, tool rail, options bar, status bar, and the dock in the body.
//!
//! The shell is the only UI code that holds `&mut FontCraftApp`. It hands each view its props and nothing else.

pub mod axis_sweep;
pub mod bars;
pub mod bitmap_editor;
pub mod command_palette;
pub mod command_surfaces;
pub mod commands;
pub mod compare_family;
pub mod compare_fonts;
pub mod compare_in_list;
pub mod compatibility;
pub mod components;
pub mod context_menu;
pub mod dock;
pub mod encoding;
pub mod export_names;
pub mod extension_downloads;
pub mod extension_manager;
pub mod extensions;
pub mod external_changes;
pub mod feature_work;
pub mod font_audit;
pub mod font_info;
pub mod font_info_masters;
pub mod font_info_tabs;
pub mod font_view;
pub mod getters;
pub mod glyph_info;
pub mod images;
pub mod kept_layout;
pub mod kerning_review;
pub mod kerning_tools;
pub mod kerning_window;
pub mod macro_editor;
pub mod masters;
pub mod numbered_keys;
pub mod overlays;
pub mod palette;
pub mod palette_modules;
pub mod params;
pub mod preview;
pub mod project;
pub mod rail;
pub mod reference_font;
pub mod selection;
pub mod settings;
pub mod shape_order;
pub mod show_all_masters;
pub mod sidebar;
pub mod space_center;
pub mod start;
pub mod table_viewer;
pub mod tabs;
pub mod text_preview;
pub mod url;
pub mod view;
pub mod window;

use std::borrow::Cow;

use craft::menu::MenuBarUi;
use craft::ui::contract::{BodyCx, Tracked};
use craft::ui::runtime::env::{Env, WindowKey};
use craft::ui::shell::{AppShell, ShellDialogsCx as _, Slot, SlotSize, Title, TitleBar};
use egui::Ui;

use crate::app::FontCraftApp;

craft::ui::words! {
    /// The words of the window's own chrome.
    pub struct ChromeWords {
        /// The tool rail's name.
        tools: "fontcraftShell.tools" = "Tools";
        /// The options bar's name.
        options: "fontcraftShell.options" = "Options";
        /// The status bar's name.
        status: "fontcraftShell.status" = "Status";
        /// After the title of a document that Save cannot write back.
        view_only: "fontcraftShell.viewOnly" = "View Only";
        /// The notice bar's name.
        notice: "fontcraftShell.notice" = "Document notice";
        /// The status line with nothing to report.
        ready: "fontcraftShell.ready" = "Ready";
        /// The document area's name.
        document: "fontcraftShell.document" = "Glyphs";
        /// The canvas's scroll bars, as assistive technology names them.
        scroll_bars: "fontcraftShell.scrollBars" = "Scroll bars";
        /// The options bar's button that shows or hides the Palette.
        palette: "fontcraftShell.palette" = "Palette";
    }
}

/// Shows the window for one frame.
pub fn show(app: &mut FontCraftApp, ui: &mut Ui) {
    let notice = crate::app::open_report::notice(app);
    AppShell::new(WindowKey::main())
        .title_bar(title_bar)
        .options_bar(true, |ui, _, app: &mut FontCraftApp| {
            let palette = window::palette_shows(app);
            bars::options(ui, bars::OptionsProps { tool: app.view.tool, palette, sent: &mut app.sent })
        })
        .top(Slot::new("fontcraft.notice").visible(notice.is_some()), move |ui, _, app: &mut FontCraftApp| {
            if let Some(notice) = &notice {
                bars::notice(ui, notice, &mut app.sent);
            }
        })
        .status(true, |ui, _, app: &mut FontCraftApp| {
            let edit = bars::edit_state(app);
            let hoi = crate::canvas::hoi::axis_choice(app);
            let FontCraftApp { status, models, sent, .. } = app;
            bars::status(ui, bars::StatusProps { status, reason: models.compatibility.line(), features: &models.features, edit, hoi, sent });
        })
        .start(Slot::new("fontcraft.tools").size(SlotSize::RailWidth), |ui, _, app: &mut FontCraftApp| {
            let FontCraftApp { tool_slots, view, sent, .. } = app;
            rail::show(ui, rail::RailProps { slots: tool_slots, current: &view.tool, last_used: &view.tools_last_used, sent });
        })
        .body(body)
        .show(ui, app);
}

/// The document's title, with the view-only mark after it while Save has no format to write it back in
/// (`open.compiledIsViewOnly`).
fn document_title(doc: &fontcraft_engine::DocState, words: &ChromeWords) -> String {
    if doc.view_only() { format!("{} ({})", doc.title, words.view_only.as_str()) } else { doc.title.clone() }
}

fn title_bar(ui: &mut Ui, _: &mut BodyCx<'_>, app: &mut FontCraftApp) {
    let title = match app.session.active() {
        Some(doc) => {
            let text = document_title(doc, &Env::of(ui).words::<ChromeWords>());
            if doc.modified() { Title::unsaved(text) } else { Title::new(text) }
        }
        None => Title::new("FontCraft"),
    };
    let held = craft_ext::menu::held_alternate_labels(ui, &app.catalog.menu, |command| {
        crate::app::menu_alternates::alternate_label(&command.id).map(str::to_string)
    });
    // While a held modifier relabels rows the bar is a value of this pass alone; otherwise the catalog's own.
    let relabelled = match held {
        Cow::Owned(bar) => Some(Tracked::new(bar)),
        Cow::Borrowed(_) => None,
    };
    let menu = relabelled.as_ref().unwrap_or(&app.catalog.menu);
    let documents = crate::app::document_menu::open_documents(&app.session);
    let several = documents.len() > 1;
    let mut button = None;
    let opener = &mut button;
    let _ = TitleBar::new().show_titled(
        ui,
        title,
        |ui, _| {
            MenuBarUi::new(menu).show(ui);
        },
        move |ui, _| {
            if several {
                *opener = Some(crate::app::document_menu::button(ui));
            }
        },
    );
    // Outside the bar's own layout passes, where a popup would show in a pass that is thrown away.
    let picked = button.and_then(|button| crate::app::document_menu::show_menu(ui, &button, &documents));
    if let Some(command) = picked {
        app.fire(command);
    }
}

fn body(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut FontCraftApp) {
    dock::show(ui, app);
    command_palette::show(ui, cx, app);
    context_menu::show(ui, cx, app);
    crate::canvas::text::edit::shell_pass(ui, cx, app);
    if let Some(dialogs) = cx.dialogs() {
        dialogs.with(|host| crate::dialogs::run(host, app));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod access_tests;
