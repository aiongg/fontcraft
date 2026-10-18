//! The FontCraft module ABI, version 1: the four functions the host calls. This file is the same
//! in every template; copy it unchanged and write the module's behaviour in `lib.rs`.
//!
//! The host loads the module once and calls `fc_abi_version` and `fc_manifest`. For each run it
//! makes a fresh instance, places the input and the parameters in the module's memory through
//! `fc_alloc`, calls `fc_run` and reads the output it points to. Nothing survives between runs,
//! so the memory handed out here is never freed.
//!
//! A returned `i64` packs a length (high 32 bits) and an address (low 32 bits) of UTF-8 JSON in
//! the module's memory. Reference: `docs/plugins.md` in the FontCraft repository.

/// The ABI version this file implements.
const ABI_VERSION: i32 = 1;

#[unsafe(no_mangle)]
pub extern "C" fn fc_abi_version() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn fc_manifest() -> i64 {
    pack(crate::MANIFEST.as_bytes())
}

/// A block of `size` bytes for the host to write into; 0 when there is no memory left.
#[unsafe(no_mangle)]
pub extern "C" fn fc_alloc(size: i32) -> i32 {
    let Ok(size) = usize::try_from(size) else { return 0 };
    // A block of at least one byte, so its address is never 0 (0 means "out of memory").
    let size = size.max(1);
    let mut block: Vec<u8> = Vec::new();
    if block.try_reserve_exact(size).is_err() {
        return 0;
    }
    block.resize(size, 0);
    // Addresses are 32-bit on wasm32; the cast keeps every bit.
    Vec::leak(block).as_mut_ptr().expose_provenance() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn fc_run(input: i32, input_len: i32, params: i32, params_len: i32) -> i64 {
    let input = bytes_at(input, input_len);
    let params = bytes_at(params, params_len);
    pack(Vec::leak(crate::run(input, params)))
}

/// The `len` bytes the host wrote at `address`.
fn bytes_at(address: i32, len: i32) -> &'static [u8] {
    let Ok(len) = usize::try_from(len) else { return &[] };
    if len == 0 {
        return &[];
    }
    let ptr = std::ptr::with_exposed_provenance::<u8>(address as u32 as usize);
    // SAFETY: the host calls `fc_run` with the address of a block `fc_alloc` handed out (and
    // leaked, so it lives for the rest of the run) and the number of bytes it wrote into it.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// `bytes`' length and address packed into the `i64` the host expects.
fn pack(bytes: &[u8]) -> i64 {
    let len = bytes.len() as u32;
    let address = bytes.as_ptr().expose_provenance() as u32;
    (i64::from(len) << 32) | i64::from(address)
}
