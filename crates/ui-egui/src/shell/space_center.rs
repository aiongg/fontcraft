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
        "{open?: bool (default true), text?: string, before?: string, after?: string, size?: points (4 to 1000), lineMode?: \"multi\"|\"single\", select?: column index|null} show or hide the Space Center and set its text, its control glyphs (placed before and after every glyph of the text), its size, multi-line or single-line display and the selected glyph. Texts take characters, /name, /name@layer, /? (the glyph being edited), /! (the Font View selection), /:group (the members of a kerning group) and \\n (a new line); at most 10000 characters each → {open, text, before, after, size, lineMode, master, glyphs: [names of the text's glyphs, one per matrix column], count, selected, cutOff: {on, bottom, top}, suffix, message}",
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
];

/// Multi-line text that wraps at the window's width, or one line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineMode {
    #[default]
    Multi,
    Single,
}

impl LineMode {
    pub fn name(self) -> &'static str {
        match self {
            LineMode::Multi => "multi",
            LineMode::Single => "single",
        }
    }

    fn from_name(name: &str) -> Option<LineMode> {
        [LineMode::Multi, LineMode::Single].into_iter().find(|m| m.name() == name)
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

/// The cut-off band's bottom and top in `master`: the heights set, else the baseline and the
/// master's x-height.
pub fn band(font: &Font, master: &str, cut: &CutOff) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let x_height = font.masters.iter().find(|m| m.id == master).map_or(upm * 0.5, |m| font.metric_pos(m, metric::X_HEIGHT, upm * 0.5));
    let bottom = cut.bottom.unwrap_or(0.0);
    let top = cut.top.unwrap_or(if x_height.is_finite() { x_height } else { upm * 0.5 });
    (bottom.min(top), bottom.max(top))
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
                Some(name) => Some(LineMode::from_name(name).ok_or_else(|| args.err("`lineMode` must be multi or single"))?),
            };
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
        _ => Err(args.err("unknown command")),
    }
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
        "message": r.message,
    })
}

/// Draws this module's windows while they are open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    crate::dialogs::space_center::show(app, ui, pending);
}
