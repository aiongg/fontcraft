//! The Script menu's commands over macros (`crate::macros`). Rows: `script.run`,
//! `script.rerunLast`, `script.openScriptsFolder`, `script.reloadScripts`, `script.runActionSet`,
//! `script.saveActionSet`, `script.repeatLastAction`, `script.recordMacro`. `script.runActionSet`
//! and `script.repeatLastAction` are aliases of the macro commands (decision D8).
//!
//! A macro runs as one transaction: its steps run inside the command, so the session's rollback
//! undoes all of them when one fails, and their edits become one undo step per document labelled
//! with the macro's title, as `command.batch` does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::*;
use crate::extensions::{LoadedMacro, MacroRun};
use crate::macros::{
    self, BUILT_IN_ARGS, MACRO_EXTENSION, MAX_MACRO_NESTING, MAX_MACRO_RUN_BYTES, MAX_MACRO_RUN_STEPS, MAX_MACRO_TEXT, Macro, MacroError, Recording,
    Step,
};
use crate::params::Params;
use crate::{Batch, EngineError};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "script.run",
            "(script name)",
            [],
            None,
            "{macro: name | path | {title?, description?, args?, requires?, steps: [{command: id, params?: object}]}, args?: object, glyphs?: [name], layers?: [id] | \"active\" | \"visible\" | \"masters\", dryRun?: bool} run a macro: `macro` is a macro of the Macros folder (its path there, with or without `.fcmacro`, or its title), a `.fcmacro` file, or the macro itself. Every step's command and every {\"$arg\": name} placeholder is checked first; `args` fill the declared arguments (defaults fill the rest), `glyphs` and `layers` fill the built-in arguments of the same names. The steps then run in order as one undo step labelled with the title; if one fails, none takes effect and the error names it (steps counted from 1). A macro may run macros 8 deep and never itself. dryRun: return the filled-in steps without running them → {title, steps: count | [{command, params}], results: [value]}",
            always,
            run
        ),
        cmd!(
            "script.rerunLast",
            "Re-run Last Script",
            ["Script"],
            Some("Cmd+Alt+R"),
            "{glyphs?: [name], layers?: [id]} run the macro that ran last again with the same arguments; `glyphs` and `layers` replace the ones it ran with (the current selection) → as script.run",
            has_last_macro,
            rerun_last
        ),
        cmd!(
            "script.repeatLastAction",
            "Repeat Actions",
            ["Script"],
            None,
            "{glyphs?: [name], layers?: [id]} the same as script.rerunLast: run the last macro or action set again → as script.run",
            has_last_macro,
            repeat_last
        ),
        cmd!(
            "script.runActionSet",
            "Run Action Set",
            ["Script"],
            None,
            "{set: name | path | [{command: id, params?: object}], args?: object, glyphs?: [name], layers?: [id] | \"active\" | \"visible\" | \"masters\", dryRun?: bool} run an action set, which is a macro: a macro of the Macros folder, a `.fcmacro` file or the steps themselves; as script.run → as script.run",
            always,
            run_action_set
        ),
        cmd!(
            "script.saveActionSet",
            "Save Action Set",
            ["Script"],
            None,
            "{name: string, steps: [{command: id, params?: object}], title?: string, description?: string, args?: object, requires?: [module id], path?: path, replace?: bool} write the steps as a macro file: `path`, or `name`.fcmacro in the Macros folder. Every step's command and placeholder is checked first; an existing file is replaced only with replace: true → {path}",
            always,
            save_action_set
        ).files(const { &[FileParam::write("path")] }).writes_files(),
        cmd!(
            "script.recordMacro",
            "Record Macro",
            ["Script"],
            None,
            "{action: \"start\" | \"stop\" | \"cancel\", name?: string, title?: string, description?: string, replace?: bool} start: record every font-changing command that succeeds from now on, from any surface (queries, undo and redo are left out); stop: end the recording and return it as a macro, written as `name`.fcmacro in the Macros folder when `name` is given; cancel: drop it. A recording keeps at most 100,000 steps and 64 MiB of step parameters; past either limit it stops taking steps and stop reports full: true with a message → {recording: bool, macro?, steps?: count, full?: bool, message?: string, path?: path}",
            always,
            record_macro
        ).writes_files(),
        cmd!(
            "script.openScriptsFolder",
            "Open Scripts Folder",
            ["Script"],
            Some("Cmd+Shift+Y"),
            "{} create the Macros folder when it is missing and return its path; the desktop app then shows it in the file manager → {path}",
            has_macros_dir,
            open_folder
        ),
        cmd!(
            "script.reloadScripts",
            "Reload Scripts",
            ["Script"],
            Some("Cmd+Alt+Shift+Y"),
            "{} read the Macros folder again and rebuild the Script menu's list (the folder is also checked when the app gains focus and every 5 seconds) → {changed: bool, macros: [{path, title, description, error?}]}",
            has_macros_dir,
            reload
        ),
    ]
}

