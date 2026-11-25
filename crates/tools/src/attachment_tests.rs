//! Attachments in the Edit View: the hit on the fitted outline of corner, cap and segment
//! components and brushes, click and `Shift`-click, the marquee leaving them out, and the
//! selection keeping them through an edit.

use fontcraft_geom::{ParamCurve, Point};
use fontcraft_model::attach::{Attachment, CapHint, SegmentHint};
use fontcraft_model::corner::CornerHint;
use fontcraft_model::{Anchor, Font, Node, NodeKind, Path, Shape};

use crate::attachment::{AttachmentKind, copied_kind, selected, selected_corners};
use crate::event::PointerKind::{Down, Drag, Up};
use crate::select::SelectTool;
use crate::*;

fn on(x: f64, y: f64) -> Node {
    Node::new(x, y, NodeKind::Line)
}

/// Glyph `A`: the rectangle (100, 0) (400, 0) (400, 500) (100, 500) with `_corner.one` on its
/// bottom-left node (hint 0) and its bottom-right node (hint 1). Hint node indices count in file
/// order, with the start node last.
fn corner_font() -> Font {
    let mut font = Font::new("Test");
    font.add_glyph("_corner.one", Vec::new(), 0.0).unwrap();
    let corner = Path::new(false, vec![on(0.0, 50.0), on(-50.0, 50.0), on(-50.0, 0.0), on(25.0, 0.0)]);
    font.glyph_mut("_corner.one").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(corner));
    {
        let mut glyph = font.glyph_mut("A").unwrap();
        let layer = glyph.layer_mut("m01").unwrap();
        layer.shapes.push(Shape::Path(Path::new(true, vec![on(100.0, 0.0), on(400.0, 0.0), on(400.0, 500.0), on(100.0, 500.0)])));
        layer.info.hints.push(CornerHint::new("_corner.one", 0, 3).to_hint());
        layer.info.hints.push(CornerHint::new("_corner.one", 0, 0).to_hint());
    }
    font
}

/// Runs `kinds` at `p` through the Select tool and applies the selections it asks for; returns
/// every action.
fn gesture(tool: &mut SelectTool, scene: &Scene, selection: &mut Selection, kinds: &[(PointerKind, Point)], mods: Modifiers) -> Vec<Action> {
    let mut all = Vec::new();
    for &(kind, p) in kinds {
        let actions = {
            let cx = Context { scene, view: View::default(), selection: &*selection, settings: ToolSettings::default() };
            tool.pointer(&cx, &PointerEvent::new(kind, p.x, p.y, mods)).actions
        };
        for a in &actions {
            if let Action::Select { selection: s } = a {
                *selection = s.clone();
            }
        }
        all.extend(actions);
    }
    all
}

/// A click on a corner's fitted outline selects it, `Shift`-click adds a second; the host node
/// and the stem far from the corner are still a node and a segment. A drag from a corner moves
/// nothing, and a marquee over the glyph takes nodes, not corners.
#[test]
fn corners_are_selected_by_a_click_on_their_fitted_outline() {
    let font = corner_font();
    let scene = Scene::from_font(&font, "A", None).unwrap();
    let hints: Vec<(usize, AttachmentKind)> = scene.attachments.iter().map(|m| (m.hint, m.kind)).collect();
    assert_eq!(hints, vec![(0, AttachmentKind::Corner), (1, AttachmentKind::Corner)]);
    let view = View::default();
    // The bottom-left corner sticks out to x = 50, half a stem away from the path.
    assert_eq!(hit_test_with_corners(&scene, &view, Point::new(50.0, 25.0), 6.0).map(|h| h.hit), Some(Hit::Attachment { hint: 0 }));
    assert_eq!(hit_test_with_corners(&scene, &view, Point::new(100.0, 0.0), 6.0).map(|h| h.hit), Some(Hit::Node { shape: 0, node: 0 }));
    assert!(matches!(hit_test_with_corners(&scene, &view, Point::new(100.0, 250.0), 6.0).map(|h| h.hit), Some(Hit::Segment { .. })));
    let second = scene.attachments[1].outline.segments().next().unwrap().eval(0.5);
    assert_eq!(hit_test_with_corners(&scene, &view, second, 6.0).map(|h| h.hit), Some(Hit::Attachment { hint: 1 }));

    let mut tool = SelectTool::default();
    let mut selection = Selection::new();
    let first = Point::new(50.0, 25.0);
    gesture(&mut tool, &scene, &mut selection, &[(Down, first), (Up, first)], Modifiers::NONE);
    assert_eq!(selection, Selection::from_items([Item::Hint(0)]));
    gesture(&mut tool, &scene, &mut selection, &[(Down, second), (Up, second)], Modifiers::SHIFT);
    assert_eq!(selection, Selection::from_items([Item::Hint(0), Item::Hint(1)]));
    assert_eq!(selected_corners(&scene, &selection), vec![0, 1]);
    assert_eq!(selection.targets(&scene).get("hints"), Some(&serde_json::json!([0, 1])));

    let actions =
        gesture(&mut tool, &scene, &mut selection, &[(Down, first), (Drag, Point::new(90.0, 80.0)), (Up, Point::new(90.0, 80.0))], Modifiers::NONE);
    assert!(!actions.iter().any(|a| matches!(a, Action::Begin { .. } | Action::Preview { .. } | Action::Commit)), "{actions:?}");

    let (a, b) = (Point::new(-200.0, -200.0), Point::new(600.0, 700.0));
    let mut marquee = Selection::new();
    gesture(&mut tool, &scene, &mut marquee, &[(Down, a), (Drag, b), (Up, b)], Modifiers::NONE);
    assert_eq!(marquee.points.len(), 4);
    assert!(marquee.hints.is_empty(), "the marquee leaves corners out");
}

