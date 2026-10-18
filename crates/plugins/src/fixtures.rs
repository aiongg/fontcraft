//! Module binaries built without the `wat` crate, for the tests of crates that drive modules
//! through the engine (the engine, the shell) and do not depend on `wat`.
//!
//! [`returning`] encodes the WebAssembly binary of a module with ABI v1 whose `fc_run` returns the
//! same output whatever it is given: the manifest sits at address 16, the output at 64 KiB, and
//! `fc_alloc` grows memory by whole pages so the host's copies never overlap either. It is the
//! binary form of [`crate::wat::returning`].

/// Where the output is kept in memory.
const OUTPUT_AT: u32 = 0x1_0000;
/// Where the manifest is kept in memory.
const MANIFEST_AT: u32 = 16;

/// A module of ABI version 1 with the manifest `manifest` (JSON, under 64 KiB) whose `fc_run`
/// always returns `output`.
pub fn returning(manifest: &str, output: &str) -> Vec<u8> {
    let out_len = u32::try_from(output.len()).unwrap_or(u32::MAX);
    let man_len = u32::try_from(manifest.len()).unwrap_or(u32::MAX);
    let pages = OUTPUT_AT.saturating_add(out_len) / 0x1_0000 + 1;
    let packed = |len: u32, at: u32| (i64::from(len) << 32) | i64::from(at);

    let mut m = b"\0asm\x01\x00\x00\x00".to_vec();
    // Types: 0 () -> i32, 1 () -> i64, 2 (i32) -> i32, 3 (i32 i32 i32 i32) -> i64.
    section(&mut m, 1, &[4, 0x60, 0, 1, 0x7f, 0x60, 0, 1, 0x7e, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7e]);
    // Functions: fc_abi_version, fc_manifest, fc_alloc, fc_run.
    section(&mut m, 3, &[4, 0, 1, 2, 3]);
    // One memory with no maximum.
    let mut memory = vec![1, 0];
    uleb(&mut memory, u64::from(pages));
    section(&mut m, 5, &memory);
    let mut exports = vec![5];
    for (name, kind, index) in [("memory", 2u8, 0u8), ("fc_abi_version", 0, 0), ("fc_manifest", 0, 1), ("fc_alloc", 0, 2), ("fc_run", 0, 3)] {
        name_bytes(&mut exports, name.as_bytes());
        exports.extend([kind, index]);
    }
    section(&mut m, 7, &exports);

    let mut abi = vec![0x41];
    sleb(&mut abi, 1);
    let mut man = vec![0x42];
    sleb(&mut man, packed(man_len, MANIFEST_AT));
    // memory.grow by ceil(n / 65536) pages; the old size in pages, shifted to an address.
    let mut alloc = vec![0x20, 0, 0x41];
    sleb(&mut alloc, 65_535);
    alloc.extend([0x6a, 0x41, 16, 0x76, 0x40, 0, 0x41, 16, 0x74]);
    let mut run = vec![0x42];
    sleb(&mut run, packed(out_len, OUTPUT_AT));
    let mut code = vec![4];
    for body in [abi, man, alloc, run] {
        // No locals, the instructions, `end`.
        let mut b = vec![0];
        b.extend(body);
        b.push(0x0b);
        uleb(&mut code, b.len() as u64);
        code.extend(b);
    }
    section(&mut m, 10, &code);

    let mut data = vec![2];
    for (at, bytes) in [(MANIFEST_AT, manifest.as_bytes()), (OUTPUT_AT, output.as_bytes())] {
        data.extend([0, 0x41]);
        sleb(&mut data, i64::from(at));
        data.push(0x0b);
        name_bytes(&mut data, bytes);
    }
    section(&mut m, 11, &data);
    m
}

fn section(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    uleb(out, body.len() as u64);
    out.extend_from_slice(body);
}

/// A length-prefixed byte string (names and data segments).
fn name_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    uleb(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
        if done {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Kind, Limits, Plugin};

    #[test]
    fn a_returning_binary_loads_and_runs() {
        let manifest = crate::wat::manifest("org.test.fixed", "filter", "{}");
        let p = Plugin::load(&super::returning(&manifest, r#"{"shapes":[]}"#), Limits::default()).unwrap();
        assert_eq!((p.id(), p.kind()), ("org.test.fixed", Kind::Filter));
        assert_eq!(p.call(b"{\"layer\":{}}", b"{}").unwrap(), br#"{"shapes":[]}"#);
    }
}
