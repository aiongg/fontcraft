//! Paste Kerning: existing pairs. Asked after `kerning.pastePairs` with `onConflict: "ask"`
//! found pasted pairs that the master has with other values, and so pasted nothing. The dialog
//! lists those pairs; Overwrite runs the paste again replacing them, Keep Existing runs it adding
//! only the pairs the master lacks, and Cancel pastes nothing at all. Nothing runs until a
//! button is pressed. [`follow_up`] gives the parameters to open it with from the request.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{After, Dialog, Fields, Mode, Third};

/// Most conflicting pairs the list shows (the engine reports at most this many).
const MAX_ROWS: usize = 1000;

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    follow_up: true,
    after: Some(After { command: "kerning.pastePairs", params: follow_up }),
    buttons: ["Overwrite", "Cancel"],
    third: Some(Third { id: "keepExisting", caption: "Keep Existing", key: "onConflict", ok: "overwrite", value: "keepExisting", ok_caption: None }),
    message: "Some pasted pairs exist in this master with other values.",
    ..Dialog::new("dialog.pasteKerningConflict", "Paste Kerning: existing pairs", "kerning.pastePairs", Fields::Targeted(fields))
};

/// The conflicting pairs as a list, and the answer the buttons give.
fn fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    if !targets.contains_key("text") && !targets.contains_key("pairs") {
        return Err("Paste Kerning: give the pasted pairs as `text` or `pairs`".into());
    }
    let report = asked(session, targets)?;
    let conflicts = report.get("conflicts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let count = report.get("conflictCount").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(conflicts.len());
    let rows: Vec<String> = conflicts.iter().take(MAX_ROWS).map(row).collect();
    let label = match count {
        1 => "1 pair exists with another value".to_string(),
        n if n > rows.len() => format!("{n} pairs exist with other values; the first {} are listed", rows.len()),
        n => format!("{n} pairs exist with other values"),
    };
    Ok(vec![
        Field::rows("conflicts", &label, rows, "each pasted pair the master has with another value: left, right, pasted value, value now"),
        Field::choice(
            "onConflict",
            "",
            &[("overwrite", "Overwrite"), ("keepExisting", "Keep Existing")],
            "overwrite",
            "overwrite pastes every pair and replaces the conflicting ones; keepExisting pastes only the pairs the master lacks",
        ),
    ])
}

/// The paste's report with `onConflict: "ask"`, run on a preview that is put back at once, so
/// the font does not change even when nothing conflicts.
fn asked(session: &mut Session, targets: &Map<String, Value>) -> Result<Value, String> {
    let mut params = targets.clone();
    params.insert("onConflict".into(), json!("ask"));
    params.remove("conflicts");
    session.begin("Paste Kerning Pairs").map_err(|e| e.to_string())?;
    let report = session.preview("kerning.pastePairs", &Value::Object(params));
    session.cancel().map_err(|e| e.to_string())?;
    report.map_err(|e| e.to_string())
}

/// One conflict as a list row: `left  right  pasted (now existing)`.
fn row(conflict: &Value) -> String {
    let text = |key: &str| conflict.get(key).and_then(Value::as_str).unwrap_or_default();
    let number = |key: &str| conflict.get(key).and_then(Value::as_f64).map(super::form::format_number).unwrap_or_default();
    format!("{}  {}  {} (now {})", text("left"), text("right"), number("value"), number("existing"))
}

/// The parameters to open the question with after `kerning.pastePairs` ran with `request` and
/// returned `result`: the request with Overwrite as the answer. `None` unless the request asked
/// (`onConflict` "ask" or left out) and the result says it did.
pub fn follow_up(request: &Value, result: &Value) -> Option<Map<String, Value>> {
    let request = request.as_object()?;
    if !matches!(request.get("onConflict").and_then(Value::as_str), None | Some("ask")) {
        return None;
    }
    if result.get("asked").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let mut m = request.clone();
    m.insert("onConflict".into(), json!("overwrite"));
    Some(m)
}
