//! `ui.screenshot {path?}`: the window captured as PNG through the screenshot queue of `craft::automation`,
//! written to `path` or returned as base64 `png`; `{width, height, pixelsPerPoint, path? | png?}`.

use std::sync::mpsc::Sender;

use craft::automation::screenshot::{Finished, screenshot_reply};
use fontcraft_engine::file_access;
use serde_json::{Value, json};

/// The path `ui.screenshot` writes, from its params. The frame is written later, after the roots are out of force,
/// so while they are in force this is the absolute path checked against the write root; without them, the path as
/// given.
pub fn path(params: &Value) -> Result<Option<String>, String> {
    let path = match params.get("path") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(p)) if !p.is_empty() => p,
        Some(_) => return Err("`path` must be a file path".into()),
    };
    let Some(roots) = file_access::current() else { return Ok(Some(path.clone())) };
    let resolved = roots.resolved(path, file_access::Access::Write)?;
    resolved.to_str().map(|p| Some(p.to_string())).ok_or_else(|| format!("{path} does not resolve to a UTF-8 path"))
}

/// Answers finished captures: the PNG written or inline under `png`, with the window's pixels per point.
pub fn finish(finished: Vec<Finished>, pixels_per_point: f32) {
    let mut write = |path: &str, bytes: &[u8]| fontcraft_engine::extensions::write_file(std::path::Path::new(path), bytes);
    for shot in finished {
        let reply = match shot.result {
            Ok(image) => {
                let mut reply = screenshot_reply(shot.token, image, shot.path.as_deref(), &mut write);
                if let Some(result) = reply.get_mut("result").and_then(Value::as_object_mut) {
                    if let Some(png) = result.remove("base64") {
                        result.insert("png".into(), png);
                    }
                    result.remove("mime");
                    result.insert("pixelsPerPoint".into(), json!(pixels_per_point));
                }
                reply
            }
            Err(error) => json!({"ok": false, "error": format!("ui.screenshot: {error}")}),
        };
        send(shot.reply.as_ref(), reply);
    }
}

fn send(to: Option<&Sender<Value>>, reply: Value) {
    if let Some(to) = to {
        let _ = to.send(reply);
    }
}
