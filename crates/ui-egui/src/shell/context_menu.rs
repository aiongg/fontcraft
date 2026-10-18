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
//!   items are (`tools_glue::selection_supplies`). The selection also hides items that do not
//!   apply (Unlock Path on an unlocked path) and checks checkable ones ([`super::item_state`]).
//! - Add Global Guide replaces Add Guide while Alt is held ([`ContextItem::alternate`]).
//! - A guide's menu ends with Guide Properties…, which opens that guide's dialog.
//! - Items that need a choice first ask for it: Add Component and Change Component Glyph open the
//!   glyph picker, Add Corner Component the corner glyph picker
//!   (`dialogs::glyph_picker_component`), and Component from Selection its name dialog. A node
//!   that holds a corner component offers Decompose Corner. The attachment anchor is chosen in
//!   the component Info box, not here.
//! - An item runs through `run_from_shell` with its targets from the selection
//!   (`tools_glue::with_targets`), plus what the click adds: the clicked segment, node or guide,
//!   and the click position for the commands that place something.
//! - The glyph section starts with the twelve colour label swatches and a cross
//!   (`editView.contextMenu`): they set the edited glyph's colour, or with Alt held the active
//!   layer's. It offers Exports, Locked and Show Glyph in Font View, and ends with the Copy
//!   Glyph Names submenu.
//!
//! The Font View glyph menu (`fontView.glyphContextMenu`, [`font_view`]) opens on a right-click
//! on a cell or a List View row. A glyph that is not selected becomes the selection first; on a
//! selected glyph the menu acts on the whole selection. It holds the colour swatches (Alt: the
//! layer colour in Font View's master), the Tags submenu (`fontView.contextMenu.tags`) and the
//! Copy Glyph Names submenu, whose text goes to the system clipboard.

use std::collections::{BTreeSet, HashMap};

use egui::{Align, Color32, Layout, Pos2, Sense, Stroke, vec2};
use fontcraft_engine::params::TargetKind;
use fontcraft_engine::{CommandSpec, Session, command_specs, find_command};
use fontcraft_geom::Point;
use fontcraft_model::ColorLabel;
use fontcraft_tools::{Hit, HitResult, Scene, Selection};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::UI_COMMANDS;
use super::font_view::columns::COLOR_LABEL_NAMES;
use super::item_state::{self, GUIDE_PROPERTIES};
use crate::app::FontcraftApp;
use crate::panels::widgets;
use crate::state::UiState;
use crate::theme::Tokens;
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
const UI_CONTEXT: &[(&str, &[&str])] =
    &[("path.focusOnSelectedPath", &[NODE, PATH]), ("path.clearFocusOnPaths", &[CANVAS]), ("glyph.showGlyphsUsingAsComponent", &[CANVAS])];

/// Engine commands the object sections show, whose registry entries list no context.
const OBJECT_ITEMS: &[(&str, &[&str])] = &[("cornerComponent.add", &[NODE]), ("cornerComponent.decompose", &[NODE])];

/// Commands whose registry entries list a context but that the menus leave out: the
/// attachment anchor is chosen from the anchor menu of the component Info box.
const HIDDEN: &[&str] = &["component.setAnchor"];

/// Items that open a picker or a dialog for their choice instead of running at once.
const ASKS: &[&str] = &["glyph.addComponent", "component.setBaseGlyph", "cornerComponent.add", "glyph.componentFromSelection"];

/// Engine commands the glyph section shows, whose registry entries list no context.
const GLYPH_ITEMS: &[&str] = &["glyph.setExport"];

/// The glyph section's Show Glyph in Font View: selects the edited glyph in Font View and shows
/// Font View.
pub const SHOW_IN_FONT_VIEW: &str = "contextMenu.showGlyphInFontView";

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
    "glyph.setExport",
    "glyph.toggleLocked",
    SHOW_IN_FONT_VIEW,
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
    /// Whether a checkable item is checked; absent for other items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Drawn only while Alt is held, in place of the item before it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub alternate: bool,
}

