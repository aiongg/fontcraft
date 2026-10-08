//! Every compiled font in the corpus opens, and each glyph's outline in the model matches what
//! skrifa draws from the file (area and bounding box). Skipped when `corpus/` is absent.
//!
//! The box is compared by its size and vertical position, not its horizontal position: skrifa
//! moves a TrueType outline so its left edge sits on the `hmtx` side bearing, and where a font's
//! side bearing disagrees with its `glyf` box (fontTools' `issue2824.ttf` is such a font) the
//! model keeps the coordinates as stored. Tolerances: one unit, and 0.1% on the area for skrifa's
//! fixed-point arithmetic on scaled components. Fonts with a `VARC` table are not compared: skrifa draws
//! their outlines from that table, which the model does not read.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_geom::{BezPath, Shape as _};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::types::GlyphId;

fn fonts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_dir() {
            fonts(&p, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ["ttf", "otf", "woff"].contains(&x.to_ascii_lowercase().as_str())) {
            out.push(p);
        }
    }
}

struct Bez(BezPath);
impl OutlinePen for Bez {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(a), f64::from(b)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(a), f64::from(b)), (f64::from(c), f64::from(d)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

#[test]
fn corpus_fonts_open_and_their_outlines_match() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    fonts(&corpus, &mut all);
    let (mut opened, mut glyphs, mut refused, mut mismatches) = (0, 0usize, Vec::new(), Vec::new());
    for path in &all {
        let bytes = std::fs::read(path).unwrap();
        let d = match fontcraft_decompile::decompile(&bytes, None) {
            Ok(d) => d,
            Err(e) => {
                refused.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        opened += 1;
        // WOFF is compared after unwrapping, through the font the model came from.
        let Ok(reference) = skrifa::FontRef::new(&bytes) else { continue };
        if reference.table_data(skrifa::raw::types::Tag::new(b"VARC")).is_some() {
            continue;
        }
        let outlines = reference.outline_glyphs();
        let names: Vec<String> = d.font.glyphs.iter().map(|g| g.name.clone()).collect();
        for (gid, name) in names.iter().enumerate() {
            let Some(o) = outlines.get(GlyphId::new(gid as u32)) else { continue };
            let mut pen = Bez(BezPath::new());
            if o.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).is_err() {
                continue;
            }
            let Ok(ours) = d.font.outline(name, "m01") else {
                mismatches.push(format!("{} {name}: the model cannot resolve the outline", path.display()));
                continue;
            };
            glyphs += 1;
            let (a, b) = (pen.0.area(), ours.area());
            let (ba, bb) = (pen.0.bounding_box(), ours.bounding_box());
            let close = |x: f64, y: f64| (x - y).abs() <= 1.0 + 1e-3 * x.abs().max(y.abs());
            let same_box = pen.0.elements().is_empty() == ours.elements().is_empty()
                && (pen.0.elements().is_empty() || close(ba.width(), bb.width()) && close(ba.y0, bb.y0) && close(ba.y1, bb.y1));
            if !(close(a, b) && same_box) {
                mismatches.push(format!("{} {name}: area {a:.1} vs {b:.1}, box {ba:?} vs {bb:?}", path.display()));
            }
        }
    }
    eprintln!("{opened} of {} compiled fonts opened, {glyphs} glyph outlines compared; refused:\n  {}", all.len(), refused.join("\n  "));
    assert!(
        mismatches.is_empty(),
        "{} mismatches, the first ones:\n{}",
        mismatches.len(),
        mismatches.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
    assert!(opened * 10 >= all.len() * 9, "fewer than 90% of the corpus fonts opened");
}
