//! The TrueType hinting dialogs: the TTFZones and TTFStems editors, the Delta table they share, the TrueType
//! Instructions dialog and the TrueType Debugger; and the shell commands that open them and set the Hinting Preview's
//! scale.
//!
//! A shell command records what to open in [`TtState`] (app state); [`run`] opens the dialog on the window's host on
//! the next pass, copies what it shows into it when the document or the state moved, and posts the commands its
//! controls queued.

pub mod debugger;
pub mod deltas;
pub mod instructions;
pub mod zones;

use craft::ui::dialog::DialogHost;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::hinting::settings::main_master;
use serde_json::Value;

use self::debugger::{DebuggerDialog, DebuggerState};
use self::instructions::{InstructionsDialog, Program, Request};
use self::zones::{DefinitionsDialog, Kind, List};
use crate::app::FontCraftApp;
use crate::shell::commands::{Module, UiCommand};

/// The hinting dialogs' app state.
#[derive(Default)]
pub struct TtState {
    pub debugger: DebuggerState,
    /// What a shell command asked to open or close, taken by [`run`].
    pub asks: Vec<Ask>,
    /// The document revision the open dialogs last read.
    read_at: Option<(u64, u64)>,
}

/// A request to [`run`].
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Instructions(Request),
    CloseInstructions,
    /// The editor of a master's TT zones or TT stems; the master's id.
    Definitions(Kind, String),
}

/// The shell commands of the hinting dialogs.
pub const SHELL: Module = Module { commands: COMMANDS, run: shell };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: debugger::ID,
        label: "TrueType Debugger",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true), glyph?: name (default: the shown glyph), ppem?: number (default 12), intent?: 'grayscale'|'cleartype'|'directwrite', action?: 'start'|'into'|'over'|'out'|'continue'|'stop', toggleBreakpoint?: byte offset in the glyph program} shows or hides the TrueType Debugger and runs `hint.tt.debugStep` for it: a new glyph, size or intent starts again at the first instruction, `action` steps from the state shown",
    },
    UiCommand {
        id: "hint.tt.editInstructions",
        label: "Edit TrueType Instructions…",
        menu: &[],
        shortcut: None,
        params: "{target?: {glyph: name} | \"fpgm\" | \"prep\" (default the shown glyph), master?: id (a glyph: the layer of that master, default the main master; fpgm and prep: shown from that master and stored to it only, default the main master shown and every master stored), open?: bool (false closes the dialog)} opens the TrueType Instructions dialog on the program",
    },
    UiCommand {
        id: "window.ttfZones",
        label: "Edit TT Zones…",
        menu: &[],
        shortcut: None,
        params: "{master?: id or name (default the main master)} opens the TTFZones editor on the master's TT zones",
    },
    UiCommand {
        id: "window.ttfStems",
        label: "Edit TT Stems…",
        menu: &[],
        shortcut: None,
        params: "{master?: id or name (default the main master)} opens the TTFStems editor on the master's TT stems",
    },
    UiCommand {
        id: crate::app::models::hinting::SCALE_COMMAND,
        label: "Preview Scale",
        menu: &[],
        shortcut: None,
        params: "{scale: 1 | 2 | 4} draw the Hinting Preview's pixels at 1x (one screen pixel per device pixel), 2x or 4x",
    },
];

fn shell(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = if command.params.is_null() { Value::Object(Default::default()) } else { command.params.clone() };
    match command.id.as_str() {
        debugger::ID => {
            let current = shown_glyph(app);
            debugger::command(app, &params, current)
        }
        "hint.tt.editInstructions" => {
            if params.get("open").and_then(Value::as_bool) == Some(false) {
                app.tt.asks.push(Ask::CloseInstructions);
                return Ok(());
            }
            let target = match params.get("target") {
                Some(target) => fontcraft_engine::cmd::tt_assembly::ProgramTarget::from_json(target)?.to_json(),
                None => serde_json::json!({ "glyph": shown_glyph(app).ok_or("no glyph is shown: give `target`")? }),
            };
            let master = params.get("master").and_then(Value::as_str).map(str::to_string);
            app.tt.asks.push(Ask::Instructions(Request { target, master }));
            Ok(())
        }
        "window.ttfZones" | "window.ttfStems" => {
            let kind = if command.id == "window.ttfZones" { Kind::Zones } else { Kind::Stems };
            let font = app.session.font().map_err(|e| e.to_string())?;
            let master = match params.get("master").and_then(Value::as_str) {
                Some(master) => font.master(master).map_err(|e| e.to_string())?.id.clone(),
                None => main_master(font).to_string(),
            };
            app.tt.asks.push(Ask::Definitions(kind, master));
            Ok(())
        }
        crate::app::models::hinting::SCALE_COMMAND => crate::app::models::hinting::set_scale(&mut app.models.hinting_waterfall, &params),
        other => Err(format!("no hinting command `{other}`")),
    }
}

