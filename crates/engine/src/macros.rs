//! Macros: `.fcmacro` files of command steps, macro recording, and reading the Macros folder.
//! Plan: `docs/plans/M13.md` § 3 (decision D8, running and recording), § 4 › M13.7.
//!
//! Format (JSON, at most [`MAX_MACRO_BYTES`]):
//! `{"format": "fontcraft.macro", "version": 1, "title", "description", "args": {name: spec},
//! "requires": [module ids], "steps": [{"command", "params"}]}`. A value `{"$arg": "name"}`
//! anywhere in a step's params is replaced by the argument; `glyphs` and `layers` are built-in
//! arguments the shell fills from the selection. No conditionals or loops. A reader refuses a
//! newer `version`.
//!
//! Running (`script.run`, in `cmd/script.rs`): [`Macro::resolve`] resolves every step's command
//! id (renamed ids through their aliases, decision 0004 item 8) and every placeholder first, so
//! an unknown command or argument changes nothing; then the steps run as one transaction and one
//! undo step, like `command.batch`, and the first failing step is named. A macro may run another
//! macro up to [`MAX_MACRO_NESTING`] deep; one that reaches itself is refused. One top-level run
//! executes at most [`MAX_MACRO_RUN_STEPS`] steps and fills in at most [`MAX_RESOLVED_VALUES`]
//! values, nested macros included, and reads each macro file once. Steps are counted from 1 in
//! messages.
//!
//! Recording (`script.recordMacro`): while a [`Recording`] is set in the session's
//! `ExtensionState`, the hook in `Session::execute` hands it every successful font-changing
//! command (queries, shell commands, undo and redo are excluded).
//!
//! The Macros folder: the desktop app calls [`poll`] when it gains focus and every
//! [`FOLDER_POLL_MS`](crate::extensions::FOLDER_POLL_MS); it compares the folder's
//! [`stamp_folder`] with the last one and re-reads the files only when they differ.
//! `script.reloadScripts` re-reads them unconditionally.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::EngineError;
use crate::extensions::{ExtensionState, FileStamp, FolderStamps, MAX_FOLDER_FILES, MAX_MACRO_FOLDER_DEPTH};

/// The `format` value of a macro file.
pub const MACRO_FORMAT: &str = "fontcraft.macro";
/// The newest macro format version this build reads and the one it writes.
pub const MACRO_FORMAT_VERSION: u32 = 1;
/// The file name extension of macros (without the dot).
pub const MACRO_EXTENSION: &str = "fcmacro";
/// Largest macro file, in bytes.
pub const MAX_MACRO_BYTES: usize = 4 << 20;
/// Most steps in one macro (the same limit as `command.batch`).
pub const MAX_MACRO_STEPS: usize = 100_000;
/// How deeply macros may run macros.
pub const MAX_MACRO_NESTING: usize = 8;
/// Most declared arguments.
pub const MAX_MACRO_ARGS: usize = 64;
/// How deeply placeholders are searched for in one step's params; deeper params are refused.
pub const MAX_PLACEHOLDER_DEPTH: usize = 32;
/// Most steps one recording keeps; recording then stops with a message and keeps what it has.
pub const MAX_RECORDED_STEPS: usize = 100_000;
/// Most bytes of step parameters (as JSON) one recording keeps; past it the recording is full.
pub const MAX_RECORDED_BYTES: usize = 64 << 20;
/// Most steps one top-level macro run executes, the steps of every macro it runs included. Without
/// it, macros that run macros multiply their step counts level by level.
pub const MAX_MACRO_RUN_STEPS: usize = 1_000_000;
/// Most bytes of macro files one top-level macro run reads, nested macros included.
pub const MAX_MACRO_RUN_BYTES: usize = 256 << 20;
/// The arguments the shell fills from its selection without a declaration.
pub const BUILT_IN_ARGS: &[&str] = &["glyphs", "layers"];
/// Most module ids in `requires` (as many as can be installed).
pub const MAX_MACRO_REQUIRES: usize = 256;
/// Longest string argument, macro name or path, in bytes.
pub const MAX_MACRO_TEXT: usize = 4096;
/// Most JSON values the resolved steps of one run may hold, placeholders filled in. A macro that
/// pastes a long glyph list into many steps is refused instead of filling memory.
pub const MAX_RESOLVED_VALUES: usize = 20_000_000;
/// Most bytes read from the Macros folder in one listing; files past it are listed with an error.
pub const MAX_MACRO_FOLDER_BYTES: u64 = 256 << 20;
/// Most directory entries looked at in one listing of the Macros folder, other files included.
pub const MAX_FOLDER_ENTRIES: usize = 100_000;

