//! PostScript hints on a layer: stem hints and ghost hints, parsed from and written back to the
//! stored hint values.
//!
//! Forms read (`docs/plans/M10.md` § 2): `Stem` with a horizontal flag and `origin`/`target` as
//! node references or `place` = (position, width); `TopGhost` (target `down`) and `BottomGhost`
//! (target `up`) in node and `place` forms; format 2 strings `"{0, 0}"`; glyphsLib's dictionaries
//! in a UFO glyph's lib. Rows: `hinting.ps.twoLevels`, `hinting.ps.manual.hintKinds`,
//! `hinting.ps.masters.mainMasterOnly`, `hinting.ps.masters.fallbackToOtherMasters`,
//! `hinting.ps.masters.specialLayers`.
//!
//! How the edges are stored:
//!
//! | Edge | Stored as |
//! |---|---|
//! | origin on a node | `origin` = the node reference |
//! | origin as a number | `place` = (origin, …) |
//! | other edge of a stem on a node | `target` = the node reference |
//! | other edge of a stem as a number | `place` = (p, w), the edge at p + w |
//! | ghost | `target` = `down` (top ghost) or `up` (bottom ghost); a numeric ghost keeps the width the format reserves, −20 top and 21 bottom, in `place` |
//!
//! A stem with one edge on a node and the other a number (what node-reference upkeep leaves when
//! a linked node is deleted) stores the number as `place` = (number, 0). The format samples hold
//! no such hint; this is our choice (guess on `hinting.ps.manual.hintKinds`).
//!
//! Writing back changes only the keys whose meaning changed, so unknown keys, key order and the
//! source's spellings survive a read and write of an unchanged hint byte for byte.

use fontcraft_model::plist::{Dict, Value};
use fontcraft_model::{Family, Font, Glyph, Hint, Layer, LayerInfo};

use crate::noderef::{Syntax, number, put, put_if_changed, read_flag, read_numbers, remove, type_is};
use crate::{Edge, HintSettings, HintingError, MAX_LAYER_HINTS, NodeOrder, NodeRef};

/// Width the format stores for a numeric top ghost hint.
pub const TOP_GHOST_WIDTH: f64 = -20.0;
/// Width the format stores for a numeric bottom ghost hint.
pub const BOTTOM_GHOST_WIDTH: f64 = 21.0;

/// The kind of a PostScript hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsHintKind {
    Stem,
    /// A ghost hint at a top edge (the format's `TopGhost`).
    GhostTop,
    /// A ghost hint at a bottom edge (`BottomGhost`).
    GhostBottom,
}

/// A PostScript hint.
#[derive(Clone, Debug, PartialEq)]
pub struct PsHint {
    /// Measured vertically: a horizontal stem (`hstem`), its edges are y values.
    pub horizontal: bool,
    pub kind: PsHintKind,
    /// The first edge; for a ghost, its only edge.
    pub origin: Edge,
    /// The second edge of a stem; `None` for ghosts.
    pub other: Option<Edge>,
    /// The stored value this hint was read from. [`set_ps_hints`] writes only the keys the typed
    /// fields cover over it, so unknown keys and the stored form (format 2 strings, `place`)
    /// survive. `None` for a new hint.
    pub source: Option<Value>,
}

impl PsHint {
    /// The hint's (bottom or left, top or right) edges on a layer, ghosts as the edge twice.
    pub fn edges(&self, layer: &fontcraft_model::Layer) -> Result<(f64, f64), HintingError> {
        let a = self.origin.position(layer, self.horizontal)?;
        let b = match &self.other {
            Some(e) => e.position(layer, self.horizontal)?,
            None => a,
        };
        Ok(if a <= b { (a, b) } else { (b, a) })
    }

    /// A stem hint between two edges.
    pub fn stem(horizontal: bool, origin: Edge, other: Edge) -> Self {
        Self { horizontal, kind: PsHintKind::Stem, origin, other: Some(other), source: None }
    }

    /// A ghost hint (always horizontal) at one edge.
    pub fn ghost(top: bool, edge: Edge) -> Self {
        let kind = if top { PsHintKind::GhostTop } else { PsHintKind::GhostBottom };
        Self { horizontal: true, kind, origin: edge, other: None, source: None }
    }

