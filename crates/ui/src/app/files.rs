//! Commands whose path parameter comes from a file picker.
//!
//! A command in [`PICKED`] that arrives without its path asks the host's file service instead of running. The
//! answer comes back on a later frame ([`Picks::poll`]) and posts the command again with the path filled: once per
//! chosen file for File › Open…, once with every chosen file for File › Import › Outlines….
//!
//! | Command | Picker |
//! |---|---|
//! | `file.open` | several files of every format `file.open` reads ([`super::open::OPEN_EXTENSIONS`]) |
//! | `file.openFolder` | a folder (a `.ufo`, `.glyphspackage` or `.sfdir` source) |
//! | `file.saveAs` | a file to write, offered as the document's title with its format's extension |
//! | `file.save` | as Save As, for a document without a file it can write back to (never saved, or a compiled font) |
//! | `file.importOutlines` | several SVG or EPS files, into `files` |
//! | `project.save` | a `.fontcraftproject` file, for a project never saved |
//! | `project.linkFont` | a font source for the project to link, into `font` |
//! | `project.setExportDestination` | a folder to export into, into `folder` |
//! | `tableViewer.openFont`, `tableViewer.compareWithFile` | a compiled font for the Table Viewer ([`COMPILED_EXTENSIONS`]) |
//! | `plugin.installFromFile` | a `.wasm` module or `.fcmacro` macro, unless `dataBase64` or `name` names the file |
//! | `view.filterByEncodingFile` | an encoding file, unless `name` names a built-in encoding |
//! | `macroPanel.open`, `macroPanel.save` | a `.fcmacro` file to read into the Macro Panel's editor, or to write it to |
//!
//! Engine and shell commands alike: a shell command of the table asks before it runs, as an engine command does.
//!
//! A chosen save name without an extension gets the offered one (the file service's extension policy).

use craft::host::{Host, PickId, PickRequest, Picked};
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::{DocState, Session};
use fontcraft_model::SourceFormat;
use serde_json::{Value, json};

/// `(command, parameter)`: the commands the app completes with a picker, and the parameter the path goes into.
pub const PICKED: &[(&str, &str)] = &[
    ("file.open", "path"),
    ("file.openFolder", "path"),
    ("file.saveAs", "path"),
    ("file.save", "path"),
    ("file.importOutlines", "files"),
    ("project.save", "path"),
    ("project.linkFont", "font"),
    ("project.setExportDestination", "folder"),
    ("tableViewer.openFont", "path"),
    ("tableViewer.compareWithFile", "path"),
    ("plugin.installFromFile", "path"),
    ("view.filterByEncodingFile", "path"),
    ("macroPanel.open", "path"),
    ("macroPanel.save", "path"),
];

/// Most files one picker answer opens; the rest are left alone.
pub const MAX_OPENED: usize = 64;

/// The formats the save commands write (the `format` names of `file.saveAs`).
const SAVE_EXTENSIONS: [&str; 6] = ["fontcraft", "glyphs", "glyphspackage", "ufo", "ufoz", "designspace"];
/// A new document's format, and the format a document Save cannot write back to is saved in.
const NATIVE_EXTENSION: &str = "fontcraft";
/// A project file's extension.
const PROJECT_EXTENSION: &str = "fontcraftproject";
/// The compiled fonts the Table Viewer reads.
pub const COMPILED_EXTENSIONS: [&str; 6] = ["ttf", "otf", "ttc", "otc", "woff", "woff2"];
/// The file kinds `plugin.installFromFile` installs: modules and macros.
const EXTENSION_FILES: [&str; 2] = ["wasm", "fcmacro"];
/// The file kinds a project can link.
pub const FONT_EXTENSIONS: [&str; 5] = ["glyphs", "glyphspackage", "ufo", "designspace", "fontcraft"];

/// The extension of the format a document read from `source` is saved in; `None` for a source Save cannot write back
/// to (a compiled font).
fn save_extension(source: SourceFormat) -> Option<&'static str> {
    match source {
        SourceFormat::Native => Some(NATIVE_EXTENSION),
        SourceFormat::Glyphs => Some("glyphs"),
        SourceFormat::GlyphsPackage => Some("glyphspackage"),
        SourceFormat::Ufo => Some("ufo"),
        SourceFormat::Designspace => Some("designspace"),
        SourceFormat::Ufoz => Some("ufoz"),
        SourceFormat::Binary => None,
    }
}

/// Whether saving `doc` needs a file chosen first: it was never saved, or Save cannot write its format back.
pub fn needs_save_path(doc: &DocState) -> bool {
    doc.path.is_none() || save_extension(doc.font.source.format).is_none()
}

/// How a picker's answer fills the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// The first path goes into the parameter.
    One,
    /// Each path runs the command once.
    Each,
    /// Every path goes into the parameter as a list.
    All,
}

