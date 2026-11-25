//! What a command the person fires (menu, shortcut, dialog, panel button) acts on, and the parameters that say so.
//!
//! | In front | Glyphs | Layer of each |
//! |---|---|---|
//! | Font View with glyphs selected | the selected ones | its layer of the current master |
//! | Font View with nothing selected | none: a glyph command refuses | |
//! | Edit View in text mode with glyphs of the text selected | the selected ones, each once per layer | the layer its text item shows |
//! | Edit View | the edited glyph, with the canvas selection | the layer the canvas shows |
//!
//! Text mode gives its selection only to commands that take `glyphs`; a command that takes nodes, paths or one layer keeps
//! the edited glyph. The glyph behind Font View is never a target.

use std::collections::{HashMap, HashSet};

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::CommandSpec;
use fontcraft_engine::params::TargetKind;
use serde_json::{Map, Value, json};

use super::font_view::MAX_BATCH_GLYPHS;
use super::{FontCraftApp, ViewState, select_glyph_first};
use crate::app::context_menu::item_state;
use crate::canvas::text;
use crate::canvas::tools::targets::{LayerParam, layer_param, with_targets};
use crate::shell::masters;

/// What runs several commands as one undo step.
pub(crate) const BATCH: &str = "command.batch";

/// Commands that address one glyph and run once per selected glyph, as one undo step
/// (`fontView.batch.addComponent`).
const PER_GLYPH: [&str; 2] = ["glyph.addComponent", "glyph.addComponentAllMasters"];

/// Glyphs a command acts on in one of their layers: the layer has the same id in every one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlyphLayers {
    /// The layer's id.
    pub layer: String,
    /// The id of its master.
    pub master: String,
    pub glyphs: Vec<String>,
}

/// What fired commands act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Acting {
    /// Font View's selection.
    FontView,
    /// The edited glyph, or the text selection ([`text_selection`]).
    EditView,
    /// Nothing the person chose.
    Nothing,
}

/// What fired commands act on now.
pub(crate) fn acting(app: &FontCraftApp) -> Acting {
    let view = &app.view;
    if view.tabs.font_view_shows() {
        return if view.font_view.selected.is_empty() { Acting::Nothing } else { Acting::FontView };
    }
    // `view.glyph` may name a glyph the font no longer has; the canvas then draws the font's first.
    let shown = view.glyph.as_deref().zip(app.session.font().ok()).is_some_and(|(name, font)| font.glyph(name).is_some());
    if shown { Acting::EditView } else { Acting::Nothing }
}

/// The glyphs fired commands act on when those are more than the edited glyph: Font View's selection, or the text
/// selection. Empty otherwise.
pub(crate) fn chosen_glyphs(font: &fontcraft_engine::model::Font, view: &ViewState) -> Vec<GlyphLayers> {
    if view.tabs.font_view_shows() { font_view_selection(font, view) } else { text_selection(font, view) }
}

/// Font View's selected glyphs, in the current master.
fn font_view_selection(font: &fontcraft_engine::model::Font, view: &ViewState) -> Vec<GlyphLayers> {
    let Some(master) = masters::current_master(font, &view.layers) else { return Vec::new() };
    let glyphs: Vec<String> = view.font_view.selected.iter().filter(|n| font.glyph(n).is_some()).take(MAX_BATCH_GLYPHS).cloned().collect();
    if glyphs.is_empty() {
        return Vec::new();
    }
    vec![GlyphLayers { layer: master.id.clone(), master: master.id.clone(), glyphs }]
}

