//! Unwraps WOFF and WOFF2 files for the cross-check (crates/container/crosscheck/crosscheck.py):
//! for the n-th file named, writes the sfnt inside to `OUT_DIR/n.sfnt`. Errors go to stderr, one
//! line per file.
//!
//!     cargo run -p fontcraft-container --example unwrap -- OUT_DIR FILE...

use std::path::PathBuf;

use fontcraft_container::to_sfnt;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: unwrap OUT_DIR FILE...");
        std::process::exit(2);
    };
    for (i, path) in args.enumerate() {
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| to_sfnt(&bytes).map(|s| s.into_owned()).map_err(|e| e.to_string()))
            .and_then(|sfnt| std::fs::write(out.join(format!("{i}.sfnt")), sfnt).map_err(|e| e.to_string()));
        if let Err(e) = result {
            eprintln!("{i}\t{path}\t{e}");
        }
    }
}
