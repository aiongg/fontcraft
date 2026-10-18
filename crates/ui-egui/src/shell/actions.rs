//! The Actions dialog (`dialog.actions`): any command that takes `glyphs` run over a glyph scope
//! and layers with a before-and-after preview, and a sidebar that builds a macro. The dialog is
//! drawn by `crate::dialogs::actions`.
//!
//! - The actions are the editing commands whose targets include `glyphs`, grouped by menu.
//! - An action's parameters are the fields of its own dialog (`crate::dialogs::for_command`) when
//!   that dialog's fields can be drawn here (numbers, checkboxes, locks, pop-ups, check lists and
//!   one-line text), else a JSON object whose keys are checked against the command's params doc.
//! - The scope gives `glyphs`: the edited glyph, the selected glyphs, the glyphs of a category or
//!   every glyph. The layer scope gives `layers`: the active layer, the visible layers or every
//!   master. A command that takes only one `layer` runs once per layer; one that takes
//!   `allMasters` gets it for every master; one that takes no layer ignores the layer scope.
//! - The preview runs the action on the preview glyph alone, inside an interaction that is
//!   cancelled straight away, so the font and its undo history stay as they were.
//! - Run runs the action as one undo step. The sidebar holds an action set: steps whose `glyphs`
//!   and `layers` are the built-in macro arguments, so a saved set runs on any scope with
//!   `script.run`. Run Set runs it through `script.runActionSet`.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, LazyLock};

use fontcraft_engine::CommandSpec;
use fontcraft_engine::macros::{self, MAX_MACRO_STEPS, MAX_MACRO_TEXT, Macro, Step};
use fontcraft_engine::params::TargetKind;
use fontcraft_model::{Font, MAX_GLYPHS};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::dialogs::form::{self, Field, Kind};
use crate::dialogs::{Fields, Mode, Shaped};
use crate::panels::Pending;

/// Longest JSON parameter text the editor takes, in bytes.
pub const MAX_JSON_BYTES: usize = 1 << 20;
/// Most layers one scope names.
pub const MAX_SCOPE_LAYERS: usize = 1_024;
/// Longest search text and category name, in characters.
pub const MAX_SEARCH_CHARS: usize = 256;
/// The parameters the scope supplies; the JSON editor refuses them.
pub const SCOPE_KEYS: &[&str] = &["glyph", "glyphs", "layer", "layers", "allMasters"];
/// The undo label of the preview's interaction (never kept).
const PREVIEW_LABEL: &str = "Actions Preview";

pub const COMMANDS: &[UiCommand] = &[(
    "dialog.actions",
    "Actions",
    &["Script"],
    "",
    "{open?: bool, search?: string, command?: id, params?: object, set?: object, scope?: \"glyph\" | \"selected\" | \"category\" | \"font\", category?: string, layers?: \"active\" | \"visible\" | \"masters\", previewGlyph?: name, preview?: bool, wireframe?: bool, run?: bool, add?: bool, remove?: index, clear?: bool, title?: string, openSet?: name | path, save?: name, savePath?: path, replace?: bool, runSet?: bool} show the Actions window (hide it with open: false) and drive it. command chooses the action, an editing command that takes `glyphs`. Its parameters are the fields of its own dialog where it has one, else a JSON object checked against its params doc; `params` replaces them, `set` changes some of them. scope says which glyphs it acts on (the edited glyph, the selected glyphs, the glyphs of `category`, every glyph) and layers which layers (the active layer, the visible layers, every master). preview shows the action's result on previewGlyph (default the scope's first glyph) beside the glyph as it is, computed in an interaction that is rolled back. run runs the action on the scope as one undo step. The action set in the sidebar: add appends the action, remove takes out a step (counted from 0), clear empties the set, openSet reads a macro of the Macros folder or a macro file into it, save writes it as name.fcmacro in the Macros folder and savePath to a file (replace overwrites), runSet runs it on the scope as script.run runs a macro, as one undo step. At most one of run, runSet, openSet, save and savePath per call → {open, command, label, form: \"dialog\" | \"json\", fields?, params, json?, error?, scope, category, layers, glyphs: count, layerIds, steps: count, preview: {on, wireframe, glyph?, master?, error?}, set: {title, steps}, status, result?}",
)];

/// Which glyphs an action acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlyphScope {
    /// The glyph being edited, else the first glyph selected in Font View.
    Glyph,
    /// The glyphs selected in Font View, or in the text of the showing tab.
    #[default]
    Selected,
    /// The glyphs whose category is [`ActionsState::category`].
    Category,
    /// Every glyph of the font.
    Font,
}

impl GlyphScope {
    pub const ALL: [GlyphScope; 4] = [GlyphScope::Glyph, GlyphScope::Selected, GlyphScope::Category, GlyphScope::Font];

    pub fn word(self) -> &'static str {
        match self {
            GlyphScope::Glyph => "glyph",
            GlyphScope::Selected => "selected",
            GlyphScope::Category => "category",
            GlyphScope::Font => "font",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GlyphScope::Glyph => "Current Glyph",
            GlyphScope::Selected => "Selected Glyphs",
            GlyphScope::Category => "Category",
            GlyphScope::Font => "Whole Font",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.word() == word)
    }
}

