//! `fontcraft-cli check`: runs the audit (`audit.run`) on sources and compiled fonts and prints
//! the findings; `check outlines`, `check family` and `check interpolation` run the three
//! shortcut commands with the reference tools' options (`docs/spec/21-qa.md` § 3 and § 6).
//!
//! Exit status (`audit.exitStatus`): 0 when no reported result reaches `--fail-on` (default
//! fail), 1 when one does or a check failed to run, 2 for a usage error, 3 for an input that
//! cannot be read, 4 when the run itself failed. Ignored and skipped results never count.

use fontcraft_engine::{EngineError, Session};
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "check",
    summary: "Run quality checks on fonts and sources",
    usage: USAGE,
    options: OPTIONS,
    actions: &["outlines", "family", "interpolation"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli check [outlines|family|interpolation] INPUT... [--profile ID] [--check ID]... [--exclude ID]... [--fix] [--report PATH] [options]
  fontcraft-cli check outlines INPUT... [--fix] [--no-overlap-checks] [--no-basic-checks] [--min-area N] [--tolerance N] [--glyphs LIST] [--all] [-o OUT]
  fontcraft-cli check family DIR|FONT... [--tolerance N] [--menu-names] [--metrics] [--panose] [--layout] [--single LIST] [--family LIST] [--no-hints]
  fontcraft-cli check interpolation INPUT... [--glyphs LIST] [--tolerance F] [--kinkiness F] [--show-all] [--ignore-missing] [--master NAME]... [--report PATH]
";

/// Options every form shares: output, report and exit status.
macro_rules! common {
    () => {
        [
            OptSpec::value("format", "FORMAT", "text", "how findings are printed: text, json, markdown or html"),
            OptSpec::switch("json", false, "the same as --format json"),
            OptSpec::value("report", "PATH", "", "also write the findings to PATH, in the format its extension names"),
            OptSpec::value("min-severity", "SEVERITY", "info", "lowest severity reported: info, warn or fail"),
            OptSpec::value("fail-on", "SEVERITY", "fail", "lowest severity that makes the exit status 1: info, warn or fail"),
            OptSpec::switch("include-ignored", false, "also list findings on the source's ignore list"),
            OptSpec::switch("quiet", false, "print nothing; only the exit status tells the outcome").short('q'),
        ]
    };
}

const OPTIONS: &[OptSpec] = &{
    let c = common!();
    [
        OptSpec::value("profile", "ID", "", "profile to run; default the source's own default profile, else universal"),
        OptSpec::list("check", "ID_OR_GLOB", "", "run only matching checks (an id, or a prefix ending in *)"),
        OptSpec::list("exclude", "ID_OR_GLOB", "", "leave matching checks out"),
        OptSpec::list("group", "NAME", "", "run only the checks of these groups"),
        OptSpec::list("glyphs", "SELECTOR", "", "restrict glyph-level checks to these glyphs: names, indexes, first-last ranges, /CID"),
        OptSpec::value("glyphs-file", "FILE", "", "read the glyph selection from FILE"),
        OptSpec::list("master", "ID", "", "restrict to these masters"),
        OptSpec::list("set", "CHECK.NAME=VALUE", "", "override a threshold of a check"),
        OptSpec::switch("all", false, "examine every glyph, ignoring the cache of unchanged glyphs"),
        OptSpec::switch("network", false, "allow the checks that need network access"),
        OptSpec::switch("fix", false, "after the run, apply every safe repair to the source and save it"),
        OptSpec::switch("fix-unsafe", false, "also apply repairs that alter the design"),
        OptSpec::value("output", "PATH", "", "with --fix, save the repaired source to PATH instead of over the input").short('o'),
        OptSpec::switch("list", false, "print the check registry and exit"),
        OptSpec::value("describe", "ID", "", "print everything about one check and exit"),
        OptSpec::switch("profiles", false, "list the available profiles and exit"),
        OptSpec::switch("include-passed", false, "also list the checks that passed"),
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        c[6],
    ]
};

const OUTLINES: &[OptSpec] = &{
    let c = common!();
    [
        OptSpec::switch("fix", false, "repair the reported problems and save the source").short('e'),
        OptSpec::switch("error-correction-mode", false, "the same as --fix"),
        OptSpec::switch("overlap-checks", true, "run the overlap and tiny-contour tests"),
        OptSpec::switch("basic-checks", true, "run the coincident-point, flat-curve and collinear-line tests"),
        OptSpec::value("min-area", "N", "25", "bounding-box area below which a contour of at most four nodes is tiny"),
        OptSpec::value("tolerance", "N", "0", "largest distance from a straight line, in units, for a flat curve or a collinear point"),
        OptSpec::list("glyphs", "SELECTOR", "", "check only these glyphs: names, indexes, first-last ranges, /CID").short('g'),
        OptSpec::list("glyph-list", "SELECTOR", "", "the same as --glyphs"),
        OptSpec::value("glyphs-file", "FILE", "", "read the glyph selection from FILE").short('f'),
        OptSpec::value("glyph-file", "FILE", "", "the same as --glyphs-file"),
        OptSpec::switch("decimal", false, "keep fractional coordinates when fixing").short('d'),
        OptSpec::switch("keep-contour-order", true, "after overlap removal, restore the original contour order"),
        OptSpec::switch("ignore-contour-order", false, "the same as --no-keep-contour-order"),
        OptSpec::value("keep-original", "backup-layer|none", "backup-layer", "when fixing, keep the unfixed outline in a backup layer or discard it"),
        OptSpec::switch("write-to-default-layer", false, "the same as --keep-original none").short('w'),
        OptSpec::switch("all", false, "check every glyph even if it has not changed since the last run"),
        OptSpec::switch("clear-cache", false, "delete the stored glyph hashes and exit"),
        OptSpec::switch("clear-hash-map", false, "the same as --clear-cache"),
        OptSpec::value("output", "PATH", "", "with --fix, save the repaired source to PATH").short('o'),
        OptSpec::value("output-file", "PATH", "", "the same as --output"),
        OptSpec::switch("quiet-mode", false, "the same as --quiet"),
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        c[6],
    ]
};

const FAMILY: &[OptSpec] = &{
    let c = common!();
    [
        OptSpec::value("directory", "DIR", "", "folder to scan for fonts").short('d'),
        OptSpec::value("tolerance", "N", "0", "allowed difference in units before a metric mismatch is reported"),
        OptSpec::switch("menu-names", false, "print the table of menu names per font"),
        OptSpec::switch("metrics", false, "print the table of global metrics per typographic family"),
        OptSpec::switch("panose", false, "print the PANOSE digits of each font with their labels"),
        OptSpec::switch("layout", false, "print the report of features and lookups per script and language system"),
        OptSpec::list("single", "LIST", "", "run only these single-face tests, by number 1 to 34"),
        OptSpec::list("family", "LIST", "", "run only these family tests, by number 1 to 21"),
        OptSpec::switch("hints", true, "check stem-hint widths and positions"),
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        c[6],
    ]
};

const INTERPOLATION: &[OptSpec] = &{
    let c = common!();
    [
        OptSpec::list("glyphs", "SELECTOR", "", "glyphs to test; default all"),
        OptSpec::value(
            "tolerance",
            "F",
            "0.95",
            "report a contour order or start point whose score falls below this share of the best (10 or more: a percentage)",
        ),
        OptSpec::value("kinkiness", "F", "0.5", "sensitivity of the kink test; larger reports smaller kinks"),
        OptSpec::switch("show-all", false, "also list glyphs without problems"),
        OptSpec::switch("ignore-missing", false, "do not report glyphs absent from a sparse master"),
        OptSpec::list("master", "NAME", "", "restrict the test to these masters"),
        OptSpec::list("name", "NAME", "", "the same as --master"),
        OptSpec::value("output", "PATH", "", "the same as --report"),
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        c[6],
    ]
};

fn help_for(action: &str, specs: &[OptSpec]) -> String {
    let line = USAGE.lines().find(|l| l.contains(&format!("check {action} "))).unwrap_or(USAGE);
    format!("fontcraft-cli check {action}: {}\n\nUSAGE:\n{}", SUBCOMMAND.summary, args::help(&format!("{line}\n"), specs))
}

fn run(argv: &[String]) -> Outcome {
    match argv.split_first() {
        Some((a, rest)) if a == "outlines" => outlines(rest),
        Some((a, rest)) if a == "family" => family(rest),
        Some((a, rest)) if a == "interpolation" => interpolation(rest),
        _ => audit(argv),
    }
}

/// An engine error as an exit status: bad parameters are usage errors, unreadable inputs are 3.
fn failure(e: EngineError) -> Failure {
    match e {
        EngineError::BadParams { .. } => Failure::Usage(e.to_string()),
        EngineError::Io(_) | EngineError::Decompile(_) | EngineError::Glyphs(_) | EngineError::Ufo(_) | EngineError::Format(_) => {
            Failure::Input(e.to_string())
        }
        other => Failure::Failed(other.to_string()),
    }
}

fn exec(s: &mut Session, id: &str, p: &Value) -> Result<Value, Failure> {
    s.execute(id, p).map_err(failure)
}

/// One target value: a path, or the list of paths.
fn target(inputs: &[String]) -> Value {
    match inputs {
        [one] => json!(one),
        many => json!(many),
    }
}

fn severity_rank(s: &str) -> Option<u8> {
    match s {
        "fail" => Some(0),
        "warn" => Some(1),
        "info" => Some(2),
        _ => None,
    }
}

/// What the common options ask for.
struct Output {
    format: String,
    report: Option<String>,
    min: u8,
    fail_on: u8,
    quiet: bool,
}

impl Output {
    fn read(p: &args::Parsed) -> Result<Self, Failure> {
        let format = if p.switch("json") { "json".to_string() } else { p.value("format").unwrap_or("text").to_string() };
        if !matches!(format.as_str(), "text" | "json" | "markdown" | "html") {
            return Err(usage(format!("--format takes text, json, markdown or html, not {format}")));
        }
        let level = |opt: &str| {
            let v = p.value(opt).unwrap_or("info");
            severity_rank(v).ok_or_else(|| usage(format!("--{opt} takes info, warn or fail, not {v}")))
        };
        Ok(Output {
            format,
            report: p.value("report").filter(|r| !r.is_empty()).map(str::to_string),
            min: level("min-severity")?,
            fail_on: p.value("fail-on").map_or(Ok(0), |_| level("fail-on"))?,
            quiet: p.switch("quiet") || p.switch("quiet-mode"),
        })
    }

    /// The `minSeverity` the engine runs with: wide enough for both the printed results and the
    /// exit status.
    fn engine_min(&self) -> &'static str {
        match self.min.max(self.fail_on) {
            0 => "fail",
            1 => "warn",
            _ => "info",
        }
    }
}

