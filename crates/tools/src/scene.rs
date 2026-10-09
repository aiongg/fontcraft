//! What a tool sees of the document: one layer and what is drawn around it.

use fontcraft_geom::{BezPath, CubicBez, ParamCurve, PathEl, Point, QuadBez};
use fontcraft_model::{Font, Guide, Layer, Node, Path, Shape, metric};

use crate::MAX_COMPONENT_POINTS;
use crate::geometry::{bounds, usable, winding};

/// Points sampled along each curve of a component's outline for inside tests.
const CURVE_SAMPLES: usize = 8;

/// A component's outline, resolved by the caller, in the layer's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentGeom {
    /// The component's index among the layer's shapes.
    pub shape: usize,
    pub locked: bool,
    /// On-curve points of the outline (snap targets).
    pub nodes: Vec<Point>,
    /// Off-curve points of the outline (snap targets).
    pub handles: Vec<Point>,
    polygons: Vec<Vec<Point>>,
    bounds: Option<(Point, Point)>,
}

impl ComponentGeom {
    /// Whether `p` is inside the component's outline (nonzero rule).
    pub fn contains(&self, p: Point) -> bool {
        match self.bounds {
            Some((lo, hi)) if (lo.x..=hi.x).contains(&p.x) && (lo.y..=hi.y).contains(&p.y) => winding(p, &self.polygons) != 0,
            _ => false,
        }
    }
}

/// One layer as the tools see it, with what is drawn around it.
#[derive(Clone, Debug)]
pub struct Scene<'a> {
    /// The glyph's name, for the commands' `glyph` parameter.
    pub glyph: String,
    /// The layer's id, for the commands' `layer` parameter.
    pub layer_id: String,
    pub layer: &'a Layer,
    /// The guides of the layer's master.
    pub global_guides: &'a [Guide],
    /// Horizontal metric lines (baseline, x-height, cap height, ascender, descender), in font units.
    pub metric_lines: Vec<f64>,
    /// The glyph is locked: points can be selected but not moved.
    pub locked: bool,
    /// Path focus: when set, only these shapes can be hit and selected.
    pub focus: Option<Vec<usize>>,
    components: Vec<ComponentGeom>,
    component_points: usize,
}

impl<'a> Scene<'a> {
    /// A scene of a bare layer: no component outlines, no metric lines, no global guides.
    pub fn new(glyph: impl Into<String>, layer_id: impl Into<String>, layer: &'a Layer) -> Self {
        Self {
            glyph: glyph.into(),
            layer_id: layer_id.into(),
            layer,
            global_guides: &[],
            metric_lines: Vec::new(),
            locked: false,
            focus: None,
            components: Vec::new(),
            component_points: 0,
        }
    }

