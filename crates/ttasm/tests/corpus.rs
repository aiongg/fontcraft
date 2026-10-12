//! Every `fpgm`, `prep` and glyph program of the corpus's TrueType fonts disassembles and
//! assembles back to the same bytes, and the analysis runs on each of them without an error
//! other than an unbalanced block. Skipped when `corpus/` is absent. The sfnt reading here is
//! the minimum the test needs (table directory, `head`, `maxp`, `loca`, `glyf`); WOFF, WOFF2
//! and collections are left out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_ttasm::{AsmError, FunctionTable, analyse, assemble, disassemble};

fn fonts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_dir() {
            fonts(&p, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ["ttf", "otf"].contains(&x.to_ascii_lowercase().as_str())) {
            out.push(p);
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let n = usize::from(u16_at(font, 4)?);
    (0..n).find_map(|i| {
        let rec = 12 + 16 * i;
        if font.get(rec..rec + 4)? != tag {
            return None;
        }
        let offset = u32_at(font, rec + 8)? as usize;
        let length = u32_at(font, rec + 12)? as usize;
        font.get(offset..offset + length)
    })
}

/// The instructions of every glyph, simple and composite.
fn glyph_programs(font: &[u8]) -> Vec<(usize, Vec<u8>)> {
    let (Some(head), Some(maxp), Some(loca), Some(glyf)) = (table(font, b"head"), table(font, b"maxp"), table(font, b"loca"), table(font, b"glyf"))
    else {
        return Vec::new();
    };
    let long = u16_at(head, 50) == Some(1);
    let count = usize::from(u16_at(maxp, 4).unwrap_or(0));
    let offset =
        |i: usize| -> Option<usize> { if long { u32_at(loca, 4 * i).map(|v| v as usize) } else { u16_at(loca, 2 * i).map(|v| 2 * usize::from(v)) } };
    let mut out = Vec::new();
    for gid in 0..count {
        let (Some(start), Some(end)) = (offset(gid), offset(gid + 1)) else { break };
        let Some(g) = glyf.get(start..end) else { continue };
        if g.len() < 10 {
            continue;
        }
        let contours = u16_at(g, 0).unwrap() as i16;
        let at = if contours >= 0 {
            Some(10 + 2 * contours as usize)
        } else {
            // Walk the components; the program follows the last one when any sets
            // WE_HAVE_INSTRUCTIONS.
            let mut pos = 10;
            let mut has = false;
            loop {
                let Some(flags) = u16_at(g, pos) else { break None };
                has |= flags & 0x0100 != 0;
                pos += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
                pos += if flags & 0x0008 != 0 {
                    2
                } else if flags & 0x0040 != 0 {
                    4
                } else if flags & 0x0080 != 0 {
                    8
                } else {
                    0
                };
                if flags & 0x0020 == 0 {
                    break has.then_some(pos);
                }
            }
        };
        let Some(at) = at else { continue };
        let Some(len) = u16_at(g, at) else { continue };
        if let Some(program) = g.get(at + 2..at + 2 + usize::from(len))
            && !program.is_empty()
        {
            out.push((gid, program.to_vec()));
        }
    }
    out
}

#[test]
fn corpus_programs_disassemble_and_assemble_to_the_same_bytes() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut paths = Vec::new();
    fonts(&corpus, &mut paths);
    paths.sort();
    let (mut programs, mut fonts_with_programs) = (0usize, 0usize);
    let mut differences = Vec::new();
    for path in &paths {
        let font = std::fs::read(path).unwrap();
        let mut list: Vec<(String, Vec<u8>)> = Vec::new();
        for tag in [b"fpgm", b"prep"] {
            if let Some(t) = table(&font, tag) {
                list.push((String::from_utf8_lossy(tag).into_owned(), t.to_vec()));
            }
        }
        list.extend(glyph_programs(&font).into_iter().map(|(gid, p)| (format!("glyph {gid}"), p)));
        if list.is_empty() {
            continue;
        }
        fonts_with_programs += 1;
        let functions = table(&font, b"fpgm").map(FunctionTable::from_fpgm);
        let functions = match functions {
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                differences.push(format!("{}: fpgm functions: {e}", path.display()));
                FunctionTable::default()
            }
            None => FunctionTable::default(),
        };
        for (what, bytes) in list {
            programs += 1;
            let back = disassemble(&bytes).and_then(|text| assemble(&text));
            match back {
                Ok(b) if b == bytes => {}
                Ok(_) => differences.push(format!("{} {what}: assembles to different bytes", path.display())),
                Err(e) => differences.push(format!("{} {what}: {e}", path.display())),
            }
            match analyse(&bytes, &functions) {
                Ok(_) | Err(AsmError::Unbalanced { .. }) => {}
                Err(e) => differences.push(format!("{} {what}: analysis: {e}", path.display())),
            }
        }
    }
    eprintln!("{programs} programs in {fonts_with_programs} fonts, {} differences", differences.len());
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
