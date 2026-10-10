//! What the selection and the clicked object say about a context menu item, beyond the targets
//! it needs (`tools_glue::selection_supplies`):
//!
//! | Rule | Items |
//! |---|---|
//! | [`visible`] | Lock Path or Unlock Path, never both; a guide's Lock or Unlock, Make Global or Make Local; Guide Properties… for one editable guide; Open Corner for a selected corner node that is not an open end; Make Node First for nodes of closed paths |
//! | [`ready`] | Connect Nodes needs two open ends; Close Open Paths needs an open path |
//! | [`checked`] | Locked (the glyph), Measurement Guide |
//!
//! Menu bar items use [`ready`] too.

use std::collections::BTreeSet;

use fontcraft_model::Path;
use fontcraft_tools::{GuideScope, Scene, Selection, guide_drag};

/// The context menu item that opens the Guide Properties dialog.
pub const GUIDE_PROPERTIES: &str = guide_drag::PROPERTIES_DIALOG;

/// The shapes an item acts on: the selected shapes and the shapes of the selected points.
fn target_shapes(selection: &Selection) -> BTreeSet<usize> {
    selection.shapes.iter().copied().chain(selection.points.iter().map(|&(s, _)| s)).collect()
}

fn target_paths<'a>(scene: &Scene<'a>, selection: &Selection) -> Vec<&'a Path> {
    target_shapes(selection).into_iter().filter_map(|s| scene.path(s)).collect()
}

/// Whether node `node` of `path` is one of its two open ends.
fn open_end(path: &Path, node: usize) -> bool {
    !path.closed && path.nodes.len() > 1 && (node == 0 || node + 1 == path.nodes.len())
}

/// Whether the item shows at all.
pub fn visible(scene: &Scene, selection: &Selection, id: &str) -> bool {
    let paths = target_paths(scene, selection);
    let all_locked = !paths.is_empty() && paths.iter().all(|p| p.info.locked);
    let guides: Vec<_> = selection.guides.iter().filter_map(|&r| guide_drag::guide(scene, r).map(|g| (r, g))).collect();
    let guides_locked = !guides.is_empty() && guides.iter().all(|(_, g)| g.locked);
    match id {
        "path.lockPath" => !all_locked,
        "path.unlockPath" => all_locked,
        "guide.unlock" => guides_locked,
        _ if id.starts_with("guide.") && guides_locked && id != "guide.add" && id != "guide.addGlobal" => false,
        "guide.lock" => !guides_locked,
        "guide.makeGlobal" => guides.is_empty() || guides.iter().any(|(r, _)| r.scope == GuideScope::Local),
        "guide.makeLocal" => guides.iter().any(|(r, _)| r.scope == GuideScope::Global),
        GUIDE_PROPERTIES => matches!(guides.as_slice(), [(r, g)] if guide_drag::editable(scene, *r, g)),
        "path.openCorner" => selection
            .on_curve_points(scene)
            .iter()
            .any(|&(s, n)| scene.path(s).is_some_and(|p| p.nodes.get(n).is_some_and(|node| node.on_curve() && !node.smooth) && !open_end(p, n))),
        "path.makeNodeFirst" => {
            let nodes = selection.on_curve_points(scene);
            !nodes.is_empty() && nodes.iter().all(|&(s, _)| scene.path(s).is_some_and(|p| p.closed))
        }
        _ => true,
    }
}

/// Whether the item can run on what is selected; the reason when it cannot.
pub fn ready(scene: &Scene, selection: &Selection, id: &str) -> Result<(), String> {
    match id {
        "path.connectNodes" => {
            let ends = selection.on_curve_points(scene);
            let both_open = ends.iter().all(|&(s, n)| scene.path(s).is_some_and(|p| open_end(p, n)));
            if ends.len() == 2 && both_open { Ok(()) } else { Err("select two open ends of paths".into()) }
        }
        "path.closeOpenPaths" => {
            let open = target_paths(scene, selection).iter().any(|p| !p.closed && p.nodes.iter().filter(|n| n.on_curve()).count() >= 2);
            if open { Ok(()) } else { Err("none of the selected paths is open".into()) }
        }
        _ => Ok(()),
    }
}

/// Whether a checkable item shows its check mark; `None` for an item that is not checkable.
pub fn checked(scene: &Scene, selection: &Selection, id: &str) -> Option<bool> {
    match id {
        "glyph.toggleLocked" => Some(scene.locked),
        "guide.toggleMeasurement" => {
            let shown: Vec<bool> = selection.guides.iter().filter_map(|&r| guide_drag::guide(scene, r)).map(|g| g.show_measurement).collect();
            Some(!shown.is_empty() && shown.iter().all(|&s| s))
        }
        _ => None,
    }
}