/// Renamed command ids, `(old, new)`: a macro naming the old id runs the new one (decision 0004
/// item 8).
pub const COMMAND_ALIASES: &[(&str, &str)] = &[("element.copyTransformation", "component.copyTransformation"), ("collab.showServerInfo", "app.info")];
/// Renamed parameters, `(command, old name, new name)`; a step's params are renamed before it runs.
pub const PARAM_ALIASES: &[(&str, &str, &str)] = &[];

/// Commands a macro cannot run as a step: undo and redo act on history outside the macro's own
/// undo step, a recording is not part of an edit, and re-running the last macro would make the
/// macro depend on what ran before it.
pub const NOT_STEPS: &[&str] = &["edit.undo", "edit.redo", "script.recordMacro", "script.rerunLast", "script.repeatLastAction"];

/// Commands a recording leaves out: they change the history, the recording, the macro list or
/// the folder, not the font.
const NOT_RECORDED: &[&str] =
    &["edit.undo", "edit.redo", "script.recordMacro", "script.saveActionSet", "script.openScriptsFolder", "script.reloadScripts"];

/// Why a macro cannot be read or run.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MacroError {
    #[error("the macro is {0} bytes; at most {MAX_MACRO_BYTES}")]
    TooLarge(usize),
    #[error("the macro is not valid JSON: {0}")]
    Json(String),
    #[error("not a macro: {0}")]
    Format(String),
    #[error("the macro is format version {0}; this version of the app reads up to {MACRO_FORMAT_VERSION}")]
    NewerVersion(u64),
    #[error("the macro has too many {0}")]
    Limit(String),
    #[error("step {step}: unknown command `{command}`")]
    UnknownCommand { step: usize, command: String },
    #[error("step {step}: unknown argument `{arg}`")]
    UnknownArg { step: usize, arg: String },
    #[error("argument `{0}`: {1}")]
    BadArg(String, String),
    #[error("macros nested more than {MAX_MACRO_NESTING} deep")]
    Nesting,
    #[error("the macro `{0}` runs itself")]
    SelfReference(String),
    #[error("step {step}: `{command}` cannot be a macro step")]
    NotAStep { step: usize, command: String },
    #[error("the macro needs the module `{0}`, which is not installed")]
    MissingModule(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

impl From<MacroError> for EngineError {
    fn from(e: MacroError) -> Self {
        match e {
            MacroError::Unsupported(what) => EngineError::Unsupported(what),
            other => EngineError::BadParams { command: "script.run".into(), message: other.to_string() },
        }
    }
}

/// One step: a command id and its parameters (placeholders still in them until resolved).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub command: String,
    #[serde(default)]
    pub params: Value,
}

/// A macro as read from a `.fcmacro` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macro {
    /// The Script menu label; empty: the file name.
    pub title: String,
    /// The Script menu tooltip (`script.menu.tooltip`).
    pub description: String,
    /// Declared arguments: name → parameter spec in the sibling apps' schema
    /// (`{"type": "number"|"int"|"bool"|"choice"|"string", "default"?, …}`).
    pub args: Map<String, Value>,
    /// Module ids the macro needs (`script.requiresModules`); running without one installed
    /// fails with a message naming it.
    pub requires: Vec<String>,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
