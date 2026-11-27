//! The window's layout kept between runs: which panels show and where, the sidebars' widths and the sections' heights
//! (the dock's [`Workspace`]), and the panels Window › Palette opens again. Rows: `window.frameKept`,
//! `window.sidebarWidthsKept`.
//!
//! The desktop app's `UserStore` keeps the value [`to_value`] makes and hands it back at the next start, where
//! [`restore`] fits it to the panels this version has: a panel it does not know is left out, a panel the kept layout
//! does not know takes its place in the default layout, and a value that is no layout gives the default. Floating
//! panels are not kept: each opens at its usual place, so none can open off the desktop's monitors.

use craft::behavior::dock::{Location, Permissions, Workspace};
use serde::Deserialize as _;
use serde_json::{Value, json};

use super::dock::{self, PanelId};

/// The panels `window.palette` closed, each with whether it was in front of its group.
pub type ClosedPalette = Vec<(PanelId, bool)>;

/// The kept form of `workspace` and `palette`; `None` when it has none.
pub fn to_value(workspace: &Workspace<PanelId>, palette: &[(PanelId, bool)]) -> Option<Value> {
    let workspace = serde_json::to_value(workspace).ok()?;
    Some(json!({ "workspace": workspace, "palette": palette }))
}

/// The layout `value` keeps, fitted to the panels this version has.
pub fn restore(value: &Value) -> Result<(Workspace<PanelId>, ClosedPalette), String> {
    let unreadable = |e: serde_json::Error| format!("the kept window layout cannot be read: {e}");
    // Read with any panel ids, so one panel of another version does not lose the layout.
    let kept = value.get("workspace").ok_or("the kept window layout has no workspace")?;
    let mut any = Workspace::<String>::deserialize(kept).map_err(unreadable)?;
    let default = serde_json::to_value(dock::default_workspace()).and_then(Workspace::<String>::deserialize).map_err(unreadable)?;
    let permissions = |id: &String| PanelId::from_id(id).map_or(Permissions::ALL, PanelId::permissions);
    any.reconcile(|id| PanelId::from_id(id).is_some(), permissions, &default);
    // The unknown panels are hidden now, and some places name them.
    let mut known = serde_json::to_value(&any).map_err(unreadable)?;
    for list in ["hidden", "floated"] {
        if let Some(entries) = known.get_mut(list).and_then(Value::as_array_mut) {
            entries.retain(|entry| <(PanelId, Location<PanelId>)>::deserialize(entry).is_ok());
        }
    }
    let mut workspace = Workspace::<PanelId>::deserialize(&known).map_err(unreadable)?;
    let floating: Vec<PanelId> = workspace.layout().floating.iter().flat_map(|window| window.column.panels()).copied().collect();
    for panel in floating {
        workspace.set_visible(panel, false, None, false, |p| p.permissions()).map_err(|e| format!("the kept window layout cannot be used: {e:?}"))?;
    }
    let floated: Vec<PanelId> = workspace.hidden().iter().filter(|(_, place)| place.floating.is_some()).map(|(panel, _)| *panel).collect();
    workspace.forget_hidden(|panel| floated.contains(panel));
    if workspace.validate().is_err() || !workspace.layout().contains(&PanelId::Document) {
        return Err("the kept window layout cannot be used".into());
    }
    let closed = value.get("palette").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut palette = ClosedPalette::new();
    for (panel, front) in closed.iter().take(PanelId::ALL.len()).filter_map(|entry| <(PanelId, bool)>::deserialize(entry).ok()) {
        if panel != PanelId::Document && !workspace.layout().contains(&panel) && palette.iter().all(|(p, _)| *p != panel) {
            palette.push((panel, front));
        }
    }
    Ok((workspace, palette))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use craft::behavior::Axis;
    use craft::behavior::dock::{Action, Column, DockEdge, Node, PanelState, Target};
    use craft::ui::runtime::access::Role;
    use craft::ui::runtime::commands::CommandRef;

    use super::*;
    use crate::testing::TestApp;

    /// The dock columns at `edge` of the docked tree.
    fn columns(node: &mut Node<PanelId>, edge: DockEdge) -> Option<&mut Vec<Column<PanelId>>> {
        match node {
            Node::Dock { side, columns, .. } if *side == edge => Some(columns),
            Node::Split { first, second, .. } => columns(first, edge).or_else(|| columns(second, edge)),
            _ => None,
        }
    }

    /// Drags the nth divider (in reading order along `axis`) whose centre is at x `from` or more by `by` points.
    fn drag_divider(t: &mut TestApp, axis: Axis, from: f32, nth: usize, by: f32) {
        let tree = t.frames.tree();
        let along = |p: egui::Pos2| if axis == Axis::X { p.x } else { p.y };
        let mut bars: Vec<egui::Rect> = tree
            .keys()
            .into_iter()
            .filter(|key| tree.role_of(*key) == Some(Role::SPLITTER))
            .filter_map(|key| tree.bounds_of(key))
            .filter(|bar| (bar.height() > bar.width()) == (axis == Axis::X) && bar.center().x >= from)
            .collect();
        bars.sort_by(|a, b| along(a.center()).total_cmp(&along(b.center())));
        let at = bars.get(nth).unwrap_or_else(|| panic!("divider {nth} of {bars:?}")).center();
        let to = if axis == Axis::X { at + egui::vec2(by, 0.0) } else { at + egui::vec2(0.0, by) };
        t.ok("ui.drag", json!({"x": at.x, "y": at.y, "toX": to.x, "toY": to.y}));
        t.frame();
    }

    /// The dock's own dividers resize the sidebar, the Palette and the Layers section, and the sizes are in the kept
    /// layout.
    #[test]
    fn the_sidebars_and_the_layers_section_resize_by_their_edges() {
        let mut t = TestApp::new();
        drag_divider(&mut t, Axis::X, 0.0, 0, 70.0);
        // The Palette is at the window's end; its first divider is under Metrics, its second under Layers.
        drag_divider(&mut t, Axis::Y, 800.0, 1, 60.0);
        drag_divider(&mut t, Axis::X, 800.0, 0, -50.0);
        let kept = to_value(&t.app.view.workspace, &[]).unwrap();
        let (workspace, _) = restore(&kept).unwrap();
        let mut layout = workspace.layout().clone();
        let root = layout.root.as_mut().unwrap();
        let sidebar = columns(root, DockEdge::Start).unwrap()[0].width;
        let end = &columns(root, DockEdge::End).unwrap()[0];
        let layers = end.groups.iter().find(|group| group.panels.contains(&PanelId::Layers)).unwrap().height;
        assert!(sidebar.is_some() && end.width.is_some(), "{sidebar:?} {:?}", end.width);
        assert!(layers.is_some_and(|height| height > 230.0), "{layers:?}");
    }

    /// The sidebars' widths, the Layers section's height and a hidden Palette come back, the Palette with the panels it
    /// had; a floating panel is closed, and a panel of another version is left out without losing the rest.
    #[test]
    fn the_layout_is_kept_and_a_floating_panel_is_not() {
        let mut t = TestApp::new();
        let mut layout = t.app.view.workspace.layout().clone();
        let root = layout.root.as_mut().unwrap();
        columns(root, DockEdge::Start).unwrap()[0].width = Some(333.0);
        let end = columns(root, DockEdge::End).unwrap();
        end[0].width = Some(401.0);
        end[0].groups[1].height = Some(260.0);
        let (hidden, floated) = (t.app.view.workspace.hidden().to_vec(), t.app.view.workspace.floated().to_vec());
        t.app.view.workspace.replace(Workspace::from_parts(layout, hidden, floated));
        let float = egui::Rect::from_min_size(egui::pos2(9000.0, 9000.0), egui::vec2(300.0, 200.0));
        let actions = [Action::open(PanelId::FontInfo, None), Action::move_panel(PanelId::FontInfo, Target::float(float))];
        t.app.view.workspace.apply_all(actions, false, |p| p.permissions()).unwrap();

        let mut kept = to_value(&t.app.view.workspace, &t.app.view.palette.hidden).unwrap();
        let text = kept.to_string().replace("\"sidebar\"", "\"a panel of another version\"");
        let (workspace, _) = restore(&kept).unwrap();
        let mut layout = workspace.layout().clone();
        let root = layout.root.as_mut().unwrap();
        assert_eq!(columns(root, DockEdge::Start).unwrap()[0].width, Some(333.0));
        let end = columns(root, DockEdge::End).unwrap();
        assert_eq!((end[0].width, end[0].groups[1].height), (Some(401.0), Some(260.0)));
        assert_eq!(workspace.layout().panel_state(&PanelId::FontInfo), PanelState::Hidden, "a floating panel is closed");
        assert!(workspace.hidden().iter().all(|(p, _)| *p != PanelId::FontInfo), "and opens at its usual place");

        let (workspace, _) = restore(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_ne!(workspace.layout().panel_state(&PanelId::Layers), PanelState::Hidden);
        assert_ne!(workspace.layout().panel_state(&PanelId::Document), PanelState::Hidden);

        // Font Info opened in front of the Glyph Metrics and the Info box; moved out, it left the Info box in front.
        assert_eq!(t.app.view.workspace.layout().panel_state(&PanelId::InfoBox), PanelState::Front);
        t.app.fire(CommandRef::new("window.palette"));
        t.frame();
        kept = to_value(&t.app.view.workspace, &t.app.view.palette.hidden).unwrap();
        let mut next = TestApp::new();
        (next.app.view.workspace, next.app.view.palette.hidden) = restore(&kept).unwrap();
        next.frame();
        assert!(!crate::shell::window::palette_shows(&next.app), "a hidden Palette stays hidden");
        next.app.fire(CommandRef::new("window.palette"));
        next.frame();
        let layout = next.app.view.workspace.layout();
        assert_eq!(layout.panel_state(&PanelId::InfoBox), PanelState::Front, "the same tab is in front");
        assert_eq!(layout.panel_state(&PanelId::GlyphMetrics), PanelState::Open);
        let mut layout = next.app.view.workspace.layout().clone();
        let root = layout.root.as_mut().unwrap();
        assert_eq!(columns(root, DockEdge::End).unwrap()[0].width, Some(401.0), "at its width");

        for broken in [json!(null), json!({}), json!({"workspace": 3}), json!({"workspace": {"layout": {"root": 1, "floating": []}}})] {
            assert!(restore(&broken).is_err(), "{broken}");
        }
    }
}
