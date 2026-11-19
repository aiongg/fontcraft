//! `fontcraft-cli completion` (row `completion`): prints a completion script for bash, zsh, fish
//! or PowerShell.
//!
//! The scripts are generated from the subcommand table ([`super::ALL`]), so they offer exactly
//! what the binary accepts: subcommand names, the actions of multi-action subcommands, every
//! option (with the `--no-` form of switches) and the enumerated values of options and of the
//! first positional argument. A subcommand's options come from its option table and, for
//! subcommands that parse their own arguments, from the option names its usage text shows; an
//! enumerated value is an alternative of a `a|b|c` group in either. Everything else falls back to
//! the shell's file-name completion. Every subcommand also completes the common options
//! ([`crate::args::GLOBAL`]) it does not declare itself.

use serde_json::{Value, json};

use crate::args::{GLOBAL, Kind, OptSpec};
use crate::io::{Outcome, out, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "completion", summary: "Print a shell completion script", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli completion bash|zsh|fish|powershell
      Print a completion script for the shell to standard output. Load it with
      `eval \"$(fontcraft-cli completion bash)\"` (bash, zsh), `fontcraft-cli completion fish | source`
      (fish) or `fontcraft-cli completion powershell | Out-String | Invoke-Expression` (PowerShell).
";

/// The options, as data: none besides `--help`.
const OPTIONS: &[OptSpec] = &[];

/// The shells a script can be generated for.
pub const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];

/// The program name the scripts complete.
const PROGRAM: &str = "fontcraft-cli";

/// Words usage texts write in capitals that are placeholders, never literal values.
const PLACEHOLDERS: &[&str] = &[
    "N", "NAME", "FILE", "DIR", "FONT", "PATH", "LIST", "ID", "TAG", "INPUT", "OUT", "SOURCE", "PATTERN", "ADDR", "SECS", "EM", "PCT", "TEXT",
    "VIEW", "LEVEL", "INDEX", "UNITS", "KEY", "JSON",
];

fn run(args: &[String]) -> Outcome {
    let parsed = crate::args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let shell = match parsed.positional.as_slice() {
        [shell] => shell.as_str(),
        [] => return Err(usage(super::help(&SUBCOMMAND))),
        _ => return Err(usage("completion takes one shell name")),
    };
    let mut model = model(super::ALL);
    for entry in &mut model {
        for spec in GLOBAL {
            if !entry.opts.iter().any(|o| o.long.as_deref() == Some(spec.name)) {
                let mut opt = from_spec(spec);
                if entry.opts.iter().any(|o| o.short.is_some() && o.short == opt.short) {
                    opt.short = None;
                }
                entry.opts.push(opt);
            }
        }
    }
    let script = script(shell, &model).ok_or_else(|| usage(format!("unknown shell `{shell}` (shells: {})", SHELLS.join(", "))))?;
    out(script.trim_end())
}

/// The script for `shell`, or `None` for a shell we do not support.
fn script(shell: &str, model: &[Entry]) -> Option<String> {
    match shell {
        "bash" => Some(bash(model)),
        "zsh" => Some(zsh(model)),
        "fish" => Some(fish(model)),
        "powershell" | "pwsh" => Some(powershell(model)),
        _ => None,
    }
}

/// One option as the scripts offer it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Opt {
    long: Option<String>,
    short: Option<char>,
    takes_value: bool,
    /// The enumerated values, when the value is one of a fixed set.
    values: Vec<String>,
    /// A switch with a `--no-` form.
    negatable: bool,
    help: String,
}

impl Opt {
    /// The option as `commands --cli --json` lists it.
    pub(super) fn to_json(&self) -> Value {
        json!({
            "long": self.long,
            "short": self.short.map(String::from),
            "takes_value": self.takes_value,
            "values": self.values,
            "negatable": self.negatable,
            "help": self.help,
        })
    }

    /// The spellings that name it on a command line (`--x`, `--no-x`, `-x`).
    fn forms(&self) -> Vec<String> {
        let mut forms = Vec::new();
        if let Some(long) = &self.long {
            forms.push(format!("--{long}"));
            if self.negatable {
                forms.push(format!("--no-{long}"));
            }
        }
        if let Some(c) = self.short {
            forms.push(format!("-{c}"));
        }
        forms
    }