/// Items drawn only while Alt is held.
const ALTERNATES: &[&str] = &["guide.addGlobal"];

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
            all.push(Candidate { id: c.0, label: c.1, context, spec: None });
        }
    }
    all.push(Candidate { id: GUIDE_PROPERTIES, label: "Guide Properties…", context: &[GUIDE], spec: None });
    for spec in GLYPH_ITEMS.iter().filter_map(|id| find_command(id)) {
        all.push(Candidate { id: spec.id, label: spec.label, context: &[CANVAS], spec: Some(spec) });
    }
    for (id, context) in OBJECT_ITEMS {
        if let Some(spec) = find_command(id).filter(|s| s.context.is_empty()) {
            all.push(Candidate { id: spec.id, label: spec.label, context, spec: Some(spec) });
        }
    }
    all.push(Candidate { id: SHOW_IN_FONT_VIEW, label: "Show Glyph in Font View", context: &[CANVAS], spec: None });
    let rank = |id: &str| ORDER.iter().position(|o| *o == id).unwrap_or(ORDER.len());
    all.sort_by_key(|c| rank(c.id));
    let font = session.font().ok();
    let tab = state.tab();
    let scene = font.zip(tab).and_then(|(font, tab)| tools_glue::scene(font, tab));
    let selection = tab.map(|t| &t.selection);
    // On a guide the menu offers the guide's commands; adding a guide is for clicks off guides.
    let on_guide = kinds.contains(&GUIDE);
    let visible = |id: &str| match (&scene, selection) {
        _ if on_guide && matches!(id, "guide.add" | "guide.addGlobal") => false,
        _ if HIDDEN.contains(&id) => false,
        (Some(scene), Some(selection)) if id == "cornerComponent.decompose" => !selected_corners(scene, selection).is_empty(),
        (Some(scene), Some(selection)) => item_state::visible(scene, selection, id),
        _ => id != GUIDE_PROPERTIES,
    };
    let mut shown: Vec<&'static str> = Vec::new();
    let mut out = Vec::new();
    for &kind in kinds {
        let items: Vec<ContextItem> = all
            .iter()
            .filter(|c| c.context.contains(&kind) && !shown.contains(&c.id) && visible(c.id))
            .map(|c| item(session, state, scene.as_ref().zip(selection), c))
            .collect();
        shown.extend(items.iter().map(|i| i.command));
        if !items.is_empty() {
            out.push(Section { context: kind, items });
        }
    }
    out
}

fn item(session: &Session, state: &UiState, at: Option<(&Scene, &Selection)>, c: &Candidate) -> ContextItem {
    let check = match c.spec {
        Some(spec) => (spec.enabled)(session).and_then(|()| tools_glue::selection_supplies(session, state, spec)),
        None => ui_enabled(state, c.id),
    };
    ContextItem {
        command: c.id,
        label: c.label,
        enabled: check.is_ok(),
        disabled_reason: check.err(),
        checked: match c.id {
            "glyph.setExport" => edited_exports(session, state),
            _ => at.and_then(|(scene, selection)| item_state::checked(scene, selection, c.id)),
        },
        alternate: ALTERNATES.contains(&c.id),
    }
}

/// Whether the edited glyph exports.
fn edited_exports(session: &Session, state: &UiState) -> Option<bool> {
    let font = session.font().ok()?;
    font.glyph(state.tab()?.edited_glyph()?).map(|g| g.export)
}

/// Whether a shell command of [`UI_CONTEXT`] can run now.
fn ui_enabled(state: &UiState, id: &str) -> Result<(), String> {
    let tab = state.tab().ok_or("no Edit View tab is showing")?;
    match id {
        "path.focusOnSelectedPath" if tab.selection.points.is_empty() => Err("select a point of the paths to focus on first".into()),
        "path.clearFocusOnPaths" if tab.focus.is_none() => Err("no path has the focus".into()),
        GUIDE_PROPERTIES if tab.selection.guides.len() != 1 => Err("select one guide first".into()),
        _ => Ok(()),
    }
}

