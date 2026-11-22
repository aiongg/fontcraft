//! The open dialog: its state in the app layer and what OK, Cancel and every edit do to the engine.
//!
//! Opening a [`Mode::Preview`] dialog begins an interaction (`Session::begin`); every change of a field previews the
//! command with the dialog's parameters on the font as it was when the dialog opened. OK commits the last preview as
//! one undo step; Cancel puts back the very font the dialog opened on. A [`Mode::RunOnOk`] dialog runs nothing while
//! it is open; OK runs the command once and a failure keeps the dialog open with its message. Agents can skip the
//! dialog and run the command with the same parameters: the result is the same font.
//!
//! The state lives in [`FontCraftApp::dialogs`] so the window and the control channel act on the same dialog: the
//! window's view (`host.rs`) shows it and turns what the person does into calls here; `control/dialogs.rs` calls the
//! same functions. These are the only calls outside `drain` that drive the engine: the interaction calls and the run
//! of OK, whose result decides whether the dialog stays open.

use craft::host::{PickId, PickRequest, Picked};
use serde_json::{Map, Value, json};

use std::collections::HashMap;

use super::fields::{self, Field, Kind, format_value, lock_key};
use super::spec::{Action, DialogSpec, Fields, Mode, Shaped, Tab, Third};
use crate::app::FontCraftApp;
use crate::shell::masters::current_master_id;
use craft::ui::runtime::commands::CommandRef;

/// The dialog state the app keeps: the open dialog and the folder chooser it waits for.
#[derive(Default)]
pub struct DialogState {
    /// The dialog that is open.
    pub open: Option<OpenDialog>,
    /// Counts the dialogs opened, so the window knows a dialog from the one before it.
    opened: u64,
    /// The folder chooser asked for a field: its pick, the dialog's serial, the field, and whether OK follows the
    /// answer.
    choosing: Option<(PickId, u64, String, bool)>,
    /// Text an action put out for the system clipboard, until the window copies it.
    pub clipboard: Option<String>,
    /// The values of each filter dialog's last OK, by dialog id: the dialog opens with them again for the rest of the
    /// session.
    remembered: HashMap<String, Map<String, Value>>,
}

/// Most dialogs whose values [`DialogState`] keeps.
const MAX_REMEMBERED: usize = 256;

impl DialogState {
    /// Keeps `values` as the last OK of the dialog `id`.
    fn remember(&mut self, id: String, values: Map<String, Value>) {
        if self.remembered.len() < MAX_REMEMBERED || self.remembered.contains_key(&id) {
            self.remembered.insert(id, values);
        }
    }
}

