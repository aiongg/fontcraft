//! The context menus: the canvas menu (`editView.contextMenu`) and Font View's glyph menu
//! (`fontView.glyphContextMenu`, [`glyph_menu`]).
//!
//! The canvas menu opens on a secondary click on the canvas (or the Menu key), which craft's `ToolHost` reports as a
//! context request; the canvas posts it as `contextMenu.open` (`shell::context_menu`), and the control channel opens
//! it with `ui.contextMenu.list {x, y}`.
//!
//! - The click is hit-tested as a tool click is. An object that is not selected becomes the selection first, so the
//!   items act on what the menu was opened on; a click on empty canvas keeps the selection.
//! - The menu has a section for the object, then a section for the glyph (`canvas`). The object section is
//!   `canvas.node`, `canvas.segment`, `canvas.anchor`, `canvas.component` or `canvas.guide` for the clicked object,
//!   and `canvas.path` when that object's path is wholly selected. On empty canvas it follows the selection.
//! - Items are the commands whose inventory row names the section in `context`, in the order of their rows
//!   ([`ORDER`]), greyed out as menu bar items are ([`item_state::supplies`]). The selection also hides items that do
//!   not apply (Unlock Path on an unlocked path) and checks checkable ones ([`item_state`]).
//! - Add Global Guide replaces Add Guide while Alt is held ([`ContextItem::alternate`]).
//! - While the layer's PostScript hints show, the hint items join the menu ([`HINT_ITEMS`]); a click on a hint's badge
//!   or marker selects that hint first. Their parameters come from `fontcraft_tools::hint_drag::menu_params`.
//! - Items that need a choice first ask for it through `dialog.open` ([`ASKS`]).
//! - An item runs with its targets from the selection, plus what the click adds: the clicked segment, node or guide,
//!   and the click position for the commands that place something ([`params_for`], [`steps_for`]).
//! - The glyph section starts with the twelve colour label swatches and a cross: they set the edited glyph's colour,
//!   or with Alt held the active layer's. It offers Exports, Locked and Show Glyph in Font View, and ends with the
//!   Copy Glyph Names submenu.

pub mod glyph_menu;
pub mod item_state;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::params::TargetKind;
use fontcraft_engine::{CommandSpec, command_specs, find_command};
use fontcraft_geom::Point;
use fontcraft_tools::{Hit, HitResult, Item, Scene, Selection, guide_drag, hint_drag, hit_test};
use serde_json::{Map, Value, json};

use self::glyph_menu::GlyphMenu;
use self::item_state::GUIDE_PROPERTIES;
use crate::app::{Closing, FontCraftApp, Msg};
use crate::canvas::tools::{self, apply};

pub const CANVAS: &str = "canvas";
pub const NODE: &str = "canvas.node";
pub const SEGMENT: &str = "canvas.segment";
pub const PATH: &str = "canvas.path";
pub const ANCHOR: &str = "canvas.anchor";
pub const COMPONENT: &str = "canvas.component";
pub const GUIDE: &str = "canvas.guide";
/// A PostScript hint's badge or marker, while the hints show.
pub const HINT: &str = "canvas.hint";

/// Largest coordinate, in font units or window points, a menu opens at.
pub const MAX_COORDINATE: f64 = 100_000.0;

/// The PostScript hint commands the menus show while the hints show, with the menus that show them (their rows name
/// no `context`: these menus exist only while hints are shown).
pub const HINT_ITEMS: &[(&str, &[&str])] = &[
    ("hint.ps.addHorizontalHint", &[NODE, PATH, CANVAS]),
    ("hint.ps.addVerticalHint", &[NODE, PATH, CANVAS]),
    ("hint.ps.makeGhostHint", &[HINT]),
    ("hint.ps.deleteHints", &[HINT]),
    ("hint.ps.autohintGlyph", &[CANVAS]),
    ("hint.ps.clearGlyphHints", &[CANVAS]),
];

/// The component section's Show Smart Glyph Settings: the settings of the clicked component's glyph.
const SMART_GLYPH_SETTINGS: &str = "smartGlyph.showSettings";

