//! The targets a command run from a menu or a key gets from the canvas's selection.

use std::collections::BTreeSet;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::CommandSpec;
use fontcraft_engine::params::TargetKind;
use fontcraft_tools::{Scene, Selection};
use serde_json::{Map, Value, json};

use crate::app::FontCraftApp;

/// Whether `spec`'s params doc names `param` among its parameters (not in its result, after `→`).
pub(crate) fn takes_param(spec: &CommandSpec, param: &str) -> bool {
    spec.params.split('→').next().is_some_and(|given| given.contains(param))
}

/// `command` with its targets filled from the selection, when it takes targets and was given no parameters.
pub fn with_targets(app: &mut FontCraftApp, command: CommandRef) -> CommandRef {
    let empty = command.params.is_null() || command.params.as_object().is_some_and(Map::is_empty);
    let Some(spec) = fontcraft_engine::find_command(&command.id) else { return command };
    let hinting = command.id.starts_with("hint.ps.") || command.id.starts_with("hint.tt.");
    if !empty || (spec.targets.is_empty() && !hinting) {
        return command;
    }
    super::apply::sync(app);
    let image = takes_param(spec, "image?: bool") && crate::canvas::images::is_selected(app);
    let marks = crate::canvas::hinting::ps_marks(&mut app.session, &app.view);
    let Ok(font) = app.session.font() else { return command };
    let Some(mut scene) = super::scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref()) else { return command };
    scene.ps_hints = marks;
    if let Some(params) = crate::canvas::hinting::targets(&scene, &app.tools.selection, &app.view.hinting, &command.id) {
        return CommandRef::with_params(command.id, params);
    }
    if spec.targets.is_empty() {
        return command;
    }
    let mut targets = selection_targets(&scene, &app.tools.selection, spec);
    // The selected placed image is the target of the commands that take it (`canvas::images`).
    if image {
        targets.insert("image".into(), json!(true));
    }
    CommandRef::with_params(command.id, Value::Object(targets))
}

/// The target parameters of `spec` that `selection` supplies: `glyph`, `layer`, `side` while the background is
/// edited (when the command's params doc names `side`), and each list the command takes that the selection fills.
///
/// A command whose targets include `glyphs` but not `layer` gets the edited glyph as `glyphs`. A command that takes
/// `segments` but not `points` gets the selected segments ([`selected_segments`]); one that takes `shapes` but not
/// `points` also gets the paths whose on-curve nodes are all selected, which is what a selected path is.
pub fn selection_targets(scene: &Scene, selection: &Selection, spec: &CommandSpec) -> Map<String, Value> {
    let all = selection.targets(scene);
    let takes_points = spec.targets.contains(&TargetKind::Points);
    let mut m = Map::new();
    let copy = |m: &mut Map<String, Value>, key: &str| {
        if let Some(v) = all.get(key) {
            m.insert(key.into(), v.clone());
        }
    };
    for kind in spec.targets {
        match kind {
            TargetKind::Layer => {
                copy(&mut m, "glyph");
                copy(&mut m, "layer");
                if takes_param(spec, "side?") {
                    copy(&mut m, "side");
                }
            }
            TargetKind::Points => copy(&mut m, "points"),
            TargetKind::Segments if !takes_points => {
                let segments = selected_segments(scene, selection);
                if !segments.is_empty() {
                    m.insert("segments".into(), Value::Array(segments.iter().map(|&(s, n)| json!([s, n])).collect()));
                }
            }
            TargetKind::Shapes => {
                let mut shapes: BTreeSet<usize> = selection.shapes.clone();
                if !takes_points {
                    shapes.extend(selected_paths(scene, selection));
                }
                if !shapes.is_empty() {
                    m.insert("shapes".into(), Value::Array(shapes.iter().map(|&s| json!(s)).collect()));
                }
            }
            TargetKind::Anchors => copy(&mut m, "anchors"),
            TargetKind::Guides => copy(&mut m, "guides"),
            TargetKind::Annotations => copy(&mut m, "annotations"),
            _ => {}
        }
    }
    if spec.targets.contains(&TargetKind::Glyphs) && !m.contains_key("glyph") {
        m.insert("glyphs".into(), json!([scene.glyph]));
    }
    corner_targets(scene, selection, spec.id, &mut m);
    m
}

/// The selected attachments as the commands that act on them take them: `edit.delete` as `hints`, beside the other
/// lists; Copy and Cut as `corners`, `caps`, `segmentComponents` or `brushes` (one kind, corners first), alone; the two
/// mirrors as `corners`, alone. Copy takes attachments without their nodes, Cut refuses them rather than cutting the
/// whole layer, and the mirrors flip corners in place.
fn corner_targets(scene: &Scene, selection: &Selection, id: &str, m: &mut Map<String, Value>) {
    let hints: Vec<usize> = fontcraft_tools::attachment::items(scene, selection)
        .into_iter()
        .filter_map(|item| match item {
            fontcraft_tools::Item::Hint(i) => Some(i),
            _ => None,
        })
        .collect();
    if hints.is_empty() {
        return;
    }
    match id {
        "edit.delete" => {
            m.insert("hints".into(), json!(hints));
        }
        "edit.copy" | "edit.cut" | "transform.mirrorHorizontal" | "transform.mirrorVertical" => {
            let picked = if id.starts_with("edit.") {
                fontcraft_tools::attachment::copied_kind(scene, selection)
            } else {
                let corners = fontcraft_tools::attachment::selected_corners(scene, selection);
                (!corners.is_empty()).then_some((fontcraft_tools::attachment::AttachmentKind::Corner, corners))
            };
            let Some((kind, list)) = picked else { return };
            for key in ["points", "segments", "shapes", "anchors", "guides", "annotations", "side"] {
                m.remove(key);
            }
            m.insert(kind.list_key().into(), json!(list));
        }
        _ => {}
    }
}

/// The paths of `scene` whose on-curve nodes are all in `selection`.
pub fn selected_paths(scene: &Scene, selection: &Selection) -> Vec<usize> {
    let touched: BTreeSet<usize> = selection.points.iter().map(|&(s, _)| s).collect();
    touched
        .into_iter()
        .filter(|&shape| {
            scene.path(shape).is_some_and(|path| {
                let mut on_curve = path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).peekable();
                on_curve.peek().is_some() && on_curve.all(|(i, _)| selection.points.contains(&(shape, i)))
            })
        })
        .collect()
}

/// The selected segments of `scene`, by the node that ends them: those whose two end nodes or one of whose handles
/// are in `selection`.
pub fn selected_segments(scene: &Scene, selection: &Selection) -> Vec<(usize, usize)> {
    let touched: BTreeSet<usize> = selection.points.iter().map(|&(s, _)| s).collect();
    let mut out = Vec::new();
    for shape in touched {
        let Some(path) = scene.path(shape) else { continue };
        let has = |node: usize| selection.points.contains(&(shape, node));
        for seg in fontcraft_tools::segment::segments(shape, path) {
            if (has(seg.start) && has(seg.end)) || seg.handles.iter().any(|&h| has(h)) {
                out.push((shape, seg.end));
            }
        }
    }
    out
}
