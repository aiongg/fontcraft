//! The feature code editor of Font Info › Features: egui's multi-line text box with a layouter
//! that colours the code by the classes `features::edit::highlight` gives and underlines the
//! ranges of the last compile's errors. The box itself knows nothing of entries or commands: the
//! Features tab ([`super::features_tab`]) gives it the text and acts on what [`show`] returns
//! (the typed text, an Alt-click, the keys of the suggestion list).
//!
//! Rows: `fontInfo.features.syntaxColouring`, `fontInfo.features.inlineErrors` (the
//! underlines) (M8.17).

use std::ops::Range;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontId, Id, Key, Modifiers, Stroke};
use fontcraft_engine::features::edit::{Highlight, HighlightKind, highlight};

use crate::theme::Tokens;

/// Above this many bytes the code is shown without colours and underlines.
pub const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;
/// Most error ranges underlined at once.
pub const MAX_UNDERLINES: usize = 1_000;

/// The colour of one class of code. The theme has no syntax colours of its own: keywords and
/// tags take the accent, names the label colours, comments and punctuation the dim text.
pub fn colour(kind: HighlightKind, tokens: &Tokens) -> Color32 {
    let label = |i: usize| tokens.color_labels.get(i).copied().unwrap_or(tokens.text);
    match kind {
        HighlightKind::Keyword | HighlightKind::Tag => tokens.accent,
        HighlightKind::GlyphName => tokens.text,
        HighlightKind::ClassName => label(5),
        HighlightKind::LookupName => label(1),
        HighlightKind::Number => label(8),
        HighlightKind::String => label(2),
        HighlightKind::Comment | HighlightKind::Punctuation => tokens.text_dim,
        HighlightKind::Token => label(9),
    }
}

/// The largest character boundary of `text` at or before `at`.
pub fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The byte offset of character `index` of `text` (its length past the last character).
pub fn char_to_byte(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(b, _)| b)
}

/// The character index of byte offset `at` of `text`, counted to the boundary at or before it.
pub fn byte_to_char(text: &str, at: usize) -> usize {
    text.get(..floor_boundary(text, at)).map_or(0, |s| s.chars().count())
}

/// The highlighted stretch of `code` that holds byte `at` (or ends just before it, for a click
/// right after a name), when it is a glyph name, a class name or a token: what an Alt-click on it
/// previews. Its range and kind.
pub fn previewable_at(code: &str, at: usize) -> Option<(Range<usize>, HighlightKind)> {
    if code.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let hits = |h: &Highlight| h.range.start <= at && at <= h.range.end;
    let previewable = |h: &Highlight| matches!(h.kind, HighlightKind::GlyphName | HighlightKind::ClassName | HighlightKind::Token);
    highlight(code).into_iter().find(|h| hits(h) && previewable(h)).map(|h| (h.range, h.kind))
}

/// Sorted, disjoint byte ranges on character boundaries of `text`, from `ranges` in any order.
fn merged(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> =
        ranges.iter().take(MAX_UNDERLINES).map(|r| floor_boundary(text, r.start)..floor_boundary(text, r.end)).filter(|r| r.start < r.end).collect();
    out.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Whether one of the sorted, disjoint `ranges` holds `at`.
fn covers(ranges: &[Range<usize>], at: usize) -> bool {
    let i = ranges.partition_point(|r| r.end <= at);
    ranges.get(i).is_some_and(|r| r.start <= at)
}

/// The code as the box lays it out: each class in its colour, `errors` (byte ranges) underlined
/// in the error colour. Past [`MAX_HIGHLIGHT_BYTES`] the code is plain.
pub fn layout_job(code: &str, tokens: &Tokens, font: &FontId, errors: &[Range<usize>]) -> LayoutJob {
    let plain = TextFormat { font_id: font.clone(), color: tokens.text, ..Default::default() };
    let mut job = LayoutJob::default();
    if code.len() > MAX_HIGHLIGHT_BYTES {
        job.append(code, 0.0, plain);
        return job;
    }
    let mut marks: Vec<Highlight> = highlight(code);
    marks.sort_by_key(|h| h.range.start);
    let errors = merged(code, errors);
    // Every place the format may change, in order.
    let mut cuts: Vec<usize> = Vec::with_capacity(2 * (marks.len() + errors.len()) + 2);
    cuts.push(0);
    for r in marks.iter().map(|h| &h.range).chain(errors.iter()) {
        cuts.push(floor_boundary(code, r.start));
        cuts.push(floor_boundary(code, r.end));
    }
    cuts.push(code.len());
    cuts.sort_unstable();
    cuts.dedup();
    let underline = Stroke::new(1.0, tokens.error_text);
    let mut mark = 0usize;
    for pair in cuts.windows(2) {
        let [from, to] = pair else { continue };
        let Some(part) = code.get(*from..*to).filter(|p| !p.is_empty()) else { continue };
        while marks.get(mark).is_some_and(|h| h.range.end <= *from) {
            mark += 1;
        }
        let color = marks.get(mark).filter(|h| h.range.start <= *from).map_or(tokens.text, |h| colour(h.kind, tokens));
        let mut format = TextFormat { color, ..plain.clone() };
        if covers(&errors, *from) {
            format.underline = underline;
        }
        job.append(part, 0.0, format);
    }
    job
}

/// The keys the suggestion list takes from the box while it is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListKeys {
    /// Down presses less Up presses.
    pub moved: i32,
    pub accept: bool,
    pub close: bool,
}

