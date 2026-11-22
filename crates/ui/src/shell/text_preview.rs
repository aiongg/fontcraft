//! The Text Preview window's shell commands and view state: the text, the style and size it draws in, the layout
//! orientation, the writing directions and the find bar.
//!
//! | Command | Does |
//! |---|---|
//! | `window.textPreview {open?}` | shows or hides the Text Preview panel |
//! | `textPreview.setText {text}` | replaces the text; ranges with their own direction move with the characters around them |
//! | `textPreview.setStyle {instance?}` | draws a master, or an instance at a master's location; none: the current master |
//! | `textPreview.setFontSize {size}` | the size in points |
//! | `textPreview.togglePinned {on?}` | keeps the window in front |
//! | `textPreview.setLayoutOrientation {orientation}` | lines, or columns from right to left |
//! | `textPreview.setWritingDirection {scope, direction, range?}` | the direction of the whole text or of a range |
//! | `textPreview.find {open?, query?, replace?, options?}` | the find bar: search, step through matches, replace |
//!
//! Text positions are character indexes; a line break is one character. Shell commands reply with nothing:
//! `ui.inspect` reports the preview's state under `textPreview`. The layout, shaping and drawing are the projection's
//! (`app::models::text_preview`).

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_model::Font;
use fontcraft_textlayout::{MAX_PREVIEW_ITEMS, bidi};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::quoted;
use crate::app::FontCraftApp;
use crate::app::ViewState;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most characters the preview holds.
pub const MAX_CHARS: usize = MAX_PREVIEW_ITEMS;
/// The text field edits texts up to this many characters; longer ones change through `textPreview.setText` and
/// Find and Replace.
pub const FIELD_LIMIT: usize = 100_000;
/// Allowed font sizes, in points.
pub const SIZE_RANGE: RangeInclusive<f64> = 4.0..=1_000.0;
/// The font size of a new preview, in points.
pub const DEFAULT_SIZE: f64 = 36.0;
/// Most runs with their own writing direction.
pub const MAX_RUNS: usize = bidi::MAX_EMBEDDINGS;
/// Longest search or replacement, in characters.
pub const MAX_QUERY_CHARS: usize = 1_000;
/// Match ranges `ui.inspect` lists.
const MAX_LISTED: usize = 1_000;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.textPreview",
        label: "Text Preview",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true)} show or hide the Text Preview window. ui.inspect reports the preview under textPreview",
    },
    UiCommand {
        id: "textPreview.setText",
        label: "Set Preview Text",
        menu: &[],
        shortcut: None,
        params: "{text: string} replace the preview text (at most 1,000,000 characters; \\r\\n and \\r become \\n). Directions set on parts of the text move with the characters around them",
    },
    UiCommand {
        id: "textPreview.setStyle",
        label: "Preview Font Style",
        menu: &[],
        shortcut: None,
        params: "{instance?: name | index into styles} the style the preview draws: a master, or an instance at a master's location. Without instance the preview follows the current master",
    },
    UiCommand {
        id: "textPreview.setFontSize",
        label: "Preview Font Size",
        menu: &[],
        shortcut: None,
        params: "{size: number of points, 4 to 1000} the size the preview draws at",
    },
    UiCommand {
        id: "textPreview.togglePinned",
        label: "Pin Text Preview",
        menu: &[],
        shortcut: None,
        params: "{on?: bool} keep the Text Preview window above the other windows",
    },
    UiCommand {
        id: "textPreview.setLayoutOrientation",
        label: "Layout Orientation",
        menu: &[],
        shortcut: None,
        params: "{orientation: \"horizontal\" | \"vertical\"} lay the preview out in lines, or in columns from right to left",
    },
    UiCommand {
        id: "textPreview.setWritingDirection",
        label: "Writing Direction",
        menu: &[],
        shortcut: None,
        params: "{scope: \"text\" | \"selection\", direction: \"default\" | \"ltr\" | \"rtl\", range?: [start, end] (character indexes, required for selection)} set the base direction of the whole text (default: from its first letter) or of a range of it (default: as the text)",
    },
    UiCommand {
        id: "textPreview.find",
        label: "Find…",
        menu: &[],
        shortcut: None,
        params: "{open?: bool (default true), query?: string, replace?: string, options?: {ignoreCase?: bool (default true), match?: \"contains\" | \"startsWith\" | \"fullWord\", index?: number}} show the find bar and search the preview text; index picks the current match. With replace, replace every match, or only match index when it is given. ui.inspect reports find.matches, find.ranges (the first 1,000) and find.replaced",
    },
];

