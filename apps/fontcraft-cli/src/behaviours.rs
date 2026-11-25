//! Which of the shared behaviours (`cli.global.*` rows) each subcommand gives. The behaviours the
//! common options bring apply to every subcommand but the ones listed here; the rest follow from
//! the subcommand's own options. `commands --cli --json` prints the table, and
//! `cargo xtask cli-options` writes it to `docs/cli-options.md`.

use serde_json::{Map, Value, json};

/// Every behaviour, as the table's columns name it.
pub const COLUMNS: &[&str] =
    &["dry-run", "no-clobber", "in-place", "output-dir", "stdin", "stdout", "face", "jobs", "json", "options", "save-options", "trace", "layer"];

/// Subcommands whose work runs outside this process or lasts as long as a client wants, so a dry
/// run or a refusal to overwrite cannot cover it.
const OUTSIDE: &[&str] = &["mcp", "oracle", "bench"];

/// Subcommands that take a font from standard input as `-`.
const STDIN: &[&str] =
    &["info", "tables", "dump", "subset", "instance", "transform", "optimize", "woff", "outlines", "dehint", "proof", "variations", "convert"];

/// Subcommands that write one font to standard output with `-o -`.
const STDOUT: &[&str] = &["compile", "subset", "instance", "merge", "transform", "optimize", "woff", "outlines", "dehint", "convert", "features"];

/// Subcommands whose own code writes text to standard output for `-o -`.
const OWN_STDOUT: &[&str] = &["dump", "stems"];

/// Subcommands that work on several inputs at once.
const JOBS: &[&str] = &["compile", "info", "tables", "subset", "optimize", "woff", "outlines", "autohint", "stems", "dehint", "check", "metrics"];

/// Subcommands that open a UFO as a document, so `--layer` can choose its glyph layer.
const LAYER: &[&str] = &["compile", "convert", "autohint", "stems", "run"];

/// Subcommands without an option table to fill from a file.
const NO_OPTIONS_FILE: &[&str] = &["mcp", "oracle", "run", "commands", "completion"];

/// Whether the subcommand `name`, which accepts the long options `own`, gives `behaviour`.
pub fn gives(name: &str, own: &[String], behaviour: &str) -> bool {
    let has = |option: &str| own.iter().any(|o| o == option);
    match behaviour {
        "dry-run" | "no-clobber" => !OUTSIDE.contains(&name),
        "stdin" => STDIN.contains(&name),
        "stdout" => STDOUT.contains(&name) || OWN_STDOUT.contains(&name),
        "jobs" => JOBS.contains(&name),
        "options" | "save-options" => !NO_OPTIONS_FILE.contains(&name),
        "trace" => true,
        "layer" => LAYER.contains(&name),
        option => has(option),
    }
}

/// Whether `-` among the inputs of `name` is replaced by standard input before it runs.
pub fn reads_stdin(name: &str) -> bool {
    STDIN.contains(&name)
}

/// Whether `-o -` of `name` is replaced by a place whose content goes to standard output.
pub fn writes_stdout(name: &str) -> bool {
    STDOUT.contains(&name)
}

/// The common options `name` refuses: it does not give what they ask for.
pub fn refuses(name: &str, option: &str) -> bool {
    match option {
        "dry-run" | "no-clobber" | "options" | "save-options" | "layer" => !gives(name, &[], option),
        _ => false,
    }
}

/// The row of `name` as JSON: each column with whether the subcommand gives it.
pub fn row(name: &str, own: &[String]) -> Value {
    let cells: Map<String, Value> = COLUMNS.iter().map(|c| ((*c).to_string(), json!(gives(name, own, c)))).collect();
    Value::Object(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_name_subcommands() {
        for name in [OUTSIDE, STDIN, STDOUT, OWN_STDOUT, JOBS, LAYER, NO_OPTIONS_FILE].concat() {
            assert!(crate::sub::find(name).is_some(), "{name}");
        }
        assert!(refuses("bench", "dry-run") && !refuses("compile", "dry-run"));
        assert!(refuses("merge", "layer") && !refuses("compile", "layer"));
    }
}
