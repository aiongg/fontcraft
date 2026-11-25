//! The question asked before unsaved work is closed: the window's close box, quitting (`app.quit`, Cmd+Q) and File ›
//! Close on a modified document.
//!
//! It asks about one document at a time and shows it: Save, Don't Save or Cancel. Save and Don't Save move to the
//! next document with unsaved changes; "Apply to All" gives the same answer to each one left. Once every one is
//! answered the window closes, or for File › Close the document does. Cancel, a cancelled save picker and a failed
//! save stop there and keep everything open. Work changed while the question is open (by the control channel) is
//! asked about before the window closes.
//!
//! What the person fires in the window (a menu item, a shortcut) opens the question ([`fired`]). Run as commands,
//! `app.quit` and `file.close` refuse unsaved work unless `discard` is true. `window.closePrompt` reports and answers
//! the question, so the control channel can drive it. `dialogs/unsaved.rs` shows it.

use craft::ui::runtime::commands::CommandRef;
use craft::ui::shell::embed::CloseDecision;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params;

/// The documents an error message names before it says how many more there are.
const NAMED_IN_ERRORS: usize = 5;

pub const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "app.quit",
        label: "Quit FontCraft",
        menu: &["File"],
        shortcut: Some("Cmd+Q"),
        params: "{discard?: bool} close the window and quit. Refused while a document or project has unsaved changes, unless discard is true; in the window the menu item and Cmd+Q ask about each one instead (window.closePrompt)",
    },
    UiCommand {
        id: "window.closePrompt",
        label: "Unsaved Changes",
        menu: &[],
        shortcut: None,
        params: "{answer?: \"save\" | \"dontSave\" | \"cancel\", strict?: bool} the question asked before unsaved work is closed (the window's close box, Quit, File › Close on a modified document). With answer, answers it as its buttons do: save writes the document asked about (the question's own Save button opens the save picker for one without a file; a save answer from anywhere else is refused for it: give it a file with file.saveAs or project.save and a path, then answer again; the question's own Save asks again when the format leaves data out (window.fileQuestion), while a save answer from anywhere else saves without asking, or with strict: true fails instead of leaving data out), dontSave leaves its changes unsaved, and both move to the next document with unsaved changes; once none is left the window closes (File › Close: the document closes). cancel stops closing. A failed save also stops closing. ui.inspect reports the question as closePrompt: {open, purpose: \"close\" | \"quit\" | null, document, kind: \"font\" | \"project\" | null, remaining: [titles]}",
    },
];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// A document the question asks about, by its session id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Font(u64),
    Project(u64),
}

/// What happens once every document is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// File › Close: the document closes.
    Close,
    /// The window closes and the app quits.
    Quit,
}

/// An answer to the question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    DontSave,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Asking {
    purpose: Purpose,
    /// The documents still to ask about; the first is the one asked about now.
    queue: Vec<Item>,
}

/// The question's state, kept by the app between frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosePrompt {
    asking: Option<Asking>,
    /// Every document was answered, or `app.quit` was told to discard: the next close request goes through, unless
    /// something outside `discarded` has unsaved changes by then.
    approved: bool,
    /// The documents answered Don't Save before quitting, or left unsaved by `app.quit` with `discard`.
    discarded: Vec<Item>,
    /// The next answer comes from the question's own buttons, so its Save may open the save picker. An answer from
    /// anywhere else never does: a picker would hold a control request until someone answers it.
    pub from_window: bool,
    /// "Apply to All" was ticked: the same answer goes to each document left.
    pub apply_all: Option<Answer>,
    /// The document whose save waits for the save picker, or for the answer about the data its format leaves out.
    saving: Option<Item>,
    /// Ask the window to close in the next `logic`.
    close_window: bool,
}

impl ClosePrompt {
    /// Whether the question is showing (or waits for a save it started).
    pub fn is_open(&self) -> bool {
        self.asking.is_some()
    }

