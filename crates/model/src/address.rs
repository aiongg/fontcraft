//! Addresses inside a layer, as editing commands name them (decision 0004): shapes by index,
//! points as `(shape, node)`, segments by the node that ends them, anchors by name, guides and
//! annotations by index. A layer has two sides that hold such things: the foreground and the
//! background.

use fontcraft_geom::{NodePoint, NodeType, Point, Segment};

use crate::{Anchor, Annotation, Guide, Layer, Node, NodeKind, Path, Shape};

/// Which drawing of a layer a command addresses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerSide {
    #[default]
    Foreground,
    Background,
}

impl LayerSide {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerSide::Foreground => "foreground",
            LayerSide::Background => "background",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "foreground" => Some(LayerSide::Foreground),
            "background" => Some(LayerSide::Background),
            _ => None,
        }
    }
}

/// The addressable lists of one side of a layer.
#[derive(Clone, Copy, Debug)]
pub struct LayerParts<'a> {
    pub shapes: &'a [Shape],
    pub anchors: &'a [Anchor],
    /// The side's own guides (the master's guides are the global ones).
    pub guides: &'a [Guide],
    pub annotations: &'a [Annotation],
}

/// [`LayerParts`] for editing.
#[derive(Debug)]
pub struct LayerPartsMut<'a> {
    pub shapes: &'a mut Vec<Shape>,
    pub anchors: &'a mut Vec<Anchor>,
    pub guides: &'a mut Vec<Guide>,
    pub annotations: &'a mut Vec<Annotation>,
}

impl Layer {
    /// The lists of one side. A layer without a background has an empty one.
    pub fn parts(&self, side: LayerSide) -> LayerParts<'_> {
        match side {
            LayerSide::Foreground => {
                LayerParts { shapes: &self.shapes, anchors: &self.anchors, guides: &self.info.guides, annotations: &self.info.annotations }
            }
            LayerSide::Background => match self.info.background.as_ref() {
                Some(b) => LayerParts { shapes: &b.shapes, anchors: &b.anchors, guides: &b.guides, annotations: &b.annotations },
                None => LayerParts { shapes: &[], anchors: &[], guides: &[], annotations: &[] },
            },
        }
    }

    /// The lists of one side, for editing. Asking for the background of a layer that has none
    /// gives it an empty one.
    pub fn parts_mut(&mut self, side: LayerSide) -> LayerPartsMut<'_> {
        match side {
            LayerSide::Foreground => {
                let info = &mut *self.info;
                LayerPartsMut { shapes: &mut self.shapes, anchors: &mut self.anchors, guides: &mut info.guides, annotations: &mut info.annotations }
            }
            LayerSide::Background => {
                let b = self.info.background.get_or_insert_with(Default::default);
                LayerPartsMut { shapes: &mut b.shapes, anchors: &mut b.anchors, guides: &mut b.guides, annotations: &mut b.annotations }
            }
        }
    }

    /// The path at shape index `shape` of the foreground.
    pub fn path(&self, shape: usize) -> Option<&Path> {
        self.parts(LayerSide::Foreground).path(shape)
    }

    /// The node at `(shape, node)` of the foreground.
    pub fn node(&self, shape: usize, node: usize) -> Option<&Node> {
        self.parts(LayerSide::Foreground).node(shape, node)
    }
}

impl<'a> LayerParts<'a> {
    /// The path at shape index `shape` (`None` for a component, an image or a missing index).
    pub fn path(&self, shape: usize) -> Option<&'a Path> {
        match self.shapes.get(shape)? {
            Shape::Path(p) => Some(p),
            _ => None,
        }
    }

    pub fn node(&self, shape: usize, node: usize) -> Option<&'a Node> {
        self.path(shape)?.nodes.get(node)
    }

    /// The anchor called `name` and its index.
    pub fn anchor(&self, name: &str) -> Option<(usize, &'a Anchor)> {
        self.anchors.iter().enumerate().find(|(_, a)| a.name == name)
    }
}

