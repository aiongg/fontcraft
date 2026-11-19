//! The tool rail: one slot per tool group. Picking a tool fires its `tool.<id>` command, which the app layer turns
//! into `ViewMsg::SelectTool`; the tools themselves arrive with the canvas port.

use craft::ui::rails::{LastUsed, ToolRail, ToolSlot};
use craft::ui::runtime::env::Env;
use egui::Ui;

use super::ChromeWords;
use crate::app::{Msg, ViewMsg};

/// What the rail reads and sends.
pub struct RailProps<'a> {
    pub slots: &'a [ToolSlot<&'static str>],
    pub current: &'a &'static str,
    pub last_used: &'a LastUsed<&'static str>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: RailProps<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut RailProps<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ChromeWords>();
    let out = ToolRail::new(words.tools.clone(), props.slots, props.current, props.last_used).show_column(ui, |_, _| {});
    if let Some(tool) = out.select {
        props.sent.push(Msg::View(ViewMsg::SelectTool(tool)));
    }
    if let Some(last_used) = out.last_used {
        props.sent.push(Msg::View(ViewMsg::ToolsLastUsed(last_used)));
    }
}
