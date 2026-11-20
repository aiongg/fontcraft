//! The Space Center's preview: the projected lines painted at the window's size.
//!
//! Each line, or each size of a waterfall's one line, is a row. Multi-line text wraps at the panel's width after
//! the last space that fits, else before the glyph that overflows. Outlines are flattened by the canvas's paint
//! helper and filled by the nonzero rule in horizontal bands, so counters stay open. Upside down, the whole preview
//! turns half a circle about its middle.
//!
//! The preview is a craft `Canvas` whose document is the laid-out text in points: the canvas scrolls it, and craft's
//! `ToolHost` runs the drag tool (`shell::space_center::drag`) on the canvas's pass, as the edit view runs its tool.
//! Painting hands the tool where each glyph sits, so its hit test sees wrapping, right-to-left placement and the half
//! turn as drawn.

use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Affine, Point, Size};
use craft::ui::canvas::{Canvas, ToolHost, canvas_space, tool_pass};
use craft::ui::contract::{Bind, ShowContainer as _};
use craft::ui::layout_components::Stack;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::{Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::SpaceCenterWords;
use crate::app::Msg;
use crate::app::models::space_center::{PlacedGlyph, SpaceCenterModel, SpaceLine};
use crate::canvas::paint::flatten;
use crate::canvas::tokens::Tokens;
use crate::shell::space_center::drag::{GlyphBox, MAX_BOXES, PreviewFrame, SpaceEdit, SpacePreview, drag_message};
use crate::shell::space_center::{LineMode, waterfall_sizes};

/// Most rows painted.
const MAX_ROWS: usize = 2_000;
/// Most bands one glyph is filled in.
const MAX_BANDS: f32 = 512.0;
/// Glyphs whose margins are labelled on the beam at most.
const MAX_BEAM_LABELS: usize = 2_000;
/// A row's margins are labelled on the beam when its em is at least this many label sizes.
const BEAM_LABEL_EMS: f32 = 4.0;

/// One row of the preview: glyphs of one line, their offsets measured from `start`, at `scale` points per unit.
struct PreviewRow<'m> {
    glyphs: &'m [PlacedGlyph],
    start: f64,
    length: f64,
    scale: f32,
}

/// Splits `line` into rows no longer than `limit` font units: after the last glyph that allows a break, else
/// before the glyph that overflows. A row holds at least one glyph.
fn wrap<'m>(line: &'m SpaceLine, limit: f64, scale: f32, rtl: bool, out: &mut Vec<PreviewRow<'m>>) {
    let glyphs = line.glyphs.as_slice();
    if glyphs.is_empty() {
        out.push(PreviewRow { glyphs, start: 0.0, length: 0.0, scale });
        return;
    }
    // Right-to-left text reads from the right end: rows are cut from the end of the visual order.
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let span = |from: usize, to: usize| -> f64 {
        match (glyphs.get(from), to.checked_sub(1).and_then(|l| glyphs.get(l))) {
            (Some(a), Some(b)) => b.offset + b.advance - a.offset,
            _ => 0.0,
        }
    };
    if rtl {
        let mut end = glyphs.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && span(start - 1, end) <= limit {
                start -= 1;
            }
            if start > 0
                && let Some(cut) = (start..end).find(|&i| glyphs.get(i).is_some_and(|g| g.breaks_after))
            {
                start = cut;
            }
            rows.push((start, end));
            end = start;
        }
    } else {
        let mut start = 0;
        while start < glyphs.len() {
            let mut end = start + 1;
            while end < glyphs.len() && span(start, end + 1) <= limit {
                end += 1;
            }
            if end < glyphs.len()
                && let Some(cut) = (start..end).rev().find(|&i| glyphs.get(i).is_some_and(|g| g.breaks_after))
            {
                end = cut + 1;
            }
            rows.push((start, end));
            start = end;
        }
    }
    for (from, to) in rows {
        let part = glyphs.get(from..to).unwrap_or_default();
        let start = part.first().map_or(0.0, |g| g.offset);
        out.push(PreviewRow { glyphs: part, start, length: span(from, to), scale });
    }
}

