//! `fontcraft-cli tables`.

use super::Subcommand;
use super::options;
use crate::io::{Failure, Outcome, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "tables",
    summary: "Work on the tables of a compiled font",
    usage: USAGE,
    options: &[],
    actions: &["list", "extract", "add", "delete", "edit", "copy", "checksum", "import", "reorder", "show"],
    run: tables,
};

const USAGE: &str = "\
  fontcraft-cli tables list FONT [--sort file|tag|size] [--face N|all]
      Print the table directory of a compiled font, collection, WOFF or WOFF2 file as JSON.
  fontcraft-cli tables extract FONT TAG[=FILE]... [--output-dir DIR]
  fontcraft-cli tables add FONT TAG=FILE... [-o OUT | --in-place]
  fontcraft-cli tables delete FONT TAG... [-o OUT | --in-place]
  fontcraft-cli tables edit FONT [--extract TAG[=FILE],...] [--delete TAG,...] [--add TAG=FILE,...] [-o OUT | --in-place]
  fontcraft-cli tables copy FROM TO TAG... [-o OUT | --in-place]
  fontcraft-cli tables checksum FONT... [--fix] [-o OUT | --in-place]
  fontcraft-cli tables import FONT DUMP.ttx... [-o OUT | --in-place]
  fontcraft-cli tables reorder FONT [--order recommended|TAG,...] [-o OUT | --in-place]
  fontcraft-cli tables show FONT [TAG...] [--path PATH] [--depth N] [--offsets] [--format json|text]
";

fn tables(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("tables needs a subcommand: list")) };
    if verb == "-h" || verb == "--help" {
        return out(&super::help(&SUBCOMMAND));
    }
    if verb != "list" {
        // M12.2: the other actions.
        return super::not_built(&SUBCOMMAND, args);
    }
    let (files, mut params) = options(rest, &[("--sort", "sort"), ("--face", "face")], &[])?;
    let [font] = files.as_slice() else { return Err(usage("tables list needs one FONT")) };
    params.insert("font".into(), json!(font));
    let mut session = Session::new();
    let result = session.execute("tables.list", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&result).map_err(|e| Failure::Failed(e.to_string()))?)
}
