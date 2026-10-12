//! FontCraft, the desktop app.
//!
//! ```text
//! fontcraft [FILE.fontcraft] [--control PORT]
//! ```
//! `--control PORT` listens on `127.0.0.1:PORT` for the JSON control protocol
//! (`docs/control-protocol.md`), so agents can drive the window.
//!
//! The app gives the engine the system clock (Last Changed stamps), the user's settings and the
//! user's custom glyph data from the settings folder ([`user_files`]), and saves the settings
//! whenever they change. Started without a file, it shows the Start Window unless the person
//! turned that off. A document opens with one Edit View tab per display string it stores, and
//! saving writes the open tabs back.
#![forbid(unsafe_code)]

mod control_server;
mod pickers;
#[cfg(not(target_arch = "wasm32"))]
mod preview_server;
mod user_files;

use std::process::ExitCode;

use fontcraft_engine::Session;
use fontcraft_ui_egui::FontcraftApp;
use fontcraft_ui_egui::shell::start_window;
use serde_json::json;

const USAGE: &str = "\
FontCraft, the font editor

USAGE:
  fontcraft [FILE] [--control PORT]

  FILE             a font file to open (default: the Start Window, or a new font when the
                   Start Window is turned off)
  --control PORT   listen on 127.0.0.1:PORT for the JSON control protocol (no authentication:
                   enable it only while you use it)
  -h, --help       this text
  -V, --version    the version

  Settings and custom glyph data are read from the settings folder (FONTCRAFT_SETTINGS_DIR
  replaces it).
";

struct Shell {
    app: FontcraftApp,
    control: Option<control_server::Inbox>,
    user_files: user_files::UserFiles,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        // Control requests run on the UI thread, between frames, like a click would.
        if let Some(inbox) = &mut self.control {
            inbox.deliver_screenshots(ui.ctx());
            inbox.drain(&mut self.app);
        }
        // A bug in drawing costs one frame, not the session.
        if let Err(panic) = fontcraft_engine::guard::catch_panic(|| self.app.ui(ui)) {
            self.app.state.status = format!("internal error (please report it): {panic}");
        }
        if let Some(problem) = self.user_files.save_if_changed(self.app.session.settings()) {
            eprintln!("fontcraft: {problem}");
            self.app.state.status = problem;
        }
    }
}

fn run() -> Result<(), String> {
    let mut file = None;
    let mut port = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("fontcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--control" => {
                let value = args.next().ok_or("--control needs a port number")?;
                port = Some(value.parse::<u16>().map_err(|_| format!("--control: `{value}` is not a port number"))?);
            }
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
            other => file = Some(other.to_string()),
        }
    }
    let mut session = Session::new();
    session.set_clock(Some(fontcraft_engine::clock::system_clock()));
    // What the user runs in the window may export where an Export Folder parameter points; the
    // control channel turns this off for its own requests.
    session.set_export_outside(true);
    let (user_files, problems) = user_files::UserFiles::load(&mut session);
    for p in &problems {
        eprintln!("fontcraft: {p}");
    }
    // With no file to open, the Start Window takes the place of a new document while its Show
    // window on startup setting is on.
    let show_start = file.is_none() && session.settings().get_bool(start_window::SHOW_AT_LAUNCH).unwrap_or(true);
    let opened = file.is_some();
    match file {
        Some(path) => {
            session.execute("file.open", &json!({"path": path})).map_err(|e| e.to_string())?;
        }
        None if show_start => {}
        None => session.execute("file.new", &json!({})).map(|_| ()).map_err(|e| e.to_string())?,
    }
    let mut app = FontcraftApp::new(session);
    // The document's tabs open from its display strings, as they do for File › Open.
    if opened {
        for warning in app.open_display_strings() {
            eprintln!("fontcraft: {warning}");
        }
    }
    app.picker = Some(Box::new(pickers::SystemPicker));
    app.state.start.reveal = start_window::Reveal(Some(pickers::reveal));
    if show_start {
        app.run("window.startWindow", &json!({}))?;
    }
    if let Some(first) = problems.first() {
        app.state.status = first.clone();
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("FontCraft")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "FontCraft",
        options,
        Box::new(move |cc| {
            let control = match port {
                Some(port) => match control_server::start(port, cc.egui_ctx.clone()) {
                    Ok(inbox) => Some(inbox),
                    Err(e) => {
                        // Another app may hold the port: say so, or requests reach the wrong app.
                        eprintln!("fontcraft: the control channel is off: cannot listen on 127.0.0.1:{port}: {e}");
                        None
                    }
                },
                None => None,
            };
            Ok(Box::new(Shell { app, control, user_files }))
        }),
    )
    .map_err(|e| format!("cannot open the window: {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fontcraft: {e}");
            ExitCode::FAILURE
        }
    }
}
