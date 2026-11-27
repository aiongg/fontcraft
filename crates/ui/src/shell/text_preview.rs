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
//! | `textPreview.insertPattern {pattern, at?}` | puts a pattern token (any space, digit or letter, a line end) into the search |
//! | `view.previewWaterfall {on?, sizes?, useOpticalSize?}` | the waterfall: one line repeated at a list of sizes |
//! | `textPreview.showExportedFonts {source, folder?, font?}` | the open document, the last export's fonts, or a folder's |
//!
//! The search holds plain characters and pattern tokens: `\s` any space (white space other than a line break), `\d`
//! any digit, `\l` any letter, `\n` a line end, and `\\` a backslash. Each token matches one character.
//!
//! Text positions are character indexes; a line break is one character. Shell commands reply with nothing:
//! `ui.inspect` reports the preview's state under `textPreview`. The layout, shaping and drawing are the projection's
//! (`app::models::text_preview`).

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;
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
/// Most font files the preview lists from the last export or a folder.
pub const MAX_FONT_FILES: usize = 64;
/// Longest folder path `textPreview.showExportedFonts` takes, in bytes.
const MAX_PATH_BYTES: usize = 4096;
/// Most sizes a waterfall shows.
pub const MAX_WATERFALL_SIZES: usize = 32;
/// The sizes of a new waterfall, in points.
pub const DEFAULT_WATERFALL: [f64; 15] = [72.0, 64.0, 48.0, 36.0, 30.0, 24.0, 18.0, 14.0, 12.0, 10.0, 9.0, 8.0, 6.0, 5.0, 4.0];

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
        params: "{open?: bool (default true), query?: string, replace?: string, options?: {ignoreCase?: bool (default true), match?: \"contains\" | \"startsWith\" | \"fullWord\", index?: number}} show the find bar and search the preview text; index picks the current match. With replace, replace every match, or only match index when it is given. The query may hold the pattern tokens \\s (any space), \\d (any digit), \\l (any letter), \\n (a line end) and \\\\ (a backslash). ui.inspect reports find.matches, find.ranges (the first 1,000) and find.replaced",
    },
    UiCommand {
        id: "textPreview.insertPattern",
        label: "Insert Pattern",
        menu: &[],
        shortcut: None,
        params: "{pattern: \"space\" | \"digit\" | \"letter\" | \"lineEnd\", at?: number (a character index into the query; default its end)} put a pattern token into the find bar's search and search again; the find bar opens",
    },
    UiCommand {
        id: "textPreview.showExportedFonts",
        label: "Preview Exported Fonts",
        menu: &[],
        shortcut: None,
        params: "{source: \"document\" | \"lastExport\" | \"folder\", folder?: path (required for folder), font?: file name} what the preview draws: the open document, the font files the last export of this session wrote, or the font files (.ttf, .otf, .woff, .woff2, .ttc, .otc; at most 64) in a folder. Files are read in the background, again when they change on disk (looked at once a second), and laid out and shaped from their own tables; font picks one by file name, else the first is drawn. ui.inspect reports textPreview.source and textPreview.exported",
    },
    UiCommand {
        id: "view.previewWaterfall",
        label: "Preview Waterfall",
        menu: &["View"],
        shortcut: None,
        params: "{on?: bool (default: toggle), sizes?: [points, 4 to 1000] (at most 32), useOpticalSize?: bool} show the Text Preview as a waterfall: the Edit View tab's text, or Font View's selection while Font View shows, once per size. With useOpticalSize, each line draws in the master whose optical-size axis value lies nearest the line's size",
    },
];

/// A pattern token of the find bar: it matches one character of a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pattern {
    /// White space other than a line break.
    Space,
    Digit,
    Letter,
    LineEnd,
}

impl Pattern {
    pub const ALL: [Pattern; 4] = [Pattern::Space, Pattern::Digit, Pattern::Letter, Pattern::LineEnd];