    /// Whether the question waits for the save picker or the save's own question; the dialog stays hidden meanwhile.
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }

    /// Whether the app decided to close the window and nothing holds it open.
    pub fn quitting(&self) -> bool {
        self.approved
    }

    /// The document asked about now, its title, and how many documents are left after it.
    pub fn current(&self, session: &Session) -> Option<(Item, String, usize)> {
        let asking = self.asking.as_ref()?;
        let first = *asking.queue.first()?;
        Some((first, title(session, first).unwrap_or_default(), asking.queue.len().saturating_sub(1)))
    }

    /// The purpose of the open question.
    pub fn purpose(&self) -> Option<Purpose> {
        self.asking.as_ref().map(|a| a.purpose)
    }

    /// Takes the request to close the window.
    pub fn take_close_window(&mut self) -> bool {
        std::mem::take(&mut self.close_window)
    }

    /// The window stays open: the next close asks about every document with unsaved changes.
    fn keep_window(&mut self) {
        self.approved = false;
        self.close_window = false;
        self.discarded.clear();
        self.apply_all = None;
        self.saving = None;
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "app.quit" => {
            let discard = params::flag(p, "discard")?.unwrap_or(false);
            let unsaved = unsaved(&app.session);
            if !discard && !unsaved.is_empty() {
                return Err(format!("{} unsaved changes: save first, or pass discard: true", named(&app.session, &unsaved)));
            }
            app.closing.discarded = unsaved;
            quit(app);
            Ok(())
        }
        "window.closePrompt" => match params::string(p, "answer")? {
            None => Ok(()),
            Some("save") => {
                let strict = params::flag(p, "strict")?.unwrap_or(false);
                answer(app, Answer::Save, strict)
            }
            Some("dontSave") => answer(app, Answer::DontSave, false),
            Some("cancel") => answer(app, Answer::Cancel, false),
            Some(other) => Err(format!("`answer` is save, dontSave or cancel, not `{other}`")),
        },
        other => Err(format!("unknown command `{other}`")),
    }
}

/// A command the person fired from a menu item or shortcut: `app.quit` or `file.close` with unsaved work to ask
/// about opens the question and returns true. Any other command, nothing unsaved or `discard: true` returns false
/// and runs as given.
pub fn fired(app: &mut FontCraftApp, command: &CommandRef) -> bool {
    if command.params.get("discard").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    match command.id.as_str() {
        "app.quit" => {
            let unsaved = unsaved(&app.session);
            if unsaved.is_empty() {
                return false;
            }
            start(app, Purpose::Quit, unsaved);
        }
        "file.close" => {
            let Some(doc) = app.session.active().filter(|d| d.modified()) else { return false };
            let item = Item::Font(doc.id);
            if app.closing.is_open() {
                app.status = "file.close: the question about unsaved changes is open: answer it first".into();
                return true;
            }
            start(app, Purpose::Close, vec![item]);
        }
        _ => return false,
    }
    true
}

/// The window was asked to close (close box, the system): with unsaved work the close is held and the question asks
/// about each document.
pub fn close_requested(app: &mut FontCraftApp) -> CloseDecision {
    let prompt = &app.closing;
    // Once every document is answered, only work changed since is asked about.
    let left: Vec<Item> = unsaved(&app.session).into_iter().filter(|i| !(prompt.approved && prompt.discarded.contains(i))).collect();
    if left.is_empty() {
        return CloseDecision::Close;
    }
    if app.closing.approved {
        app.closing.approved = false;
        app.closing.close_window = false;
        ask(app, Purpose::Quit, left);
    } else {
        start(app, Purpose::Quit, left);
    }
    CloseDecision::Hold
}

/// After every `drain`: a save the question started through the save picker, or that asked about data the format
/// leaves out, either wrote the document, and the
/// question moves on, or did not (cancelled picker or question, failed save), and closing stops.
pub fn after_drain(app: &mut FontCraftApp) {
    let Some(item) = app.closing.saving else {
        prune(app);
        return;
    };
    if is_unsaved(&app.session, item) {
        let asking = matches!(item, Item::Font(id) if app.view.file_questions.saving(id));
        if app.picks.waiting() || asking {
            return;
        }
        let name = title(&app.session, item).unwrap_or_default();
        cancel(app);
        app.status = format!("`{name}` was not saved, so nothing was closed");
        return;
    }
    app.closing.saving = None;
    if let Err(e) = advance(app, item, Answer::Save) {
        app.status = e;
    }
}

/// Whether a document or project has unsaved changes.
pub fn has_unsaved(session: &Session) -> bool {
    !unsaved(session).is_empty()
}

