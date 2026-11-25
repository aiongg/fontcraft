//! The kerning part of the Info box: while the edit view shows the edited glyph inside its text, the kerning of that
//! glyph towards its neighbours, the locks of the glyph in each pair, and in text mode the glyph before it with its
//! lock and the group that faces the edited glyph.
//!
//! Pairs are in logical order, as `kerning.getPair` and the setters take them. The edges are as drawn: left to right,
//! the left edge is the pair with the glyph before; right to left, it is the pair with the glyph after, which is
//! drawn on the left; top to bottom, the first edge is the pair with the glyph above.
//!
//! A pair is read and stored in the master both glyphs are shown in; glyphs shown in different masters have no kerning
//! field between them.

use craft::ui::contract::PlainData;
use serde_json::{Value, json};

use super::ProjectCx;
use crate::canvas::text::{self, WritingDirection};

/// A glyph's lock in a pair, as `kerning.getPair` reports it.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KernLock {
    /// The glyph has no kerning group on that side: there is nothing to lock.
    #[default]
    Ungrouped,
    /// Kerning goes to the glyph's group.
    Closed,
    /// The glyph is an exception: kerning goes to the glyph.
    Open,
}

/// A side of a pair in logical order.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PairSide {
    #[default]
    Left,
    Right,
}

impl PairSide {
    /// The side as the kerning commands spell it.
    pub fn name(self) -> &'static str {
        match self {
            PairSide::Left => "left",
            PairSide::Right => "right",
        }
    }
}

/// Two glyphs next to each other in the text, in logical order, and the kerning between them.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct KernPair {
    pub left: String,
    pub right: String,
    /// The master the pair is read from and stored in.
    pub master: String,
    /// The kerning that applies; `None` when no stored pair applies.
    pub value: Option<f64>,
    /// The stored pair that applies, in command spelling (a glyph name or `@group`).
    pub applied: Option<[String; 2]>,
    pub locks: [KernLock; 2],
    /// Each glyph's kerning group that faces the pair.
    pub groups: [Option<String>; 2],
}

impl KernPair {
    fn at<T: Clone>(pair: &[T; 2], side: PairSide) -> T {
        let [left, right] = pair;
        match side {
            PairSide::Left => left.clone(),
            PairSide::Right => right.clone(),
        }
    }

    pub fn lock(&self, side: PairSide) -> KernLock {
        Self::at(&self.locks, side)
    }

    pub fn group(&self, side: PairSide) -> Option<String> {
        Self::at(&self.groups, side)
    }

    pub fn glyph(&self, side: PairSide) -> &str {
        match side {
            PairSide::Left => &self.left,
            PairSide::Right => &self.right,
        }
    }

    /// The side a typed value is stored on: the group behind a closed lock, else the glyph.
    pub fn edited_side(&self, side: PairSide) -> String {
        match (self.lock(side), self.group(side)) {
            (KernLock::Closed, Some(group)) => format!("@{group}"),
            _ => self.glyph(side).to_string(),
        }
    }

    fn from_reply(left: &str, right: &str, master: &str, reply: &Value) -> KernPair {
        let applies = reply.get("step").is_some_and(|s| !s.is_null());
        let value = reply.get("value").and_then(Value::as_f64).filter(|v| applies && v.is_finite());
        let pair = reply.get("pair");
        let spelled = |k: &str| pair.and_then(|p| p.get(k)).and_then(Value::as_str).map(str::to_string);
        let applied = spelled("left").zip(spelled("right")).map(|(l, r)| [l, r]);
        let lock = |k: &str| match reply.get("locks").and_then(|l| l.get(k)).and_then(Value::as_str) {
            Some("closed") => KernLock::Closed,
            Some("open") => KernLock::Open,
            _ => KernLock::Ungrouped,
        };
        let group = |k: &str| reply.get("groups").and_then(|g| g.get(k)).and_then(Value::as_str).map(str::to_string);
        KernPair {
            left: left.to_string(),
            right: right.to_string(),
            master: master.to_string(),
            value,
            applied,
            locks: [lock("left"), lock("right")],
            groups: [group("left"), group("right")],
        }
    }
}

/// An edge of the edited glyph as drawn.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

/// One edge: the pair with the neighbour drawn there and the edited glyph's side in it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct KernEdge {
    pub edge: Edge,
    /// `None` at the start or the end of a line and towards a glyph shown in another master.
    pub pair: Option<KernPair>,
    pub own: PairSide,
}

/// The kerning part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct KernBox {
    pub glyph: String,
    /// `ltr`, `rtl` or `vertical`, as the kerning commands spell it.
    pub direction: String,
    /// The edge drawn first (left or top), then the other.
    pub edges: Vec<KernEdge>,
    /// In text mode: the pair with the glyph before the edited one, and the command that sets that glyph's group
    /// facing the edited glyph.
    pub preceding: Option<KernPair>,
    pub preceding_command: String,
}

impl KernBox {
    /// The parameters every kerning command on `pair` shares.
    pub fn place(&self, pair: &KernPair) -> Value {
        json!({"master": pair.master, "direction": self.direction})
    }
}