struct MacroFile {
    format: String,
    version: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    args: Map<String, Value>,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

impl Macro {
    /// Reads a macro file: size, JSON, `format`, `version` (newer refused) and the step and
    /// argument limits.
    pub fn parse(bytes: &[u8]) -> Result<Macro, MacroError> {
        if bytes.len() > MAX_MACRO_BYTES {
            return Err(MacroError::TooLarge(bytes.len()));
        }
        let file: MacroFile = serde_json::from_slice(bytes).map_err(|e| MacroError::Json(e.to_string()))?;
        if file.format != MACRO_FORMAT {
            return Err(MacroError::Format(format!("`format` is {:?}, not {MACRO_FORMAT:?}", file.format)));
        }
        if file.version > u64::from(MACRO_FORMAT_VERSION) {
            return Err(MacroError::NewerVersion(file.version));
        }
        if file.steps.len() > MAX_MACRO_STEPS {
            return Err(MacroError::Limit(format!("steps ({}; at most {MAX_MACRO_STEPS})", file.steps.len())));
        }
        if file.args.len() > MAX_MACRO_ARGS {
            return Err(MacroError::Limit(format!("arguments ({}; at most {MAX_MACRO_ARGS})", file.args.len())));
        }
        if file.requires.len() > MAX_MACRO_REQUIRES {
            return Err(MacroError::Limit(format!("required modules ({}; at most {MAX_MACRO_REQUIRES})", file.requires.len())));
        }
        Ok(Macro { title: file.title, description: file.description, args: file.args, requires: file.requires, steps: file.steps })
    }

    /// A macro given as a JSON object, as `script.run` and `script.saveActionSet` take one inline:
    /// the file form, where `format` and `version` may be left out.
    pub fn from_value(value: &Value) -> Result<Macro, MacroError> {
        let Value::Object(map) = value else {
            return Err(MacroError::Format("a macro is a JSON object".into()));
        };
        let mut map = map.clone();
        map.entry("format").or_insert_with(|| json!(MACRO_FORMAT));
        map.entry("version").or_insert_with(|| json!(MACRO_FORMAT_VERSION));
        let bytes = serde_json::to_vec(&Value::Object(map)).map_err(|e| MacroError::Json(e.to_string()))?;
        Macro::parse(&bytes)
    }

    /// The macro as file JSON, with the current format and version.
    pub fn to_json(&self) -> Value {
        json!({
            "format": MACRO_FORMAT,
            "version": MACRO_FORMAT_VERSION,
            "title": self.title,
            "description": self.description,
            "args": self.args,
            "requires": self.requires,
            "steps": self.steps,
        })
    }

    /// The steps ready to run: every command id resolved (aliases included) and every
    /// `{"$arg": name}` replaced from `args` (an object; declared defaults fill what is missing,
    /// `glyphs` and `layers` always allowed). An unknown command or argument is refused before
    /// anything runs.
    pub fn resolve(&self, args: &Value) -> Result<Vec<Step>, MacroError> {
        let mut left = MAX_RESOLVED_VALUES;
        self.resolve_with(args, COMMAND_ALIASES, PARAM_ALIASES, &mut left)
    }

    /// [`Macro::resolve`] with the values still allowed in `left`, which a run shares with the
    /// macros it runs; what the steps hold is taken from it.
    pub fn resolve_within(&self, args: &Value, left: &mut usize) -> Result<Vec<Step>, MacroError> {
        self.resolve_with(args, COMMAND_ALIASES, PARAM_ALIASES, left)
    }

    /// Checks what can be checked without arguments: every command id resolves and may be a
    /// step, and every placeholder names a declared or built-in argument.
    pub fn check(&self) -> Result<(), MacroError> {
        self.check_limits()?;
        for (i, step) in self.steps.iter().enumerate() {
            let n = i + 1;
            step_command(&step.command, n, COMMAND_ALIASES)?;
            check_placeholders(&step.params, &self.args, n, 0)?;
        }
        Ok(())
    }

    fn check_limits(&self) -> Result<(), MacroError> {
        if self.steps.len() > MAX_MACRO_STEPS {
            return Err(MacroError::Limit(format!("steps ({}; at most {MAX_MACRO_STEPS})", self.steps.len())));
        }
        if self.args.len() > MAX_MACRO_ARGS {
            return Err(MacroError::Limit(format!("arguments ({}; at most {MAX_MACRO_ARGS})", self.args.len())));
        }
        Ok(())
    }

