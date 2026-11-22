//! The Bitmap Glyph Editor: one glyph's bitmap in one strike, drawn pixel by pixel over the strike's line band, with
//! the glyph's outline scaled to the strike behind it as a guide.
//!
//! The top row holds the glyph and strike, the tool (Pencil, Line, Shift), the Outline and Grid switches, Recalculate
//! (`bitmap.regenerateGlyphs` on this glyph and strike) and Strike Properties (`dialog.strikeProperties` on this
//! strike). A grey strike adds the Shades palette: one swatch per level, the drawing level marked; a click picks the
//! level. The canvas reports presses, drags and releases in strike pixels as `ViewMsg::Bitmap`; the shell turns each
//! stroke into one `bitmap.setPixels` (`shell::bitmap_editor`).

use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Point, Size};
use craft::ui::buttons::Button;
use craft::ui::canvas::{Canvas, ToolHost, canvas_space, tool_pass};
use craft::ui::choice::{Checkbox, Segmented, SwatchTable};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use egui::{Mesh, Painter, Rect, Shape, Stroke, Ui, pos2};
use serde_json::json;

use crate::app::models::bitmap_editor::BitmapEditorModel;
use crate::app::models::font_view::StrikeShown;
use crate::app::{Msg, ViewMsg};
use crate::canvas::tokens::Tokens;
use crate::panels::attributes::srgba;
use crate::panels::font_view::cells::strike_texture;
use crate::shell::bitmap_editor::{BitmapCanvas, PixelFrame, SET};

/// Pixels of room around the glyph's box and line band.
const PAD: i32 = 2;
/// Farthest pixel from the glyph origin the canvas shows either way.
const FRAME_LIMIT: i32 = 2_048;
/// Smallest and largest side of a pixel, in points.
const MIN_CELL: f64 = 2.0;
const MAX_CELL: f64 = 48.0;
/// The grid shows while a pixel is at least this many points a side.
const MIN_GRID_CELL: f64 = 4.0;
/// How opaque the grid's lines are against the metric lines.
const GRID_SHARE: f32 = 0.35;
/// Swatches to a row of the Shades palette.
const SHADES_PER_ROW: usize = 16;

