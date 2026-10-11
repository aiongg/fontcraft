//! The wasmi host: loading, validating and calling a module under resource limits (M13.2 fills
//! the bodies, ported from VectorCraft's `crates/plugins/src/runtime.rs` with the exports renamed
//! to `fc_*`, credit in the header and `NOTICE`).
//!
//! Loading checks the size, compiles the module with full validation, refuses any import, checks
//! the exports and their types, calls `fc_abi_version` (must be [`ABI_VERSION`]) and
//! `fc_manifest`, and parses the manifest. Each [`Plugin::call`] makes a fresh store and
//! instance, copies the input and parameters into memory through `fc_alloc`, runs `fc_run` with
//! fuel handed out in slices (between slices the native build checks the wall clock), and copies
//! out at most [`Limits::max_output_bytes`] of output.

use crate::error::{
    FUEL_BASE, FUEL_PER_INPUT_BYTE, MAX_CALL_DEPTH, MAX_MEMORY_BYTES, MAX_MODULE_BYTES, MAX_OUTPUT_BYTES, REPORTER_WALL_TIME_MS, WALL_TIME_MS,
};
use crate::manifest::{Kind, Manifest};
use crate::{Error, Result};

/// The module ABI version this host implements (`fc_abi_version` must return it).
pub const ABI_VERSION: i32 = 1;

/// Resource limits for one module run. [`Limits::default`] is the filter and check budget;
/// [`Limits::for_kind`] gives reporters their shorter deadline.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Largest module accepted.
    pub max_module_bytes: usize,
    /// Cap on the module's linear memory. Input and parameters may use half of it.
    pub max_memory_bytes: usize,
    /// Largest output JSON read back.
    pub max_output_bytes: usize,
    /// Instructions any call may use, plus [`Limits::fuel_per_byte`] per byte of input to `fc_run`.
    pub fuel_base: u64,
    pub fuel_per_byte: u64,
    /// Deepest call nesting.
    pub max_call_depth: usize,
    /// Wall-clock budget for one run, in milliseconds (native builds only).
    pub wall_time_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_module_bytes: MAX_MODULE_BYTES,
            max_memory_bytes: MAX_MEMORY_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            fuel_base: FUEL_BASE,
            fuel_per_byte: FUEL_PER_INPUT_BYTE,
            max_call_depth: MAX_CALL_DEPTH,
            wall_time_ms: WALL_TIME_MS,
        }
    }
}

impl Limits {
    /// The limits a module of `kind` runs under: reporters get [`REPORTER_WALL_TIME_MS`], the
    /// others [`WALL_TIME_MS`].
    pub fn for_kind(kind: Kind) -> Self {
        match kind {
            Kind::Reporter => Self { wall_time_ms: REPORTER_WALL_TIME_MS, ..Self::default() },
            Kind::Filter | Kind::Check => Self::default(),
        }
    }
}

/// A loaded, validated module. Cheap to share (`Arc`) and safe to call from several threads:
/// every call gets its own store and instance.
pub struct Plugin {
    manifest: Manifest,
    limits: Limits,
    /// Size of the module binary, in bytes.
    size: usize,
    /// A number unique to this load in the process; caches key on it so a reinstalled module
    /// never reuses an old result.
    serial: u64,
    // M13.2: the wasmi engine and compiled module.
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("id", &self.manifest.id)
            .field("kind", &self.manifest.kind)
            .field("size", &self.size)
            .field("serial", &self.serial)
            .finish()
    }
}

impl Plugin {
    /// Compiles and validates `bytes` as a module and reads its manifest. Fails without side
    /// effects on anything malformed, any import, a wrong ABI version or a bad manifest.
    pub fn load(bytes: &[u8], limits: Limits) -> Result<Plugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        // M13.2: compile, check exports, read the ABI version and the manifest.
        Err(Error::Unsupported("loading modules".into()))
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The manifest's id.
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn kind(&self) -> Kind {
        self.manifest.kind
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Size of the module binary, in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// A number unique to this load in the process.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// Runs `fc_run` once on `input` (the kind's input JSON) and `params` (the parameter object
    /// as JSON) in a fresh instance under the module's limits, and returns the output bytes
    /// unparsed. Parse them with the kind's parser ([`crate::layer_json::FilterOutput::parse`],
    /// [`crate::reporter::Drawing::parse`], [`crate::check::parse_findings`]).
    pub fn call(&self, input: &[u8], params: &[u8]) -> Result<Vec<u8>> {
        self.call_limited(input, params, &self.limits)
    }

    /// [`Plugin::call`] under other limits (a reporter's deadline, a test's tiny budget).
    pub fn call_limited(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        let _ = (input, params, limits);
        // M13.2: instantiate, copy in, run with fuel slices, copy out.
        Err(Error::Unsupported("running modules".into()))
    }
}
