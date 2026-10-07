//! FontCraft, the desktop app.
//!
//! ```text
//! fontcraft [FILE.fontcraft] [--control PORT]
//! ```
//! `--control PORT` listens on `127.0.0.1:PORT` for the JSON control protocol
//! (`docs/control-protocol.md`), so agents can drive the window.
#![forbid(unsafe_code)]

mod control_server;

use std::process::ExitCode;

use fontcraft_engine::Session;
use fontcraft_ui_egui::FontcraftApp;
use serde_json::json;

const USAGE: &str = "\
FontCraft, the font editor

USAGE:
  fontcraft [FILE] [--control PORT]

  FILE             a .fontcraft file to open (default: a new font)
  --control PORT   listen on 127.0.0.1:PORT for the JSON control protocol (no authentication:
                   enable it only while you use it)
  -h, --help       this text
  -V, --version    the version
";

struct Shell {
    app: FontcraftApp,
    control: Option<control_server::Inbox>,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        // Control requests run on the UI thread, between frames, like a click would.
        if let Some(inbox) = &self.control {
            inbox.drain(&mut self.app);
        }
        // A bug in drawing costs one frame, not the session.
        if let Err(panic) = fontcraft_engine::guard::catch_panic(|| self.app.ui(ui)) {
            self.app.state.status = format!("internal error (please report it): {panic}");
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
    match file {
        Some(path) => session.execute("file.open", &json!({"path": path})).map(|_| ()).map_err(|e| e.to_string())?,
        None => session.execute("file.new", &json!({})).map(|_| ()).map_err(|e| e.to_string())?,
    }
    let app = FontcraftApp::new(session);
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
            Ok(Box::new(Shell { app, control }))
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