/// The glyph the glyph panels show: the selected one, else the font's first.
fn shown_glyph(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    match &app.view.glyph {
        Some(name) if font.glyphs.get(name).is_some() => Some(name.clone()),
        _ => font.glyphs.first().map(|g| g.name.clone()),
    }
}

/// Opens what was asked for, refreshes the open dialogs and posts what they queued.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    for ask in std::mem::take(&mut app.tt.asks) {
        match ask {
            Ask::Instructions(request) => _ = host.open(InstructionsDialog::new(request)),
            Ask::CloseInstructions => {
                if let Some(dialog) = host.get_mut::<InstructionsDialog>() {
                    dialog.closing = true;
                }
            }
            Ask::Definitions(kind, master) => _ = host.open(DefinitionsDialog::new(kind, master)),
        }
        app.tt.read_at = None;
    }
    if host.take::<DebuggerDialog>().is_some() {
        app.tt.debugger.open = false;
    }
    let _ = host.take::<InstructionsDialog>();
    let _ = host.take::<DefinitionsDialog>();
    debugger_window(host, app);
    let revision = app.session.active().map(|d| (d.id, d.revision));
    let moved = app.tt.read_at != revision;
    app.tt.read_at = revision;
    if let Some(dialog) = host.get_mut::<InstructionsDialog>() {
        if moved && let Ok(font) = app.session.font() {
            dialog.program = Program::read(font, &dialog.request);
        }
        app.sent.append(&mut dialog.outbox);
    }
    if let Some(dialog) = host.get_mut::<DefinitionsDialog>() {
        if moved {
            dialog.list = app.session.font().ok().and_then(|font| List::read(font, dialog.kind, &dialog.master).ok());
        }
        app.sent.append(&mut dialog.outbox);
    }
}

/// Opens, refreshes or closes the debugger window to match [`DebuggerState`].
fn debugger_window(host: &mut DialogHost, app: &mut FontCraftApp) {
    let st = &app.tt.debugger;
    match host.get_mut::<DebuggerDialog>() {
        Some(dialog) => {
            if !st.open {
                dialog.closing = true;
            } else if dialog.state.generation != st.generation {
                dialog.state = st.clone();
            }
            app.sent.append(&mut dialog.outbox);
        }
        None if st.open => _ = host.open(DebuggerDialog::new(st.clone())),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::*;
    use crate::app::{APP, Msg};

    fn app() -> FontCraftApp {
        FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()))
    }

    /// The open commands queue their dialog; a bad target or master is refused and queues nothing.
    #[test]
    fn open_commands_queue_their_dialogs() {
        let mut app = app();
        app.sent.push(Msg::Shell(CommandRef::with_params("hint.tt.editInstructions", json!({"target": "fpgm"}))));
        app.sent.push(Msg::Shell(CommandRef::with_params("window.ttfZones", json!({}))));
        app.sent.push(Msg::Shell(CommandRef::with_params("window.ttfStems", json!({"master": "no such master"}))));
        app.drain();
        let master = app.session.font().map(|f| main_master(f).to_string()).unwrap_or_default();
        assert_eq!(app.tt.asks, vec![Ask::Instructions(Request { target: json!("fpgm"), master: None }), Ask::Definitions(Kind::Zones, master)]);
        assert!(app.status.contains("window.ttfStems"), "the refused master reaches the status line: {}", app.status);
    }

    /// The preview scale takes 1, 2 or 4 and refuses anything else.
    #[test]
    fn the_preview_scale_takes_one_two_or_four() {
        let mut app = app();
        assert_eq!(app.models.hinting_waterfall.scale(), 1);
        for (scale, shown) in [(json!(2), 2), (json!(3), 2), (json!("4"), 2), (json!(4), 4)] {
            app.sent.push(Msg::Shell(CommandRef::with_params(crate::app::models::hinting::SCALE_COMMAND, json!({ "scale": scale }))));
            app.drain();
            assert_eq!(app.models.hinting_waterfall.scale(), shown, "after {scale}");
        }
    }
}
