//! The question asked before unsaved work is closed: closing the window, quitting (Cmd+Q, File ›
//! Quit, the app menu's Quit) and File › Close on a modified document. It asks about one
//! document at a time and shows it: Save, Don't Save or Cancel. Save and Don't Save move to the
//! next document with unsaved changes. Once every one is answered the window closes, or for File
//! › Close the document does. Cancel, a cancelled save chooser and a failed save stop there and
//! keep everything open. Work changed while the question is open (by the control channel) is
//! asked about before the window closes.
//!
//! While the question is open it takes the keyboard: Return is Save, Escape is Cancel, and
//! nothing behind it sees a key ([`take_keys`]).
//!
//! What the person asks for in the window (a menu item, a shortcut, `ui.menu.invoke`) opens the
//! question ([`menu_command`]). Run as commands, `app.quit` and `file.close` refuse unsaved work
//! unless `discard` is true. `window.closePrompt` reports and answers the question, so the control
//! channel can drive it.
//!
//! The window's close box reaches [`on_close_request`] through `FontcraftApp::logic`, which the
//! desktop shell runs before it hands a close it lets through to the platform's quit.
//!
//! The window title names the active document and says when it has unsaved changes
//! ([`window_title`]).

use egui::{ViewportCommand, ViewportId};
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{Args, UiCommand, pickers};
use crate::app::FontcraftApp;

/// The app's name in the window title.
const APP_NAME: &str = "FontCraft";

/// The file extension of a FontCraft project.
const PROJECT_EXTENSION: &str = "fontcraftproject";

/// The documents an error message names before it says how many more there are.
const NAMED_IN_ERRORS: usize = 5;

pub const COMMANDS: &[UiCommand] = &[
    (
        "app.quit",
        "Quit FontCraft",
        &["File"],
        "Cmd+Q",
        "{discard?: bool} close the window and quit. Refused while a document or project has unsaved changes, unless discard is true; in the window the menu item and Cmd+Q ask about each one instead (window.closePrompt) → {quitting: true}",
    ),
    (
        "window.closePrompt",
        "Unsaved Changes",
        &[],
        "",
        "{answer?: \"save\" | \"dontSave\" | \"cancel\"} the question asked before unsaved work is closed (the window's close box, Quit, File › Close on a modified document). With answer, answers it as its buttons do: save writes the document asked about (the question's own Save button opens the save chooser for one without a file; a save answer from anywhere else is refused for it and the question stays open: give it a file with file.saveAs or project.save and a path, then answer again), dontSave leaves its changes unsaved, and both move to the next document with unsaved changes; once none is left, and nothing else changed meanwhile, the window closes (File › Close: the document closes). cancel stops closing. A save that fails or whose chooser is cancelled also stops closing, and the answer is an error → {open, purpose: \"close\" | \"quit\" | null, document, kind: \"font\" | \"project\" | null, remaining: [titles]}",
    ),
];

/// A document the question asks about, by its session id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Font(u64),
    Project(u64),
}

/// What happens once every document is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    /// File › Close: the document closes.
    Close,
    /// The window closes and the app quits.
    Quit,
}

struct Asking {
    purpose: Purpose,
    /// The documents still to ask about; the first is the one asked about now.
    queue: Vec<Item>,
}

/// The question's state, kept by the app between frames.
#[derive(Default)]
pub struct ClosePrompt {
    asking: Option<Asking>,
    /// Every document was answered, or `app.quit` was told to discard: the next close request of
    /// the window goes through, unless something outside `discarded` has unsaved changes by then.
    approved: bool,
    /// The documents answered Don't Save before quitting, or left unsaved by `app.quit` with
    /// `discard`: closing the window does not ask about them again.
    discarded: Vec<Item>,
    /// The answer run this frame came from the question's own buttons or keys, so its Save may
    /// open the save chooser. An answer from anywhere else never does: a chooser would hold the
    /// window, and a control request with it, until someone answers it.
    from_window: bool,
    /// The question's key this frame, taken before anything else sees the keyboard.
    key: Option<&'static str>,
    /// Ask the window to close at the end of the frame.
    close_window: bool,
    /// The title last given to the window.
    title: Option<String>,
}

impl ClosePrompt {
    /// Whether the question is showing.
    pub fn is_open(&self) -> bool {
        self.asking.is_some()
    }

    /// Whether the app asked the window to close and nothing holds it open.
    pub fn quitting(&self) -> bool {
        self.approved
    }