/// Shell commands that context menus show, with their rows' `context` (engine commands carry theirs in
/// `CommandSpec::context`). One the app does not have yet is left out.
const UI_CONTEXT: &[(&str, &[&str])] = &[
    ("path.focusOnSelectedPath", &[NODE, PATH]),
    ("path.clearFocusOnPaths", &[CANVAS]),
    ("glyph.showGlyphsUsingAsComponent", &[CANVAS]),
    (SMART_GLYPH_SETTINGS, &[COMPONENT]),
];

/// Engine commands the object sections show, whose registry entries list no context.
const OBJECT_ITEMS: &[(&str, &[&str])] = &[("cornerComponent.add", &[NODE]), ("cornerComponent.decompose", &[NODE])];

/// Commands whose registry entries list a context but that the menus leave out: the attachment anchor is chosen from
/// the anchor menu of the component Info box.
const HIDDEN: &[&str] = &["component.setAnchor"];

/// Items that open a picker or a dialog for their choice instead of running at once.
pub const ASKS: &[&str] = &["glyph.addComponent", "component.setBaseGlyph", "cornerComponent.add", "glyph.componentFromSelection"];

/// Engine commands the glyph section shows, whose registry entries list no context.
const GLYPH_ITEMS: &[&str] = &["glyph.setExport"];

/// The glyph section's Show Glyph in Font View: selects the edited glyph in Font View and shows Font View.
pub const SHOW_IN_FONT_VIEW: &str = "contextMenu.showGlyphInFontView";

/// Items drawn only while Alt is held, in place of the item before them.
const ALTERNATES: &[&str] = &["guide.addGlobal"];

/// The order of context menu items: the order of their inventory rows. A command missing here comes after these, in
/// registry order.
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
    "hint.ps.addHorizontalHint",
    "hint.ps.addVerticalHint",
    "hint.ps.makeGhostHint",
    "hint.ps.deleteHints",
    "hint.ps.autohintGlyph",
    "hint.ps.clearGlyphHints",
];

/// One entry of a context menu.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextItem {
    pub command: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    /// Whether a checkable item is checked; absent for other items.
    pub checked: Option<bool>,
    /// Drawn only while Alt is held, in place of the item before it.
    pub alternate: bool,
}

/// The items of one `context` value; sections are drawn with a separator between them.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub context: &'static str,
    pub items: Vec<ContextItem>,
}

/// An open canvas context menu.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasMenu {
    /// The glyph it was opened on; it closes when the edited glyph changes.
    pub glyph: String,
    /// Where it opens, in window points.
    pub screen: [f32; 2],
    /// The click in the edited glyph's font units.
    pub at: [f64; 2],
    /// The object under the click.
    pub hit: Option<HitResult>,
    pub sections: Vec<Section>,
    /// Alt was held at the click: the swatches colour the layer, and alternate items replace the items before them.
    pub alt: bool,
}

impl CanvasMenu {
    /// The menu as `ui.contextMenu.list` reports it.
    pub fn to_json(&self) -> Value {
        let sections: Vec<Value> = self
            .sections
            .iter()
            .map(|s| {
                let items: Vec<Value> = s
                    .items
                    .iter()
                    .map(|i| {
                        let mut m = Map::new();
                        m.insert("command".into(), json!(i.command));
                        m.insert("label".into(), json!(i.label));
                        m.insert("enabled".into(), json!(i.enabled));
                        if let Some(why) = &i.disabled_reason {
                            m.insert("disabled_reason".into(), json!(why));
                        }
                        if let Some(on) = i.checked {
                            m.insert("checked".into(), json!(on));
                        }
                        if i.alternate {
                            m.insert("alternate".into(), json!(true));
                        }
                        Value::Object(m)
                    })
                    .collect();
                json!({"context": s.context, "items": items})
            })
            .collect();
        let hit = self.hit.as_ref().and_then(|h| serde_json::to_value(h).ok()).unwrap_or(Value::Null);
        json!({"glyph": self.glyph, "screen": self.screen, "at": self.at, "hit": hit, "sections": sections, "alt": self.alt})
    }

    pub fn items(&self) -> impl Iterator<Item = &ContextItem> {
        self.sections.iter().flat_map(|s| s.items.iter())
    }

    pub fn item(&self, id: &str) -> Option<&ContextItem> {
        self.items().find(|i| i.command == id)
    }
}

