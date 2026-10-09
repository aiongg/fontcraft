//! Every compiled font in the corpus survives WOFF and WOFF2: wrapped and unwrapped, each table
//! comes back byte for byte (`head` without `checkSumAdjustment`, and for WOFF2 without bit 11 of
//! `flags`, which the writer sets, and without `DSIG`, which it drops). WOFF2 is tried untransformed,
//! with the default options (`glyf` and `loca` transformed) and with the `hmtx` transform too. A
//! transformed `glyf` comes back with other padding and other flag and coordinate packing, so
//! `glyf` and `loca` are compared glyph by glyph as read-fonts reads them, and `head` also without
//! `indexToLocFormat`. Every WOFF2 file in the corpus must unwrap to a font read-fonts can parse.
//! Skipped when `corpus/` is absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_container::sfnt::{self, Table};
use fontcraft_container::{Extras, woff, woff2};
use read_fonts::tables::glyf::Glyph;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};

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

/// How the two fonts are compared.
#[derive(Clone, Copy, PartialEq)]
enum Compare {
    Woff,
    /// WOFF2 with every table stored as it is.
    Woff2,
    /// WOFF2 with transformed tables: `glyf` and `loca` glyph by glyph, `indexToLocFormat` ignored.
    Transformed,
}

/// Every glyph's outline as read-fonts reads it: bounding box, contour ends, instructions, points
/// and the overlap flag of simple glyphs; bounding box, components (with the flags a writer keeps)
/// and instructions of composites. A glyph without contours reads as empty.
fn outlines(sfnt_bytes: &[u8]) -> Vec<String> {
    let font = FontRef::new(sfnt_bytes).unwrap();
    let (Ok(loca), Ok(glyf)) = (font.loca(None), font.glyf()) else { return vec!["no outlines".into()] };
    (0..loca.len())
        .map(|i| match loca.get_glyf(GlyphId::new(i as u32), &glyf) {
            Ok(None) => "empty".into(),
            Ok(Some(Glyph::Simple(g))) if g.number_of_contours() == 0 => "empty".into(),
            Ok(Some(Glyph::Simple(g))) => format!(
                "{:?} {:?} {:?} {:?} {}",
                [g.x_min(), g.y_min(), g.x_max(), g.y_max()],
                g.end_pts_of_contours().iter().map(|e| e.get()).collect::<Vec<_>>(),
                g.instructions(),
                g.points().collect::<Vec<_>>(),
                g.has_overlapping_contours()
            ),
            Ok(Some(Glyph::Composite(g))) => format!(
                "{:?} {:?} {:?}",
                [g.x_min(), g.y_min(), g.x_max(), g.y_max()],
                g.components().map(|c| (c.flags.bits() & 0x1e14, c.glyph, c.anchor, c.transform)).collect::<Vec<_>>(),
                g.instructions()
            ),
            Err(e) => format!("unreadable: {e}"),
        })
        .collect()
}

/// Where two fonts' tables differ, `head` compared without `checkSumAdjustment` (and for WOFF2
/// with `flags` bit 11 taken from the second font).
fn differences(a: &[u8], b: &[u8], how: Compare) -> Vec<String> {
    let (fa, fb) = (sfnt::parse(a).unwrap(), sfnt::parse(b).unwrap());
    let expected: Vec<&Table<'_>> = fa.tables.iter().filter(|t| !(how != Compare::Woff && t.tag == *b"DSIG")).collect();
    let mut out = Vec::new();
    if expected.len() != fb.tables.len() {
        out.push(format!("{} tables instead of {}", fb.tables.len(), expected.len()));
    }
    for t in expected {
        let Some(other) = fb.get(t.tag) else {
            out.push(format!("'{}' missing", sfnt::tag_str(t.tag)));
            continue;
        };
        if how == Compare::Transformed && (t.tag == *b"glyf" || t.tag == *b"loca") {
            if t.tag == *b"glyf" && outlines(a) != outlines(b) {
                out.push("'glyf' outlines differ".into());
            }
            continue;
        }
        let (mut x, mut y) = (t.data.to_vec(), other.to_vec());
        if t.tag == *b"head" && x.len() >= 54 && y.len() >= 54 {
            x[8..12].fill(0);
            y[8..12].fill(0);
            if how != Compare::Woff {
                x[16] |= 0x08;
            }
            if how == Compare::Transformed {
                x[50..52].fill(0);
                y[50..52].fill(0);
            }
        }
        if x != y {
            out.push(format!("'{}' differs", sfnt::tag_str(t.tag)));
        }
    }
    out
}

/// What one corpus file gave.
#[derive(Default)]
struct Tally {
    fonts: usize,
    /// Round trips with equal tables: WOFF, WOFF2 untransformed, WOFF2, WOFF2 with `hmtx`.
    ok: [usize; 4],
    glyf_transformed: usize,
    /// TrueType fonts whose `glyf` the writer leaves untransformed.
    glyf_kept: Vec<String>,
    hmtx_transformed: usize,
    woff2_read: usize,
    woff2_transformed: usize,
    refused: Vec<String>,
    woff2_refused: Vec<String>,
    mismatches: Vec<String>,
}

