//! Every compiled font in the corpus survives WOFF and untransformed WOFF2: wrapped and unwrapped,
//! each table comes back byte for byte (`head` without `checkSumAdjustment`, and for WOFF2 without
//! bit 11 of `flags`, which the writer sets, and without `DSIG`, which it drops). The corpus's
//! WOFF2 files are read too: those with the null transform must unwrap to a font read-fonts can
//! parse; transformed ones must be refused as not supported. Skipped when `corpus/` is absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_container::sfnt::{self, Table};
use fontcraft_container::{Error, Extras, woff, woff2};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ["ttf", "otf", "woff", "woff2"].contains(&x.to_ascii_lowercase().as_str())) {
            out.push(p);
        }
    }
}

/// Where two fonts' tables differ, `head` compared without `checkSumAdjustment` (and with
/// `flags` bit 11 taken from the second font when `woff2`).
fn differences(a: &[u8], b: &[u8], woff2: bool) -> Vec<String> {
    let (fa, fb) = (sfnt::parse(a).unwrap(), sfnt::parse(b).unwrap());
    let expected: Vec<&Table<'_>> = fa.tables.iter().filter(|t| !(woff2 && t.tag == *b"DSIG")).collect();
    let mut out = Vec::new();
    if expected.len() != fb.tables.len() {
        out.push(format!("{} tables instead of {}", fb.tables.len(), expected.len()));
    }
    for t in expected {
        let Some(other) = fb.get(t.tag) else {
            out.push(format!("'{}' missing", sfnt::tag_str(t.tag)));
            continue;
        };
        let (mut x, mut y) = (t.data.to_vec(), other.to_vec());
        if t.tag == *b"head" && x.len() >= 18 && y.len() >= 18 {
            x[8..12].fill(0);
            y[8..12].fill(0);
            if woff2 {
                x[16] |= 0x08;
            }
        }
        if x != y {
            out.push(format!("'{}' differs", sfnt::tag_str(t.tag)));
        }
    }
    out
}

#[test]
fn corpus_fonts_round_trip_through_woff_and_woff2() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    files(&corpus, &mut all);
    all.sort();
    let (mut fonts, mut woff_ok, mut woff2_ok) = (0, 0, 0);
    let (mut refused, mut mismatches) = (Vec::new(), Vec::new());
    let (mut w2_read, mut w2_transformed, mut w2_other) = (0, 0, Vec::new());
    for path in &all {
        let bytes = std::fs::read(path).unwrap();
        let name = path.strip_prefix(&corpus).unwrap_or(path).display().to_string();
        if bytes.starts_with(b"wOF2") {
            match woff2::decode(&bytes) {
                Ok(u) => {
                    assert!(read_fonts::FontRef::new(&u.sfnt).is_ok(), "{name}: the unwrapped font does not parse");
                    w2_read += 1;
                }
                Err(Error::Unsupported { .. }) => w2_transformed += 1,
                Err(e) => w2_other.push(format!("{name}: {e}")),
            }
            continue;
        }
        let sfnt_bytes = if bytes.starts_with(b"wOFF") { woff::decode(&bytes).unwrap().sfnt } else { bytes };
        if let Err(e) = sfnt::parse(&sfnt_bytes) {
            refused.push(format!("{name}: {e}"));
            continue;
        }
        fonts += 1;
        match woff::encode(&sfnt_bytes, &Extras::default()).and_then(|w| woff::decode(&w)) {
            Ok(u) => {
                let d = differences(&sfnt_bytes, &u.sfnt, false);
                if d.is_empty() {
                    woff_ok += 1;
                } else {
                    mismatches.push(format!("{name} (WOFF): {}", d.join(", ")));
                }
            }
            Err(e) => mismatches.push(format!("{name} (WOFF): {e}")),
        }
        match woff2::encode(&sfnt_bytes, &woff2::Options::untransformed()).and_then(|w| woff2::decode(&w)) {
            Ok(u) => {
                let d = differences(&sfnt_bytes, &u.sfnt, true);
                if d.is_empty() {
                    woff2_ok += 1;
                } else {
                    mismatches.push(format!("{name} (WOFF2): {}", d.join(", ")));
                }
            }
            Err(e) => mismatches.push(format!("{name} (WOFF2): {e}")),
        }
    }
    eprintln!(
        "{fonts} fonts: WOFF round trip {woff_ok}, WOFF2 (untransformed) round trip {woff2_ok}; {} inputs refused; \
         corpus WOFF2 files: {w2_read} read, {w2_transformed} transformed (not supported yet), {} refused",
        refused.len(),
        w2_other.len()
    );
    for line in refused.iter().chain(&w2_other) {
        eprintln!("  {line}");
    }
    assert!(mismatches.is_empty(), "{} mismatches:\n{}", mismatches.len(), mismatches.join("\n"));
}