/// Prints `doc` (narrowed to `--min-severity`), writes `--report`, and turns the findings into
/// the exit status.
fn finish(s: &mut Session, doc: Value, o: &Output) -> Outcome {
    let rank = |r: &Value| r.get("severity").and_then(Value::as_str).and_then(severity_rank).unwrap_or(2);
    let ignored = |r: &Value| r.get("ignored").and_then(Value::as_bool).unwrap_or(false);
    let results = doc.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    let found = results.iter().filter(|r| !ignored(r) && rank(r) <= o.fail_on).count();
    let errors = doc.pointer("/summary/errors").and_then(Value::as_u64).unwrap_or(0);
    let mut shown = doc;
    if let Some(obj) = shown.as_object_mut() {
        let kept: Vec<Value> = results.into_iter().filter(|r| rank(r) <= o.min).collect();
        let mut counts = [0usize; 3];
        for r in kept.iter().filter(|r| !ignored(r)) {
            if let Some(n) = counts.get_mut(usize::from(rank(r))) {
                *n += 1;
            }
        }
        if let Some(Value::Object(summary)) = obj.get_mut("summary") {
            for (key, n) in ["fail", "warn", "info"].iter().zip(counts) {
                summary.insert((*key).into(), json!(n));
            }
        }
        obj.insert("results".into(), Value::Array(kept));
    }
    if let Some(path) = &o.report {
        exec(s, "audit.export", &json!({"path": path, "results": shown}))?;
    }
    if !o.quiet {
        print_doc(s, &shown, &o.format)?;
    }
    if found > 0 || errors > 0 {
        let level = ["fail", "warn", "info"].get(usize::from(o.fail_on)).copied().unwrap_or("fail");
        let message = if o.quiet { String::new() } else { format!("{found} results at or above {level}, {errors} checks failed to run") };
        return Err(Failure::Found(message));
    }
    Ok(())
}

