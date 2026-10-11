//! Macros: `.fcmacro` files of command steps, and macro recording (M13.7 fills `resolve` and the
//! recorder; parsing and writing are here already). Plan: `docs/plans/M13.md` § 3 (decision D8,
//! running and recording), § 4 › M13.7.
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
//! macro up to [`MAX_MACRO_NESTING`] deep; one that reaches itself is refused.
//!
//! Recording (`script.recordMacro`): while a [`Recording`] is set in the session's
//! `ExtensionState`, the hook in `Session::execute` hands it every successful font-changing
//! command (queries, shell commands, undo and redo are excluded).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::EngineError;

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
/// How deeply placeholders are searched for in one step's params.
pub const MAX_PLACEHOLDER_DEPTH: usize = 32;
/// Most steps one recording keeps; recording then stops with a message and keeps what it has.
pub const MAX_RECORDED_STEPS: usize = 100_000;
/// The arguments the shell fills from its selection without a declaration.
pub const BUILT_IN_ARGS: &[&str] = &["glyphs", "layers"];

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
        Ok(Macro { title: file.title, description: file.description, args: file.args, requires: file.requires, steps: file.steps })
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
        let _ = args;
        // M13.7.
        Err(MacroError::Unsupported("running macros".into()))
    }
}

/// A macro being recorded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recording {
    pub steps: Vec<Step>,
    /// Recording reached [`MAX_RECORDED_STEPS`] and stopped taking steps.
    pub full: bool,
}

impl Recording {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one successful font-changing command.
    pub fn record(&mut self, command: &str, params: &Value) {
        let _ = (command, params);
        // M13.7: push, stopping at MAX_RECORDED_STEPS.
    }

    /// The recorded steps as a macro titled `title`.
    pub fn finish(self, title: &str) -> Macro {
        Macro { title: title.to_string(), steps: self.steps, ..Macro::default() }
    }
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
}
