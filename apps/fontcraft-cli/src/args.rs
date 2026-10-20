//! The option parser (`docs/plans/M12.md` § 3, "The option parser"; rows
//! `cli.global.responseFiles`, `.optionsFile`, `.optionOrder`, `.booleanFlags`, `.listValues`,
//! `.abbreviations`).
//!
//! Each subcommand declares its options as data ([`OptSpec`]); [`parse`] reads a command line
//! against them and [`help`] prints them. Rules:
//!
//! - `--x` turns a switch on and `--no-x` off.
//! - A value follows as `--opt=v` or `--opt v`. An option whose value is optional
//!   ([`OptSpec::optional_value`]) takes the next argument only when it does not start with `-`
//!   and holds no `=`; given bare, its value is empty. A list is comma-separated (`--opt=a,b`) and
//!   replaces the current list, unless every item starts with `+` or `-`, which adds or removes
//!   items from it (`--opt=+c,-a`). `--opt+=a,b` adds items and `--opt-=a,b` removes them, and
//!   `--opt=` empties the list. The current list starts as the default. Items that are numbers
//!   compare by value when removed (`0x409` removes `1033`); in a list of hexadecimal code
//!   points ([`OptSpec::hex`]) they are read as hexadecimal (`E9` removes `U+00E9`).
//! - `@FILE` and `--options FILE` are replaced by the arguments the file holds (whitespace
//!   separated, quotes group, `#` starts a comment line), at most [`MAX_RESPONSE_FILE`] bytes,
//!   nested at most [`MAX_RESPONSE_DEPTH`] deep, a file that includes itself refused.
//! - Later options win: the command line is read left to right after expansion.
//! - A long option may be shortened to any prefix that names one option only; a name that
//!   differs from an option only in letter case names that option (`--name-IDs`).
//! - `--` ends the options; `-h` and `--help` ask for help; a lone `-` is a positional (stdin).
//! - Where a subcommand reads queries ([`parse_with_queries`]), `--opt?` records the value
//!   `opt` has at that point of the line; elsewhere it is an unknown option.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::io::{Failure, usage};

/// Largest response or options file, in bytes.
pub const MAX_RESPONSE_FILE: usize = 1 << 20;
/// Deepest nesting of response and options files.
pub const MAX_RESPONSE_DEPTH: usize = 8;

/// What an option takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// On or off: `--x`, `--no-x`.
    Switch,
    /// One value.
    Value,
    /// One value that may be left out: `--opt`, `--opt v`, `--opt=v` ([`OptSpec::optional_value`]).
    OptionalValue,
    /// A comma-separated list with `+`/`-` edits.
    List,
}

/// One option of a subcommand.
#[derive(Clone, Copy, Debug)]
pub struct OptSpec {
    /// The long name without dashes (`retain-gids`).
    pub name: &'static str,
    pub short: Option<char>,
    pub kind: Kind,
    /// The placeholder help shows for the value (`LIST`, `FILE`); empty for a switch.
    pub value_name: &'static str,
    /// The default as text: `true`/`false` for a switch, comma-separated for a list, empty for
    /// none.
    pub default: &'static str,
    pub help: &'static str,
    /// The list's items are hexadecimal code points or ranges (`41`, `U+0041`, `41-5A`).
    pub hex: bool,
}

impl OptSpec {
    pub const fn switch(name: &'static str, default: bool, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::Switch, value_name: "", default: if default { "true" } else { "false" }, help, hex: false }
    }
    pub const fn value(name: &'static str, value_name: &'static str, default: &'static str, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::Value, value_name, default, help, hex: false }
    }
    pub const fn list(name: &'static str, value_name: &'static str, default: &'static str, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::List, value_name, default, help, hex: false }
    }
    /// A value option whose value may be left out; given bare, its value is empty.
    pub const fn optional_value(name: &'static str, value_name: &'static str, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::OptionalValue, value_name, default: "", help, hex: false }
    }
    pub const fn short(mut self, c: char) -> Self {
        self.short = Some(c);
        self
    }
    pub const fn hex(mut self) -> Self {
        self.hex = true;
        self
    }
}

/// The value of one option after parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptValue {
    Switch(bool),
    Value(String),
    List(Vec<String>),
}

