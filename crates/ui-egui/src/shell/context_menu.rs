//! The canvas context menu: a right-click (or a Ctrl+click on the Mac) on the canvas opens a menu
//! of the commands whose inventory row names the clicked kind of object in `context`.
//!
//! - The click is hit-tested as a tool click is. An object that is not selected becomes the
//!   selection first, so the items act on what the menu was opened on; a click on empty canvas
//!   keeps the selection.
//! - The menu has a section for the object, then a section for the glyph (`canvas`), split by a
//!   separator. The object section is `canvas.node`, `canvas.segment`, `canvas.anchor`,
//!   `canvas.component` or `canvas.guide` for the clicked object, and `canvas.path` when that
//!   object's path is wholly selected. On empty canvas it follows the selection.
//! - Items keep the order of their inventory rows ([`ORDER`]) and are greyed out as menu bar
//!   items are (`tools_glue::selection_supplies`).
//! - An item runs through `run_from_shell` with its targets from the selection
//!   (`tools_glue::with_targets`), plus what the click adds: the clicked segment, node or guide,
//!   and the click position for the commands that place something.

use egui::{Align, Layout, Pos2};
use fontcraft_engine::params::TargetKind;
use fontcraft_engine::{CommandSpec, Session, command_specs, find_command};
use fontcraft_geom::Point;
use fontcraft_tools::{Hit, HitResult, Scene, Selection};
use serde::Serialize;
use serde_json::{Value, json};

use super::UI_COMMANDS;
use crate::app::FontcraftApp;
use crate::state::UiState;
use crate::tools_glue::{self, Space};

pub const CANVAS: &str = "canvas";
pub const NODE: &str = "canvas.node";
pub const SEGMENT: &str = "canvas.segment";
pub const PATH: &str = "canvas.path";
pub const ANCHOR: &str = "canvas.anchor";
pub const COMPONENT: &str = "canvas.component";
pub const GUIDE: &str = "canvas.guide";

/// Shell commands that context menus show, with their rows' `context` (engine commands carry
/// theirs in `CommandSpec::context`).
const UI_CONTEXT: &[(&str, &[&str])] = &[("path.focusOnSelectedPath", &[NODE, PATH]), ("path.clearFocusOnPaths", &[CANVAS])];

/// The order of context menu items: the order of their inventory rows. A command missing here
/// comes after these, in registry order.
const ORDER: &[&str] = &[
    "path.toggleNodeType",
    "path.convertSegmentToCurve",
    "path.connectNodes",
    "path.closeOpenPaths",
    "path.openCorner",
    "path.reconnectNodes",
    "path.sharpenCorner",
    "path.makeNodeFirst",
    "path.reverseContours",
    "path.insertNode",
    "path.focusOnSelectedPath",
    "path.clearFocusOnPaths",
    "path.lockPath",
    "path.unlockPath",
    "glyph.toggleLocked",
    "path.expandOutline",
    "path.addPenPoint",
    "path.setMask",
    "guide.add",
    "guide.addGlobal",
    "guide.makeGlobal",
    "guide.makeLocal",
    "guide.lock",
    "guide.unlock",
    "guide.remove",
    "guide.toggleMeasurement",
];

/// One entry of a context menu.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContextItem {
    pub command: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

/// The items of one `context` value; sections are drawn with a separator between them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Section {
    pub context: &'static str,
    pub items: Vec<ContextItem>,
}

/// An open canvas context menu.
#[derive(Clone, Debug, Serialize)]
pub struct ContextMenu {
    /// The tab and glyph it was opened on; it closes when either changes.
    pub tab: u64,
    pub glyph: String,
    /// Where it opens, in window points.
    pub screen: Point,
    /// The click in the edited glyph's font units.
    pub at: Point,
    /// The object under the click.
    pub hit: Option<HitResult>,
    pub sections: Vec<Section>,
}

impl ContextMenu {
    pub fn items(&self) -> impl Iterator<Item = &ContextItem> {
        self.sections.iter().flat_map(|s| s.items.iter())
    }

    pub fn item(&self, id: &str) -> Option<&ContextItem> {
        self.items().find(|i| i.command == id)
    }
}