    /// The spellings after which a value is expected.
    fn value_forms(&self) -> Vec<String> {
        if !self.takes_value {
            return Vec::new();
        }
        self.long.iter().map(|l| format!("--{l}")).chain(self.short.map(|c| format!("-{c}"))).collect()
    }
}

/// One subcommand as the scripts offer it.
#[derive(Clone, Debug)]
pub(super) struct Entry {
    name: &'static str,
    summary: &'static str,
    /// Actions and enumerated values of the first positional argument.
    words: Vec<String>,
    pub(super) opts: Vec<Opt>,
}

/// What the scripts offer for every subcommand, from its own options and usage text, in the
/// order of `subs`.
pub(super) fn model(subs: &[Subcommand]) -> Vec<Entry> {
    subs.iter()
        .map(|sub| {
            let mut entry = Entry { name: sub.name, summary: sub.summary, words: Vec::new(), opts: Vec::new() };
            for action in sub.actions {
                push_word(&mut entry.words, action);
            }
            for spec in sub.options {
                merge(&mut entry.opts, from_spec(spec));
            }
            scrape_usage(sub.usage, &mut entry);
            entry
        })
        .collect()
}

fn push_word(words: &mut Vec<String>, word: &str) {
    if !words.iter().any(|w| w == word) {
        words.push(word.to_string());
    }
}

pub(super) fn from_spec(spec: &OptSpec) -> Opt {
    Opt {
        long: Some(spec.name.to_string()),
        short: spec.short,
        takes_value: spec.kind != Kind::Switch,
        values: if spec.kind == Kind::Switch { Vec::new() } else { literals(spec.value_name) },
        negatable: spec.kind == Kind::Switch,
        help: spec.help.to_string(),
    }
}

/// Adds `opt`, or folds it into the option with the same long or short name.
fn merge(opts: &mut Vec<Opt>, opt: Opt) {
    let same = opts.iter_mut().find(|o| (opt.long.is_some() && o.long == opt.long) || (opt.short.is_some() && o.short == opt.short));
    let Some(existing) = same else {
        opts.push(opt);
        return;
    };
    if existing.long.is_none() {
        existing.long = opt.long;
    }
    if existing.short.is_none() {
        existing.short = opt.short;
    }
    existing.takes_value |= opt.takes_value;
    existing.negatable |= opt.negatable;
    for v in opt.values {
        push_word(&mut existing.values, &v);
    }
    if existing.help.is_empty() {
        existing.help = opt.help;
    }
}

/// The options and positional value sets a usage text shows: `--name`, `-x`, a placeholder or
/// `a|b` group after an option (it takes a value), and `a|b` groups on their own (positional
/// words).
fn scrape_usage(text: &str, entry: &mut Entry) {
    let tokens: Vec<&str> = synopsis_lines(text).flat_map(str::split_whitespace).map(clean).collect();
    for (i, token) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1).copied().unwrap_or("");
        if let Some(body) = token.strip_prefix("--") {
            // `--name`, `--name=VALUE`, `--name[=VALUE]`
            let name: String = body.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            if !valid_long(&name) {
                continue;
            }
            let attached = body.get(name.len()..).unwrap_or("");
            let (takes_value, values) = if let Some(value) = attached.strip_prefix('=').or_else(|| attached.strip_prefix("[=")) {
                (true, literals(value))
            } else if attached.is_empty() && is_value(next) {
                (true, literals(next))
            } else {
                (false, Vec::new())
            };
            merge(&mut entry.opts, Opt { long: Some(name), takes_value, values, ..Opt::default() });
        } else if let Some(c) = short_option(token) {
            let takes_value = is_value(next);
            let values = if takes_value { literals(next) } else { Vec::new() };
            merge(&mut entry.opts, Opt { short: Some(c), takes_value, values, ..Opt::default() });
        } else if token.contains('|') {
            let after_option = i.checked_sub(1).and_then(|p| tokens.get(p)).is_some_and(|prev| prev.starts_with('-') && prev.len() > 1);
            if !after_option {
                for word in literals(token).into_iter().filter(|w| w.chars().any(|c| c.is_ascii_lowercase())) {
                    push_word(&mut entry.words, &word);
                }
            }
        }
    }
}