    fn resolve_with(
        &self,
        args: &Value,
        commands: &[(&str, &str)],
        renames: &[(&str, &str, &str)],
        left: &mut usize,
    ) -> Result<Vec<Step>, MacroError> {
        self.check_limits()?;
        let values = self.arg_values(args)?;
        let sizes: HashMap<String, usize> = values.iter().map(|(k, v)| (k.clone(), count_values(v, MAX_RESOLVED_VALUES))).collect();
        let mut budget = Budget { left: *left, sizes: &sizes };
        let mut out = Vec::with_capacity(self.steps.len());
        for (i, step) in self.steps.iter().enumerate() {
            let n = i + 1;
            let command = step_command(&step.command, n, commands)?;
            let mut params = match &step.params {
                Value::Null => Value::Object(Map::new()),
                p => fill(p, &values, &self.args, n, 0, &mut budget)?,
            };
            if let Value::Object(map) = &mut params {
                for (cmd, old, new) in renames {
                    if *cmd == command
                        && !map.contains_key(*new)
                        && let Some(v) = map.remove(*old)
                    {
                        map.insert((*new).to_string(), v);
                    }
                }
            }
            out.push(Step { command: command.to_string(), params });
        }
        *left = budget.left;
        Ok(out)
    }

    /// The argument values of one run: the given ones checked against their declarations, then
    /// the declared defaults for the rest.
    fn arg_values(&self, args: &Value) -> Result<Map<String, Value>, MacroError> {
        let given = match args {
            Value::Null => None,
            Value::Object(map) => Some(map),
            _ => return Err(MacroError::BadArg("args".into(), "the arguments are a JSON object".into())),
        };
        let mut values = Map::new();
        for (name, value) in given.into_iter().flatten() {
            if value.is_null() {
                continue;
            }
            if BUILT_IN_ARGS.contains(&name.as_str()) {
                values.insert(name.clone(), value.clone());
                continue;
            }
            let Some(spec) = self.args.get(name) else {
                return Err(MacroError::BadArg(name.clone(), "the macro declares no such argument".into()));
            };
            check_arg(name, spec, value)?;
            values.insert(name.clone(), value.clone());
        }
        for (name, spec) in &self.args {
            if values.contains_key(name) {
                continue;
            }
            if let Some(default) = spec.get("default").filter(|v| !v.is_null()) {
                check_arg(name, spec, default)?;
                values.insert(name.clone(), default.clone());
            }
        }
        Ok(values)
    }
}

/// The registered id a step's command names, through [`COMMAND_ALIASES`] when it was renamed.
pub fn resolve_command(id: &str) -> Option<&'static str> {
    resolve_command_with(id, COMMAND_ALIASES)
}

fn resolve_command_with(id: &str, aliases: &[(&str, &str)]) -> Option<&'static str> {
    if let Some(spec) = crate::cmd::registry::find(id) {
        return Some(spec.id);
    }
    aliases.iter().find(|(old, _)| *old == id).and_then(|(_, new)| crate::cmd::registry::find(new)).map(|spec| spec.id)
}

/// Step `n`'s command, resolved and allowed as a step.
fn step_command(id: &str, n: usize, aliases: &[(&str, &str)]) -> Result<&'static str, MacroError> {
    let command = resolve_command_with(id, aliases).ok_or_else(|| MacroError::UnknownCommand { step: n, command: id.to_string() })?;
    if NOT_STEPS.contains(&command) {
        return Err(MacroError::NotAStep { step: n, command: command.to_string() });
    }
    Ok(command)
}

/// Checks a given or default value against its declaration.
fn check_arg(name: &str, spec: &Value, value: &Value) -> Result<(), MacroError> {
    let bad = |why: String| Err(MacroError::BadArg(name.to_string(), why));
    let Value::Object(spec) = spec else {
        return bad("its declaration is not a JSON object".into());
    };
    let bound = |key: &str| spec.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
    let in_range = |v: f64| bound("min").is_none_or(|min| v >= min) && bound("max").is_none_or(|max| v <= max);
    match spec.get("type").and_then(Value::as_str) {
        None => Ok(()),
        Some("number") => match value.as_f64() {
            Some(v) if v.is_finite() && in_range(v) => Ok(()),
            _ => bad(format!("{value} is not a number in the declared range")),
        },
        Some("int") => match value.as_i64() {
            #[allow(clippy::cast_precision_loss)]
            Some(v) if in_range(v as f64) => Ok(()),
            _ => bad(format!("{value} is not a whole number in the declared range")),
        },
        Some("bool") if value.is_boolean() => Ok(()),
        Some("bool") => bad(format!("{value} is not true or false")),
        Some("choice") => {
            let options = spec.get("options").and_then(Value::as_array);
            if options.is_some_and(|o| o.contains(value)) { Ok(()) } else { bad(format!("{value} is not one of the declared options")) }
        }
        Some("string") => match value.as_str() {
            Some(text) if text.len() <= MAX_MACRO_TEXT => Ok(()),
            Some(_) => bad(format!("longer than {MAX_MACRO_TEXT} bytes")),
            None => bad(format!("{value} is not a string")),
        },
        Some(other) => bad(format!("unknown type `{other}`")),
    }
}

