//! The Bitmap Glyph Editor's shell commands, view state and strokes.
//!
//! | Command | Does |
//! |---|---|
//! | `window.bitmapEditor {open?, glyph?, strike?}` | shows or hides the window, on a named glyph and strike or on the glyph being edited and the font's first strike |
//! | `view.setBitmapEditor {tool?, level?, outline?, grid?, pixelSize?, fit?}` | picks the tool and the shade it draws with, switches the outline guide and the pixel grid, and sets how large a pixel shows |
//!
//! A press on the pixel canvas starts a stroke of the current tool (`fontcraft_tools::bitmap`); the stroke previews
//! as `bitmap.setPixels` on a snapshot while the pointer moves, and its release keeps it as one undo step. Alt with a
//! press in a grey strike picks the level under the pointer as the shade instead. `ui.inspect` reports the window
//! under `bitmapEditor`.

mod canvas;

use craft::behavior::dock::PanelState;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::strike::{BitmapStrike, StrikeGlyph, max_level};
use fontcraft_tools::bitmap::{BitmapStroke, BitmapTool, StrokeStart, pixel_at};
use serde_json::{Map, Value, json};

pub use canvas::{BitmapCanvas, PixelFrame, PixelTool};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;
use crate::app::models::bitmap_editor::{draw_level, shown};

pub const MODULE: Module = Module { commands: COMMANDS, run };

pub const WINDOW: &str = "window.bitmapEditor";
pub const SET: &str = "view.setBitmapEditor";

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: WINDOW,
        label: "Bitmap Glyph Editor",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true), glyph?: name (default: the glyph being edited, followed as it changes), strike?: {ppem, depth?} (default: the font's first strike; without depth the lowest depth of that size)} show or hide the Bitmap Glyph Editor: the glyph's bitmap in the strike, its outline scaled to the strike behind it, the bitmap at its actual size, the drawing, magnify and hand tools, the Shades palette in a grey strike and the Strike Properties button. Each stroke is one bitmap.setPixels. ui.inspect reports {open, glyph, strike, tool, level, maxLevel, outline, grid, pixelSize (null: fitted to the window), bitmap: {width, height, left, bottom, advance} | null} under bitmapEditor",
    },
    UiCommand {
        id: SET,
        label: "Bitmap Glyph Editor Options",
        menu: &[],
        shortcut: None,
        params: "{tool?: \"pencil\" | \"line\" | \"shift\" | \"rectangle\" | \"filledRectangle\" | \"ellipse\" | \"filledEllipse\" | \"magnify\" | \"hand\", level?: number (0 to 255, the shade the tools draw with; above the strike's top level draws the top level), outline?: bool, grid?: bool, pixelSize?: number (points a pixel shows at, 2 to 48), fit?: bool (true: the pixels fit the window again)} the Bitmap Glyph Editor's tool, shade, overlays and pixel size",
    },
];

/// Smallest and largest size a pixel shows at, in points.
pub const MIN_PIXEL_SIZE: u8 = 2;
pub const MAX_PIXEL_SIZE: u8 = 48;

/// A tool of the editor: one that draws a stroke, or one that moves the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorTool {
    Draw(BitmapTool),
    Magnify,
    Hand,
}

impl EditorTool {
    /// The word `view.setBitmapEditor` takes for the tool.
    pub fn word(self) -> &'static str {
        match self {
            EditorTool::Draw(tool) => tool.word(),
            EditorTool::Magnify => "magnify",
            EditorTool::Hand => "hand",
        }
    }

    pub fn from_word(word: &str) -> Option<EditorTool> {
        match word {
            "magnify" => Some(EditorTool::Magnify),
            "hand" => Some(EditorTool::Hand),
            _ => BitmapTool::from_word(word).map(EditorTool::Draw),
        }
    }
}

/// The window's view state.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapEditorView {
    /// The glyph the window was opened on; `None` follows the glyph being edited.
    pub glyph: Option<String>,
    /// The strike `(ppem, depth)` it was opened on; `None` shows the font's first strike.
    pub strike: Option<(u16, u8)>,
    pub tool: EditorTool,
    /// The shade the tools draw with; `None` draws the strike's top level.
    pub level: Option<u8>,
    /// Whether the outline guide and the pixel grid show.
    pub outline: bool,
    pub grid: bool,
    /// Points a pixel shows at; `None` fits the pixels to the window.
    pub pixel_size: Option<u8>,
    /// The stroke in progress and the glyph and strike it draws on.
    pub stroke: Option<(BitmapStroke, String, (u16, u8))>,
}

impl Default for BitmapEditorView {
    fn default() -> Self {
        BitmapEditorView {
            glyph: None,
            strike: None,
            tool: EditorTool::Draw(BitmapTool::Pencil),
            level: None,
            outline: true,
            grid: true,
            pixel_size: None,
            stroke: None,
        }
    }
}