/// How the preview arranges its text.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Orientation {
    pub const ALL: [Orientation; 2] = [Orientation::Horizontal, Orientation::Vertical];

    pub fn name(self) -> &'static str {
        match self {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        }
    }

    fn from_name(name: &str) -> Option<Orientation> {
        Orientation::ALL.into_iter().find(|o| o.name() == name)
    }
}

/// A base writing direction. `Default` takes it from the text (for the whole text) or from the text around it (for
/// a range).
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BaseDirection {
    #[default]
    Default,
    Ltr,
    Rtl,
}

impl BaseDirection {
    pub const ALL: [BaseDirection; 3] = [BaseDirection::Default, BaseDirection::Ltr, BaseDirection::Rtl];

    pub fn name(self) -> &'static str {
        match self {
            BaseDirection::Default => "default",
            BaseDirection::Ltr => "ltr",
            BaseDirection::Rtl => "rtl",
        }
    }

    fn from_name(name: &str) -> Option<BaseDirection> {
        BaseDirection::ALL.into_iter().find(|d| d.name() == name)
    }
}

/// A range of the text with its own direction (`Ltr` or `Rtl`), in character indexes.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub direction: BaseDirection,
}

/// What a search matches.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Matching {
    #[default]
    Contains,
    StartsWith,
    FullWord,
}

impl Matching {
    pub const ALL: [Matching; 3] = [Matching::Contains, Matching::StartsWith, Matching::FullWord];

    pub fn name(self) -> &'static str {
        match self {
            Matching::Contains => "contains",
            Matching::StartsWith => "startsWith",
            Matching::FullWord => "fullWord",
        }
    }

    fn from_name(name: &str) -> Option<Matching> {
        Matching::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// The find bar.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub ignore_case: bool,
    pub matching: Matching,
    /// The match the find bar points at.
    pub current: Option<usize>,
    /// Matches the last replace replaced.
    pub replaced: usize,
}

impl Default for FindState {
    fn default() -> Self {
        Self { open: false, query: String::new(), ignore_case: true, matching: Matching::Contains, current: None, replaced: 0 }
    }
}

/// The window's view state.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPreviewState {
    pub text: Arc<str>,
    /// Characters in `text`.
    pub length: usize,
    /// Moves with every change of the text.
    pub revision: u64,
    /// The style drawn, by name; `None`: the current master.
    pub style: Option<String>,
    /// Points per em.
    pub size: f64,
    pub pinned: bool,
    pub orientation: Orientation,
    pub direction: BaseDirection,
    /// Ranges with their own direction, sorted, not overlapping.
    pub runs: Vec<Run>,
    pub find: FindState,
    /// The text area as the view last reported it: the line length in points (0 until it does) and the lines in
    /// view.
    pub area: Area,
}

/// The text area as the view reports it.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Area {
    /// The length of a line (a column's height, in vertical text), in points.
    pub span: u32,
    /// The first line in view and the one after the last.
    pub first: usize,
    pub last: usize,
    /// The window's pixels per point, as `f32` bits: the size colour glyphs are drawn at.
    pub ppp: u32,
    /// The text colour as straight sRGB RGBA: the colour of `CPAL` palette index 65,535.
    pub ink: [u8; 4],
}

impl Default for TextPreviewState {
    fn default() -> Self {
        Self {
            text: Arc::from(""),
            length: 0,
            revision: 0,
            style: None,
            size: DEFAULT_SIZE,
            pinned: false,
            orientation: Orientation::Horizontal,
            direction: BaseDirection::Default,
            runs: Vec::new(),
            find: FindState::default(),
            area: Area::default(),
        }
    }
}

impl TextPreviewState {
    /// Replaces the text (already normalised and within the limit); runs move with the characters around them.
    fn set_text(&mut self, text: String) {
        let new_len = text.chars().count();
        self.runs = shift_runs(&self.text, self.length, &text, new_len, &self.runs);
        self.text = Arc::from(text);
        self.length = new_len;
        self.revision = self.revision.wrapping_add(1);
        self.find.current = None;
    }
}