/// Prints a result document: JSON as is, text as one line per result and a summary, Markdown
/// and HTML through `audit.export` into a temporary file.
fn print_doc(s: &mut Session, doc: &Value, format: &str) -> Outcome {
    match format {
        "json" => io::out_json(doc),
        "text" => {
            for w in doc.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                io::note(w);
            }
            for r in doc.get("results").and_then(Value::as_array).into_iter().flatten() {
                out(&text_line(r))?;
            }
            out(&summary_line(doc))
        }
        other => {
            let dir = std::env::temp_dir().join(format!("fontcraft-check-{}", std::process::id()));
            std::fs::create_dir_all(&dir).map_err(|e| Failure::Failed(format!("cannot create a temporary folder: {e}")))?;
            let path = dir.join(if other == "html" { "report.html" } else { "report.md" });
            let written = exec(s, "audit.export", &json!({"path": path, "results": doc, "format": other}));
            let text = written.and_then(|_| std::fs::read_to_string(&path).map_err(|e| Failure::Failed(format!("cannot read the report back: {e}"))));
            let _ = std::fs::remove_dir_all(&dir);
            out(text?.trim_end())
        }
    }
}

/// `SEVERITY check-id [font] glyph/master: message`.
fn text_line(r: &Value) -> String {
    let get = |k: &str| r.pointer(k).and_then(Value::as_str).unwrap_or("");
    let mut place = String::new();
    if !get("/location/glyph").is_empty() {
        place.push(' ');
        place.push_str(get("/location/glyph"));
        if !get("/location/master").is_empty() {
            place.push('/');
            place.push_str(get("/location/master"));
        }
    } else if !get("/location/table").is_empty() {
        place.push(' ');
        place.push_str(get("/location/table"));
        if !get("/location/field").is_empty() {
            place.push('.');
            place.push_str(get("/location/field"));
        }
    }
    let ignored = if r.get("ignored").and_then(Value::as_bool).unwrap_or(false) { " (ignored)" } else { "" };
    format!("{} {} [{}]{place}: {}{ignored}", get("/severity").to_uppercase(), get("/check"), get("/location/font"), get("/message"))
}

