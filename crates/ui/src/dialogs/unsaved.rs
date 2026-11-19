//! The unsaved-changes question when the window closes.

use craft::quit::{CloseAnswer, CloseScope};
use craft::ui::dialog::{DialogHost, DialogResult, UnsavedChangesDialog};
use serde_json::json;

use crate::app::{Closing, FontCraftApp, Msg};

pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    match app.closing {
        Closing::Ask => {
            let name = app.session.documents().iter().find(|d| d.modified()).map(|d| d.title.clone()).unwrap_or_default();
            host.open(UnsavedChangesDialog::new(CloseScope::Quit, name));
            app.closing = Closing::Asking;
        }
        Closing::Asking => {
            let Some(result) = host.take::<UnsavedChangesDialog>() else { return };
            app.closing = match result {
                DialogResult::Ok(answer) => match answer.answer {
                    CloseAnswer::Save => {
                        app.sent.push(Msg::run("file.save", json!({})));
                        Closing::SaveThenClose
                    }
                    CloseAnswer::DontSave => Closing::Confirmed,
                    _ => Closing::Idle,
                },
                _ => Closing::Idle,
            };
        }
        _ => {}
    }
}