/// The JSON values in `v` (itself included), counted up to `cap`.
fn count_values(v: &Value, cap: usize) -> usize {
    let mut n = 0usize;
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        n += 1;
        if n >= cap {
            return cap;
        }
        match v {
            Value::Array(items) => stack.extend(items.iter()),
            Value::Object(map) => stack.extend(map.values()),
            _ => {}
        }
    }
    n
}

/// What filling placeholders may still produce.
struct Budget<'a> {
    left: usize,
    /// Each argument value's size in JSON values.
    sizes: &'a HashMap<String, usize>,
}

impl Budget<'_> {
    fn spend(&mut self, n: usize) -> Result<(), MacroError> {
        self.left = self
            .left
            .checked_sub(n)
            .ok_or_else(|| MacroError::Limit(format!("values once the arguments are filled in (at most {MAX_RESOLVED_VALUES})")))?;
        Ok(())
    }
}

/// The placeholder `{"$arg": name}`'s name, if `map` is one.
fn placeholder(map: &Map<String, Value>) -> Option<&Value> {
    if map.len() == 1 { map.get("$arg") } else { None }
}

fn fill(
    value: &Value,
    values: &Map<String, Value>,
    declared: &Map<String, Value>,
    step: usize,
    depth: usize,
    budget: &mut Budget,
) -> Result<Value, MacroError> {
    if depth > MAX_PLACEHOLDER_DEPTH {
        return Err(MacroError::Limit(format!("levels of nesting in step {step}'s params (at most {MAX_PLACEHOLDER_DEPTH})")));
    }
    budget.spend(1)?;
    match value {
        Value::Object(map) => {
            if let Some(name) = placeholder(map) {
                let Some(name) = name.as_str() else {
                    return Err(MacroError::UnknownArg { step, arg: name.to_string() });
                };
                if let Some(v) = values.get(name) {
                    let size = budget.sizes.get(name).copied().unwrap_or(MAX_RESOLVED_VALUES);
                    budget.spend(size)?;
                    return Ok(v.clone());
                }
                if declared.contains_key(name) || BUILT_IN_ARGS.contains(&name) {
                    return Err(MacroError::BadArg(name.to_string(), format!("step {step} uses it, but no value was given and it has no default")));
                }
                return Err(MacroError::UnknownArg { step, arg: name.to_string() });
            }
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k.clone(), fill(v, values, declared, step, depth + 1, budget)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => {
            items.iter().map(|v| fill(v, values, declared, step, depth + 1, budget)).collect::<Result<Vec<_>, _>>().map(Value::Array)
        }
        other => Ok(other.clone()),
    }
}

fn check_placeholders(value: &Value, declared: &Map<String, Value>, step: usize, depth: usize) -> Result<(), MacroError> {
    if depth > MAX_PLACEHOLDER_DEPTH {
        return Err(MacroError::Limit(format!("levels of nesting in step {step}'s params (at most {MAX_PLACEHOLDER_DEPTH})")));
    }
    match value {
        Value::Object(map) => {
            if let Some(name) = placeholder(map) {
                return match name.as_str() {
                    Some(name) if declared.contains_key(name) || BUILT_IN_ARGS.contains(&name) => Ok(()),
                    _ => Err(MacroError::UnknownArg { step, arg: name.as_str().map_or_else(|| name.to_string(), str::to_string) }),
                };
            }
            map.values().try_for_each(|v| check_placeholders(v, declared, step, depth + 1))
        }
        Value::Array(items) => items.iter().try_for_each(|v| check_placeholders(v, declared, step, depth + 1)),
        _ => Ok(()),
    }
}

/// A macro being recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recording {
    pub steps: Vec<Step>,
    /// Recording reached [`MAX_RECORDED_STEPS`] or [`MAX_RECORDED_BYTES`] and stopped taking
    /// steps.
    pub full: bool,
    /// The recorded steps' parameters, in JSON bytes.
    pub bytes: usize,
}

