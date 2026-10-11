//! `fontcraft-cli sources`: `diff` and `merge` over the engine commands `source.diff` and
//! `source.merge`.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{Failure, Outcome, execute, out, out_json, usage};

use super::{Subcommand, help};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "sources", summary: "Compare and merge sources", usage: USAGE, options: OPTIONS, actions: &["diff", "merge"], run };

const USAGE: &str = "\
  fontcraft-cli sources (diff | merge) ...
  fontcraft-cli sources diff A B [--parts glyphs,info,masters,instances,axes,kerning,features] [--glyphs LIST] [--tolerance UNITS] [--format text|json]
  fontcraft-cli sources merge BASE OTHER (--take ID,... | --take-all) [--parts LIST] [--glyphs LIST] (-o OUT | --in-place)
";

/// Largest glyph file read by `--glyph-file`, in bytes.
const MAX_GLYPH_FILE: u64 = 16 << 20;

/// The options, as data.
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("parts", "LIST", "", "parts to compare: glyphs, info, masters, instances, axes, kerning, features (default all)"),
    OptSpec::list("glyphs", "LIST", "", "compare only these glyphs (names in A or B)"),
    OptSpec::value("glyph-file", "PATH", "", "read glyph names from a file, separated by commas or whitespace, # starts a comment"),
    OptSpec::value("tolerance", "UNITS", "", "coordinates closer than this count as equal (default 0)"),
    OptSpec::value("format", "text|json", "text", "diff: one line per change, or the result as JSON"),
    OptSpec::list("take", "ID,...", "", "merge: the change ids printed by sources diff"),
    OptSpec::switch("take-all", false, "merge: every change of the selected parts and glyphs"),
    OptSpec::value("output", "OUT", "", "merge: where to write the result (in BASE's format)").short('o'),
    OptSpec::switch("in-place", false, "merge: replace BASE"),
    OptSpec::switch("no-clobber", false, "merge: refuse to replace an existing output"),
];

fn run(argv: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, argv)?;
    if parsed.help {
        return out(&help(&SUBCOMMAND));
    }
    let Some((action, files)) = parsed.positional.split_first() else { return Err(usage(help(&SUBCOMMAND))) };
    match action.as_str() {
        "diff" => diff(&parsed, files),
        "merge" => merge(&parsed, files),
        other => Err(usage(format!("unknown sources action `{other}` (actions: diff, merge)"))),
    }
}

/// `parts`, `glyphs` and `tolerance`, as the engine commands take them.
fn common(parsed: &Parsed) -> Result<Map<String, Value>, Failure> {
    let mut params = Map::new();
    if !parsed.list("parts").is_empty() {
        params.insert("parts".into(), json!(parsed.list("parts")));
    }
    let mut glyphs: Vec<String> = parsed.list("glyphs").to_vec();
    if let Some(path) = parsed.value("glyph-file") {
        glyphs.extend(read_glyph_file(path)?);
    }
    if !glyphs.is_empty() || parsed.value("glyph-file").is_some() {
        params.insert("glyphs".into(), json!(glyphs));
    }
    if let Some(t) = parsed.value("tolerance") {
        let t: f64 = t.parse().map_err(|_| usage(format!("--tolerance needs a number, got `{t}`")))?;
        params.insert("tolerance".into(), json!(t));
    }
    Ok(params)
}

fn read_glyph_file(path: &str) -> Result<Vec<String>, Failure> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_GLYPH_FILE + 1).read_to_string(&mut text))
        .map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))?;
    if text.len() as u64 > MAX_GLYPH_FILE {
        return Err(Failure::Input(format!("{path} is larger than {MAX_GLYPH_FILE} bytes")));
    }
    Ok(glyph_names(&text))
}

