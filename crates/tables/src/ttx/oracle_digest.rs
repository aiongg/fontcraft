//! The expected tables of the `read_*` tests against `ttx`'s compiler: a digest file holds the
//! tag, byte length and SHA-256 (lowercase hex) of each table fontTools compiles, one per line,
//! the tag with `_` for a space. A mismatch prints our bytes as hex, to compare with `ttx`.

/// The tag, length and SHA-256 of each table of a digest file.
pub(super) fn expected(digest: &str) -> Vec<([u8; 4], usize, String)> {
    digest
        .lines()
        .map(|line| {
            let mut parts = line.split(' ');
            let tag: [u8; 4] = parts.next().unwrap().replace('_', " ").as_bytes().try_into().unwrap();
            (tag, parts.next().unwrap().parse().unwrap(), parts.next().unwrap().to_string())
        })
        .collect()
}

/// Panics, printing `got` as hex, unless it has the expected length and SHA-256.
pub(super) fn assert_table(tag: &[u8; 4], got: &[u8], len: usize, sha: &str) {
    if got.len() != len || fontcraft_plugins::sha256::hex(got) != sha {
        let hex: String = got.iter().map(|b| format!("{b:02x}")).collect();
        panic!("table '{}' differs from what ttx compiles ({} bytes, {len} expected); ours: {hex}", String::from_utf8_lossy(tag), got.len());
    }
}