    /// Reads one stored hint of a layer whose node order is `order`; `None` when it is not a
    /// PostScript hint.
    pub fn from_hint(index: usize, hint: &Hint, order: &NodeOrder) -> Option<Result<Self, HintingError>> {
        let d = hint.value.as_dict()?;
        let kind = ps_kind(d)?;
        Some(read(index, d, kind, order).map(|mut h| {
            h.source = Some(hint.value.clone());
            h
        }))
    }

    /// The stored value of this hint on a layer whose node order is `order`, over
    /// [`PsHint::source`].
    pub fn to_hint(&self, family: Family, order: &NodeOrder) -> Hint {
        let syntax = match &self.source {
            Some(v) => Syntax::of_hint(family, v),
            None if family == Family::Ufo => Syntax::Ufo,
            None => Syntax::Glyphs3,
        };
        self.to_hint_in(family, syntax, order)
    }

    /// Whether `self` and `other` say the same thing, their stored values aside.
    pub fn same_as(&self, other: &PsHint) -> bool {
        self.horizontal == other.horizontal && self.kind == other.kind && self.origin == other.origin && self.other == other.other
    }

    pub(crate) fn to_hint_in(&self, family: Family, syntax: Syntax, order: &NodeOrder) -> Hint {
        let old = self.source.as_ref().and_then(|v| {
            let d = v.as_dict()?;
            let kind = ps_kind(d)?;
            read(0, d, kind, order).ok()
        });
        if let (Some(v), Some(old)) = (&self.source, &old)
            && old.same_as(self)
        {
            return Hint { family, value: v.clone() };
        }
        let mut d = self.source.as_ref().and_then(|v| v.as_dict().cloned()).unwrap_or_default();
        self.write(&mut d, old.as_ref(), syntax, order);
        Hint { family, value: Value::Dict(d) }
    }

    fn write(&self, d: &mut Dict, old: Option<&PsHint>, syntax: Syntax, order: &NodeOrder) {
        // A UFO dictionary carries every key glyphsLib's reader looks up.
        if syntax == Syntax::Ufo {
            for (k, v) in [("name", Value::String("".into())), ("options", number(0.0)), ("stem", number(-2.0))] {
                if d.get(k).is_none() {
                    put(d, k, v);
                }
            }
        }
        match syntax.flag(self.horizontal) {
            Some(v) => put_if_changed(d, "horizontal", v, |x| read_flag(x) == Some(self.horizontal)),
            None => remove(d, "horizontal"),
        }
        let name = match self.kind {
            PsHintKind::Stem => "Stem",
            PsHintKind::GhostTop => "TopGhost",
            PsHintKind::GhostBottom => "BottomGhost",
        };
        if old.map(|o| o.kind) != Some(self.kind) || d.get("type").is_none() {
            put(d, "type", Value::String(name.into()));
        }
        let like = |d: &Dict, key: &str| d.get(key).cloned().or_else(|| syntax.example());
        match self.origin {
            Edge::Node(r) => {
                let v = r.to_value(order, like(d, "origin").as_ref());
                put_if_changed(d, "origin", v, |x| NodeRef::from_value(x, order) == Some(r));
            }
            Edge::Value(_) => remove(d, "origin"),
        }
        match self.kind {
            PsHintKind::Stem => match self.other {
                Some(Edge::Node(r)) => {
                    let v = r.to_value(order, like(d, "target").as_ref());
                    put_if_changed(d, "target", v, |x| NodeRef::from_value(x, order) == Some(r));
                }
                _ => remove(d, "target"),
            },
            PsHintKind::GhostTop => put_if_changed(d, "target", Value::String("down".into()), |x| x.as_str() == Some("down")),
            PsHintKind::GhostBottom => put_if_changed(d, "target", Value::String("up".into()), |x| x.as_str() == Some("up")),
        }
        let place = match (self.kind, self.origin, self.other) {
            (PsHintKind::Stem, Edge::Value(a), Some(Edge::Value(b))) => Some((a, b - a)),
            (PsHintKind::Stem, Edge::Value(a), _) => Some((a, 0.0)),
            (PsHintKind::Stem, Edge::Node(_), Some(Edge::Value(b))) => Some((b, 0.0)),
            (PsHintKind::Stem, Edge::Node(_), _) => None,
            (ghost, Edge::Value(a), _) => {
                let reserved = if ghost == PsHintKind::GhostTop { TOP_GHOST_WIDTH } else { BOTTOM_GHOST_WIDTH };
                // The stored width of the same ghost kind is kept.
                let kept = (old.map(|o| o.kind) == Some(ghost)).then(|| d.get("place").and_then(read_numbers)).flatten();
                Some((a, kept.and_then(|p| p.get(1).copied()).unwrap_or(reserved)))
            }
            (_, Edge::Node(_), _) => None,
        };
        match place {
            Some((p, w)) => {
                let same = |x: &Value| read_numbers(x).is_some_and(|n| n.len() == 2 && n.first() == Some(&p) && n.get(1) == Some(&w));
                put_if_changed(d, "place", syntax.pair(p, w), same);
            }
            None => remove(d, "place"),
        }
    }
}