/// Which layers of those glyphs an action acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerScope {
    /// The layer the Edit View edits for the current glyph scope, else the master the view
    /// shows.
    #[default]
    Active,
    /// The active layer and every layer shown with the others (`layers.toggleVisibility`) in the
    /// scope's glyphs.
    Visible,
    /// Every master.
    Masters,
}

impl LayerScope {
    pub const ALL: [LayerScope; 3] = [LayerScope::Active, LayerScope::Visible, LayerScope::Masters];

    pub fn word(self) -> &'static str {
        match self {
            LayerScope::Active => "active",
            LayerScope::Visible => "visible",
            LayerScope::Masters => "masters",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LayerScope::Active => "Active Layer",
            LayerScope::Visible => "Visible Layers",
            LayerScope::Masters => "All Masters",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.word() == word)
    }
}

/// The fields of the chosen action's own dialog, and what they hold.
#[derive(Clone, Debug)]
pub struct Form {
    /// The dialog's title.
    pub title: &'static str,
    pub fields: Vec<Field>,
    /// Each field's value, by key.
    pub values: Map<String, Value>,
    /// How the values become the command's parameters, for a dialog that shapes them.
    pub shaped: Option<Shaped>,
}

impl PartialEq for Form {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title && self.fields == other.fields && self.values == other.values
    }
}

impl Form {
    /// The command's parameters from the field values.
    pub fn params(&self) -> Map<String, Value> {
        match &self.shaped {
            Some(s) => (s.params)(&self.values),
            None => self.values.clone(),
        }
    }

    /// Gives the field `key` the checked `value` and brings linked fields in step.
    pub fn put(&mut self, key: &str, value: &Value) -> Result<(), String> {
        let field = self.fields.iter().find(|f| f.key == key).ok_or_else(|| format!("`{}` has no field `{key}`", self.title))?;
        let checked = field.check(value)?;
        self.values.insert(key.to_string(), checked);
        if let Some(s) = &self.shaped {
            (s.link)(&mut self.values, key);
        }
        Ok(())
    }
}

/// The preview: the font before and after the action, and the glyph and master drawn.
#[derive(Clone, Default)]
pub struct Shown {
    /// What the preview was computed from; a change computes it again.
    pub key: Option<u64>,
    pub before: Option<Arc<Font>>,
    pub after: Option<Arc<Font>>,
    pub glyph: String,
    pub master: String,
    pub error: Option<String>,
    /// The status line: what a run would do (the action, how many glyphs, which layers), or why
    /// it cannot run.
    pub summary: String,
}

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        let same = |a: &Option<Arc<Font>>, b: &Option<Arc<Font>>| match (a, b) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.key == other.key
            && self.glyph == other.glyph
            && self.master == other.master
            && self.error == other.error
            && self.summary == other.summary
            && same(&self.before, &other.before)
            && same(&self.after, &other.after)
    }
}

impl std::fmt::Debug for Shown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shown").field("key", &self.key).field("glyph", &self.glyph).field("master", &self.master).field("error", &self.error).finish()
    }
}

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ActionsState {
    /// The window is showing.
    pub open: bool,
    /// The text that narrows the action list.
    pub search: String,
    pub scope: GlyphScope,
    /// The category of [`GlyphScope::Category`].
    pub category: String,
    pub layers: LayerScope,
    /// The before-and-after preview is shown.
    pub preview: bool,
    /// The preview draws outlines instead of filled glyphs.
    pub wireframe: bool,
    /// The chosen action.
    #[serde(skip)]
    pub command: Option<String>,
    /// The fields of the action's dialog; `None`: the JSON editor.
    #[serde(skip)]
    pub form: Option<Form>,
    /// The JSON editor's text.
    #[serde(skip)]
    pub json: String,
    /// The glyph the preview shows; `None` or a glyph outside the scope: the scope's first.
    #[serde(skip)]
    pub preview_glyph: Option<String>,
    /// The action set in the sidebar.
    #[serde(skip)]
    pub set: Vec<Step>,
    #[serde(skip)]
    pub set_title: String,
    /// The arguments and modules of the macro last opened into the set, kept when it is saved.
    #[serde(skip)]
    pub set_args: Map<String, Value>,
    #[serde(skip)]
    pub set_requires: Vec<String>,
    /// The sidebar's Name or Path field.
    #[serde(skip)]
    pub set_name: String,
    /// The step selected in the sidebar.
    #[serde(skip)]
    pub set_selected: Option<usize>,
    /// What the last run, save or open did.
    #[serde(skip)]
    pub status: String,
    #[serde(skip)]
    pub shown: Shown,
}

impl ActionsState {
    /// The action's parameters: the form's, or the JSON editor's checked against `spec`.
    pub fn params(&self, spec: &CommandSpec) -> Result<Map<String, Value>, String> {
        match &self.form {
            Some(form) => Ok(form.params()),
            None => check_json(spec, &self.json),
        }
    }
}

/// One entry of the action list.
#[derive(Clone, Debug)]
pub struct ActionEntry {
    pub id: &'static str,
    pub label: &'static str,
    /// The menu path the command sits in (`Path › Transformations`), or `Other`.
    pub group: String,
}