/// A dialog that is open, with what its fields hold.
#[derive(Clone, Debug)]
pub struct OpenDialog {
    pub id: String,
    pub title: String,
    pub command: String,
    pub mode: Mode,
    pub buttons: [&'static str; 2],
    pub third: Option<Third>,
    pub message: &'static str,
    pub fields: Vec<Field>,
    /// Each field's value, by key.
    pub values: Map<String, Value>,
    /// What the command acts on (glyph, glyphs, layer, points…), fixed when the dialog opened.
    pub targets: Map<String, Value>,
    /// The text field taking the keyboard of the control channel (`ui.dialog.click`, then `ui.text` and `ui.key`).
    pub editing: Option<Editing>,
    /// Why the last preview failed (OK waits until a preview succeeds), or, in [`Mode::RunOnOk`], why OK's run failed
    /// (until a field changes).
    pub error: Option<String>,
    pub actions: &'static [Action],
    /// How the field values become the command's parameters, for a [`Fields::Shaped`] dialog.
    pub shaped: Option<Shaped>,
    /// The tabs, when the dialog is one tab of several.
    pub tabs: Option<Tabs>,
    /// Which dialog this is among those opened; a tab shown keeps it.
    pub serial: u64,
    /// Moves with every change, so the window shows the values the app holds.
    pub revision: u64,
    /// A filter dialog: its values at OK are kept for the next time it opens ([`DialogState::remember`]).
    pub remember: bool,
    /// Fills the fields that show what OK will do ([`super::spec::Annotate`]).
    pub annotate: Option<super::spec::Annotate>,
}

/// The tabs of an open dialog.
#[derive(Clone, Debug)]
pub struct Tabs {
    pub list: &'static [Tab],
    /// The tab showing.
    pub active: usize,
    /// The values of the tabs shown before, by the tab's dialog id.
    pub saved: Map<String, Value>,
}

/// The text field taking the keyboard and its text.
#[derive(Clone, Debug)]
pub struct Editing {
    pub field: String,
    pub text: String,
    /// Typing replaces the text: the field's text is selected when Tab or a click brings the keyboard to it.
    pub replace: bool,
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
    /// The key named `name` as the control channel names keys.
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
    /// The command's parameters: the targets and every field's value (shaped, for a [`Fields::Shaped`] dialog). An
    /// empty folder or file field is left out.
    pub fn params(&self) -> Value {
        let values: Map<String, Value> = self
            .values
            .iter()
            .filter(|(k, v)| !self.fields.iter().any(|f| f.key == **k && f.is_empty_path(v)))
            .filter(|(k, _)| !self.fields.iter().any(|f| f.same_as.is_some() && lock_key(&f.key) == **k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut m = self.targets.clone();
        m.extend(match &self.shaped {
            Some(s) => (s.params)(&values),
            None => values,
        });
        Value::Object(m)
    }

    /// Gives the field `key` the checked `value` and brings linked fields in step.
    fn put(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_string(), value);
        if let Some(s) = &self.shaped {
            (s.link)(&mut self.values, key);
        }
        self.sync_locks();
        self.revision += 1;
    }

    /// Gives every field whose lock is closed the value of the field it is the same as.
    fn sync_locks(&mut self) {
        for f in &self.fields {
            let Some(other) = &f.same_as else { continue };
            if self.locked(&f.key)
                && let Some(value) = self.values.get(other).cloned()
            {
                self.values.insert(f.key.clone(), value);
            }
        }
    }

    /// Whether field `key` is tied to its `same_as` field by its closed lock.
    pub fn locked(&self, key: &str) -> bool {
        self.fields.iter().any(|f| f.key == key && f.same_as.is_some()) && self.values.get(&lock_key(key)).and_then(Value::as_bool).unwrap_or(false)
    }

    /// The third button, while the dialog has the field it answers.
    pub fn third(&self) -> Option<Third> {
        self.third.filter(|t| self.fields.iter().any(|f| f.key == t.key))
    }

    /// The OK button's caption; empty for the common OK.
    pub fn ok_caption(&self) -> &'static str {
        self.third().and_then(|t| t.ok_caption).unwrap_or(self.buttons[0])
    }

    /// Whether OK can be pressed: a preview that failed leaves nothing to keep.
    pub fn can_confirm(&self) -> bool {
        self.error.is_none() || self.mode == Mode::RunOnOk
    }

    pub fn field(&self, key: &str) -> Result<&Field, String> {
        self.fields.iter().find(|f| f.key == key).ok_or_else(|| {
            let keys: Vec<&str> = self.fields.iter().map(|f| f.key.as_str()).collect();
            format!("the dialog has no field `{key}`; its fields are {}", keys.join(", "))
        })
    }
}

/// `dialog.open {dialog | command, params?}`: opens a command's dialog → the `ui.dialog.inspect` reply.
pub fn open_command(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let given = match params.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("`params` must be a JSON object".into()),
    };
    let text = |key: &str| params.get(key).and_then(Value::as_str);
    let spec = match (text("dialog"), text("command")) {
        (Some(d), None) => super::find(d).ok_or_else(|| format!("no dialog `{d}`"))?,
        (None, Some(c)) => super::for_command(c).ok_or_else(|| format!("`{c}` has no dialog"))?,
        _ => return Err("give `dialog` or `command`".into()),
    };
    open(app, &spec, &given)
}

/// Opens `spec`: the first values from `given` or the defaults, the targets from `given` or the selection, and the
/// first preview. A dialog with tabs opens on its first tab; a tab opens with its siblings.
pub fn open(app: &mut FontCraftApp, spec: &DialogSpec, given: &Map<String, Value>) -> Result<Value, String> {
    if let Fields::Tabs(list) = spec.fields {
        return open_tab(app, list, 0, given, &Map::new());
    }
    if let Some((list, index)) = tab_of(spec.id) {
        return open_tab(app, list, index, given, &Map::new());
    }
    open_one(app, spec, given, None, None)
}

/// The tab list that holds the dialog `id`, and its index there.
fn tab_of(id: &str) -> Option<(&'static [Tab], usize)> {
    super::SPECS.iter().flat_map(|specs| specs.iter()).find_map(|d| match d.fields {
        Fields::Tabs(list) => list.iter().position(|t| t.dialog.id == id).map(|i| (list, i)),
        _ => None,
    })
}

