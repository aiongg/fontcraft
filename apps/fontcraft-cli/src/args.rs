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
//!   `--opt=` empties the list. A repeatable list ([`OptSpec::repeat`]) given again with a plain
//!   value adds its items (`--check a --check b`). The current list starts as the default. Items that are numbers
//!   compare by value when removed (`0x409` removes `1033`); in a list of hexadecimal code
//!   points ([`OptSpec::hex`]) they are read as hexadecimal (`E9` removes `U+00E9`).
//! - `@FILE` and `--options FILE` are replaced by the arguments the file holds (whitespace
//!   separated, quotes group, `#` starts a comment line), at most [`MAX_RESPONSE_FILE`] bytes,
//!   nested at most [`MAX_RESPONSE_DEPTH`] deep, a file that includes itself refused.
//! - Later options win: the command line is read left to right after expansion.
//! - A long option may be shortened to any prefix that names one option only; a name that
//!   differs from an option only in letter case names that option (`--name-IDs`). Subcommands
//!   that look for some options before [`parse`] first write the names out with [`spell_out`];
//!   those that read their own arguments resolve them with [`spell_long`], and [`take_globals`]
//!   leaves a prefix of one of their own options to them.
//! - `--` ends the options; `-h` and `--help` ask for help; a lone `-` is a positional (stdin).
//! - Where a subcommand reads queries ([`parse_with_queries`]), `--opt?` records the value
//!   `opt` has at that point of the line; elsewhere it is an unknown option.
//! - The common options ([`GLOBAL`]: `-q`, `-v`, `-vv`, `--log`, `--log-file`, `--color`,
//!   `--timing`, `--backtrace`, `-j`/`--jobs`, `-V`/`--version`) are added to every table. An
//!   option the subcommand declares with the same long name wins, and one with the same short
//!   letter keeps the letter. When a name could be either, the subcommand's own option is tried
//!   first at each step: exact name, then letter case, then prefix. [`parse`] applies the common
//!   options it reads ([`crate::io::configure`]); `-q`, `-v` and `--log` act in the order given.
//!   Subcommands that read their own arguments call [`take_globals`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::io::{self, Color, Failure, Level, Setup, out, usage};

/// Largest response or options file, in bytes.
pub const MAX_RESPONSE_FILE: usize = 1 << 20;
/// Deepest nesting of response and options files.
pub const MAX_RESPONSE_DEPTH: usize = 8;

/// The options every subcommand takes (`cli.global.logging`, `.color`, `.timing`, `.backtrace`,
/// `.parallelism`, `.help`), added to its own by [`parse`].
pub const GLOBAL: &[OptSpec] = &[
    OptSpec::switch("quiet", false, "print errors only").short('q'),
    OptSpec::switch("verbose", false, "also print informational messages; -vv adds debug output").short('v'),
    OptSpec::value("log", "LEVEL", "", "error, warning, note (default), info or debug, then per-module levels: info,engine=debug"),
    OptSpec::value("log-file", "FILE", "", "also write the messages to FILE"),
    OptSpec::value("color", "auto|always|never", "", "colour the messages (default: auto, on a terminal unless NO_COLOR is set)"),
    OptSpec::switch("timing", false, "print the time spent in each stage when the run ends"),
    OptSpec::switch("backtrace", false, "print the stack with an internal error, for bug reports"),
    OptSpec::value("jobs", "N", "", "worker threads where inputs are worked on at once: 0 every core (default), -N all but N").short('j'),
    OptSpec::switch("version", false, "print the version (with -v also the build target) and stop").short('V'),
];

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
    /// A list option that may be given several times: each plain value after the first adds its
    /// items instead of replacing the list ([`OptSpec::repeat`]).
    pub repeat: bool,
}

