//! Wraps fonts for the cross-check (crates/container/crosscheck/crosscheck.py): for the n-th
//! file named, writes `OUT_DIR/n.woff` (WOFF 1.0), `OUT_DIR/n.woff2` (WOFF2, every table
//! untransformed), `OUT_DIR/n.t.woff2` (WOFF2 with the default options: `glyf` and `loca`
//! transformed) and `OUT_DIR/n.h.woff2` (WOFF2 with the `hmtx` transform too). A WOFF or WOFF2
//! input is unwrapped first. Errors go to stderr, one line per file.
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
    let with_hmtx = woff2::Options { transform_hmtx: true, ..woff2::Options::default() };
    for (i, path) in args.enumerate() {
        let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
            let sfnt = to_sfnt(&bytes).map_err(|e| e.to_string())?;
            let files = [
                ("woff", woff::encode(&sfnt, &Extras::default())),
                ("woff2", woff2::encode(&sfnt, &woff2::Options::untransformed())),
                ("t.woff2", woff2::encode(&sfnt, &woff2::Options::default())),
                ("h.woff2", woff2::encode(&sfnt, &with_hmtx)),
            ];
            for (ext, data) in files {
                std::fs::write(out.join(format!("{i}.{ext}")), data.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        if let Err(e) = result {
            eprintln!("{i}\t{path}\t{e}");
        }
    }
}
