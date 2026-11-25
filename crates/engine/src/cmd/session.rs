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
            "{label?: string, steps: [{command: id, params?: object}]} run the steps in order as one transaction: if one fails, none takes effect (files the steps before it wrote stay, and the error names them). The edits are one undo step per document, labelled `label` (default \"Batch\"); undo and redo cannot be steps → {results: [value]}",
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
        cmd!(
            "document.showNext",
            "Show Next Document",
            ["Window"],
            Some("Cmd+`"),
            "{} make the open document after the active one the active one; after the last comes the first → {title, index}",
            has_doc,
            show_next
        ),
        cmd!(
            "document.showPrevious",
            "Show Previous Document",
            ["Window"],
            Some("Cmd+Shift+`"),
            "{} make the open document before the active one the active one; before the first comes the last → {title, index}",
            has_doc,
            show_previous
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
    one_step_naming_files(s, label, "command.batch", |s| run_steps(s, &p, steps, &mut results))?;
    Ok(json!({ "results": results }))
}

/// Runs the steps `f` as one undo step of `command` ([`Session::one_step`]). When they fail or are cancelled, the
/// documents are put back but files are not: the error names the files the steps wrote. Inside a running batch the
/// batch names them.
pub(super) fn one_step_naming_files<T>(s: &mut Session, label: &str, command: &str, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let outermost = s.batch_is_idle();
    let (outcome, written) = fontcraft_format::written::collect(|| s.one_step(label, f));
    if !outermost || written.count == 0 {
        return outcome;
    }
    // A cancel after the last step is either noticed here or too late.
    outcome.and_then(|out| crate::work::seal().map(|()| out)).map_err(|e| {
        let why = match e {
            crate::EngineError::BadParams { message, .. } => message,
            e => e.to_string(),
        };
        let message = format!("{why}; the files its steps wrote stay: {}", files_named(&written));
        crate::EngineError::BadParams { command: command.into(), message }
    })
}

/// The names of the files written, and how many more there are.
fn files_named(written: &fontcraft_format::written::Written) -> String {
    /// Most files named.
    const MAX_NAMED: usize = 20;
    let mut names: Vec<String> = written.files.iter().take(MAX_NAMED).map(|f| f.display().to_string()).collect();
    match written.count.saturating_sub(names.len()) {
        0 => {}
        more => names.push(format!("{more} more")),
    }
    names.join(", ")
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

fn show_next(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("document.showNext", p)?;
    show_beside(s, &p, true)
}

fn show_previous(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("document.showPrevious", p)?;
    show_beside(s, &p, false)
}

/// Makes the document after (`next`) or before the active one active, around the ends of the list.
fn show_beside(s: &mut Session, p: &Params, next: bool) -> Result<Value> {
    let active = s.doc()?.id;
    let count = s.documents().len();
    let at = s.documents().iter().position(|d| d.id == active).ok_or(crate::EngineError::NoDocument)?;
    let step = if next { 1 } else { count.saturating_sub(1) };
    // An active document is in the list, so `count` is at least 1.
    let index = at.saturating_add(step).checked_rem(count).unwrap_or(at);
    s.set_active(index).map_err(|_| p.bad(format!("there is no open document {index}")))?;
    Ok(json!({ "title": s.doc()?.title, "index": index }))
}
