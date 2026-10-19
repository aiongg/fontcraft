//! The Space Center window: the fields, the menus, the preview and the matrix. State, tokens and
//! the commands are in `crate::shell::space_center`.
//!
//! | Input | Does |
//! |---|---|
//! | text, before and after fields | `window.spaceCenter {text}`, `{before}`, `{after}` on every change |
//! | Samples menu | `window.spaceCenter {sample}` with the string chosen |
//! | size, line mode | `window.spaceCenter {size}`, `{lineMode}` |
//! | Cut-Off, suffix field | `view.setSpacingCutOff {on}`, `view.setPreferredSuffix {suffix}` |
//! | Display menu | `view.setSpaceCenterDisplay` with the option changed |
//! | click a glyph of the text | selects it (`window.spaceCenter {select}`) and gives the preview the keyboard |
//! | drag a glyph's left or right edge, or its inside | the margin, or the outline inside the width, by the distance dragged; with Alt in every master |
//! | Left, Right; Alt with them; Ctrl+Cmd with them | the selected glyph's right margin, left margin, outline (steps 1, Shift 10, Shift+Cmd 100) |
//! | click a matrix cell | edits it: Return or leaving confirms, Escape cancels, Up and Down step the value, Tab and Alt-arrows move to the next cell |
//!
//! The text is laid out with `textlayout` and kerned. Each line, or each size of a waterfall's
//! one line, is a row with its own placement on the screen; upside down, every row is turned
//! half a circle about the middle of the content. The outlines of the area in view are built in
//! screen points once per change: filled into one texture (which the cut-off band shows again in
//! the inverse colours) and flattened for the stroke and the overlay layers.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{
    Align2, Color32, ColorImage, Event, FontId, Key, Popup, PopupCloseBehavior, Pos2, Rect, Sense, Shape, Stroke, TextureHandle, TextureOptions,
    Vec2, pos2, vec2,
};
use fontcraft_engine::Session;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::{Font, Guide, Layer, MAX_COORDINATE};
use fontcraft_render::{Style, render_path};
use fontcraft_spacing::kerning::KernIndex;
use fontcraft_textlayout::{Direction, KerningMode, LENGTH_RANGE, Layout, Options, Text};
use serde_json::json;

use crate::app::FontcraftApp;
use crate::canvas::composite::CompositeCache;
use crate::dialogs::sample_strings;
use crate::panels::{Pending, attributes, widgets};
use crate::shell::space_center::{
    self as sc, Column, Context, Drag, Editing, LINE_SPACE_RANGE, LineMode, MAX_CELL_CHARS, MAX_GLYPHS_SHOWN, Resolved, Row, SpaceCenterState,
    TRACKING_RANGE,
};
use crate::theme::Tokens;

/// Matrix columns drawn at most; the rest are reached through `window.spaceCenter {select}`.
const MAX_MATRIX_COLUMNS: usize = 1_000;
/// Guide lines drawn in one frame at most.
const MAX_GUIDE_LINES: usize = 4_000;
/// Alignment zones drawn on each row at most.
const MAX_ZONES: usize = 64;
/// Glyphs whose margins are labelled on the beam in one frame at most.
const MAX_BEAM_LABELS: usize = 2_000;
/// A row's margins are labelled on the beam when its em is at least this many label sizes.
const BEAM_LABEL_EMS: f32 = 4.0;
/// Characters of a sample string shown in the Samples menu.
const MAX_SAMPLE_LABEL: usize = 48;
/// Masters offered as overlay layers in the Display menu.
const MAX_OVERLAY_CHOICES: usize = 64;

/// What the window keeps between frames: the resolved text, its layout and matrix values, the
/// drawn texture and outlines, the beam's margins, a drag in progress, and what the last frame
/// drew (tests read it).
#[derive(Default)]
pub struct Cache {
    key: Option<u64>,
    resolved: Option<Arc<Resolved>>,
    layout: Option<Arc<Layout>>,
    /// Width, left and right of each column.
    values: Arc<Vec<[Option<f64>; 3]>>,
    error: Option<String>,
    /// The key of the texture, the strokes and the overlays below.
    texture_key: Option<u64>,
    texture: Option<TextureHandle>,
    /// The outlines in view as screen polylines, while the stroke is on.
    strokes: Arc<Vec<Vec<Pos2>>>,
    /// The overlay layers' outlines in view as screen polylines.
    overlays: Arc<Vec<Vec<Pos2>>>,
    /// The margins on the beam of each layout item, and the key they were measured for.
    beam_key: Option<u64>,
    beam_values: Arc<Vec<Option<(f64, f64)>>>,
    outlines: CompositeCache,
    drag: Option<Dragging>,
    /// Why the last matrix entry was refused.
    pub matrix_error: Option<String>,
    pub drawn: Drawn,
}

// The cache is rebuilt from the state, so copies start empty and all caches compare equal.
impl Clone for Cache {
    fn clone(&self) -> Self {
        Cache::default()
    }
}

impl PartialEq for Cache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache").field("drawn", &self.drawn).finish_non_exhaustive()
    }
}

/// A drag in the preview: what it changes, the matrix column, the row it began in and where.
#[derive(Clone, Copy, Debug)]
struct Dragging {
    drag: Drag,
    column: usize,
    row: usize,
    start: Pos2,
}

/// What the last frame drew.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawn {
    /// Glyphs drawn and their origins on the screen, row by row in text order.
    pub glyphs: Vec<(Arc<str>, Pos2)>,
    /// The cut-off band of each row drawn, while it is on.
    pub bands: Vec<Rect>,
    /// The screen y of the first row's baseline, and points per font unit at the window's size.
    pub baseline: Option<f32>,
    pub scale: f32,
    /// Each row's points per font unit and the screen y of its baseline, top to bottom.
    pub rows: Vec<(f32, f32)>,
    /// The matrix columns' glyphs and their width, left and right values.
    pub columns: Vec<String>,
    pub values: Vec<[Option<f64>; 3]>,
    /// The selected glyph's box (in the first row that shows it).
    pub selected: Option<Rect>,
    pub lines: usize,
    pub master: Option<String>,
    pub message: Option<String>,
    /// The screen y of the beam on the first row, and the margins measured on it there:
    /// glyph, left, right.
    pub beam: Option<f32>,
    pub beam_labels: Vec<(Arc<str>, f64, f64)>,
    /// The outlines were filled; polylines stroked and overlaid; guide lines and zones drawn.
    pub filled: bool,
    pub strokes: usize,
    pub overlays: usize,
    pub guides: usize,
    pub zones: usize,
    /// The colour behind the glyphs.
    pub ground: Option<Color32>,
}