/// The open documents, then the open projects, with unsaved changes.
fn unsaved(session: &Session) -> Vec<Item> {
    let fonts = session.documents().iter().filter(|d| d.modified()).map(|d| Item::Font(d.id));
    let projects = session.projects().iter().filter(|p| p.modified()).map(|p| Item::Project(p.id));
    fonts.chain(projects).collect()
}

fn is_unsaved(session: &Session, item: Item) -> bool {
    match item {
        Item::Font(id) => session.documents().iter().any(|d| d.id == id && d.modified()),
        Item::Project(id) => session.projects().iter().any(|p| p.id == id && p.modified()),
    }
}

fn title(session: &Session, item: Item) -> Option<String> {
    match item {
        Item::Font(id) => session.documents().iter().find(|d| d.id == id).map(|d| d.title.clone()),
        Item::Project(id) => session.projects().iter().find(|p| p.id == id).map(|p| p.title.clone()),
    }
}

/// "`A` has" or "`A`, `B` and 3 more have", for an error message.
fn named(session: &Session, items: &[Item]) -> String {
    let titles: Vec<String> = items.iter().take(NAMED_IN_ERRORS).filter_map(|i| title(session, *i)).map(|t| format!("`{t}`")).collect();
    let rest = items.len().saturating_sub(titles.len());
    let list = match (titles.split_last(), rest) {
        (None, _) => "documents".to_string(),
        (Some((last, [])), 0) => last.clone(),
        (Some((last, init)), 0) => format!("{} and {last}", init.join(", ")),
        (Some(_), n) => format!("{} and {n} more", titles.join(", ")),
    };
    let verb = if items.len() == 1 { "has" } else { "have" };
    format!("{list} {verb}")
}

/// Opens the question for `purpose` about `queue`, unless it is already asking before quitting.
fn start(app: &mut FontCraftApp, purpose: Purpose, queue: Vec<Item>) {
    if purpose == Purpose::Quit && app.closing.purpose() == Some(Purpose::Quit) {
        return;
    }
    // A new question: earlier answers and a quit decided before it no longer count.
    if !app.closing.is_open() {
        app.closing.keep_window();
    }
    ask(app, purpose, queue);
}

fn ask(app: &mut FontCraftApp, purpose: Purpose, queue: Vec<Item>) {
    app.closing.asking = Some(Asking { purpose, queue });
    show_first(app);
}

/// Makes the document asked about the active one, so the person sees it.
fn show_first(app: &mut FontCraftApp) {
    let Some((Item::Font(id), _, _)) = app.closing.current(&app.session) else { return };
    // A failure goes to the status line; the question still asks.
    if let Err(e) = activate(app, id) {
        app.status = e;
    }
}

/// Makes the document `id` the active one.
pub(super) fn activate(app: &mut FontCraftApp, id: u64) -> Result<(), String> {
    if app.session.active().is_some_and(|d| d.id == id) {
        return Ok(());
    }
    let index = app.session.documents().iter().position(|d| d.id == id).ok_or("the document is no longer open")?;
    app.apply_msg(Msg::run("document.activate", json!({ "index": index })))
}

/// Drops the documents that were closed or saved meanwhile; with none left the question is done.
fn prune(app: &mut FontCraftApp) {
    let Some(asking) = app.closing.asking.as_mut() else { return };
    let before = asking.queue.first().copied();
    asking.queue.retain(|i| is_unsaved(&app.session, *i));
    if asking.queue.is_empty() {
        let purpose = asking.purpose;
        app.closing.asking = None;
        if purpose == Purpose::Quit {
            quit_or_ask(app);
        }
    } else if asking.queue.first().copied() != before {
        show_first(app);
    }
}

/// Every document asked about before quitting is answered: the window closes, unless a document not answered
/// Don't Save has unsaved changes by now, which is asked about next.
fn quit_or_ask(app: &mut FontCraftApp) {
    let left: Vec<Item> = unsaved(&app.session).into_iter().filter(|i| !app.closing.discarded.contains(i)).collect();
    if left.is_empty() {
        quit(app);
    } else {
        ask(app, Purpose::Quit, left);
    }
}

fn quit(app: &mut FontCraftApp) {
    let prompt = &mut app.closing;
    prompt.asking = None;
    prompt.saving = None;
    prompt.approved = true;
    prompt.close_window = true;
}

fn cancel(app: &mut FontCraftApp) {
    app.closing.asking = None;
    app.closing.keep_window();
}