/// Whether `spec` can be an action: an editing command whose targets include `glyphs` and that a
/// macro may run.
pub fn is_action(spec: &CommandSpec) -> bool {
    spec.journal
        && spec.targets.contains(&TargetKind::Glyphs)
        && names_param(spec.params, "glyphs")
        && !spec.id.starts_with("script.")
        && !macros::NOT_STEPS.contains(&spec.id)
}

/// Every action, grouped by menu in menu-bar order, by label within a group.
pub static ACTIONS: LazyLock<Vec<ActionEntry>> = LazyLock::new(|| {
    let order = |spec: &CommandSpec| {
        spec.menu.first().and_then(|m| super::menus::MENU_ORDER.iter().position(|o| o == m)).unwrap_or(super::menus::MENU_ORDER.len())
    };
    let mut list: Vec<(usize, ActionEntry)> = fontcraft_engine::command_specs()
        .iter()
        .filter(|s| is_action(s))
        .map(|s| {
            let group = if s.menu.is_empty() { "Other".to_string() } else { s.menu.join(" › ") };
            (order(s), ActionEntry { id: s.id, label: s.label, group })
        })
        .collect();
    list.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.group.cmp(&y.group)).then_with(|| x.label.cmp(y.label)).then_with(|| x.id.cmp(y.id)));
    list.into_iter().map(|(_, e)| e).collect()
});

/// The actions whose label or id contains `search`, ignoring case.
pub fn matching(search: &str) -> impl Iterator<Item = &'static ActionEntry> {
    let needle = search.trim().to_lowercase();
    ACTIONS.iter().filter(move |e| needle.is_empty() || e.label.to_lowercase().contains(&needle) || e.id.to_lowercase().contains(&needle))
}

/// The command of the action `id`.
pub fn action_spec(id: &str) -> Result<&'static CommandSpec, String> {
    let spec = fontcraft_engine::find_command(id).ok_or_else(|| format!("unknown command `{id}`"))?;
    if is_action(spec) { Ok(spec) } else { Err(format!("`{id}` is not an action: an action is an editing command that takes `glyphs`")) }
}

/// Whether the params doc `doc` names the parameter `key` (`key:` or `key?:`, not part of a
/// longer word), before the result.
pub fn names_param(doc: &str, key: &str) -> bool {
    let given = doc.split('→').next().unwrap_or(doc);
    given.match_indices(key).any(|(at, _)| {
        let before = given.get(..at).and_then(|s| s.chars().next_back());
        let after = given.get(at + key.len()..).unwrap_or("");
        before.is_none_or(|c| !(c.is_alphanumeric() || c == '_')) && (after.starts_with(':') || after.starts_with("?:"))
    })
}

/// The JSON editor's text as parameters: an object whose keys the command's params doc names,
/// none of them a scope key.
pub fn check_json(spec: &CommandSpec, text: &str) -> Result<Map<String, Value>, String> {
    if text.len() > MAX_JSON_BYTES {
        return Err(format!("the parameters hold more than {} MiB", MAX_JSON_BYTES >> 20));
    }
    let value: Value =
        if text.trim().is_empty() { json!({}) } else { serde_json::from_str(text).map_err(|e| format!("the parameters are not JSON: {e}"))? };
    let Value::Object(map) = value else { return Err("the parameters must be a JSON object".into()) };
    for key in map.keys() {
        if SCOPE_KEYS.contains(&key.as_str()) {
            return Err(format!("`{key}` comes from the scope: leave it out"));
        }
        if !names_param(spec.params, key) {
            return Err(format!("`{key}` is not a parameter of {}", spec.id));
        }
    }
    Ok(map)
}

/// The fields of `spec`'s own dialog, when it has one in preview mode whose fields this window can
/// draw.
pub fn form_for(session: &fontcraft_engine::Session, spec: &CommandSpec) -> Option<Form> {
    let dialog = crate::dialogs::for_command(spec.id)?;
    if dialog.mode != Mode::Preview || dialog.follow_up {
        return None;
    }
    let (fields, shaped) = match dialog.fields {
        Fields::Filter => (form::filter_fields(&filter_entry(spec.id)?).ok()?, None),
        Fields::Fixed(list) => (list(session), None),
        Fields::Shaped(s) => ((s.fields)(session), Some(s)),
        Fields::Targeted(_) | Fields::Tabs(_) => return None,
    };
    if fields.len() > form::MAX_FIELDS || !fields.iter().all(drawable) {
        return None;
    }
    let given = match shaped {
        Some(s) => (s.given)(session, &Map::new()).ok()?,
        None => Map::new(),
    };
    let mut values = form::initial_values(&fields, &given).ok()?;
    if let Some(s) = shaped {
        (s.link)(&mut values, "");
    }
    Some(Form { title: dialog.title, fields, values, shaped })
}

/// Whether the window draws a field of this kind.
fn drawable(field: &Field) -> bool {
    match &field.kind {
        Kind::Number { .. } | Kind::Integer { .. } | Kind::Bool | Kind::Lock | Kind::Choice { .. } | Kind::Multi { .. } => true,
        Kind::Text { lines, .. } => *lines <= 1,
        Kind::Folder | Kind::Order { .. } | Kind::List { .. } | Kind::Rules | Kind::Palettes { .. } | Kind::Color => false,
    }
}