/// The parameters item `id` of `menu` runs with: the targets from the selection, plus what the
/// click adds.
pub fn params_for(app: &FontcraftApp, menu: &ContextMenu, id: &str) -> Value {
    if id == GUIDE_PROPERTIES {
        return guide_properties(app).unwrap_or(Value::Null);
    }
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
        "cornerComponent.decompose" => {
            if let Some(corners) = with_scene(app, selected_corners) {
                m.insert("corners".into(), json!(corners));
            }
        }
        "glyph.setExport" => {
            let exports = edited_exports(&app.session, &app.state).unwrap_or(true);
            m.insert("glyphs".into(), json!([menu.glyph]));
            m.insert("export".into(), json!(!exports));
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

/// The corner components on the selected nodes, by hint index.
fn selected_corners(scene: &Scene, selection: &Selection) -> Vec<usize> {
    crate::dialogs::glyph_picker_component::corners_on_nodes(&scene.layer, selection.points.iter().copied())
}

/// The `dialog.open` parameters of Guide Properties… for the one selected guide.
fn guide_properties(app: &FontcraftApp) -> Option<Value> {
    with_scene(app, |scene, selection| {
        let r = *selection.guides.iter().next()?;
        let g = fontcraft_tools::guide_drag::guide(scene, r)?;
        Some(json!({"dialog": GUIDE_PROPERTIES, "params": fontcraft_tools::guide_drag::properties(scene, r, g)}))
    })
    .flatten()
}

/// The commands item `id` of `menu` runs, in order: the item's command, then for a new guide
/// the selection of that guide.
pub fn steps_for(app: &FontcraftApp, menu: &ContextMenu, id: &'static str) -> Vec<(&'static str, Value)> {
    let params = params_for(app, menu, id);
    if id == GUIDE_PROPERTIES {
        return vec![("dialog.open", params)];
    }
    if id == SHOW_IN_FONT_VIEW {
        return vec![("fontView.setSelection", json!({"glyphs": [menu.glyph]})), ("fontView.show", json!({}))];
    }
    if ASKS.contains(&id) {
        return vec![ask(id, params)];
    }
    let new_guide = match id {
        "guide.add" => with_scene(app, |scene, _| json!({"scope": "local", "index": scene.local_guides().len()})),
        "guide.addGlobal" => with_scene(app, |scene, _| json!({"scope": "global", "index": scene.global_guides.len()})),
        _ => None,
    };
    let mut out = vec![(id, params)];
    if let Some(guide) = new_guide {
        out.push(("selection.set", json!({ "guides": [guide] })));
    }
    out
}

/// The picker or dialog an item of [`ASKS`] opens, with the item's targets: the picker takes the
/// rest from the selection the click made.
fn ask(id: &'static str, params: Value) -> (&'static str, Value) {
    use crate::dialogs::glyph_picker_component::{COMMAND, Purpose};
    let purpose = match id {
        "glyph.addComponent" => Purpose::AddComponent,
        "component.setBaseGlyph" => Purpose::BaseGlyph,
        "cornerComponent.add" => Purpose::AddCorner,
        _ => return ("dialog.open", json!({"command": id, "params": params})),
    };
    (COMMAND, json!({"purpose": purpose.name()}))
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
    let mut result = None;
    for (command, params) in steps_for(app, &menu, item.command) {
        let value = app.run_from_shell(command, &params)?;
        // The first step is the item's command, or the picker or dialog that asks for it.
        result.get_or_insert(value);
    }
    Ok(result.unwrap_or(Value::Null))
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
    let mut action = None;
    let anchor = Pos2::new(menu.screen.x as f32, menu.screen.y as f32);
    let alt = ui.input(|i| i.modifiers.alt);
    let tokens = app.tokens.clone();
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
                if section.context == CANVAS
                    && let Some(color) = swatches(ui, &tokens, CANVAS_SALT, alt)
                {
                    action = Some(if alt { GlyphAction::LayerColor(color) } else { GlyphAction::Color(color) });
                }
                for (at, item) in section.items.iter().enumerate() {
                    let replaced = section.items.get(at + 1).is_some_and(|next| next.alternate);
                    if (item.alternate && !alt) || (replaced && alt) {
                        continue;
                    }
                    let response = match item.checked {
                        Some(mut on) => ui.add_enabled(item.enabled, egui::Checkbox::new(&mut on, item.label)),
                        None => ui.add_enabled(item.enabled, egui::Button::new(item.label)),
                    };
                    let response = match &item.disabled_reason {
                        Some(why) => response.on_disabled_hover_text(why),
                        None => response,
                    };
                    if response.clicked() {
                        picked = Some(item.command);
                    }
                }
            }
            if let Some(format) = copy_names_menu(ui) {
                action = Some(GlyphAction::CopyNames(format));
            }
        });
    if let Some(id) = picked {
        pending.extend(steps_for(app, &menu, id));
        open = false;
    }
    if let Some(action) = action {
        let layer =
            app.session.font().ok().zip(app.state.tab()).and_then(|(font, tab)| crate::canvas::view::active_layer(font, tab)).map(|l| l.id.clone());
        let target = GlyphMenu { glyphs: vec![menu.glyph.clone()], screen: [anchor.x, anchor.y], tags: Vec::new(), layer, pass: 0 };
        run_glyph_action(app, &target, &action, pending);
        open = false;
    }
    if !open {
        app.tools.context_menu = None;
    }
}

