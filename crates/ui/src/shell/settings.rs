//! The Settings window's commands and state (`app.settings`), and the sample-string commands of its Sample Strings
//! section (`settings.*SampleString*`). The window itself is `dialogs/settings.rs`.
//!
//! The seven sections come in the reference's order. A section that is not built yet is listed greyed with the
//! reason, and `app.settings` refuses to open it. Every control writes its setting with `settings.set` (or its own
//! command), so the sections do not depend on where the settings are stored.
//!
//! The sample strings live in the user settings as `sampleStrings.groups`, a list of `{name, strings}`; while nothing
//! is stored, the built-in groups stand in ([`default_groups`]). A string is written as the settings write it:
//! `/name ` names a glyph, `/Placeholder` is a placeholder, and the two characters `\n` are a line break.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::params;
use crate::app::FontCraftApp;

/// Most groups read from or written to the settings.
pub const MAX_GROUPS: usize = 256;
/// Most strings read or written, over all groups.
pub const MAX_STRINGS: usize = 10_000;
/// Longest sample string, in bytes; longer ones are left out on reading and refused on writing.
pub const MAX_STRING_BYTES: usize = 4096;
/// Longest group name, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// The name of a group added without one.
const NEW_GROUP: &str = "New Group";

pub const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "app.settings",
        label: "Settings…",
        menu: &["Edit"],
        shortcut: Some("Cmd+Comma"),
        params: "{section?: \"updates\" | \"appearance\" | \"userSettings\" | \"sampleStrings\" | \"sharing\" | \"addons\" | \"shortcuts\", open?: bool, group?: number} open the Settings window (open: false closes it), on a section or else on the section last shown; a section that is not available yet is refused. group marks a group of the Sample Strings section. ui.inspect reports the window as settings: {open, section, sections: [{id, label, available, reason?}]}",
    },
    UiCommand {
        id: "settings.addSampleStringGroup",
        label: "Add Group",
        menu: &[],
        shortcut: None,
        params: "{name?: string, strings?: [string]} append a sample-string group (the Sample Strings section's plus button); without a name it is called New Group",
    },
    UiCommand {
        id: "settings.removeSampleStringGroup",
        label: "Remove Group",
        menu: &[],
        shortcut: None,
        params: "{group: name | index} delete a sample-string group with all its strings (the minus button)",
    },
    UiCommand {
        id: "settings.setSampleStrings",
        label: "Set Sample Strings",
        menu: &[],
        shortcut: None,
        params: "{group: name | index, strings: [string]} replace the strings of one group (the section's text field, one string per line)",
    },
    UiCommand {
        id: "settings.resetSampleStrings",
        label: "Default",
        menu: &[],
        shortcut: None,
        params: "{} restore the built-in sample strings, removing the groups added",
    },
];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// One section of the Settings window.
#[derive(Clone, Copy, Debug)]
pub struct Section {
    /// What `app.settings {section}` takes.
    pub id: &'static str,
    pub label: &'static str,
    /// Whether its body is built.
    pub available: bool,
    /// Why the section is greyed while it is not available.
    pub unavailable: &'static str,
}

const NOT_YET: &str = "Not available yet.";
/// The section the window shows when none was chosen.
pub const USER_SETTINGS: &str = "userSettings";

/// The sections, in the reference's order.
pub const SECTIONS: &[Section] = &[
    Section { id: "updates", label: "Updates", available: false, unavailable: "FontCraft does not look for new versions by itself." },
    Section {
        id: "appearance",
        label: "Appearance",
        available: false,
        unavailable: "Not available yet. Its settings can be changed with settings.set in the Command Console.",
    },
    Section { id: USER_SETTINGS, label: "User Settings", available: true, unavailable: "" },
    Section { id: "sampleStrings", label: "Sample Strings", available: true, unavailable: "" },
    Section { id: "sharing", label: "Sharing", available: true, unavailable: "" },
    Section { id: "addons", label: "Extensions", available: true, unavailable: "" },
    Section { id: "shortcuts", label: "Shortcuts", available: false, unavailable: NOT_YET },
];

/// The window's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsState {
    pub open: bool,
    /// The section last shown; the window opens on it again.
    pub section: String,
    /// The group the Sample Strings section shows.
    pub group: usize,
    /// The External Preview devices already asked about with the notice.
    pub noticed: Vec<String>,
}

/// The section the window shows: the one last chosen while it is available, else User Settings.
pub fn current(st: &SettingsState) -> &'static str {
    SECTIONS.iter().find(|s| s.id == st.section && s.available).map_or(USER_SETTINGS, |s| s.id)
}

