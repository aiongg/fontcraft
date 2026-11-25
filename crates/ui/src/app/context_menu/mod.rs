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
//! - A click on the edited side's shown image, where no other object lies, opens the image section (`canvas.image`)
//!   and the glyph section: Unlock Image on a locked image, Set Crop to Layer Bounds and Reset Crop on an unlocked one,
//!   which the click selects (`canvas::images`).
//! - Add Global Guide replaces Add Guide, Expand Outline for All Masters replaces Expand Outline, and a node command's
//!   form "in All Masters" replaces it, while Alt is held ([`ContextItem::alternate`]).
//! - While the layer's PostScript hints show, the hint items join the menu ([`HINT_ITEMS`]); a click on a hint's badge
//!   or marker selects that hint first. Their parameters come from `fontcraft_tools::hint_drag::menu_params`.
//! - While the TrueType Instructor is the active tool, the node and path sections start with the instructions added on
//!   the selected points: Snap Point, Stem, Shift Points, Interpolate and Delta (their rows name those menus in
//!   `context`, and their `when` the Instructor; a command whose `when` names another tool is left out). Each is greyed
//!   out with the refusal its command would give for the selection, the Instructor's direction and the touched state
//!   ([`point_ready`]).
//! - While the TrueType Instructor is the active tool, a click on an instruction's badge of the current direction
//!   selects that instruction first (unless it is selected) and opens the instruction section ([`TT_INSTRUCTION`]):
//!   Reverse and Set Stem Reference on Stem instructions, Make Triple Hint on three vertical Stem instructions, and
//!   Delete Instructions, which then clears the instruction selection ([`TT_ITEMS`]). They act on the selected
//!   instructions. Set Stem Reference is a submenu of its choices (Auto, No Stem, the TT stems of the direction), the
//!   one the instructions share checked ([`ContextItem::choices`]); a pick runs the command with that `stem`.
//! - While the TrueType Instructor is the active tool, the glyph section offers Autohint and Remove Hints on the
//!   edited glyph's layer.
//! - While the TrueType Instructor is the active tool, a last section ([`TT_INSTRUCTOR`]) offers its view items: Show
//!   Pixel Preview, the Hinting Preview's three scales ([`PREVIEW_SCALES`]), Show Preview Address… and Show Point
//!   Indexes, checked as their state is. Other tools' menus leave them out.
//! - On a knob of the Higher-Order Interpolation tool, the first section ([`HOI`]) is the Interpolation submenu: the
//!   four types, the one the picked knobs share checked.
//! - Items that need a choice first ask for it through `dialog.open` ([`ASKS`]); Add Corner Component, Add Cap
//!   Component, Add Segment Component and Add Brush open the glyph picker of their kind ([`PICKS`]).
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
use fontcraft_engine::hinting::settings::main_master;
use fontcraft_engine::hinting::{Direction, NodeRef, StemRef, TouchedState, TtInstruction, TtKind, TtPoint, ZoneRef, tt_instructions};
use fontcraft_engine::model::LayerSide;
use fontcraft_engine::params::TargetKind;
use fontcraft_engine::{CommandSpec, command_specs, find_command};
use fontcraft_geom::Point;
use fontcraft_tools::hoi::Kind;
use fontcraft_tools::tt_instructor;
use fontcraft_tools::{Hit, HitResult, Item, Scene, Selection, guide_drag, hint_drag, hit_test_with_corners};
use serde_json::{Map, Value, json};

use self::glyph_menu::GlyphMenu;
use self::item_state::GUIDE_PROPERTIES;
use crate::app::models::hint_canvas::{BadgeKind, Choice, TtMark};
use crate::app::{FontCraftApp, Msg, targets};
use crate::canvas::hinting::SelectedInstructions;
use crate::canvas::tools::{self, apply};

pub const CANVAS: &str = "canvas";
pub const NODE: &str = "canvas.node";
pub const SEGMENT: &str = "canvas.segment";
pub const PATH: &str = "canvas.path";
pub const ANCHOR: &str = "canvas.anchor";
pub const COMPONENT: &str = "canvas.component";
pub const GUIDE: &str = "canvas.guide";
/// The edited side's placed image.
pub const IMAGE: &str = "canvas.image";
/// The image section's items: Unlock Image shows on a locked image, the others on an unlocked one.
const IMAGE_ITEMS: &[&str] = &["image.unlock", "image.setCropToLayerBounds", "image.resetCrop"];
/// A PostScript hint's badge or marker, while the hints show.
pub const HINT: &str = "canvas.hint";
/// A TrueType instruction's badge, while the TrueType Instructor is the active tool.
pub const TT_INSTRUCTION: &str = "canvas.ttInstruction";
/// The TrueType Instructor's view items, the last section while that tool is active.
pub const TT_INSTRUCTOR: &str = "canvas.ttInstructor";
/// The knobs of the Higher-Order Interpolation tool, and the command of their submenu.
pub const HOI: &str = "canvas.hoi";
pub const INTERPOLATION: &str = "hoi.setInterpolationType";

/// The Hinting Preview scale items: (item id, label, `hint.tt.setPreviewScale` scale).
pub const PREVIEW_SCALES: [(&str, &str, u8); 3] = [
    ("contextMenu.hintingPreviewScale1x", "Hinting Preview 1×", 1),
    ("contextMenu.hintingPreviewScale2x", "Hinting Preview 2×", 2),
    ("contextMenu.hintingPreviewScale4x", "Hinting Preview 4×", 4),
];