/// The glyphs of the text selection while text mode shows it, grouped by the layer their items show, both in text order.
/// Empty with only a cursor, outside text mode, and while the canvas shows a glyph that is not the text's.
pub(crate) fn text_selection(font: &fontcraft_engine::model::Font, view: &ViewState) -> Vec<GlyphLayers> {
    if !text::text_mode(view) || view.tabs.font_view_shows() || !text::shows_text(font, view) {
        return Vec::new();
    }
    let Some(current) = masters::current_master(font, &view.layers).map(|m| m.id.as_str()) else { return Vec::new() };
    let mut groups: Vec<GlyphLayers> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for index in view.text.text.selection().take(MAX_BATCH_GLYPHS) {
        let Some(glyph) = view.text.item_glyph(index).and_then(|name| font.glyph(name)) else { continue };
        let own = view.text.text.layer_of(index, &glyph.name);
        let layer = crate::app::models::text::item_layer(font, view, glyph, own);
        let (id, master) = layer.map_or((current, current), |l| (l.id.as_str(), l.master.as_str()));
        if !seen.insert((glyph.name.as_str(), id)) {
            continue;
        }
        let group = *at.entry(id).or_insert_with(|| {
            groups.push(GlyphLayers { layer: id.to_string(), master: master.to_string(), glyphs: Vec::new() });
            groups.len().saturating_sub(1)
        });
        if let Some(group) = groups.get_mut(group) {
            group.glyphs.push(glyph.name.clone());
        }
    }
    groups
}

/// Whether `spec` acts on every glyph it is given: its targets include `glyphs`, or it runs once per glyph.
pub(crate) fn takes_glyphs(spec: &CommandSpec) -> bool {
    spec.targets.contains(&TargetKind::Glyphs) || PER_GLYPH.contains(&spec.id)
}

/// `command` on the glyphs of `groups`: `glyphs`, with their layer when the command changes one ([`layer_param`]).
/// Glyphs in several layers become one `command.batch` with a step per layer, and [`PER_GLYPH`] commands one with a
/// step per glyph. A command that takes no glyphs, or was given parameters, stays as it is; a `PER_GLYPH` command
/// keeps the parameters it was given unless they name a glyph.
pub(crate) fn on_glyphs(command: CommandRef, groups: &[GlyphLayers]) -> CommandRef {
    let given = match &command.params {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return command,
    };
    let Some(spec) = fontcraft_engine::find_command(&command.id).filter(|_| !groups.is_empty()) else { return command };
    let param = layer_param(spec);
    if PER_GLYPH.contains(&spec.id) && !given.contains_key("glyph") {
        let sets =
            groups.iter().flat_map(|group| group.glyphs.iter().map(move |glyph| (group, glyph))).take(MAX_BATCH_GLYPHS).map(|(group, glyph)| {
                let mut m = given.clone();
                m.insert("glyph".into(), json!(glyph));
                if let Some(param) = param {
                    let (key, value) = param.entry(&group.layer, &group.master);
                    m.entry(key).or_insert(value);
                }
                m
            });
        return batch(spec, sets);
    }
    if !given.is_empty() || !spec.targets.contains(&TargetKind::Glyphs) {
        return command;
    }
    let mut sets = glyph_sets(param, groups);
    match sets.pop() {
        Some(only) if sets.is_empty() => CommandRef::with_params(command.id, Value::Object(only)),
        Some(last) => {
            sets.push(last);
            batch(spec, sets)
        }
        None => command,
    }
}

/// A layer parameter's key and value, and the glyphs that go with it.
type GlyphSet<'g> = (Option<(&'static str, Value)>, Vec<&'g str>);

/// The parameters of a command whose layer parameter is `param`: one set for each value of it, with the glyphs it goes
/// with, each once.
fn glyph_sets(param: Option<LayerParam>, groups: &[GlyphLayers]) -> Vec<Map<String, Value>> {
    let mut sets: Vec<GlyphSet<'_>> = Vec::new();
    let mut at: HashMap<Option<&str>, usize> = HashMap::new();
    let mut seen: HashSet<(usize, &str)> = HashSet::new();
    for group in groups {
        // Layers of one master are one set for a parameter that names the master.
        let key = param.map(|p| match p {
            LayerParam::Layer | LayerParam::Layers => group.layer.as_str(),
            LayerParam::LayerMaster | LayerParam::Master => group.master.as_str(),
        });
        let set = *at.entry(key).or_insert_with(|| {
            sets.push((param.map(|p| p.entry(&group.layer, &group.master)), Vec::new()));
            sets.len().saturating_sub(1)
        });
        if let Some((_, glyphs)) = sets.get_mut(set) {
            glyphs.extend(group.glyphs.iter().map(String::as_str).filter(|glyph| seen.insert((set, *glyph))));
        }
    }
    sets.into_iter()
        .map(|(layer, glyphs)| {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(glyphs));
            if let Some((key, value)) = layer {
                m.insert(key.into(), value);
            }
            m
        })
        .collect()
}