/// One entry of the style pop-up.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StyleEntry {
    pub name: String,
    /// The master whose layers it draws.
    pub master: String,
}

/// The styles the preview can draw: every master, then every instance at a master's location.
pub fn styles(font: &Font) -> Vec<StyleEntry> {
    let mut out: Vec<StyleEntry> = font.masters.iter().map(|m| StyleEntry { name: m.name.clone(), master: m.id.clone() }).collect();
    for instance in font.instances.iter() {
        if instance.kind.as_deref() == Some("variable") {
            continue;
        }
        if let Some(m) = font.masters.iter().find(|m| m.location == instance.location) {
            out.push(StyleEntry { name: instance.name.clone(), master: m.id.clone() });
        }
    }
    out
}

/// The master the preview draws: the chosen style's, else the current master.
pub fn shown_master(font: &Font, view: &ViewState) -> Option<String> {
    if let Some(name) = &view.text_preview.style
        && let Some(entry) = styles(font).into_iter().find(|s| s.name == *name)
    {
        return Some(entry.master);
    }
    super::masters::current_master(font, &view.layers).map(|m| m.id.clone())
}

/// Whether the Text Preview panel shows.
pub fn shows(view: &ViewState) -> bool {
    view.workspace.layout.panel_state(&PanelId::TextPreview) != craft::behavior::dock::PanelState::Hidden
}

/// `n` with a comma between groups of three digits.
pub fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `s` with `\r\n` and `\r` as `\n`, when it holds at most [`MAX_CHARS`] characters.
fn normalised(s: &str) -> Option<String> {
    // A character takes at most four bytes, so a longer string cannot fit.
    if s.len() > MAX_CHARS.saturating_mul(4) {
        return None;
    }
    let out = s.replace("\r\n", "\n").replace('\r', "\n");
    (out.chars().count() <= MAX_CHARS).then_some(out)
}

/// Runs moved through an edit from `old` to `new`: the characters before the first difference and after the last one
/// keep their runs; a run reaching into the changed part ends (or starts) at its edge.
fn shift_runs(old: &str, old_len: usize, new: &str, new_len: usize, runs: &[Run]) -> Vec<Run> {
    if runs.is_empty() {
        return Vec::new();
    }
    let prefix = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let room = old_len.min(new_len).saturating_sub(prefix);
    let suffix = old.chars().rev().zip(new.chars().rev()).take(room).take_while(|(a, b)| a == b).count();
    let old_tail = old_len.saturating_sub(suffix);
    let new_tail = new_len.saturating_sub(suffix);
    // At an insertion point (`prefix == old_tail`) a run that starts there moves with the text after it, and a run
    // that ends there keeps its end.
    let map = |pos: usize, end: bool| {
        if (end && pos <= prefix) || (!end && pos < old_tail && pos <= prefix) {
            pos
        } else if pos >= old_tail {
            (pos - old_tail) + new_tail
        } else if end {
            new_tail
        } else {
            prefix
        }
    };
    runs.iter()
        .map(|r| Run { start: map(r.start, false), end: map(r.end, true), direction: r.direction })
        .filter(|r| r.start < r.end && r.end <= new_len)
        .collect()
}

