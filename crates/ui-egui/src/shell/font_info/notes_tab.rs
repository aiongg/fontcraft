//! The Notes tab: the font note in a text box that highlights Markdown, through `font.note.set`.
//! The note is stored as typed; the highlighting only styles the text in the box: headings of
//! level one to six, bold and italic with asterisks or underscores, inline links and autolinks.
//! The typed text is kept while the box has the keyboard and committed when it loses it, as one
//! `font.note.set`.
//!
//! Rows: `fontInfo.notes`, `fontInfo.notes.markdownHighlighting` (M7.16).

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Id};
use serde_json::json;

use crate::app::FontcraftApp;
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// Above this many bytes the note is shown without highlighting.
pub const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;
/// The box's height, in lines, while the note is shorter.
const NOTE_ROWS: usize = 20;

/// The id of the note's text box.
pub fn note_id() -> Id {
    widgets::id(("fontInfo.notes", "text"))
}

/// What a highlighted stretch of the note is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A heading line, with its level (1–6).
    Heading(u8),
    Bold,
    Italic,
    /// An inline link `[text](url)` or an autolink `<https://…>`.
    Link,
}

/// One highlighted stretch: byte offsets into the note, and how many of its bytes at each end
/// are Markdown syntax (`**`, `#`, `](url)`…), drawn dimmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub mark: Mark,
    pub open: usize,
    pub close: usize,
}

/// The highlighted stretches of `text`, in order and without overlaps; none when the text is
/// longer than [`MAX_HIGHLIGHT_BYTES`]. A heading takes its whole line; the other marks stay
/// within one line. Work is linear in the text's length.
pub fn highlight(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return out;
    }
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        match heading(body) {
            Some((level, open)) => out.push(Span { start: offset, end: offset + body.len(), mark: Mark::Heading(level), open, close: 0 }),
            None => inline(body, offset, &mut out),
        }
        offset += line.len();
    }
    out
}

/// A heading line: its level and the length of its number signs and the space after them.
fn heading(line: &str) -> Option<(u8, usize)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line.get(hashes..)?;
    let title = rest.trim_start_matches([' ', '\t']);
    if title.len() == rest.len() && !rest.is_empty() {
        return None;
    }
    let level = u8::try_from(hashes).ok()?;
    Some((level, line.len() - title.len()))
}

/// The syntax an inline search looks for. Once a search for one finds nothing, no later
/// position on the line has it either, so it is not searched again: [`Exhausted`] remembers.
#[derive(Clone, Copy)]
enum Seek {
    DoubleStar,
    DoubleUnderscore,
    Star,
    Underscore,
    LinkMiddle,
    LinkEnd,
    Angle,
}

/// The [`Seek`]s that found nothing on the line, one bit each.
#[derive(Default)]
struct Exhausted(u8);

impl Exhausted {
    fn has(&self, seek: Seek) -> bool {
        self.0 & (1 << seek as u8) != 0
    }

    fn mark(&mut self, seek: Seek) {
        self.0 |= 1 << seek as u8;
    }
}

/// Whether the byte before `at` in `line` is a letter or digit (for `_`, which inside a word is
/// part of the word).
fn word_before(line: &str, at: usize) -> bool {
    at.checked_sub(1).and_then(|i| line.as_bytes().get(i)).is_some_and(u8::is_ascii_alphanumeric)
}

fn word_at(line: &str, at: usize) -> bool {
    line.as_bytes().get(at).is_some_and(u8::is_ascii_alphanumeric)
}

/// Emphasis text: not empty, and neither starting nor ending with white space.
fn emphasis_text(text: &str) -> bool {
    !text.is_empty() && !text.starts_with(char::is_whitespace) && !text.ends_with(char::is_whitespace)
}

/// The bold, italic and link stretches of one line, `offset` bytes into the note.
fn inline(line: &str, offset: usize, out: &mut Vec<Span>) {
    let mut exhausted = Exhausted::default();
    let mut i = 0usize;
    while i < line.len() {
        let Some(rest) = line.get(i..) else {
            i += 1;
            continue;
        };
        let found = if rest.starts_with("**") || rest.starts_with("__") {
            let (delim, seek) = if rest.starts_with("**") { ("**", Seek::DoubleStar) } else { ("__", Seek::DoubleUnderscore) };
            emphasis(line, i, delim, seek, Mark::Bold, &mut exhausted)
        } else if rest.starts_with('*') {
            emphasis(line, i, "*", Seek::Star, Mark::Italic, &mut exhausted)
        } else if rest.starts_with('_') {
            emphasis(line, i, "_", Seek::Underscore, Mark::Italic, &mut exhausted)
        } else if rest.starts_with('[') {
            link(line, i, &mut exhausted)
        } else if rest.starts_with('<') {
            autolink(line, i, &mut exhausted)
        } else {
            Err(rest.chars().next().map_or(1, char::len_utf8))
        };
        match found {
            Ok(span) => {
                i = span.end;
                out.push(Span { start: span.start + offset, end: span.end + offset, ..span });
            }
            Err(skip) => i += skip.max(1),
        }
    }
}

