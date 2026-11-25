//! The user's keyboard shortcuts: `settings.listShortcuts`, `settings.setShortcut`, `settings.removeShortcut` and
//! `settings.resetShortcut` (`crate::shortcuts`). A combination another command has, or the system keeps, is answered
//! with data and changes nothing, so a person or an agent can decide.

use serde_json::{Map, Value, json};

use super::*;
use crate::params::Params;
use crate::settings::Reserved;
use crate::shortcuts::{Chord, Custom, MAX_CUSTOM, meet, system};

/// Longest search text, in characters.
const MAX_SEARCH_CHARS: usize = 128;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "settings.listShortcuts",
            "List Shortcuts",
            [],
            None,
            "{scope?: \"all\" | \"customized\" | \"menu\" | \"commands\" (default all; commands: what the open document allows now), search?: string} every command that can have a keyboard shortcut, under its menu or the area its id starts with. shortcut is the one it has now, default the built-in one; fixed ones cannot be changed; when says where it works; system says what the operating system does with the combination; problems names stored shortcuts that cannot be read → {shortcuts: [{command, label, section, default, shortcut, customized, fixed?, when?, system?}], problems: [string]}",
            always,
            list
        ),
        cmd!(
            "settings.setShortcut",
            "Set Shortcut",
            [],
            None,
            "{command: id, shortcut: string in Mac notation (Cmd+Alt+K: at least one modifier and a key), replace?: bool} give a command a keyboard shortcut in place of its own. When another command that can get the same press has it, or the system keeps it, nothing changes and the reply says which (set false); with replace true it is taken from those commands, unless one of them is fixed → {command, shortcut, set, conflicts: [{command, label, fixed?}], system?}",
            always,
            set
        ),
        cmd!(
            "settings.removeShortcut",
            "Remove Shortcut",
            [],
            None,
            "{command: id} take a command's keyboard shortcut away, so that no combination runs it → {command, shortcut: null}",
            always,
            remove
        ),
        cmd!(
            "settings.resetShortcut",
            "Reset Shortcut",
            [],
            None,
            "{command?: id, replace?: bool} give a command its built-in keyboard shortcut back, or every command without one. When another command has the built-in shortcut now, nothing changes and conflicts names it, unless replace is true, which takes it back → {reset: [id], conflicts: [{command, label, fixed?}]}",
            always,
            reset
        ),
    ]
}

/// A command that can have a shortcut, or a key the window keeps.
struct Action {
    id: String,
    label: String,
    section: String,
    default: Option<String>,
    when: Option<&'static str>,
    menu: bool,
    fixed: bool,
    allowed: bool,
}

impl Action {
    fn shortcut(&self, custom: &Custom) -> Option<String> {
        if self.fixed { self.default.clone() } else { custom.shortcut(&self.id, self.default.as_deref()) }
    }
}

/// The group a command without a menu row is listed under: the first part of its id, as a title.
fn area_title(id: &str) -> String {
    let area = id.split('.').next().unwrap_or_default();
    let mut out = String::new();
    for (at, c) in area.chars().enumerate() {
        match (at, c.is_uppercase()) {
            (0, _) => out.extend(c.to_uppercase()),
            (_, true) => out.extend([' ', c]),
            _ => out.push(c),
        }
    }
    out
}

/// The engine's commands that change or show something, then the window's.
fn actions(s: &Session) -> Vec<Action> {
    let engine = command_specs().iter().filter(|spec| spec.journal).map(|spec| Action {
        id: spec.id.to_string(),
        label: spec.label.to_string(),
        section: if spec.menu.is_empty() { area_title(spec.id) } else { spec.menu.join(" › ") },
        default: spec.shortcut.map(str::to_string),
        when: spec.when,
        menu: !spec.menu.is_empty(),
        fixed: false,
        allowed: (spec.enabled)(s).is_ok(),
    });
    let window = s.window_commands().iter().map(|w| Action {
        id: w.id.clone(),
        label: w.label.clone(),
        section: if w.section.is_empty() { area_title(&w.id) } else { w.section.clone() },
        default: w.shortcut.clone(),
        when: None,
        menu: w.menu,
        fixed: w.fixed,
        allowed: true,
    });
    engine.chain(window).collect()
}

fn list(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.listShortcuts", v)?;
    let scope = p.opt_str("scope")?.unwrap_or("all");
    if !matches!(scope, "all" | "customized" | "menu" | "commands") {
        return Err(p.bad("`scope` is all, customized, menu or commands"));
    }
    let search: String = p.opt_str("search")?.unwrap_or_default().chars().take(MAX_SEARCH_CHARS).collect::<String>().to_lowercase();
    let custom = Custom::of(s.settings());
    let rows: Vec<Value> = actions(s)
        .iter()
        .filter(|a| match scope {
            "customized" => !a.fixed && custom.get(&a.id).is_some(),
            "menu" => a.menu,
            "commands" => a.allowed,
            _ => true,
        })
        .filter(|a| search.is_empty() || a.label.to_lowercase().contains(&search) || a.id.to_lowercase().contains(&search))
        .map(|a| row(a, &custom))
        .collect();
    Ok(json!({ "shortcuts": rows, "problems": custom.problems() }))
}