    /// The window stays open: the next close asks about every document with unsaved changes.
    /// For a close cancelled outside the question (a bug in its handling).
    pub fn keep_window(&mut self) {
        self.approved = false;
        self.close_window = false;
        self.discarded.clear();
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "app.quit" => {
            let discard = args.flag("discard")?.unwrap_or(false);
            let unsaved = unsaved(&app.session);
            if !discard && !unsaved.is_empty() {
                return Err(args.err(format!("{} unsaved changes: save first, or pass discard: true", named(&app.session, &unsaved))));
            }
            app.close_prompt.discarded = unsaved;
            quit(app);
            Ok(json!({ "quitting": true }))
        }
        "window.closePrompt" => match args.string("answer")? {
            None => Ok(state(app)),
            Some("save") => answer(app, Answer::Save),
            Some("dontSave") => answer(app, Answer::DontSave),
            Some("cancel") => answer(app, Answer::Cancel),
            Some(other) => Err(args.err(format!("`answer` is save, dontSave or cancel, not `{other}`"))),
        },
        _ => Err(args.err("unknown command")),
    }
}

/// A menu item or shortcut of `app.quit` or `file.close` with `params`: opens the question when
/// there is unsaved work to ask about and returns its state. `None` runs the command as given:
/// nothing is unsaved, `discard` is true, or it is another command.
pub fn menu_command(app: &mut FontcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if params.get("discard").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    match id {
        "app.quit" => {
            let unsaved = unsaved(&app.session);
            if unsaved.is_empty() {
                return None;
            }
            start(app, Purpose::Quit, unsaved);
        }
        "file.close" => {
            let doc = app.session.active()?;
            if !doc.modified() {
                return None;
            }
            let item = Item::Font(doc.id);
            if app.close_prompt.is_open() {
                return Some(Err("file.close: the question about unsaved changes is open: answer it first (window.closePrompt)".into()));
            }
            start(app, Purpose::Close, vec![item]);
        }
        _ => return None,
    }
    Some(Ok(state(app)))
}

/// The window was asked to close (close box, the app menu's Quit, the system): with unsaved
/// work the close is cancelled and the question asks about each document. Runs before the
/// shell's platform quit, which leaves a cancelled close alone.
pub fn on_close_request(app: &mut FontcraftApp, ctx: &egui::Context) {
    if ctx.viewport_id() != ViewportId::ROOT || !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    let prompt = &app.close_prompt;
    // Once every document is answered, only work changed since (a control request run while the
    // question was open, or after it) is asked about.
    let unsaved: Vec<Item> = unsaved(&app.session).into_iter().filter(|i| !(prompt.approved && prompt.discarded.contains(i))).collect();
    if unsaved.is_empty() {
        return;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    // The person has to see the question to answer it.
    if ctx.input(|i| i.viewport().minimized == Some(true)) {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    }
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    if app.close_prompt.approved {
        app.close_prompt.approved = false;
        app.close_prompt.close_window = false;
        ask(app, Purpose::Quit, unsaved);
    } else {
        start(app, Purpose::Quit, unsaved);
    }
    ctx.request_repaint();
}

/// Draws the question while it is open. Its buttons queue `window.closePrompt`; Return is Save
/// and Escape is Cancel.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    prune(app);
    let Some(asking) = &app.close_prompt.asking else { return };
    let Some(&first) = asking.queue.first() else { return };
    let name = title(&app.session, first).unwrap_or_default();
    let more = asking.queue.len().saturating_sub(1);
    let tokens = &app.tokens;
    let mut choice = None;
    egui::Modal::new(egui::Id::new("window.closePrompt")).backdrop_color(tokens.dialog_backdrop).show(ui.ctx(), |ui| {
        ui.set_max_width(tokens.dialog_field_width * 6.0);
        ui.spacing_mut().item_spacing.y = tokens.dialog_spacing;
        ui.label(egui::RichText::new(format!("Save the changes to “{name}”?")).strong().size(tokens.dialog_title_size));
        ui.label("Your changes are lost if you don't save them.");
        if more > 0 {
            let others = if more == 1 { "1 more document has".to_string() } else { format!("{more} more documents have") };
            ui.label(format!("{others} unsaved changes."));
        }
        ui.horizontal(|ui| {
            if ui.button("Don't Save").clicked() {
                choice = Some("dontSave");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save").clicked() {
                    choice = Some("save");
                }
                if ui.button("Cancel").clicked() {
                    choice = Some("cancel");
                }
            });
        });
    });
    let key = app.close_prompt.key.take();
    if let Some(answer) = choice.or(key) {
        app.close_prompt.from_window = true;
        pending.push(("window.closePrompt", json!({ "answer": answer })));
    }
}

