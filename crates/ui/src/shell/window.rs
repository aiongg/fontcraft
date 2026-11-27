//! Window commands: which panels show, and the Window menu's panel rows.
//!
//! The Palette (`window.palette`) is every panel docked at the window's end edge. Hiding it closes them and keeps
//! their list; showing it opens that list again where each one was, or the panels the default layout docks there.
//! Window › Panels lists every dock panel but the document area as a check row that runs `window.togglePanel`,
//! checked while the panel is open; the rows are rebuilt when a panel opens or closes ([`refresh_menu`]). craft's
//! `WindowMenuRows` draws such rows into a menu body of its own, which the data-driven menu bar has no place for.
//!
//! A Palette section is a dock panel. `palette.toggleSection` collapses the group that holds it to its tab strip, or
//! expands it again, as the group's own minimize button does; the panels tabbed with it collapse with it. A
//! plug-in's section is a form in the Modules panel, so it collapses with that panel.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::behavior::dock::{Action, DockEdge, Layout, PanelState};
use craft::menu::{ItemKind, Row};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::dock::{self, PanelId, PanelWords};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.togglePanel",
        label: "Show or Hide Panel",
        menu: &[],
        shortcut: None,
        params: "{panel: string} show the panel with this id, or hide it when it shows (ids: `PanelId::id`)",
    },
    UiCommand {
        id: "window.resetLayout",
        label: "Reset Layout",
        menu: &["Window"],
        shortcut: None,
        params: "{} put every panel back where it starts",
    },
    UiCommand {
        id: "window.palette",
        label: "Palette",
        menu: &["Window"],
        shortcut: Some("Cmd+Alt+P"),
        params: "{on?: bool} show or hide the Palette: the panels docked at the window's end edge (omitted: toggle)",
    },
    UiCommand {
        id: "palette.toggleSection",
        label: "Collapse / Expand Section",
        menu: &[],
        shortcut: None,
        params: "{section: panel id (`dimensions`, `fitCurve`, `layers`, `transformations`, `infoBox`, `modules` for the plug-ins' sections, …), expanded?: bool (omitted: toggle)} collapse an open Palette section to its title row, with the panels tabbed with it, or expand it again",
    },
];

/// The submenu that lists the dock panels.
pub const PANELS_MENU: &[&str] = &["Window", "Panels"];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "window.togglePanel" => {
            let id = command.params.get("panel").and_then(|p| p.as_str()).ok_or("`panel` is required")?;
            let panel = PanelId::from_id(id).ok_or_else(|| format!("no panel `{id}`"))?;
            dock::toggle(&mut app.view.workspace, panel);
            Ok(())
        }
        "window.resetLayout" => {
            app.view.workspace = dock::default_workspace();
            app.view.palette.hidden.clear();
            Ok(())
        }
        "window.palette" => {
            let shown = palette_panels(app.view.workspace.layout());
            let on = match command.params.get("on") {
                None | Some(Value::Null) => shown.is_empty(),
                Some(Value::Bool(on)) => *on,
                Some(_) => return Err("`on` must be true or false".into()),
            };
            if on {
                show_palette(app);
            } else {
                hide_palette(app, shown);
            }
            Ok(())
        }
        "palette.toggleSection" => toggle_section(app, &command.params),
        other => Err(format!("no window command `{other}`")),
    }
}

/// `palette.toggleSection`: minimizes or expands the dock group of the panel `section`.
fn toggle_section(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let id = super::params::string(params, "section")?.ok_or("`section` (a panel id) is required")?;
    let panel = PanelId::from_id(id).filter(|p| *p != PanelId::Document).ok_or_else(|| format!("no Palette section `{id}`"))?;
    let layout = app.view.workspace.layout();
    let place = layout.place(&panel).filter(|_| layout.panel_state(&panel) != PanelState::Hidden);
    let place = place.ok_or_else(|| format!("the {id} section is not open (Window › Panels shows it)"))?;
    let minimized = layout.group(&place).map(|g| g.minimized).ok_or_else(|| format!("the {id} section is not in a panel group"))?;
    let expanded = super::params::flag(params, "expanded")?.unwrap_or(minimized);
    if expanded != minimized {
        return Ok(());
    }
    app.view
        .workspace
        .apply(Action::set_minimized(panel, !expanded), false, |p| p.permissions())
        .map_err(|e| format!("the {id} section cannot change: {e:?}"))?;
    Ok(())
}

