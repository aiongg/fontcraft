//! The options bar and the status bar.

use craft::ui::bars::{Bar, BarKind};
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::runtime::commands::{self, CommandRef};
use craft::ui::runtime::env::Env;
use egui::Ui;

use super::ChromeWords;
use crate::app::catalog::TOOL_PREFIX;

/// What the options bar reads: the current tool, whose options it will show.
pub struct OptionsProps {
    pub tool: &'static str,
}

pub fn options(ui: &mut Ui, mut props: OptionsProps) {
    options_view(&mut props, ui);
}

fn options_view(props: &mut OptionsProps, ui: &mut Ui) {
    let words = Env::of(ui).words::<ChromeWords>();
    let tool = commands::resolve(ui.ctx(), CommandRef::new(format!("{TOOL_PREFIX}{}", props.tool))).label;
    let _ = Bar::new(words.options.clone(), BarKind::Options).show(ui, |ui, _| {
        Text::new(tool.to_string()).show(ui);
    });
}

/// What the status bar reads.
pub struct StatusProps<'a> {
    /// The last error or notice; empty when there is none.
    pub status: &'a str,
}

pub fn status(ui: &mut Ui, mut props: StatusProps<'_>) {
    status_view(&mut props, ui);
}

fn status_view(props: &mut StatusProps<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ChromeWords>();
    let _ = Bar::new(words.status.clone(), BarKind::Status).show(ui, |ui, _| {
        if props.status.is_empty() {
            Text::new(words.ready.clone()).show(ui);
        } else {
            Text::new(props.status.to_string()).show(ui);
        }
    });
}