/// The open context menus.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextMenus {
    pub canvas: Option<CanvasMenu>,
    pub glyph: Option<GlyphMenu>,
}

/// What a point given to [`open_canvas`] is measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// Window points, as a click is.
    Screen,
    /// The edited glyph's font units.
    Font,
}

impl Space {
    pub fn parse(name: &str) -> Option<Space> {
        match name {
            "screen" => Some(Space::Screen),
            "font" => Some(Space::Font),
            _ => None,
        }
    }
}

/// The edited glyph's scene and the selection, as the tools see them.
fn scene_of(app: &FontCraftApp) -> Option<(Scene<'_>, &Selection)> {
    let font = app.session.font().ok()?;
    let scene = tools::scene(font, &app.view, app.tools.focus.as_ref())?;
    Some((scene, &app.tools.selection))
}

/// The glyph the canvas edits.
pub fn scene_glyph(app: &FontCraftApp) -> Option<String> {
    scene_of(app).map(|(scene, _)| scene.glyph)
}

/// Opens the canvas context menu for a click at `at`: selects the clicked object when it is not selected, then lists
/// the commands for it. Does nothing while a tool is dragging or a dialog is open.
pub fn open_canvas(app: &mut FontCraftApp, at: Point, space: Space) -> Result<(), String> {
    if !(at.x.is_finite() && at.y.is_finite()) || at.x.abs() > MAX_COORDINATE || at.y.abs() > MAX_COORDINATE {
        return Err(format!("the point must be finite and within {MAX_COORDINATE} either way"));
    }
    apply::sync(app);
    if app.tools.toolbox.busy() || app.dialogs.open.is_some() {
        app.context_menus.canvas = None;
        return Ok(());
    }
    let camera = apply::camera(app);
    let view = apply::tools_view(&camera);
    let (pos, screen) = match space {
        Space::Screen => {
            let p = camera.to_font(egui::pos2(at.x as f32, at.y as f32));
            (Point::new(p.x, p.y), [at.x as f32, at.y as f32])
        }
        Space::Font => {
            let s = camera.to_screen(craft::behavior::kurbo::Point::new(at.x, at.y));
            (at, [s.x, s.y])
        }
    };
    let tolerance = f64::from(crate::canvas::tools::tokens::ToolTokens::standard().hit_tolerance);
    let (hint, hit) = {
        let (scene, _) = scene_of(app).ok_or("no glyph is being edited")?;
        let hint = scene.ps_hints.hit(&view, pos, tolerance).map(|h| h.index);
        let hit = if hint.is_some() { None } else { hit_test(&scene, &view, pos, tolerance) };
        (hint, hit)
    };
    if let Some(index) = hint {
        if !app.tools.selection.hints.contains(&index) {
            app.tools.selection = Selection::from_items([Item::Hint(index)]);
        }
    } else if let Some(h) = &hit
        && !h.selected_in(&app.tools.selection)
    {
        app.tools.selection = Selection::from_items(h.items());
    }
    app.tools.refresh();
    let (glyph, kinds) = {
        let (scene, selection) = scene_of(app).ok_or("no glyph is being edited")?;
        let kinds = if hint.is_some() { vec![HINT, CANVAS] } else { kinds(&scene, selection, hit.as_ref()) };
        (scene.glyph.clone(), kinds)
    };
    let sections = sections(app, &kinds);
    app.context_menus.canvas = Some(CanvasMenu { glyph, screen, at: [pos.x, pos.y], hit, sections, alt: false });
    Ok(())
}

/// The context menus a click shows, object first, `canvas` last.
pub fn kinds(scene: &Scene, selection: &Selection, hit: Option<&HitResult>) -> Vec<&'static str> {
    let whole = tools::targets::selected_paths(scene, selection);
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
        None if !selection.hints.is_empty() => Some(HINT),
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

