//! The Macro Panel's editor: a macro written in the Command Console window, one step per line, run in place, opened
//! from and saved to `.fcmacro` files, and filled by recording.
//!
//! | Command | Does |
//! |---|---|
//! | `macroPanel.setText {text}` | puts the text in the editor |
//! | `macroPanel.runAll` | runs every line as one macro (`script.run`), one undo step |
//! | `macroPanel.runSelection {first, last}` | runs those lines, counted from 1, as one macro |
//! | `macroPanel.open {path}` | reads a macro file into the editor, one step per line |
//! | `macroPanel.save {path}` | writes the editor as a macro file (`script.saveActionSet`) |
//! | `macroPanel.record {on?}` | starts recording (`script.recordMacro`), or stops it and appends the steps |
//!
//! A line is a step as the console's input line takes it (`app::console::parse_line`): a command id then its JSON
//! parameters, or a run-style line, which gives one step per `--cmd`. Blank lines are skipped. The file is the
//! engine's macro format (`fontcraft_engine::macros`): the editor holds its steps, and the title, description,
//! arguments and required modules of the file opened last ride along unchanged, so placeholders (`{"$arg": name}`)
//! keep working. Running fills `glyphs` and `layers` from the selection as the Script menu does
//! (`app::menus_dynamic::fill_targets`).
//!
//! Without `path`, Open and Save ask for the file first (`app::files::PICKED`).

use std::path::Path;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::macros::{self, MAX_MACRO_BYTES, MAX_MACRO_STEPS, Macro, Step};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;
use crate::app::console;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Largest editor text, in bytes: a macro file's limit.
pub const MAX_EDITOR_BYTES: usize = MAX_MACRO_BYTES;
/// The macro file extension the pickers offer.
pub const EXTENSION: &str = macros::MACRO_EXTENSION;
/// A macro's title when the editor's was never named.
const UNTITLED: &str = "Macro Panel";

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "macroPanel.setText",
        label: "Set Macro Text",
        menu: &[],
        shortcut: None,
        params: "{text: string (at most 4 MiB)} put the text in the Macro Panel's editor: one step per line, a command id then its JSON parameters, or a run-style line (`run --cmd ID --params JSON`, the pair repeated)",
    },
    UiCommand {
        id: "macroPanel.runAll",
        label: "Run All",
        menu: &[],
        shortcut: None,
        params: "{} run every line of the Macro Panel's editor as one macro: one undo step; if a step fails, none takes effect and the error names it. `glyphs` and `layers` placeholders take the selection",
    },
    UiCommand {
        id: "macroPanel.runSelection",
        label: "Run Selection",
        menu: &[],
        shortcut: None,
        params: "{first: line, last: line} run the editor's lines from first to last (counted from 1) as one macro, as macroPanel.runAll runs them all",
    },
    UiCommand {
        id: "macroPanel.open",
        label: "Open Macro…",
        menu: &[],
        shortcut: None,
        params: "{path?: path} read a .fcmacro file into the Macro Panel's editor in place of its text, one step per line; its title, description, arguments and required modules are kept for running and saving. Without path the window asks for the file",
    },
    UiCommand {
        id: "macroPanel.save",
        label: "Save Macro…",
        menu: &[],
        shortcut: None,
        params: "{path?: path} write the Macro Panel's editor as a .fcmacro file at path, replacing a file there; every line must read as steps first. Without path the window asks for the file",
    },
    UiCommand {
        id: "macroPanel.record",
        label: "Record into Editor",
        menu: &[],
        shortcut: None,
        params: "{on?: bool (default: the opposite of now)} on: start recording every font-changing command that succeeds, as script.recordMacro does; off: stop and add the recorded steps to the end of the Macro Panel's editor, one per line",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "macroPanel.setText" => {
            let text = p.get("text").and_then(Value::as_str).ok_or("`text` (a string) is required")?;
            if text.len() > MAX_EDITOR_BYTES {
                return Err(format!("the editor holds at most {} MiB", MAX_EDITOR_BYTES >> 20));
            }
            app.view.commands.console.editor = text.to_string();
            Ok(())
        }
        "macroPanel.runAll" => {
            let count = line_count(&app.view.commands.console.editor);
            run_lines(app, 1, count.max(1))
        }
        "macroPanel.runSelection" => {
            let line = |key: &str| {
                p.get(key)
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| format!("`{key}` (a line, counted from 1) is required"))
            };
            let (first, last) = (line("first")?, line("last")?);
            let count = line_count(&app.view.commands.console.editor);
            if first == 0 || first > last || last > count {
                return Err(format!("the lines run from 1 to {count}, first before last; got {first} to {last}"));
            }
            run_lines(app, first, last)
        }
        "macroPanel.open" => open(app, path(p)?),
        "macroPanel.save" => save(app, path(p)?),
        "macroPanel.record" => {
            let on = match p.get("on") {
                None | Some(Value::Null) => app.session.extensions().recording.is_none(),
                Some(Value::Bool(on)) => *on,
                Some(_) => return Err("`on` must be true or false".into()),
            };
            if on { app.session.execute("script.recordMacro", &json!({"action": "start"})).map(|_| ()).map_err(|e| e.to_string()) } else { stop(app) }
        }
        other => Err(format!("no Macro Panel command `{other}`")),
    }
}