/// The id of the window.
pub fn window_id() -> egui::Id {
    egui::Id::new("fontcraft.spaceCenter")
}

/// The id of the preview area, which takes the keyboard for the arrow keys.
pub fn area_id() -> egui::Id {
    widgets::id(("spaceCenter", "area"))
}

fn cell_id(column: usize, row: Row) -> egui::Id {
    widgets::id(("spaceCenter", "cell", column, row))
}

/// Where a row of the text sits on the screen: a layout point (x, y) is at
/// (`ax` + x·`scale`, `ay` − y·`scale`), turned half a circle about `flip` when upside down.
#[derive(Clone, Copy, Debug)]
struct Placement {
    ax: f32,
    ay: f32,
    scale: f32,
    flip: Option<Pos2>,
}

impl Placement {
    /// Half a circle about `flip`; its own inverse.
    fn turn(self, p: Pos2) -> Pos2 {
        match self.flip {
            Some(c) => pos2(2.0 * c.x - p.x, 2.0 * c.y - p.y),
            None => p,
        }
    }

    fn screen(self, x: f64, y: f64) -> Pos2 {
        self.turn(pos2(self.ax + x as f32 * self.scale, self.ay - y as f32 * self.scale))
    }

    fn font(self, p: Pos2) -> (f64, f64) {
        let p = self.turn(p);
        let s = f64::from(self.scale.max(f32::MIN_POSITIVE));
        (f64::from(p.x - self.ax) / s, f64::from(self.ay - p.y) / s)
    }

    fn affine(self) -> Affine {
        let s = f64::from(self.scale);
        let upright = Affine::new([s, 0.0, 0.0, -s, f64::from(self.ax), f64::from(self.ay)]);
        match self.flip {
            Some(c) => Affine::new([-1.0, 0.0, 0.0, -1.0, 2.0 * f64::from(c.x), 2.0 * f64::from(c.y)]) * upright,
            None => upright,
        }
    }

    fn moved(self, by: Vec2) -> Placement {
        Placement { ax: self.ax + by.x, ay: self.ay + by.y, flip: self.flip.map(|c| c + by), ..self }
    }

    fn hash_into(self, h: &mut impl Hasher) {
        let (fx, fy) = self.flip.map_or((f32::NAN, f32::NAN), |c| (c.x, c.y));
        for v in [self.ax, self.ay, self.scale, fx, fy] {
            v.to_bits().hash(h);
        }
    }
}

/// One row of the preview: a line of the layout and where it is drawn.
#[derive(Clone, Copy, Debug)]
struct Shown {
    line: usize,
    place: Placement,
}

/// A matrix value as a cell shows it: whole numbers without decimals, others to two places.
pub fn format_value(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r}") }
}

/// The matrix values of one glyph of the line.
pub type MatrixRow = [Option<f64>; 3];

/// Lays the window's text out for `master` at `line_width` font units, with the matrix values:
/// in the direction, line spacing and tracking of the display options.
pub fn lay_out(
    font: &Font,
    master: &str,
    state: &SpaceCenterState,
    cx: &Context<'_>,
    line_width: f64,
    index: Option<&KernIndex>,
) -> Result<(Resolved, Layout, Vec<MatrixRow>), String> {
    let resolved = sc::resolve(font, master, state, cx);
    let mut text = Text::with_limit(MAX_GLYPHS_SHOWN).map_err(|e| e.to_string())?;
    text.insert(0, resolved.items.clone()).map_err(|e| e.to_string())?;
    let display = &state.display;
    let spaced = display.line_space();
    let line_height = if spaced == 1.0 {
        None
    } else {
        font.masters
            .iter()
            .find(|m| m.id == master)
            .map(|m| fontcraft_textlayout::line_height(font, m) * spaced)
            .filter(|h| h.is_finite())
            .map(|h| h.clamp(*LENGTH_RANGE.start(), *LENGTH_RANGE.end()))
    };
    let options = Options {
        direction: if display.right_to_left { Direction::Rtl } else { Direction::Ltr },
        kerning: KerningMode::Kerning,
        line_width,
        line_height,
        single_line: state.line_mode != LineMode::Multi,
        index,
        layers: Some(&resolved.layers),
        ..Options::default()
    };
    let mut layout = fontcraft_textlayout::layout(font, master, &text, &options).map_err(|e| e.to_string())?;
    track(&mut layout, display.tracking());
    let values = resolved.columns.iter().map(|c| Row::ALL.map(|row| sc::value(font, &c.glyph, &c.layer, row))).collect();
    Ok((resolved, layout, values))
}

/// Adds `tracking` font units between neighbouring items of each line, after the layout, so the
/// lines break where the layout broke them.
fn track(layout: &mut Layout, tracking: f64) {
    if tracking == 0.0 || !tracking.is_finite() {
        return;
    }
    let sign = if layout.direction == Direction::Rtl { -1.0 } else { 1.0 };
    let Layout { placed, lines, .. } = layout;
    for line in lines.iter_mut() {
        let mut before = 0.0_f64;
        for i in line.items.clone() {
            let Some(p) = placed.get_mut(i) else { continue };
            if p.glyph.is_none() && p.advance <= 0.0 {
                continue;
            }
            let shift = before * tracking;
            p.x += sign * shift;
            p.offset += shift;
            for extra in p.extra.iter_mut() {
                extra.1 += sign * shift;
            }
            before += 1.0;
        }
        line.length = (line.length + (before - 1.0).max(0.0) * tracking).max(0.0);
    }
}

