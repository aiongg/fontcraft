//! The Edit View keys the tools answer (D1: every key without `Cmd` has a `when`, so it never
//! fires while a text field has focus).
//!
//! The shell resolves a key in this order: the focused text field or panel, then the active tool
//! (`tool:<id>`, e.g. `Escape` during a gesture), then these Edit View keys, then the global
//! shortcuts of the menus. [`Toolbox::key`](crate::Toolbox::key) does the middle two.

use serde_json::{Map, Value, json};

use crate::event::{Key, KeyEvent};
use crate::output::{Action, Invocation};
use crate::select::{MoveMode, move_params};
use crate::tool::Context;

/// One key binding, for `tool.list`, the keyboard-shortcut docs and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct KeyBinding {
    pub keys: &'static str,
    pub when: &'static str,
    /// The command the key runs; empty for keys that only switch to a temporary tool.
    pub command: &'static str,
    pub note: &'static str,
}

/// The key map of the pointer tools.
pub const KEYMAP: &[KeyBinding] = &[
    KeyBinding { keys: "V", when: "editView", command: "tool.select", note: "the Select group's tool last used (Select or Lasso)" },
    KeyBinding { keys: "Shift+V", when: "editView", command: "tool.select", note: "the other tool of the Select group" },
    KeyBinding { keys: "H", when: "editView", command: "tool.select", note: "Hand" },
    KeyBinding { keys: "Z", when: "editView", command: "tool.select", note: "Zoom" },
    KeyBinding { keys: "L", when: "editView", command: "tool.select", note: "Measurement" },
    KeyBinding { keys: "T", when: "editView", command: "tool.select", note: "Text: text mode" },
    KeyBinding { keys: "A", when: "editView", command: "tool.select", note: "the annotation sub-tool in use, else Annotation Selection" },
    KeyBinding { keys: "Space (held)", when: "editView", command: "", note: "the Hand tool while held" },
    KeyBinding { keys: "Cmd+Space (held)", when: "editView", command: "", note: "the Zoom tool while held; Alt+click zooms out" },
    KeyBinding { keys: "Escape", when: "tool:*", command: "", note: "cancel the gesture in progress" },
    KeyBinding {
        keys: "Left, Right, Up, Down",
        when: "editView",
        command: "selection.move",
        note: "the font's increments: plain, Shift, Cmd; Alt moves only the selected points, Ctrl+Alt nudges",
    },
    KeyBinding { keys: "Return", when: "editView", command: "path.toggleNodeType", note: "the selected on-curve nodes" },
    KeyBinding { keys: "Return", when: "editView", command: "anchor.editName", note: "with one anchor selected alone" },
    KeyBinding { keys: "Alt+Return", when: "editView", command: "path.toggleNodeTypeKeepingOutline", note: "the selected on-curve nodes" },
    KeyBinding { keys: "Delete", when: "editView", command: "edit.delete", note: "the selection" },
    KeyBinding { keys: "Shift+Delete", when: "editView", command: "path.deleteAndBreak", note: "the selected points" },
    KeyBinding { keys: "Alt+Delete", when: "editView", command: "edit.deleteInAllMasters", note: "the selected points and shapes" },
    KeyBinding { keys: "Tab", when: "editView", command: "selection.selectNext", note: "" },
    KeyBinding { keys: "Shift+Tab", when: "editView", command: "selection.selectPrevious", note: "" },
    KeyBinding { keys: "Tab", when: "tool:select", command: "hint.ps.selectNextHint", note: "with PostScript hints selected" },
    KeyBinding { keys: "Shift+Tab", when: "tool:select", command: "hint.ps.selectPreviousHint", note: "with PostScript hints selected" },
    KeyBinding { keys: "Delete", when: "tool:select", command: "hint.ps.deleteHints", note: "with PostScript hints selected" },
    KeyBinding {
        keys: "Left, Right, Up, Down",
        when: "tool:select",
        command: "hint.ps.moveHints",
        note: "with PostScript hints selected: moves them by the font's increments (plain, Shift, Cmd), as one undo step",
    },
    KeyBinding {
        keys: "Delete",
        when: "tool:trueTypeInstructor",
        command: "hint.tt.deleteInstructions",
        note: "with TrueType instructions selected",
    },
    KeyBinding { keys: "Cmd+A", when: "tool:annotation", command: "annotation.selectAll", note: "every annotation of the edited layer" },
    KeyBinding {
        keys: "Left, Right, Up, Down",
        when: "tool:annotation",
        command: "annotation.move",
        note: "the selected annotations; the font's increments",
    },
    KeyBinding { keys: "Delete", when: "tool:annotation", command: "annotation.delete", note: "the selected annotations" },
    KeyBinding { keys: "G", when: "tool:measurement", command: "command.batch", note: "while dragging the ruler: a measurement guide along it" },
    KeyBinding { keys: "Escape", when: "editView", command: "editView.toggleTextMode", note: "with no gesture in progress: text mode" },
    KeyBinding { keys: "Escape", when: "textMode", command: "editView.toggleTextMode", note: "edit the current glyph with the Select tool" },
    KeyBinding {
        keys: "characters, Space, Return",
        when: "textMode",
        command: "editView.insertText",
        note: "typed at the cursor, in place of the selection",
    },
    KeyBinding {
        keys: "Backspace, Delete",
        when: "textMode",
        command: "editView.insertText",
        note: "the selection, else the item before / after the cursor",
    },
    KeyBinding {
        keys: "Left, Right, Up, Down",
        when: "textMode",
        command: "editView.setCursor",
        note: "move the cursor in the writing direction and between lines; Shift extends the selection",
    },
    KeyBinding { keys: "Cmd+A", when: "textMode", command: "editView.setCursor", note: "select the whole text" },
    KeyBinding {
        keys: "Home, End",
        when: "textMode",
        command: "editView.previousGlyph",
        note: "the previous / next glyph of the font in place of the current one",
    },
    KeyBinding {
        keys: "Shift+Home, Shift+End",
        when: "textMode",
        command: "editView.previousVisibleGlyph",
        note: "the previous / next glyph Font View lists",
    },
    KeyBinding { keys: "Cmd+Space (held)", when: "textMode", command: "", note: "the Hand tool while Space is held; Cmd may be let go" },
];

