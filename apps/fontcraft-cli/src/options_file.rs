//! Option files (`cli.global.optionsFile`): `--options FILE.toml` gives the options of the run and
//! `--save-options FILE` writes them. In the file the keys are long option names without dashes: a
//! switch is `true` or `false`, a list an array, any other value a string or a number. Relative
//! paths in it are relative to the file's folder.

use std::path::Path;

use fontcraft_engine::format::write_atomic;

use crate::args::{MAX_RESPONSE_DEPTH, read_small, split_words};
use crate::io::{Failure, usage};

/// Most keys one options file holds.
const MAX_KEYS: usize = 1_000;

/// What an option takes, for [`words`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `--name` and, when there is one, the spelling that turns it off.
    Switch { on: String, off: Option<String> },
    /// `--name=value`; `path` when the value names a file or folder.
    Value { name: String, path: bool },
}

/// Whether a value placeholder (`FILE`, `DIR`) stands for a path.
pub fn is_path(value_name: &str) -> bool {
    ["FILE", "DIR", "PATH", "OUT"].iter().any(|w| value_name.contains(w))
}

/// A path as an options file holds it: absolute, since a relative one is read relative to the file.
pub fn path_value(path: &str) -> String {
    if path.is_empty() || path.contains(['<', ',']) || path == "-" {
        return path.to_string();
    }
    std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())
}

/// Whether `file` is read as TOML rather than as a response file.
pub fn is_toml(file: &str) -> bool {
    Path::new(file).extension().is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

/// Whether `file` is a makeotf project file.
pub fn is_project(file: &str) -> bool {
    Path::new(file).extension().is_some_and(|e| e.eq_ignore_ascii_case("fpr"))
}

/// The arguments the TOML options file `file` stands for. `shape` describes the option a key names.
pub fn words(file: &str, shape: &dyn Fn(&str) -> Option<Shape>) -> Result<Vec<String>, Failure> {
    let text = read_small(file)?;
    let table: toml::Table = toml::from_str(&text).map_err(|e| Failure::Input(format!("{file}: {}", e.message())))?;
    if table.len() > MAX_KEYS {
        return Err(usage(format!("{file}: more than {MAX_KEYS} options")));
    }
    let folder = Path::new(file).parent().unwrap_or(Path::new(""));
    let mut out = Vec::new();
    for (key, value) in &table {
        let shape = shape(key).ok_or_else(|| usage(format!("{file}: `{key}` is not an option of this subcommand")))?;
        let bad = |what: &str| usage(format!("{file}: `{key}` needs {what}"));
        match shape {
            Shape::Switch { on, off } => match value {
                toml::Value::Boolean(true) => out.push(on),
                toml::Value::Boolean(false) => out.push(off.ok_or_else(|| bad("to be true or left out"))?),
                _ => return Err(bad("true or false")),
            },
            Shape::Value { name, path } => {
                let one = |v: &toml::Value| -> Option<String> {
                    match v {
                        toml::Value::String(s) if path && !s.is_empty() && s != "-" => Some(folder.join(s).to_string_lossy().into_owned()),
                        toml::Value::String(s) => Some(s.clone()),
                        toml::Value::Integer(i) => Some(i.to_string()),
                        toml::Value::Float(f) if f.is_finite() => Some(f.to_string()),
                        toml::Value::Boolean(b) => Some(b.to_string()),
                        _ => None,
                    }
                };
                let text = match value {
                    toml::Value::Array(items) => {
                        items.iter().map(one).collect::<Option<Vec<_>>>().ok_or_else(|| bad("strings or numbers"))?.join(",")
                    }
                    v => one(v).ok_or_else(|| bad("a string, a number or a list of them"))?,
                };
                out.push(format!("{name}={text}"));
            }
        }
    }
    Ok(out)
}

/// `args` with every `--options FILE` and `@FILE` replaced by what the file holds, for a
/// subcommand with its own parser. The options of a TOML or project file come first, so that the
/// line overrides them. `takes_next` says whether the argument after one is its value.
pub fn expand(args: &[String], shape: &dyn Fn(&str) -> Option<Shape>, takes_next: &dyn Fn(&str) -> bool) -> Result<Vec<String>, Failure> {
    let (mut first, mut rest) = (Vec::new(), Vec::new());
    expand_into(args, shape, takes_next, 0, &mut first, &mut rest)?;
    first.extend(rest);
    Ok(first)
}

fn expand_into(
    args: &[String],
    shape: &dyn Fn(&str) -> Option<Shape>,
    takes_next: &dyn Fn(&str) -> bool,
    depth: usize,
    first: &mut Vec<String>,
    rest: &mut Vec<String>,
) -> Result<bool, Failure> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--" {
            rest.push(arg.clone());
            rest.extend(it.by_ref().cloned());
            return Ok(true);
        }
        let (file, options) = if let Some(path) = arg.strip_prefix('@').filter(|p| !p.is_empty()) {
            (Some(path.to_string()), false)
        } else if arg == "--options" {
            (Some(it.next().ok_or_else(|| usage("--options needs a file"))?.clone()), true)
        } else {
            (arg.strip_prefix("--options=").map(str::to_string), true)
        };
        let Some(file) = file else {
            rest.push(arg.clone());
            if takes_next(arg) {
                rest.extend(it.next().cloned());
            }
            continue;
        };
        if options && is_toml(&file) {
            first.extend(words(&file, shape)?);
        } else if options && is_project(&file) {
            first.extend(["--project-file".to_string(), file]);
        } else {
            if depth >= MAX_RESPONSE_DEPTH {
                return Err(usage(format!("response files nest more than {MAX_RESPONSE_DEPTH} deep at {file}")));
            }
            let inner = split_words(&read_small(&file)?);
            if expand_into(&inner, shape, takes_next, depth.saturating_add(1), first, rest)? {
                rest.extend(it.by_ref().cloned());
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Writes `table` to `file`, where `--options` reads it back.
pub fn save(file: &str, table: &toml::Table) -> Result<(), Failure> {
    let text = toml::to_string(table).map_err(|e| Failure::Failed(format!("cannot write {file}: {e}")))?;
    write_atomic(Path::new(file), text.as_bytes()).map_err(|e| Failure::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toml_file_gives_options_and_the_line_overrides_them() {
        let dir = std::env::temp_dir().join(format!("fontcraft-options-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("o.toml");
        std::fs::write(&file, "output = \"a.ttf\"\nhinting = false\nglyphs = [\"A\", \"B\"]\nunits = 4\n").unwrap();
        let file = file.to_str().unwrap().to_string();
        let shape = |key: &str| match key {
            "hinting" => Some(Shape::Switch { on: "--hinting".into(), off: Some("--no-hinting".into()) }),
            "output" => Some(Shape::Value { name: "--output".into(), path: true }),
            "glyphs" | "units" => Some(Shape::Value { name: format!("--{key}"), path: false }),
            _ => None,
        };
        let line = ["in.ttf", "--units", "9", "--options", &file].map(String::from);
        let out = expand(&line, &shape, &|a| a == "--units").unwrap();
        let path = format!("--output={}", dir.join("a.ttf").display());
        let (head, tail) = out.split_at(4);
        for word in ["--no-hinting", &path, "--glyphs=A,B", "--units=4"] {
            assert!(head.iter().any(|w| w == word), "{word}: {out:?}");
        }
        assert_eq!(tail, ["in.ttf", "--units", "9"]);
        std::fs::write(&file, "nosuch = 1\n").unwrap();
        assert_eq!(expand(&line, &shape, &|a| a == "--units").unwrap_err().exit_code(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