/// Opens the tab `index` of `list`. `saved`: the values of the tabs shown before ([`Tabs::saved`]).
fn open_tab(
    app: &mut FontCraftApp,
    list: &'static [Tab],
    index: usize,
    given: &Map<String, Value>,
    saved: &Map<String, Value>,
) -> Result<Value, String> {
    open_tab_as(app, list, index, given, saved, None)
}

fn open_tab_as(
    app: &mut FontCraftApp,
    list: &'static [Tab],
    index: usize,
    given: &Map<String, Value>,
    saved: &Map<String, Value>,
    serial: Option<u64>,
) -> Result<Value, String> {
    let tab = list.get(index).ok_or_else(|| format!("the dialog has no tab {index}"))?;
    if let Fields::Tabs(_) = tab.dialog.fields {
        return Err(format!("the tab `{}` holds tabs of its own", tab.id));
    }
    open_one(app, &tab.dialog, given, Some(Tabs { list, active: index, saved: saved.clone() }), serial)
}

/// The key of [`Tabs::saved`] that holds the values of the tab shown last, for a tab shown for the first time.
const CARRY: &str = "";

fn open_one(app: &mut FontCraftApp, spec: &DialogSpec, given: &Map<String, Value>, tabs: Option<Tabs>, serial: Option<u64>) -> Result<Value, String> {
    if let Some(d) = &app.dialogs.open {
        return Err(format!("the dialog `{}` is open: confirm or cancel it first", d.title));
    }
    let mut shaped = None;
    let mut given = given.clone();
    if let Some(window) = spec.window {
        for (key, value) in window(app) {
            given.entry(key).or_insert(value);
        }
    }
    let fields = match spec.fields {
        Fields::Filter => fields::filter_fields(&super::filter_entry(spec.command).ok_or_else(|| format!("`{}` is not a filter", spec.command))?)?,
        Fields::Fixed(list) => list(&app.session),
        Fields::Targeted(list) => {
            let mut targets = given.clone();
            if !targets.contains_key("glyph") && !targets.contains_key("glyphs") {
                targets.extend(selection_targets(app, spec.command));
            }
            list(&mut app.session, &targets)?
        }
        Fields::Shaped(s) => {
            shaped = Some(s);
            given = (s.given)(&app.session, &given)?;
            (s.fields)(&app.session)
        }
        Fields::Tabs(_) => return Err(format!("`{}` has tabs: open one of them", spec.title)),
    };
    if fields.len() > fields::MAX_FIELDS {
        return Err(format!("a dialog has at most {} fields", fields::MAX_FIELDS));
    }
    let mut values = fields::initial_values(&fields, &given)?;
    let mut targets: Map<String, Value> = given.iter().filter(|(k, _)| !values.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let id = if spec.id.is_empty() { super::generated_id(spec.command) } else { spec.id.to_string() };
    // A filter dialog opens with the values of its last OK, where they still fit.
    let remember = matches!(spec.fields, Fields::Filter);
    if remember && let Some(last) = app.dialogs.remembered.get(&id) {
        for f in &fields {
            if given.get(&f.key).is_some_and(|v| !v.is_null()) {
                continue;
            }
            if let Some(v) = last.get(&f.key).and_then(|v| f.check(v).ok()) {
                values.insert(f.key.clone(), v);
            }
        }
    }
    if targets.is_empty() {
        targets = selection_targets(app, spec.command);
    }
    // A tab shown before gets its values back; one shown first takes the values of the fields it shares with the tab
    // shown before it.
    if let Some(t) = &tabs {
        let earlier = t.saved.get(spec.id).and_then(Value::as_object).or_else(|| t.saved.get(CARRY).and_then(Value::as_object));
        for f in &fields {
            if given.contains_key(&f.key) {
                continue;
            }
            if let Some(v) = earlier.and_then(|m| m.get(&f.key)).and_then(|v| f.check(v).ok()) {
                values.insert(f.key.clone(), v);
            }
        }
    }
    // A master field opens on the current master, unless it was given or the tab shown before had one.
    let carried = tabs
        .as_ref()
        .is_some_and(|t| t.saved.get(spec.id).or_else(|| t.saved.get(CARRY)).and_then(Value::as_object).is_some_and(|m| m.contains_key("master")));
    if !given.contains_key("master")
        && !carried
        && let Some(field) = fields.iter().find(|f| f.key == "master")
        && let Some(v) = app.session.font().ok().and_then(|f| current_master_id(f, &app.view.layers)).and_then(|m| field.check(&json!(m)).ok())
    {
        values.insert("master".into(), v);
    }
    if spec.mode == Mode::Preview && targets.is_empty() {
        return Err(format!("{}: select a glyph first", spec.title));
    }
    match spec.mode {
        Mode::Preview => app.session.begin(spec.title).map_err(|e| e.to_string())?,
        // The command acts on the active document as a whole.
        Mode::RunOnOk => {
            app.session.doc().map_err(|e| e.to_string())?;
        }
    }
    let serial = serial.unwrap_or_else(|| {
        app.dialogs.opened += 1;
        app.dialogs.opened
    });
    let mut dialog = OpenDialog {
        id,
        title: spec.title.to_string(),
        command: spec.command.to_string(),
        mode: spec.mode,
        buttons: spec.buttons,
        third: spec.third,
        message: spec.message,
        fields,
        values,
        targets,
        editing: None,
        error: None,
        actions: spec.actions,
        shaped,
        tabs,
        serial,
        revision: 0,
        remember,
        annotate: spec.annotate,
    };
    if let Some(s) = dialog.shaped {
        (s.link)(&mut dialog.values, "");
    }
    dialog.sync_locks();
    app.dialogs.open = Some(dialog);
    preview(app);
    Ok(inspect(app))
}

/// Shows the tab `index` of the open dialog: the tab showing is put away as Cancel would, and its values are kept for
/// when it shows again.
pub fn switch_tab(app: &mut FontCraftApp, index: usize) -> Result<Value, String> {
    let d = app.dialogs.open.as_ref().ok_or("no dialog is open")?;
    let Some(tabs) = &d.tabs else { return Err(format!("`{}` has no tabs", d.title)) };
    if index == tabs.active {
        return Ok(inspect(app));
    }
    let list = tabs.list;
    if list.get(index).is_none() {
        return Err(format!("the dialog has no tab {index}"));
    }
    commit_edit(app)?;
    let d = app.dialogs.open.take().ok_or("no dialog is open")?;
    if d.mode == Mode::Preview && app.session.interaction().is_some() {
        app.session.cancel().map_err(|e| e.to_string())?;
    }
    let mut saved = d.tabs.as_ref().map(|t| t.saved.clone()).unwrap_or_default();
    saved.insert(d.id.clone(), Value::Object(d.values.clone()));
    saved.insert(CARRY.into(), Value::Object(d.values.clone()));
    match open_tab_as(app, list, index, &d.targets, &saved, Some(d.serial)) {
        Ok(shown) => Ok(shown),
        Err(e) => {
            // The tab showing stays, unless putting its preview away already closed it.
            if d.mode == Mode::RunOnOk {
                app.dialogs.open = Some(d);
            }
            app.status = e.clone();
            Err(e)
        }
    }
}

/// What the command acts on when the dialog names nothing: when the person chose glyphs
/// ([`FontCraftApp::targets_chosen`]), what a fired command takes ([`FontCraftApp::with_targets`]: Font View's
/// selection while Font View shows with glyphs selected, else the canvas selection), else the glyph the glyph panels
/// show, as `glyph` for a command that acts on one layer and as `glyphs` for one that acts on glyphs.
fn selection_targets(app: &mut FontCraftApp, command: &str) -> Map<String, Value> {
    use fontcraft_engine::params::TargetKind;
    if !app.targets_chosen() {
        return Map::new();
    }
    let targeted = app.with_targets(CommandRef::new(command));
    if targeted.id == command
        && let Value::Object(m) = targeted.params
        && !m.is_empty()
    {
        return m;
    }
    let mut m = Map::new();
    let (Some(spec), Some(glyph)) = (fontcraft_engine::find_command(command), app.view.glyph.as_ref()) else { return m };
    if spec.targets.contains(&TargetKind::Layer) {
        m.insert("glyph".into(), json!(glyph));
    } else if spec.targets.contains(&TargetKind::Glyphs) {
        m.insert("glyphs".into(), json!([glyph]));
    }
    m
}

/// Runs the dialog's command on the font as it was when the dialog opened. A dialog that runs on OK only forgets the
/// failure of the last OK.
fn preview(app: &mut FontCraftApp) {
    let Some(d) = app.dialogs.open.as_mut() else { return };
    d.revision += 1;
    if let Some(annotate) = d.annotate {
        let shown = annotate(&app.session, &d.params());
        d.values.extend(shown);
    }
    if d.mode == Mode::RunOnOk {
        d.error = None;
        return;
    }
    let (command, params) = (d.command.clone(), d.params());
    let result = app.session.preview(&command, &params);
    if let Some(d) = app.dialogs.open.as_mut() {
        d.error = result.err().map(|e| e.to_string());
        if let Some(e) = &d.error {
            app.status = e.clone();
        }
    }
}

/// `ui.dialog.set`: gives a field a value and previews it → the `ui.dialog.inspect` reply.
pub fn set(app: &mut FontCraftApp, key: &str, value: &Value) -> Result<Value, String> {
    let d = app.dialogs.open.as_mut().ok_or("no dialog is open")?;
    let checked = d.field(key)?.check(value)?;
    d.put(key, checked);
    if d.editing.as_ref().is_some_and(|e| e.field == key) {
        d.editing = None;
    }
    preview(app);
    Ok(inspect(app))
}

/// Takes the text of the field being edited as its value. A text that is not a value puts the field back and is the
/// error.
fn commit_edit(app: &mut FontCraftApp) -> Result<(), String> {
    let Some(d) = app.dialogs.open.as_mut() else { return Ok(()) };
    let Some(editing) = d.editing.take() else { return Ok(()) };
    d.revision += 1;
    let field = d.field(&editing.field)?;
    let current = d.values.get(&editing.field).map(format_value).unwrap_or_default();
    if editing.text.trim() == current {
        return Ok(());
    }
    let value = field.check(&json!(editing.text)).inspect_err(|e| app.status = e.clone())?;
    d.put(&editing.field, value);
    preview(app);
    Ok(())
}

/// A button that answers the third button's question: gives its field `value`, then confirms.
fn answer(app: &mut FontCraftApp, value: Option<&'static str>, person: bool) -> Result<Value, String> {
    if let Some((key, value)) = app.dialogs.open.as_ref().and_then(OpenDialog::third).zip(value).map(|(t, v)| (t.key, v)) {
        commit_edit(app)?;
        set(app, key, &json!(value))?;
    }
    confirm_by(app, person)
}

/// OK in the window or `Return` there: may ask for a missing folder.
pub fn confirm_by_person(app: &mut FontCraftApp) -> Result<Value, String> {
    answer(app, app.dialogs.open.as_ref().and_then(OpenDialog::third).map(|t| t.ok), true)
}

/// The third button in the window.
pub fn third_by_person(app: &mut FontCraftApp) -> Result<Value, String> {
    answer(app, app.dialogs.open.as_ref().and_then(OpenDialog::third).map(|t| t.value), true)
}

/// `ui.dialog.confirm`: OK over the control channel.
pub fn confirm(app: &mut FontCraftApp) -> Result<Value, String> {
    confirm_by(app, false)
}

/// OK: the last preview becomes one undo step, or, in [`Mode::RunOnOk`], the command runs. `person`: the OK button or
/// `Return` in the window, which may ask for a missing folder.
fn confirm_by(app: &mut FontCraftApp, person: bool) -> Result<Value, String> {
    if app.dialogs.open.is_none() {
        return Err("no dialog is open".into());
    }
    commit_edit(app)?;
    let Some(d) = &app.dialogs.open else { return Err("no dialog is open".into()) };
    if d.mode == Mode::RunOnOk {
        return run_on_ok(app, person);
    }
    if let Some(e) = &d.error {
        return Err(format!("{}: the preview failed, so OK has nothing to keep: {e}", d.title));
    }
    let (title, command, params) = (d.title.clone(), d.command.clone(), d.params());
    let kept = d.remember.then(|| (d.id.clone(), d.values.clone()));
    let changed = app.session.commit().map_err(|e| e.to_string())?;
    if let Some((id, values)) = kept {
        app.dialogs.remember(id, values);
    }
    app.dialogs.open = None;
    app.status = format!("{title}: done");
    Ok(json!({"confirmed": true, "changed": changed, "command": command, "params": params}))
}

/// OK of a [`Mode::RunOnOk`] dialog: runs the command once, then opens the question its result asks for, as after a
/// menu item's run. A failure keeps the dialog open with the message. An empty folder field asks the folder chooser
/// when the person pressed OK, and OK follows the folder chosen; over the control channel it is an error.
fn run_on_ok(app: &mut FontCraftApp, person: bool) -> Result<Value, String> {
    let Some(d) = &app.dialogs.open else { return Err("no dialog is open".into()) };
    let missing = d.fields.iter().find(|f| f.needs_destination(d.values.get(&f.key).unwrap_or(&Value::Null))).map(|f| f.key.clone());
    if let Some(key) = missing {
        if person {
            choose(app, &key, true)?;
            return Ok(inspect(app));
        }
        let message = format!("`{key}` needs a folder: set it with ui.dialog.set");
        let title = d.title.clone();
        if let Some(d) = app.dialogs.open.as_mut() {
            d.error = Some(message.clone());
            d.revision += 1;
        }
        app.status = message.clone();
        return Err(format!("{title}: {message}"));
    }
    let (title, command, params) = (d.title.clone(), d.command.clone(), d.params());
    let kept = d.remember.then(|| (d.id.clone(), d.values.clone()));
    match app.session.execute(&command, &params) {
        Ok(result) => {
            if let Some((id, values)) = kept {
                app.dialogs.remember(id, values);
            }
            app.dialogs.open = None;
            app.status = format!("{title}: done");
            super::after::after_run(app, &command, &params, &result);
            Ok(json!({"confirmed": true, "command": command, "params": params, "result": result}))
        }
        Err(e) => {
            let e = e.to_string();
            if let Some(d) = app.dialogs.open.as_mut() {
                d.error = Some(e.clone());
                d.revision += 1;
            }
            app.status = e.clone();
            Err(format!("{title}: {e}"))
        }
    }
}

/// The chooser for the folder or file field `key`; the answer arrives on a later pass ([`poll_chooser`]). `then_ok`:
/// OK follows the path chosen.
pub fn choose(app: &mut FontCraftApp, key: &str, then_ok: bool) -> Result<(), String> {
    let d = app.dialogs.open.as_ref().ok_or("no dialog is open")?;
    let field = d.field(key)?;
    let request = match &field.kind {
        Kind::Folder => PickRequest::folder(field.label.clone()),
        Kind::File { save, extensions } => {
            let mut request = if *save { PickRequest::save(field.label.clone()) } else { PickRequest::open(field.label.clone()) };
            for extension in extensions {
                request = request.filter(extension.to_ascii_uppercase(), &[extension.as_str()]);
            }
            match extensions.first() {
                Some(first) if *save => request.suggested_extension(first.as_str()),
                _ => request,
            }
        }
        _ => return Err(format!("`{key}` is not a folder or file field")),
    };
    let request = request.title(field.label.clone());
    let serial = d.serial;
    let pick = app.host.files().pick(request);
    app.dialogs.choosing = Some((pick, serial, key.to_string(), then_ok));
    Ok(())
}

/// Takes the chooser's answer: the field gets the path, and OK follows when the person pressed it. A cancelled chooser
/// changes nothing.
pub fn poll_chooser(app: &mut FontCraftApp) {
    let Some((pick, serial, key, then_ok)) = app.dialogs.choosing.take() else { return };
    let Some(answer) = app.host.files().take_answer(pick) else {
        app.dialogs.choosing = Some((pick, serial, key, then_ok));
        return;
    };
    if app.dialogs.open.as_ref().is_none_or(|d| d.serial != serial) {
        return;
    }
    let result = match answer {
        Picked::Paths(paths) => match paths.first() {
            Some(path) => {
                set(app, &key, &json!(path.to_string_lossy())).and_then(|_| if then_ok { confirm_by(app, false).map(drop) } else { Ok(()) })
            }
            None => Ok(()),
        },
        Picked::Cancelled => Ok(()),
        other => Err(format!("`{key}`: no chooser ({other:?}); set the path with ui.dialog.set")),
    };
    if let Err(e) = result {
        app.status = e;
    }
}

/// An item of the actions menu: runs its query with the dialog's parameters and puts the `text` of the result out for
/// the system clipboard → `{action, command, text, result}`.
pub fn run_action(app: &mut FontCraftApp, id: &str) -> Result<Value, String> {
    let d = app.dialogs.open.as_ref().ok_or("no dialog is open")?;
    let action = *d.actions.iter().find(|a| a.id == id).ok_or_else(|| format!("the dialog has no action `{id}`"))?;
    commit_edit(app)?;
    let params = app.dialogs.open.as_ref().map_or(Value::Null, OpenDialog::params);
    let result = app.session.query(action.command, &params).map_err(|e| e.to_string())?;
    let text = result.get("text").and_then(Value::as_str).map(str::to_string);
    if let Some(t) = &text {
        app.dialogs.clipboard = Some(t.clone());
        app.status = format!("{}: copied", action.label);
    }
    Ok(json!({"action": action.id, "command": action.command, "text": text, "result": result}))
}

/// `ui.dialog.cancel` and Cancel: the font as it was when the dialog opened.
pub fn cancel(app: &mut FontCraftApp) -> Result<Value, String> {
    let d = app.dialogs.open.take().ok_or("no dialog is open")?;
    if d.mode == Mode::Preview && app.session.interaction().is_some() {
        app.session.cancel().map_err(|e| e.to_string())?;
    }
    app.status = format!("{}: cancelled", d.title);
    Ok(json!({"cancelled": true}))
}

/// Closes a dialog whose document went away under it: nothing is left to act on.
pub fn close_if_gone(app: &mut FontCraftApp) {
    let Some(d) = &app.dialogs.open else { return };
    let gone = match d.mode {
        Mode::Preview => app.session.interaction().is_none(),
        Mode::RunOnOk => app.session.doc().is_err(),
    };
    if gone {
        app.dialogs.open = None;
    }
}

/// `ui.dialog.inspect`: the open dialog, its fields and values, and the parameters OK runs.
pub fn inspect(app: &FontCraftApp) -> Value {
    let Some(d) = &app.dialogs.open else { return json!({"open": false}) };
    let fields: Vec<Value> = d
        .fields
        .iter()
        .map(|f| {
            let mut m = f.describe();
            m.insert("value".into(), d.values.get(&f.key).cloned().unwrap_or(Value::Null));
            m.insert("focused".into(), json!(d.editing.as_ref().is_some_and(|e| e.field == f.key)));
            m.insert("enabled".into(), json!(!d.locked(&f.key)));
            Value::Object(m)
        })
        .collect();
    let tabs = d.tabs.as_ref().map(|t| {
        t.list
            .iter()
            .enumerate()
            .map(|(i, tab)| json!({"id": tab.dialog.id, "tab": tab.id, "label": tab.label, "active": i == t.active}))
            .collect::<Vec<_>>()
    });
    let actions: Vec<Value> = d.actions.iter().map(|a| json!({"id": a.id, "label": a.label, "command": a.command})).collect();
    let caption = |given: &'static str, common: &'static str| if given.is_empty() { common } else { given };
    let (ok, cancel) = (caption(d.ok_caption(), "OK"), caption(d.buttons[1], "Cancel"));
    let mut captions = Map::new();
    captions.insert("ok".into(), json!(ok));
    let mut buttons = vec![json!("ok")];
    if let Some(t) = d.third() {
        captions.insert(t.id.into(), json!(t.caption));
        buttons.push(json!(t.id));
    }
    captions.insert("cancel".into(), json!(cancel));
    buttons.push(json!("cancel"));
    json!({
        "open": true,
        "dialog": d.id,
        "title": d.title,
        "command": d.command,
        "mode": d.mode.word(),
        "message": if d.message.is_empty() { Value::Null } else { json!(d.message) },
        "tabs": tabs,
        "fields": fields,
        "editing": d.editing.as_ref().map(|e| json!({"field": e.field, "text": e.text})),
        "params": d.params(),
        "error": d.error,
        "actions": if actions.is_empty() { Value::Null } else { json!(actions) },
        "buttons": buttons,
        "captions": captions,
        "canChooseFolder": true,
    })
}

/// `ui.dialog.click {id}`: presses a control of the open dialog: `ok`, `cancel`, the third button, a tab or an action
/// by its id, or a field by its key (a checkbox or lock toggles; a text field takes the keyboard with its text
/// selected; a folder or file field opens the chooser).
pub fn click(app: &mut FontCraftApp, id: &str) -> Result<Value, String> {
    let d = app.dialogs.open.as_ref().ok_or_else(|| format!("no control `{id}`: no dialog is open"))?;
    let third = d.third();
    match id {
        "ok" => return answer(app, third.map(|t| t.ok), false),
        "cancel" => return cancel(app),
        _ if third.is_some_and(|t| t.id == id) => return answer(app, third.map(|t| t.value), false),
        _ => {}
    }
    if let Some(index) = d.tabs.as_ref().and_then(|t| t.list.iter().position(|tab| tab.dialog.id == id)) {
        return switch_tab(app, index);
    }
    if d.actions.iter().any(|a| a.id == id) {
        return run_action(app, id);
    }
    let field = d.field(id)?.clone();
    match &field.kind {
        Kind::Bool | Kind::Lock => {
            let on = d.values.get(id).and_then(Value::as_bool).unwrap_or(false);
            commit_edit(app)?;
            set(app, id, &json!(!on))
        }
        Kind::Folder | Kind::File { .. } => {
            commit_edit(app)?;
            choose(app, id, false)?;
            Ok(inspect(app))
        }
        Kind::Number { .. } | Kind::Integer { .. } | Kind::Text { .. } => {
            commit_edit(app)?;
            focus(app, &field.key);
            Ok(inspect(app))
        }
        Kind::Choice { .. } => Err(format!("`{id}` is a pop-up menu: choose with ui.dialog.set")),
        Kind::Multi { .. } => Err(format!("`{id}` is a row of checkboxes: give the ticked words with ui.dialog.set")),
        Kind::Order { .. } => Err(format!("`{id}` is a list whose rows are dragged: give the order with ui.dialog.set")),
        Kind::List { select: true, .. } => Err(format!("`{id}` is a list: give the selected rows with ui.dialog.set")),
        Kind::List { select: false, .. } => Err(format!("`{id}` only shows its rows")),
        Kind::Custom { kind, .. } => Err(format!("`{id}` is a {}: give its value with ui.dialog.set", kind.name)),
    }
}

/// Gives the text field `key` the keyboard, its text selected.
fn focus(app: &mut FontCraftApp, key: &str) {
    if let Some(d) = app.dialogs.open.as_mut() {
        let text = d.values.get(key).map(format_value).unwrap_or_default();
        d.editing = Some(Editing { field: key.to_string(), text, replace: true });
        d.revision += 1;
    }
}

/// A key for the open dialog from the control channel: in a text field `Return` takes the typed value, `Escape` drops
/// it, `Tab` and `Shift+Tab` take it and move to the next or previous text field, `Up` and `Down` step a number by 1,
/// by 10 with `Shift`; with no field taking the keyboard `Return` is OK and `Escape` is Cancel. `None` when no dialog is
/// open.
pub fn key(app: &mut FontCraftApp, key: DialogKey, shift: bool) -> Option<Result<Value, String>> {
    let d = app.dialogs.open.as_mut()?;
    let editing = d.editing.clone();
    let result = match (key, editing) {
        (DialogKey::Return, Some(_)) => commit_edit(app).map(|()| inspect(app)),
        (DialogKey::Return, None) => confirm_by(app, false),
        (DialogKey::Escape, Some(_)) => {
            d.editing = None;
            d.revision += 1;
            Ok(inspect(app))
        }
        (DialogKey::Escape, None) => cancel(app),
        (DialogKey::Tab, editing) => {
            let text_fields: Vec<String> = d.fields.iter().filter(|f| f.is_text()).map(|f| f.key.clone()).collect();
            let at = editing.and_then(|e| text_fields.iter().position(|k| *k == e.field));
            let count = text_fields.len();
            let next = match (at, shift) {
                _ if count == 0 => None,
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
fn step(app: &mut FontCraftApp, editing: &Editing, up: bool, shift: bool) -> Result<Value, String> {
    let d = app.dialogs.open.as_ref().ok_or("no dialog is open")?;
    let field = d.field(&editing.field)?.clone();
    let current = field.check(&json!(editing.text)).or_else(|_| d.values.get(&field.key).cloned().ok_or("no value"))?;
    let Some(next) = field.step(&current, up, shift) else { return Ok(inspect(app)) };
    set(app, &field.key, &next)?;
    focus(app, &field.key);
    Ok(inspect(app))
}

/// `ui.text` with a text field taking the keyboard: types into it. `false` when none does.
pub fn type_text(app: &mut FontCraftApp, text: &str) -> bool {
    let Some(d) = app.dialogs.open.as_mut() else { return false };
    let max = d.editing.as_ref().and_then(|e| d.fields.iter().find(|f| f.key == e.field)).map_or(fields::MAX_FIELD_TEXT, Field::max_text);
    let Some(editing) = d.editing.as_mut() else { return false };
    if editing.replace {
        editing.text.clear();
        editing.replace = false;
    }
    let room = max.saturating_sub(editing.text.chars().count());
    editing.text.extend(text.chars().take(room));
    d.revision += 1;
    true
}

/// The shell command that opens a dialog.
pub const MODULE: crate::shell::commands::Module = crate::shell::commands::Module { commands: COMMANDS, run: run_command };

const COMMANDS: &[crate::shell::commands::UiCommand] = &[crate::shell::commands::UiCommand {
    id: "dialog.open",
    label: "Open Dialog",
    menu: &[],
    shortcut: None,
    params: "{dialog: id (e.g. \"dialog.filterRoundCorners\") | command: the id of the command it runs, params?: {…}} opens a command's dialog with a live preview. Parameters of its fields set their first values; the rest (glyph, glyphs, layer, points…) say what it acts on, else the selection does. Then ui.dialog.set, ui.dialog.confirm, ui.dialog.cancel → ui.dialog.inspect's reply",
}];

fn run_command(app: &mut FontCraftApp, command: &craft::ui::runtime::commands::CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "dialog.open" => open_command(app, &command.params).map(drop),
        other => Err(format!("no dialog command `{other}`")),
    }
}
