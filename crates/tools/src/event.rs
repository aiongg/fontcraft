//! Pointer and key events, as the shell and the control channel (`ui.pointer`, `ui.key`) deliver
//! them.

use fontcraft_geom::Point;
use serde::{Deserialize, Serialize};

/// The modifier keys held during an event. `cmd` is Command on a Mac and Control elsewhere;
/// `ctrl` is the Control key on a Mac (the one that suspends snapping).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
    pub ctrl: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers { shift: false, alt: false, cmd: false, ctrl: false };
    pub const SHIFT: Modifiers = Modifiers { shift: true, alt: false, cmd: false, ctrl: false };
    pub const ALT: Modifiers = Modifiers { shift: false, alt: true, cmd: false, ctrl: false };
    pub const CMD: Modifiers = Modifiers { shift: false, alt: false, cmd: true, ctrl: false };
    pub const CTRL: Modifiers = Modifiers { shift: false, alt: false, cmd: false, ctrl: true };

    /// Modifiers from names such as `["shift", "alt"]`. Unknown names are an error.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let mut m = Modifiers::NONE;
        for name in names {
            match name.to_ascii_lowercase().as_str() {
                "shift" => m.shift = true,
                "alt" | "option" | "opt" => m.alt = true,
                "cmd" | "command" | "meta" | "super" => m.cmd = true,
                "ctrl" | "control" => m.ctrl = true,
                other => return Err(format!("unknown modifier `{other}`")),
            }
        }
        Ok(m)
    }

    /// No modifier at all.
    pub fn is_empty(&self) -> bool {
        *self == Modifiers::NONE
    }

    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn with_cmd(mut self) -> Self {
        self.cmd = true;
        self
    }

    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
}

/// What the pointer did. A double click arrives as its own event after the clicks that make it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerKind {
    Down,
    Drag,
    Up,
    Move,
    DoubleClick,
}

/// A pointer event at `pos`, in font units of the layer being edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub pos: Point,
    pub mods: Modifiers,
}

impl PointerEvent {
    pub fn new(kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Self {
        Self { kind, pos: Point::new(x, y), mods }
    }
}

/// A key the tools know. Letters are kept lower case; `Shift` is in the modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Return,
    Delete,
    Backspace,
    Tab,
    Escape,
    Space,
    Home,
    End,
}

impl Key {
    /// A key from its name as `ui.key` spells it: `ArrowLeft` or `Left`, `Return` or `Enter`,
    /// `Delete`, `Backspace`, `Tab`, `Escape` or `Esc`, `Space`, `Home`, `End`, or a single character.
    pub fn parse(name: &str) -> Option<Key> {
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Some(if c == ' ' { Key::Space } else { Key::Char(c.to_ascii_lowercase()) });
        }
        let key = match name.to_ascii_lowercase().as_str() {
            "arrowleft" | "left" => Key::Left,
            "arrowright" | "right" => Key::Right,
            "arrowup" | "up" => Key::Up,
            "arrowdown" | "down" => Key::Down,
            "return" | "enter" => Key::Return,
            "delete" | "del" => Key::Delete,
            "backspace" => Key::Backspace,
            "tab" => Key::Tab,
            "escape" | "esc" => Key::Escape,
            "space" => Key::Space,
            "home" => Key::Home,
            "end" => Key::End,
            _ => return None,
        };
        Some(key)
    }
}

/// A key going down (`pressed`) or coming up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Modifiers,
    pub pressed: bool,
}

impl KeyEvent {
    /// A key press.
    pub fn press(key: Key, mods: Modifiers) -> Self {
        Self { key, mods, pressed: true }
    }

    /// A key release.
    pub fn release(key: Key, mods: Modifiers) -> Self {
        Self { key, mods, pressed: false }
    }
}