/// The Instructor section's shell commands, before and after the scale items.
const TT_VIEW_BEFORE: &[&str] = &["hint.tt.showPixelPreview"];
const TT_VIEW_AFTER: &[&str] = &[crate::dialogs::tt::preview_address::ID, "hint.tt.showPointIndexes"];

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

/// The TrueType commands the menus show while the Instructor is active, with their sections (their rows name no
/// `context`: the instruction section exists only on an instruction's badge, and the glyph section offers the two
/// glyph items only in the Instructor).
pub const TT_ITEMS: &[(&str, &[&str])] = &[
    ("hint.tt.reverse", &[TT_INSTRUCTION]),
    (SET_STEM_REFERENCE, &[TT_INSTRUCTION]),
    ("hint.tt.makeTripleHint", &[TT_INSTRUCTION]),
    ("hint.tt.deleteInstructions", &[TT_INSTRUCTION]),
    ("hint.tt.autohintGlyph", &[CANVAS]),
    ("hint.tt.removeHints", &[CANVAS]),
];

/// The instruction section's items, which act on the selected instructions.
const INSTRUCTION_ITEMS: &[&str] = &["hint.tt.reverse", SET_STEM_REFERENCE, "hint.tt.makeTripleHint", "hint.tt.deleteInstructions"];

/// The instruction section's submenu of stem choices.
pub const SET_STEM_REFERENCE: &str = "hint.tt.setStemReference";

/// The instructions the node and path sections add on the selected points while the Instructor is active.
pub const POINT_ITEMS: &[&str] = &["hint.tt.snapPoint", "hint.tt.addStemHint", "hint.tt.shiftPoints", "hint.tt.interpolate", "hint.tt.addDelta"];

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
/// The attachment items show only for a selection they can act on ([`item_state::visible`]).
const OBJECT_ITEMS: &[(&str, &[&str])] = &[
    ("cornerComponent.add", &[NODE]),
    ("cornerComponent.decompose", &[NODE]),
    ("capComponent.add", &[NODE]),
    ("capComponent.decompose", &[NODE]),
    ("segmentComponent.add", &[NODE, SEGMENT, PATH]),
    ("segmentComponent.decompose", &[NODE, SEGMENT]),
    ("brush.add", &[NODE, SEGMENT, PATH]),
    ("brush.decompose", &[NODE, SEGMENT]),
];

/// Commands whose registry entries list a context but that the menus leave out: the attachment anchor is chosen from
/// the anchor menu of the component Info box.
const HIDDEN: &[&str] = &["component.setAnchor"];

/// Items that open a dialog for their choice instead of running at once.
pub const ASKS: &[&str] = &["glyph.componentFromSelection"];

/// Items that open the glyph picker of their kind on the targets the selection gives (`dialogs::glyph_picker`), and
/// the picker's purpose.
const PICKS: &[(&str, &str)] = &[
    ("glyph.addComponent", "addComponent"),
    ("component.setBaseGlyph", "baseGlyph"),
    ("cornerComponent.add", "addCorner"),
    ("capComponent.add", "addCap"),
    ("segmentComponent.add", "addSegment"),
    ("brush.add", "addBrush"),
];

/// Engine commands the glyph section shows, whose registry entries list no context.
const GLYPH_ITEMS: &[&str] = &["glyph.setExport"];

/// The glyph section's Show Glyph in Font View: selects the edited glyph in Font View and shows Font View.
pub const SHOW_IN_FONT_VIEW: &str = "contextMenu.showGlyphInFontView";

/// Items drawn only while Alt is held, in place of the item before them.
const ALTERNATES: &[&str] = &[
    "guide.addGlobal",
    "path.expandOutlineForAllMasters",
    "path.toggleNodeTypeInAllMasters",
    "path.convertSegmentToCurveInAllMasters",
    "path.connectNodesInAllMasters",
    "path.closeOpenPathsInAllMasters",
    "path.openCornerInAllMasters",
    "path.reconnectNodesInAllMasters",
    "path.makeNodeFirstInAllMasters",
    "path.insertNodeInAllMasters",
];

/// Items whose command removes something from the glyph: their rows show the danger tone.
pub const DESTRUCTIVE: &[&str] =
    &["guide.remove", "hint.ps.deleteHints", "hint.ps.clearGlyphHints", "hint.tt.deleteInstructions", "hint.tt.removeHints"];

/// The order of context menu items: the order of their inventory rows. A command missing here comes after these, in
/// registry order.
const ORDER: &[&str] = &[
    "hint.tt.snapPoint",
    "hint.tt.addStemHint",
    "hint.tt.shiftPoints",
    "hint.tt.interpolate",
    "hint.tt.addDelta",
    "path.toggleNodeType",
    "path.toggleNodeTypeInAllMasters",
    "path.convertSegmentToCurve",
    "path.convertSegmentToCurveInAllMasters",
    "path.connectNodes",
    "path.connectNodesInAllMasters",
    "path.closeOpenPaths",
    "path.closeOpenPathsInAllMasters",
    "path.openCorner",
    "path.openCornerInAllMasters",
    "path.reconnectNodes",
    "path.reconnectNodesInAllMasters",
    "path.sharpenCorner",
    "path.makeNodeFirst",
    "path.makeNodeFirstInAllMasters",
    "path.reverseContours",
    "path.insertNode",
    "path.insertNodeInAllMasters",
    "path.focusOnSelectedPath",
    "path.clearFocusOnPaths",
    "path.lockPath",
    "path.unlockPath",
    "glyph.setExport",
    "glyph.toggleLocked",
    SHOW_IN_FONT_VIEW,
    "path.expandOutline",
    "path.expandOutlineForAllMasters",
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
    "hint.tt.reverse",
    SET_STEM_REFERENCE,
    "hint.tt.makeTripleHint",
    "hint.tt.deleteInstructions",
    "hint.tt.autohintGlyph",
    "hint.tt.removeHints",
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
    /// The values of a submenu item, in order: picking one runs the command with it ([`invoke_choice`]). Empty for a
    /// plain item.
    pub choices: Vec<ItemChoice>,
}