/// `runs` with `start..end` set to `direction` (`Default` clears it).
fn set_run(runs: &[Run], start: usize, end: usize, direction: BaseDirection) -> Vec<Run> {
    let mut out = Vec::with_capacity(runs.len() + 2);
    for r in runs {
        if r.end <= start || r.start >= end {
            out.push(*r);
            continue;
        }
        if r.start < start {
            out.push(Run { start: r.start, end: start, direction: r.direction });
        }
        if r.end > end {
            out.push(Run { start: end, end: r.end, direction: r.direction });
        }
    }
    if direction != BaseDirection::Default && start < end {
        out.push(Run { start, end, direction });
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Whether the text runs right to left: as set, or by default from its first letter.
pub fn base_is_rtl(text: &str, direction: BaseDirection) -> bool {
    match direction {
        BaseDirection::Ltr => false,
        BaseDirection::Rtl => true,
        BaseDirection::Default => text.chars().find(|c| c.is_alphabetic()).is_some_and(bidi::is_strong_rtl),
    }
}

/// The start of every match of `query` in `text`, without overlaps, as character indexes.
pub fn find_matches(text: &str, query: &str, ignore_case: bool, matching: Matching) -> Vec<usize> {
    let fold = |c: char| if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c };
    let needle: Vec<char> = query.chars().map(fold).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.chars().map(fold).collect();
    // Knuth–Morris–Pratt, so a hostile query costs no more than the text.
    let mut fail = vec![0usize; needle.len()];
    let mut k = 0;
    for i in 1..needle.len() {
        while k > 0 && needle.get(i) != needle.get(k) {
            k = fail.get(k - 1).copied().unwrap_or(0);
        }
        if needle.get(i) == needle.get(k) {
            k += 1;
        }
        if let Some(f) = fail.get_mut(i) {
            *f = k;
        }
    }
    let word = |i: Option<usize>| i.and_then(|i| hay.get(i)).is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = Vec::new();
    let mut k = 0;
    for (i, c) in hay.iter().enumerate() {
        while k > 0 && needle.get(k) != Some(c) {
            k = fail.get(k - 1).copied().unwrap_or(0);
        }
        if needle.get(k) == Some(c) {
            k += 1;
        }
        if k == needle.len() {
            let start = i + 1 - k;
            let starts = !word(start.checked_sub(1));
            let ends = !word(Some(i + 1));
            let ok = match matching {
                Matching::Contains => true,
                Matching::StartsWith => starts,
                Matching::FullWord => starts && ends,
            };
            if ok {
                out.push(start);
                k = 0;
            } else {
                k = fail.get(k - 1).copied().unwrap_or(0);
            }
        }
    }
    out
}

/// `text` with the matches at `starts` (each `len` characters, not overlapping) replaced by `with`, or `None` when
/// the result would hold more than `limit` characters. The length is worked out before anything is built: a short
/// text and replacement can multiply past memory.
fn replaced(text: &str, starts: &[usize], len: usize, with: &str, limit: usize) -> Option<String> {
    let removed = starts.len().saturating_mul(len);
    let added = starts.len().saturating_mul(with.chars().count());
    if text.chars().count().saturating_sub(removed).saturating_add(added) > limit {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut next = starts.iter().peekable();
    let mut skip = 0;
    for (i, c) in text.chars().enumerate() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if next.next_if(|s| **s == i).is_some() {
            out.push_str(with);
            skip = len.saturating_sub(1);
            continue;
        }
        out.push(c);
    }
    Some(out)
}

fn string<'a>(p: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn flag(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = command.params.as_object().cloned().unwrap_or_default();
    let p = &params;
    match command.id.as_str() {
        "window.textPreview" => {
            let open = flag(p, "open")?.unwrap_or(true);
            crate::shell::dock::show_window(&mut app.view.workspace, PanelId::TextPreview, open, app.view.canvas.rect);
        }
        "textPreview.setText" => {
            let text = string(p, "text")?.ok_or("`text` is required")?;
            let text = normalised(text).ok_or_else(|| format!("the text holds at most {MAX_CHARS} characters"))?;
            app.view.text_preview.set_text(text);
        }
        "textPreview.setStyle" => {
            let list = app.session.font().map(styles).unwrap_or_default();
            let style = match p.get("instance") {
                None | Some(Value::Null) => None,
                Some(Value::String(name)) => Some(
                    list.iter().find(|s| s.name == *name).map(|s| s.name.clone()).ok_or_else(|| format!("no style is named `{}`", quoted(name)))?,
                ),
                Some(v) => {
                    let i = v.as_u64().ok_or("`instance` must be a style name or an index")?;
                    let entry = usize::try_from(i).ok().and_then(|i| list.get(i));
                    Some(entry.map(|s| s.name.clone()).ok_or_else(|| format!("there are {} styles, no style {i}", list.len()))?)
                }
            };
            app.view.text_preview.style = style;
        }
        "textPreview.setFontSize" => {
            let size = p.get("size").and_then(Value::as_f64).ok_or("`size` is required and must be a number")?;
            if !SIZE_RANGE.contains(&size) {
                return Err(format!("`size` lies between {} and {} points", SIZE_RANGE.start(), SIZE_RANGE.end()));
            }
            app.view.text_preview.size = size;
        }
        "textPreview.togglePinned" => {
            let tp = &mut app.view.text_preview;
            tp.pinned = flag(p, "on")?.unwrap_or(!tp.pinned);
        }
        "textPreview.setLayoutOrientation" => {
            let name = string(p, "orientation")?.ok_or("`orientation` is required")?;
            let orientation = Orientation::from_name(name).ok_or("`orientation` must be horizontal or vertical")?;
            app.view.text_preview.orientation = orientation;
        }
        "textPreview.setWritingDirection" => writing_direction(&mut app.view.text_preview, p)?,
        "textPreview.find" => find(&mut app.view.text_preview, p)?,
        other => return Err(format!("no Text Preview command `{other}`")),
    }
    Ok(())
}

