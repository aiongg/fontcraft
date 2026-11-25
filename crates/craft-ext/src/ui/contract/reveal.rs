//! Bringing a rect into view in the scroll body around it, for content whose place of interest is no component of its
//! own (a line of a long text field).
//!
//! Meant for craft-ui-contract, `src/contract/`: craft reveals a component for focus or assistive technology, and
//! `Focus::reveal` lets a component ask for any rect. This stand-in is an invisible component shown at the rect, which
//! asks for itself.

use craft::behavior::{Input, Metrics, Sense};
use craft::ui::contract::{Common, Component, Cx, Focus as _, Props as _, Role, Show as _};
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Rect, Response, Ui};

/// The style of a reveal: no look.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RevealRectStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// An invisible component that asks the scroll body around it to bring its own rect into view.
#[derive(craft::ui::contract::Component)]
#[component(unregistered)]
struct RevealRect {
    #[common]
    common: Common,
}

impl Component for RevealRect {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = RevealRectStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RevealRectStyleKey) -> RevealRectStyle {
        RevealRectStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let rect = cx.rect;
        cx.reveal(rect);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Asks the scroll body around `ui` to bring `rect` (screen points, inside the body's content) into view by the least
/// move that shows it; the bodies around that one follow. Takes no room. Call it in the pass that wants the move.
pub fn reveal_rect(ui: &mut Ui, rect: Rect) {
    if rect.is_finite() {
        RevealRect { common: Common::new() }.id_salt("craftExt.reveal").show_at(ui, rect);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use craft::ui::contract::{Scroll, ShowContainer as _};
    use craft::ui::layout_components::Stack;
    use craft::ui::testing::Fixture;
    use egui::{Sense, vec2};

    use super::*;

    /// A rect far down a scroll body is in the view after it was asked for.
    #[test]
    fn a_rect_outside_the_view_is_scrolled_to() {
        let ask = Rc::new(Cell::new(false));
        let top = Rc::new(Cell::new(0.0f32));
        let (asked, seen) = (Rc::clone(&ask), Rc::clone(&top));
        let mut fixture = Fixture::new(move |ui| {
            Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| {
                let (rect, _) = ui.allocate_exact_size(vec2(50.0, 4_000.0), Sense::hover());
                let place = Rect::from_min_size(rect.min + vec2(0.0, 3_000.0), vec2(10.0, 10.0));
                seen.set(place.top());
                if asked.get() {
                    reveal_rect(ui, place);
                    asked.set(false);
                }
            });
        });
        fixture.room(vec2(300.0, 200.0)).settle();
        assert!(top.get() > 1_000.0, "far below the view: {}", top.get());
        ask.set(true);
        fixture.frame(Vec::new()).settle();
        assert!(top.get() > 0.0 && top.get() < 200.0, "in the view: {}", top.get());
    }
}
