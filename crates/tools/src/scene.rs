//! What a tool sees of the document: one layer and what is drawn around it.

use std::borrow::Cow;

use fontcraft_geom::{BezPath, CubicBez, ParamCurve, PathEl, Point, QuadBez};
use fontcraft_model::{Font, Guide, Layer, LayerSide, Node, Path, Shape, metric};

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
    /// The side being edited, for the commands' `side` parameter.
    pub side: LayerSide,
    /// The edited side as a layer: for the background, [`side_view`] of the layer.
    pub layer: Cow<'a, Layer>,
    /// The guides of the layer's master.
    pub global_guides: &'a [Guide],
    /// The global guides, by index, whose filter leaves out this glyph. They are not drawn, so
    /// they are not hit, snapped to or stepped through either.
    pub hidden_global_guides: Vec<usize>,
    /// Horizontal metric lines (baseline, x-height, cap height, ascender, descender), in font units.
    pub metric_lines: Vec<f64>,
    /// The glyph is locked: points can be selected but not moved.
    pub locked: bool,
    /// Path focus: when set, only these shapes can be hit and selected.
    pub focus: Option<Vec<usize>>,
    /// The background and the guides are showing (`view.showBackground`, `view.showGuides`);
    /// a dragged point snaps only to what is showing.
    pub background_shown: bool,
    pub guides_shown: bool,
    /// The `.glyphs` format version whose conventions the stroke positions follow (3 unless the font came from a version 4 file).
    pub glyphs_version: u32,
    /// The smart handles of the selected smart components, resolved by the caller.
    pub smart_handles: Vec<SmartHandle>,
    /// The selected smart components and their Width and Height properties, resolved by the
    /// caller (`smart.widthHeightFollowBoundingBox`).
    pub smart_sizes: Vec<SmartSize>,
    components: Vec<ComponentGeom>,
    component_points: usize,
}

/// A smart handle (`smart.handlesFromPropertyAnchors`): the anchor named after a property of a
/// selected smart component, where it lies now and at the property's two ends, in the layer's
/// coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SmartHandle {
    /// The component's index among the layer's shapes.
    pub shape: usize,
    pub property: String,
    pub at: Point,
    pub at_min: Point,
    pub at_max: Point,
    pub min: f64,
    pub max: f64,
}

/// A selected smart component: the bounding box knobs set its Width and Height properties
/// instead of scaling it, and do nothing on an axis without such a property.
#[derive(Clone, Debug, PartialEq)]
pub struct SmartSize {
    /// The component's index among the layer's shapes.
    pub shape: usize,
    pub width: Option<SmartAxis>,
    pub height: Option<SmartAxis>,
}

/// One size property of a smart component: its range and the component's extent at each end.
#[derive(Clone, Debug, PartialEq)]
pub struct SmartAxis {
    pub property: String,
    pub min: f64,
    pub max: f64,
    pub size_at_min: f64,
    pub size_at_max: f64,
}

impl SmartAxis {
    /// The value at which the component's extent is `size`, read off the line through the two
    /// ends, not clamped to the range.
    pub fn value_for(&self, size: f64) -> Option<f64> {
        let span = self.size_at_max - self.size_at_min;
        if span.is_nan() || span.abs() <= 1e-9 {
            return None;
        }
        let v = self.min + (size - self.size_at_min) / span * (self.max - self.min);
        v.is_finite().then_some(v)
    }
}

impl SmartHandle {
    /// The value whose handle position is nearest `p`: `p` projected on the line from the
    /// minimum's position to the maximum's, not clamped to the range. `None` when the two
    /// positions coincide or the result is not finite.
    pub fn value_at(&self, p: Point) -> Option<f64> {
        let (d, q) = (self.at_max - self.at_min, p - self.at_min);
        let len = d.hypot2();
        if len.is_nan() || len <= 1e-12 {
            return None;
        }
        let v = self.min + (q.dot(d) / len) * (self.max - self.min);
        v.is_finite().then_some(v)
    }
}

impl<'a> Scene<'a> {
    /// A scene of a bare layer: no component outlines, no metric lines, no global guides.
    pub fn new(glyph: impl Into<String>, layer_id: impl Into<String>, layer: &'a Layer) -> Self {
        Self {
            glyph: glyph.into(),
            layer_id: layer_id.into(),
            side: LayerSide::Foreground,
            layer: Cow::Borrowed(layer),
            global_guides: &[],
            hidden_global_guides: Vec::new(),
            metric_lines: Vec::new(),
            locked: false,
            focus: None,
            background_shown: true,
            guides_shown: true,
            glyphs_version: 3,
            components: Vec::new(),
            smart_handles: Vec::new(),
            smart_sizes: Vec::new(),
            component_points: 0,
        }
    }

