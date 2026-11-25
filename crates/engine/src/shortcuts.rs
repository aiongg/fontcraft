//! The user's keyboard shortcuts: which key combination runs which command in place of its built-in one.
//!
//! The overrides are kept in the settings under `shortcuts.custom` ([`Reserved::Shortcuts`]), a list of
//! `{command, shortcut}` with an empty shortcut for a command that has none any more. A command without an entry keeps
//! its built-in shortcut. [`Custom::of`] reads the list; an entry that cannot be read is left out and reported, and one
//! for a command this build does not have is kept. The commands that change the list are in `cmd/shortcuts.rs`.
//!
//! A custom shortcut is one key with at least one modifier, written in Mac notation (`Cmd+Alt+K`). Off macOS Cmd is
//! Ctrl, so `Cmd+K` and `Ctrl+K` are one combination there ([`Chord::pressed`]).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::settings::{Reserved, UserSettings};

/// Most commands with a shortcut of the user's own.
pub const MAX_CUSTOM: usize = 2048;
/// Longest text read as a shortcut, in bytes.
pub const MAX_TEXT_BYTES: usize = 64;
/// Longest command id read, in bytes.
pub const MAX_COMMAND_BYTES: usize = 128;
/// Most unreadable entries reported.
const MAX_PROBLEMS: usize = 16;

/// The keys a shortcut may end in, by the name it is stored under.
const KEYS: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "Escape",
    "Tab",
    "Backspace",
    "Enter",
    "Space",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Up",
    "Down",
    "Left",
    "Right",
    "-",
    "=",
    "+",
    "[",
    "]",
    "\\",
    ";",
    "'",
    "`",
    ",",
    ".",
    "/",
];

/// Other names a key is written with.
const ALIASES: &[(&str, &str)] = &[
    ("Esc", "Escape"),
    ("Return", "Enter"),
    ("Del", "Delete"),
    ("ArrowUp", "Up"),
    ("ArrowDown", "Down"),
    ("ArrowLeft", "Left"),
    ("ArrowRight", "Right"),
    ("Minus", "-"),
    ("Equals", "="),
    ("Equal", "="),
    ("Plus", "+"),
    ("OpenBracket", "["),
    ("CloseBracket", "]"),
    ("Backslash", "\\"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("Backtick", "`"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
];

/// A key combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key, by its name in [`KEYS`].
    pub key: &'static str,
}

impl Chord {
    /// Reads `Cmd+Ctrl+Alt+Shift+Key`, the modifiers in any order; case does not matter.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(format!("a shortcut is from 1 to {MAX_TEXT_BYTES} characters, such as Cmd+Alt+K"));
        }
        // The plus key is the last character after the modifiers' own plus signs.
        let (mods, key) = match text.strip_suffix('+') {
            Some("") => ("", "+"),
            Some(rest) => (rest.strip_suffix('+').ok_or_else(|| format!("`{text}` names no key"))?, "+"),
            None => text.rsplit_once('+').unwrap_or(("", text)),
        };
        let mut chord =
            Chord { cmd: false, ctrl: false, alt: false, shift: false, key: key_named(key.trim()).ok_or_else(|| format!("`{text}` names no key"))? };
        for name in mods.split('+').map(str::trim).filter(|m| !m.is_empty()) {
            let held = match name.to_ascii_lowercase().as_str() {
                "cmd" | "command" => &mut chord.cmd,
                "ctrl" | "control" => &mut chord.ctrl,
                "alt" | "option" => &mut chord.alt,
                "shift" => &mut chord.shift,
                _ => return Err(format!("`{name}` is no modifier: they are Cmd, Ctrl, Alt and Shift")),
            };
            if std::mem::replace(held, true) {
                return Err(format!("`{text}` names {name} twice"));
            }
        }
        Ok(chord)
    }

    /// Reads a shortcut the user may give a command: a key with at least one modifier.
    pub fn custom(text: &str) -> Result<Chord, String> {
        let chord = Chord::parse(text)?;
        if chord.cmd || chord.ctrl || chord.alt || chord.shift {
            Ok(chord)
        } else {
            Err(format!("`{}` needs Cmd, Ctrl, Alt or Shift with it", chord.key))
        }
    }

    /// The combination in Mac notation, as it is stored.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (held, name) in [(self.cmd, "Cmd+"), (self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if held {
                out.push_str(name);
            }
        }
        out.push_str(self.key);
        out
    }

    /// The keys pressed for it on this platform: off macOS Cmd is Ctrl.
    pub fn pressed(&self) -> Chord {
        if cfg!(target_os = "macos") { *self } else { Chord { cmd: self.cmd || self.ctrl, ctrl: false, ..*self } }
    }
}

