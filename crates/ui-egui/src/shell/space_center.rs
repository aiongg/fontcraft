//! The Space Center (`window.spaceCenter`): text with the tokens of `spaceCenter.inputSyntax`,
//! matrix entry through `fontcraft_geom::expr`, pointer and key steps, the spacing cut-off and
//! preferred suffix. The window is drawn by `crate::dialogs::space_center`.
//!
//! | Token | Gives |
//! |---|---|
//! | a character | the glyph that encodes it |
//! | `/name` | the glyph `name`; the name runs to the next `/`, space, backslash or line break, and one space after it is dropped |
//! | `/name@layer` | that glyph drawn from its layer named (or with the id) `layer`, else from the master named `layer` |
//! | `/?` | the glyph being edited (Font View: the first selected glyph) |
//! | `/!` | the glyphs selected in Font View |
//! | `/:group` | every glyph whose left or right kerning group is `group`, in glyph order |
//! | `\n` (backslash, n) or a line break | a new line |
//!
//! The control-glyph fields are read the same way; their glyphs stand before and after every
//! glyph of the text. The preferred suffix then replaces each glyph by its variant with that
//! suffix where the font has one. The matrix has one column per glyph of the text (control
//! glyphs have none) and rows for the width and the left and right margins of the drawn layer.
//!
//! A stored sample string (`window.spaceCenter {sample}`) is read with the same tokens; its
//! `/Placeholder` becomes `/?`. The display options (`view.setSpaceCenterDisplay`) set the
//! direction, centring and turning of the preview and what is drawn over the glyphs; the metrics
//! beam takes its height from `view.showMetricsBeam`, which Edit View shares.
//!
//! The font changes only through engine commands: the matrix and the keys run
//! `spacing.setWidth`, `spacing.setLeftSidebearing`, `spacing.setRightSidebearing`,
//! `spacing.increaseLeftSidebearing`, `spacing.increaseRightSidebearing` and
//! `spacing.shiftOutlineRight` with a number, so nothing stays linked as a metrics key would.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use egui::{Key, Modifiers};
use fontcraft_geom::expr::Expr;
use fontcraft_model::{Font, MAX_COORDINATE, MAX_NAME_LEN, metric};
use fontcraft_spacing::{Field, SIDEBEARING_RANGE, WIDTH_RANGE, round_half_up};
use fontcraft_textlayout::{Charmap, Item, MAX_ITEMS};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Longest text, and longest control-glyph field, in characters.
pub const MAX_TEXT_CHARS: usize = 10_000;
/// Most glyphs laid out, control glyphs included; longer texts are cut.
pub const MAX_GLYPHS_SHOWN: usize = MAX_ITEMS;
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
/// Glyph names listed in a reply.
const MAX_LISTED: usize = 1_000;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;

pub const COMMANDS: &[UiCommand] = &[
    (
        "window.spaceCenter",
        "Space Center",
        &["Window"],
        "",
        "{open?: bool (default true), text?: string, sample?: index, before?: string, after?: string, size?: points (4 to 1000), lineMode?: \"multi\"|\"single\"|\"waterfall\", select?: column index|null} show or hide the Space Center and set its text, its control glyphs (placed before and after every glyph of the text), its size, the display (several lines wrapped at the window's width, one line, or a waterfall: one line at a series of sizes up to the window's size) and the selected glyph. Texts take characters, /name, /name@layer, /? (the glyph being edited), /! (the Font View selection), /:group (the members of a kerning group) and \\n (a new line); at most 10000 characters each. `sample` puts in the stored sample string with that index, counted over all groups in order as edit.selectSampleText lists them, its /Placeholder read as /? → {open, text, before, after, size, lineMode, master, glyphs: [names of the text's glyphs, one per matrix column], count, selected, cutOff: {on, bottom, top}, suffix, display, message}",
    ),
    (
        "view.setSpacingCutOff",
        "Cut-Off Band",
        &["View"],
        "",
        "{on?: bool (default: true when a height is given, else the opposite of now), bottom?: units|null, top?: units|null} draw the Space Center's preview inverted between two heights; a height left out keeps its value, and null means the baseline (bottom) or the x-height of the master shown (top) → {on, bottom, top}",
    ),
    (
        "view.setPreferredSuffix",
        "Prefer Suffix",
        &["View"],
        "",
        "{suffix?: string (absent or empty: none)} show, for every glyph of the Space Center's text, its variant named with this suffix (`sc` or `.sc` both mean `.sc`) when the font has one → {suffix, replaced: number of glyphs shown as their variant}",
    ),
    (
        "view.setSpaceCenterDisplay",
        "Space Center Display Options",
        &[],
        "",
        "{rightToLeft?: bool, centered?: bool, upsideDown?: bool, metrics?: bool, beam?: bool, beamY?: units, fill?: bool, stroke?: bool, inverse?: bool, guides?: bool, zones?: bool, layers?: [layer or master names] (at most 8), lineSpace?: 0.5 to 4, tracking?: units (-1000 to 1000)} set how the Space Center draws its text; options left out keep their value. rightToLeft lays the text out right to left from the right edge with the right-to-left kerning, centered centres every line, upsideDown turns the whole preview half a circle; metrics draws the glyphs' edges, beam a line at height beamY (shared with view.showMetricsBeam; by default half the x-height) with the margins measured where it meets each outline, fill and stroke the outlines filled and outlined, inverse light glyphs on a dark ground, guides the master's and the glyphs' guides, zones the master's alignment zones, layers the outlines of those layers (by layer name or id, else the master of that name) over each glyph; lineSpace multiplies the line height and tracking adds space between glyphs. Giving beamY without beam also shows the beam → {rightToLeft, centered, upsideDown, metrics, beam, beamY, fill, stroke, inverse, guides, zones, layers, lineSpace, tracking}",
    ),
];

