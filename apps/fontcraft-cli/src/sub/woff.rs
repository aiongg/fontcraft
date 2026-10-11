//! `fontcraft-cli woff`: `compress` and `decompress`, over the engine commands `woff.compress`
//! and `woff.decompress`. Several inputs run as a batch (`--jobs`), printed in input order.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "woff",
    summary: "Compress fonts to WOFF or WOFF2 and back",
    usage: USAGE,
    options: OPTIONS,
    actions: &["compress", "decompress"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli woff compress|decompress FONT... [options]
  fontcraft-cli woff compress FONT... [--format woff2|woff] [--no-glyf-transform] [--hmtx-transform] [--metadata FILE] [--private-data FILE] [-o OUT]
      Each font or collection (collections only in WOFF2) as FONT.woff2 or FONT.woff.
  fontcraft-cli woff decompress FONT.woff2|FONT.woff... [-o OUT] [--extract-metadata FILE] [--extract-private-data FILE]
      The font inside, named .ttf, .otf or .ttc by its content.
";

/// The options, as data (M12.3).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("output", "FILE", "", "the output file (one input only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into (default: beside each input)"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing file"),
    OptSpec::value("format", "woff2|woff", "woff2", "compress: the container to write"),
    OptSpec::switch("glyf-transform", true, "compress: apply the WOFF2 glyf and loca transform"),
    OptSpec::switch("hmtx-transform", false, "compress: apply the WOFF2 hmtx transform"),
    OptSpec::value("quality", "N", "", "compress: Brotli quality for WOFF2 (11) or zlib level for WOFF (6); only these are written"),
    OptSpec::switch("zopfli", false, "compress: the zopfli compressor for WOFF (not available yet)"),
    OptSpec::value("metadata", "FILE", "", "compress: an extended metadata XML block to embed"),
    OptSpec::value("private-data", "FILE", "", "compress: a private data block to embed"),
    OptSpec::value("extract-metadata", "FILE", "", "decompress: write the metadata block, when there is one, to FILE"),
    OptSpec::value("extract-private-data", "FILE", "", "decompress: write the private data block, when there is one, to FILE"),
    OptSpec::value("jobs", "N", "1", "inputs worked on at the same time").short('j'),
    OptSpec::switch("json", false, "print each result as a JSON line"),
    OptSpec::switch("quiet", false, "print nothing but errors").short('q'),
    OptSpec::switch("verbose", false, "print more detail on stderr").short('v'),
];

fn run(raw: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, raw)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    io::set_verbosity(if parsed.switch("quiet") {
        0
    } else if parsed.switch("verbose") {
        2
    } else {
        1
    });
    let Some((action, files)) = parsed.positional.split_first() else { return Err(usage(super::help(&SUBCOMMAND))) };
    let command = match action.as_str() {
        "compress" => "woff.compress",
        "decompress" => "woff.decompress",
        other => return Err(usage(format!("unknown woff action `{other}` (actions: compress, decompress)"))),
    };
    if files.is_empty() {
        return Err(usage(format!("woff {action} needs at least one FONT")));
    }
    if files.len() > 1 && ["output", "extract-metadata", "extract-private-data"].iter().any(|o| parsed.value(o).is_some()) {
        return Err(usage("-o, --extract-metadata and --extract-private-data name one file: give one FONT"));
    }
    let jobs: usize = parsed.value("jobs").unwrap_or("1").parse().map_err(|_| usage("--jobs takes a number"))?;
    let mut base = Map::new();
    let mut put = |key: &str, value: Value| {
        base.insert(key.to_string(), value);
    };
    for (option, key) in [("output", "output"), ("output-dir", "outputDir")] {
        if let Some(v) = parsed.value(option) {
            put(key, json!(v));
        }
    }
    put("noClobber", json!(parsed.switch("no-clobber")));
    if command == "woff.compress" {
        let format = parsed.value("format").unwrap_or("woff2");
        put("format", json!(format));
        if format == "woff2" {
            put("glyfTransform", json!(parsed.switch("glyf-transform")));
        }
        if parsed.switch("hmtx-transform") {
            put("hmtxTransform", json!(true));
        }
        if let Some(q) = parsed.value("quality") {
            put("quality", json!(q.parse::<i64>().map_err(|_| usage("--quality takes a number"))?));
        }
        if parsed.switch("zopfli") {
            put("zopfli", json!(true));
        }
        for (option, key) in [("metadata", "metadata"), ("private-data", "privateData")] {
            if let Some(v) = parsed.value(option) {
                put(key, json!(v));
            }
        }
    } else {
        for (option, key) in [("extract-metadata", "extractMetadata"), ("extract-private-data", "extractPrivateData")] {
            if let Some(v) = parsed.value(option) {
                put(key, json!(v));
            }
        }
    }
    let results = io::batch(files, jobs, |file| {
        if !std::path::Path::new(file).is_file() {
            return Err(Failure::Input(format!("cannot read {file}: not a file")));
        }
        let mut params = base.clone();
        params.insert("font".into(), json!(file));
        let mut session = Session::new();
        io::execute(&mut session, command, &Value::Object(params))
    })?;
    for (file, result) in files.iter().zip(&results) {
        match result {
            Ok(r) if parsed.switch("json") => io::out_json_line(r)?,
            Ok(r) => {
                io::detail(&format!("{file}: {} bytes", r["size"]));
                out(r["output"].as_str().unwrap_or_default())?;
            }
            Err(e) => io::note(&format!("{file}: {}", e.message())),
        }
    }
    io::batch_outcome(&results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn woff_cli_refuses_bad_lines() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(run(&s(&["squash", "a.ttf"])).unwrap_err().exit_code(), 2);
        assert_eq!(run(&s(&["compress"])).unwrap_err().exit_code(), 2);
        assert_eq!(run(&s(&["compress", "a.ttf", "b.ttf", "-o", "x.woff2"])).unwrap_err().exit_code(), 2, "-o with two inputs");
        assert_eq!(run(&s(&["compress", "no-such-font.ttf"])).unwrap_err().exit_code(), 3);
    }
}