/// The `filter.list` entry of the filter command `command`. A scratch session answers it, so the
/// app's session is left alone.
fn filter_entry(command: &str) -> Option<Value> {
    let mut scratch = fontcraft_engine::Session::new();
    let all = scratch.execute("filter.list", &Value::Null).ok()?;
    all.get("filters").and_then(Value::as_array)?.iter().find(|f| f.get("command").and_then(Value::as_str) == Some(command)).cloned()
}

/// Chooses the action `id`: its dialog's fields with their first values, or an empty JSON object.
fn choose(app: &FontcraftApp, st: &mut ActionsState, id: &str) -> Result<(), String> {
    let spec = action_spec(id)?;
    st.form = form_for(&app.session, spec);
    st.json = if st.form.is_some() { String::new() } else { "{}".to_string() };
    st.command = Some(spec.id.to_string());
    Ok(())
}

/// `params`: the action's parameters in the command's shape, replacing the ones it had.
fn replace_params(app: &FontcraftApp, st: &mut ActionsState, spec: &CommandSpec, given: &Map<String, Value>) -> Result<(), String> {
    match &mut st.form {
        Some(form) => {
            let values = match form.shaped {
                Some(s) => (s.given)(&app.session, given)?,
                None => given.clone(),
            };
            if let Some(key) = values.keys().find(|k| !form.fields.iter().any(|f| f.key == **k)) {
                return Err(format!("`{key}` is not a field of {}", form.title));
            }
            let mut values = form::initial_values(&form.fields, &values)?;
            if let Some(s) = form.shaped {
                (s.link)(&mut values, "");
            }
            form.values = values;
        }
        None => {
            let text = serde_json::to_string_pretty(&Value::Object(given.clone())).map_err(|e| e.to_string())?;
            check_json(spec, &text)?;
            st.json = text;
        }
    }
    Ok(())
}

/// `set`: changes some of the action's parameters (fields of its dialog, or keys of the JSON).
fn change_params(st: &mut ActionsState, spec: &CommandSpec, changes: &Map<String, Value>) -> Result<(), String> {
    match &mut st.form {
        Some(form) => {
            for (key, value) in changes {
                form.put(key, value)?;
            }
        }
        None => {
            let mut map = check_json(spec, &st.json)?;
            for (key, value) in changes {
                if value.is_null() {
                    map.remove(key);
                } else {
                    map.insert(key.clone(), value.clone());
                }
            }
            let text = serde_json::to_string_pretty(&Value::Object(map)).map_err(|e| e.to_string())?;
            check_json(spec, &text)?;
            st.json = text;
        }
    }
    Ok(())
}

/// The glyphs of the scope, in font order for a category or the font.
pub fn scope_glyphs(app: &mut FontcraftApp, scope: GlyphScope, category: &str) -> Result<Vec<String>, String> {
    let names: Vec<String> = match scope {
        GlyphScope::Glyph => {
            let current = app.state.editing().map(str::to_string).or_else(|| app.state.selected_glyphs.first().cloned());
            vec![current.ok_or("no glyph is being edited or selected")?]
        }
        GlyphScope::Selected => selected_glyphs(app),
        GlyphScope::Category => {
            let wanted = category.trim();
            if wanted.is_empty() {
                return Err("give the category whose glyphs the action acts on".into());
            }
            let props = app.session.effective_props().map_err(|e| e.to_string())?;
            let font = app.session.font().map_err(|e| e.to_string())?;
            font.glyphs
                .iter()
                .filter(|g| props.get(g).and_then(|p| p.category.as_deref()).is_some_and(|c| c.eq_ignore_ascii_case(wanted)))
                .map(|g| g.name.clone())
                .collect()
        }
        GlyphScope::Font => app.session.font().map_err(|e| e.to_string())?.glyphs.iter().map(|g| g.name.clone()).collect(),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let names: Vec<String> = names.into_iter().filter(|n| font.glyph(n).is_some() && seen.insert(n.clone())).collect();
    if names.is_empty() {
        return Err(format!("the scope ({}) holds no glyphs", scope.label()));
    }
    if names.len() > MAX_GLYPHS {
        return Err(format!("the scope holds more than {MAX_GLYPHS} glyphs"));
    }
    Ok(names)
}

/// The glyphs selected in the text of the showing tab (else the edited glyph), or in Font View.
fn selected_glyphs(app: &FontcraftApp) -> Vec<String> {
    match app.state.tab() {
        Some(tab) => {
            let from_text: Vec<String> =
                if tab.text_mode { tab.text.selection().filter_map(|i| tab.item_glyph(i)).map(str::to_string).collect() } else { Vec::new() };
            if from_text.is_empty() { tab.edited_glyph().map(|g| vec![g.to_string()]).unwrap_or_default() } else { from_text }
        }
        None => app.state.selected_glyphs.clone(),
    }
}

/// The layer ids of the layer scope for `glyphs`.
pub fn scope_layers(app: &FontcraftApp, scope: GlyphScope, layers: LayerScope, glyphs: &[String]) -> Result<Vec<String>, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let active = active_layer(app, font, scope).ok_or("the font has no master")?;
    let mut out = vec![active];
    match layers {
        LayerScope::Active => {}
        LayerScope::Visible => {
            for glyph in glyphs.iter().filter_map(|g| font.glyph(g)) {
                for layer in glyph.layers.iter().filter(|l| l.info.visible) {
                    if !out.contains(&layer.id) {
                        if out.len() >= MAX_SCOPE_LAYERS {
                            return Err(format!("the visible layers are more than {MAX_SCOPE_LAYERS}"));
                        }
                        out.push(layer.id.clone());
                    }
                }
            }
        }
        LayerScope::Masters => out = font.masters.iter().take(MAX_SCOPE_LAYERS).map(|m| m.id.clone()).collect(),
    }
    Ok(out)
}

