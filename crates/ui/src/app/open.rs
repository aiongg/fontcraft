//! File › Open Folder… and font files dropped on the window. Rows: `file.open`, `file.openFolder`,
//! `file.openTwiceActivates`.
//!
//! File › Open… without a `path` asks the picker for files of every format `file.open` reads ([`OPEN_EXTENSIONS`],
//! `app/files.rs`) and opens each chosen file. A font source kept as a folder (`.ufo`, `.glyphspackage`, `.sfdir`) is
//! a file on some platforms' pickers and a folder on others, so File › Open Folder… asks for a folder instead. A file
//! that is already open is shown, not opened twice; a project file opens as a project (both decided by `file.open`).
//!
//! [`take_dropped`] runs before the frame is drawn: it removes the font files from the frame's dropped files and
//! posts a `file.open` for each, so the canvas, which places dropped images, sees only the rest.

use std::path::Path;

use craft::ui::runtime::commands::CommandRef;
use serde_json::json;

use super::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params;

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "file.openFolder",
    label: "Open Folder…",
    menu: &["File"],
    shortcut: None,
    params: "{path?: string} open a font source kept as a folder (a .ufo, .glyphspackage or .sfdir) as file.open does; a path that is not a folder is refused. From the menu, without a path, the system folder picker asks for it",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// The extensions File › Open… offers: every format `file.open` reads. `gz` stands for the gzip-compressed `.pcf.gz`
/// and `.bdf.gz`, since a picker filters by the last extension only.
pub const OPEN_EXTENSIONS: &[&str] = &[
    // Sources and projects.
    "fontcraft",
    "glyphs",
    "glyphspackage",
    "ufo",
    "ufoz",
    "designspace",
    "sfd",
    "fontcraftproject",
    // Compiled fonts.
    "otf",
    "ttf",
    "ttc",
    "otc",
    "woff",
    "woff2",
    "pfb",
    "pfa",
    "t1",
    "cff",
    "svg",
    // Bitmap fonts.
    "bdf",
    "pcf",
    "gz",
    "fnt",
    "fon",
    "pk",
    "gf",
    "dfont",
];

/// Most files one drop opens; the rest are left alone.
pub const MAX_OPENED: usize = super::files::MAX_OPENED;

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let Some(path) = params::string(&command.params, "path")? else {
        if app.picks.ask(&app.host, &app.session, command) {
            return Ok(());
        }
        return Err("give `path`, the folder to open".into());
    };
    if !Path::new(path).is_dir() {
        return Err(format!("`{}` is not a folder; file.open opens a file", params::quoted(path)));
    }
    app.apply_msg(Msg::run("file.open", json!({ "path": path })))
}

/// Whether a file named `name` is one a drop opens: a name with an extension of [`OPEN_EXTENSIONS`] in any case, a
/// `.sfdir` folder, or a TeX bitmap font named by its resolution (`cmr10.600pk`). SVG files are left to the image
/// drop, which places them on a glyph; a gzip file opens only as a `.pcf.gz` or `.bdf.gz`.
pub fn opens_on_drop(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if name.ends_with(".pcf.gz") || name.ends_with(".bdf.gz") {
        return true;
    }
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    if ext == "svg" || ext == "gz" {
        return false;
    }
    if ext == "sfdir" || OPEN_EXTENSIONS.contains(&ext) {
        return true;
    }
    ["pk", "gf"].iter().any(|kind| ext.strip_suffix(kind).is_some_and(|dpi| !dpi.is_empty() && dpi.bytes().all(|b| b.is_ascii_digit())))
}

fn is_font(file: &egui::DroppedFileHandle) -> bool {
    file.path().file_name().and_then(|n| n.to_str()).is_some_and(opens_on_drop)
}

/// Takes the dropped font files out of this frame's input and posts a `file.open` for each one that has a path on
/// disk, at most [`MAX_OPENED`]. On the web a dropped file has no path, so it is not opened from a drop there. While
/// the question about unsaved changes is open nothing opens. The status line says what was not opened.
pub fn take_dropped(app: &mut FontCraftApp, ctx: &egui::Context) {
    let files: Vec<egui::DroppedFileHandle> = ctx.input_mut(|i| {
        if !i.raw.dropped_files.iter().any(is_font) {
            return Vec::new();
        }
        let (ours, rest) = std::mem::take(&mut i.raw.dropped_files).into_iter().partition(is_font);
        i.raw.dropped_files = rest;
        ours
    });
    if files.is_empty() {
        return;
    }
    if app.closing.is_open() {
        app.status = format!("Dropped font files not opened ({}): answer the question about unsaved changes first", files.len());
        return;
    }
    for file in files.iter().take(MAX_OPENED) {
        let path = file.path();
        if path.is_absolute() {
            app.sent.push(Msg::run("file.open", json!({ "path": path.to_string_lossy() })));
        }
    }
    if files.len() > MAX_OPENED {
        app.status = format!("opened the first {MAX_OPENED} of {} dropped font files", files.len());
    }
}
