//! The Edit View selection, and the target object commands take (decision 0004, "How commands
//! name their targets").
//!
//! The selection lives in the shell's UI state; the engine never stores it. Its lists use the
//! same addresses as command targets: points as `[shape, node]`, shapes by index, anchors by
//! name, guides as `{scope, index}`, annotations by index, PostScript hints and corner components
//! by their position in the layer's hint list. Extra nodes, which only the TrueType Instructor selects, are
//! `[shape, node, shape, node]`: the two crossing segments by the nodes they end at
//! ([`crate::extra_nodes`]).

use std::collections::BTreeSet;

use fontcraft_geom::Point;
use fontcraft_model::{NodeKind, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::MAX_SELECTION_LIST;
use crate::attachment::AttachmentKind;
use crate::geometry::point_in_polygon;
use crate::scene::Scene;

/// Whether a guide belongs to the layer or to its master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideScope {
    Local,
    Global,
}

impl GuideScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            GuideScope::Local => "local",
            GuideScope::Global => "global",
        }
    }
}

/// A guide's address: local guides are indexed in the layer, global ones in its master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GuideRef {
    pub scope: GuideScope,
    pub index: usize,
}

/// One selectable thing.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    /// A node or a handle, as `(shape, node)`.
    Point(usize, usize),
    /// A path or a component, by index.
    Shape(usize),
    Anchor(String),
    Guide(GuideRef),
    Annotation(usize),
    /// A PostScript hint or a corner component ([`crate::attachment`]), by its position in the
    /// layer's hint list.
    Hint(usize),
    /// An extra node, as `[shape, node, shape, node]` ([`crate::extra_nodes::ExtraNode::key`]).
    Intersection([usize; 4]),
}

/// How a list of items changes a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectMode {
    #[default]
    Replace,
    Add,
    Remove,
    Toggle,
}