/// The active layer: the layer the Edit View edits when the scope is the edited glyph, else the
/// master the view shows.
fn active_layer(app: &FontcraftApp, font: &Font, scope: GlyphScope) -> Option<String> {
    match app.state.tab() {
        Some(tab) => {
            if scope == GlyphScope::Glyph
                && let Some(layer) = crate::canvas::view::active_layer(font, tab)
            {
                return Some(layer.id.clone());
            }
            crate::canvas::view::active_master(font, tab).map(|m| m.id.clone())
        }
        None => super::font_view::master(app).or_else(|| font.masters.first().map(|m| m.id.clone())),
    }
}

/// The steps that run `spec` with `params` on `glyphs` and `layers` (values or placeholders). A
/// command that takes `layers` gets them in one step; with every master and a command that takes
/// `allMasters`, that; one that takes one `layer` runs once per layer; else the command's own
/// layer applies.
fn build_steps(
    spec: &CommandSpec,
    params: &Map<String, Value>,
    glyphs: &Value,
    layers: &[String],
    layers_value: &Value,
    scope: LayerScope,
) -> Vec<Step> {
    let step = |extra: Option<(&str, Value)>| {
        let mut p = params.clone();
        p.insert("glyphs".into(), glyphs.clone());
        if let Some((key, value)) = extra {
            p.insert(key.into(), value);
        }
        Step { command: spec.id.to_string(), params: Value::Object(p) }
    };
    if names_param(spec.params, "layers") {
        return vec![step(Some(("layers", layers_value.clone())))];
    }
    if scope == LayerScope::Masters && names_param(spec.params, "allMasters") {
        return vec![step(Some(("allMasters", json!(true))))];
    }
    if names_param(spec.params, "layer") {
        return layers.iter().map(|id| step(Some(("layer", json!(id))))).collect();
    }
    vec![step(None)]
}

/// What a run of the chosen action does.
pub struct Plan {
    pub spec: &'static CommandSpec,
    pub params: Map<String, Value>,
    pub glyphs: Vec<String>,
    pub layers: Vec<String>,
}

impl Plan {
    /// The steps on `glyphs` (the plan's, or the preview glyph alone).
    pub fn steps(&self, glyphs: &[String], scope: LayerScope) -> Vec<Step> {
        build_steps(self.spec, &self.params, &json!(glyphs), &self.layers, &json!(self.layers), scope)
    }
}

/// The chosen action with its parameters, glyphs and layers.
pub fn plan(app: &mut FontcraftApp, st: &ActionsState) -> Result<Plan, String> {
    let id = st.command.as_deref().ok_or("choose an action first")?;
    let spec = action_spec(id)?;
    let params = st.params(spec)?;
    let glyphs = scope_glyphs(app, st.scope, &st.category)?;
    let layers = scope_layers(app, st.scope, st.layers, &glyphs)?;
    Ok(Plan { spec, params, glyphs, layers })
}

/// Runs the chosen action on the scope as one undo step.
fn run_action(app: &mut FontcraftApp, st: &ActionsState) -> Result<Value, String> {
    let plan = plan(app, st)?;
    let steps = plan.steps(&plan.glyphs, st.layers);
    match steps.as_slice() {
        [] => Err(format!("{}: the layer scope names no layer", plan.spec.label)),
        [one] => app.run(&one.command, &one.params),
        many => app.run("command.batch", &json!({"label": plan.spec.label, "steps": many})),
    }
}

/// The chosen action as set steps: `glyphs` and, for a command that takes them, `layers` are the
/// macro's built-in arguments; other layer parameters are fixed from the scope now. A scope
/// without glyphs still gives its layers.
fn set_steps(app: &mut FontcraftApp, st: &ActionsState) -> Result<Vec<Step>, String> {
    let id = st.command.as_deref().ok_or("choose an action first")?;
    let spec = action_spec(id)?;
    let params = st.params(spec)?;
    let glyphs = scope_glyphs(app, st.scope, &st.category).unwrap_or_default();
    let layers = scope_layers(app, st.scope, st.layers, &glyphs)?;
    let steps = build_steps(spec, &params, &json!({"$arg": "glyphs"}), &layers, &json!({"$arg": "layers"}), st.layers);
    if steps.is_empty() {
        return Err(format!("{}: the layer scope names no layer", spec.label));
    }
    Ok(steps)
}

