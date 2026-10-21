//! File › Open… and File › Open Folder… with the system chooser, and font files dropped on the
//! window. Rows: `file.open`, `file.openFolder`, `file.openTwiceActivates`.
//!
//! File › Open… without a `path` asks the chooser for files of every format `file.open` reads
//! ([`OPEN_EXTENSIONS`]) and opens each chosen file. A font source kept as a folder (`.ufo`,
//! `.glyphspackage`, `.sfdir`) is a file on some platforms' choosers and a folder on others, so
//! File › Open Folder… asks for a folder instead. A file that is already open is shown, not opened
//! twice; a project file opens as a project (both decided by `file.open`).
//!
//! [`take_dropped`] runs before anything is drawn, beside the module and macro drop
//! (`extensions::dropped`): it removes the font files from the frame's dropped files and queues a
//! `file.open` for each, so the canvas and the Font View, which place dropped images, see only the
//! rest.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use egui::DroppedFileHandle;
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

/// The command File › Open… runs.
const OPEN: &str = "file.open";
/// The command File › Open Folder… runs.
const OPEN_FOLDER: &str = "file.openFolder";

pub const COMMANDS: &[UiCommand] = &[(
    OPEN_FOLDER,
    "Open Folder…",
    &["File"],
    "",
    "{path?: string} open a font source kept as a folder (a .ufo, .glyphspackage or .sfdir) as file.open does; a path that is not a folder is refused. From the menu, without a path, the system folder chooser asks for it → the reply of file.open",
)];

/// The extensions File › Open… offers: every format `file.open` reads. `gz` stands for the
/// gzip-compressed `.pcf.gz` and `.bdf.gz`, since a chooser filters by the last extension only.
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

/// Most files one chooser answer or one drop opens; the rest are left alone.
pub const MAX_OPENED: usize = 64;

/// Most failures the status bar lists after opening several files.
const MAX_REPORTED: usize = 4;

pub fn run(app: &mut FontcraftApp, _id: &str, args: &Args) -> Result<Value, String> {
    let path = args.string("path")?.ok_or_else(|| args.err("give `path`, the folder to open"))?;
    if !Path::new(path).is_dir() {
        return Err(args.err(format!("`{path}` is not a folder; file.open opens a file")));
    }
    app.run(OPEN, &json!({ "path": path }))
}

/// A run of `id` with `params` that the menu or a shortcut started and that asks a chooser here:
/// File › Open… or File › Open Folder… without a `path`. Returns whether it was handled.
pub fn from_menu(app: &mut FontcraftApp, id: &str, params: &Value) -> bool {
    if params.get("path").is_some_and(|p| !p.is_null()) {
        return false;
    }
    let open: fn(&mut FontcraftApp) = match id {
        OPEN => open_files,
        OPEN_FOLDER => open_folder,
        _ => return false,
    };
    // While a dialog is open only view commands run.
    if app.dialog.is_none() {
        open(app);
    }
    true
}

/// File › Open…: asks for font files and opens each chosen one. A cancelled chooser opens nothing.
pub fn open_files(app: &mut FontcraftApp) {
    let Some(picker) = app.picker.as_mut() else {
        app.state.status = "Open: this window has no file chooser; run file.open with `path`".into();
        return;
    };
    let files = picker.files("Open", "Fonts", OPEN_EXTENSIONS);
    open_each(app, &files);
}

/// File › Open Folder…: asks for a folder and opens it. A cancelled chooser opens nothing.
pub fn open_folder(app: &mut FontcraftApp) {
    let Some(picker) = app.picker.as_mut() else {
        app.state.status = "Open Folder: this window has no folder chooser; run file.openFolder with `path`".into();
        return;
    };
    let Some(folder) = picker.folder("Open Folder") else { return };
    open_each(app, &[folder]);
}

/// Opens each of `paths` as the person's run of `file.open`. When several are opened and some
/// fail, the status bar lists the failures, not the last success.
fn open_each(app: &mut FontcraftApp, paths: &[PathBuf]) {
    let mut failed = Vec::new();
    for path in paths.iter().take(MAX_OPENED) {
        if let Err(e) = app.run_from_shell(OPEN, &json!({ "path": path.to_string_lossy() })) {
            failed.push(e);
        }
    }
    if paths.len() > MAX_OPENED {
        failed.push(format!("opened the first {MAX_OPENED} of {} files", paths.len()));
    }
    if paths.len() > 1 && !failed.is_empty() {
        let more = failed.len().saturating_sub(MAX_REPORTED);
        let mut status = failed.iter().take(MAX_REPORTED).cloned().collect::<Vec<_>>().join("; ");
        if more > 0 {
            let _ = write!(status, "; and {more} more");
        }
        app.state.status = status;
    }
}

/// Whether a file named `name` is one a drop opens: a name with an extension of
/// [`OPEN_EXTENSIONS`] in any case, a `.sfdir` folder, or a TeX bitmap font named by its
/// resolution (`cmr10.600pk`). SVG files are left to the image drop, which places them on a glyph;
/// a gzip file opens only as a `.pcf.gz` or `.bdf.gz`.
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

fn is_font(file: &DroppedFileHandle) -> bool {
    file.path().file_name().and_then(|n| n.to_str()).is_some_and(opens_on_drop)
}

/// Takes the dropped font files out of this frame's input and queues a `file.open` for each one
/// that has a path on disk, at most [`MAX_OPENED`]. On the web a dropped file has no path, only a
/// name, and its bytes arrive asynchronously, so it is not opened from a drop there. While a
/// dialog or the question about unsaved changes is open nothing opens. The status bar says what
/// was not opened.
pub fn take_dropped(app: &mut FontcraftApp, ctx: &egui::Context, pending: &mut Vec<(&'static str, Value)>) {
    let files: Vec<DroppedFileHandle> = ctx.input_mut(|i| {
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
    if app.dialog.is_some() || app.close_prompt.is_open() {
        app.state.status = format!("Dropped font files not opened ({}): close the open dialog or question first", files.len());
        return;
    }
    let mut no_path = 0usize;
    for file in files.iter().take(MAX_OPENED) {
        let path = file.path();
        if path.is_absolute() {
            pending.push((OPEN, json!({ "path": path.to_string_lossy() })));
        } else {
            no_path += 1;
        }
    }
    let mut notes = Vec::new();
    if no_path > 0 {
        notes.push(format!("dropped font files with no path on disk not opened ({no_path})"));
    }
    if files.len() > MAX_OPENED {
        notes.push(format!("opened the first {MAX_OPENED} of {} dropped font files", files.len()));
    }
    if !notes.is_empty() {
        app.state.status = notes.join("; ");
    }
}

#[cfg(test)]
#[path = "open_chooser_tests.rs"]
mod tests;