/// Multi-line text that wraps at the window's width, one line, or one line at a series of sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineMode {
    #[default]
    Multi,
    Single,
    Waterfall,
}

impl LineMode {
    pub const ALL: [LineMode; 3] = [LineMode::Multi, LineMode::Single, LineMode::Waterfall];

    pub fn name(self) -> &'static str {
        match self {
            LineMode::Multi => "multi",
            LineMode::Single => "single",
            LineMode::Waterfall => "waterfall",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineMode::Multi => "Lines",
            LineMode::Single => "One Line",
            LineMode::Waterfall => "Waterfall",
        }
    }

    fn from_name(name: &str) -> Option<LineMode> {
        LineMode::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The sizes of a waterfall ending at `size`, in points, smallest first.
pub fn waterfall_sizes(size: f64) -> Vec<f64> {
    WATERFALL_SIZES.iter().copied().filter(|s| *s < size).chain(std::iter::once(size)).collect()
}

/// How the preview is drawn: `view.setSpaceCenterDisplay`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Display {
    pub right_to_left: bool,
    pub centered: bool,
    pub upside_down: bool,
    pub metrics: bool,
    pub beam: bool,
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

    /// The line spacing, within [`LINE_SPACE_RANGE`] (stored state is not trusted).
    pub fn line_space(&self) -> f64 {
        if LINE_SPACE_RANGE.contains(&self.line_space) { self.line_space } else { 1.0 }
    }

    /// The tracking, within [`TRACKING_RANGE`].
    pub fn tracking(&self) -> f64 {
        if TRACKING_RANGE.contains(&self.tracking) { self.tracking } else { 0.0 }
    }

    /// The overlay layers, at most [`MAX_OVERLAY_LAYERS`].
    pub fn overlay_layers(&self) -> &[String] {
        self.layers.get(..MAX_OVERLAY_LAYERS).unwrap_or(&self.layers)
    }
}

/// The cut-off band: on or off, and its heights (`None`: the baseline and the x-height).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CutOff {
    pub on: bool,
    pub bottom: Option<f64>,
    pub top: Option<f64>,
}

/// One row of the matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Row {
    Width,
    Left,
    Right,
}

impl Row {
    pub const ALL: [Row; 3] = [Row::Width, Row::Left, Row::Right];

    pub fn field(self) -> Field {
        match self {
            Row::Width => Field::Width,
            Row::Left => Field::Left,
            Row::Right => Field::Right,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Row::Width => "Width",
            Row::Left => "Left",
            Row::Right => "Right",
        }
    }

    /// The engine command that sets this row's value.
    pub fn setter(self) -> &'static str {
        match self {
            Row::Width => "spacing.setWidth",
            Row::Left => "spacing.setLeftSidebearing",
            Row::Right => "spacing.setRightSidebearing",
        }
    }

    /// Its position in [`Row::ALL`].
    pub fn index(self) -> usize {
        match self {
            Row::Width => 0,
            Row::Left => 1,
            Row::Right => 2,
        }
    }
}

/// The matrix cell being edited and what has been typed into it.
#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub column: usize,
    pub row: Row,
    pub buffer: String,
    /// The cell's text when editing began; confirming it unchanged runs nothing.
    pub original: String,
    /// Focus is asked for once, on the first frame.
    pub focused: bool,
}

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpaceCenterState {
    pub open: bool,
    pub text: String,
    pub before: String,
    pub after: String,
    pub size: f64,
    pub line_mode: LineMode,
    pub cut_off: CutOff,
    pub suffix: Option<String>,
    pub display: Display,
    /// The selected glyph, as a matrix column.
    pub selected: Option<usize>,
    /// The window has the keyboard (a press inside it gives it, a press elsewhere takes it).
    #[serde(skip)]
    pub keyboard: bool,
    #[serde(skip)]
    pub editing: Option<Editing>,
    #[serde(skip)]
    pub cache: crate::dialogs::space_center::Cache,
}

