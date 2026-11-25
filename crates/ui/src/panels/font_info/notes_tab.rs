//! The Notes tab: the font note in a field of several lines, its Markdown coloured as plain text ([`highlight`]). The
//! typed text is committed when the field loses the keyboard, as one `font.note.set`.

use std::ops::Range;

use craft::theme::{Rgba, Srgba};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use craft_ext::ui::text::{Highlight, HighlightText as _};
use egui::{Color32, Ui};
use serde_json::json;

use crate::app::Msg;

craft::ui::words! {
    /// The words of the Notes tab.
    pub struct NotesWords {
        note: "fontcraftFontInfoNotes.note" = "Font Note";
        hint: "fontcraftFontInfoNotes.hint" = "A note about this font, kept in the source and never exported";
    }
}

/// Lines the note field shows.
const NOTE_LINES: usize = 16;
/// Longest note, in bytes, whose Markdown is coloured.
const MAX_HIGHLIGHTED: usize = 256 * 1024;
/// Most spans one note gets.
const MAX_SPANS: usize = 10_000;

pub fn view(ui: &mut Ui, note: &str, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<NotesWords>();
    let field = TextField::new(words.note.clone(), Bind::given(note.to_string())).lines(NOTE_LINES).placeholder(words.hint.clone());
    let out = field.show(ui);
    let env = Env::of(ui);
    let color = |c: Rgba| Color32::from(Srgba::from(c));
    let (strong, muted, accent) = (env.tokens.color.accent.solid, env.tokens.color.text.muted, env.tokens.color.accent.solid);
    out.highlight_text(ui, |text| {
        highlight(text)
            .into_iter()
            .map(|(range, form)| match form {
                Form::Heading | Form::Bold => Highlight::ink(range, color(strong)),
                Form::Italic => Highlight::underline(range, color(muted)),
                Form::Link => Highlight { underline: Some(color(accent)), ..Highlight::ink(range, color(accent)) },
                Form::Mark => Highlight::ink(range, color(muted)),
            })
            .collect()
    });
    if let Some(text) = out.events.committed()
        && text != note
    {
        sent.push(Msg::run("font.note.set", json!({ "text": text })));
    }
}

/// What a span of a note is in Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Heading,
    Bold,
    Italic,
    /// A link's text or address.
    Link,
    /// The Markdown characters themselves.
    Mark,
}

/// The Markdown spans of `text`, in order, none overlapping: headings of one to six number signs, bold, italic, inline
/// links and autolinks, each within its line. None for a text past [`MAX_HIGHLIGHTED`] bytes.
pub fn highlight(text: &str) -> Vec<(Range<usize>, Form)> {
    let mut out = Vec::new();
    if text.len() > MAX_HIGHLIGHTED {
        return out;
    }
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if out.len() >= MAX_SPANS {
            break;
        }
        let body = line.trim_end_matches(['\n', '\r']);
        line_spans(body, start, &mut out);
        start += line.len();
    }
    out
}