    /// The scene of a glyph's layer in `font`: the layer by id, or the first master's layer when
    /// `layer_id` is `None`. `None` when the glyph or the layer does not exist.
    pub fn from_font(font: &'a Font, glyph: &str, layer_id: Option<&str>) -> Option<Self> {
        Self::from_font_side(font, glyph, layer_id, LayerSide::Foreground)
    }

    /// [`Scene::from_font`] for one side of the layer.
    pub fn from_font_side(font: &'a Font, glyph: &str, layer_id: Option<&str>, side: LayerSide) -> Option<Self> {
        let g = font.glyph(glyph)?;
        let layer: &'a Layer = match layer_id {
            Some(id) => g.layers.iter().find(|l| l.id == id)?,
            None => g.layer(&font.masters.first()?.id)?,
        };
        let mut scene = Scene::new(g.name.clone(), layer.id.clone(), layer);
        scene.side = side;
        scene.layer = side_view(layer, side);
        scene.locked = g.info.locked;
        scene.glyphs_version = font.source.glyphs_version.unwrap_or(3);
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
        let mut components = Vec::new();
        for (i, shape) in scene.layer.shapes.iter().enumerate() {
            let Shape::Component(c) = shape else { continue };
            let Ok(outline) = font.component_outline(c, &layer.master) else { continue };
            components.push((i, outline, c.info.locked));
        }
        for (i, outline, locked) in components {
            scene.add_component(i, &outline, locked);
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

    /// The command target of the edited layer: `glyph`, `layer`, and `side` when it is the
    /// background. Commands add their own parameters to it.
    pub fn target(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        m.insert("glyph".into(), self.glyph.as_str().into());
        m.insert("layer".into(), self.layer_id.as_str().into());
        if self.side == LayerSide::Background {
            m.insert("side".into(), self.side.as_str().into());
        }
        m
    }

    /// The resolved components, in the order they were added.
    pub fn components(&self) -> &[ComponentGeom] {
        &self.components
    }

    /// The global guides that show in this glyph, with their indices.
    pub fn shown_global_guides(&self) -> impl Iterator<Item = (usize, &'a Guide)> + '_ {
        self.global_guides.iter().enumerate().filter(|(i, _)| !self.hidden_global_guides.contains(i))
    }

    /// The layer's own guides.
    pub fn local_guides(&self) -> &[Guide] {
        &self.layer.info.guides
    }

    /// Whether the shape at `shape` can be hit and selected (path focus).
    pub fn shape_editable(&self, shape: usize) -> bool {
        self.focus.as_ref().is_none_or(|f| f.contains(&shape))
    }

    /// The path at shape index `shape`.
    pub fn path(&self, shape: usize) -> Option<&Path> {
        match self.layer.shapes.get(shape)? {
            Shape::Path(p) => Some(p),
            _ => None,
        }
    }

    /// The node `[shape, node]`.
    pub fn node(&self, shape: usize, node: usize) -> Option<&Node> {
        self.path(shape)?.nodes.get(node)
    }

    /// The paths that can be hit and selected, with their shape indices.
    pub fn paths(&self) -> impl Iterator<Item = (usize, &Path)> + '_ {
        self.layer.shapes.iter().enumerate().filter_map(move |(i, s)| match s {
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

/// `layer` as seen from `side`: the layer itself for the foreground; for the background, a copy
/// whose shapes, anchors, guides, hints and annotations are the background's and whose background
/// holds the foreground's. Addresses into the copy are addresses into the background, and what
/// the tools treat as "the background" (snapping) is the foreground.
pub fn side_view(layer: &Layer, side: LayerSide) -> Cow<'_, Layer> {
    if side == LayerSide::Foreground {
        return Cow::Borrowed(layer);
    }
    let mut view = layer.clone();
    let info = &mut *view.info;
    let mut b = info.background.take().unwrap_or_default();
    std::mem::swap(&mut view.shapes, &mut b.shapes);
    std::mem::swap(&mut view.anchors, &mut b.anchors);
    std::mem::swap(&mut info.guides, &mut b.guides);
    std::mem::swap(&mut info.hints, &mut b.hints);
    std::mem::swap(&mut info.annotations, &mut b.annotations);
    info.background = (!b.is_empty()).then_some(b);
    Cow::Owned(view)
}