/// Names separated by commas or whitespace; `#` starts a comment that runs to the end of the line.
fn glyph_names(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .flat_map(|l| l.split(|c: char| c == ',' || c.is_whitespace()))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn diff(parsed: &Parsed, files: &[String]) -> Outcome {
    let [a, b] = files else { return Err(usage("sources diff needs two sources: A B")) };
    let json_out = match parsed.value("format").unwrap_or("text") {
        "text" => false,
        "json" => true,
        other => return Err(usage(format!("--format must be text or json, got `{other}`"))),
    };
    let mut params = common(parsed)?;
    params.insert("a".into(), json!(a));
    params.insert("b".into(), json!(b));
    let result = execute(&mut Session::new(), "source.diff", &Value::Object(params))?;
    let changes = result.get("changes").and_then(Value::as_array).cloned().unwrap_or_default();
    if json_out {
        out_json(&result)?;
    } else {
        for c in &changes {
            out(&change_line(c))?;
        }
    }
    if changes.is_empty() { Ok(()) } else { Err(Failure::Found(format!("the sources differ in {} places", changes.len()))) }
}

/// One change as a line: id, kind, where, and short values.
fn change_line(c: &Value) -> String {
    let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default();
    let mut place = vec![s("part").to_string()];
    for (key, label) in [("glyph", "glyph "), ("master", "master "), ("field", "")] {
        if let Some(v) = c.get(key).and_then(Value::as_str) {
            place.push(format!("{label}{v}"));
        }
    }
    let mut line = format!("{}  {}  {}", s("id"), s("kind"), place.join(", "));
    let short = |v: Option<&Value>| v.map(Value::to_string).filter(|t| t.len() <= 60);
    match (short(c.get("a")), short(c.get("b"))) {
        (Some(x), Some(y)) => line.push_str(&format!(": {x} -> {y}")),
        (None, Some(y)) if c.get("a").is_none() => line.push_str(&format!(": {y}")),
        (Some(x), None) if c.get("b").is_none() => line.push_str(&format!(": {x}")),
        _ => {}
    }
    line
}

fn merge(parsed: &Parsed, files: &[String]) -> Outcome {
    let [base, other] = files else { return Err(usage("sources merge needs BASE and OTHER")) };
    let take = match (parsed.switch("take-all"), parsed.list("take")) {
        (true, []) => json!("all"),
        (false, []) => return Err(usage("sources merge needs --take ID,... or --take-all")),
        (false, ids) => json!(ids),
        (true, _) => return Err(usage("give --take or --take-all, not both")),
    };
    let mut params = common(parsed)?;
    params.insert("base".into(), json!(base));
    params.insert("other".into(), json!(other));
    params.insert("take".into(), take);
    match (parsed.value("output"), parsed.switch("in-place")) {
        (Some(_), true) => return Err(usage("give -o OUT or --in-place, not both")),
        (Some(o), false) => {
            params.insert("output".into(), json!(o));
        }
        (None, true) => {
            params.insert("inPlace".into(), json!(true));
        }
        (None, false) => return Err(usage("sources merge needs -o OUT or --in-place")),
    }
    if parsed.switch("no-clobber") {
        params.insert("noClobber".into(), json!(true));
    }
    let result = execute(&mut Session::new(), "source.merge", &Value::Object(params))?;
    out_json(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn glyph_files_split_on_commas_whitespace_and_comments() {
        assert_eq!(glyph_names("A, B\n# skipped\nC D # E\n"), ["A", "B", "C", "D"]);
    }

    /// diff exits 1 on different sources, merge --take-all makes BASE equal to OTHER, and diff
    /// then exits 0.
    #[test]
    fn sources_diff_and_merge_round_trip() {
        let dir = std::env::temp_dir().join(format!("fontcraft-cli-sources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = |n: &str| dir.join(n).to_string_lossy().into_owned();
        let mut s = Session::new();
        s.execute("file.new", &json!({ "familyName": "Sample" })).unwrap();
        s.execute("file.saveAs", &json!({ "path": path("A.fontcraft") })).unwrap();
        s.execute("glyph.newGlyph", &json!({ "name": "A.alt" })).unwrap();
        s.execute("file.saveAs", &json!({ "path": path("B.fontcraft") })).unwrap();

        let e = run(&args(&["diff", &path("A.fontcraft"), &path("B.fontcraft")])).unwrap_err();
        assert_eq!(e.exit_code(), 1, "{}", e.message());
        run(&args(&["merge", &path("A.fontcraft"), &path("B.fontcraft"), "--take-all", "-o", &path("C.fontcraft")])).unwrap();
        run(&args(&["diff", &path("C.fontcraft"), &path("B.fontcraft"), "--format", "json"])).unwrap();
        let e = run(&args(&["merge", &path("A.fontcraft"), &path("B.fontcraft")])).unwrap_err();
        assert_eq!(e.exit_code(), 2);
    }
}