impl OptSpec {
    pub const fn switch(name: &'static str, default: bool, help: &'static str) -> Self {
        OptSpec {
            name,
            short: None,
            kind: Kind::Switch,
            value_name: "",
            default: if default { "true" } else { "false" },
            help,
            hex: false,
            repeat: false,
        }
    }
    pub const fn value(name: &'static str, value_name: &'static str, default: &'static str, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::Value, value_name, default, help, hex: false, repeat: false }
    }
    pub const fn list(name: &'static str, value_name: &'static str, default: &'static str, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::List, value_name, default, help, hex: false, repeat: false }
    }
    /// A value option whose value may be left out; given bare, its value is empty.
    pub const fn optional_value(name: &'static str, value_name: &'static str, help: &'static str) -> Self {
        OptSpec { name, short: None, kind: Kind::OptionalValue, value_name, default: "", help, hex: false, repeat: false }
    }
    pub const fn short(mut self, c: char) -> Self {
        self.short = Some(c);
        self
    }
    pub const fn hex(mut self) -> Self {
        self.hex = true;
        self
    }
    /// Marks a list option as repeatable (`--check a --check b` keeps both).
    pub const fn repeat(mut self) -> Self {
        self.repeat = true;
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

    /// The worker threads `--jobs` asks for, the subcommand's own option or the common one
    /// ([`io::jobs`]).
    #[allow(dead_code)] // The subcommands that work on inputs in parallel adopt it.
    pub fn jobs(&self) -> Result<usize, Failure> {
        io::jobs(self.value("jobs"))
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
    let globals = globals_for(specs);
    let tiers: [&[OptSpec]; 2] = [specs, &globals];
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
    // The list options given so far on the line, for repeatable lists.
    let mut given: Vec<&'static str> = Vec::new();
    // The level `-q`, `-v` and `--log` set so far, in the order given.
    let mut levels = Levels::default();
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
                let (spec, on) = resolve(&tiers, name)?;
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
            let (spec, on) = resolve(&tiers, name)?;
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
                    set_value(&mut parsed, spec, value, edit, &mut given);
                }
            }
            if is_global(&globals, spec) {
                levels.track(spec.name, &parsed)?;
            }
            continue;
        }
        if let Some(count) = verbose_cluster(&arg)
            && globals.iter().any(|g| g.short == Some('v'))
        {
            parsed.values.insert("verbose", OptValue::Switch(true));
            for _ in 0..count {
                levels.track("verbose", &parsed)?;
            }
            continue;
        }
        let mut chars = arg.chars();
        if let (Some('-'), Some(c), None) = (chars.next(), chars.next(), chars.next()) {
            let spec = specs.iter().chain(&globals).find(|s| s.short == Some(c)).ok_or_else(|| usage(format!("unknown option -{c}")))?;
            match spec.kind {
                Kind::Switch => {
                    parsed.values.insert(spec.name, OptValue::Switch(true));
                }
                Kind::OptionalValue => {
                    let value = optional_value(&mut it);
                    set_value(&mut parsed, spec, value, Edit::Set, &mut given);
                }
                Kind::Value | Kind::List => {
                    let value = it.next().ok_or_else(|| usage(format!("-{c} needs a value")))?;
                    set_value(&mut parsed, spec, value, Edit::Set, &mut given);
                }
            }
            if is_global(&globals, spec) {
                levels.track(spec.name, &parsed)?;
            }
            continue;
        }
        parsed.positional.push(arg);
    }
    apply_globals(&parsed, &globals, levels)?;
    Ok(parsed)
}

/// The common options `own` leaves free: those whose long name it does not declare, without
/// their short letter where `own` uses it.
fn globals_for(own: &[OptSpec]) -> Vec<OptSpec> {
    GLOBAL
        .iter()
        .filter(|g| !own.iter().any(|o| o.name == g.name))
        .map(|g| {
            let mut g = *g;
            if g.short.is_some_and(|c| own.iter().any(|o| o.short == Some(c))) {
                g.short = None;
            }
            g
        })
        .collect()
}

/// Whether `spec` is one of the common options (a subcommand's own option of the same name
/// replaces it, so the name decides).
fn is_global(globals: &[OptSpec], spec: &OptSpec) -> bool {
    globals.iter().any(|g| g.name == spec.name)
}

