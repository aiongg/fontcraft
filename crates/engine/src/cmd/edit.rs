//! Edit menu: undo and redo.

use serde_json::{Value, json};

use super::*;
use crate::{EngineError, HistoryEntry};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{} → {undone: label, revision}", can_undo, undo),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{} → {redone: label, revision}", can_redo, redo),
    ]
}

fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = s.doc_mut()?;
    let step = doc.history.undo.pop().ok_or_else(|| EngineError::Disabled("edit.undo".into(), "nothing to undo".into()))?;
    let current = std::mem::replace(&mut doc.font, step.font);
    doc.history.redo.push(HistoryEntry { label: step.label.clone(), font: current });
    doc.revision += 1;
    Ok(json!({ "undone": step.label.as_ref(), "revision": doc.revision }))
}

fn redo(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = s.doc_mut()?;
    let step = doc.history.redo.pop().ok_or_else(|| EngineError::Disabled("edit.redo".into(), "nothing to redo".into()))?;
    let current = std::mem::replace(&mut doc.font, step.font);
    doc.history.undo.push(HistoryEntry { label: step.label.clone(), font: current });
    doc.revision += 1;
    Ok(json!({ "redone": step.label.as_ref(), "revision": doc.revision }))
}
