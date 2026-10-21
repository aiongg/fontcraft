//! The question asked before unsaved work is closed: closing the window, quitting (Cmd+Q, File ›
//! Quit, the app menu's Quit) and File › Close on a modified document. It asks about one
//! document at a time and shows it: Save, Don't Save or Cancel. Save and Don't Save move to the
//! next document with unsaved changes. Once every one is answered the window closes, or for File
//! › Close the document does. Cancel, a cancelled save chooser and a failed save stop there and
//! keep everything open.
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
        "{answer?: \"save\" | \"dontSave\" | \"cancel\"} the question asked before unsaved work is closed (the window's close box, Quit, File › Close on a modified document). With answer, answers it as its buttons do: save writes the document asked about (with the save chooser when it has no file), dontSave leaves its changes unsaved, and both move to the next document with unsaved changes; once none is left the window closes (File › Close: the document closes). cancel stops closing. A save that fails or whose chooser is cancelled also stops closing, and the answer is an error → {open, purpose: \"close\" | \"quit\" | null, document, kind: \"font\" | \"project\" | null, remaining: [titles]}",
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
    /// the window goes through.
    approved: bool,
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
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "app.quit" => {
            let discard = args.flag("discard")?.unwrap_or(false);
            let unsaved = unsaved(&app.session);
            if !discard && !unsaved.is_empty() {
                return Err(args.err(format!("{} unsaved changes: save first, or pass discard: true", named(&app.session, &unsaved))));
            }
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
    if ctx.viewport_id() != ViewportId::ROOT || !ctx.input(|i| i.viewport().close_requested()) || app.close_prompt.approved {
        return;
    }
    let unsaved = unsaved(&app.session);
    if unsaved.is_empty() {
        return;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    // The person has to see the question to answer it.
    if ctx.input(|i| i.viewport().minimized == Some(true)) {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    }
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    start(app, Purpose::Quit, unsaved);
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
    if choice.is_none() {
        let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        choice = if escape {
            Some("cancel")
        } else if enter {
            Some("save")
        } else {
            None
        };
    }
    if let Some(answer) = choice {
        pending.push(("window.closePrompt", json!({ "answer": answer })));
    }
}

/// End of a frame: asks the window to close when the app decided to quit, and keeps the window
/// title up to date.
pub fn end_frame(app: &mut FontcraftApp, ctx: &egui::Context) {
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
    app.close_prompt.asking = Some(Asking { purpose, queue });
    show_first(app);
}

/// Shows the document the question asks about now.
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
            quit(app);
        }
    }
}

/// The window closes at the end of the frame, and its close request goes through.
fn quit(app: &mut FontcraftApp) {
    let prompt = &mut app.close_prompt;
    prompt.asking = None;
    prompt.approved = true;
    prompt.close_window = true;
}

fn answer(app: &mut FontcraftApp, answer: Answer) -> Result<Value, String> {
    prune(app);
    let Some((purpose, item)) = app.close_prompt.asking.as_ref().and_then(|a| Some((a.purpose, *a.queue.first()?))) else {
        return Err("window.closePrompt: no question about unsaved changes is open".into());
    };
    match answer {
        Answer::Cancel => {
            app.close_prompt.asking = None;
            app.state.status = "Nothing was closed.".into();
            return Ok(state(app));
        }
        Answer::Save => {
            if let Err(e) = save(app, item) {
                app.close_prompt.asking = None;
                return Err(e);
            }
        }
        Answer::DontSave => {}
    }
    if purpose == Purpose::Close
        && let Item::Font(id) = item
    {
        let closed = activate(app, id).and_then(|()| app.run("file.close", &json!({ "discard": answer == Answer::DontSave })).map(|_| ()));
        if let Err(e) = closed {
            app.close_prompt.asking = None;
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
            quit(app);
        }
    }
    prune(app);
    show_first(app);
    Ok(state(app))
}

/// Saves `item` as File › Save does; `Err` when it is still unsaved afterwards (the save failed
/// or its chooser was cancelled), with the save's message when it gave one.
fn save(app: &mut FontcraftApp, item: Item) -> Result<(), String> {
    let name = title(&app.session, item).unwrap_or_default();
    let before = std::mem::take(&mut app.state.status);
    let saved = match item {
        Item::Font(id) => activate(app, id).and_then(|()| {
            let params = json!({});
            if pickers::wants_save_path(app, "file.save", &params) {
                pickers::save_with_chooser(app, "file.save", params);
                Ok(())
            } else {
                app.run_from_shell("file.save", &params).map(|_| ())
            }
        }),
        Item::Project(id) => save_project(app, id),
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

/// Saves the project `id` to its file, or to the file the save chooser gives for one never saved.
fn save_project(app: &mut FontcraftApp, id: u64) -> Result<(), String> {
    let Some(project) = app.session.projects().iter().find(|p| p.id == id) else { return Ok(()) };
    let mut params = serde_json::Map::new();
    params.insert("project".into(), json!(id));
    if project.path.is_none() {
        let suggested = format!("{}.{PROJECT_EXTENSION}", project.title);
        let picker = app.picker.as_mut().ok_or("the project was never saved and this window has no file chooser")?;
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

    /// One frame of the window, its close box pressed or not, with `keys` pressed; the root
    /// window's commands.
    fn frame_with(app: &mut FontcraftApp, close: bool, keys: &[Key]) -> Vec<ViewportCommand> {
        let ctx = egui::Context::default();
        let mut input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        if close {
            input.viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
        }
        for key in keys {
            input.events.push(Event::Key { key: *key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        }
        let mut out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear(); // not painted
        out.viewport_output.remove(&ViewportId::ROOT).map(|o| o.commands).unwrap_or_default()
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
        // No chooser and no file: the save fails.
        let mut app = FontcraftApp::default();
        edit(&mut app, "x.alt");
        frame(&mut app, true);
        let err = answer(&mut app, "save").unwrap_err();
        assert!(err.contains("not saved"), "{err}");
        assert_eq!(prompt(&mut app)["open"], false);
        assert!(!app.close_prompt.quitting());
        assert!(!closes(&frame(&mut app, false)));
        assert!(cancels(&frame(&mut app, true)), "the next close asks again");

        // The chooser is cancelled.
        let asked = Arc::new(Mutex::new(Vec::new()));
        app.picker = Some(Box::new(SavePicker { path: None, asked: asked.clone() }));
        assert!(answer(&mut app, "save").is_err());
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert!(!closes(&frame(&mut app, false)));
        assert!(app.session.active().is_some_and(|d| d.modified()));
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
