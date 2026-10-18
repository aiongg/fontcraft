//! Dialogs: a form over one command's parameters with a live preview (decision 0004, item 6).
//!
//! Opening a dialog begins an interaction ([`fontcraft_engine::Session::begin`]); every change
//! of a field previews the command with the dialog's parameters on the font as it was when the
//! dialog opened. OK commits the last preview as one undo step; Cancel puts back the very font
//! the dialog opened on. Agents can skip the dialog and run the command with the same
//! parameters: the result is the same font.
//!
//! A dialog is modal: a light backdrop (`dialog_backdrop`) covers the window behind it, which
//! takes no pointer input, hover or tooltip until the dialog closes.
//!
//! A dialog whose command acts outside the font (exporting writes files) has
//! [`Mode::RunOnOk`]: nothing runs while it is open, OK runs the command once as a menu item
//! would, and a command that fails keeps the dialog open with its message.
//!
//! A question asked after a command ([`Dialog::after`]) opens when the person runs that command
//! (menu, shortcut, tool, `ui.menu.invoke`, or OK of a [`Mode::RunOnOk`] dialog that runs it) and
//! its result calls for it; `engine.execute` (control channel, MCP) never opens one, as agents
//! read the result themselves.
//!
//! The fields come from the command's parameters: a filter's from its `filter.list` schema
//! ([`Fields::Filter`]), any other command's from a list in its dialog's file
//! ([`Fields::Fixed`]), a list that depends on what the command acts on ([`Fields::Targeted`]),
//! or fields that the dialog turns into the command's parameters ([`Fields::Shaped`]: two
//! number fields for one `{x, y}` parameter, a lock that keeps two fields equal). Every filter
//! with parameters gets a dialog generated from its schema; a file in this directory names a
//! dialog's spec row and can replace its fields.
//!
//! A dialog with tabs ([`Fields::Tabs`]) shows one tab at a time, each a dialog of its own
//! command; values typed on a tab come back when it shows again. A dialog's actions (the round
//! "…" menu at its bottom left) run a query on the dialog's parameters and put the text of the
//! result on the system clipboard.
//!
//! A folder field shows Choose… when the app has a folder chooser. OK with an empty folder field
//! opens the chooser when the person pressed it, and is an error over the control channel.
//!
//! Keys while a dialog is open (the field rules of `infoBox.fieldEditing` and
//! `infoBox.arrowKeyStepping`): in a number field, `Return` takes the typed value, `Escape`
//! drops it, `Tab` and `Shift+Tab` take it and move to the next or previous field, `Up` and
//! `Down` step the value by 1, by 10 with `Shift`. With no field taking the keyboard, `Return`
//! is OK and `Escape` is Cancel. The rest of the app takes no input until the dialog closes.

pub mod actions;
mod add_glyphs;
mod add_glyphs_existing;
mod add_other_font;
pub mod attach_picker;
mod axis_location;
mod axis_mappings;
pub mod baselines;
mod color_palettes;
pub mod compare_fonts;
pub mod cross_axis;
pub mod custom_parameter_picker;
mod export;
pub mod export_profiles;
mod feature_picker;
pub mod field_picker;
mod find_and_replace;
pub mod find_glyphs;
pub mod find_outline;
pub mod form;
mod guide_properties;
mod import_features;
mod import_metrics;
pub mod import_outlines_missing_glyph;
mod interpolate_with_background;
pub mod kerning_groups_editor;
pub mod kerning_pairs_preview;
mod languages_list;
mod list_filter_editor;
mod master_color;
pub mod master_icon_picker;
pub mod math_table;
pub mod metric_editor;
mod missing_glyphs;
pub mod paste_kerning_conflict;
mod paste_special;
pub mod pasted_outline_bounds;
pub mod primitive_measurements;
mod remove_glyph;
mod rename_kerning_group;
mod round_corners;
pub mod rule_tree;
pub mod sample_strings;
pub mod save_changed_on_disk;
mod shape_order;
mod shape_order_window;
pub mod smart_component_settings;
mod smart_filter_editor;
pub mod smart_glyph_settings;
pub mod space_center;
pub mod stem_filters;
mod transform_metrics;
mod transformations;
pub mod tt_debugger;
mod tt_deltas;
pub mod tt_instructions;
mod tt_preview_address;
mod ttf_stems;
mod ttf_zones;
mod virtual_master;

use serde::Serialize;
use serde_json::{Map, Value, json};

use self::form::{Field, Kind, format_value};
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
    pub mode: Mode,
    /// Asked only after another command (see `after`): never the dialog of its own command's
    /// menu item or of `dialog.open {command}`.
    pub follow_up: bool,
    /// The command whose result can ask this dialog, and the parameters to open it with.
    pub after: Option<After>,
    /// The captions of the confirm and cancel buttons.
    pub buttons: [&'static str; 2],
    /// A third button that answers the dialog's question another way.
    pub third: Option<Third>,
    /// A line above the fields; empty for none.
    pub message: &'static str,
    /// The items of the round "…" menu at the bottom left.
    pub actions: &'static [Action],
}