fn path(p: &Value) -> Result<&str, String> {
    match p.get("path") {
        Some(Value::String(s)) if !s.is_empty() && s.len() <= macros::MAX_MACRO_TEXT => Ok(s),
        Some(Value::String(_)) => Err(format!("`path` has 1 to {} bytes", macros::MAX_MACRO_TEXT)),
        _ => Err("`path` (a .fcmacro file) is required".into()),
    }
}

/// The number of lines of `text`: a last line without a line break counts, an empty text has none.
pub fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// The lines `first` to `last` (counted from 1) of `text` as macro steps; blank lines give none.
pub fn steps(text: &str, first: usize, last: usize) -> Result<Vec<Step>, String> {
    let mut out = Vec::new();
    for (at, line) in text.lines().enumerate().skip(first.saturating_sub(1)).take(last.saturating_sub(first.saturating_sub(1))) {
        let parsed = console::parse_line(line).map_err(|e| format!("line {}: {e}", at + 1))?;
        for (command, params) in parsed {
            if out.len() >= MAX_MACRO_STEPS {
                return Err(format!("a macro has at most {MAX_MACRO_STEPS} steps"));
            }
            out.push(Step { command, params });
        }
    }
    Ok(out)
}

/// A step as one editor line: the command id, then its parameters as JSON unless it has none.
pub fn line_of(step: &Step) -> String {
    match &step.params {
        Value::Null => step.command.clone(),
        Value::Object(map) if map.is_empty() => step.command.clone(),
        params => format!("{} {params}", step.command),
    }
}

/// The editor's macro with `steps`: the title, description, arguments and required modules of the file opened last.
fn macro_of(header: &Macro, steps: Vec<Step>, fallback_title: &str) -> Map<String, Value> {
    let mut m = Map::new();
    let title = if header.title.trim().is_empty() { fallback_title } else { header.title.as_str() };
    m.insert("title".into(), json!(title));
    m.insert("description".into(), json!(header.description));
    if !header.args.is_empty() {
        m.insert("args".into(), Value::Object(header.args.clone()));
    }
    if !header.requires.is_empty() {
        m.insert("requires".into(), json!(header.requires));
    }
    m.insert("steps".into(), json!(steps));
    m
}

/// Runs lines `first` to `last` of the editor as one macro.
fn run_lines(app: &mut FontCraftApp, first: usize, last: usize) -> Result<(), String> {
    let state = &app.view.commands.console;
    let steps = steps(&state.editor, first, last)?;
    if steps.is_empty() {
        return Err("those lines hold no steps: write a command id and its JSON parameters on a line".into());
    }
    let m = macro_of(&state.editor_macro, steps, UNTITLED);
    let command = crate::app::menus_dynamic::fill_targets(app, CommandRef::with_params("script.run", json!({ "macro": m })))?;
    app.session.execute(&command.id, &command.params).map(|_| ()).map_err(|e| e.to_string())
}

/// Reads the macro file at `path` into the editor.
fn open(app: &mut FontCraftApp, path: &str) -> Result<(), String> {
    let bytes = macros::read_macro_file(Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
    let mut m = Macro::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let lines: Vec<String> = m.steps.iter().map(line_of).collect();
    let text = lines.join("\n");
    if text.len() > MAX_EDITOR_BYTES {
        return Err(format!("{path}: its steps take more than the editor's {} MiB", MAX_EDITOR_BYTES >> 20));
    }
    m.steps.clear();
    let state = &mut app.view.commands.console;
    state.editor = text;
    state.editor_macro = m;
    Ok(())
}

/// Writes the editor to `path` as a macro file.
fn save(app: &mut FontCraftApp, path: &str) -> Result<(), String> {
    let state = &app.view.commands.console;
    let steps = steps(&state.editor, 1, line_count(&state.editor).max(1))?;
    if steps.is_empty() {
        return Err("the editor holds no steps to save".into());
    }
    let name = Path::new(path).file_stem().and_then(|s| s.to_str()).filter(|s| !s.trim().is_empty()).unwrap_or(UNTITLED).to_string();
    let mut p = macro_of(&state.editor_macro, steps, &name);
    p.insert("name".into(), json!(name));
    p.insert("path".into(), json!(path));
    // The file picker asked before replacing a file.
    p.insert("replace".into(), json!(true));
    app.session.execute("script.saveActionSet", &Value::Object(p)).map(|_| ()).map_err(|e| e.to_string())
}

/// Stops recording and adds the recorded steps to the end of the editor. A recording too large for the editor keeps
/// going, so nothing is lost.
fn stop(app: &mut FontCraftApp) -> Result<(), String> {
    let Some(recording) = app.session.extensions().recording.as_ref() else {
        return Err("no macro is being recorded".into());
    };
    let lines: Vec<String> = recording.steps.iter().map(line_of).collect();
    let editor = &app.view.commands.console.editor;
    let mut text = editor.clone();
    if !text.is_empty() && !text.ends_with('\n') && !lines.is_empty() {
        text.push('\n');
    }
    text.push_str(&lines.join("\n"));
    if text.len() > MAX_EDITOR_BYTES {
        return Err(format!("the recorded steps do not fit in the editor's {} MiB: the recording goes on", MAX_EDITOR_BYTES >> 20));
    }
    app.session.execute("script.recordMacro", &json!({"action": "stop"})).map_err(|e| e.to_string())?;
    app.view.commands.console.editor = text;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