/// Opens the canvas context menu for a click at `at`: selects the clicked object when it is not
/// selected, then lists the commands for it. Does nothing while a tool is dragging or a dialog
/// is open.
pub fn open(app: &mut FontcraftApp, at: Point, space: Space) -> Result<(), String> {
    if app.tools.toolbox.busy() || app.dialog.is_some() {
        return Ok(());
    }
    let (pos, hit) = tools_glue::hit_at(app, at, space)?;
    let screen = match space {
        Space::Screen => at,
        Space::Font => tools_glue::current_view(app).map_or(at, |v| v.to_screen(at)),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab_mut().ok_or("no Edit View tab is showing")?;
    if let Some(h) = &hit
        && !h.selected_in(&tab.selection)
    {
        tab.selection = Selection::from_items(h.items());
    }
    let scene = tools_glue::scene(font, tab).ok_or("the edited glyph has no layer in the active master")?;
    let kinds = kinds(&scene, &tab.selection, hit.as_ref());
    let (id, glyph) = (tab.id, tab.edited_glyph().unwrap_or_default().to_string());
    let sections = sections(&app.session, &app.state, &kinds);
    app.tools.context_menu = Some(ContextMenu { tab: id, glyph, screen, at: pos, hit, sections });
    Ok(())
}

/// The context menus a click shows, object first, `canvas` last.
pub fn kinds(scene: &Scene, selection: &Selection, hit: Option<&HitResult>) -> Vec<&'static str> {
    let whole = tools_glue::selected_paths(scene, selection);
    let object = match hit.map(|h| &h.hit) {
        Some(Hit::Node { shape, .. } | Hit::Handle { shape, .. } | Hit::Segment { shape, .. }) if whole.contains(shape) => Some(PATH),
        Some(Hit::Node { .. } | Hit::Handle { .. }) => Some(NODE),
        Some(Hit::Segment { .. }) => Some(SEGMENT),
        Some(Hit::Anchor { .. }) => Some(ANCHOR),
        Some(Hit::Component { .. }) => Some(COMPONENT),
        Some(Hit::Guide { .. }) => Some(GUIDE),
        None if !whole.is_empty() || selection.shapes.iter().any(|&s| scene.path(s).is_some()) => Some(PATH),
        None if !selection.on_curve_points(scene).is_empty() => Some(NODE),
        None if !selection.guides.is_empty() => Some(GUIDE),
        None if !selection.anchors.is_empty() => Some(ANCHOR),
        None if !selection.shapes.is_empty() => Some(COMPONENT),
        None => None,
    };
    object.into_iter().chain([CANVAS]).collect()
}

/// A command a context menu may show.
struct Candidate {
    id: &'static str,
    label: &'static str,
    context: &'static [&'static str],
    spec: Option<&'static CommandSpec>,
}

/// The sections of a context menu showing `kinds`, in that order. A command listed for several
/// of them appears only in the first.
pub fn sections(session: &Session, state: &UiState, kinds: &[&'static str]) -> Vec<Section> {
    let mut all: Vec<Candidate> = command_specs()
        .iter()
        .filter(|c| !c.context.is_empty())
        .map(|c| Candidate { id: c.id, label: c.label, context: c.context, spec: Some(c) })
        .collect();
    for (id, context) in UI_CONTEXT {
        if let Some(c) = UI_COMMANDS.iter().find(|c| c.0 == *id) {
            all.push(Candidate { id: c.0, label: c.1, context: *context, spec: None });
        }
    }
    let rank = |id: &str| ORDER.iter().position(|o| *o == id).unwrap_or(ORDER.len());
    all.sort_by_key(|c| rank(c.id));
    let mut shown: Vec<&'static str> = Vec::new();
    let mut out = Vec::new();
    for &kind in kinds {
        let items: Vec<ContextItem> =
            all.iter().filter(|c| c.context.contains(&kind) && !shown.contains(&c.id)).map(|c| item(session, state, c)).collect();
        shown.extend(items.iter().map(|i| i.command));
        if !items.is_empty() {
            out.push(Section { context: kind, items });
        }
    }
    out
}

fn item(session: &Session, state: &UiState, c: &Candidate) -> ContextItem {
    let check = match c.spec {
        Some(spec) => (spec.enabled)(session).and_then(|()| tools_glue::selection_supplies(session, state, spec)),
        None => ui_enabled(state, c.id),
    };
    ContextItem { command: c.id, label: c.label, enabled: check.is_ok(), disabled_reason: check.err() }
}

/// Whether a shell command of [`UI_CONTEXT`] can run now.
fn ui_enabled(state: &UiState, id: &str) -> Result<(), String> {
    let tab = state.tab().ok_or("no Edit View tab is showing")?;
    match id {
        "path.focusOnSelectedPath" if tab.selection.points.is_empty() => Err("select a point of the paths to focus on first".into()),
        "path.clearFocusOnPaths" if tab.focus.is_none() => Err("no path has the focus".into()),
        _ => Ok(()),
    }
}

/// The parameters item `id` of `menu` runs with: the targets from the selection, plus what the
/// click adds.
pub fn params_for(app: &FontcraftApp, menu: &ContextMenu, id: &str) -> Value {
    let base = tools_glue::with_targets(app, id, Value::Null);
    let Some(spec) = find_command(id) else { return base };
    let mut m = match base {
        Value::Object(m) => m,
        other => return other,
    };
    let takes = |k: TargetKind| spec.targets.contains(&k);
    let hit = menu.hit.as_ref().map(|h| &h.hit);
    let hit_at = menu.hit.as_ref().map_or(menu.at, |h| h.at);
    if takes(TargetKind::Segments)
        && !takes(TargetKind::Points)
        && let Some(Hit::Segment { shape, node, .. }) = hit
    {
        m.insert("segments".into(), json!([[shape, node]]));
    }
    // A guide goes through the clicked node, else through the click.
    let place = if matches!(hit, Some(Hit::Node { .. })) { hit_at } else { menu.at };
    match id {
        "path.makeNodeFirst" => {
            if let Some(Hit::Node { shape, node }) = hit {
                m.remove("points");
                m.insert("point".into(), json!([shape, node]));
            }
        }
        "path.insertNode" => {
            let near = if matches!(hit, Some(Hit::Segment { .. })) { hit_at } else { menu.at };
            m.insert("near".into(), json!([near.x, near.y]));
        }
        "guide.add" => match with_scene(app, |scene, selection| selection.on_curve_points(scene)).as_deref() {
            Some([a, b]) => {
                m.insert("throughPoints".into(), json!([[a.0, a.1], [b.0, b.1]]));
            }
            _ => {
                m.insert("x".into(), json!(place.x));
                m.insert("y".into(), json!(place.y));
            }
        },
        "guide.addGlobal" => {
            m.insert("x".into(), json!(place.x));
            m.insert("y".into(), json!(place.y));
        }
        "guide.toggleMeasurement" => {
            let guide = match hit {
                Some(Hit::Guide { guide }) => Some(*guide),
                _ => app.state.tab().and_then(|t| t.selection.guides.iter().next().copied()),
            };
            if let Some(g) = guide {
                m.insert("guide".into(), json!({"scope": g.scope.as_str(), "index": g.index}));
            }
        }
        "path.setMask" => {
            let shapes: Vec<usize> = m
                .get("shapes")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_u64).filter_map(|v| usize::try_from(v).ok()).collect())
                .unwrap_or_default();
            let masked =
                with_scene(app, |scene, _| !shapes.is_empty() && shapes.iter().all(|&s| scene.path(s).is_some_and(|p| p.info.attrs.mask.is_some())));
            m.insert("mask".into(), json!(!masked.unwrap_or(false)));
        }
        _ => {}
    }
    Value::Object(m)
}

