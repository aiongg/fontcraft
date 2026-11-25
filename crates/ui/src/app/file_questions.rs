//! The questions a person's file commands ask before they run. Rows: `roundtrip.lossReport`, `ttc.open`.
//!
//! - Save, Save As and Export UFO, when the format cannot hold part of the document that its file does not already
//!   leave out (the `fresh` entries of `file.lossReport`): Save Anyway or Cancel, and for a UFO format whether smart
//!   components are drawn as outlines. Nothing is written before the answer. A command that gives `strict` has its
//!   answer and runs.
//! - File › Open… of a font collection of several fonts: which members to open, each as its own document. A command
//!   that gives `face` opens that one.
//!
//! Only what the person does in the window asks ([`ask`]). A request that names the command (`engine.execute`, MCP)
//! runs as given: its reply lists the `losses`, and a collection without `face` is an error. `window.fileQuestion`
//! answers the question, and `ui.inspect` reports it. `dialogs/loss_question.rs` and `dialogs/collection_members.rs`
//! show it. Questions wait in order; the first one shows.

use std::path::Path;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params;

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "window.fileQuestion",
    label: "File Question",
    menu: &[],
    shortcut: None,
    params: "{answer?: \"proceed\" | \"cancel\", faces?: [number], smartComponents?: \"components\" | \"outlines\"} the question a person's Save, Save As, Export UFO or File › Open… of a font collection asked before running. With answer, answers it as its buttons do: proceed saves although the format leaves data out (smartComponents draws smart components as outlines in a UFO, or keeps them), or opens the collection members faces lists, by index; cancel does nothing. Refused when no question is open. ui.inspect reports it as fileQuestion: {open, kind: \"losses\" | \"collection\" | null, command, document?, format?, losses?: [{what, entries, details}], smartComponents?: number, file?, members?: [{index, family, style, postscriptName, glyphs}], waiting: number}",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most questions that wait at a time; a command past it runs as given.
pub const MAX_WAITING: usize = 64;
/// Most members of a collection the question lists.
pub const MAX_MEMBERS: usize = 2_000;
/// Most kinds of loss the question lists.
pub const MAX_LOSSES: usize = 500;
/// Most glyphs, items or files one kind names in the question.
const MAX_DETAILS: usize = 12;

/// The format names of the save commands that are UFOs, whose smart components can be drawn as outlines.
const UFO_FORMATS: [&str; 3] = ["ufo", "ufoz", "designspace"];

/// One kind of data the save leaves out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    pub what: String,
    pub entries: u64,
    /// The glyphs, items or files, at most [`MAX_DETAILS`] and then an ellipsis.
    pub details: String,
}

/// The question before a save that leaves data out.
#[derive(Clone, Debug, PartialEq)]
pub struct LossQuestion {
    command: CommandRef,
    doc: u64,
    pub title: String,
    /// The format's name as the save commands give it.
    pub format: String,
    pub losses: Vec<Loss>,
    /// How many smart components the person can choose to draw as outlines; 0 offers no choice.
    pub smart: u64,
    /// The choice the command came with.
    pub outlines: bool,
}

impl LossQuestion {
    /// Whether it asks about an export, not a save.
    pub fn export(&self) -> bool {
        self.command.id == "file.exportUfo"
    }
}

/// A member of a font collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub index: u32,
    pub family: String,
    pub style: String,
    pub postscript: String,
    pub glyphs: u64,
}

/// The question which members of a collection to open.
#[derive(Clone, Debug, PartialEq)]
pub struct MembersQuestion {
    command: CommandRef,
    /// The collection's file name.
    pub file: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    Losses(LossQuestion),
    Members(MembersQuestion),
}

/// The questions waiting for an answer, kept by the app between frames.
#[derive(Clone, Debug, Default)]
pub struct FileQuestions {
    waiting: Vec<(u64, Question)>,
    last: u64,
}

impl FileQuestions {
    /// The question that shows, with the number that tells it from the next one.
    pub fn first(&self) -> Option<&(u64, Question)> {
        self.waiting.first()
    }

    /// Whether a save of document `doc` waits for its answer.
    pub fn saving(&self, doc: u64) -> bool {
        self.waiting.iter().any(|(_, q)| matches!(q, Question::Losses(l) if l.doc == doc && !l.export()))
    }

    fn push(&mut self, question: Question) {
        self.last = self.last.saturating_add(1);
        self.waiting.push((self.last, question));
    }

    /// Drops the questions about documents that were closed meanwhile.
    pub fn prune(&mut self, session: &Session) {
        self.waiting.retain(|(_, q)| match q {
            Question::Losses(l) => session.documents().iter().any(|d| d.id == l.doc),
            Question::Members(_) => true,
        });
    }
}