fn answer(app: &mut FontCraftApp, answer: Answer, strict: bool) -> Result<(), String> {
    let from_window = std::mem::take(&mut app.closing.from_window);
    let was_open = app.closing.is_open();
    if app.closing.saving.is_some() {
        return Err("window.closePrompt: a save is waiting for its file or its question: answer the save picker or window.fileQuestion first".into());
    }
    prune(app);
    let Some((item, name, _)) = app.closing.current(&app.session) else {
        // Every document was saved or closed meanwhile, which ended the question.
        if was_open {
            return Ok(());
        }
        return Err("window.closePrompt: no question about unsaved changes is open".into());
    };
    match answer {
        Answer::Cancel => {
            cancel(app);
            app.status = "Nothing was closed.".into();
            return Ok(());
        }
        Answer::Save => {
            if let Some(how) = needs_file(&app.session, item) {
                if !from_window {
                    return Err(format!("window.closePrompt: `{name}` has no file: save it with {how} and a `path`, then answer again"));
                }
                return save_with_picker(app, item);
            }
            match save(app, item, if from_window { None } else { Some(strict) }) {
                Ok(true) => {}
                // It asked what to do about the data the format leaves out: `after_drain` goes on.
                Ok(false) => {
                    app.closing.saving = Some(item);
                    return Ok(());
                }
                Err(e) => {
                    cancel(app);
                    return Err(format!("`{name}` was not saved, so nothing was closed: {e}"));
                }
            }
        }
        Answer::DontSave => {}
    }
    advance(app, item, answer)
}

/// `item` was answered: File › Close closes it, and the question moves to the next document.
fn advance(app: &mut FontCraftApp, item: Item, answer: Answer) -> Result<(), String> {
    let Some(purpose) = app.closing.purpose() else { return Ok(()) };
    if answer == Answer::DontSave && purpose == Purpose::Quit && !app.closing.discarded.contains(&item) {
        app.closing.discarded.push(item);
    }
    if purpose == Purpose::Close
        && let Item::Font(id) = item
    {
        let closed = activate(app, id).and_then(|()| app.apply_msg(Msg::run("file.close", json!({ "discard": answer == Answer::DontSave }))));
        if let Err(e) = closed {
            cancel(app);
            return Err(e);
        }
    }
    if let Some(asking) = app.closing.asking.as_mut()
        && asking.queue.first() == Some(&item)
    {
        asking.queue.remove(0);
        if asking.queue.is_empty() {
            app.closing.asking = None;
            if purpose == Purpose::Quit {
                quit_or_ask(app);
            }
            return Ok(());
        }
    }
    prune(app);
    show_first(app);
    Ok(())
}

/// The command that gives `item` a file, when saving it needs the save picker.
fn needs_file(session: &Session, item: Item) -> Option<&'static str> {
    match item {
        Item::Font(id) => session.documents().iter().any(|d| d.id == id && super::files::needs_save_path(d)).then_some("file.saveAs"),
        Item::Project(id) => session.projects().iter().any(|p| p.id == id && p.path.is_none()).then_some("project.save"),
    }
}

/// Saves `item`, which has a file, as File › Save does; with `strict` given, as a command that gives it does. False:
/// the save asks a question first (`app/file_questions.rs`).
fn save(app: &mut FontCraftApp, item: Item, strict: Option<bool>) -> Result<bool, String> {
    match item {
        Item::Font(id) => {
            activate(app, id)?;
            app.apply_msg(Msg::run("file.save", strict.map_or_else(|| json!({}), |strict| json!({ "strict": strict }))))?;
            if app.view.file_questions.saving(id) {
                return Ok(false);
            }
        }
        Item::Project(id) => app.apply_msg(Msg::run("project.save", json!({ "project": id })))?,
    }
    if is_unsaved(&app.session, item) {
        return Err("it still has unsaved changes".into());
    }
    Ok(true)
}

/// Asks the save picker for `item`'s file; the question waits for its answer ([`after_drain`]).
fn save_with_picker(app: &mut FontCraftApp, item: Item) -> Result<(), String> {
    let command = match item {
        Item::Font(id) => {
            activate(app, id)?;
            CommandRef::with_params("file.saveAs", json!({}))
        }
        Item::Project(id) => CommandRef::with_params("project.save", json!({ "project": id })),
    };
    if !app.picks.ask(&app.host, &app.session, &command) {
        cancel(app);
        return Err("this window has no save picker, so nothing was closed".into());
    }
    app.closing.saving = Some(item);
    Ok(())
}