/// How many `v`s a `-vv` (`-vvv`…) argument holds; `None` for anything else, `-v` included.
fn verbose_cluster(arg: &str) -> Option<usize> {
    let letters = arg.strip_prefix('-')?;
    (letters.len() > 1 && letters.chars().all(|c| c == 'v')).then_some(letters.len())
}

/// The level `-q`, `-v` and `--log` set, read in the order they appear.
#[derive(Default)]
struct Levels {
    level: Option<Level>,
    modules: Vec<(String, Level)>,
}

impl Levels {
    /// Takes in the common option `name` just set in `parsed`.
    fn track(&mut self, name: &str, parsed: &Parsed) -> Result<(), Failure> {
        match name {
            "quiet" => self.level = Some(if parsed.switch("quiet") { Level::Error } else { Level::Note }),
            "verbose" => self.level = Some(if parsed.switch("verbose") { self.level.unwrap_or(Level::Note).louder() } else { Level::Note }),
            "log" => {
                let text = parsed.value("log").unwrap_or_default();
                let mut modules = Vec::new();
                for item in text.split(',').map(str::trim).filter(|i| !i.is_empty()) {
                    let bad = || usage(format!("--log: `{item}` is not a level (error, warning, note, info, debug) or MODULE=LEVEL"));
                    match item.split_once('=') {
                        Some((module, level)) if !module.trim().is_empty() => {
                            modules.push((module.trim().to_string(), Level::parse(level).ok_or_else(bad)?))
                        }
                        Some(_) => return Err(bad()),
                        None => self.level = Some(Level::parse(item).ok_or_else(bad)?),
                    }
                }
                self.modules = modules;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Applies the common options given on the line. `--version` prints the version and stops the
/// run with [`Failure::Done`].
fn apply_globals(parsed: &Parsed, globals: &[OptSpec], levels: Levels) -> Result<(), Failure> {
    let global = |name: &str| globals.iter().any(|g| g.name == name);
    let value = |name: &str| if global(name) { parsed.value(name) } else { None };
    let switch = |name: &str| global(name) && parsed.switch(name);
    let color = match value("color") {
        Some(v) => Some(Color::parse(v).ok_or_else(|| usage(format!("--color takes auto, always or never, not `{v}`")))?),
        None => None,
    };
    if let Some(jobs) = value("jobs") {
        io::jobs(Some(jobs))?;
    }
    let setup = Setup {
        level: levels.level,
        modules: levels.modules,
        log_file: value("log-file").map(str::to_string),
        color,
        timing: switch("timing"),
        backtrace: switch("backtrace"),
    };
    if setup != Setup::default() {
        io::configure(setup)?;
    }
    if switch("version") {
        out(&version_text(io::level() >= Level::Info))?;
        return Err(Failure::Done);
    }
    Ok(())
}

/// `fontcraft-cli VERSION`; `detailed` adds the build target and profile.
fn version_text(detailed: bool) -> String {
    let mut text = format!("fontcraft-cli {}", env!("CARGO_PKG_VERSION"));
    if detailed {
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        text.push_str(&format!("\ntarget: {}-{}\nbuild: {profile}", std::env::consts::ARCH, std::env::consts::OS));
    }
    text
}

/// For a subcommand that reads its own arguments: applies the common options in `args` whose
/// spelling `own` does not claim (`["--face", "-o"]`), as [`parse`] does, and returns the other
/// arguments in order. Nothing after `--` is read.
pub fn take_globals(args: &[String], own: &[&str]) -> Result<Vec<String>, Failure> {
    let mut globals = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--" {
            rest.push(arg.clone());
            rest.extend(it.by_ref().cloned());
            break;
        }
        match global_arg(arg, own) {
            Some(takes_next) => {
                globals.push(arg.clone());
                if takes_next && let Some(value) = it.next() {
                    globals.push(value.clone());
                }
            }
            None => rest.push(arg.clone()),
        }
    }
    if !globals.is_empty() {
        parse(&[], &globals)?;
    }
    Ok(rest)
}

/// How many arguments the common options at the start of a command line take
/// (`fontcraft-cli -q --log-file x convert …` → 3).
pub fn leading_globals(args: &[String]) -> usize {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match global_arg(arg, &[]) {
            Some(true) => i += 2,
            Some(false) => i += 1,
            None => break,
        }
    }
    i.min(args.len())
}

/// Whether `arg` is a common option that `own` does not claim, and if so whether its value is
/// the next argument. A long name is read as [`parse`] reads it, `own`'s long spellings tried
/// before the common options at each step: exact, letter case, prefix. A prefix of one of `own`'s
/// spellings, or of several common options, is left to the subcommand.
fn global_arg(arg: &str, own: &[&str]) -> Option<bool> {
    if let Some(body) = arg.strip_prefix("--") {
        let (name, inline) = match body.split_once('=') {
            Some((n, _)) => (n, true),
            None => (body, false),
        };
        if name.is_empty() {
            return None;
        }
        let given = format!("--{name}");
        let own_long: Vec<&str> = own.iter().copied().filter(|o| o.starts_with("--")).collect();
        let common: Vec<(String, &OptSpec, bool)> = forms(GLOBAL).map(|(n, s, on)| (format!("--{n}"), s, on)).collect();
        let steps: [fn(&str, &str) -> bool; 3] =
            [|c: &str, g: &str| c == g, |c: &str, g: &str| c.eq_ignore_ascii_case(g), |c: &str, g: &str| c.starts_with(g)];
        for step in steps {
            if own_long.iter().any(|o| step(o, &given)) {
                return None;
            }
            let hits: Vec<&(String, &OptSpec, bool)> = common.iter().filter(|(form, _, _)| step(form, &given)).collect();
            match hits.as_slice() {
                [] => continue,
                [(_, spec, on)] => return Some(spec.kind != Kind::Switch && !inline && *on),
                _ => return None,
            }
        }
        return None;
    }
    let claimed = |form: String| own.contains(&form.as_str());
    let c = if verbose_cluster(arg).is_some() {
        'v'
    } else {
        let mut chars = arg.chars();
        match (chars.next(), chars.next(), chars.next()) {
            (Some('-'), Some(c), None) => c,
            _ => return None,
        }
    };
    let spec = GLOBAL.iter().find(|g| g.short == Some(c))?;
    if claimed(format!("-{c}")) {
        return None;
    }
    Some(spec.kind != Kind::Switch)
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

/// Sets option `spec` to `value`. `given` lists the list options already set on this line.
fn set_value(parsed: &mut Parsed, spec: &OptSpec, value: String, edit: Edit, given: &mut Vec<&'static str>) {
    match spec.kind {
        Kind::Value | Kind::OptionalValue => {
            parsed.values.insert(spec.name, OptValue::Value(value));
        }
        Kind::List => {
            let items = split_list(&value);
            let mut current = parsed.list(spec.name).to_vec();
            let edits = !items.is_empty() && items.iter().all(|i| i.len() > 1 && (i.starts_with('+') || i.starts_with('-')));
            let again = spec.repeat && given.contains(&spec.name) && !items.is_empty();
            if !given.contains(&spec.name) {
                given.push(spec.name);
            }
            let edit = if again && edit == Edit::Set && !edits { Edit::Add } else { edit };
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

/// The option `name` names, and whether it is the `no-` form. Each step looks through the
/// tiers in order (the subcommand's own options, then the common ones): the exact name, then the
/// name in another letter case, then a prefix that names one option only.
fn resolve<'a>(tiers: &[&'a [OptSpec]], name: &str) -> Result<(&'a OptSpec, bool), Failure> {
    lookup(tiers, name)?.ok_or_else(|| usage(format!("unknown option --{name}")))
}

/// As [`resolve`], with `None` for a name that is no option and begins none.
fn lookup<'a>(tiers: &[&'a [OptSpec]], name: &str) -> Result<Option<(&'a OptSpec, bool)>, Failure> {
    for &specs in tiers {
        if let Some((_, s, on)) = forms(specs).find(|(n, _, _)| n == name) {
            return Ok(Some((s, on)));
        }
    }
    for &specs in tiers {
        if let Some((_, s, on)) = forms(specs).find(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
            return Ok(Some((s, on)));
        }
    }
    for &specs in tiers {
        let matches: Vec<(String, &OptSpec, bool)> = forms(specs).filter(|(n, _, _)| n.starts_with(name)).collect();
        match matches.as_slice() {
            [(_, s, on)] => return Ok(Some((s, *on))),
            [] => continue,
            many => {
                let names: Vec<String> = many.iter().map(|(n, _, _)| format!("--{n}")).collect();
                return Err(usage(format!("--{name} is ambiguous: {}", names.join(", "))));
            }
        }
    }
    Ok(None)
}

/// Whether the argument after an option is its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    No,
    Always,
    /// When it does not start with `-` and holds no `=` ([`optional_value`]).
    Maybe,
}

