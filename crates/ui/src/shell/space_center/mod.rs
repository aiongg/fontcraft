//! The Space Center's shell commands and view state: its text, control glyphs, size, line mode, cut-off band,
//! preferred suffix, display options and selected glyph.
//!
//! | Command | Does |
//! |---|---|
//! | `window.spaceCenter {open?, text?, before?, after?, size?, lineMode?, select?}` | shows or hides the window and sets its fields |
//! | `view.setSpacingCutOff {on?, bottom?, top?}` | the band drawn inverted between two heights |
//! | `view.setPreferredSuffix {suffix?}` | shows each glyph's variant with the suffix where the font has one |
//! | `view.setSpaceCenterDisplay {…}` | direction, centring, turning and what is drawn over the glyphs |
//! | `spaceCenter.setValue {column, row, text}` | a matrix entry: evaluates `text` and runs the row's spacing setter |
//!
//! The text is read by [`text`]; the projection (`app::models::space_center`) lays it out for the master shown. The
//! font changes only through engine commands with a number, so nothing stays linked as a metrics key would.

pub mod drag;
pub mod text;

use std::ops::RangeInclusive;

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_model::{Font, MAX_COORDINATE, MAX_NAME_LEN};
use serde_json::{Map, Value, json};

use self::text::{Context, Input, MAX_TEXT_CHARS, Resolved, Row};
use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{self, quoted};
use crate::app::{FontCraftApp, Msg};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Allowed preview sizes, in points.
pub const SIZE_RANGE: RangeInclusive<f64> = 4.0..=1_000.0;
/// The preview size of a new window, in points.
pub const DEFAULT_SIZE: f64 = 72.0;
/// The text of a new window.
pub const DEFAULT_TEXT: &str = "hamburgefontsiv";
/// Longest text typed into a matrix cell, in characters.
pub const MAX_CELL_CHARS: usize = 64;
/// Most overlay layers drawn over the text.
pub const MAX_OVERLAY_LAYERS: usize = 8;
/// Allowed line spacing, as a multiple of the master's line height.
pub const LINE_SPACE_RANGE: RangeInclusive<f64> = 0.5..=4.0;
/// Allowed tracking, in font units between neighbouring glyphs.
pub const TRACKING_RANGE: RangeInclusive<f64> = -1_000.0..=1_000.0;
/// The sizes of a waterfall below the window's size, in points; the window's size ends it.
pub const WATERFALL_SIZES: [f64; 10] = [9.0, 10.0, 12.0, 14.0, 18.0, 24.0, 36.0, 48.0, 72.0, 96.0];
/// Glyph names listed by `ui.inspect`.
const MAX_LISTED: usize = 1_000;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.spaceCenter",
        label: "Space Center",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true), text?: string, before?: string, after?: string, size?: points (4 to 1000), lineMode?: \"multi\"|\"single\"|\"waterfall\", select?: column index|null} show or hide the Space Center and set its text, its control glyphs (placed before and after every glyph of the text), its size, the display (several lines wrapped at the window's width, one line, or a waterfall: one line at a series of sizes up to the window's size) and the selected glyph. Texts take characters, /name, /name@layer, /? (the glyph the glyph panels show), /! (the Font View selection), /:group (the members of a kerning group) and \\n (a new line); at most 10000 characters each. ui.inspect reports the window under spaceCenter",
    },
    UiCommand {
        id: "view.setSpacingCutOff",
        label: "Cut-Off Band",
        menu: &["View"],
        shortcut: None,
        params: "{on?: bool (default: true when a height is given, else the opposite of now), bottom?: units|null, top?: units|null} draw the Space Center's preview inverted between two heights; a height left out keeps its value, and null means the baseline (bottom) or the x-height of the master shown (top)",
    },
    UiCommand {
        id: "view.setPreferredSuffix",
        label: "Prefer Suffix",
        menu: &["View"],
        shortcut: None,
        params: "{suffix?: string (absent or empty: none)} show, for every glyph of the Space Center's text, its variant named with this suffix (`sc` or `.sc` both mean `.sc`) when the font has one",
    },
    UiCommand {
        id: "view.setSpaceCenterDisplay",
        label: "Space Center Display Options",
        menu: &[],
        shortcut: None,
        params: "{rightToLeft?: bool, centered?: bool, upsideDown?: bool, metrics?: bool, beam?: bool, beamY?: units|null, fill?: bool, stroke?: bool, inverse?: bool, guides?: bool, zones?: bool, layers?: [layer or master names] (at most 8), lineSpace?: 0.5 to 4, tracking?: units (-1000 to 1000)} set how the Space Center draws its text; options left out keep their value. rightToLeft lays the text out right to left from the right edge with the right-to-left kerning, centered centres every line, upsideDown turns the whole preview half a circle; metrics draws the glyphs' edges, beam a line at height beamY (null: half the x-height) with the margins measured where it meets each outline, fill and stroke the outlines filled and outlined, inverse light glyphs on a dark ground, guides the master's and the glyphs' guides, zones the master's alignment zones, layers the outlines of those layers (by layer name or id, else the master of that name) over each glyph; lineSpace multiplies the line height and tracking adds space between glyphs. Giving beamY without beam also shows the beam",
    },
    UiCommand {
        id: "spaceCenter.setValue",
        label: "Set Space Center Value",
        menu: &[],
        shortcut: None,
        params: "{column: index, row: \"width\"|\"left\"|\"right\", text: string (at most 64 characters)} enter a value in the Space Center's matrix for the glyph of that column: a number, or = and an expression whose names are glyphs standing for their value in the same row and master (=n, =e+10, =(n+o)/2). The result rounds halves up and runs spacing.setWidth, spacing.setLeftSidebearing or spacing.setRightSidebearing on the column's layer with that number",
    },
];

