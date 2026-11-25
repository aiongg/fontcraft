//! Presses, drags and clicks on a painted picture, for a small graph a dialog edits in place (a curve's points, a
//! mapping's points) where a whole canvas with a tool would be too much.
//!
//! Meant for craft-ui-widgets, `src/display/picture.rs`: a `pointer` prop on `Picture` that makes the picture sense
//! presses (`Sense::PRESS`), runs them through the press lifecycle (the drag threshold, capture, a cancel on Escape)
//! and reports them in its output as picture events in the picture's own rect. `Picture` senses hover only and its
//! output is a plain `Response`, so this stand-in senses a click or a drag on the shown picture's rect right after it
//! showed, and reads the pointer from egui. The picture keeps its role and label; the stand-in adds no node to the
//! AccessKit tree, so the keyboard way to the same edits stays the caller's (fields beside the picture).

use egui::{Pos2, Rect, Response, Sense, Ui};

/// What the pointer did on a picture in one pass. Positions are in screen points, inside or near [`rect`](Self::rect).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PicturePointer {
    /// The picture's rect: the one its painter painted into.
    pub rect: Rect,
    /// A press became a drag this pass: where the press was.
    pub drag_started: Option<Pos2>,
    /// A drag goes on: where the pointer is now. Also set in the pass the drag starts.
    pub dragging: Option<Pos2>,
    /// A drag ended this pass: where the pointer was let go.
    pub drag_ended: Option<Pos2>,
    /// A press and release without a drag: where it was.
    pub clicked: Option<Pos2>,
    /// A press that began on the picture is still down, wherever the pointer is now.
    pub held: bool,
}

/// The pointer on a shown picture.
pub trait PointerOnPicture {
    /// Senses a click or a drag on the picture this response belongs to and reports it. Call it every pass right after
    /// the picture showed: a drag is followed only while it is asked about.
    fn pointer(&self, ui: &Ui) -> PicturePointer;
}

impl PointerOnPicture for Response {
    fn pointer(&self, ui: &Ui) -> PicturePointer {
        // Not focusable: a focusable rect would be a stop of the Tab key and a node without a name.
        let sensed = ui.interact(self.rect, self.id.with("craft_ext.picture.pointer"), Sense::CLICK | Sense::DRAG);
        let latest = || ui.ctx().input(|input| input.pointer.latest_pos());
        let at = sensed.interact_pointer_pos().or_else(latest);
        let origin = ui.ctx().input(|input| input.pointer.press_origin()).or(at);
        PicturePointer {
            rect: self.rect,
            drag_started: origin.filter(|_| sensed.drag_started()),
            dragging: at.filter(|_| sensed.dragged()),
            drag_ended: at.filter(|_| sensed.drag_stopped()),
            clicked: at.filter(|_| sensed.clicked()),
            held: sensed.is_pointer_button_down_on(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Show as _;
    use craft::ui::display::{Picture, PictureSize};
    use craft::ui::list::{ThumbAspect, ThumbSize};
    use craft::ui::testing::Fixture;
    use egui::vec2;

    use super::*;

    /// A click is reported where it was; a drag reports its start, the pointer while it moves, and its end.
    #[test]
    fn clicks_and_drags_on_a_picture_are_reported() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let shown = Picture::painter("Graph", ThumbAspect::SQUARE, |_, _| {}).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui);
            let pointer = shown.pointer(ui);
            let quiet = PicturePointer { drag_started: None, dragging: None, drag_ended: None, clicked: None, ..pointer };
            if pointer != quiet {
                out.borrow_mut().push(pointer);
            }
        });
        fixture.settle();
        let graph = fixture.by_label("Graph").expect("the picture");
        let middle = graph.center();
        fixture.click_at(middle).settle();
        assert!(seen.borrow().iter().any(|p| p.clicked.is_some_and(|at| at.distance(middle) < 1.0)), "{:?}", seen.borrow());
        assert!(seen.borrow().iter().all(|p| p.drag_started.is_none()), "a click is no drag");

        seen.borrow_mut().clear();
        let from = graph.rect.left_top() + vec2(4.0, 4.0);
        fixture.drag_at(from, vec2(40.0, 30.0)).settle();
        let seen = seen.borrow();
        assert!(seen.iter().any(|p| p.drag_started.is_some_and(|at| at.distance(from) < 1.0)), "{seen:?}");
        assert!(seen.iter().any(|p| p.dragging.is_some()), "{seen:?}");
        let end = from + vec2(40.0, 30.0);
        assert!(seen.iter().any(|p| p.drag_ended.is_some_and(|at| at.distance(end) < 1.0)), "{seen:?}");
        assert!(seen.iter().all(|p| p.clicked.is_none()), "a drag is no click");
    }
}