/// Start of a frame while the question is open: it takes Return (Save) and Escape (Cancel), and
/// no other part of the window sees a key press, typed text or a paste, so nothing behind the
/// question changes. Key releases stay, so nothing is left held down.
pub fn take_keys(app: &mut FontcraftApp, ctx: &egui::Context) {
    app.close_prompt.key = None;
    if !app.close_prompt.is_open() {
        return;
    }
    let (enter, escape) = ctx.input_mut(|i| {
        let pressed = (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape));
        i.events.retain(|e| {
            !matches!(
                e,
                egui::Event::Key { pressed: true, .. }
                    | egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Copy
                    | egui::Event::Cut
                    | egui::Event::Ime(_)
            )
        });
        pressed
    });
    app.close_prompt.key = if escape {
        Some("cancel")
    } else if enter {
        Some("save")
    } else {
        None
    };
}

/// Whether a document or project has unsaved changes.
pub fn has_unsaved(session: &Session) -> bool {
    session.documents().iter().any(|d| d.modified()) || session.projects().iter().any(|p| p.modified())
}

/// End of a frame: asks the window to close when the app decided to quit, and keeps the window
/// title up to date.
pub fn end_frame(app: &mut FontcraftApp, ctx: &egui::Context) {
    app.close_prompt.from_window = false;
    if std::mem::take(&mut app.close_prompt.close_window) {
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        ctx.request_repaint();
    }
    let title = window_title(&app.session);
    if app.close_prompt.title.as_deref() != Some(title.as_str()) {
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Title(title.clone()));
        app.close_prompt.title = Some(title);
    }
}