/// The rows of the preview and the size they cover, padding included, for lines `span` points
/// long, with the content's top left corner at (0, 0) and at least `at_least` in size.
fn arrange(layout: &Layout, st: &SpaceCenterState, upm: f64, pad: f32, span: f32, at_least: Vec2) -> (Vec<Shown>, Vec2) {
    let rows: Vec<(usize, f32)> = if st.line_mode == LineMode::Waterfall {
        sc::waterfall_sizes(st.size).into_iter().map(|s| (0, (s / upm) as f32)).collect()
    } else {
        let scale = (st.size / upm) as f32;
        (0..layout.lines.len()).map(|l| (l, scale)).collect()
    };
    let length = |line: usize| layout.lines.get(line).map_or(0.0, |l| l.length) as f32;
    let wide = rows.iter().map(|&(l, s)| length(l) * s).fold(span, f32::max);
    let rtl = layout.direction == Direction::Rtl;
    let mut y = pad;
    let mut out = Vec::with_capacity(rows.len());
    for (line, scale) in rows {
        let baseline = layout.lines.get(line).map_or(0.0, |l| l.baseline) as f32;
        // The row's top, its ascender above the baseline, is at `y`.
        let ay = y + (layout.ascender as f32 + baseline) * scale;
        let l = length(line) * scale;
        // A right-to-left line runs leftwards from its origin.
        let shift = match (rtl, st.display.centered) {
            (false, false) => 0.0,
            (false, true) => (wide - l) / 2.0,
            (true, false) => wide,
            (true, true) => (wide + l) / 2.0,
        };
        out.push(Shown { line, place: Placement { ax: pad + shift, ay, scale, flip: None } });
        y += layout.line_height as f32 * scale;
    }
    let content = vec2(wide + 2.0 * pad, y + pad).max(at_least);
    if st.display.upside_down {
        let middle = (content / 2.0).to_pos2();
        for row in &mut out {
            row.place.flip = Some(middle);
        }
    }
    (out, content)
}

/// The advance box of item `i` drawn in a row, ascender to descender.
fn advance_box(layout: &Layout, place: Placement, i: usize) -> Option<Rect> {
    let p = layout.placed.get(i)?;
    let line = layout.lines.get(p.line)?;
    let top = line.baseline + layout.ascender;
    Some(Rect::from_two_pos(place.screen(p.x, top), place.screen(p.x + p.advance, top - layout.line_height)))
}

/// The row and the text item under a screen point: a glyph whose advance box holds it.
fn hit(layout: &Layout, rows: &[Shown], at: Pos2) -> Option<(usize, usize)> {
    rows.iter().enumerate().find_map(|(r, row)| {
        let line = layout.lines.get(row.line)?;
        let (x, y) = row.place.font(at);
        let top = line.baseline + layout.ascender;
        if !(y <= top && y > top - layout.line_height) {
            return None;
        }
        line.items.clone().find(|&i| layout.placed.get(i).is_some_and(|p| p.glyph.is_some() && x >= p.x && x < p.x + p.advance)).map(|i| (r, i))
    })
}

/// The layer a glyph of the text is drawn from: the one named after `@`, else the master's.
fn drawn_layer<'f>(font: &'f Font, master: &str, resolved: &Resolved, name: &str) -> Option<&'f Layer> {
    let glyph = font.glyph(name)?;
    let id = resolved.layers.get(name).map_or(master, String::as_str);
    glyph.layers.iter().find(|l| l.id == id).or_else(|| glyph.layer(master))
}

/// The margins measured on the beam at height `y` for each item of the layout.
fn beam_values(font: &Font, master: &str, layout: &Layout, resolved: &Resolved, y: f64) -> Vec<Option<(f64, f64)>> {
    let mut measured: HashMap<Arc<str>, Option<(f64, f64)>> = HashMap::new();
    layout
        .placed
        .iter()
        .map(|p| {
            let name = p.glyph.as_ref()?;
            *measured.entry(name.clone()).or_insert_with(|| {
                let layer = drawn_layer(font, master, resolved, name)?;
                fontcraft_spacing::sidebearings_at_height(font, layer, y).ok().flatten()
            })
        })
        .collect()
}

/// The part of the line through `at` at `angle` degrees inside the box `x` × `y`.
fn clip_line(at: (f64, f64), angle: f64, x: (f64, f64), y: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    if !(at.0.is_finite() && at.1.is_finite() && angle.is_finite()) {
        return None;
    }
    let (dy, dx) = angle.to_radians().sin_cos();
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (d, p, (lo, hi)) in [(dx, at.0, x), (dy, at.1, y)] {
        if d.abs() < 1e-12 {
            if p < lo || p > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - p) / d, (hi - p) / d);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t0.is_finite() && t1.is_finite() && t0 <= t1).then_some(((at.0 + t0 * dx, at.1 + t0 * dy), (at.0 + t1 * dx, at.1 + t1 * dy)))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.space_center.open {
        return;
    }
    let master = sc::shown_master(app);
    let index = app.session.kern_index().ok();
    let doc = app.session.active().map(|d| (d.id, d.revision));
    let (current, selection) = sc::context(app);
    let beam = match (app.session.font().ok(), master.as_deref(), doc) {
        (Some(font), Some(master), Some(doc)) => Some(sc::beam_height(app, font, doc.0, master)),
        _ => None,
    };
    let FontcraftApp { session, state, tokens, .. } = app;
    let session: &Session = session;
    let font = session.font().ok();
    let st = &mut state.panels.space_center;
    let mut open = true;
    let ctx = ui.ctx().clone();
    let shown = egui::Window::new("Space Center")
        .id(window_id())
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.text_preview_width, tokens.text_preview_height))
        .pivot(Align2::CENTER_TOP)
        .default_pos(pos2(ctx.content_rect().center().x, tokens.dialog_top))
        .show(&ctx, |ui| {
            fields(ui, st, tokens, session, font, master.as_deref(), beam, pending);
            ui.add_space(tokens.dialog_spacing);
            match (font, master.as_deref(), doc) {
                (Some(font), Some(master), Some(doc)) => {
                    let cx = Context { current: current.as_deref(), selection: &selection };
                    preview(ui, st, tokens, font, master, doc, &cx, index.as_deref(), beam, pending);
                    matrix(ui, st, tokens, font, master, pending);
                }
                _ => {
                    ui.label(egui::RichText::new("No font is open").color(tokens.text_dim));
                    st.cache.drawn = Drawn { message: Some("No font is open".into()), ..Drawn::default() };
                }
            }
        });
    // A press inside the window gives it the keyboard; a press anywhere else takes it away.
    if let (Some(shown), Some(at)) = (shown, ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten())) {
        st.keyboard = shown.response.rect.contains(at);
    }
    if !open {
        pending.push(("window.spaceCenter", json!({ "open": false })));
    }
}