/// The selection in one layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub points: BTreeSet<(usize, usize)>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub shapes: BTreeSet<usize>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub anchors: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub guides: BTreeSet<GuideRef>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub annotations: BTreeSet<usize>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub hints: BTreeSet<usize>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub intersections: BTreeSet<[usize; 4]>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_items(items: impl IntoIterator<Item = Item>) -> Self {
        let mut s = Self::default();
        for item in items {
            s.insert(item);
        }
        s
    }

    /// A selection from a target-shaped JSON object (`points`, `shapes`, `anchors`, `guides`,
    /// `annotations`, `hints`, `intersections`; other keys are ignored). Each list is capped at [`MAX_SELECTION_LIST`].
    pub fn from_value(value: &Value) -> Result<Self, String> {
        for key in ["points", "shapes", "anchors", "guides", "annotations", "hints", "intersections"] {
            if let Some(list) = value.get(key).and_then(Value::as_array)
                && list.len() > MAX_SELECTION_LIST
            {
                return Err(format!("`{key}` holds {} entries; at most {MAX_SELECTION_LIST} are allowed", list.len()));
            }
        }
        let mut object = Map::new();
        for key in ["points", "shapes", "anchors", "guides", "annotations", "hints", "intersections"] {
            if let Some(v) = value.get(key) {
                object.insert(key.to_string(), v.clone());
            }
        }
        serde_json::from_value(Value::Object(object)).map_err(|e| format!("selection: {e}"))
    }

    /// The selection a command's reply names: [`from_value`](Self::from_value), with the attachments the commands list
    /// by kind (`corners`, `caps`, `segmentComponents`, `brushes`) as the hints they are.
    pub fn from_reply(value: &Value) -> Result<Self, String> {
        let mut out = Self::from_value(value)?;
        for key in AttachmentKind::ALL.map(AttachmentKind::list_key) {
            let Some(list) = value.get(key).and_then(Value::as_array) else { continue };
            if list.len() > MAX_SELECTION_LIST {
                return Err(format!("`{key}` holds {} entries; at most {MAX_SELECTION_LIST} are allowed", list.len()));
            }
            out.hints.extend(list.iter().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()));
        }
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
            && self.shapes.is_empty()
            && self.anchors.is_empty()
            && self.guides.is_empty()
            && self.annotations.is_empty()
            && self.hints.is_empty()
            && self.intersections.is_empty()
    }

    pub fn len(&self) -> usize {
        self.points.len()
            + self.shapes.len()
            + self.anchors.len()
            + self.guides.len()
            + self.annotations.len()
            + self.hints.len()
            + self.intersections.len()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn contains(&self, item: &Item) -> bool {
        match item {
            Item::Point(s, n) => self.points.contains(&(*s, *n)),
            Item::Shape(s) => self.shapes.contains(s),
            Item::Anchor(name) => self.anchors.contains(name),
            Item::Guide(g) => self.guides.contains(g),
            Item::Annotation(i) => self.annotations.contains(i),
            Item::Hint(i) => self.hints.contains(i),
            Item::Intersection(k) => self.intersections.contains(k),
        }
    }

    /// Adds `item`; `false` when it was already there.
    pub fn insert(&mut self, item: Item) -> bool {
        match item {
            Item::Point(s, n) => self.points.insert((s, n)),
            Item::Shape(s) => self.shapes.insert(s),
            Item::Anchor(name) => self.anchors.insert(name),
            Item::Guide(g) => self.guides.insert(g),
            Item::Annotation(i) => self.annotations.insert(i),
            Item::Hint(i) => self.hints.insert(i),
            Item::Intersection(k) => self.intersections.insert(k),
        }
    }

    /// Removes `item`; `false` when it was not there.
    pub fn remove(&mut self, item: &Item) -> bool {
        match item {
            Item::Point(s, n) => self.points.remove(&(*s, *n)),
            Item::Shape(s) => self.shapes.remove(s),
            Item::Anchor(name) => self.anchors.remove(name),
            Item::Guide(g) => self.guides.remove(g),
            Item::Annotation(i) => self.annotations.remove(i),
            Item::Hint(i) => self.hints.remove(i),
            Item::Intersection(k) => self.intersections.remove(k),
        }
    }

    pub fn toggle(&mut self, item: Item) {
        if !self.remove(&item) {
            self.insert(item);
        }
    }

    /// Every item, points first.
    pub fn items(&self) -> Vec<Item> {
        let mut out = Vec::with_capacity(self.len());
        out.extend(self.points.iter().map(|&(s, n)| Item::Point(s, n)));
        out.extend(self.shapes.iter().map(|&s| Item::Shape(s)));
        out.extend(self.anchors.iter().map(|a| Item::Anchor(a.clone())));
        out.extend(self.guides.iter().map(|&g| Item::Guide(g)));
        out.extend(self.annotations.iter().map(|&i| Item::Annotation(i)));
        out.extend(self.hints.iter().map(|&i| Item::Hint(i)));
        out.extend(self.intersections.iter().map(|&k| Item::Intersection(k)));
        out
    }

    /// Changes the selection by `items` as `mode` says.
    pub fn apply(&mut self, mode: SelectMode, items: impl IntoIterator<Item = Item>) {
        if mode == SelectMode::Replace {
            self.clear();
        }
        for item in items {
            match mode {
                SelectMode::Replace | SelectMode::Add => {
                    self.insert(item);
                }
                SelectMode::Remove => {
                    self.remove(&item);
                }
                SelectMode::Toggle => self.toggle(item),
            }
        }
    }

    /// The command target object for this selection in `scene`'s layer: `glyph`, `layer`,
    /// `side` for the background, and each non-empty list. Commands add their own parameters to it.
    /// Extra nodes are left out: only the Instructor's hinting commands take them, with
    /// parameters of their own.
    pub fn targets(&self, scene: &Scene) -> Map<String, Value> {
        let mut m = scene.target();
        if !self.points.is_empty() {
            m.insert("points".into(), Value::Array(self.points.iter().map(|&(s, n)| json!([s, n])).collect()));
        }
        if !self.shapes.is_empty() {
            m.insert("shapes".into(), Value::Array(self.shapes.iter().map(|&s| json!(s)).collect()));
        }
        if !self.anchors.is_empty() {
            m.insert("anchors".into(), Value::Array(self.anchors.iter().map(|a| json!(a)).collect()));
        }
        if !self.guides.is_empty() {
            m.insert("guides".into(), Value::Array(self.guides.iter().map(|g| json!({"scope": g.scope.as_str(), "index": g.index})).collect()));
        }
        if !self.annotations.is_empty() {
            m.insert("annotations".into(), Value::Array(self.annotations.iter().map(|&i| json!(i)).collect()));
        }
        if !self.hints.is_empty() {
            m.insert("hints".into(), Value::Array(self.hints.iter().map(|&i| json!(i)).collect()));
        }
        m
    }

    /// Drops every entry that does not resolve in `scene` (after undo, redo or another edit).
    pub fn retain_valid(&mut self, scene: &Scene) {
        let layer = &scene.layer;
        self.points.retain(|&(s, n)| scene.node(s, n).is_some());
        self.shapes.retain(|&s| s < layer.shapes.len());
        self.anchors.retain(|name| layer.anchors.iter().any(|a| &a.name == name));
        let (local, global) = (scene.local_guides().len(), scene.global_guides.len());
        self.guides.retain(|g| match g.scope {
            GuideScope::Local => g.index < local,
            GuideScope::Global => g.index < global,
        });
        let annotations = layer.info.annotations.len();
        self.annotations.retain(|&i| i < annotations);
        self.intersections.retain(|&[s1, n1, s2, n2]| scene.node(s1, n1).is_some() && scene.node(s2, n2).is_some());
        // Corner components stay while the layer has them. PostScript hints are known only while
        // they are shown; otherwise the shell keeps them in step.
        let (shown, ps) = (scene.ps_hints.shown, scene.ps_hints.hints.len());
        self.hints.retain(|&i| match layer.info.hints.get(i) {
            None => false,
            Some(_) if crate::attachment::is_selectable(layer, i) => true,
            Some(_) => !shown || i < ps,
        });
    }

    /// The selected on-curve nodes.
    pub fn on_curve_points(&self, scene: &Scene) -> Vec<(usize, usize)> {
        self.points.iter().copied().filter(|&(s, n)| scene.node(s, n).is_some_and(|node| node.on_curve())).collect()
    }

    /// Whether dragging a copy of the selection copies anything: a shape, an anchor, a guide, an
    /// annotation, a whole path, or a segment whose two end nodes are both selected (what
    /// `selection.move` with `duplicate` copies of a partly selected path).
    pub fn has_copyable_part(&self, scene: &Scene) -> bool {
        if !(self.shapes.is_empty() && self.anchors.is_empty() && self.guides.is_empty() && self.annotations.is_empty()) {
            return true;
        }
        let mut by_shape: std::collections::BTreeMap<usize, BTreeSet<usize>> = Default::default();
        for &(s, n) in &self.points {
            by_shape.entry(s).or_default().insert(n);
        }
        by_shape.iter().any(|(&s, nodes)| {
            scene.path(s).is_some_and(|path| {
                nodes.len() == path.nodes.len() || path.segments().iter().any(|g| nodes.contains(&g.start) && nodes.contains(&g.end))
            })
        })
    }

    /// Whether the selection is handles and nothing else.
    pub fn is_handles_only(&self, scene: &Scene) -> bool {
        !self.points.is_empty()
            && self.shapes.is_empty()
            && self.anchors.is_empty()
            && self.guides.is_empty()
            && self.annotations.is_empty()
            && self.points.iter().all(|&(s, n)| scene.node(s, n).is_some_and(|node| !node.on_curve()))
    }
}