/// Whether the Palette shows: a panel is docked at the window's end edge.
pub(crate) fn palette_shows(app: &FontCraftApp) -> bool {
    !palette_panels(app.view.workspace.layout()).is_empty()
}

/// The panels docked at the window's end edge, in layout order.
fn palette_panels(layout: &Layout<PanelId>) -> Vec<PanelId> {
    let at_end =
        |panel: &PanelId| layout.location(panel).ok().and_then(|l| l.group).and_then(|g| g.column).and_then(|c| c.side) == Some(DockEdge::End);
    layout.panels().into_iter().copied().filter(at_end).collect()
}

/// Closes the Palette's panels and keeps their list, with whether each one was in front of its group.
fn hide_palette(app: &mut FontCraftApp, shown: Vec<PanelId>) {
    if shown.is_empty() {
        return;
    }
    let layout = app.view.workspace.layout();
    let hidden: Vec<(PanelId, bool)> = shown.into_iter().map(|panel| (panel, layout.panel_state(&panel) == PanelState::Front)).collect();
    for (panel, _) in &hidden {
        // A refused close (a protected panel) leaves that panel open.
        let _ = app.view.workspace.set_visible(*panel, false, None, false, |p| p.permissions());
    }
    app.view.palette.hidden = hidden;
}

/// Opens the panels the Palette last closed where each one was, the ones that were in front of their group in front
/// again, or else the panels the default layout docks at the end edge.
fn show_palette(app: &mut FontCraftApp) {
    let mut wanted = std::mem::take(&mut app.view.palette.hidden);
    if wanted.is_empty() {
        wanted = palette_panels(dock::default_workspace().layout()).into_iter().map(|panel| (panel, false)).collect();
    }
    let mut first = None;
    for (panel, _) in &wanted {
        // Opening a panel can open the hidden panels of its group with it.
        if app.view.workspace.layout().panel_state(panel) != PanelState::Hidden {
            continue;
        }
        // A panel with no recorded place opens beside the first one opened.
        if app.view.workspace.set_visible(*panel, true, first, false, |p| p.permissions()).is_ok() && first.is_none() {
            first = Some(*panel);
        }
    }
    for (panel, front) in wanted {
        if front {
            // Showing an open panel brings it to the front of its group.
            let _ = app.view.workspace.set_visible(panel, true, None, false, |p| p.permissions());
        }
    }
}

/// Rebuilds the Window menu's panel rows when a panel opened or closed.
pub fn refresh_menu(app: &mut FontCraftApp) {
    let layout = app.view.workspace.layout();
    let open: Vec<bool> = PanelId::ALL.iter().map(|panel| layout.panel_state(panel) != PanelState::Hidden).collect();
    let mut h = DefaultHasher::new();
    open.hash(&mut h);
    app.catalog.set_panel_rows(h.finish(), || panel_rows(&open));
}

