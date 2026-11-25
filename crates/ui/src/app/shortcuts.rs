//! The user's keyboard shortcuts in the window. The overrides live in the settings
//! (`fontcraft_engine::shortcuts`); [`refresh`] hands them to the catalog when they change, so the keys, the menus, the
//! tooltips, the command palette and `ui.menu.list` follow at once. [`window_commands`] is what the engine's shortcut
//! commands list beside its own.
//!
//! Fixed keys are listed and reported as conflicts, and cannot be changed: the tools' letters, the keys an area takes
//! (`shell::commands::SCOPED_KEYS`) and the numbered keys. Rows: `shortcuts.custom`, `shortcuts.oneRegistry`.

use craft::keymap::keymap::Overrides;
use fontcraft_engine::shortcuts::{Custom, WindowCommand};

use super::FontCraftApp;
use super::catalog::{Kind, TOOL_PREFIX, kind};
use crate::shell::commands::{MODULES, SCOPED_KEYS, find};
use crate::shell::{getters, numbered_keys};

/// The section the tools' letters are listed under.
const TOOLS_SECTION: &str = "Tools";
/// The section the keys of one area are listed under.
const AREA_SECTION: &str = "Keys of one area";

/// The shell commands, the tools' letters, the areas' keys and the numbered keys.
pub fn window_commands() -> Vec<WindowCommand> {
    let shell = MODULES.iter().flat_map(|m| m.commands.iter()).filter(|c| !matches!(kind(c.id), Kind::Tool(_)) && getters::find(c.id).is_none());
    let mut out: Vec<WindowCommand> = shell
        .map(|c| WindowCommand {
            id: c.id.to_string(),
            label: c.label.to_string(),
            section: c.menu.join(" › "),
            menu: !c.menu.is_empty(),
            shortcut: c.shortcut.map(str::to_string),
            fixed: false,
        })
        .collect();
    let fixed = |id: String, label: &str, section: &str, shortcut: &str| WindowCommand {
        id,
        label: label.to_string(),
        section: section.to_string(),
        menu: false,
        shortcut: Some(shortcut.to_string()),
        fixed: true,
    };
    out.extend(fontcraft_tools::TOOLS.iter().map(|t| fixed(format!("{TOOL_PREFIX}{}", t.id), t.label, TOOLS_SECTION, t.shortcut)));
    let numbered = numbered_keys::all();
    // The first of a numbered series is listed with the series.
    let scoped = SCOPED_KEYS.iter().filter(|(id, _)| !numbered.iter().any(|n| n.command.id == *id));
    out.extend(scoped.map(|(id, key)| fixed((*id).to_string(), find(id).map_or(*id, |(_, c)| c.label), AREA_SECTION, *key)));
    out.extend(numbered.iter().map(|n| fixed(n.command.id.clone(), &n.label, AREA_SECTION, &n.shortcut)));
    out
}

/// The overrides as the keymap takes them: the command's key to its combinations, none for a removed one.
pub fn overrides(custom: &Custom) -> Overrides {
    let mut out = Overrides::new();
    for (id, chord) in custom.iter() {
        // Tools have fixed letters; no command writes an override of one.
        if matches!(kind(id), Kind::Tool(_)) {
            continue;
        }
        out.insert(id, chord.map(|c| vec![c.text()]).unwrap_or_default());
    }
    out
}

