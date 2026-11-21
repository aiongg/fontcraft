//! Reading and setting the caret and selection of a field, for commands that act on the selected characters (a
//! direction for the selected run, a snippet in place of the selected code).
//!
//! Meant for craft-ui-contract: there the selection is a field of `EditResponse` that the text part fills from its
//! adapter, and a `TextField::selection` prop (`Bind<TextSelection>`, as craft-text's `TextEditor` takes one) sets it.
//! The adapter keeps its state to itself, so this stand-in reads and writes the selection where craft's adapter does:
//! the egui state of the field's text widget ([`super::text_widget`]). That widget drops its selection when it loses
//! focus, and a command usually runs from a menu or a button that took the focus, so [`FieldSelection::last_selection`]
//! keeps the last selection seen while the field had focus. It is kept only for fields it is asked about every pass.

use std::ops::Range;

use craft::ui::contract::EditResponse;
use craft::ui::runtime::memory;
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Context, Id, Response};

/// A field's selection in characters: `anchor` stays where it started, `caret` moves. Equal, a caret with nothing
/// selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextSelection {
    pub anchor: usize,
    pub caret: usize,
}

impl TextSelection {
    /// A selection from `anchor` to `caret`.
    pub fn new(anchor: usize, caret: usize) -> Self {
        TextSelection { anchor, caret }
    }

    /// A caret at character `at`, nothing selected.
    pub fn caret_at(at: usize) -> Self {
        TextSelection { anchor: at, caret: at }
    }

    /// The selected characters, start first.
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    /// Nothing is selected.
    pub fn is_empty(self) -> bool {
        self.anchor == self.caret
    }

    /// The selection when it lies in a text of `len` characters; a kept selection can outlive the text it was made in.
    pub fn within(self, len: usize) -> Option<Self> {
        (self.anchor <= len && self.caret <= len).then_some(self)
    }
}

/// The selection of a field's text widget, from the response of the field.
pub trait FieldSelection {
    /// The selection while the field has focus; `None` otherwise.
    fn selection(&self, ctx: &Context) -> Option<TextSelection>;

    /// The selection while the field has focus, else the last one it had while it had focus. Ask every pass the field
    /// shows: the selection is recorded only when asked.
    fn last_selection(&self, ctx: &Context) -> Option<TextSelection>;

    /// Selects `selection` in the field; the field shows it from the next pass. Character indexes past the text are
    /// clamped by the field.
    fn select(&self, ctx: &Context, selection: TextSelection);
}

/// Where the last selection of the field whose component id is `field` is kept.
fn kept_id(field: Id) -> Id {
    memory::sub(super::text_widget(field), "craftExtLastSelection")
}

fn read(ctx: &Context, field: Id) -> Option<TextSelection> {
    if !super::text_focused(ctx, field) {
        return None;
    }
    let range = TextEditState::load(ctx, super::text_widget(field))?.cursor.char_range()?;
    Some(TextSelection::new(range.secondary.index.0, range.primary.index.0))
}

fn read_kept(ctx: &Context, field: Id) -> Option<TextSelection> {
    match read(ctx, field) {
        Some(selection) => {
            ctx.data_mut(|data| data.insert_temp(kept_id(field), selection));
            Some(selection)
        }
        None => ctx.data(|data| data.get_temp::<TextSelection>(kept_id(field))),
    }
}

fn write(ctx: &Context, field: Id, selection: TextSelection) {
    let widget = super::text_widget(field);
    let mut state = TextEditState::load(ctx, widget).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange {
        primary: CCursor::new(selection.caret),
        secondary: CCursor::new(selection.anchor),
        h_pos: None,
    }));
    state.store(ctx, widget);
    ctx.data_mut(|data| data.insert_temp(kept_id(field), selection));
}

impl FieldSelection for Response {
    fn selection(&self, ctx: &Context) -> Option<TextSelection> {
        read(ctx, self.id)
    }

    fn last_selection(&self, ctx: &Context) -> Option<TextSelection> {
        read_kept(ctx, self.id)
    }

    fn select(&self, ctx: &Context, selection: TextSelection) {
        write(ctx, self.id, selection);
    }
}

impl<T> FieldSelection for EditResponse<T> {
    fn selection(&self, ctx: &Context) -> Option<TextSelection> {
        read(ctx, self.response.id)
    }

    fn last_selection(&self, ctx: &Context) -> Option<TextSelection> {
        read_kept(ctx, self.response.id)
    }

    fn select(&self, ctx: &Context, selection: TextSelection) {
        write(ctx, self.response.id, selection);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use craft::ui::contract::{Bind, Show as _};
    use craft::ui::testing::Fixture;
    use craft::ui::text::TextField;

    use super::*;

    #[test]
    fn a_selection_reads_start_first_and_checks_its_text() {
        let s = TextSelection::new(5, 2);
        assert_eq!(s.range(), 2..5);
        assert!(!s.is_empty() && TextSelection::caret_at(3).is_empty());
        assert_eq!(s.within(5), Some(s));
        assert_eq!(s.within(4), None);
    }

    /// A selection set on the field reads back while it has focus, and the last one is kept once the focus moves on.
    #[test]
    fn a_set_selection_reads_back_and_outlives_the_focus() {
        let select = Rc::new(Cell::new(None::<TextSelection>));
        let seen = Rc::new(RefCell::new((None, None)));
        let (ask, record) = (Rc::clone(&select), Rc::clone(&seen));
        let mut fixture = Fixture::new(move |ui| {
            let out = TextField::new("Text", Bind::given("Hamburgefonstiv".to_string())).show(ui);
            if let Some(selection) = ask.take() {
                out.select(ui.ctx(), selection);
            }
            *record.borrow_mut() = (out.selection(ui.ctx()), out.last_selection(ui.ctx()));
            let _ = TextField::new("Other", Bind::given(String::new())).show(ui);
        });
        fixture.settle();
        let field = fixture.by_label("Text").expect("the field");
        fixture.click(&field).settle();
        select.set(Some(TextSelection::new(2, 6)));
        fixture.settle();
        assert_eq!(*seen.borrow(), (Some(TextSelection::new(2, 6)), Some(TextSelection::new(2, 6))));
        let other = fixture.by_label("Other").expect("the other field");
        fixture.click(&other).settle();
        assert_eq!(*seen.borrow(), (None, Some(TextSelection::new(2, 6))));
    }
}