#[allow(clippy::too_many_arguments)]
fn fields(
    ui: &mut egui::Ui,
    st: &SpaceCenterState,
    tokens: &Tokens,
    session: &Session,
    font: Option<&Font>,
    master: Option<&str>,
    beam: Option<f64>,
    pending: &mut Pending,
) {
    ui.horizontal_wrapped(|ui| {
        let mut text = st.text.clone();
        let edit = egui::TextEdit::singleline(&mut text)
            .id(widgets::id(("spaceCenter", "text")))
            .char_limit(sc::MAX_TEXT_CHARS)
            .hint_text("Text")
            .desired_width(tokens.dialog_field_width * 4.0);
        if ui.add(edit).changed() {
            pending.push(("window.spaceCenter", json!({ "text": text })));
        }
        let samples = attributes::menu_button(ui, widgets::id(("spaceCenter", "samples")), "Samples", "Put a stored sample string into the text");
        Popup::menu(&samples).id(widgets::id(("spaceCenter", "samples", "menu"))).show(|ui| samples_menu(ui, session, pending));
        for (key, value, hint) in [("before", &st.before, "Before"), ("after", &st.after, "After")] {
            let mut v = value.clone();
            let edit = egui::TextEdit::singleline(&mut v)
                .id(widgets::id(("spaceCenter", key)))
                .char_limit(sc::MAX_TEXT_CHARS)
                .hint_text(hint)
                .desired_width(tokens.field_width);
            if ui.add(edit).changed() {
                pending.push(("window.spaceCenter", json!({ key: v })));
            }
        }
        let mut size = st.size;
        if ui.add(egui::DragValue::new(&mut size).range(sc::SIZE_RANGE).suffix(" pt")).changed() {
            pending.push(("window.spaceCenter", json!({ "size": size })));
        }
        for mode in LineMode::ALL {
            if ui.selectable_label(st.line_mode == mode, mode.label()).clicked() && st.line_mode != mode {
                pending.push(("window.spaceCenter", json!({ "lineMode": mode.name() })));
            }
        }
        let mut cut = st.cut_off.on;
        if ui.checkbox(&mut cut, "Cut-Off").changed() {
            pending.push(("view.setSpacingCutOff", json!({ "on": cut })));
        }
        let mut suffix = st.suffix.as_deref().unwrap_or("").trim_start_matches('.').to_string();
        let edit = egui::TextEdit::singleline(&mut suffix)
            .id(widgets::id(("spaceCenter", "suffix")))
            .char_limit(MAX_CELL_CHARS)
            .hint_text("Suffix")
            .desired_width(tokens.field_width);
        if ui.add(edit).changed() {
            pending.push(("view.setPreferredSuffix", json!({ "suffix": suffix })));
        }
        let display = attributes::menu_button(ui, widgets::id(("spaceCenter", "display")), "Display", "What the preview shows");
        Popup::menu(&display)
            .id(widgets::id(("spaceCenter", "display", "menu")))
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| display_menu(ui, st, font, master, beam, pending));
    });
}

/// The stored sample strings by group; a click puts one into the text.
fn samples_menu(ui: &mut egui::Ui, session: &Session, pending: &mut Pending) {
    let groups = sample_strings::groups(session);
    // Headings (no index) and strings (their index over all groups), one per menu row.
    let mut entries: Vec<(Option<usize>, &str)> = Vec::new();
    let mut index = 0usize;
    for group in &groups {
        entries.push((None, group.name.as_str()));
        for s in &group.strings {
            entries.push((Some(index), s.as_str()));
            index += 1;
        }
    }
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.6).show_rows(ui, row_height, entries.len(), |ui, range| {
        for (index, text) in entries.get(range).unwrap_or_default() {
            let shown: String = text.chars().take(MAX_SAMPLE_LABEL).collect();
            match index {
                None => {
                    ui.label(egui::RichText::new(shown).strong());
                }
                Some(i) => {
                    if ui.button(shown).clicked() {
                        pending.push(("window.spaceCenter", json!({ "sample": i })));
                    }
                }
            }
        }
    });
}

