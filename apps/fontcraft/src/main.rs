//! FontCraft, the desktop app.
//!
//! ```text
//! fontcraft [FILE.fontcraft | fontcraft://show/?…] [--safe-mode] [--control PORT
//!           [--automation-read-root DIR] [--automation-write-root DIR]]
//! ```
//! `--control PORT` listens on `127.0.0.1:PORT` for the JSON control protocol
//! (`docs/control-protocol.md`), so agents can drive the window. `--automation-read-root` and
//! `--automation-write-root` (or `FONTCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`) confine the
//! files control requests read and write ([`fontcraft_engine::file_access`]).
//!
//! The app gives the engine the system clock (Last Changed stamps), the user's settings and the
//! user's custom glyph data from the settings folder ([`user_files`]), and saves the settings
//! whenever they change. It sets the extension folders (Modules and Macros in the settings
//! folder) and polls them on the first frame, when the window gains focus and every 5 seconds
//! while it has it, and gives the Extension Manager HTTPS downloads and the system browser
//! ([`net`]); `--safe-mode`, or Alt and Shift held as the window opens, lists the modules
//! but runs none. Started without a file, it shows the Start Window unless the person
//! turned that off. A document opens with one Edit View tab per display string it stores, and
//! saving writes the open tabs back.
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod clipboard;
mod control_server;
mod net;
mod pickers;
#[cfg(not(target_arch = "wasm32"))]
mod preview_server;
mod user_files;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use fontcraft_engine::Session;
use fontcraft_ui_egui::FontcraftApp;
use fontcraft_ui_egui::shell::start_window;
use serde_json::json;

const USAGE: &str = "\
FontCraft, the font editor

USAGE:
  fontcraft [FILE | URL] [--safe-mode] [--control PORT [--automation-read-root DIR]
            [--automation-write-root DIR]]

  FILE             a font file to open (default: the Start Window, or a new font when the
                   Start Window is turned off)
  URL              a fontcraft://show/?path=…&glyph=… URL: open its files and show its glyphs
                   in a new tab
  --safe-mode      start with every extension module off (also: hold Alt and Shift while the
                   window opens); Help › Load Extensions turns them on for the session
  --control PORT   listen on 127.0.0.1:PORT for the JSON control protocol (no authentication:
                   enable it only while you use it)
  --automation-read-root DIR, --automation-write-root DIR
                   control requests read files only inside the read root and write files only
                   inside the write root (also --flag=DIR, or FONTCRAFT_AUTOMATION_READ_ROOT and
                   FONTCRAFT_AUTOMATION_WRITE_ROOT; a flag replaces its variable). Each must be an
                   existing folder; a root left out grants none of its access. What you do in the
                   window is not confined. Without --control they are ignored.
  -h, --help       this text
  -V, --version    the version

  Settings and custom glyph data are read from the settings folder (FONTCRAFT_SETTINGS_DIR
  replaces it); extension modules and macros from its Modules and Macros folders.
";

struct Shell {
    app: FontcraftApp,
    control: Option<control_server::Inbox>,
    user_files: user_files::UserFiles,
    /// When the extension folders were last polled; `None` before the first frame.
    last_poll: Option<Instant>,
    /// Whether the window had focus at the last frame.
    focused: bool,
}