/// What the pixel canvas reports, in strike pixels from the glyph origin (x rightwards, y upwards).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BitmapInput {
    /// The primary button went down; `alt` when Alt was held.
    Press {
        at: [f64; 2],
        alt: bool,
    },
    Move([f64; 2]),
    Release([f64; 2]),
    /// The gesture was given up (Escape, a lost pointer): the stroke changes nothing.
    Cancel,
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        WINDOW => window(app, &command.params),
        SET => set(app, &command.params),
        other => Err(format!("no Bitmap Glyph Editor command `{other}`")),
    }
}

fn window(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let open = flag(p, "open")?.unwrap_or(true);
    if open {
        let font = app.session.font().map_err(|e| e.to_string())?;
        if font.bitmap_strikes.is_empty() {
            return Err("the font has no bitmap strikes: add one with bitmap.setStrikes first".into());
        }
        let named = string(p, "glyph")?.map(str::to_string);
        match &named {
            Some(name) if font.glyph(name).is_none() => return Err(format!("the font has no glyph `{}`", quoted(name))),
            _ => {}
        }
        let strike = match p.get("strike") {
            None | Some(Value::Null) => None,
            Some(v) => Some(find_strike(&font.bitmap_strikes, v)?),
        };
        cancel_stroke(app);
        app.view.bitmap_editor.glyph = named;
        app.view.bitmap_editor.strike = strike;
    }
    crate::shell::dock::show_window(&mut app.view.workspace, PanelId::BitmapEditor, open, app.view.canvas.rect);
    Ok(())
}

/// The strike `{ppem, depth?}` names in `strikes`: without `depth`, the lowest depth of that size.
fn find_strike(strikes: &[BitmapStrike], v: &Value) -> Result<(u16, u8), String> {
    let Value::Object(s) = v else { return Err("`strike` must be {ppem, depth?}".into()) };
    let ppem = s.get("ppem").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or("`strike.ppem` must be a whole number of pixels")?;
    let depth = match s.get("depth") {
        None | Some(Value::Null) => None,
        Some(d) => Some(d.as_u64().and_then(|d| u8::try_from(d).ok()).ok_or("`strike.depth` must be 1, 2, 4 or 8")?),
    };
    strikes.iter().filter(|s| s.ppem == ppem && depth.is_none_or(|d| s.depth == d)).min_by_key(|s| s.depth).map(|s| (s.ppem, s.depth)).ok_or_else(
        || {
            let label = depth.map_or_else(|| ppem.to_string(), |d| fontcraft_engine::model::strike::label(ppem, d));
            let all: Vec<String> = strikes.iter().map(BitmapStrike::label).collect();
            format!("the font has no strike {label}; its strikes: {}", all.join(", "))
        },
    )
}

fn set(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let tool = match string(p, "tool")? {
        None => None,
        Some(word) => Some(EditorTool::from_word(word).ok_or_else(|| {
            let draw: Vec<&str> = BitmapTool::ALL.iter().map(|t| t.word()).collect();
            format!("`tool` must be {}, magnify or hand, not `{}`", draw.join(", "), quoted(word))
        })?),
    };
    let level = match p.get("level") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|l| u8::try_from(l).ok()).ok_or("`level` must be a whole number from 0 to 255")?),
    };
    let outline = flag(p, "outline")?;
    let grid = flag(p, "grid")?;
    let pixel_size = match p.get("pixelSize") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .filter(|n| (MIN_PIXEL_SIZE..=MAX_PIXEL_SIZE).contains(n))
                .ok_or_else(|| format!("`pixelSize` must be a whole number from {MIN_PIXEL_SIZE} to {MAX_PIXEL_SIZE}"))?,
        ),
    };
    let fit = flag(p, "fit")?.unwrap_or(false);
    if fit && pixel_size.is_some() {
        return Err("give `pixelSize` or `fit`, not both".into());
    }
    if tool.is_some_and(|t| t != app.view.bitmap_editor.tool) {
        cancel_stroke(app);
    }
    let state = &mut app.view.bitmap_editor;
    if pixel_size.is_some() || fit {
        state.pixel_size = pixel_size;
    }
    if let Some(tool) = tool {
        state.tool = tool;
    }
    if level.is_some() {
        state.level = level;
    }
    if let Some(on) = outline {
        state.outline = on;
    }
    if let Some(on) = grid {
        state.grid = on;
    }
    Ok(())
}

/// Applies what the pixel canvas reported: starts, previews, keeps or drops a stroke.
pub fn input(app: &mut FontCraftApp, input: BitmapInput) -> Result<(), String> {
    match input {
        BitmapInput::Press { at, alt } => press(app, at, alt),
        BitmapInput::Move(at) => moved(app, at),
        BitmapInput::Release(at) => {
            moved(app, at)?;
            let Some((stroke, glyph, strike)) = app.view.bitmap_editor.stroke.take() else { return Ok(()) };
            let changed = stroke.params(&glyph, strike_param(strike)).is_some();
            if app.session.interaction().is_none() {
                return Ok(());
            }
            if changed { app.session.commit().map(drop).map_err(|e| e.to_string()) } else { app.session.cancel().map_err(|e| e.to_string()) }
        }
        BitmapInput::Cancel => {
            cancel_stroke(app);
            Ok(())
        }
    }
}