/// A parsed command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub positional: Vec<String>,
    /// Every option with a default or a value given, by long name.
    pub values: BTreeMap<&'static str, OptValue>,
    /// `-h` or `--help` was given.
    pub help: bool,
    /// The `--opt?` queries, in order, read by [`parse_with_queries`].
    pub queries: Vec<Query>,
}

/// One `--opt?` on a command line: the option's value at that point of the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// The name as given, without dashes, `no-` or `?` (`name-IDs`).
    pub given: String,
    /// The option's long name.
    pub name: &'static str,
    /// The value set by the defaults and the options to the left; `None` for a value option
    /// with no default that was not given.
    pub value: Option<OptValue>,
}

impl Parsed {
    /// A switch's value; false for an unknown name.
    pub fn switch(&self, name: &str) -> bool {
        matches!(self.values.get(name), Some(OptValue::Switch(true)))
    }

    /// A value option's value, when given or defaulted.
    pub fn value(&self, name: &str) -> Option<&str> {
        match self.values.get(name) {
            Some(OptValue::Value(v)) => Some(v),
            _ => None,
        }
    }

    /// A list option's items; empty when unset.
    pub fn list(&self, name: &str) -> &[String] {
        match self.values.get(name) {
            Some(OptValue::List(v)) => v,
            _ => &[],
        }
    }
}

/// Reads `args` against `specs`.
pub fn parse(specs: &[OptSpec], args: &[String]) -> Result<Parsed, Failure> {
    parse_inner(specs, args, false)
}

/// Reads `args` against `specs`, also accepting `--opt?` (and `--no-opt?`), which records the
/// option's value at that point of the line in [`Parsed::queries`].
pub fn parse_with_queries(specs: &[OptSpec], args: &[String]) -> Result<Parsed, Failure> {
    parse_inner(specs, args, true)
}

fn parse_inner(specs: &[OptSpec], args: &[String], queries: bool) -> Result<Parsed, Failure> {
    let args = expand(args, 0, &mut Vec::new())?;
    let mut parsed = Parsed::default();
    for spec in specs {
        let value = match spec.kind {
            Kind::Switch => OptValue::Switch(spec.default == "true"),
            Kind::Value | Kind::OptionalValue if spec.default.is_empty() => continue,
            Kind::Value | Kind::OptionalValue => OptValue::Value(spec.default.to_string()),
            Kind::List => OptValue::List(split_list(spec.default)),
        };
        parsed.values.insert(spec.name, value);
    }
    let mut it = args.into_iter().peekable();
    while let Some(arg) = it.next() {
        if arg == "--" {
            parsed.positional.extend(it.by_ref());
            break;
        }
        if arg == "-h" || arg == "--help" {
            parsed.help = true;
            continue;
        }
        if let Some(body) = arg.strip_prefix("--") {
            if queries
                && !body.contains('=')
                && let Some(name) = body.strip_suffix('?')
            {
                let (spec, on) = resolve(specs, name)?;
                let given = if on { name } else { name.get(3..).unwrap_or(name) };
                parsed.queries.push(Query { given: given.to_string(), name: spec.name, value: parsed.values.get(spec.name).cloned() });
                continue;
            }
            let (name, edit, inline) = match body.split_once('=') {
                Some((n, v)) => match (n.strip_suffix('+'), n.strip_suffix('-')) {
                    (Some(n), _) => (n, Edit::Add, Some(v.to_string())),
                    (_, Some(n)) => (n, Edit::Remove, Some(v.to_string())),
                    _ => (n, Edit::Set, Some(v.to_string())),
                },
                None => (body, Edit::Set, None),
            };
            let (spec, on) = resolve(specs, name)?;
            if edit != Edit::Set && spec.kind != Kind::List {
                return Err(usage(format!("--{}: += and -= edit list options only", spec.name)));
            }
            match spec.kind {
                Kind::Switch => {
                    if inline.is_some() {
                        return Err(usage(format!("--{} takes no value", spec.name)));
                    }
                    parsed.values.insert(spec.name, OptValue::Switch(on));
                }
                Kind::Value | Kind::OptionalValue | Kind::List => {
                    if !on {
                        return Err(usage(format!("--no-{} is not an option: --{} takes a value", spec.name, spec.name)));
                    }
                    let value = match inline {
                        Some(v) => v,
                        None if spec.kind == Kind::OptionalValue => optional_value(&mut it),
                        None => it.next().ok_or_else(|| usage(format!("--{} needs a value", spec.name)))?,
                    };
                    set_value(&mut parsed, spec, value, edit);
                }
            }
            continue;
        }
        let mut chars = arg.chars();
        if let (Some('-'), Some(c), None) = (chars.next(), chars.next(), chars.next()) {
            let spec = specs.iter().find(|s| s.short == Some(c)).ok_or_else(|| usage(format!("unknown option -{c}")))?;
            match spec.kind {
                Kind::Switch => {
                    parsed.values.insert(spec.name, OptValue::Switch(true));
                }
                Kind::OptionalValue => {
                    let value = optional_value(&mut it);
                    set_value(&mut parsed, spec, value, Edit::Set);
                }
                Kind::Value | Kind::List => {
                    let value = it.next().ok_or_else(|| usage(format!("-{c} needs a value")))?;
                    set_value(&mut parsed, spec, value, Edit::Set);
                }
            }
            continue;
        }
        parsed.positional.push(arg);
    }
    Ok(parsed)
}