fn summary_line(doc: &Value) -> String {
    let n = |k: &str| doc.pointer(&format!("/summary/{k}")).and_then(Value::as_u64).unwrap_or(0);
    format!(
        "{} fail, {} warn, {} info; {} passed, {} skipped, {} ignored, {} errors",
        n("fail"),
        n("warn"),
        n("info"),
        n("passed"),
        n("skipped"),
        n("ignored"),
        n("errors")
    )
}

fn list_param(p: &args::Parsed, opts: &[&str]) -> Vec<String> {
    opts.iter().flat_map(|o| p.list(o).iter().cloned()).collect()
}

fn number(p: &args::Parsed, opt: &str) -> Result<Option<f64>, Failure> {
    match p.value(opt).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => v.parse::<f64>().ok().filter(|n| n.is_finite()).map(Some).ok_or_else(|| usage(format!("--{opt} takes a number, not {v}"))),
    }
}

/// `--set CHECK.NAME=VALUE` entries as `config`: the threshold name is the part after the last
/// dot of the left side.
fn config(sets: &[String]) -> Result<Map<String, Value>, Failure> {
    let mut config: Map<String, Value> = Map::new();
    for item in sets {
        let (key, value) = item.split_once('=').ok_or_else(|| usage(format!("--set takes CHECK.NAME=VALUE, not {item}")))?;
        let (check, name) = key.rsplit_once('.').ok_or_else(|| usage(format!("--set takes CHECK.NAME=VALUE, not {item}")))?;
        let v: f64 = value.parse().ok().filter(|v: &f64| v.is_finite()).ok_or_else(|| usage(format!("--set: {value} is not a number")))?;
        if let Value::Object(m) = config.entry(check.to_string()).or_insert_with(|| json!({})) {
            m.insert(name.to_string(), json!(v));
        }
    }
    Ok(config)
}