fn writing_direction(tp: &mut TextPreviewState, p: &Map<String, Value>) -> Result<(), String> {
    let scope = string(p, "scope")?.ok_or("`scope` is required")?;
    let name = string(p, "direction")?.ok_or("`direction` is required")?;
    let direction = BaseDirection::from_name(name).ok_or("`direction` must be default, ltr or rtl")?;
    match scope {
        "text" => tp.direction = direction,
        "selection" => {
            let range = p.get("range").and_then(Value::as_array).ok_or("`range` is required for the selection")?;
            let whole = |v: Option<&Value>| v.and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).filter(|v| *v <= tp.length);
            let (Some(start), Some(end), 2) = (whole(range.first()), whole(range.get(1)), range.len()) else {
                return Err(format!("`range` must be two character indexes from 0 to {}", tp.length));
            };
            if start > end {
                return Err("`range` must not end before it starts".into());
            }
            let runs = set_run(&tp.runs, start, end, direction);
            if runs.len() > MAX_RUNS {
                return Err(format!("the text holds at most {MAX_RUNS} ranges with their own direction"));
            }
            tp.runs = runs;
        }
        _ => return Err("`scope` must be text or selection".into()),
    }
    Ok(())
}

fn find(tp: &mut TextPreviewState, p: &Map<String, Value>) -> Result<(), String> {
    let open = flag(p, "open")?.unwrap_or(true);
    let short = |key: &str| -> Result<Option<String>, String> {
        match string(p, key)? {
            Some(s) if s.chars().count() > MAX_QUERY_CHARS => Err(format!("`{key}` holds at most {MAX_QUERY_CHARS} characters")),
            other => Ok(other.map(str::to_string)),
        }
    };
    let query = short("query")?;
    let replace = short("replace")?;
    let empty = Map::new();
    let options = match p.get("options") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return Err("`options` must be an object".into()),
    };
    let ignore_case = match options.get("ignoreCase").filter(|v| !v.is_null()) {
        None => tp.find.ignore_case,
        Some(v) => v.as_bool().ok_or("`options.ignoreCase` must be true or false")?,
    };
    let matching = match options.get("match").filter(|v| !v.is_null()) {
        None => tp.find.matching,
        Some(v) => v.as_str().and_then(Matching::from_name).ok_or("`options.match` must be contains, startsWith or fullWord")?,
    };
    let index = match options.get("index").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`options.index` must be a whole number")?),
    };
    let query = query.unwrap_or_else(|| tp.find.query.clone());
    let qlen = query.chars().count();
    let starts = find_matches(&tp.text, &query, ignore_case, matching);
    if let Some(i) = index
        && i >= starts.len()
    {
        return Err(format!("there are {} matches, no match {i}", starts.len()));
    }
    let mut replaced_count = 0;
    let mut new_text = None;
    if let Some(with) = &replace {
        let chosen: Vec<usize> = match index {
            Some(i) => starts.get(i).copied().into_iter().collect(),
            None => starts.clone(),
        };
        if !chosen.is_empty() {
            let text = replaced(&tp.text, &chosen, qlen, with, MAX_CHARS)
                .ok_or_else(|| format!("replacing would make the text longer than {MAX_CHARS} characters"))?;
            replaced_count = chosen.len();
            new_text = Some(text);
        }
    }
    if let Some(text) = new_text {
        tp.set_text(text);
    }
    let any = if replace.is_some() { !find_matches(&tp.text, &query, ignore_case, matching).is_empty() } else { !starts.is_empty() };
    tp.find = FindState {
        open,
        query,
        ignore_case,
        matching,
        current: match (&replace, index) {
            (Some(_), _) => None,
            (None, Some(i)) => Some(i),
            (None, None) => any.then_some(0),
        },
        replaced: if replace.is_some() { replaced_count } else { tp.find.replaced },
    };
    Ok(())
}