impl Tally {
    fn add(&mut self, o: Tally) {
        self.fonts += o.fonts;
        for (a, b) in self.ok.iter_mut().zip(o.ok) {
            *a += b;
        }
        self.glyf_transformed += o.glyf_transformed;
        self.glyf_kept.extend(o.glyf_kept);
        self.hmtx_transformed += o.hmtx_transformed;
        self.woff2_read += o.woff2_read;
        self.woff2_transformed += o.woff2_transformed;
        self.refused.extend(o.refused);
        self.woff2_refused.extend(o.woff2_refused);
        self.mismatches.extend(o.mismatches);
    }
}

fn transformed(w: &[u8], tag: &[u8; 4]) -> bool {
    woff2::directory(w).unwrap().tables.iter().any(|t| &t.tag == tag && t.transformed())
}

fn check(path: &Path, name: String, t: &mut Tally) {
    let bytes = std::fs::read(path).unwrap();
    if bytes.starts_with(b"wOF2") {
        match woff2::decode(&bytes) {
            Ok(u) => {
                assert!(read_fonts::FontRef::new(&u.sfnt).is_ok(), "{name}: the unwrapped font does not parse");
                t.woff2_read += 1;
                t.woff2_transformed += usize::from(woff2::directory(&bytes).unwrap().tables.iter().any(|t| t.transformed()));
            }
            Err(e) => t.woff2_refused.push(format!("{name}: {e}")),
        }
        return;
    }
    let sfnt_bytes = if bytes.starts_with(b"wOFF") { woff::decode(&bytes).unwrap().sfnt } else { bytes };
    if let Err(e) = sfnt::parse(&sfnt_bytes) {
        t.refused.push(format!("{name}: {e}"));
        return;
    }
    t.fonts += 1;
    let with_hmtx = woff2::Options { transform_hmtx: true, ..woff2::Options::default() };
    let runs: [(&str, Compare, Result<Vec<u8>, _>); 4] = [
        ("WOFF", Compare::Woff, woff::encode(&sfnt_bytes, &Extras::default())),
        ("WOFF2 untransformed", Compare::Woff2, woff2::encode(&sfnt_bytes, &woff2::Options::untransformed())),
        ("WOFF2", Compare::Transformed, woff2::encode(&sfnt_bytes, &woff2::Options::default())),
        ("WOFF2 with hmtx", Compare::Transformed, woff2::encode(&sfnt_bytes, &with_hmtx)),
    ];
    for (i, (label, how, wrapped)) in runs.into_iter().enumerate() {
        if let Ok(w) = &wrapped {
            match i {
                2 if transformed(w, b"glyf") => t.glyf_transformed += 1,
                2 if sfnt::parse(&sfnt_bytes).unwrap().get(*b"glyf").is_some() => t.glyf_kept.push(name.clone()),
                3 => t.hmtx_transformed += usize::from(transformed(w, b"hmtx")),
                _ => {}
            }
        }
        let back = wrapped.and_then(|w| if how == Compare::Woff { woff::decode(&w) } else { woff2::decode(&w) });
        match back {
            Ok(u) => {
                let d = differences(&sfnt_bytes, &u.sfnt, how);
                if d.is_empty() {
                    t.ok[i] += 1;
                } else {
                    t.mismatches.push(format!("{name} ({label}): {}", d.join(", ")));
                }
            }
            Err(e) => t.mismatches.push(format!("{name} ({label}): {e}")),
        }
    }
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
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(16);
    let mut total = Tally::default();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|k| {
                let (all, corpus) = (&all, &corpus);
                scope.spawn(move || {
                    let mut t = Tally::default();
                    for path in all.iter().skip(k).step_by(threads) {
                        check(path, path.strip_prefix(corpus).unwrap_or(path).display().to_string(), &mut t);
                    }
                    t
                })
            })
            .collect();
        for w in workers {
            total.add(w.join().unwrap());
        }
    });
    let t = total;
    eprintln!(
        "{} fonts: round trips WOFF {}, WOFF2 untransformed {}, WOFF2 {} ({} with glyf transformed), WOFF2 with hmtx {} \
         ({} with hmtx transformed); {} inputs refused; corpus WOFF2 files: {} read ({} transformed), {} refused",
        t.fonts,
        t.ok[0],
        t.ok[1],
        t.ok[2],
        t.glyf_transformed,
        t.ok[3],
        t.hmtx_transformed,
        t.refused.len(),
        t.woff2_read,
        t.woff2_transformed,
        t.woff2_refused.len()
    );
    for line in &t.glyf_kept {
        eprintln!("  glyf left untransformed: {line}");
    }
    for line in t.refused.iter().chain(&t.woff2_refused) {
        eprintln!("  {line}");
    }
    assert!(t.woff2_refused.is_empty(), "corpus WOFF2 files refused:\n{}", t.woff2_refused.join("\n"));
    assert!(t.mismatches.is_empty(), "{} mismatches:\n{}", t.mismatches.len(), t.mismatches.join("\n"));
}
