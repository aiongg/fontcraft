//! Wraps fonts for the cross-check (crates/container/crosscheck/crosscheck.py): for the n-th
//! file named, writes `OUT_DIR/n.woff` (WOFF 1.0) and `OUT_DIR/n.woff2` (WOFF2, every table
//! untransformed). A WOFF or WOFF2 input is unwrapped first. Errors go to stderr, one line per file.
//!
//!     cargo run -p fontcraft-container --example wrap -- OUT_DIR FILE...

use std::path::PathBuf;

use fontcraft_container::{Extras, to_sfnt, woff, woff2};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: wrap OUT_DIR FILE...");
        std::process::exit(2);
    };
    for (i, path) in args.enumerate() {
        let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
            let sfnt = to_sfnt(&bytes).map_err(|e| e.to_string())?;
            let w1 = woff::encode(&sfnt, &Extras::default()).map_err(|e| e.to_string())?;
            let w2 = woff2::encode(&sfnt, &woff2::Options::untransformed()).map_err(|e| e.to_string())?;
            std::fs::write(out.join(format!("{i}.woff")), w1).map_err(|e| e.to_string())?;
            std::fs::write(out.join(format!("{i}.woff2")), w2).map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            eprintln!("{i}\t{path}\t{e}");
        }
    }
}