// ── Colour swatches and the Font View glyph menu ──

/// The salt of the canvas menu's swatches (`widgets::id((CANVAS_SALT, "color", i))`).
pub const CANVAS_SALT: &str = "canvas.contextMenu";
/// The salt of the Font View glyph menu's swatches.
pub const GLYPH_MENU_SALT: &str = "fontView.glyphMenu";
/// Most tags the Tags submenu lists.
pub const MAX_MENU_TAGS: usize = 512;

/// The formats of Copy Glyph Names: the `glyph.copyGlyphNames` format and the item's label.
pub const COPY_FORMATS: [(&str, &str); 6] = [
    ("spaceSeparated", "Space Separated"),
    ("onePerLine", "One per Line"),
    ("commaSeparated", "Comma Separated"),
    ("slashed", "Slashed"),
    ("pythonList", "Python List"),
    ("unicodes", "Unicodes"),
];

/// How many of the menu's glyphs carry a tag: a checkmark, a dash or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TagMark {
    All,
    Some,
    None,
}

/// An open Font View glyph context menu. It lives in egui's memory under [`glyph_menu_id`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GlyphMenu {
    /// The glyphs its items act on.
    pub glyphs: Vec<String>,
    /// Where it opens, in window points.
    pub screen: [f32; 2],
    /// Every tag the font uses, sorted, with how many of `glyphs` carry it. Empty: no Tags
    /// submenu.
    pub tags: Vec<(String, TagMark)>,
    /// The layer the swatches colour while Alt is held: Font View's master.
    pub layer: Option<String>,
    /// The egui pass that last drew it: a menu Font View stopped drawing is dropped.
    #[serde(skip)]
    pub pass: u64,
}

