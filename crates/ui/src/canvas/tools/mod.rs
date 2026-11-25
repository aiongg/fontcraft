//! The pointer tools on the canvas: `fontcraft_tools::Toolbox` behind the canvas's active-tool slot.
//!
//! [`bridge::ToolBridge`] sits in the slot. Craft's `ToolHost` hands it the canvas's press, drags, hover and keys, and
//! it posts each one in the edited glyph's font units as `Msg::Tool`. `drain` passes them to [`apply::input`], which
//! runs the toolbox on the edited glyph's scene and the selection and carries out its answer in order: interaction
//! steps as `Msg::Preview`, commands as `Msg::Run` (or shell and tool messages), selection and view changes directly.
//! Each event sees what the one before it did. [`overlays`] paints the selection and the tool's overlays as layers.
//!
//! The selection belongs to the edited glyph's layer on the edited side (decision 0004: the engine never stores it).
//! A command that returns a `selection` of that layer replaces it.

pub mod apply;
pub mod bridge;
pub mod overlays;
pub mod targets;
pub mod tokens;

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_engine::Session;
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::{Font, Layer, LayerSide, Master};
use fontcraft_tools::extra_nodes::{MAX_CROSSING_POINT_WORK, extra_nodes_within};
use fontcraft_tools::toolbox::Keyboard;
use fontcraft_tools::{KeyEvent, Modifiers, Overlay, PointerEvent, Scene, Selection, Toolbox};

use crate::app::ViewState;
use crate::shell::masters;

/// Input for the active tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolInput {
    /// A pointer event in the edited glyph's font units.
    Pointer(PointerEvent),
    /// The modifier keys changed while the pointer stood still.
    Modifiers(Modifiers),
    /// A key going down or coming up.
    Key(KeyEvent),
}

/// What the selection belongs to: the edited glyph, its active layer and the edited side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub glyph: String,
    pub layer: String,
    pub side: LayerSide,
}

impl Target {
    /// The target's layer: a master, special or backup layer. [`Glyph::layer`](fontcraft_engine::model::Glyph::layer)
    /// finds master layers only.
    pub fn layer_in<'f>(&self, font: &'f Font) -> Option<&'f Layer> {
        font.glyph(&self.glyph)?.layers.iter().find(|l| l.id == self.layer)
    }
}

/// What the canvas's tool layers paint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolView {
    pub selection: Selection,
    /// The paths whose nodes alone show and take the selection (`path.focusOnSelectedPath`).
    pub focus: Option<Vec<usize>>,
    /// What the active tool asks the canvas to draw, in font units.
    pub overlays: Vec<Overlay>,
    /// The edited glyph's placed images and the selected one (`canvas::images::refresh` keeps it).
    pub images: crate::canvas::images::ImagePaint,
    /// The selection's box, the transformation origin and the smart handles (`canvas::bounding_box::refresh` keeps them).
    pub marks: crate::canvas::bounding_box::Marks,
    /// The marks that attach at the selected anchors (`canvas::mark_cloud::refresh` keeps it).
    pub cloud: crate::canvas::mark_cloud::Cloud,
    /// The active tool is one under which Fill Preview fills the edited glyph: Text, Hand or Zoom.
    pub fills: bool,
}

/// The tools' side of the app: the toolbox, the selection and the running interaction.
pub struct Tools {
    pub toolbox: Toolbox,
    pub selection: Selection,
    pub focus: Option<Vec<usize>>,
    /// The glyph, layer and side the selection belongs to.
    target: Option<Target>,
    /// The selection when the running interaction began: put back when it is cancelled. `Some` while an interaction
    /// a tool began runs.
    before: Option<Selection>,
    /// The document revision the selection was last checked against.
    checked: Option<(u64, u64)>,
    /// The modifiers the toolbox last saw.
    mods: Modifiers,
    /// The edited side's placed image is selected (`canvas::images`); it belongs to the same target as the selection.
    pub image: bool,
    /// The selected smart components' handles and sizes.
    pub smart: crate::canvas::smart::SmartCache,
    /// What [`ToolView::cloud`] was worked out from.
    pub cloud: crate::canvas::mark_cloud::CloudCache,
    /// What [`ToolView::marks`] was worked out from.
    pub marks_key: Option<crate::canvas::bounding_box::MarksKey>,
    /// The nodes last selected on the foreground of each glyph that has some, by document and glyph
    /// ([`Tools::remembered`]).
    nodes: BTreeMap<(u64, String), BTreeSet<(usize, usize)>>,
    /// What the tool layers paint; [`Tools::refresh`] copies it from the state above.
    pub view: ToolView,
}