impl Default for SpaceCenterState {
    fn default() -> Self {
        SpaceCenterState {
            open: false,
            text: DEFAULT_TEXT.to_string(),
            before: String::new(),
            after: String::new(),
            size: DEFAULT_SIZE,
            line_mode: LineMode::Multi,
            cut_off: CutOff::default(),
            suffix: None,
            display: Display::default(),
            selected: None,
            keyboard: false,
            editing: None,
            cache: Default::default(),
        }
    }
}

// ── Tokens ────────────────────────────────────────────────────────────────────────────────────

/// One element of a Space Center text after its tokens are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// A glyph by name, from another layer when `layer` is given (as typed after `@`).
    Glyph {
        name: String,
        layer: Option<String>,
    },
    /// A character no glyph encodes.
    Missing(char),
    LineBreak,
}

/// What the tokens `/?` and `/!` stand for.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context<'a> {
    /// The glyph being edited.
    pub current: Option<&'a str>,
    /// The Font View selection.
    pub selection: &'a [String],
}

fn ends_name(c: char) -> bool {
    matches!(c, '/' | ' ' | '\\' | '\n' | '\r')
}

/// Reads a Space Center text into tokens, at most `limit` of them. Problems (a name that is too
/// long, an unknown group, a text cut at the limit) are collected in `problems`; the tokens
/// read so far stay.
pub fn tokens(font: &Font, map: &Charmap, text: &str, cx: &Context<'_>, limit: usize, problems: &mut Vec<String>) -> Vec<Token> {
    let chars: Vec<char> = text.chars().take(MAX_TEXT_CHARS).collect();
    let mut out: Vec<Token> = Vec::new();
    let mut full = false;
    let mut push = |out: &mut Vec<Token>, t: Token| {
        if out.len() < limit {
            out.push(t);
        } else {
            full = true;
        }
    };
    let at = |i: usize| chars.get(i).copied();
    // A run of name characters from `start`: its end.
    let name_end = |start: usize| (start..chars.len()).find(|&j| at(j).is_none_or(ends_name)).unwrap_or(chars.len());
    let mut i = 0;
    while let Some(c) = at(i) {
        match c {
            '\\' if at(i + 1) == Some('n') => {
                push(&mut out, Token::LineBreak);
                i += 2;
            }
            '\r' => {
                push(&mut out, Token::LineBreak);
                i += if at(i + 1) == Some('\n') { 2 } else { 1 };
            }
            '\n' => {
                push(&mut out, Token::LineBreak);
                i += 1;
            }
            '/' => {
                let next = at(i + 1);
                let mut end = match next {
                    Some('?') => {
                        if let Some(name) = cx.current {
                            push(&mut out, Token::Glyph { name: name.to_string(), layer: None });
                        }
                        i + 2
                    }
                    Some('!') => {
                        for name in cx.selection {
                            push(&mut out, Token::Glyph { name: name.clone(), layer: None });
                        }
                        i + 2
                    }
                    Some(':') => {
                        let end = name_end(i + 2);
                        let group: String = chars.get(i + 2..end).unwrap_or_default().iter().collect();
                        let group = group.strip_prefix('@').unwrap_or(&group);
                        let members: Vec<&str> = font
                            .glyphs
                            .iter()
                            .filter(|g| g.kern_groups.left.as_deref() == Some(group) || g.kern_groups.right.as_deref() == Some(group))
                            .map(|g| g.name.as_str())
                            .collect();
                        if members.is_empty() {
                            problems.push(format!("no glyph is in a kerning group named `{}`", quoted(group)));
                        }
                        for name in members {
                            push(&mut out, Token::Glyph { name: name.to_string(), layer: None });
                        }
                        end
                    }
                    _ => {
                        let end = name_end(i + 1);
                        let word: String = chars.get(i + 1..end).unwrap_or_default().iter().collect();
                        if word.is_empty() {
                            push(&mut out, glyph_or_missing(map, '/'));
                            // A lone slash is the slash character; nothing after it is dropped.
                            i = end;
                            continue;
                        }
                        let (name, layer) = match word.split_once('@') {
                            Some((n, l)) => (n.to_string(), (!l.is_empty()).then(|| l.to_string())),
                            None => (word, None),
                        };
                        if name.len() > MAX_NAME_LEN {
                            problems.push(format!("a glyph name after `/` is longer than {MAX_NAME_LEN} bytes"));
                        } else if !name.is_empty() {
                            push(&mut out, Token::Glyph { name, layer });
                        }
                        end
                    }
                };
                // One space after a slash token separates it from what follows and is dropped.
                if at(end) == Some(' ') {
                    end += 1;
                }
                i = end;
            }
            c => {
                push(&mut out, glyph_or_missing(map, c));
                i += 1;
            }
        }
    }
    if text.chars().nth(MAX_TEXT_CHARS).is_some() {
        problems.push(format!("only the first {MAX_TEXT_CHARS} characters are read"));
    }
    if full {
        problems.push(format!("the text is cut at {limit} glyphs"));
    }
    out
}

