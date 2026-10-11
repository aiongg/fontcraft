//! `fontcraft-cli glyphs`.

use super::Subcommand;
use super::options;
use crate::io::{Failure, Outcome, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "glyphs",
    summary: "List, rename and reorder glyphs",
    usage: USAGE,
    options: &[],
    actions: &["list", "rename", "reorder"],
    run: glyphs,
};

const USAGE: &str = "\
  fontcraft-cli glyphs list FONT [--face N]
      Print the glyphs of FONT (a compiled font or a source) in glyph order as JSON.
  fontcraft-cli glyphs rename FONT (--goadb FILE | --map FILE | --production-names | OLD=NEW...) [-o OUT | --in-place]
  fontcraft-cli glyphs reorder FONT (--order-file FILE | --goadb FILE | --sort KEY | NAME...) [-o OUT | --in-place]
";

fn glyphs(args: &[String]) -> Outcome {
    let Some((verb, rest)) = args.split_first() else { return Err(usage("glyphs needs a subcommand: list")) };
    if verb == "-h" || verb == "--help" {
        return out(&super::help(&SUBCOMMAND));
    }
    if verb != "list" {
        // M12.15: the other actions.
        return super::not_built(&SUBCOMMAND, args);
    }
    let (files, mut params) = options(rest, &[("--face", "face")], &[])?;
    let [font] = files.as_slice() else { return Err(usage("glyphs list needs one FONT")) };
    params.insert("font".into(), json!(font));
    let mut session = Session::new();
    let result = session.execute("font.listGlyphs", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&result).map_err(|e| Failure::Failed(e.to_string()))?)
}
