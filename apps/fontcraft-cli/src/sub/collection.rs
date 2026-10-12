//! `fontcraft-cli collection`: `list`, `pack` and `unpack`, over the engine commands
//! `collection.list`, `collection.pack` and `collection.unpack`.

use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "collection",
    summary: "List, pack and unpack font collections",
    usage: USAGE,
    options: OPTIONS,
    actions: &["list", "pack", "unpack"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli collection list|pack|unpack FILE... [args]
  fontcraft-cli collection [list] FILE.ttc [--json]
      The members of a collection (or a WOFF2 collection): index, PostScript name, outlines and
      tables; tables another member shares are marked with *.
  fontcraft-cli collection pack FONT FONT... -o OUT.ttc [--share-table TAG=INDEX]... [--no-share] [--container plain|woff2]
      A collection of the fonts in order; members of input collections are appended in order.
  fontcraft-cli collection unpack FILE.ttc [--face N|NAME] [--output-dir DIR] [-o OUT] [--name-by postscript-name|index]
      Each member as a font of its own, named after its PostScript name.
";

/// The options, as data (M12.3).
const OPTIONS: &[OptSpec] = &[
    OptSpec::value("output", "FILE", "", "the output file (pack; unpack with one --face)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder unpacked fonts go into (default: beside the collection)"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing file"),
    OptSpec::value("face", "N|NAME", "", "unpack only this member, by index or PostScript name"),
    OptSpec::list("share-table", "TAG=INDEX", "", "pack: every member uses table TAG of input font INDEX (zero-based); repeatable"),
    OptSpec::switch("share", true, "pack: store identical tables once (--no-share stores every table separately)"),
    OptSpec::value("container", "plain|woff2", "plain", "pack: write a plain collection or a WOFF2 collection"),
    OptSpec::value("name-by", "postscript-name|index", "postscript-name", "unpack: name files by PostScript name or as STEM-fontN"),
    OptSpec::switch("json", false, "print the engine's result as JSON"),
    OptSpec::switch("quiet", false, "print nothing but errors").short('q'),
    OptSpec::switch("verbose", false, "print more detail on stderr").short('v'),
];

fn run(raw: &[String]) -> Outcome {
    let merged = merge_repeated(raw, "share-table");
    let parsed = args::parse(OPTIONS, &merged)?;
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
    let Some((first, rest)) = parsed.positional.split_first() else { return Err(usage(super::help(&SUBCOMMAND))) };
    let (action, files): (&str, &[String]) = if SUBCOMMAND.actions.contains(&first.as_str()) {
        (first.as_str(), rest)
    } else if std::path::Path::new(first).is_file() {
        ("list", parsed.positional.as_slice())
    } else {
        return Err(usage(format!("unknown collection action `{first}` (actions: {})", SUBCOMMAND.actions.join(", "))));
    };
    for f in files {
        if !std::path::Path::new(f).is_file() {
            return Err(Failure::Input(format!("cannot read {f}: not a file")));
        }
    }
    let json_out = parsed.switch("json");
    let mut session = Session::new();
    match action {
        "list" => {
            if files.is_empty() {
                return Err(usage("collection list needs FILE"));
            }
            for file in files {
                let result = io::execute(&mut session, "collection.list", &json!({ "file": file }))?;
                if json_out {
                    io::out_json(&result)?;
                } else {
                    print_list(file, &result)?;
                }
            }
            Ok(())
        }
        "pack" => {
            if files.is_empty() {
                return Err(usage("collection pack needs at least one FONT"));
            }
            let output = parsed.value("output").ok_or_else(|| usage("collection pack needs -o OUT"))?;
            let mut forced = Vec::new();
            for item in parsed.list("share-table") {
                let (tag, index) = item.rsplit_once('=').ok_or_else(|| usage(format!("--share-table takes TAG=INDEX, got `{item}`")))?;
                let index: u64 = index.trim().parse().map_err(|_| usage(format!("--share-table: `{index}` is not a font index")))?;
                forced.push(json!({ "tag": tag.trim_matches(|c| c == '\'' || c == '"'), "index": index }));
            }
            let params = json!({
                "fonts": files,
                "output": output,
                "share": parsed.switch("share"),
                "shareTables": forced,
                "container": parsed.value("container").unwrap_or("plain"),
                "noClobber": parsed.switch("no-clobber"),
            });
            let result = io::execute(&mut session, "collection.pack", &params)?;
            if json_out {
                return io::out_json(&result);
            }
            io::note(&format!("packed {} fonts into {}", result.get("fonts").unwrap_or(&serde_json::Value::Null), output));
            if let Some(saved) = result.get("savedBySharing").and_then(|s| s.as_object()).filter(|s| !s.is_empty()) {
                let shared: Vec<String> = saved.iter().map(|(tag, bytes)| format!("{tag} ({bytes} bytes)")).collect();
                io::detail(&format!("shared: {}", shared.join(", ")));
            }
            Ok(())
        }
        _ => {
            let [file] = files else { return Err(usage("collection unpack needs one FILE")) };
            let mut params = json!({
                "file": file,
                "nameBy": parsed.value("name-by").unwrap_or("postscript-name"),
                "noClobber": parsed.switch("no-clobber"),
            });
            if let Some(obj) = params.as_object_mut() {
                if let Some(face) = parsed.value("face") {
                    obj.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
                }
                if let Some(dir) = parsed.value("output-dir") {
                    obj.insert("outputDir".into(), json!(dir));
                }
                if let Some(o) = parsed.value("output") {
                    obj.insert("output".into(), json!(o));
                }
            }
            let result = io::execute(&mut session, "collection.unpack", &params)?;
            if json_out {
                return io::out_json(&result);
            }
            for o in result.get("outputs").and_then(|o| o.as_array()).map(Vec::as_slice).unwrap_or_default() {
                out(o.get("path").and_then(|p| p.as_str()).unwrap_or_default())?;
            }
            Ok(())
        }
    }
}

fn print_list(file: &str, result: &Value) -> Outcome {
    let members = result["members"].as_array().map(Vec::as_slice).unwrap_or_default();
    out(&format!("{file}: {} font{}", members.len(), if members.len() == 1 { "" } else { "s" }))?;
    for m in members {
        let shared: Vec<&str> = m["shared"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let tables: Vec<String> = m["tables"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).map(|t| if shared.contains(&t) { format!("{t}*") } else { t.to_string() }).collect())
            .unwrap_or_default();
        out(&format!(
            "  {:>3}  {:<32}  {:<8}  {}",
            m["index"].as_u64().unwrap_or(0),
            m["postscriptName"].as_str().unwrap_or("-"),
            m["flavour"].as_str().unwrap_or("?"),
            tables.join(" ")
        ))?;
    }
    Ok(())
}

/// The values of every `--NAME V` and `--NAME=V` in `args` joined into one `--NAME=V1,V2`, so a
/// list option can be repeated.
pub(super) fn merge_repeated(args: &[String], name: &str) -> Vec<String> {
    let flag = format!("--{name}");
    let prefix = format!("--{name}=");
    let mut values: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::with_capacity(args.len());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            rest.push(a.clone());
            rest.extend(it.by_ref().cloned());
            break;
        }
        if *a == flag {
            match it.next() {
                Some(v) => values.push(v.clone()),
                None => rest.push(a.clone()),
            }
        } else if let Some(v) = a.strip_prefix(&prefix) {
            values.push(v.to_string());
        } else {
            rest.push(a.clone());
        }
    }
    if !values.is_empty() {
        rest.push(format!("{prefix}{}", values.join(",")));
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn collection_cli_merges_repeated_share_tables_and_refuses_bad_lines() {
        assert_eq!(
            merge_repeated(&s(&["pack", "--share-table", "CFF =2", "a.otf", "--share-table=name=0", "-o", "x.ttc"]), "share-table"),
            s(&["pack", "a.otf", "-o", "x.ttc", "--share-table=CFF =2,name=0"])
        );
        let e = run(&s(&["bogus"])).unwrap_err();
        assert_eq!(e.exit_code(), 2, "an unknown action is a usage error");
        let e = run(&s(&["pack", "missing.ttf", "-o", "x.ttc"])).unwrap_err();
        assert_eq!(e.exit_code(), 3, "an unreadable input");
        assert!(run(&s(&["--help"])).is_ok());
    }
}