/// A stored sample string as Space Center text: its `/Placeholder` tokens become `/?`; the rest
/// (`/name`, `\n`) reads the same in both.
pub fn sample_text(sample: &str) -> String {
    let token = format!("/{}", fontcraft_textlayout::PLACEHOLDER);
    let mut out = String::with_capacity(sample.len());
    let mut rest = sample;
    while let Some((head, tail)) = rest.split_once(token.as_str()) {
        out.push_str(head);
        // `/Placeholder.alt` names another glyph; only the whole name is the placeholder.
        out.push_str(if tail.chars().next().is_none_or(ends_name) { "/?" } else { token.as_str() });
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn glyph_or_missing(map: &Charmap, c: char) -> Token {
    match map.item(c) {
        Item::Glyph { name, .. } => Token::Glyph { name: name.to_string(), layer: None },
        _ => Token::Missing(c),
    }
}

fn quoted(s: &str) -> String {
    let cut: String = s.chars().take(MAX_QUOTED).collect();
    if cut.len() < s.len() { format!("{cut}…") } else { cut }
}

/// The glyph shown for `name` under the preferred suffix (with its dot): the variant when the
/// font has it, else `name`.
pub fn preferred(font: &Font, name: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(s) if !s.is_empty() => {
            let variant = format!("{name}{s}");
            if font.glyph(&variant).is_some() { variant } else { name.to_string() }
        }
        _ => name.to_string(),
    }
}

/// A suffix as typed, with its leading dot: `sc` and `.sc` are both `.sc`; empty is none.
pub fn normalised_suffix(text: &str) -> Result<Option<String>, String> {
    let bare = text.trim().trim_start_matches('.');
    if bare.is_empty() {
        return Ok(None);
    }
    if bare.len() >= MAX_NAME_LEN {
        return Err(format!("a suffix is shorter than {MAX_NAME_LEN} bytes"));
    }
    if bare.chars().any(|c| c.is_whitespace() || c.is_control() || c == '/') {
        return Err("a suffix has no spaces, slashes or control characters".to_string());
    }
    Ok(Some(format!(".{bare}")))
}

/// The layer id `typed` (after `@`) names for `glyph` in `master`: a layer of that master with
/// that name or id, else any layer of the glyph with that name or id, else the glyph's layer of
/// the master named `typed`.
pub fn layer_named(font: &Font, glyph: &str, master: &str, typed: &str) -> Option<String> {
    let g = font.glyph(glyph)?;
    let named = |l: &&fontcraft_model::Layer| l.id == typed || l.info.name.as_deref() == Some(typed);
    if let Some(l) = g.layers.iter().filter(|l| l.master == master).find(named) {
        return Some(l.id.clone());
    }
    if let Some(l) = g.layers.iter().find(named) {
        return Some(l.id.clone());
    }
    let m = font.masters.iter().find(|m| m.name == typed)?;
    g.layer(&m.id).map(|l| l.id.clone())
}

/// One column of the matrix: a glyph of the text as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The glyph drawn (after the preferred suffix).
    pub glyph: String,
    /// The layer drawn: another layer named after `@`, else the master's layer.
    pub layer: String,
    /// Its item in [`Resolved::items`].
    pub item: usize,
}

/// A Space Center text read for one master: the items to lay out, the matrix columns, the
/// layers other than the master's, and the first problem met.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub items: Vec<Item>,
    pub columns: Vec<Column>,
    /// Glyph name → layer id, for glyphs drawn from another layer than the master's. A glyph
    /// asked for in two layers is drawn from the last one named.
    pub layers: BTreeMap<String, String>,
    /// Glyphs shown as their preferred-suffix variant.
    pub replaced: usize,
    pub message: Option<String>,
}

