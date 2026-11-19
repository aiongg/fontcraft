//! The Notes tab: the font note in a field of several lines. The typed text is committed when the field loses the
//! keyboard, as one `font.note.set`.

use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
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

pub fn view(ui: &mut Ui, note: &str, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<NotesWords>();
    let field = TextField::new(words.note.clone(), Bind::given(note.to_string())).lines(NOTE_LINES).placeholder(words.hint.clone());
    if let Some(text) = field.show(ui).events.committed()
        && text != note
    {
        sent.push(Msg::run("font.note.set", json!({ "text": text })));
    }
}