/// The synopsis lines of a usage text, without its prose: a synopsis line starts with the
/// program name, or with `[` where a long synopsis continues.
fn synopsis_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|l| {
        let l = l.trim_start();
        l.starts_with("fontcraft-cli ") || l.starts_with('[')
    })
}

/// A usage token without the brackets, parentheses, repetition marks and punctuation around it.
fn clean(token: &str) -> &str {
    token.trim_start_matches(['[', '(', '`', '"']).trim_end_matches(['.', ']', ')', ',', ':', ';', '`', '"'])
}

fn valid_long(name: &str) -> bool {
    name.len() > 1 && name.starts_with(|c: char| c.is_ascii_lowercase()) && !name.ends_with('-')
}

fn short_option(token: &str) -> Option<char> {
    let mut chars = token.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some('-'), Some(c), None) if c.is_ascii_alphabetic() => Some(c),
        _ => None,
    }
}

/// Whether a usage token after an option is its value: a placeholder in capitals or a `a|b`
/// group, not another option, a `|` between alternatives or a word of prose.
fn is_value(token: &str) -> bool {
    !token.is_empty() && !token.starts_with('-') && token != "|" && (token.starts_with(|c: char| c.is_ascii_uppercase()) || token.contains('|'))
}

/// The literal values of a `a|b|c` group; empty for a placeholder. Alternatives in capitals are
/// literals (table tags) unless they are placeholder words.
fn literals(text: &str) -> Vec<String> {
    if !text.contains('|') {
        return Vec::new();
    }
    let mut values = Vec::new();
    for alt in text.split('|') {
        let alt = alt.split(['=', '[']).next().unwrap_or("");
        let alt = alt.trim_end_matches("...").trim_end_matches(",").trim_matches(['[', ']', '(', ')', ',']);
        let safe = !alt.is_empty() && alt.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'));
        if safe && !PLACEHOLDERS.contains(&alt) && !alt.ends_with("...") {
            push_word(&mut values, alt);
        }
    }
    values
}

/// Top-level words besides the subcommands.
const TOP_LEVEL: &[&str] = &["help", "--help", "--version", "-V"];

fn bash(model: &[Entry]) -> String {
    let top: Vec<&str> = model.iter().map(|e| e.name).chain(TOP_LEVEL.iter().copied()).collect();
    let mut s = String::new();
    s.push_str(&format!(
        "# bash completion for {PROGRAM}\n# Load it with: eval \"$({PROGRAM} completion bash)\"\n\n_fontcraft_cli() {{\n    local cur prev words opts values\n    cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n    prev=\"${{COMP_WORDS[COMP_CWORD-1]}}\"\n    if [ \"$COMP_CWORD\" -eq 1 ]; then\n        COMPREPLY=($(compgen -W \"{}\" -- \"$cur\"))\n        return 0\n    fi\n    words=\"\"\n    opts=\"\"\n    values=\"\"\n    case \"${{COMP_WORDS[1]}}\" in\n",
        top.join(" ")
    ));
    for e in model {
        let opts: Vec<String> = e.opts.iter().flat_map(Opt::forms).collect();
        s.push_str(&format!("        {})\n            words=\"{}\"\n            opts=\"{}\"\n", e.name, e.words.join(" "), opts.join(" ")));
        let with_values: Vec<&Opt> = e.opts.iter().filter(|o| !o.values.is_empty()).collect();
        if !with_values.is_empty() {
            s.push_str("            case \"$prev\" in\n");
            for o in with_values {
                s.push_str(&format!("                {}) values=\"{}\" ;;\n", o.value_forms().join("|"), o.values.join(" ")));
            }
            s.push_str("            esac\n");
        }
        s.push_str("            ;;\n");
    }
    s.push_str(
        "    esac\n    if [ -n \"$values\" ]; then\n        COMPREPLY=($(compgen -W \"$values\" -- \"$cur\"))\n    elif [[ \"$cur\" == -* ]]; then\n        COMPREPLY=($(compgen -W \"$opts\" -- \"$cur\"))\n    elif [ \"$COMP_CWORD\" -eq 2 ] && [ -n \"$words\" ]; then\n        COMPREPLY=($(compgen -W \"$words\" -- \"$cur\"))\n    else\n        COMPREPLY=()\n    fi\n    return 0\n}\n\n",
    );
    s.push_str(&format!("complete -o default -F _fontcraft_cli {PROGRAM}\n"));
    s
}