/// The rows the model's lines make at `span` points wide.
fn rows<'m>(model: &'m SpaceCenterModel, span: f32) -> Vec<PreviewRow<'m>> {
    let state = &model.state;
    let upm = model.upm.max(1.0);
    let scale = (state.size / upm) as f32;
    let mut out = Vec::new();
    match state.line_mode {
        LineMode::Waterfall => {
            if let Some(line) = model.lines.first() {
                for size in waterfall_sizes(state.size) {
                    out.push(PreviewRow { glyphs: &line.glyphs, start: 0.0, length: line.length, scale: (size / upm) as f32 });
                }
            }
        }
        LineMode::Single => {
            for line in &model.lines {
                out.push(PreviewRow { glyphs: &line.glyphs, start: 0.0, length: line.length, scale });
            }
        }
        LineMode::Multi => {
            let limit = f64::from(span / scale.max(f32::EPSILON));
            for line in &model.lines {
                wrap(line, limit, scale, state.display.right_to_left, &mut out);
                if out.len() >= MAX_ROWS {
                    break;
                }
            }
        }
    }
    out.truncate(MAX_ROWS);
    out
}

/// Shows the preview as a canvas that shares the panel's room with the matrix. The canvas scrolls over the lines;
/// its tool (`shell::space_center::drag::SpaceTool`) takes the presses, drags and arrows, whose edits become messages in `sent`.
pub fn show(ui: &mut Ui, words: &SpaceCenterWords, model: &SpaceCenterModel, tokens: &Tokens, preview: &mut SpacePreview, sent: &mut Vec<Msg>) {
    let pad = tokens.label_size;
    let SpacePreview { view, tool } = preview;
    let drag_at = tool.drag_at();
    let _ = Stack::new().fill().show(ui, |ui, _| {
        let span = (ui.available_width() - 2.0 * pad).max(pad);
        let rows = rows(model, span);
        let wide = rows.iter().map(|r| r.length as f32 * r.scale).fold(span, f32::max) + 2.0 * pad;
        let tall: f32 = rows.iter().map(|r| model.line_height as f32 * r.scale).sum::<f32>() + 2.0 * pad;
        // A view never shown starts with the first line at the top left, at one point per preview unit.
        if !view.rect.is_positive() {
            let room = ui.available_rect_before_wrap();
            *view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        }
        // At least as tall as the view, so short text starts at the top instead of in the middle.
        let content = Rect::from_min_size(Pos2::ZERO, vec2(wide, tall.max(view.rect.height())));
        let canvas = Canvas::new(words.preview.clone())
            .view(Bind::bound(&mut *view))
            .document(Size::new(f64::from(content.width()), f64::from(content.height())));
        let mut reply = None;
        let tool = &mut *tool;
        let _ = canvas.show(ui, |ui, body| {
            let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
            let clip = space.view.rect;
            let painter = ui.painter_at(clip);
            let frame = paint(&painter, clip, content, space.view.affine(), pad, &rows, model, tokens);
            if let Some(at) = drag_at {
                painter
                    .line_segment([pos2(at.x, clip.min.y), pos2(at.x, clip.max.y)], Stroke::new(tokens.metric_line_width, tokens.metric_highlight));
            }
            tool.set_frame(frame);
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool));
            }
        });
        if let Some(moved) = reply.and_then(|r| r.view) {
            *view = moved;
        }
    });
    // The Space Center's size sets the scale.
    view.zoom = 1.0;
    view.rotation = 0.0;
    view.flip_x = false;
    for edit in tool.take_edits() {
        if let Some(message) = edit_message(model, edit) {
            sent.push(message);
        }
    }
}

