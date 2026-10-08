//! The comparison set: the corpus sources the harness runs over with `--corpus`.
//!
//! fontc's test data (every `.glyphs`, `.glyphspackage`, `.designspace` and `.ufo` under
//! `fontc/resources/testdata`, one compiler behaviour each) and Oswald's main source. The corpus
//! is fetched by `cargo xtask corpus --fetch`; nothing here downloads.

use std::path::{Path, PathBuf};

use crate::tools::SourceKind;

/// The folders and files of the comparison set, relative to the corpus folder.
pub const SET: &[&str] = &["fontc/resources/testdata", "oswald/sources/Oswald.glyphs"];
/// Deepest folder level searched below a set folder.
const MAX_DEPTH: usize = 12;
/// Most folder entries looked at in one walk.
const MAX_ENTRIES: usize = 200_000;

/// The comparison set's sources under `corpus`, sorted. Missing parts are listed in the error
/// when nothing at all is found.
pub fn discover(corpus: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    for part in SET {
        let path = corpus.join(part);
        if path.is_file() || (path.is_dir() && SourceKind::of(&path).is_some()) {
            out.push(path);
        } else if path.is_dir() {
            walk(&path, 0, &mut out, &mut seen);
        }
    }
    if out.is_empty() {
        return Err(format!(
            "no comparison sources under {} (expected {}); fetch the corpus with `cargo xtask corpus --fetch`",
            corpus.display(),
            SET.join(", ")
        ));
    }
    out.sort();
    Ok(out)
}

/// Sources under `dir`. A source that is a folder (UFO, package) is not searched inside.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, seen: &mut usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        *seen += 1;
        if *seen > MAX_ENTRIES {
            return;
        }
        paths.push(entry.path());
    }
    paths.sort();
    for path in paths {
        let is_dir = path.is_dir();
        match SourceKind::of(&path) {
            Some(SourceKind::Ufo | SourceKind::GlyphsPackage) if is_dir => out.push(path),
            Some(SourceKind::Glyphs | SourceKind::Designspace) if !is_dir => out.push(path),
            _ if is_dir => walk(&path, depth + 1, out, seen),
            _ => {}
        }
    }
}
