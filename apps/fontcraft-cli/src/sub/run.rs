//! `fontcraft-cli run` (row `run`): engine commands by id, headless, given on the command line or
//! read from a script file.

use super::Subcommand;
use crate::args;
use crate::io::{self, Failure, Outcome, execute, open, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "run", summary: "Run engine commands headless on a source", usage: USAGE, options: &[], actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli run [--in FILE] [--cmd ID [--params JSON | --params-file FILE]]...
                    [--script FILE]... [--keep-going] [--save [FILE]] [--export FILE]...
      Headless batch: open FILE (a source) or start a new font, run the steps in order and print
      one JSON result line per engine command. --params - reads the parameters from standard
      input; --params-file reads them from a JSON file.
      --script FILE adds the steps of a file, one per line: `ID {JSON}` runs an engine command on
      the open document, `fontcraft-cli SUBCOMMAND ARGS...` runs a subcommand on its own (not run
      or mcp). Blank lines and lines starting with # are skipped.
      A failing step stops the run, and nothing after it runs; with --keep-going the other steps
      run and every failure is reported. Exit status 5 when some steps succeeded and one failed.
      --save writes the document (to FILE, or back to the file it was opened from) and --export
      compiles it to a font, both only when every step succeeded.
";

/// The option spellings of `run`, which reads its own arguments.
const SPELLINGS: &[&str] = &["--help", "--in", "--cmd", "--params", "--params-file", "--script", "--keep-going", "--save", "--export"];

/// Most steps one script file holds.
pub const MAX_SCRIPT_STEPS: usize = 10_000;

/// One step of a run.
#[derive(Debug)]
enum Step {
    /// An engine command on the open document, with its parameters.
    Engine { id: String, params: Value },
    /// A subcommand and its arguments, from a script line.
    Cli(Vec<String>),
}

/// A step and where it came from: `script.txt:3` for a script line, nothing for `--cmd`.
type Planned = (Option<String>, Step);

fn run(args: &[String]) -> Outcome {
    let args = args::take_globals(args, SPELLINGS)?;
    let mut input = None;
    let mut steps: Vec<Planned> = Vec::new();
    // The `--cmd` step that a `--params` would set.
    let mut last_cmd: Option<usize> = None;
    let mut save: Option<Option<String>> = None;
    let mut exports = Vec::new();
    let mut keep_going = false;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        let arg = args::spell_long(SPELLINGS, arg)?;
        match arg.as_str() {
            "-h" | "--help" => return out(&super::help(&SUBCOMMAND)),
            "--in" => input = Some(it.next().ok_or_else(|| usage("--in needs a file"))?.clone()),
            "--cmd" => {
                let id = it.next().ok_or_else(|| usage("--cmd needs a command id"))?.clone();
                last_cmd = Some(steps.len());
                steps.push((None, Step::Engine { id, params: json!({}) }));
            }
            "--params" => {
                let text = it.next().ok_or_else(|| usage("--params needs a JSON object, or - for standard input"))?;
                let value = if text == "-" {
                    serde_json::from_slice(&io::read_input("-")?)
                        .map_err(|e| usage(format!("the parameters on standard input are not valid JSON: {e}")))?
                } else {
                    serde_json::from_str(text).map_err(|e| usage(format!("--params is not valid JSON: {e}")))?
                };
                set_params(&mut steps, last_cmd, value, "--params")?;
            }
            "--params-file" => {
                let path = it.next().ok_or_else(|| usage("--params-file needs a file"))?;
                let text = args::read_small(path)?;
                let value = serde_json::from_str(&text).map_err(|e| usage(format!("{path} is not valid JSON: {e}")))?;
                set_params(&mut steps, last_cmd, value, "--params-file")?;
            }
            "--script" => {
                let path = it.next().ok_or_else(|| usage("--script needs a file"))?;
                steps.extend(read_script(path)?);
                last_cmd = None;
            }
            "--keep-going" => keep_going = true,
            "--save" => save = Some(it.next_if(|next| !next.starts_with("--")).cloned()),
            "--export" => exports.push(it.next().ok_or_else(|| usage("--export needs a file"))?.clone()),
            other => return Err(usage(format!("unknown run option `{other}`"))),
        }
    }
    let mut session = Session::new();
    match &input {
        Some(path) => open(&mut session, path)?,
        None => execute(&mut session, "file.new", &json!({}))?,
    };
    let total = steps.len();
    let mut done = 0usize;
    let mut failed = 0usize;
    for (n, (origin, step)) in steps.iter().enumerate() {
        let result = match origin {
            Some(origin) => io::with_input(origin, || run_step(&mut session, step)),
            None => run_step(&mut session, step),
        };
        let Err(failure) = result else {
            done += 1;
            continue;
        };
        let place = origin.as_ref().map(|o| format!("{o}: ")).unwrap_or_default();
        let what = match step {
            Step::Engine { id, .. } => format!("step `{id}`"),
            Step::Cli(words) => format!("`fontcraft-cli {}`", words.join(" ")),
        };
        if keep_going {
            io::error(&format!("{place}{what} failed: {}", failure.message()));
            failed += 1;
            continue;
        }
        let failure = prefixed(failure, &format!("{place}{what} failed, nothing after it ran: "));
        if done == 0 {
            return Err(failure);
        }
        io::error(failure.message());
        return Err(Failure::Partial(format!("step {} of {total} failed after {done} succeeded", n.saturating_add(1))));
    }
    if failed > 0 {
        let skipped = if save.is_some() || !exports.is_empty() { "; nothing was saved or exported" } else { "" };
        return Err(if done == 0 {
            Failure::Failed(format!("all {total} steps failed{skipped}"))
        } else {
            Failure::Partial(format!("{failed} of {total} steps failed{skipped}"))
        });
    }
    if let Some(path) = save {
        let result = execute(&mut session, "file.save", &json!({"path": path}))?;
        out(&json!({"command": "file.save", "result": result}).to_string())?;
    }
    for path in exports {
        let result = execute(&mut session, "file.exportStatic", &json!({"destination": path}))?;
        out(&json!({"command": "file.exportStatic", "result": result}).to_string())?;
    }
    Ok(())
}