impl LayerPartsMut<'_> {
    pub fn path_mut(&mut self, shape: usize) -> Option<&mut Path> {
        match self.shapes.get_mut(shape)? {
            Shape::Path(p) => Some(p),
            _ => None,
        }
    }

    pub fn node_mut(&mut self, shape: usize, node: usize) -> Option<&mut Node> {
        self.path_mut(shape)?.nodes.get_mut(node)
    }
}

impl From<NodeKind> for NodeType {
    fn from(k: NodeKind) -> Self {
        match k {
            NodeKind::Line => NodeType::Line,
            NodeKind::Curve => NodeType::Curve,
            NodeKind::QCurve => NodeType::QCurve,
            NodeKind::OffCurve => NodeType::OffCurve,
        }
    }
}

impl Path {
    /// The nodes as the geometry crate reads them.
    pub fn node_points(&self) -> Vec<NodePoint> {
        self.nodes.iter().map(|n| NodePoint::new(n.point(), n.kind.into())).collect()
    }

    /// The segments in path order, each with the indices of its nodes.
    pub fn segments(&self) -> Vec<Segment> {
        fontcraft_geom::segments(&self.node_points(), self.closed)
    }

    /// The segment that ends at node `node`, if that node ends one: an on-curve node, other than
    /// the first on-curve node of an open path.
    pub fn segment_ending_at(&self, node: usize) -> Option<Segment> {
        fontcraft_geom::segment_ending_at(&self.node_points(), self.closed, node)
    }

    /// The implied nodes of quadratic segments: the two neighbouring off-curve nodes each lies halfway between, in path
    /// order, and its place.
    pub fn implied_nodes(&self) -> Vec<((usize, usize), Point)> {
        let n = self.nodes.len();
        let mut out = Vec::new();
        let mut pair = |a: usize, b: usize| {
            if let (Some(p), Some(q)) = (self.nodes.get(a), self.nodes.get(b)) {
                out.push(((a, b), p.point().midpoint(q.point())));
            }
        };
        let Some(first) = self.nodes.iter().position(Node::on_curve) else {
            if self.closed && n > 1 {
                for a in 0..n {
                    pair(a, if a + 1 == n { 0 } else { a + 1 });
                }
            }
            return out;
        };
        let mut run: Vec<usize> = Vec::new();
        let steps = if self.closed { n } else { n - first - 1 };
        for step in 1..=steps {
            let i = (first + step) % n;
            let Some(node) = self.nodes.get(i) else { continue };
            if !node.on_curve() {
                run.push(i);
                continue;
            }
            if node.kind == NodeKind::QCurve {
                for w in run.windows(2) {
                    if let [a, b] = w {
                        pair(*a, *b);
                    }
                }
            }
            run.clear();
        }
        out
    }

    /// Whether node `node` ends a segment, without building the segments.
    pub fn ends_segment(&self, node: usize) -> bool {
        let Some(n) = self.nodes.get(node) else { return false };
        if !n.on_curve() {
            return false;
        }
        self.closed || self.nodes.iter().position(Node::on_curve).is_some_and(|first| node > first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Background;

    #[test]
    fn sides_and_addresses_resolve() {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        layer.anchors.push(Anchor::new("top", 50.0, 700.0));
        assert_eq!(layer.node(0, 2).map(|n| (n.x, n.y)), Some((100.0, 100.0)));
        assert!(layer.node(0, 4).is_none() && layer.node(1, 0).is_none());
        assert_eq!(layer.parts(LayerSide::Foreground).anchor("top").map(|(i, _)| i), Some(0));
        assert!(layer.parts(LayerSide::Background).shapes.is_empty());
        // Editing the background of a layer that has none creates it.
        layer.parts_mut(LayerSide::Background).shapes.push(Shape::Path(Path::rect(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(layer.info.background.as_ref().map(|b: &Background| b.shapes.len()), Some(1));
        assert!(layer.parts(LayerSide::Background).path(0).is_some());

        let p = Path::rect(0.0, 0.0, 10.0, 10.0);
        assert_eq!(p.segments().len(), 4);
        assert!(p.ends_segment(0));
        let open = Path::new(false, p.nodes.clone());
        assert!(!open.ends_segment(0) && open.ends_segment(3) && !open.ends_segment(4));
        assert_eq!(open.segment_ending_at(2).map(|s| (s.start, s.end)), Some((1, 2)));
    }
}