impl Recording {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one successful font-changing command. Commands that change only the history,
    /// the recording or the Macros folder are left out.
    pub fn record(&mut self, command: &str, params: &Value) {
        if self.full || NOT_RECORDED.contains(&command) {
            return;
        }
        let size = json_size(params).saturating_add(command.len());
        let Some(bytes) = self.bytes.checked_add(size).filter(|b| *b <= MAX_RECORDED_BYTES) else {
            self.full = true;
            return;
        };
        self.bytes = bytes;
        let params = if params.is_null() { Value::Object(Map::new()) } else { params.clone() };
        self.steps.push(Step { command: command.to_string(), params });
        if self.steps.len() >= MAX_RECORDED_STEPS {
            self.full = true;
        }
    }

    /// The recorded steps as a macro titled `title`.
    pub fn finish(self, title: &str) -> Macro {
        Macro { title: title.to_string(), steps: self.steps, ..Macro::default() }
    }
}

/// How many bytes `value` takes as compact JSON, without building the text.
fn json_size(value: &Value) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(buf.len());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    match serde_json::to_writer(&mut count, value) {
        Ok(()) => count.0,
        Err(_) => usize::MAX,
    }
}

/// Whether a file name ends in `.fcmacro`, in any case.
pub fn is_macro_file(name: &str) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(MACRO_EXTENSION))
}

/// The file name of a folder path without its extension: the menu label of an untitled macro.
pub fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.eq_ignore_ascii_case(MACRO_EXTENSION) => stem,
        _ => name,
    }
}

/// Reads a macro file, refusing one over [`MAX_MACRO_BYTES`] without reading the rest of it.
pub fn read_macro_file(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let cap = u64::try_from(MAX_MACRO_BYTES).unwrap_or(u64::MAX).saturating_add(1);
    file.take(cap).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MACRO_BYTES {
        return Err(MacroError::TooLarge(bytes.len()).to_string());
    }
    Ok(bytes)
}