/// One `command.batch` that runs `spec` with each set of parameters.
pub(crate) fn batch(spec: &CommandSpec, sets: impl IntoIterator<Item = Map<String, Value>>) -> CommandRef {
    let steps: Vec<Value> = sets.into_iter().map(|params| json!({"command": spec.id, "params": params})).collect();
    CommandRef::with_params(BATCH, json!({"label": spec.label, "steps": steps}))
}

/// The parameters of each step of `command` when it is a batch of runs of `id` ([`on_glyphs`]), else its own when it
/// is `id` with parameters.
pub(crate) fn target_sets(command: &CommandRef, id: &str) -> Vec<Map<String, Value>> {
    let object = |v: &Value| v.as_object().filter(|m| !m.is_empty()).cloned();
    if command.id == id {
        return object(&command.params).into_iter().collect();
    }
    if command.id != BATCH {
        return Vec::new();
    }
    let steps = command.params.get("steps").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    steps.iter().filter(|s| s.get("command").and_then(Value::as_str) == Some(id)).filter_map(|s| s.get("params").and_then(object)).collect()
}

impl FontCraftApp {
    /// `command` with the targets it acts on filled in (module doc), when it takes targets and was given none. While
    /// Font View is in front with nothing selected, it stays as it is.
    pub(crate) fn with_targets(&mut self, command: CommandRef) -> CommandRef {
        if self.view.tabs.font_view_shows() {
            return self.with_font_view_targets(command);
        }
        if fontcraft_engine::find_command(&command.id).is_some_and(takes_glyphs)
            && let Ok(font) = self.session.font()
        {
            let groups = text_selection(font, &self.view);
            if !groups.is_empty() {
                return on_glyphs(command, &groups);
            }
        }
        with_targets(self, command)
    }

    /// `command` fired while Font View shows, with its targets filled from the Font View selection.
    pub fn with_font_view_targets(&self, command: CommandRef) -> CommandRef {
        if !self.view.tabs.font_view_shows() {
            return command;
        }
        let Ok(font) = self.session.font() else { return command };
        on_glyphs(command, &font_view_selection(font, &self.view))
    }

    /// Whether the person chose what a fired command acts on: glyphs selected in Font View while it is in front, or a
    /// glyph opened in the edit view while the edit view is in front. With neither, a dialog's command acts on every
    /// glyph or the dialog asks for a glyph, and any other glyph command refuses ([`Self::refused`]).
    pub(crate) fn targets_chosen(&self) -> bool {
        acting(self) != Acting::Nothing
    }

    /// Whether fired commands take Font View's selection: glyphs are selected and Font View is the showing tab.
    pub(super) fn font_view_targets(&self) -> bool {
        acting(self) == Acting::FontView
    }

    /// Why `fired`, which became `targeted` with its targets filled, does not run: it acts on glyphs and nothing is
    /// chosen, or it acts on parts of one glyph and Font View is in front. `None` for a command that names its own
    /// targets, that takes none, and without an open font, where it says so itself.
    pub(crate) fn refused(&self, fired: &CommandRef, targeted: &CommandRef) -> Option<String> {
        let spec = fontcraft_engine::find_command(&fired.id)?;
        if !is_empty(&fired.params) || targeted.id != fired.id || self.session.font().is_err() {
            return None;
        }
        let unfilled = is_empty(&targeted.params);
        match acting(self) {
            // What the canvas filled then is the font's first glyph, which the person never picked.
            Acting::Nothing => (!spec.targets.is_empty() || !unfilled).then(|| select_glyph_first(spec.id)),
            Acting::FontView if unfilled && !spec.targets.is_empty() => Some(match item_state::objects_wanted(spec) {
                Some(what) => format!("{}: edit a glyph and select {what} first", spec.label),
                None => format!("{}: edit a glyph first", spec.label),
            }),
            Acting::FontView | Acting::EditView => None,
        }
    }
}

fn is_empty(params: &Value) -> bool {
    params.is_null() || params.as_object().is_some_and(Map::is_empty)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
#[path = "targets_tests.rs"]
mod tests;