/// `ui.inspect`'s `settings`.
pub fn inspect(app: &FontCraftApp, out: &mut Map<String, Value>) {
    let st = &app.view.settings;
    let sections: Vec<Value> = SECTIONS
        .iter()
        .map(|s| {
            if s.available {
                json!({"id": s.id, "label": s.label, "available": true})
            } else {
                json!({"id": s.id, "label": s.label, "available": false, "reason": s.unavailable})
            }
        })
        .collect();
    out.insert("settings".into(), json!({"open": st.open, "section": current(st), "sections": sections, "group": st.group}));
}

/// A group of sample strings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub strings: Vec<String>,
}

/// The built-in groups, used while the settings hold none.
pub fn default_groups() -> Vec<Group> {
    let group = |name: &str, strings: &[&str]| Group { name: name.into(), strings: strings.iter().map(|s| (*s).to_string()).collect() };
    vec![
        group(
            "Spacing",
            &[
                "nnnnnnnn",
                "oooooooo",
                "nnonnoonno",
                "nn/Placeholder nn oo/Placeholder oo",
                "HHOHHOOHHO",
                "HH/Placeholder HH OO/Placeholder OO",
                "HnHoHnHo\\nnHnoHonHo",
            ],
        ),
        group("Figures", &["0123456789", "H0H1H2H3H4H5H6H7H8H9H", "n0n1n2n3n4n5n6n7n8n9n"]),
        group(
            "Latin",
            &[
                "Hamburgefonstiv\\nHAMBURGEFONSTIV",
                "Five quacking zephyrs jolt my wax bed.",
                "Jackdaws love my big sphinx of quartz.",
                "Pack my box with five dozen liquor jugs.",
            ],
        ),
        group("Greek", &["Ξεσκεπάζω την ψυχοφθόρα βδελυγμία.", "ΞΕΣΚΕΠΑΖΩ ΤΗΝ ΨΥΧΟΦΘΟΡΑ ΒΔΕΛΥΓΜΙΑ"]),
        group("Cyrillic", &["Съешь же ещё этих мягких французских булок, да выпей чаю.", "СЪЕШЬ ЖЕ ЕЩЁ ЭТИХ МЯГКИХ ФРАНЦУЗСКИХ БУЛОК"]),
    ]
}