fn key_named(name: &str) -> Option<&'static str> {
    let named = |name: &str| KEYS.iter().copied().find(|k| k.eq_ignore_ascii_case(name));
    named(name).or_else(|| ALIASES.iter().find(|(alias, _)| alias.eq_ignore_ascii_case(name)).and_then(|(_, key)| named(key)))
}

/// What the operating system does with a combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct System {
    pub reason: &'static str,
    /// No application receives it; otherwise it depends on the system's settings.
    pub blocked: bool,
}

/// What the operating system of this build does with `chord`, when it uses it.
pub fn system(chord: &Chord) -> Option<System> {
    let text = chord.pressed().text();
    if cfg!(target_os = "macos") {
        let (reason, blocked) = match text.as_str() {
            "Cmd+Tab" | "Cmd+Shift+Tab" => ("macOS uses it to switch applications.", true),
            "Cmd+Alt+Escape" => ("macOS uses it to force applications to quit.", true),
            "Cmd+`" | "Cmd+Shift+`" => ("macOS may use it to move between an application's windows.", false),
            "Cmd+Space" | "Cmd+Alt+Space" => ("macOS uses it for Spotlight unless the system's keyboard settings say otherwise.", false),
            "Ctrl+Left" | "Ctrl+Right" | "Ctrl+Up" | "Ctrl+Down" => {
                ("macOS uses it for Mission Control unless the system's keyboard settings say otherwise.", false)
            }
            _ => return None,
        };
        return Some(System { reason, blocked });
    }
    let blocked = match text.as_str() {
        "Alt+Tab" | "Alt+Shift+Tab" | "Alt+F4" | "Cmd+Alt+Delete" => true,
        "Cmd+Escape" | "Cmd+Shift+Escape" => cfg!(target_os = "windows"),
        _ => !cfg!(target_os = "windows") && text.strip_prefix("Cmd+Alt+F").is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n))),
    };
    blocked.then_some(System { reason: "The system uses it.", blocked: true })
}

/// Whether two commands can both get a key press, by where each one's shortcut works (`CommandSpec::when`). One that
/// works in a panel or dialog of its own shares no press with one that works everywhere: the panel that has the
/// keyboard keeps its keys.
pub fn meet(a: Option<&str>, b: Option<&str>) -> bool {
    let place = |when: &str| match when {
        "fontView" => Some((false, None)),
        "editView" => Some((true, None)),
        "textMode" => Some((true, Some("textMode".to_string()))),
        other => other.starts_with("tool:").then(|| (true, Some(other.to_string()))),
    };
    match (a, b) {
        (None, None) => true,
        (Some(one), None) | (None, Some(one)) => place(one).is_some(),
        (Some(a), Some(b)) => match (place(a), place(b)) {
            (Some((edit_a, in_a)), Some((edit_b, in_b))) => edit_a == edit_b && (in_a.is_none() || in_b.is_none() || in_a == in_b),
            _ => a == b,
        },
    }
}

/// A command of the window, or a key the window keeps for itself, that the shortcut commands list with the engine's
/// own. The window hands them over when it starts (`Session::set_window_commands`); without a window there are none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowCommand {
    pub id: String,
    pub label: String,
    /// The group it is listed under.
    pub section: String,
    /// It has a menu row.
    pub menu: bool,
    /// Its built-in shortcut, in Mac notation.
    pub shortcut: Option<String>,
    /// Its key cannot be changed: a tool's letter, or a key an area takes.
    pub fixed: bool,
}

/// The user's own shortcuts: command id to a combination, or to none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Custom {
    map: BTreeMap<String, Option<Chord>>,
    problems: Vec<String>,
}

impl Custom {
    /// The stored overrides. Entries that cannot be read are left out and named in [`Self::problems`].
    pub fn of(settings: &UserSettings) -> Custom {
        match settings.reserved::<Value>(Reserved::Shortcuts) {
            Ok(None | Some(Value::Null)) => Custom::default(),
            Ok(Some(value)) => Custom::read(&value),
            Err(e) => Custom { map: BTreeMap::new(), problems: vec![e.to_string()] },
        }
    }

