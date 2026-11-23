//! What views send to the app layer.

use craft::ui::rails::LastUsed;
use craft::ui::runtime::commands::CommandRef;

use crate::canvas::tools::ToolInput;
use crate::shell::dock::PanelId;

/// A message a view posts. The app layer applies them in [`FontCraftApp::drain`](super::FontCraftApp::drain), after
/// the frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// An engine command with explicit targets in its parameters: one undo step.
    Run(CommandRef),
    /// A shell command (`shell::commands`): view and window state, no engine change.
    Shell(CommandRef),
    /// A step of an interaction: a drag previews commands on a snapshot and ends as one undo step or as nothing.
    Preview(Preview),
    /// Pointer, modifier or key input for the active tool, in the edited glyph's font units. `drain` hands it to the
    /// toolbox, whose answer comes back as more messages.
    Tool(ToolInput),
    /// View state that is no command: the current tool, the shown glyph, the dock.
    View(ViewMsg),
}

/// One step of an interaction (`Session::begin`, `preview`, `commit`, `cancel`).
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    /// Take a snapshot of the document; the label names the undo step.
    Begin(String),
    /// Run a command on the snapshot without recording history.
    Update(CommandRef),
    /// Keep the last preview as one undo step.
    Commit,
    /// Put the snapshot back, and the selection as it was at `Begin`.
    Cancel,
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
    /// The Layers row whose context menu is open; `None` when it closed.
    LayerMenu(Option<String>),
    /// The Text Preview's line length and the lines in view, as its text area reports them.
    TextPreviewArea(crate::shell::text_preview::Area),
    /// The sidebar entry whose context menu is open; `None` when it closed.
    SidebarMenu(Option<Vec<String>>),
    /// The feature code editor asks completions for its code and caret; `None` drops the question.
    FeatureCompletion(Option<crate::shell::font_info_tabs::CompletionAsk>),
    /// The code as typed in the Features tab's editor, or `None` once it is the stored code again.
    FeatureDraft(Option<crate::shell::font_info_tabs::FeatureDraft>),
    /// The feature code editor placed the caret a snippet left.
    FeatureCaretPlaced,
    /// The feature code editor asks a preview of an Alt-clicked name; `None` closes it.
    FeaturePreview(Option<crate::shell::font_info_tabs::PreviewAsk>),
    /// The feature code editor's find bar changed, or closed (`None`).
    FeatureFind(Option<crate::shell::font_info_tabs::FindState>),
    /// Font View's search field took the keyboard (`fontView.focusSearch`).
    FontViewSearchFocused,
    /// A press, drag or release on the Bitmap Glyph Editor's pixel canvas (`shell::bitmap_editor::input`).
    Bitmap(crate::shell::bitmap_editor::BitmapInput),
}

impl Msg {
    /// An engine command `id` with `params`.
    pub fn run(id: &str, params: serde_json::Value) -> Msg {
        Msg::Run(CommandRef::with_params(id, params))
    }

    /// The id of the command the message runs; `None` for view changes, previews and tool input.
    pub fn command_id(&self) -> Option<&str> {
        match self {
            Msg::Run(command) | Msg::Shell(command) => Some(&command.id),
            Msg::View(_) | Msg::Preview(_) | Msg::Tool(_) => None,
        }
    }
}