impl Dialog {
    /// A previewing dialog with OK and Cancel, no message and no actions.
    pub const fn new(id: &'static str, title: &'static str, command: &'static str, fields: Fields) -> Self {
        Self {
            id,
            title,
            command,
            fields,
            mode: Mode::Preview,
            follow_up: false,
            after: None,
            buttons: ["OK", "Cancel"],
            third: None,
            message: "",
            actions: &[],
        }
    }
}

/// A third button beside OK and Cancel, for a question with two answers that both go ahead
/// (Keep Existing or Replace; Remove or Decompose). It shows while the dialog has the field
/// `key`, which the buttons answer instead of a control of its own: OK gives it `ok`, the third
/// button gives it `value`, and either then confirms. `ui.click {id}` presses it. While it shows,
/// `ok_caption`, when given, names the OK button in place of the dialog's own caption.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Third {
    pub id: &'static str,
    pub caption: &'static str,
    pub key: &'static str,
    pub ok: &'static str,
    pub value: &'static str,
    pub ok_caption: Option<&'static str>,
}

/// An item of a dialog's "…" menu: a query run with the dialog's parameters whose `text` goes
/// to the system clipboard. `ui.click {id}` runs it too.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
}

/// One tab of a dialog with tabs: its id for `file.export {tab}`, its label, and its dialog.
#[derive(Clone, Copy, Debug)]
pub struct Tab {
    pub id: &'static str,
    pub label: &'static str,
    pub dialog: Dialog,
}

/// Field values from the parameters of `dialog.open`, read with the session (see [`Shaped::given`]).
pub type Given = fn(&fontcraft_engine::Session, &Map<String, Value>) -> Result<Map<String, Value>, String>;

/// Fields that are not the command's parameters one to one.
#[derive(Clone, Copy, Debug)]
pub struct Shaped {
    pub fields: fn(&fontcraft_engine::Session) -> Vec<Field>,
    /// Field values from parameters in the command's shape (`dialog.open`'s `params`); keys that
    /// are no field's pass through (the targets). The session answers what the parameters name
    /// (a stored filter); an error keeps the dialog closed.
    pub given: Given,
    /// The command's parameters from the field values.
    pub params: fn(&Map<String, Value>) -> Map<String, Value>,
    /// Brings linked fields in step after the field named by the `&str` took a value (an
    /// empty name when the dialog opens).
    pub link: fn(&mut Map<String, Value>, &str),
}

/// How OK applies the dialog's command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Every change previews the command; OK keeps the last preview as one undo step.
    Preview,
    /// Nothing runs until OK, which runs the command once (journaled, as a menu item runs it).
    RunOnOk,
}

/// The parameters of a follow-up dialog from the request and the result of the command that
/// asks it; `None` when the result asks nothing.
pub type FollowUp = fn(&Value, &Value) -> Option<Map<String, Value>>;

/// The command that asks a follow-up dialog.
#[derive(Clone, Copy, Debug)]
pub struct After {
    pub command: &'static str,
    pub params: FollowUp,
}

/// Where a dialog's fields come from.
/// Builds the fields of [`Fields::Targeted`] from the session and the dialog's targets.
pub type TargetedFields = fn(&mut fontcraft_engine::Session, &Map<String, Value>) -> Result<Vec<Field>, String>;

