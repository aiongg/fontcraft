//! The Bitmap Glyph Editor: one glyph's bitmap in one strike, drawn pixel by pixel over the strike's line band, with
//! the glyph's outline scaled to the strike behind it as a guide.
//!
//! The top row holds the glyph and strike, the bitmap at its actual size (one screen pixel a pixel), the tool (Magnify,
//! Pencil, Line, Shift, Hand), the Outline and Grid switches, Recalculate (`bitmap.regenerateGlyphs` on this glyph and
//! strike) and Strike Properties (`dialog.strikeProperties` on this strike). A secondary click on the canvas opens its
//! menu: the rectangle and ellipse tools, the flips and quarter turns (`bitmap.transform`), Set Width
//! (`dialog.bitmapWidth`) and Fit in Window. A Magnify click shows the pixels larger around the pixel clicked, with Alt
//! smaller; a Hand drag moves the view. A grey strike adds the Shades palette: one swatch per level, the drawing level marked; a click picks the
//! level. The canvas reports presses, drags and releases in strike pixels as `ViewMsg::Bitmap`; the shell turns each
//! stroke into one `bitmap.setPixels` (`shell::bitmap_editor`).

use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Point, Size};
use craft::ui::buttons::Button;
use craft::ui::canvas::{Canvas, ToolHost, canvas_space, tool_pass};
use craft::ui::choice::{Checkbox, Segmented, SwatchTable};
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::menu::Menu;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use egui::{Mesh, Painter, Rect, Response, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::app::models::bitmap_editor::BitmapEditorModel;
use crate::app::models::font_view::{StrikeCell, StrikeShown};
use crate::app::{Msg, ViewMsg};
use crate::canvas::tokens::Tokens;
use crate::panels::attributes::srgba;
use crate::panels::font_view::cells::strike_texture;
use crate::shell::bitmap_editor::{BitmapCanvas, MAX_PIXEL_SIZE, MIN_PIXEL_SIZE, PixelFrame, SET};

/// Pixels of room around the glyph's box and line band.
const PAD: i32 = 2;
/// Farthest pixel from the glyph origin the canvas shows either way.
const FRAME_LIMIT: i32 = 2_048;
/// The pixel sizes the Magnify tool steps through, in points.
const ZOOM_STEPS: [u8; 9] = [2, 3, 4, 6, 8, 12, 16, 32, 48];
/// Largest side of the actual-size preview, in points; a larger bitmap is cut off.
const MAX_PREVIEW: f32 = 256.0;
/// The grid shows while a pixel is at least this many points a side.
const MIN_GRID_CELL: f64 = 4.0;
/// How opaque the grid's lines are against the metric lines.
const GRID_SHARE: f32 = 0.35;
/// The values of the canvas menu's items: a tool's word, a transform's `op`, or one of these.
const FIT: &str = "fit";
const SET_WIDTH: &str = "setWidth";
const TRANSFORMS: [&str; 5] = ["flipH", "flipV", "rotate90cw", "rotate90ccw", "rotate180"];
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
        magnify: "fontcraftBitmapEditor.magnify" = "Magnify";
        hand: "fontcraftBitmapEditor.hand" = "Hand";
        actual_size: "fontcraftBitmapEditor.actualSize" = "Actual Size";
        menu: "fontcraftBitmapEditor.menu" = "Bitmap Menu";
        rectangle: "fontcraftBitmapEditor.rectangle" = "Rectangle";
        filled_rectangle: "fontcraftBitmapEditor.filledRectangle" = "Filled Rectangle";
        ellipse: "fontcraftBitmapEditor.ellipse" = "Ellipse";
        filled_ellipse: "fontcraftBitmapEditor.filledEllipse" = "Filled Ellipse";
        flip_h: "fontcraftBitmapEditor.flipH" = "Flip Horizontally";
        flip_v: "fontcraftBitmapEditor.flipV" = "Flip Vertically";
        rotate_cw: "fontcraftBitmapEditor.rotateCw" = "Rotate 90° Clockwise";
        rotate_ccw: "fontcraftBitmapEditor.rotateCcw" = "Rotate 90° Counterclockwise";
        rotate_180: "fontcraftBitmapEditor.rotate180" = "Rotate 180°";
        set_width: "fontcraftBitmapEditor.setWidth" = "Set Width…";
        /// Why Set Width is off in a font with outlines.
        width_follows: "fontcraftBitmapEditor.widthFollows" = "The width follows the outline";
        fit: "fontcraftBitmapEditor.fit" = "Fit in Window";
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
    top_row(ui, &words, model, glyph, strike, props.tokens, sent);
    if strike.depth > 1 {
        shades(ui, &words, model, sent);
    }
    if model.bitmap.is_none() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
    }
    pixels(ui, &words, model, strike, props.tokens, props.canvas, sent);
}

