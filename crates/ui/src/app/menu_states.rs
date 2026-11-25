//! What the live state says about menu bar rows beyond each engine command's `enabled` rule: the check mark of every
//! on/off switch ([`SWITCHES`], [`checks`]) and the rows the selection greys out ([`greyed`]).
//!
//! An engine row is greyed out when firing it would be refused for what the window gives it as targets:
//!
//! | Window state | Rows greyed out | Why (the row's tooltip, `disabled_reason` in `ui.menu.list`) |
//! |---|---|---|
//! | No glyph chosen ([`FontCraftApp::targets_chosen`]) | every command that takes targets | `<label>: select a glyph first`, the refusal firing it gives |
//! | Font View in front with glyphs selected | commands that act on selected nodes, paths, anchors… and not on whole glyphs | `edit a glyph and select … first` |
//! | A glyph in the edit view | commands whose objects the canvas selection does not supply ([`item_state::supplies`]) | `select … first` |
//!
//! A selected placed image supplies the commands that take `image` (Move, Delete, the transformations), with or without
//! other objects selected.
//!
//! Never greyed: commands that open a dialog when fired without parameters (the dialog asks for what is missing), the
//! component pickers, the Edit menu's clipboard rows (a focused list or text mode takes them), and in text mode the
//! Delete row.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::behavior::dock::PanelState;
use fontcraft_engine::CommandSpec;

use super::FontCraftApp;
use super::context_menu::{item_state, scene_of};
use crate::canvas::Display;
use crate::canvas::overlays::Switches;
use crate::shell::dock::PanelId;

/// The menu commands that flip an on/off switch: each row shows a check mark while its switch is on. A command that
/// only opens a window (its row never closes it) is not a switch.
pub const SWITCHES: &[&str] = &[
    "audit.toggleLiveAudit",
    "view.showNodes",
    "view.showNodesInForeground",
    "view.showNodesInBackground",
    "view.showExtraNodes",
    "view.showMetrics",
    "view.showMetricsNames",
    "view.showInfo",
    "view.showBackground",
    "view.showGuides",
    "view.showBoundingBox",
    "view.showAnnotations",
    "view.showAnchors",
    "view.fillPreview",
    "view.showImage",
    "view.showOutlineAtLocation",
    "view.showMasterCompatibility",
    "edit.keepLayerSelectionInSync",
    "edit.toggleMatchWhenEditing",
    "view.showGroupMembers",
    "view.showHints",
    "view.showTunniLines",
    "view.showCurveTension",
    "view.showCurveLength",
    "view.showIndexes",
    "view.showDetailsAcrossGlyphs",
    "view.lockOutline",
    "view.showMetricsBeam",
    "view.suggestHandles",
    "view.suggestLines",
    "view.suggestDistance",
    "view.suggestStems",
    "view.showStemWidths",
    "view.showCounterWidths",
    "view.showCornerAngles",
    "view.showQuickMeasurement",
    "view.showMeasurementLine",
    "view.setSpacingCutOff",
    "path.editBackground",
    "path.togglePowerNudge",
    "window.palette",
    "window.previewPanel",
    "view.animateAxis",
    "audit.showPanel",
];

/// Edit menu rows that a focused list or text mode takes before the selection is asked (`app::clipboard`).
const EDIT_ROWS: &[&str] = &["edit.copy", "edit.cut", "edit.paste", "edit.pasteSpecial", "edit.selectAll"];

/// Rows that open the component picker, which asks for what is missing (`dialogs::glyph_picker`).
const PICKER_ROWS: &[&str] =
    &["glyph.addComponent", "glyph.addComponentAllMasters", "cornerComponent.add", "capComponent.add", "segmentComponent.add", "brush.add"];

/// Brings the menu bar's check marks and greyed rows up to date; the bar is rebuilt only when one changed. The greyed
/// rows are worked out again only when what they read moved, and not while a tool drags.
pub fn refresh(app: &mut FontCraftApp) {
    let checks = checks(app);
    app.catalog.set_checks(checks);
    if app.tools.toolbox.busy() {
        return;
    }
    let key = greyed_key(app);
    if !app.catalog.greyed_for(key) {
        let greyed = greyed(app);
        app.catalog.set_greyed(key, greyed);
    }
}