/// The value of an option whose value may be left out: the next argument when it is not an
/// option and not a `NAME=VALUE` argument, else empty.
fn optional_value(it: &mut std::iter::Peekable<std::vec::IntoIter<String>>) -> String {
    it.next_if(|next| !next.starts_with('-') && !next.contains('=')).unwrap_or_default()
}

/// How a list option's value changes the current list: `=`, `+=` or `-=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Set,
    Add,
    Remove,
}

fn set_value(parsed: &mut Parsed, spec: &OptSpec, value: String, edit: Edit) {
    match spec.kind {
        Kind::Value | Kind::OptionalValue => {
            parsed.values.insert(spec.name, OptValue::Value(value));
        }
        Kind::List => {
            let items = split_list(&value);
            let mut current = parsed.list(spec.name).to_vec();
            let edits = !items.is_empty() && items.iter().all(|i| i.len() > 1 && (i.starts_with('+') || i.starts_with('-')));
            let list = match edit {
                Edit::Add => {
                    for item in items {
                        if !current.iter().any(|c| same_item(spec, c, &item)) {
                            current.push(item);
                        }
                    }
                    current
                }
                Edit::Remove => {
                    current.retain(|c| !items.iter().any(|i| same_item(spec, c, i)));
                    current
                }
                Edit::Set if edits => {
                    for item in items {
                        let (op, name) = item.split_at(1);
                        current.retain(|c| !same_item(spec, c, name));
                        if op == "+" {
                            current.push(name.to_string());
                        }
                    }
                    current
                }
                Edit::Set => items,
            };
            parsed.values.insert(spec.name, OptValue::List(list));
        }
        Kind::Switch => {}
    }
}

/// Two items of the list `spec` are the same when their text is, or when both are numbers of
/// the same value: decimal, or hexadecimal, octal or binary with `0x`, `0o` or `0b`; for a
/// [`OptSpec::hex`] list, hexadecimal code points or ranges.
fn same_item(spec: &OptSpec, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if spec.hex {
        return matches!((hex_item(a), hex_item(b)), (Some(x), Some(y)) if x == y);
    }
    matches!((int_item(a), int_item(b)), (Some(x), Some(y)) if x == y)
}

/// A code point (`E9`, `U+00E9`, `0xE9`) or range (`41-5A`) as its first and last value.
fn hex_item(text: &str) -> Option<(u32, u32)> {
    let point = |t: &str| {
        let lower = t.trim().to_ascii_lowercase();
        let digits = lower.strip_prefix("u+").or_else(|| lower.strip_prefix("0x")).unwrap_or(&lower);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok()
    };
    match text.split_once('-') {
        Some((first, last)) => Some((point(first)?, point(last)?)),
        None => point(text).map(|p| (p, p)),
    }
}