impl Shell {
    /// Polls the extension folders on the first frame, when the window gains focus and every
    /// [`FOLDER_POLL_MS`](fontcraft_engine::extensions::FOLDER_POLL_MS) while it has it. Alt and
    /// Shift held on the first frame start safe mode, as `--safe-mode` does.
    fn poll_extensions(&mut self, ctx: &eframe::egui::Context) {
        let every = Duration::from_millis(fontcraft_engine::extensions::FOLDER_POLL_MS);
        let (focused, launch_keys) = ctx.input(|i| (i.focused, i.modifiers.alt && i.modifiers.shift));
        let first = self.last_poll.is_none();
        if first && launch_keys && !self.app.session.extensions().safe_mode {
            self.app.session.extensions_mut().safe_mode = true;
            self.app.state.status = "Safe mode: extension modules are off. Help › Load Extensions turns them on.".into();
        }
        let gained = focused && !self.focused;
        self.focused = focused;
        let due = self.last_poll.is_none_or(|t| t.elapsed() >= every);
        if first || gained || (focused && due) {
            self.last_poll = Some(Instant::now());
            let problems = fontcraft_engine::extensions::poll_folders(self.app.session.extensions_mut());
            for p in &problems {
                eprintln!("fontcraft: {p}");
            }
            if let Some(first) = problems.first() {
                self.app.state.status = first.clone();
            }
        }
        if focused {
            ctx.request_repaint_after(every);
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        // Control requests run on the UI thread, between frames, like a click would.
        if let Some(inbox) = &mut self.control {
            inbox.deliver_screenshots(ui.ctx());
            inbox.drain(&mut self.app);
        }
        self.poll_extensions(ui.ctx());
        // A bug in drawing costs one frame, not the session. A frame that carries control-channel
        // input is confined as the request that queued it.
        if let Err(panic) = self.app.ui_with_control_input(ui) {
            self.app.state.status = format!("internal error (please report it): {panic}");
        }
        if let Some(problem) = self.user_files.save_if_changed(self.app.session.settings()) {
            eprintln!("fontcraft: {problem}");
            self.app.state.status = problem;
        }
    }

    /// The control channel's window input (`ui.click`, `ui.drag`, `ui.wheel`, `ui.move`) joins the
    /// window's own, one event per frame.
    fn raw_input_hook(&mut self, _ctx: &eframe::egui::Context, raw_input: &mut eframe::egui::RawInput) {
        self.app.raw_input_hook(raw_input);
    }
}

fn run() -> Result<(), String> {
    let mut file = None;
    let mut url = None;
    let mut port = None;
    let mut safe_mode = false;
    let root_env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let mut read_root = root_env("FONTCRAFT_AUTOMATION_READ_ROOT");
    let mut write_root = root_env("FONTCRAFT_AUTOMATION_WRITE_ROOT");
    let (mut read_given, mut write_given) = (false, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        // The roots also come as `--flag=DIR`.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--automation-") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        if flag == "--automation-read-root" || flag == "--automation-write-root" {
            let dir = inline.or_else(|| args.next()).ok_or_else(|| format!("{flag} needs a folder"))?;
            let (slot, given) =
                if flag == "--automation-read-root" { (&mut read_root, &mut read_given) } else { (&mut write_root, &mut write_given) };
            if std::mem::replace(given, true) {
                return Err(format!("{flag} is given twice: it takes one folder"));
            }
            *slot = Some(dir.into());
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("fontcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--safe-mode" => safe_mode = true,
            "--control" => {
                let value = args.next().ok_or("--control needs a port number")?;
                port = Some(value.parse::<u16>().map_err(|_| format!("--control: `{value}` is not a port number"))?);
            }
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
            other if other.get(..10).is_some_and(|s| s.eq_ignore_ascii_case("fontcraft:")) => url = Some(other.to_string()),
            other => file = Some(other.to_string()),
        }
    }
    // Checked before anything opens, so a mistyped root never leaves an unconfined channel.
    let roots = match port {
        Some(_) => fontcraft_engine::file_access::AutomationRoots::new(read_root.as_deref(), write_root.as_deref())?,
        None => None,
    };
    let mut session = Session::new();
    session.set_clock(Some(fontcraft_engine::clock::system_clock()));
    session.set_snippets_dir(user_files::settings_dir().map(|d| d.join("snippets")));
    let (user_files, mut problems) = user_files::UserFiles::load(&mut session);
    let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok());
    session.extensions_mut().started_at = started;
    problems.extend(fontcraft_engine::extensions::open_folders(session.extensions_mut(), user_files::settings_dir().as_deref(), safe_mode));
    for p in &problems {
        eprintln!("fontcraft: {p}");
    }
    // With no file to open, the Start Window takes the place of a new document while its Show
    // window on startup setting is on.
    let show_start = file.is_none() && url.is_none() && session.settings().get_bool(start_window::SHOW_AT_LAUNCH).unwrap_or(true);
    let opened = file.is_some();
    match file {
        Some(path) => {
            session.execute("file.open", &json!({"path": path})).map_err(|e| e.to_string())?;
        }
        None if show_start || url.is_some() => {}
        None => session.execute("file.new", &json!({})).map(|_| ()).map_err(|e| e.to_string())?,
    }
    let mut app = FontcraftApp::new(session);
    if let Some(roots) = &roots {
        eprintln!("fontcraft: control requests: {}", roots.describe());
    }
    app.automation_roots = roots;
    // The document's tabs open from its display strings, as they do for File › Open.
    if opened {
        for warning in app.open_display_strings() {
            eprintln!("fontcraft: {warning}");
        }
    }
    // A URL opens its files and shows its glyphs; when it opens nothing, the Start Window shows.
    if let Some(url) = url {
        match app.run("url.show", &json!({"url": url})) {
            Ok(reply) => reply
                .get("warnings")
                .and_then(|w| w.as_array())
                .into_iter()
                .flatten()
                .for_each(|w| eprintln!("fontcraft: {}", w.as_str().unwrap_or_default())),
            Err(e) => eprintln!("fontcraft: {e}"),
        }
        if app.session.documents().is_empty() {
            app.run("window.startWindow", &json!({}))?;
        }
    }
    app.picker = Some(Box::new(pickers::SystemPicker));
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.preview_address.host = Some(Box::new(preview_server::Host::default()));
        app.clipboard_flavours = Some(Box::new(clipboard::SystemClipboard::default()));
    }
    app.state.start.reveal = start_window::Reveal(Some(pickers::reveal));
    app.state.panels.extensions.manager.platform =
        fontcraft_ui_egui::shell::extensions::manager::Platform { fetch: Some(net::fetch), open_url: Some(net::open_url) };
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
                    Ok(inbox) => {
                        app.session.extensions_mut().surfaces.insert("control".into(), format!("listening on 127.0.0.1:{port}"));
                        Some(inbox)
                    }
                    Err(e) => {
                        // Another app may hold the port: say so, or requests reach the wrong app.
                        eprintln!("fontcraft: the control channel is off: cannot listen on 127.0.0.1:{port}: {e}");
                        None
                    }
                },
                None => None,
            };
            Ok(Box::new(Shell { app, control, user_files, last_poll: None, focused: false }))
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
