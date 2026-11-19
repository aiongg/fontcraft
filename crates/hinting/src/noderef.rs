//! Node references: how hints and instructions point at nodes of a layer.
//!
//! A reference is a (path, node) pair of indexes. `path` counts the layer's paths in shape order,
//! components and other shapes skipped, as glyphsLib's `_find_node_by_indices` reads them
//! (`layer.paths`); every node reference of the format samples resolves this way. A reference is
//! checked against the layer before use; a dangling one is [`HintingError::DanglingNode`], never
//! an index panic. Rows: `hinting.ps.manual.linkOnDrop`, `hinting.tt.instructions.addFromSelection`.
//!
//! Two node orders meet here. Stored hint values count nodes in the file's order: a closed path's
//! start node comes last, as `.glyphs` files write it and as glyphsLib copies the indexes into a
//! UFO glyph's lib. The model keeps a closed path's start node first, so stored node `k` of a
//! closed path of `n` nodes is the model's node `(k + 1) % n`; on an open path the orders agree.
//! The stored values stay in file order in every format (so they travel between formats
//! unchanged, as corner hints do); a [`NodeRef`] always counts in the model's order. [`NodeOrder`]
//! converts when a hint is read from or written to its stored value. A stored index past the end
//! of its path is kept as it is, so it still dangles.
//!
//! Stored forms (the format specifications of the GlyphsSDK and the format samples):
//!
//! | Form | Format 3 and 4 | Format 2 | glyphsLib's UFO key |
//! |---|---|---|---|
//! | node | `(0,3)` | `"{0, 3}"` | `[0, 3]` |
//! | intersection of two segments | `(0,1,1,3)` | `"{0, 1, 1, 3}"` | `[0, 1, 1, 3]` |
//! | side bearing | `lsb`, `rsb` | `"{lsb}"`, `"{rsb}"` | `["lsb"]` |
//!
//! The format's three-number form (an inflection on a curve) is not read: no source uses it and no
//! instruction type of the Instructor creates it.

use fontcraft_model::plist::{Array, Dict, Layout, Num, Number, Str, Value};
use fontcraft_model::{Family, Layer, LayerInfo, Node, Path, Shape};

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

    /// Reads the stored form (`"{0, 3}"` in format 2, `(0, 3)` in format 3), in file order on the
    /// layer whose node order is `order`.
    pub fn from_value(v: &Value, order: &NodeOrder) -> Option<Self> {
        match index_list(v)?.as_slice() {
            [path, node] => Some(order.to_model(Self::new(*path, *node))),
            _ => None,
        }
    }

    /// The stored form, in file order on the layer whose node order is `order`, in the syntax of
    /// `like` (the value it replaces) when given.
    pub fn to_value(&self, order: &NodeOrder, like: Option<&Value>) -> Value {
        let r = order.to_file(*self);
        Syntax::of(like).indexes(&[r.path, r.node])
    }
}

/// A layer's node order, to convert node references between the stored (file) order and the
/// model's (module doc). The default converts nothing: a layer without closed paths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeOrder {
    /// Per path (paths only, in shape order): its node count when it is closed, else 0.
    closed: Vec<usize>,
}

impl NodeOrder {
    /// The node order of a layer's paths.
    pub fn of(layer: &Layer) -> Self {
        Self::of_shapes(&layer.shapes)
    }

    /// The node order of a list of shapes (a layer's or its background's).
    pub fn of_shapes(shapes: &[Shape]) -> Self {
        let closed = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) => Some(if p.closed { p.nodes.len() } else { 0 }),
                _ => None,
            })
            .collect();
        Self { closed }
    }

    /// The node count of closed path `path`; `None` for an open path or one that does not exist.
    pub fn closed_len(&self, path: usize) -> Option<usize> {
        self.closed.get(path).copied().filter(|n| *n > 0)
    }

    /// A reference read in file order, in the model's order.
    pub fn to_model(&self, r: NodeRef) -> NodeRef {
        match self.closed_len(r.path) {
            Some(n) if r.node < n => NodeRef::new(r.path, (r.node + 1) % n),
            _ => r,
        }
    }

    /// A reference in the model's order, in file order.
    pub fn to_file(&self, r: NodeRef) -> NodeRef {
        match self.closed_len(r.path) {
            Some(n) if r.node < n => NodeRef::new(r.path, (r.node + n - 1) % n),
            _ => r,
        }
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
    /// `b` (the format's 4-tuple form, as its specification describes it).
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
    /// Reads the stored form: `(path, node)`, a 4-tuple, `lsb` or `rsb`; node indexes in file
    /// order on the layer whose node order is `order`.
    pub fn from_value(v: &Value, order: &NodeOrder) -> Option<Self> {
        if let Some(side) = side_bearing(v) {
            return Some(side);
        }
        let at = |path: usize, node: usize| order.to_model(NodeRef::new(path, node));
        match index_list(v)?.as_slice() {
            [path, node] => Some(TtPoint::Node(at(*path, *node))),
            [p1, n1, p2, n2] => Some(TtPoint::Intersection { a: at(*p1, *n1), b: at(*p2, *n2) }),
            _ => None,
        }
    }

    /// The stored form, node indexes in file order on the layer whose node order is `order`, in
    /// the syntax of `like` when given.
    pub fn to_value(&self, order: &NodeOrder, like: Option<&Value>) -> Value {
        let syntax = Syntax::of(like);
        match self {
            TtPoint::Node(r) => {
                let r = order.to_file(*r);
                syntax.indexes(&[r.path, r.node])
            }
            TtPoint::Intersection { a, b } => {
                let (a, b) = (order.to_file(*a), order.to_file(*b));
                syntax.indexes(&[a.path, a.node, b.path, b.node])
            }
            TtPoint::Lsb => syntax.word("lsb"),
            TtPoint::Rsb => syntax.word("rsb"),
        }
    }

    /// The node, for points that are nodes.
    pub fn node(&self) -> Option<NodeRef> {
        match self {
            TtPoint::Node(r) => Some(*r),
            _ => None,
        }
    }
}

