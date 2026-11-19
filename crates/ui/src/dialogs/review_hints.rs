//! The Review Hints dialog (`hint.ps.reviewHints`): steps through the PostScript hints of one direction of a glyph's
//! layer, one at a time, for inspection and numeric correction.
//!
//! | Control | Does |
//! |---|---|
//! | Horizontal / Vertical | `hint.ps.reviewHints {direction}`: the first hint of that direction becomes the reviewed one |
//! | Position field | `hint.ps.setHintValues {origin}`: the origin edge |
//! | Width field | `hint.ps.setHintValues {size}`: the signed distance to the other edge; empty for a ghost hint |
//! | Previous / Next | `hint.ps.reviewHints {step}`; off at the first and the last |
//! | Create | `hint.ps.addHorizontalHint` or `hint.ps.addVerticalHint` without nodes; the new hint, at the end of the list, becomes the reviewed one |
//! | Remove | `hint.ps.deleteHints` on the reviewed hint; the next hint of the direction, else the previous one, becomes the reviewed one |
//!
//! The shell command keeps the state (`HintingView::review`) and turns on Show Hints; this module opens the dialog
//! while the state says it is open, reads the reviewed layer each pass, and posts what the controls queued.

use craft::ui::buttons::Button;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;
use fontcraft_engine::hinting::ps_hints;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::canvas::hinting::ReviewState;
use crate::canvas::hinting::commands::{hints_of, layer, neighbour};
use crate::panels::widgets;

craft::ui::words! {
    /// The words of the Review Hints dialog.
    pub struct ReviewWords {
        title: "fontcraftReviewHints.title" = "Review Hints";
        horizontal: "fontcraftReviewHints.horizontal" = "Horizontal";
        vertical: "fontcraftReviewHints.vertical" = "Vertical";
        position: "fontcraftReviewHints.position" = "Position";
        width: "fontcraftReviewHints.width" = "Width";
        previous: "fontcraftReviewHints.previous" = "Previous";
        next: "fontcraftReviewHints.next" = "Next";
        create: "fontcraftReviewHints.create" = "Create";
        remove: "fontcraftReviewHints.remove" = "Remove";
        done: "fontcraftReviewHints.done" = "Done";
        /// Before the reviewed hint's place among the hints of the direction.
        hint: template "fontcraftReviewHints.hint" = "Hint {at} of {count}";
        none: "fontcraftReviewHints.none" = "The layer has no hints in this direction.";
    }
}

/// What the dialog shows, read from the font each pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Review {
    pub state: ReviewState,
    /// The reviewed hint's place among the direction's hints (from 1) and their count.
    pub at: usize,
    pub count: usize,
    /// Hints in the layer's whole list: a new hint's position.
    pub total: usize,
    pub origin: Option<f64>,
    /// `None` for a ghost.
    pub size: Option<f64>,
    pub previous: Option<usize>,
    pub next: Option<usize>,
}

impl Review {
    /// The dialog's picture of `state` in `font`; `None` when the glyph or layer is gone.
    pub fn read(font: &Font, state: &ReviewState) -> Option<Review> {
        let l = layer(font, &state.glyph, &state.layer).ok()?;
        let total = ps_hints(l).map(|h| h.len()).unwrap_or(0);
        let list = hints_of(l, state.horizontal).ok()?;
        let reviewed = state.hint.and_then(|h| list.iter().position(|(i, _)| *i == h).map(|at| (at, h)));
        let (origin, size) = reviewed
            .and_then(|(at, _)| list.get(at))
            .map(|(_, h)| {
                let origin = h.origin.position(l, h.horizontal).ok();
                let size = match (&h.other, origin) {
                    (Some(other), Some(o)) => other.position(l, h.horizontal).ok().map(|v| v - o),
                    _ => None,
                };
                (origin, size)
            })
            .unwrap_or((None, None));
        Some(Review {
            state: state.clone(),
            at: reviewed.map_or(0, |(at, _)| at + 1),
            count: list.len(),
            total,
            origin,
            size,
            previous: neighbour(&list, state.hint, false),
            next: neighbour(&list, state.hint, true),
        })
    }
}

fn shell(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("hint.ps.reviewHints", params))
}