/// `args` with response files read and every long option written out in full as [`parse`]
/// reads it (`--retain` → `--retain-gids`, `--No-Hinting` → `--no-hinting`), for a subcommand that
/// looks for some options itself before calling [`parse`]. Values, `--opt?` queries and
/// everything after `--` stay as given; a name that is no option stays for [`parse`] to refuse,
/// and a prefix of several options is refused here. A response file is read only where an
/// option or a positional argument may stand: an option's value that starts with `@`
/// (`dump -t @glyphs`) is a value.
pub fn spell_out(specs: &[OptSpec], args: &[String]) -> Result<Vec<String>, Failure> {
    let globals = globals_for(specs);
    let tiers: [&[OptSpec]; 2] = [specs, &globals];
    let mut spelled = Vec::with_capacity(args.len());
    spell_into(&tiers, args, &mut spelled)?;
    Ok(spelled)
}

/// [`spell_out`] of `args` into `out`; `true` once `--` has ended the options.
fn spell_into(tiers: &[&[OptSpec]], args: &[String], out: &mut Vec<String>) -> Result<bool, Failure> {
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        if arg == "--" {
            out.push(arg.clone());
            out.extend(it.by_ref().cloned());
            return Ok(true);
        }
        let response = if arg.strip_prefix('@').is_some_and(|p| !p.is_empty()) || arg.starts_with("--options=") {
            Some(vec![arg.clone()])
        } else if arg == "--options" {
            Some(std::iter::once(arg.clone()).chain(it.next().cloned()).collect())
        } else {
            None
        };
        if let Some(response) = response {
            // `expand` reads nested files and bounds their depth.
            if spell_into(tiers, &expand(&response, 0, &mut Vec::new())?, out)? {
                out.extend(it.by_ref().cloned());
                return Ok(true);
            }
            continue;
        }
        let (written, next) = spell_one(tiers, arg)?;
        out.push(written.unwrap_or_else(|| arg.clone()));
        match next {
            Next::No => {}
            Next::Always => out.extend(it.next().cloned()),
            Next::Maybe => out.extend(it.next_if(|n| !n.starts_with('-') && !n.contains('=')).cloned()),
        }
    }
    Ok(false)
}