/// Only the Select tool sees corners: where a corner's fitted outline lies on the bottom stem,
/// the plain hit (Pen, Lasso and the other tools) is still the segment. Within the hit tolerance
/// of the bottom-left node (x < 107) the node wins either way.
#[test]
fn other_tools_hit_the_segment_under_a_corner() {
    let font = corner_font();
    let scene = Scene::from_font(&font, "A", None).unwrap();
    let view = View::default();
    let on_stem: Vec<Point> = (108..200).map(|x| Point::new(f64::from(x), 0.0)).collect();
    assert!(
        on_stem.iter().any(|&p| hit_test_with_corners(&scene, &view, p, 6.0).map(|h| h.hit) == Some(Hit::Attachment { hint: 0 })),
        "the corner overlaps the stem somewhere"
    );
    for p in on_stem {
        assert!(matches!(hit_test(&scene, &view, p, 6.0).map(|h| h.hit), Some(Hit::Segment { .. })), "{p:?}");
    }
}

/// Selected corners survive an edit while the layer has them; a removed one drops out.
#[test]
fn selected_corners_stay_while_the_layer_has_them() {
    let mut font = corner_font();
    let mut selection = Selection::from_items([Item::Hint(0), Item::Hint(1)]);
    selection.retain_valid(&Scene::from_font(&font, "A", None).unwrap());
    assert_eq!(selection.hints.len(), 2);
    font.glyph_mut("A").unwrap().layer_mut("m01").unwrap().info.hints.truncate(1);
    selection.retain_valid(&Scene::from_font(&font, "A", None).unwrap());
    assert_eq!(selection, Selection::from_items([Item::Hint(0)]));
}

/// Glyph `A`: the rectangle (100, 0) (400, 0) (400, 500) (100, 500) with `_cap.flag` (Fit on) on
/// its top segment (hint 0), `_segment.wave` on its bottom segment (hint 1) and `_brush.bar` on
/// its right segment (hint 2). Hint node indices count in file order, with the start node last.
fn attachment_font() -> Font {
    let mut font = Font::new("Test");
    let glyphs: [(&str, Path, f64, Vec<Anchor>); 3] = [
        (
            "_cap.flag",
            Path::new(false, vec![on(0.0, 20.0), on(-40.0, -30.0), on(100.0, -30.0), on(100.0, 20.0)]),
            100.0,
            vec![Anchor::new("origin", 0.0, 0.0)],
        ),
        (
            "_segment.wave",
            Path::new(false, vec![on(0.0, 0.0), on(100.0, 60.0), on(200.0, 0.0)]),
            200.0,
            vec![Anchor::new("start", 0.0, 0.0), Anchor::new("end", 200.0, 0.0)],
        ),
        (
            "_brush.bar",
            Path::new(true, vec![on(0.0, -20.0), on(100.0, -20.0), on(100.0, 20.0), on(0.0, 20.0)]),
            100.0,
            vec![Anchor::new("start", 0.0, 0.0), Anchor::new("end", 100.0, 0.0)],
        ),
    ];
    for (name, path, width, anchors) in glyphs {
        font.add_glyph(name, Vec::new(), width).unwrap();
        let mut g = font.glyph_mut(name).unwrap();
        let l = g.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Path(path));
        l.anchors = anchors;
    }
    {
        let mut glyph = font.glyph_mut("A").unwrap();
        let layer = glyph.layer_mut("m01").unwrap();
        layer.shapes.push(Shape::Path(Path::new(true, vec![on(100.0, 0.0), on(400.0, 0.0), on(400.0, 500.0), on(100.0, 500.0)])));
        let mut cap = CapHint::new("_cap.flag", 0, 1);
        cap.fit = true;
        layer.info.hints.push(cap.to_hint());
        layer.info.hints.push(Attachment::Segment(SegmentHint::new("_segment.wave", 0, 0)).to_hint());
        layer.info.hints.push(Attachment::Brush(SegmentHint::new("_brush.bar", 0, 1)).to_hint());
    }
    font
}