// ── check ───────────────────────────────────────────────────────────────────────────────────────

fn audit(argv: &[String]) -> Outcome {
    let p = args::parse(OPTIONS, argv)?;
    if p.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let o = Output::read(&p)?;
    let mut s = Session::new();
    if p.switch("profiles") {
        return io::out_json(&exec(&mut s, "audit.profile.list", &json!({}))?);
    }
    if let Some(id) = p.value("describe").filter(|v| !v.is_empty()) {
        return io::out_json(&exec(&mut s, "audit.describe", &json!({"check": id}))?);
    }
    if p.switch("list") {
        let mut q = Map::new();
        if let Some(profile) = p.value("profile").filter(|v| !v.is_empty()) {
            q.insert("profile".into(), json!(profile));
        }
        if !p.list("group").is_empty() {
            q.insert("groups".into(), json!(p.list("group")));
        }
        let listed = exec(&mut s, "audit.list", &Value::Object(q))?;
        if o.format == "json" {
            return io::out_json(&listed);
        }
        for c in listed.get("checks").and_then(Value::as_array).into_iter().flatten() {
            let get = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("");
            let fixable = if c.get("fixable").and_then(Value::as_bool).unwrap_or(false) { "fixable" } else { "-" };
            out(&format!("{}\t{}\t{}\t{}\t{fixable}\t{}", get("id"), get("group"), get("severity"), get("scope"), get("description")))?;
        }
        return Ok(());
    }
    if p.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let mut q = Map::new();
    if let Some(profile) = p.value("profile").filter(|v| !v.is_empty()) {
        q.insert("profile".into(), json!(profile));
    }
    for (opt, key) in [("check", "checks"), ("exclude", "exclude"), ("group", "groups"), ("glyphs", "glyphs"), ("master", "masters")] {
        if !p.list(opt).is_empty() {
            q.insert(key.into(), json!(p.list(opt)));
        }
    }
    if let Some(file) = p.value("glyphs-file").filter(|v| !v.is_empty()) {
        q.insert("glyphFile".into(), json!(file));
    }
    let set = config(p.list("set"))?;
    if !set.is_empty() {
        q.insert("config".into(), Value::Object(set));
    }
    q.insert("minSeverity".into(), json!(o.engine_min()));
    for (opt, key) in [("network", "network"), ("include-passed", "includePassed"), ("include-ignored", "includeIgnored")] {
        if p.switch(opt) {
            q.insert(key.into(), json!(true));
        }
    }
    let fix = p.switch("fix") || p.switch("fix-unsafe");
    if !fix {
        if p.value("output").is_some_and(|v| !v.is_empty()) {
            return Err(usage("--output needs --fix"));
        }
        q.insert("target".into(), target(&p.positional));
        let doc = exec(&mut s, "audit.run", &Value::Object(q))?;
        return finish(&mut s, doc, &o);
    }
    let [input] = p.positional.as_slice() else { return Err(usage("--fix takes one source")) };
    io::open(&mut s, input)?;
    let doc = exec(&mut s, "audit.run", &Value::Object(q.clone()))?;
    let mut f = q;
    f.remove("minSeverity");
    f.remove("network");
    f.remove("includePassed");
    f.remove("includeIgnored");
    f.insert("safeOnly".into(), json!(!p.switch("fix-unsafe")));
    let fixed = exec(&mut s, "audit.fix", &Value::Object(f))?;
    save(&mut s, &p, input)?;
    if !o.quiet {
        let applied = fixed.get("applied").and_then(Value::as_array).map_or(0, Vec::len);
        io::note(&format!("{applied} repairs applied"));
    }
    finish(&mut s, doc, &o)
}