/// The display options; each change runs `view.setSpaceCenterDisplay` with that option.
fn display_menu(ui: &mut egui::Ui, st: &SpaceCenterState, font: Option<&Font>, master: Option<&str>, beam: Option<f64>, pending: &mut Pending) {
    let d = &st.display;
    let flags = [
        ("rightToLeft", d.right_to_left, "Right to Left"),
        ("centered", d.centered, "Centred"),
        ("upsideDown", d.upside_down, "Upside Down"),
        ("metrics", d.metrics, "Metrics"),
        ("beam", d.beam, "Beam"),
        ("fill", d.fill, "Fill"),
        ("stroke", d.stroke, "Stroke"),
        ("inverse", d.inverse, "Inverse"),
        ("guides", d.guides, "Guides"),
        ("zones", d.zones, "Zones"),
    ];
    for (key, on, label) in flags {
        let mut v = on;
        if ui.checkbox(&mut v, label).changed() {
            pending.push(("view.setSpaceCenterDisplay", json!({ key: v })));
        }
    }
    if d.beam
        && let Some(y) = beam
    {
        let mut v = y;
        if ui.add(egui::DragValue::new(&mut v).range(-MAX_COORDINATE..=MAX_COORDINATE).prefix("Beam at ")).changed() {
            pending.push(("view.setSpaceCenterDisplay", json!({ "beamY": v })));
        }
    }
    let mut space = d.line_space();
    if ui.add(egui::DragValue::new(&mut space).range(LINE_SPACE_RANGE).speed(0.01).prefix("Line space ")).changed() {
        pending.push(("view.setSpaceCenterDisplay", json!({ "lineSpace": space })));
    }
    let mut tracking = d.tracking();
    if ui.add(egui::DragValue::new(&mut tracking).range(TRACKING_RANGE).prefix("Tracking ")).changed() {
        pending.push(("view.setSpaceCenterDisplay", json!({ "tracking": tracking })));
    }
    let (Some(font), Some(master)) = (font, master) else { return };
    if font.masters.len() < 2 {
        return;
    }
    ui.separator();
    ui.label("Overlay");
    for m in font.masters.iter().filter(|m| m.id != master).take(MAX_OVERLAY_CHOICES) {
        let mut on = d.layers.contains(&m.name);
        if ui.checkbox(&mut on, &m.name).changed() {
            let mut layers: Vec<String> = d.layers.iter().filter(|l| **l != m.name).cloned().collect();
            if on {
                layers.push(m.name.clone());
            }
            layers.truncate(sc::MAX_OVERLAY_LAYERS);
            pending.push(("view.setSpaceCenterDisplay", json!({ "layers": layers })));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn preview(
    ui: &mut egui::Ui,
    st: &mut SpaceCenterState,
    tokens: &Tokens,
    font: &Font,
    master: &str,
    doc: (u64, u64),
    cx: &Context<'_>,
    index: Option<&KernIndex>,
    beam: Option<f64>,
    pending: &mut Pending,
) {
    let pad = tokens.text_preview_padding;
    let avail = ui.available_size();
    let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    let matrix_h = row_h * 5.0 + ui.spacing().scroll.bar_width + tokens.dialog_spacing;
    let size = vec2(avail.x.max(pad * 8.0), (avail.y - matrix_h).max(pad * 8.0));
    let upm = f64::from(font.units_per_em.max(1));
    let scale = (st.size / upm) as f32;
    // The line length, in whole steps of 8 points so that resizing re-lays out only now and then.
    let span = (((size.x - 2.0 * pad) / 8.0).floor() * 8.0).max(8.0);
    let line_width = (f64::from(span) / f64::from(scale)).clamp(*LENGTH_RANGE.start(), *LENGTH_RANGE.end());
    let display = st.display.clone();

    let mut h = std::collections::hash_map::DefaultHasher::new();
    (doc, master, &st.text, &st.before, &st.after, &st.suffix, st.line_mode, line_width.to_bits(), cx.current, cx.selection).hash(&mut h);
    (display.right_to_left, display.line_space().to_bits(), display.tracking().to_bits()).hash(&mut h);
    let key = h.finish();
    if st.cache.key != Some(key) {
        st.cache.key = Some(key);
        match lay_out(font, master, st, cx, line_width, index) {
            Ok((resolved, layout, values)) => {
                st.cache.error = resolved.message.clone();
                st.cache.resolved = Some(Arc::new(resolved));
                st.cache.layout = Some(Arc::new(layout));
                st.cache.values = Arc::new(values);
            }
            Err(e) => {
                st.cache.resolved = None;
                st.cache.layout = None;
                st.cache.values = Arc::default();
                st.cache.error = Some(e);
            }
        }
    }
    let mut drawn = Drawn { scale, master: Some(master.to_string()), message: st.cache.error.clone(), ..Drawn::default() };
    let (Some(layout), Some(resolved)) = (st.cache.layout.clone(), st.cache.resolved.clone()) else {
        if let Some(e) = &st.cache.error {
            ui.label(egui::RichText::new(e).color(tokens.error_text));
        }
        st.cache.drawn = drawn;
        return;
    };
    drawn.lines = layout.lines.len();
    drawn.columns = resolved.columns.iter().map(|c| c.glyph.clone()).collect();
    drawn.values = st.cache.values.to_vec();
    let (arranged, content) = arrange(&layout, st, upm, pad, span, size);
    let beam = beam.filter(|y| display.beam && y.is_finite());
    if let Some(y) = beam {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (key, y.to_bits()).hash(&mut h);
        let beam_key = h.finish();
        if st.cache.beam_key != Some(beam_key) {
            st.cache.beam_key = Some(beam_key);
            st.cache.beam_values = Arc::new(beam_values(font, master, &layout, &resolved, y));
        }
    }
    let ppp = ui.ctx().pixels_per_point();
    let (bottom, top) = sc::band(font, master, &st.cut_off);
    let selected = st.selected.and_then(|i| resolved.columns.get(i)).map(|c| c.item);
    let (ground, ink) = if display.inverse { (tokens.glyph_fill, tokens.canvas) } else { (tokens.canvas, tokens.glyph_fill) };
    drawn.ground = Some(ground);
    let shown_master = font.masters.iter().find(|m| m.id == master);
    let zones = match shown_master {
        Some(m) if display.zones => font.alignment_zones(m),
        _ => Vec::new(),
    };
    let master_guides: &[Guide] = match shown_master {
        Some(m) if display.guides => &m.guides,
        _ => &[],
    };
    let overlay = display.overlay_layers().to_vec();
    let is_column = |i: usize| resolved.columns.binary_search_by_key(&i, |c| c.item).is_ok();

    let scroll = egui::ScrollArea::both().id_salt(widgets::id(("spaceCenter", "scroll"))).auto_shrink([false, false]).max_height(size.y);
    scroll.show_viewport(ui, |ui, viewport| {
        ui.set_width(content.x);
        ui.set_height(content.y);
        let origin = ui.max_rect().min;
        let visible = viewport.translate(origin.to_vec2());
        let response = ui.interact(visible, area_id(), Sense::click_and_drag());
        let painter = ui.painter_at(visible);
        painter.rect_filled(visible, 0.0, ground);
        let rows: Vec<Shown> = arranged.iter().map(|r| Shown { line: r.line, place: r.place.moved(origin.to_vec2()) }).collect();
        drawn.rows = rows.iter().filter_map(|r| layout.lines.get(r.line).map(|l| (r.place.scale, r.place.screen(0.0, l.baseline).y))).collect();
        drawn.baseline = drawn.rows.first().map(|r| r.1);

        // What the outlines in view depend on; when it is unchanged the cached ones are drawn.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (key, &overlay, display.fill, display.stroke).hash(&mut hasher);
        for row in &rows {
            row.place.hash_into(&mut hasher);
        }
        for v in [visible.min.x, visible.min.y, visible.max.x, visible.max.y, ppp] {
            v.to_bits().hash(&mut hasher);
        }
        let paths_key = hasher.finish();
        let fresh = st.cache.texture_key != Some(paths_key);

        // The glyphs in view: their boxes, and their outlines in screen points when fresh.
        let mut outline = BezPath::new();
        let mut over = BezPath::new();
        let mut boxes: Vec<(usize, usize, Rect)> = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let Some(line) = layout.lines.get(row.line) else { continue };
            let reach = visible.expand(row.place.scale * upm as f32);
            for i in line.items.clone() {
                let Some(p) = layout.placed.get(i) else { continue };
                let Some(name) = p.glyph.as_ref() else { continue };
                let Some(bx) = advance_box(&layout, row.place, i) else { continue };
                if !reach.intersects(bx) {
                    continue;
                }
                let Some(layer) = drawn_layer(font, master, &resolved, name) else { continue };
                if fresh {
                    let to_screen = row.place.affine() * Affine::translate((p.x, p.y));
                    if let Some(composite) = st.cache.outlines.get(doc, font, name, layer) {
                        let mut g = composite.outline.clone();
                        g.apply_affine(to_screen);
                        outline.extend(g.elements().iter().copied());
                    }
                    for typed in &overlay {
                        let Some(id) = sc::layer_named(font, name, master, typed) else { continue };
                        let Some(other) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == id)) else { continue };
                        if other.id == layer.id {
                            continue;
                        }
                        if let Some(composite) = st.cache.outlines.get(doc, font, name, other) {
                            let mut g = composite.outline.clone();
                            g.apply_affine(to_screen);
                            over.extend(g.elements().iter().copied());
                        }
                    }
                }
                drawn.glyphs.push((name.clone(), row.place.screen(p.x, p.y)));
                boxes.push((r, i, bx));
            }
        }
        if fresh {
            st.cache.texture_key = Some(paths_key);
            st.cache.texture = if display.fill { render(ui, &outline, visible, ppp) } else { None };
            st.cache.strokes = Arc::new(if display.stroke { crate::canvas::flatten(&outline) } else { Vec::new() });
            st.cache.overlays = Arc::new(crate::canvas::flatten(&over));
        }

        // The alignment zones of each row, behind everything else.
        for row in &rows {
            let Some(line) = layout.lines.get(row.line) else { continue };
            for z in zones.iter().take(MAX_ZONES) {
                let (a, b) = (row.place.screen(0.0, line.baseline + z.pos).y, row.place.screen(0.0, line.baseline + z.pos + z.size).y);
                let r = Rect::from_x_y_ranges(visible.x_range(), a.min(b)..=a.max(b));
                if r.is_finite() && r.intersects(visible) {
                    painter.rect_filled(r, 0.0, tokens.zone_fill);
                    drawn.zones += 1;
                }
            }
        }
        // The selected glyph's box, in every row that shows it.
        if let Some(item) = selected {
            for row in &rows {
                if layout.placed.get(item).is_some_and(|p| p.line == row.line)
                    && let Some(r) = advance_box(&layout, row.place, item)
                {
                    painter.rect_filled(r, 0.0, tokens.text_selection);
                    drawn.selected.get_or_insert(r);
                }
            }
        }
        // The glyphs' edges on each baseline, so the margins can be seen and grabbed.
        if display.metrics {
            let edge = Stroke::new(tokens.metric_line_width, tokens.metric_line);
            for &(_, i, r) in &boxes {
                if !is_column(i) || !visible.intersects(r) {
                    continue;
                }
                for x in [r.min.x, r.max.x] {
                    painter.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], edge);
                }
            }
        }
        let texture = st.cache.texture.clone();
        let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        if let Some(texture) = &texture {
            painter.image(texture.id(), visible, whole, ink);
            drawn.filled = true;
        }
        // The cut-off band: glyph colour behind, the outlines in the ground colour on top.
        if st.cut_off.on {
            for row in &rows {
                let Some(line) = layout.lines.get(row.line) else { continue };
                let (y0, y1) = (row.place.screen(0.0, line.baseline + top).y, row.place.screen(0.0, line.baseline + bottom).y);
                let band = Rect::from_x_y_ranges(visible.x_range(), y0.min(y1)..=y0.max(y1));
                if !band.is_finite() || !band.intersects(visible) {
                    continue;
                }
                let band = band.intersect(visible);
                painter.rect_filled(band, 0.0, ink);
                if let Some(texture) = &texture {
                    painter.with_clip_rect(band).image(texture.id(), visible, whole, ground);
                }
                drawn.bands.push(band);
            }
        }
        let outline_colour = if display.inverse { ink } else { tokens.outline };
        for contour in st.cache.strokes.iter() {
            painter.add(Shape::line(contour.clone(), Stroke::new(tokens.outline_width, outline_colour)));
            drawn.strokes += 1;
        }
        for contour in st.cache.overlays.iter() {
            painter.add(Shape::line(contour.clone(), Stroke::new(tokens.visible_layer_outline_width, tokens.visible_layer_outline)));
            drawn.overlays += 1;
        }
        // The master's guides and each glyph's own, within its box.
        if display.guides {
            let mut budget = MAX_GUIDE_LINES;
            let (lo, hi) = (layout.ascender - layout.line_height, layout.ascender);
            'boxes: for &(r, i, bx) in &boxes {
                let (Some(row), Some(p)) = (rows.get(r), layout.placed.get(i)) else { continue };
                if !visible.intersects(bx) {
                    continue;
                }
                let Some(name) = p.glyph.as_deref() else { continue };
                let own: &[Guide] = drawn_layer(font, master, &resolved, name).map_or(&[], |l| l.info.guides.as_slice());
                let all = master_guides.iter().map(|g| (g, tokens.guide_global)).chain(own.iter().map(|g| (g, tokens.guide_local)));
                for (g, colour) in all {
                    if budget == 0 {
                        break 'boxes;
                    }
                    let Some((a, b)) = clip_line((g.x, g.y), g.angle, (0.0, p.advance), (lo, hi)) else { continue };
                    painter.line_segment(
                        [row.place.screen(p.x + a.0, p.y + a.1), row.place.screen(p.x + b.0, p.y + b.1)],
                        Stroke::new(tokens.guide_width, colour),
                    );
                    budget -= 1;
                }
            }
            drawn.guides = MAX_GUIDE_LINES - budget;
        }
        // The beam across each row, with the margins measured on it.
        if let Some(y) = beam {
            let line_stroke = Stroke::new(tokens.measurement_line_width, tokens.measurement_line);
            for (r, row) in rows.iter().enumerate() {
                let Some(line) = layout.lines.get(row.line) else { continue };
                let at = row.place.screen(0.0, line.baseline + y).y;
                if r == 0 {
                    drawn.beam = Some(at);
                }
                if at >= visible.min.y && at <= visible.max.y {
                    painter.line_segment([pos2(visible.min.x, at), pos2(visible.max.x, at)], line_stroke);
                }
            }
            let values = st.cache.beam_values.clone();
            let font_id = FontId::proportional(tokens.measurement_line_label_size);
            let mut budget = MAX_BEAM_LABELS;
            for &(r, i, bx) in &boxes {
                let (Some(row), Some(p), Some(Some((left, right)))) = (rows.get(r), layout.placed.get(i), values.get(i).copied()) else { continue };
                if r == 0
                    && let Some(name) = &p.glyph
                {
                    drawn.beam_labels.push((name.clone(), left, right));
                }
                if budget == 0 || row.place.scale * (upm as f32) < tokens.measurement_line_label_size * BEAM_LABEL_EMS || !visible.intersects(bx) {
                    continue;
                }
                budget -= 1;
                for (x, v) in [(p.x + left / 2.0, left), (p.x + p.advance - right / 2.0, right)] {
                    painter.text(
                        row.place.screen(x, p.y + y),
                        Align2::CENTER_BOTTOM,
                        widgets::format_number(v),
                        font_id.clone(),
                        tokens.measurement_line_label,
                    );
                }
            }
        }
        pointer(ui, &response, st, tokens, font, &layout, &resolved, &rows, pending);
    });
    keys(ui, st, &resolved, pending);
    st.cache.drawn = drawn;
}

