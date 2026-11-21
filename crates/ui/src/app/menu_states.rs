//! What the live state says about menu bar rows beyond each engine command's `enabled` rule: the check mark of every
//! on/off switch ([`SWITCHES`], [`checks`]).

use craft::behavior::dock::PanelState;

use super::FontCraftApp;
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
    "view.showMasterCompatibility",
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
    "audit.showPanel",
];

/// Brings the menu bar's check marks up to date; the bar is rebuilt only when one changed.
pub fn refresh(app: &mut FontCraftApp) {
    let checks = checks(app);
    app.catalog.set_checks(checks);
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
    use crate::app::{APP, FontCraftApp, Msg};

    fn app() -> FontCraftApp {
        FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()))
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
