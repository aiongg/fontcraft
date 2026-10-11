//! Edit View's canvas: the showing tab's glyphs with the edited one's outline, nodes and
//! metric lines, and the wheel and gesture input that zooms and scrolls it.
//!
//! | Module | Draws |
//! |---|---|
//! | [`view`] | (no drawing) the camera and the cached text layout, fitting, the tools' view transform |
//! | [`metrics`] | metric lines, zones and their names |
//! | [`guides`] | guides, the grid, parameter metric lines, CJK grids, measurement labels |
//! | [`background`] | the other side of the layer and the other visible layers, behind the outline; the darker canvas while the background is edited |
//! | [`fill`] | filled glyphs, rasterised by the `render` crate into a texture |
//! | [`composite`] | (no drawing) each glyph's outline with strokes expanded and masks applied, cached |
//! | [`nodes`] | nodes and handles with the display conventions of `docs/spec/03a` § 4 |
//! | [`selection`] | the selection, and the active tool's overlays (marquee, snapping lines…) |
//! | [`bbox`] | the selection's bounding box and knobs, and the transformation origin |
//! | [`annotations`] | notes and correction marks |
//! | [`audit_markers`] | audit results on the edited layer; also runs live audit |
//! | [`text`] | text mode: kerned gaps, the text selection and cursor, metrics labels; the pointer in text mode |
//! | [`group_members`] | Show Group Members: the other members of the edited pair's kerning groups, behind the glyphs |
//!
//! Every colour and size comes from [`Tokens`]. Wheel and gestures act through `view.scrollTo`
//! and `view.setZoom`, like every other input. Primary-button input goes to the pointer tools
//! ([`crate::tools_glue`]) after the frame is drawn.

pub mod annotations;
pub mod audit_markers;
pub mod background;
pub mod bbox;
pub mod composite;
pub mod fill;
pub mod group_members;
pub mod guides;
pub mod metrics;
pub mod nodes;
pub mod selection;
pub mod text;
pub mod view;

use egui::{Pos2, Rect, Sense, Stroke};
use fontcraft_geom::{Affine, PathEl, Point};
use fontcraft_tools::View;
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::state::TabView;
use crate::theme::Tokens;

/// How much one point of Alt-scrolling zooms: the scale is multiplied by `exp(points × this)`.
const WHEEL_ZOOM_RATE: f64 = 0.004;

/// What every drawing part needs: where to draw, the edited glyph's transform and the tokens.
pub struct Painting<'a> {
    pub painter: &'a egui::Painter,
    pub rect: Rect,
    pub view: View,
    pub tokens: &'a Tokens,
}

