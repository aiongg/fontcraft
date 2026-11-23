//! The command surfaces' shell commands: the Command Console, the command palette and About.

use craft::behavior::dock::PanelState;
use craft::ui::runtime::commands::CommandRef;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use super::dock::{self, PanelId};
use crate::app::FontCraftApp;
use crate::app::console::{self, About, CONSOLE_COMMAND, MAX_LINE_BYTES};

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: CONSOLE_COMMAND,
        label: "Macro Panel",
        menu: &["Window"],
        shortcut: Some("Cmd+Alt+M"),
        params: "{open?: bool, line?: string, input?: string, clear?: bool, copy?: bool} show or hide the Command Console (with no parameters: show it). line: run it as Return in the console does: a command id then JSON parameters (`glyph.newGlyph {\"name\": \"a.alt\"}`), or a run-style line (`run --cmd ID --params JSON`, the pair repeated for several engine commands, which run as one step: if one fails, none takes effect); at most 1 MiB. The line joins the history, and each command it runs, with its result or error, joins the log. input: put this text in the input line. clear: empty the log pane (the session's command log keeps its entries). copy: put the log pane's text on the clipboard",
    },
    UiCommand {
        id: "app.commandPalette",
        label: "Commands…",
        menu: &["Help"],
        shortcut: Some("Cmd+Shift+P"),
        params: "{} open the search over the commands in the menus; Return runs the one picked",
    },
    UiCommand {
        id: "app.about",
        label: "About FontCraft",
        menu: &["Help"],
        shortcut: None,
        params: "{} show the version, the graphics backend and the licences of FontCraft and of what it ships",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        CONSOLE_COMMAND => console_command(app, &command.params),
        "app.commandPalette" => {
            if app.view.commands.palette.is_empty() {
                app.view.commands.palette = console::palette_entries();
            }
            app.view.commands.palette_open = true;
            Ok(())
        }
        "app.about" => {
            if app.view.commands.about == About::Closed {
                app.view.commands.about = About::Open;
            }
            Ok(())
        }
        other => Err(format!("no command-surface command `{other}`")),
    }
}

fn flag(params: &Value, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(on)) => Ok(Some(*on)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= MAX_LINE_BYTES => Ok(Some(s)),
        Some(Value::String(_)) => Err(format!("`{key}` has at most {MAX_LINE_BYTES} bytes")),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn console_command(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let open = flag(params, "open")?;
    let line = text(params, "line")?;
    let input = text(params, "input")?;
    let clear = flag(params, "clear")?.unwrap_or(false);
    let copy = flag(params, "copy")?.unwrap_or(false);
    let only_open = line.is_none() && input.is_none() && !clear && !copy;
    if let Some(open) = open.or(only_open.then_some(true)) {
        let shown = app.view.workspace.layout.panel_state(&PanelId::CommandConsole) != PanelState::Hidden;
        if shown != open {
            dock::toggle(&mut app.view.workspace, PanelId::CommandConsole);
        }
    }
    if let Some(input) = input {
        app.view.commands.console.input = input.to_string();
    }
    if clear {
        app.view.commands.console.shown_from = app.session.extensions().log.next_seq();
    }
    if let Some(line) = line {
        app.view.commands.console.input.clear();
        // The outcome is in the log the pane shows; the status line gets the error too.
        console::run_line(app, line)?;
    }
    if copy {
        let from = app.view.commands.console.shown_from;
        app.view.commands.console.copy_out = Some(console::pane_text(&app.session.extensions().log, from));
    }
    Ok(())
}