/// The preview's state under `textPreview` in `ui.inspect`: what the old commands replied, the find results, and
/// what the last projection drew (`drawn`).
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let tp = &app.view.text_preview;
    let font = app.session.font().ok();
    let styles: Vec<String> = font.map(styles).unwrap_or_default().into_iter().map(|s| s.name).collect();
    let starts = find_matches(&tp.text, &tp.find.query, tp.find.ignore_case, tp.find.matching);
    let qlen = tp.find.query.chars().count();
    let model = &app.models.text_preview.view;
    let page = model.page.as_deref();
    state.insert(
        "textPreview".into(),
        json!({
            "open": shows(&app.view),
            "length": tp.length,
            "style": tp.style,
            "styles": styles,
            "master": font.and_then(|f| shown_master(f, &app.view)),
            "size": tp.size,
            "pinned": tp.pinned,
            "orientation": tp.orientation.name(),
            "direction": tp.direction.name(),
            "rtl": base_is_rtl(&tp.text, tp.direction),
            "runs": tp.runs.iter().map(|r| json!({"start": r.start, "end": r.end, "direction": r.direction.name()})).collect::<Vec<_>>(),
            "find": {
                "open": tp.find.open,
                "query": tp.find.query,
                "ignoreCase": tp.find.ignore_case,
                "match": tp.find.matching.name(),
                "current": tp.find.current,
                "matches": starts.len(),
                "ranges": starts.iter().take(MAX_LISTED).map(|s| [*s, s + qlen]).collect::<Vec<_>>(),
                "replaced": tp.find.replaced,
            },
            "drawn": {
                "lines": page.map_or(0, |p| p.lines),
                "shaped": page.map_or(0, |p| p.shaped),
                "glyphs": page.map_or(0, |p| p.glyphs.len()),
                "message": model.message,
                "building": model.building,
            },
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_follow_the_characters_around_an_edit() {
        let runs = [Run { start: 2, end: 4, direction: BaseDirection::Rtl }, Run { start: 6, end: 8, direction: BaseDirection::Ltr }];
        // "abCDefGH" → "abXCDefGH": both runs move one to the right.
        let moved = shift_runs("abCDefGH", 8, "abXCDefGH", 9, &runs);
        assert_eq!(moved.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(3, 5), (7, 9)]);
        // Setting a range inside a run splits it.
        let split = set_run(&runs, 3, 4, BaseDirection::Default);
        assert_eq!(split.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(2, 3), (6, 8)]);
    }

    #[test]
    fn find_matches_whole_words_and_ignores_case() {
        assert_eq!(find_matches("Hat hat that", "hat", true, Matching::Contains), [0, 4, 9]);
        assert_eq!(find_matches("Hat hat that", "hat", false, Matching::Contains), [4, 9]);
        assert_eq!(find_matches("Hat hat that", "hat", true, Matching::FullWord), [0, 4]);
        assert_eq!(find_matches("aaaa", "aa", true, Matching::Contains), [0, 2]);
        assert_eq!(replaced("Hat hat that", &[0, 4], 3, "cap", 12).as_deref(), Some("cap cap that"));
        assert_eq!(replaced("Hat hat that", &[0, 4], 3, "caps", 13), None);
    }

    #[test]
    fn replace_all_refuses_a_result_past_the_limit_before_building_it() {
        // A full-length text with every character replaced by the longest replacement would be a billion
        // characters; the refusal must come from arithmetic alone.
        let text = "a".repeat(MAX_CHARS);
        let starts: Vec<usize> = (0..MAX_CHARS).collect();
        let with = "b".repeat(MAX_QUERY_CHARS);
        let started = std::time::Instant::now();
        assert_eq!(replaced(&text, &starts, 1, &with, MAX_CHARS), None);
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "refused without building: {:?}", started.elapsed());
    }
}

#[cfg(test)]
#[path = "text_preview_tests.rs"]
mod app_tests;
