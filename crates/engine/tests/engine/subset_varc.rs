//! `VARC` in a subset: every kept glyph of the fontTools corpus VARC fonts draws the same outline
//! before and after, at the default location and at each axis end. Skipped without `corpus/`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_tables::subset::{SubsetOptions, subset};
use fontcraft_tables::{TableOrder, WriteOptions};
use skrifa::MetadataProvider;
use skrifa::instance::Size;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::types::GlyphId;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// The pen commands, coordinates to a hundredth of a unit.
#[derive(Default)]
struct Commands(Vec<String>);

impl OutlinePen for Commands {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(format!("M{x:.2},{y:.2}"));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(format!("L{x:.2},{y:.2}"));
    }
    fn quad_to(&mut self, x0: f32, y0: f32, x: f32, y: f32) {
        self.0.push(format!("Q{x0:.2},{y0:.2},{x:.2},{y:.2}"));
    }
    fn curve_to(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.push(format!("C{x0:.2},{y0:.2},{x1:.2},{y1:.2},{x:.2},{y:.2}"));
    }
    fn close(&mut self) {
        self.0.push("Z".into());
    }
}

fn draw(bytes: &[u8], gid: u32, location: &[(String, f32)]) -> Vec<String> {
    let font = skrifa::FontRef::new(bytes).unwrap();
    let location = font.axes().location(location.iter().map(|(t, v)| (t.as_str(), *v)));
    let glyph = font.outline_glyphs().get(GlyphId::new(gid)).unwrap_or_else(|| panic!("no outline for glyph {gid}"));
    let mut pen = Commands::default();
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), &location), &mut pen).unwrap();
    pen.0
}

#[test]
fn varc_composites_draw_the_same_after_subsetting() {
    let dir = corpus().join("fonttools/Tests/ttLib/data");
    if !dir.is_dir() {
        eprintln!("skipped: no corpus at {}", dir.display());
        return;
    }
    let unicodes = |u: u32| SubsetOptions { unicodes: vec![u], ..SubsetOptions::default() };
    let gids = |g: u32| SubsetOptions { gids: vec![g], ..SubsetOptions::default() };
    let cases = [
        ("varc-ac00-ac01.ttf", unicodes(0xAC00)),
        ("varc-ac00-ac01.ttf", unicodes(0xAC01)),
        ("varc-ac01-conditional.ttf", unicodes(0xAC01)),
        ("varc-ac01-conditional.ttf", gids(6)),
        ("varc-6868.ttf", unicodes(0x6868)),
        ("varc-6868.ttf", gids(5)),
        ("varc-static-gvar.ttf", unicodes(0x61)),
    ];
    for (name, options) in cases {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let result = subset(&fontcraft_tables::read(&bytes, 0).unwrap(), &options).unwrap();
        assert!(result.font.get(*b"VARC").is_some(), "{name}: the VARC table is kept");
        let out = fontcraft_tables::write(&result.font, &WriteOptions { order: TableOrder::Keep }).unwrap();
        let reference = skrifa::FontRef::new(&bytes).unwrap();
        let mut locations = vec![Vec::new()];
        for axis in reference.axes().iter() {
            for v in [axis.min_value(), axis.max_value()] {
                locations.push(vec![(axis.tag().to_string(), v)]);
            }
        }
        locations.push(reference.axes().iter().map(|a| (a.tag().to_string(), a.max_value())).collect());
        // .notdef is emptied by default.
        let kept: Vec<(u32, u32)> = result.glyph_map.iter().filter(|(old, _)| **old != 0).map(|(o, n)| (*o, *n)).collect();
        assert!(!kept.is_empty(), "{name}: glyphs kept");
        for (old, new) in kept {
            for location in &locations {
                assert_eq!(draw(&out, new, location), draw(&bytes, old, location), "{name}: glyph {old} (now {new}) at {location:?}");
            }
        }
    }
}
