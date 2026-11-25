//! Writes a font's grid-fitted outlines as the hinted previews draw them, for
//! `crates/render/crosscheck/hinted_freetype.py`:
//!
//! `hinted_dump FONT grayscale|cleartype|directwrite FIRST-PPEM LAST-PPEM STEP`
//!
//! Every `STEP`th glyph at every size, one line each: the size, the glyph id, `1` or `0` for hinted
//! or not, the path with its coordinates in 64ths of a pixel, and after a tab the message, if any.

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Arc;

use fontcraft_geom::PathEl;
use fontcraft_render::hinted::{HintedRenderer, Intent};
use skrifa::{FontRef, MetadataProvider};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [font, intent, first, last, step] = args.as_slice() else {
        return Err("usage: hinted_dump FONT grayscale|cleartype|directwrite FIRST-PPEM LAST-PPEM STEP".into());
    };
    let intent = match intent.as_str() {
        "grayscale" => Intent::Grayscale,
        "cleartype" => Intent::GdiClearType,
        "directwrite" => Intent::DirectWrite,
        other => return Err(format!("`{other}` is no intent").into()),
    };
    let (first, last, step): (u16, u16, usize) = (first.parse()?, last.parse()?, step.parse()?);
    let bytes: Arc<[u8]> = std::fs::read(font)?.into();
    let glyphs = u32::try_from(FontRef::new(&bytes)?.outline_glyphs().iter().count())?;
    let renderer = HintedRenderer::new(bytes, intent)?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for ppem in first..=last {
        for gid in (0..glyphs).step_by(step.max(1)) {
            let o = renderer.outline(gid, ppem)?;
            let mut path = String::new();
            for el in o.path.elements() {
                let (name, points) = match *el {
                    PathEl::MoveTo(p) => ('M', vec![p]),
                    PathEl::LineTo(p) => ('L', vec![p]),
                    PathEl::QuadTo(a, p) => ('Q', vec![a, p]),
                    PathEl::CurveTo(a, b, p) => ('C', vec![a, b, p]),
                    PathEl::ClosePath => ('Z', Vec::new()),
                };
                path.push(name);
                for p in points {
                    write!(path, " {} {}", p.x * 64.0, p.y * 64.0)?;
                }
                path.push(' ');
            }
            writeln!(out, "{ppem} {gid} {} {}\t{}", u8::from(o.hinted), path.trim_end(), o.message.unwrap_or_default())?;
        }
    }
    out.flush()?;
    Ok(())
}