/// Clicks select a glyph of the text; drags change a margin or move the outline.
#[allow(clippy::too_many_arguments)]
fn pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    st: &mut SpaceCenterState,
    tokens: &Tokens,
    font: &Font,
    layout: &Layout,
    resolved: &Resolved,
    rows: &[Shown],
    pending: &mut Pending,
) {
    // The matrix column, the item and the row under a screen point.
    let column_at = |at: Pos2| -> Option<(usize, usize, usize)> {
        let (r, item) = hit(layout, rows, at)?;
        resolved.columns.iter().position(|c| c.item == item).map(|c| (c, item, r))
    };
    if response.clicked() || response.drag_started() {
        response.request_focus();
        st.editing = None;
        let hit = response.interact_pointer_pos().and_then(column_at);
        let select = hit.map(|(c, _, _)| c);
        if select != st.selected {
            pending.push(("window.spaceCenter", json!({ "select": select })));
        }
        if response.drag_started()
            && let (Some((column, item, r)), Some(at)) = (hit, response.interact_pointer_pos())
            && let (Some(p), Some(row)) = (layout.placed.get(item), rows.get(r))
        {
            let grip = tokens.selection_ring_radius.max(1.0);
            let (left, right) = (row.place.screen(p.x, 0.0).x, row.place.screen(p.x + p.advance, 0.0).x);
            let drag = if (at.x - left).abs() <= grip {
                Drag::Left
            } else if (at.x - right).abs() <= grip {
                Drag::Right
            } else {
                Drag::Outline
            };
            st.cache.drag = Some(Dragging { drag, column, row: r, start: at });
        }
    }
    // Where a drag in progress has got to.
    if st.cache.drag.is_some()
        && let Some(at) = response.interact_pointer_pos()
    {
        let painter = ui.painter_at(response.rect);
        painter.line_segment(
            [pos2(at.x, response.rect.min.y), pos2(at.x, response.rect.max.y)],
            Stroke::new(tokens.metric_line_width, tokens.metric_highlight),
        );
    }
    if response.drag_stopped()
        && let Some(d) = st.cache.drag.take()
        && let Some(at) = response.interact_pointer_pos().or_else(|| ui.ctx().pointer_latest_pos())
        && let (Some(col), Some(row)) = (resolved.columns.get(d.column), rows.get(d.row))
    {
        // In font units, so a row of any size, turned or not, moves the edge the way it was dragged.
        let dx = row.place.font(at).0 - row.place.font(d.start).0;
        let every = ui.input(|i| i.modifiers.alt);
        let layers = if every { master_layers(font, col) } else { vec![col.layer.clone()] };
        pending.extend(sc::drag_commands(d.drag, &col.glyph, &layers, dx));
    }
}

