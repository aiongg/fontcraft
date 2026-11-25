//! What the canvas shows of the edited glyph, for assistive technology and `ui.elements`: its nodes, handles,
//! components and anchors as named children of the canvas, in the layer's order, with their coordinates as values and
//! whether they are selected.
//!
//! The list follows what is drawn: no nodes without View › Show Nodes, no anchors without Show Anchors, nothing outside
//! the canvas, and [`MAX_ITEMS`] things at most. It is made only on a pass whose AccessKit tree somebody reads
//! (`craft_ext::ui::runtime::access::listening`).

use craft_ext::ui::runtime::access::Drawn;
use egui::{Pos2, Rect, Vec2};
use fontcraft_engine::model::{Path, Shape};
use fontcraft_geom::{Point, Shape as _};
use fontcraft_tools::Selection;

use super::Display;
use super::camera::Camera;
use super::tokens::Tokens;
use crate::app::models::canvas::{ComponentMark, EditedGlyph};

craft::ui::words! {
    /// The names of what the canvas shows. `path` and `point` count from 1.
    pub struct AccessWords {
        node: template "fontcraftCanvasAccess.node" = "Path {path}, node {point}";
        handle: template "fontcraftCanvasAccess.handle" = "Path {path}, handle {point}";
        component: template "fontcraftCanvasAccess.component" = "Component {name}";
        anchor: template "fontcraftCanvasAccess.anchor" = "Anchor {name}";
    }
}

/// Most things listed.
pub const MAX_ITEMS: usize = 2_000;

/// The edited glyph's nodes, handles and components in the order of its shapes, then its anchors.
pub fn items<'a>(
    words: &'a AccessWords,
    glyph: &'a EditedGlyph,
    display: Display,
    selection: &'a Selection,
    camera: &'a Camera,
    tokens: &'a Tokens,
) -> impl Iterator<Item = Drawn> + 'a {
    // A component that cannot be resolved has no mark.
    let mut marks = glyph.components.iter().peekable();
    let mut paths = 0usize;
    let shapes = glyph.layer.shapes.iter().enumerate().flat_map(move |(at, shape)| {
        let mut out: Vec<Drawn> = Vec::new();
        match shape {
            Shape::Path(path) => {
                paths = paths.saturating_add(1);
                if display.nodes {
                    out.extend(points(words, path, at, paths, selection, camera, tokens));
                }
            }
            Shape::Component(placed) => {
                if let Some(mark) = marks.next_if(|mark| mark.base == placed.glyph) {
                    out.extend(component(words, mark, at, selection, camera));
                }
            }
            _ => {}
        }
        out
    });
    let anchors = glyph.layer.anchors.iter().enumerate().filter(move |_| display.anchors).map(move |(at, anchor)| Drawn {
        key: ("anchor", at, 0),
        name: words.anchor.fill(&[("name", anchor.name.as_str())]),
        value: Some(coordinates(anchor.x, anchor.y)),
        rect: around(camera.to_screen(Point::new(anchor.x, anchor.y)), tokens.anchor_size),
        selected: Some(selection.anchors.contains(&anchor.name)),
    });
    shapes.chain(anchors)
}

/// The nodes and handles of the path at `shape`, the `number`th path.
fn points<'a>(
    words: &'a AccessWords,
    path: &'a Path,
    shape: usize,
    number: usize,
    selection: &'a Selection,
    camera: &'a Camera,
    tokens: &'a Tokens,
) -> impl Iterator<Item = Drawn> + 'a {
    // A path past the cap adds nothing to the list, so no more of it is named.
    path.nodes.iter().enumerate().take(MAX_ITEMS).map(move |(at, node)| {
        let (word, size) = if node.on_curve() { (&words.node, tokens.node_size) } else { (&words.handle, tokens.handle_radius * 2.0) };
        let (path, point) = (number.to_string(), at.saturating_add(1).to_string());
        Drawn {
            key: ("point", shape, at),
            name: word.fill(&[("path", path.as_str()), ("point", point.as_str())]),
            value: Some(coordinates(node.x, node.y)),
            rect: around(camera.to_screen(node.point()), size),
            selected: Some(selection.points.contains(&(shape, at))),
        }
    })
}

/// The component at `shape`: its outline's box, or its placeholder's.
fn component(words: &AccessWords, mark: &ComponentMark, shape: usize, selection: &Selection, camera: &Camera) -> Option<Drawn> {
    let corners: Vec<Point> = match &mark.placeholder {
        Some(corners) => corners.iter().map(|[x, y]| Point::new(*x, *y)).collect(),
        None if mark.outline.elements().is_empty() => return None,
        None => {
            let b = mark.outline.bounding_box();
            vec![Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]
        }
    };
    let corners: Vec<Pos2> = corners.into_iter().map(|p| camera.to_screen(p)).collect();
    Some(Drawn {
        key: ("component", shape, 0),
        name: words.component.fill(&[("name", mark.base.as_str())]),
        value: None,
        rect: Rect::from_points(&corners),
        selected: Some(selection.shapes.contains(&shape)),
    })
}

fn around(at: Pos2, size: f32) -> Rect {
    Rect::from_center_size(at, Vec2::splat(size))
}

/// Coordinates as a value: whole numbers without a fraction.
fn coordinates(x: f64, y: f64) -> String {
    format!("{x}, {y}")
}