/// A point on the outline of attachment `hint` that a click there hits.
fn point_on(scene: &Scene, hint: usize) -> Point {
    let mark = scene.attachments.iter().find(|m| m.hint == hint).unwrap_or_else(|| panic!("attachment {hint} is fitted"));
    let view = View::default();
    mark.outline
        .segments()
        .flat_map(|seg| [0.25, 0.5, 0.75].map(|t| seg.eval(t)))
        .find(|&p| hit_test_with_corners(scene, &view, p, 6.0).map(|h| h.hit) == Some(Hit::Attachment { hint }))
        .unwrap_or_else(|| panic!("no point of attachment {hint}'s outline hits it"))
}

/// Caps, segment components and brushes are marked and selected as corners are: a click on the
/// fitted outline selects one, `Shift`-click adds the others, and the selection hands each kind to
/// `edit.copy` under its own key (`cap.highlightAndSelection`, `segment.highlightAndSelection`,
/// `brush.highlightAndSelection`). The brush is marked along the path it adds, not inside its host.
#[test]
fn caps_segments_and_brushes_are_marked_and_selected_by_a_click() {
    let font = attachment_font();
    let scene = Scene::from_font(&font, "A", None).unwrap();
    let kinds: Vec<(usize, AttachmentKind)> = scene.attachments.iter().map(|m| (m.hint, m.kind)).collect();
    assert_eq!(kinds, vec![(0, AttachmentKind::Cap), (1, AttachmentKind::Segment), (2, AttachmentKind::Brush)]);
    // The cap's band is its own outline, not the rectangle's two sides below it.
    let cap = scene.attachments.iter().find(|m| m.kind == AttachmentKind::Cap).unwrap();
    let ends: Vec<(f64, f64)> = cap.band.elements().iter().filter_map(|e| e.end_point()).map(|p| (p.x, p.y)).collect();
    assert_eq!(ends, [(400.0, 480.0), (520.0, 530.0), (100.0, 530.0), (100.0, 480.0)]);
    let brush = scene.attachments.iter().find(|m| m.kind == AttachmentKind::Brush).unwrap();
    let ends: Vec<Point> = brush.band.elements().iter().filter_map(|e| e.end_point()).collect();
    assert!(ends.iter().any(|p| (p.x - 420.0).abs() < 1.0) && ends.iter().any(|p| (p.x - 380.0).abs() < 1.0), "{ends:?}");

    let (cap, segment, brush) = (point_on(&scene, 0), point_on(&scene, 1), point_on(&scene, 2));
    let mut tool = SelectTool::default();
    let mut selection = Selection::new();
    gesture(&mut tool, &scene, &mut selection, &[(Down, segment), (Up, segment)], Modifiers::NONE);
    assert_eq!(selection, Selection::from_items([Item::Hint(1)]));
    assert_eq!(copied_kind(&scene, &selection), Some((AttachmentKind::Segment, vec![1])));
    gesture(&mut tool, &scene, &mut selection, &[(Down, cap), (Up, cap)], Modifiers::SHIFT);
    gesture(&mut tool, &scene, &mut selection, &[(Down, brush), (Up, brush)], Modifiers::SHIFT);
    assert_eq!(selection, Selection::from_items([Item::Hint(0), Item::Hint(1), Item::Hint(2)]));
    assert_eq!(selected(&scene, &selection, AttachmentKind::Brush), vec![2]);
    assert_eq!(copied_kind(&scene, &selection), Some((AttachmentKind::Cap, vec![0])));
    assert_eq!(selection.targets(&scene).get("hints"), Some(&serde_json::json!([0, 1, 2])));
    // A second Shift-click takes the brush out again.
    gesture(&mut tool, &scene, &mut selection, &[(Down, brush), (Up, brush)], Modifiers::SHIFT);
    assert_eq!(selection, Selection::from_items([Item::Hint(0), Item::Hint(1)]));
    assert!(selected_corners(&scene, &selection).is_empty());
}