/// The commands of the dialog's controls.
pub fn view(ui: &mut Ui, r: &Review, sent: &mut Vec<Msg>) {
    let w = Env::of(ui).words::<ReviewWords>();
    let st = &r.state;
    let target = |extra: Value| {
        let mut p = json!({"glyph": st.glyph, "layer": st.layer});
        if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        p
    };
    Stack::new().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            for (label, horizontal, name) in [(&w.horizontal, true, "horizontal"), (&w.vertical, false, "vertical")] {
                let on = st.horizontal == horizontal;
                if ToggleButton::new(label.clone(), Bind::given(on)).show(ui).events.committed().is_some() && !on {
                    sent.push(shell(json!({"glyph": st.glyph, "layer": st.layer, "direction": name})));
                }
            }
        });
        let hint = st.hint.filter(|_| r.at > 0);
        match hint {
            Some(index) => {
                let (at, count) = (r.at.to_string(), r.count.to_string());
                Text::new(w.hint.fill(&[("at", at.as_str()), ("count", count.as_str())]).to_string()).show(ui);
                Row::new().show(ui, |ui, _| {
                    if let Some(v) = widgets::number(ui, w.position.clone(), r.origin) {
                        sent.push(Msg::run("hint.ps.setHintValues", target(json!({"hint": index, "origin": v}))));
                    }
                    if let Some(v) = widgets::number(ui, w.width.clone(), r.size) {
                        sent.push(Msg::run("hint.ps.setHintValues", target(json!({"hint": index, "size": v}))));
                    }
                });
            }
            None => _ = Text::new(w.none.clone()).shade(Shade::Muted).show(ui),
        }
        Row::new().show(ui, |ui, _| {
            if Button::new(w.previous.clone()).enabled(r.previous.is_some()).show(ui).clicked() {
                sent.push(shell(json!({"step": "previous"})));
            }
            if Button::new(w.next.clone()).enabled(r.next.is_some()).show(ui).clicked() {
                sent.push(shell(json!({"step": "next"})));
            }
            if Button::new(w.create.clone()).show(ui).clicked() {
                let id = if st.horizontal { "hint.ps.addHorizontalHint" } else { "hint.ps.addVerticalHint" };
                sent.push(Msg::run(id, target(json!({}))));
                sent.push(shell(json!({"glyph": st.glyph, "layer": st.layer, "hint": r.total})));
            }
            if Button::new(w.remove.clone()).enabled(hint.is_some()).show(ui).clicked()
                && let Some(index) = hint
            {
                sent.push(Msg::run("hint.ps.deleteHints", target(json!({"hints": [index]}))));
                // The hints after the removed one move up one place.
                let then = match (r.next, r.previous) {
                    (Some(next), _) => json!({"glyph": st.glyph, "layer": st.layer, "hint": next.saturating_sub(1)}),
                    (None, Some(previous)) => json!({"glyph": st.glyph, "layer": st.layer, "hint": previous}),
                    (None, None) => json!({"glyph": st.glyph, "layer": st.layer, "direction": if st.horizontal { "horizontal" } else { "vertical" }}),
                };
                sent.push(shell(then));
            }
        });
    });
}

/// The open dialog on the window's host.
pub struct ReviewDialog {
    pub review: Option<Review>,
    pub outbox: Vec<Msg>,
    open: bool,
}

impl StackDialog for ReviewDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftReviewHints")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let Some(review) = self.review.clone() else { return Some(DialogResult::Cancel) };
        let words = Env::of(ui).words::<ReviewWords>();
        let mut sent = Vec::new();
        let shown = Dialog::new(words.title.clone())
            .opened(Bind::bound(&mut self.open))
            .buttons([FooterButton::new(words.done.clone(), DialogRole::Accept)])
            .show(ui, |ui, _| view(ui, &review, &mut sent));
        self.outbox.extend(sent);
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// Opens the dialog while the review state is open, refreshes it, posts what it queued and records its closing.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<ReviewDialog>().is_some() {
        app.view.hinting.review.open = false;
        app.view_changed();
    }
    let state = app.view.hinting.review.clone();
    if !state.open {
        if let Some(dialog) = host.get_mut::<ReviewDialog>() {
            dialog.review = None;
        }
        return;
    }
    if host.get_mut::<ReviewDialog>().is_none() {
        let _ = host.open(ReviewDialog { review: None, outbox: Vec::new(), open: true });
    }
    let review = app.session.font().ok().and_then(|font| Review::read(font, &state));
    if let Some(dialog) = host.get_mut::<ReviewDialog>() {
        dialog.review = review;
        app.sent.append(&mut dialog.outbox);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    #[test]
    fn review_hints_steps_creates_and_removes_through_commands() {
        let r = Review {
            state: ReviewState { open: true, glyph: "A".into(), layer: "m01".into(), horizontal: true, hint: Some(2) },
            at: 2,
            count: 3,
            total: 4,
            origin: Some(340.0),
            size: Some(20.0),
            previous: Some(0),
            next: Some(3),
        };
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &r, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.settle();
        assert!(f.by_label("Hint 2 of 3").is_some());
        for label in ["Next", "Create", "Remove", "Vertical"] {
            let button = f.by_label(label).expect(label);
            f.click(&button).settle();
        }
        assert_eq!(
            *posted.borrow(),
            vec![
                shell(json!({"step": "next"})),
                Msg::run("hint.ps.addHorizontalHint", json!({"glyph": "A", "layer": "m01"})),
                shell(json!({"glyph": "A", "layer": "m01", "hint": 4})),
                Msg::run("hint.ps.deleteHints", json!({"glyph": "A", "layer": "m01", "hints": [2]})),
                shell(json!({"glyph": "A", "layer": "m01", "hint": 2})),
                shell(json!({"glyph": "A", "layer": "m01", "direction": "vertical"})),
            ]
        );
    }
}