/// The PostScript kind a stored hint's `type` names.
fn ps_kind(d: &Dict) -> Option<PsHintKind> {
    let t = d.get("type")?;
    if type_is(t, &["Stem"], &[0]) {
        Some(PsHintKind::Stem)
    } else if type_is(t, &["TopGhost"], &[-1]) {
        Some(PsHintKind::GhostTop)
    } else if type_is(t, &["BottomGhost"], &[1]) {
        Some(PsHintKind::GhostBottom)
    } else {
        None
    }
}

/// Whether a stored hint is a PostScript hint (by its type alone).
pub(crate) fn is_ps(hint: &Hint) -> bool {
    hint.value.as_dict().and_then(ps_kind).is_some()
}

fn read(index: usize, d: &Dict, kind: PsHintKind, order: &NodeOrder) -> Result<PsHint, HintingError> {
    let bad = |message: &str| HintingError::BadHint { index, message: message.to_string() };
    let horizontal = match d.get("horizontal") {
        Some(v) => read_flag(v).ok_or_else(|| bad("`horizontal` is not a flag"))?,
        None => false,
    };
    let place = match d.get("place") {
        Some(v) => match read_numbers(v).as_deref() {
            Some([p, w]) => Some((*p, *w)),
            _ => return Err(bad("`place` is not a pair of numbers")),
        },
        None => None,
    };
    let origin = match d.get("origin") {
        Some(v) => Edge::Node(NodeRef::from_value(v, order).ok_or_else(|| bad("`origin` is not a node reference"))?),
        None => Edge::Value(place.map(|p| p.0).ok_or_else(|| bad("the hint has neither `origin` nor `place`"))?),
    };
    let other = match kind {
        PsHintKind::Stem => Some(match d.get("target") {
            Some(v) => Edge::Node(NodeRef::from_value(v, order).ok_or_else(|| bad("`target` is not a node reference"))?),
            None => {
                let (p, w) = place.ok_or_else(|| bad("the stem hint has neither `target` nor `place`"))?;
                let edge = p + w;
                if !edge.is_finite() {
                    return Err(bad("the stem hint's width is not a usable number"));
                }
                Edge::Value(edge)
            }
        }),
        PsHintKind::GhostTop | PsHintKind::GhostBottom => None,
    };
    Ok(PsHint { horizontal, kind, origin, other, source: None })
}

/// The layer's PostScript hints in stored order, each with its index in
/// [`LayerInfo::hints`]. Node references count in the model's order (`noderef` module doc).
pub fn ps_hints(layer: &Layer) -> Result<Vec<(usize, PsHint)>, HintingError> {
    let info = &layer.info;
    if info.hints.len() > MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "hints on a layer", limit: MAX_LAYER_HINTS });
    }
    let order = NodeOrder::of(layer);
    info.hints.iter().enumerate().filter_map(|(i, h)| PsHint::from_hint(i, h, &order).map(|r| r.map(|p| (i, p)))).collect()
}