fn top_row(
    ui: &mut Ui,
    words: &BitmapEditorWords,
    model: &BitmapEditorModel,
    glyph: &str,
    strike: StrikeShown,
    tokens: &Tokens,
    sent: &mut Vec<Msg>,
) {
    let strike_param = json!({"ppem": strike.ppem, "depth": strike.depth});
    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        Text::new(Label::from(format!("{glyph}  {}", model.strike_label))).show(ui);
        actual_size(ui, words, model, strike, tokens);
        let entries = vec![
            Entry::new("magnify".to_string(), words.magnify.clone()),
            Entry::new("pencil".to_string(), words.pencil.clone()),
            Entry::new("line".to_string(), words.line.clone()),
            Entry::new("shift".to_string(), words.shift.clone()),
            Entry::new("hand".to_string(), words.hand.clone()),
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

/// The glyph's line band and bitmap at one screen pixel a pixel.
fn actual_size(ui: &mut Ui, words: &BitmapEditorWords, model: &BitmapEditorModel, strike: StrikeShown, tokens: &Tokens) {
    let (left, bottom, right, top) = shown_box(model, strike);
    let pixel = 1.0 / ui.pixels_per_point().max(f32::MIN_POSITIVE);
    let side = |pixels: i32| (pixels.max(1) as f32 * pixel).min(MAX_PREVIEW);
    let (rect, response) = ui.allocate_exact_size(vec2(side(right.saturating_sub(left)), side(top.saturating_sub(bottom))), Sense::hover());
    let label = words.actual_size.as_str().to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, label.clone()));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, tokens.canvas);
    let Some(cell) = &model.bitmap else { return };
    // On whole screen pixels, so the bitmap shows unscaled.
    let snap = |v: f32| (v / pixel).round() * pixel;
    let min = pos2(
        snap(rect.left()) + cell.left.saturating_sub(left) as f32 * pixel,
        snap(rect.top()) + top.saturating_sub(cell.bottom.saturating_add(i32::from(cell.height))) as f32 * pixel,
    );
    let size = vec2(f32::from(cell.width) * pixel, f32::from(cell.height) * pixel);
    paint_bitmap(&painter, Rect::from_min_size(min, size), cell, tokens);
}

/// Paints `cell`'s bitmap over `rect`.
fn paint_bitmap(painter: &Painter, rect: Rect, cell: &StrikeCell, tokens: &Tokens) {
    let mut mesh = Mesh::with_texture(strike_texture(painter.ctx(), "fontcraftBitmapEditor", cell));
    mesh.add_rect_with_uv(rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tokens.outline);
    painter.add(Shape::mesh(mesh));
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
    let BitmapCanvas { view, tool, hand, centred, focus, menu_at } = canvas;
    let room = if view.rect.is_positive() { view.rect } else { ui.available_rect_before_wrap() };
    let fit = (f64::from(room.width()) / cols).min(f64::from(room.height()) / rows);
    let (least, most) = (f64::from(MIN_PIXEL_SIZE), f64::from(MAX_PIXEL_SIZE));
    let fitted = if fit.is_finite() { fit.floor().clamp(least, most) } else { least };
    let cell = model.pixel_size.map_or(fitted, |size| f64::from(size).clamp(least, most));
    let size = (cols * cell, rows * cell);
    // A new glyph, strike or room size shows the whole box in the middle of the view; a pixel size the person set keeps
    // the pixel it was set on there, else the one that was there.
    let key = (size.0 as u32, size.1 as u32);
    if !view.rect.is_positive() || *centred != Some(key) {
        let kept = focus.take().or_else(|| tool.frame().and_then(|f| f.to_pixels(room.center()))).filter(|_| model.pixel_size.is_some());
        let centre =
            kept.map_or(Point::new(size.0 / 2.0, size.1 / 2.0), |[x, y]| Point::new((x - f64::from(left)) * cell, (f64::from(top) - y) * cell));
        *view = CanvasView::new(room, centre, 1.0);
        *centred = Some(key);
    }
    let canvas = Canvas::new(words.canvas.clone()).view(Bind::bound(&mut *view)).document(Size::new(size.0, size.1));
    let mut reply = None;
    let tool = &mut *tool;
    tool.set_magnify(model.tool == "magnify");
    let shown = canvas.show(ui, |ui, body| {
        let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
        let frame = PixelFrame { to_screen: space.view.affine(), cell, left: f64::from(left), top: f64::from(top) };
        let painter = ui.painter_at(space.view.rect);
        paint(&painter, &frame, (left, bottom, right, top), model, strike, tokens);
        tool.set_frame(Some(frame));
        if let Some(pass) = tool_pass(&body.env) {
            let host = ToolHost::new();
            reply = Some(if model.tool == "hand" { host.run(ui, &pass, &mut *hand) } else { host.run(ui, &pass, &mut *tool) });
        }
    });
    let opened = reply.as_ref().and_then(|r| r.context).map(|c| c.at);
    if let Some(moved) = reply.and_then(|r| r.view) {
        *view = moved;
    }
    // The editor sets the pixel size itself.
    view.zoom = 1.0;
    view.rotation = 0.0;
    view.flip_x = false;
    for (at, down) in tool.take_zooms() {
        if let Some(size) = zoom_step(cell, down) {
            *focus = Some(at);
            sent.push(set_msg(json!({"pixelSize": size})));
        }
    }
    sent.extend(tool.take_inputs().into_iter().map(|input| Msg::View(ViewMsg::Bitmap(input))));
    if let Some(at) = *menu_at
        && !menu(ui, &shown.output.response, at, words, model, strike, sent)
    {
        *menu_at = None;
    }
    // The menu shows from the next pass on.
    if opened.is_some() {
        *menu_at = opened;
        ui.ctx().request_repaint();
    }
}