/// How [`spell_out`] writes `arg`: its full spelling (`None`: as given), and whether the next
/// argument is its value.
fn spell_one(tiers: &[&[OptSpec]], arg: &str) -> Result<(Option<String>, Next), Failure> {
    let takes = |spec: &OptSpec| match spec.kind {
        Kind::Switch => Next::No,
        Kind::OptionalValue => Next::Maybe,
        Kind::Value | Kind::List => Next::Always,
    };
    if let Some(body) = arg.strip_prefix("--") {
        if body.is_empty() || (!body.contains('=') && body.ends_with('?')) {
            return Ok((None, Next::No));
        }
        let (name, edit, inline) = match body.split_once('=') {
            Some((n, v)) => match (n.strip_suffix('+'), n.strip_suffix('-')) {
                (Some(n), _) => (n, "+=", Some(v)),
                (_, Some(n)) => (n, "-=", Some(v)),
                _ => (n, "=", Some(v)),
            },
            None => (body, "", None),
        };
        let Some((spec, on)) = lookup(tiers, name)? else { return Ok((None, Next::No)) };
        let full = if on { spec.name.to_string() } else { format!("no-{}", spec.name) };
        return Ok(match inline {
            Some(v) => (Some(format!("--{full}{edit}{v}")), Next::No),
            None if on => (Some(format!("--{full}")), takes(spec)),
            None => (Some(format!("--{full}")), Next::No),
        });
    }
    let mut chars = arg.chars();
    if let (Some('-'), Some(c), None) = (chars.next(), chars.next(), chars.next()) {
        let next = tiers.iter().flat_map(|t| t.iter()).find(|s| s.short == Some(c)).map_or(Next::No, takes);
        return Ok((None, next));
    }
    Ok((None, Next::No))
}

