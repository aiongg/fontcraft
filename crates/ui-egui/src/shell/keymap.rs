//! Menu shortcuts: the default shortcut of every engine and shell command that has one with a
//! modifier. Keys without a modifier (tool letters, arrows, Return, Delete, Tab) belong to the
//! tools' keymap, which knows the focus (decision 0004, D1). The `textMode` shortcuts (the
//! spacing and kerning keys) are left out here: in text mode `shell::text::key` runs them with
//! the targets the cursor gives, and elsewhere they do nothing.
//!
//! Shortcuts are written in Mac notation; `Cmd` is the platform's command key (Ctrl off the Mac).

use std::sync::LazyLock;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use super::UI_COMMANDS;

/// The `when` of the shortcuts that work only while the Text tool is active.
pub const TEXT_MODE: &str = "textMode";

/// Every shortcut, the most specific first: egui matches a shortcut while extra Shift or Alt are
/// held, so `Cmd+Shift+W` must be tried before a `Cmd+W` would be.
static KEYMAP: LazyLock<Vec<(&'static str, KeyboardShortcut)>> = LazyLock::new(|| {
    // The `textMode` shortcuts work only in text mode, where [`text_mode_command`] finds them.
    let engine = fontcraft_engine::command_specs().iter().filter(|c| c.when != Some(TEXT_MODE)).filter_map(|c| Some((c.id, c.shortcut?)));
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
/// the keyboard, or had it when the last frame ended (`text_focus`).
pub fn pressed(ctx: &egui::Context, text_focus: bool) -> Vec<(&'static str, Value)> {
    if text_focus || ctx.egui_wants_keyboard_input() {
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
            if !NUMBERED.contains(id) && input.consume_shortcut(shortcut) {
                out.push((*id, json!({})));
            }
        }
        // `Cmd+1` … `Cmd+9` select the first nine masters; after the keymap, so that a shortcut
        // with more modifiers on the same digit wins.
        for (n, key) in (0u64..).zip(MASTER_DIGITS) {
            if input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key)) {
                out.push(("view.selectMaster", json!({"master": n})));
            }
        }
    });
    out
}

/// Commands whose shortcut stands for a numbered series and carries the number as a parameter.
const NUMBERED: &[&str] = &["view.showTab", "view.selectMaster"];

/// The digit keys of `Cmd+1` … `Cmd+9`.
const MASTER_DIGITS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];

/// The command a key with these modifiers runs as a menu shortcut, matched as [`pressed`] matches
/// it (`ui.key` on the control channel).
pub fn command_for(key: Key, modifiers: Modifiers) -> Option<(&'static str, Value)> {
    let digits = [Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    if modifiers.matches_logically(Modifiers::COMMAND | Modifiers::ALT)
        && let Some(n) = (2u64..).zip(digits).find(|(_, k)| *k == key).map(|(n, _)| n)
    {
        return Some(("view.showTab", json!({"index": n})));
    }
    let found = KEYMAP
        .iter()
        .find(|(id, s)| !NUMBERED.contains(id) && s.logical_key == key && modifiers.matches_logically(s.modifiers))
        .map(|(id, _)| (*id, json!({})));
    if found.is_some() {
        return found;
    }
    if modifiers.matches_logically(Modifiers::COMMAND)
        && let Some(n) = (0u64..).zip(MASTER_DIGITS).find(|(_, k)| *k == key).map(|(n, _)| n)
    {
        return Some(("view.selectMaster", json!({"master": n})));
    }
    None
}

/// The `textMode` shortcuts (the spacing and kerning keys of the Text tool): the key, the exact
/// modifiers and the command.
static TEXT_MODE_KEYS: LazyLock<Vec<(fontcraft_tools::Key, fontcraft_tools::Modifiers, &'static str)>> = LazyLock::new(|| {
    fontcraft_engine::command_specs()
        .iter()
        .filter(|c| c.when == Some(TEXT_MODE))
        .filter_map(|c| {
            let (key, mods) = parse_tool_key(c.shortcut?)?;
            Some((key, mods, c.id))
        })
        .collect()
});

/// A shortcut such as `Cmd+Ctrl+Shift+Left` as a key of the tools and its modifiers.
fn parse_tool_key(text: &str) -> Option<(fontcraft_tools::Key, fontcraft_tools::Modifiers)> {
    let mut mods = fontcraft_tools::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part {
            "Cmd" => mods.cmd = true,
            "Ctrl" => mods.ctrl = true,
            "Alt" | "Option" => mods.alt = true,
            "Shift" => mods.shift = true,
            other => key = Some(fontcraft_tools::Key::parse(other)?),
        }
    }
    Some((key?, mods))
}

/// The `textMode` command whose shortcut is exactly `key` with `mods`.
pub fn text_mode_command(key: fontcraft_tools::Key, mods: fontcraft_tools::Modifiers) -> Option<&'static str> {
    TEXT_MODE_KEYS.iter().find(|(k, m, _)| *k == key && *m == mods).map(|(_, _, id)| *id)
}