/// The groups the settings hold, within the limits; the built-in ones while the settings hold none or something
/// unreadable.
pub fn groups(session: &Session) -> Vec<Group> {
    let Ok(Some(stored)) = session.settings().reserved::<Vec<Value>>(Reserved::SampleStrings) else { return default_groups() };
    let mut left = MAX_STRINGS;
    stored
        .iter()
        .take(MAX_GROUPS)
        .map(|g| {
            let strings: Vec<String> = g
                .get("strings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|s| s.len() <= MAX_STRING_BYTES)
                .take(left)
                .map(str::to_string)
                .collect();
            left = left.saturating_sub(strings.len());
            Group { name: g.get("name").and_then(Value::as_str).unwrap_or_default().chars().take(MAX_NAME_CHARS).collect(), strings }
        })
        .collect()
}

fn store(session: &mut Session, groups: &[Group]) -> Result<(), String> {
    let list: Vec<Value> = groups.iter().map(|g| json!({"name": g.name, "strings": g.strings})).collect();
    session.settings_mut().set_reserved(Reserved::SampleStrings, &list).map_err(|e| e.to_string())
}

/// The strings at `key`, checked against the limits; `left` is how many strings the other groups leave room for.
fn checked_strings(p: &Value, key: &str, left: usize) -> Result<Option<Vec<String>>, String> {
    let Some(strings) = params::strings(p, key, MAX_STRINGS)? else { return Ok(None) };
    if strings.len() > left {
        return Err(format!("the sample strings hold at most {MAX_STRINGS} strings; {left} more fit"));
    }
    if let Some(long) = strings.iter().find(|s| s.len() > MAX_STRING_BYTES) {
        return Err(format!("a sample string holds at most {MAX_STRING_BYTES} bytes: `{}`", params::quoted(long)));
    }
    Ok(Some(strings))
}

/// The group `group` names, by name or index.
fn group_index(groups: &[Group], p: &Value) -> Result<usize, String> {
    match p.get("group") {
        Some(Value::String(name)) => {
            groups.iter().position(|g| g.name == *name).ok_or_else(|| format!("there is no sample-string group `{}`", params::quoted(name)))
        }
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < groups.len())
            .ok_or_else(|| format!("`group` must be below {}, the number of groups", groups.len())),
        _ => Err("give `group`, a group's name or index".into()),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "app.settings" => settings(app, p),
        "settings.addSampleStringGroup" => {
            let mut groups = groups(&app.session);
            if groups.len() >= MAX_GROUPS {
                return Err(format!("there are at most {MAX_GROUPS} sample-string groups"));
            }
            let used: usize = groups.iter().map(|g| g.strings.len()).sum();
            let name = params::string(p, "name")?.unwrap_or(NEW_GROUP);
            if name.trim().is_empty() || name.chars().count() > MAX_NAME_CHARS {
                return Err(format!("`name` must be from 1 to {MAX_NAME_CHARS} characters"));
            }
            let strings = checked_strings(p, "strings", MAX_STRINGS.saturating_sub(used))?.unwrap_or_default();
            groups.push(Group { name: name.to_string(), strings });
            store(&mut app.session, &groups)?;
            app.view.settings.group = groups.len().saturating_sub(1);
            Ok(())
        }
        "settings.removeSampleStringGroup" => {
            let mut groups = groups(&app.session);
            let at = group_index(&groups, p)?;
            groups.remove(at);
            store(&mut app.session, &groups)?;
            let st = &mut app.view.settings;
            st.group = st.group.min(groups.len().saturating_sub(1));
            Ok(())
        }
        "settings.setSampleStrings" => {
            let mut groups = groups(&app.session);
            let at = group_index(&groups, p)?;
            let others: usize = groups.iter().enumerate().filter(|(i, _)| *i != at).map(|(_, g)| g.strings.len()).sum();
            let strings = checked_strings(p, "strings", MAX_STRINGS.saturating_sub(others))?.ok_or("give `strings`, a list of strings")?;
            if let Some(g) = groups.get_mut(at) {
                g.strings = strings;
            }
            store(&mut app.session, &groups)
        }
        "settings.resetSampleStrings" => {
            app.session.settings_mut().clear_reserved(Reserved::SampleStrings);
            app.view.settings.group = 0;
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn settings(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let open = params::flag(p, "open")?.unwrap_or(true);
    let section = match params::string(p, "section")? {
        None => None,
        Some(name) => {
            let Some(s) = SECTIONS.iter().find(|s| s.id == name) else {
                let ids: Vec<&str> = SECTIONS.iter().map(|s| s.id).collect();
                return Err(format!("there is no section `{}`; the sections are {}", params::quoted(name), ids.join(", ")));
            };
            if !s.available {
                return Err(format!("the {} section is not available: {}", s.label, s.unavailable));
            }
            Some(s.id)
        }
    };
    let group = match p.get("group") {
        None | Some(Value::Null) => None,
        Some(_) => Some(group_index(&groups(&app.session), p)?),
    };
    let st = &mut app.view.settings;
    if let Some(s) = section {
        st.section = s.to_string();
    }
    if let Some(g) = group {
        st.group = g;
    }
    st.open = open;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{MAX_STRING_BYTES, default_groups, groups};
    use crate::app::{APP, FontCraftApp, Msg};

    fn shell(app: &mut FontCraftApp, id: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
    }

    /// The sample-string commands write the settings within their limits, and Default brings the built-in groups back.
    #[test]
    fn sample_string_commands_edit_and_reset_the_groups() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        assert_eq!(groups(&app.session), default_groups());
        let before = default_groups().len();
        shell(&mut app, "settings.addSampleStringGroup", json!({"name": "Mine", "strings": ["abc"]})).unwrap();
        assert_eq!(app.view.settings.group, before, "the new group is shown");
        shell(&mut app, "settings.setSampleStrings", json!({"group": "Mine", "strings": ["x", "y"]})).unwrap();
        assert_eq!(groups(&app.session).last().map(|g| g.strings.clone()), Some(vec!["x".to_string(), "y".to_string()]));

        let long = "a".repeat(MAX_STRING_BYTES + 1);
        assert!(shell(&mut app, "settings.setSampleStrings", json!({"group": "Mine", "strings": [long]})).is_err());
        assert!(shell(&mut app, "settings.removeSampleStringGroup", json!({"group": 99})).is_err());
        assert_eq!(groups(&app.session).len(), before + 1, "a refused command changes nothing");

        shell(&mut app, "settings.removeSampleStringGroup", json!({"group": 0})).unwrap();
        assert_eq!(groups(&app.session).len(), before);
        shell(&mut app, "settings.resetSampleStrings", json!({})).unwrap();
        assert_eq!(groups(&app.session), default_groups());
    }

    /// `app.settings` opens on an available section and refuses one that is not built.
    #[test]
    fn settings_opens_on_available_sections_only() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        shell(&mut app, "app.settings", json!({"section": "sharing"})).unwrap();
        assert!(app.view.settings.open);
        assert_eq!(super::current(&app.view.settings), "sharing");
        let err = shell(&mut app, "app.settings", json!({"section": "shortcuts"})).unwrap_err();
        assert!(err.contains("not available"), "{err}");
        assert_eq!(super::current(&app.view.settings), "sharing");
        shell(&mut app, "app.settings", json!({"open": false})).unwrap();
        assert!(!app.view.settings.open);
    }
}
