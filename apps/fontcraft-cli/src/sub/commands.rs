//! `fontcraft-cli commands`.

use super::Subcommand;
use crate::io::{Failure, Outcome, execute, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "commands", summary: "Print the command catalogue as JSON", usage: USAGE, options: &[], actions: &[], run: commands };

const USAGE: &str = "\
  fontcraft-cli commands [PATTERN]
      Print the command catalogue as JSON (only commands whose id or label contains PATTERN).
";

fn commands(args: &[String]) -> Outcome {
    let pattern = match args {
        [] => Value::Null,
        [pattern] => json!(pattern),
        _ => return Err(usage("commands takes at most one PATTERN")),
    };
    let list = execute(&mut Session::new(), "commands.list", &json!({"pattern": pattern}))?;
    out(&serde_json::to_string_pretty(&list).map_err(|e| Failure::Failed(e.to_string()))?)
}