// ── stored forms shared by `ps` and `tt` ───────────────────────────────────────────────────────

/// The syntax a stored hint value is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Syntax {
    /// Format 3 and 4: lists of numbers, bare words, `horizontal = 1`.
    #[default]
    Glyphs3,
    /// Format 2: index paths and points as brace strings (`"{0, 3}"`).
    Glyphs2,
    /// glyphsLib's dictionaries in a UFO glyph's lib: booleans, lists, every key present.
    Ufo,
}

impl Syntax {
    /// The syntax of one stored point or index path; the default when there is none.
    pub(crate) fn of(like: Option<&Value>) -> Self {
        match like {
            Some(Value::String(s)) if s.as_str().trim_start().starts_with('{') => Syntax::Glyphs2,
            Some(Value::Array(a)) if a.items.iter().any(|i| matches!(i, Value::String(_))) => Syntax::Ufo,
            _ => Syntax::Glyphs3,
        }
    }

    /// The syntax of a whole stored hint: UFO for the UFO family, else format 2 when any of its
    /// points is a brace string.
    pub(crate) fn of_hint(family: Family, value: &Value) -> Self {
        if family == Family::Ufo {
            return Syntax::Ufo;
        }
        let Some(d) = value.as_dict() else { return Syntax::Glyphs3 };
        let braces = ["origin", "target", "other1", "other2", "place", "scale"]
            .iter()
            .filter_map(|k| d.get(k))
            .any(|v| matches!(v, Value::String(s) if s.as_str().trim_start().starts_with('{')));
        if braces { Syntax::Glyphs2 } else { Syntax::Glyphs3 }
    }

    /// The syntax new hints of a layer are written in: that of the layer's stored hints (the
    /// first one's family; format 2 when any hint uses brace strings), else format 3.
    pub(crate) fn of_layer(info: &LayerInfo) -> (Family, Self) {
        let family = info.hints.first().map(|h| h.family).unwrap_or(Family::Glyphs);
        if family == Family::Ufo {
            return (family, Syntax::Ufo);
        }
        let v2 = info.hints.iter().any(|h| Syntax::of_hint(h.family, &h.value) == Syntax::Glyphs2);
        (family, if v2 { Syntax::Glyphs2 } else { Syntax::Glyphs3 })
    }

    /// A representative stored point of this syntax, to pass as `like`.
    pub(crate) fn example(self) -> Option<Value> {
        match self {
            Syntax::Glyphs3 => None,
            Syntax::Glyphs2 => Some(Value::String(Str::new("{0, 0}"))),
            Syntax::Ufo => Some(Value::Array(Array { items: vec![Value::String(Str::new("lsb"))], ..Default::default() })),
        }
    }

    /// An index path (`(0,3)`, `"{0, 3}"`).
    pub(crate) fn indexes(self, items: &[usize]) -> Value {
        match self {
            Syntax::Glyphs2 => Value::String(Str::new(braced(&items.iter().map(|i| i.to_string()).collect::<Vec<_>>()))),
            Syntax::Glyphs3 | Syntax::Ufo => {
                compact(items.iter().map(|i| Value::Number(Number::int(i64::try_from(*i).unwrap_or(i64::MAX)))).collect())
            }
        }
    }

    fn word(self, w: &str) -> Value {
        match self {
            Syntax::Glyphs3 => Value::String(Str::new(w)),
            Syntax::Glyphs2 => Value::String(Str::new(format!("{{{w}}}"))),
            Syntax::Ufo => Value::Array(Array { items: vec![Value::String(Str::new(w))], ..Default::default() }),
        }
    }

    /// A pair of numbers (`place`): a compact list, or a brace string in format 2.
    pub(crate) fn pair(self, a: f64, b: f64) -> Value {
        match self {
            Syntax::Glyphs2 => Value::String(Str::new(braced(&[number_text(a), number_text(b)]))),
            Syntax::Glyphs3 | Syntax::Ufo => compact(vec![number(a), number(b)]),
        }
    }

