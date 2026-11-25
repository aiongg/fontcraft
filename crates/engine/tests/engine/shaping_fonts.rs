//! First step of the shaping measure (M8 plan § 5, shaping against HarfBuzz): writes the fonts
//! FontCraft shapes with, and the sample lines of their sources, to a folder. The second step is
//! `crates/shaping/crosscheck/harfbuzz.py`, the third `crates/shaping/tests/harfbuzz.rs`.
//!
//! Per source `NAME`:
//! - `NAME.full.ttf`: the exported font (`compile_ttf`), outlines included;
//! - `NAME.text.ttf`: Text Preview's font (`GSUB` and `GPOS`, generated kerning and marks);
//! - `NAME.edit.ttf`: Edit View's font (`GSUB` only);
//! - `NAME.text.pua`, `NAME.edit.pua`: one `code point in hex, tab, glyph id` line per glyph
//!   without a code point;
//! - `NAME.txt`: the source's display strings as characters, one line each, a glyph typed by
//!   name as its code point or private-use code point.
//!
//! `FONTCRAFT_HB_DIR=<folder> cargo test -p fontcraft-engine --test engine shaping_fonts:: --
//! --ignored --nocapture`; `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's
//! `corpus/`), `FONTCRAFT_HB_SOURCES` other sources, separated by `:`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use fontcraft_engine::compile::preview::{PreviewFont, layout_fonts_with};
use fontcraft_engine::compile::{Options, compile_ttf};
use fontcraft_engine::model::Font;
use fontcraft_textlayout::{Item, MAX_ITEMS, parse};

const SOURCES: &[&str] = &[
    "inter/src/Inter-Roman.glyphspackage",
    "oswald/sources/Oswald.glyphs",
    "oswald/sources/Italic_WIP/Oswald-Italic.glyphs",
    "markow/sources/Markow.glyphs",
    "glyphspack/tests/fixtures/Andika Mini 4.glyphs",
    "fontc/resources/testdata/glyphs3/PropagateAnchorsTest.glyphs",
];

/// The characters of `line`, or `None` when an item has none in the preview font.
fn characters(font: &Font, preview: &PreviewFont, line: &str) -> Option<String> {
    let gids: HashMap<&str, u16> = (0u16..).zip(&preview.glyph_order).map(|(gid, name)| (name.as_str(), gid)).collect();
    let private: HashMap<u16, u32> = preview.private_use.iter().map(|(cp, gid)| (*gid, *cp)).collect();
    let mut out = String::new();
    for item in parse(font, line, MAX_ITEMS).ok()? {
        match item {
            Item::Glyph { name, typed } => {
                let gid = gids.get(&*name)?;
                let own = private.get(gid).copied().and_then(char::from_u32);
                out.push(own.or(typed).or_else(|| font.glyph(&name)?.unicodes.first().copied().and_then(char::from_u32))?);
            }
            Item::Missing(c) => out.push(c),
            Item::Placeholder => return None,
            Item::LineBreak => break,
        }
    }
    (!out.is_empty()).then_some(out)
}

fn write_pua(path: &Path, font: &PreviewFont) {
    let mut text = String::new();
    for (cp, gid) in &font.private_use {
        writeln!(text, "{cp:X}\t{gid}").unwrap();
    }
    std::fs::write(path, text).unwrap();
}

#[test]
#[ignore = "first step of the shaping measure: writes fonts to FONTCRAFT_HB_DIR"]
fn write_the_fonts_and_sample_lines() {
    let Some(dir) = std::env::var_os("FONTCRAFT_HB_DIR").map(PathBuf::from) else {
        eprintln!("skipped: set FONTCRAFT_HB_DIR to the folder to write to");
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let corpus = std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    let sources: Vec<PathBuf> = match std::env::var("FONTCRAFT_HB_SOURCES") {
        Ok(list) => list.split(':').map(PathBuf::from).collect(),
        Err(_) => SOURCES.iter().map(|s| corpus.join(s)).collect(),
    };
    let mut written = 0;
    for source in sources {
        let Ok((font, _)) = fontcraft_engine::io::open(&source, None) else {
            eprintln!("skipped: {} cannot be read", source.display());
            continue;
        };
        let Some(master) = fontcraft_engine::model::axes::origin(&font).or_else(|| font.masters.first()).map(|m| m.id.clone()) else { continue };
        let name = source.file_stem().unwrap().to_string_lossy().replace(' ', "");
        let build = match layout_fonts_with(&font, &master, &Options::default()) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{name}: the preview fonts do not build: {e}");
                continue;
            }
        };
        std::fs::write(dir.join(format!("{name}.text.ttf")), &build.full.bytes).unwrap();
        std::fs::write(dir.join(format!("{name}.edit.ttf")), &build.gsub_only.bytes).unwrap();
        write_pua(&dir.join(format!("{name}.text.pua")), &build.full);
        write_pua(&dir.join(format!("{name}.edit.pua")), &build.gsub_only);
        match compile_ttf(&font, &Options::default()) {
            Ok(c) => {
                assert_eq!(c.glyph_order, build.full.glyph_order, "{name}: the preview font has the export's glyph order");
                std::fs::write(dir.join(format!("{name}.full.ttf")), &c.bytes).unwrap();
            }
            Err(e) => eprintln!("{name}: the export does not build: {e}"),
        }
        let lines: Vec<String> =
            font.display_strings.iter().flat_map(|s| s.lines()).filter_map(|line| characters(&font, &build.gsub_only, line)).collect();
        std::fs::write(dir.join(format!("{name}.txt")), lines.join("\n")).unwrap();
        eprintln!("{name}: {} glyphs, {} sample lines", build.full.glyph_order.len(), lines.len());
        written += 1;
    }
    assert!(written > 0, "no source could be built");
}