/// A single-quoted word for bash, zsh or fish (fish takes `\'`, the others `'\''`).
fn sq(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn zsh(model: &[Entry]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "#compdef {PROGRAM}\n# zsh completion for {PROGRAM}\n# Load it with: eval \"$({PROGRAM} completion zsh)\", or save it as _{PROGRAM} in a folder of $fpath.\n\n_{PROGRAM}() {{\n    local -a subs opts actions values\n    subs=(\n"
    ));
    for e in model {
        s.push_str(&format!("        {}\n", sq(&format!("{}:{}", e.name, e.summary))));
    }
    s.push_str("        'help:Print the usage'\n    )\n    if (( CURRENT == 2 )); then\n        _describe 'subcommand' subs\n        return\n    fi\n    local prev=${words[CURRENT-1]} cur=${words[CURRENT]}\n    case ${words[2]} in\n");
    for e in model {
        s.push_str(&format!("        {})\n", e.name));
        let opts: Vec<String> = e
            .opts
            .iter()
            .flat_map(|o| {
                let help = o.help.replace('\n', " ");
                o.forms().into_iter().map(move |f| sq(&if help.is_empty() { f } else { format!("{f}:{help}") }))
            })
            .collect();
        s.push_str(&format!("            opts=({})\n            actions=({})\n", opts.join(" "), e.words.join(" ")));
        let with_values: Vec<&Opt> = e.opts.iter().filter(|o| !o.values.is_empty()).collect();
        if !with_values.is_empty() {
            s.push_str("            case $prev in\n");
            for o in with_values {
                s.push_str(&format!("                {}) values=({}) ;;\n", o.value_forms().join("|"), o.values.join(" ")));
            }
            s.push_str("            esac\n");
        }
        s.push_str("            ;;\n");
    }
    s.push_str(&format!(
        "    esac\n    if (( ${{#values}} )); then\n        compadd -a values\n    elif [[ $cur == -* ]] && (( ${{#opts}} )); then\n        _describe 'option' opts\n    elif (( CURRENT == 3 && ${{#actions}} )); then\n        compadd -a actions\n    else\n        _files\n    fi\n}}\n\nif [ \"$funcstack[1]\" = \"_{PROGRAM}\" ]; then\n    _{PROGRAM} \"$@\"\nelse\n    compdef _{PROGRAM} {PROGRAM}\nfi\n"
    ));
    s
}