/// The set as a macro object for `script.runActionSet` and `script.saveActionSet`.
fn set_macro(st: &ActionsState) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("steps".into(), json!(st.set));
    if !st.set_args.is_empty() {
        m.insert("args".into(), Value::Object(st.set_args.clone()));
    }
    if !st.set_requires.is_empty() {
        m.insert("requires".into(), json!(st.set_requires));
    }
    m
}

/// Runs the action set on the scope.
fn run_set(app: &mut FontcraftApp, st: &ActionsState) -> Result<Value, String> {
    if st.set.is_empty() {
        return Err("the action set is empty: add an action first".into());
    }
    let glyphs = scope_glyphs(app, st.scope, &st.category)?;
    let layers = scope_layers(app, st.scope, st.layers, &glyphs)?;
    let mut set = set_macro(st);
    set.insert("title".into(), json!(set_title(st)));
    app.run("script.run", &json!({"macro": set, "glyphs": glyphs, "layers": layers}))
}

fn set_title(st: &ActionsState) -> &str {
    if st.set_title.trim().is_empty() { "Action Set" } else { st.set_title.trim() }
}

/// Writes the action set: as `name`.fcmacro in the Macros folder, or to `path`.
fn save_set(app: &mut FontcraftApp, st: &ActionsState, name: Option<&str>, path: Option<&str>, replace: bool) -> Result<Value, String> {
    if st.set.is_empty() {
        return Err("the action set is empty: add an action first".into());
    }
    let name = match (name, path) {
        (Some(name), _) => name.to_string(),
        (None, Some(path)) => Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("Action Set").to_string(),
        (None, None) => return Err("give `save` or `savePath`".into()),
    };
    let mut p = set_macro(st);
    p.insert("name".into(), json!(name));
    p.insert("title".into(), json!(if st.set_title.trim().is_empty() { name.as_str() } else { st.set_title.trim() }));
    p.insert("replace".into(), json!(replace));
    if let Some(path) = path {
        p.insert("path".into(), json!(path));
    }
    app.run("script.saveActionSet", &Value::Object(p))
}

/// Reads a macro of the Macros folder (by path or title) or a macro file into the set.
fn open_set(app: &FontcraftApp, st: &mut ActionsState, spec: &str) -> Result<Value, String> {
    let m = match app.session.extensions().find_macro(spec) {
        Some((_, m)) => m.clone(),
        None => {
            if !(macros::is_macro_file(spec) || spec.contains(['/', '\\'])) {
                return Err(format!("there is no macro named {spec:?} in the Macros folder"));
            }
            let bytes = macros::read_macro_file(Path::new(spec)).map_err(|e| format!("{spec}: {e}"))?;
            Macro::parse(&bytes).map_err(|e| format!("{spec}: {e}"))?
        }
    };
    m.check().map_err(|e| format!("{spec}: {e}"))?;
    st.set_title = if m.title.trim().is_empty() { macros::file_stem(spec).to_string() } else { m.title.clone() };
    st.set = m.steps;
    st.set_args = m.args;
    st.set_requires = m.requires;
    st.set_selected = None;
    Ok(json!({"title": st.set_title, "steps": st.set.len()}))
}

/// What the preview depends on, apart from the font's contents.
fn preview_key(app: &FontcraftApp) -> u64 {
    let st = &app.state.panels.actions;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(doc) = app.session.active() {
        (doc.id, Arc::as_ptr(&doc.font) as usize).hash(&mut h);
    }
    (&st.command, &st.json, st.scope, &st.category, st.layers, &st.preview_glyph, st.preview).hash(&mut h);
    st.form.as_ref().map(|f| Value::Object(f.values.clone()).to_string()).hash(&mut h);
    (&app.state.selected_glyphs, app.state.editing(), app.state.active_tab).hash(&mut h);
    if let Some(tab) = app.state.tab() {
        (&tab.master, &tab.layers, tab.text_mode, tab.text.selection()).hash(&mut h);
    }
    h.finish()
}

/// Computes the status line and the preview again when what they depend on changed. Nothing is
/// computed while a dialog or another interaction runs; the last ones stay.
pub fn refresh_preview(app: &mut FontcraftApp) {
    let st = &app.state.panels.actions;
    if !st.open || st.command.is_none() {
        if st.shown.key.is_some() {
            app.state.panels.actions.shown = Shown::default();
        }
        return;
    }
    if app.dialog.is_some() || app.session.interaction().is_some() {
        return;
    }
    let key = preview_key(app);
    if app.state.panels.actions.shown.key == Some(key) {
        return;
    }
    let mut shown = if app.state.panels.actions.preview { compute_preview(app) } else { Shown::default() };
    shown.summary = summary(app);
    shown.key = Some(key);
    app.state.panels.actions.shown = shown;
}

