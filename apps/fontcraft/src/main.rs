//! FontCraft, the desktop app: the window of `fontcraft-ui` on the craft framework.
//!
//! A FILE argument opens that font with a tab per display string it stores; a `fontcraft://show/?…` URL opens its
//! files and shows its glyphs (`url.show`). Either takes the place of the Start Window, which also shows when they
//! open nothing.
//! `--safe-mode`, or Alt and Shift held as the window opens, lists the extension modules but runs none.
//! `--control PORT` listens on `127.0.0.1:PORT` for the JSON control protocol
//! (`docs/control-protocol.md`). `--automation-read-root` and `--automation-write-root` (or
//! `FONTCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`) confine the files control requests read and write.
//!
//! While `sharing.enableExternalPreview` is on, it serves External Preview to the local network
//! ([`external_preview`], run each frame through the UI's frame hook). [`preview_server`] is the
//! web server behind it and behind the TrueType preview page. The user's settings, the window's layout and custom glyph
//! data are read from the settings folder when the window opens, and the settings and the layout written back whenever
//! they change ([`user_files`]; `FONTCRAFT_SETTINGS_DIR` replaces the folder); the window's place, the snippets and the
//! extension folders (Modules and Macros) are in it too. The engine gets the system clock for Last Changed stamps. The PDF a drawing app puts on
//! the clipboard is read off the window's thread for a paste on the canvas ([`clipboard`]). The Extension Manager's
//! HTTPS downloads are [`net`]. File › Print… prints through the system's queue ([`spooler`]).
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod app_icon;
#[cfg(not(target_arch = "wasm32"))]
mod clipboard;
#[cfg(not(target_arch = "wasm32"))]
mod external_preview;
#[cfg(not(target_arch = "wasm32"))]
mod layout_file;
#[cfg(not(target_arch = "wasm32"))]
mod net;
#[cfg(not(target_arch = "wasm32"))]
mod preview_server;
#[cfg(not(target_arch = "wasm32"))]
mod recovery;
#[cfg(not(target_arch = "wasm32"))]
mod settings_file;
#[cfg(not(target_arch = "wasm32"))]
mod spooler;
#[cfg(not(target_arch = "wasm32"))]
mod user_files;

use std::process::ExitCode;

#[cfg(not(target_arch = "wasm32"))]
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

  Settings, the window's layout and custom glyph data are read from the settings folder, and the
  settings and the layout are written back there when they change; extension modules and macros
  come from its Modules and Macros folders. FONTCRAFT_SETTINGS_DIR replaces the folder; set but
  empty, it keeps everything in memory.
";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    let options = match options(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fontcraft: {error}");
            return ExitCode::from(2);
        }
    };
    match fontcraft_ui::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fontcraft: {error}");
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
    let mut launch = None;
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
                println!("fontcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--safe-mode" => safe_mode = true,
            "--control" => {
                let value = args.next().ok_or("--control needs a port number")?;
                port = Some(value.parse::<u16>().map_err(|_| format!("--control: `{value}` is not a port number"))?);
            }
            other => launch = Some(launch_arg(other)?),
        }
    }
    let (control, roots) = match port {
        Some(port) => control(port, read_root.as_deref(), write_root.as_deref())?,
        None => (None, None),
    };
    Ok(Some(desktop(fontcraft_ui::Options { control, roots, safe_mode, launch, ..Default::default() })))
}

/// What a command-line argument that is no option opens: a `fontcraft:` URL, else a font file. An unknown option is
/// refused.
#[cfg(not(target_arch = "wasm32"))]
fn launch_arg(arg: &str) -> Result<fontcraft_ui::app::startup::Launch, String> {
    use fontcraft_ui::app::startup::Launch;
    if arg.starts_with('-') {
        return Err(format!("unknown option `{arg}`\n\n{USAGE}"));
    }
    if arg.get(..10).is_some_and(|s| s.eq_ignore_ascii_case("fontcraft:")) {
        return Ok(Launch::Url(arg.to_string()));
    }
    Ok(Launch::File(arg.to_string()))
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
        eprintln!("fontcraft: control requests: {}", roots.describe());
    }
    // A port another program holds is reported, and the window opens without the channel.
    let control = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => {
            eprintln!("fontcraft: control channel listening on 127.0.0.1:{port}");
            Some(listener)
        }
        Err(e) => {
            eprintln!("fontcraft: control channel: cannot listen on 127.0.0.1:{port}: {e}");
            None
        }
    };
    Ok((control, roots))
}

/// `options` with what only the desktop app supplies: the system clock, External Preview's frame hook, the preview web
/// server, the folder of copies of unsaved work ([`recovery`]), the settings folder (settings, snippets and extension folders), the system browser and file manager, the
/// system clipboard's text and PDF ([`clipboard`]), and the HTTPS downloads of the Extension Manager.
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
        print_host: Some(std::sync::Arc::new(spooler::Spooler)),
        recovery: recovery::Copies::open(user_files::settings_dir().as_deref())
            .map(|copies| Box::new(copies) as Box<dyn fontcraft_ui::app::recovery::RecoveryHost>),
        user_store: Some(Box::new(user_files::UserFiles::in_settings_folder())),
        opener: fontcraft_ui::Opener::system(),
        clipboard_reader: Some(fontcraft_ui::app::clipboard::system_text),
        clipboard_flavours: Some(Box::new(clipboard::SystemClipboard::default())),
        fetch: Some(net::fetch),
        settings_dir: user_files::settings_dir(),
        clock: Some(fontcraft_engine::clock::system_clock()),
        icon: Some(app_icon::viewport_icon(std::env::current_exe().ok().as_deref())),
        ..options
    }
}

#[cfg(target_arch = "wasm32")]
fn main() -> ExitCode {
    ExitCode::SUCCESS
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use fontcraft_ui::app::startup::Launch;

    use super::launch_arg;

    #[test]
    fn an_argument_names_a_url_or_a_file_and_an_unknown_option_is_refused() {
        let url = "FontCraft://show/?path=/fonts/a.glyphs&glyph=A";
        assert_eq!(launch_arg(url), Ok(Launch::Url(url.to_string())));
        assert_eq!(launch_arg("fonts/a.glyphs"), Ok(Launch::File("fonts/a.glyphs".to_string())));
        assert!(launch_arg("--safe").is_err_and(|e| e.starts_with("unknown option `--safe`")));
    }
}
