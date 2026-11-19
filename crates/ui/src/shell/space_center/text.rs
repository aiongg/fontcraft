//! Reading a Space Center text (`spaceCenter.inputSyntax`) and the matrix values (`spaceCenter.matrixEntry`).
//!
//! | Token | Gives |
//! |---|---|
//! | a character | the glyph that encodes it |
//! | `/name` | the glyph `name`; the name runs to the next `/`, space, backslash or line break, and one space after it is dropped |
//! | `/name@layer` | that glyph drawn from its layer named (or with the id) `layer`, else from the master named `layer` |
//! | `/?` | the glyph the glyph panels show |
//! | `/!` | the glyphs selected in Font View |
//! | `/:group` | every glyph whose left or right kerning group is `group`, in glyph order |
//! | `\n` (backslash, n) or a line break | a new line |
//!
//! The control-glyph fields are read the same way; their glyphs stand before and after every glyph of the text. The
//! preferred suffix then replaces each glyph by its variant with that suffix where the font has one. The matrix has
//! one column per glyph of the text (control glyphs have none).

use std::collections::BTreeMap;

use fontcraft_geom::expr::Expr;
use fontcraft_model::{Font, MAX_NAME_LEN, metric};
use fontcraft_spacing::{Field, SIDEBEARING_RANGE, WIDTH_RANGE, round_half_up};
use fontcraft_textlayout::{Charmap, Item, MAX_ITEMS};
use serde_json::{Value, json};

use crate::shell::params::quoted;

/// Longest text, and longest control-glyph field, in characters.
pub const MAX_TEXT_CHARS: usize = 10_000;
/// Most glyphs laid out, control glyphs included; longer texts are cut.
pub const MAX_GLYPHS_SHOWN: usize = MAX_ITEMS;

/// One row of the matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    /// Its id in `spaceCenter.setValue {row}`.
    pub fn id(self) -> &'static str {
        match self {
            Row::Width => "width",
            Row::Left => "left",
            Row::Right => "right",
        }
    }

    pub fn of_id(id: &str) -> Option<Row> {
        Row::ALL.into_iter().find(|r| r.id() == id)
    }

    /// The engine command that sets this row's value.
    pub fn setter(self) -> &'static str {
        match self {
            Row::Width => "spacing.setWidth",
            Row::Left => "spacing.setLeftSidebearing",
            Row::Right => "spacing.setRightSidebearing",
        }
    }
}

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
    /// The glyph the glyph panels show.
    pub current: Option<&'a str>,
    /// The Font View selection.
    pub selection: &'a [String],
}

/// What the window's fields hold: the text, the control glyphs and the preferred suffix.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub text: &'a str,
    pub before: &'a str,
    pub after: &'a str,
    /// With its leading dot.
    pub suffix: Option<&'a str>,
}

fn ends_name(c: char) -> bool {
    matches!(c, '/' | ' ' | '\\' | '\n' | '\r')
}

