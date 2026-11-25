//! The question before a save or a UFO export that leaves data out (`app/file_questions.rs`): the list by kind, Save
//! Anyway (Export Anyway), Cancel and, for a UFO, whether smart components are drawn as outlines. An answer posts
//! `window.fileQuestion`.

use craft::behavior::{ColumnKey, ColumnWidth};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, ColumnSpec, Props as _, Show as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::Ui;
use serde_json::{Value, json};

use super::report_list::{self, Line};
use crate::app::file_questions::{LossQuestion, Question};
use crate::app::{FontCraftApp, Msg};

craft::ui::words! {
    /// The words of the question before a save that leaves data out.
    pub struct LossQuestionWords {
        title: "fontcraftLossQuestion.title" = "Some Data Will Be Left Out";
        /// After the document's title, before the format's name.
        cannot_hold: "fontcraftLossQuestion.cannotHold" = "has data that cannot be kept in this format:";
        kept: "fontcraftLossQuestion.kept" = "The open document keeps it until it is closed.";
        list: "fontcraftLossQuestion.list" = "Data left out";
        what: "fontcraftLossQuestion.what" = "What";
        count: "fontcraftLossQuestion.count" = "Count";
        place: "fontcraftLossQuestion.where" = "Where";
        outlines: "fontcraftLossQuestion.outlines" = "Draw smart components as outlines";
        save: "fontcraftLossQuestion.save" = "Save Anyway";
        export: "fontcraftLossQuestion.export" = "Export Anyway";
    }
}

/// The question, with the number `app/file_questions.rs` gave it.
pub struct LossDialog {
    id: DialogId,
    serial: u64,
    /// The serial as text: the host keeps it with a Cancel ([`DialogHost::take_keyed`]).
    key: String,
    question: LossQuestion,
    outlines: bool,
    /// The question was answered elsewhere: the next pass ends the dialog.
    closing: bool,
    open: bool,
}

impl LossDialog {
    pub fn new(serial: u64, question: LossQuestion) -> LossDialog {
        let outlines = question.outlines;
        LossDialog { id: dialog_id("fontcraft.lossQuestion"), serial, key: serial.to_string(), question, outlines, closing: false, open: true }
    }
}

const WHAT: ColumnKey = ColumnKey::new("what");
const COUNT: ColumnKey = ColumnKey::new("count");
const PLACE: ColumnKey = ColumnKey::new("where");

impl StackDialog for LossDialog {
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
        let words = Env::of(ui).words::<LossQuestionWords>();
        let LossDialog { question, outlines, open, .. } = self;
        let question = &*question;
        let message = format!("{} {} {}", question.title, words.cannot_hold.as_str(), question.format);
        let ok = if question.export() { words.export.clone() } else { words.save.clone() };
        let shown = FormDialog::new(words.title.clone(), Bind::bound(outlines)).opened(Bind::bound(open)).ok_label(ok).show(ui, |ui, _, outlines| {
            Text::new(message.clone()).show(ui);
            let columns = vec![
                ColumnSpec::new(WHAT, words.what.clone(), ColumnWidth::Content),
                ColumnSpec::new(COUNT, words.count.clone(), ColumnWidth::Content).numeric(),
                ColumnSpec::new(PLACE, words.place.clone(), ColumnWidth::Content),
            ];
            let lines: Vec<Line> =
                question.losses.iter().map(|l| Line::new(l.what.clone()).cell(COUNT, l.entries.to_string()).cell(PLACE, l.details.clone())).collect();
            let _ = report_list::show(ui, words.list.clone(), columns, &lines, None);
            if question.smart > 0 {
                let label = format!("{} ({})", words.outlines.as_str(), question.smart);
                let _ = Checkbox::new(Bind::bound(outlines)).label(label).show(ui);
            }
            Text::new(words.kept.clone()).shade(Shade::Muted).show(ui);
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(outlines), _) => {
                let mut answer = json!({ "answer": "proceed" });
                if self.question.smart > 0
                    && let Some(map) = answer.as_object_mut()
                {
                    map.insert("smartComponents".into(), json!(if outlines { "outlines" } else { "components" }));
                }
                Some(DialogResult::Ok(answer))
            }
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the question while it is the first one waiting, and posts its answer.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    app.view.file_questions.prune(&app.session);
    let first = app.view.file_questions.first().map(|(serial, q)| (*serial, matches!(q, Question::Losses(_))));
    if let Some((result, key)) = host.take_keyed::<LossDialog>() {
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
    match host.get_mut::<LossDialog>() {
        Some(dialog) => dialog.closing = first != Some((dialog.serial, true)),
        None => {
            if let Some((serial, Question::Losses(q))) = app.view.file_questions.first() {
                host.open(LossDialog::new(*serial, q.clone()));
            }
        }
    }
}

/// Posts the answer `params` to the first question.
pub(super) fn post(app: &mut FontCraftApp, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params("window.fileQuestion", params)));
}

/// Whether an answer was posted that the app has not taken yet.
pub(super) fn pending(app: &FontCraftApp) -> bool {
    app.sent.iter().any(|m| matches!(m, Msg::Shell(c) if c.id == "window.fileQuestion"))
}
