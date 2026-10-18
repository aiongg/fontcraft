//! The subroutiniser's measure on compiled fonts (M12.25). Run locally; not part of the gate.
//!
//! For each OpenType font with a `CFF ` or `CFF2` table it subroutinises the table, checks that
//! the result expands to the same charstrings and that read-fonts draws the same outlines and
//! widths, and prints one tab-separated line: the font, its glyph count, the bytes of the
//! CharStrings and subroutine INDEXes as given, expanded, after our subroutiniser and (with
//! `--reference DIR`) in the font of the same name in DIR, typically cffsubr's output; then
//! ours divided by the reference, whether the outlines are equal, and the time taken.
//!
//!     cargo run --release -p fontcraft-cff --example subr_measure -- \
//!         [--reference DIR] [--out DIR] FONT...
//!
//! `--out DIR` writes each font with our table into DIR, for `crosscheck/subr_check.py` to
//! decode with fontTools.
#![allow(clippy::indexing_slicing)]

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fontcraft_cff::subr::{CharstringSet, SubrOptions, charstrings_of_table, desubroutinize, subroutinize_table};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Default, PartialEq)]
struct Pen(Vec<i32>);

impl CommandSink for Pen {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.extend([1, x.to_bits(), y.to_bits()]);
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.extend([2, x.to_bits(), y.to_bits()]);
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
        self.0.extend([3, a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits(), e.to_bits(), f.to_bits()]);
    }
    fn close(&mut self) {
        self.0.push(4);
    }
}

/// The sfnt table directory: (tag, offset, length) per table.
fn directory(font: &[u8]) -> Result<Vec<([u8; 4], usize, usize)>> {
    let u16_at = |at: usize| font.get(at..at + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])));
    let u32_at = |at: usize| font.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let count = u16_at(4).ok_or("not an sfnt")?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let rec = 12 + 16 * i;
        let tag: [u8; 4] = font.get(rec..rec + 4).ok_or("cut short")?.try_into()?;
        out.push((tag, u32_at(rec + 8).ok_or("cut short")?, u32_at(rec + 12).ok_or("cut short")?));
    }
    Ok(out)
}

fn table<'a>(font: &'a [u8], want: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    for (tag, at, len) in directory(font)? {
        if &tag == want {
            return Ok(Some(font.get(at..at + len).ok_or("table outside the file")?));
        }
    }
    Ok(None)
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut word = [0u8; 4];
        word[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// `font` with table `tag` replaced by `data`, checksums recomputed.
fn replace_table(font: &[u8], tag: &[u8; 4], data: &[u8]) -> Result<Vec<u8>> {
    let dir = directory(font)?;
    let header_len = 12 + 16 * dir.len();
    let mut out = font.get(..header_len).ok_or("cut short")?.to_vec();
    let mut head_at = None;
    for (i, (t, at, len)) in dir.iter().enumerate() {
        let mut bytes = if t == tag { data.to_vec() } else { font.get(*at..at + len).ok_or("cut short")?.to_vec() };
        if t == b"head" && bytes.len() >= 12 {
            bytes[8..12].fill(0);
            head_at = Some(out.len());
        }
        let rec = 12 + 16 * i;
        let at = out.len() as u32;
        out[rec + 4..rec + 8].copy_from_slice(&checksum(&bytes).to_be_bytes());
        out[rec + 8..rec + 12].copy_from_slice(&at.to_be_bytes());
        out[rec + 12..rec + 16].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&bytes);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    if let Some(h) = head_at {
        let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[h + 8..h + 12].copy_from_slice(&adjust.to_be_bytes());
    }
    Ok(out)
}

fn outlines(data: &[u8], cff2: bool) -> Result<Vec<(Pen, Option<i32>)>> {
    let cff = if cff2 { CffFontRef::new_cff2(data, None)? } else { CffFontRef::new_cff(data, 0, None)? };
    let mut out = Vec::new();
    for gid in 0..cff.num_glyphs() {
        let gid = GlyphId::new(gid);
        let subfont = cff.subfont(cff.subfont_index(gid).unwrap_or(0), &[])?;
        let mut pen = Pen::default();
        let width = cff.evaluate_charstring(&subfont, gid, &[], &mut pen)?;
        out.push((pen, width.map(Fixed::to_bits)));
    }
    Ok(out)
}

fn index_bytes(font: &[u8]) -> Result<Option<usize>> {
    for (tag, cff2) in [(b"CFF ", false), (b"CFF2", true)] {
        if let Some(t) = table(font, tag)? {
            return Ok(Some(charstrings_of_table(t, cff2)?.index_bytes()));
        }
    }
    Ok(None)
}

fn measure(path: &Path, reference: Option<&Path>, out_dir: Option<&Path>) -> Result<String> {
    let font = std::fs::read(path)?;
    let (tag, cff2, data) = match (table(&font, b"CFF ")?, table(&font, b"CFF2")?) {
        (Some(t), _) => (b"CFF ", false, t),
        (None, Some(t)) => (b"CFF2", true, t),
        (None, None) => return Err("no CFF or CFF2 table".into()),
    };
    let given: CharstringSet = charstrings_of_table(data, cff2)?;
    let flat = desubroutinize(&given)?;
    let started = Instant::now();
    let ours = subroutinize_table(data, cff2, &SubrOptions::default())?;
    let millis = started.elapsed().as_millis();
    let ours_set = charstrings_of_table(&ours, cff2)?;
    let expands = desubroutinize(&ours_set)?.charstrings == flat.charstrings;
    let draws = outlines(&ours, cff2)? == outlines(data, cff2)?;
    let name = path.file_name().ok_or("no file name")?;
    let reference_bytes = match reference {
        Some(dir) => index_bytes(&std::fs::read(dir.join(name))?)?,
        None => None,
    };
    if let Some(dir) = out_dir {
        std::fs::write(dir.join(name), replace_table(&font, tag, &ours)?)?;
    }
    let ratio = reference_bytes.map_or(String::from("-"), |r| format!("{:.4}", ours_set.index_bytes() as f64 / r.max(1) as f64));
    Ok(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{ratio}\t{}\t{millis}",
        path.display(),
        given.charstrings.len(),
        given.index_bytes(),
        flat.index_bytes(),
        ours_set.index_bytes(),
        reference_bytes.map_or(String::from("-"), |r| r.to_string()),
        if expands && draws { "equal" } else { "DIFFERENT" },
    ))
}

fn main() -> Result<()> {
    let mut reference: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut fonts = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--reference" => reference = args.next().map(PathBuf::from),
            "--out" => out_dir = args.next().map(PathBuf::from),
            _ => fonts.push(PathBuf::from(a)),
        }
    }
    println!("font\tglyphs\tgiven\texpanded\tours\treference\tours/reference\toutlines\tms");
    for f in &fonts {
        match measure(f, reference.as_deref(), out_dir.as_deref()) {
            Ok(line) => println!("{line}"),
            Err(e) => println!("{}\terror: {e}", f.display()),
        }
    }
    Ok(())
}
