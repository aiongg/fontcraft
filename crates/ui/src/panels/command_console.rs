//! Command Console: Clear and Copy, the log pane, the input line with completion, the parameters doc of the
//! command being typed, and the Macro Panel's editor under them (`shell::macro_editor`).
//!
//! | Control | Posts |
//! |---|---|
//! | Clear | `window.macroPanel {clear}`: empties the pane |
//! | Copy | `window.macroPanel {copy}`: the pane's text to the clipboard |
//! | typing in the input line | `window.macroPanel {input}` |
//! | Return in the input line | `window.macroPanel {line}`: runs the line |
//! | Tab, or a pick in the completion list | puts the suggestion in place of the word being typed |
//! | typing in the editor | `macroPanel.setText {text}` |
//! | Run All | `macroPanel.runAll` |
//! | Run Selection | `macroPanel.runSelection {first, last}`: the lines the editor's last selection touches, or the caret's line |
//! | Open…, Save… | `macroPanel.open`, `macroPanel.save`: the window asks for the file |
//! | Record, Stop | `macroPanel.record {on}`: Stop adds the recorded steps to the editor |
//!
//! While the input line is empty, its completion list offers the lines run before, newest first.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::primitives::{Completer, Completion};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use craft_ext::ui::text::FieldSelection as _;
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::console::{self, CONSOLE_COMMAND, MAX_LINE_BYTES, SuggestionKind};
use crate::app::models::console::ConsoleModel;
use crate::shell::macro_editor::MAX_EDITOR_BYTES;

craft::ui::words! {
    /// The words of the Command Console.
    pub struct ConsoleWords {
        /// The button that empties the pane.
        clear: "fontcraftConsole.clear" = "Clear";
        /// The button that copies the pane's text.
        copy: "fontcraftConsole.copy" = "Copy";
        /// The log pane's name.
        log: "fontcraftConsole.log" = "Command Log";
        /// The pane with nothing in it.
        empty: "fontcraftConsole.empty" = "No commands run yet";
        /// The input line's name.
        input: "fontcraftConsole.input" = "Command";
        /// The input line's placeholder.
        placeholder: "fontcraftConsole.placeholder" = "command {parameters}, or run --cmd ID --params JSON";
        /// The macro editor's name.
        editor: "fontcraftConsole.editor" = "Macro";
        run_all: "fontcraftConsole.runAll" = "Run All";
        run_selection: "fontcraftConsole.runSelection" = "Run Selection";
        open: "fontcraftConsole.open" = "Open…";
        save: "fontcraftConsole.save" = "Save…";
        record: "fontcraftConsole.record" = "Record";
        stop: "fontcraftConsole.stop" = "Stop";
    }
}

/// Lines the macro editor shows.
const EDITOR_LINES: usize = 10;

/// What the console reads and sends.
pub struct Props<'a> {
    pub model: &'a Tracked<ConsoleModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ConsoleWords>();
    let model = props.model;
    let sent = &mut *props.sent;
    Row::new().show(ui, |ui, _| {
        if Button::new(words.clear.clone()).show(ui).clicked() {
            sent.push(console_msg(json!({ "clear": true })));
        }
        if Button::new(words.copy.clone()).show(ui).clicked() {
            sent.push(console_msg(json!({ "copy": true })));
        }
    });
    let source = Lines(&model.lines);
    let _ = List::new(words.log.clone())
        .empty(words.empty.clone())
        .show_items(ui, &source, &mut |at, _| ListRow::new(model.lines.get(*at).cloned().unwrap_or_default()));
    let completer = ConsoleCompleter { docs: Arc::clone(&model.docs), glyphs: Arc::clone(&model.glyphs), history: Arc::clone(&model.history) };
    let typed = TextField::new(words.input.clone(), Bind::given(model.input.clone()))
        .placeholder(words.placeholder.clone())
        .mono()
        .limit(MAX_LINE_BYTES)
        .submits()
        .complete(completer)
        .show(ui);
    let text = typed.events.value().cloned();
    if typed.submitted {
        let line = text.unwrap_or_else(|| model.input.clone());
        if !line.trim().is_empty() {
            sent.push(console_msg(json!({ "line": line })));
        }
    } else if let Some(text) = text.filter(|t| *t != model.input) {
        sent.push(console_msg(json!({ "input": text })));
    }
    if let Some(doc) = &model.doc {
        Text::new(doc.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    editor(ui, &words, model, sent);
}

/// The macro editor and its buttons.
fn editor(ui: &mut Ui, words: &ConsoleWords, model: &ConsoleModel, sent: &mut Vec<Msg>) {
    let out = TextField::new(words.editor.clone(), Bind::given(model.editor.clone()))
        .mono()
        .lines(EDITOR_LINES)
        .tab_inserts(true)
        .limit(MAX_EDITOR_BYTES)
        .id_salt("fontcraftConsole.editor")
        .show(ui);
    if let Some(text) = out.events.value()
        && **text != model.editor
    {
        sent.push(Msg::Shell(CommandRef::with_params("macroPanel.setText", json!({ "text": text }))));
    }
    let chars = model.editor.chars().count();
    let lines = out.last_selection(ui.ctx()).and_then(|s| s.within(chars)).and_then(|s| selected_lines(&model.editor, s.range()));
    let has_steps = !model.editor.trim().is_empty();
    Row::new().wrap(true).show(ui, |ui, _| {
        if Button::new(words.run_all.clone()).enabled(has_steps).show(ui).clicked() && has_steps {
            sent.push(Msg::Shell(CommandRef::new("macroPanel.runAll")));
        }
        if Button::new(words.run_selection.clone()).enabled(lines.is_some()).show(ui).clicked()
            && let Some((first, last)) = lines
        {
            sent.push(Msg::Shell(CommandRef::with_params("macroPanel.runSelection", json!({ "first": first, "last": last }))));
        }
        if Button::new(words.open.clone()).show(ui).clicked() {
            sent.push(Msg::Shell(CommandRef::new("macroPanel.open")));
        }
        if Button::new(words.save.clone()).enabled(has_steps).show(ui).clicked() && has_steps {
            sent.push(Msg::Shell(CommandRef::new("macroPanel.save")));
        }
        let record = if model.recording { &words.stop } else { &words.record };
        if Button::new(record.clone()).show(ui).clicked() {
            sent.push(Msg::Shell(CommandRef::with_params("macroPanel.record", json!({ "on": !model.recording }))));
        }
    });
}

/// The lines, counted from 1, that the characters `range` of `text` touch: an empty range gives its caret's line,
/// and a range ending at the start of a line leaves that line out. `None` when the text has no lines.
pub fn selected_lines(text: &str, range: Range<usize>) -> Option<(usize, usize)> {
    let count = crate::shell::macro_editor::line_count(text);
    if count == 0 {
        return None;
    }
    let line_at = |at: usize| text.chars().take(at).filter(|c| *c == '\n').count() + 1;
    let first = line_at(range.start);
    let end = if range.end > range.start && text.chars().nth(range.end - 1) == Some('\n') { range.end - 1 } else { range.end };
    let last = line_at(end).max(first);
    Some((first.min(count), last.min(count)))
}

fn console_msg(params: serde_json::Value) -> Msg {
    Msg::Shell(CommandRef::with_params(CONSOLE_COMMAND, params))
}

/// The pane's lines as a collection keyed by position.
struct Lines<'a>(&'a [String]);