/// Reads the window's text and control glyphs for `master`.
pub fn resolve(font: &Font, master: &str, state: &SpaceCenterState, cx: &Context<'_>) -> Resolved {
    let map = Charmap::new(font);
    let mut problems = Vec::new();
    let text = tokens(font, &map, &state.text, cx, MAX_GLYPHS_SHOWN, &mut problems);
    let before = tokens(font, &map, &state.before, cx, MAX_GLYPHS_SHOWN, &mut problems);
    let after = tokens(font, &map, &state.after, cx, MAX_GLYPHS_SHOWN, &mut problems);
    let suffix = state.suffix.as_deref();
    let mut out = Resolved::default();
    let mut cut = false;
    let mut item = |out: &mut Resolved, token: &Token, replaced: &mut usize| -> Option<usize> {
        if out.items.len() >= MAX_GLYPHS_SHOWN {
            cut = true;
            return None;
        }
        let at = out.items.len();
        match token {
            Token::Glyph { name, layer } => {
                let shown = preferred(font, name, suffix);
                if shown != *name {
                    *replaced += 1;
                }
                if let Some(typed) = layer {
                    match layer_named(font, &shown, master, typed) {
                        Some(id) => {
                            out.layers.insert(shown.clone(), id);
                        }
                        None => problems.push(format!("`{}` has no layer `{}`", quoted(&shown), quoted(typed))),
                    }
                }
                out.items.push(Item::glyph(&shown));
            }
            Token::Missing(c) => out.items.push(Item::Missing(*c)),
            Token::LineBreak => out.items.push(Item::LineBreak),
        }
        Some(at)
    };
    let mut replaced = 0;
    for token in &text {
        let Token::Glyph { .. } = token else {
            item(&mut out, token, &mut replaced);
            continue;
        };
        for t in &before {
            item(&mut out, t, &mut replaced);
        }
        if let Some(at) = item(&mut out, token, &mut replaced) {
            out.columns.push(Column { glyph: String::new(), layer: String::new(), item: at });
        }
        for t in &after {
            item(&mut out, t, &mut replaced);
        }
    }
    if cut {
        problems.push(format!("the text is cut at {MAX_GLYPHS_SHOWN} glyphs"));
    }
    for column in &mut out.columns {
        if let Some(Item::Glyph { name, .. }) = out.items.get(column.item) {
            column.glyph = name.to_string();
            column.layer = out.layers.get(&**name).cloned().unwrap_or_else(|| master.to_string());
        }
    }
    out.replaced = replaced;
    out.message = problems.into_iter().next();
    out
}

// ── Matrix ────────────────────────────────────────────────────────────────────────────────────

/// The value of `row` for `glyph` in its layer `layer`: `None` for a glyph or layer the font
/// lacks and for the margins of an empty layer.
pub fn value(font: &Font, glyph: &str, layer: &str, row: Row) -> Option<f64> {
    let l = font.glyph(glyph)?.layers.iter().find(|l| l.id == layer)?;
    fontcraft_spacing::metric(font, l, row.field()).ok().flatten()
}

/// What a matrix cell holds once `typed` is entered for `glyph` in `master`: a number, or `=`
/// and an expression whose names are glyphs, each standing for that glyph's value in the same
/// row and master (`=n`, `=e+10`, `=(n+o)/2`). The result rounds halves up and must lie in the
/// row's range.
pub fn entered_value(font: &Font, master: &str, row: Row, typed: &str) -> Result<f64, String> {
    let typed = typed.trim();
    let v = match typed.strip_prefix('=') {
        Some(source) => {
            let expr = Expr::parse(source).map_err(|e| e.to_string())?;
            let lookup = |name: &str| font.glyph(name).and_then(|g| g.layer(master)).and_then(|l| value(font, name, &l.id, row));
            if let Some(missing) = expr.names().into_iter().find(|n| lookup(n).is_none()) {
                return Err(match font.glyph(&missing) {
                    Some(_) => format!("`{}` has no {} value in this master", quoted(&missing), row.label().to_lowercase()),
                    None => format!("the font has no glyph `{}`", quoted(&missing)),
                });
            }
            expr.eval(lookup).map_err(|e| e.to_string())?
        }
        None => typed.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("`{}` is not a number", quoted(typed)))?,
    };
    let v = round_half_up(v);
    let range = if row == Row::Width { WIDTH_RANGE } else { SIDEBEARING_RANGE };
    if !range.contains(&v) {
        return Err(format!("{} must lie between {} and {}, not {v}", row.label(), range.start(), range.end()));
    }
    Ok(v)
}

/// The engine command a matrix entry runs: the row's setter with the evaluated number.
pub fn matrix_command(font: &Font, master: &str, column: &Column, row: Row, typed: &str) -> Result<(&'static str, Value), String> {
    let v = entered_value(font, master, row, typed)?;
    Ok((row.setter(), json!({"glyphs": [column.glyph], "layer": column.layer, "value": v})))
}

/// The step of an arrow key: 100 with Shift and Cmd, 10 with Shift, else 1.
pub fn step(m: Modifiers) -> f64 {
    if m.shift && m.command && !m.ctrl {
        100.0
    } else if m.shift {
        10.0
    } else {
        1.0
    }
}

