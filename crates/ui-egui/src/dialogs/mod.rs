//! Dialogs: a form over one command's parameters with a live preview (decision 0004, item 6).
//!
//! Opening a dialog begins an interaction ([`fontcraft_engine::Session::begin`]); every change
//! of a field previews the command with the dialog's parameters on the font as it was when the
//! dialog opened. OK commits the last preview as one undo step; Cancel puts back the very font
//! the dialog opened on. Agents can skip the dialog and run the command with the same
//! parameters: the result is the same font.
//!
//! The fields come from the command's parameters: a filter's from its `filter.list` schema
//! ([`Fields::Filter`]), any other command's from a list in its dialog's file
//! ([`Fields::Fixed`]). Every filter with parameters gets a dialog generated from its schema;
//! a file in this directory names a dialog's spec row and can replace its fields.
//!
//! Keys while a dialog is open (the field rules of `infoBox.fieldEditing` and
//! `infoBox.arrowKeyStepping`): in a number field, `Return` takes the typed value, `Escape`
//! drops it, `Tab` and `Shift+Tab` take it and move to the next or previous field, `Up` and
//! `Down` step the value by 1, by 10 with `Shift`. With no field taking the keyboard, `Return`
//! is OK and `Escape` is Cancel. The rest of the app takes no input until the dialog closes.

mod export;
pub mod form;
pub mod import_outlines_missing_glyph;
mod interpolate_with_background;
pub mod pasted_outline_bounds;
pub mod primitive_measurements;
mod round_corners;
mod transformations;

use serde::Serialize;
use serde_json::{Map, Value, json};

use self::form::{Field, format_value};
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

/// One dialog: the command it runs and where its fields come from.
#[derive(Clone, Copy, Debug)]
pub struct Dialog {
    /// The dialog's row in `spec/inventory` (`dialog.filterRoundCorners`).
    pub id: &'static str,
    /// The window title, also the label of the undo step OK makes.
    pub title: &'static str,
    /// The command OK runs.
    pub command: &'static str,
    pub fields: Fields,
}

/// Where a dialog's fields come from.
#[derive(Clone, Copy, Debug)]
pub enum Fields {
    /// The parameters of the filter `command`, as `filter.list` describes them.
    Filter,
    /// A list for a command that is not a filter. The session gives values that depend on the
    /// font (a default taken from the font's metrics).
    Fixed(fn(&fontcraft_engine::Session) -> Vec<Field>),
}

/// Every dialog with its own file, one line each.
pub const DIALOGS: &[Dialog] = &[
    round_corners::DIALOG,
    interpolate_with_background::DIALOG,
    primitive_measurements::RECTANGLE,
    primitive_measurements::ELLIPSE,
    import_outlines_missing_glyph::DIALOG,
    pasted_outline_bounds::DIALOG,
    transformations::DIALOG,
    export::DIALOG,
];

/// The shell commands of this module.
pub const COMMANDS: &[UiCommand] = &[(
    "dialog.open",
    "Open Dialog",
    &[],
    "",
    "{dialog: id (e.g. \"dialog.filterRoundCorners\") | command: the id of the command it runs, params?: {…}} opens a command's dialog with a live preview. Parameters of its fields set their first values; the rest (glyph, glyphs, layer, points…) say what it acts on, else the selection does. Then ui.dialog.set, ui.dialog.confirm, ui.dialog.cancel → ui.dialog.inspect's reply",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "dialog.open" => {
            let params = match args.get("params") {
                None => Map::new(),
                Some(Value::Object(m)) => m.clone(),
                Some(_) => return Err(args.err("`params` must be a JSON object")),
            };
            let dialog = match (args.string("dialog")?, args.string("command")?) {
                (Some(d), None) => find(d).ok_or_else(|| args.err(format!("no dialog `{d}`")))?,
                (None, Some(c)) => for_command(c).ok_or_else(|| args.err(format!("`{c}` has no dialog")))?,
                _ => return Err(args.err("give `dialog` or `command`")),
            };
            open(app, &dialog, &params).map_err(|e| args.err(e))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// A dialog that is open, with what its fields hold. Serialized as `ui.dialog.inspect` shows it.
#[derive(Clone, Debug, Serialize)]
pub struct OpenDialog {
    pub id: String,
    pub title: String,
    pub command: String,
    pub fields: Vec<Field>,
    /// Each field's value, by key.
    pub values: Map<String, Value>,
    /// What the command acts on (glyph, glyphs, layer, points…), fixed when the dialog opened.
    pub targets: Map<String, Value>,
    /// The number field taking the keyboard.
    pub editing: Option<Editing>,
    /// Why the last preview failed; OK waits until a preview succeeds.
    pub error: Option<String>,
    /// A key took the keyboard away from a field: the next frame unfocuses its widget.
    #[serde(skip)]
    pub(crate) blur: bool,
}

/// The number field taking the keyboard and its text.
#[derive(Clone, Debug, Serialize)]
pub struct Editing {
    pub field: String,
    pub text: String,
    /// Typing replaces the text: the field's text is selected when Tab or `ui.click` brings the
    /// keyboard to it.
    #[serde(skip)]
    pub(crate) replace: bool,
}

/// What the person did in a frame, applied after the drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Set(String, Value),
    StartEdit(String, String),
    Text(String),
    CommitEdit,
    Confirm,
    Cancel,
}

/// A key a dialog takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKey {
    Return,
    Escape,
    Tab,
    Up,
    Down,
}

impl DialogKey {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "Return" | "Enter" => Self::Return,
            "Escape" | "Esc" => Self::Escape,
            "Tab" => Self::Tab,
            "ArrowUp" | "Up" => Self::Up,
            "ArrowDown" | "Down" => Self::Down,
            _ => return None,
        })
    }
}

