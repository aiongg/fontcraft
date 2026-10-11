//! The Font Info window: one per document, with the Font, Masters, Exports, Other Settings and
//! Notes tabs ([`Tab`]). Every control acts through `app.run` with an engine command
//! (`font.info.*`, `font.customParameters.*`, `font.master.*`, `font.instance.*`,
//! `font.metrics.*`, `font.stems.*`, `font.settings.*`, `font.note.*`); the window keeps only view
//! state ([`FontInfoState`]).
//!
//! | Module | Package | What it draws |
//! |---|---|---|
//! | `mod.rs` | M7.15 | the window, the tab bar, the `font.info` and `font.info.selectTab` commands |
//! | [`rows`] | M7.15 | the shared row: plus and minus, Alt, selection, Copy and Paste, localizable rows |
//! | [`param_value`] | M7.15 | the value editor of each parameter kind |
//! | [`font_tab`] | M7.16 | Family Name, Units per Em, Version, Date, General fields, Axes, Custom Parameters |
//! | [`masters_tab`] | M7.17 | the master list and its General, Metrics, Stems and Custom Parameters sections |
//! | [`exports_tab`] | M7.18 | the export list, the instance preview, the instance fields |
//! | [`other_tab`] | M7.16 | Other Settings |
//! | [`notes_tab`] | M7.16 | the font note with Markdown highlighting |
//!
//! Rows: `fontInfo`, `font.info`, `font.info.selectTab` (M7.15) and the tab rows of each package.

pub mod exports_tab;
pub mod font_tab;
pub mod masters_tab;
pub mod notes_tab;
pub mod other_tab;
pub mod param_value;
pub mod rows;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// The window's commands (`font.info`, `font.info.selectTab`).
pub const COMMANDS: &[UiCommand] = &[
    // M7.15: the two commands.
];

/// The tabs, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tab {
    #[default]
    Font,
    Masters,
    Exports,
    Other,
    Notes,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Font, Tab::Masters, Tab::Exports, Tab::Other, Tab::Notes];

    /// The `tab` parameter of `font.info.selectTab`: `font`, `masters`, `exports`, `other`,
    /// `notes`.
    pub fn name(self) -> &'static str {
        match self {
            Tab::Font => "font",
            Tab::Masters => "masters",
            Tab::Exports => "exports",
            Tab::Other => "other",
            Tab::Notes => "notes",
        }
    }

    pub fn from_name(name: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The tab bar's label.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Font => "Font",
            Tab::Masters => "Masters",
            Tab::Exports => "Exports",
            Tab::Other => "Other Settings",
            Tab::Notes => "Notes",
        }
    }
}

/// The window's view state. Selections are not saved with the settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontInfoState {
    pub open: bool,
    pub tab: Tab,
    /// Selected rows of the shown tab, as indices into its row list.
    #[serde(skip)]
    pub selected_rows: Vec<usize>,
    /// Selected masters of the Masters tab, as indices into the font's masters.
    #[serde(skip)]
    pub selected_masters: Vec<usize>,
    /// Selected export entries of the Exports tab, as indices into the font's instances.
    #[serde(skip)]
    pub selected_instances: Vec<usize>,
}

/// Runs one of [`COMMANDS`].
pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    // M7.15: open the window, select a tab.
    let _ = app;
    Err(args.err(format!("unknown command `{id}`")))
}

/// Draws the window while it is open: the tab bar, then the shown tab's `show`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.font_info.open {
        return;
    }
    // M7.15: the window and the tab bar.
    let _ = (ui, pending);
}
