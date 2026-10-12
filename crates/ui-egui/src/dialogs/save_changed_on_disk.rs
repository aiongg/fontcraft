//! Save: the file changed on disk. Asked when `file.save` refuses because another program
//! changed the document's file since it was read. Overwrite runs the save again with
//! `force: true`, so the person can always save their work; Cancel leaves the file and the
//! document as they are (File › Review External Changes… can then take the other program's
//! changes in).

use serde_json::{Map, Value, json};

use super::{Dialog, Fields, Mode};

/// The start of `file.save`'s refusal when the file changed on disk.
const REFUSAL: &str = "changed on disk since it was opened";

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    follow_up: true,
    buttons: ["Overwrite", "Cancel"],
    message: "The file changed on disk since it was opened. Overwrite it with this document?",
    ..Dialog::new("dialog.saveChangedOnDisk", "Save: file changed on disk", "file.save", Fields::Fixed(|_| Vec::new()))
};

/// The parameters to open the question with after `file.save` ran with `request` and failed
/// with `error`: the request with `force: true`. `None` for any other failure.
pub fn after_error(command: &str, request: &Value, error: &str) -> Option<Map<String, Value>> {
    if command != "file.save" || !error.contains(REFUSAL) {
        return None;
    }
    let mut given = request.as_object().cloned().unwrap_or_default();
    given.insert("force".into(), json!(true));
    Some(given)
}