/// `ui.inspect`'s `closePrompt`.
pub fn inspect(app: &FontCraftApp, out: &mut serde_json::Map<String, Value>) {
    out.insert("closePrompt".into(), state(app));
}

/// The question as `ui.inspect` reports it.
pub fn state(app: &FontCraftApp) -> Value {
    let Some(asking) = &app.closing.asking else {
        return json!({ "open": false, "purpose": null, "document": null, "kind": null, "remaining": [] });
    };
    let first = asking.queue.first().copied();
    let kind = match first {
        Some(Item::Font(_)) => json!("font"),
        Some(Item::Project(_)) => json!("project"),
        None => Value::Null,
    };
    let remaining: Vec<String> = asking.queue.iter().skip(1).filter_map(|i| title(&app.session, *i)).collect();
    json!({
        "open": true,
        "purpose": match asking.purpose { Purpose::Close => "close", Purpose::Quit => "quit" },
        "document": first.and_then(|i| title(&app.session, i)),
        "kind": kind,
        "remaining": remaining,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::shell::app::ShellApp;
    use craft::ui::shell::embed::CloseDecision;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg};

    fn app_with_two_edited_fonts() -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.run_now(Msg::run("glyph.newGlyph", json!({"name": "a.one"})), false).unwrap();
        app.run_now(Msg::run("file.new", json!({})), false).unwrap();
        app.run_now(Msg::run("glyph.newGlyph", json!({"name": "a.two"})), false).unwrap();
        app
    }

    fn answer(app: &mut FontCraftApp, answer: &str) -> Result<serde_json::Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params("window.closePrompt", json!({ "answer": answer }))), false)
    }

    /// The close box asks about each modified document in turn; Don't Save on both lets the window close.
    #[test]
    fn closing_asks_about_each_document_then_closes() {
        let mut app = app_with_two_edited_fonts();
        assert!(matches!(app.close_requested(), CloseDecision::Hold));
        let first = super::state(&app);
        assert_eq!(first["open"], json!(true));
        assert_eq!(first["remaining"].as_array().map(Vec::len), Some(1));
        let asked = app.session.active().map(|d| d.title.clone());
        assert_eq!(first["document"], json!(asked), "the document asked about is shown");

        answer(&mut app, "dontSave").unwrap();
        assert_eq!(super::state(&app)["remaining"], json!([]), "the second document is asked about next");
        answer(&mut app, "dontSave").unwrap();
        assert!(!app.closing.is_open());
        assert!(app.closing.take_close_window(), "the window closes once every document is answered");
        assert!(matches!(app.close_requested(), CloseDecision::Close));
    }

    /// Cancel keeps everything open, and a save answer from the control channel is refused for a document without a
    /// file instead of opening a picker.
    #[test]
    fn cancel_keeps_the_window_and_save_needs_a_file() {
        let mut app = app_with_two_edited_fonts();
        let _ = app.close_requested();
        let err = answer(&mut app, "save").unwrap_err();
        assert!(err.contains("has no file"), "{err}");
        assert!(app.closing.is_open(), "the question stays open");
        answer(&mut app, "cancel").unwrap();
        assert!(!app.closing.is_open() && !app.closing.take_close_window());
        assert_eq!(app.session.documents().len(), 2, "nothing was closed");
    }

    /// `app.quit` refuses unsaved work unless told to discard it; File › Close fired on a modified document asks.
    #[test]
    fn quit_refuses_unsaved_work_and_file_close_asks() {
        let mut app = app_with_two_edited_fonts();
        let err = app.run_now(Msg::Shell(CommandRef::with_params("app.quit", json!({}))), false).unwrap_err();
        assert!(err.contains("unsaved changes"), "{err}");
        assert!(super::fired(&mut app, &CommandRef::with_params("file.close", json!({}))));
        assert_eq!(super::state(&app)["purpose"], json!("close"));
        answer(&mut app, "dontSave").unwrap();
        assert_eq!(app.session.documents().len(), 1, "File › Close closed the document answered Don't Save");
        assert!(!app.closing.take_close_window(), "File › Close leaves the window open");
    }
}
