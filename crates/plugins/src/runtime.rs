//! The wasmi host: loading, validating and calling a module under resource limits.
//!
//! Ported from VectorCraft's `crates/plugins/src/runtime.rs` (MIT OR Apache-2.0, the same
//! owner; see `NOTICE`): the exports are renamed to `fc_*`, memory growth past the cap ends the
//! run, and the reporter deadline is added.
//!
//! Loading checks the size, compiles the module with full validation, refuses any import, checks
//! the exports and their types, calls `fc_abi_version` (must be [`ABI_VERSION`]) and
//! `fc_manifest`, and parses the manifest. Each [`Plugin::call`] makes a fresh store and
//! instance, copies the input and parameters into memory through `fc_alloc`, runs `fc_run` with
//! fuel handed out in slices (between slices the native build checks the wall clock), and copies
//! out at most [`Limits::max_output_bytes`] of output.

use std::sync::atomic::{AtomicU64, Ordering};

use wasmi::{
    Config, EnforcedLimits, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode, TypedFunc,
    TypedResumableCall, WasmParams, WasmResults,
};

use crate::error::{
    FUEL_BASE, FUEL_PER_INPUT_BYTE, MAX_CALL_DEPTH, MAX_MANIFEST_BYTES, MAX_MEMORY_BYTES, MAX_MODULE_BYTES, MAX_OUTPUT_BYTES, REPORTER_WALL_TIME_MS,
    WALL_TIME_MS,
};
use crate::manifest::{Kind, Manifest};
use crate::{Error, Result};

/// The module ABI version this host implements (`fc_abi_version` must return it).
pub const ABI_VERSION: i32 = 1;

/// Fuel handed out per slice; between slices the native build checks the deadline.
const FUEL_SLICE: u64 = 20_000_000;
/// Most tables and table elements a module may have (function tables of compiled Rust are small).
const MAX_TABLES: usize = 4;
const MAX_TABLE_ELEMENTS: usize = 100_000;
/// The smallest call depth a [`Limits`] can ask for (wasmi needs a few frames of its own).
const MIN_CALL_DEPTH: usize = 16;

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
    /// Deepest call nesting. Fixed when the module is loaded: a call under other limits keeps the
    /// loaded module's depth.
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

/// Load serials: unique per process, so caches can tell two loads of one module apart.
static SERIAL: AtomicU64 = AtomicU64::new(0);

/// A loaded, validated module. Cheap to share (`Arc`) and safe to call from several threads:
/// every call gets its own store and instance.
pub struct Plugin {
    manifest: Manifest,
    engine: Engine,
    module: Module,
    limits: Limits,
    /// Size of the module binary, in bytes.
    size: usize,
    /// A number unique to this load in the process; caches key on it so a reinstalled module
    /// never reuses an old result.
    serial: u64,
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

/// When a run must stop (native: a wall-clock deadline; the web build has no monotonic clock in
/// `std`, so fuel alone bounds it there).
struct Deadline {
    #[cfg(not(target_arch = "wasm32"))]
    at: Option<std::time::Instant>,
}

impl Deadline {
    fn new(limits: &Limits) -> Self {
        #[cfg(target_arch = "wasm32")]
        let _ = limits;
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            at: std::time::Instant::now().checked_add(std::time::Duration::from_millis(limits.wall_time_ms)),
        }
    }

    fn check(&self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.at.is_some_and(|at| std::time::Instant::now() >= at) {
            return Err(Error::Limit("time budget".into()));
        }
        Ok(())
    }
}

/// One live instance: a store with its own memory limit, and the module's exports.
struct Live {
    store: Store<StoreLimits>,
    instance: Instance,
    memory: Memory,
}

impl Live {
    fn func<P: WasmParams, R: WasmResults>(&self, name: &str) -> Result<TypedFunc<P, R>> {
        self.instance.get_typed_func::<P, R>(&self.store, name).map_err(|e| Error::Abi(format!("export `{name}`: {e}")))
    }

    /// The `len << 32 | ptr` block a module returned, copied out (at most `max` bytes).
    fn read_packed(&self, packed: i64, what: &str, max: usize) -> Result<Vec<u8>> {
        let packed = packed as u64;
        let (ptr, len) = ((packed & 0xffff_ffff) as usize, (packed >> 32) as usize);
        if len > max {
            return Err(Error::Limit(format!("output size ({what} is {len} bytes, limit {max})")));
        }
        let bytes = self.memory.data(&self.store).get(ptr..ptr.saturating_add(len));
        bytes.map(<[u8]>::to_vec).ok_or_else(|| Error::Abi(format!("{what} points outside the module's memory")))
    }
}

fn wasm_err(e: wasmi::Error) -> Error {
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => Error::Limit("instruction budget".into()),
        Some(TrapCode::StackOverflow) => Error::Limit("call depth".into()),
        Some(TrapCode::GrowthOperationLimited) => Error::Limit("memory budget".into()),
        _ => Error::Trap(e.to_string()),
    }
}

/// `fc_run(input_ptr, input_len, params_ptr, params_len) -> len << 32 | ptr` (negative: error).
type RunArgs = (i32, i32, i32, i32);