/// The cell an editing key moves to from (`column`, `row`) among `columns` columns: Tab and
/// Shift+Tab along the row, Alt with an arrow to the neighbouring cell. `None` when the key
/// does not move, or would leave the matrix.
pub fn neighbour(column: usize, row: Row, key: Key, m: Modifiers, columns: usize) -> Option<(usize, Row)> {
    let right = |c: usize| (c + 1 < columns).then_some((c + 1, row));
    let left = |c: usize| c.checked_sub(1).map(|c| (c, row));
    match key {
        Key::Tab if m.shift => left(column),
        Key::Tab => right(column),
        Key::ArrowRight if m.alt => right(column),
        Key::ArrowLeft if m.alt => left(column),
        Key::ArrowUp if m.alt => row.index().checked_sub(1).and_then(|i| Row::ALL.get(i)).map(|r| (column, *r)),
        Key::ArrowDown if m.alt => Row::ALL.get(row.index() + 1).map(|r| (column, *r)),
        _ => None,
    }
}

/// The engine command an arrow key runs on the selected glyph in the preview: Left and Right
/// the right margin, Alt with them the left margin, Ctrl+Cmd with them the outline inside the
/// width; each side's arrow moves its edge that way. Steps from [`step`].
pub fn key_command(key: Key, m: Modifiers) -> Option<(&'static str, f64)> {
    let sign = match key {
        Key::ArrowRight => 1.0,
        Key::ArrowLeft => -1.0,
        _ => return None,
    };
    let s = step(m);
    if m.ctrl && m.command {
        return Some(("spacing.shiftOutlineRight", sign * s));
    }
    if m.alt {
        return Some(("spacing.increaseLeftSidebearing", -sign * s));
    }
    if m.ctrl || (m.command && !m.shift) {
        return None;
    }
    Some(("spacing.increaseRightSidebearing", sign * s))
}

/// What a drag in the preview changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Left,
    Right,
    Outline,
}

/// The commands a finished drag of `dx` units runs on `glyph`: on `layers` (one layer, or every
/// master layer with Alt). A left margin dragged left grows; a right margin dragged right grows.
pub fn drag_commands(drag: Drag, glyph: &str, layers: &[String], dx: f64) -> Vec<(&'static str, Value)> {
    let dx = dx.round();
    if dx == 0.0 || !dx.is_finite() {
        return Vec::new();
    }
    let (id, amount) = match drag {
        Drag::Left => ("spacing.increaseLeftSidebearing", -dx),
        Drag::Right => ("spacing.increaseRightSidebearing", dx),
        Drag::Outline => ("spacing.shiftOutlineRight", dx),
    };
    layers.iter().map(|layer| (id, json!({"glyphs": [glyph], "layer": layer, "amount": amount}))).collect()
}

/// The x-height of `master`; half the em when it has none.
pub fn x_height(font: &Font, master: &str) -> f64 {
    let upm = f64::from(font.units_per_em);
    let x = font.masters.iter().find(|m| m.id == master).map_or(upm * 0.5, |m| font.metric_pos(m, metric::X_HEIGHT, upm * 0.5));
    if x.is_finite() { x } else { upm * 0.5 }
}

/// The cut-off band's bottom and top in `master`: the heights set, else the baseline and the
/// master's x-height.
pub fn band(font: &Font, master: &str, cut: &CutOff) -> (f64, f64) {
    let bottom = cut.bottom.unwrap_or(0.0);
    let top = cut.top.unwrap_or_else(|| x_height(font, master));
    (bottom.min(top), bottom.max(top))
}

/// The metrics beam's height for document `doc` in `master`: the height `view.showMetricsBeam`
/// keeps for the document, else half the master's x-height, as in Edit View.
pub fn beam_height(app: &FontcraftApp, font: &Font, doc: u64, master: &str) -> f64 {
    app.state.panels.edit_aids.beam_heights.get(&doc).copied().filter(|y| y.is_finite()).unwrap_or_else(|| x_height(font, master) / 2.0)
}

/// The beam height of the document and master shown.
fn shown_beam(app: &FontcraftApp) -> Option<f64> {
    let font = app.session.font().ok()?;
    let doc = app.session.active()?.id;
    let master = shown_master(app)?;
    Some(beam_height(app, font, doc, &master))
}

// ── Commands ──────────────────────────────────────────────────────────────────────────────────

/// The master the window draws: the showing tab's, else the first.
pub fn shown_master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    match app.state.tab() {
        Some(tab) => crate::canvas::view::tab_master(font, tab).map(|m| m.id.clone()),
        None => font.masters.first().map(|m| m.id.clone()),
    }
}