#[derive(Clone, Copy, Debug)]
pub enum Fields {
    /// The parameters of the filter `command`, as `filter.list` describes them.
    Filter,
    /// A list for a command that is not a filter. The session gives values that depend on the
    /// font (a default taken from the font's metrics).
    Fixed(fn(&fontcraft_engine::Session) -> Vec<Field>),
    /// A list that depends on what the command acts on: the targets the dialog opens with (the
    /// given ones, else the selection). The session answers queries such as `layer.inspect`.
    Targeted(TargetedFields),
    /// Fields the dialog shapes into the command's parameters.
    Shaped(Shaped),
    /// One dialog per tab; the dialog opens on the first.
    Tabs(&'static [Tab]),
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
    shape_order::DIALOG,
    export::DIALOG,
    export::OTF,
    export::UFO,
    guide_properties::DIALOG,
    // The editors of stored filters come before those of new ones: `find` gives the first.
    smart_filter_editor::DIALOG,
    smart_filter_editor::NEW,
    list_filter_editor::DIALOG,
    list_filter_editor::NEW,
    languages_list::DIALOG,
    missing_glyphs::DIALOG,
    add_glyphs::DIALOG,
    add_glyphs_existing::DIALOG,
    remove_glyph::DIALOG,
    paste_special::DIALOG,
    find_and_replace::DIALOG,
    rename_kerning_group::DIALOG,
    paste_kerning_conflict::DIALOG,
    save_changed_on_disk::DIALOG,
    transform_metrics::DIALOG,
    // Metrics export and import (the tab dialog first: the menu item opens it).
    export::METRICS,
    import_metrics::DIALOG,
    import_metrics::FILE,
    import_metrics::FONT,
    import_features::DIALOG,
    stem_filters::ADJUST,
    stem_filters::WEIGHT,
    stem_filters::CONDENSE,
    stem_filters::X_HEIGHT,
    export_profiles::DIALOG,
    export_profiles::EXPORT,
    export_profiles::ARTWORK,
];

/// The shell commands of this module.
pub const COMMANDS: &[UiCommand] = &[
    (
        "dialog.open",
        "Open Dialog",
        &[],
        "",
        "{dialog: id (e.g. \"dialog.filterRoundCorners\") | command: the id of the command it runs, params?: {…}} opens a command's dialog with a live preview. Parameters of its fields set their first values; the rest (glyph, glyphs, layer, points…) say what it acts on, else the selection does. Then ui.dialog.set, ui.dialog.confirm, ui.dialog.cancel → ui.dialog.inspect's reply",
    ),
    (
        "file.export",
        "Export…",
        &["File"],
        "Cmd+E",
        "{tab?: \"otf\" (default) | \"ufo\" | \"metrics\", params?: {…}} opens the export dialog of the active document on a tab: static OpenType (file.exportStatic), UFO (file.exportUfo) or metrics (file.exportMetrics); params set its first values. Nothing is written until OK; OK with no destination folder asks for one → ui.dialog.inspect's reply",
    ),
];

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
        "file.export" => {
            let params = match args.get("params") {
                None => Map::new(),
                Some(Value::Object(m)) => m.clone(),
                Some(_) => return Err(args.err("`params` must be a JSON object")),
            };
            let tab = args.string("tab")?.unwrap_or("otf");
            match export::TABS.iter().position(|t| t.id == tab) {
                Some(index) => open_tab(app, &export::TABS, index, &params, &Map::new()).map_err(|e| args.err(e)),
                None if tab == "variable" => Err(args.err(format!("the `{tab}` tab is not built yet"))),
                None => Err(args.err(format!("unknown tab `{tab}`: otf, variable, ufo or metrics"))),
            }
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
    pub mode: Mode,
    pub buttons: [&'static str; 2],
    pub third: Option<Third>,
    pub message: &'static str,
    pub fields: Vec<Field>,
    /// Each field's value, by key.
    pub values: Map<String, Value>,
    /// What the command acts on (glyph, glyphs, layer, points…), fixed when the dialog opened.
    pub targets: Map<String, Value>,
    /// The number field taking the keyboard.
    pub editing: Option<Editing>,
    /// Why the last preview failed (OK waits until a preview succeeds), or, in
    /// [`Mode::RunOnOk`], why OK's run failed (until a field changes).
    pub error: Option<String>,
    /// A key took the keyboard away from a field: the next frame unfocuses its widget.
    #[serde(skip)]
    pub(crate) blur: bool,
    #[serde(skip)]
    pub actions: &'static [Action],
    /// How the field values become the command's parameters, for a [`Fields::Shaped`] dialog.
    #[serde(skip)]
    pub shaped: Option<Shaped>,
    /// The tabs, when the dialog is one tab of several.
    #[serde(skip)]
    pub tabs: Option<Tabs>,
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
    /// Choose… of a folder field.
    Choose(String),
    /// An item of the "…" menu, by its id.
    Action(String),
    /// A tab, by its index.
    Tab(usize),
    Confirm,
    /// The third button.
    Third,
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
    /// The command's parameters: the targets and every field's value (shaped, for a
    /// [`Fields::Shaped`] dialog). An empty folder field is left out.
    pub fn params(&self) -> Value {
        let values: Map<String, Value> = self
            .values
            .iter()
            .filter(|(k, v)| !self.fields.iter().any(|f| f.key == **k && f.is_empty_folder(v)))
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
    }

    /// The third button, while the dialog has the field it answers.
    pub fn third(&self) -> Option<Third> {
        self.third.filter(|t| self.fields.iter().any(|f| f.key == t.key))
    }

    /// The OK button's caption.
    pub fn ok_caption(&self) -> &'static str {
        self.third().and_then(|t| t.ok_caption).unwrap_or(self.buttons[0])
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

/// The dialog of the command `command`, if it has one. Follow-up questions are not.
pub fn for_command(command: &str) -> Option<Dialog> {
    if let Some(d) = DIALOGS.iter().find(|d| d.command == command && !d.follow_up) {
        return Some(*d);
    }
    filters().into_iter().find(|(c, _)| *c == command).map(|(command, title)| generated(command, title))
}

fn generated(command: &'static str, title: &'static str) -> Dialog {
    Dialog::new("", title, command, Fields::Filter)
}

/// The follow-up dialog that `command`, run with `request` and returning `result`, asks, with
/// the parameters to open it with.
pub fn after(command: &str, request: &Value, result: &Value) -> Option<(Dialog, Map<String, Value>)> {
    DIALOGS.iter().find_map(|d| {
        let after = d.after.filter(|a| a.command == command)?;
        (after.params)(request, result).map(|given| (*d, given))
    })
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
/// the selection, and the first preview. A dialog with tabs opens on its first tab; a tab opens
/// with its siblings.
pub fn open(app: &mut FontcraftApp, dialog: &Dialog, given: &Map<String, Value>) -> Result<Value, String> {
    if let Fields::Tabs(list) = dialog.fields {
        return open_tab(app, list, 0, given, &Map::new());
    }
    if let Some((list, index)) = tab_of(dialog.id) {
        return open_tab(app, list, index, given, &Map::new());
    }
    open_one(app, dialog, given, None)
}

/// The tab list that holds the dialog `id`, and its index there.
fn tab_of(id: &str) -> Option<(&'static [Tab], usize)> {
    DIALOGS.iter().find_map(|d| match d.fields {
        Fields::Tabs(list) => list.iter().position(|t| t.dialog.id == id).map(|i| (list, i)),
        _ => None,
    })
}

/// Opens the tab `index` of `list`. `saved`: the values of the tabs shown before ([`Tabs::saved`]).
fn open_tab(
    app: &mut FontcraftApp,
    list: &'static [Tab],
    index: usize,
    given: &Map<String, Value>,
    saved: &Map<String, Value>,
) -> Result<Value, String> {
    let tab = list.get(index).ok_or_else(|| format!("the dialog has no tab {index}"))?;
    if let Fields::Tabs(_) = tab.dialog.fields {
        return Err(format!("the tab `{}` holds tabs of its own", tab.id));
    }
    open_one(app, &tab.dialog, given, Some(Tabs { list, active: index, saved: saved.clone() }))
}

fn end_tool(app: &mut FontcraftApp) {
    let out = app.tools.toolbox.cancel();
    crate::tools_glue::apply(app, out);
}

fn open_one(app: &mut FontcraftApp, dialog: &Dialog, given: &Map<String, Value>, tabs: Option<Tabs>) -> Result<Value, String> {
    if let Some(d) = &app.dialog {
        return Err(format!("the dialog `{}` is open: confirm or cancel it first", d.title));
    }
    let mut shaped = None;
    let mut given = given.clone();
    let fields = match dialog.fields {
        Fields::Filter => form::filter_fields(&filter_entry(dialog.command).ok_or_else(|| format!("`{}` is not a filter", dialog.command))?)?,
        Fields::Fixed(list) => list(&app.session),
        Fields::Targeted(list) => {
            // The fields are built with a running drag left as it is: one that measures the font
            // does so on a scratch session, so a dialog that does not open leaves the drag running.
            let mut targets = given.clone();
            if !targets.contains_key("glyph") && !targets.contains_key("glyphs") {
                targets.extend(selection_targets(app, dialog.command));
            }
            list(&mut app.session, &targets)?
        }
        Fields::Shaped(s) => {
            shaped = Some(s);
            given = (s.given)(&app.session, &given)?;
            (s.fields)(&app.session)
        }
        Fields::Tabs(_) => return Err(format!("`{}` has tabs: open one of them", dialog.title)),
    };
    if fields.len() > form::MAX_FIELDS {
        return Err(format!("a dialog has at most {} fields", form::MAX_FIELDS));
    }
    let mut values = form::initial_values(&fields, &given)?;
    let mut targets: Map<String, Value> = given.iter().filter(|(k, _)| !values.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    if targets.is_empty() {
        targets = selection_targets(app, dialog.command);
    }
    // A tab shown before gets its values back; one shown first takes the values of the fields
    // it shares with the tab shown before it.
    if let Some(t) = &tabs {
        let earlier = t.saved.get(dialog.id).and_then(Value::as_object).or_else(|| t.saved.get(CARRY).and_then(Value::as_object));
        for f in &fields {
            if given.contains_key(&f.key) {
                continue;
            }
            if let Some(v) = earlier.and_then(|m| m.get(&f.key)).and_then(|v| f.check(v).ok()) {
                values.insert(f.key.clone(), v);
            }
        }
    }
    // A master field opens on the master the toolbar shows, unless it was given or the tab
    // shown before had one.
    let carried = tabs
        .as_ref()
        .is_some_and(|t| t.saved.get(dialog.id).or_else(|| t.saved.get(CARRY)).and_then(Value::as_object).is_some_and(|m| m.contains_key("master")));
    if !given.contains_key("master")
        && !carried
        && let Some(field) = fields.iter().find(|f| f.key == "master")
        && let Some(v) = import_metrics::current_master(app).and_then(|m| field.check(&json!(m)).ok())
    {
        values.insert("master".into(), v);
    }
    if dialog.mode == Mode::Preview && targets.is_empty() {
        return Err(format!("{}: select a glyph first", dialog.title));
    }
    // The canvas takes no input while the dialog is open: end what the tool was doing and drop
    // its hover marker, which would otherwise stay drawn under the dialog. Only now, so that a
    // dialog that does not open leaves a drag running.
    end_tool(app);
    match dialog.mode {
        Mode::Preview => app.session.begin(dialog.title).map_err(|e| e.to_string())?,
        // The command acts on the active document as a whole.
        Mode::RunOnOk => {
            app.session.doc().map_err(|e| e.to_string())?;
        }
    }
    let id = if dialog.id.is_empty() { generated_id(dialog.command) } else { dialog.id.to_string() };
    app.dialog = Some(OpenDialog {
        id,
        title: dialog.title.to_string(),
        command: dialog.command.to_string(),
        mode: dialog.mode,
        buttons: dialog.buttons,
        third: dialog.third,
        message: dialog.message,
        fields,
        values,
        targets,
        editing: None,
        error: None,
        blur: false,
        actions: dialog.actions,
        shaped,
        tabs,
    });
    if let Some(d) = app.dialog.as_mut()
        && let Some(s) = d.shaped
    {
        (s.link)(&mut d.values, "");
    }
    preview(app);
    Ok(inspect(app))
}

/// The key of [`Tabs::saved`] that holds the values of the tab shown last, for a tab shown for
/// the first time.
const CARRY: &str = "";

/// Shows the tab `index` of the open dialog: the tab showing is put away as Cancel would, and
/// its values are kept for when it shows again.
fn switch_tab(app: &mut FontcraftApp, index: usize) -> Result<Value, String> {
    let d = app.dialog.as_ref().ok_or("no dialog is open")?;
    let Some(tabs) = &d.tabs else { return Err(format!("`{}` has no tabs", d.title)) };
    if index == tabs.active {
        return Ok(inspect(app));
    }
    let list = tabs.list;
    if list.get(index).is_none() {
        return Err(format!("the dialog has no tab {index}"));
    }
    commit_edit(app)?;
    let d = app.dialog.take().ok_or("no dialog is open")?;
    if d.mode == Mode::Preview && app.session.interaction().is_some() {
        app.session.cancel().map_err(|e| e.to_string())?;
    }
    let mut saved = d.tabs.as_ref().map(|t| t.saved.clone()).unwrap_or_default();
    saved.insert(d.id.clone(), Value::Object(d.values.clone()));
    saved.insert(CARRY.into(), Value::Object(d.values.clone()));
    match open_tab(app, list, index, &d.targets, &saved) {
        Ok(shown) => Ok(shown),
        Err(e) => {
            // The tab showing stays, unless putting its preview away already closed it.
            if d.mode == Mode::RunOnOk {
                app.dialog = Some(d);
            }
            app.state.status = e.clone();
            Err(e)
        }
    }
}

/// What the command acts on when the dialog names nothing: the Edit View selection, else the
/// glyphs selected in Font View.
fn selection_targets(app: &FontcraftApp, command: &str) -> Map<String, Value> {
    // Import Metrics' Selected Glyphs are the glyphs selected in Font View, also while a tab
    // shows.
    if command == import_metrics::FILE.command {
        let mut m = Map::new();
        if !app.state.selected_glyphs.is_empty() {
            m.insert("glyphs".into(), json!(app.state.selected_glyphs));
        }
        return m;
    }
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

/// Runs the dialog's command on the font as it was when the dialog opened. A dialog that runs on
/// OK only forgets the failure of the last OK.
fn preview(app: &mut FontcraftApp) {
    let Some(d) = app.dialog.as_mut() else { return };
    if d.mode == Mode::RunOnOk {
        d.error = None;
        return;
    }
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
    d.put(key, checked);
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
    d.put(&editing.field, value);
    preview(app);
    Ok(())
}

/// A button that answers the third button's question: gives its field `value`, then confirms.
fn answer(app: &mut FontcraftApp, value: Option<&'static str>, person: bool) -> Result<Value, String> {
    if let Some((key, value)) = app.dialog.as_ref().and_then(OpenDialog::third).zip(value).map(|(t, v)| (t.key, v)) {
        commit_edit(app)?;
        set(app, key, &json!(value))?;
    }
    confirm_by(app, person)
}

/// `ui.dialog.confirm`: OK over the control channel (see [`confirm_by`]).
pub fn confirm(app: &mut FontcraftApp) -> Result<Value, String> {
    confirm_by(app, false)
}

/// OK: the last preview becomes one undo step, or, in [`Mode::RunOnOk`], the command runs.
/// `person`: the OK button or `Return` in the window, which may ask for a missing folder.
pub(crate) fn confirm_by(app: &mut FontcraftApp, person: bool) -> Result<Value, String> {
    if app.dialog.is_none() {
        return Err("no dialog is open".into());
    }
    commit_edit(app)?;
    let Some(d) = &app.dialog else { return Err("no dialog is open".into()) };
    if d.mode == Mode::RunOnOk {
        return run_on_ok(app, person);
    }
    if let Some(e) = &d.error {
        return Err(format!("{}: the preview failed, so OK has nothing to keep: {e}", d.title));
    }
    let (title, command, params) = (d.title.clone(), d.command.clone(), d.params());
    let changed = app.session.commit().map_err(|e| e.to_string())?;
    app.dialog = None;
    app.state.status = format!("{title}: done");
    Ok(json!({"confirmed": true, "changed": changed, "command": command, "params": params}))
}

/// OK of a [`Mode::RunOnOk`] dialog: runs the command once, then opens the question its result
/// asks for, as after a menu item's run. A failure keeps the dialog open with the message. An empty folder field is asked for with the folder chooser when the person
/// pressed OK (`export.promptWhenNoDestination`); over the control channel it is an error.
fn run_on_ok(app: &mut FontcraftApp, person: bool) -> Result<Value, String> {
    let Some(d) = &app.dialog else { return Err("no dialog is open".into()) };
    let missing = d.fields.iter().find(|f| f.is_empty_folder(d.values.get(&f.key).unwrap_or(&Value::Null))).map(|f| f.key.clone());
    if let Some(key) = missing {
        if person && app.picker.is_some() {
            if !choose(app, &key)? {
                // The person cancelled the chooser: nothing runs, the dialog stays.
                return Ok(inspect(app));
            }
        } else {
            let message = format!("`{key}` needs a folder: set it with ui.dialog.set");
            let title = d.title.clone();
            if let Some(d) = app.dialog.as_mut() {
                d.error = Some(message.clone());
            }
            app.state.status = message.clone();
            return Err(format!("{title}: {message}"));
        }
    }
    let Some(d) = &app.dialog else { return Err("no dialog is open".into()) };
    let (title, command, params) = (d.title.clone(), d.command.clone(), d.params());
    match app.run(&command, &params) {
        Ok(result) => {
            app.dialog = None;
            app.state.status = format!("{title}: done");
            if let Some((next, given)) = after(&command, &params, &result)
                && let Err(e) = open(app, &next, &given)
            {
                app.state.status = e;
            }
            Ok(json!({"confirmed": true, "command": command, "params": params, "result": result}))
        }
        Err(e) => {
            if let Some(d) = app.dialog.as_mut() {
                d.error = Some(e.clone());
            }
            app.state.status = e.clone();
            Err(format!("{title}: {e}"))
        }
    }
}

/// Choose… of the folder field `key`: asks the app's folder chooser and gives the field the
/// folder chosen. `false` when the person cancelled.
fn choose(app: &mut FontcraftApp, key: &str) -> Result<bool, String> {
    let d = app.dialog.as_ref().ok_or("no dialog is open")?;
    let field = d.field(key)?;
    if field.kind != Kind::Folder {
        return Err(format!("`{key}` is not a folder field"));
    }
    let title = field.label.clone();
    let picker = app.picker.as_mut().ok_or_else(|| format!("`{key}`: this window has no folder chooser; set the folder with ui.dialog.set"))?;
    let Some(path) = picker.folder(&title) else { return Ok(false) };
    set(app, key, &json!(path.to_string_lossy()))?;
    Ok(true)
}

/// An item of the "…" menu: runs its query with the dialog's parameters and puts the `text` of
/// the result on the system clipboard → `{action, command, text, result}`.
fn run_action(app: &mut FontcraftApp, id: &str) -> Result<Value, String> {
    let d = app.dialog.as_ref().ok_or("no dialog is open")?;
    let action = *d.actions.iter().find(|a| a.id == id).ok_or_else(|| format!("the dialog has no action `{id}`"))?;
    commit_edit(app)?;
    let params = app.dialog.as_ref().map_or(Value::Null, OpenDialog::params);
    let result = app.run(action.command, &params)?;
    let text = result.get("text").and_then(Value::as_str).map(str::to_string);
    if let Some(t) = &text {
        app.clipboard_out = Some(t.clone());
        app.state.status = format!("{}: copied", action.label);
    }
    Ok(json!({"action": action.id, "command": action.command, "text": text, "result": result}))
}

/// `ui.dialog.cancel` and Cancel: the font as it was when the dialog opened.
pub fn cancel(app: &mut FontcraftApp) -> Result<Value, String> {
    let d = app.dialog.take().ok_or("no dialog is open")?;
    if d.mode == Mode::Preview && app.session.interaction().is_some() {
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
    let tabs = d.tabs.as_ref().map(|t| {
        t.list
            .iter()
            .enumerate()
            .map(|(i, tab)| json!({"id": tab.dialog.id, "tab": tab.id, "label": tab.label, "active": i == t.active}))
            .collect::<Vec<_>>()
    });
    json!({
        "open": true,
        "dialog": d.id,
        "title": d.title,
        "command": d.command,
        "mode": d.mode,
        "message": if d.message.is_empty() { Value::Null } else { json!(d.message) },
        "tabs": tabs,
        "fields": fields,
        "editing": d.editing,
        "params": d.params(),
        "error": d.error,
        "actions": if d.actions.is_empty() { Value::Null } else { json!(d.actions) },
        "buttons": match d.third() {
            Some(t) => json!(["ok", t.id, "cancel"]),
            None => json!(["ok", "cancel"]),
        },
        "captions": match d.third() {
            Some(t) => json!({"ok": d.ok_caption(), t.id: t.caption, "cancel": d.buttons[1]}),
            None => json!({"ok": d.ok_caption(), "cancel": d.buttons[1]}),
        },
        "canChooseFolder": app.picker.is_some(),
    })
}

/// `ui.click`: presses a control of the open dialog: `ok`, `cancel`, a tab or an action by its
/// id, or a field by its key (a checkbox toggles; a number field takes the keyboard with its text
/// selected; a folder field opens the folder chooser, or without one takes the keyboard).
pub fn click(app: &mut FontcraftApp, id: &str) -> Result<Value, String> {
    let d = app.dialog.as_ref().ok_or_else(|| format!("no control `{id}`: no dialog is open"))?;
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
        Kind::Folder if app.picker.is_some() => {
            commit_edit(app)?;
            choose(app, id)?;
            Ok(inspect(app))
        }
        Kind::Number { .. } | Kind::Integer { .. } | Kind::Text { .. } | Kind::Folder => {
            commit_edit(app)?;
            focus(app, &field.key);
            Ok(inspect(app))
        }
        Kind::Choice { .. } => Err(format!("`{id}` is a pop-up menu: choose with ui.dialog.set")),
        Kind::Multi { .. } => Err(format!("`{id}` is a row of checkboxes: give the ticked words with ui.dialog.set")),
        Kind::Order { .. } => Err(format!("`{id}` is a list whose rows are dragged: give the order with ui.dialog.set")),
        Kind::List { select: true, .. } => Err(format!("`{id}` is a list: give the selected rows with ui.dialog.set")),
        Kind::List { select: false, .. } => Err(format!("`{id}` only shows its rows")),
        Kind::Rules => Err(format!("`{id}` is a rule tree: give the rules with ui.dialog.set")),
    }
}

/// Gives the text field `key` the keyboard, its text selected.
fn focus(app: &mut FontcraftApp, key: &str) {
    if let Some(d) = app.dialog.as_mut() {
        let text = d.values.get(key).map(format_value).unwrap_or_default();
        d.editing = Some(Editing { field: key.to_string(), text, replace: true });
        d.blur = false;
    }
}

/// A key for the open dialog from the control channel (see the module docs). `None` when no
/// dialog is open.
pub fn key(app: &mut FontcraftApp, key: DialogKey, shift: bool) -> Option<Result<Value, String>> {
    key_by(app, key, shift, false)
}

/// A key for the open dialog; `person`: it came from the keyboard, so `Return` is the person's OK.
fn key_by(app: &mut FontcraftApp, key: DialogKey, shift: bool, person: bool) -> Option<Result<Value, String>> {
    let d = app.dialog.as_mut()?;
    let editing = d.editing.clone();
    let result = match (key, editing) {
        (DialogKey::Return, Some(_)) => {
            d.blur = true;
            commit_edit(app).map(|()| inspect(app))
        }
        (DialogKey::Return, None) => confirm_by(app, person),
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

/// `ui.text` with a text field taking the keyboard: types into it. `false` when none does.
pub fn type_text(app: &mut FontcraftApp, text: &str) -> bool {
    let Some(d) = app.dialog.as_mut() else { return false };
    let Some(editing) = d.editing.as_mut() else { return false };
    let max = d.fields.iter().find(|f| f.key == editing.field).map_or(form::MAX_FIELD_TEXT, Field::max_text);
    if editing.replace {
        editing.text.clear();
        editing.replace = false;
    }
    let room = max.saturating_sub(editing.text.chars().count());
    editing.text.extend(text.chars().take(room));
    true
}

/// The dialog's keys among this frame's events, taken before anything else sees them. `Tab`
/// stays: egui moves the keyboard between the fields itself.
pub fn frame_keys(app: &mut FontcraftApp, ctx: &egui::Context) {
    let Some(d) = &app.dialog else { return };
    // Cmd+A selects every row of the dialog's list while no text field takes the keyboard,
    // before the menu shortcuts see it.
    let list = d.fields.iter().find_map(|f| match &f.kind {
        Kind::List { rows, select: true } => Some((f.key.clone(), json!(rows))),
        _ => None,
    });
    if let Some((key, all)) = list
        && d.editing.is_none()
        && !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A))
        && let Err(e) = set(app, &key, &all)
    {
        app.state.status = e;
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
        // In a box of several lines Return and the arrows belong to the text; Cmd+Return is OK.
        let multiline =
            app.dialog.as_ref().is_some_and(|d| d.editing.as_ref().is_some_and(|e| d.fields.iter().any(|f| f.key == e.field && f.is_multiline())));
        if multiline && k != DialogKey::Escape {
            if k == DialogKey::Return && modifiers.command {
                let result = commit_edit(app).and_then(|()| answer(app, app.dialog.as_ref().and_then(OpenDialog::third).map(|t| t.ok), true));
                if let Err(e) = result {
                    app.state.status = e;
                }
                used.push(index);
            }
            continue;
        }
        if let Some(Err(e)) = key_by(app, k, modifiers.shift, true) {
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
    let gone = match d.mode {
        Mode::Preview => app.session.interaction().is_none(),
        Mode::RunOnOk => app.session.doc().is_err(),
    };
    if gone {
        // The document closed under the dialog: nothing is left to act on.
        app.dialog = None;
        return;
    }
    let tokens = &app.tokens;
    let can_choose = app.picker.is_some();
    let mut events = Vec::new();
    let mut menu_events = Vec::new();
    // A modal area: the window behind it takes no pointer input, so no canvas hover or tooltip
    // reacts while the dialog is open.
    let area = egui::Modal::default_area(egui::Id::new("fontcraft-dialog")).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, tokens.dialog_top));
    egui::Modal::new(egui::Id::new("fontcraft-dialog"))
        .area(area)
        .backdrop_color(tokens.dialog_backdrop)
        .frame(egui::Frame::window(ui.style()))
        .show(ui.ctx(), |ui| {
            ui.label(egui::RichText::new(d.title.as_str()).strong().size(tokens.dialog_title_size));
            ui.add_space(tokens.dialog_spacing);
            if let Some(tabs) = &d.tabs {
                ui.horizontal(|ui| {
                    for (i, tab) in tabs.list.iter().enumerate() {
                        if ui.selectable_label(i == tabs.active, tab.label).clicked() && i != tabs.active {
                            events.push(Event::Tab(i));
                        }
                    }
                });
                ui.separator();
            }
            if !d.message.is_empty() {
                ui.label(d.message);
                ui.add_space(tokens.dialog_spacing);
            }
            form::draw(ui, d, tokens, can_choose, &mut events);
            if let Some(e) = &d.error {
                ui.colored_label(tokens.error_text, e);
            }
            ui.add_space(tokens.dialog_spacing);
            egui::Sides::new().show(
                ui,
                |ui| {
                    if !d.actions.is_empty() {
                        // A round button with three dots drawn at its centre: the "…" glyph sits on
                        // the text baseline, low in the button.
                        let size = ui.spacing().interact_size.y;
                        let button = egui::Button::new("")
                            .min_size(egui::vec2(size, size))
                            .corner_radius(size / 2.0)
                            .stroke(egui::Stroke::new(1.0, tokens.text_dim));
                        let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                            for a in d.actions {
                                if ui.button(a.label).clicked() {
                                    menu_events.push(Event::Action(a.id.to_string()));
                                    ui.close();
                                }
                            }
                        });
                        let response = response.on_hover_text("Actions");
                        let color = ui.style().interact(&response).fg_stroke.color;
                        let c = response.rect.center();
                        for dx in [-1.0, 0.0, 1.0] {
                            ui.painter().circle_filled(c + egui::vec2(dx * tokens.dialog_dots_gap, 0.0), tokens.dialog_dot_radius, color);
                        }
                    }
                },
                |ui| {
                    // Right to left: OK on the right, then the third button, then Cancel.
                    let (ok, cancel) = (d.ok_caption(), d.buttons[1]);
                    let can_confirm = d.error.is_none() || d.mode == Mode::RunOnOk;
                    if ui.add_enabled(can_confirm, egui::Button::new(ok)).clicked() {
                        events.push(Event::Confirm);
                    }
                    if let Some(t) = d.third()
                        && ui.add_enabled(can_confirm, egui::Button::new(t.caption)).clicked()
                    {
                        events.push(Event::Third);
                    }
                    if ui.button(cancel).clicked() {
                        events.push(Event::Cancel);
                    }
                },
            );
        });
    if let Some(d) = app.dialog.as_mut() {
        d.blur = false;
    }
    events.extend(menu_events);
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
            Event::Choose(key) => commit_edit(app).and_then(|()| choose(app, &key)).map(drop),
            Event::Action(id) => run_action(app, &id).map(drop),
            Event::Tab(index) => switch_tab(app, index).map(drop),
            Event::Confirm => answer(app, app.dialog.as_ref().and_then(OpenDialog::third).map(|t| t.ok), true).map(drop),
            Event::Third => answer(app, app.dialog.as_ref().and_then(OpenDialog::third).map(|t| t.value), true).map(drop),
            Event::Cancel => cancel(app).map(drop),
        };
        if let Err(e) = result {
            app.state.status = e;
        }
    }
}
