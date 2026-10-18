//! The import measure of `docs/plans/M10.md` § 5 over the corpus fonts:
//!
//! - every CFF font with charstring hints opens with, on each glyph's layer, as many stem hints
//!   as the charstring has distinct stems (at most 96);
//! - every TrueType font with glyph programs re-exports with byte-equal `cvt `, `fpgm`, `prep`
//!   and glyph programs.
//!
//! It needs the corpus (`cargo xtask corpus --fetch`) and compiles every hinted TrueType font,
//! so it is left out of the normal test run:
//! `cargo test -p fontcraft-decompile --release --test corpus_import -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile_ttf};
use fontcraft_decompile::decompile;
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::cs::CommandSink;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::{Fixed, GlyphId, Tag};
use skrifa::raw::{FileRef, FontRef, TableProvider};

const MAX_STEMS: usize = 96;

fn fonts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.is_dir() && depth < 32 {
            fonts(&path, depth + 1, out);
        } else if meta.is_file()
            && path.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf"))
        {
            out.push(path);
        }
    }
}

/// A glyph's distinct stems, keyed as the importer keys them: ghost hints by their edge,
/// stems by their lower edge and positive width.
#[derive(Default)]
struct Stems(HashSet<(bool, u8, u64, u64)>);

impl Stems {
    fn add(&mut self, horizontal: bool, u: Fixed, v: Fixed) {
        let (u, v) = (u.to_f64(), v.to_f64());
        let d = v - u;
        let key = if horizontal && d == -20.0 {
            (1, u, 0.0)
        } else if horizontal && d == -21.0 {
            (2, v, 0.0)
        } else if d < 0.0 {
            (0, v, -d)
        } else {
            (0, u, d)
        };
        self.0.insert((horizontal, key.0, key.1.to_bits(), key.2.to_bits()));
    }
}

impl CommandSink for Stems {
    fn move_to(&mut self, _: Fixed, _: Fixed) {}
    fn line_to(&mut self, _: Fixed, _: Fixed) {}
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed) {}
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, v: Fixed) {
        self.add(true, y, v);
    }
    fn vstem(&mut self, x: Fixed, v: Fixed) {
        self.add(false, x, v);
    }
}

/// The distinct stems of every glyph of a CFF font; `None` when it has none.
fn charstring_stems(font: &FontRef) -> Option<Vec<usize>> {
    let table = font.table_data(Tag::new(b"CFF "))?;
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).ok()?;
    let mut counts = Vec::new();
    for gid in 0..cff.num_glyphs() {
        let g = GlyphId::new(gid);
        let mut sink = Stems::default();
        if let Some(fd) = cff.subfont_index(g)
            && let Ok(sub) = cff.subfont(fd, &[])
        {
            let _ = cff.evaluate_charstring(&sub, g, &[], &mut sink);
        }
        counts.push(sink.0.len().min(MAX_STEMS));
    }
    counts.iter().any(|n| *n > 0).then_some(counts)
}

/// Each glyph's program, by glyph id; `None` when no glyph has one.
fn glyph_programs(font: &FontRef) -> Option<Vec<Vec<u8>>> {
    let (glyf, loca) = (font.glyf().ok()?, font.loca(None).ok()?);
    let count = font.maxp().ok()?.num_glyphs();
    let programs: Vec<Vec<u8>> = (0..u32::from(count))
        .map(|gid| match loca.get_glyf(GlyphId::new(gid), &glyf) {
            Ok(Some(RawGlyph::Simple(s))) => s.instructions().to_vec(),
            Ok(Some(RawGlyph::Composite(c))) => c.instructions().unwrap_or_default().to_vec(),
            _ => Vec::new(),
        })
        .collect();
    programs.iter().any(|p| !p.is_empty()).then_some(programs)
}

fn table(font: &FontRef, tag: &[u8; 4]) -> Vec<u8> {
    font.table_data(Tag::new(tag)).map(|d| d.as_bytes().to_vec()).unwrap_or_default()
}

/// What differs between an original CFF font and its opened document, or `None` when nothing.
fn check_cff(bytes: &[u8], counts: &[usize]) -> Option<String> {
    let d = match decompile(bytes, None) {
        Ok(d) => d,
        Err(e) => return Some(format!("does not open: {e}")),
    };
    let wrong: Vec<String> = counts
        .iter()
        .enumerate()
        .filter_map(|(gid, want)| {
            let got = d.font.glyphs.as_arcs().get(gid).and_then(|g| g.layers.first()).map_or(0, |l| l.info.hints.len());
            (got != *want).then(|| format!("gid {gid}: {got} hints, {want} stems"))
        })
        .collect();
    (!wrong.is_empty()).then(|| format!("{} glyphs differ, first: {}", wrong.len(), wrong.iter().take(3).cloned().collect::<Vec<_>>().join("; ")))
}

/// What differs between an original TrueType font and the export of its opened document.
fn check_tt(bytes: &[u8], original: &FontRef, programs: &[Vec<u8>]) -> Option<String> {
    let d = match decompile(bytes, None) {
        Ok(d) => d,
        Err(e) => return Some(format!("does not open: {e}")),
    };
    let out = match compile_ttf(&d.font, &Options::default()) {
        Ok(o) => o,
        Err(e) => return Some(format!("does not export: {e}")),
    };
    let again = FontRef::new(&out.bytes).unwrap();
    let mut problems = Vec::new();
    for tag in [b"cvt ", b"fpgm", b"prep"] {
        if table(original, tag) != table(&again, tag) {
            problems.push(format!("{} differs", String::from_utf8_lossy(tag)));
        }
    }
    let Some(exported) = glyph_programs(&again) else {
        return Some(format!("the export has no glyph programs ({})", problems.join(", ")));
    };
    let mut differing = Vec::new();
    for (gid, program) in programs.iter().enumerate() {
        let Some(name) = d.font.glyphs.as_arcs().get(gid).map(|g| g.name.clone()) else { continue };
        let got = out.glyph_order.iter().position(|n| *n == name).and_then(|i| exported.get(i));
        if got != Some(program) {
            differing.push(name);
        }
    }
    if !differing.is_empty() {
        problems.push(format!(
            "{} glyph programs differ, first: {}",
            differing.len(),
            differing.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    (!problems.is_empty()).then(|| problems.join("; "))
}

#[test]
#[ignore = "needs the corpus and compiles every hinted TrueType font"]
fn corpus_hinted_fonts_open_with_their_hinting() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !root.exists() {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    let mut files = Vec::new();
    fonts(&root, 0, &mut files);
    files.sort();
    let (mut cff_total, mut tt_total) = (0usize, 0usize);
    let mut failures = Vec::new();
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(FileRef::Font(font)) = FileRef::new(&bytes) else { continue };
        let name = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        if let Some(counts) = charstring_stems(&font) {
            cff_total += 1;
            if let Some(why) = check_cff(&bytes, &counts) {
                failures.push(format!("CFF {name}: {why}"));
            }
        }
        if let Some(programs) = glyph_programs(&font) {
            tt_total += 1;
            if let Some(why) = check_tt(&bytes, &font, &programs) {
                failures.push(format!("TrueType {name}: {why}"));
            }
        }
    }
    println!("CFF fonts with charstring hints: {cff_total}; TrueType fonts with glyph programs: {tt_total}; failing: {}", failures.len());
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "{} of {} hinted fonts fail the import measure", failures.len(), cff_total + tt_total);
}