/// What `/?` and `/!` stand for in this app.
pub fn context(app: &FontcraftApp) -> (Option<String>, Vec<String>) {
    let current = app.state.editing().map(str::to_string).or_else(|| app.state.selected_glyphs.first().cloned());
    (current, app.state.selected_glyphs.clone())
}

/// The window's text read for the master shown.
pub fn resolved(app: &FontcraftApp) -> Option<(String, Resolved)> {
    let font = app.session.font().ok()?;
    let master = shown_master(app)?;
    let (current, selection) = context(app);
    let cx = Context { current: current.as_deref(), selection: &selection };
    let r = resolve(font, &master, &app.state.panels.space_center, &cx);
    Some((master, r))
}

fn text_arg(args: &Args, key: &str) -> Result<Option<String>, String> {
    match args.string(key)? {
        None => Ok(None),
        Some(t) if t.chars().nth(MAX_TEXT_CHARS).is_some() => Err(args.err(format!("`{key}` holds at most {MAX_TEXT_CHARS} characters"))),
        Some(t) => Ok(Some(t.to_string())),
    }
}

fn height_arg(args: &Args, key: &str) -> Result<Option<f64>, String> {
    match args.number(key)? {
        Some(v) if v.abs() > MAX_COORDINATE => Err(args.err(format!("`{key}` lies between -{MAX_COORDINATE} and {MAX_COORDINATE}"))),
        v => Ok(v),
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.spaceCenter" => {
            // Every parameter is checked before anything changes.
            let open = args.flag("open")?.unwrap_or(true);
            let text = text_arg(args, "text")?;
            let before = text_arg(args, "before")?;
            let after = text_arg(args, "after")?;
            let size = args.number("size")?;
            if let Some(s) = size
                && !SIZE_RANGE.contains(&s)
            {
                return Err(args.err(format!("`size` lies between {} and {} points", SIZE_RANGE.start(), SIZE_RANGE.end())));
            }
            let mode = match args.string("lineMode")? {
                None => None,
                Some(name) => Some(LineMode::from_name(name).ok_or_else(|| args.err("`lineMode` must be multi, single or waterfall"))?),
            };
            let sample = match args.index("sample")? {
                None => None,
                Some(_) if text.is_some() => return Err(args.err("give `text` or `sample`, not both")),
                Some(i) => {
                    let groups = crate::dialogs::sample_strings::groups(&app.session);
                    let all: Vec<&String> = groups.iter().flat_map(|g| g.strings.iter()).collect();
                    let s = usize::try_from(i)
                        .ok()
                        .and_then(|i| all.get(i))
                        .ok_or_else(|| args.err(format!("there are {} sample strings, no string {i}", all.len())))?;
                    Some(sample_text(s).chars().take(MAX_TEXT_CHARS).collect::<String>())
                }
            };
            let text = text.or(sample);
            let mut next = app.state.panels.space_center.clone();
            next.open = open;
            next.keyboard = open;
            if let Some(t) = text {
                next.text = t;
            }
            if let Some(t) = before {
                next.before = t;
            }
            if let Some(t) = after {
                next.after = t;
            }
            if let Some(s) = size {
                next.size = s;
            }
            if let Some(m) = mode {
                next.line_mode = m;
            }
            let columns = {
                let saved = std::mem::replace(&mut app.state.panels.space_center, next.clone());
                let count = resolved(app).map_or(0, |(_, r)| r.columns.len());
                app.state.panels.space_center = saved;
                count
            };
            match args.get("select") {
                None => {}
                Some(v) => {
                    let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| args.err("`select` must be a column index or null"))?;
                    if i >= columns {
                        return Err(args.err(format!("the text has {columns} glyphs, no glyph {i}")));
                    }
                    next.selected = Some(i);
                }
            }
            if args.value().get("select").is_some_and(Value::is_null) {
                next.selected = None;
            }
            if next.selected.is_some_and(|s| s >= columns) {
                next.selected = None;
            }
            let was = &app.state.panels.space_center;
            if next.text != was.text || next.before != was.before || next.after != was.after {
                next.editing = None;
            }
            app.state.panels.space_center = next;
            Ok(summary(app))
        }
        "view.setSpacingCutOff" => {
            let current = app.state.panels.space_center.cut_off;
            let (new_bottom, new_top) = (height_arg(args, "bottom")?, height_arg(args, "top")?);
            // Heights alone turn the band on; nothing at all toggles it, as the menu item does.
            let on = match args.flag("on")? {
                Some(on) => on,
                None if new_bottom.is_some() || new_top.is_some() => true,
                None => !current.on,
            };
            let reset = |key: &str| args.value().get(key).is_some_and(Value::is_null);
            let bottom = if reset("bottom") { None } else { new_bottom.or(current.bottom) };
            let top = if reset("top") { None } else { new_top.or(current.top) };
            let next = CutOff { on, bottom, top };
            if let (Some(b), Some(t)) = (bottom, top)
                && b >= t
            {
                return Err(args.err("`bottom` must lie below `top`"));
            }
            app.state.panels.space_center.cut_off = next;
            Ok(cut_off_json(app))
        }
        "view.setPreferredSuffix" => {
            let suffix = match args.string("suffix")? {
                None => None,
                Some(t) => normalised_suffix(t).map_err(|e| args.err(e))?,
            };
            app.state.panels.space_center.suffix = suffix;
            let replaced = resolved(app).map_or(0, |(_, r)| r.replaced);
            Ok(json!({"suffix": app.state.panels.space_center.suffix, "replaced": replaced}))
        }
        "view.setSpaceCenterDisplay" => set_display(app, args),
        _ => Err(args.err("unknown command")),
    }
}

