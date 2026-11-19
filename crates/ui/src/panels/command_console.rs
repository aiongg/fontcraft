//! Command Console: Clear and Copy, the log pane, the input line with completion, and the parameters doc of the
//! command being typed.
//!
//! | Control | Posts |
//! |---|---|
//! | Clear | `window.macroPanel {clear}`: empties the pane |
//! | Copy | `window.macroPanel {copy}`: the pane's text to the clipboard |
//! | typing in the input line | `window.macroPanel {input}` |
//! | Return in the input line | `window.macroPanel {line}`: runs the line |
//! | Tab, or a pick in the completion list | puts the suggestion in place of the word being typed |
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
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::console::{self, CONSOLE_COMMAND, MAX_LINE_BYTES, SuggestionKind};
use crate::app::models::console::ConsoleModel;

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
    }
}

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
}
