//! The canvas's scroll bars (`pan.scrolling`): thin bars along the right and bottom edges, sized from the extent of
//! the text and the view. Dragging a thumb, or pressing a track to page, posts `view.scrollTo` with the view's new
//! centre, so the bars scroll as an agent's request does.

use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Point, Rect as DocRect, Size};
use craft::ui::canvas::CanvasScrollbars;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Label;
use egui::{Rect, Ui};
use serde_json::json;

use crate::app::Msg;

/// The extent the bars measure, in the canvas's document space (font units, y down from `top`): the edited glyph's
/// box (`glyph`, from the document's origin) and the text around it (`text`, `[x, y, width, height]` in font units,
/// y up). `None` when neither has an area.
pub fn extent(glyph: Option<Size>, text: Option<[f64; 4]>, top: f64) -> Option<DocRect> {
    let glyph = glyph.map(|size| DocRect::from_origin_size(Point::ZERO, size));
    let text = text
        .filter(|&[x, y, w, h]| [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0)
        .map(|[x, y, w, h]| DocRect::new(x, top - (y + h), x + w, top - y));
    let rect = match (glyph, text) {
        (Some(g), Some(t)) => g.union(t),
        (g, t) => g.or(t)?,
    };
    (rect.width() > 0.0 && rect.height() > 0.0 && rect.x0.is_finite() && rect.y0.is_finite()).then_some(rect)
}

/// The `view.scrollTo` that shows `moved` (a view the bars moved) when it differs from `view`: the font point at the
/// window's centre. `top` is the glyph's ascender, where document y is 0.
pub fn scroll_to(view: &CanvasView, moved: &CanvasView, top: f64) -> Option<Msg> {
    let (x, y) = (moved.center.x, top - moved.center.y);
    let unchanged = (moved.center.x - view.center.x).abs() < 1e-9 && (moved.center.y - view.center.y).abs() < 1e-9;
    if unchanged || !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(Msg::Shell(CommandRef::with_params("view.scrollTo", json!({"x": x, "y": y}))))
}

/// Shows the bars over `rect` for `view` and the `document` extent; what they scrolled goes to `scrolled`. True while
/// the pointer is over a bar or drags one, so the tool leaves the press alone.
pub fn show(ui: &mut Ui, label: impl Into<Label>, view: CanvasView, rect: Rect, document: DocRect, top: f64, scrolled: &mut Option<Msg>) -> bool {
    let bars = CanvasScrollbars::new(label, Bind::given(view), document).overscroll(true).show_at(ui, rect);
    if let Some(moved) = bars.view.value() {
        *scrolled = scroll_to(&view, moved, top);
    }
    bars.over
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extent_covers_the_glyph_and_the_text_and_a_moved_view_scrolls_to_its_centre() {
        // Glyph 500 wide, ascender 800, descender -200; the text reaches 1,000 units left of it.
        let text = Some([-1_000.0, -200.0, 1_500.0, 1_000.0]);
        let rect = extent(Some(Size::new(500.0, 1_000.0)), text, 800.0).unwrap();
        assert_eq!((rect.x0, rect.y0, rect.x1, rect.y1), (-1_000.0, 0.0, 500.0, 1_000.0));
        assert!(extent(None, Some([0.0, 0.0, f64::NAN, 1.0]), 800.0).is_none());

        let view = CanvasView::new(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0)), Point::new(250.0, 400.0), 0.5);
        assert!(scroll_to(&view, &view, 800.0).is_none(), "an unmoved view posts nothing");
        let mut moved = view;
        moved.center = Point::new(100.0, 300.0);
        let Some(Msg::Shell(command)) = scroll_to(&view, &moved, 800.0) else { panic!("a moved view scrolls") };
        assert_eq!((command.id.as_str(), &command.params), ("view.scrollTo", &json!({"x": 100.0, "y": 500.0})));
    }
}