/// Every point of the path at `shape` (what a double-click near a segment selects).
pub fn path_points(scene: &Scene, shape: usize) -> Selection {
    let mut s = Selection::new();
    if let Some(path) = scene.path(shape) {
        s.points.extend((0..path.nodes.len()).map(|n| (shape, n)));
    }
    s
}

/// `edit.selectAll`: every point of every path; when they are all selected already, the anchors
/// and components too.
pub fn select_all(scene: &Scene, current: &Selection) -> Selection {
    let mut s = Selection::new();
    for (i, path) in scene.paths() {
        s.points.extend((0..path.nodes.len()).map(|n| (i, n)));
    }
    let all_points = s.points.iter().all(|p| current.points.contains(p));
    if all_points {
        s.anchors.extend(scene.layer.anchors.iter().map(|a| a.name.clone()));
        for (i, shape) in scene.layer.shapes.iter().enumerate() {
            if matches!(shape, Shape::Component(_)) && scene.shape_editable(i) {
                s.shapes.insert(i);
            }
        }
    }
    s
}

/// `edit.invertSelection`: the points and anchors that are not selected.
pub fn invert(scene: &Scene, current: &Selection) -> Selection {
    let mut s = Selection::new();
    for (i, path) in scene.paths() {
        s.points.extend((0..path.nodes.len()).map(|n| (i, n)).filter(|p| !current.points.contains(p)));
    }
    s.anchors.extend(scene.layer.anchors.iter().map(|a| a.name.clone()).filter(|a| !current.anchors.contains(a)));
    s
}