/// The window title: the active document's name, with "Edited" while it has unsaved changes.
/// Off macOS the app's name follows, as the title is all the taskbar shows.
pub fn window_title(session: &Session) -> String {
    let Some(doc) = session.active() else { return APP_NAME.to_string() };
    let edited = if doc.modified() { " — Edited" } else { "" };
    if cfg!(target_os = "macos") { format!("{}{edited}", doc.title) } else { format!("{}{edited} — {APP_NAME}", doc.title) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Save,
    DontSave,
    Cancel,
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
fn start(app: &mut FontcraftApp, purpose: Purpose, queue: Vec<Item>) {
    if app.close_prompt.asking.as_ref().is_some_and(|a| a.purpose == Purpose::Quit) && purpose == Purpose::Quit {
        return;
    }
    // A new question: earlier answers and a quit decided before it no longer count.
    if !app.close_prompt.is_open() {
        app.close_prompt.keep_window();
    }
    ask(app, purpose, queue);
}

/// Asks about `queue` for `purpose`, from its first document.
fn ask(app: &mut FontcraftApp, purpose: Purpose, queue: Vec<Item>) {
    app.close_prompt.asking = Some(Asking { purpose, queue });
    show_first(app);
}

/// Shows the document the question asks about now. A project's window shows while the project
/// is open, so a project needs nothing more.
fn show_first(app: &mut FontcraftApp) {
    let Some(Item::Font(id)) = app.close_prompt.asking.as_ref().and_then(|a| a.queue.first().copied()) else { return };
    // A failure is in the status bar; the question still asks.
    let _ = activate(app, id);
}

/// Makes the document `id` the active one.
fn activate(app: &mut FontcraftApp, id: u64) -> Result<(), String> {
    if app.session.active().is_some_and(|d| d.id == id) {
        return Ok(());
    }
    let index = app.session.documents().iter().position(|d| d.id == id).ok_or("the document is no longer open")?;
    let status = std::mem::take(&mut app.state.status);
    let shown = app.run("document.activate", &json!({ "index": index })).map(|_| ());
    if shown.is_ok() {
        app.state.status = status;
    }
    shown
}

/// Drops the documents that were closed or saved meanwhile; with none left the question is done.
fn prune(app: &mut FontcraftApp) {
    let Some(asking) = app.close_prompt.asking.as_mut() else { return };
    asking.queue.retain(|i| is_unsaved(&app.session, *i));
    if asking.queue.is_empty() {
        let purpose = asking.purpose;
        app.close_prompt.asking = None;
        if purpose == Purpose::Quit {
            quit_or_ask(app);
        }
    }
}

/// Every document asked about before quitting is answered: the window closes, unless a document
/// not answered Don't Save has unsaved changes by now (it changed while the question was open),
/// which is asked about next.
fn quit_or_ask(app: &mut FontcraftApp) {
    let left: Vec<Item> = unsaved(&app.session).into_iter().filter(|i| !app.close_prompt.discarded.contains(i)).collect();
    if left.is_empty() {
        quit(app);
    } else {
        ask(app, Purpose::Quit, left);
    }
}

/// The window closes at the end of the frame, and its close request goes through.
fn quit(app: &mut FontcraftApp) {
    let prompt = &mut app.close_prompt;
    prompt.asking = None;
    prompt.approved = true;
    prompt.close_window = true;
}

/// Stops closing: nothing closes, and the next close asks about everything again.
fn cancel(app: &mut FontcraftApp) {
    app.close_prompt.asking = None;
    app.close_prompt.keep_window();
}

fn answer(app: &mut FontcraftApp, answer: Answer) -> Result<Value, String> {
    let from_window = std::mem::take(&mut app.close_prompt.from_window);
    let was_open = app.close_prompt.is_open();
    prune(app);
    let Some((purpose, item)) = app.close_prompt.asking.as_ref().and_then(|a| Some((a.purpose, *a.queue.first()?))) else {
        // Every document was saved or closed meanwhile, which ended the question.
        if was_open {
            return Ok(state(app));
        }
        return Err("window.closePrompt: no question about unsaved changes is open".into());
    };
    match answer {
        Answer::Cancel => {
            cancel(app);
            app.state.status = "Nothing was closed.".into();
            return Ok(state(app));
        }
        Answer::Save => {
            if !from_window && let Some(how) = needs_file(app, item) {
                let name = title(&app.session, item).unwrap_or_default();
                return Err(format!("window.closePrompt: `{name}` has no file: save it with {how} and a `path`, then answer again"));
            }
            if let Err(e) = save(app, item, from_window) {
                cancel(app);
                return Err(e);
            }
        }
        Answer::DontSave => {
            if purpose == Purpose::Quit && !app.close_prompt.discarded.contains(&item) {
                app.close_prompt.discarded.push(item);
            }
        }
    }
    if purpose == Purpose::Close
        && let Item::Font(id) = item
    {
        let closed = activate(app, id).and_then(|()| app.run("file.close", &json!({ "discard": answer == Answer::DontSave })).map(|_| ()));
        if let Err(e) = closed {
            cancel(app);
            return Err(e);
        }
    }
    if let Some(asking) = app.close_prompt.asking.as_mut()
        && asking.queue.first() == Some(&item)
    {
        asking.queue.remove(0);
    }
    if app.close_prompt.asking.as_ref().is_some_and(|a| a.queue.is_empty()) {
        app.close_prompt.asking = None;
        if purpose == Purpose::Quit {
            quit_or_ask(app);
        }
    }
    prune(app);
    show_first(app);
    Ok(state(app))
}

/// The command that gives `item` a file, when it has none: saving it would need the save
/// chooser.
fn needs_file(app: &FontcraftApp, item: Item) -> Option<&'static str> {
    match item {
        Item::Font(id) => app.session.documents().iter().any(|d| d.id == id && d.path.is_none()).then_some("file.saveAs"),
        Item::Project(id) => app.session.projects().iter().any(|p| p.id == id && p.path.is_none()).then_some("project.save"),
    }
}

/// Saves `item` as File › Save does, with the save chooser when `chooser` is true and it needs
/// one; `Err` when it is still unsaved afterwards (the save failed or its chooser was cancelled),
/// with the save's message when it gave one.
fn save(app: &mut FontcraftApp, item: Item, chooser: bool) -> Result<(), String> {
    let name = title(&app.session, item).unwrap_or_default();
    let before = std::mem::take(&mut app.state.status);
    let saved = match item {
        Item::Font(id) => activate(app, id).and_then(|()| {
            let params = json!({});
            if chooser && pickers::wants_save_path(app, "file.save", &params) {
                pickers::save_with_chooser(app, "file.save", params);
                Ok(())
            } else {
                app.run_from_shell("file.save", &params).map(|_| ())
            }
        }),
        Item::Project(id) => save_project(app, id, chooser),
    };
    if let Err(e) = saved {
        return Err(format!("`{name}` was not saved, so nothing was closed: {e}"));
    }
    if is_unsaved(&app.session, item) {
        let why = std::mem::replace(&mut app.state.status, before);
        return Err(if why.is_empty() {
            format!("`{name}` was not saved, so nothing was closed")
        } else {
            format!("`{name}` was not saved, so nothing was closed: {why}")
        });
    }
    Ok(())
}

/// Saves the project `id` to its file, or, with `chooser`, to the file the save chooser gives for
/// one never saved.
fn save_project(app: &mut FontcraftApp, id: u64, chooser: bool) -> Result<(), String> {
    let Some(project) = app.session.projects().iter().find(|p| p.id == id) else { return Ok(()) };
    let mut params = serde_json::Map::new();
    params.insert("project".into(), json!(id));
    if project.path.is_none() {
        let suggested = format!("{}.{PROJECT_EXTENSION}", project.title);
        let picker = app.picker.as_mut().filter(|_| chooser).ok_or("the project was never saved and this window has no file chooser")?;
        let path = picker.save("Save Project", "Project", &[PROJECT_EXTENSION], &suggested).ok_or("the chooser was cancelled")?;
        let path = pickers::with_extension(path, &[PROJECT_EXTENSION], PROJECT_EXTENSION)?;
        params.insert("path".into(), json!(path.to_string_lossy()));
    }
    app.run_from_shell("project.save", &Value::Object(params)).map(|_| ())
}

/// What `window.closePrompt` reports.
fn state(app: &FontcraftApp) -> Value {
    let Some(asking) = &app.close_prompt.asking else {
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
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use egui::{Event, Key, RawInput, ViewportCommand, ViewportEvent, ViewportId};
    use serde_json::{Value, json};

    use crate::FontcraftApp;
    use crate::shell::pickers::stub::SavePicker;

    fn press(key: Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    /// One frame of the window in `ctx`, its close box pressed or not, with `events`; the root
    /// window's commands.
    fn run_frame(ctx: &egui::Context, app: &mut FontcraftApp, close: bool, events: Vec<Event>) -> Vec<ViewportCommand> {
        let mut input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        if close {
            input.viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
        }
        let mut out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear(); // not painted
        out.viewport_output.remove(&ViewportId::ROOT).map(|o| o.commands).unwrap_or_default()
    }

    /// One frame of a new window, its close box pressed or not, with `keys` pressed.
    fn frame_with(app: &mut FontcraftApp, close: bool, keys: &[Key]) -> Vec<ViewportCommand> {
        run_frame(&egui::Context::default(), app, close, keys.iter().map(|k| press(*k)).collect())
    }

    fn frame(app: &mut FontcraftApp, close: bool) -> Vec<ViewportCommand> {
        frame_with(app, close, &[])
    }

    fn cancels(commands: &[ViewportCommand]) -> bool {
        commands.contains(&ViewportCommand::CancelClose)
    }

    fn closes(commands: &[ViewportCommand]) -> bool {
        commands.contains(&ViewportCommand::Close)
    }

    fn edit(app: &mut FontcraftApp, name: &str) {
        app.run("glyph.newGlyph", &json!({ "name": name })).unwrap();
    }

    fn prompt(app: &mut FontcraftApp) -> Value {
        app.run("window.closePrompt", &json!({})).unwrap()
    }

    fn answer(app: &mut FontcraftApp, answer: &str) -> Result<Value, String> {
        app.run("window.closePrompt", &json!({ "answer": answer }))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-close-prompt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_close_box_on_unsaved_work_asks_and_keeps_the_window_until_answered() {
        let mut app = FontcraftApp::default();
        // Nothing unsaved: the close goes through.
        assert!(!cancels(&frame(&mut app, true)));

        let mut app = FontcraftApp::default();
        edit(&mut app, "x.alt");
        let commands = frame(&mut app, true);
        assert!(cancels(&commands), "the window stays open");
        let state = prompt(&mut app);
        assert_eq!(state["open"], true);
        assert_eq!(state["purpose"], "quit");
        assert_eq!(state["document"], "Untitled");

        // Escape is Cancel: nothing closes, and the next close asks again.
        let commands = frame_with(&mut app, false, &[Key::Escape]);
        assert!(!closes(&commands));
        assert_eq!(prompt(&mut app)["open"], false);
        assert_eq!(app.session.documents().len(), 1);
        assert!(cancels(&frame(&mut app, true)));

        // Don't Save: the window is asked to close, and that close goes through.
        answer(&mut app, "dontSave").unwrap();
        assert!(closes(&frame(&mut app, false)));
        assert!(!cancels(&frame(&mut app, true)));
    }

    #[test]
    fn save_asks_about_each_document_in_turn_then_quits() {
        let dir = scratch("save");
        let named = dir.join("Named.fontcraft");
        let chosen = dir.join("Chosen.fontcraft");
        let mut app = FontcraftApp::default();
        let asked = Arc::new(Mutex::new(Vec::new()));
        app.picker = Some(Box::new(SavePicker { path: Some(chosen.clone()), asked: asked.clone() }));
        edit(&mut app, "a.alt");
        app.run("file.new", &json!({})).unwrap();
        app.run("file.saveAs", &json!({ "path": named.to_string_lossy() })).unwrap();
        edit(&mut app, "b.alt");

        assert!(cancels(&frame(&mut app, true)));
        let state = prompt(&mut app);
        assert_eq!(state["document"], "Untitled", "the first document is asked about first");
        assert_eq!(state["remaining"], json!(["Named"]));
        assert_eq!(app.session.active().map(|d| d.title.as_str()), Some("Untitled"), "the document asked about shows");

        // Return is Save: the chooser gives the never-saved document a file.
        let commands = frame_with(&mut app, false, &[Key::Enter]);
        assert!(!closes(&commands), "one more document to ask about");
        assert!(chosen.exists());
        assert_eq!(asked.lock().unwrap().len(), 1);
        let state = prompt(&mut app);
        assert_eq!(state["document"], "Named");
        assert_eq!(app.session.active().map(|d| d.title.as_str()), Some("Named"));

        let state = answer(&mut app, "save").unwrap();
        assert_eq!(state["open"], false);
        assert!(app.session.documents().iter().all(|d| !d.modified()));
        assert!(app.close_prompt.quitting());
        assert!(closes(&frame(&mut app, false)));
        assert!(!cancels(&frame(&mut app, true)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_or_cancelled_save_keeps_the_window() {
        // No chooser and no file: Save in the question fails.
        let mut app = FontcraftApp::default();
        edit(&mut app, "x.alt");
        frame(&mut app, true);
        assert!(!closes(&frame_with(&mut app, false, &[Key::Enter])));
        assert!(app.state.status.contains("not saved"), "{}", app.state.status);
        assert_eq!(prompt(&mut app)["open"], false);
        assert!(!app.close_prompt.quitting());
        assert!(!closes(&frame(&mut app, false)));
        assert!(cancels(&frame(&mut app, true)), "the next close asks again");

        // The chooser is cancelled.
        let asked = Arc::new(Mutex::new(Vec::new()));
        app.picker = Some(Box::new(SavePicker { path: None, asked: asked.clone() }));
        assert!(!closes(&frame_with(&mut app, false, &[Key::Enter])));
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert_eq!(prompt(&mut app)["open"], false);
        assert!(!closes(&frame(&mut app, false)));
        assert!(app.session.active().is_some_and(|d| d.modified()));
    }

    #[test]
    fn a_save_answer_from_the_control_channel_never_opens_the_chooser() {
        let dir = scratch("control-save");
        let path = dir.join("Given.fontcraft");
        let mut app = FontcraftApp::default();
        let asked = Arc::new(Mutex::new(Vec::new()));
        app.picker = Some(Box::new(SavePicker { path: Some(dir.join("Chosen.fontcraft")), asked: asked.clone() }));
        edit(&mut app, "x.alt");
        app.run_menu_command("app.quit", json!({})).unwrap();
        let call =
            |app: &mut FontcraftApp, method: &str, params: Value| crate::control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        let reply = call(&mut app, "engine.execute", json!({ "command": "window.closePrompt", "params": { "answer": "save" } }));
        let err = reply["error"].as_str().unwrap_or_default();
        assert!(err.contains("`Untitled` has no file") && err.contains("file.saveAs"), "{reply}");
        assert!(asked.lock().unwrap().is_empty(), "no chooser");
        assert_eq!(prompt(&mut app)["open"], true, "the question stays open");
        assert!(!app.close_prompt.quitting());

        // File › Close cannot start a second question meanwhile.
        assert!(app.run_menu_command("file.close", json!({})).is_err());
        assert_eq!(prompt(&mut app)["purpose"], "quit");

        // Given a file, the document is saved, which answers the question: the window closes.
        app.run("file.saveAs", &json!({ "path": path.to_string_lossy() })).unwrap();
        let state = answer(&mut app, "save").unwrap();
        assert_eq!(state["open"], false);
        assert!(app.close_prompt.quitting());
        assert!(closes(&frame(&mut app, false)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_changed_while_the_question_is_open_is_asked_about_before_quitting() {
        let mut app = FontcraftApp::default();
        edit(&mut app, "x.alt");
        assert!(cancels(&frame(&mut app, true)));
        // A control client changes another font while the question is open.
        app.run("file.new", &json!({})).unwrap();
        edit(&mut app, "y.alt");
        let state = answer(&mut app, "dontSave").unwrap();
        assert_eq!((&state["open"], &state["document"]), (&json!(true), &json!("Untitled 2")), "{state}");
        assert!(!app.close_prompt.quitting());
        answer(&mut app, "dontSave").unwrap();
        assert!(app.close_prompt.quitting());

        // A font changed after the last answer stops the close and is asked about; the fonts
        // answered Don't Save are not.
        app.run("file.new", &json!({})).unwrap();
        edit(&mut app, "z.alt");
        assert!(closes(&frame(&mut app, false)));
        assert!(cancels(&frame(&mut app, true)));
        assert!(!app.close_prompt.quitting(), "the cancelled close is no longer approved");
        let state = prompt(&mut app);
        assert_eq!((&state["document"], &state["remaining"]), (&json!("Untitled 3"), &json!([])), "{state}");

        // Cancel: the next close asks about every font again.
        answer(&mut app, "cancel").unwrap();
        assert!(!app.close_prompt.quitting());
        assert!(cancels(&frame(&mut app, true)));
        let state = prompt(&mut app);
        assert_eq!((&state["document"], &state["remaining"]), (&json!("Untitled"), &json!(["Untitled 2", "Untitled 3"])), "{state}");
    }

    #[test]
    fn save_writes_a_modified_font_and_a_modified_project_then_quits() {
        let dir = scratch("project");
        let mut app = FontcraftApp::default();
        let asked = Arc::new(Mutex::new(Vec::new()));
        // A name without an extension: each save adds its own.
        app.picker = Some(Box::new(SavePicker { path: Some(dir.join("Chosen")), asked: asked.clone() }));
        edit(&mut app, "x.alt");
        app.run("file.newProject", &json!({})).unwrap();
        app.run("project.addInstance", &json!({ "styleName": "Bold" })).unwrap();
        assert!(app.session.projects().iter().all(|p| p.modified()));

        assert!(cancels(&frame(&mut app, true)));
        let state = prompt(&mut app);
        assert_eq!((&state["document"], &state["kind"]), (&json!("Untitled"), &json!("font")), "{state}");
        assert_eq!(state["remaining"].as_array().map(Vec::len), Some(1), "{state}");
        assert!(!closes(&frame_with(&mut app, false, &[Key::Enter])));
        assert!(dir.join("Chosen.fontcraft").exists());
        assert_eq!(prompt(&mut app)["kind"], "project");

        assert!(closes(&frame_with(&mut app, false, &[Key::Enter])));
        assert!(dir.join("Chosen.fontcraftproject").exists());
        assert_eq!(asked.lock().unwrap().len(), 2);
        assert!(app.session.projects().iter().all(|p| !p.modified() && p.path.is_some()));
        assert!(app.close_prompt.quitting());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Glyph A with a square and a 100 × 50 image at the origin, open in Edit View at actual size.
    fn image_app(dir: &std::path::Path) -> FontcraftApp {
        use fontcraft_engine::imagery::{Rgba8, to_png};
        let png = to_png(&Rgba8 { width: 100, height: 50, pixels: [255, 0, 0, 255].repeat(100 * 50) }).unwrap();
        let file = dir.join("A.png");
        std::fs::write(&file, png).unwrap();
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 300, "y": 300, "width": 100, "height": 100})).unwrap();
        app.run("glyph.addImage", &json!({"files": [file.to_string_lossy()], "glyphs": ["A"]})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
        app
    }

    #[test]
    fn while_the_question_is_open_no_key_reaches_the_edit_view_or_a_dialog() {
        let dir = scratch("keys");
        let mut app = image_app(&dir);
        let ctx = egui::Context::default();
        for _ in 0..5 {
            run_frame(&ctx, &mut app, false, Vec::new());
        }
        let call =
            |app: &mut FontcraftApp, method: &str, params: Value| crate::control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        let clicked = call(&mut app, "ui.pointer", json!({"events": [{"kind": "down", "x": 50, "y": 25}, {"kind": "up", "x": 50, "y": 25}]}));
        assert_eq!(clicked["ok"], true, "{clicked}");
        assert!(crate::canvas::images::is_selected(&app), "the click selects the image");
        let font = app.session.font().unwrap().clone();
        let text_mode = |app: &FontcraftApp| app.state.tab().map(|t| t.text_mode);

        // A selected image: Delete, the arrows and typing change nothing, nor does ui.key.
        assert!(cancels(&run_frame(&ctx, &mut app, true, Vec::new())));
        run_frame(&ctx, &mut app, false, vec![press(Key::Delete), press(Key::ArrowRight), Event::Text("a".into())]);
        let reply = call(&mut app, "ui.key", json!({"key": "Delete"}));
        assert_eq!(reply["result"]["handled"], false, "{reply}");
        assert!(app.session.font().unwrap().same_as(&font), "nothing behind the question changed");
        assert_eq!(prompt(&mut app)["open"], true);
        // Escape cancels the question and does nothing else.
        run_frame(&ctx, &mut app, false, vec![press(Key::Escape)]);
        assert_eq!(prompt(&mut app)["open"], false);
        assert!(crate::canvas::images::is_selected(&app), "the image stays selected");
        assert_eq!(text_mode(&app), Some(false), "Escape did not switch to text mode");

        // Text mode: typing, Backspace and Escape reach only the question.
        app.run("editView.toggleTextMode", &json!({ "mode": "text" })).unwrap();
        let text = app.run("editView.getText", &json!({})).unwrap()["text"].clone();
        assert!(cancels(&run_frame(&ctx, &mut app, true, Vec::new())));
        run_frame(&ctx, &mut app, false, vec![Event::Text("b".into()), press(Key::Backspace)]);
        run_frame(&ctx, &mut app, false, vec![press(Key::Escape)]);
        assert_eq!(prompt(&mut app)["open"], false);
        assert_eq!(text_mode(&app), Some(true), "still in text mode");
        assert_eq!(app.run("editView.getText", &json!({})).unwrap()["text"], text);
        assert!(app.session.font().unwrap().same_as(&font));

        // A dialog behind the question keeps its Return and Escape.
        app.run("editView.toggleTextMode", &json!({ "mode": "edit" })).unwrap();
        let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.filterRoundCorners"}}));
        assert_eq!(opened["ok"], true, "{opened}");
        assert!(cancels(&run_frame(&ctx, &mut app, true, Vec::new())));
        run_frame(&ctx, &mut app, false, vec![press(Key::Escape)]);
        assert_eq!(prompt(&mut app)["open"], false);
        assert!(app.dialog.is_some(), "Escape went to the question, not the dialog");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_close_from_the_menu_asks_about_the_modified_document() {
        let dir = scratch("close");
        let path = dir.join("Kept.fontcraft");
        let mut app = FontcraftApp::default();
        app.run("file.new", &json!({})).unwrap();
        edit(&mut app, "x.alt");
        let state = app.run_menu_command("file.close", json!({})).unwrap();
        assert_eq!(state["purpose"], "close");
        assert_eq!(state["document"], "Untitled 2");
        assert_eq!(app.session.documents().len(), 2, "nothing closed yet");
        answer(&mut app, "cancel").unwrap();
        assert_eq!(app.session.documents().len(), 2);

        app.run_menu_command("file.close", json!({})).unwrap();
        answer(&mut app, "dontSave").unwrap();
        assert_eq!(app.session.documents().len(), 1, "Don't Save closes it");
        assert!(!app.close_prompt.quitting(), "File › Close leaves the window open");

        app.run("file.saveAs", &json!({ "path": path.to_string_lossy() })).unwrap();
        edit(&mut app, "y.alt");
        app.run_menu_command("file.close", json!({})).unwrap();
        answer(&mut app, "save").unwrap();
        assert!(app.session.documents().is_empty(), "Save writes it, then closes it");
        assert!(std::fs::read_to_string(&path).unwrap().contains("y.alt"));

        // An unmodified document closes without a question.
        app.run("file.new", &json!({})).unwrap();
        let closed = app.run_menu_command("file.close", json!({})).unwrap();
        assert!(closed.get("closed").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn app_quit_as_a_command_refuses_unsaved_work_unless_told_to_discard() {
        let mut app = FontcraftApp::default();
        edit(&mut app, "x.alt");
        let err = app.run("app.quit", &json!({})).unwrap_err();
        assert!(err.contains("`Untitled` has unsaved changes"), "{err}");
        assert!(!app.close_prompt.quitting());
        assert_eq!(app.run("app.quit", &json!({ "discard": true })).unwrap(), json!({ "quitting": true }));
        assert!(closes(&frame(&mut app, false)));

        let mut app = FontcraftApp::default();
        app.run("app.quit", &json!({})).unwrap();
        assert!(closes(&frame(&mut app, false)), "nothing unsaved: it quits");
    }

    #[test]
    fn quit_over_the_control_channel_opens_the_question_and_answers_close_the_window() {
        let mut app = FontcraftApp::default();
        edit(&mut app, "x.alt");
        app.run("file.new", &json!({})).unwrap();
        edit(&mut app, "y.alt");
        let call =
            |app: &mut FontcraftApp, method: &str, params: Value| crate::control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        let reply = call(&mut app, "ui.menu.invoke", json!({ "command": "app.quit" }));
        assert_eq!(reply["result"]["open"], true, "{reply}");
        assert_eq!(reply["result"]["remaining"], json!(["Untitled 2"]));
        let ask = |answer: &str| json!({ "command": "window.closePrompt", "params": { "answer": answer } });
        let reply = call(&mut app, "engine.execute", ask("dontSave"));
        assert_eq!(reply["result"]["document"], "Untitled 2", "{reply}");
        let reply = call(&mut app, "engine.execute", ask("dontSave"));
        assert_eq!(reply["result"]["open"], false, "{reply}");
        assert!(closes(&frame(&mut app, false)));
    }

    #[test]
    fn the_window_title_names_the_document_and_says_when_it_is_edited() {
        let title = |commands: Vec<ViewportCommand>| {
            commands.into_iter().find_map(|c| match c {
                ViewportCommand::Title(t) => Some(t),
                _ => None,
            })
        };
        let mut app = FontcraftApp::default();
        let first = title(frame(&mut app, false)).unwrap();
        assert!(first.starts_with("Untitled") && !first.contains("Edited"), "{first}");
        assert_eq!(title(frame(&mut app, false)), None, "sent only when it changes");
        edit(&mut app, "x.alt");
        let edited = title(frame(&mut app, false)).unwrap();
        assert!(edited.starts_with("Untitled — Edited"), "{edited}");
        app.run("file.close", &json!({ "discard": true })).unwrap();
        assert_eq!(title(frame(&mut app, false)).as_deref(), Some("FontCraft"));
    }
}
