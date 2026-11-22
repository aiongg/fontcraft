//! The kerning part of the Info box, while the edit view shows the edited glyph inside its text.
//!
//! | Control | Runs |
//! |---|---|
//! | kern field of an edge, a number committed | `kerning.setPair` on the sides the locks name: a closed lock gives the glyph's group, an open lock or no group the glyph |
//! | kern field emptied | `kerning.removePair` on the stored pair that applies |
//! | lock, closed | `kerning.addException` for the edited glyph's side of the pair |
//! | lock, open | `kerning.removeException` for that side; a refusal goes to the status line |
//! | in text mode, the preceding glyph's lock and group | `kerning.addException` / `removeException` for its side; `kerning.setRightGroup` (`…LeftGroup` right to left, `…BottomGroup` top to bottom) for its group facing the edited glyph |
//!
//! Every command names its pair, its master and its direction. A kern field without a neighbour is disabled.

use craft::behavior::IconId;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};

use super::widgets::{self, NumberEdit};
use crate::app::Msg;
use crate::app::models::info_box_kerning::{Edge, KernBox, KernEdge, KernLock, KernPair, PairSide};

craft::ui::words! {
    /// The words of the Info box's kerning part.
    pub struct InfoBoxKerningWords {
        /// The part's title.
        title: "fontcraftInfoBoxKerning.title" = "Kerning";
        left_kern: "fontcraftInfoBoxKerning.leftKern" = "Left Kerning";
        right_kern: "fontcraftInfoBoxKerning.rightKern" = "Right Kerning";
        top_kern: "fontcraftInfoBoxKerning.topKern" = "Top Kerning";
        bottom_kern: "fontcraftInfoBoxKerning.bottomKern" = "Bottom Kerning";
        left_lock: "fontcraftInfoBoxKerning.leftLock" = "Left Kerning Lock";
        right_lock: "fontcraftInfoBoxKerning.rightLock" = "Right Kerning Lock";
        top_lock: "fontcraftInfoBoxKerning.topLock" = "Top Kerning Lock";
        bottom_lock: "fontcraftInfoBoxKerning.bottomLock" = "Bottom Kerning Lock";
        /// The lock of the glyph before the edited one, in text mode.
        preceding_lock: "fontcraftInfoBoxKerning.precedingLock" = "Preceding Glyph Lock";
        /// The group of the glyph before the edited one that faces it.
        preceding_group: "fontcraftInfoBoxKerning.precedingGroup" = "Preceding Glyph Group";
        /// Tooltip of a closed lock.
        closed: "fontcraftInfoBoxKerning.closed" = "Kerning goes to the group; click to make this glyph an exception";
        /// Tooltip of an open lock.
        open: "fontcraftInfoBoxKerning.open" = "This glyph is an exception; click to kern it through its group again";
        /// Tooltip of a lock without a group.
        ungrouped: "fontcraftInfoBoxKerning.ungrouped" = "No kerning group on this side";
    }
}

/// Shows the kerning part of `k` and pushes what the person did.
pub fn view(ui: &mut Ui, k: &KernBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<InfoBoxKerningWords>();
    Section::new(words.title.clone()).show(ui, |ui, _| {
        let _ = Row::new().show(ui, |ui, _| {
            for (index, edge) in k.edges.iter().enumerate() {
                let (kern_label, lock_label) = edge_labels(&words, edge.edge);
                // The lock sits on the outer side of its kern field.
                if index == 0 {
                    lock(ui, &words, lock_label, k, edge.pair.as_ref(), edge.own, sent);
                    kern(ui, kern_label, k, edge, sent);
                } else {
                    kern(ui, kern_label, k, edge, sent);
                    lock(ui, &words, lock_label, k, edge.pair.as_ref(), edge.own, sent);
                }
            }
        });
        if let Some(p) = &k.preceding {
            preceding(ui, &words, k, p, sent);
        }
    });
}

fn edge_labels(words: &InfoBoxKerningWords, edge: Edge) -> (Label, Label) {
    match edge {
        Edge::Left => (words.left_kern.clone(), words.left_lock.clone()),
        Edge::Right => (words.right_kern.clone(), words.right_lock.clone()),
        Edge::Top => (words.top_kern.clone(), words.top_lock.clone()),
        Edge::Bottom => (words.bottom_kern.clone(), words.bottom_lock.clone()),
    }
}

/// The parameters of a kerning command: `fields` with the box's master and direction.
fn params(k: &KernBox, fields: Value) -> Value {
    let mut out = k.place();
    if let (Value::Object(out), Value::Object(fields)) = (&mut out, fields) {
        out.extend(fields);
    }
    out
}

fn kern(ui: &mut Ui, label: Label, k: &KernBox, edge: &KernEdge, sent: &mut Vec<Msg>) {
    let Some(pair) = &edge.pair else {
        let _ = widgets::clearable(ui, NumberField::new(label, Bind::given(None)).enabled(false), None);
        return;
    };
    match widgets::clearable_number(ui, label, pair.value) {
        Some(NumberEdit::Set(v)) => sent.push(Msg::run(
            "kerning.setPair",
            params(k, json!({"left": pair.edited_side(PairSide::Left), "right": pair.edited_side(PairSide::Right), "value": v})),
        )),
        Some(NumberEdit::Clear) => {
            if let Some([left, right]) = &pair.applied {
                sent.push(Msg::run("kerning.removePair", params(k, json!({"left": left, "right": right}))));
            }
        }
        None => {}
    }
}

