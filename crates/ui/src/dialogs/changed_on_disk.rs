//! Save: the file changed on disk. Asked when `file.save` refuses because another program changed the document's
//! file since it was read. Replace runs the save again with `force: true` and the document it was asked for, so the
//! person can always save their work; Cancel leaves the file and the document as they are (File › Review External
//! Changes… can then take the other program's changes in). When another document is active by the time Replace is
//! pressed, the engine refuses the save and says so.

use craft::ui::dialog::{DialogHost, DialogResult, PromptChoice, StandardPrompt};
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};

/// The start of `file.save`'s refusal when the file changed on disk.
const REFUSAL: &str = "changed on disk since it was opened";

/// The question waiting to be asked or answered: the document's title and the save to run on Replace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asked {
    pub title: String,
    pub params: Map<String, Value>,
    pub shown: bool,
}

/// After `file.save` ran with `request` and failed with `error`: the refusal for a file changed on disk asks the
/// question about the active document.
pub fn after_error(app: &mut FontCraftApp, command: &str, request: &Value, error: &str) {
    if command != "file.save" || !error.contains(REFUSAL) {
        return;
    }
    let Some(doc) = app.session.active() else { return };
    let mut params = request.as_object().cloned().unwrap_or_default();
    params.insert("force".into(), json!(true));
    params.entry("document").or_insert(json!(doc.id));
    app.view.changed_on_disk = Some(Asked { title: doc.title.clone(), params, shown: false });
}

pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    let Some(asked) = app.view.changed_on_disk.as_mut() else { return };
    if !asked.shown {
        asked.shown = true;
        host.open(StandardPrompt::changed_on_disk(asked.title.clone()));
        return;
    }
    let Some(answer) = host.take::<StandardPrompt>() else { return };
    let asked = app.view.changed_on_disk.take();
    if let (DialogResult::Ok(answer), Some(asked)) = (answer, asked)
        && answer.choice == PromptChoice::Proceed
    {
        app.sent.push(Msg::run("file.save", Value::Object(asked.params)));
    }
}