/// The spans of the line `line`, which starts at byte `at` of the note.
fn line_spans(line: &str, at: usize, out: &mut Vec<(Range<usize>, Form)>) {
    let signs = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&signs) && line.get(signs..).is_some_and(|rest| rest.is_empty() || rest.starts_with(' ')) {
        out.push((at..at + signs, Form::Mark));
        if line.len() > signs {
            out.push((at + signs..at + line.len(), Form::Heading));
        }
        return;
    }
    let mut i = 0;
    // Each search for a closing mark that failed, so a line of opening marks alone is read once.
    let mut none_after: [bool; 5] = [false; 5];
    while i < line.len() && out.len() < MAX_SPANS {
        let Some(rest) = line.get(i..) else { break };
        let before = line.get(..i).and_then(|s| s.chars().next_back());
        if let Some(len) = span(rest, before, &mut none_after, at + i, out) {
            i += len;
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
}

/// The span `rest` starts with, pushed to `out` as spans at byte `at`; its length in bytes.
fn span(rest: &str, before: Option<char>, none_after: &mut [bool; 5], at: usize, out: &mut Vec<(Range<usize>, Form)>) -> Option<usize> {
    for (slot, (mark, form)) in [("**", Form::Bold), ("__", Form::Bold), ("*", Form::Italic), ("_", Form::Italic)].into_iter().enumerate() {
        if none_after.get(slot).copied().unwrap_or(false) || !rest.starts_with(mark) {
            continue;
        }
        let inner = rest.get(mark.len()..)?;
        if mark.starts_with('_') && before.is_some_and(char::is_alphanumeric) {
            continue;
        }
        if inner.starts_with(char::is_whitespace) || inner.is_empty() {
            continue;
        }
        let Some(end) = inner.find(mark).filter(|end| *end > 0) else {
            if let Some(none) = none_after.get_mut(slot) {
                *none = !inner.contains(mark);
            }
            continue;
        };
        let after = inner.get(end + mark.len()..).and_then(|s| s.chars().next());
        if mark.starts_with('_') && after.is_some_and(char::is_alphanumeric) || inner.get(..end).is_some_and(|s| s.ends_with(char::is_whitespace)) {
            continue;
        }
        let (a, b) = (at + mark.len(), at + mark.len() + end);
        out.push((at..a, Form::Mark));
        out.push((a..b, form));
        out.push((b..b + mark.len(), Form::Mark));
        return Some(end + 2 * mark.len());
    }
    if let Some(none) = none_after.get_mut(4)
        && !*none
        && rest.starts_with('[')
    {
        // The text holds no bracket, so a run of opening brackets is not searched to the line's end from each.
        let mid = rest.get(1..).and_then(|t| t.find(['[', ']'])).map(|n| n + 1);
        let mid = mid.filter(|mid| *mid > 1 && rest.get(*mid..).is_some_and(|t| t.starts_with("](")));
        match mid.map(|mid| (mid, rest.get(mid + 2..).and_then(|t| t.find(')')))) {
            Some((mid, Some(close))) => {
                let close = mid + 2 + close;
                out.push((at..at + 1, Form::Mark));
                out.push((at + 1..at + mid, Form::Link));
                out.push((at + mid..at + mid + 2, Form::Mark));
                out.push((at + mid + 2..at + close, Form::Link));
                out.push((at + close..at + close + 1, Form::Mark));
                return Some(close + 1);
            }
            Some((_, None)) => *none = true,
            None => {}
        }
    }
    let address = rest.strip_prefix('<')?;
    if !["http://", "https://", "mailto:"].iter().any(|s| address.starts_with(s)) {
        return None;
    }
    let end = address.find(['>', ' ', '<']).filter(|end| address.get(*end..).is_some_and(|s| s.starts_with('>')))?;
    out.push((at..at + 1, Form::Mark));
    out.push((at + 1..at + 1 + end, Form::Link));
    out.push((at + 1 + end..at + 2 + end, Form::Mark));
    Some(end + 2)
}

#[cfg(test)]
mod tests {
    use super::{Form, highlight};

    #[test]
    fn each_markdown_form_is_found_within_its_line() {
        let spans =
            |text: &str| highlight(text).into_iter().filter(|(_, f)| *f != Form::Mark).map(|(r, f)| (text[r].to_string(), f)).collect::<Vec<_>>();
        let one = |text: &str, inner: &str, form: Form| assert_eq!(spans(text), [(inner.to_string(), form)], "{text}");
        one("## Head *no*", " Head *no*", Form::Heading);
        one("é *it*", "it", Form::Italic);
        one("_it_", "it", Form::Italic);
        one("a **bold** b", "bold", Form::Bold);
        one("__bold__", "bold", Form::Bold);
        one("<https://example.org>", "https://example.org", Form::Link);
        assert_eq!(spans("[Text](https://x.org)"), [("Text".to_string(), Form::Link), ("https://x.org".to_string(), Form::Link)]);
        for plain in ["#tag", "####### seven", "a_b_c", "2 * 3 * 4", "*open\nclose*", "<b>", "[x] (y)", "**", "***"] {
            assert!(spans(plain).is_empty(), "{plain}: {:?}", spans(plain));
        }
        // Many opening marks are read in time linear in the line.
        assert!(highlight(&"[".repeat(100_000)).is_empty());
        assert!(highlight(&"_a ".repeat(50_000)).len() < 200_000);
    }
}