/// Reads a Space Center text into tokens, at most `limit` of them. Problems (a name that is too long, an unknown
/// group, a text cut at the limit) are collected in `problems`; the tokens read so far stay.
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
    let collect = |from: usize, to: usize| -> String { chars.get(from..to).unwrap_or_default().iter().collect() };
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
                let mut end = match at(i + 1) {
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
                        let group = collect(i + 2, end);
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
                        let word = collect(i + 1, end);
                        if word.is_empty() {
                            // A lone slash is the slash character; nothing after it is dropped.
                            push(&mut out, glyph_or_missing(map, '/'));
                            i = end.max(i + 1);
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

/// The glyph shown for `name` under the preferred suffix (with its dot): the variant when the font has it, else
/// `name`.
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

/// The layer id `typed` (after `@`) names for `glyph` in `master`: a layer of that master with that name or id, else
/// any layer of the glyph with that name or id, else the glyph's layer of the master named `typed`.
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

/// A Space Center text read for one master: the items to lay out, the matrix columns, the layers other than the
/// master's, and the first problem met.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub items: Vec<Item>,
    pub columns: Vec<Column>,
    /// Glyph name → layer id, for glyphs drawn from another layer than the master's. A glyph asked for in two layers
    /// is drawn from the last one named.
    pub layers: BTreeMap<String, String>,
    /// Glyphs shown as their preferred-suffix variant.
    pub replaced: usize,
    pub message: Option<String>,
}

/// Reads the window's text and control glyphs for `master`.
pub fn resolve(font: &Font, master: &str, input: &Input<'_>, cx: &Context<'_>) -> Resolved {
    let map = Charmap::new(font);
    let mut problems = Vec::new();
    let text = tokens(font, &map, input.text, cx, MAX_GLYPHS_SHOWN, &mut problems);
    let before = tokens(font, &map, input.before, cx, MAX_GLYPHS_SHOWN, &mut problems);
    let after = tokens(font, &map, input.after, cx, MAX_GLYPHS_SHOWN, &mut problems);
    let mut out = Resolved::default();
    let mut cut = false;
    let mut replaced = 0;
    let mut item = |out: &mut Resolved, token: &Token, problems: &mut Vec<String>| -> Option<usize> {
        if out.items.len() >= MAX_GLYPHS_SHOWN {
            cut = true;
            return None;
        }
        let at = out.items.len();
        match token {
            Token::Glyph { name, layer } => {
                let shown = preferred(font, name, input.suffix);
                if shown != *name {
                    replaced += 1;
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
    for token in &text {
        let Token::Glyph { .. } = token else {
            item(&mut out, token, &mut problems);
            continue;
        };
        for t in &before {
            item(&mut out, t, &mut problems);
        }
        if let Some(at) = item(&mut out, token, &mut problems) {
            out.columns.push(Column { glyph: String::new(), layer: String::new(), item: at });
        }
        for t in &after {
            item(&mut out, t, &mut problems);
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

/// The value of `row` for `glyph` in its layer `layer`: `None` for a glyph or layer the font lacks and for the
/// margins of an empty layer.
pub fn value(font: &Font, glyph: &str, layer: &str, row: Row) -> Option<f64> {
    let l = font.glyph(glyph)?.layers.iter().find(|l| l.id == layer)?;
    fontcraft_spacing::metric(font, l, row.field()).ok().flatten()
}

/// What a matrix cell holds once `typed` is entered in `master`: a number, or `=` and an expression whose names are
/// glyphs, each standing for that glyph's value in the same row and master (`=n`, `=e+10`, `=(n+o)/2`). The result
/// rounds halves up and must lie in the row's range.
pub fn entered_value(font: &Font, master: &str, row: Row, typed: &str) -> Result<f64, String> {
    let typed = typed.trim();
    let v = match typed.strip_prefix('=') {
        Some(source) => {
            let expr = Expr::parse(source).map_err(|e| e.to_string())?;
            let lookup = |name: &str| font.glyph(name).and_then(|g| g.layer(master)).and_then(|l| value(font, name, &l.id, row));
            if let Some(missing) = expr.names().into_iter().find(|n| lookup(n).is_none()) {
                return Err(match font.glyph(&missing) {
                    Some(_) => format!("`{}` has no {} value in this master", quoted(&missing), row.id()),
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
        return Err(format!("the {} must lie between {} and {}, not {v}", row.id(), range.start(), range.end()));
    }
    Ok(v)
}

/// The engine command a matrix entry runs: the row's setter with the evaluated number.
pub fn matrix_command(font: &Font, master: &str, column: &Column, row: Row, typed: &str) -> Result<(&'static str, Value), String> {
    let v = entered_value(font, master, row, typed)?;
    Ok((row.setter(), json!({"glyphs": [column.glyph], "layer": column.layer, "value": v})))
}

/// The x-height of `master`; half the em when it has none.
pub fn x_height(font: &Font, master: &str) -> f64 {
    let upm = f64::from(font.units_per_em);
    let x = font.masters.iter().find(|m| m.id == master).map_or(upm * 0.5, |m| font.metric_pos(m, metric::X_HEIGHT, upm * 0.5));
    if x.is_finite() { x } else { upm * 0.5 }
}