/// Saves the fixed document to `--output` (or `--output-file`), else over the input.
fn save(s: &mut Session, p: &args::Parsed, input: &str) -> Outcome {
    let output = p.value("output").or_else(|| p.value("output-file")).filter(|v| !v.is_empty());
    match output {
        Some(path) => exec(s, "file.saveAs", &json!({"path": path})).map(|_| ()),
        None if std::path::Path::new(input).exists() => exec(s, "file.save", &json!({})).map(|_| ()),
        None => Err(Failure::Failed(format!("cannot save {input}"))),
    }
}

// ── check outlines ──────────────────────────────────────────────────────────────────────────────

fn outlines(argv: &[String]) -> Outcome {
    let p = args::parse(OUTLINES, argv)?;
    if p.help {
        return out(&help_for("outlines", OUTLINES));
    }
    let o = Output::read(&p)?;
    let mut s = Session::new();
    if p.switch("clear-cache") || p.switch("clear-hash-map") {
        // No glyph hashes are stored yet: every run examines every glyph.
        return Ok(());
    }
    if p.positional.is_empty() {
        return Err(usage(help_for("outlines", OUTLINES)));
    }
    let fix = p.switch("fix") || p.switch("error-correction-mode");
    let mut q = Map::new();
    let glyphs = list_param(&p, &["glyphs", "glyph-list"]);
    if !glyphs.is_empty() {
        q.insert("glyphs".into(), json!(glyphs));
    }
    if let Some(file) = p.value("glyphs-file").or_else(|| p.value("glyph-file")).filter(|v| !v.is_empty()) {
        q.insert("glyphFile".into(), json!(file));
    }
    for (opt, key) in [("min-area", "minArea"), ("tolerance", "tolerance")] {
        if let Some(n) = number(&p, opt)? {
            q.insert(key.into(), json!(n));
        }
    }
    q.insert("overlapChecks".into(), json!(p.switch("overlap-checks")));
    q.insert("basicChecks".into(), json!(p.switch("basic-checks")));
    q.insert("keepContourOrder".into(), json!(p.switch("keep-contour-order") && !p.switch("ignore-contour-order")));
    q.insert("decimal".into(), json!(p.switch("decimal")));
    let keep = match (p.switch("write-to-default-layer"), p.value("keep-original").unwrap_or("backup-layer")) {
        (true, _) | (false, "none") => "none",
        (false, "backup-layer") => "backupLayer",
        (false, other) => return Err(usage(format!("--keep-original takes backup-layer or none, not {other}"))),
    };
    q.insert("keepOriginal".into(), json!(keep));
    let mut results = Vec::with_capacity(p.positional.len());
    for input in &p.positional {
        let mut call = q.clone();
        let outcome = if fix {
            if p.positional.len() > 1 && p.value("output").is_some_and(|v| !v.is_empty()) {
                return Err(usage("--output takes one input"));
            }
            io::open(&mut s, input).and_then(|_| {
                call.insert("fix".into(), json!(true));
                let doc = exec(&mut s, "audit.checkOutlines", &Value::Object(call))?;
                save(&mut s, &p, input)?;
                Ok(doc)
            })
        } else {
            call.insert("target".into(), json!(input));
            exec(&mut s, "audit.checkOutlines", &Value::Object(call))
        };
        let result = outcome.and_then(|doc| finish(&mut s, doc, &o));
        if let Err(e) = &result
            && !matches!(e, Failure::Found(_))
        {
            io::note(&format!("{input}: {}", e.message()));
        }
        results.push(result);
    }
    match results.as_slice() {
        [one] => one.clone(),
        many if many.iter().all(|r| matches!(r, Ok(()) | Err(Failure::Found(_)))) => {
            if many.iter().any(Result::is_err) {
                Err(Failure::Found(String::new()))
            } else {
                Ok(())
            }
        }
        many => io::batch_outcome(many),
    }
}