/// The status line: `Round Corners: 2 glyphs, layers Regular and Bold (1 step)`, or why the
/// action cannot run.
fn summary(app: &mut FontcraftApp) -> String {
    let st = app.state.panels.actions.clone();
    let plan = match plan(app, &st) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let steps = plan.steps(&plan.glyphs, st.layers).len();
    let Ok(font) = app.session.font() else { return "no document is open".into() };
    let names: Vec<&str> = plan.layers.iter().map(|id| font.masters.iter().find(|m| m.id == *id).map_or(id.as_str(), |m| m.name.as_str())).collect();
    let layers = match names.as_slice() {
        [one] => format!("layer {one}"),
        [rest @ .., last] => format!("layers {} and {last}", rest.join(", ")),
        [] => "no layer".to_string(),
    };
    let glyphs = plan.glyphs.len();
    format!("{}: {glyphs} glyph{}, {layers} ({steps} step{})", plan.spec.label, if glyphs == 1 { "" } else { "s" }, if steps == 1 { "" } else { "s" })
}

/// Runs the action on the preview glyph inside an interaction, keeps the font before and after,
/// and cancels the interaction.
fn compute_preview(app: &mut FontcraftApp) -> Shown {
    let st = app.state.panels.actions.clone();
    let failed = |error: String| Shown { error: Some(error), ..Shown::default() };
    let plan = match plan(app, &st) {
        Ok(p) => p,
        Err(e) => return failed(e),
    };
    let glyph = st.preview_glyph.filter(|g| plan.glyphs.contains(g)).or_else(|| plan.glyphs.first().cloned()).unwrap_or_default();
    let Ok(font) = app.session.font() else { return failed("no document is open".into()) };
    let master = plan
        .layers
        .iter()
        .find_map(|id| font.glyph(&glyph).and_then(|g| g.layers.iter().find(|l| l.id == *id)).map(|l| l.master.clone()))
        .or_else(|| font.masters.first().map(|m| m.id.clone()))
        .unwrap_or_default();
    let steps = plan.steps(std::slice::from_ref(&glyph), st.layers);
    let Ok(before) = app.session.doc().map(|d| d.font.clone()) else { return failed("no document is open".into()) };
    if let Err(e) = app.session.begin(PREVIEW_LABEL) {
        return failed(e.to_string());
    }
    let ran = match steps.as_slice() {
        [] => Err(format!("{}: the layer scope names no layer", plan.spec.label)),
        [one] => app.session.preview(&one.command, &one.params).map_err(|e| e.to_string()),
        many => app.session.preview("command.batch", &json!({"steps": many})).map_err(|e| e.to_string()),
    };
    let after = app.session.doc().map(|d| d.font.clone()).ok();
    let _ = app.session.cancel();
    match ran {
        Ok(_) => Shown { before: Some(before), after, glyph, master, ..Shown::default() },
        Err(e) => Shown { before: Some(before), glyph, master, error: Some(e), ..Shown::default() },
    }
}

/// The window's state for `ui.inspect`-style replies; `result`: what the call ran.
pub fn inspect(app: &mut FontcraftApp, result: Option<Value>) -> Value {
    let st = app.state.panels.actions.clone();
    let planned = st.command.as_ref().map(|_| plan(app, &st));
    let (glyphs, layer_ids, steps, error) = match &planned {
        Some(Ok(p)) => (p.glyphs.len(), json!(p.layers), p.steps(&p.glyphs, st.layers).len(), None),
        Some(Err(e)) => (0, json!([]), 0, Some(e.clone())),
        None => (0, json!([]), 0, None),
    };
    let spec = st.command.as_deref().and_then(|id| action_spec(id).ok());
    let mut out = json!({
        "open": st.open,
        "command": st.command,
        "label": spec.map(|s| s.label),
        "form": if st.form.is_some() { "dialog" } else { "json" },
        "params": spec.and_then(|s| st.params(s).ok()).map(Value::Object),
        "error": error,
        "scope": st.scope.word(),
        "category": st.category,
        "layers": st.layers.word(),
        "glyphs": glyphs,
        "layerIds": layer_ids,
        "steps": steps,
        "preview": {
            "on": st.preview,
            "wireframe": st.wireframe,
            "glyph": (!st.shown.glyph.is_empty()).then_some(&st.shown.glyph),
            "master": (!st.shown.master.is_empty()).then_some(&st.shown.master),
            "changed": match (&st.shown.before, &st.shown.after) {
                (Some(b), Some(a)) => Some(!Arc::ptr_eq(a, b) && !a.same_as(b)),
                _ => None,
            },
            "error": st.shown.error,
        },
        "set": {"title": set_title(&st), "steps": st.set},
        "summary": st.shown.summary,
        "status": st.status,
    });
    if let Value::Object(m) = &mut out {
        match &st.form {
            Some(form) => {
                let fields: Vec<Value> = form
                    .fields
                    .iter()
                    .map(|f| {
                        let mut v = serde_json::to_value(f).unwrap_or(Value::Null);
                        if let Value::Object(o) = &mut v {
                            o.insert("value".into(), form.values.get(&f.key).cloned().unwrap_or(Value::Null));
                        }
                        v
                    })
                    .collect();
                m.insert("fields".into(), Value::Array(fields));
            }
            None if st.command.is_some() => {
                m.insert("json".into(), json!(st.json));
            }
            None => {}
        }
        if let Some(r) = result {
            m.insert("result".into(), r);
        }
    }
    out
}