/// Replaces the layer's PostScript hints by `hints`, in order, at the place of the first old one
/// (at the end when there was none). TrueType instructions and other stored hints keep their
/// order. More than [`MAX_LAYER_HINTS`] in all is an error and changes nothing.
///
/// New hints (no [`PsHint::source`]) take the family and syntax of the layer's stored hints, else
/// format 3. Edges that are numbers must be finite; a ghost must be horizontal. Node references
/// count in the model's order on `layer`'s paths as they are.
pub fn set_ps_hints(layer: &mut Layer, hints: &[PsHint]) -> Result<(), HintingError> {
    let order = NodeOrder::of(layer);
    let info = &mut layer.info;
    for (i, h) in hints.iter().enumerate() {
        let edges = [Some(h.origin), h.other];
        if edges.iter().flatten().any(|e| matches!(e, Edge::Value(v) if !v.is_finite())) {
            return Err(HintingError::BadNumber { what: format!("an edge of PostScript hint {i}") });
        }
        if h.kind != PsHintKind::Stem && !h.horizontal {
            return Err(HintingError::BadHint { index: i, message: "a ghost hint is always horizontal".to_string() });
        }
        if h.kind == PsHintKind::Stem && h.other.is_none() {
            return Err(HintingError::BadHint { index: i, message: "a stem hint needs two edges".to_string() });
        }
    }
    let (family, syntax) = Syntax::of_layer(info);
    let new: Vec<Hint> =
        hints.iter().map(|h| h.to_hint_in(family, h.source.as_ref().map(|v| Syntax::of_hint(family, v)).unwrap_or(syntax), &order)).collect();
    splice(info, is_ps, new)
}

/// Replaces the stored hints `mine` selects by `new`, at the place of the first of them (at the
/// end when there is none); the others keep their order. Changes nothing on error.
pub(crate) fn splice(info: &mut LayerInfo, mine: fn(&Hint) -> bool, new: Vec<Hint>) -> Result<(), HintingError> {
    let others = info.hints.iter().filter(|h| !mine(h)).count();
    if others + new.len() > MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "hints on a layer", limit: MAX_LAYER_HINTS });
    }
    let mut out = Vec::with_capacity(others + new.len());
    let mut new = Some(new);
    for h in info.hints.drain(..) {
        if mine(&h) {
            if let Some(n) = new.take() {
                out.extend(n);
            }
        } else {
            out.push(h);
        }
    }
    if let Some(n) = new {
        out.extend(n);
    }
    info.hints = out;
    Ok(())
}

/// Whether a layer holds any PostScript hint.
pub fn has_ps_hints(layer: &Layer) -> bool {
    layer.info.hints.iter().any(is_ps)
}