/// The candidate `given` names: the one spelled so, else the one equal to it in another letter
/// case, else the one it begins. `None` when it names none; a prefix of several candidates is a
/// usage error that lists them.
pub fn pick<'a>(candidates: &[&'a str], given: &str) -> Result<Option<&'a str>, Failure> {
    if given.is_empty() {
        return Ok(None);
    }
    if let Some(c) = candidates.iter().find(|c| **c == given) {
        return Ok(Some(*c));
    }
    if let Some(c) = candidates.iter().find(|c| c.eq_ignore_ascii_case(given)) {
        return Ok(Some(*c));
    }
    let mut matches: Vec<&'a str> = candidates.iter().copied().filter(|c| c.starts_with(given)).collect();
    matches.sort_unstable();
    matches.dedup();
    match matches.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        many => Err(usage(format!("`{given}` is ambiguous: {}", many.join(", ")))),
    }
}

/// `arg` with its long option name written out in full among `spellings` (`--out=x` →
/// `--output=x` when `--output` is the one spelling it begins), for a subcommand that reads its
/// own arguments. Only the `--` spellings take part. Anything else, and a name no spelling
/// matches, comes back as given; a prefix of several spellings is a usage error.
pub fn spell_long(spellings: &[&str], arg: &str) -> Result<String, Failure> {
    let Some(body) = arg.strip_prefix("--") else { return Ok(arg.to_string()) };
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    };
    if name.is_empty() {
        return Ok(arg.to_string());
    }
    let long: Vec<&str> = spellings.iter().copied().filter(|s| s.starts_with("--")).collect();
    Ok(match (pick(&long, &format!("--{name}"))?, value) {
        (Some(full), Some(v)) => format!("{full}={v}"),
        (Some(full), None) => full.to_string(),
        (None, _) => arg.to_string(),
    })
}