/// Runs `f` with the edited glyph's scene and the selection.
fn with_scene<R>(app: &FontcraftApp, f: impl FnOnce(&Scene, &Selection) -> R) -> Option<R> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    let scene = tools_glue::scene(font, tab)?;
    Some(f(&scene, &tab.selection))
}

/// Runs item `id` of the open context menu as a click on it does, and closes the menu.
pub fn invoke(app: &mut FontcraftApp, id: &str) -> Result<Value, String> {
    let menu = app.tools.context_menu.take().ok_or("no context menu is open")?;
    let item = menu.item(id).ok_or_else(|| format!("`{id}` is not in the context menu"))?;
    if !item.enabled {
        return Err(item.disabled_reason.clone().unwrap_or_else(|| format!("`{id}` is not available now")));
    }
    let params = params_for(app, &menu, id);
    app.run_from_shell(id, &params)
}

/// Draws the open context menu, and queues the command of the item picked in it.
pub fn show(app: &mut FontcraftApp, ui: &egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    let Some(menu) = app.tools.context_menu.clone() else { return };
    let same = app.state.tab().is_some_and(|t| t.id == menu.tab && t.edited_glyph() == Some(menu.glyph.as_str()));
    if !same || app.dialog.is_some() {
        app.tools.context_menu = None;
        return;
    }
    let mut open = true;
    let mut picked = None;
    let anchor = Pos2::new(menu.screen.x as f32, menu.screen.y as f32);
    egui::Popup::new(egui::Id::new("fontcraft.canvas.contextMenu"), ui.ctx().clone(), anchor, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .layout(Layout::top_down_justified(Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_bool(&mut open)
        .show(|ui| {
            for (index, section) in menu.sections.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                for item in &section.items {
                    let response = ui.add_enabled(item.enabled, egui::Button::new(item.label));
                    let response = match &item.disabled_reason {
                        Some(why) => response.on_disabled_hover_text(why),
                        None => response,
                    };
                    if response.clicked() {
                        picked = Some(item.command);
                    }
                }
            }
        });
    if let Some(id) = picked {
        let params = params_for(app, &menu, id);
        pending.push((id, params));
        open = false;
    }
    if !open {
        app.tools.context_menu = None;
    }
}
