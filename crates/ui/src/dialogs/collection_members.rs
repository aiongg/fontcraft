//! The question which members of a font collection File › Open… opens (`app/file_questions.rs`): family, style, name
//! and number of glyphs of each; several can be chosen, and each opens as its own document. An answer posts
//! `window.fileQuestion`.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::contract::{Bind, ColumnSpec, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use egui::Ui;
use serde_json::{Value, json};

use super::loss_question::{pending, post};
use crate::app::FontCraftApp;
use crate::app::file_questions::{Member, MembersQuestion, Question};

craft::ui::words! {
    /// The words of the question which fonts of a collection to open.
    pub struct CollectionMembersWords {
        title: "fontcraftCollection.title" = "Open Font Collection";
        /// After the file's name.
        message: "fontcraftCollection.message" = "holds several fonts. Each one chosen opens as its own document.";
        fonts: "fontcraftCollection.fonts" = "Fonts";
        family: "fontcraftCollection.family" = "Family";
        style: "fontcraftCollection.style" = "Style";
        postscript: "fontcraftCollection.postscript" = "PostScript Name";
        glyphs: "fontcraftCollection.glyphs" = "Glyphs";
        open: "fontcraftCollection.open" = "Open";
    }
}

/// Rows the list shows before it scrolls.
const LIST_ROWS: u16 = 12;

/// The question, with the number `app/file_questions.rs` gave it.
pub struct MembersDialog {
    id: DialogId,
    serial: u64,
    key: String,
    question: MembersQuestion,
    /// The chosen rows, by position.
    chosen: Selection<usize>,
    closing: bool,
    seen: bool,
    open: bool,
}

impl MembersDialog {
    /// The question with its first font chosen.
    pub fn new(serial: u64, question: MembersQuestion) -> MembersDialog {
        MembersDialog {
            id: dialog_id("fontcraft.collectionMembers"),
            serial,
            key: serial.to_string(),
            question,
            chosen: Selection::new(vec![0], None, None),
            closing: false,
            seen: true,
            open: true,
        }
    }

    /// `window.fileQuestion`'s parameters for the chosen rows.
    fn answer(&self) -> Value {
        let faces: Vec<u32> =
            self.question.members.iter().enumerate().filter(|(at, _)| self.chosen.items.contains(at)).map(|(_, m)| m.index).collect();
        json!({ "answer": "proceed", "faces": faces })
    }
}

/// The first column's text: the family, else the PostScript name, else the font's number.
fn family_of(m: &Member) -> String {
    if !m.family.is_empty() {
        m.family.clone()
    } else if !m.postscript.is_empty() {
        m.postscript.clone()
    } else {
        format!("#{}", m.index)
    }
}

const FAMILY: ColumnKey = ColumnKey::new("family");
const STYLE: ColumnKey = ColumnKey::new("style");
const POSTSCRIPT: ColumnKey = ColumnKey::new("postscript");
const GLYPHS: ColumnKey = ColumnKey::new("glyphs");

impl StackDialog for MembersDialog {
    /// `window.fileQuestion`'s parameters.
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<CollectionMembersWords>();
        let MembersDialog { question, chosen, seen, open, .. } = self;
        let question = &*question;
        let message = format!("{} {}", question.file, words.message.as_str());
        let shown =
            FormDialog::new(words.title.clone(), Bind::bound(seen)).opened(Bind::bound(open)).ok_label(words.open.clone()).show(ui, |ui, _, _| {
                Text::new(message.clone()).show(ui);
                let columns = vec![
                    ColumnSpec::new(FAMILY, words.family.clone(), ColumnWidth::Content),
                    ColumnSpec::new(STYLE, words.style.clone(), ColumnWidth::Content),
                    ColumnSpec::new(POSTSCRIPT, words.postscript.clone(), ColumnWidth::Content),
                    ColumnSpec::new(GLYPHS, words.glyphs.clone(), ColumnWidth::Content).numeric(),
                ];
                let members = &question.members;
                let positions: Vec<usize> = (0..members.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(members.get(*at).map_or_else(String::new, family_of)));
                let _ = Table::<usize>::new(words.fonts.clone(), columns)
                    .max_rows(LIST_ROWS)
                    .select(SelectMode::Multi)
                    .selection(Bind::bound(&mut *chosen))
                    .show_items(ui, &source, &mut |at, icx| {
                        let Some(m) = members.get(*at) else { return ListRow::new("") };
                        ListRow::new(family_of(m))
                            .cells(icx.cells())
                            .cell(STYLE, m.style.clone())
                            .cell(POSTSCRIPT, m.postscript.clone())
                            .cell(GLYPHS, m.glyphs.to_string())
                    });
                !chosen.items.is_empty()
            });
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(self.answer())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the question while it is the first one waiting, and posts its answer.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    let first = app.view.file_questions.first().map(|(serial, q)| (*serial, matches!(q, Question::Members(_))));
    if let Some((result, key)) = host.take_keyed::<MembersDialog>() {
        // A dialog that ended because its question was answered elsewhere posts nothing.
        if key.is_some() && key == first.map(|(serial, _)| serial.to_string()) {
            post(
                app,
                match result {
                    DialogResult::Ok(answer) => answer,
                    _ => json!({ "answer": "cancel" }),
                },
            );
        }
        return;
    }
    if pending(app) {
        return;
    }
    match host.get_mut::<MembersDialog>() {
        Some(dialog) => dialog.closing = first != Some((dialog.serial, true)),
        None => {
            if let Some((serial, Question::Members(q))) = app.view.file_questions.first() {
                host.open(MembersDialog::new(*serial, q.clone()));
            }
        }
    }
}