/// Multi-line text that wraps at the window's width, one line, or one line at a series of sizes.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineMode {
    #[default]
    Multi,
    Single,
    Waterfall,
}

impl LineMode {
    pub const ALL: [LineMode; 3] = [LineMode::Multi, LineMode::Single, LineMode::Waterfall];

    pub fn id(self) -> &'static str {
        match self {
            LineMode::Multi => "multi",
            LineMode::Single => "single",
            LineMode::Waterfall => "waterfall",
        }
    }

    pub fn of_id(id: &str) -> Option<LineMode> {
        LineMode::ALL.into_iter().find(|m| m.id() == id)
    }
}

/// The sizes of a waterfall ending at `size`, in points, smallest first.
pub fn waterfall_sizes(size: f64) -> Vec<f64> {
    WATERFALL_SIZES.iter().copied().filter(|s| *s < size).chain(std::iter::once(size)).collect()
}

/// How the preview is drawn: `view.setSpaceCenterDisplay`.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct Display {
    pub right_to_left: bool,
    pub centered: bool,
    pub upside_down: bool,
    pub metrics: bool,
    pub beam: bool,
    /// The beam's height; `None`: half the x-height of the master shown.
    pub beam_y: Option<f64>,
    pub fill: bool,
    pub stroke: bool,
    pub inverse: bool,
    pub guides: bool,
    pub zones: bool,
    /// Layers drawn as outlines over each glyph, as typed (a layer name or id, or a master name).
    pub layers: Vec<String>,
    pub line_space: f64,
    pub tracking: f64,
}

impl Default for Display {
    fn default() -> Self {
        Display {
            right_to_left: false,
            centered: false,
            upside_down: false,
            metrics: true,
            beam: false,
            beam_y: None,
            fill: true,
            stroke: false,
            inverse: false,
            guides: false,
            zones: false,
            layers: Vec::new(),
            line_space: 1.0,
            tracking: 0.0,
        }
    }
}

/// The parameters of `view.setSpaceCenterDisplay` that switch something on or off.
pub const DISPLAY_FLAGS: [&str; 10] = ["rightToLeft", "centered", "upsideDown", "metrics", "beam", "fill", "stroke", "inverse", "guides", "zones"];

