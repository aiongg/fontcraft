//! Commands whose path parameter comes from a file picker.
//!
//! A command in [`PICKED`] that arrives without its path asks the host's file service instead of running. The
//! answer comes back on a later frame ([`Picks::poll`]) and posts the command again with the path filled.

use craft::host::Host;
use craft::host::{PickId, PickRequest, Picked};
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

/// What a picker asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// An existing document.
    Document,
    /// A font source a project links.
    FontSource,
    /// A folder to export into.
    Folder,
    /// Where to save a project.
    SaveProject,
}

/// The file kinds a project can link.
pub const FONT_EXTENSIONS: [&str; 5] = ["glyphs", "glyphspackage", "ufo", "designspace", "fontcraft"];
/// The suffix a project is saved with.
pub const PROJECT_EXTENSION: &str = "fontcraftproject";

/// `(command, parameter, ask)`: the commands the app completes with a picker, the parameter the path goes into, and
/// what the picker asks for.
pub const PICKED: &[(&str, &str, Ask)] = &[
    ("file.open", "path", Ask::Document),
    ("project.linkFont", "font", Ask::FontSource),
    ("project.setExportDestination", "folder", Ask::Folder),
    ("project.save", "path", Ask::SaveProject),
];

/// The picker request for `ask`.
fn request(ask: Ask) -> PickRequest {
    match ask {
        Ask::Document => PickRequest::open("document"),
        Ask::FontSource => PickRequest::open("document").filter("Font sources", &FONT_EXTENSIONS),
        Ask::Folder => PickRequest::folder("export"),
        Ask::SaveProject => PickRequest::save("project").filter("Project", &[PROJECT_EXTENSION]).suggested_extension(PROJECT_EXTENSION),
    }
}

/// The pickers waiting for an answer.
#[derive(Default)]
pub struct Picks {
    waiting: Vec<(PickId, CommandRef, &'static str)>,
}

impl Picks {
    /// Asks for the path `command` lacks and returns true, or returns false when it lacks none.
    pub fn ask(&mut self, host: &Host, command: &CommandRef) -> bool {
        let Some(&(_, param, ask)) = PICKED.iter().find(|(id, param, _)| *id == command.id && command.params.get(*param).is_none()) else {
            return false;
        };
        let id = host.files().pick(request(ask));
        self.waiting.push((id, command.clone(), param));
        true
    }

    /// The commands whose picker answered with a path, ready to run; cancelled pickers are dropped. A picker
    /// that could not be shown ends with a message in `failed`.
    pub fn poll(&mut self, host: &Host, failed: &mut Vec<String>) -> Vec<CommandRef> {
        let mut ready = Vec::new();
        self.waiting.retain(|(id, command, param)| {
            let Some(answer) = host.files().take_answer(*id) else { return true };
            match answer {
                Picked::Paths(paths) => {
                    if let Some(path) = paths.first() {
                        let mut params = match &command.params {
                            Value::Object(map) => map.clone(),
                            _ => serde_json::Map::new(),
                        };
                        params.insert((*param).into(), json!(path.to_string_lossy()));
                        ready.push(CommandRef::with_params(command.id.clone(), Value::Object(params)));
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