/// The layer whose PostScript hints `glyph` uses where `built` (one of its layers) is exported:
///
/// - a special layer (Alternate, Intermediate…) uses its own hints (`hinting.ps.masters.specialLayers`);
/// - a master layer uses the main master's layer (`hinting.ps.masters.mainMasterOnly`); when that
///   has no PostScript hints, the first other master's layer in master order that has some
///   (`hinting.ps.masters.fallbackToOtherMasters`; the order is a guess on that row).
///
/// `None`: no manual hints apply, so the autohinter decides.
pub fn ps_hint_layer<'a>(font: &Font, glyph: &'a Glyph, settings: &HintSettings, built: &'a Layer) -> Option<&'a Layer> {
    if !built.is_master_layer() {
        return has_ps_hints(built).then_some(built);
    }
    if let Some(main) = glyph.layer(&settings.main_master).filter(|l| has_ps_hints(l)) {
        return Some(main);
    }
    font.masters.iter().filter(|m| m.id != settings.main_master).filter_map(|m| glyph.layer(&m.id)).find(|l| has_ps_hints(l))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::openstep::{self, Dialect};
    use fontcraft_model::{Family, Font, Glyph, Hint, Layer, Master};

    use super::*;
    use crate::HintSettings;

    fn hints(text: &str) -> Vec<Hint> {
        let v = openstep::parse_value(text).unwrap();
        v.as_array().unwrap().items.iter().map(|value| Hint { family: Family::Glyphs, value: value.clone() }).collect()
    }

    fn text(v: &Value) -> String {
        let mut out = String::new();
        openstep::write_value(&mut out, v, Dialect::Glyphs3);
        out
    }

    fn info(hints: Vec<Hint>) -> Layer {
        let mut layer = Layer::new("m01", 500.0);
        layer.info.hints = hints;
        layer
    }

    // The ghost and stem forms of the format 3 sample (`alef-ar`, `A`), plus a non-hint.
    const FORMAT3: &str = "(
{
horizontal = 1;
origin = (0,3);
target = down;
type = TopGhost;
},
{
horizontal = 1;
place = (19,21);
target = up;
type = BottomGhost;
},
{
horizontal = 1;
place = (-46,58);
type = Stem;
},
{
place = (165,101);
type = Stem;
},
{
horizontal = 1;
origin = (0,0);
target = (0,7);
type = Stem;
},
{
name = _corner.cut;
options = 1;
origin = (0,7);
scale = (1.1,0.9);
type = Corner;
}
)";

    #[test]
    fn format3_stem_and_ghost_forms_read_and_write_back_unchanged() {
        let stored = hints(FORMAT3);
        let typed = ps_hints(&info(stored.clone())).unwrap();
        assert_eq!(typed.len(), 5, "the corner component is not a PostScript hint");
        let h: Vec<&PsHint> = typed.iter().map(|(_, h)| h).collect();
        assert_eq!((h[0].kind, h[0].origin, h[0].other), (PsHintKind::GhostTop, Edge::Node(NodeRef::new(0, 3)), None));
        assert_eq!((h[1].kind, h[1].origin), (PsHintKind::GhostBottom, Edge::Value(19.0)));
        assert_eq!((h[2].horizontal, h[2].origin, h[2].other), (true, Edge::Value(-46.0), Some(Edge::Value(12.0))));
        assert_eq!((h[3].horizontal, h[3].origin, h[3].other), (false, Edge::Value(165.0), Some(Edge::Value(266.0))));
        assert_eq!(h[4].other, Some(Edge::Node(NodeRef::new(0, 7))));
        for (i, hint) in &typed {
            let back = hint.to_hint(Family::Glyphs, &NodeOrder::default());
            assert_eq!(text(&back.value), text(&stored[*i].value), "hint {i} is written back byte for byte");
        }
        // Writing the whole list back leaves the layer as it was.
        let mut layer = info(stored.clone());
        let list: Vec<PsHint> = typed.into_iter().map(|(_, h)| h).collect();
        set_ps_hints(&mut layer, &list).unwrap();
        assert_eq!(layer.info.hints, stored);
    }

    /// The format 3 sample's `A`: the file writes the closed path's start node (10, 608) last,
    /// the model keeps it first.
    #[test]
    fn stored_indexes_count_in_file_order_and_typed_ones_in_model_order() {
        use fontcraft_model::{Node, NodeKind, Path, Shape};

        use crate::TtPoint;

        let (l, o, c) = (NodeKind::Line, NodeKind::OffCurve, NodeKind::Curve);
        let file =
            [(10., 66., l), (92., -21., o), (338., -35., o), (439., 66., c), (578., 205., o), (522., 462., o), (439., 608., c), (10., 608., l)];
        let mut nodes: Vec<Node> = file.iter().map(|&(x, y, k)| Node::new(x, y, k)).collect();
        nodes.rotate_right(1);
        let closed = Path::new(true, nodes);
        let open = Path::new(false, vec![Node::new(100.0, 0.0, l), Node::new(200.0, 0.0, l)]);
        let mut layer =
            info(hints("({horizontal = 1; origin = (0,0); target = (0,7); type = Stem;}, {origin = (1,0); target = (1,1); type = Stem;})"));
        layer.shapes = vec![Shape::Path(closed), Shape::Path(open)];
        let stored = layer.info.hints.clone();

        let typed = ps_hints(&layer).unwrap();
        assert_eq!((typed[0].1.origin, typed[0].1.other), (Edge::Node(NodeRef::new(0, 1)), Some(Edge::Node(NodeRef::new(0, 0)))));
        assert_eq!(typed[0].1.edges(&layer).unwrap(), (66.0, 608.0));
        assert_eq!(typed[1].1.origin, Edge::Node(NodeRef::new(1, 0)), "an open path's indexes agree");
        assert_eq!(typed[1].1.edges(&layer).unwrap(), (100.0, 200.0));

        // Written afresh, the hints store the file's indexes again.
        let fresh: Vec<PsHint> = typed.iter().map(|(_, h)| PsHint { source: None, ..h.clone() }).collect();
        set_ps_hints(&mut layer, &fresh).unwrap();
        let texts = |hints: &[Hint]| hints.iter().map(|h| text(&h.value).replace(char::is_whitespace, "")).collect::<Vec<_>>();
        assert_eq!(texts(&layer.info.hints), texts(&stored));

        // TrueType points, intersections included, convert alike; an index past the path's end
        // stays as stored.
        let order = NodeOrder::of(&layer);
        let v = openstep::parse_value("(0,7,1,1)").unwrap();
        let p = TtPoint::from_value(&v, &order).unwrap();
        assert_eq!(p, TtPoint::Intersection { a: NodeRef::new(0, 0), b: NodeRef::new(1, 1) });
        assert_eq!(text(&p.to_value(&order, None)), "(0,7,1,1)");
        assert_eq!(NodeRef::from_value(&openstep::parse_value("(0,8)").unwrap(), &order), Some(NodeRef::new(0, 8)));
    }

    #[test]
    fn format2_brace_strings_read_and_new_hints_follow_the_layer_syntax() {
        let stored = hints("({horizontal = 1; origin = \"{0, 0}\"; target = \"{0, 3}\"; type = Stem;})");
        let mut layer = info(stored);
        let mut list: Vec<PsHint> = ps_hints(&layer).unwrap().into_iter().map(|(_, h)| h).collect();
        assert_eq!(list[0].origin, Edge::Node(NodeRef::new(0, 0)));
        list.push(PsHint::ghost(true, Edge::Value(778.0)));
        list.push(PsHint::stem(true, Edge::Node(NodeRef::new(1, 2)), Edge::Node(NodeRef::new(1, 5))));
        set_ps_hints(&mut layer, &list).unwrap();
        let ghost = layer.info.hints[1].value.as_dict().unwrap();
        assert_eq!(ghost.get("place").and_then(Value::as_str), Some("{778, -20}"));
        assert_eq!(ghost.get("target").and_then(Value::as_str), Some("down"));
        let stem = layer.info.hints[2].value.as_dict().unwrap();
        assert_eq!(stem.get("origin").and_then(Value::as_str), Some("{1, 2}"));
        let keys: Vec<&str> = stem.entries.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["horizontal", "origin", "target", "type"], "keys in the reference's sorted order");
        let reread: Vec<PsHint> = ps_hints(&layer).unwrap().into_iter().map(|(_, h)| h).collect();
        assert!(reread.iter().zip(&list).all(|(a, b)| a.same_as(b)));
    }

    #[test]
    fn editing_one_edge_rewrites_only_what_changed() {
        let stored = hints("({horizontal = 1; place = (-46,58); type = Stem; zzz = kept;})");
        let mut h = PsHint::from_hint(0, &stored[0], &NodeOrder::default()).unwrap().unwrap();
        h.other = Some(Edge::Value(20.0));
        let d = h.to_hint(Family::Glyphs, &NodeOrder::default()).value.as_dict().cloned().unwrap();
        assert_eq!(text(d.get("place").unwrap()), "(-46,66)");
        assert_eq!(d.get("zzz").and_then(Value::as_str), Some("kept"));
        // A linked edge whose node is gone becomes a number; the other edge stays linked.
        let stored = hints("({horizontal = 1; origin = (0,0); target = (0,7); type = Stem;})");
        let mut h = PsHint::from_hint(0, &stored[0], &NodeOrder::default()).unwrap().unwrap();
        h.other = Some(Edge::Value(120.0));
        let back = h.to_hint(Family::Glyphs, &NodeOrder::default());
        let again = PsHint::from_hint(0, &back, &NodeOrder::default()).unwrap().unwrap();
        assert!(again.same_as(&h), "a stem with one linked and one numeric edge reads back the same");
        assert!(back.value.as_dict().unwrap().get("target").is_none());
    }

    #[test]
    fn set_ps_hints_keeps_other_hints_in_place_and_refuses_bad_edges() {
        let stored = hints(
            "({name = _corner.cut; type = Corner;}, {horizontal = 1; place = (0,10); type = Stem;}, {horizontal = 1; origin = (0,1); type = TTSnap;})",
        );
        let mut layer = info(stored.clone());
        let two = [PsHint::stem(true, Edge::Value(0.0), Edge::Value(10.0)), PsHint::stem(false, Edge::Value(50.0), Edge::Value(90.0))];
        set_ps_hints(&mut layer, &two).unwrap();
        let kinds: Vec<Option<&str>> = layer.info.hints.iter().map(Hint::kind).collect();
        assert_eq!(kinds, [Some("Corner"), Some("Stem"), Some("Stem"), Some("TTSnap")]);

        let before = layer.clone();
        let bad = [PsHint::stem(true, Edge::Value(f64::NAN), Edge::Value(10.0))];
        assert!(matches!(set_ps_hints(&mut layer, &bad), Err(HintingError::BadNumber { .. })));
        let vertical_ghost = [PsHint { horizontal: false, ..PsHint::ghost(true, Edge::Value(700.0)) }];
        assert!(set_ps_hints(&mut layer, &vertical_ghost).is_err());
        assert_eq!(layer, before, "a refused write changes nothing");

        set_ps_hints(&mut layer, &[]).unwrap();
        let kinds: Vec<Option<&str>> = layer.info.hints.iter().map(Hint::kind).collect();
        assert_eq!(kinds, [Some("Corner"), Some("TTSnap")]);
    }

    #[test]
    fn a_dangling_or_unreadable_hint_fails_with_a_message() {
        let stored = hints("({horizontal = 1; origin = (0); type = Stem;})");
        assert!(matches!(ps_hints(&info(stored)), Err(HintingError::BadHint { index: 0, .. })));
        let stored = hints("({horizontal = 1; origin = (99999,99999); target = (0,1); type = Stem;})");
        let h = ps_hints(&info(stored)).unwrap().remove(0).1;
        let layer = Layer::new("m01", 500.0);
        assert_eq!(h.edges(&layer), Err(HintingError::DanglingNode { path: 99999, node: 99999 }));
    }

    #[test]
    fn ufo_family_hints_carry_every_key_glyphslib_reads() {
        let mut layer = info(vec![Hint { family: Family::Ufo, value: openstep::parse_value("{type = Corner; name = x;}").unwrap() }]);
        set_ps_hints(&mut layer, &[PsHint::ghost(false, Edge::Node(NodeRef::new(0, 2)))]).unwrap();
        let h = &layer.info.hints[1];
        assert_eq!(h.family, Family::Ufo);
        let d = h.value.as_dict().unwrap();
        assert_eq!(d.get("horizontal"), Some(&Value::Bool(true)));
        for key in ["name", "options", "stem", "type", "origin", "target"] {
            assert!(d.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn hints_come_from_the_main_master_else_the_first_other_master_with_hints() {
        let mut font = Font::new("T");
        let base = font.masters[0].clone();
        let masters = Arc::make_mut(&mut font.masters);
        for id in ["m02", "m03"] {
            masters.push(Master { id: id.into(), name: id.into(), ..base.clone() });
        }
        let stem = hints("({horizontal = 1; place = (0,10); type = Stem;})");
        let mut g = Glyph::new("e");
        for id in ["m01", "m02", "m03"] {
            let mut l = Layer::new(id, 500.0);
            if id != "m01" {
                l.info.hints = stem.clone();
            }
            g.layers.push(l);
        }
        let mut special = Layer::new("m01", 500.0);
        special.id = "brace".into();
        g.layers.push(special.clone());
        let settings = HintSettings::for_master(&font, "m01").unwrap();
        let built = g.layer("m01").unwrap();
        assert_eq!(ps_hint_layer(&font, &g, &settings, built).map(|l| l.id.as_str()), Some("m02"));
        let built = g.layers.iter().find(|l| l.id == "brace").unwrap();
        assert!(ps_hint_layer(&font, &g, &settings, built).is_none(), "a special layer uses its own hints");
    }
}
