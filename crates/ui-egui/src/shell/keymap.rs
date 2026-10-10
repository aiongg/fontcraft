//! Menu shortcuts: the default shortcut of every engine and shell command that has one with a
//! modifier. Keys without a modifier (tool letters, arrows, Return, Delete, Tab) belong to the
//! tools' keymap, which knows the focus (decision 0004, D1).
//!
//! Shortcuts are written in Mac notation; `Cmd` is the platform's command key (Ctrl off the Mac).

use std::sync::LazyLock;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use super::UI_COMMANDS;

/// Every shortcut, the most specific first: egui matches a shortcut while extra Shift or Alt are
/// held, so `Cmd+Shift+W` must be tried before a `Cmd+W` would be.
static KEYMAP: LazyLock<Vec<(&'static str, KeyboardShortcut)>> = LazyLock::new(|| {
    let engine = fontcraft_engine::command_specs().iter().filter_map(|c| Some((c.id, c.shortcut?)));
    let shell = UI_COMMANDS.iter().map(|c| (c.0, c.3));
    let mut map: Vec<(&'static str, KeyboardShortcut)> = Vec::new();
    for (id, text) in engine.chain(shell) {
        let Some(shortcut) = parse(text).filter(|s| has_modifier(s.modifiers)) else { continue };
        map.push((id, shortcut));
        // On most layouts Plus is Shift and Equals: take the unshifted key too.
        if shortcut.logical_key == Key::Plus {
            map.push((id, KeyboardShortcut::new(shortcut.modifiers, Key::Equals)));
        }
    }
    map.sort_by_key(|(_, s)| std::cmp::Reverse(modifier_count(s.modifiers)));
    map
});

fn has_modifier(m: Modifiers) -> bool {
    m.command || m.ctrl || m.alt || m.mac_cmd
}

fn modifier_count(m: Modifiers) -> u8 {
    u8::from(m.command || m.mac_cmd) + u8::from(m.ctrl) + u8::from(m.alt) + u8::from(m.shift)
}

/// Parses a shortcut such as `Cmd+Shift+M` or `Cmd+Plus`. `None` for an empty or unknown one.
pub fn parse(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').filter(|p| !p.is_empty()) {
        match part {
            "Cmd" => modifiers.command = true,
            "Ctrl" => modifiers.ctrl = true,
            "Alt" | "Option" => modifiers.alt = true,
            "Shift" => modifiers.shift = true,
            other => key = Some(Key::from_name(other)?),
        }
    }
    // A trailing `+` is the plus key (`Cmd++`).
    if text.ends_with("++") {
        key = Some(Key::Plus);
    }
    Some(KeyboardShortcut::new(modifiers, key?))
}

/// The commands whose shortcuts were pressed this frame. Nothing fires while a text field has
/// the keyboard.
pub fn pressed(ctx: &egui::Context) -> Vec<(&'static str, Value)> {
    if ctx.egui_wants_keyboard_input() {
        return Vec::new();
    }
    let mut out = Vec::new();
    ctx.input_mut(|input| {
        // `Cmd+Alt+2` … `Cmd+Alt+9` show tabs 2 to 9; `Cmd+Alt+1` is Font View's own shortcut.
        let digits = [Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        for (n, key) in (2u64..).zip(digits) {
            if input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::ALT, key)) {
                out.push(("view.showTab", json!({"index": n})));
            }
        }
        for (id, shortcut) in KEYMAP.iter() {
            if *id != "view.showTab" && input.consume_shortcut(shortcut) {
                out.push((*id, json!({})));
            }
        }
    });
    out
}