/// One check row per dock panel but the document area, in [`PanelId::ALL`] order; `open` answers for each panel.
fn panel_rows(open: &[bool]) -> Vec<Row> {
    let words = PanelWords::english();
    PanelId::ALL
        .iter()
        .zip(open)
        .filter(|(panel, _)| **panel != PanelId::Document)
        .map(|(panel, on)| {
            let command = CommandRef::with_params("window.togglePanel", json!({ "panel": panel.id() }));
            Row::new(PANELS_MENU, command, panel.title(&words).as_str()).kind(ItemKind::Check(*on))
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use craft::behavior::dock::PanelState;
    use craft::menu::{ItemKind, Node};
    use craft::ui::runtime::commands::CommandRef;
    use serde_json::json;

    use super::PanelId;
    use crate::testing::TestApp;

    /// The check mark of the Window › Panels row for the panel `id`.
    fn checked(t: &TestApp, id: &str) -> Option<bool> {
        let window = t.app.catalog.menu.menus.iter().find(|m| m.title == "Window")?;
        let rows = window.children.iter().find_map(|node| match node {
            Node::Submenu { label, children, .. } if label == "Panels" => Some(children),
            _ => None,
        })?;
        rows.iter().find_map(|node| match node {
            Node::Item(item) if item.command.params["panel"] == id => match item.kind {
                ItemKind::Check(on) => Some(on),
                _ => None,
            },
            _ => None,
        })
    }

    fn state(t: &TestApp, panel: PanelId) -> PanelState {
        t.app.view.workspace.layout().panel_state(&panel)
    }

    #[test]
    fn the_palette_and_the_panel_rows_show_and_hide_panels() {
        let mut t = TestApp::new();
        assert_eq!((checked(&t, "layers"), checked(&t, "fontInfo"), checked(&t, "document")), (Some(true), Some(false), None));

        t.app.fire(CommandRef::new("window.palette"));
        t.frame();
        assert_eq!((state(&t, PanelId::Layers), state(&t, PanelId::GlyphMetrics)), (PanelState::Hidden, PanelState::Hidden), "the Palette hides");
        assert_eq!(state(&t, PanelId::Sidebar), PanelState::Front, "a panel at the start edge is no Palette panel");
        assert_eq!(checked(&t, "layers"), Some(false), "the row follows the panel");
        t.app.fire(CommandRef::new("window.palette"));
        t.frame();
        assert_eq!(
            (state(&t, PanelId::GlyphMetrics), state(&t, PanelId::InfoBox)),
            (PanelState::Front, PanelState::Open),
            "the same tab is in front"
        );
        assert_eq!(checked(&t, "layers"), Some(true));
        t.app.fire(CommandRef::with_params("window.palette", json!({"on": true})));
        t.frame();
        assert_eq!(state(&t, PanelId::GlyphMetrics), PanelState::Front, "showing a shown Palette changes nothing");

        // A closed panel comes back from its row.
        t.ok("ui.menu.invoke", json!({"command": "window.togglePanel", "params": {"panel": "fontInfo"}}));
        t.frame();
        assert_ne!(state(&t, PanelId::FontInfo), PanelState::Hidden);
        assert_eq!(checked(&t, "fontInfo"), Some(true));
    }

    /// Whether the dock group that holds `panel` shows only its tab strip.
    fn minimized(t: &TestApp, panel: PanelId) -> bool {
        let layout = t.app.view.workspace.layout();
        layout.place(&panel).and_then(|place| layout.group(&place)).unwrap().minimized
    }

    #[test]
    fn a_palette_section_collapses_and_expands() {
        let mut t = TestApp::new();
        assert!(!minimized(&t, PanelId::Layers));
        t.ok("engine.execute", json!({"command": "palette.toggleSection", "params": {"section": "layers"}}));
        assert!(minimized(&t, PanelId::Layers), "omitted, `expanded` toggles");
        assert!(!minimized(&t, PanelId::Dimensions), "other groups stay as they are");
        t.ok("engine.execute", json!({"command": "palette.toggleSection", "params": {"section": "layers", "expanded": false}}));
        assert!(minimized(&t, PanelId::Layers), "collapsing a collapsed section changes nothing");
        t.ok("engine.execute", json!({"command": "palette.toggleSection", "params": {"section": "layers", "expanded": true}}));
        assert!(!minimized(&t, PanelId::Layers));
        t.frame();

        let unknown = t.error("engine.execute", json!({"command": "palette.toggleSection", "params": {"section": "nope"}}));
        assert!(unknown.contains("nope"), "{unknown}");
        let closed = t.error("engine.execute", json!({"command": "palette.toggleSection", "params": {"section": "fontInfo"}}));
        assert!(closed.contains("not open"), "{closed}");
        assert!(!t.error("engine.execute", json!({"command": "palette.toggleSection", "params": {}})).is_empty());
    }
}