/// One value of a submenu item.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemChoice {
    /// The value the command gets.
    pub value: String,
    pub label: String,
    /// The value the targets share now.
    pub checked: bool,
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
    /// The layer and the side it was opened on; it closes when the canvas edits another one (a master switch).
    pub target: Option<tools::Target>,
    /// Where it opens, in window points.
    pub screen: [f32; 2],
    /// The click in the edited glyph's font units.
    pub at: [f64; 2],
    /// The object under the click.
    pub hit: Option<HitResult>,
    pub sections: Vec<Section>,
    /// Alt was held at the click: the swatches colour the layer, and alternate items replace the items before them.
    pub alt: bool,
    /// In text mode with glyphs of the text selected: those glyphs, which the items act on. Empty otherwise.
    pub glyphs: Vec<String>,
}

impl CanvasMenu {
    /// The glyphs its glyph items act on.
    pub fn glyphs(&self) -> Vec<String> {
        if self.glyphs.is_empty() { vec![self.glyph.clone()] } else { self.glyphs.clone() }
    }

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
                        if !i.choices.is_empty() {
                            let choices: Vec<Value> =
                                i.choices.iter().map(|c| json!({"value": c.value, "label": c.label, "checked": c.checked})).collect();
                            m.insert("choices".into(), Value::Array(choices));
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

/// The edited glyph's scene and the selection, as the tools see them; the PostScript hints come from the hint canvas
/// model and the interpolation paths from the canvas model.
pub(crate) fn scene_of(app: &FontCraftApp) -> Option<(Scene<'_>, &Selection)> {
    let font = app.session.font().ok()?;
    let mut scene = tools::scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref())?;
    scene.ps_hints = app.models.hint_canvas.ps.clone();
    scene.hoi = app.models.canvas.hoi.clone();
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
    if crate::canvas::text::text_mode(&app.view) {
        return open_on_text(app, screen, pos);
    }
    if apply::pick_knob(app, pos) {
        app.tools.refresh();
        let glyph = scene_glyph(app).ok_or("no glyph is being edited")?;
        let sections = sections(app, &[HOI, CANVAS]);
        let target = tools::target(&app.session, &app.view);
        app.context_menus.canvas =
            Some(CanvasMenu { glyph, screen, at: [pos.x, pos.y], hit: None, sections, alt: false, target, glyphs: Vec::new() });
        return Ok(());
    }
    if let Some(index) = crate::canvas::hinting::badges::badge_at(app, egui::pos2(screen[0], screen[1])) {
        return open_on_instruction(app, index, screen, pos);
    }
    let tolerance = f64::from(crate::canvas::tools::tokens::ToolTokens::standard().hit_tolerance);
    let (hint, hit) = {
        let (scene, _) = scene_of(app).ok_or("no glyph is being edited")?;
        let hint = scene.ps_hints.hit(&view, pos, tolerance).map(|h| h.index);
        let hit = if hint.is_some() { None } else { hit_test_with_corners(&scene, &view, pos, tolerance) };
        (hint, hit)
    };
    // The image lies under every other object.
    let on_image = hint.is_none() && hit.is_none() && image_hit(app, pos);
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
        let kinds = if hint.is_some() {
            vec![HINT, CANVAS]
        } else if on_image {
            vec![IMAGE, CANVAS]
        } else {
            kinds(&scene, selection, hit.as_ref())
        };
        (scene.glyph.clone(), kinds)
    };
    let sections = sections(app, &with_instructor(app, kinds));
    let target = tools::target(&app.session, &app.view);
    app.context_menus.canvas = Some(CanvasMenu { glyph, screen, at: [pos.x, pos.y], hit, sections, alt: false, target, glyphs: Vec::new() });
    Ok(())
}

/// Opens the canvas menu in text mode: its items act on the glyphs of the text selection, else on the edited glyph.
fn open_on_text(app: &mut FontCraftApp, screen: [f32; 2], pos: Point) -> Result<(), String> {
    let glyph = scene_glyph(app).ok_or("no glyph is being edited")?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let selected = targets::text_selection(font, &app.view).into_iter().flat_map(|group| group.glyphs);
    let glyphs: Vec<String> = selected.filter(|name| seen.insert(name.clone())).collect();
    let sections = sections(app, &[CANVAS]);
    let target = tools::target(&app.session, &app.view);
    app.context_menus.canvas = Some(CanvasMenu { glyph, screen, at: [pos.x, pos.y], hit: None, sections, alt: false, target, glyphs });
    Ok(())
}

/// Whether item `id` acts on whole glyphs: the items text mode shows.
fn glyph_item(id: &str) -> bool {
    id == SHOW_IN_FONT_VIEW || find_command(id).is_some_and(|spec| spec.targets.contains(&TargetKind::Glyphs))
}