fn set_display(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    // Every parameter is checked before anything changes.
    let mut next = app.state.panels.space_center.display.clone();
    for key in DISPLAY_FLAGS {
        if let (Some(on), Some(slot)) = (args.flag(key)?, next.flag_mut(key)) {
            *slot = on;
        }
    }
    if let Some(names) = args.strings("layers", MAX_OVERLAY_LAYERS)? {
        let mut layers: Vec<String> = Vec::new();
        for name in names {
            let name = name.trim().to_string();
            if name.is_empty() || name.len() > MAX_NAME_LEN {
                return Err(args.err(format!("a layer name in `layers` has 1 to {MAX_NAME_LEN} bytes")));
            }
            if !layers.contains(&name) {
                layers.push(name);
            }
        }
        next.layers = layers;
    }
    if let Some(v) = args.number("lineSpace")? {
        if !LINE_SPACE_RANGE.contains(&v) {
            return Err(args.err(format!("`lineSpace` lies between {} and {}", LINE_SPACE_RANGE.start(), LINE_SPACE_RANGE.end())));
        }
        next.line_space = v;
    }
    if let Some(v) = args.number("tracking")? {
        if !TRACKING_RANGE.contains(&v) {
            return Err(args.err(format!("`tracking` lies between {} and {} units", TRACKING_RANGE.start(), TRACKING_RANGE.end())));
        }
        next.tracking = v;
    }
    let beam_y = height_arg(args, "beamY")?;
    if let Some(y) = beam_y {
        if app.session.active().is_none() {
            return Err(args.err("no font is open"));
        }
        if args.flag("beam")?.is_none() {
            next.beam = true;
        }
        // The height is the one Edit View's beam keeps; its own beam stays on or off.
        let keep = json!({"on": app.state.panels.edit_aids.beam, "y": y});
        let beam_args = Args::new("view.showMetricsBeam", &keep)?;
        super::edit_aids::run(app, "view.showMetricsBeam", &beam_args)?;
    }
    app.state.panels.space_center.display = next;
    Ok(display_json(app))
}

fn display_json(app: &FontcraftApp) -> Value {
    let mut out = serde_json::to_value(&app.state.panels.space_center.display).unwrap_or_else(|_| json!({}));
    if let Some(map) = out.as_object_mut() {
        map.insert("beamY".into(), shown_beam(app).map_or(Value::Null, |y| json!(y)));
    }
    out
}

fn cut_off_json(app: &FontcraftApp) -> Value {
    let cut = app.state.panels.space_center.cut_off;
    let shown = app.session.font().ok().zip(shown_master(app)).map(|(font, master)| band(font, &master, &cut));
    match shown {
        Some((bottom, top)) => json!({"on": cut.on, "bottom": bottom, "top": top}),
        None => json!({"on": cut.on, "bottom": cut.bottom, "top": cut.top}),
    }
}

fn summary(app: &FontcraftApp) -> Value {
    let sc = &app.state.panels.space_center;
    let (master, r) = match resolved(app) {
        Some((m, r)) => (Some(m), r),
        None => (None, Resolved::default()),
    };
    let glyphs: Vec<&str> = r.columns.iter().take(MAX_LISTED).map(|c| c.glyph.as_str()).collect();
    json!({
        "open": sc.open,
        "text": sc.text,
        "before": sc.before,
        "after": sc.after,
        "size": sc.size,
        "lineMode": sc.line_mode.name(),
        "master": master,
        "glyphs": glyphs,
        "count": r.columns.len(),
        "selected": sc.selected,
        "cutOff": cut_off_json(app),
        "suffix": sc.suffix,
        "display": display_json(app),
        "message": r.message,
    })
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    crate::dialogs::space_center::show(app, ui, pending);
}
