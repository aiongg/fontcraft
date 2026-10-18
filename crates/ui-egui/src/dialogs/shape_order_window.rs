//! The Shape Order window (M9.23): the shapes of each master in order, reordered by dragging;
//! it stays open and blocks nothing.
//!
//! Rows: `window.shapeOrder` (the window and the shell command that shows it).
//!
//! One column per layer that takes part in interpolation (each master layer, followed by that
//! master's intermediate and alternate layers), one numbered thumbnail per path and component in
//! stacking order, the first at the top. A thumbnail draws its shape in a colour that follows its
//! index, over the whole layer in a dim colour, so the same index reads as the same colour in
//! every column. Dropping a thumbnail on another moves it to that place in its own layer through
//! `glyph.setShapeOrder`, an undo step like any edit. The window follows the glyph being edited
//! unless it was opened on a named glyph.

use egui::{Pos2, Rect, RichText, Sense, Shape as EguiShape, Stroke, Vec2};
use fontcraft_engine::model::{Font, Glyph, Layer, Shape};
use fontcraft_geom::{Affine, BezPath, Shape as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::form::moved;
use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

/// Most layers and shapes the window shows; the rest are left out with a note.
pub const MAX_COLUMNS: usize = 64;
pub const MAX_SHAPES: usize = 256;
/// Room around the layer inside a thumbnail, as a share of its side.
const MARGIN: f64 = 0.08;

pub const COMMANDS: &[UiCommand] = &[(
    "window.shapeOrder",
    "Shape Order",
    &["Window"],
    "",
    "{open?: bool = true, glyph?: name} show or hide the Shape Order window: every path and component of every master layer and special layer of a glyph (default: the glyph being edited, followed as it changes) as numbered thumbnails in stacking order, one column per layer. Dropping a thumbnail on another reorders that layer with glyph.setShapeOrder {glyph, layer, order} → {open, glyph, layers: [{layer, name, shapes: [{index, kind, glyph?}]}]}",
)];

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapeOrderWindowState {
    pub open: bool,
    /// The glyph shown; `None` follows the glyph being edited.
    pub glyph: Option<String>,
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "window.shapeOrder" {
        return Err(format!("unknown command `{id}`"));
    }
    let open = args.flag("open")?.unwrap_or(true);
    let named = args.string("glyph")?.map(str::to_string);
    if open {
        let font = app.session.font().map_err(|e| args.err(e))?;
        let glyph =
            named.clone().or_else(|| crate::panels::current_glyph(app)).ok_or_else(|| args.err("open a glyph first, or name one with `glyph`"))?;
        if font.glyph(&glyph).is_none() {
            return Err(args.err(format!("the font has no glyph `{glyph}`")));
        }
    }
    let st = &mut app.state.panels.shape_order_window;
    st.open = open;
    if open {
        st.glyph = named;
    }
    Ok(reply(app))
}

/// The glyph the window shows.
fn shown_glyph(app: &FontcraftApp) -> Option<String> {
    app.state.panels.shape_order_window.glyph.clone().or_else(|| crate::panels::current_glyph(app))
}

/// Whether a layer is an intermediate or alternate layer.
fn is_special(layer: &Layer) -> bool {
    !layer.info.attrs.coordinates.is_empty() || !layer.info.attrs.axis_rules.is_empty()
}

/// The layers the window shows, in order, with their column titles: each master's layer, then
/// that master's special layers.
pub fn columns<'a>(font: &Font, glyph: &'a Glyph) -> Vec<(&'a Layer, String)> {
    let mut out = Vec::new();
    for m in font.masters.iter() {
        for layer in glyph.layers.iter().filter(|l| l.master == m.id && (l.is_master_layer() || is_special(l))) {
            let name = if layer.is_master_layer() {
                m.name.clone()
            } else {
                let label = fontcraft_engine::interp::labels::brace_label(font, layer)
                    .into_iter()
                    .chain(fontcraft_engine::interp::labels::bracket_label(font, layer))
                    .collect::<Vec<_>>()
                    .join(" ");
                match (label.is_empty(), layer.info.name.as_deref()) {
                    (false, _) => format!("{} {label}", m.name),
                    (true, Some(n)) if !n.is_empty() => n.to_string(),
                    _ => layer.id.clone(),
                }
            };
            out.push((layer, name));
            if out.len() >= MAX_COLUMNS {
                return out;
            }
        }
    }
    out
}

fn kind(shape: &Shape) -> &'static str {
    match shape {
        Shape::Path(_) => "path",
        Shape::Component(_) => "component",
        Shape::Image(_) => "image",
        _ => "other",
    }
}

fn reply(app: &FontcraftApp) -> Value {
    let st = &app.state.panels.shape_order_window;
    let glyph = shown_glyph(app);
    let layers: Vec<Value> = match (app.session.font(), &glyph) {
        (Ok(font), Some(name)) => font.glyph(name).map_or_else(Vec::new, |g| {
            columns(font, g)
                .iter()
                .map(|(layer, title)| {
                    let shapes: Vec<Value> = layer
                        .shapes
                        .iter()
                        .take(MAX_SHAPES)
                        .enumerate()
                        .map(|(i, s)| match s {
                            Shape::Component(c) => json!({"index": i, "kind": "component", "glyph": c.glyph}),
                            other => json!({"index": i, "kind": kind(other)}),
                        })
                        .collect();
                    json!({"layer": layer.id, "name": title, "shapes": shapes})
                })
                .collect()
        }),
        _ => Vec::new(),
    };
    json!({"open": st.open, "glyph": glyph, "layers": layers})
}

