//! Import Kerning from Metrics File (`file.importMetricsFile`): PFM files built here and an AFM
//! text, both merge modes, several masters, scaling to the font's units per em, one undo step,
//! and hostile files that change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Layer};
use fontcraft_spacing::pfm;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-metrics-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Masters `m01` and `m02`; the default glyphs plus `quotedblleft` (U+201C). Pairs of m01:
/// A–V −50, T–o −20.
fn font(units_per_em: u16) -> Font {
    let mut f = Font::new("Metrics Files Test");
    f.units_per_em = units_per_em;
    f.add_glyph("quotedblleft", vec![0x201C], 500.0).unwrap();
    let first = f.masters[0].id.clone();
    let mut second = f.masters[0].clone();
    second.id = "m02".into();
    second.name = "Bold".into();
    Arc::make_mut(&mut f.masters).push(second);
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let width = g.layers[0].width;
        g.layers.push(Layer::new("m02", width));
    }
    let k = &mut Arc::make_mut(&mut f.kerning).ltr;
    k.set(&first, "A", "V", -50.0);
    k.set(&first, "T", "o", -20.0);
    f
}

fn session(units_per_em: u16) -> Session {
    let mut s = Session::new();
    s.add_document(font(units_per_em), None).unwrap();
    s
}

/// The stored value of the glyph pair `left`–`right` of `master`.
fn pair(s: &Session, master: &str, left: &str, right: &str) -> Option<f64> {
    let f = s.font().unwrap();
    let m = f.kerning.ltr.0.iter().find(|m| m.master == master)?;
    m.firsts.iter().filter(|k| k.first == left).flat_map(|k| k.seconds.iter()).find(|(r, _)| r == right).map(|(_, v)| *v)
}

/// A PFM file as PostScript printer drivers expect it: header (ANSI character set, codes
/// 32–255), extension, extended metrics (1,000 units per em, `pairs.len()` pairs), and the
/// pairs after a count word.
fn pfm_file(pairs: &[(u8, u8, i16)]) -> Vec<u8> {
    let mut d = vec![0u8; pfm::HEADER_LEN];
    d[0..2].copy_from_slice(&0x0100u16.to_le_bytes());
    d[85] = pfm::CHARSET_ANSI;
    d[95] = 32;
    d[96] = 255;
    let ext = d.len();
    d.extend_from_slice(&[0u8; pfm::EXTENSION_LEN]);
    d[ext..ext + 2].copy_from_slice(&(pfm::EXTENSION_LEN as u16).to_le_bytes());
    let etm = d.len();
    d.extend_from_slice(&[0u8; pfm::EXT_METRICS_LEN]);
    d[etm + 12..etm + 14].copy_from_slice(&1000u16.to_le_bytes());
    d[etm + 48..etm + 50].copy_from_slice(&(pairs.len() as u16).to_le_bytes());
    let kern = d.len();
    d.extend_from_slice(&(pairs.len() as u16).to_le_bytes());
    for &(a, b, v) in pairs {
        d.extend_from_slice(&[a, b]);
        d.extend_from_slice(&v.to_le_bytes());
    }
    for (i, at) in [(0, etm), (3, kern)] {
        d[ext + 2 + 4 * i..ext + 6 + 4 * i].copy_from_slice(&(at as u32).to_le_bytes());
    }
    d
}

#[test]
fn import_metrics_file_merges_pfm_pairs_in_both_modes() {
    let dir = temp("modes");
    let file = dir.join("Test.pfm");
    // 0x93 is U+201C in Windows-1252; 0xC0 (Agrave) has no glyph.
    std::fs::write(&file, pfm_file(&[(b'A', b'V', -80), (b'T', b'o', -20), (0x93, b'A', -30), (0xC0, b'V', -15)])).unwrap();
    let mut s = session(1000);
    let m = s.font().unwrap().masters[0].id.clone();

    let r = run(&mut s, "file.importMetricsFile", json!({"path": file, "merge": "addMissing"}));
    assert_eq!(r["format"], "pfm");
    assert_eq!(r["pairs"], 4);
    assert_eq!(r["imported"], 3);
    assert_eq!(r["unknownCodes"], json!([0xC0]));
    assert_eq!(r["masters"], json!([{"master": m, "added": 1, "replaced": 0, "unchanged": 1, "kept": 1}]));
    assert_eq!(pair(&s, &m, "A", "V"), Some(-50.0), "addMissing keeps the master's value");
    assert_eq!(pair(&s, &m, "quotedblleft", "A"), Some(-30.0));

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(pair(&s, &m, "quotedblleft", "A"), None, "the import is one undo step");

    let r = run(&mut s, "file.importMetricsFile", json!({"path": file}));
    assert_eq!(r["merge"], "replace");
    assert_eq!(r["masters"], json!([{"master": m, "added": 1, "replaced": 1, "unchanged": 1, "kept": 0}]));
    assert_eq!(pair(&s, &m, "A", "V"), Some(-80.0), "replace takes the file's value");
    assert_eq!(pair(&s, "m02", "A", "V"), None, "only the first master by default");
}