/// The order `Tab` walks: the points of each path in path order, then the components that can be
/// selected, in shape order, then the anchors, then the local guides, then the global guides.
pub fn tab_order(scene: &Scene) -> Vec<Item> {
    let mut out = Vec::new();
    for (i, path) in scene.paths() {
        out.extend((0..path.nodes.len()).map(|n| Item::Point(i, n)));
    }
    for (i, shape) in scene.layer.shapes.iter().enumerate() {
        if matches!(shape, Shape::Component(_)) && scene.shape_editable(i) {
            out.push(Item::Shape(i));
        }
    }
    out.extend(scene.layer.anchors.iter().map(|a| Item::Anchor(a.name.clone())));
    out.extend((0..scene.local_guides().len()).map(|index| Item::Guide(GuideRef { scope: GuideScope::Local, index })));
    out.extend(scene.shown_global_guides().map(|(index, _)| Item::Guide(GuideRef { scope: GuideScope::Global, index })));
    out
}

/// `selection.selectNext` (`forward`) and `selection.selectPrevious`: with exactly one item
/// selected, the selection of the item after (or before) it in [`tab_order`], wrapping around.
/// `None` when the selection is not exactly one item that the order knows.
pub fn step(scene: &Scene, current: &Selection, forward: bool) -> Option<Selection> {
    let items = current.items();
    let [only] = items.as_slice() else { return None };
    let order = tab_order(scene);
    let count = order.len();
    let i = order.iter().position(|it| it == only)?;
    let next = if forward { (i + 1) % count } else { (i + count - 1) % count };
    order.get(next).cloned().map(|it| Selection::from_items([it]))
}

/// `selection.selectInRegion`: the points and anchors inside `polygon` (a marquee's four corners
/// or a lasso's outline), with the scene's extra nodes. `nodes_only` leaves the handles out. `components` also takes each
/// component that can be selected and has an on-curve point of its outline inside.
pub fn in_region(scene: &Scene, polygon: &[Point], nodes_only: bool, components: bool) -> Selection {
    let mut s = Selection::new();
    if polygon.len() < 3 {
        return s;
    }
    for (i, path) in scene.paths() {
        for (n, node) in path.nodes.iter().enumerate() {
            if nodes_only && node.kind == NodeKind::OffCurve {
                continue;
            }
            if point_in_polygon(node.point(), polygon) {
                s.points.insert((i, n));
            }
        }
    }
    for e in scene.extra_nodes.iter().filter(|e| scene.shape_editable(e.a.0) && scene.shape_editable(e.b.0)) {
        if point_in_polygon(e.at, polygon) {
            s.intersections.insert(e.key());
        }
    }
    for a in &scene.layer.anchors {
        if point_in_polygon(Point::new(a.x, a.y), polygon) {
            s.anchors.insert(a.name.clone());
        }
    }
    if components {
        for c in scene.components() {
            if scene.shape_editable(c.shape) && c.nodes.iter().any(|p| point_in_polygon(*p, polygon)) {
                s.shapes.insert(c.shape);
            }
        }
    }
    s
}