impl OpenDialog {
    /// The command's parameters: the targets and every field's value.
    pub fn params(&self) -> Value {
        let mut m = self.targets.clone();
        m.extend(self.values.iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(m)
    }

    fn field(&self, key: &str) -> Result<&Field, String> {
        self.fields.iter().find(|f| f.key == key).ok_or_else(|| {
            let keys: Vec<&str> = self.fields.iter().map(|f| f.key.as_str()).collect();
            format!("the dialog has no field `{key}`; its fields are {}", keys.join(", "))
        })
    }
}

/// The dialog with this id: one with its own file, else one generated for a filter.
pub fn find(id: &str) -> Option<Dialog> {
    if let Some(d) = DIALOGS.iter().find(|d| d.id == id) {
        return Some(*d);
    }
    filters().into_iter().find(|(command, _)| generated_id(command) == id).map(|(command, title)| generated(command, title))
}

/// The dialog of the command `command`, if it has one.
pub fn for_command(command: &str) -> Option<Dialog> {
    if let Some(d) = DIALOGS.iter().find(|d| d.command == command) {
        return Some(*d);
    }
    filters().into_iter().find(|(c, _)| *c == command).map(|(command, title)| generated(command, title))
}

fn generated(command: &'static str, title: &'static str) -> Dialog {
    Dialog { id: "", title, command, fields: Fields::Filter }
}

/// The id of a generated filter dialog: the command's, with `filter.` turned into
/// `dialog.filter` and the next letter capitalized (`filter.roundCorners` →
/// `dialog.filterRoundCorners`).
fn generated_id(command: &str) -> String {
    let rest = command.strip_prefix("filter.").unwrap_or(command);
    let mut chars = rest.chars();
    chars.next().map_or_else(String::new, |c| format!("dialog.filter{}{}", c.to_ascii_uppercase(), chars.as_str()))
}

/// The filter commands that take parameters, with their labels.
fn filters() -> Vec<(&'static str, &'static str)> {
    let entries = filter_list();
    fontcraft_engine::command_specs()
        .iter()
        .filter(|c| c.id.starts_with("filter.") && c.menu == ["Filter"])
        .filter(|c| {
            entries.iter().any(|e| {
                e.get("command").and_then(Value::as_str) == Some(c.id) && e.get("params").and_then(Value::as_array).is_some_and(|p| !p.is_empty())
            })
        })
        .map(|c| (c.id, c.label))
        .collect()
}

/// The entries of `filter.list`. It reads no document, so a scratch session answers it and the
/// app's session (perhaps inside an interaction) is left alone.
fn filter_list() -> Vec<Value> {
    let mut scratch = fontcraft_engine::Session::new();
    let Ok(all) = scratch.execute("filter.list", &Value::Null) else { return Vec::new() };
    all.get("filters").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The `filter.list` entry of the filter command `command`.
fn filter_entry(command: &str) -> Option<Value> {
    filter_list().into_iter().find(|f| f.get("command").and_then(Value::as_str) == Some(command))
}

/// Opens `dialog`: the first values from `given` or the defaults, the targets from `given` or
/// the selection, and the first preview.
pub fn open(app: &mut FontcraftApp, dialog: &Dialog, given: &Map<String, Value>) -> Result<Value, String> {
    if let Some(d) = &app.dialog {
        return Err(format!("the dialog `{}` is open: confirm or cancel it first", d.title));
    }
    let fields = match dialog.fields {
        Fields::Filter => form::filter_fields(&filter_entry(dialog.command).ok_or_else(|| format!("`{}` is not a filter", dialog.command))?)?,
        Fields::Fixed(list) => list(&app.session),
    };
    let values = form::initial_values(&fields, given)?;
    let mut targets: Map<String, Value> = given.iter().filter(|(k, _)| !values.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    if targets.is_empty() {
        targets = selection_targets(app, dialog.command);
    }
    if targets.is_empty() {
        return Err(format!("{}: select a glyph first", dialog.title));
    }
    app.session.begin(dialog.title).map_err(|e| e.to_string())?;
    let id = if dialog.id.is_empty() { generated_id(dialog.command) } else { dialog.id.to_string() };
    app.dialog = Some(OpenDialog {
        id,
        title: dialog.title.to_string(),
        command: dialog.command.to_string(),
        fields,
        values,
        targets,
        editing: None,
        error: None,
        blur: false,
    });
    preview(app);
    Ok(inspect(app))
}

/// What the command acts on when the dialog names nothing: the Edit View selection, else the
/// glyphs selected in Font View.
fn selection_targets(app: &FontcraftApp, command: &str) -> Map<String, Value> {
    let from_view = crate::tools_glue::with_targets(app, command, Value::Null);
    match from_view {
        Value::Object(m) if !m.is_empty() => m,
        _ if app.state.tab().is_none() && !app.state.selected_glyphs.is_empty() => {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(app.state.selected_glyphs));
            m
        }
        _ => Map::new(),
    }
}

/// Runs the dialog's command on the font as it was when the dialog opened.
fn preview(app: &mut FontcraftApp) {
    let Some(d) = &app.dialog else { return };
    let (command, params) = (d.command.clone(), d.params());
    let result = app.session.preview(&command, &params);
    if let Some(d) = app.dialog.as_mut() {
        d.error = result.err().map(|e| e.to_string());
        if let Some(e) = &d.error {
            app.state.status = e.clone();
        }
    }
}

/// `ui.dialog.set`: gives a field a value and previews it.
pub fn set(app: &mut FontcraftApp, key: &str, value: &Value) -> Result<Value, String> {
    let d = app.dialog.as_mut().ok_or("no dialog is open")?;
    let checked = d.field(key)?.check(value)?;
    d.values.insert(key.to_string(), checked);
    if d.editing.as_ref().is_some_and(|e| e.field == key) {
        d.editing = None;
    }
    preview(app);
    Ok(inspect(app))
}

/// Takes the text of the field being edited as its value. A text that is not a value puts the
/// field back and is the error.
fn commit_edit(app: &mut FontcraftApp) -> Result<(), String> {
    let Some(d) = app.dialog.as_mut() else { return Ok(()) };
    let Some(editing) = d.editing.take() else { return Ok(()) };
    let field = d.field(&editing.field)?;
    let current = d.values.get(&editing.field).map(format_value).unwrap_or_default();
    if editing.text.trim() == current {
        return Ok(());
    }
    let value = field.check(&json!(editing.text)).inspect_err(|e| app.state.status = e.clone())?;
    d.values.insert(editing.field, value);
    preview(app);
    Ok(())
}

/// `ui.dialog.confirm` and OK: the last preview becomes one undo step.
pub fn confirm(app: &mut FontcraftApp) -> Result<Value, String> {
    if app.dialog.is_none() {
        return Err("no dialog is open".into());
    }
    commit_edit(app)?;
    let Some(d) = &app.dialog else { return Err("no dialog is open".into()) };
    if let Some(e) = &d.error {
        return Err(format!("{}: the preview failed, so OK has nothing to keep: {e}", d.title));
    }
    let (title, command, params) = (d.title.clone(), d.command.clone(), d.params());
    let changed = app.session.commit().map_err(|e| e.to_string())?;
    app.dialog = None;
    app.state.status = format!("{title}: done");
    Ok(json!({"confirmed": true, "changed": changed, "command": command, "params": params}))
}

/// `ui.dialog.cancel` and Cancel: the font as it was when the dialog opened.
pub fn cancel(app: &mut FontcraftApp) -> Result<Value, String> {
    let d = app.dialog.take().ok_or("no dialog is open")?;
    if app.session.interaction().is_some() {
        app.session.cancel().map_err(|e| e.to_string())?;
    }
    app.state.status = format!("{}: cancelled", d.title);
    Ok(json!({"cancelled": true}))
}

/// `ui.dialog.inspect`: the open dialog, its fields and values, and the parameters OK runs.
pub fn inspect(app: &FontcraftApp) -> Value {
    let Some(d) = &app.dialog else { return json!({"open": false}) };
    let fields: Vec<Value> = d
        .fields
        .iter()
        .map(|f| {
            let mut v = serde_json::to_value(f).unwrap_or(Value::Null);
            if let Some(m) = v.as_object_mut() {
                m.insert("id".into(), json!(f.key));
                m.insert("value".into(), d.values.get(&f.key).cloned().unwrap_or(Value::Null));
                m.insert("focused".into(), json!(d.editing.as_ref().is_some_and(|e| e.field == f.key)));
            }
            v
        })
        .collect();
    json!({
        "open": true,
        "dialog": d.id,
        "title": d.title,
        "command": d.command,
        "fields": fields,
        "editing": d.editing,
        "params": d.params(),
        "error": d.error,
        "buttons": ["ok", "cancel"],
    })
}

/// `ui.click`: presses a control of the open dialog: `ok`, `cancel`, or a field by its key (a
/// checkbox toggles; a number field takes the keyboard with its text selected).
pub fn click(app: &mut FontcraftApp, id: &str) -> Result<Value, String> {
    let d = app.dialog.as_ref().ok_or_else(|| format!("no control `{id}`: no dialog is open"))?;
    match id {
        "ok" => return confirm(app),
        "cancel" => return cancel(app),
        _ => {}
    }
    let field = d.field(id)?.clone();
    match &field.kind {
        form::Kind::Bool => {
            let on = d.values.get(id).and_then(Value::as_bool).unwrap_or(false);
            commit_edit(app)?;
            set(app, id, &json!(!on))
        }
        form::Kind::Number { .. } | form::Kind::Integer { .. } => {
            commit_edit(app)?;
            focus(app, &field.key);
            Ok(inspect(app))
        }
        form::Kind::Choice { .. } => Err(format!("`{id}` is a pop-up menu: choose with ui.dialog.set")),
    }
}

/// Gives the number field `key` the keyboard, its text selected.
fn focus(app: &mut FontcraftApp, key: &str) {
    if let Some(d) = app.dialog.as_mut() {
        let text = d.values.get(key).map(format_value).unwrap_or_default();
        d.editing = Some(Editing { field: key.to_string(), text, replace: true });
        d.blur = false;
    }
}

/// A key for the open dialog (see the module docs). `None` when no dialog is open.
pub fn key(app: &mut FontcraftApp, key: DialogKey, shift: bool) -> Option<Result<Value, String>> {
    let d = app.dialog.as_mut()?;
    let editing = d.editing.clone();
    let result = match (key, editing) {
        (DialogKey::Return, Some(_)) => {
            d.blur = true;
            commit_edit(app).map(|()| inspect(app))
        }
        (DialogKey::Return, None) => confirm(app),
        (DialogKey::Escape, Some(_)) => {
            d.editing = None;
            d.blur = true;
            Ok(inspect(app))
        }
        (DialogKey::Escape, None) => cancel(app),
        (DialogKey::Tab, editing) => {
            let text_fields: Vec<String> = d.fields.iter().filter(|f| f.is_text()).map(|f| f.key.clone()).collect();
            let at = editing.and_then(|e| text_fields.iter().position(|k| *k == e.field));
            let count = text_fields.len();
            let next = match (at, shift) {
                (_, _) if count == 0 => None,
                (None, false) => Some(0),
                (None, true) => Some(count - 1),
                (Some(i), false) => Some((i + 1) % count),
                (Some(i), true) => Some((i + count - 1) % count),
            };
            commit_edit(app).map(|()| {
                if let Some(k) = next.and_then(|i| text_fields.get(i)) {
                    focus(app, k);
                }
                inspect(app)
            })
        }
        (DialogKey::Up | DialogKey::Down, Some(e)) => step(app, &e, key == DialogKey::Up, shift),
        (DialogKey::Up | DialogKey::Down, None) => Ok(inspect(app)),
    };
    Some(result)
}

/// `Up` or `Down` in a number field: the typed text (or the value) one step on, previewed.
fn step(app: &mut FontcraftApp, editing: &Editing, up: bool, shift: bool) -> Result<Value, String> {
    let d = app.dialog.as_ref().ok_or("no dialog is open")?;
    let field = d.field(&editing.field)?.clone();
    let current = field.check(&json!(editing.text)).or_else(|_| d.values.get(&field.key).cloned().ok_or("no value"))?;
    let Some(next) = field.step(&current, up, shift) else { return Ok(inspect(app)) };
    set(app, &field.key, &next)?;
    focus(app, &field.key);
    Ok(inspect(app))
}

/// `ui.text` with a number field taking the keyboard: types into it. `false` when none does.
pub fn type_text(app: &mut FontcraftApp, text: &str) -> bool {
    let Some(editing) = app.dialog.as_mut().and_then(|d| d.editing.as_mut()) else { return false };
    if editing.replace {
        editing.text.clear();
        editing.replace = false;
    }
    let room = form::MAX_FIELD_TEXT.saturating_sub(editing.text.chars().count());
    editing.text.extend(text.chars().take(room));
    true
}

/// The dialog's keys among this frame's events, taken before anything else sees them. `Tab`
/// stays: egui moves the keyboard between the fields itself.
pub fn frame_keys(app: &mut FontcraftApp, ctx: &egui::Context) {
    if app.dialog.is_none() {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let mut used = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
        let k = match key {
            egui::Key::Enter => DialogKey::Return,
            egui::Key::Escape => DialogKey::Escape,
            egui::Key::ArrowUp => DialogKey::Up,
            egui::Key::ArrowDown => DialogKey::Down,
            _ => continue,
        };
        // Up and Down only matter in a number field; elsewhere they belong to the pop-up menus.
        let editing = app.dialog.as_ref().is_some_and(|d| d.editing.is_some());
        if matches!(k, DialogKey::Up | DialogKey::Down) && !editing {
            continue;
        }
        if let Some(Err(e)) = self::key(app, k, modifiers.shift) {
            app.state.status = e;
        }
        used.push(index);
    }
    if !used.is_empty() {
        ctx.input_mut(|i| {
            let mut index = 0;
            i.events.retain(|_| {
                let keep = !used.contains(&index);
                index += 1;
                keep
            });
        });
    }
}

/// Draws the open dialog over the window and applies what was done in it.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui) {
    let Some(d) = &app.dialog else { return };
    if app.session.interaction().is_none() {
        // The document closed under the dialog: nothing is left to preview.
        app.dialog = None;
        return;
    }
    let tokens = &app.tokens;
    let mut events = Vec::new();
    egui::Window::new(d.title.as_str())
        .id(egui::Id::new("fontcraft-dialog"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            form::draw(ui, d, tokens, &mut events);
            if let Some(e) = &d.error {
                ui.colored_label(tokens.error_text, e);
            }
            ui.add_space(tokens.dialog_spacing);
            egui::Sides::new().show(
                ui,
                |_| {},
                |ui| {
                    if ui.add_enabled(d.error.is_none(), egui::Button::new("OK")).clicked() {
                        events.push(Event::Confirm);
                    }
                    if ui.button("Cancel").clicked() {
                        events.push(Event::Cancel);
                    }
                },
            );
        });
    if let Some(d) = app.dialog.as_mut() {
        d.blur = false;
    }
    for event in events {
        let result = match event {
            Event::Set(key, value) => set(app, &key, &value).map(drop),
            Event::StartEdit(key, text) => commit_edit(app).map(|()| {
                if let Some(d) = app.dialog.as_mut() {
                    d.editing = Some(Editing { field: key, text, replace: false });
                }
            }),
            Event::Text(text) => {
                if let Some(e) = app.dialog.as_mut().and_then(|d| d.editing.as_mut()) {
                    e.text = text;
                }
                Ok(())
            }
            Event::CommitEdit => commit_edit(app),
            Event::Confirm => confirm(app).map(drop),
            Event::Cancel => cancel(app).map(drop),
        };
        if let Err(e) = result {
            app.state.status = e;
        }
    }
}