/// Takes Up, Down, Return, Tab and Escape before the box sees them.
fn take_list_keys(ui: &egui::Ui) -> ListKeys {
    ui.input_mut(|i| {
        let down = i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown);
        let up = i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp);
        let accept = i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab);
        let close = i.consume_key(Modifiers::NONE, Key::Escape);
        let moved = i32::try_from(down).unwrap_or(i32::MAX).saturating_sub(i32::try_from(up).unwrap_or(i32::MAX));
        ListKeys { moved, accept, close }
    })
}

/// What the box did this frame.
#[derive(Debug)]
pub struct Output {
    pub response: egui::Response,
    /// The text changed by typing, pasting or cutting.
    pub changed: bool,
    pub focused: bool,
    /// The selection after the frame, in characters, sorted.
    pub selection: Option<Range<usize>>,
    /// An Alt-click landed on this byte of the text.
    pub alt_click: Option<usize>,
    /// The suggestion list's keys, when it was open.
    pub list_keys: ListKeys,
    /// The screen position under the cursor's line, where the suggestion list opens.
    pub cursor_pos: Option<egui::Pos2>,
}

/// Puts the box's cursor at character `index` before it draws.
pub fn set_cursor(ctx: &egui::Context, id: Id, index: usize) {
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(index))));
    state.store(ctx, id);
}

/// What the box draws: the text it edits, or the code it shows read-only.
pub enum Text<'a> {
    Edit(&'a mut String),
    ReadOnly(&'a str),
}

/// Draws the box `id`, `height` points tall, over `text`. `errors` are byte ranges to underline;
/// `list_open` gives the suggestion list the arrows, Return, Tab and Escape.
pub fn show(ui: &mut egui::Ui, tokens: &Tokens, id: Id, text: Text<'_>, height: f32, errors: &[Range<usize>], list_open: bool) -> Output {
    let focused_before = ui.ctx().memory(|m| m.has_focus(id));
    let list_keys = if list_open && focused_before { take_list_keys(ui) } else { ListKeys::default() };
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = layout_job(buffer.as_str(), tokens, &font, errors);
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let rows = (height / tokens.list_row_height.max(1.0)).floor().max(1.0) as usize;
    let alt = ui.input(|i| i.modifiers.alt);
    let mut shown: &str;
    let (buffer, editable): (&mut dyn egui::TextBuffer, bool) = match text {
        Text::Edit(s) => (s, true),
        Text::ReadOnly(s) => {
            shown = s;
            (&mut shown, false)
        }
    };
    let mut output = None;
    egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let edit = egui::TextEdit::multiline(buffer)
            .id(id)
            .code_editor()
            .lock_focus(true)
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .layouter(&mut layouter);
        output = Some(edit.show(ui));
    });
    let Some(output) = output else {
        let response = ui.label("");
        return Output { response, changed: false, focused: false, selection: None, alt_click: None, list_keys, cursor_pos: None };
    };
    let response = output.response.response.clone();
    let selection = output.cursor_range.map(|r| {
        let range = r.as_sorted_char_range();
        usize::from(range.start)..usize::from(range.end)
    });
    let alt_click = (alt && response.clicked()).then(|| selection.as_ref().map(|s| s.start)).flatten().map(|c| char_to_byte(output.galley.text(), c));
    let cursor_pos = output.cursor_range.map(|r| {
        let rect = output.galley.pos_from_cursor(r.primary);
        output.galley_pos + rect.left_bottom().to_vec2()
    });
    Output { changed: editable && response.changed(), focused: response.has_focus(), response, selection, alt_click, list_keys, cursor_pos }
}

#[cfg(test)]
#[path = "code_editor_tests.rs"]
mod tests;