/// A single-quoted word for fish.
fn fish_sq(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn fish(model: &[Entry]) -> String {
    let mut s = format!("# fish completion for {PROGRAM}\n# Load it with: {PROGRAM} completion fish | source\n\n");
    let c = format!("complete -c {PROGRAM}");
    for e in model {
        s.push_str(&format!("{c} -n __fish_use_subcommand -f -a {} -d {}\n", e.name, fish_sq(e.summary)));
    }
    s.push_str(&format!("{c} -n __fish_use_subcommand -f -a help -d 'Print the usage'\n"));
    s.push_str(&format!(
        "{c} -n __fish_use_subcommand -l help -d 'Print the usage'\n{c} -n __fish_use_subcommand -l version -d 'Print the version'\n"
    ));
    for e in model {
        let when = format!("-n {}", fish_sq(&format!("__fish_seen_subcommand_from {}", e.name)));
        if !e.words.is_empty() {
            let first = fish_sq(&format!("__fish_seen_subcommand_from {}; and test (count (commandline -opc)) -eq 2", e.name));
            s.push_str(&format!("{c} -n {first} -f -a {}\n", fish_sq(&e.words.join(" "))));
        }
        for o in &e.opts {
            let mut line = format!("{c} {when}");
            if let Some(ch) = o.short {
                line.push_str(&format!(" -s {ch}"));
            }
            if let Some(long) = &o.long {
                line.push_str(&format!(" -l {long}"));
            }
            if !o.values.is_empty() {
                line.push_str(&format!(" -x -a {}", fish_sq(&o.values.join(" "))));
            } else if o.takes_value {
                line.push_str(" -r");
            }
            if !o.help.is_empty() {
                line.push_str(&format!(" -d {}", fish_sq(&o.help)));
            }
            s.push_str(&line);
            s.push('\n');
            if let (true, Some(long)) = (o.negatable, &o.long) {
                s.push_str(&format!("{c} {when} -l no-{long}\n"));
            }
        }
    }
    s
}

/// A single-quoted string for PowerShell.
fn ps_sq(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn ps_array<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let items: Vec<String> = items.into_iter().map(ps_sq).collect();
    format!("@({})", items.join(", "))
}

fn powershell(model: &[Entry]) -> String {
    let mut s = format!(
        "# PowerShell completion for {PROGRAM}\n# Load it with: {PROGRAM} completion powershell | Out-String | Invoke-Expression\n\nRegister-ArgumentCompleter -Native -CommandName {PROGRAM} -ScriptBlock {{\n    param($wordToComplete, $commandAst, $cursorPosition)\n    $subs = [ordered]@{{\n"
    );
    for e in model {
        let opts: Vec<String> = e.opts.iter().flat_map(Opt::forms).collect();
        let values: Vec<String> = e
            .opts
            .iter()
            .filter(|o| !o.values.is_empty())
            .flat_map(|o| {
                let vals = ps_array(o.values.iter().map(String::as_str));
                o.value_forms().into_iter().map(move |f| format!("{} = {vals}", ps_sq(&f)))
            })
            .collect();
        s.push_str(&format!(
            "        {} = @{{ wordList = {}; optList = {}; valueMap = @{{ {} }} }}\n",
            ps_sq(e.name),
            ps_array(e.words.iter().map(String::as_str)),
            ps_array(opts.iter().map(String::as_str)),
            values.join("; ")
        ));
    }
    s.push_str(&format!(
        "    }}\n    $elements = @($commandAst.CommandElements | ForEach-Object {{ $_.ToString() }})\n    $count = $elements.Count\n    if ($wordToComplete -ne '') {{ $count = $count - 1 }}\n    $candidates = @()\n    if ($count -le 1) {{\n        $candidates = @($subs.Keys) + {}\n    }} else {{\n        $sub = $subs[$elements[1]]\n        if ($sub) {{\n            $prev = $elements[$count - 1]\n            if ($sub.valueMap.ContainsKey($prev)) {{\n                $candidates = $sub.valueMap[$prev]\n            }} elseif ($wordToComplete.StartsWith('-')) {{\n                $candidates = $sub.optList\n            }} elseif ($count -eq 2) {{\n                $candidates = $sub.wordList\n            }}\n        }}\n    }}\n    $candidates | Where-Object {{ $_ -like \"$wordToComplete*\" }} | ForEach-Object {{\n        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)\n    }}\n}}\n",
        ps_array(TOP_LEVEL.iter().copied())
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `--name` and `-x` a usage text shows, found by a plain scan.
    fn usage_options(text: &str) -> Vec<String> {
        synopsis_lines(text)
            .flat_map(|l| {
                l.split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '(' | ')' | '=' | ',' | ':' | '`' | '"'))
                    .filter(|t| (t.starts_with("--") && valid_long(t.get(2..).unwrap_or("").trim_end_matches('.'))) || short_option(t).is_some())
            })
            .map(|t| t.trim_end_matches('.').to_string())
            .collect()
    }

    #[test]
    fn completion_offers_every_subcommand_and_option() {
        let model = model(super::super::ALL);
        for sub in super::super::ALL {
            let entry = model.iter().find(|e| e.name == sub.name).unwrap();
            let forms: Vec<String> = entry.opts.iter().flat_map(Opt::forms).collect();
            for spec in sub.options {
                assert!(forms.contains(&format!("--{}", spec.name)), "{}: --{}", sub.name, spec.name);
                if spec.kind == Kind::Switch {
                    assert!(forms.contains(&format!("--no-{}", spec.name)), "{}: --no-{}", sub.name, spec.name);
                }
            }
            for opt in usage_options(sub.usage) {
                assert!(forms.contains(&opt), "{}: {opt} from the usage text", sub.name);
            }
            for action in sub.actions {
                assert!(entry.words.iter().any(|w| w == action), "{}: action {action}", sub.name);
            }
        }
        for shell in SHELLS {
            let text = script(shell, &model).unwrap();
            for e in &model {
                assert!(text.contains(e.name), "{shell}: {}", e.name);
                for w in &e.words {
                    assert!(text.contains(w.as_str()), "{shell}: {} {w}", e.name);
                }
                for o in &e.opts {
                    let wanted = match (*shell, &o.long) {
                        ("fish", Some(long)) => vec![format!("-l {long}")],
                        ("fish", None) => o.short.map(|c| format!("-s {c}")).into_iter().collect(),
                        _ => o.forms(),
                    };
                    for w in wanted {
                        assert!(text.contains(&w), "{shell}: {} {w}", e.name);
                    }
                    for v in &o.values {
                        assert!(text.contains(v.as_str()), "{shell}: {} value {v}", e.name);
                    }
                }
            }
        }
    }

    #[test]
    fn usage_texts_give_option_values_and_positional_words() {
        let mut entry = Entry { name: "x", summary: "", words: Vec::new(), opts: Vec::new() };
        scrape_usage(
            "  fontcraft-cli x bash|zsh FONT [--format ttf|otf]... [-o FILE | --output-dir DIR] [--instances[=NAME,...]] [--face N|all]\n                [--plan] [--table GSUB|GPOS,...]\n      --prose-only prints it.",
            &mut entry,
        );
        assert_eq!(entry.words, ["bash", "zsh"]);
        let get = |name: &str| entry.opts.iter().find(|o| o.long.as_deref() == Some(name)).unwrap().clone();
        assert_eq!(get("format").values, ["ttf", "otf"]);
        assert!(get("output-dir").takes_value && get("output-dir").values.is_empty());
        assert!(get("instances").takes_value, "an attached optional value");
        assert_eq!(get("face").values, ["all"], "N is a placeholder");
        assert!(!get("plan").takes_value);
        assert!(entry.opts.iter().all(|o| o.long.as_deref() != Some("prose-only")), "prose is not read");
        assert_eq!(get("table").values, ["GSUB", "GPOS"]);
        let o = entry.opts.iter().find(|o| o.short == Some('o')).unwrap();
        assert!(o.takes_value);
    }

    #[test]
    fn unknown_shell_is_a_usage_error() {
        let e = run(&["tcsh".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert_eq!(run(&[]).unwrap_err().exit_code(), 2);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-cli-completion-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The bash script, sourced into bash and asked as bash asks it.
    #[test]
    fn bash_script_completes_subcommands_actions_options_and_values() {
        if std::process::Command::new("bash").arg("--version").output().is_err() {
            return;
        }
        let dir = scratch("bash");
        let file = dir.join("fontcraft-cli.bash");
        std::fs::write(&file, bash(&model(super::super::ALL))).unwrap();
        let ask = |words: &str, cword: usize| -> String {
            let driver = format!("source '{}'; COMP_WORDS=({words}); COMP_CWORD={cword}; _fontcraft_cli; echo \"${{COMPREPLY[*]}}\"", file.display());
            let out = std::process::Command::new("bash").args(["--norc", "--noprofile", "-c", &driver]).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let subs = ask("fontcraft-cli co", 1);
        assert!(subs.split(' ').any(|w| w == "completion") && subs.split(' ').any(|w| w == "compile"), "{subs}");
        assert_eq!(ask("fontcraft-cli completion ''", 2), "bash zsh fish powershell");
        assert_eq!(ask("fontcraft-cli compile in.glyphs --format ''", 4), "ttf otf");
        assert!(ask("fontcraft-cli tables ''", 2).split(' ').any(|w| w == "list"));
        assert!(ask("fontcraft-cli bench --ru", 2).contains("--runs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The zsh script parses (zsh -n), when zsh is installed.
    #[test]
    fn zsh_script_parses() {
        if std::process::Command::new("zsh").arg("--version").output().is_err() {
            return;
        }
        let dir = scratch("zsh");
        let file = dir.join("_fontcraft-cli");
        std::fs::write(&file, zsh(&model(super::super::ALL))).unwrap();
        let out = std::process::Command::new("zsh").arg("-n").arg(&file).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