impl Display {
    /// The switch named by one of [`DISPLAY_FLAGS`].
    pub fn flag(&self, key: &str) -> Option<bool> {
        Some(match key {
            "rightToLeft" => self.right_to_left,
            "centered" => self.centered,
            "upsideDown" => self.upside_down,
            "metrics" => self.metrics,
            "beam" => self.beam,
            "fill" => self.fill,
            "stroke" => self.stroke,
            "inverse" => self.inverse,
            "guides" => self.guides,
            "zones" => self.zones,
            _ => return None,
        })
    }

    /// The switch named by one of [`DISPLAY_FLAGS`], to change.
    pub fn flag_mut(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key {
            "rightToLeft" => &mut self.right_to_left,
            "centered" => &mut self.centered,
            "upsideDown" => &mut self.upside_down,
            "metrics" => &mut self.metrics,
            "beam" => &mut self.beam,
            "fill" => &mut self.fill,
            "stroke" => &mut self.stroke,
            "inverse" => &mut self.inverse,
            "guides" => &mut self.guides,
            "zones" => &mut self.zones,
            _ => return None,
        })
    }

    pub fn json(&self) -> Value {
        json!({
            "rightToLeft": self.right_to_left,
            "centered": self.centered,
            "upsideDown": self.upside_down,
            "metrics": self.metrics,
            "beam": self.beam,
            "beamY": self.beam_y,
            "fill": self.fill,
            "stroke": self.stroke,
            "inverse": self.inverse,
            "guides": self.guides,
            "zones": self.zones,
            "layers": self.layers,
            "lineSpace": self.line_space,
            "tracking": self.tracking,
        })
    }
}

/// The cut-off band: on or off, and its heights (`None`: the baseline and the x-height).
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct CutOff {
    pub on: bool,
    pub bottom: Option<f64>,
    pub top: Option<f64>,
}

/// The Space Center's view state, in `ViewState::space_center`.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct SpaceCenterState {
    pub text: String,
    pub before: String,
    pub after: String,
    pub size: f64,
    pub line_mode: LineMode,
    pub cut_off: CutOff,
    /// With its leading dot.
    pub suffix: Option<String>,
    pub display: Display,
    /// The selected glyph, as a matrix column.
    pub selected: Option<usize>,
}

impl Default for SpaceCenterState {
    fn default() -> Self {
        SpaceCenterState {
            text: DEFAULT_TEXT.to_string(),
            before: String::new(),
            after: String::new(),
            size: DEFAULT_SIZE,
            line_mode: LineMode::Multi,
            cut_off: CutOff::default(),
            suffix: None,
            display: Display::default(),
            selected: None,
        }
    }
}

impl SpaceCenterState {
    pub fn input(&self) -> Input<'_> {
        Input { text: &self.text, before: &self.before, after: &self.after, suffix: self.suffix.as_deref() }
    }
}

/// What `/?` and `/!` stand for: the glyph the glyph panels show and the Font View selection.
pub fn context_names(font: &Font, view: &crate::app::ViewState) -> (Option<String>, Vec<String>) {
    let current = match &view.glyph {
        Some(name) if font.glyph(name).is_some() => Some(name.clone()),
        _ => font.glyphs.first().map(|g| g.name.clone()),
    };
    (current, view.font_view.selected.clone())
}

/// `state`'s text read for the master shown.
pub fn resolved(font: &Font, view: &crate::app::ViewState, state: &SpaceCenterState) -> Option<(String, Resolved)> {
    let master = super::masters::current_master_id(font, &view.layers)?;
    let (current, selection) = context_names(font, view);
    let cx = Context { current: current.as_deref(), selection: &selection };
    let r = text::resolve(font, &master, &state.input(), &cx);
    Some((master, r))
}

/// The band's bottom and top in `master`: the heights set, else the baseline and the master's x-height.
pub fn band(font: &Font, master: &str, cut: &CutOff) -> (f64, f64) {
    let bottom = cut.bottom.unwrap_or(0.0);
    let top = cut.top.unwrap_or_else(|| text::x_height(font, master));
    (bottom.min(top), bottom.max(top))
}

pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout.panel_state(&PanelId::SpaceCenter) != craft::behavior::dock::PanelState::Hidden
}