    pub fn name(self) -> &'static str {
        match self {
            Pattern::Space => "space",
            Pattern::Digit => "digit",
            Pattern::Letter => "letter",
            Pattern::LineEnd => "lineEnd",
        }
    }

    fn from_name(name: &str) -> Option<Pattern> {
        Pattern::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The token the search holds.
    pub fn token(self) -> &'static str {
        match self {
            Pattern::Space => "\\s",
            Pattern::Digit => "\\d",
            Pattern::Letter => "\\l",
            Pattern::LineEnd => "\\n",
        }
    }

    fn from_letter(c: char) -> Option<Pattern> {
        match c {
            's' => Some(Pattern::Space),
            'd' => Some(Pattern::Digit),
            'l' => Some(Pattern::Letter),
            'n' => Some(Pattern::LineEnd),
            _ => None,
        }
    }

    fn matches(self, c: char) -> bool {
        match self {
            Pattern::Space => c.is_whitespace() && c != '\n',
            Pattern::Digit => c.is_numeric(),
            Pattern::Letter => c.is_alphabetic(),
            Pattern::LineEnd => c == '\n',
        }
    }
}

/// One element of a search: a character, or a pattern token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Element {
    Char(char),
    Pattern(Pattern),
}

/// The elements of `query`: `\\` is a backslash, `\` before `s`, `d`, `l` or `n` a pattern token, and any other
/// backslash itself.
fn elements(query: &str) -> Vec<Element> {
    let mut out = Vec::new();
    let mut chars = query.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(Element::Char(c));
            continue;
        }
        let next = chars.peek().copied();
        let element = match next {
            Some('\\') => Some(Element::Char('\\')),
            Some(letter) => Pattern::from_letter(letter).map(Element::Pattern),
            None => None,
        };
        match element {
            Some(e) => {
                chars.next();
                out.push(e);
            }
            None => out.push(Element::Char('\\')),
        }
    }
    out
}

/// The characters one match of `query` spans: each pattern token matches one.
pub fn match_len(query: &str) -> usize {
    elements(query).len()
}

/// Where the preview's font comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontSource {
    /// The open document.
    #[default]
    Document,
    /// The font files the last export of this session wrote.
    LastExport,
    /// The font files in a folder.
    Folder(PathBuf),
}

impl FontSource {
    pub fn name(&self) -> &'static str {
        match self {
            FontSource::Document => "document",
            FontSource::LastExport => "lastExport",
            FontSource::Folder(_) => "folder",
        }
    }
}

/// Whether `path` names a compiled font file the preview reads, by its extension.
pub fn is_font_file(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("ttf" | "otf" | "woff" | "woff2" | "ttc" | "otc"))
}

/// Keeps the font files an export command wrote (its reply's `fonts[].files[].path` and `collections[].path`) as
/// the last export, when it wrote any.
pub fn note_export(tp: &mut TextPreviewState, reply: &serde_json::Value) {
    let paths = |v: &Value| v.get("path").and_then(Value::as_str).map(PathBuf::from);
    let fonts = reply.get("fonts").and_then(Value::as_array).into_iter().flatten();
    let files = fonts.flat_map(|f| f.get("files").and_then(Value::as_array).into_iter().flatten()).filter_map(paths);
    let collections = reply.get("collections").and_then(Value::as_array).into_iter().flatten().filter_map(paths);
    let written: Vec<PathBuf> = files.chain(collections).filter(|p| is_font_file(p)).take(MAX_FONT_FILES).collect();
    if !written.is_empty() {
        tp.last_export = written;
    }
}

/// A waterfall: one line repeated at each size.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct Waterfall {
    pub on: bool,
    /// Points, largest first as given; each within [`SIZE_RANGE`], at most [`MAX_WATERFALL_SIZES`].
    pub sizes: Vec<f64>,
    /// Each line draws in the master whose optical-size value lies nearest its size, when the font has the axis.
    pub optical: bool,
}

impl Default for Waterfall {
    fn default() -> Self {
        Self { on: false, sizes: DEFAULT_WATERFALL.to_vec(), optical: true }
    }
}

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
    pub waterfall: Waterfall,
    pub source: FontSource,
    /// The file the preview draws when the source lists several, by file name; `None`: the first.
    pub exported_font: Option<String>,
    /// The font files the last export of this session wrote.
    pub last_export: Vec<PathBuf>,
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
            waterfall: Waterfall::default(),
            source: FontSource::Document,
            exported_font: None,
            last_export: Vec::new(),
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
    view.workspace.layout().panel_state(&PanelId::TextPreview) != craft::behavior::dock::PanelState::Hidden
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

