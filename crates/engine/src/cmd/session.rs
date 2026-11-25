//! Commands about the session: running several commands as one step, switching documents.

use serde_json::{Value, json};

use super::shared::find_open;
use super::*;
use crate::params::Params;

/// Most steps one batch may have.
pub const MAX_BATCH_STEPS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "command.batch",
            "Run Commands as One Step",
            [],
            None,
            "{label?: string, steps: [{command: id, params?: object}]} run the steps in order as one transaction: if one fails, none takes effect. The edits are one undo step per document, labelled `label` (default \"Batch\"); undo and redo cannot be steps → {results: [value]}",
            always,
            batch
        ),
        cmd!(
            "document.activate",
            "Show Document",
            [],
            None,
            "{index?: number, path?: string} make an open document the active one, by its position among the open documents (document.inspect lists them) or the path it was opened from → {title, index}",
            has_doc,
            activate
        ),
    ]
}

fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("command.batch", p)?;
    let label = p.opt_str("label")?.unwrap_or("Batch");
    let steps = p.array("steps")?;
    if steps.len() > MAX_BATCH_STEPS {
        return Err(p.bad(format!("`steps` has {} entries; at most {MAX_BATCH_STEPS}", steps.len())));
    }
    // A batch inside a batch is part of the outer one.
    let mut results = Vec::with_capacity(steps.len());
    s.one_step(label, |s| run_steps(s, &p, steps, &mut results))?;
    Ok(json!({ "results": results }))
}

fn run_steps(s: &mut Session, p: &Params, steps: &[Value], results: &mut Vec<Value>) -> Result<()> {
    crate::work::plan(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let id = step.get("command").and_then(Value::as_str).ok_or_else(|| p.bad(format!("step {i} has no `command`")))?;
        if matches!(id, "edit.undo" | "edit.redo") {
            return Err(p.bad(format!("step {i}: `{id}` cannot be part of a batch")));
        }
        let mut params = step.get("params").cloned().unwrap_or_else(|| json!({}));
        // Each step is a request of its own while automation roots are in force.
        if let (Some(roots), Some(spec)) = (crate::file_access::current(), super::registry::find(id)) {
            params = crate::file_access::command::check(s, spec, &params, &roots).map_err(|e| p.bad(format!("step {i} (`{id}`) failed: {e}")))?;
        }
        let value = s.execute(id, &params).map_err(|e| p.bad(format!("step {i} (`{id}`) failed: {e}")))?;
        results.push(value);
        crate::work::advance(1, id);
    }
    Ok(())
}

fn activate(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("document.activate", p)?;
    let index = match (p.opt_int("index", 0, i64::from(u32::MAX))?, p.opt_str("path")?) {
        (Some(i), None) => usize::try_from(i).map_err(|_| p.bad("`index` is out of range"))?,
        (None, Some(path)) => find_open(s, std::path::Path::new(path)).ok_or_else(|| p.bad(format!("no open document was opened from {path}")))?,
        _ => return Err(p.bad("give either `index` or `path`")),
    };
    s.set_active(index).map_err(|_| p.bad(format!("there is no open document {index}")))?;
    let doc = s.doc()?;
    Ok(json!({ "title": doc.title, "index": index }))
}
