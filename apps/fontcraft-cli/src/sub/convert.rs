//! `fontcraft-cli convert`.

use super::Subcommand;
use crate::io::{Outcome, execute, open, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "convert",
    summary: "Convert a source to another format or compile it to one font",
    usage: USAGE,
    options: &[],
    actions: &[],
    run: convert,
};

const USAGE: &str = "\
  fontcraft-cli convert IN OUT [--smart-components components|outlines]
      Open IN (a source) and write OUT in the format its extension names (.fontcraft, .glyphs,
      .glyphspackage, .ufo for one master, .designspace with a UFO per master), or compile it to
      one static font (.ttf, .otf with CFF outlines, .woff, .woff2) of its first exportable
      instance. --smart-components: a UFO or designspace keeps smart components as components
      with their values (default) or holds them drawn as outlines.
";

fn convert(args: &[String]) -> Outcome {
    let mut files: Vec<&String> = Vec::new();
    let mut smart: Option<&str> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--smart-components" => match it.next().map(String::as_str) {
                Some(v @ ("components" | "outlines")) => smart = Some(v),
                _ => return Err(usage("--smart-components needs components or outlines")),
            },
            other if other.starts_with("--") => return Err(usage(format!("unknown convert option `{other}`"))),
            _ => files.push(arg),
        }
    }
    let [input, output] = files.as_slice() else { return Err(usage("convert needs IN and OUT")) };
    let mut session = Session::new();
    open(&mut session, input)?;
    let extension = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if smart.is_some() && !matches!(extension.as_str(), "ufo" | "designspace") {
        return Err(usage("--smart-components applies to .ufo and .designspace output"));
    }
    let result = match extension.as_str() {
        "ttf" | "otf" | "woff" | "woff2" => execute(&mut session, "file.exportStatic", &json!({"destination": output}))?,
        "fontcraft" | "glyphs" | "glyphspackage" | "ufo" | "designspace" => {
            let mut params = json!({"path": output});
            if let (Some(v), Value::Object(m)) = (smart, &mut params) {
                m.insert("smartComponents".into(), json!(v));
            }
            execute(&mut session, "file.saveAs", &params)?
        }
        other => {
            return Err(usage(format!(
                "cannot write `.{other}` yet (writable: .ttf, .otf, .woff, .woff2, .fontcraft, .glyphs, .glyphspackage, .ufo, .designspace; see ROADMAP.md)"
            )));
        }
    };
    out(&result.to_string())
}
