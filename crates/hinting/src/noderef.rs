//! Node references: how hints and instructions point at nodes of a layer.
//!
//! The source's form is kept: a (path, node) pair of indexes. `path` counts the layer's paths in
//! shape order, components and other shapes skipped (M10.2 confirms this against the format
//! samples and fixes this sentence if they say otherwise). A reference is checked against the
//! layer before use; a dangling one is [`HintingError::DanglingNode`], never an index panic.
//! Rows: `hinting.ps.manual.linkOnDrop`, `hinting.tt.instructions.addFromSelection`. M10.2 fills
//! the bodies marked `// M10.2:`.

use fontcraft_model::plist::Value;
use fontcraft_model::{Layer, Node, Path, Shape};

use crate::HintingError;

/// A node of a layer: path index (paths only, in shape order) and node index in that path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeRef {
    pub path: usize,
    pub node: usize,
}

impl NodeRef {
    pub const fn new(path: usize, node: usize) -> Self {
        Self { path, node }
    }

    /// The path this reference names.
    pub fn path<'a>(&self, layer: &'a Layer) -> Option<&'a Path> {
        layer
            .shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) => Some(p),
                _ => None,
            })
            .nth(self.path)
    }

    /// The node this reference names.
    pub fn resolve<'a>(&self, layer: &'a Layer) -> Option<&'a Node> {
        self.path(layer)?.nodes.get(self.node)
    }

    /// The node, or [`HintingError::DanglingNode`].
    pub fn check<'a>(&self, layer: &'a Layer) -> Result<&'a Node, HintingError> {
        self.resolve(layer).ok_or(HintingError::DanglingNode { path: self.path, node: self.node })
    }

    /// Reads the stored form (`"{0, 3}"` in format 2, `(0, 3)` in format 3).
    pub fn from_value(v: &Value) -> Option<Self> {
        // M10.2: both forms.
        let _ = v;
        None
    }

    /// The stored form, in the syntax of `like` (the value it replaces) when given.
    pub fn to_value(&self, like: Option<&Value>) -> Value {
        // M10.2: write in the source's form.
        let _ = like;
        Value::Array(Default::default())
    }
}

/// One edge of a PostScript hint: a number, or a node whose coordinate (y for horizontal hints,
/// x for vertical ones) gives the edge in every master.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Value(f64),
    Node(NodeRef),
}

impl Edge {
    /// The edge's coordinate on `layer`: the number, or the node's x or y.
    pub fn position(&self, layer: &Layer, horizontal: bool) -> Result<f64, HintingError> {
        match self {
            Edge::Value(v) => Ok(*v),
            Edge::Node(r) => r.check(layer).map(|n| if horizontal { n.y } else { n.x }),
        }
    }
}

/// A point a TrueType instruction names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TtPoint {
    Node(NodeRef),
    /// An extra node: the intersection of the segment that ends at `a` with the one that ends at
    /// `b` (the format's 4-tuple form; meaning to be confirmed by M10.2 on the samples).
    Intersection {
        a: NodeRef,
        b: NodeRef,
    },
    /// The left sidebearing phantom point.
    Lsb,
    /// The right sidebearing (advance) phantom point.
    Rsb,
}

impl TtPoint {
    /// Reads the stored form: `(path, node)`, a 4-tuple, `lsb` or `rsb`.
    pub fn from_value(v: &Value) -> Option<Self> {
        // M10.2: every form of the samples.
        let _ = v;
        None
    }

    /// The stored form, in the syntax of `like` when given.
    pub fn to_value(&self, like: Option<&Value>) -> Value {
        // M10.2: write in the source's form.
        let _ = like;
        Value::Array(Default::default())
    }

    /// The node, for points that are nodes.
    pub fn node(&self) -> Option<NodeRef> {
        match self {
            TtPoint::Node(r) => Some(*r),
            _ => None,
        }
    }
}