impl DisplayOrder<usize> for Lines<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|line| Cow::Borrowed(line.as_str()))
    }
}

impl ItemSource<usize> for Lines<'_> {}

/// The input line's completion: command ids, glyph names inside parameters, and the history on an empty line.
struct ConsoleCompleter {
    docs: Arc<[(&'static str, &'static str)]>,
    glyphs: Arc<[String]>,
    history: Arc<[String]>,
}

impl Completer for ConsoleCompleter {
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>> {
        let before = text.get(..caret)?;
        let found = console::complete(&self.docs, &self.glyphs, &self.history, before);
        for item in &found.items {
            let Some(insert) = item.insert() else { continue };
            let mut candidate = Completion::new(item.text.clone(), insert);
            if item.kind == SuggestionKind::Command
                && let Ok(at) = self.docs.binary_search_by(|d| d.0.cmp(item.text.as_str()))
                && let Some((_, doc)) = self.docs.get(at)
            {
                candidate = candidate.description(*doc);
            }
            out.push(candidate);
        }
        (!out.is_empty()).then_some(found.start..caret)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::{Props, console_msg, show};
    use crate::app::models::console::ConsoleModel;

    /// Clear posts `{clear}`, and Return in the input line posts the line to run.
    #[test]
    fn clear_and_return_post_console_commands() {
        let model = Tracked::new(ConsoleModel { lines: vec!["edit.undo {} → null".into()], ..Default::default() });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let clear = fixture.by_label("Clear").expect("the Clear button");
        fixture.click(&clear).settle();
        assert_eq!(*posted.borrow(), vec![console_msg(json!({ "clear": true }))]);
        posted.borrow_mut().clear();

        let input = fixture.by_label("Command").expect("the input line");
        fixture.click(&input).type_text("edit.undo").key(Key::Enter, Modifiers::NONE).settle();
        let sent = posted.borrow();
        assert_eq!(sent.last(), Some(&console_msg(json!({ "line": "edit.undo" }))), "{sent:?}");
    }

    /// Run Selection posts the lines the editor's selection touches: all selected, then the caret moved up a line
    /// with Shift held, leaves the last line out. Save posts `macroPanel.save` for the window to ask for the file.
    #[test]
    fn run_selection_posts_the_selected_lines_and_save_posts_save() {
        use craft::ui::runtime::commands::CommandRef;

        use crate::app::Msg;

        let editor = "glyph.newGlyph {\"name\":\"x1\"}\nglyph.newGlyph {\"name\":\"x2\"}\nglyph.newGlyph {\"name\":\"x3\"}";
        let model = Tracked::new(ConsoleModel { editor: editor.into(), ..Default::default() });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.room(egui::vec2(700.0, 900.0)).settle();
        let field = fixture.by_label("Macro").expect("the macro editor");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).key(Key::ArrowUp, Modifiers::SHIFT).settle();
        let run = fixture.by_label("Run Selection").expect("the Run Selection button");
        fixture.click(&run).settle();
        let selection = Msg::Shell(CommandRef::with_params("macroPanel.runSelection", json!({ "first": 1, "last": 2 })));
        assert!(posted.borrow().contains(&selection), "{:?}", posted.borrow());
        assert!(!posted.borrow().iter().any(|m| matches!(m, Msg::Shell(c) if c.id == "macroPanel.setText")), "selecting edits nothing");

        posted.borrow_mut().clear();
        let save = fixture.by_label("Save…").expect("the Save button");
        fixture.click(&save).settle();
        assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::new("macroPanel.save"))]);
    }

    /// The lines a selection touches: an empty selection is its caret's line, and a selection ending at a line's
    /// start leaves that line out.
    #[test]
    fn selections_cover_whole_lines() {
        use super::selected_lines;

        let text = "ab\ncd\nef";
        assert_eq!(selected_lines(text, 4..4), Some((2, 2)));
        assert_eq!(selected_lines(text, 0..3), Some((1, 1)), "ends at the start of line 2");
        assert_eq!(selected_lines(text, 1..7), Some((1, 3)));
        assert_eq!(selected_lines("", 0..0), None);
    }
}
