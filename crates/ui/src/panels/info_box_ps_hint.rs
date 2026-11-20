//! The Info box of a selected PostScript hint (`infoBox.psHint`), in place of the selection part while Show Hints is
//! on and hints are selected. It shows the first selected hint in list order:
//!
//! | Hint | Fields | Editing runs |
//! |---|---|---|
//! | stem | Origin, Size | `hint.ps.setHintValues` with `origin` or `size` |
//! | ghost | Origin, and a top and a bottom button for its direction | `hint.ps.setHintValues` with `origin`; `hint.ps.setGhostDirection` |
//!
//! A typed value unlinks the edge it sets, as `hint.ps.setHintValues` says. Under the fields, the hint's warnings.

use craft::theme::variant::Tone;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::json;

use super::widgets;
use crate::app::Msg;
use crate::app::models::hint_canvas::{PsHintBox, PsHintKind};

craft::ui::words! {
    /// The words of the PostScript hint Info box.
    pub struct PsHintWords {
        horizontal: "fontcraftInfoBoxPsHint.horizontal" = "Horizontal hint";
        vertical: "fontcraftInfoBoxPsHint.vertical" = "Vertical hint";
        ghost_top: "fontcraftInfoBoxPsHint.ghostTop" = "Top ghost hint";
        ghost_bottom: "fontcraftInfoBoxPsHint.ghostBottom" = "Bottom ghost hint";
        /// After the hint's kind when more than one hint is selected.
        selected: template "fontcraftInfoBoxPsHint.selected" = "{count} selected";
        origin: "fontcraftInfoBoxPsHint.origin" = "Origin";
        size: "fontcraftInfoBoxPsHint.size" = "Size";
        /// The button that makes a ghost hint mark a top edge.
        top: "fontcraftInfoBoxPsHint.top" = "↑ Top";
        /// The button that makes a ghost hint mark a bottom edge.
        bottom: "fontcraftInfoBoxPsHint.bottom" = "↓ Bottom";
    }
}

/// The box of `h`; what the person changed goes to `sent`.
pub fn view(ui: &mut Ui, h: &PsHintBox, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<PsHintWords>();
    let kind = match h.kind {
        PsHintKind::Horizontal => &words.horizontal,
        PsHintKind::Vertical => &words.vertical,
        PsHintKind::GhostTop => &words.ghost_top,
        PsHintKind::GhostBottom => &words.ghost_bottom,
    };
    let count = h.count.to_string();
    let title =
        if h.count > 1 { format!("{} ({})", kind.as_str(), words.selected.fill(&[("count", count.as_str())])) } else { kind.as_str().to_string() };
    let target = |extra: serde_json::Value| {
        let mut p = json!({"glyph": h.glyph, "layer": h.layer});
        if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        p
    };
    Section::new(title).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                if let Some(v) = widgets::number(ui, words.origin.clone(), h.origin) {
                    sent.push(Msg::run("hint.ps.setHintValues", target(json!({"hint": h.index, "origin": v}))));
                }
                let ghost = matches!(h.kind, PsHintKind::GhostTop | PsHintKind::GhostBottom);
                if ghost {
                    for (label, kind, name) in [(&words.top, PsHintKind::GhostTop, "top"), (&words.bottom, PsHintKind::GhostBottom, "bottom")] {
                        let on = h.kind == kind;
                        let out = ToggleButton::new(label.clone(), Bind::given(on)).tooltip(label.clone()).show(ui);
                        if out.events.committed().is_some() && !on {
                            sent.push(Msg::run("hint.ps.setGhostDirection", target(json!({"hints": [h.index], "direction": name}))));
                        }
                    }
                } else if let Some(v) = widgets::number(ui, words.size.clone(), h.size) {
                    sent.push(Msg::run("hint.ps.setHintValues", target(json!({"hint": h.index, "size": v}))));
                }
            });
            for w in &h.warnings {
                Text::new(w.clone()).tone(Tone::Warn).show(ui);
            }
        });
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};

    use super::*;

    fn fixture(h: PsHintBox) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &h, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        (fixture, posted)
    }

    fn hint(kind: PsHintKind) -> PsHintBox {
        PsHintBox {
            glyph: "A".into(),
            layer: "m01".into(),
            index: 2,
            kind,
            origin: Some(100.0),
            size: Some(20.0),
            count: 1,
            warnings: vec!["not on a node".into()],
        }
    }

    #[test]
    fn a_typed_size_sets_the_hint_and_a_ghost_flips_with_its_buttons() {
        let (mut f, posted) = fixture(hint(PsHintKind::Horizontal));
        assert!(f.by_label("not on a node").is_some(), "the warning shows");
        let size = f.by_label("Size").expect("the Size field");
        f.click(&size).key(Key::A, Modifiers::COMMAND).type_text("30").key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("hint.ps.setHintValues", json!({"glyph": "A", "layer": "m01", "hint": 2, "size": 30.0}))]);

        let (mut f, posted) = fixture(PsHintBox { size: None, ..hint(PsHintKind::GhostTop) });
        assert!(f.by_label("Size").is_none(), "a ghost has no size");
        let bottom = f.by_label("↓ Bottom").expect("the bottom button");
        f.click(&bottom).settle();
        assert_eq!(
            *posted.borrow(),
            vec![Msg::run("hint.ps.setGhostDirection", json!({"glyph": "A", "layer": "m01", "hints": [2], "direction": "bottom"}))]
        );
    }
}