impl Plugin {
    /// Compiles and validates `bytes` as a module and reads its manifest. Fails without side
    /// effects on anything malformed, any import, a wrong ABI version or a bad manifest.
    pub fn load(bytes: &[u8], limits: Limits) -> Result<Plugin> {
        if bytes.len() > limits.max_module_bytes {
            return Err(Error::Module(format!("module is {} bytes (limit {})", bytes.len(), limits.max_module_bytes)));
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(Error::Module("not a WebAssembly binary (the \\0asm header is missing)".into()));
        }
        let mut config = Config::default();
        config.consume_fuel(true).enforced_limits(EnforcedLimits::strict()).set_max_recursion_depth(limits.max_call_depth.max(MIN_CALL_DEPTH));
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| Error::Module(e.to_string()))?;
        if let Some(imp) = module.imports().next() {
            return Err(Error::Module(format!(
                "the module imports `{}.{}`, but modules get no host imports (no files, network, clock or interface)",
                imp.module(),
                imp.name()
            )));
        }
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let mut p = Plugin { manifest: Manifest::placeholder(), engine, module, limits, size: bytes.len(), serial };
        let deadline = Deadline::new(&p.limits);
        let mut live = p.guarded(|| p.instantiate(&p.limits))?;
        let version: TypedFunc<(), i32> = live.func("fc_abi_version")?;
        let v = p.guarded(|| p.call_func(&mut live, &version, (), p.limits.fuel_base, &deadline))?;
        if v != ABI_VERSION {
            return Err(Error::Abi(format!("fc_abi_version returned {v}; this host implements version {ABI_VERSION}")));
        }
        let manifest: TypedFunc<(), i64> = live.func("fc_manifest")?;
        let packed = p.guarded(|| p.call_func(&mut live, &manifest, (), p.limits.fuel_base, &deadline))?;
        let text = live.read_packed(packed, "fc_manifest", MAX_MANIFEST_BYTES).map_err(|e| match e {
            Error::Limit(_) => Error::Manifest(format!("manifest is too long (limit {MAX_MANIFEST_BYTES} bytes)")),
            e => e,
        })?;
        p.manifest = Manifest::parse(&text)?;
        // The other exports must exist with the right types.
        live.func::<i32, i32>("fc_alloc")?;
        live.func::<RunArgs, i64>("fc_run")?;
        Ok(p)
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
        self.guarded(|| self.call_inner(input, params, limits))
    }

    /// Runs `f`, turning a panic inside wasmi into an error. wasmi reports misbehaving modules as
    /// errors; a panic would be a bug in it and must still not take the document down (native
    /// builds; the web build aborts on panic anyway).
    fn guarded<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| Error::Trap("the module runtime failed".into()))?
    }

    fn instantiate(&self, limits: &Limits) -> Result<Live> {
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes.min(self.limits.max_memory_bytes))
            .memories(1)
            .tables(MAX_TABLES)
            .table_elements(MAX_TABLE_ELEMENTS)
            .instances(1)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, store_limits);
        store.limiter(|l| l);
        // The start function (if any) runs on this fuel.
        store.set_fuel(limits.fuel_base).map_err(wasm_err)?;
        let linker = Linker::<StoreLimits>::new(&self.engine);
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(wasm_err)?;
        let memory =
            instance.get_memory(&store, "memory").ok_or_else(|| Error::Abi("the module must export its linear memory as `memory`".into()))?;
        Ok(Live { store, instance, memory })
    }

    /// Calls `f` with at most `fuel` instructions, in slices, checking the deadline between them.
    fn call_func<P: WasmParams, R: WasmResults>(&self, live: &mut Live, f: &TypedFunc<P, R>, params: P, fuel: u64, deadline: &Deadline) -> Result<R> {
        let mut left = fuel;
        let give = left.min(FUEL_SLICE);
        left -= give;
        live.store.set_fuel(give).map_err(wasm_err)?;
        let mut call = f.call_resumable(&mut live.store, params).map_err(wasm_err)?;
        loop {
            match call {
                TypedResumableCall::Finished(r) => return Ok(r),
                TypedResumableCall::HostTrap(_) => return Err(Error::Trap("unexpected host trap".into())),
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    if left == 0 || need > left {
                        return Err(Error::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    let give = left.min(FUEL_SLICE.max(need));
                    left -= give;
                    live.store.set_fuel(give).map_err(wasm_err)?;
                    call = inv.resume(&mut live.store).map_err(wasm_err)?;
                }
            }
        }
    }

    fn call_inner(&self, input: &[u8], params: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        let memory = limits.max_memory_bytes.min(self.limits.max_memory_bytes);
        let total = input.len().checked_add(params.len()).filter(|b| *b <= i32::MAX as usize && *b <= memory / 2);
        total.ok_or_else(|| Error::Limit("memory budget (the input is too large to hand over)".into()))?;
        let deadline = Deadline::new(limits);
        let mut live = self.instantiate(limits)?;
        let alloc: TypedFunc<i32, i32> = live.func("fc_alloc")?;
        let run: TypedFunc<RunArgs, i64> = live.func("fc_run")?;
        let place = |live: &mut Live, bytes: &[u8]| -> Result<usize> {
            let ptr = self.call_func(live, &alloc, bytes.len().max(1) as i32, limits.fuel_base, &deadline)? as u32 as usize;
            if ptr == 0 {
                return Err(Error::Failed("fc_alloc returned 0 (out of memory)".into()));
            }
            let outside = || Error::Abi("fc_alloc returned a block outside the module's memory".into());
            let end = ptr.checked_add(bytes.len()).ok_or_else(outside)?;
            live.memory.data_mut(&mut live.store).get_mut(ptr..end).ok_or_else(outside)?.copy_from_slice(bytes);
            Ok(ptr)
        };
        let iptr = place(&mut live, input)?;
        let pptr = place(&mut live, params)?;
        let fuel = limits.fuel_base.saturating_add((input.len() as u64).saturating_mul(limits.fuel_per_byte));
        let packed = self.call_func(&mut live, &run, (iptr as i32, input.len() as i32, pptr as i32, params.len() as i32), fuel, &deadline)?;
        if packed < 0 {
            return Err(Error::Failed(format!("fc_run returned error code {packed}")));
        }
        live.read_packed(packed, "the output", limits.max_output_bytes.min(MAX_OUTPUT_BYTES))
    }
}