/// The picker `command` needs, or `None` when it runs as given.
fn request(session: &Session, command: &CommandRef) -> Option<(PickRequest, &'static str, Fill)> {
    let &(_, param) = PICKED.iter().find(|(id, param)| *id == command.id && command.params.get(*param).is_none_or(Value::is_null))?;
    let save_as = |session: &Session, title: &str| {
        let doc = session.active()?;
        let extension = save_extension(doc.font.source.format).unwrap_or(NATIVE_EXTENSION);
        let request = PickRequest::save("document")
            .title(title)
            .filter("Font source", &SAVE_EXTENSIONS)
            .file_name(format!("{}.{extension}", doc.title))
            .suggested_extension(extension);
        Some(request)
    };
    let request = match command.id.as_str() {
        "file.open" => (PickRequest::open_many("document").title("Open").filter("Fonts", super::open::OPEN_EXTENSIONS), Fill::Each),
        "file.openFolder" => (PickRequest::folder("document").title("Open Folder"), Fill::One),
        "file.saveAs" => (save_as(session, "Save As")?, Fill::One),
        "file.save" => {
            if !session.active().is_some_and(needs_save_path) {
                return None;
            }
            (save_as(session, "Save")?, Fill::One)
        }
        "file.importOutlines" => (PickRequest::open_many("import").title("Import Outlines").filter("SVG or EPS", &["svg", "eps"]), Fill::All),
        "project.save" => {
            let named = command.params.get("project").and_then(Value::as_u64);
            let project = match named {
                Some(id) => session.projects().iter().find(|p| p.id == id)?,
                None => session.projects().last()?,
            };
            if project.path.is_some() {
                return None;
            }
            let request = PickRequest::save("document")
                .title("Save Project")
                .filter("Project", &[PROJECT_EXTENSION])
                .file_name(format!("{}.{PROJECT_EXTENSION}", project.title))
                .suggested_extension(PROJECT_EXTENSION);
            (request, Fill::One)
        }
        "project.linkFont" => (PickRequest::open("document").filter("Font sources", &FONT_EXTENSIONS), Fill::One),
        "project.setExportDestination" => (PickRequest::folder("export"), Fill::One),
        "tableViewer.openFont" => (PickRequest::open("document").title("Open Font").filter("Fonts", &COMPILED_EXTENSIONS), Fill::One),
        "tableViewer.compareWithFile" => (PickRequest::open("document").title("Compare With").filter("Fonts", &COMPILED_EXTENSIONS), Fill::One),
        "plugin.installFromFile" => {
            if ["dataBase64", "name"].iter().any(|k| command.params.get(*k).is_some_and(|v| !v.is_null())) {
                return None;
            }
            (PickRequest::open("extension").title("Install from File").filter("Modules and macros", &EXTENSION_FILES), Fill::One)
        }
        "view.filterByEncodingFile" => {
            if command.params.get("name").is_some_and(|v| !v.is_null()) {
                return None;
            }
            (PickRequest::open("encoding").title("Filter by Encoding File"), Fill::One)
        }
        "macroPanel.open" => {
            let filter = [crate::shell::macro_editor::EXTENSION];
            (PickRequest::open("macro").title("Open Macro").filter("Macros", &filter), Fill::One)
        }
        "macroPanel.save" => {
            let extension = crate::shell::macro_editor::EXTENSION;
            let request = PickRequest::save("macro")
                .title("Save Macro")
                .filter("Macros", &[extension])
                .file_name(format!("Macro.{extension}"))
                .suggested_extension(extension);
            (request, Fill::One)
        }
        _ => return None,
    };
    Some((request.0, param, request.1))
}

/// The pickers waiting for an answer.
#[derive(Default)]
pub struct Picks {
    waiting: Vec<(PickId, CommandRef, &'static str, Fill)>,
}

impl Picks {
    /// Asks for the path `command` lacks and returns true, or returns false when it runs as given.
    pub fn ask(&mut self, host: &Host, session: &Session, command: &CommandRef) -> bool {
        let Some((request, param, fill)) = request(session, command) else { return false };
        let id = host.files().pick(request);
        self.waiting.push((id, command.clone(), param, fill));
        true
    }

    /// Whether a picker is still waiting for its answer.
    pub fn waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// The commands whose picker answered with a path, ready to run; cancelled pickers are dropped. A picker
    /// that could not be shown ends with a message in `failed`.
    pub fn poll(&mut self, host: &Host, failed: &mut Vec<String>) -> Vec<CommandRef> {
        let mut ready = Vec::new();
        self.waiting.retain(|(id, command, param, fill)| {
            let Some(answer) = host.files().take_answer(*id) else { return true };
            match answer {
                Picked::Paths(paths) => {
                    let with = |value: Value| {
                        let mut params = match &command.params {
                            Value::Object(map) => map.clone(),
                            _ => serde_json::Map::new(),
                        };
                        params.insert((*param).into(), value);
                        CommandRef::with_params(command.id.clone(), Value::Object(params))
                    };
                    let paths: Vec<String> = paths.iter().take(MAX_OPENED).map(|p| p.to_string_lossy().into_owned()).collect();
                    match fill {
                        Fill::One => ready.extend(paths.first().map(|p| with(json!(p)))),
                        Fill::Each => ready.extend(paths.iter().map(|p| with(json!(p)))),
                        Fill::All if !paths.is_empty() => ready.push(with(json!(paths))),
                        Fill::All => {}
                    }
                }
                Picked::Cancelled => {}
                other => failed.push(format!("{}: no file picker ({other:?})", command.id)),
            }
            false
        });
        ready
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::SourceFormat;

    use super::{SAVE_EXTENSIONS, save_extension};

    /// A UFOZ document saves back to a UFOZ, and the save chooser offers the format (M1.11).
    #[test]
    fn ufoz_is_a_save_format() {
        assert_eq!(save_extension(SourceFormat::Ufoz), Some("ufoz"));
        assert!(SAVE_EXTENSIONS.contains(&"ufoz"));
        assert_eq!(save_extension(SourceFormat::Binary), None);
    }
}
