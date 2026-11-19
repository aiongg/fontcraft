//! The unsaved-changes question (`app/close.rs`): one document at a time, Save, Don't Save or Cancel, with "Apply to
//! All" while more documents are left. An answer posts `window.closePrompt` as from the question's own buttons, so
//! Save may open the save picker. With "Apply to All" the same answer goes to each document left without asking.

use craft::quit::{CloseAnswer, CloseScope};
use craft::ui::dialog::{DialogHost, DialogResult, UnsavedChangesDialog};
use craft::ui::runtime::commands::CommandRef;
use serde_json::json;

use crate::app::close::{Answer, Item, Purpose};
use crate::app::{FontCraftApp, Msg};

/// The question as the dialog shows it: the document and how many are left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown(Option<(Item, usize)>);

fn post(app: &mut FontCraftApp, answer: Answer) {
    let word = match answer {
        Answer::Save => "save",
        Answer::DontSave => "dontSave",
        Answer::Cancel => "cancel",
    };
    app.closing.from_window = true;
    app.sent.push(Msg::Shell(CommandRef::with_params("window.closePrompt", json!({ "answer": word }))));
}

pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(result) = host.take::<UnsavedChangesDialog>() {
        app.view.unsaved_shown = Shown::default();
        let answer = match result {
            DialogResult::Ok(given) => {
                let answer = match given.answer {
                    CloseAnswer::Save => Answer::Save,
                    CloseAnswer::DontSave => Answer::DontSave,
                    _ => Answer::Cancel,
                };
                if given.apply_to_all && answer != Answer::Cancel {
                    app.closing.apply_all = Some(answer);
                }
                answer
            }
            _ => Answer::Cancel,
        };
        post(app, answer);
        return;
    }
    let pending = app.sent.iter().any(|m| matches!(m, Msg::Shell(c) if c.id == "window.closePrompt"));
    let current = if app.closing.saving() || pending { None } else { app.closing.current(&app.session) };
    let Some((item, name, left)) = current else { return };
    if let Some(answer) = app.closing.apply_all {
        post(app, answer);
        return;
    }
    let shown = Shown(Some((item, left)));
    if app.view.unsaved_shown == shown && host.is_open::<UnsavedChangesDialog>() {
        return;
    }
    let scope = if app.closing.purpose() == Some(Purpose::Close) { CloseScope::Close } else { CloseScope::Quit };
    if !host.is_open::<UnsavedChangesDialog>() {
        host.open(UnsavedChangesDialog::new(scope, name).remaining(left + 1));
        app.view.unsaved_shown = shown;
    }
}