/// The kerning part, when the canvas shows the edited glyph inside the edit view's text.
pub fn kern_box(cx: &mut ProjectCx<'_>) -> Option<KernBox> {
    let font = cx.session.font().ok()?;
    if !text::shows_text(font, cx.view) {
        return None;
    }
    let edit = &cx.view.text;
    let glyph = edit.current()?.to_string();
    let index = edit.edited_index()?;
    // The items of a pair and their master, when both are glyphs shown in it.
    let items = |first: Option<usize>| -> Option<(String, String, String)> {
        let first = first?;
        let second = first.checked_add(1)?;
        let master = edit.item_master(font, &cx.view.layers, first)?;
        let same = edit.item_master(font, &cx.view.layers, second)?.id == master.id;
        same.then_some((edit.item_glyph(first)?.to_string(), edit.item_glyph(second)?.to_string(), master.id.clone()))
    };
    let (before, after) = (items(index.checked_sub(1)), items(Some(index)));
    let writing = edit.direction;
    let direction = writing.layout().name();
    let session = &mut *cx.session;
    let mut pair = |(left, right, master): (String, String, String)| -> Option<KernPair> {
        let query = json!({"left": left, "right": right, "master": master, "direction": direction});
        let reply = session.query("kerning.getPair", &query).ok()?;
        Some(KernPair::from_reply(&left, &right, &master, &reply))
    };
    let before_pair = before.and_then(&mut pair);
    let after_pair = after.and_then(&mut pair);
    let edge = |edge, pair, own| KernEdge { edge, pair, own };
    let (edges, preceding_command) = match writing {
        WritingDirection::Ltr => {
            (vec![edge(Edge::Left, before_pair.clone(), PairSide::Right), edge(Edge::Right, after_pair, PairSide::Left)], "kerning.setRightGroup")
        }
        WritingDirection::Rtl => {
            (vec![edge(Edge::Left, after_pair, PairSide::Left), edge(Edge::Right, before_pair.clone(), PairSide::Right)], "kerning.setLeftGroup")
        }
        WritingDirection::Ttb => {
            (vec![edge(Edge::Top, before_pair.clone(), PairSide::Right), edge(Edge::Bottom, after_pair, PairSide::Left)], "kerning.setBottomGroup")
        }
    };
    let preceding = if text::text_mode(cx.view) { before_pair } else { None };
    Some(KernBox { glyph, direction: direction.to_string(), edges, preceding, preceding_command: preceding_command.into() })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use super::{Edge, KernLock, KernPair, PairSide};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    fn post(app: &mut FontCraftApp, msg: Msg) {
        app.status.clear();
        app.sent.push(msg);
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
        post(app, Msg::Shell(CommandRef::with_params(id, params)));
    }

    /// With "T o A" in the edit view and the cursor before o, the Info box shows o's kerning towards T and A; text
    /// mode adds T's lock and group, and another tool takes them away.
    #[test]
    fn the_info_box_reads_the_pairs_around_the_edited_glyph() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        post(&mut app, Msg::run("kerning.setRightGroup", json!({"glyphs": ["T"], "group": "T"})));
        post(&mut app, Msg::run("kerning.setPair", json!({"left": "@T", "right": "o", "value": -40})));
        post(&mut app, Msg::View(ViewMsg::SelectGlyph("A".into())));
        shell(&mut app, "editView.setText", json!({"glyphs": ["T", "o", "A"]}));
        shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
        shell(&mut app, "editView.setCursor", json!({"index": 1}));
        let k = app.models.palette.info_box.kerning.clone().expect("the kerning part");
        assert_eq!((k.glyph.as_str(), k.direction.as_str()), ("o", "ltr"));
        let left = k.edges[0].pair.clone().unwrap();
        assert_eq!((k.edges[0].edge, k.edges[0].own), (Edge::Left, PairSide::Right));
        assert_eq!((left.left.as_str(), left.value, left.lock(PairSide::Left)), ("T", Some(-40.0), KernLock::Closed));
        assert_eq!(left.edited_side(PairSide::Left), "@T");
        let right = k.edges[1].pair.clone().unwrap();
        assert_eq!((right.left.as_str(), right.right.as_str(), right.value), ("o", "A", None));
        assert_eq!(k.preceding.as_ref().map(|p| p.left.as_str()), Some("T"));

        shell(&mut app, "editView.toggleTextMode", json!({"mode": "edit"}));
        let k = app.models.palette.info_box.kerning.clone();
        assert!(k.is_none_or(|k| k.preceding.is_none()), "the preceding glyph's box belongs to text mode");
    }

    /// A closed lock stores a typed value on the group; an open lock or a glyph without a group, on the glyph.
    #[test]
    fn a_pair_reads_locks_and_stores_on_the_locked_group() {
        let reply = json!({
            "value": -40, "step": "groupGroup", "pair": {"left": "@T", "right": "@o"},
            "locks": {"left": "closed", "right": "open"}, "groups": {"left": "T", "right": "o"},
        });
        let pair = KernPair::from_reply("T", "o", "m01", &reply);
        assert_eq!(pair.value, Some(-40.0));
        assert_eq!(pair.applied, Some(["@T".to_string(), "@o".to_string()]));
        assert_eq!(pair.lock(PairSide::Left), KernLock::Closed);
        assert_eq!(pair.edited_side(PairSide::Left), "@T");
        assert_eq!(pair.edited_side(PairSide::Right), "o");
        let none = KernPair::from_reply("a", "b", "m01", &json!({"value": 0, "step": null, "pair": null}));
        assert_eq!((none.lock(PairSide::Right), none.value, none.applied), (KernLock::Ungrouped, None, None));
    }
}