fn row(a: &Action, custom: &Custom) -> Value {
    let shortcut = a.shortcut(custom);
    let mut out = Map::new();
    out.insert("command".into(), json!(a.id));
    out.insert("label".into(), json!(a.label));
    out.insert("section".into(), json!(a.section));
    out.insert("default".into(), json!(a.default));
    out.insert("customized".into(), json!(!a.fixed && custom.get(&a.id).is_some()));
    if a.fixed {
        out.insert("fixed".into(), json!(true));
    }
    if let Some(when) = a.when {
        out.insert("when".into(), json!(when));
    }
    if let Some(found) = shortcut.as_deref().and_then(|text| Chord::parse(text).ok()).and_then(|chord| system(&chord)) {
        out.insert("system".into(), json!(found.reason));
    }
    out.insert("shortcut".into(), json!(shortcut));
    Value::Object(out)
}

/// The action `id` whose shortcut can be changed.
fn changeable<'a>(actions: &'a [Action], p: &Params, id: &str) -> Result<&'a Action> {
    match actions.iter().find(|a| a.id == id && !a.fixed) {
        Some(action) => Ok(action),
        None if actions.iter().any(|a| a.id == id) => Err(p.bad(format!("the key of `{id}` is fixed"))),
        None => Err(p.bad(format!("there is no command `{id}` a shortcut can run"))),
    }
}

/// The other actions that have `chord` now and can get the same press as `of`.
fn holders<'a>(actions: &'a [Action], of: &Action, chord: &Chord, custom: &Custom) -> Vec<&'a Action> {
    let pressed = chord.pressed();
    actions
        .iter()
        .filter(|a| a.id != of.id && meet(a.when, of.when))
        .filter(|a| a.shortcut(custom).and_then(|text| Chord::parse(&text).ok()).is_some_and(|c| c.pressed() == pressed))
        .collect()
}

fn conflicts_json(holders: &[&Action]) -> Vec<Value> {
    holders
        .iter()
        .map(|a| {
            let mut out = json!({"command": a.id, "label": a.label});
            if a.fixed
                && let Some(map) = out.as_object_mut()
            {
                map.insert("fixed".into(), json!(true));
            }
            out
        })
        .collect()
}

/// Takes the shortcut away from each of `holders`.
fn take_from(custom: &mut Custom, holders: &[&Action], p: &Params) -> Result<()> {
    for holder in holders {
        if holder.default.is_none() {
            custom.remove(&holder.id);
        } else if !custom.set(&holder.id, None) {
            return Err(p.bad(format!("at most {MAX_CUSTOM} commands have a shortcut of their own")));
        }
    }
    Ok(())
}

fn store(s: &mut Session, custom: &Custom, p: &Params) -> Result<()> {
    if custom.is_empty() {
        s.settings_mut().clear_reserved(Reserved::Shortcuts);
        return Ok(());
    }
    s.settings_mut().set_reserved(Reserved::Shortcuts, &custom.to_value()).map_err(|e| p.bad(e.to_string()))
}

fn set(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.setShortcut", v)?;
    let id = p.str("command")?;
    let chord = Chord::custom(p.str("shortcut")?).map_err(|e| p.bad(e))?;
    let replace = p.bool_or("replace", false)?;
    let actions = actions(s);
    let action = changeable(&actions, &p, id)?;
    let mut custom = Custom::of(s.settings());
    let found = system(&chord);
    let holders = holders(&actions, action, &chord, &custom);
    let blocked = found.is_some_and(|f| f.blocked);
    let held = !holders.is_empty() && (!replace || holders.iter().any(|h| h.fixed));
    let set = !blocked && !held;
    if set {
        take_from(&mut custom, &holders, &p)?;
        let own = action.default.as_deref().and_then(|text| Chord::parse(text).ok()) == Some(chord);
        if own {
            custom.remove(id);
        } else if !custom.set(id, Some(chord)) {
            return Err(p.bad(format!("at most {MAX_CUSTOM} commands have a shortcut of their own")));
        }
        store(s, &custom, &p)?;
    }
    let mut out = json!({"command": id, "shortcut": action.shortcut(&custom), "set": set, "conflicts": conflicts_json(&holders)});
    if let (Some(found), Some(map)) = (found, out.as_object_mut()) {
        map.insert("system".into(), json!(found.reason));
    }
    Ok(out)
}

fn remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.removeShortcut", v)?;
    let id = p.str("command")?;
    let actions = actions(s);
    let action = changeable(&actions, &p, id)?;
    let mut custom = Custom::of(s.settings());
    if action.default.is_none() {
        custom.remove(id);
    } else if !custom.set(id, None) {
        return Err(p.bad(format!("at most {MAX_CUSTOM} commands have a shortcut of their own")));
    }
    store(s, &custom, &p)?;
    Ok(json!({ "command": id, "shortcut": null }))
}