fn run(id: &str, params: Map<String, Value>) -> Action {
    Action::Run { invocation: Invocation::new(id, Value::Object(params)) }
}

/// The Edit View key `ev` as a command to run, or `None` when the key has no binding here or
/// the selection gives it nothing to act on. Tool letters and `Space` are the toolbox's.
pub fn edit_view_key(cx: &Context<'_>, ev: &KeyEvent) -> Option<Action> {
    let arrow = matches!(ev.key, Key::Left | Key::Right | Key::Up | Key::Down);
    if !ev.pressed || (ev.mods.cmd && !arrow) {
        return None;
    }
    let scene = cx.scene;
    let sel = cx.selection;
    let m = ev.mods;
    match ev.key {
        Key::Left | Key::Right | Key::Up | Key::Down => {
            if sel.is_empty() {
                return None;
            }
            let [plain, big, huge] = cx.settings.increments;
            let step = if m.cmd {
                huge
            } else if m.shift {
                big
            } else {
                plain
            };
            let (dx, dy) = match ev.key {
                Key::Left => (-step, 0.0),
                Key::Right => (step, 0.0),
                Key::Up => (0.0, step),
                _ => (0.0, -step),
            };
            if !step.is_finite() {
                return None;
            }
            let mode = if m.alt && m.ctrl {
                MoveMode::Nudge
            } else if m.alt {
                MoveMode::PointsOnly
            } else {
                MoveMode::Normal
            };
            Some(Action::Run { invocation: Invocation::new("selection.move", move_params(scene, sel, dx, dy, mode, false)) })
        }
        Key::Return if !m.ctrl => {
            let points = sel.on_curve_points(scene);
            if points.is_empty() {
                // One anchor selected alone is renamed.
                return match sel.anchors.first() {
                    Some(name) if sel.len() == 1 && !m.alt && !m.shift && !scene.locked => {
                        let mut params = Map::new();
                        params.insert("anchor".into(), json!(name));
                        Some(run("anchor.editName", params))
                    }
                    _ => None,
                };
            }
            let mut params = crate::Selection::new().targets(scene);
            params.insert("points".into(), json!(points.iter().map(|&(s, n)| [s, n]).collect::<Vec<_>>()));
            let id = if m.alt { "path.toggleNodeTypeKeepingOutline" } else { "path.toggleNodeType" };
            Some(run(id, params))
        }
        Key::Delete | Key::Backspace if !m.ctrl => {
            if sel.is_empty() {
                return None;
            }
            if m.shift {
                if sel.points.is_empty() {
                    return None;
                }
                let mut params = crate::Selection::new().targets(scene);
                params.insert("points".into(), json!(sel.points.iter().map(|&(s, n)| [s, n]).collect::<Vec<_>>()));
                Some(run("path.deleteAndBreak", params))
            } else if m.alt {
                let all = sel.targets(scene);
                let mut params = Map::new();
                for key in ["glyph", "layer", "side", "points", "shapes"] {
                    if let Some(v) = all.get(key) {
                        params.insert(key.into(), v.clone());
                    }
                }
                // Without a target list the command would act on the whole layer.
                if sel.points.is_empty() && sel.shapes.is_empty() {
                    return None;
                }
                Some(run("edit.deleteInAllMasters", params))
            } else {
                Some(run("edit.delete", sel.targets(scene)))
            }
        }
        Key::Tab if !m.ctrl && !m.alt => {
            let id = if m.shift { "selection.selectPrevious" } else { "selection.selectNext" };
            Some(run(id, Map::new()))
        }
        _ => None,
    }
}