/// Gives the `--cmd` step `last_cmd` its parameters.
fn set_params(steps: &mut [Planned], last_cmd: Option<usize>, value: Value, option: &str) -> Outcome {
    match last_cmd.and_then(|i| steps.get_mut(i)) {
        Some((_, Step::Engine { params, .. })) => {
            *params = value;
            Ok(())
        }
        _ => Err(usage(format!("{option} must follow a --cmd"))),
    }
}

/// The steps of a script file. A line that cannot be read is a usage error naming it, before any
/// step runs.
fn read_script(path: &str) -> Result<Vec<Planned>, Failure> {
    let text = args::read_small(path)?;
    let mut steps = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if steps.len() >= MAX_SCRIPT_STEPS {
            return Err(usage(format!("{path}: more than {MAX_SCRIPT_STEPS} steps")));
        }
        let origin = format!("{path}:{}", n.saturating_add(1));
        let words = args::split_words(line);
        let step = if words.first().is_some_and(|w| w == "fontcraft-cli") {
            let rest: Vec<String> = words.into_iter().skip(1).collect();
            let name = rest.first().ok_or_else(|| usage(format!("{origin}: a subcommand must follow fontcraft-cli")))?;
            let sub = super::lookup(name).map_err(|e| usage(format!("{origin}: {}", e.message())))?;
            let Some(sub) = sub else { return Err(usage(format!("{origin}: unknown subcommand `{name}`"))) };
            if sub.name == "run" || sub.name == "mcp" {
                return Err(usage(format!("{origin}: a script cannot run `{}`", sub.name)));
            }
            Step::Cli(rest)
        } else {
            let (id, params) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let params = match params.trim() {
                "" => json!({}),
                text => serde_json::from_str(text).map_err(|e| usage(format!("{origin}: the parameters of `{id}` are not valid JSON: {e}")))?,
            };
            Step::Engine { id: id.to_string(), params }
        };
        steps.push((Some(origin), step));
    }
    Ok(steps)
}

/// Runs one step: an engine command prints its result line; a subcommand prints what it prints.
fn run_step(session: &mut Session, step: &Step) -> Outcome {
    match step {
        Step::Engine { id, params } => {
            let result = execute(session, id, params)?;
            out(&json!({"command": id, "result": result}).to_string())
        }
        Step::Cli(words) => {
            let Some((name, rest)) = words.split_first() else { return Err(usage("an empty command line")) };
            let sub = super::lookup(name)?.ok_or_else(|| usage(format!("unknown subcommand `{name}`")))?;
            match super::run(sub, rest) {
                Err(Failure::Done) => Ok(()),
                other => other,
            }
        }
    }
}

/// `failure` with `prefix` before its message.
fn prefixed(failure: Failure, prefix: &str) -> Failure {
    match failure {
        Failure::Usage(m) => Failure::Usage(format!("{prefix}{m}")),
        Failure::Input(m) => Failure::Input(format!("{prefix}{m}")),
        Failure::Failed(m) => Failure::Failed(format!("{prefix}{m}")),
        Failure::Found(m) => Failure::Found(format!("{prefix}{m}")),
        Failure::Partial(m) => Failure::Partial(format!("{prefix}{m}")),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_lines_become_steps_and_bad_lines_are_refused() {
        let dir = std::env::temp_dir().join(format!("fontcraft-cli-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("steps.txt");
        std::fs::write(&script, "# build a glyph\nglyph.newGlyph {\"name\": \"A.alt\"}\n\nfile.new\nfontcraft-cli info \"My Font.ttf\"\n").unwrap();
        let path = script.to_str().unwrap();
        let steps = read_script(path).unwrap();
        assert_eq!(steps.len(), 3);
        assert!(
            matches!(&steps[0], (Some(o), Step::Engine { id, params }) if o.ends_with("steps.txt:2") && id == "glyph.newGlyph" && params["name"] == "A.alt")
        );
        assert!(matches!(&steps[1], (_, Step::Engine { params, .. }) if params == &json!({})), "no parameters is an empty object");
        assert!(matches!(&steps[2], (_, Step::Cli(words)) if words == &["info", "My Font.ttf"]), "quotes group words");

        for bad in ["glyph.newGlyph {not json", "fontcraft-cli run --cmd x", "fontcraft-cli nosuch", "fontcraft-cli"] {
            std::fs::write(&script, format!("file.new\n{bad}\n")).unwrap();
            let e = read_script(path).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad}");
            assert!(e.message().contains("steps.txt:2"), "the message names the line: {}", e.message());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