/// What an item of a glyph menu does.
#[derive(Clone, Debug, PartialEq)]
pub enum GlyphAction {
    /// A swatch (`Some(index)`) or the cross (`None`).
    Color(Option<u8>),
    /// A swatch or the cross with Alt held.
    LayerColor(Option<u8>),
    /// A tag of the Tags submenu: adds it to every glyph.
    Tag(String),
    /// A format of Copy Glyph Names.
    CopyNames(&'static str),
}

pub fn glyph_menu_id() -> egui::Id {
    egui::Id::new("fontcraft.fontView.glyphMenu")
}

/// The glyph menu for `glyphs`, opening at `screen`.
pub fn glyph_menu(app: &FontcraftApp, glyphs: Vec<String>, screen: [f32; 2]) -> GlyphMenu {
    let mut tags = Vec::new();
    if let Ok(font) = app.session.font() {
        let used: BTreeSet<&str> = font.glyphs.iter().flat_map(|g| g.info.tags.iter().map(String::as_str)).collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for g in glyphs.iter().filter_map(|n| font.glyph(n)) {
            for tag in &g.info.tags {
                *counts.entry(tag.as_str()).or_insert(0) += 1;
            }
        }
        let total = glyphs.iter().filter(|n| font.glyph(n).is_some()).count();
        tags = used
            .into_iter()
            .take(MAX_MENU_TAGS)
            .map(|tag| {
                let mark = match counts.get(tag).copied().unwrap_or(0) {
                    0 => TagMark::None,
                    n if n == total => TagMark::All,
                    _ => TagMark::Some,
                };
                (tag.to_string(), mark)
            })
            .collect();
    }
    GlyphMenu { glyphs, screen, tags, layer: super::font_view::master(app), pass: 0 }
}

/// The commands `action` runs on the menu's glyphs.
pub fn glyph_steps(menu: &GlyphMenu, action: &GlyphAction) -> Vec<(&'static str, Value)> {
    let color = |c: &Option<u8>| c.map_or(Value::Null, |i| json!(i));
    match action {
        GlyphAction::Color(c) => vec![("glyph.setColorLabel", json!({"glyphs": menu.glyphs, "color": color(c)}))],
        GlyphAction::LayerColor(c) => {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(menu.glyphs));
            if let Some(layer) = &menu.layer {
                m.insert("layer".into(), json!(layer));
            }
            m.insert("color".into(), color(c));
            vec![("glyph.setLayerColorLabel", Value::Object(m))]
        }
        GlyphAction::Tag(tag) => vec![("glyph.addTag", json!({"glyphs": menu.glyphs, "tag": tag}))],
        GlyphAction::CopyNames(format) => vec![("glyph.copyGlyphNames", json!({"glyphs": menu.glyphs, "format": format}))],
    }
}

/// Runs `action` as a click on its item does: Copy Glyph Names at once, its text to the system
/// clipboard; the others as commands queued for the end of the frame.
fn run_glyph_action(app: &mut FontcraftApp, menu: &GlyphMenu, action: &GlyphAction, pending: &mut Vec<(&'static str, Value)>) {
    if matches!(action, GlyphAction::CopyNames(_)) {
        if let Err(e) = invoke_glyph(app, menu, action) {
            app.state.status = e;
        }
    } else {
        pending.extend(glyph_steps(menu, action));
    }
}

/// Runs `action` of `menu` now; Copy Glyph Names also puts its text on the system clipboard.
pub fn invoke_glyph(app: &mut FontcraftApp, menu: &GlyphMenu, action: &GlyphAction) -> Result<Value, String> {
    let mut result = Value::Null;
    for (id, params) in glyph_steps(menu, action) {
        result = app.run_from_shell(id, &params)?;
    }
    if matches!(action, GlyphAction::CopyNames(_))
        && let Some(text) = result.get("text").and_then(Value::as_str)
    {
        app.clipboard_out = Some(text.to_string());
    }
    Ok(result)
}

/// The colour of a label as a swatch shows it.
pub fn label_color(tokens: &Tokens, label: ColorLabel) -> Color32 {
    match label {
        ColorLabel::Index(i) => tokens.color_labels.get(usize::from(i)).copied().unwrap_or(tokens.text_dim),
        ColorLabel::Rgba([r, g, b, _]) => {
            let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            Color32::from_rgb(byte(r), byte(g), byte(b))
        }
    }
}

/// The swatch row: a caption, the twelve label colours and a cross. Returns the colour picked
/// (`Some(None)`: the cross).
fn swatches(ui: &mut egui::Ui, tokens: &Tokens, salt: &str, alt: bool) -> Option<Option<u8>> {
    let mut picked = None;
    ui.label(egui::RichText::new(if alt { "Layer Color" } else { "Glyph Color" }).color(tokens.text_dim));
    let side = ui.spacing().interact_size.y * 0.7;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = side * 0.25;
        for (i, (color, name)) in tokens.color_labels.iter().zip(COLOR_LABEL_NAMES).enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            let response = ui.interact(rect, widgets::id((salt, "color", i)), Sense::click()).on_hover_text(name);
            ui.painter().circle_filled(rect.center(), side / 2.0 - 1.0, *color);
            if response.hovered() {
                ui.painter().circle_stroke(rect.center(), side / 2.0, Stroke::new(1.0, tokens.text));
            }
            if response.clicked() {
                picked = Some(u8::try_from(i).ok());
            }
        }
        let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        let response = ui.interact(rect, widgets::id((salt, "color", "none")), Sense::click()).on_hover_text("No Color");
        let color = if response.hovered() { tokens.text } else { tokens.text_dim };
        let r = rect.shrink(side * 0.2);
        ui.painter().line_segment([r.left_top(), r.right_bottom()], Stroke::new(1.5, color));
        ui.painter().line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.5, color));
        if response.clicked() {
            picked = Some(None);
        }
    });
    ui.separator();
    picked
}