/// Emphasis opening at `i` with `delim`: the span, or how far to move on.
fn emphasis(line: &str, i: usize, delim: &str, seek: Seek, mark: Mark, exhausted: &mut Exhausted) -> Result<Span, usize> {
    let n = delim.len();
    let underscore = delim.starts_with('_');
    if exhausted.has(seek) || (underscore && word_before(line, i)) {
        return Err(n);
    }
    let from = i + n;
    let Some(k) = line.get(from..).and_then(|r| r.find(delim)) else {
        exhausted.mark(seek);
        return Err(n);
    };
    let close = from + k;
    let text = line.get(from..close).unwrap_or_default();
    if !emphasis_text(text) || (underscore && word_at(line, close + n)) {
        return Err(n);
    }
    Ok(Span { start: i, end: close + n, mark, open: n, close: n })
}

/// An inline link `[text](url)` opening at `i`.
fn link(line: &str, i: usize, exhausted: &mut Exhausted) -> Result<Span, usize> {
    if exhausted.has(Seek::LinkMiddle) || exhausted.has(Seek::LinkEnd) {
        return Err(1);
    }
    let Some(middle) = line.get(i + 1..).and_then(|r| r.find("](")).map(|k| i + 1 + k) else {
        exhausted.mark(Seek::LinkMiddle);
        return Err(1);
    };
    let Some(end) = line.get(middle + 2..).and_then(|r| r.find(')')).map(|k| middle + 2 + k) else {
        exhausted.mark(Seek::LinkEnd);
        return Err(1);
    };
    Ok(Span { start: i, end: end + 1, mark: Mark::Link, open: 1, close: end + 1 - middle })
}

/// An autolink `<scheme:…>` opening at `i`. The search stops at the first `>`, `<` or white
/// space, so no stretch of the line is searched twice.
fn autolink(line: &str, i: usize, exhausted: &mut Exhausted) -> Result<Span, usize> {
    if exhausted.has(Seek::Angle) {
        return Err(1);
    }
    let Some(rest) = line.get(i + 1..) else { return Err(1) };
    let Some(k) = rest.find(|c: char| c == '>' || c == '<' || c.is_whitespace()) else {
        exhausted.mark(Seek::Angle);
        return Err(1);
    };
    let target = rest.get(..k).unwrap_or_default();
    if !rest.get(k..).is_some_and(|r| r.starts_with('>')) {
        return Err(1);
    }
    if ["http://", "https://", "mailto:"].iter().any(|s| target.strip_prefix(s).is_some_and(|r| !r.is_empty())) {
        return Ok(Span { start: i, end: i + k + 2, mark: Mark::Link, open: 1, close: 1 });
    }
    Err(1)
}

/// The note as the text box lays it out: plain text in the theme's colour, the highlighted
/// stretches styled, their syntax dimmed.
pub fn layout_job(text: &str, tokens: &Tokens, body: &FontId, strong: Color32) -> LayoutJob {
    let plain = TextFormat { font_id: body.clone(), color: tokens.text, ..Default::default() };
    let mut job = LayoutJob::default();
    let append = |job: &mut LayoutJob, from: usize, to: usize, format: &TextFormat| {
        if let Some(part) = text.get(from..to).filter(|p| !p.is_empty()) {
            job.append(part, 0.0, format.clone());
        }
    };
    let mut at = 0usize;
    for span in highlight(text) {
        append(&mut job, at, span.start, &plain);
        let format = match span.mark {
            Mark::Heading(_) => TextFormat { font_id: FontId::new(tokens.section_header_size, body.family.clone()), color: strong, ..plain.clone() },
            Mark::Bold => TextFormat { color: strong, ..plain.clone() },
            Mark::Italic => TextFormat { italics: true, ..plain.clone() },
            Mark::Link => TextFormat { color: tokens.accent, ..plain.clone() },
        };
        let syntax = TextFormat { color: tokens.text_dim, ..format.clone() };
        let inner_start = span.start + span.open;
        let inner_end = span.end.saturating_sub(span.close).max(inner_start);
        append(&mut job, span.start, inner_start, &syntax);
        append(&mut job, inner_start, inner_end, &format);
        append(&mut job, inner_end, span.end, &syntax);
        at = span.end;
    }
    append(&mut job, at, text.len(), &plain);
    job
}

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let FontcraftApp { session, tokens, .. } = app;
    let Ok(font) = session.font() else { return };
    let id = note_id();
    let stored = font.note.as_str();
    let kept = ui.data_mut(|d| d.get_temp::<String>(id));
    let editing = kept.is_some();
    let mut text = kept.unwrap_or_else(|| stored.to_string());
    let body = egui::TextStyle::Body.resolve(ui.style());
    let strong = ui.visuals().strong_text_color();
    let tokens: &Tokens = tokens;
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = layout_job(buffer.as_str(), tokens, &body, strong);
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    ui.add_space(tokens.dialog_spacing);
    let response = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id(id)
            .desired_width(f32::INFINITY)
            .desired_rows(NOTE_ROWS)
            .hint_text("A note about this font, kept in the source and never exported")
            .layouter(&mut layouter),
    );
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else if editing {
        // The box lost the keyboard (a click elsewhere, Escape): the typed text is committed.
        ui.data_mut(|d| d.remove::<String>(id));
        if text != stored {
            pending.push(("font.note.set", json!({"text": text})));
        }
    }
}

#[cfg(test)]
#[path = "notes_tab_tests.rs"]
mod tests;