/// Whether a click at `pos` (font units) lies on the edited side's shown image, locked or not. An unlocked image
/// becomes the selection.
fn image_hit(app: &mut FontCraftApp, pos: Point) -> bool {
    let Some((_, geom, on)) = crate::canvas::images::current(app) else { return false };
    if !on || !geom.covers(pos) {
        return false;
    }
    if !geom.locked {
        crate::canvas::images::select(app);
    }
    true
}

/// Whether image item `id` shows: Unlock Image on a locked image, the crop items on an unlocked one.
fn image_item_visible(app: &FontCraftApp, id: &str) -> bool {
    match (id, crate::canvas::images::current(app).map(|(_, geom, _)| geom.locked)) {
        (_, None) => false,
        ("image.unlock", Some(locked)) => locked,
        (_, Some(locked)) => !locked,
    }
}

/// Opens the canvas menu on instruction `index`'s badge: the instruction becomes the selection unless it is selected.
fn open_on_instruction(app: &mut FontCraftApp, index: usize, screen: [f32; 2], pos: Point) -> Result<(), String> {
    let tt = app.models.hint_canvas.tt.as_ref().ok_or("the Instructor shows no instructions")?;
    let (glyph, layer) = (tt.glyph.clone(), tt.layer.clone());
    let state = &mut app.view.hinting.instructor;
    if !state.selected_in(&glyph, &layer).contains(&index) {
        state.selected = SelectedInstructions { glyph: glyph.clone(), layer, instructions: [index].into_iter().collect() };
        app.view_changed();
    }
    let sections = sections(app, &with_instructor(app, vec![TT_INSTRUCTION, CANVAS]));
    let target = tools::target(&app.session, &app.view);
    app.context_menus.canvas = Some(CanvasMenu { glyph, screen, at: [pos.x, pos.y], hit: None, sections, alt: false, target, glyphs: Vec::new() });
    Ok(())
}

/// Whether the TrueType Instructor is the active tool.
fn instructor_active(app: &FontCraftApp) -> bool {
    app.view.tool == crate::canvas::hinting::INSTRUCTOR
}

/// `kinds` with the Instructor's section last while that tool is active.
fn with_instructor(app: &FontCraftApp, mut kinds: Vec<&'static str>) -> Vec<&'static str> {
    if instructor_active(app) && !kinds.contains(&TT_INSTRUCTOR) {
        kinds.push(TT_INSTRUCTOR);
    }
    kinds
}

/// The Interpolation submenu of the knobs picked with the Higher-Order Interpolation tool; `None` without one.
fn hoi_section(app: &FontCraftApp) -> Option<Section> {
    let spans = app.models.canvas.hoi.as_ref()?.by_key();
    let mut kinds = app.tools.toolbox.hoi_picks().iter().filter_map(|k| spans.get(&k.key())).map(|s| s.kind);
    let first = kinds.next()?;
    let shared = kinds.all(|k| k == first).then_some(first);
    let label = |kind: Kind| match kind {
        Kind::Linear => "Linear",
        Kind::IntermediatePoint => "Intermediate Point",
        Kind::Handles => "Handles",
        Kind::Rotation => "Rotation",
    };
    let choices = Kind::ALL
        .into_iter()
        .map(|kind| ItemChoice { value: kind.as_str().to_string(), label: label(kind).to_string(), checked: shared == Some(kind) })
        .collect();
    let check = (find_command(INTERPOLATION)?.enabled)(&app.session);
    let item = ContextItem {
        command: INTERPOLATION,
        label: "Interpolation",
        enabled: check.is_ok(),
        disabled_reason: check.err(),
        checked: None,
        alternate: false,
        choices,
    };
    Some(Section { context: HOI, items: vec![item] })
}

/// The TrueType Instructor's view items, checked as their state is.
fn instructor_section(app: &FontCraftApp) -> Section {
    let state = &app.view.hinting.instructor;
    let scale = app.models.hinting_waterfall.scale();
    let command = |id: &&'static str| {
        let label = crate::shell::commands::find(id).map_or(*id, |(_, c)| c.label);
        let checked = match *id {
            "hint.tt.showPixelPreview" => Some(state.pixel_preview),
            "hint.tt.showPointIndexes" => Some(state.point_indexes),
            _ => None,
        };
        ContextItem { command: id, label, enabled: true, disabled_reason: None, checked, alternate: false, choices: Vec::new() }
    };
    let scales = PREVIEW_SCALES.iter().map(|&(id, label, s)| ContextItem {
        command: id,
        label,
        enabled: true,
        disabled_reason: None,
        checked: Some(s == scale),
        alternate: false,
        choices: Vec::new(),
    });
    let items = TT_VIEW_BEFORE.iter().map(command).chain(scales).chain(TT_VIEW_AFTER.iter().map(command)).collect();
    Section { context: TT_INSTRUCTOR, items }
}

/// The messages an item of the Instructor section posts: a toggle sets the state its check mark does not show.
fn instructor_steps(app: &FontCraftApp, id: &str) -> Option<Vec<Msg>> {
    if let Some(&(_, _, scale)) = PREVIEW_SCALES.iter().find(|(item, _, _)| *item == id) {
        return Some(vec![shell(crate::app::models::hinting::SCALE_COMMAND, json!({ "scale": scale }))]);
    }
    let state = &app.view.hinting.instructor;
    match id {
        "hint.tt.showPixelPreview" => Some(vec![shell(id, json!({ "on": !state.pixel_preview }))]),
        "hint.tt.showPointIndexes" => Some(vec![shell(id, json!({ "on": !state.point_indexes }))]),
        _ if TT_VIEW_AFTER.contains(&id) => Some(vec![shell(id, json!({}))]),
        _ => None,
    }
}