/// The spellings of each option: its name, and `no-NAME` for a switch.
fn forms(specs: &[OptSpec]) -> impl Iterator<Item = (String, &OptSpec, bool)> {
    specs.iter().flat_map(|s| {
        let negative = (s.kind == Kind::Switch).then(|| (format!("no-{}", s.name), s, false));
        std::iter::once((s.name.to_string(), s, true)).chain(negative)
    })
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

/// A text file of at most [`MAX_RESPONSE_FILE`] bytes of UTF-8.
pub(crate) fn read_small(path: &str) -> Result<String, Failure> {
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
pub(crate) fn split_words(text: &str) -> Vec<String> {
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
        text.push_str(&option_lines(specs));
    }
    text
}

/// One line per option: its forms, its help and its default.
pub fn option_lines(specs: &[OptSpec]) -> String {
    let mut text = String::new();
    for s in specs {
        let short = s.short.map(|c| format!("-{c}, ")).unwrap_or_default();
        let form = match s.kind {
            // A switch named `no-…` is already the negative form.
            Kind::Switch if s.name.starts_with("no-") => format!("{short}--{}", s.name),
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
    fn a_repeatable_list_keeps_every_value() {
        const SPECS: &[OptSpec] = &[OptSpec::list("check", "ID", "", "checks").repeat(), OptSpec::list("drop", "TAG", "", "tables")];
        let p = parse(SPECS, &args(&["--check", "a", "--check=b,c", "--check", "a", "--drop", "x", "--drop", "y"])).unwrap();
        assert_eq!(p.list("check"), ["a", "b", "c"], "each value adds its items once");
        assert_eq!(p.list("drop"), ["y"], "a list that is not repeatable is replaced");
        let p = parse(SPECS, &args(&["--check", "a", "--check=", "--check", "b"])).unwrap();
        assert_eq!(p.list("check"), ["b"], "an empty value still empties the list");
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
    fn common_options_join_every_table_and_own_options_win() {
        const OWN: &[OptSpec] = &[
            OptSpec::switch("version", true, "keep the version fields"),
            OptSpec::value("timestamp", "N", "", "time stamp").short('j'),
            OptSpec::value("log-output", "PATH", "", "log"),
        ];
        let p = parse(OWN, &args(&["--no-version", "--jobs", "-1", "--tim", "5", "--log-o", "x"])).unwrap();
        assert!(!p.switch("version"), "the subcommand's own --version wins");
        assert_eq!(p.value("jobs"), Some("-1"), "the common --jobs is read, with a negative value");
        assert_eq!(p.value("timestamp"), Some("5"), "a prefix names the subcommand's own option before a common one (--timing)");
        assert_eq!(p.value("log-output"), Some("x"));
        let p = parse(OWN, &args(&["-j", "7"])).unwrap();
        assert_eq!(p.value("timestamp"), Some("7"), "the subcommand keeps its short letter");
        assert!(parse(OWN, &args(&["--jobs", "many"])).is_err(), "--jobs needs a number");
        assert!(parse(OWN, &args(&["--color", "sometimes"])).is_err());
        assert!(parse(OWN, &args(&["--log", "loud"])).is_err());
        assert!(parse(OWN, &args(&["--log", "=debug"])).is_err(), "a module needs a name");
        assert!(parse(OWN, &args(&["-vv", "--log", "loud"])).is_err());
    }

    #[test]
    fn log_levels_read_a_level_and_module_levels() {
        let mut parsed = Parsed::default();
        parsed.values.insert("log", OptValue::Value("info, engine=debug,compile=warn".into()));
        let mut levels = Levels::default();
        levels.track("log", &parsed).unwrap();
        assert_eq!(levels.level, Some(Level::Info));
        assert_eq!(levels.modules, [("engine".to_string(), Level::Debug), ("compile".to_string(), Level::Warning)]);
        parsed.values.insert("verbose", OptValue::Switch(true));
        levels.track("verbose", &parsed).unwrap();
        assert_eq!(levels.level, Some(Level::Debug), "-v after --log info raises it one step");
        parsed.values.insert("quiet", OptValue::Switch(true));
        levels.track("quiet", &parsed).unwrap();
        assert_eq!(levels.level, Some(Level::Error), "a later -q wins");
    }

    #[test]
    fn hand_parsers_and_the_top_level_take_the_common_options_out() {
        let rest = take_globals(&args(&["in.ttf", "--jobs", "3", "--face", "2", "-j", "1", "--", "--jobs"]), &["--face", "-j"]).unwrap();
        assert_eq!(rest, ["in.ttf", "--face", "2", "-j", "1", "--", "--jobs"], "a claimed spelling and everything after -- stay");
        assert_eq!(leading_globals(&args(&["--jobs", "2", "-vv", "--log-file=x", "convert", "-q"])), 4);
        assert_eq!(leading_globals(&args(&["convert", "--jobs", "2"])), 0);
        assert_eq!(leading_globals(&args(&["--jobs"])), 1, "a missing value is left to the parser");
    }

    #[test]
    fn hand_parsers_leave_prefixes_of_their_own_options_to_them() {
        let rest = take_globals(&args(&["--tim", "5", "--job", "3", "--jo", "x"]), &["--timestamp", "--joined"]).unwrap();
        assert_eq!(rest, ["--tim", "5", "--jo", "x"], "--tim and --jo begin options of the subcommand; --job names the common --jobs");
        assert_eq!(leading_globals(&args(&["--job", "2", "--Quiet", "convert"])), 3, "prefixes and letter case, as parse reads them");
        assert_eq!(leading_globals(&args(&["--ver", "convert"])), 0, "--ver begins --verbose and --version");
    }

    #[test]
    fn spell_out_writes_names_in_full_and_leaves_values_alone() {
        let given = args(&[
            "in.ttf",
            "--retain",
            "--No-Hinting",
            "--layout-f+=smcp",
            "--output-d",
            "--retain",
            "-o",
            "--hinting",
            "--tim",
            "--bogus",
            "--",
            "--retain",
        ]);
        let spelled = spell_out(SPECS, &given).unwrap();
        assert_eq!(
            spelled,
            [
                "in.ttf",
                "--retain-gids",
                "--no-hinting",
                "--layout-features+=smcp",
                "--output-dir",
                "--retain",
                "-o",
                "--hinting",
                "--timing",
                "--bogus",
                "--",
                "--retain"
            ]
        );
        let parsed = parse(SPECS, &spell_out(SPECS, &args(&["--output-d", "--retain", "-o", "--hinting"])).unwrap()).unwrap();
        assert_eq!((parsed.value("output-dir"), parsed.value("output")), (Some("--retain"), Some("--hinting")), "values are never read as options");
        assert!(spell_out(SPECS, &args(&["--out", "x"])).is_err(), "a prefix of several options is refused");

        // A value starting with `@` is a value; a response file elsewhere is read and spelled out.
        assert_eq!(spell_out(SPECS, &args(&["-o", "@glyphs"])).unwrap(), ["-o", "@glyphs"]);
        let dir = std::env::temp_dir().join(format!("fontcraft-cli-spell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("words");
        std::fs::write(&file, "--retain --output-d out").unwrap();
        let at = format!("@{}", file.display());
        assert_eq!(spell_out(SPECS, &args(&["in.ttf", &at, "-o", "x"])).unwrap(), ["in.ttf", "--retain-gids", "--output-dir", "out", "-o", "x"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spell_long_resolves_the_spellings_of_a_hand_parser() {
        const OWN: &[&str] = &["--output", "--output-dir", "--json", "-o"];
        assert_eq!(spell_long(OWN, "--js").unwrap(), "--json");
        assert_eq!(spell_long(OWN, "--output-d=x").unwrap(), "--output-dir=x");
        assert_eq!(spell_long(OWN, "--OUTPUT").unwrap(), "--output");
        assert_eq!(spell_long(OWN, "--output").unwrap(), "--output", "an exact name wins over the longer ones it begins");
        for unchanged in ["--nope", "file.ttf", "-o", "--"] {
            assert_eq!(spell_long(OWN, unchanged).unwrap(), unchanged);
        }
        let e = spell_long(OWN, "--out").unwrap_err();
        assert!(e.message().contains("--output, --output-dir"), "{e:?}");
    }

    #[test]
    fn jobs_counts_from_the_cores() {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        assert_eq!(io::jobs(Some("3")).unwrap(), 3);
        assert_eq!(io::jobs(None).unwrap(), cores);
        assert_eq!(io::jobs(Some("0")).unwrap(), cores);
        assert_eq!(io::jobs(Some("-1")).unwrap(), cores.saturating_sub(1).max(1));
        assert_eq!(io::jobs(Some("-100000")).unwrap(), 1, "at least one");
        assert_eq!(io::jobs(Some("100000")).unwrap(), io::MAX_JOBS);
        assert!(io::jobs(Some("2.5")).is_err());
    }

    #[test]
    fn help_lists_every_option() {
        let text = help("fontcraft-cli subset FONT", SPECS);
        assert!(text.contains("--[no-]hinting") && text.contains("-o, --output FILE") && text.contains("(default: kern,liga)"));
    }
}