fn has_last_macro(s: &Session) -> std::result::Result<(), String> {
    if s.extensions().last_macro.is_some() { Ok(()) } else { Err("no macro has run yet".into()) }
}

fn has_macros_dir(s: &Session) -> std::result::Result<(), String> {
    if s.extensions().macros_dir.is_some() { Ok(()) } else { Err("this session has no Macros folder".into()) }
}

/// A macro ready to run, and where it came from.
struct Loaded {
    /// What the self-reference check compares; `None` for a macro given inline.
    key: Option<String>,
    /// What `script.rerunLast` keeps: the macro's path in the Macros folder or its file.
    path: Option<String>,
    m: Arc<Macro>,
}

fn run(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("script.run", value)?;
    let spec = p.get("macro").ok_or_else(|| p.bad("give `macro`: a macro's name, a path to a macro file or a macro object"))?;
    let loaded = load(s, &p, spec)?;
    let args = call_args(&p)?;
    run_loaded(s, loaded, args, p.bool_or("dryRun", false)?)
}

fn run_action_set(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("script.runActionSet", value)?;
    let set = p.get("set").ok_or_else(|| p.bad("give `set`: an action set's name, a path to its file or its steps"))?;
    let loaded = match set {
        Value::Array(steps) => Loaded { key: None, path: None, m: Arc::new(Macro::from_value(&json!({ "title": "Action Set", "steps": steps }))?) },
        other => load(s, &p, other)?,
    };
    let args = call_args(&p)?;
    run_loaded(s, loaded, args, p.bool_or("dryRun", false)?)
}

fn rerun_last(s: &mut Session, value: &Value) -> Result<Value> {
    rerun(s, Params::new("script.rerunLast", value)?)
}

fn repeat_last(s: &mut Session, value: &Value) -> Result<Value> {
    rerun(s, Params::new("script.repeatLastAction", value)?)
}

