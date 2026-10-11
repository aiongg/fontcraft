//! `fontcraft-cli info`.

use super::Subcommand;
use super::options;
use crate::io::{Failure, Outcome, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "info", summary: "Describe a font or source as JSON", usage: USAGE, options: &[], actions: &[], run: info };

const USAGE: &str = "\
  fontcraft-cli info FILE [--section NAME,...] [--all-names] [--face N]
      Print a JSON summary of FILE (a compiled font or a source) without opening it: format, names,
      metrics, style, tables, cmap, layout, axes, instances, masters, color, hinting, faces.
";

fn info(args: &[String]) -> Outcome {
    let (files, mut params) = options(args, &[("--section", "sections"), ("--face", "face")], &[("--all-names", "allNames")])?;
    let [input] = files.as_slice() else { return Err(usage("info needs one FILE")) };
    params.insert("input".into(), json!(input));
    let mut session = Session::new();
    let summary = session.execute("font.inspect", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&summary).map_err(|e| Failure::Failed(e.to_string()))?)
}