/// The Copy Glyph Names submenu. Returns the format picked.
fn copy_names_menu(ui: &mut egui::Ui) -> Option<&'static str> {
    let mut picked = None;
    ui.menu_button("Copy Glyph Names", |ui| {
        for (format, label) in COPY_FORMATS {
            if ui.button(label).clicked() {
                picked = Some(format);
            }
        }
    });
    picked
}

/// Opens the Font View glyph menu on a right-click on a laid-out cell or row, and draws the open
/// one. Call before Font View's central area is placed: the click is tested against the cells
/// of the last frame, inside the space left for them.
pub fn font_view(app: &mut FontcraftApp, ui: &egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    let id = glyph_menu_id();
    let area = ui.available_rect_before_wrap();
    let click = ui.input(|i| if i.pointer.secondary_clicked() { i.pointer.interact_pos() } else { None });
    if let Some(at) = click
        && area.contains(at)
        && app.dialog.is_none()
        && ui.ctx().layer_id_at(at).is_none_or(|layer| layer == ui.layer_id())
        && let Some(name) = app.font_view.laid_out.iter().find(|(_, r)| r.contains(at)).map(|(n, _)| n.clone())
    {
        let glyphs = if app.state.selected_glyphs.contains(&name) {
            app.state.selected_glyphs.clone()
        } else {
            pending.push(("fontView.setSelection", json!({ "glyphs": [name] })));
            vec![name]
        };
        let mut menu = glyph_menu(app, glyphs, [at.x, at.y]);
        menu.pass = ui.ctx().cumulative_pass_nr();
        ui.ctx().data_mut(|d| d.insert_temp(id, menu));
        // The menu draws from the next frame on, so this click does not count as a click
        // outside it.
        return;
    }
    let Some(mut menu) = ui.ctx().data(|d| d.get_temp::<GlyphMenu>(id)) else { return };
    let pass = ui.ctx().cumulative_pass_nr();
    if app.dialog.is_some() || pass.saturating_sub(menu.pass) > 1 {
        ui.ctx().data_mut(|d| d.remove::<GlyphMenu>(id));
        return;
    }
    menu.pass = pass;
    ui.ctx().data_mut(|d| d.insert_temp(id, menu.clone()));
    let mut open = true;
    let mut action = None;
    let alt = ui.input(|i| i.modifiers.alt);
    let tokens = app.tokens.clone();
    let [x, y] = menu.screen;
    let anchor = Pos2::new(x, y);
    egui::Popup::new(id.with("popup"), ui.ctx().clone(), anchor, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .layout(Layout::top_down_justified(Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_bool(&mut open)
        .show(|ui| {
            if let Some(color) = swatches(ui, &tokens, GLYPH_MENU_SALT, alt) {
                action = Some(if alt { GlyphAction::LayerColor(color) } else { GlyphAction::Color(color) });
            }
            if !menu.tags.is_empty() {
                ui.menu_button("Tags", |ui| {
                    for (tag, mark) in &menu.tags {
                        let mut on = *mark == TagMark::All;
                        let check = egui::Checkbox::new(&mut on, tag.as_str()).indeterminate(*mark == TagMark::Some);
                        if ui.add(check).clicked() {
                            action = Some(GlyphAction::Tag(tag.clone()));
                        }
                    }
                });
            }
            if let Some(format) = copy_names_menu(ui) {
                action = Some(GlyphAction::CopyNames(format));
            }
        });
    if let Some(action) = action {
        run_glyph_action(app, &menu, &action, pending);
        open = false;
    }
    if !open {
        ui.ctx().data_mut(|d| d.remove::<GlyphMenu>(id));
    }
}