/// Most glyphs whose last node selection is remembered.
const MAX_REMEMBERED: usize = 4096;

impl Default for Tools {
    fn default() -> Self {
        let mut toolbox = Toolbox::new();
        toolbox.set_keyboard(keyboard());
        Tools {
            toolbox,
            selection: Selection::new(),
            focus: None,
            target: None,
            before: None,
            checked: None,
            mods: Modifiers::NONE,
            image: false,
            smart: crate::canvas::smart::SmartCache::default(),
            cloud: crate::canvas::mark_cloud::CloudCache::default(),
            marks_key: None,
            nodes: BTreeMap::new(),
            view: ToolView::default(),
        }
    }
}

/// Where the modifier keys come from: a Mac keeps Command and Control apart.
fn keyboard() -> Keyboard {
    if cfg!(target_os = "macos") { Keyboard::Mac } else { Keyboard::Other }
}

/// The selection of an Edit View tab that is not showing (`shell::tabs`), with the path focus and what they belong to.
#[derive(Clone, Debug)]
pub struct ParkedSelection {
    selection: Selection,
    focus: Option<Vec<usize>>,
    target: Option<Target>,
}

impl Default for ParkedSelection {
    fn default() -> Self {
        ParkedSelection { selection: Selection::new(), focus: None, target: None }
    }
}

impl ParkedSelection {
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Keeps the selection on glyph `old` after it was renamed `new`.
    pub fn rename_glyph(&mut self, old: &str, new: &str) {
        rename_target(&mut self.target, old, new);
    }
}

fn rename_target(target: &mut Option<Target>, old: &str, new: &str) {
    if let Some(t) = target
        && t.glyph == old
    {
        t.glyph = new.to_string();
    }
}

impl Tools {
    /// The nodes last selected in the edit view in `glyph` of document `document`, by shape and node; `None` when none
    /// were, or when the selection was last cleared there.
    pub fn remembered(&self, document: u64, glyph: &str) -> Option<&BTreeSet<(usize, usize)>> {
        let shown = self.target.as_ref().filter(|t| t.glyph == glyph && t.side == LayerSide::Foreground);
        let points = match shown.filter(|_| self.checked.is_some_and(|(id, _)| id == document)) {
            Some(_) => Some(&self.selection.points),
            None => self.nodes.get(&(document, glyph.to_string())),
        };
        points.filter(|points| !points.is_empty())
    }

    /// Remembers the nodes selected in the glyph the selection belongs to, before it leaves that glyph.
    fn remember(&mut self) {
        let (Some(target), Some((document, _))) = (&self.target, self.checked) else { return };
        if target.side != LayerSide::Foreground {
            return;
        }
        let key = (document, target.glyph.clone());
        if self.selection.points.is_empty() {
            self.nodes.remove(&key);
        } else if self.nodes.len() < MAX_REMEMBERED || self.nodes.contains_key(&key) {
            self.nodes.insert(key, self.selection.points.clone());
        }
    }

    /// Takes the selection out, leaving none: its tab goes behind another. A running interaction is not cancelled
    /// here; tabs switch between messages, when none runs.
    pub fn park(&mut self) -> ParkedSelection {
        self.remember();
        self.checked = None;
        self.before = None;
        // A tab's image selection is not kept: the image is selected again with a click.
        self.image = false;
        ParkedSelection { selection: std::mem::replace(&mut self.selection, Selection::new()), focus: self.focus.take(), target: self.target.take() }
    }

