//! The components and corner components of the edited layer, over the glyph's outline.
//!
//! | Item | Drawn as |
//! |---|---|
//! | component aligned automatically | its outline over a light fill, both in `component_aligned` (`components.autoAlignColors`) |
//! | component aligned horizontally only | the same in `component_shiftable` |
//! | component placed by hand | the same in `component_manual` |
//! | selected component or placeholder | its outline wider, in the tools' `selection` (`components.clickSelects`) |
//! | empty base layer, missing base glyph, circular reference | a crossed box with a label, `component_placeholder` (`components.placeholder*`) |
//! | corner or cap component | a wide, faint band along the outline fitting it put in, `attachment_band` (`corner.highlightAndSelection`); a cap's band leaves out the stem's two sides |
//! | selected corner component | the band in `attachment_band_selected` |
//!
//! Text mode marks nothing selected and fills nothing, and Fill Preview's fill stands in for the light fill. While the
//! background is edited only its selected components are drawn.
//!
//! The projection resolves outlines per engine revision, so composites follow a base glyph while it is being
//! edited (`components.liveUpdateFromBaseGlyph`).

use egui::{Align2, Color32, FontId, Shape, Stroke};

use super::super::layers::PaintCx;
use super::{fill_shapes, outline_shapes, screen};
use crate::app::models::canvas::{ComponentMark, Placeholder, State};
use crate::canvas::tools::tokens::ToolTokens;

/// Most outline elements the light fills of one glyph's components cover; components past it show their outline alone.
/// Filling takes time in proportion to it, every frame.
const MAX_FILLED_ELEMENTS: usize = 1500;

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
    let t = cx.tokens;
    for (mark, selected) in marks(cx) {
        let (State::Placeholder(kind), Some([.., top_left])) = (mark.state, mark.placeholder) else { continue };
        let Some(at) = screen(cx, top_left) else { continue };
        let why = match kind {
            Placeholder::Empty => &cx.words.empty,
            Placeholder::Missing => &cx.words.missing,
            Placeholder::Circular => &cx.words.circular,
        };
        let label = format!("{}: {}", mark.base, why.as_str());
        cx.painter.text(at, Align2::LEFT_BOTTOM, label, FontId::proportional(t.label_size), stroke(cx, mark, selected).color);
    }
}

/// The components drawn, with whether each is selected: all of them, or only the selected ones while the background is
/// edited.
fn marks<'a>(cx: &PaintCx<'a>) -> impl Iterator<Item = (&'a ComponentMark, bool)> {
    let background = cx.edit_background;
    let selection = (!cx.text.text_mode).then_some(&cx.tools.selection.shapes);
    cx.model
        .glyph
        .iter()
        .flat_map(|g| &g.components)
        .map(move |mark| (mark, selection.is_some_and(|s| s.contains(&mark.shape))))
        .filter(move |(_, selected)| *selected || !background)
}

fn state_color(cx: &PaintCx<'_>, state: State) -> Color32 {
    let t = cx.tokens;
    match state {
        State::Aligned => t.component_aligned,
        State::Shiftable => t.component_shiftable,
        State::Manual => t.component_manual,
        State::Placeholder(_) => t.component_placeholder,
    }
}

fn stroke(cx: &PaintCx<'_>, mark: &ComponentMark, selected: bool) -> Stroke {
    if selected {
        Stroke::new(cx.tokens.component_selected_width, ToolTokens::standard().selection)
    } else {
        Stroke::new(cx.tokens.component_outline_width, state_color(cx, mark.state))
    }
}

/// The fills, outlines and placeholder boxes of the components, then the bands of corners and caps.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    let t = cx.tokens;
    let mut out = Vec::new();
    let filled = !(cx.edit_background || cx.text.text_mode || (cx.tools.fills && cx.display.fill_preview));
    let mut budget = if filled { MAX_FILLED_ELEMENTS } else { 0 };
    for (mark, _) in marks(cx) {
        let elements = mark.outline.elements().len();
        let Some(left) = budget.checked_sub(elements).filter(|_| elements > 0) else { continue };
        budget = left;
        out.extend(fill_shapes(cx, &mark.outline, state_color(cx, mark.state).gamma_multiply(t.component_fill_opacity)));
    }
    for (mark, selected) in marks(cx) {
        let stroke = stroke(cx, mark, selected);
        match mark.placeholder {
            Some(corners) => {
                let Some(points) = corners.iter().map(|p| screen(cx, *p)).collect::<Option<Vec<_>>>() else { continue };
                let [a, b, c, d] = [0, 1, 2, 3].map(|i| points.get(i).copied().unwrap_or_default());
                out.push(Shape::closed_line(points, stroke));
                out.push(Shape::line_segment([a, c], stroke));
                out.push(Shape::line_segment([b, d], stroke));
            }
            None => out.extend(outline_shapes(cx, &mark.outline, stroke)),
        }
    }
    if cx.edit_background {
        return out;
    }
    let selected = &cx.tools.selection.hints;
    for a in &glyph.attachments {
        let color = if selected.contains(&a.hint) { t.attachment_band_selected } else { t.attachment_band };
        out.extend(outline_shapes(cx, &a.band, Stroke::new(t.attachment_band_width, color)));
    }
    out
}