/// Hands the overrides to the catalog when the settings changed. Stored shortcuts that cannot be read are named in the
/// status line once.
pub fn refresh(app: &mut FontCraftApp) {
    let revision = app.session.settings().revision();
    if app.catalog.shortcuts_at() == Some(revision) {
        return;
    }
    let custom = Custom::of(app.session.settings());
    let notice = custom.problems().first().map(|first| {
        let more = custom.problems().len().saturating_sub(1);
        let rest = if more == 0 { String::new() } else { format!(" (and {more} more)") };
        format!("A stored keyboard shortcut could not be read and was left out: {first}{rest}")
    });
    let problems_changed = custom.problems() != app.catalog.custom().problems();
    app.catalog.set_custom(revision, custom);
    if problems_changed && let Some(notice) = notice {
        log::warn!("{notice}");
        app.status = notice;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use craft::ui::testing::input::key_press;
    use egui::{Key, Modifiers};
    use fontcraft_engine::settings::Reserved;
    use serde_json::{Value, json};

    use crate::testing::TestApp;

    fn press(t: &mut TestApp, key: Key, mods: Modifiers) {
        t.frames.hold(mods);
        t.frame_with(vec![key_press(key, mods)]);
        t.frames.hold(Modifiers::NONE);
        t.frame();
    }

    fn menu_shortcut(t: &mut TestApp, id: &str) -> Value {
        let menus = t.ok("ui.menu.list", json!({}));
        let items: Vec<Value> = menus.as_array().unwrap().iter().flat_map(|m| m["items"].as_array().unwrap().clone()).collect();
        items.iter().find(|i| i["command"] == id).expect("the row")["shortcut"].clone()
    }

    /// An override moves the key, the menu row and the control channel's keys at once, and resetting brings the built-in
    /// one back.
    #[test]
    fn an_override_reaches_the_keys_and_the_menu() {
        let mut t = TestApp::new();
        assert_eq!(menu_shortcut(&mut t, "view.newTab"), "Cmd+T");
        let tabs = t.app.view.tabs.tabs.len();
        let reply =
            t.ok("engine.execute", json!({"command": "settings.setShortcut", "params": {"command": "view.newTab", "shortcut": "Cmd+Alt+Shift+E"}}));
        assert_eq!(reply["set"], true, "{reply}");
        t.frame();
        assert_eq!(menu_shortcut(&mut t, "view.newTab"), "Cmd+Alt+Shift+E");
        press(&mut t, Key::T, Modifiers::COMMAND);
        assert_eq!(t.app.view.tabs.tabs.len(), tabs, "the built-in key no longer runs it");
        press(&mut t, Key::E, Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT);
        assert_eq!(t.app.view.tabs.tabs.len(), tabs + 1, "the new key does");
        let reply = t.ok("ui.key", json!({"key": "E", "cmd": true, "alt": true, "shift": true}));
        assert_eq!(reply["command"], "view.newTab");
        t.ok("engine.execute", json!({"command": "settings.resetShortcut", "params": {"command": "view.newTab"}}));
        t.frame();
        assert_eq!(menu_shortcut(&mut t, "view.newTab"), "Cmd+T");

        // The tools' letters and the numbered keys are listed, and stay where they are.
        let list = t.ok("engine.execute", json!({"command": "settings.listShortcuts", "params": {}}));
        let rows = list["shortcuts"].as_array().unwrap();
        let draw = rows.iter().find(|r| r["command"] == "tool.draw").expect("the Draw tool");
        assert_eq!((&draw["fixed"], &draw["shortcut"]), (&json!(true), &json!("P")));
        let reply = t.ok(
            "engine.execute",
            json!({"command": "settings.setShortcut", "params": {"command": "view.newTab", "shortcut": "Cmd+1", "replace": true}}),
        );
        assert!(reply["set"] == false && reply["conflicts"][0]["fixed"] == true, "{reply}");
    }

    #[test]
    fn a_corrupt_stored_shortcut_is_dropped_with_a_notice() {
        let mut t = TestApp::new();
        t.app
            .session
            .settings_mut()
            .set_reserved(Reserved::Shortcuts, &json!([{"command": "view.newTab", "shortcut": "Nope+9"}, {"command": "edit.undo", "shortcut": ""}]))
            .unwrap();
        t.frame();
        assert!(t.app.status.contains("view.newTab"), "{}", t.app.status);
        assert_eq!(menu_shortcut(&mut t, "view.newTab"), "Cmd+T");
        assert_eq!(menu_shortcut(&mut t, "edit.undo"), "");
    }
}