fn int_item(text: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let (digits, radix) = match lower.get(..2) {
        Some("0x") => (lower.get(2..)?, 16),
        Some("0o") => (lower.get(2..)?, 8),
        Some("0b") => (lower.get(2..)?, 2),
        _ => (lower.as_str(), 10),
    };
    if digits.is_empty() || !digits.starts_with(|c: char| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, radix).ok()
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// The option `name` names, exactly or by a unique prefix, and whether it is the `no-` form.
fn resolve<'a>(specs: &'a [OptSpec], name: &str) -> Result<(&'a OptSpec, bool), Failure> {
    let forms = || {
        specs.iter().flat_map(|s| {
            let negative = (s.kind == Kind::Switch).then(|| (format!("no-{}", s.name), s, false));
            std::iter::once((s.name.to_string(), s, true)).chain(negative)
        })
    };
    if let Some((_, s, on)) = forms().find(|(n, _, _)| n == name) {
        return Ok((s, on));
    }
    if let Some((_, s, on)) = forms().find(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
        return Ok((s, on));
    }
    let matches: Vec<(String, &OptSpec, bool)> = forms().filter(|(n, _, _)| n.starts_with(name)).collect();
    match matches.as_slice() {
        [(_, s, on)] => Ok((s, *on)),
        [] => Err(usage(format!("unknown option --{name}"))),
        many => {
            let names: Vec<String> = many.iter().map(|(n, _, _)| format!("--{n}")).collect();
            Err(usage(format!("--{name} is ambiguous: {}", names.join(", "))))
        }
    }
}

/// `args` with every `@FILE` and `--options FILE` replaced by the file's arguments.
fn expand(args: &[String], depth: usize, stack: &mut Vec<PathBuf>) -> Result<Vec<String>, Failure> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let file = if let Some(path) = arg.strip_prefix('@').filter(|p| !p.is_empty()) {
            Some(path.to_string())
        } else if arg == "--options" {
            Some(it.next().ok_or_else(|| usage("--options needs a file"))?.clone())
        } else {
            arg.strip_prefix("--options=").map(str::to_string)
        };
        let Some(file) = file else {
            out.push(arg.clone());
            continue;
        };
        if depth >= MAX_RESPONSE_DEPTH {
            return Err(usage(format!("response files nest more than {MAX_RESPONSE_DEPTH} deep at {file}")));
        }
        let key = std::fs::canonicalize(&file).map_err(|e| Failure::Input(format!("cannot read {file}: {e}")))?;
        if stack.contains(&key) {
            return Err(usage(format!("{file} includes itself")));
        }
        let text = read_small(&file)?;
        stack.push(key);
        out.extend(expand(&split_words(&text), depth + 1, stack)?);
        stack.pop();
    }
    Ok(out)
}

fn read_small(path: &str) -> Result<String, Failure> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_RESPONSE_FILE as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))?;
    if bytes.len() > MAX_RESPONSE_FILE {
        return Err(Failure::Input(format!("{path} is larger than {MAX_RESPONSE_FILE} bytes")));
    }
    String::from_utf8(bytes).map_err(|_| Failure::Input(format!("{path} is not UTF-8 text")))
}

/// Words of a response file: separated by whitespace, grouped by single or double quotes, lines
/// starting with `#` skipped.
fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        let mut word = String::new();
        let mut in_word = false;
        let mut quote: Option<char> = None;
        for c in line.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => word.push(c),
                (None, '"' | '\'') => {
                    quote = Some(c);
                    in_word = true;
                }
                (None, c) if c.is_whitespace() => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                (None, c) => {
                    word.push(c);
                    in_word = true;
                }
            }
        }
        if in_word {
            words.push(word);
        }
    }
    words
}