/// The stamps of the macro files under `dir` (subfolders down to
/// [`MAX_MACRO_FOLDER_DEPTH`], at most [`MAX_FOLDER_FILES`] files), sorted by path. Hidden files,
/// folders reached through symbolic links and other files are skipped; a missing folder has none.
pub fn stamp_folder(dir: &Path) -> FolderStamps {
    let mut out = Vec::new();
    let mut seen = 0usize;
    walk(dir, "", 0, &mut seen, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    FolderStamps(out)
}

fn walk(dir: &Path, prefix: &str, depth: usize, seen: &mut usize, out: &mut Vec<FileStamp>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        *seen += 1;
        if *seen > MAX_FOLDER_ENTRIES || out.len() >= MAX_FOLDER_FILES {
            return;
        }
        let Ok(name) = entry.file_name().into_string() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if path.len() > MAX_MACRO_TEXT {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if depth < MAX_MACRO_FOLDER_DEPTH {
                walk(&entry.path(), &path, depth + 1, seen, out);
            }
            continue;
        }
        if !is_macro_file(&name) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(entry.path()) else { continue };
        if !meta.is_file() {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        out.push(FileStamp { path, size: meta.len(), modified_ms });
    }
}

/// One file of a folder listing: its path relative to the folder and its bytes, or why they
/// could not be read.
pub type FolderFile = (String, Result<Vec<u8>, String>);

/// Reads the macro files under `dir`: their stamps and contents, at most
/// [`MAX_MACRO_FOLDER_BYTES`] in all.
pub fn read_folder(dir: &Path) -> (FolderStamps, Vec<FolderFile>) {
    let stamps = stamp_folder(dir);
    let mut left = MAX_MACRO_FOLDER_BYTES;
    let files = stamps
        .0
        .iter()
        .map(|stamp| {
            let bytes = if stamp.size > left {
                Err(format!("not read: the Macros folder holds more than {} MiB of macros", MAX_MACRO_FOLDER_BYTES >> 20))
            } else {
                read_macro_file(&dir.join(&stamp.path)).inspect(|b| left = left.saturating_sub(u64::try_from(b.len()).unwrap_or(u64::MAX)))
            };
            (stamp.path.clone(), bytes)
        })
        .collect();
    (stamps, files)
}

/// Re-reads the session's Macros folder when its files changed since the last poll; returns
/// whether the macro listing changed. Without a Macros folder, nothing happens.
pub fn poll(ext: &mut ExtensionState) -> bool {
    let Some(dir) = ext.macros_dir.clone() else { return false };
    if !ext.macros_stamps.changed(&stamp_folder(&dir)) {
        return false;
    }
    let (stamps, files) = read_folder(&dir);
    ext.sync_macros(files, stamps)
}

/// Compares two labels as the Script menu sorts them: ignoring case, with runs of digits in
/// numeric order (`Step 2` before `Step 10`).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (m, n) = (digits(&mut x), digits(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        out.push(c);
        it.next();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macro_with_no_steps_parses_and_writes_back() {
        let m = Macro::parse(br#"{"format": "fontcraft.macro", "version": 1}"#).unwrap();
        assert!(m.steps.is_empty());
        assert_eq!(Macro::parse(m.to_json().to_string().as_bytes()).unwrap(), m);
        assert!(matches!(Macro::parse(br#"{"format": "fontcraft.macro", "version": 2}"#), Err(MacroError::NewerVersion(2))));
    }

    #[test]
    fn a_macro_round_trips_through_its_file_form() {
        let text = br#"{"format": "fontcraft.macro", "version": 1, "title": "Widen", "description": "Two\nlines",
            "args": {"width": {"type": "int", "min": 0, "max": 2000, "default": 600}}, "requires": ["com.example.shift"],
            "steps": [{"command": "spacing.setWidth", "params": {"glyphs": {"$arg": "glyphs"}, "value": {"$arg": "width"}}}]}"#;
        let m = Macro::parse(text).unwrap();
        assert_eq!(m.title, "Widen");
        assert_eq!(m.description, "Two\nlines");
        assert_eq!(m.requires, ["com.example.shift"]);
        assert_eq!(Macro::parse(m.to_json().to_string().as_bytes()).unwrap(), m);
        assert_eq!(Macro::from_value(&m.to_json()).unwrap(), m);
        assert!(matches!(Macro::parse(br#"{"format": "something.else", "version": 1}"#), Err(MacroError::Format(_))));
        assert!(matches!(Macro::parse(&vec![b' '; MAX_MACRO_BYTES + 1]), Err(MacroError::TooLarge(_))));
    }

    #[test]
    fn placeholders_are_filled_from_arguments_defaults_and_the_selection() {
        // Steps resolve against the installed command tables; no session installs them here.
        crate::cmd::install().unwrap();
        let m = Macro::from_value(&json!({
            "args": {"width": {"type": "int", "min": 0, "max": 2000, "default": 600}},
            "steps": [{"command": "spacing.setWidth", "params": {"glyphs": {"$arg": "glyphs"}, "value": {"$arg": "width"}}}],
        }))
        .unwrap();
        let steps = m.resolve(&json!({"glyphs": ["A", "B"]})).unwrap();
        assert_eq!(steps, [Step { command: "spacing.setWidth".into(), params: json!({"glyphs": ["A", "B"], "value": 600}) }]);
        let steps = m.resolve(&json!({"glyphs": ["A"], "width": 700})).unwrap();
        assert_eq!(steps.first().map(|s| &s.params), Some(&json!({"glyphs": ["A"], "value": 700})));
        assert!(matches!(m.resolve(&json!({"glyphs": ["A"], "width": 9000})), Err(MacroError::BadArg(..))));
        assert!(matches!(m.resolve(&json!({"glyphs": ["A"], "height": 1})), Err(MacroError::BadArg(..))));
        // `glyphs` is built in, but a run without a selection has no value for it.
        assert!(matches!(m.resolve(&json!({})), Err(MacroError::BadArg(..))));
    }

    #[test]
    fn unknown_commands_and_arguments_are_refused_by_step() {
        crate::cmd::install().unwrap();
        let unknown = Macro::from_value(&json!({"steps": [{"command": "glyph.newGlyph"}, {"command": "no.suchCommand"}]})).unwrap();
        assert_eq!(unknown.resolve(&Value::Null), Err(MacroError::UnknownCommand { step: 2, command: "no.suchCommand".into() }));
        assert!(unknown.check().is_err());
        let arg = Macro::from_value(&json!({"steps": [{"command": "glyph.newGlyph", "params": {"name": {"$arg": "nope"}}}]})).unwrap();
        assert_eq!(arg.resolve(&Value::Null), Err(MacroError::UnknownArg { step: 1, arg: "nope".into() }));
        assert!(arg.check().is_err());
        let undo = Macro::from_value(&json!({"steps": [{"command": "edit.undo"}]})).unwrap();
        assert!(matches!(undo.resolve(&Value::Null), Err(MacroError::NotAStep { step: 1, .. })));
        let mut deep = json!(1);
        for _ in 0..40 {
            deep = json!([deep]);
        }
        let deep = Macro::from_value(&json!({"steps": [{"command": "glyph.newGlyph", "params": {"name": deep}}]})).unwrap();
        assert!(matches!(deep.resolve(&Value::Null), Err(MacroError::Limit(_))));
    }

    #[test]
    fn renamed_commands_and_parameters_resolve_through_their_aliases() {
        crate::cmd::install().unwrap();
        let m = Macro::from_value(&json!({"steps": [{"command": "glyph.makeGlyph", "params": {"glyphName": "x"}}]})).unwrap();
        let mut left = MAX_RESOLVED_VALUES;
        let steps =
            m.resolve_with(&Value::Null, &[("glyph.makeGlyph", "glyph.newGlyph")], &[("glyph.newGlyph", "glyphName", "name")], &mut left).unwrap();
        assert_eq!(steps, [Step { command: "glyph.newGlyph".into(), params: json!({"name": "x"}) }]);
        assert!(m.resolve(&Value::Null).is_err(), "the test alias is not a real one");
        for (old, new) in COMMAND_ALIASES {
            assert!(crate::find_command(old).is_none(), "{old} is still registered, so it needs no alias");
            assert_eq!(resolve_command(old), crate::find_command(new).map(|c| c.id));
        }
    }

    #[test]
    fn a_long_list_pasted_into_many_steps_is_refused() {
        crate::cmd::install().unwrap();
        let glyphs: Vec<String> = (0..10_000).map(|i| format!("g{i}")).collect();
        let step = json!({"command": "spacing.setWidth", "params": {"glyphs": {"$arg": "glyphs"}, "value": 1}});
        let m = Macro::from_value(&json!({"steps": vec![step; 5_000]})).unwrap();
        assert!(matches!(m.resolve(&json!({"glyphs": glyphs})), Err(MacroError::Limit(_))));
    }

    #[test]
    fn recording_stops_at_its_limit_and_skips_its_own_commands() {
        let mut r = Recording::new();
        r.record("script.recordMacro", &json!({"action": "start"}));
        r.record("glyph.newGlyph", &json!({"name": "a"}));
        r.record("edit.undo", &Value::Null);
        assert_eq!(r.steps.len(), 1);
        for _ in 0..MAX_RECORDED_STEPS + 5 {
            r.record("glyph.newGlyph", &Value::Null);
        }
        assert_eq!(r.steps.len(), MAX_RECORDED_STEPS);
        assert!(r.full);
    }

    #[test]
    fn recording_stops_at_its_byte_limit() {
        let mut r = Recording::new();
        let big = json!({"name": "x".repeat(1 << 20)});
        for _ in 0..(MAX_RECORDED_BYTES >> 20) + 5 {
            r.record("glyph.newGlyph", &big);
        }
        assert!(r.full);
        assert!(r.bytes <= MAX_RECORDED_BYTES);
        assert!(r.steps.len() < (MAX_RECORDED_BYTES >> 20));
        let kept = r.steps.len();
        r.record("glyph.newGlyph", &json!({}));
        assert_eq!(r.steps.len(), kept, "a full recording takes no more steps");
    }

    #[test]
    fn menu_labels_sort_naturally() {
        let mut v = vec!["step 10", "Step 2", "alpha", "Zulu", "step 1"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["alpha", "step 1", "Step 2", "step 10", "Zulu"]);
        assert_eq!(file_stem("Tools/Tighten.fcmacro"), "Tighten");
        assert_eq!(file_stem("x.FCMACRO"), "x");
    }
}