/// The pixel size one Magnify step up (or `down`) from `cell`; `None` at the end of the steps.
fn zoom_step(cell: f64, down: bool) -> Option<u8> {
    let mut steps = ZOOM_STEPS.iter().copied();
    if down { steps.rfind(|s| f64::from(*s) < cell) } else { steps.find(|s| f64::from(*s) > cell) }
}

/// The canvas menu at `at`; returns whether it stays open.
fn menu(
    ui: &mut Ui,
    opener: &Response,
    at: egui::Pos2,
    words: &BitmapEditorWords,
    model: &BitmapEditorModel,
    strike: StrikeShown,
    sent: &mut Vec<Msg>,
) -> bool {
    let Some(glyph) = model.glyph.as_deref() else { return false };
    let drawn = model.bitmap.is_some();
    let tool = |word: &'static str, label: &Label| Entry::new(word, label.clone()).checked(model.tool == word);
    let transform = |op: &'static str, label: &Label| Entry::new(op, label.clone()).enabled(drawn);
    let width = Entry::new(SET_WIDTH, words.set_width.clone());
    let width = if model.has_outlines { width.disabled_reason(words.width_follows.clone()) } else { width };
    let entries: Vec<Entry<&'static str>> = vec![
        tool("rectangle", &words.rectangle),
        tool("filledRectangle", &words.filled_rectangle),
        tool("ellipse", &words.ellipse),
        tool("filledEllipse", &words.filled_ellipse),
        Entry::separator(),
        transform("flipH", &words.flip_h),
        transform("flipV", &words.flip_v),
        transform("rotate90cw", &words.rotate_cw),
        transform("rotate90ccw", &words.rotate_ccw),
        transform("rotate180", &words.rotate_180),
        Entry::separator(),
        width,
        Entry::separator(),
        Entry::new(FIT, words.fit.clone()).checked(model.pixel_size.is_none()),
    ];
    let mut open = true;
    let shown = Menu::context(words.menu.clone(), opener).anchor(Anchor::Point(at)).open(Bind::bound(&mut open)).pick(ui, &entries);
    let strike = json!({"ppem": strike.ppem, "depth": strike.depth});
    match shown.picked {
        Some(FIT) => sent.push(set_msg(json!({"fit": true}))),
        Some(SET_WIDTH) => {
            let params = json!({"dialog": "dialog.bitmapWidth", "params": {"glyph": glyph, "strike": strike}});
            sent.push(Msg::Shell(CommandRef::with_params("dialog.open", params)));
        }
        Some(op) if TRANSFORMS.contains(&op) => {
            sent.push(Msg::run("bitmap.transform", json!({"glyphs": [glyph], "strike": strike, "op": op})));
        }
        Some(word) => sent.push(set_msg(json!({"tool": word}))),
        None => {}
    }
    open && shown.picked.is_none() && shown.closed.is_none()
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
            paint_bitmap(painter, Rect::from_two_pos(p, q), cell, tokens);
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
