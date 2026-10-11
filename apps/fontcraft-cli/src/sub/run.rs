//! `fontcraft-cli run`.

use super::Subcommand;
use crate::io::{Failure, Outcome, execute, open, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "run", summary: "Run engine commands headless on a source", usage: USAGE, options: &[], actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli run [--in FILE] [--cmd ID [--params JSON]]... [--save [FILE]] [--export FILE]
      Headless batch: open FILE (a source) or start a new font, run the commands in order and
      print one JSON result per line. Stops at the first command that fails. --save writes the
      document (to FILE, or back to the file it was opened from); --export compiles it to a font.
";

fn run(args: &[String]) -> Outcome {
    let mut input = None;
    let mut steps: Vec<(String, Value)> = Vec::new();
    let mut save: Option<Option<String>> = None;
    let mut export = None;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--in" => input = Some(it.next().ok_or_else(|| usage("--in needs a file"))?.clone()),
            "--cmd" => steps.push((it.next().ok_or_else(|| usage("--cmd needs a command id"))?.clone(), json!({}))),
            "--params" => {
                let text = it.next().ok_or_else(|| usage("--params needs a JSON object"))?;
                let value: Value = serde_json::from_str(text).map_err(|e| usage(format!("--params is not valid JSON: {e}")))?;
                match steps.last_mut() {
                    Some(step) => step.1 = value,
                    None => return Err(usage("--params must follow a --cmd")),
                }
            }
            "--save" => save = Some(it.next_if(|next| !next.starts_with("--")).cloned()),
            "--export" => export = Some(it.next().ok_or_else(|| usage("--export needs a file"))?.clone()),
            other => return Err(usage(format!("unknown run option `{other}`"))),
        }
    }
    let mut session = Session::new();
    match &input {
        Some(path) => open(&mut session, path)?,
        None => execute(&mut session, "file.new", &json!({}))?,
    };
    for (id, params) in &steps {
        let result = execute(&mut session, id, params).map_err(|e| match e {
            Failure::Failed(message) => Failure::Failed(format!("step `{id}` failed, nothing after it ran: {message}")),
            other => other,
        })?;
        out(&json!({"command": id, "result": result}).to_string())?;
    }
    if let Some(path) = save {
        let result = execute(&mut session, "file.save", &json!({"path": path}))?;
        out(&json!({"command": "file.save", "result": result}).to_string())?;
    }
    if let Some(path) = export {
        let result = execute(&mut session, "file.exportStatic", &json!({"destination": path}))?;
        out(&json!({"command": "file.exportStatic", "result": result}).to_string())?;
    }
    Ok(())
}