/// The lock of `side` in `pair`: a closed lock opens with `kerning.addException`, an open one closes with
/// `kerning.removeException`; without a group on that side it is disabled.
fn lock(ui: &mut Ui, words: &InfoBoxKerningWords, label: Label, k: &KernBox, pair: Option<&KernPair>, side: PairSide, sent: &mut Vec<Msg>) {
    let state = pair.map_or(KernLock::Ungrouped, |p| p.lock(side));
    let tip = match state {
        KernLock::Closed => words.closed.clone(),
        KernLock::Open => words.open.clone(),
        KernLock::Ungrouped => words.ungrouped.clone(),
    };
    let enabled = state != KernLock::Ungrouped;
    let closed = state != KernLock::Open;
    let out = ToggleButton::new(label, Bind::given(closed))
        .icon(IconId::new(if closed { "lock" } else { "lock_open" }))
        .icon_only()
        .tooltip(tip)
        .enabled(enabled)
        .show(ui);
    if out.events.committed().is_some()
        && enabled
        && let Some(p) = pair
    {
        let command = if state == KernLock::Open { "kerning.removeException" } else { "kerning.addException" };
        sent.push(Msg::run(command, params(k, json!({"left": p.left, "right": p.right, "side": side.name()}))));
    }
}

/// The glyph before the edited one: its name, its lock in the pair and its group that faces the edited glyph.
fn preceding(ui: &mut Ui, words: &InfoBoxKerningWords, k: &KernBox, p: &KernPair, sent: &mut Vec<Msg>) {
    let _ = Row::new().show(ui, |ui, _| {
        Text::new(p.left.clone()).shade(Shade::Muted).show(ui);
        lock(ui, words, words.preceding_lock.clone(), k, Some(p), PairSide::Left, sent);
        let shown = p.group(PairSide::Left).unwrap_or_default();
        let out = TextField::new(words.preceding_group.clone(), Bind::given(shown.clone())).select_on_focus().show(ui);
        if let Some(text) = out.events.committed()
            && text.as_str() != shown.as_str()
        {
            sent.push(Msg::run(&k.preceding_command, json!({"glyphs": [p.left], "group": text.trim()})));
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::view;
    use crate::app::Msg;
    use crate::app::models::info_box_kerning::{Edge, KernBox, KernEdge, KernLock, KernPair, PairSide};

    fn kern_box() -> KernBox {
        // "T o": the edited glyph is o; T kerns to o through T's group, o is an exception on its left side.
        let pair = KernPair {
            left: "T".into(),
            right: "o".into(),
            value: Some(-40.0),
            applied: Some(["@T".into(), "o".into()]),
            locks: [KernLock::Closed, KernLock::Open],
            groups: [Some("T".into()), Some("o".into())],
        };
        KernBox {
            glyph: "o".into(),
            master: "m01".into(),
            direction: "ltr".into(),
            edges: vec![
                KernEdge { edge: Edge::Left, pair: Some(pair.clone()), own: PairSide::Right },
                KernEdge { edge: Edge::Right, pair: None, own: PairSide::Left },
            ],
            preceding: Some(pair),
            preceding_command: "kerning.setRightGroup".into(),
        }
    }

    fn fixture(k: KernBox) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &k, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        (fixture, posted)
    }

    /// A typed kern goes to the sides the locks name, in the box's master and direction; an emptied field removes the
    /// stored pair that applies.
    #[test]
    fn kern_fields_set_and_remove_pairs() {
        let (mut fixture, posted) = fixture(kern_box());
        let field = fixture.by_label("Left Kerning").expect("the left kern field");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("-55").key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(
            *posted.borrow(),
            vec![Msg::run("kerning.setPair", json!({"left": "@T", "right": "o", "value": -55.0, "master": "m01", "direction": "ltr"}))]
        );
        posted.borrow_mut().clear();
        let field = fixture.by_label("Left Kerning").expect("the left kern field");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).key(Key::Backspace, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("kerning.removePair", json!({"left": "@T", "right": "o", "master": "m01", "direction": "ltr"}))]);
    }

    /// The edited glyph's open lock closes with `kerning.removeException`; the preceding glyph's closed lock opens
    /// with `kerning.addException` on its own side.
    #[test]
    fn locks_open_and_close_exceptions() {
        let (mut fixture, posted) = fixture(kern_box());
        let own = fixture.by_label("Left Kerning Lock").expect("the left lock");
        fixture.click(&own).settle();
        let theirs = fixture.by_label("Preceding Glyph Lock").expect("the preceding glyph's lock");
        fixture.click(&theirs).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("kerning.removeException", json!({"left": "T", "right": "o", "side": "right", "master": "m01", "direction": "ltr"})),
                Msg::run("kerning.addException", json!({"left": "T", "right": "o", "side": "left", "master": "m01", "direction": "ltr"})),
            ]
        );
    }
}
