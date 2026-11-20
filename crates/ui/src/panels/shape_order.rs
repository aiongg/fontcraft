//! Shape Order: the shapes of every layer of one glyph that takes part in interpolation, one list per layer, in
//! stacking order with shape 0 first, reordered by dragging until the layers match.
//!
//! Each row is a thumbnail of the whole layer, dim, with the row's shape drawn over it in a tint that follows its
//! index, so the same index has the same tint in every layer. A row dropped at another place in its list (or moved
//! with the list's reorder keys) runs `glyph.setShapeOrder` for that layer, one undo step.

use std::borrow::Cow;

use craft::behavior::{EditEvent, GesturePhase, keyed};
use craft::ui::contract::{CollectionEvent, Props as _, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::list::{List, ListRow, ThumbSize};
use craft::ui::primitives::{ContentPainterCx, StrokeWidth};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::{Color32, Rect, Stroke, Ui, pos2};
use serde_json::json;

use crate::app::Msg;
use crate::app::models::shape_order::{ShapeColumn, ShapeKind, ShapeOrderModel};
use crate::canvas::tokens::{CompatTokens, Tokens};
use crate::dialogs::fields::reordered;

/// The share of a thumbnail's side the layer takes.
const FIT_SHARE: f32 = 0.84;
/// How opaque the whole layer is under the row's shape.
const DIM_SHARE: f32 = 0.35;

craft::ui::words! {
    /// The words of the Shape Order panel.
    pub struct ShapeOrderWords {
        /// Shown without a glyph.
        no_glyph: "fontcraftShapeOrder.noGlyph" = "Open a glyph to see its shapes";
        /// Shown for a layer without shapes.
        empty: "fontcraftShapeOrder.empty" = "No shapes";
        path: "fontcraftShapeOrder.path" = "Path";
        nodes: "fontcraftShapeOrder.nodes" = "nodes";
        open: "fontcraftShapeOrder.open" = "open";
        component: "fontcraftShapeOrder.component" = "Component";
        image: "fontcraftShapeOrder.image" = "Image";
        other: "fontcraftShapeOrder.other" = "Other shape";
        /// After the count of shapes or layers left out.
        more_shapes: "fontcraftShapeOrder.moreShapes" = "more shapes not shown";
        more_layers: "fontcraftShapeOrder.moreLayers" = "more layers not shown";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<ShapeOrderModel>,
    /// The tints of the shapes, shared with the Master Compatibility view.
    pub tokens: &'a Tokens,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ShapeOrderWords>();
    let model: &ShapeOrderModel = props.model;
    let Some(glyph) = model.glyph.as_deref() else {
        Text::new(words.no_glyph.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let compat = &props.tokens.compat;
    let sent = &mut *props.sent;
    Text::new(Label::from(glyph.to_string())).show(ui);
    for column in &model.columns {
        Section::new(Label::from(column.title.clone())).id_salt(("fontcraftShapeOrder", column.layer.as_str())).show(ui, |ui, _| {
            layer_list(ui, &words, glyph, column, compat, sent);
        });
    }
    if model.more_layers > 0 {
        Text::new(format!("{} {}", model.more_layers, words.more_layers.as_str())).shade(Shade::Muted).show(ui);
    }
}

/// One layer's shapes as a list that reorders by drag.
fn layer_list(ui: &mut Ui, words: &ShapeOrderWords, glyph: &str, column: &ShapeColumn, compat: &CompatTokens, sent: &mut Vec<Msg>) {
    if column.shapes.is_empty() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let positions: Vec<usize> = (0..column.shapes.len()).collect();
    let text = |at: usize| row_text(words, at, column);
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(text(*at)));
    let out = List::<usize>::new(Label::from(column.title.clone()))
        .header(false)
        .natural_height(true)
        .reorder(ReorderRule::new())
        .id_salt(("fontcraftShapeOrderList", column.layer.as_str()))
        .show_items(ui, &source, &mut |at, icx| {
            let at = *at;
            let tint = compat.tint(at, true);
            ListRow::new(text(at)).state(icx.row.clone()).thumb(ThumbSize::Large, move |cx, rect| paint(cx, rect, column, at, tint))
        });
    for event in &out.events {
        if let CollectionEvent::Reorder(edit) = event
            && let Some(moved) = committed(edit)
        {
            // The list shows the first shapes of the layer; the order the command takes holds all of them.
            let order: Vec<usize> = (0..column.total.max(column.shapes.len())).collect();
            if let Some(next) = reordered(&order, moved) {
                sent.push(Msg::run("glyph.setShapeOrder", json!({"glyph": glyph, "layer": column.layer, "order": next})));
            }
        }
    }
    let left_out = column.total.saturating_sub(column.shapes.len());
    if left_out > 0 {
        Text::new(format!("{left_out} {}", words.more_shapes.as_str())).shade(Shade::Muted).show(ui);
    }
}

/// The value of a finished edit: a drop or a key move, not a drag still under way.
fn committed<T>(event: &EditEvent<T>) -> Option<&T> {
    match event.phase() {
        Some(GesturePhase::Commit) | None => event.value(),
        _ => None,
    }
}

/// A row's text: the shape's index and what it is.
fn row_text(words: &ShapeOrderWords, at: usize, column: &ShapeColumn) -> String {
    match column.shapes.get(at).map(|s| &s.kind) {
        Some(ShapeKind::Path { nodes, closed }) => {
            let open = if *closed { String::new() } else { format!(", {}", words.open.as_str()) };
            format!("{at}: {} ({nodes} {}{open})", words.path.as_str(), words.nodes.as_str())
        }
        Some(ShapeKind::Component { glyph }) => format!("{at}: {} {glyph}", words.component.as_str()),
        Some(ShapeKind::Image) => format!("{at}: {}", words.image.as_str()),
        Some(ShapeKind::Other) | None => format!("{at}: {}", words.other.as_str()),
    }
}

/// The thumbnail of row `at`: the whole layer dim, fitted to the layer's bounds, with shape `at` over it in `tint`.
fn paint(cx: &ContentPainterCx<'_>, rect: Rect, column: &ShapeColumn, at: usize, tint: Color32) {
    let Some([x0, y0, x1, y1]) = column.bounds else { return };
    let side = (x1 - x0).max(y1 - y0);
    if !(side.is_finite() && side > 0.0) {
        return;
    }
    let scale = rect.width().min(rect.height()) * FIT_SHARE / side;
    let (mid_x, mid_y) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let centre = rect.center();
    let to_screen = |[x, y]: [f32; 2]| pos2(centre.x + (x - mid_x) * scale, centre.y - (y - mid_y) * scale);
    let hairline = cx.stroke_width(StrokeWidth::Hairline);
    cx.faded(DIM_SHARE, |dim| {
        let stroke = Stroke::new(hairline, dim.ink());
        for contour in column.shapes.iter().flat_map(|s| s.art.contours.iter()) {
            dim.painter.add(egui::Shape::closed_line(contour.iter().copied().map(to_screen).collect(), stroke));
        }
    });
    if let Some(shape) = column.shapes.get(at) {
        let stroke = Stroke::new(cx.stroke_width(StrokeWidth::Medium), tint);
        for contour in &shape.art.contours {
            cx.painter.add(egui::Shape::closed_line(contour.iter().copied().map(to_screen).collect(), stroke));
        }
    }
}

#[cfg(test)]
mod tests;