#[test]
fn import_metrics_file_scales_pfm_and_reads_afm_into_several_masters() {
    let dir = temp("scale");
    let file = dir.join("Test.pfm");
    std::fs::write(&file, pfm_file(&[(b'A', b'V', -80)])).unwrap();
    let mut s = session(2048);
    let m = s.font().unwrap().masters[0].id.clone();
    run(&mut s, "file.importMetricsFile", json!({"path": file}));
    // −80 × 2048 / 1000 = −163.84, rounded half up.
    assert_eq!(pair(&s, &m, "A", "V"), Some(-164.0));

    let afm = dir.join("Test.afm");
    std::fs::write(
        &afm,
        "StartFontMetrics 4.1\nFontName TestSans\nStartKernData\nStartKernPairs 3\nKPX A V -75\nKPX L T -40\nKPX A Missing -10\nEndKernPairs\nEndKernData\nEndFontMetrics\n",
    )
    .unwrap();
    let r = run(&mut s, "file.importMetricsFile", json!({"path": afm, "masters": [m, "m02"]}));
    assert_eq!(r["format"], "afm");
    assert_eq!(r["face"], "TestSans");
    assert_eq!(r["imported"], 2);
    assert_eq!(r["unknownGlyphs"], json!(["Missing"]));
    for master in [m.as_str(), "m02"] {
        assert_eq!(pair(&s, master, "A", "V"), Some(-75.0), "{master}");
        assert_eq!(pair(&s, master, "L", "T"), Some(-40.0), "{master}");
    }
    assert_eq!(pair(&s, "m02", "A", "Missing"), None, "a pair naming a missing glyph is left out");
}

#[test]
fn import_metrics_file_refuses_hostile_files_and_changes_nothing() {
    let dir = temp("hostile");
    let mut s = session(1000);
    let id = "file.importMetricsFile";

    // The pair count points past the end of the file.
    let mut past = pfm_file(&[(b'A', b'V', -80)]);
    let etm = pfm::HEADER_LEN + pfm::EXTENSION_LEN;
    past[etm + 48..etm + 50].copy_from_slice(&40_000u16.to_le_bytes());
    let file = dir.join("past.pfm");
    std::fs::write(&file, &past).unwrap();
    let e = refused(&mut s, id, json!({"path": file}));
    assert!(e.contains("past the end"), "{e}");

    // The extension's pair-table offset is far beyond the file.
    let mut far = pfm_file(&[(b'A', b'V', -80)]);
    let at = pfm::HEADER_LEN + 2 + 4 * 3;
    far[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&file, &far).unwrap();
    refused(&mut s, id, json!({"path": file}));

    // Larger than a PFM can be.
    let mut big = pfm_file(&[]);
    big.resize(pfm::MAX_BYTES + 1, 0);
    std::fs::write(&file, &big).unwrap();
    let e = refused(&mut s, id, json!({"path": file}));
    assert!(e.contains("limit"), "{e}");

    // Neither format, a missing file, bad parameters.
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, [0xFFu8; 300]).unwrap();
    let e = refused(&mut s, id, json!({"path": junk}));
    assert!(e.contains("neither"), "{e}");
    refused(&mut s, id, json!({"path": dir.join("absent.pfm")}));
    refused(&mut s, id, json!({}));
    std::fs::write(&file, pfm_file(&[(b'A', b'V', -80)])).unwrap();
    refused(&mut s, id, json!({"path": file, "merge": "sometimes"}));
    refused(&mut s, id, json!({"path": file, "masters": ["nope"]}));
}