/// The context menus a click shows, object first, `canvas` last.
pub fn kinds(scene: &Scene, selection: &Selection, hit: Option<&HitResult>) -> Vec<&'static str> {
    let whole = tools::targets::selected_paths(scene, selection);
    let object = match hit.map(|h| &h.hit) {
        Some(Hit::Node { shape, .. } | Hit::Handle { shape, .. } | Hit::Implied { shape, .. } | Hit::Segment { shape, .. })
            if whole.contains(shape) =>
        {
            Some(PATH)
        }
        Some(Hit::Node { .. } | Hit::Handle { .. } | Hit::Implied { .. }) => Some(NODE),
        Some(Hit::Segment { .. }) => Some(SEGMENT),
        Some(Hit::Anchor { .. }) => Some(ANCHOR),
        Some(Hit::Component { .. }) => Some(COMPONENT),
        Some(Hit::Guide { .. }) => Some(GUIDE),
        // An attachment's items (Decompose Corner, Decompose Cap…) are in the node menu, and so is an extra node's
        // Add Corner Component.
        Some(Hit::Attachment { .. } | Hit::ExtraNode { .. }) => Some(NODE),
        None if !whole.is_empty() || selection.shapes.iter().any(|&s| scene.path(s).is_some()) => Some(PATH),
        None if !selection.on_curve_points(scene).is_empty() || !selection.intersections.is_empty() => Some(NODE),
        None if !selection.guides.is_empty() => Some(GUIDE),
        None if !selection.anchors.is_empty() => Some(ANCHOR),
        None if !selection.shapes.is_empty() => Some(COMPONENT),
        None if !scene.ps_hints.selected(selection).is_empty() => Some(HINT),
        None if !fontcraft_tools::attachment::items(scene, selection).is_empty() => Some(NODE),
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

/// Every command a menu may show, in menu order. A command whose `when` names a tool shows only while `tool` is it.
fn candidates(hints_shown: bool, instructor: bool, tool: &str) -> Vec<Candidate> {
    let mut all: Vec<Candidate> = command_specs()
        .iter()
        .filter(|c| !c.context.is_empty())
        .filter(|c| c.when.and_then(|w| w.strip_prefix("tool:")).is_none_or(|t| t == tool))
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
    if instructor {
        for (id, context) in TT_ITEMS {
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
    let all = candidates(hints_shown, crate::canvas::hinting::badges::instructor_shows(app), app.view.tool);
    // On a guide the menu offers the guide's commands; adding a guide is for clicks off guides.
    let on_guide = kinds.contains(&GUIDE);
    let text_mode = crate::canvas::text::text_mode(&app.view);
    let visible = |id: &str| match &scene {
        _ if text_mode && !glyph_item(id) => false,
        _ if on_guide && matches!(id, "guide.add" | "guide.addGlobal") => false,
        _ if HIDDEN.contains(&id) => false,
        _ if IMAGE_ITEMS.contains(&id) => image_item_visible(app, id),
        Some((scene, selection)) => {
            item_state::component_targets(app.session.font().ok(), scene, selection, id).is_none_or(|list| !list.is_empty())
                && item_state::visible(scene, selection, id)
        }
        None => id != GUIDE_PROPERTIES,
    };
    let mut shown: Vec<&'static str> = Vec::new();
    let mut out = Vec::new();
    for &kind in kinds {
        if kind == HOI {
            out.extend(hoi_section(app));
            continue;
        }
        if kind == TT_INSTRUCTOR {
            if instructor_active(app) {
                out.push(instructor_section(app));
            }
            continue;
        }
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
            .and_then(|()| hint_ready(selection, c.id))
            .and_then(|()| instruction_ready(app, c.id))
            .and_then(|()| point_ready(app, at, c.id)),
        None => ui_enabled(app, c.id),
    };
    let choices = if c.id == SET_STEM_REFERENCE && check.is_ok() { stem_choices(app) } else { Vec::new() };
    ContextItem {
        command: c.id,
        label: at.and_then(|(scene, selection)| item_state::label(scene, selection, c.id)).unwrap_or(c.label),
        enabled: check.is_ok(),
        disabled_reason: check.err(),
        checked: match c.id {
            "glyph.setExport" => at.and_then(|(scene, _)| edited_exports(app, &scene.glyph)),
            _ => at.and_then(|(scene, selection)| item_state::checked(scene, selection, c.id)),
        },
        alternate: ALTERNATES.contains(&c.id),
        choices,
    }
}

/// Set Stem Reference's choices for the selected instructions.
fn stem_choices(app: &FontCraftApp) -> Vec<ItemChoice> {
    let (Some(tt), Ok(font)) = (app.models.hint_canvas.tt.as_ref(), app.session.font()) else { return Vec::new() };
    let chosen = app.view.hinting.instructor.selected_in(&tt.glyph, &tt.layer);
    crate::app::models::hint_canvas::stem_choices(font, &tt.glyph, &tt.layer, &chosen)
        .into_iter()
        .map(|(choice, checked)| {
            let label = match &choice {
                Choice::Auto => "Auto".to_string(),
                Choice::None => "No Stem".to_string(),
                Choice::Named(name) => name.clone(),
            };
            ItemChoice { value: choice.param().to_string(), label, checked }
        })
        .collect()
}

/// The hint items that act on selected hints need one.
fn hint_ready(selection: Option<&Selection>, id: &str) -> Result<(), String> {
    match id {
        "hint.ps.makeGhostHint" | "hint.ps.deleteHints" if selection.is_none_or(|s| s.hints.is_empty()) => Err("select a hint first".into()),
        _ => Ok(()),
    }
}

/// The instruction items need selected instructions of the edited layer: Reverse and Set Stem Reference Stem
/// instructions only, Make Triple Hint three vertical Stem instructions outside any triple hint.
fn instruction_ready(app: &FontCraftApp, id: &str) -> Result<(), String> {
    if !INSTRUCTION_ITEMS.contains(&id) {
        return Ok(());
    }
    let Some(tt) = app.models.hint_canvas.tt.as_ref() else { return Err("select an instruction first".into()) };
    let chosen = app.view.hinting.instructor.selected_in(&tt.glyph, &tt.layer);
    let marks: Vec<&TtMark> = tt.instructions.iter().filter(|m| chosen.contains(&m.index)).collect();
    if marks.is_empty() {
        return Err("select an instruction first".into());
    }
    let stem = |m: &&TtMark| matches!(m.kind, BadgeKind::Stem | BadgeKind::TripleStem);
    match id {
        "hint.tt.reverse" if !marks.iter().all(stem) => Err("Reverse swaps the points of Stem instructions; select only Stem instructions".into()),
        SET_STEM_REFERENCE if !marks.iter().all(stem) => {
            Err("Set Stem Reference sets the stem of Stem instructions; select only Stem instructions".into())
        }
        "hint.tt.makeTripleHint" if marks.len() != 3 || !marks.iter().all(|m| m.kind == BadgeKind::Stem && !m.horizontal) => {
            Err("select three vertical Stem instructions that belong to no triple hint".into())
        }
        _ => Ok(()),
    }
}

/// The point items need selected points that the command would take: Snap Point untouched ones, Stem pairs that
/// are not both touched, Shift Points one touched origin and untouched targets, Interpolate two touched references
/// and an untouched target, Delta a point of the main master's layer in the vertical direction. Touched means touched
/// in the Instructor's direction by the layer's instructions, as the commands check it
/// (`fontcraft_hinting::TouchedState`), and the reason is the commands' own where they give one.
fn point_ready(app: &FontCraftApp, at: Option<&(Scene<'_>, &Selection)>, id: &str) -> Result<(), String> {
    if !POINT_ITEMS.contains(&id) {
        return Ok(());
    }
    let Some((scene, selection)) = at else { return Err("select a point first".into()) };
    if scene.side != LayerSide::Foreground {
        return Err("instructions belong to the foreground; leave the background first".into());
    }
    let tt_direction = app.view.hinting.instructor.direction;
    let direction = crate::canvas::hinting::hinting_direction(tt_direction);
    let mut chosen = tt_instructor::selected_points(scene, selection);
    if id == "hint.tt.addStemHint" {
        chosen = tt_instructor::stem_pairs(chosen, tt_direction);
    }
    let points: Vec<TtPoint> = chosen.iter().map(tt_point).collect();
    let list: Vec<TtInstruction> = tt_instructions(&scene.layer).map_err(|e| e.to_string())?.into_iter().map(|(_, t)| t).collect();
    let mut state = TouchedState::replay(&list, usize::MAX);
    match id {
        "hint.tt.snapPoint" => {
            if points.is_empty() {
                return Err("select a point first".into());
            }
            points.iter().try_for_each(|&p| add(&mut state, TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, direction, p)))
        }
        "hint.tt.addStemHint" => {
            if points.len() < 2 {
                return Err("select two points first".into());
            }
            if !points.len().is_multiple_of(2) {
                return Err(format!("{} points are selected: Stem pairs them, so select an even number first", points.len()));
            }
            for pair in points.chunks(2) {
                let [a, b] = pair else { continue };
                let (origin, target) = if state.is_touched(b, direction) && !state.is_touched(a, direction) { (*b, *a) } else { (*a, *b) };
                let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, direction, target);
                t.origin = Some(origin);
                add(&mut state, t)?;
            }
            Ok(())
        }
        "hint.tt.shiftPoints" => {
            let (touched, untouched): (Vec<TtPoint>, Vec<TtPoint>) = points.iter().copied().partition(|p| state.is_touched(p, direction));
            let ([origin], false) = (touched.as_slice(), untouched.is_empty()) else {
                return Err("select one touched point, the origin, and untouched points to shift first".into());
            };
            let origin = *origin;
            untouched.into_iter().try_for_each(|target| {
                let mut t = TtInstruction::new(TtKind::Shift, direction, target);
                t.origin = Some(origin);
                add(&mut state, t)
            })
        }
        "hint.tt.interpolate" => {
            let (touched, untouched): (Vec<TtPoint>, Vec<TtPoint>) = points.iter().copied().partition(|p| state.is_touched(p, direction));
            let ([a, b], [target]) = (touched.as_slice(), untouched.as_slice()) else {
                return Err("select two touched points, the references, and one untouched point first".into());
            };
            let mut t = TtInstruction::new(TtKind::Interpolate, direction, *target);
            t.origin = Some(*a);
            t.other = Some(*b);
            add(&mut state, t)
        }
        _ => {
            if points.is_empty() {
                return Err("select a point first".into());
            }
            if direction != Direction::Vertical {
                return Err("Delta instructions are vertical; switch the Instructor to the vertical direction first".into());
            }
            let main = app.session.font().map(main_master).unwrap_or_default();
            if scene.layer.master != main {
                return Err("Delta instructions belong to the main master; edit its layer first".into());
            }
            Ok(())
        }
    }
}

/// Checks `t` against `state` as the commands do, then runs it, so the next instruction sees the points it touched.
fn add(state: &mut TouchedState, t: TtInstruction) -> Result<(), String> {
    state.check(&t)?;
    state.run(&t);
    Ok(())
}

/// A selected point as the instructions name it.
fn tt_point(p: &tt_instructor::HintPoint) -> TtPoint {
    match p.other {
        None => TtPoint::Node(NodeRef::new(p.path, p.node)),
        Some([path, node]) => TtPoint::Intersection { a: NodeRef::new(p.path, p.node), b: NodeRef::new(path, node) },
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
    let base = app.with_targets(CommandRef::new(id)).params;
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
    let components = scene_of(app).and_then(|(scene, selection)| item_state::component_targets(app.session.font().ok(), &scene, selection, id));
    if let Some(components) = components {
        m.insert("components".into(), json!(components));
    }
    match item_state::one_layer(id) {
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
        "capComponent.decompose" | "segmentComponent.decompose" | "brush.decompose" => {
            let kind = item_state::decompose_kind(id);
            if let Some(kind) = kind
                && let Some(list) = scene_says(&|scene, selection| json!(item_state::selected_attachments(scene, selection, kind)))
            {
                m.insert(kind.list_key().into(), list);
            }
            if let Some(Value::Object(target)) = scene_says(&|scene, _| json!({"glyph": scene.glyph, "layer": scene.layer_id})) {
                for (k, v) in target {
                    m.entry(k).or_insert(v);
                }
            }
        }
        "hint.tt.autohintGlyph" => {
            m.insert("glyphs".into(), json!(menu.glyphs()));
            if let Some(master) = scene_says(&|scene, _| json!(scene.layer.master)) {
                m.insert("layer".into(), master);
            }
        }
        "hint.tt.removeHints" => {
            m.insert("glyphs".into(), json!(menu.glyphs()));
            if let Some(layer) = scene_says(&|scene, _| json!(scene.layer_id)) {
                m.insert("layer".into(), layer);
            }
        }
        "glyph.setExport" => {
            let exports = edited_exports(app, &menu.glyph).unwrap_or(true);
            m.insert("glyphs".into(), json!(menu.glyphs()));
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
        return vec![shell("fontView.setSelection", json!({"glyphs": menu.glyphs(), "reveal": true})), shell("fontView.show", json!({}))];
    }
    if !menu.glyphs.is_empty() {
        // Selected glyphs shown in several layers: one run per layer, as one step.
        let run = app.with_targets(CommandRef::new(id));
        if run.id == targets::BATCH {
            return vec![Msg::Run(run)];
        }
    }
    if menu.sections.iter().any(|s| s.context == TT_INSTRUCTOR)
        && let Some(steps) = instructor_steps(app, id)
    {
        return steps;
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
    if let Some((_, purpose)) = PICKS.iter().find(|(item, _)| *item == id) {
        let mut m = params.as_object().cloned().unwrap_or_default();
        m.insert("purpose".into(), json!(purpose));
        return vec![shell(crate::dialogs::glyph_picker::COMMAND, Value::Object(m))];
    }
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
    // The positions after deleted instructions move, so no instruction stays selected.
    if id == "hint.tt.deleteInstructions" {
        out.push(shell("hint.tt.selectInstructions", json!({"instructions": [], "mode": "replace"})));
    }
    out
}

/// Runs item `id` of the open canvas menu as a click on it does, and closes the menu; `run` applies each message and
/// gives its result. The item is checked as the menu would draw it now: a menu whose glyph changed, or that a dialog
/// covers, closes; an item that is gone or disabled is refused and leaves the menu open. Returns the first step's
/// result: the item's command, or the dialog that asks for it.
pub fn invoke(app: &mut FontCraftApp, id: &str, run: fn(&mut FontCraftApp, Msg) -> Result<Value, String>) -> Result<Value, String> {
    invoke_choice(app, id, None, run)
}

/// Runs item `id` of the open canvas menu as [`invoke`] does; a submenu item needs `choice`, one of its values, which
/// its command gets (Set Stem Reference's `stem`). A choice for a plain item, or one the submenu does not hold, is
/// refused and leaves the menu open.
pub fn invoke_choice(
    app: &mut FontCraftApp,
    id: &str,
    choice: Option<&str>,
    run: fn(&mut FontCraftApp, Msg) -> Result<Value, String>,
) -> Result<Value, String> {
    let menu = app.context_menus.canvas.clone().ok_or("no context menu is open")?;
    let same = still(app, &menu);
    if !same || app.dialogs.open.is_some() {
        app.context_menus.canvas = None;
        let why = if same { "a dialog is open" } else { "the edited glyph or layer changed" };
        return Err(format!("{why}; the context menu closed"));
    }
    if app.closing.is_open() {
        return Err("the question about unsaved changes is open".into());
    }
    let kinds: Vec<&'static str> = menu.sections.iter().map(|s| s.context).collect();
    let item = sections(app, &kinds).into_iter().flat_map(|s| s.items).find(|i| i.command == id);
    let item = item.ok_or_else(|| format!("`{id}` is not in the context menu"))?;
    if !item.enabled {
        return Err(item.disabled_reason.unwrap_or_else(|| format!("`{id}` is not available now")));
    }
    let steps = match (item.choices.is_empty(), choice) {
        (true, None) => None,
        (true, Some(_)) => return Err(format!("`{id}` is not a submenu: give no `value`")),
        (false, Some(value)) if item.choices.iter().any(|c| c.value == value) => Some(choice_steps(app, &menu, item.command, value)),
        (false, _) => {
            let values: Vec<&str> = item.choices.iter().map(|c| c.value.as_str()).collect();
            return Err(format!("`{id}` is a submenu: give `value`, one of {}", values.join(", ")));
        }
    };
    app.context_menus.canvas = None;
    let steps = match steps {
        Some(steps) => steps,
        None => steps_for(app, &menu, item.command),
    };
    let (first, steps) = one_transaction(steps, item.label);
    let adds_anchor = item.command == "anchor.add";
    let mut result = None;
    for msg in steps {
        let value = run(app, msg)?;
        result.get_or_insert(if first { value.pointer("/results/0").cloned().unwrap_or(value) } else { value });
    }
    let result = result.unwrap_or(Value::Null);
    if adds_anchor {
        crate::canvas::anchor_editor::after_add(app, &result);
    }
    Ok(result)
}

/// `steps` with the engine commands they start with as one `command.batch` labelled `label`, so that they are one undo
/// step and all or none take effect; and whether there were several.
fn one_transaction(mut steps: Vec<Msg>, label: &str) -> (bool, Vec<Msg>) {
    let runs = steps.iter().take_while(|msg| matches!(msg, Msg::Run(_))).count();
    if runs < 2 {
        return (false, steps);
    }
    let rest = steps.split_off(runs);
    let batch: Vec<Value> = steps
        .iter()
        .filter_map(|msg| match msg {
            Msg::Run(command) if command.params.is_null() => Some(json!({"command": command.id})),
            Msg::Run(command) => Some(json!({"command": command.id, "params": command.params})),
            _ => None,
        })
        .collect();
    let batch = Msg::run("command.batch", json!({"label": label, "steps": batch}));
    (true, std::iter::once(batch).chain(rest).collect())
}

/// Whether the canvas still edits what `menu` was opened on.
pub fn still(app: &FontCraftApp, menu: &CanvasMenu) -> bool {
    scene_of(app).is_some_and(|(scene, _)| scene.glyph == menu.glyph) && tools::target(&app.session, &app.view) == menu.target
}

/// The messages a submenu item posts for `value`: its command with the value under the key the submenu chooses. The
/// Interpolation submenu's command runs once per side of the layer the picked knobs are on.
fn choice_steps(app: &mut FontCraftApp, menu: &CanvasMenu, id: &str, value: &str) -> Vec<Msg> {
    if id == INTERPOLATION {
        let picks = app.tools.toolbox.hoi_picks();
        let runs = scene_of(app)
            .map(|(scene, _)| fontcraft_tools::hoi::invocations(&scene, picks, id, &[("type", json!(value))], |_| true))
            .unwrap_or_default();
        return runs.into_iter().map(|i| FontCraftApp::route(CommandRef::with_params(i.id, i.params))).collect();
    }
    let mut params = params_for(app, menu, id);
    if let Some(m) = params.as_object_mut() {
        m.insert("stem".into(), json!(value));
    }
    vec![FontCraftApp::route(CommandRef::with_params(id, params))]
}

/// The canvas menu's colour swatches: the edited glyph's colour label, or with Alt the active layer's. With glyphs of the
/// text selected, theirs, or with Alt that of the layer each shows.
pub fn color_steps(app: &FontCraftApp, menu: &CanvasMenu, color: Option<u8>, layer: bool) -> Vec<Msg> {
    if !menu.glyphs.is_empty() {
        if !layer {
            let glyph_menu = GlyphMenu { glyphs: menu.glyphs(), ..GlyphMenu::default() };
            return glyph_menu::steps(&glyph_menu, &glyph_menu::GlyphAction::Color(color));
        }
        let Ok(font) = app.session.font() else { return Vec::new() };
        let groups = targets::text_selection(font, &app.view);
        // The menu's glyphs were fixed when it opened; a selection that moved since is not theirs.
        let now: std::collections::HashSet<&str> = groups.iter().flat_map(|group| group.glyphs.iter().map(String::as_str)).collect();
        if now != menu.glyphs.iter().map(String::as_str).collect() {
            return Vec::new();
        }
        let steps: Vec<Msg> = groups
            .into_iter()
            .flat_map(|group| {
                let glyph_menu = GlyphMenu { glyphs: group.glyphs, layer: Some(group.layer), ..GlyphMenu::default() };
                glyph_menu::steps(&glyph_menu, &glyph_menu::GlyphAction::LayerColor(color))
            })
            .collect();
        let label = find_command("glyph.setLayerColorLabel").map_or("Layer Color", |spec| spec.label);
        return one_transaction(steps, label).1;
    }
    let target = tools::target(&app.session, &app.view).map(|t| t.layer);
    let glyph_menu = GlyphMenu { glyphs: vec![menu.glyph.clone()], layer: target, ..GlyphMenu::default() };
    let action = if layer { glyph_menu::GlyphAction::LayerColor(color) } else { glyph_menu::GlyphAction::Color(color) };
    glyph_menu::steps(&glyph_menu, &action)
}