/// The start of every match of `query` in `text`, without overlaps, as character indexes. The query's pattern tokens
/// match one character of their kind.
pub fn find_matches(text: &str, query: &str, ignore_case: bool, matching: Matching) -> Vec<usize> {
    let fold = |c: char| if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c };
    let needle: Vec<Element> = elements(query)
        .into_iter()
        .map(|e| match e {
            Element::Char(c) => Element::Char(fold(c)),
            other => other,
        })
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.chars().map(fold).collect();
    let word = |i: Option<usize>| i.and_then(|i| hay.get(i)).is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = Vec::new();
    // The end of the last match kept: matches do not overlap, and the leftmost one wins.
    let mut free = 0;
    for start in occurrences(&hay, &needle) {
        if start < free {
            continue;
        }
        let end = start + needle.len();
        let starts = !word(start.checked_sub(1));
        let ends = !word(Some(end));
        let ok = match matching {
            Matching::Contains => true,
            Matching::StartsWith => starts,
            Matching::FullWord => starts && ends,
        };
        if ok {
            out.push(start);
            free = end;
        }
    }
    out
}

/// The start of every occurrence of `needle` in `hay`, overlapping ones included, in order. Bit-parallel
/// (shift-and), so a hostile query costs the text's length times the query's length over 64, at most.
fn occurrences(hay: &[char], needle: &[Element]) -> Vec<usize> {
    let Some(last) = needle.len().checked_sub(1) else { return Vec::new() };
    let words = needle.len().div_ceil(64);
    // For each character and each pattern: the bits of the needle's elements it matches.
    let mut literal: HashMap<char, Vec<u64>> = HashMap::new();
    let mut patterns: Vec<(Pattern, Vec<u64>)> = Vec::new();
    for (i, e) in needle.iter().enumerate() {
        let mask = match e {
            Element::Char(c) => Some(literal.entry(*c).or_insert_with(|| vec![0; words])),
            Element::Pattern(p) => {
                let at = match patterns.iter().position(|(q, _)| q == p) {
                    Some(at) => at,
                    None => {
                        patterns.push((*p, vec![0; words]));
                        patterns.len() - 1
                    }
                };
                patterns.get_mut(at).map(|(_, m)| m)
            }
        };
        if let Some(word) = mask.and_then(|m| m.get_mut(i / 64)) {
            *word |= 1 << (i % 64);
        }
    }
    let (last_word, last_bit) = (last / 64, 1u64 << (last % 64));
    let mut state = vec![0u64; words];
    let mut mask = vec![0u64; words];
    let mut out = Vec::new();
    for (i, c) in hay.iter().enumerate() {
        mask.iter_mut().for_each(|m| *m = 0);
        let sources = literal.get(c).into_iter().chain(patterns.iter().filter(|(p, _)| p.matches(*c)).map(|(_, m)| m));
        for source in sources {
            for (m, s) in mask.iter_mut().zip(source) {
                *m |= s;
            }
        }
        // Every partial match moves on one element, and a new one starts here.
        let mut carry = 1u64;
        for (s, m) in state.iter_mut().zip(&mask) {
            let next = *s >> 63;
            *s = ((*s << 1) | carry) & m;
            carry = next;
        }
        if state.get(last_word).is_some_and(|s| s & last_bit != 0) {
            out.push((i + 1).saturating_sub(needle.len()));
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
        "textPreview.insertPattern" => insert_pattern(&mut app.view.text_preview, p)?,
        "textPreview.showExportedFonts" => {
            let source = match string(p, "source")?.ok_or("`source` is required")? {
                "document" => FontSource::Document,
                "lastExport" => FontSource::LastExport,
                "folder" => {
                    let folder = string(p, "folder")?.filter(|f| !f.is_empty()).ok_or("`folder` is required for the folder source")?;
                    if folder.len() > MAX_PATH_BYTES {
                        return Err(format!("`folder` holds at most {MAX_PATH_BYTES} bytes"));
                    }
                    FontSource::Folder(PathBuf::from(folder))
                }
                _ => return Err("`source` must be document, lastExport or folder".into()),
            };
            let font = string(p, "font")?.map(str::to_string);
            if font.as_ref().is_some_and(|f| f.len() > MAX_PATH_BYTES) {
                return Err(format!("`font` holds at most {MAX_PATH_BYTES} bytes"));
            }
            let tp = &mut app.view.text_preview;
            tp.source = source;
            tp.exported_font = font;
        }
        "view.previewWaterfall" => {
            let tp = &mut app.view.text_preview;
            let waterfall = waterfall(&tp.waterfall, p)?;
            let open = waterfall.on;
            tp.waterfall = waterfall;
            if open {
                crate::shell::dock::show_window(&mut app.view.workspace, PanelId::TextPreview, true, app.view.canvas.rect);
            }
        }
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

/// `textPreview.insertPattern`: the token goes into the search at `at` (a character index; default its end), and the
/// search runs again with the find bar open.
fn insert_pattern(tp: &mut TextPreviewState, p: &Map<String, Value>) -> Result<(), String> {
    let name = string(p, "pattern")?.ok_or("`pattern` is required")?;
    let pattern = Pattern::from_name(name).ok_or("`pattern` must be space, digit, letter or lineEnd")?;
    let query = &tp.find.query;
    let len = query.chars().count();
    let at = match p.get("at").filter(|v| !v.is_null()) {
        None => len,
        Some(v) => v
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i <= len)
            .ok_or_else(|| format!("`at` must be a character index from 0 to {len}"))?,
    };
    let token = pattern.token();
    if len.saturating_add(token.chars().count()) > MAX_QUERY_CHARS {
        return Err(format!("the search holds at most {MAX_QUERY_CHARS} characters"));
    }
    let mut next: String = query.chars().take(at).collect();
    next.push_str(token);
    next.extend(query.chars().skip(at));
    let mut params = Map::new();
    params.insert("query".into(), Value::String(next));
    find(tp, &params)
}

/// The waterfall `view.previewWaterfall` asks for: `on` toggles when not given; sizes are checked and kept in order.
fn waterfall(old: &Waterfall, p: &Map<String, Value>) -> Result<Waterfall, String> {
    let on = flag(p, "on")?.unwrap_or(!old.on);
    let optical = flag(p, "useOpticalSize")?.unwrap_or(old.optical);
    let sizes = match p.get("sizes").filter(|v| !v.is_null()) {
        None => old.sizes.clone(),
        Some(v) => {
            let list = v.as_array().ok_or("`sizes` must be a list of numbers")?;
            if list.is_empty() || list.len() > MAX_WATERFALL_SIZES {
                return Err(format!("`sizes` holds from 1 to {MAX_WATERFALL_SIZES} sizes"));
            }
            list.iter()
                .map(|v| v.as_f64().filter(|s| SIZE_RANGE.contains(s)))
                .collect::<Option<Vec<f64>>>()
                .ok_or_else(|| format!("each size lies between {} and {} points", SIZE_RANGE.start(), SIZE_RANGE.end()))?
        }
    };
    Ok(Waterfall { on, sizes, optical })
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
    let qlen = match_len(&query);
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
    let qlen = match_len(&tp.find.query);
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
            "source": tp.source.name(),
            "folder": match &tp.source {
                FontSource::Folder(f) => Some(f.to_string_lossy().to_string()),
                _ => None,
            },
            "exported": model.exported.iter().map(|e| json!({"file": e.file, "error": e.error, "shown": e.shown})).collect::<Vec<_>>(),
            "lastExport": tp.last_export.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "waterfall": {
                "on": tp.waterfall.on,
                "sizes": tp.waterfall.sizes,
                "useOpticalSize": tp.waterfall.optical,
                "lines": model.waterfall.iter().map(|r| json!({"size": r.size, "opticalSize": r.optical_size, "master": r.master})).collect::<Vec<_>>(),
            },
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

    /// Pattern tokens match one character of their kind, a doubled backslash is a backslash, and a query longer than a
    /// machine word still matches.
    #[test]
    fn pattern_tokens_match_one_character_of_their_kind() {
        let text = "a1 b22\nc3\\d";
        assert_eq!(find_matches(text, "\\l\\d", true, Matching::Contains), [0, 3, 7]);
        assert_eq!(find_matches(text, "\\d\\s", true, Matching::Contains), [1]);
        assert_eq!(find_matches(text, "\\d\\n", true, Matching::Contains), [5]);
        assert_eq!(find_matches(text, "\\\\d", true, Matching::Contains), [9], "a literal backslash before d");
        assert_eq!(find_matches(text, "\\l\\d", true, Matching::FullWord), [0, 7]);
        assert_eq!(match_len("\\l\\d\\\\"), 3);
        assert_eq!(match_len("a\\"), 2, "a backslash at the end is itself");
        let long = "x".repeat(70) + "7";
        let text = format!("{long} {long}");
        let query = "x".repeat(70) + "\\d";
        assert_eq!(find_matches(&text, &query, false, Matching::Contains), [0, 72]);
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
