//! `ToolBridge`: the canvas's active tool. It turns what craft's `ToolHost` hands it into `fontcraft_tools` events in
//! the edited glyph's font units and posts them as `Msg::Tool`; the toolbox runs in `drain` (`super::apply`).
//!
//! The press, drags and release of the primary button are the toolbox's `down`, `drag` and `up`; a second press in a
//! click run adds a `doubleclick` after its `up`. Hover is `move`. Modifier changes while the pointer stands still,
//! arrows, Enter, Escape, the deletes, the tool letters and Space reach the toolbox as its keys.

use craft::behavior::{CancelReason, PointerButton};
use craft::keymap::{Chord, Key as CKey, Mods};
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};

use super::ToolInput;
use crate::app::Msg;
use crate::canvas::ActiveTool;
use crate::canvas::camera::Camera;

/// The chords the tools take from the app's shortcuts beside the plain deletes: the tool letters (Shift steps through
/// a shared letter), `G` (a guide along the measurement ruler), Space and Cmd+Space (the Hand and the Zoom tool while
/// held), Tab and Shift+Tab (select the next or previous object), Shift+Delete and Alt+Delete.
const KEYS: &[Chord] = &[
    Chord::new(Mods::NONE, CKey::V),
    Chord::new(Mods::SHIFT, CKey::V),
    Chord::new(Mods::NONE, CKey::P),
    Chord::new(Mods::NONE, CKey::B),
    Chord::new(Mods::SHIFT, CKey::B),
    Chord::new(Mods::NONE, CKey::X),
    Chord::new(Mods::SHIFT, CKey::X),
    Chord::new(Mods::NONE, CKey::F),
    Chord::new(Mods::SHIFT, CKey::F),
    Chord::new(Mods::NONE, CKey::T),
    Chord::new(Mods::NONE, CKey::H),
    Chord::new(Mods::NONE, CKey::Z),
    Chord::new(Mods::NONE, CKey::E),
    Chord::new(Mods::NONE, CKey::R),
    Chord::new(Mods::NONE, CKey::S),
    Chord::new(Mods::NONE, CKey::L),
    Chord::new(Mods::NONE, CKey::I),
    Chord::new(Mods::NONE, CKey::A),
    Chord::new(Mods::SHIFT, CKey::A),
    Chord::new(Mods::NONE, CKey::G),
    Chord::new(Mods::NONE, CKey::Space),
    Chord::new(Mods::CMD, CKey::Space),
    Chord::new(Mods::NONE, CKey::Tab),
    Chord::new(Mods::SHIFT, CKey::Tab),
    Chord::new(Mods::SHIFT, CKey::Delete),
    Chord::new(Mods::SHIFT, CKey::Backspace),
    Chord::new(Mods::ALT, CKey::Delete),
    Chord::new(Mods::ALT, CKey::Backspace),
];

/// The canvas's active tool over the toolbox.
#[derive(Debug, Default)]
pub struct ToolBridge {
    /// The camera of this pass.
    camera: Option<Camera>,
    /// What the tool posted since the last `take_sent`.
    sent: Vec<Msg>,
    /// The modifiers last posted.
    mods: Option<Modifiers>,
    /// Space went down and its release has not been posted.
    space: bool,
}

/// Modifiers of a pointer event. Off the Mac, Control is the command key and also counts as `ctrl`, which suspends
/// snapping; the toolbox's keyboard knows which keys hold a tool.
pub fn pointer_mods(m: Mods) -> Modifiers {
    let ctrl = if cfg!(target_os = "macos") { m.ctrl } else { m.ctrl || m.cmd };
    Modifiers { shift: m.shift, alt: m.alt, cmd: m.cmd, ctrl }
}

/// Modifiers of a key. Off the Mac, Control is the command key only: `Ctrl+Right` steps by the largest increment.
pub fn key_mods(m: Mods) -> Modifiers {
    Modifiers { shift: m.shift, alt: m.alt, cmd: m.cmd, ctrl: m.ctrl }
}

impl ToolBridge {
    fn post(&mut self, input: ToolInput) {
        if self.sent.len() < super::apply::MAX_POINTER_EVENTS {
            self.sent.push(Msg::Tool(input));
        }
    }

    fn pointer(&mut self, kind: PointerKind, pointer: &ToolPointer) {
        let Some(camera) = self.camera else { return };
        let pos = camera.to_font(pointer.pos);
        if !(pos.x.is_finite() && pos.y.is_finite()) {
            return;
        }
        let mods = pointer_mods(pointer.mods);
        self.mods = Some(mods);
        self.post(ToolInput::Pointer(PointerEvent { kind, pos, mods }));
    }

    fn key(&mut self, key: Key, mods: Modifiers, pressed: bool) {
        self.post(ToolInput::Key(KeyEvent { key, mods, pressed }));
    }
}

impl CanvasTool for ToolBridge {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        if self.space && !cx.held(CKey::Space) {
            self.space = false;
            self.key(Key::Space, key_mods(cx.mods), false);
        }
        let mods = pointer_mods(cx.mods);
        if self.mods != Some(mods) {
            self.mods = Some(mods);
            self.post(ToolInput::Modifiers(mods));
        }
    }

    fn press(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) -> ToolAction {
        self.pointer(PointerKind::Down, pointer);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        self.pointer(PointerKind::Drag, pointer);
    }

    fn release(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        self.pointer(PointerKind::Up, pointer);
        if pointer.clicks == 2 {
            self.pointer(PointerKind::DoubleClick, pointer);
        }
    }

    fn hover(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        self.pointer(PointerKind::Move, pointer);
    }

    fn cancel(&mut self, _reason: CancelReason, cx: &mut ToolContext<'_>) {
        // The toolbox ends a gesture on Escape.
        self.key(Key::Escape, key_mods(cx.mods), true);
    }

    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE.deletes().keys(KEYS)
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let mods = key_mods(cx.mods);
        match key {
            ToolKey::Nudge { direction, mods, .. } => {
                let arrow = if direction.x < 0.0 {
                    Key::Left
                } else if direction.x > 0.0 {
                    Key::Right
                } else if direction.y < 0.0 {
                    Key::Up
                } else {
                    Key::Down
                };
                self.key(arrow, key_mods(*mods), true);
            }
            ToolKey::Delete { forward, .. } => self.key(if *forward { Key::Delete } else { Key::Backspace }, mods, true),
            ToolKey::Submit => self.key(Key::Return, mods, true),
            ToolKey::Cancel => self.key(Key::Escape, mods, true),
            ToolKey::Kept(chord) => {
                let Some(k) = Key::parse(chord.key.name()) else { return false };
                if k == Key::Space {
                    self.space = true;
                }
                self.key(k, key_mods(chord.mods), true);
            }
            _ => return false,
        }
        true
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {
        // The selection and the toolbox's overlays are canvas layers (`super::overlays`).
    }

    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    fn takes_every_move(&self) -> bool {
        // The toolbox decides its own clicks with its drag threshold.
        true
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Edit menu's selection commands and the Path menu")
    }
}

impl ActiveTool for ToolBridge {
    fn take_sent(&mut self) -> Vec<Msg> {
        std::mem::take(&mut self.sent)
    }

    fn set_camera(&mut self, camera: Camera) {
        self.camera = Some(camera);
    }
}