/// What the greyed rows read: the documents, the view state, what is chosen and the canvas selection.
fn greyed_key(app: &FontCraftApp) -> u64 {
    let mut h = DefaultHasher::new();
    app.session.documents().len().hash(&mut h);
    app.session.active().map(|d| (d.id, d.revision)).hash(&mut h);
    app.view_revision.hash(&mut h);
    app.targets_chosen().hash(&mut h);
    app.font_view_targets().hash(&mut h);
    app.view.font_view.selected.hash(&mut h);
    app.view.glyph.hash(&mut h);
    crate::canvas::text::text_mode(&app.view).hash(&mut h);
    app.tools.focus.hash(&mut h);
    app.tools.image.hash(&mut h);
    serde_json::to_string(&app.tools.selection).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Every engine menu row the selection greys out, with why.
pub fn greyed(app: &FontCraftApp) -> Vec<(&'static str, String)> {
    if app.session.font().is_err() {
        return Vec::new();
    }
    let chosen = app.targets_chosen();
    let font_view = app.font_view_targets();
    let canvas = if chosen && !font_view { scene_of(app) } else { None };
    let text_mode = crate::canvas::text::text_mode(&app.view);
    let image = canvas.is_some() && crate::canvas::images::is_selected(app);
    let mut out = Vec::new();
    for spec in fontcraft_engine::cmd::command_specs() {
        if spec.menu.is_empty() || spec.targets.is_empty() || exempt(app, spec, text_mode) {
            continue;
        }
        let refused = if !chosen {
            Err(super::select_glyph_first(spec.id))
        } else if font_view {
            item_state::objects_wanted(spec).map_or(Ok(()), |what| Err(format!("edit a glyph and select {what} first")))
        } else if image && crate::canvas::tools::targets::takes_param(spec, "image?: bool") {
            Ok(())
        } else {
            canvas.as_ref().map_or(Ok(()), |(scene, selection)| item_state::supplies(scene, selection, spec))
        };
        if let Err(why) = refused {
            out.push((spec.id, why));
        }
    }
    out
}

/// Whether something other than the selection answers for the row of `spec` when it is fired.
fn exempt(app: &FontCraftApp, spec: &CommandSpec, text_mode: bool) -> bool {
    EDIT_ROWS.contains(&spec.id) || PICKER_ROWS.contains(&spec.id) || (text_mode && spec.id == "edit.delete") || app.catalog.has_dialog(spec.id)
}

/// Each switch of [`SWITCHES`] that the window has, and whether it is on.
pub fn checks(app: &FontCraftApp) -> Vec<(&'static str, bool)> {
    // `flag` and `switch` hand out the field a command flips; copies keep the state untouched.
    let mut display = app.view.display;
    let mut switches = app.overlays.switches.clone();
    SWITCHES.iter().filter_map(|&id| checked(app, &mut display, &mut switches, id).map(|on| (id, on))).collect()
}

/// The state of the switch `id` flips; `None` for a command that is no switch. Edit Background reads the edit view's
/// side, Power Nudge the user setting, the panel switches whether their panel shows.
fn checked(app: &FontCraftApp, display: &mut Display, switches: &mut Switches, id: &str) -> Option<bool> {
    if let Some(on) = crate::shell::view::flag(display, id) {
        return Some(*on);
    }
    if let Some(on) = crate::shell::overlays::switch(switches, id) {
        return Some(*on);
    }
    let shows = |panel: PanelId| app.view.workspace.layout.panel_state(&panel) != PanelState::Hidden;
    Some(match id {
        "view.showMasterCompatibility" => app.view.display.master_compatibility,
        "edit.keepLayerSelectionInSync" => app.view.display.keep_layer_selection_in_sync,
        "edit.toggleMatchWhenEditing" => app.view.display.match_when_editing,
        "view.showHints" => app.view.display.hints,
        "view.showGroupMembers" => app.view.text.group_members,
        "view.showMeasurementLine" => app.view.text.measurement,
        "view.showIndexes" => app.overlays.switches.indexes.any(),
        "view.showMetricsBeam" => app.overlays.switches.beam,
        "view.setSpacingCutOff" => app.view.space_center.cut_off.on,
        "path.editBackground" => app.view.edit_background,
        "path.togglePowerNudge" => app.session.settings().get_bool(fontcraft_tools::drag_aids::POWER_NUDGE_SETTING).unwrap_or(false),
        "window.palette" => crate::shell::window::palette_shows(app),
        "window.previewPanel" => shows(PanelId::Preview),
        "view.animateAxis" => app.view.preview.sweep.is_some(),
        "audit.showPanel" => shows(PanelId::FontAudit),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::menu::{Item, ItemKind, Node};
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::SWITCHES;
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    fn app() -> FontCraftApp {
        FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
    }

    fn find<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Item> {
        nodes.iter().find_map(|node| match node {
            Node::Item(item) if item.command.id == id => Some(item),
            Node::Submenu { children, .. } => find(children, id),
            _ => None,
        })
    }

    /// The menu bar row that runs `id`.
    fn row<'a>(app: &'a FontCraftApp, id: &str) -> Option<&'a Item> {
        app.catalog.menu.menus.iter().find_map(|menu| find(&menu.children, id))
    }

    fn check(app: &FontCraftApp, id: &str) -> Option<bool> {
        match row(app, id)?.kind {
            ItemKind::Check(on) => Some(on),
            _ => None,
        }
    }

    /// Every switch with a menu row shows a check mark, and the mark follows the switch whichever way it is flipped.
    #[test]
    fn switch_rows_show_the_switch_state() {
        let mut app = app();
        assert_eq!(check(&app, "view.showMetrics"), Some(true), "metrics show by default");
        assert_eq!(check(&app, "view.lockOutline"), Some(false));
        assert_eq!(check(&app, "view.suggestHandles"), Some(false));
        assert_eq!(check(&app, "path.editBackground"), Some(false));
        assert_eq!(check(&app, "view.showGroupMembers"), Some(false));

        for id in ["view.showMetrics", "view.lockOutline", "view.suggestHandles", "path.editBackground", "view.showGroupMembers"] {
            let before = check(&app, id);
            app.fire(CommandRef::new(id));
            app.drain();
            assert_eq!(check(&app, id), before.map(|on| !on), "{id} flips its check mark");
        }
        app.sent.push(Msg::Shell(CommandRef::with_params("view.lockOutline", json!({"on": false}))));
        app.drain();
        assert_eq!(check(&app, "view.lockOutline"), Some(false), "set from the control channel");

        for id in SWITCHES {
            if row(&app, id).is_some() {
                assert!(check(&app, id).is_some(), "{id}'s row is a check row");
            }
        }
        assert_eq!(check(&app, "view.zoomIn"), None, "a plain command shows no mark");
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: serde_json::Value) {
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
    }

    /// Whether the row of `id` can be chosen, and the tooltip that says why not.
    fn state(app: &FontCraftApp, id: &str) -> (bool, Option<String>) {
        let row = row(app, id).unwrap_or_else(|| panic!("{id} has a menu row"));
        (row.enabled, row.tooltip.clone())
    }

    /// A row is greyed out while firing it would be refused for its targets: with nothing chosen it says what firing
    /// says; in Font View a command on nodes asks for a glyph to be edited; in the edit view it waits for the nodes.
    #[test]
    fn rows_grey_out_until_the_selection_supplies_their_targets() {
        let mut app = app();
        shell(&mut app, "fontView.show", json!({}));
        assert!(!app.targets_chosen());
        assert_eq!(state(&app, "path.removeOverlap"), (false, Some("Remove Overlap: select a glyph first".into())));
        app.status.clear();
        app.fire(CommandRef::new("path.removeOverlap"));
        assert_eq!(app.status, "Remove Overlap: select a glyph first", "the row and the refusal agree");
        assert!(row(&app, "edit.copy").is_none_or(|r| r.enabled), "a focused list can take Copy");
        let listed = crate::control::dispatch(&mut app, "ui.menu.list", &json!({})).unwrap();
        let entry = listed
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|menu| menu["items"].as_array().unwrap())
            .find(|item| item["command"] == "path.removeOverlap")
            .unwrap();
        assert_eq!(entry["disabled_reason"], "Remove Overlap: select a glyph first", "{entry}");

        shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A"]}));
        assert!(state(&app, "path.removeOverlap").0, "it acts on the selected glyphs");
        assert_eq!(state(&app, "path.spacePoints"), (false, Some("edit a glyph and select nodes first".into())));

        let mut app = self::app();
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.drain();
        assert!(app.targets_chosen());
        assert!(state(&app, "path.removeOverlap").0, "it acts on the edited glyph");
        assert_eq!(state(&app, "path.spacePoints"), (false, Some("select nodes first".into())));
        app.tools.selection = fontcraft_tools::Selection::from_items([fontcraft_tools::Item::Point(0, 0), fontcraft_tools::Item::Point(0, 2)]);
        app.drain();
        assert_eq!(state(&app, "path.spacePoints"), (true, None), "the selected nodes supply it");
    }

    /// The panel switches read whether their panel shows.
    #[test]
    fn panel_switch_rows_follow_the_panel() {
        let mut app = app();
        let before = check(&app, "window.previewPanel").expect("a check row");
        app.fire(CommandRef::new("window.previewPanel"));
        app.drain();
        assert_eq!(check(&app, "window.previewPanel"), Some(!before));
    }
}