fn reset(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.resetShortcut", v)?;
    let mut custom = Custom::of(s.settings());
    let Some(id) = p.opt_str("command")? else {
        let reset: Vec<String> = custom.iter().map(|(id, _)| id.to_string()).collect();
        s.settings_mut().clear_reserved(Reserved::Shortcuts);
        return Ok(json!({ "reset": reset, "conflicts": [] }));
    };
    let replace = p.bool_or("replace", false)?;
    // An override of a command this build does not have can be dropped too.
    if custom.get(id).is_none() {
        changeable(&actions(s), &p, id)?;
        return Ok(json!({ "reset": [], "conflicts": [] }));
    }
    let actions = actions(s);
    let mut conflicts = Vec::new();
    if let Some(action) = actions.iter().find(|a| a.id == id && !a.fixed)
        && let Some(chord) = action.default.as_deref().and_then(|text| Chord::parse(text).ok())
    {
        // Only a command the user gave the shortcut to stands in the way; built-in ones may share it.
        let taken: Vec<&Action> =
            holders(&actions, action, &chord, &custom).into_iter().filter(|h| matches!(custom.get(&h.id), Some(Some(_)))).collect();
        conflicts = conflicts_json(&taken);
        if !taken.is_empty() && !replace {
            return Ok(json!({ "reset": [], "conflicts": conflicts }));
        }
        take_from(&mut custom, &taken, &p)?;
    }
    custom.remove(id);
    store(s, &custom, &p)?;
    Ok(json!({ "reset": [id], "conflicts": conflicts }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn shown(s: &mut Session, id: &str) -> Value {
        let list = s.query("settings.listShortcuts", &json!({})).unwrap();
        list["shortcuts"].as_array().unwrap().iter().find(|r| r["command"] == id && r["fixed"] != true).cloned().unwrap()
    }

    /// A combination another command has changes nothing until it is taken, removing and resetting bring the built-in
    /// one back, and the system's combinations are refused as data.
    #[test]
    fn shortcuts_are_set_taken_removed_and_reset() {
        let mut s = Session::with_document();
        let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap();
        let undo = shown(&mut s, "edit.undo");
        assert_eq!((&undo["shortcut"], &undo["section"], &undo["customized"]), (&json!("Cmd+Z"), &json!("Edit"), &json!(false)));
        let r = run(&mut s, "settings.setShortcut", json!({"command": "edit.redo", "shortcut": "cmd+z"}));
        assert_eq!((&r["set"], &r["conflicts"][0]["command"], &r["shortcut"]), (&json!(false), &json!("edit.undo"), &json!("Cmd+Shift+Z")));
        assert_eq!(shown(&mut s, "edit.undo")["shortcut"], "Cmd+Z", "a conflict leaves the old binding");
        run(&mut s, "settings.setShortcut", json!({"command": "edit.redo", "shortcut": "Cmd+Z", "replace": true}));
        assert_eq!((shown(&mut s, "edit.redo")["shortcut"].clone(), shown(&mut s, "edit.undo")["shortcut"].clone()), (json!("Cmd+Z"), Value::Null));
        let customized = s.query("settings.listShortcuts", &json!({"scope": "customized"})).unwrap();
        assert_eq!(customized["shortcuts"].as_array().unwrap().len(), 2);
        let r = run(&mut s, "settings.resetShortcut", json!({"command": "edit.undo"}));
        assert_eq!((&r["reset"], &r["conflicts"][0]["command"]), (&json!([]), &json!("edit.redo")));
        run(&mut s, "settings.resetShortcut", json!({"command": "edit.undo", "replace": true}));
        assert_eq!((shown(&mut s, "edit.undo")["shortcut"].clone(), shown(&mut s, "edit.redo")["shortcut"].clone()), (json!("Cmd+Z"), Value::Null));
        run(&mut s, "settings.resetShortcut", json!({}));
        assert_eq!(s.settings().get("shortcuts.custom").unwrap(), Value::Null, "nothing is stored when nothing is customized");
        run(&mut s, "settings.removeShortcut", json!({"command": "edit.undo"}));
        assert_eq!(shown(&mut s, "edit.undo")["shortcut"], Value::Null);

        let blocked = if cfg!(target_os = "macos") { "Cmd+Tab" } else { "Alt+F4" };
        let r = run(&mut s, "settings.setShortcut", json!({"command": "edit.redo", "shortcut": blocked, "replace": true}));
        assert!(r["set"] == false && r["system"].is_string(), "{r}");
        for bad in [json!({"command": "edit.redo", "shortcut": "K"}), json!({"command": "no.such", "shortcut": "Cmd+Alt+Shift+K"})] {
            assert!(s.execute("settings.setShortcut", &bad).is_err());
        }
        assert!(s.execute("settings.set", &json!({"id": "shortcuts.custom", "value": []})).is_err(), "only these commands write it");
    }
}