/// A string parameter of at most `max` characters.
fn short<'a>(args: &Args<'a>, key: &str, max: usize) -> Result<Option<&'a str>, String> {
    match args.string(key)? {
        Some(s) if s.chars().count() > max => Err(args.err(format!("`{key}` holds at most {max} characters"))),
        other => Ok(other),
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "dialog.actions" {
        return Err(format!("unknown command `{id}`"));
    }
    let mut st = app.state.panels.actions.clone();
    st.open = args.flag("open")?.unwrap_or(true);
    if let Some(search) = short(args, "search", MAX_SEARCH_CHARS)? {
        st.search = search.to_string();
    }
    if let Some(command) = args.string("command")? {
        choose(app, &mut st, command).map_err(|e| args.err(e))?;
    }
    let spec = match &st.command {
        Some(c) => Some(action_spec(c).map_err(|e| args.err(e))?),
        None => None,
    };
    for (key, replace) in [("params", true), ("set", false)] {
        let Some(v) = args.get(key) else { continue };
        let map = v.as_object().ok_or_else(|| args.err(format!("`{key}` must be a JSON object")))?;
        let spec = spec.ok_or_else(|| args.err("choose an action with `command` first"))?;
        if replace {
            replace_params(app, &mut st, spec, map).map_err(|e| args.err(e))?;
        } else {
            change_params(&mut st, spec, map).map_err(|e| args.err(e))?;
        }
    }
    if let Some(word) = args.string("scope")? {
        st.scope =
            GlyphScope::parse(word).ok_or_else(|| args.err(format!("`scope` is \"glyph\", \"selected\", \"category\" or \"font\", not {word:?}")))?;
    }
    if let Some(category) = short(args, "category", MAX_SEARCH_CHARS)? {
        st.category = category.to_string();
    }
    if let Some(word) = args.string("layers")? {
        st.layers = LayerScope::parse(word).ok_or_else(|| args.err(format!("`layers` is \"active\", \"visible\" or \"masters\", not {word:?}")))?;
    }
    if let Some(glyph) = short(args, "previewGlyph", fontcraft_model::MAX_NAME_LEN)? {
        if app.session.font().map_err(|e| args.err(e))?.glyph(glyph).is_none() {
            return Err(args.err(format!("there is no glyph `{glyph}`")));
        }
        st.preview_glyph = Some(glyph.to_string());
    }
    if let Some(on) = args.flag("wireframe")? {
        st.wireframe = on;
    }
    if let Some(on) = args.flag("preview")? {
        st.preview = on;
    }
    if !st.open {
        st.shown = Shown::default();
    }

    // The sidebar's edits.
    if args.flag("add")? == Some(true) {
        let steps = set_steps(app, &st).map_err(|e| args.err(e))?;
        if st.set.len() + steps.len() > MAX_MACRO_STEPS {
            return Err(args.err(format!("an action set holds at most {MAX_MACRO_STEPS} steps")));
        }
        st.set.extend(steps);
    }
    if let Some(index) = args.index("remove")? {
        let i = usize::try_from(index).ok().filter(|i| *i < st.set.len()).ok_or_else(|| args.err(format!("the set has no step {index}")))?;
        st.set.remove(i);
        st.set_selected = None;
    }
    if args.flag("clear")? == Some(true) {
        st.set.clear();
        st.set_args.clear();
        st.set_requires.clear();
        st.set_selected = None;
    }
    if let Some(title) = short(args, "title", MAX_MACRO_TEXT)? {
        st.set_title = title.to_string();
    }

    // At most one action that runs, reads or writes.
    let effects =
        ["run", "runSet", "openSet", "save", "savePath"].into_iter().filter(|k| args.get(k).is_some_and(|v| v != &Value::Bool(false))).count();
    if effects > 1 {
        return Err(args.err("give at most one of run, runSet, openSet, save and savePath"));
    }
    let mut result = None;
    if args.flag("run")? == Some(true) {
        let r = run_action(app, &st).map_err(|e| args.err(e))?;
        st.status = format!("{}: done", spec.map_or("Action", |s| s.label));
        result = Some(r);
    } else if args.flag("runSet")? == Some(true) {
        let r = run_set(app, &st).map_err(|e| args.err(e))?;
        st.status = format!("{}: done", set_title(&st));
        result = Some(r);
    } else if let Some(source) = short(args, "openSet", MAX_MACRO_TEXT)? {
        let r = open_set(app, &mut st, source).map_err(|e| args.err(e))?;
        st.status = format!("opened {}", st.set_title);
        result = Some(r);
    } else if args.get("save").is_some() || args.get("savePath").is_some() {
        let replace = args.flag("replace")?.unwrap_or(false);
        let r = save_set(app, &st, args.string("save")?, args.string("savePath")?, replace).map_err(|e| args.err(e))?;
        st.status = format!("saved {}", r.get("path").and_then(Value::as_str).unwrap_or(""));
        result = Some(r);
    }
    app.state.panels.actions = st;
    refresh_preview(app);
    Ok(inspect(app, result))
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.actions.open {
        return;
    }
    refresh_preview(app);
    crate::dialogs::actions::show(app, ui, pending);
}