/// The glyph's master layers, for edits made in every master.
fn master_layers(font: &Font, col: &Column) -> Vec<String> {
    let Some(glyph) = font.glyph(&col.glyph) else { return Vec::new() };
    font.masters.iter().filter_map(|m| glyph.layer(&m.id).map(|l| l.id.clone())).collect()
}

/// The arrow keys while the preview has the keyboard and a glyph is selected.
fn keys(ui: &egui::Ui, st: &SpaceCenterState, resolved: &Resolved, pending: &mut Pending) {
    if st.editing.is_some() || !ui.memory(|m| m.has_focus(area_id())) {
        return;
    }
    ui.memory_mut(|m| m.set_focus_lock_filter(area_id(), egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() }));
    let Some(col) = st.selected.and_then(|i| resolved.columns.get(i)) else { return };
    let events = ui.input(|i| i.events.clone());
    for event in events {
        let Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
        let Some((id, amount)) = sc::key_command(key, modifiers) else { continue };
        ui.input_mut(|i| i.consume_key(modifiers, key));
        pending.push((id, json!({"glyphs": [col.glyph], "layer": col.layer, "amount": amount})));
    }
}

/// Fills `outline`, in screen points, in white into a texture of the area `clip`; the caller
/// tints it.
fn render(ui: &egui::Ui, outline: &BezPath, clip: Rect, ppp: f32) -> Option<TextureHandle> {
    if outline.elements().is_empty() {
        return None;
    }
    let (w, h) = ((clip.width() * ppp).round(), (clip.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let to_pixels = Affine::scale(f64::from(ppp)) * Affine::translate((-f64::from(clip.min.x), -f64::from(clip.min.y)));
    let style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };
    render_path(outline, to_pixels, w as u32, h as u32, &style).ok().map(|image| {
        let colour = ColorImage::from_rgba_premultiplied([w as usize, h as usize], &image.pixels);
        ui.ctx().load_texture("space-center-fill", colour, TextureOptions::LINEAR)
    })
}

/// The matrix: a column per glyph of the text, rows for the width and the margins.
fn matrix(ui: &mut egui::Ui, st: &mut SpaceCenterState, tokens: &Tokens, font: &Font, master: &str, pending: &mut Pending) {
    let Some(resolved) = st.cache.resolved.clone() else { return };
    let values = st.cache.values.clone();
    let shown = resolved.columns.len().min(MAX_MATRIX_COLUMNS);
    egui::ScrollArea::horizontal().id_salt(widgets::id(("spaceCenter", "matrix", "scroll"))).show(ui, |ui| {
        egui::Grid::new(widgets::id(("spaceCenter", "matrix"))).striped(true).min_col_width(tokens.field_width * 0.6).show(ui, |ui| {
            ui.label(egui::RichText::new("Glyph").color(tokens.text_dim));
            for (i, col) in resolved.columns.iter().take(shown).enumerate() {
                if ui.selectable_label(st.selected == Some(i), &col.glyph).clicked() {
                    pending.push(("window.spaceCenter", json!({ "select": i })));
                }
            }
            ui.end_row();
            for row in Row::ALL {
                ui.label(egui::RichText::new(row.label()).color(tokens.text_dim));
                for (i, col) in resolved.columns.iter().take(shown).enumerate() {
                    let value = values.get(i).and_then(|v| v.get(row.index()).copied().flatten());
                    cell(ui, st, tokens, font, master, &resolved, &values, i, col, row, value, pending);
                }
                ui.end_row();
            }
        });
    });
    if let Some(e) = &st.cache.matrix_error {
        ui.label(egui::RichText::new(e).color(tokens.error_text));
    }
}

fn shown_value(value: Option<f64>) -> String {
    value.map_or_else(String::new, format_value)
}

#[allow(clippy::too_many_arguments)]
fn cell(
    ui: &mut egui::Ui,
    st: &mut SpaceCenterState,
    tokens: &Tokens,
    font: &Font,
    master: &str,
    resolved: &Resolved,
    values: &[[Option<f64>; 3]],
    column: usize,
    col: &Column,
    row: Row,
    value: Option<f64>,
    pending: &mut Pending,
) {
    let editing_here = st.editing.as_ref().is_some_and(|e| e.column == column && e.row == row);
    if !editing_here {
        let text = value.map_or_else(|| "–".to_string(), format_value);
        if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
            let shown = shown_value(value);
            st.editing = Some(Editing { column, row, buffer: shown.clone(), original: shown, focused: false });
            st.cache.matrix_error = None;
        }
        return;
    }
    let id = cell_id(column, row);
    let focused = ui.memory(|m| m.has_focus(id));
    // Keys the cell handles itself, taken before the text field sees them.
    let mut confirm = false;
    let mut cancel = false;
    let mut moved: Option<(usize, Row)> = None;
    let mut stepped: Option<f64> = None;
    if focused {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            let Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
            match key {
                Key::Enter => confirm = true,
                Key::Escape => cancel = true,
                Key::ArrowUp | Key::ArrowDown if !modifiers.alt => {
                    stepped = Some(if key == Key::ArrowUp { sc::step(modifiers) } else { -sc::step(modifiers) });
                }
                _ => match sc::neighbour(column, row, key, modifiers, resolved.columns.len().min(MAX_MATRIX_COLUMNS)) {
                    Some(to) => moved = Some(to),
                    None => continue,
                },
            }
            ui.input_mut(|i| i.consume_key(modifiers, key));
        }
    }
    let Some(editing) = st.editing.as_mut() else { return };
    let response = ui.add(egui::TextEdit::singleline(&mut editing.buffer).id(id).char_limit(MAX_CELL_CHARS).desired_width(tokens.field_width * 0.6));
    // After the text field, which sets its own filter: the cell keeps the arrows, Tab and
    // Escape, so they reach the handling above on the next frame instead of moving focus.
    if response.has_focus() {
        let all = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: true, escape: true };
        ui.memory_mut(|m| m.set_focus_lock_filter(id, all));
    }
    if !editing.focused {
        response.request_focus();
        editing.focused = true;
    }
    if cancel {
        st.editing = None;
        return;
    }
    if let Some(delta) = stepped {
        let base = sc::entered_value(font, master, row, &editing.buffer).ok().or(value);
        if let Some(base) = base {
            let next = base + delta;
            match sc::matrix_command(font, master, col, row, &format_value(next)) {
                Ok(command) => {
                    pending.push(command);
                    editing.buffer = format_value(next);
                    editing.original = editing.buffer.clone();
                    st.cache.matrix_error = None;
                }
                Err(e) => st.cache.matrix_error = Some(e),
            }
        }
        return;
    }
    let leaving = response.lost_focus() && !confirm && moved.is_none();
    if confirm || moved.is_some() || leaving {
        let typed = editing.buffer.clone();
        let unchanged = typed.trim() == editing.original.trim();
        st.editing = None;
        if !unchanged && !typed.trim().is_empty() {
            match sc::matrix_command(font, master, col, row, &typed) {
                Ok(command) => {
                    pending.push(command);
                    st.cache.matrix_error = None;
                }
                Err(e) => {
                    st.cache.matrix_error = Some(e);
                    return;
                }
            }
        }
        if let Some((c, r)) = moved {
            let v = values.get(c).and_then(|v| v.get(r.index()).copied().flatten());
            let shown = shown_value(v);
            st.editing = Some(Editing { column: c, row: r, buffer: shown.clone(), original: shown, focused: false });
        }
    }
}