/// The question the person's `command` asks, kept for its answer; `None` runs the command as given.
pub fn ask(app: &mut FontCraftApp, command: &CommandRef) -> Option<Value> {
    if app.view.file_questions.waiting.len() >= MAX_WAITING {
        return None;
    }
    let (kind, question) = match command.id.as_str() {
        "file.open" => ("collection", Question::Members(members(app, command)?)),
        "file.save" | "file.saveAs" | "file.exportUfo" => ("losses", Question::Losses(losses(app, command)?)),
        _ => return None,
    };
    app.view.file_questions.push(question);
    app.view_changed();
    Some(json!({ "question": kind }))
}

fn given<'a>(command: &'a CommandRef, key: &str) -> Option<&'a Value> {
    command.params.get(key).filter(|v| !v.is_null())
}

/// The members of the collection a `file.open` names, when it names no `face` and there are several.
fn members(app: &mut FontCraftApp, command: &CommandRef) -> Option<MembersQuestion> {
    if given(command, "face").is_some() {
        return None;
    }
    let path = given(command, "path")?.as_str()?;
    if !fontcraft_engine::cmd::collection::is_collection_file(Path::new(path)) {
        return None;
    }
    // A collection that cannot be listed goes to `file.open`, which says why it cannot be opened.
    let reply = app.session.execute("collection.list", &json!({ "file": path })).ok()?;
    let text = |m: &Value, key: &str| m.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let members: Vec<Member> = reply
        .get("members")?
        .as_array()?
        .iter()
        .take(MAX_MEMBERS)
        .filter_map(|m| {
            let index = u32::try_from(m.get("index")?.as_u64()?).ok()?;
            let glyphs = m.get("glyphs").and_then(Value::as_u64).unwrap_or(0);
            Some(Member { index, family: text(m, "family"), style: text(m, "style"), postscript: text(m, "postscriptName"), glyphs })
        })
        .collect();
    if members.len() < 2 {
        return None;
    }
    let file = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Some(MembersQuestion { command: command.clone(), file, members })
}

/// What the save or export `command` would leave out that the document's file does not already.
fn losses(app: &mut FontCraftApp, command: &CommandRef) -> Option<LossQuestion> {
    if given(command, "strict").is_some() {
        return None;
    }
    let doc = app.session.active()?;
    if given(command, "document").is_some_and(|d| d.as_u64() != Some(doc.id)) {
        return None;
    }
    let (id, title) = (doc.id, doc.title.clone());
    let mut asked = Map::new();
    for key in ["version", "smartComponents", "decomposeSmartStuff"] {
        if let Some(value) = given(command, key) {
            asked.insert(key.into(), value.clone());
        }
    }
    let export = command.id == "file.exportUfo";
    if export {
        asked.insert("exportUfo".into(), json!(true));
    } else if let Some(format) = given(command, "format") {
        asked.insert("format".into(), format.clone());
    } else if let Some(path) = given(command, "path").and_then(Value::as_str) {
        // A name without a source extension is saved in the native format, or refused.
        asked.insert("format".into(), json!(super::files::format_of(path)?));
    } else if super::files::needs_save_path(doc) {
        return None;
    }
    // A request the report refuses is refused by the command, which says why.
    let report = app.session.execute("file.lossReport", &Value::Object(asked)).ok()?;
    let fresh = report.get("fresh")?.as_array()?;
    if fresh.is_empty() {
        return None;
    }
    let format = report.get("format").and_then(Value::as_str).unwrap_or_default().to_string();
    let choice = !export && UFO_FORMATS.contains(&format.as_str());
    Some(LossQuestion {
        doc: id,
        title,
        losses: fresh.iter().take(MAX_LOSSES).map(loss).collect(),
        smart: if choice { report.get("smartComponents").and_then(Value::as_u64).unwrap_or(0) } else { 0 },
        outlines: given(command, "smartComponents").and_then(Value::as_str) == Some("outlines"),
        format,
        command: command.clone(),
    })
}