fn strike_param((ppem, depth): (u16, u8)) -> Value {
    json!({"ppem": ppem, "depth": depth})
}

fn press(app: &mut FontCraftApp, at: [f64; 2], alt: bool) -> Result<(), String> {
    cancel_stroke(app);
    let EditorTool::Draw(tool) = app.view.bitmap_editor.tool else { return Ok(()) };
    let Some(pixel) = pixel_at(at[0], at[1]) else { return Ok(()) };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let Some((glyph, strike)) = shown(font, &app.view) else { return Err("the Bitmap Glyph Editor shows no glyph".into()) };
    let own = strike.glyph(&glyph.name);
    let under = own.map_or(0, |b| level_at(b, strike.depth, pixel));
    if alt && strike.depth > 1 {
        app.view.bitmap_editor.level = Some(under.min(max_level(strike.depth)));
        return Ok(());
    }
    if tool == BitmapTool::Shift && own.is_none_or(|b| b.width == 0 || b.height == 0) {
        return Ok(());
    }
    let start = StrokeStart { level_under: under, draw_level: draw_level(&app.view, strike), origin: own.map_or((0, 0), |b| (b.left, b.bottom)) };
    let stroke = BitmapStroke::begin(tool, pixel, start);
    let (name, key) = (glyph.name.clone(), (strike.ppem, strike.depth));
    if app.session.interaction().is_some() {
        return Err("another edit is under way".into());
    }
    app.session.begin(tool.undo_label()).map_err(|e| e.to_string())?;
    if let Some(params) = stroke.params(&name, strike_param(key))
        && let Err(e) = app.session.preview("bitmap.setPixels", &params)
    {
        let _ = app.session.cancel();
        return Err(format!("bitmap.setPixels: {e}"));
    }
    app.view.bitmap_editor.stroke = Some((stroke, name, key));
    Ok(())
}

fn moved(app: &mut FontCraftApp, at: [f64; 2]) -> Result<(), String> {
    let Some(pixel) = pixel_at(at[0], at[1]) else { return Ok(()) };
    let Some((stroke, glyph, strike)) = app.view.bitmap_editor.stroke.as_mut() else { return Ok(()) };
    let before = stroke.params(glyph, strike_param(*strike));
    stroke.extend(pixel);
    let after = stroke.params(glyph, strike_param(*strike));
    if after == before {
        return Ok(());
    }
    // A shift back to where it started previews the bitmap as it was.
    let result = match after {
        Some(params) => app.session.preview("bitmap.setPixels", &params),
        None => {
            let (x, y) = stroke.origin();
            app.session.preview("bitmap.setPixels", &json!({"glyph": glyph, "strike": strike_param(*strike), "origin": [x, y]}))
        }
    };
    result.map(drop).map_err(|e| format!("bitmap.setPixels: {e}"))
}

/// Drops a stroke in progress and puts the bitmap back as it was.
fn cancel_stroke(app: &mut FontCraftApp) {
    if app.view.bitmap_editor.stroke.take().is_some() && app.session.interaction().is_some() {
        let _ = app.session.cancel();
    }
}

/// The level of the pixel `(x, y)` (from the glyph origin, y upwards) in `bitmap`; 0 outside it.
fn level_at(bitmap: &StrikeGlyph, depth: u8, (x, y): (i32, i32)) -> u8 {
    let column = i64::from(x) - i64::from(bitmap.left);
    // Rows count from the top.
    let row = i64::from(bitmap.bottom) + i64::from(bitmap.height) - 1 - i64::from(y);
    match (u16::try_from(column), u16::try_from(row)) {
        (Ok(c), Ok(r)) => bitmap.pixel(depth, c, r),
        _ => 0,
    }
}

/// `ui.inspect`'s `bitmapEditor`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let open = app.view.workspace.layout.panel_state(&PanelId::BitmapEditor) != PanelState::Hidden;
    let view = &app.view.bitmap_editor;
    let shown = app.session.font().ok().and_then(|font| shown(font, &app.view));
    let bitmap = shown
        .and_then(|(glyph, strike)| strike.glyph(&glyph.name))
        .map(|b| json!({"width": b.width, "height": b.height, "left": b.left, "bottom": b.bottom, "advance": b.advance}));
    let out = json!({
        "open": open,
        "glyph": shown.map(|(glyph, _)| glyph.name.clone()),
        "strike": shown.map(|(_, strike)| strike.label()),
        "tool": view.tool.word(),
        "level": shown.map(|(_, strike)| draw_level(&app.view, strike)),
        "maxLevel": shown.map(|(_, strike)| max_level(strike.depth)),
        "outline": view.outline,
        "grid": view.grid,
        "pixelSize": view.pixel_size,
        "bitmap": bitmap,
    });
    state.insert("bitmapEditor".into(), out);
}

#[cfg(test)]
mod tests;
