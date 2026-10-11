//! The module error and the limits of `docs/plans/M13.md` § 3 › Limits. Every way a module can
//! misbehave ends in an [`Error`]; every count or size read from a module, a manifest or a
//! parameter string is capped by one of these constants.

/// Largest module file accepted, in bytes.
pub const MAX_MODULE_BYTES: usize = 32 << 20;
/// Largest manifest read from `fc_manifest`, in bytes.
pub const MAX_MANIFEST_BYTES: usize = 64 << 10;
/// Most modules installed at once in a session.
pub const MAX_INSTALLED: usize = 256;
/// Most parameters a manifest may declare.
pub const MAX_PARAMS: usize = 64;
/// Longest module id (`[A-Za-z0-9._-]`), in characters.
pub const MAX_ID_LEN: usize = 64;
/// Instructions (fuel) any one call may use before the per-byte allowance.
pub const FUEL_BASE: u64 = 50_000_000;
/// Extra instructions per byte of input to `fc_run`.
pub const FUEL_PER_INPUT_BYTE: u64 = 4_000;
/// Cap on a module's linear memory, in bytes (native builds).
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_MEMORY_BYTES: usize = 512 << 20;
/// Cap on a module's linear memory, in bytes (the web build, which has less memory to spare).
#[cfg(target_arch = "wasm32")]
pub const MAX_MEMORY_BYTES: usize = 256 << 20;
/// Wall-clock budget of one filter or check run, in milliseconds (native builds only: the web
/// build has no monotonic clock in `std`, so fuel alone bounds it there).
pub const WALL_TIME_MS: u64 = 60_000;
/// Wall-clock budget of one reporter run, in milliseconds (native builds only).
pub const REPORTER_WALL_TIME_MS: u64 = 5_000;
/// Deepest call nesting inside a module.
pub const MAX_CALL_DEPTH: usize = 1_024;
/// Largest output JSON read back from `fc_run`, in bytes.
pub const MAX_OUTPUT_BYTES: usize = 64 << 20;
/// Most log lines a filter may return per layer.
pub const MAX_LOG_LINES: usize = 1_000;
/// Longest log line, in characters; longer lines are cut.
pub const MAX_LOG_LINE_CHARS: usize = 1_000;
/// Most primitives a reporter may return per layer (background and foreground together).
pub const MAX_PRIMITIVES: usize = 100_000;
/// Longest text item of a reporter drawing, in characters.
pub const MAX_TEXT_CHARS: usize = 1_000;
/// Most findings a check may return per layer.
pub const MAX_FINDINGS: usize = 10_000;
/// Longest `Filter` / `PreFilter` parameter string, in bytes.
pub const MAX_FILTER_STRING_BYTES: usize = 4_096;
/// Most `;`-separated parts of one filter string.
pub const MAX_FILTER_PARTS: usize = 256;

/// Every limit above as `(name, value)`, for documentation, `app.info` and tests. Byte sizes and
/// milliseconds are given as plain numbers.
pub const LIMITS_TABLE: &[(&str, u64)] = &[
    ("maxModuleBytes", MAX_MODULE_BYTES as u64),
    ("maxManifestBytes", MAX_MANIFEST_BYTES as u64),
    ("maxInstalled", MAX_INSTALLED as u64),
    ("maxParams", MAX_PARAMS as u64),
    ("maxIdLen", MAX_ID_LEN as u64),
    ("fuelBase", FUEL_BASE),
    ("fuelPerInputByte", FUEL_PER_INPUT_BYTE),
    ("maxMemoryBytes", MAX_MEMORY_BYTES as u64),
    ("wallTimeMs", WALL_TIME_MS),
    ("reporterWallTimeMs", REPORTER_WALL_TIME_MS),
    ("maxCallDepth", MAX_CALL_DEPTH as u64),
    ("maxOutputBytes", MAX_OUTPUT_BYTES as u64),
    ("maxLogLines", MAX_LOG_LINES as u64),
    ("maxLogLineChars", MAX_LOG_LINE_CHARS as u64),
    ("maxPrimitives", MAX_PRIMITIVES as u64),
    ("maxTextChars", MAX_TEXT_CHARS as u64),
    ("maxFindings", MAX_FINDINGS as u64),
    ("maxFilterStringBytes", MAX_FILTER_STRING_BYTES as u64),
    ("maxFilterParts", MAX_FILTER_PARTS as u64),
];

/// A module error. The message names what went wrong in words a user can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a valid WebAssembly module, or it imports something.
    #[error("not a valid module: {0}")]
    Module(String),
    /// A required export is missing or has the wrong type, or `fc_abi_version` is not 1.
    #[error("module ABI: {0}")]
    Abi(String),
    /// The manifest is too large, not JSON, or breaks a rule.
    #[error("module manifest: {0}")]
    Manifest(String),
    /// The module trapped (unreachable, out-of-bounds access, division by zero…).
    #[error("module trapped: {0}")]
    Trap(String),
    /// The module ran out of its instructions, memory, time or call depth, or returned too much.
    #[error("module exceeded its {0}")]
    Limit(String),
    /// Parameters are not an object or a value has the wrong type.
    #[error("module parameters: {0}")]
    Params(String),
    /// The output is not the JSON its kind expects, or breaks the model's limits.
    #[error("module output: {0}")]
    Output(String),
    /// The module reported a failure of its own (`{"error": "…"}`).
    #[error("module failed: {0}")]
    Failed(String),
    /// No installed module has this id.
    #[error("no module {0:?} is installed")]
    NotFound(String),
    /// A filter string or other text handed to this crate is malformed.
    #[error("{0}")]
    Syntax(String),
    /// What the skeleton has not built yet.
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;
