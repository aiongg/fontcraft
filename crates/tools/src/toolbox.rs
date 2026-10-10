//! The active tool, the temporary tools held with `Space`, and key routing.

use std::fmt;

use serde_json::{Value, json};

use crate::cycling::{remember, tool_for_letter};
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent};
use crate::keymap::edit_view_key;
use crate::output::{Action, Invocation, Output, Overlay};
use crate::select::SelectTool;
use crate::tool::{Context, TOOLS, Tool, make_tool};

/// The result of `tool.select`.
#[derive(Debug)]
pub struct Switch {
    pub tool: &'static str,
    pub previous: &'static str,
    /// What leaving the previous tool asks for (a `Cancel` when a drag was in progress).
    pub output: Output,
}

/// The tools of one Edit View: the active one, and a temporary one while `Space` is held.
pub struct Toolbox {
    active: Box<dyn Tool>,
    /// The tools chosen so far, the latest last: a letter shared by several tools returns to the
    /// one used last (`tools.sharedButtonCycling`).
    recent: Vec<&'static str>,
    temporary: Option<Box<dyn Tool>>,
}

impl fmt::Debug for Toolbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Toolbox").field("active", &self.active.id()).field("temporary", &self.temporary.as_ref().map(|t| t.id())).finish()
    }
}

impl Default for Toolbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Toolbox {
    /// A toolbox with the Select tool active.
    pub fn new() -> Self {
        Self { active: Box::new(SelectTool::default()), recent: vec!["select"], temporary: None }
    }

    /// The tool that receives events now: the temporary one while `Space` is held.
    pub fn active_id(&self) -> &'static str {
        self.temporary.as_ref().map_or_else(|| self.active.id(), |t| t.id())
    }

    /// The chosen tool, ignoring a temporary one.
    pub fn base_id(&self) -> &'static str {
        self.active.id()
    }

    /// `tool.select`: makes `id` the active tool. A gesture in progress is cancelled.
    pub fn select(&mut self, id: &str) -> Result<Switch, String> {
        let Some(tool) = make_tool(id) else {
            let known: Vec<&str> = TOOLS.iter().map(|t| t.id).collect();
            return Err(format!("unknown tool `{id}`; the tools are {}", known.join(", ")));
        };
        let previous = self.active.id();
        let mut output = self.end_temporary();
        if previous != tool.id() {
            output.extend(self.active.cancel());
            self.active = tool;
        }
        let now = self.active.id();
        remember(&mut self.recent, now);
        Ok(Switch { tool: now, previous, output })
    }

    /// The tool of toolbar group `group` chosen last, if any of its tools was chosen.
    pub fn last_in_group(&self, group: &str) -> Option<&'static str> {
        self.recent.iter().rev().copied().find(|id| TOOLS.iter().any(|t| t.id == *id && !t.group.is_empty() && t.group == group))
    }

    /// `tool.list`: `{active, tools: [{id, label, shortcut, group}]}`.
    pub fn list(&self) -> Value {
        json!({ "active": self.active_id(), "tools": TOOLS })
    }

    fn current(&mut self) -> &mut Box<dyn Tool> {
        match self.temporary.as_mut() {
            Some(t) => t,
            None => &mut self.active,
        }
    }

    fn end_temporary(&mut self) -> Output {
        match self.temporary.take() {
            Some(mut t) => t.cancel(),
            None => Output::default(),
        }
    }

    /// A pointer event for the current tool.
    pub fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        self.current().pointer(cx, ev)
    }

    /// The modifier keys changed without the pointer moving.
    pub fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        self.current().modifiers(cx, mods)
    }

    /// A key while Edit View has focus and no text field does. Resolves, in order: `Space` (the
    /// temporary tools), the current tool's own keys, the tool letters, then the Edit View keys
    /// of [`crate::keymap`]. An output that is not `handled` goes on to the global shortcuts.
    pub fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.key == Key::Space {
            // During a drag the tool itself may use Space (the Draw tool moves the node).
            if self.temporary.is_none() && self.active.busy() {
                let mut out = self.active.key(cx, ev);
                out.handled = true;
                return out;
            }
            if !ev.pressed {
                let mut out = self.end_temporary();
                out.handled = true;
                return out;
            }
            if self.temporary.is_none() && !self.active.busy() {
                self.temporary = make_tool(if ev.mods.cmd { "zoom" } else { "hand" });
            }
            return Output::handled();
        }
        if !ev.pressed {
            return Output::default();
        }
        let out = self.current().key(cx, ev);
        if out.handled {
            return out;
        }
        if let Some(action) = self.letter(ev) {
            return Output::with(action);
        }
        edit_view_key(cx, ev).map_or_else(Output::default, Output::with)
    }

    /// The tool letters of [`TOOLS`] (`Shift` steps through the tools sharing a letter), as
    /// `tool.select` invocations.
    fn letter(&self, ev: &KeyEvent) -> Option<Action> {
        let Key::Char(c) = ev.key else { return None };
        if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
            return None;
        }
        let id = tool_for_letter(c, ev.mods.shift, self.active.id(), &self.recent)?;
        Some(Action::Run { invocation: Invocation::new("tool.select", json!({ "tool": id })) })
    }

    /// Cancels the gesture of the current tool and drops a temporary tool.
    pub fn cancel(&mut self) -> Output {
        let mut out = self.end_temporary();
        out.extend(self.active.cancel());
        out
    }

    /// A gesture is in progress.
    pub fn busy(&self) -> bool {
        self.temporary.as_ref().is_some_and(|t| t.busy()) || self.active.busy()
    }

    /// What the canvas draws for the current tool.
    pub fn overlays(&self) -> Vec<Overlay> {
        match &self.temporary {
            Some(t) => t.overlays(),
            None => self.active.overlays(),
        }
    }
}