fn text_arg(p: &Value, key: &str) -> Result<Option<String>, String> {
    match params::string(p, key)? {
        Some(t) if t.chars().nth(MAX_TEXT_CHARS).is_some() => Err(format!("`{key}` holds at most {MAX_TEXT_CHARS} characters")),
        t => Ok(t.map(str::to_string)),
    }
}

fn number(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).map(Some).ok_or_else(|| format!("`{key}` must be a number")),
    }
}

fn height(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match number(p, key)? {
        Some(v) if v.abs() > MAX_COORDINATE => Err(format!("`{key}` lies between -{MAX_COORDINATE} and {MAX_COORDINATE}")),
        v => Ok(v),
    }
}

fn is_null(p: &Value, key: &str) -> bool {
    p.get(key).is_some_and(Value::is_null)
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = if command.params.is_null() { Value::Object(Map::new()) } else { command.params.clone() };
    match command.id.as_str() {
        "window.spaceCenter" => window(app, &p),
        "view.setSpacingCutOff" => cut_off(app, &p),
        "view.setPreferredSuffix" => {
            let suffix = match params::string(&p, "suffix")? {
                None => None,
                Some(t) => text::normalised_suffix(t)?,
            };
            app.view.space_center.suffix = suffix;
            Ok(())
        }
        "view.setSpaceCenterDisplay" => set_display(app, &p),
        "spaceCenter.setValue" => set_value(app, &p),
        other => Err(format!("no Space Center command `{other}`")),
    }
}

fn window(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    // Every parameter is checked before anything changes.
    let open = params::flag(p, "open")?.unwrap_or(true);
    let mut next = app.view.space_center.clone();
    if let Some(t) = text_arg(p, "text")? {
        next.text = t;
    }
    if let Some(t) = text_arg(p, "before")? {
        next.before = t;
    }
    if let Some(t) = text_arg(p, "after")? {
        next.after = t;
    }
    if let Some(s) = number(p, "size")? {
        if !SIZE_RANGE.contains(&s) {
            return Err(format!("`size` lies between {} and {} points", SIZE_RANGE.start(), SIZE_RANGE.end()));
        }
        next.size = s;
    }
    if let Some(name) = params::string(p, "lineMode")? {
        next.line_mode = LineMode::of_id(name).ok_or("`lineMode` must be multi, single or waterfall")?;
    }
    let columns = app.session.font().ok().and_then(|font| resolved(font, &app.view, &next)).map_or(0, |(_, r)| r.columns.len());
    match p.get("select") {
        None => {}
        Some(Value::Null) => next.selected = None,
        Some(v) => {
            let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`select` must be a column index or null")?;
            if i >= columns {
                return Err(format!("the text has {columns} glyphs, no glyph {i}"));
            }
            next.selected = Some(i);
        }
    }
    if next.selected.is_some_and(|s| s >= columns) {
        next.selected = None;
    }
    // A refused move (the layout has no room) leaves the panel where it was.
    let _ = app.view.workspace.set_visible(PanelId::SpaceCenter, open, None, false, |panel| panel.permissions());
    app.view.space_center = next;
    Ok(())
}

fn cut_off(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let current = app.view.space_center.cut_off;
    let (new_bottom, new_top) = (height(p, "bottom")?, height(p, "top")?);
    // Heights alone turn the band on; nothing at all toggles it, as the menu item does.
    let on = match params::flag(p, "on")? {
        Some(on) => on,
        None if new_bottom.is_some() || new_top.is_some() => true,
        None => !current.on,
    };
    let bottom = if is_null(p, "bottom") { None } else { new_bottom.or(current.bottom) };
    let top = if is_null(p, "top") { None } else { new_top.or(current.top) };
    if let (Some(b), Some(t)) = (bottom, top)
        && b >= t
    {
        return Err("`bottom` must lie below `top`".into());
    }
    app.view.space_center.cut_off = CutOff { on, bottom, top };
    Ok(())
}