    /// Keeps the selection on glyph `old` after it was renamed `new`.
    pub fn rename_glyph(&mut self, old: &str, new: &str) {
        rename_target(&mut self.target, old, new);
        let Some((document, _)) = self.checked else { return };
        if let Some(points) = self.nodes.remove(&(document, old.to_string())) {
            self.nodes.insert((document, new.to_string()), points);
        }
    }

    /// Puts a tab's selection back: the tab shows again.
    pub fn restore(&mut self, parked: ParkedSelection) {
        self.checked = None;
        self.before = None;
        self.selection = parked.selection;
        self.focus = parked.focus;
        self.target = parked.target;
    }
}

impl Tools {
    /// Copies the selection, the focus and the active tool's overlays into [`Tools::view`], when they changed.
    pub fn refresh(&mut self) {
        let view = ToolView {
            selection: self.selection.clone(),
            focus: self.focus.clone(),
            overlays: self.toolbox.overlays(),
            images: self.view.images.clone(),
            marks: self.view.marks.clone(),
            cloud: self.view.cloud.clone(),
            fills: matches!(self.toolbox.active_id(), "text" | "hand" | "zoom"),
        };
        if view != self.view {
            self.view = view;
        }
    }

    /// The glyph, layer and side the selection belongs to.
    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }
}

/// The glyph the canvas edits: the shown one, else the font's first.
pub fn edited_glyph<'a>(font: &'a Font, view: &'a ViewState) -> Option<&'a str> {
    match &view.glyph {
        Some(name) if font.glyphs.get(name).is_some() => Some(name.as_str()),
        _ => font.glyphs.first().map(|g| g.name.as_str()),
    }
}

/// The edited side of the layer.
pub fn side(view: &ViewState) -> LayerSide {
    if view.edit_background { LayerSide::Background } else { LayerSide::Foreground }
}

/// The active layer: the edited glyph's layer the Layers panel picked (`layers.select`), else its layer of the
/// current master (`view.selectMaster`). The canvas draws it and the tools edit it.
pub fn active_layer<'a>(font: &'a Font, view: &ViewState) -> Option<&'a Layer> {
    let glyph = font.glyph(edited_glyph(font, view)?)?;
    masters::shown_layer(font, &view.layers, glyph)
}

/// The active layer's master, else the current master: its metric lines, zones and curve tension apply.
pub fn active_master<'a>(font: &'a Font, view: &ViewState) -> Option<&'a Master> {
    match active_layer(font, view) {
        Some(layer) => masters::layer_master(font, &view.layers, layer),
        None => masters::current_master(font, &view.layers),
    }
}

/// What the canvas edits now: the edited glyph's active layer, on the edited side.
pub fn target(session: &Session, view: &ViewState) -> Option<Target> {
    let font = session.font().ok()?;
    let glyph = edited_glyph(font, view)?;
    let layer = active_layer(font, view)?;
    Some(Target { glyph: glyph.to_string(), layer: layer.id.clone(), side: side(view) })
}

/// The scene the tools work on: the edited side of the edited glyph's active layer, as the canvas shows it.
/// `data` is the session's glyph data, so the snapped metric lines are the drawn ones.
pub fn scene<'a>(font: &'a Font, data: Option<&GlyphData>, view: &ViewState, focus: Option<&Vec<usize>>) -> Option<Scene<'a>> {
    let glyph = edited_glyph(font, view)?;
    let layer = active_layer(font, view)?;
    let mut scene = Scene::from_font_side(font, data, glyph, Some(&layer.id), side(view))?;
    scene.focus = focus.cloned();
    scene.background_shown = view.display.background;
    scene.guides_shown = view.display.guides;
    if view.display.extra_nodes {
        scene.extra_nodes = extra_nodes_within(&scene.layer, MAX_CROSSING_POINT_WORK);
    }
    Some(scene)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod attach_tests;