    /// The scene of a glyph's layer in `font`: the layer by id, or the first master's layer when
    /// `layer_id` is `None`. `None` when the glyph or the layer does not exist.
    pub fn from_font(font: &'a Font, glyph: &str, layer_id: Option<&str>) -> Option<Self> {
        let g = font.glyph(glyph)?;
        let layer: &'a Layer = match layer_id {
            Some(id) => g.layers.iter().find(|l| l.id == id)?,
            None => g.layer(&font.masters.first()?.id)?,
        };
        let mut scene = Scene::new(g.name.clone(), layer.id.clone(), layer);
        scene.locked = g.info.locked;
        if let Ok(master) = font.master(&layer.master) {
            scene.global_guides = &master.guides;
            let mut lines = vec![0.0];
            for kind in [metric::DESCENDER, metric::X_HEIGHT, metric::CAP_HEIGHT, metric::ASCENDER] {
                if let Some(v) = font.metric(master, kind)
                    && v.pos.is_finite()
                {
                    lines.push(v.pos);
                }
            }
            scene.metric_lines = lines;
        }
        for (i, shape) in layer.shapes.iter().enumerate() {
            let Shape::Component(c) = shape else { continue };
            let Ok(mut outline) = font.outline(&c.glyph, &layer.master) else { continue };
            outline.apply_affine(c.affine());
            scene.add_component(i, &outline, c.info.locked);
        }
        Some(scene)
    }

    /// Adds the outline of the component at shape index `shape`, already placed in the layer.
    /// Outlines with unusable coordinates are left out; past [`MAX_COMPONENT_POINTS`] sampled
    /// points in all, the rest of the outlines are cut off.
    pub fn add_component(&mut self, shape: usize, outline: &BezPath, locked: bool) {
        let mut geom = ComponentGeom { shape, locked, nodes: Vec::new(), handles: Vec::new(), polygons: Vec::new(), bounds: None };
        let mut poly: Vec<Point> = Vec::new();
        let mut last = Point::ZERO;
        let mut budget = MAX_COMPONENT_POINTS.saturating_sub(self.component_points);
        for el in outline.elements() {
            if budget == 0 {
                break;
            }
            let before = poly.len();
            match *el {
                PathEl::MoveTo(p) => {
                    if poly.len() > 2 {
                        geom.polygons.push(std::mem::take(&mut poly));
                    }
                    poly.clear();
                    poly.push(p);
                    geom.nodes.push(p);
                    last = p;
                }
                PathEl::LineTo(p) => {
                    poly.push(p);
                    geom.nodes.push(p);
                    last = p;
                }
                PathEl::QuadTo(c, p) => {
                    let q = QuadBez::new(last, c, p);
                    poly.extend((1..=CURVE_SAMPLES).map(|k| q.eval(k as f64 / CURVE_SAMPLES as f64)));
                    geom.handles.push(c);
                    geom.nodes.push(p);
                    last = p;
                }
                PathEl::CurveTo(a, b, p) => {
                    let cb = CubicBez::new(last, a, b, p);
                    poly.extend((1..=CURVE_SAMPLES).map(|k| cb.eval(k as f64 / CURVE_SAMPLES as f64)));
                    geom.handles.push(a);
                    geom.handles.push(b);
                    geom.nodes.push(p);
                    last = p;
                }
                PathEl::ClosePath => {
                    if poly.len() > 2 {
                        geom.polygons.push(std::mem::take(&mut poly));
                    }
                    poly.clear();
                }
            }
            budget = budget.saturating_sub(poly.len().saturating_sub(before).max(1));
        }
        if poly.len() > 2 {
            geom.polygons.push(poly);
        }
        let all_usable = geom.polygons.iter().flatten().chain(&geom.nodes).chain(&geom.handles).all(|p| usable(*p));
        if !all_usable {
            return;
        }
        let used = geom.polygons.iter().map(Vec::len).sum::<usize>() + geom.nodes.len() + geom.handles.len();
        self.component_points = self.component_points.saturating_add(used);
        geom.bounds = bounds(geom.polygons.iter().flatten().copied());
        self.components.retain(|c| c.shape != shape);
        self.components.push(geom);
    }

    /// The resolved components, in the order they were added.
    pub fn components(&self) -> &[ComponentGeom] {
        &self.components
    }

    /// The layer's own guides.
    pub fn local_guides(&self) -> &'a [Guide] {
        let layer: &'a Layer = self.layer;
        &layer.info.guides
    }

    /// Whether the shape at `shape` can be hit and selected (path focus).
    pub fn shape_editable(&self, shape: usize) -> bool {
        self.focus.as_ref().is_none_or(|f| f.contains(&shape))
    }

    /// The path at shape index `shape`.
    pub fn path(&self, shape: usize) -> Option<&'a Path> {
        let layer: &'a Layer = self.layer;
        match layer.shapes.get(shape)? {
            Shape::Path(p) => Some(p),
            _ => None,
        }
    }

    /// The node `[shape, node]`.
    pub fn node(&self, shape: usize, node: usize) -> Option<&'a Node> {
        self.path(shape)?.nodes.get(node)
    }

    /// The paths that can be hit and selected, with their shape indices.
    pub fn paths(&self) -> impl Iterator<Item = (usize, &'a Path)> + '_ {
        let layer: &'a Layer = self.layer;
        layer.shapes.iter().enumerate().filter_map(move |(i, s)| match s {
            Shape::Path(p) if self.shape_editable(i) => Some((i, p)),
            _ => None,
        })
    }

    /// Whether `[shape, node]` is an on-curve node at an end of an open path.
    pub fn is_open_end(&self, shape: usize, node: usize) -> bool {
        let Some(path) = self.path(shape) else { return false };
        let last = path.nodes.len().saturating_sub(1);
        !path.closed && path.nodes.get(node).is_some_and(Node::on_curve) && (node == 0 || node == last)
    }
}
