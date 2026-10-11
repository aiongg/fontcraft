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
    let Some((verb, rest)) = args.split_first() else { return Err(usage("glyphs needs a subcommand: list, rename or reorder")) };
    if verb == "-h" || verb == "--help" {
        return out(&super::help(&SUBCOMMAND));
    }
    match verb.as_str() {
        "rename" => return rename(rest),
        "reorder" => return reorder(rest),
        "list" => {}
        other => return Err(usage(format!("unknown glyphs action `{other}` (actions: list, rename, reorder)"))),
    }
    let (files, mut params) = options(rest, &[("--face", "face")], &[])?;
    let [font] = files.as_slice() else { return Err(usage("glyphs list needs one FONT")) };
    params.insert("font".into(), json!(font));
    let mut session = Session::new();
    let result = session.execute("font.listGlyphs", &Value::Object(params)).map_err(|e| Failure::Input(e.to_string()))?;
    out(&serde_json::to_string_pretty(&result).map_err(|e| Failure::Failed(e.to_string()))?)
}

/// Output options shared by `rename` and `reorder`.
const OUTPUT: [(&str, &str); 4] = [("-o", "output"), ("--output", "output"), ("--output-dir", "outputDir"), ("--face", "face")];
const OUTPUT_FLAGS: [(&str, &str); 2] = [("--in-place", "inPlace"), ("--no-clobber", "noClobber")];

fn execute(command: &str, params: serde_json::Map<String, Value>) -> Outcome {
    let mut session = Session::new();
    let result = session.execute(command, &Value::Object(params)).map_err(|e| match e {
        fontcraft_engine::EngineError::BadParams { .. } => usage(e.to_string()),
        fontcraft_engine::EngineError::Io(_) => Failure::Input(e.to_string()),
        _ => Failure::Failed(e.to_string()),
    })?;
    out(&serde_json::to_string_pretty(&result).map_err(|e| Failure::Failed(e.to_string()))?)
}

fn rename(args: &[String]) -> Outcome {
    let mut names = vec![("--goadb", "goadb"), ("--map", "map")];
    names.extend(OUTPUT);
    let mut flags =
        vec![("--production-names", "productionNames"), ("--reverse", "reverse"), ("--update-unicodes", "updateUnicodes"), ("--order", "order")];
    flags.extend(OUTPUT_FLAGS);
    let (files, mut params) = options(args, &names, &flags)?;
    let Some((font, pairs)) = files.split_first() else { return Err(usage("glyphs rename needs a FONT")) };
    params.insert("font".into(), json!(font));
    if !pairs.is_empty() {
        let mut renames = serde_json::Map::new();
        for pair in pairs {
            let Some((old, new)) = pair.split_once('=') else { return Err(usage(format!("`{pair}` is not OLD=NEW"))) };
            renames.insert(old.to_string(), json!(new));
        }
        params.insert("renames".into(), Value::Object(renames));
    }
    execute("font.renameGlyphs", params)
}

fn reorder(args: &[String]) -> Outcome {
    let mut names = vec![("--order-file", "orderFile"), ("--goadb", "goadb"), ("--sort", "sort")];
    names.extend(OUTPUT);
    let (files, mut params) = options(args, &names, &OUTPUT_FLAGS)?;
    let Some((font, order)) = files.split_first() else { return Err(usage("glyphs reorder needs a FONT")) };
    params.insert("font".into(), json!(font));
    if !order.is_empty() {
        params.insert("order".into(), json!(order));
    }
    execute("font.reorderGlyphs", params)
}
