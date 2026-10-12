//! The Notes tab: the Markdown highlighting of the handbook's examples, its linear work on
//! hostile text, and the text box, whose edit gives the font `font.note.set` gives.

use egui::Event;
use serde_json::json;

use super::super::font_tab::tests::{Tab, app, same_as_command};
use super::{MAX_HIGHLIGHT_BYTES, Mark, Span, highlight, note_id};

/// The highlighted stretches of `text` as (marked text, mark, syntax at each end).
fn marks(text: &str) -> Vec<(&str, Mark, usize, usize)> {
    highlight(text).into_iter().map(|s: Span| (text.get(s.start..s.end).unwrap(), s.mark, s.open, s.close)).collect()
}

#[test]
fn notes_highlight_headings_emphasis_and_links() {
    assert_eq!(marks("# Head"), [("# Head", Mark::Heading(1), 2, 0)]);
    assert_eq!(marks("###### Six"), [("###### Six", Mark::Heading(6), 7, 0)]);
    assert_eq!(marks("*it* and _it_"), [("*it*", Mark::Italic, 1, 1), ("_it_", Mark::Italic, 1, 1)]);
    assert_eq!(marks("**bold** and __bold__"), [("**bold**", Mark::Bold, 2, 2), ("__bold__", Mark::Bold, 2, 2)]);
    assert_eq!(marks("see [Link Text](https://example.org)."), [("[Link Text](https://example.org)", Mark::Link, 1, 22)]);
    assert_eq!(marks("<https://example.org>"), [("<https://example.org>", Mark::Link, 1, 1)]);
    // Offsets count from the start of the note, across lines.
    let note = "intro\n## Two\nend *x*";
    assert_eq!(marks(note), [("## Two", Mark::Heading(2), 3, 0), ("*x*", Mark::Italic, 1, 1)]);
    // Not Markdown: seven number signs, no space after them, a word with underscores, spaced
    // asterisks, an angle bracket that is no link, an unclosed link.
    for plain in ["####### Seven", "#tag", "snake_case_name", "a * b * c", "1 < 2 > 0", "<b>", "[open](no end", "**"] {
        assert!(highlight(plain).is_empty(), "`{plain}` is plain: {:?}", highlight(plain));
    }
}

#[test]
fn highlighting_hostile_text_stays_linear_and_long_notes_stay_plain() {
    // Each of these would take quadratic work if every opener searched the rest of the line.
    let n = MAX_HIGHLIGHT_BYTES / 2;
    for unit in ["[", "<", "*", "_", "** ", "<a ", "[x](", "a_"] {
        let text = unit.repeat(n / unit.len());
        let _ = highlight(&text);
    }
    let long = "*a* ".repeat(MAX_HIGHLIGHT_BYTES / 4 + 1);
    assert!(highlight(&long).is_empty(), "a note past the limit is shown plain");
}

#[test]
fn the_note_box_gives_the_font_font_note_set_gives() {
    let mut app = app();
    let tab = Tab::new(super::show);
    tab.frame(&mut app, Vec::new());
    tab.ctx.memory_mut(|m| m.request_focus(note_id()));
    tab.frame(&mut app, Vec::new());
    assert!(tab.frame(&mut app, vec![Event::Text("Ship **bold** soon".into())]).is_empty(), "typing queues nothing");
    assert_eq!(app.session.font().unwrap().note, "", "the note waits for the box to lose the keyboard");
    tab.ctx.memory_mut(|m| m.surrender_focus(note_id()));
    let mut pending = tab.frame(&mut app, Vec::new());
    pending.extend(tab.frame(&mut app, Vec::new()));
    let f = same_as_command(&mut app, pending, "font.note.set", json!({"text": "Ship **bold** soon"}));
    assert_eq!(f.note, "Ship **bold** soon", "the note is stored as typed");
}