fn rerun(s: &mut Session, p: Params) -> Result<Value> {
    let Some((path, m, args)) = s.extensions().last_macro.clone() else {
        return Err(p.bad("no macro has run yet"));
    };
    let mut args = match args {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for key in BUILT_IN_ARGS {
        if let Some(v) = p.get(key) {
            args.insert((*key).to_string(), v.clone());
        }
    }
    run_loaded(s, Loaded { key: None, path, m: Arc::new(m) }, Value::Object(args), false)
}

/// The macro `spec` names: one of the Macros folder, a macro file, or the macro itself. Inside a
/// run, each named macro is read once and kept in the run's [`MacroRun`].
fn load(s: &mut Session, p: &Params, spec: &Value) -> Result<Loaded> {
    match spec {
        Value::String(name) => {
            let in_run = s.batch.is_some() && !s.extensions().running_macros.is_empty();
            let by_name = format!("name:{name}");
            if in_run && let Some(hit) = s.extensions().macro_run.loaded.get(&by_name) {
                return Ok(Loaded { key: Some(hit.key.clone()), path: Some(hit.path.clone()), m: Arc::clone(&hit.m) });
            }
            let folder_key = s.extensions().find_macro(name).map(|(path, _)| format!("folder:{path}"));
            // A folder macro named by its path, its path without extension and its title is
            // copied once.
            let copied = folder_key.as_ref().filter(|_| in_run).and_then(|key| s.extensions().macro_run.loaded.get(key)).cloned();
            let loaded = match (copied, folder_key) {
                (Some(hit), _) => hit,
                (None, Some(key)) => {
                    let Some((path, m)) = s.extensions().find_macro(name) else { return Err(p.bad(format!("{name}: not found"))) };
                    let loaded = LoadedMacro { key: key.clone(), path: path.to_string(), m: Arc::new(m.clone()) };
                    if in_run {
                        s.extensions_mut().macro_run.loaded.insert(key, loaded.clone());
                    }
                    loaded
                }
                (None, None) => read_file(s, p, name, in_run)?,
            };
            if in_run {
                s.extensions_mut().macro_run.loaded.insert(by_name, loaded.clone());
            }
            Ok(Loaded { key: Some(loaded.key), path: Some(loaded.path), m: loaded.m })
        }
        Value::Object(_) => Ok(Loaded { key: None, path: None, m: Arc::new(Macro::from_value(spec)?) }),
        _ => Err(p.bad("`macro` is a macro's name, a path to a macro file or a macro object")),
    }
}

/// Reads the macro file `name`. Inside a run its bytes count against the run's
/// [`MAX_MACRO_RUN_BYTES`].
fn read_file(s: &mut Session, p: &Params, name: &str, in_run: bool) -> Result<LoadedMacro> {
    if name.len() > MAX_MACRO_TEXT || !(macros::is_macro_file(name) || name.contains(['/', '\\'])) {
        return Err(p.bad(format!("there is no macro named {name:?} in the Macros folder")));
    }
    let path = Path::new(name);
    let bytes = macros::read_macro_file(path).map_err(|e| p.bad(format!("{name}: {e}")))?;
    if in_run {
        let run = &mut s.extensions_mut().macro_run;
        run.bytes_left = run
            .bytes_left
            .checked_sub(bytes.len())
            .ok_or_else(|| p.bad(format!("{name}: the macro reads more than {} MiB of macro files in all", MAX_MACRO_RUN_BYTES >> 20)))?;
    }
    let m = Macro::parse(&bytes).map_err(|e| p.bad(format!("{name}: {e}")))?;
    let key = std::fs::canonicalize(path).map_or_else(|_| name.to_string(), |c| c.display().to_string());
    Ok(LoadedMacro { key: format!("file:{key}"), path: name.to_string(), m: Arc::new(m) })
}

/// The arguments of a run: `args`, with `glyphs` and `layers` given beside it.
fn call_args(p: &Params) -> Result<Value> {
    let mut args = p.opt_object("args")?.cloned().unwrap_or_default();
    for key in BUILT_IN_ARGS {
        if let Some(v) = p.get(key) {
            args.insert((*key).to_string(), v.clone());
        }
    }
    Ok(Value::Object(args))
}

/// The menu label and undo label of a macro.
fn label_of(m: &Macro, path: Option<&str>) -> String {
    match (m.title.trim(), path) {
        ("", Some(path)) => macros::file_stem(path).to_string(),
        ("", None) => "Macro".to_string(),
        (title, _) => title.to_string(),
    }
}

fn run_loaded(s: &mut Session, loaded: Loaded, args: Value, dry_run: bool) -> Result<Value> {
    let Loaded { key, path, m } = loaded;
    let label = label_of(&m, path.as_deref());
    if let Some(missing) = m.requires.iter().find(|id| s.extensions().registry.get(id).is_none()) {
        return Err(MacroError::MissingModule(missing.clone()).into());
    }
    // Outside a batch no macro is running: anything left on the stack is from a run that was
    // cut short.
    if s.batch.is_none() {
        s.extensions_mut().running_macros.clear();
    }
    let outermost = s.extensions().running_macros.is_empty();
    if outermost {
        s.extensions_mut().macro_run = MacroRun::default();
    }
    let running = &s.extensions().running_macros;
    if running.len() >= MAX_MACRO_NESTING {
        return Err(MacroError::Nesting.into());
    }
    if let Some(key) = &key
        && running.iter().any(|r| r == key)
    {
        return Err(MacroError::SelfReference(label).into());
    }
    let mut values_left = s.extensions().macro_run.values_left;
    let resolved = m.resolve_within(&args, &mut values_left);
    s.extensions_mut().macro_run.values_left = values_left;
    let steps = resolved?;
    if dry_run {
        return Ok(json!({ "title": label, "steps": steps }));
    }
    let outer = s.batch.is_none();
    s.extensions_mut().running_macros.push(key.unwrap_or_default());
    let ran = crate::guard::catch_panic(|| run_steps(s, &label, &steps, outer));
    if outer {
        s.batch = None;
    }
    s.extensions_mut().running_macros.pop();
    if outermost {
        // Frees the macros read during the run.
        s.extensions_mut().macro_run = MacroRun::default();
    }
    let results = ran.map_err(|panic| EngineError::Internal(format!("the macro `{label}` failed unexpectedly: {panic}")))??;
    if outermost {
        s.extensions_mut().last_macro = Some((path, Arc::unwrap_or_clone(m), args));
    }
    Ok(json!({ "title": label, "steps": steps.len(), "results": results }))
}

/// Runs resolved steps in order. `outer`: no batch or macro is running around this one, so this
/// run owns the undo step.
fn run_steps(s: &mut Session, label: &str, steps: &[Step], outer: bool) -> Result<Vec<Value>> {
    if outer {
        s.batch = Some(Batch { label: label.into(), before: Vec::new() });
    }
    let mut results = Vec::with_capacity(steps.len());
    let mut ran = Ok(());
    for (i, step) in steps.iter().enumerate() {
        let run = &mut s.extensions_mut().macro_run;
        let Some(left) = run.steps_left.checked_sub(1) else {
            ran = Err(EngineError::BadParams {
                command: "script.run".into(),
                message: format!("the macro runs more than {MAX_MACRO_RUN_STEPS} steps in all, counting the steps of the macros it runs"),
            });
            break;
        };
        run.steps_left = left;
        match s.execute(&step.command, &step.params) {
            Ok(v) => results.push(v),
            Err(e) => {
                ran =
                    Err(EngineError::BadParams { command: "script.run".into(), message: format!("step {} (`{}`) failed: {e}", i + 1, step.command) });
                break;
            }
        }
    }
    if outer
        && let Some(batch) = s.batch.take()
        && ran.is_ok()
    {
        for (id, before) in batch.before {
            let Some(i) = s.position_of(id) else { continue };
            if let Some(doc) = s.docs_mut().get_mut(i)
                && !doc.font.same_as(&before)
            {
                crate::push_undo(doc, batch.label.clone(), before);
            }
        }
    }
    ran.map(|()| results)
}

fn macros_dir(s: &Session, p: &Params) -> Result<PathBuf> {
    s.extensions().macros_dir.clone().ok_or_else(|| p.bad("this session has no Macros folder"))
}

/// Checks a macro file name given without its folder or extension.
fn check_name(p: &Params, name: &str) -> Result<()> {
    let bad = name.trim().is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    if bad {
        return Err(p.bad(format!("{name:?} cannot be a file name: give a name without folders or the characters / \\ : * ? \" < > |")));
    }
    Ok(())
}

/// Writes `m` to `target`; an existing file only when `replace`.
fn write_macro(p: &Params, target: &Path, m: &Macro, replace: bool) -> Result<()> {
    if !replace && target.exists() {
        return Err(p.bad(format!("{} exists: pass replace: true to overwrite it", target.display())));
    }
    if let Some(parent) = target.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::Io(format!("{}: {e}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(&m.to_json()).map_err(|e| EngineError::Io(e.to_string()))?;
    fontcraft_format::write_atomic(target, text.as_bytes())?;
    Ok(())
}

/// Writes `m` as `name`.fcmacro in the Macros folder and lists it there; returns the path.
fn save_in_folder(s: &mut Session, p: &Params, name: &str, m: &Macro, replace: bool) -> Result<PathBuf> {
    check_name(p, name)?;
    let file = format!("{name}.{MACRO_EXTENSION}");
    let target = macros_dir(s, p)?.join(&file);
    write_macro(p, &target, m, replace)?;
    s.extensions_mut().put_macro(file, m.clone());
    Ok(target)
}

fn save_action_set(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("script.saveActionSet", value)?;
    let name = p.str("name")?;
    check_name(&p, name)?;
    let mut set = Map::new();
    set.insert("title".into(), json!(p.opt_str("title")?.unwrap_or(name)));
    set.insert("description".into(), json!(p.opt_str("description")?.unwrap_or("")));
    set.insert("steps".into(), Value::Array(p.array("steps")?.clone()));
    if let Some(args) = p.opt_object("args")? {
        set.insert("args".into(), Value::Object(args.clone()));
    }
    if let Some(requires) = p.opt_strings("requires")? {
        set.insert("requires".into(), json!(requires));
    }
    let m = Macro::from_value(&Value::Object(set))?;
    m.check()?;
    let replace = p.bool_or("replace", false)?;
    let target = match p.opt_str("path")? {
        Some(path) => {
            let target = PathBuf::from(path);
            write_macro(&p, &target, &m, replace)?;
            target
        }
        None => save_in_folder(s, &p, name, &m, replace)?,
    };
    Ok(json!({ "path": target.display().to_string() }))
}

fn record_macro(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("script.recordMacro", value)?;
    match p.str("action")? {
        "start" => {
            if s.extensions().recording.is_some() {
                return Err(p.bad("a macro is already being recorded: stop or cancel it first"));
            }
            s.extensions_mut().recording = Some(Recording::new());
            Ok(json!({ "recording": true }))
        }
        "cancel" => {
            let Some(dropped) = s.extensions_mut().recording.take() else {
                return Err(p.bad("no macro is being recorded"));
            };
            Ok(json!({ "recording": false, "steps": dropped.steps.len() }))
        }
        "stop" => {
            let Some(recording) = s.extensions().recording.clone() else {
                return Err(p.bad("no macro is being recorded"));
            };
            let name = p.opt_str("name")?;
            let full = recording.full;
            let mut m = recording.finish(p.opt_str("title")?.or(name).unwrap_or("Recorded Macro"));
            if let Some(description) = p.opt_str("description")? {
                m.description = description.to_string();
            }
            let path = match name {
                Some(name) => Some(save_in_folder(s, &p, name, &m, p.bool_or("replace", false)?)?.display().to_string()),
                None => None,
            };
            s.extensions_mut().recording = None;
            let mut out = json!({ "recording": false, "macro": m.to_json(), "steps": m.steps.len(), "full": full, "path": path });
            if full && let Value::Object(map) = &mut out {
                map.insert(
                    "message".into(),
                    json!(format!(
                        "recording stopped taking steps at {} steps or {} MiB of parameters",
                        macros::MAX_RECORDED_STEPS,
                        macros::MAX_RECORDED_BYTES >> 20
                    )),
                );
            }
            Ok(out)
        }
        other => Err(p.bad(format!("`action` is \"start\", \"stop\" or \"cancel\", not {other:?}"))),
    }
}

fn open_folder(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("script.openScriptsFolder", value)?;
    let dir = macros_dir(s, &p)?;
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Io(format!("{}: {e}", dir.display())))?;
    Ok(json!({ "path": dir.display().to_string() }))
}

fn reload(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("script.reloadScripts", value)?;
    let dir = macros_dir(s, &p)?;
    let (stamps, files) = macros::read_folder(&dir);
    let changed = s.extensions_mut().sync_macros(files, stamps);
    Ok(json!({ "changed": changed, "macros": s.extensions().macros }))
}
