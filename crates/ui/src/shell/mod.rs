//! The window: title bar with the menu bar, tool rail, options bar, status bar, and the dock in the body.
//!
//! The shell is the only UI code that holds `&mut FontCraftApp`. It hands each view its props and nothing else.

pub mod bars;
pub mod command_palette;
pub mod command_surfaces;
pub mod commands;
pub mod compare_family;
pub mod compare_fonts;
pub mod compatibility;
pub mod components;
pub mod context_menu;
pub mod dock;
pub mod extensions;
pub mod external_changes;
pub mod font_audit;
pub mod font_info;
pub mod font_info_masters;
pub mod font_info_tabs;
pub mod font_view;
pub mod glyph_info;
pub mod kerning_review;
pub mod kerning_tools;
pub mod kerning_window;
pub mod masters;
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
pub mod sidebar;
pub mod space_center;
pub mod start;
pub mod table_viewer;
pub mod text_preview;
pub mod url;
pub mod view;
pub mod window;

use craft::menu::MenuBarUi;
use craft::ui::contract::BodyCx;
use craft::ui::runtime::env::WindowKey;
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
        /// The status line with nothing to report.
        ready: "fontcraftShell.ready" = "Ready";
        /// The document area's name.
        document: "fontcraftShell.document" = "Glyphs";
    }
}

/// Shows the window for one frame.
pub fn show(app: &mut FontCraftApp, ui: &mut Ui) {
    AppShell::new(WindowKey::main())
        .title_bar(title_bar)
        .options_bar(true, |ui, _, app: &mut FontCraftApp| bars::options(ui, bars::OptionsProps { tool: app.view.tool }))
        .status(true, |ui, _, app: &mut FontCraftApp| {
            let FontCraftApp { status, models, sent, .. } = app;
            bars::status(ui, bars::StatusProps { status, features: &models.features, sent });
        })
        .start(Slot::new("fontcraft.tools").size(SlotSize::RailWidth), |ui, _, app: &mut FontCraftApp| {
            let FontCraftApp { tool_slots, view, sent, .. } = app;
            rail::show(ui, rail::RailProps { slots: tool_slots, current: &view.tool, last_used: &view.tools_last_used, sent });
        })
        .body(body)
        .show(ui, app);
}

fn title_bar(ui: &mut Ui, _: &mut BodyCx<'_>, app: &mut FontCraftApp) {
    let title = match app.session.active() {
        Some(doc) if doc.modified() => Title::unsaved(doc.title.clone()),
        Some(doc) => Title::new(doc.title.clone()),
        None => Title::new("FontCraft"),
    };
    let menu = &app.catalog.menu;
    let _ = TitleBar::new().show_titled(
        ui,
        title,
        |ui, _| {
            MenuBarUi::new(menu).show(ui);
        },
        |_, _| {},
    );
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
