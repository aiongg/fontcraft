//! What the selection says about a context menu item, beyond the targets it needs ([`supplies`]):
//!
//! | Rule | Items |
//! |---|---|
//! | [`visible`] | Lock Path or Unlock Path, never both; a guide's Lock or Unlock, Make Global or Make Local; Guide Properties… for one editable guide; Open Corner for a selected corner node that is not an open end; Make Node First for nodes of closed paths; Decompose Corner on a node that holds a corner component |
//! | [`ready`] | Connect Nodes needs two open ends; Close Open Paths needs an open path |
//! | [`checked`] | Locked (the glyph), Measurement Guide |

use std::collections::BTreeSet;

use fontcraft_engine::CommandSpec;
use fontcraft_engine::model::corner::{CornerTarget, corner_hint, is_corner};
use fontcraft_engine::model::{Layer, Path, Shape};
use fontcraft_engine::params::TargetKind;
use fontcraft_tools::{GuideScope, Scene, Selection, guide_drag};

use crate::canvas::tools::targets::selection_targets;

/// The context menu item that opens the Guide Properties dialog.
pub const GUIDE_PROPERTIES: &str = guide_drag::PROPERTIES_DIALOG;

/// Commands that take selection targets but act on the whole layer when given none (their params docs say so), so
/// they stay enabled without a selection.
const WHOLE_LAYER_WITHOUT_TARGETS: &[&str] = &["edit.copy", "edit.cut", "path.expandOutline", "path.expandOutlineForAllMasters"];

/// The target lists a selection can fill, their parameter keys, and what to select for them.
const OBJECT_TARGETS: &[(TargetKind, &str, &str)] = &[
    (TargetKind::Points, "points", "nodes"),
    (TargetKind::Segments, "segments", "segments"),
    (TargetKind::Shapes, "shapes", "paths or components"),
    (TargetKind::Anchors, "anchors", "anchors"),
    (TargetKind::Guides, "guides", "guides"),
    (TargetKind::Annotations, "annotations", "annotations"),
];

/// What to select for `spec` (`nodes`, `paths or components`), when it acts on selected objects; `None` for a command
/// that takes no target lists, that takes `glyphs` (it then acts on whole glyphs), or that acts on the whole layer
/// without targets.
pub fn objects_wanted(spec: &CommandSpec) -> Option<String> {
    let wanted: Vec<&(TargetKind, &str, &str)> = OBJECT_TARGETS.iter().filter(|(k, _, _)| spec.targets.contains(k)).collect();
    if wanted.is_empty() || spec.targets.contains(&TargetKind::Glyphs) || WHOLE_LAYER_WITHOUT_TARGETS.contains(&spec.id) {
        return None;
    }
    Some(match wanted.as_slice() {
        [one] => one.2.to_string(),
        [a, b] => format!("{} or {}", a.2, b.2),
        _ => "something".to_string(),
    })
}

/// Whether the selection gives `spec` what it acts on; the error says what to select. A command that needs no
/// objects ([`objects_wanted`]) needs no selection.
pub fn supplies(scene: &Scene, selection: &Selection, spec: &CommandSpec) -> Result<(), String> {
    let Some(what) = objects_wanted(spec) else { return Ok(()) };
    let m = selection_targets(scene, selection, spec);
    if !OBJECT_TARGETS.iter().any(|(kind, key, _)| spec.targets.contains(kind) && m.contains_key(*key)) {
        return Err(format!("select {what} first"));
    }
    ready(scene, selection, spec.id)
}

/// The shapes an item acts on: the selected shapes and the shapes of the selected points.
fn target_shapes(selection: &Selection) -> BTreeSet<usize> {
    selection.shapes.iter().copied().chain(selection.points.iter().map(|&(s, _)| s)).collect()
}

fn target_paths<'s>(scene: &'s Scene<'_>, selection: &Selection) -> Vec<&'s Path> {
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
        "cornerComponent.decompose" => !selected_corners(scene, selection).is_empty(),
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

/// The corner components on the selected nodes, by hint index.
pub fn selected_corners(scene: &Scene, selection: &Selection) -> Vec<usize> {
    corners_on_nodes(&scene.layer, &selection.points)
}

/// The corner component hints of `layer` that sit on one of `points`, by hint index.
fn corners_on_nodes(layer: &Layer, points: &BTreeSet<(usize, usize)>) -> Vec<usize> {
    layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter(|(_, h)| is_corner(h))
        .filter_map(|(i, h)| {
            let corner = corner_hint(h).ok().flatten()?;
            let CornerTarget::Node { shape, node } = corner.target else { return None };
            let Some(Shape::Path(path)) = layer.shapes.get(shape) else { return None };
            // The hint counts a closed path's nodes with the start node last.
            let len = path.nodes.len();
            let model = if path.closed && len > 0 { (node + 1) % len } else { node };
            points.contains(&(shape, model)).then_some(i)
        })
        .collect()
}