    /// Reads a list of `{command, shortcut}`, as it is stored.
    pub fn read(value: &Value) -> Custom {
        let mut out = Custom::default();
        let Some(list) = value.as_array() else {
            out.problem("the stored shortcuts are not a list".into());
            return out;
        };
        if list.len() > MAX_CUSTOM {
            out.problem(format!("only the first {MAX_CUSTOM} of {} stored shortcuts are used", list.len()));
        }
        for entry in list.iter().take(MAX_CUSTOM) {
            let text = |key: &str| entry.get(key).and_then(Value::as_str);
            let (Some(command), Some(shortcut)) = (text("command"), text("shortcut")) else {
                out.problem("an entry without `command` and `shortcut`".into());
                continue;
            };
            if command.is_empty() || command.len() > MAX_COMMAND_BYTES {
                out.problem(format!("a command id is from 1 to {MAX_COMMAND_BYTES} bytes"));
                continue;
            }
            let chord = match shortcut {
                "" => None,
                text => match Chord::custom(text) {
                    Ok(chord) => Some(chord),
                    Err(why) => {
                        out.problem(format!("{command}: {why}"));
                        continue;
                    }
                },
            };
            out.map.insert(command.to_string(), chord);
        }
        out
    }

    fn problem(&mut self, text: String) {
        if self.problems.len() < MAX_PROBLEMS {
            self.problems.push(text);
        }
    }

    /// What reading left out.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The override of `command`: `Some(None)` when its shortcut was removed, `None` without an override.
    pub fn get(&self, command: &str) -> Option<Option<Chord>> {
        self.map.get(command).copied()
    }

    /// The shortcut of `command`, whose built-in one is `default`.
    pub fn shortcut(&self, command: &str, default: Option<&str>) -> Option<String> {
        match self.get(command) {
            Some(chord) => chord.map(|c| c.text()),
            None => default.map(str::to_string),
        }
    }

    /// Every override, in the order of the command ids.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<Chord>)> {
        self.map.iter().map(|(id, chord)| (id.as_str(), *chord))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Sets an override; false when there are [`MAX_CUSTOM`] already.
    pub(crate) fn set(&mut self, command: &str, chord: Option<Chord>) -> bool {
        if command.len() > MAX_COMMAND_BYTES || (self.map.len() >= MAX_CUSTOM && !self.map.contains_key(command)) {
            return false;
        }
        self.map.insert(command.to_string(), chord);
        true
    }

    /// Removes an override; whether there was one.
    pub(crate) fn remove(&mut self, command: &str) -> bool {
        self.map.remove(command).is_some()
    }

    /// The list as it is stored.
    pub fn to_value(&self) -> Value {
        let entry = |(id, chord): (&str, Option<Chord>)| json!({"command": id, "shortcut": chord.map(|c| c.text()).unwrap_or_default()});
        Value::Array(self.iter().map(entry).collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn chords_read_and_write_mac_notation() {
        for (text, written) in
            [("shift+cmd+k", "Cmd+Shift+K"), ("Cmd+Comma", "Cmd+,"), ("Cmd++", "Cmd++"), ("Option+Return", "Alt+Enter"), ("Ctrl+Left", "Ctrl+Left")]
        {
            assert_eq!(Chord::custom(text).unwrap().text(), written);
            assert_eq!(Chord::custom(written).unwrap().text(), written);
        }
        for bad in ["", "K", "Cmd+", "Cmd+Cmd+K", "Hyper+K", "Cmd+Nope", "+"] {
            assert!(Chord::custom(bad).is_err(), "{bad}");
        }
        assert!(Chord::custom(&"Cmd+".repeat(100)).is_err());
        assert_eq!(Chord::parse("+").unwrap().key, "+");
    }

    #[test]
    fn unreadable_entries_are_left_out_and_named() {
        let custom = Custom::read(&json!([
            {"command": "edit.undo", "shortcut": "Cmd+Alt+U"},
            {"command": "edit.redo", "shortcut": "Hyper+Q"},
            {"command": "edit.cut", "shortcut": ""},
            {"shortcut": "Cmd+K"},
            7,
        ]));
        assert_eq!((custom.len(), custom.problems().len()), (2, 3), "{:?}", custom.problems());
        assert_eq!(custom.shortcut("edit.undo", Some("Cmd+Z")).as_deref(), Some("Cmd+Alt+U"));
        assert_eq!(custom.shortcut("edit.cut", Some("Cmd+X")), None);
        assert_eq!(custom.shortcut("edit.redo", Some("Cmd+Shift+Z")).as_deref(), Some("Cmd+Shift+Z"), "an unreadable one keeps the built-in");
        assert_eq!(Custom::read(&custom.to_value()).iter().count(), 2);
    }

    #[test]
    fn places_that_share_no_press_do_not_meet() {
        assert!(meet(None, None) && meet(None, Some("editView")) && meet(Some("editView"), Some("textMode")));
        assert!(!meet(Some("fontView"), Some("editView")) && !meet(Some("tool:a"), Some("tool:b")));
        assert!(!meet(None, Some("panel:fontInfo")) && meet(Some("panel:fontInfo"), Some("panel:fontInfo")));
    }
}