fn set_display(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    // Every parameter is checked before anything changes.
    let mut next = app.view.space_center.display.clone();
    for key in DISPLAY_FLAGS {
        if let (Some(on), Some(slot)) = (params::flag(p, key)?, next.flag_mut(key)) {
            *slot = on;
        }
    }
    if let Some(names) = params::strings(p, "layers", MAX_OVERLAY_LAYERS)? {
        let mut layers: Vec<String> = Vec::new();
        for name in names {
            let name = name.trim().to_string();
            if name.is_empty() || name.len() > MAX_NAME_LEN {
                return Err(format!("a layer name in `layers` has 1 to {MAX_NAME_LEN} bytes"));
            }
            if !layers.contains(&name) {
                layers.push(name);
            }
        }
        next.layers = layers;
    }
    if let Some(v) = number(p, "lineSpace")? {
        if !LINE_SPACE_RANGE.contains(&v) {
            return Err(format!("`lineSpace` lies between {} and {}", LINE_SPACE_RANGE.start(), LINE_SPACE_RANGE.end()));
        }
        next.line_space = v;
    }
    if let Some(v) = number(p, "tracking")? {
        if !TRACKING_RANGE.contains(&v) {
            return Err(format!("`tracking` lies between {} and {} units", TRACKING_RANGE.start(), TRACKING_RANGE.end()));
        }
        next.tracking = v;
    }
    if is_null(p, "beamY") {
        next.beam_y = None;
    } else if let Some(y) = height(p, "beamY")? {
        next.beam_y = Some(y);
        if params::flag(p, "beam")?.is_none() {
            next.beam = true;
        }
    }
    app.view.space_center.display = next;
    Ok(())
}

/// A matrix entry: the value is evaluated against the font as it is now, and the setter runs on the next drain.
fn set_value(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let column = p.get("column").and_then(Value::as_u64).and_then(|c| usize::try_from(c).ok()).ok_or("`column` (a column index) is required")?;
    let row = params::string(p, "row")?.ok_or("`row` (\"width\", \"left\" or \"right\") is required")?;
    let row = Row::of_id(row).ok_or_else(|| format!("`row` must be width, left or right, got \"{}\"", quoted(row)))?;
    let typed = params::string(p, "text")?.ok_or("`text` is required")?;
    if typed.chars().nth(MAX_CELL_CHARS).is_some() {
        return Err(format!("`text` holds at most {MAX_CELL_CHARS} characters"));
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let (master, r) = resolved(font, &app.view, &app.view.space_center).ok_or("the font has no master")?;
    let col = r.columns.get(column).ok_or_else(|| format!("the text has {} glyphs, no glyph {column}", r.columns.len()))?;
    let (id, params) = text::matrix_command(font, &master, col, row, typed)?;
    app.sent.push(Msg::run(id, params));
    Ok(())
}

/// `ui.inspect`'s `spaceCenter` key.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let sc = &app.view.space_center;
    let font = app.session.font().ok();
    let (master, r) = match font.and_then(|f| resolved(f, &app.view, sc)) {
        Some((m, r)) => (Some(m), r),
        None => (None, Resolved::default()),
    };
    let glyphs: Vec<&str> = r.columns.iter().take(MAX_LISTED).map(|c| c.glyph.as_str()).collect();
    let cut = sc.cut_off;
    let shown_band = font.zip(master.as_deref()).map(|(f, m)| band(f, m, &cut));
    let cut_json = match shown_band {
        Some((bottom, top)) => json!({"on": cut.on, "bottom": bottom, "top": top}),
        None => json!({"on": cut.on, "bottom": cut.bottom, "top": cut.top}),
    };
    state.insert(
        "spaceCenter".into(),
        json!({
            "open": shows(app),
            "text": sc.text,
            "before": sc.before,
            "after": sc.after,
            "size": sc.size,
            "lineMode": sc.line_mode.id(),
            "master": master,
            "glyphs": glyphs,
            "count": r.columns.len(),
            "selected": sc.selected,
            "cutOff": cut_json,
            "suffix": sc.suffix,
            "replaced": r.replaced,
            "display": sc.display.json(),
            "message": r.message,
        }),
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests;