/// The outline of one shape on `layer`; `None` for a shape without one.
fn outline(font: &Font, layer: &Layer, shape: &Shape) -> Option<BezPath> {
    match shape {
        Shape::Path(p) => p.to_bez().ok(),
        Shape::Component(c) => font.component_outline(c, &layer.master).ok(),
        _ => None,
    }
}

/// The transform that fits `path` into `rect`, y up.
fn fit(path: &BezPath, rect: Rect) -> Option<Affine> {
    let bounds = path.bounding_box();
    let (w, h) = (bounds.width(), bounds.height());
    if !(w.is_finite() && h.is_finite()) || (w <= 0.0 && h <= 0.0) {
        return None;
    }
    let side = f64::from(rect.width().min(rect.height())) * (1.0 - 2.0 * MARGIN);
    let scale = side / w.max(h);
    let c = rect.center();
    Some(
        Affine::translate((f64::from(c.x), f64::from(c.y)))
            * Affine::scale_non_uniform(scale, -scale)
            * Affine::translate((-bounds.center().x, -bounds.center().y)),
    )
}

fn stroke_path(painter: &egui::Painter, path: &BezPath, to_rect: Affine, stroke: Stroke) {
    for contour in crate::canvas::flatten(&(to_rect * path.clone())) {
        painter.add(EguiShape::line(contour, stroke));
    }
}

/// One column: the layer's title and its thumbnails. A thumbnail dropped on another queues the
/// reorder.
fn column(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, glyph: &str, col: usize, (layer, title): &(&Layer, String), pending: &mut Pending) {
    let outlines: Vec<Option<BezPath>> = layer.shapes.iter().take(MAX_SHAPES).map(|s| outline(font, layer, s)).collect();
    let mut whole = BezPath::new();
    for p in outlines.iter().flatten() {
        whole.extend(p.iter());
    }
    let to_rect = |rect: Rect| fit(&whole, rect);
    let count = outlines.len();
    let order: Vec<usize> = (0..count).collect();
    ui.vertical(|ui| {
        ui.label(RichText::new(title).strong());
        for (at, shape) in outlines.iter().enumerate() {
            let id = egui::Id::new(("window.shapeOrder", col, at));
            let response = ui
                .dnd_drag_source(id, (col, at), |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(tokens.cell_size), Sense::hover());
                    let painter = ui.painter_at(rect);
                    painter.rect_stroke(rect, tokens.radius, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
                    if let Some(t) = to_rect(rect) {
                        stroke_path(&painter, &whole, t, Stroke::new(1.0, tokens.text_dim));
                        let colour = tokens.color_labels.get(at % tokens.color_labels.len()).copied().unwrap_or(tokens.accent);
                        if let Some(p) = shape {
                            stroke_path(&painter, p, t, Stroke::new(tokens.outline_width.max(1.5), colour));
                        }
                    }
                    let corner = rect.left_top() + Vec2::splat(tokens.dialog_spacing / 2.0);
                    painter.text(corner, egui::Align2::LEFT_TOP, format!("{at}"), egui::FontId::proportional(tokens.cell_label_size), tokens.text);
                })
                .response
                .on_hover_text(format!("Shape {at}: drop on another shape of this layer to move it there"));
            if let Some(from) = response.dnd_release_payload::<(usize, usize)>()
                && from.0 == col
                && let Some(next) = moved(&order, from.1, at)
            {
                pending.push(("glyph.setShapeOrder", json!({"glyph": glyph, "layer": layer.id, "order": next})));
            }
        }
        if layer.shapes.len() > MAX_SHAPES {
            ui.label(RichText::new(format!("and {} more", layer.shapes.len() - MAX_SHAPES)).color(tokens.text_dim));
        }
    });
}

/// Draws the window while it is open; it blocks nothing.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.shape_order_window.open {
        return;
    }
    let glyph_name = shown_glyph(app);
    let tokens = &app.tokens;
    let mut open = true;
    egui::Window::new("Shape Order")
        .id(egui::Id::new("window.shapeOrder"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_pos(Pos2::new(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            let font = app.session.font().ok();
            let glyph = font.zip(glyph_name.as_deref()).and_then(|(f, name)| f.glyph(name).map(|g| (f, g)));
            let Some((font, glyph)) = glyph else {
                ui.label(RichText::new("Open a glyph to see its shapes").color(tokens.text_dim));
                return;
            };
            ui.label(RichText::new(&glyph.name).strong().size(tokens.dialog_title_size));
            ui.add_space(tokens.dialog_spacing);
            egui::ScrollArea::both().show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (col, entry) in columns(font, glyph).iter().enumerate() {
                        column(ui, tokens, font, &glyph.name, col, entry, pending);
                    }
                });
            });
        });
    if !open {
        app.state.panels.shape_order_window.open = false;
    }
}