    /// A true flag (`horizontal`): `1`, or a boolean in a UFO.
    pub(crate) fn flag(self, on: bool) -> Option<Value> {
        match self {
            Syntax::Ufo => Some(Value::Bool(on)),
            Syntax::Glyphs2 | Syntax::Glyphs3 => on.then(|| Value::Number(Number::int(1))),
        }
    }
}

fn braced(items: &[String]) -> String {
    format!("{{{}}}", items.join(", "))
}

fn compact(items: Vec<Value>) -> Value {
    Value::Array(Array { items, layout: Layout::Compact, ..Default::default() })
}

/// A number as a stored value: an integer when it is one.
pub(crate) fn number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        return Value::Number(Number::int(f as i64));
    }
    Number::float(f).map(Value::Number).unwrap_or(Value::Number(Number::int(0)))
}

pub(crate) fn number_text(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 9.0e15 { format!("{}", f as i64) } else { format!("{f}") }
}

/// A finite number: a number value, or a string that reads as one.
pub(crate) fn read_number(v: &Value) -> Option<f64> {
    let f = match v {
        Value::Number(n) => n.value().as_f64(),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    f.is_finite().then_some(f)
}

/// An integer: a whole finite number.
pub(crate) fn read_int(v: &Value) -> Option<i64> {
    let f = read_number(v)?;
    (f.fract() == 0.0 && f.abs() < 9.0e15).then_some(f as i64)
}

/// A flag: a boolean, a number or a string that reads as one; `None` when it is none of these.
pub(crate) fn read_flag(v: &Value) -> Option<bool> {
    fontcraft_model::param_flag(v)
}

/// A list of numbers: a list value, or a brace string (`"{-46, 58}"`).
pub(crate) fn read_numbers(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Array(a) => a.items.iter().map(read_number).collect(),
        Value::String(s) => {
            let t = s.as_str().trim();
            let inner = t.strip_prefix('{')?.strip_suffix('}')?;
            inner.split(',').map(|x| x.trim().parse::<f64>().ok().filter(|f| f.is_finite())).collect()
        }
        _ => None,
    }
}

/// Node indexes: whole, non-negative and below `u32::MAX`, as the format's `u32`.
fn index_list(v: &Value) -> Option<Vec<usize>> {
    let nums = read_numbers(v)?;
    if nums.is_empty() || nums.len() > 4 {
        return None;
    }
    nums.into_iter().map(|f| (f.fract() == 0.0 && (0.0..f64::from(u32::MAX)).contains(&f)).then_some(f as usize)).collect()
}

fn side_bearing(v: &Value) -> Option<TtPoint> {
    let word = match v {
        Value::String(s) => s.as_str().trim().trim_start_matches('{').trim_end_matches('}').trim().to_string(),
        Value::Array(a) => match a.items.as_slice() {
            [Value::String(s)] => s.as_str().to_string(),
            _ => return None,
        },
        _ => return None,
    };
    match word.as_str() {
        "lsb" => Some(TtPoint::Lsb),
        "rsb" => Some(TtPoint::Rsb),
        _ => None,
    }
}

/// Sets `key` to `value`, in place when the key exists, else at its alphabetical place (the
/// reference writes hint keys sorted).
pub(crate) fn put(d: &mut Dict, key: &str, value: Value) {
    if let Some(slot) = d.get_mut(key) {
        *slot = value;
        return;
    }
    let at = d.entries.iter().position(|(k, _)| k.as_str().as_bytes() > key.as_bytes()).unwrap_or(d.entries.len());
    d.insert_at(at, key, value);
}

/// Sets `key` to `value` unless the stored value already means the same (keeping its spelling).
pub(crate) fn put_if_changed(d: &mut Dict, key: &str, value: Value, same: impl Fn(&Value) -> bool) {
    if d.get(key).is_some_and(same) {
        return;
    }
    put(d, key, value);
}

/// Removes every entry called `key`. A single entry goes with its kept whitespace; a key
/// repeated in the source is removed in one pass, after which the dictionary's kept whitespace no
/// longer matches its entries and the default layout is written.
pub(crate) fn remove(d: &mut Dict, key: &str) {
    let mut found = d.entries.iter().enumerate().filter(|(_, (k, _))| k.as_str() == key).map(|(i, _)| i);
    match (found.next(), found.next()) {
        (None, _) => {}
        (Some(i), None) => {
            d.remove_at(i);
        }
        (Some(_), Some(_)) => d.entries.retain(|(k, _)| k.as_str() != key),
    }
}

/// The `type` of a stored hint, compared without case: the format writes `TTStem`, glyphsLib may
/// hand back other spellings.
pub(crate) fn type_is(v: &Value, names: &[&str], codes: &[i64]) -> bool {
    match v {
        Value::String(s) => names.iter().any(|n| s.as_str().eq_ignore_ascii_case(n)),
        Value::Number(n) => match n.value() {
            Num::Int(i) => codes.contains(&i),
            Num::Float(_) => false,
        },
        _ => false,
    }
}