/// Draws Edit View for the showing tab, and queues the view commands its input asks for.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    audit_markers::refresh(app);
    crate::shell::context_menu::show(app, ui, pending);
    crate::shell::preview::area(app, ui, pending);
    let FontcraftApp { session, state, tokens, fill_cache, preview_tool, tools, layouts, .. } = app;
    let Some(tab) = state.tab().cloned() else { return };
    layouts.prepare(session, &tab);
    let frame = egui::Frame::NONE.fill(tokens.canvas);
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        state.canvas_size = [f64::from(rect.width()), f64::from(rect.height())];
        let extras = guides::Extras::query(session, &tab, &state.display, tools.toolbox.active_id(), &tools.toolbox.overlays());
        let Ok(font) = session.font() else { return };
        let Some(master) = view::active_master(font, &tab) else { return };
        let layout = view::text_layout(session, layouts, &tab);
        let origin = view::edited_origin(session, layouts, &tab);
        let width = view::edited_width(font, &tab);
        let center = Point::new(f64::from(rect.center().x), f64::from(rect.center().y));
        let v = view::tools_view(&tab.view, origin, center);
        if response.hovered() || response.dragged() {
            input(ui, &response, &tab, v, center, pending);
        }
        crate::tools_glue::collect(tools, ui, &response, rect);
        let painter = ui.painter_at(rect);
        let c = Painting { painter: &painter, rect, view: v, tokens };
        // Text mode draws the text as text: every glyph filled, kerned gaps and the selection
        // behind them, the cursor and the metrics labels in front, no nodes.
        let text_mode = tab.text_mode;
        let tc = Painting { painter: &painter, rect, view: text::text_view(&tab.view, center), tokens };
        if tab.edit_background {
            background::tint(&c);
        }
        let (lines, marks) = metrics::of(font, master);
        let span = view::vertical_span(font, master);
        // Vertical text has no horizontal metric lines to share: each column is its own.
        let vertical_text = text_mode && tab.direction == crate::state::WritingDirection::Ttb;
        if state.display.metrics && !vertical_text {
            metrics::draw(&c, &lines, &marks, (!text_mode).then_some(width), span, state.display.metrics_names);
        }
        guides::draw_lines(&c, &extras, &state.display);
        // The edited side as a layer: while the background is edited, its background is the
        // foreground, which `background::draw` shows as the reference outline.
        let edited = view::edited_layer(font, &tab);
        if !text_mode
            && let Some(glyph) = tab.edited_glyph().and_then(|g| font.glyph(g))
            && let Some(layer) = edited.as_deref()
        {
            background::draw(&c, font, glyph, layer, &state.display);
        }
        let doc = session.active().map(|d| (d.id, d.revision)).unwrap_or_default();
        if text_mode && let Some(layout) = layout.as_deref() {
            text::draw_behind(&tc, layout, &tab, span.0);
        }
        if state.display.group_members {
            group_members::draw(&c, fill_cache, font, &tab, layout.as_deref(), origin, doc);
        }
        let what = fill::Fill { doc, others: state.display.fill_preview, edited: state.display.fill_preview && (*preview_tool || text_mode) };
        fill::draw(&c, fill_cache, font, &tab, layout.as_deref(), origin, what);
        if text_mode {
            if let Some(layout) = layout.as_deref() {
                let overlay = text::Overlay { font, doc, descender: span.0, labels: state.display.metrics };
                text::draw_front(&tc, layout, &tab, &overlay, &mut layouts.sidebearings.borrow_mut());
            }
            if !what.edited
                && let Some(glyph) = tab.edited_glyph()
                && let Some(layer) = edited.as_deref()
                && let Some(composite) = fill_cache.outlines.get(doc, font, glyph, layer)
            {
                let to_screen = Affine::translate(v.origin.to_vec2()) * Affine::scale_non_uniform(v.scale(), -v.scale());
                for contour in flatten(&(to_screen * composite.outline.clone())) {
                    painter.add(egui::Shape::line(contour, Stroke::new(tokens.metric_line_width, tokens.glyph_fill)));
                }
            }
            guides::draw_measurement(&c, &extras);
            return;
        }
        let Some(layer) = edited.as_deref() else { return };
        let to_screen = Affine::translate(v.origin.to_vec2()) * Affine::scale_non_uniform(v.scale(), -v.scale());
        if tab.edit_background {
            for contour in flatten(&(to_screen * background::outline(font, &layer.shapes, &layer.master))) {
                painter.add(egui::Shape::line(contour, Stroke::new(tokens.outline_width, tokens.outline)));
            }
        } else if let Some(glyph) = tab.edited_glyph()
            && let Some(composite) = fill_cache.outlines.get(doc, font, glyph, layer)
        {
            // The composite in black; under it, when strokes or masks make it differ, the paths
            // as drawn in light grey: centre lines of strokes and the parts masks cut away
            // (`stroke.display`, `mask.display`).
            if composite.styled
                && let Some(source) = composite::plain_outline(font, glyph, layer)
            {
                for contour in flatten(&(to_screen * source)) {
                    painter.add(egui::Shape::line(contour, Stroke::new(tokens.source_outline_width, tokens.source_outline)));
                }
            }
            for contour in flatten(&(to_screen * composite.outline.clone())) {
                painter.add(egui::Shape::line(contour, Stroke::new(tokens.outline_width, tokens.outline)));
            }
        }
        guides::draw_guides(&c, font, layer, &tab.selection, &extras, &state.display);
        let focus = tab.focus.as_deref();
        if state.display.nodes {
            painter.extend(selection::segment_shapes(layer, &tab.selection, v, tokens, focus));
            painter.extend(nodes::shapes(layer, v, tokens, &marks, focus));
            painter.extend(selection::point_shapes(layer, &tab.selection, v, tokens, focus));
        }
        painter.extend(bbox::shapes(font, &tab, tools.toolbox.active_id(), state.display.bounding_box, &state.transform, v, tokens));
        annotations::draw(&c, layer, &tab.selection, &state.display);
        // Audit results describe the foreground.
        if !tab.edit_background {
            audit_markers::draw(ui, &c, layer, audit_markers::visible(&state.audit, session, &tab), !tools.toolbox.busy(), pending);
        }
        painter.extend(selection::overlay_shapes(&tools.toolbox.overlays(), v, tokens));
        guides::draw_measurement(&c, &extras);
    });
}

