//! FontCraft on the craft framework: the desktop window of `fontcraft-ui`, while it is built up
//! to replace `apps/fontcraft`.
//!
//! `--safe-mode`, or Alt and Shift held as the window opens, lists the extension modules but runs none.
//! `--control PORT` listens on `127.0.0.1:PORT` for the JSON control protocol
//! (`docs/control-protocol.md`). `--automation-read-root` and `--automation-write-root` (or
//! `FONTCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`) confine the files control requests read and write.
//!
//! While `sharing.enableExternalPreview` is on, it serves External Preview to the local network
//! ([`external_preview`], run each frame through the UI's frame hook). [`preview_server`] is the
//! web server behind it and behind the TrueType preview page. The user's settings and custom glyph data are read
//! from the settings folder when the window opens and the settings written back whenever they change
//! ([`user_files`]; `FONTCRAFT_SETTINGS_DIR` replaces the folder); the snippets and the extension folders (Modules
//! and Macros) are in it too. The engine gets the system clock for Last Changed stamps. The Extension Manager's
//! HTTPS downloads are [`net`].
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod external_preview;
#[cfg(not(target_arch = "wasm32"))]
mod net;
#[cfg(not(target_arch = "wasm32"))]
mod preview_server;
#[cfg(not(target_arch = "wasm32"))]
mod user_files;

use std::process::ExitCode;

#[cfg(not(target_arch = "wasm32"))]
const USAGE: &str = "\
FontCraft, the font editor (craft framework build, in progress)

USAGE:
  fontcraft-next [--safe-mode] [--control PORT [--automation-read-root DIR]
                 [--automation-write-root DIR]]

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
  replaces it), and the settings are written back there when they change; extension modules
  and macros come from its Modules and Macros folders.
";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    let options = match options(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fontcraft-next: {error}");
            return ExitCode::from(2);
        }
    };
    match fontcraft_ui::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fontcraft-next: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The options of the command line; `None` when it only asked for the help or the version.
#[cfg(not(target_arch = "wasm32"))]
fn options(mut args: impl Iterator<Item = String>) -> Result<Option<fontcraft_ui::Options>, String> {
    let root_env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let mut read_root = root_env("FONTCRAFT_AUTOMATION_READ_ROOT");
    let mut write_root = root_env("FONTCRAFT_AUTOMATION_WRITE_ROOT");
    let (mut read_given, mut write_given) = (false, false);
    let mut port = None;
    let mut safe_mode = false;
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
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("fontcraft-next {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--safe-mode" => safe_mode = true,
            "--control" => {
                let value = args.next().ok_or("--control needs a port number")?;
                port = Some(value.parse::<u16>().map_err(|_| format!("--control: `{value}` is not a port number"))?);
            }
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    let (control, roots) = match port {
        Some(port) => control(port, read_root.as_deref(), write_root.as_deref())?,
        None => (None, None),
    };
    Ok(Some(desktop(fontcraft_ui::Options { control, roots, safe_mode, ..Default::default() })))
}

/// The control channel's listener, `None` when the port could not be bound, and the folders its requests may use.
#[cfg(not(target_arch = "wasm32"))]
type Control = (Option<std::net::TcpListener>, Option<std::sync::Arc<fontcraft_engine::file_access::AutomationRoots>>);

/// The control channel on `port` and the folders its requests may use.
#[cfg(not(target_arch = "wasm32"))]
fn control(port: u16, read_root: Option<&std::path::Path>, write_root: Option<&std::path::Path>) -> Result<Control, String> {
    // Checked before anything opens, so a mistyped root never leaves an unconfined channel.
    let roots = fontcraft_engine::file_access::AutomationRoots::new(read_root, write_root)?;
    if let Some(roots) = &roots {
        eprintln!("fontcraft-next: control requests: {}", roots.describe());
    }
    // A port another program holds is reported, and the window opens without the channel.
    let control = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => {
            eprintln!("fontcraft-next: control channel listening on 127.0.0.1:{port}");
            Some(listener)
        }
        Err(e) => {
            eprintln!("fontcraft-next: control channel: cannot listen on 127.0.0.1:{port}: {e}");
            None
        }
    };
    Ok((control, roots))
}

/// `options` with what only the desktop app supplies: the system clock, External Preview's frame hook, the preview web
/// server, the settings folder (settings, snippets and extension folders), the system browser and file manager, the
/// system clipboard, and the HTTPS downloads of the Extension Manager.
#[cfg(not(target_arch = "wasm32"))]
fn desktop(options: fontcraft_ui::Options) -> fontcraft_ui::Options {
    let frame_hook: fontcraft_ui::MakeFrameHook = Box::new(|ctx| {
        let ctx = ctx.clone();
        let wake = std::sync::Arc::new(move || ctx.request_repaint());
        Box::new(external_preview::Hook(external_preview::ExternalPreview::new(fontcraft_ui::app::preview_host::Scope::LocalNetwork, wake)))
    });
    fontcraft_ui::Options {
        frame_hook: Some(frame_hook),
        preview_host: Some(Box::new(preview_server::Host::default())),
        user_store: Some(Box::new(user_files::UserFiles::in_settings_folder())),
        opener: fontcraft_ui::Opener::system(),
        clipboard_reader: Some(fontcraft_ui::app::clipboard::system_text),
        fetch: Some(net::fetch),
        settings_dir: user_files::settings_dir(),
        clock: Some(fontcraft_engine::clock::system_clock()),
        ..options
    }
}

#[cfg(target_arch = "wasm32")]
fn main() -> ExitCode {
    ExitCode::SUCCESS
}