// ── check family ────────────────────────────────────────────────────────────────────────────────

fn family(argv: &[String]) -> Outcome {
    let p = args::parse(FAMILY, argv)?;
    if p.help {
        return out(&help_for("family", FAMILY));
    }
    let o = Output::read(&p)?;
    let mut s = Session::new();
    let mut fonts: Vec<String> = p.positional.clone();
    if let Some(dir) = p.value("directory").filter(|v| !v.is_empty()) {
        fonts.push(dir.to_string());
    }
    if fonts.is_empty() {
        fonts.push(".".to_string());
    }
    let mut q = Map::new();
    q.insert("fonts".into(), json!(fonts));
    for (opt, key) in [("single", "singleTests"), ("family", "familyTests")] {
        let items = p.list(opt);
        if !items.is_empty() {
            let numbers = items
                .iter()
                .map(|v| v.parse::<u64>().map_err(|_| usage(format!("--{opt} takes test numbers, not {v}"))))
                .collect::<Result<Vec<_>, _>>()?;
            q.insert(key.into(), json!(numbers));
        }
    }
    if let Some(t) = number(&p, "tolerance")? {
        q.insert("tolerance".into(), json!(t));
    }
    if !p.switch("hints") {
        q.insert("noHints".into(), json!(true));
    }
    let reports = [
        ("menu-names", "audit.report.menuNames"),
        ("metrics", "audit.report.metrics"),
        ("panose", "audit.report.panose"),
        ("layout", "audit.report.featureLanguages"),
    ];
    let asked: Vec<&str> = reports.iter().filter(|(opt, _)| p.switch(opt)).map(|(_, id)| *id).collect();
    for id in &asked {
        let report = exec(&mut s, id, &json!({"fonts": fonts}))?;
        if !o.quiet {
            io::out_json(&report)?;
        }
    }
    if asked == ["audit.report.featureLanguages"] {
        return Ok(());
    }
    let doc = exec(&mut s, "audit.compareFamily", &Value::Object(q))?;
    finish(&mut s, doc, &o)
}

// ── check interpolation ─────────────────────────────────────────────────────────────────────────

fn interpolation(argv: &[String]) -> Outcome {
    let p = args::parse(INTERPOLATION, argv)?;
    if p.help {
        return out(&help_for("interpolation", INTERPOLATION));
    }
    let mut o = Output::read(&p)?;
    if o.report.is_none() {
        o.report = p.value("output").filter(|v| !v.is_empty()).map(str::to_string);
    }
    let [input] = p.positional.as_slice() else {
        return Err(usage("give one variable font, designspace or multi-master source"));
    };
    let mut s = Session::new();
    let mut q = Map::new();
    q.insert("target".into(), json!(input));
    if !p.list("glyphs").is_empty() {
        q.insert("glyphs".into(), json!(p.list("glyphs")));
    }
    let masters = list_param(&p, &["master", "name"]);
    if !masters.is_empty() {
        q.insert("masters".into(), json!(masters));
    }
    for (opt, key) in [("tolerance", "tolerance"), ("kinkiness", "kinkiness")] {
        if let Some(n) = number(&p, opt)? {
            q.insert(key.into(), json!(n));
        }
    }
    for (opt, key) in [("show-all", "showAll"), ("ignore-missing", "ignoreMissing")] {
        if p.switch(opt) {
            q.insert(key.into(), json!(true));
        }
    }
    let doc = exec(&mut s, "audit.checkInterpolation", &Value::Object(q))?;
    finish(&mut s, doc, &o)
}