/// The message an edit of the preview's tool posts, with the glyph and layers of its matrix column.
fn edit_message(model: &SpaceCenterModel, edit: SpaceEdit) -> Option<Msg> {
    match edit {
        SpaceEdit::Select(column) => Some(Msg::Shell(CommandRef::with_params("window.spaceCenter", json!({ "select": column })))),
        SpaceEdit::Drag { drag, column, dx, every } => {
            let col = model.columns.get(column)?;
            let layers = if every && !col.master_layers.is_empty() { col.master_layers.clone() } else { vec![col.layer.clone()] };
            drag_message(drag, &col.glyph, &layers, dx)
        }
        SpaceEdit::Nudge { column, command, amount } => {
            let col = model.columns.get(column)?;
            Some(Msg::run(command, json!({"glyphs": [col.glyph], "layer": col.layer, "amount": amount})))
        }
    }
}

/// Paints the rows of `content` (preview units, y down) through `view` (preview units to screen points) into `clip`,
/// and returns where each glyph with a matrix column sits.
#[allow(clippy::too_many_arguments)]
fn paint(
    painter: &Painter,
    clip: Rect,
    content: Rect,
    view: Affine,
    pad: f32,
    rows: &[PreviewRow<'_>],
    model: &SpaceCenterModel,
    t: &Tokens,
) -> PreviewFrame {
    let display = &model.state.display;
    let (ground, ink) = if display.inverse { (t.outline, t.canvas) } else { (t.canvas, t.outline) };
    painter.rect_filled(clip, 0.0, ground);
    let wide = content.width() - 2.0 * pad;
    // Upside down, every point turns about the middle of the content.
    let turn = if display.upside_down {
        let c = content.center();
        Affine::new([-1.0, 0.0, 0.0, -1.0, 2.0 * f64::from(c.x), 2.0 * f64::from(c.y)])
    } else {
        Affine::IDENTITY
    };
    let to_screen = view * turn;
    let turned = |p: Pos2| {
        let q = to_screen * Point::new(f64::from(p.x), f64::from(p.y));
        pos2(q.x as f32, q.y as f32)
    };
    let selected = model.state.selected;
    let mut frame = PreviewFrame { boxes: Vec::new(), to_screen, selected, grip: f64::from(t.node_size.max(1.0)) };
    let mut labels = 0usize;
    let mut top = content.top() + pad;
    for row in rows {
        let s = row.scale;
        let height = model.line_height as f32 * s;
        let baseline = top + model.ascender as f32 * s;
        let length = row.length as f32 * s;
        let left = content.left()
            + pad
            + match (display.right_to_left, display.centered) {
                (false, false) => 0.0,
                (_, true) => (wide - length) / 2.0,
                (true, false) => wide - length,
            };
        let y_of = |units: f64| baseline - units as f32 * s;
        let band_rect =
            |bottom: f64, up: f64| Rect::from_two_pos(turned(pos2(content.left(), y_of(bottom))), turned(pos2(content.right(), y_of(up))));
        for (bottom, up) in &model.zones {
            painter.rect_filled(band_rect(*bottom, *up), 0.0, t.zone_fill);
        }
        if display.metrics {
            painter
                .line_segment([turned(pos2(left, baseline)), turned(pos2(left + length, baseline))], Stroke::new(t.metric_line_width, t.metric_line));
        }
        let mut ink_shapes = Vec::new();
        let mut marks = Vec::new();
        for g in row.glyphs {
            let x = left + ((g.offset - row.start) as f32) * s;
            let advance = g.advance as f32 * s;
            let box_top = baseline - model.ascender as f32 * s;
            if let Some(column) = g.column
                && frame.boxes.len() < MAX_BOXES
            {
                frame.boxes.push(GlyphBox {
                    column,
                    left: f64::from(x),
                    right: f64::from(x + advance),
                    top: f64::from(box_top),
                    bottom: f64::from(box_top + height),
                    scale: f64::from(s),
                });
            }
            let box_rect = Rect::from_two_pos(turned(pos2(x, box_top)), turned(pos2(x + advance, box_top + height)));
            if !clip.intersects(box_rect) {
                continue;
            }
            if display.metrics {
                for edge in [x, x + advance] {
                    marks.push(Shape::line_segment(
                        [turned(pos2(edge, box_top)), turned(pos2(edge, box_top + height))],
                        Stroke::new(t.metric_line_width, t.metric_line),
                    ));
                }
            }
            if g.column.is_some() && g.column == selected {
                marks.push(Shape::rect_stroke(box_rect, 0.0, Stroke::new(t.outline_width, t.metric_highlight), egui::StrokeKind::Inside));
            }
            let Some(art) = g.glyph.as_ref().and_then(|name| model.art.get(name)) else { continue };
            let affine = to_screen * Affine::new([f64::from(s), 0.0, 0.0, -f64::from(s), f64::from(x), f64::from(baseline)]);
            let contours = flatten(&art.outline, affine);
            if display.fill
                && let Some(mesh) = fill(&contours, clip, ink)
            {
                ink_shapes.push(Shape::mesh(mesh));
            }
            if display.stroke || !display.fill {
                ink_shapes.extend(contours.iter().map(|c| Shape::line(c.clone(), Stroke::new(t.outline_width, ink))));
            }
            for overlay in &art.overlays {
                marks.extend(
                    flatten(overlay, affine).into_iter().map(|c| Shape::line(c, Stroke::new(t.visible_layer_outline_width, t.visible_layer_outline))),
                );
            }
            if let (Some(y), Some((l, r))) = (model.beam_y, g.beam)
                && s * model.upm as f32 >= BEAM_LABEL_EMS * t.label_size
                && labels < MAX_BEAM_LABELS
            {
                labels += 1;
                let at = y_of(y) - t.label_gap;
                let font = FontId::proportional(t.label_size);
                painter.text(turned(pos2(x + t.label_gap, at)), egui::Align2::LEFT_BOTTOM, super::format_value(Some(l)), font.clone(), t.metric_name);
                painter.text(
                    turned(pos2(x + advance - t.label_gap, at)),
                    egui::Align2::RIGHT_BOTTOM,
                    super::format_value(Some(r)),
                    font,
                    t.metric_name,
                );
            }
        }
        painter.extend(ink_shapes.iter().cloned());
        painter.extend(marks);
        if let Some(y) = model.beam_y {
            painter.line_segment(
                [turned(pos2(content.left(), y_of(y))), turned(pos2(content.right(), y_of(y)))],
                Stroke::new(t.metric_line_width, t.metric_highlight),
            );
        }
        // The cut-off band shows the row again in the inverse colours.
        if let Some((bottom, up)) = model.band {
            let band = band_rect(bottom, up).intersect(clip);
            let clipped = painter.with_clip_rect(band);
            clipped.rect_filled(band, 0.0, ink);
            clipped.extend(ink_shapes.into_iter().map(|shape| recolour(shape, ground)));
        }
        top += height;
    }
    frame
}

/// `shape` painted in `colour`: a mesh's vertices or a line's stroke.
fn recolour(shape: Shape, colour: Color32) -> Shape {
    match shape {
        Shape::Mesh(mut mesh) => {
            for v in &mut std::sync::Arc::make_mut(&mut mesh).vertices {
                v.color = colour;
            }
            Shape::Mesh(mesh)
        }
        Shape::Path(mut path) => {
            path.stroke.color = egui::epaint::ColorMode::Solid(colour);
            Shape::Path(path)
        }
        other => other,
    }
}

/// An edge in screen points, top end first, with its winding.
#[derive(Clone, Copy)]
struct Edge {
    top: Pos2,
    bottom: Pos2,
    winding: i32,
}

impl Edge {
    fn x_at(&self, y: f32) -> f32 {
        let dy = self.bottom.y - self.top.y;
        if dy <= f32::EPSILON {
            self.top.x
        } else {
            self.top.x + (y.clamp(self.top.y, self.bottom.y) - self.top.y) * (self.bottom.x - self.top.x) / dy
        }
    }
}

/// The nonzero fill of `contours` (screen points) inside `clip`, as one mesh of horizontal bands; `None` when
/// nothing is filled.
fn fill(contours: &[Vec<Pos2>], clip: Rect, colour: Color32) -> Option<Mesh> {
    let mut edges = Vec::new();
    for contour in contours {
        for pair in contour.windows(2) {
            let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
            if (a.y - b.y).abs() <= f32::EPSILON {
                continue;
            }
            edges.push(if a.y < b.y { Edge { top: a, bottom: b, winding: 1 } } else { Edge { top: b, bottom: a, winding: -1 } });
        }
    }
    let top = edges.iter().map(|e| e.top.y).fold(f32::INFINITY, f32::min).max(clip.top());
    let bottom = edges.iter().map(|e| e.bottom.y).fold(f32::NEG_INFINITY, f32::max).min(clip.bottom());
    if !(top.is_finite() && bottom.is_finite() && bottom > top) {
        return None;
    }
    let step = ((bottom - top) / MAX_BANDS).max(0.5);
    let mut mesh = Mesh::default();
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    let mut y = top;
    while y < bottom {
        let next = (y + step).min(bottom);
        let mid = (y + next) / 2.0;
        crossings.clear();
        crossings.extend(edges.iter().filter(|e| e.top.y <= mid && mid < e.bottom.y).map(|e| (e.x_at(mid), e.winding)));
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut from = 0.0;
        for &(x, w) in &crossings {
            let was = winding;
            winding += w;
            if was == 0 && winding != 0 {
                from = x;
            } else if was != 0 && winding == 0 && x > from {
                mesh.add_colored_rect(Rect::from_min_max(pos2(from, y), pos2(x, next)), colour);
            }
        }
        y = next;
    }
    (!mesh.is_empty()).then_some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(offset: f64, advance: f64, breaks_after: bool) -> PlacedGlyph {
        PlacedGlyph { glyph: Some("a".into()), offset, advance, column: None, breaks_after, beam: None }
    }

    /// A line wraps after its last space that fits; a word longer than the row breaks before the glyph that
    /// overflows.
    #[test]
    fn lines_wrap_after_the_last_space_that_fits() {
        // "aa aa": five glyphs of 100 units, a space in the middle.
        let glyphs = vec![
            glyph(0.0, 100.0, false),
            glyph(100.0, 100.0, false),
            glyph(200.0, 100.0, true),
            glyph(300.0, 100.0, false),
            glyph(400.0, 100.0, false),
        ];
        let line = SpaceLine { glyphs, length: 500.0 };
        let mut out = Vec::new();
        wrap(&line, 450.0, 1.0, false, &mut out);
        let counts: Vec<usize> = out.iter().map(|r| r.glyphs.len()).collect();
        assert_eq!(counts, vec![3, 2]);
        assert_eq!(out[1].start, 300.0);
        let mut narrow = Vec::new();
        wrap(&line, 150.0, 1.0, false, &mut narrow);
        assert!(narrow.iter().all(|r| !r.glyphs.is_empty() && r.length <= 150.0 || r.glyphs.len() == 1));
        assert_eq!(narrow.iter().map(|r| r.glyphs.len()).sum::<usize>(), 5);
    }

    /// The band fill leaves a counter open: a square inside a square, wound the other way, is not filled.
    #[test]
    fn fill_keeps_counters_open() {
        let outer = vec![pos2(0.0, 0.0), pos2(30.0, 0.0), pos2(30.0, 30.0), pos2(0.0, 30.0), pos2(0.0, 0.0)];
        let inner = vec![pos2(10.0, 10.0), pos2(10.0, 20.0), pos2(20.0, 20.0), pos2(20.0, 10.0), pos2(10.0, 10.0)];
        let mesh = fill(&[outer, inner], Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(100.0)), Color32::BLACK).unwrap();
        let covers = |p: Pos2| {
            mesh.indices.chunks(6).any(|quad| {
                let xs: Vec<Pos2> = quad.iter().map(|&i| mesh.vertices[i as usize].pos).collect();
                let r = Rect::from_points(&xs);
                r.contains(p)
            })
        };
        assert!(covers(pos2(5.0, 15.0)));
        assert!(!covers(pos2(15.0, 15.0)));
    }
}