craft::ui::words! {
    /// The words of the Bitmap Glyph Editor.
    pub struct BitmapEditorWords {
        no_strikes: "fontcraftBitmapEditor.noStrikes" = "The font has no bitmap strikes";
        no_glyph: "fontcraftBitmapEditor.noGlyph" = "Open a glyph to draw its bitmap";
        tool: "fontcraftBitmapEditor.tool" = "Tool";
        pencil: "fontcraftBitmapEditor.pencil" = "Pencil";
        line: "fontcraftBitmapEditor.line" = "Line";
        shift: "fontcraftBitmapEditor.shift" = "Shift";
        outline: "fontcraftBitmapEditor.outline" = "Outline";
        grid: "fontcraftBitmapEditor.grid" = "Grid";
        recalculate: "fontcraftBitmapEditor.recalculate" = "Recalculate";
        properties: "fontcraftBitmapEditor.properties" = "Strike Properties…";
        shades: "fontcraftBitmapEditor.shades" = "Shades";
        /// One swatch of the Shades palette.
        level: "fontcraftBitmapEditor.level" = "Level {n}";
        canvas: "fontcraftBitmapEditor.canvas" = "Bitmap";
        /// Shown when the glyph has no bitmap in the strike yet.
        empty: "fontcraftBitmapEditor.empty" = "No bitmap yet: draw to start one";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<BitmapEditorModel>,
    pub tokens: &'a Tokens,
    /// The pixel canvas's view and tool (`FontCraftApp::bitmap_canvas`).
    pub canvas: &'a mut BitmapCanvas,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn set_msg(params: serde_json::Value) -> Msg {
    Msg::Shell(CommandRef::with_params(SET, params))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<BitmapEditorWords>();
    let model: &BitmapEditorModel = props.model;
    if !model.has_strikes {
        Text::new(words.no_strikes.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let (Some(glyph), Some(strike)) = (model.glyph.as_deref(), model.strike) else {
        Text::new(words.no_glyph.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let sent = &mut *props.sent;
    top_row(ui, &words, model, glyph, strike, sent);
    if strike.depth > 1 {
        shades(ui, &words, model, sent);
    }
    if model.bitmap.is_none() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
    }
    pixels(ui, &words, model, strike, props.tokens, props.canvas, sent);
}

fn top_row(ui: &mut Ui, words: &BitmapEditorWords, model: &BitmapEditorModel, glyph: &str, strike: StrikeShown, sent: &mut Vec<Msg>) {
    let strike_param = json!({"ppem": strike.ppem, "depth": strike.depth});
    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        Text::new(Label::from(format!("{glyph}  {}", model.strike_label))).show(ui);
        let entries = vec![
            Entry::new("pencil".to_string(), words.pencil.clone()),
            Entry::new("line".to_string(), words.line.clone()),
            Entry::new("shift".to_string(), words.shift.clone()),
        ];
        let mut tool = model.tool.clone();
        Segmented::new(words.tool.clone(), Bind::bound(&mut tool), &entries).show(ui);
        if tool != model.tool {
            sent.push(set_msg(json!({"tool": tool})));
        }
        if let Some(on) = Checkbox::new(Bind::given(model.show_outline)).label(words.outline.clone()).show(ui).events.committed() {
            sent.push(set_msg(json!({"outline": on})));
        }
        if let Some(on) = Checkbox::new(Bind::given(model.show_grid)).label(words.grid.clone()).show(ui).events.committed() {
            sent.push(set_msg(json!({"grid": on})));
        }
        if Button::new(words.recalculate.clone()).show(ui).clicked() {
            sent.push(Msg::run("bitmap.regenerateGlyphs", json!({"glyphs": [glyph], "strikes": [strike_param]})));
        }
        if Button::new(words.properties.clone()).show(ui).clicked() {
            let params = json!({"dialog": "dialog.strikeProperties", "params": {"strike": strike_param}});
            sent.push(Msg::Shell(CommandRef::with_params("dialog.open", params)));
        }
    });
}

/// The Shades palette: one swatch per level of a grey strike, from clear to the top level, the drawing level marked.
fn shades(ui: &mut Ui, words: &BitmapEditorWords, model: &BitmapEditorModel, sent: &mut Vec<Msg>) {
    let top = u32::from(model.max_level.max(1));
    let cells: Vec<(Label, craft::theme::Srgba)> = (0..=top)
        .map(|level| {
            let grey = (255 - level * 255 / top) as u8;
            (Label::from(words.level.as_str().replace("{n}", &level.to_string())), srgba([grey, grey, grey, 0xff]))
        })
        .collect();
    let level = usize::from(model.level);
    let out = SwatchTable::new(words.shades.clone(), &cells)
        .per_row(cells.len().min(SHADES_PER_ROW))
        .marked(move |at: &usize| *at == level)
        .tips(true)
        .show_all(ui);
    if let Some(picked) = out.selected.filter(|at| *at != level) {
        sent.push(set_msg(json!({"level": picked})));
    }
}

/// The pixel box the canvas shows, `(left, bottom, right, top)` in pixels from the glyph origin: the line band, the
/// advance and the bitmap, with room around them.
fn shown_box(model: &BitmapEditorModel, strike: StrikeShown) -> (i32, i32, i32, i32) {
    let (mut left, mut bottom, mut right, mut top) = (0, -strike.descent.max(0), model.advance.max(1), strike.ascent.max(1));
    if let Some(b) = &model.bitmap {
        left = left.min(b.left);
        bottom = bottom.min(b.bottom);
        right = right.max(b.left.saturating_add(i32::from(b.width)));
        top = top.max(b.bottom.saturating_add(i32::from(b.height)));
    }
    let clamp = |v: i32| v.clamp(-FRAME_LIMIT, FRAME_LIMIT);
    (clamp(left.saturating_sub(PAD)), clamp(bottom.saturating_sub(PAD)), clamp(right.saturating_add(PAD)), clamp(top.saturating_add(PAD)))
}

fn pixels(
    ui: &mut Ui,
    words: &BitmapEditorWords,
    model: &BitmapEditorModel,
    strike: StrikeShown,
    tokens: &Tokens,
    canvas: &mut BitmapCanvas,
    sent: &mut Vec<Msg>,
) {
    let (left, bottom, right, top) = shown_box(model, strike);
    let (cols, rows) = (f64::from((right - left).max(1)), f64::from((top - bottom).max(1)));
    let BitmapCanvas { view, tool, centred } = canvas;
    let room = if view.rect.is_positive() { view.rect } else { ui.available_rect_before_wrap() };
    let fit = (f64::from(room.width()) / cols).min(f64::from(room.height()) / rows);
    let cell = if fit.is_finite() { fit.floor().clamp(MIN_CELL, MAX_CELL) } else { MIN_CELL };
    let size = (cols * cell, rows * cell);
    // A new glyph, strike or room size shows the whole box in the middle of the view.
    let key = (size.0 as u32, size.1 as u32);
    if !view.rect.is_positive() || *centred != Some(key) {
        *view = CanvasView::new(room, Point::new(size.0 / 2.0, size.1 / 2.0), 1.0);
        *centred = Some(key);
    }
    let canvas = Canvas::new(words.canvas.clone()).view(Bind::bound(&mut *view)).document(Size::new(size.0, size.1));
    let mut reply = None;
    let tool = &mut *tool;
    let _ = canvas.show(ui, |ui, body| {
        let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
        let frame = PixelFrame { to_screen: space.view.affine(), cell, left: f64::from(left), top: f64::from(top) };
        let painter = ui.painter_at(space.view.rect);
        paint(&painter, &frame, (left, bottom, right, top), model, strike, tokens);
        tool.set_frame(Some(frame));
        if let Some(pass) = tool_pass(&body.env) {
            reply = Some(ToolHost::new().run(ui, &pass, &mut *tool));
        }
    });
    if let Some(moved) = reply.and_then(|r| r.view) {
        *view = moved;
    }
    // The editor sets the pixel size itself.
    view.zoom = 1.0;
    view.rotation = 0.0;
    view.flip_x = false;
    sent.extend(tool.take_inputs().into_iter().map(|input| Msg::View(ViewMsg::Bitmap(input))));
}

/// Paints the ground, the grid, the metric lines, the bitmap and the outline guide.
fn paint(
    painter: &Painter,
    frame: &PixelFrame,
    (left, bottom, right, top): (i32, i32, i32, i32),
    model: &BitmapEditorModel,
    strike: StrikeShown,
    tokens: &Tokens,
) {
    let at = |x: i32, y: i32| frame.to_screen(f64::from(x), f64::from(y));
    let (Some(a), Some(b)) = (at(left, top), at(right, bottom)) else { return };
    painter.rect_filled(Rect::from_two_pos(a, b), 0.0, tokens.canvas);
    let line = |from: Option<egui::Pos2>, to: Option<egui::Pos2>, stroke: Stroke| {
        if let (Some(p), Some(q)) = (from, to) {
            painter.line_segment([p, q], stroke);
        }
    };
    if model.show_grid && frame.cell >= MIN_GRID_CELL {
        let grid = Stroke::new(tokens.metric_line_width, tokens.metric_line.gamma_multiply(GRID_SHARE));
        for x in left..=right {
            line(at(x, bottom), at(x, top), grid);
        }
        for y in bottom..=top {
            line(at(left, y), at(right, y), grid);
        }
    }
    let metric = Stroke::new(tokens.metric_line_width, tokens.metric_line);
    for y in [0, strike.ascent, -strike.descent] {
        line(at(left, y), at(right, y), metric);
    }
    for x in [0, model.advance] {
        line(at(x, bottom), at(x, top), metric);
    }
    if let Some(cell) = &model.bitmap {
        let corner = |x: i32, y: i32| at(x, y);
        if let (Some(p), Some(q)) = (
            corner(cell.left, cell.bottom.saturating_add(i32::from(cell.height))),
            corner(cell.left.saturating_add(i32::from(cell.width)), cell.bottom),
        ) {
            let texture = strike_texture(painter.ctx(), "fontcraftBitmapEditor", cell);
            let mut mesh = Mesh::with_texture(texture);
            mesh.add_rect_with_uv(Rect::from_two_pos(p, q), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tokens.outline);
            painter.add(Shape::mesh(mesh));
        }
    }
    if model.show_outline {
        let stroke = Stroke::new(tokens.outline_width, tokens.reference_outline);
        for contour in &model.guide.contours {
            let points: Vec<egui::Pos2> = contour.iter().filter_map(|[x, y]| frame.to_screen(f64::from(*x), f64::from(*y))).collect();
            if points.len() > 1 {
                painter.add(Shape::closed_line(points, stroke));
            }
        }
    }
}

#[cfg(test)]
mod tests;
