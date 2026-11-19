//! What views send to the app layer.

use craft::ui::rails::LastUsed;
use craft::ui::runtime::commands::CommandRef;

use crate::shell::dock::PanelId;

/// A message a view posts. The app layer applies them in [`FontCraftApp::drain`](super::FontCraftApp::drain), after
/// the frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// An engine command with explicit targets in its parameters: one undo step.
    Run(CommandRef),
    /// A shell command (`shell::commands`): view and window state, no engine change.
    Shell(CommandRef),
    /// View state that is no command: the current tool, the shown glyph, the dock.
    View(ViewMsg),
}

/// A change to view state.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewMsg {
    /// Make the tool with this id (`fontcraft_tools::TOOLS`) current.
    SelectTool(&'static str),
    /// The tool each rail slot showed last, as the rail reports it.
    ToolsLastUsed(LastUsed<&'static str>),
    /// Show this glyph in the glyph panels.
    SelectGlyph(String),
    /// Show or hide a panel.
    TogglePanel(PanelId),
}

impl Msg {
    /// An engine command `id` with `params`.
    pub fn run(id: &str, params: serde_json::Value) -> Msg {
        Msg::Run(CommandRef::with_params(id, params))
    }

    /// The id of the command the message runs; `None` for a view message.
    pub fn command_id(&self) -> Option<&str> {
        match self {
            Msg::Run(command) | Msg::Shell(command) => Some(&command.id),
            Msg::View(_) => None,
        }
    }
}