/// Every command a menu may show, in menu order.
fn candidates(hints_shown: bool) -> Vec<Candidate> {
    let mut all: Vec<Candidate> = command_specs()
        .iter()
        .filter(|c| !c.context.is_empty())
        .map(|c| Candidate { id: c.id, label: c.label, context: c.context, spec: Some(c) })
        .collect();
    for (id, context) in UI_CONTEXT {
        if let Some((_, c)) = crate::shell::commands::find(id) {
            all.push(Candidate { id: c.id, label: c.label, context, spec: None });
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
    if hints_shown {
        for (id, context) in HINT_ITEMS {
            if let Some(spec) = find_command(id) {
                all.push(Candidate { id: spec.id, label: spec.label, context, spec: Some(spec) });
            }
        }
    }
    let rank = |id: &str| ORDER.iter().position(|o| *o == id).unwrap_or(ORDER.len());
    all.sort_by_key(|c| rank(c.id));
    all
}

/// The sections of a context menu showing `kinds`, in that order. A command listed for several of them appears only
/// in the first.
pub fn sections(app: &FontCraftApp, kinds: &[&'static str]) -> Vec<Section> {
    let scene = scene_of(app);
    let hints_shown = scene.as_ref().is_some_and(|(s, _)| s.ps_hints.shown);
    let all = candidates(hints_shown);
    // On a guide the menu offers the guide's commands; adding a guide is for clicks off guides.
    let on_guide = kinds.contains(&GUIDE);
    let visible = |id: &str| match &scene {
        _ if on_guide && matches!(id, "guide.add" | "guide.addGlobal") => false,
        _ if HIDDEN.contains(&id) => false,
        Some((scene, selection)) => item_state::visible(scene, selection, id),
        None => id != GUIDE_PROPERTIES,
    };
    let mut shown: Vec<&'static str> = Vec::new();
    let mut out = Vec::new();
    for &kind in kinds {
        let items: Vec<ContextItem> = all
            .iter()
            .filter(|c| c.context.contains(&kind) && !shown.contains(&c.id) && visible(c.id))
            .map(|c| item(app, scene.as_ref(), c))
            .collect();
        shown.extend(items.iter().map(|i| i.command));
        if !items.is_empty() {
            out.push(Section { context: kind, items });
        }
    }
    out
}

fn item(app: &FontCraftApp, at: Option<&(Scene<'_>, &Selection)>, c: &Candidate) -> ContextItem {
    let selection = at.map(|(_, s)| *s);
    let check = match c.spec {
        Some(spec) => (spec.enabled)(&app.session)
            .and_then(|()| at.map_or(Ok(()), |(scene, selection)| item_state::supplies(scene, selection, spec)))
            .and_then(|()| hint_ready(selection, c.id)),
        None => ui_enabled(app, c.id),
    };
    ContextItem {
        command: c.id,
        label: c.label,
        enabled: check.is_ok(),
        disabled_reason: check.err(),
        checked: match c.id {
            "glyph.setExport" => at.and_then(|(scene, _)| edited_exports(app, &scene.glyph)),
            _ => at.and_then(|(scene, selection)| item_state::checked(scene, selection, c.id)),
        },
        alternate: ALTERNATES.contains(&c.id),
    }
}

/// The hint items that act on selected hints need one.
fn hint_ready(selection: Option<&Selection>, id: &str) -> Result<(), String> {
    match id {
        "hint.ps.makeGhostHint" | "hint.ps.deleteHints" if selection.is_none_or(|s| s.hints.is_empty()) => Err("select a hint first".into()),
        _ => Ok(()),
    }
}

/// Whether glyph `glyph` exports.
fn edited_exports(app: &FontCraftApp, glyph: &str) -> Option<bool> {
    app.session.font().ok()?.glyph(glyph).map(|g| g.export)
}

/// Whether a shell command of [`UI_CONTEXT`], or Guide Properties…, can run now.
fn ui_enabled(app: &FontCraftApp, id: &str) -> Result<(), String> {
    let selection = &app.tools.selection;
    match id {
        "path.focusOnSelectedPath" if selection.points.is_empty() => Err("select a point of the paths to focus on first".into()),
        "path.clearFocusOnPaths" if app.tools.focus.is_none() => Err("no path has the focus".into()),
        GUIDE_PROPERTIES if selection.guides.len() != 1 => Err("select one guide first".into()),
        _ => Ok(()),
    }
}

/// The parameters item `id` of `menu` runs with: the targets from the selection (for a hint item, from
/// `hint_drag::menu_params`), plus what the click adds.
pub fn params_for(app: &mut FontCraftApp, menu: &CanvasMenu, id: &str) -> Value {
    if id == GUIDE_PROPERTIES {
        return guide_properties(app).unwrap_or(Value::Null);
    }
    if let Some(params) = scene_of(app).and_then(|(scene, selection)| hint_drag::menu_params(&scene, selection, id)) {
        return params;
    }
    let base = tools::targets::with_targets(app, CommandRef::new(id)).params;
    let Some(spec) = find_command(id) else { return base };
    let mut m = match base {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => return other,
    };
    let takes = |k: TargetKind| spec.targets.contains(&k);
    let hit = menu.hit.as_ref().map(|h| &h.hit);
    let click = Point::new(menu.at[0], menu.at[1]);
    let hit_at = menu.hit.as_ref().map_or(click, |h| h.at);
    if takes(TargetKind::Segments)
        && !takes(TargetKind::Points)
        && let Some(Hit::Segment { shape, node, .. }) = hit
    {
        m.insert("segments".into(), json!([[shape, node]]));
    }
    // A guide goes through the clicked node, else through the click.
    let place = if matches!(hit, Some(Hit::Node { .. })) { hit_at } else { click };
    let scene_says = |f: &dyn Fn(&Scene, &Selection) -> Value| scene_of(app).map(|(scene, selection)| f(&scene, selection));
    match id {
        "path.makeNodeFirst" => {
            if let Some(Hit::Node { shape, node }) = hit {
                m.remove("points");
                m.insert("point".into(), json!([shape, node]));
            }
        }
        "path.insertNode" => {
            let near = if matches!(hit, Some(Hit::Segment { .. })) { hit_at } else { click };
            m.insert("near".into(), json!([near.x, near.y]));
        }
        "guide.add" => {
            let through = scene_says(&|scene, selection| match selection.on_curve_points(scene).as_slice() {
                [a, b] => json!([[a.0, a.1], [b.0, b.1]]),
                _ => Value::Null,
            });
            match through.filter(|t| !t.is_null()) {
                Some(points) => {
                    m.insert("throughPoints".into(), points);
                }
                None => {
                    m.insert("x".into(), json!(place.x));
                    m.insert("y".into(), json!(place.y));
                }
            }
        }
        "guide.addGlobal" => {
            m.insert("x".into(), json!(place.x));
            m.insert("y".into(), json!(place.y));
        }
        "guide.toggleMeasurement" => {
            let guide = match hit {
                Some(Hit::Guide { guide }) => Some(*guide),
                _ => app.tools.selection.guides.iter().next().copied(),
            };
            if let Some(g) = guide {
                m.insert("guide".into(), json!({"scope": g.scope.as_str(), "index": g.index}));
            }
        }
        "cornerComponent.decompose" => {
            if let Some(corners) = scene_says(&|scene, selection| json!(item_state::selected_corners(scene, selection))) {
                m.insert("corners".into(), corners);
            }
        }
        "glyph.setExport" => {
            let exports = edited_exports(app, &menu.glyph).unwrap_or(true);
            m.insert("glyphs".into(), json!([menu.glyph]));
            m.insert("export".into(), json!(!exports));
        }
        "path.setMask" => {
            let shapes: Vec<usize> = m
                .get("shapes")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_u64).filter_map(|v| usize::try_from(v).ok()).collect())
                .unwrap_or_default();
            let masked = scene_says(&|scene, _| {
                json!(!shapes.is_empty() && shapes.iter().all(|&s| scene.path(s).is_some_and(|p| p.info.attrs.mask.is_some())))
            });
            m.insert("mask".into(), json!(!masked.and_then(|v| v.as_bool()).unwrap_or(false)));
        }
        _ => {}
    }
    Value::Object(m)
}

/// The `dialog.open` parameters of Guide Properties… for the one selected guide.
fn guide_properties(app: &FontCraftApp) -> Option<Value> {
    let (scene, selection) = scene_of(app)?;
    let r = *selection.guides.iter().next()?;
    let g = guide_drag::guide(&scene, r)?;
    Some(json!({"dialog": GUIDE_PROPERTIES, "params": guide_drag::properties(&scene, r, g)}))
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// The messages item `id` of `menu` posts, in order: the item's command (or the dialog that asks for its choice),
/// then for a new guide the selection of that guide.
pub fn steps_for(app: &mut FontCraftApp, menu: &CanvasMenu, id: &str) -> Vec<Msg> {
    if id == GUIDE_PROPERTIES {
        let params = params_for(app, menu, id);
        return vec![shell("dialog.open", params)];
    }
    if id == SHOW_IN_FONT_VIEW {
        return vec![shell("fontView.setSelection", json!({"glyphs": [menu.glyph]})), shell("fontView.show", json!({}))];
    }
    if id == SMART_GLYPH_SETTINGS {
        let base = match menu.hit.as_ref().map(|h| &h.hit) {
            Some(Hit::Component { shape }) => scene_of(app).and_then(|(scene, _)| match scene.layer.shapes.get(*shape) {
                Some(fontcraft_engine::model::Shape::Component(c)) => Some(c.glyph.clone()),
                _ => None,
            }),
            _ => None,
        };
        let params = base.map_or_else(|| json!({}), |glyph| json!({ "glyph": glyph }));
        return vec![FontCraftApp::route(CommandRef::with_params(id, params))];
    }
    let params = params_for(app, menu, id);
    if ASKS.contains(&id) {
        return vec![shell("dialog.open", json!({"command": id, "params": params}))];
    }
    let new_guide = match id {
        "guide.add" => scene_of(app).map(|(scene, _)| json!({"scope": "local", "index": scene.local_guides().len()})),
        "guide.addGlobal" => scene_of(app).map(|(scene, _)| json!({"scope": "global", "index": scene.global_guides.len()})),
        _ => None,
    };
    let mut out = vec![FontCraftApp::route(CommandRef::with_params(id, params))];
    if let Some(guide) = new_guide {
        out.push(shell("selection.set", json!({ "guides": [guide] })));
    }
    out
}

/// Runs item `id` of the open canvas menu as a click on it does, and closes the menu; `run` applies each message and
/// gives its result. The item is checked as the menu would draw it now: a menu whose glyph changed, or that a dialog
/// covers, closes; an item that is gone or disabled is refused and leaves the menu open. Returns the first step's
/// result: the item's command, or the dialog that asks for it.
pub fn invoke(app: &mut FontCraftApp, id: &str, run: fn(&mut FontCraftApp, Msg) -> Result<Value, String>) -> Result<Value, String> {
    let menu = app.context_menus.canvas.clone().ok_or("no context menu is open")?;
    let same = scene_of(app).is_some_and(|(scene, _)| scene.glyph == menu.glyph);
    if !same || app.dialogs.open.is_some() {
        app.context_menus.canvas = None;
        return Err(if same { "a dialog is open; the context menu closed" } else { "the edited glyph changed; the context menu closed" }.into());
    }
    if matches!(app.closing, Closing::Ask | Closing::Asking) {
        return Err("the question about unsaved changes is open".into());
    }
    let kinds: Vec<&'static str> = menu.sections.iter().map(|s| s.context).collect();
    let item = sections(app, &kinds).into_iter().flat_map(|s| s.items).find(|i| i.command == id);
    let item = item.ok_or_else(|| format!("`{id}` is not in the context menu"))?;
    if !item.enabled {
        return Err(item.disabled_reason.unwrap_or_else(|| format!("`{id}` is not available now")));
    }
    app.context_menus.canvas = None;
    let mut result = None;
    for msg in steps_for(app, &menu, item.command) {
        let value = run(app, msg)?;
        result.get_or_insert(value);
    }
    Ok(result.unwrap_or(Value::Null))
}

/// The canvas menu's colour swatches: the edited glyph's colour label, or with Alt the active layer's.
pub fn color_steps(app: &FontCraftApp, menu: &CanvasMenu, color: Option<u8>, layer: bool) -> Vec<Msg> {
    let target = tools::target(&app.session, &app.view).map(|t| t.layer);
    let glyph_menu = GlyphMenu { glyphs: vec![menu.glyph.clone()], layer: target, ..GlyphMenu::default() };
    let action = if layer { glyph_menu::GlyphAction::LayerColor(color) } else { glyph_menu::GlyphAction::Color(color) };
    glyph_menu::steps(&glyph_menu, &action)
}