/// Wheel, pinch and middle-button drags: scroll and zoom through the view commands.
fn input(ui: &egui::Ui, response: &egui::Response, tab: &crate::state::Tab, v: View, center: Point, pending: &mut Vec<(&'static str, Value)>) {
    let (scroll, zoom, modifiers, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers, i.pointer.hover_pos()));
    let mut factor = f64::from(zoom);
    if modifiers.alt && scroll.y != 0.0 {
        factor *= (f64::from(scroll.y) * WHEEL_ZOOM_RATE).exp();
    }
    if (factor - 1.0).abs() > 1e-6 {
        let at = hover.map_or(center, |p| Point::new(f64::from(p.x), f64::from(p.y)));
        let scale = TabView::clamped_scale(v.scale() * factor);
        // Keep the font point under the pointer where it is.
        let under = v.to_font(at);
        let middle = Point::new(under.x + (center.x - at.x) / scale, under.y - (center.y - at.y) / scale);
        pending.push(("view.setZoom", json!({"tab": tab.id, "size": scale * 1000.0, "center": [middle.x, middle.y]})));
        return;
    }
    let mut delta = scroll;
    if modifiers.shift && delta.x == 0.0 {
        delta = egui::vec2(delta.y, 0.0);
    }
    if response.dragged_by(egui::PointerButton::Middle) {
        delta += response.drag_delta();
    }
    if delta != egui::Vec2::ZERO {
        pending.push(("view.scrollTo", json!({"tab": tab.id, "dx": delta.x, "dy": delta.y})));
    }
}

/// Most points one glyph is flattened to for drawing. The model bounds coordinates and outline
/// size, so real glyphs stay far below this; it keeps a pathological one from filling memory.
const MAX_FLATTEN_POINTS: usize = 200_000;

/// The contours of `path` as polylines (curves flattened to a quarter of a screen point). A
/// closed contour ends where it began.
pub fn flatten(path: &fontcraft_geom::BezPath) -> Vec<Vec<Pos2>> {
    let mut contours: Vec<Vec<Pos2>> = Vec::new();
    let mut count = 0usize;
    fontcraft_geom::flatten(path, 0.25, |el| {
        count += 1;
        if count > MAX_FLATTEN_POINTS {
            return;
        }
        match el {
            PathEl::MoveTo(p) => contours.push(vec![Pos2::new(p.x as f32, p.y as f32)]),
            PathEl::LineTo(p) => {
                if let Some(c) = contours.last_mut() {
                    c.push(Pos2::new(p.x as f32, p.y as f32));
                }
            }
            PathEl::ClosePath => {
                if let Some(c) = contours.last_mut()
                    && let Some(first) = c.first().copied()
                {
                    c.push(first);
                }
            }
            _ => {}
        }
    });
    contours.retain(|c| c.len() > 1);
    contours
}
