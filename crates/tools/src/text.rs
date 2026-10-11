//! The Text tool (`T`): Edit View's text mode. The tab's text, its cursor and the keys that edit
//! them belong to the shell, which holds the text; the tool marks the mode, so its keys and the
//! tool letters do not reach the other tools. Pressing `Space` types a space, and `Cmd+Space`
//! holds the Hand tool (`pan.spaceInTextMode`).

use crate::event::PointerEvent;
use crate::output::Output;
use crate::tool::{Context, Tool};

/// The Text tool.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextTool;

impl Tool for TextTool {
    fn id(&self) -> &'static str {
        "text"
    }

    /// The shell places the text cursor before pointer events reach the tools, so none comes here.
    fn pointer(&mut self, _cx: &Context<'_>, _ev: &PointerEvent) -> Output {
        Output::default()
    }

    fn cancel(&mut self) -> Output {
        Output::default()
    }

    fn busy(&self) -> bool {
        false
    }
}
