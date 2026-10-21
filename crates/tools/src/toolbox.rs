//! The active tool, the temporary tools held with `Space` or with modifiers, and key routing.
//!
//! | Held | Tool while held |
//! |---|---|
//! | `Space` | Hand (`Cmd+Space`: Zoom) |
//! | `Cmd` in the Measurement tool | Select |
//! | `Cmd+Alt+Ctrl` in any other tool but Text | Measurement (coordinates and ruler) |
//!
//! A tool held with modifiers takes over when the modifiers go down and no gesture is in
//! progress, and gives way when they come up and its own gesture is over. The modifiers that
//! hold it are taken out of the events it sees.

use std::fmt;

use serde_json::{Value, json};

use crate::cycling::{remember, tool_for_letter};
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
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
    /// The tool held with modifiers ([`held_tool`]), under a `Space` one.
    held: Option<Box<dyn Tool>>,
}

/// The tool modifiers `mods` hold while `base` is the chosen tool, if any. Off the Mac the
/// Control key reports both `cmd` and `ctrl`, so `Ctrl+Alt` there is `Cmd+Alt+Ctrl`.
fn held_tool(base: &str, mods: Modifiers) -> Option<&'static str> {
    if mods.cmd && mods.alt && mods.ctrl && !matches!(base, "measurement" | "text") {
        Some("measurement")
    } else if base == "measurement" && mods.cmd && !mods.alt {
        Some("select")
    } else {
        None
    }
}

/// `mods` without the keys that hold `tool`.
fn without_hold(tool: &str, mut mods: Modifiers) -> Modifiers {
    match tool {
        "measurement" => {
            mods.cmd = false;
            mods.alt = false;
            mods.ctrl = false;
        }
        "select" => {
            mods.cmd = false;
            mods.ctrl = false;
        }
        _ => {}
    }
    mods
}

impl fmt::Debug for Toolbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Toolbox")
            .field("active", &self.active.id())
            .field("temporary", &self.temporary.as_ref().map(|t| t.id()))
            .field("held", &self.held.as_ref().map(|t| t.id()))
            .finish()
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
        Self { active: Box::new(SelectTool::default()), recent: vec!["select"], temporary: None, held: None }
    }

    /// The tool that receives events now: the temporary one while `Space` is held, else the one
    /// held with modifiers.
    pub fn active_id(&self) -> &'static str {
        match (&self.temporary, &self.held) {
            (Some(t), _) | (None, Some(t)) => t.id(),
            (None, None) => self.active.id(),
        }
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
        output.extend(self.end_held());
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
        match (self.temporary.as_mut(), self.held.as_mut()) {
            (Some(t), _) | (None, Some(t)) => t,
            (None, None) => &mut self.active,
        }
    }

    fn end_held(&mut self) -> Output {
        match self.held.take() {
            Some(mut t) => t.cancel(),
            None => Output::default(),
        }
    }

    /// Takes up or puts down the tool modifiers hold ([`held_tool`]). Nothing changes while a
    /// `Space` tool is held or a gesture is in progress.
    fn hold(&mut self, mods: Modifiers) -> Output {
        let want = held_tool(self.active.id(), mods);
        if want == self.held.as_ref().map(|t| t.id()) || self.temporary.is_some() {
            return Output::default();
        }
        let busy = match &self.held {
            Some(t) => t.busy(),
            None => self.active.busy(),
        };
        if busy {
            return Output::default();
        }
        let out = self.end_held();
        self.held = want.and_then(make_tool);
        out
    }

    /// `mods` as the current tool sees them: without the keys that hold it.
    fn seen(&self, mods: Modifiers) -> Modifiers {
        match (&self.temporary, &self.held) {
            (None, Some(t)) => without_hold(t.id(), mods),
            _ => mods,
        }
    }

    fn end_temporary(&mut self) -> Output {
        match self.temporary.take() {
            Some(mut t) => t.cancel(),
            None => Output::default(),
        }
    }

    /// A pointer event for the current tool. The modifiers it carries may first take up or put
    /// down a held tool; after a release they are checked again, so a held tool whose keys came
    /// up during its gesture gives way.
    pub fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = self.hold(ev.mods);
        let seen = PointerEvent { mods: self.seen(ev.mods), ..*ev };
        out.extend(self.current().pointer(cx, &seen));
        if ev.kind == PointerKind::Up {
            out.extend(self.hold(ev.mods));
        }
        out
    }

    /// The modifier keys changed without the pointer moving: a held tool may come or go, and a
    /// tool in a gesture hears of it.
    pub fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        let mut out = self.hold(mods);
        if self.current().busy() {
            let seen = self.seen(mods);
            out.extend(self.current().modifiers(cx, seen));
        }
        out
    }

    /// A key while Edit View has focus and no text field does. Resolves, in order: `Space` (the
    /// temporary tools), the current tool's own keys, the tool letters, then the Edit View keys
    /// of [`crate::keymap`]. An output that is not `handled` goes on to the global shortcuts.
    pub fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.key == Key::Space {
            // During a drag the tool itself may use Space (the Draw tool moves the node).
            if self.temporary.is_none() && self.current().busy() {
                let mut out = self.current().key(cx, ev);
                out.handled = true;
                return out;
            }
            if !ev.pressed {
                let mut out = self.end_temporary();
                out.handled = true;
                return out;
            }
            if self.temporary.is_none() && !self.current().busy() {
                // In text mode a plain Space types (the shell takes it first) and Cmd+Space pans.
                let zoom = ev.mods.cmd && self.active.id() != "text";
                self.temporary = make_tool(if zoom { "zoom" } else { "hand" });
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
        out.extend(self.end_held());
        out.extend(self.active.cancel());
        out
    }

    /// A gesture is in progress.
    pub fn busy(&self) -> bool {
        self.temporary.as_ref().is_some_and(|t| t.busy()) || self.held.as_ref().is_some_and(|t| t.busy()) || self.active.busy()
    }

    /// What the canvas draws for the current tool.
    pub fn overlays(&self) -> Vec<Overlay> {
        match (&self.temporary, &self.held) {
            (Some(t), _) | (None, Some(t)) => t.overlays(),
            (None, None) => self.active.overlays(),
        }
    }
}