/// One entry of the loss report as the question shows it.
fn loss(entry: &Value) -> Loss {
    let list = ["items", "files", "glyphs"].iter().find_map(|key| entry.get(*key).and_then(Value::as_array).filter(|l| !l.is_empty()));
    let names: Vec<&str> = list.into_iter().flatten().filter_map(Value::as_str).collect();
    let mut details = names.iter().take(MAX_DETAILS).copied().collect::<Vec<_>>().join(", ");
    if names.len() > MAX_DETAILS {
        details.push_str(", …");
    }
    Loss {
        what: entry.get("what").and_then(Value::as_str).unwrap_or_default().to_string(),
        entries: entry.get("entries").and_then(Value::as_u64).unwrap_or(0),
        details,
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    app.view.file_questions.prune(&app.session);
    let proceed = match params::string(p, "answer")? {
        None => return Ok(()),
        Some("proceed") => true,
        Some("cancel") => false,
        Some(other) => return Err(format!("`answer` is proceed or cancel, not `{}`", params::quoted(other))),
    };
    if app.view.file_questions.waiting.is_empty() {
        return Err("window.fileQuestion: no question is open".into());
    }
    let smart = params::string(p, "smartComponents")?;
    let faces = match p.get("faces").filter(|v| !v.is_null()) {
        None => Vec::new(),
        Some(v) => {
            let list = v.as_array().ok_or("`faces` is a list of member indexes")?;
            let faces: Option<Vec<u32>> = list.iter().map(|f| f.as_u64().and_then(|f| u32::try_from(f).ok())).collect();
            faces.ok_or("`faces` is a list of member indexes")?
        }
    };
    if proceed && let Some((_, Question::Members(q))) = app.view.file_questions.first() {
        if faces.is_empty() {
            return Err("`faces` names the members to open".into());
        }
        if let Some(unknown) = faces.iter().find(|f| !q.members.iter().any(|m| m.index == **f)) {
            return Err(format!("the collection has no font number {unknown}"));
        }
    }
    let (_, question) = app.view.file_questions.waiting.remove(0);
    app.view_changed();
    if !proceed {
        app.status = match question {
            Question::Losses(_) => "Nothing was written.".into(),
            Question::Members(q) => format!("{} was not opened.", q.file),
        };
        return Ok(());
    }
    match question {
        Question::Losses(q) => save(app, q, smart),
        Question::Members(q) => open_members(app, &q, &faces),
    }
}

/// The command's parameters as a map.
fn params_of(command: &CommandRef) -> Map<String, Value> {
    command.params.as_object().cloned().unwrap_or_default()
}

/// Save Anyway: the command as it came, with the answer, on the document it was asked about.
fn save(app: &mut FontCraftApp, q: LossQuestion, smart: Option<&str>) -> Result<(), String> {
    super::close::activate(app, q.doc)?;
    let mut given = params_of(&q.command);
    given.insert("strict".into(), json!(false));
    if q.smart > 0
        && let Some(smart) = smart
    {
        given.insert("smartComponents".into(), json!(smart));
    }
    app.apply_msg(Msg::run(&q.command.id, Value::Object(given)))
}

/// Opens each member chosen as its own document, at most [`super::files::MAX_OPENED`]. One that cannot be opened does
/// not keep the others from opening; the last failure is reported.
fn open_members(app: &mut FontCraftApp, q: &MembersQuestion, faces: &[u32]) -> Result<(), String> {
    let mut failed = None;
    let mut seen = Vec::new();
    for face in faces {
        if seen.contains(face) || seen.len() >= super::files::MAX_OPENED {
            continue;
        }
        seen.push(*face);
        let mut given = params_of(&q.command);
        given.insert("face".into(), json!(face));
        if let Err(e) = app.apply_msg(Msg::run("file.open", Value::Object(given))) {
            failed = Some(e);
        }
    }
    failed.map_or(Ok(()), Err)
}

/// `ui.inspect`'s `fileQuestion`.
pub fn inspect(app: &FontCraftApp, out: &mut Map<String, Value>) {
    out.insert("fileQuestion".into(), state(app));
}

/// The question as `ui.inspect` reports it.
pub fn state(app: &FontCraftApp) -> Value {
    let waiting = app.view.file_questions.waiting.len();
    match app.view.file_questions.first() {
        None => json!({ "open": false, "kind": null, "waiting": waiting }),
        Some((_, Question::Losses(q))) => {
            let losses: Vec<Value> = q.losses.iter().map(|l| json!({ "what": l.what, "entries": l.entries, "details": l.details })).collect();
            json!({
                "open": true, "kind": "losses", "command": q.command.id, "document": q.title, "format": q.format, "losses": losses,
                "smartComponents": q.smart, "waiting": waiting,
            })
        }
        Some((_, Question::Members(q))) => {
            let members: Vec<Value> = q
                .members
                .iter()
                .map(|m| json!({ "index": m.index, "family": m.family, "style": m.style, "postscriptName": m.postscript, "glyphs": m.glyphs }))
                .collect();
            json!({ "open": true, "kind": "collection", "command": q.command.id, "file": q.file, "members": members, "waiting": waiting })
        }
    }
}

#[cfg(test)]
#[path = "file_questions_tests.rs"]
mod tests;