/// The help text of a subcommand: its usage, then one line per option with its default.
pub fn help(usage_text: &str, specs: &[OptSpec]) -> String {
    let mut text = usage_text.trim_end().to_string();
    if !specs.is_empty() {
        text.push_str("\n\nOPTIONS:\n");
        for s in specs {
            let short = s.short.map(|c| format!("-{c}, ")).unwrap_or_default();
            let form = match s.kind {
                Kind::Switch => format!("{short}--[no-]{}", s.name),
                Kind::Value | Kind::OptionalValue | Kind::List => format!("{short}--{} {}", s.name, s.value_name),
            };
            let default = match (s.kind, s.default) {
                (_, "") | (Kind::Switch, "false") => String::new(),
                (Kind::Switch, _) => " (default: on)".to_string(),
                (_, d) => format!(" (default: {d})"),
            };
            text.push_str(&format!("  {form:<32} {}{default}\n", s.help));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECS: &[OptSpec] = &[
        OptSpec::switch("hinting", true, "keep hints"),
        OptSpec::switch("retain-gids", false, "keep glyph ids"),
        OptSpec::list("layout-features", "LIST", "kern,liga", "features to keep"),
        OptSpec::list("name-ids", "LIST", "0,1,2", "name IDs to keep"),
        OptSpec::list("unicodes", "LIST", "", "code points to keep").hex(),
        OptSpec::value("output", "FILE", "", "output file").short('o'),
        OptSpec::value("output-dir", "DIR", "", "output folder"),
    ];

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn switches_lists_and_prefixes() {
        let p = parse(SPECS, &args(&["in.ttf", "--no-hinting", "--retain", "--layout-features=+smcp,-kern", "-o", "out.ttf"])).unwrap();
        assert_eq!(p.positional, vec!["in.ttf"]);
        assert!(!p.switch("hinting"));
        assert!(p.switch("retain-gids"), "a unique prefix names the option");
        assert_eq!(p.list("layout-features"), ["liga", "smcp"], "edits apply to the default list");
        assert_eq!(p.value("output"), Some("out.ttf"));
        let p = parse(SPECS, &args(&["--layout-features", "ss01,ss02", "--layout-features=-ss01"])).unwrap();
        assert_eq!(p.list("layout-features"), ["ss02"], "a plain list replaces; later edits apply to it");
        assert!(parse(SPECS, &args(&["--out", "x"])).is_err(), "--out is ambiguous between output and output-dir");
        assert!(parse(SPECS, &args(&["--retain-gids=yes"])).is_err());
        assert!(parse(SPECS, &args(&["--bogus"])).is_err());
        let p = parse(SPECS, &args(&["--", "--hinting", "-"])).unwrap();
        assert_eq!(p.positional, vec!["--hinting", "-"]);
    }

    #[test]
    fn list_options_take_set_add_and_remove_forms() {
        let p = parse(SPECS, &args(&["--layout-features+=smcp,liga", "--layout-features-=kern"])).unwrap();
        assert_eq!(p.list("layout-features"), ["liga", "smcp"], "+= adds what is missing, -= removes");
        let p = parse(SPECS, &args(&["--layout-features="])).unwrap();
        assert!(p.list("layout-features").is_empty(), "an empty value empties the list");
        let p = parse(SPECS, &args(&["--layout-features=", "--layout-features+=ss01"])).unwrap();
        assert_eq!(p.list("layout-features"), ["ss01"]);
        let p = parse(SPECS, &args(&["--name-IDs-=0x1,2", "--name-ids+=0x10"])).unwrap();
        assert_eq!(p.list("name-ids"), ["0", "0x10"], "numbers compare by value; the name ignores letter case");
        let p = parse(SPECS, &args(&["--unicodes=00E9,0x10,41-5a", "--unicodes-=E9,16,U+0041-005A"])).unwrap();
        assert_eq!(p.list("unicodes"), ["0x10"], "code points compare as hexadecimal: 16 is U+0016");
        assert!(parse(SPECS, &args(&["--output+=x"])).is_err(), "+= edits lists only");
        assert!(parse(SPECS, &args(&["--hinting-=x"])).is_err());
    }

    #[test]
    fn response_files_nest_in_order_and_refuse_cycles() {
        let dir = std::env::temp_dir().join(format!("fontcraft-cli-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let opts = dir.join("opts.txt");
        std::fs::write(&opts, "# defaults for this project\n--no-hinting --output \"a b.ttf\"\n").unwrap();
        let o = opts.to_str().unwrap();
        let p = parse(SPECS, &args(&["--options", o, "--hinting"])).unwrap();
        assert!(p.switch("hinting"), "options after the file win");
        assert_eq!(p.value("output"), Some("a b.ttf"));
        let p = parse(SPECS, &args(&["--hinting", &format!("@{o}")])).unwrap();
        assert!(!p.switch("hinting"), "the file's options win over earlier ones");

        let looped = dir.join("loop.txt");
        std::fs::write(&looped, format!("@{}", looped.display())).unwrap();
        let e = parse(SPECS, &args(&[&format!("@{}", looped.display())])).unwrap_err();
        assert!(e.message().contains("includes itself"), "{e:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_lists_every_option() {
        let text = help("fontcraft-cli subset FONT", SPECS);
        assert!(text.contains("--[no-]hinting") && text.contains("-o, --output FILE") && text.contains("(default: kern,liga)"));
    }
}
