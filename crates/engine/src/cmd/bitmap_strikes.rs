//! Bitmap strikes of the open font (M13.55b, M13.55c, M13.55e, M13.55f): `bitmap.importStrikes`
//! reads BDF, PCF (plain or gzip-compressed), Windows FNT and FON, TeX PK and GF and Mac bitmap
//! font files into strikes, `bitmap.exportStrikes` writes strikes as BDF, PCF, FNT, FON or PK
//! files, embeds them in a compiled font or writes a bitmap-only font, and `bitmap.setStrikes`
//! adds strikes, empty or rasterised from the outlines, and removes strikes.
//!
//! The files are read and checked by `fontcraft_io_bitmap`, whose limits bound every count and
//! size; the model's own check ([`fontcraft_model::strike`]) then runs on the edited font, so a
//! strike that does not fit leaves the font as it was. Strikes are named by size and depth: a
//! strike spec `{ppem, depth?}` without a depth matches every depth of that size, except when
//! adding, where it means 1 bit per pixel.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::file_io::read_input_up_to;
use super::{CommandSpec, FileParam, cmd, has_doc};
use crate::params::Params;
use crate::{EngineError, Result, Session};
use fontcraft_geom::{Affine, Shape as _};
use fontcraft_io_bitmap::{Bitmap, BitmapFont, BitmapGlyph, PcfLayout, Property, PropertyValue};
use fontcraft_model::strike::{self, BitmapStrike, StrikeGlyph, StrikeProperty, StrikeValue};
use fontcraft_model::{Font, MAX_NAME_LEN, metric};
use fontcraft_tables::embedded_bitmaps::{self as embedded, GlyphIds};

/// Most glyph names a result lists as left out, per file.
const MAX_LISTED: usize = 100;
/// Resolution assumed for a strike that states none, in dots per inch: a point is then a pixel.
const DEFAULT_DPI: u32 = 72;
/// Most pixels rasterised for one new strike.
const MAX_RASTER_PIXELS: u64 = 64 << 20;
/// Most pixels rasterised in one `bitmap.setStrikes` call, over every strike it adds.
const MAX_CALL_RASTER_PIXELS: u64 = 256 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "bitmap.importStrikes",
            "Import Bitmap Strikes…",
            ["File", "Import"],
            None,
            "{paths: [path of a bitmap font file: .bdf, .pcf, .pcf.gz, .bdf.gz, Windows .fnt or .fon, TeX .pk or .gf, or a Mac bitmap font (a resource fork, .dfont, AppleSingle, AppleDouble or MacBinary file)], asBackground?: false} adds the strikes each file holds to the font, as one undo step: one per file, each font of a .fon file, or each plain-style size of a Mac font family; at most 64 strikes in all. A bitmap goes to the glyph whose code point is the file's code for it (when the file's codes are Unicode), else to the glyph of its name; bitmaps that match no glyph are left out and listed, and no glyph is created. A TeX PK or GF file's codes are not Unicode, so its bitmaps match by name. A strike none of whose bitmaps match is left out with a warning when another strike of its file matches, and so is a strike of a size and depth an earlier strike of the files has. A strike of the same size and depth as one the font has replaces that strike's name, metrics and properties and the bitmaps of the glyphs it holds; the strike's other bitmaps stay. Refused, changing nothing: a file in none of these formats, gzip data that does not inflate to a PCF or BDF file, a grey Mac font, a packed run that overflows its glyph's box, a file beyond the bitmap readers' limits, files that hold more than 64 strikes together, a file none of whose strikes has a bitmap that matches a glyph, asBackground (not built) → {strikes: [{path, ppem, depth, label, glyphs, replaced, leftOut: [name], leftOutCount}], warnings: [string]}",
            has_doc,
            import_strikes
        )
        .files(const { &[FileParam::read("paths")] }),
        cmd!(
            "bitmap.exportStrikes",
            "Export Bitmap Strikes…",
            ["File", "Export"],
            None,
            "{strikes?: [{ppem: number, depth?: number}] (default: all; without depth, every depth of the size), format?: \"bdf\" | \"pcf\" | \"pcf.gz\" | \"fnt\" | \"fon\" | \"pk\" | \"embedded\" | \"otb\" (default: from a path ending in .pcf, .pcf.gz, .fnt, .fon, .pk or .<dpi>pk, or .otb, else bdf), path?: file, destination?: folder (bdf, pcf, pcf.gz, fnt and pk: one file per strike, named Family-12.bdf, or Family-12@4.bdf for a grey strike), font?: path of a compiled TrueType or OpenType font (embedded only; default: the open font compiled as TrueType)} writes strikes, atomically each file. bdf and pcf: one bitmap font file per strike; `path` takes exactly one strike. A bitmap's code is the one its file had, else the first code point of its glyph; a strike without a stated point size or resolution is written at 72 dpi, and one without a name takes the family name. PCF, FNT, FON and PK hold black-and-white strikes only. fnt: a Windows bitmap font, version 3; a bitmap's code goes through code page 1252 when the strike's codes are Unicode, and bitmaps without an 8-bit code, or with pixels outside their advance, are left out or cut and listed in the file's `warnings`. fon: every chosen strike as one font of the Windows font file at `path`. pk: a TeX packed font whose design size makes it read back at the strike's size (pixels per em ÷ (dots per inch ÷ 72.27)); bitmaps without a code, or with a code an earlier bitmap has, are left out. embedded: the chosen strikes as EBDT and EBLC tables in `font`, written to `path`, replacing the font's own bitmap tables. otb: a bitmap-only font at `path`: the open font compiled as TrueType, with its outlines emptied and the strikes added. A bitmap goes to the glyph of its name, else of its glyph's code point; bitmaps of glyphs the font lacks are left out and listed. Refused, writing nothing: a strike above 255 pixels per em, a bitmap larger than 255 pixels or offset beyond -128..127, strikes none of whose bitmaps has a glyph in the font, a collection or web font as `font` → {format, files: [{strike, path, glyphs, bytes}], replaced?, empty?: [label], leftOut?: [name], leftOutCount?}",
            has_doc,
            export_strikes
        )
        .files(const { &[FileParam::write("path"), FileParam::write("destination"), FileParam::read("font")] })
        .writes_files(),
        cmd!(
            "bitmap.setStrikes",
            "Bitmap Strikes…",
            ["Font"],
            None,
            "{add?: [{ppem: number, depth?: 1 | 2 | 4 | 8 (default 1)}], remove?: [{ppem: number, depth?: number (default: every depth of the size)}], rasterize?: bool (default true)} adds and removes the font's bitmap strikes as one undo step; removing a strike deletes its bitmaps. A new strike is filled from the outlines of the first master's exported glyphs, anti-aliased: at 1 bit a pixel is on when the outline covers at least half of it, at 2, 4 or 8 bits its grey level is the covered fraction; each bitmap spans the outline's pixel box, advances are the glyph widths scaled and rounded, and the strike's ascent and descent are the master's, scaled. rasterize: false gives an empty strike. Refused, changing nothing: neither list, a strike to add that the font has, a strike to remove that it lacks, a bitmap wider or taller than 1,024 pixels, more than 67,108,864 pixels for one strike or 268,435,456 for all the strikes added, bitmaps over 64 MiB for one strike or over 256 MiB for all the font's strikes together → {added: [label], removed: [label], strikes: [label]}",
            has_doc,
            set_strikes
        ),
    ]
}

/// A `{ppem, depth?}` item of a strike list.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StrikeSpec {
    ppem: u16,
    depth: Option<u8>,
}

impl StrikeSpec {
    fn matches(self, s: &BitmapStrike) -> bool {
        s.ppem == self.ppem && self.depth.is_none_or(|d| d == s.depth)
    }

    fn label(self) -> String {
        match self.depth {
            Some(d) => strike::label(self.ppem, d),
            None => self.ppem.to_string(),
        }
    }
}

fn strike_specs(p: &Params<'_>, key: &str) -> Result<Option<Vec<StrikeSpec>>> {
    let Some(items) = p.opt_array(key)? else { return Ok(None) };
    if items.len() > strike::MAX_STRIKES {
        return Err(p.bad(format!("`{key}` lists more than {} strikes", strike::MAX_STRIKES)));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let ppem = item
            .get("ppem")
            .and_then(Value::as_u64)
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| p.bad(format!("`{key}`: each item needs `ppem`, a whole number from 1 to {}", u16::MAX)))?;
        let depth = match item.get("depth") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|d| u8::try_from(d).ok())
                    .filter(|d| strike::valid_depth(*d))
                    .ok_or_else(|| p.bad(format!("`{key}`: `depth` must be 1, 2, 4 or 8")))?,
            ),
        };
        out.push(StrikeSpec { ppem, depth });
    }
    Ok(Some(out))
}

fn sort_strikes(strikes: &mut [BitmapStrike]) {
    strikes.sort_by_key(|s| (s.ppem, s.depth));
}

fn labels(strikes: &[BitmapStrike]) -> Vec<String> {
    strikes.iter().map(BitmapStrike::label).collect()
}

// ── bitmap.importStrikes ──────────────────────────────────────────────────────────────────────

/// One file's strike, made ready to go into the font.
struct Incoming {
    path: String,
    strike: BitmapStrike,
    left_out: Vec<String>,
    left_out_count: usize,
}

fn import_strikes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.importStrikes", v)?;
    s.font()?;
    if p.bool_or("asBackground", false)? {
        return Err(EngineError::Unsupported("importing a bitmap font as background images".into()));
    }
    let paths = p.strings("paths")?;
    if paths.is_empty() {
        return Err(p.bad("`paths` is empty: choose a bitmap font file"));
    }
    if paths.len() > strike::MAX_STRIKES {
        return Err(p.bad(format!("`paths` lists more than {} files", strike::MAX_STRIKES)));
    }
    let mut files = Vec::with_capacity(paths.len());
    let mut warnings = Vec::new();
    for path in &paths {
        let input = read_input_up_to(Path::new(path), fontcraft_io_bitmap::MAX_FILE_BYTES)?;
        if input.bytes.len() > fontcraft_io_bitmap::MAX_FILE_BYTES {
            return Err(p.bad(format!("{path} is larger than {} bytes", fontcraft_io_bitmap::MAX_FILE_BYTES)));
        }
        for import in fontcraft_io_bitmap::read_all(&input.bytes).map_err(|e| p.bad(format!("{path}: {e}")))? {
            warnings.extend(import.warnings.into_iter().map(|w| format!("{path}: {w}")));
            files.push((path.to_string(), import.font));
        }
        if files.len() > strike::MAX_STRIKES {
            return Err(p.bad(format!("the files hold more than {} strikes", strike::MAX_STRIKES)));
        }
    }
    let font = s.font()?;
    let mut incoming: Vec<Incoming> = Vec::with_capacity(files.len());
    // A file is refused when none of its strikes matches; an unmatched strike of a file whose
    // other strikes match is left out with a warning.
    let mut unmatched: Vec<(&str, String, usize)> = Vec::new();
    let mut matched_paths: HashSet<&str> = HashSet::new();
    for (path, bitmap_font) in &files {
        let one = to_strike(font, path, bitmap_font, &mut warnings);
        if one.strike.glyphs.is_empty() {
            unmatched.push((path.as_str(), one.strike.label(), bitmap_font.glyphs.len()));
            continue;
        }
        matched_paths.insert(path.as_str());
        // Two strikes of one size and depth would merge into one; the first is kept, as opening
        // the file keeps it.
        if incoming.iter().any(|i| i.strike.ppem == one.strike.ppem && i.strike.depth == one.strike.depth) {
            warnings.push(format!(
                "{path}: a further strike of {} was left out: an earlier strike of the files has that size and depth",
                one.strike.label()
            ));
            continue;
        }
        incoming.push(one);
    }
    for (path, label, count) in unmatched {
        let none = format!("none of its {count} bitmaps matches a glyph of the font by code or name");
        if !matched_paths.contains(path) {
            return Err(p.bad(format!("{path}: {none}")));
        }
        warnings.push(format!("{path}: the {label} strike was left out: {none}"));
    }
    let replaced = s.edit("Import Bitmap Strikes", |font| {
        let strikes = std::sync::Arc::make_mut(&mut font.bitmap_strikes);
        let mut replaced = Vec::with_capacity(incoming.len());
        for one in &incoming {
            let new = one.strike.clone();
            match strikes.iter_mut().find(|s| s.ppem == new.ppem && s.depth == new.depth) {
                Some(have) => {
                    let names: HashSet<&str> = new.glyphs.iter().map(|g| g.glyph.as_str()).collect();
                    let mut glyphs: Vec<StrikeGlyph> = have.glyphs.iter().filter(|g| !names.contains(g.glyph.as_str())).cloned().collect();
                    glyphs.extend(new.glyphs);
                    *have = BitmapStrike { glyphs, ..new };
                    replaced.push(true);
                }
                None => {
                    strikes.push(new);
                    replaced.push(false);
                }
            }
        }
        sort_strikes(strikes);
        Ok(replaced)
    })?;
    let strikes: Vec<Value> = incoming
        .iter()
        .zip(replaced)
        .map(|(one, replaced)| {
            json!({
                "path": one.path,
                "ppem": one.strike.ppem,
                "depth": one.strike.depth,
                "label": one.strike.label(),
                "glyphs": one.strike.glyphs.len(),
                "replaced": replaced,
                "leftOut": one.left_out,
                "leftOutCount": one.left_out_count,
            })
        })
        .collect();
    Ok(json!({ "strikes": strikes, "warnings": warnings }))
}

/// The strike of one file, with each bitmap given to the glyph of its code (when the codes are
/// Unicode), else of its name. A second bitmap for one glyph, and a bitmap whose depth is not the
/// file's, are left out with a warning.
fn to_strike(font: &Font, path: &str, file: &BitmapFont, warnings: &mut Vec<String>) -> Incoming {
    let mut by_code: HashMap<u32, &str> = HashMap::new();
    if file.codes_are_unicode() {
        for g in font.glyphs.iter() {
            for u in &g.unicodes {
                by_code.entry(*u).or_insert(g.name.as_str());
            }
        }
    }
    let mut strike = BitmapStrike::new(file.pixel_size, file.depth);
    strike.name = file.name.clone();
    strike.point_size = file.point_size;
    strike.resolution = [file.resolution.0, file.resolution.1];
    strike.ascent = file.ascent;
    strike.descent = file.descent;
    strike.properties = file.properties.iter().map(|p| StrikeProperty { name: p.name.clone(), value: to_value(&p.value) }).collect();
    let mut taken = HashSet::new();
    let mut left_out = Vec::new();
    let mut left_out_count = 0usize;
    let mut doubled = 0usize;
    for g in &file.glyphs {
        let by_encoding = g.encoding.and_then(|c| by_code.get(&c).copied());
        let target = by_encoding.or_else(|| font.glyphs.get(&g.name).map(|f| f.name.as_str()));
        let Some(name) = target else {
            left_out_count += 1;
            if left_out.len() < MAX_LISTED {
                left_out.push(g.name.chars().take(MAX_NAME_LEN).collect());
            }
            continue;
        };
        if g.bitmap.depth != file.depth {
            warnings.push(format!("{path}: the bitmap of `{name}` has {} bits per pixel, the strike {}; left out", g.bitmap.depth, file.depth));
            continue;
        }
        if !taken.insert(name) {
            doubled += 1;
            continue;
        }
        strike.glyphs.push(StrikeGlyph {
            glyph: name.to_string(),
            code: g.encoding,
            advance: g.advance,
            scalable_width: g.scalable_width,
            width: g.bitmap.width,
            height: g.bitmap.height,
            left: g.bitmap.left,
            bottom: g.bitmap.bottom,
            data: g.bitmap.data.clone(),
        });
    }
    if doubled > 0 {
        warnings.push(format!("{path}: {doubled} bitmaps matched a glyph that an earlier bitmap of the file already had; the first was kept"));
    }
    Incoming { path: path.to_string(), strike, left_out, left_out_count }
}

fn to_value(v: &PropertyValue) -> StrikeValue {
    match v {
        PropertyValue::Integer(i) => StrikeValue::Integer(*i),
        PropertyValue::Text(t) => StrikeValue::Text(t.clone()),
    }
}

// ── bitmap.exportStrikes ──────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Bdf,
    Pcf,
    PcfGz,
    Fnt,
    Fon,
    Pk,
    Embedded,
    Otb,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Format::Bdf => "bdf",
            Format::Pcf => "pcf",
            Format::PcfGz => "pcf.gz",
            Format::Fnt => "fnt",
            Format::Fon => "fon",
            Format::Pk => "pk",
            Format::Embedded => "embedded",
            Format::Otb => "otb",
        }
    }
}

fn export_strikes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.exportStrikes", v)?;
    let font = s.font()?;
    let path = p.opt_str("path")?.map(str::trim).filter(|x| !x.is_empty());
    let destination = p.opt_str("destination")?.map(str::trim).filter(|x| !x.is_empty());
    let format = match p.opt_str("format")? {
        Some("bdf") => Format::Bdf,
        Some("pcf") => Format::Pcf,
        Some("pcf.gz") => Format::PcfGz,
        Some("fnt") => Format::Fnt,
        Some("fon") => Format::Fon,
        Some("pk") => Format::Pk,
        Some("embedded") => Format::Embedded,
        Some("otb") => Format::Otb,
        Some(other) => {
            return Err(p.bad(format!(
                "`format` must be bdf, pcf, pcf.gz, fnt, fon, pk, embedded or otb, got `{}`",
                other.chars().take(32).collect::<String>()
            )));
        }
        None if path.is_some_and(|x| x.to_ascii_lowercase().ends_with(".pcf")) => Format::Pcf,
        None if path.is_some_and(|x| x.to_ascii_lowercase().ends_with(".pcf.gz")) => Format::PcfGz,
        None if path.is_some_and(|x| x.to_ascii_lowercase().ends_with(".fnt")) => Format::Fnt,
        None if path.is_some_and(|x| x.to_ascii_lowercase().ends_with(".fon")) => Format::Fon,
        None if path.is_some_and(is_pk_path) => Format::Pk,
        None if path.is_some_and(|x| x.to_ascii_lowercase().ends_with(".otb")) => Format::Otb,
        None => Format::Bdf,
    };
    if font.bitmap_strikes.is_empty() {
        return Err(p.bad("the font has no bitmap strikes"));
    }
    let chosen: Vec<&BitmapStrike> = match strike_specs(&p, "strikes")? {
        None => font.bitmap_strikes.iter().collect(),
        Some(specs) => {
            let mut out: Vec<&BitmapStrike> = Vec::new();
            for spec in specs {
                let found: Vec<&BitmapStrike> = font.bitmap_strikes.iter().filter(|s| spec.matches(s)).collect();
                if found.is_empty() {
                    return Err(p.bad(format!("the font has no strike {}", spec.label())));
                }
                for s in found {
                    if !out.iter().any(|o| std::ptr::eq(*o, s)) {
                        out.push(s);
                    }
                }
            }
            out
        }
    };
    if matches!(format, Format::Embedded | Format::Otb) {
        return export_sfnt(font, &p, format, &chosen, path, destination);
    }
    if p.opt_str("font")?.is_some() {
        return Err(p.bad("`font` goes with format embedded"));
    }
    if format == Format::Fon {
        return export_fon(font, &p, &chosen, path, destination);
    }
    let targets: Vec<PathBuf> = match (path, destination) {
        (Some(_), Some(_)) => return Err(p.bad("give `path` or `destination`, not both")),
        (None, None) => return Err(p.bad("give `path` (one strike) or `destination` (a folder)")),
        (Some(path), None) => {
            if chosen.len() != 1 {
                return Err(p.bad(format!("`path` takes one strike and {} are chosen: name one in `strikes`, or give `destination`", chosen.len())));
            }
            vec![PathBuf::from(path)]
        }
        (None, Some(dir)) => {
            let base = file_base(font);
            chosen.iter().map(|s| PathBuf::from(dir).join(format!("{base}-{}.{}", s.label(), format.extension()))).collect()
        }
    };
    let mut out = Vec::with_capacity(chosen.len());
    for strike in &chosen {
        let file = to_bitmap_font(font, strike);
        let mut file_warnings = Vec::new();
        let bytes = match format {
            Format::Bdf => fontcraft_io_bitmap::write_bdf(&file).map(String::into_bytes),
            Format::Pcf => fontcraft_io_bitmap::write_pcf(&file, PcfLayout::default()),
            Format::PcfGz => fontcraft_io_bitmap::write_pcf_gz(&file, PcfLayout::default()),
            Format::Fnt => fontcraft_io_bitmap::write_fnt(&file).map(|w| {
                file_warnings = w.warnings;
                w.bytes
            }),
            Format::Pk => fontcraft_io_bitmap::write_pk(&file),
            // Written by `export_sfnt` and `export_fon` above, as one font file.
            Format::Embedded | Format::Otb | Format::Fon => return Err(p.bad("embedded, otb and fon strikes go into one font file")),
        }
        .map_err(|e| p.bad(format!("strike {}: {e}", strike.label())))?;
        out.push((strike.label(), strike.glyphs.len(), bytes, file_warnings));
    }
    let mut files = Vec::with_capacity(out.len());
    for ((label, glyphs, bytes, file_warnings), target) in out.into_iter().zip(targets) {
        write_file(&target, &bytes)?;
        let mut entry = json!({"strike": label, "path": target.to_string_lossy(), "glyphs": glyphs, "bytes": bytes.len()});
        if !file_warnings.is_empty() {
            entry["warnings"] = json!(file_warnings);
        }
        files.push(entry);
    }
    Ok(json!({ "format": format.extension(), "files": files }))
}

/// The `fon` export: every chosen strike as one font of the Windows font file at `path`.
fn export_fon(font: &Font, p: &Params<'_>, chosen: &[&BitmapStrike], path: Option<&str>, destination: Option<&str>) -> Result<Value> {
    if destination.is_some() {
        return Err(p.bad("`destination` goes with bdf, pcf, pcf.gz, fnt and pk; give `path`, the .fon file to write"));
    }
    let Some(path) = path else { return Err(p.bad("give `path`, the .fon file to write")) };
    let files: Vec<BitmapFont> = chosen.iter().map(|s| to_bitmap_font(font, s)).collect();
    let written = fontcraft_io_bitmap::write_fon(&files).map_err(|e| p.bad(e.to_string()))?;
    write_file(Path::new(path), &written.bytes)?;
    let labels: Vec<String> = chosen.iter().map(|s| s.label()).collect();
    let glyphs: usize = chosen.iter().map(|s| s.glyphs.len()).sum();
    let mut entry = json!({"strike": labels.join(", "), "strikes": labels, "path": path, "glyphs": glyphs, "bytes": written.bytes.len()});
    if !written.warnings.is_empty() {
        entry["warnings"] = json!(written.warnings);
    }
    Ok(json!({ "format": "fon", "files": [entry] }))
}

/// Whether a path ends in `.pk` or in TeX's `.<dpi>pk`, such as `cmr10.600pk`.
fn is_pk_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let Some((_, extension)) = lower.rsplit_once('.') else { return false };
    extension.strip_suffix("pk").is_some_and(|dpi| dpi.bytes().all(|b| b.is_ascii_digit()))
}

/// The `embedded` and `otb` exports: the chosen strikes as `EBDT` and `EBLC` in a compiled font,
/// written to `path`.
fn export_sfnt(
    font: &Font,
    p: &Params<'_>,
    format: Format,
    chosen: &[&BitmapStrike],
    path: Option<&str>,
    destination: Option<&str>,
) -> Result<Value> {
    if destination.is_some() {
        return Err(p.bad("`destination` goes with bdf and pcf; give `path`, the font file to write"));
    }
    let Some(path) = path else { return Err(p.bad("give `path`, the font file to write")) };
    let source = p.opt_str("font")?.map(str::trim).filter(|x| !x.is_empty());
    if source.is_some() && format == Format::Otb {
        return Err(p.bad("`font` goes with format embedded; otb is made from the open font"));
    }
    let (mut sfnt, ids) = match source {
        Some(file) => {
            let limit = fontcraft_tables::limits::MAX_SFNT;
            let input = read_input_up_to(Path::new(file), limit)?;
            if input.bytes.len() > limit {
                return Err(p.bad(format!("{file} is larger than {limit} bytes")));
            }
            // Only a bare sfnt is written back as it was read: a collection or a web font would
            // lose its other faces or its wrapper.
            if !matches!(input.bytes.get(..4), Some([0, 1, 0, 0] | [b'O', b'T', b'T', b'O'] | [b't', b'r', b'u', b'e'])) {
                return Err(p.bad(format!("{file} is not a TrueType or OpenType font file; collections and web fonts are refused")));
            }
            let sfnt = fontcraft_tables::read(&input.bytes, 0).map_err(|e| p.bad(format!("{file}: {e}")))?;
            let ids = GlyphIds::from_font(&sfnt).map_err(|e| p.bad(format!("{file}: {e}")))?;
            (sfnt, ids)
        }
        None => {
            let compiled = fontcraft_compile::compile_ttf(font, &fontcraft_compile::Options::default())?;
            let sfnt = fontcraft_tables::read(&compiled.bytes, 0)?;
            (sfnt, GlyphIds::from_order(&compiled.glyph_order))
        }
    };
    let glyph_id = |g: &StrikeGlyph| ids.get(&g.glyph, font.glyph(&g.glyph).map(|f| f.unicodes.as_slice()).unwrap_or_default());
    let tables = embedded::build(chosen, &glyph_id).map_err(|e| p.bad(e.to_string()))?;
    let (out, replaced) = match format {
        Format::Otb => (embedded::bitmap_only(&sfnt, &tables)?, false),
        _ => {
            let replaced = embedded::embed(&mut sfnt, &tables);
            (sfnt, replaced)
        }
    };
    let bytes = fontcraft_tables::write(&out, &fontcraft_tables::WriteOptions::default())?;
    write_file(Path::new(path), &bytes)?;
    let labels: Vec<&str> = tables.strikes.iter().map(|s| s.label.as_str()).collect();
    let glyphs: usize = tables.strikes.iter().map(|s| s.glyphs).sum();
    Ok(json!({
        "format": format.extension(),
        "files": [{"strike": labels.join(" "), "path": path, "glyphs": glyphs, "bytes": bytes.len()}],
        "replaced": replaced,
        "empty": tables.empty,
        "leftOut": tables.left_out,
        "leftOutCount": tables.left_out_count,
    }))
}

/// The family name with the characters file systems refuse replaced by `-`; `Strikes` when
/// nothing usable is left.
fn file_base(font: &Font) -> String {
    let clean: String = font
        .family_name
        .trim()
        .chars()
        .map(|c| if c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .take(64)
        .collect();
    if clean.is_empty() || clean.starts_with('.') { "Strikes".to_string() } else { clean }
}

/// A strike as the bitmap writers take it.
fn to_bitmap_font(font: &Font, strike: &BitmapStrike) -> BitmapFont {
    let [x, y] = strike.resolution;
    let resolution = (if x == 0 { DEFAULT_DPI } else { x }, if y == 0 { DEFAULT_DPI } else { y });
    let point_size = if strike.point_size > 0.0 { strike.point_size } else { f64::from(strike.ppem) * 72.0 / f64::from(resolution.1.max(1)) };
    let name = if strike.name.is_empty() { font.family_name.trim().replace(['\n', '\r'], " ") } else { strike.name.clone() };
    let properties: Vec<Property> = strike
        .properties
        .iter()
        .map(|p| Property {
            name: p.name.clone(),
            value: match &p.value {
                StrikeValue::Integer(i) => PropertyValue::Integer(*i),
                StrikeValue::Text(t) => PropertyValue::Text(t.clone()),
            },
        })
        .collect();
    let mut file = BitmapFont {
        name: if name.is_empty() { "Untitled".into() } else { name },
        pixel_size: strike.ppem,
        point_size,
        resolution,
        depth: strike.depth,
        ascent: strike.ascent,
        descent: strike.descent,
        properties,
        glyphs: Vec::with_capacity(strike.glyphs.len()),
    };
    let unicode = file.codes_are_unicode();
    file.glyphs = strike
        .glyphs
        .iter()
        .map(|g| BitmapGlyph {
            name: g.glyph.clone(),
            encoding: g.code.or_else(|| if unicode { font.glyph(&g.glyph).and_then(|f| f.unicodes.first().copied()) } else { None }),
            advance: g.advance,
            scalable_width: g.scalable_width,
            bitmap: Bitmap { width: g.width, height: g.height, left: g.left, bottom: g.bottom, depth: strike.depth, data: g.data.clone() },
        })
        .collect();
    file
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    fontcraft_format::write_atomic(path, bytes)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_file(_: &Path, _: &[u8]) -> Result<()> {
    Err(EngineError::Unsupported("writing a file by path in the browser".into()))
}

// ── bitmap.setStrikes ─────────────────────────────────────────────────────────────────────────

fn set_strikes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.setStrikes", v)?;
    let add = strike_specs(&p, "add")?.unwrap_or_default();
    let remove = strike_specs(&p, "remove")?.unwrap_or_default();
    if add.is_empty() && remove.is_empty() {
        return Err(p.bad("give `add` or `remove`"));
    }
    let rasterize = p.bool_or("rasterize", true)?;
    let font = s.font()?;
    for spec in &remove {
        if !font.bitmap_strikes.iter().any(|s| spec.matches(s)) {
            return Err(p.bad(format!("the font has no strike {}", spec.label())));
        }
    }
    let mut added = Vec::with_capacity(add.len());
    for spec in &add {
        let depth = spec.depth.unwrap_or(1);
        let gone = remove.iter().any(|r| r.ppem == spec.ppem && r.depth.is_none_or(|d| d == depth));
        let have = font.bitmap_strikes.iter().any(|s| s.ppem == spec.ppem && s.depth == depth);
        if (have && !gone) || added.iter().any(|(pp, d)| *pp == spec.ppem && *d == depth) {
            return Err(p.bad(format!("the font already has strike {}", strike::label(spec.ppem, depth))));
        }
        added.push((spec.ppem, depth));
    }
    let removed: Vec<String> = font.bitmap_strikes.iter().filter(|s| remove.iter().any(|r| r.matches(s))).map(BitmapStrike::label).collect();
    let kept = font.bitmap_strikes.iter().filter(|s| !remove.iter().any(|r| r.matches(s))).fold(0usize, |n, s| n.saturating_add(s.bytes()));
    let mut budget = RasterBudget { pixels: MAX_CALL_RASTER_PIXELS, bytes: strike::MAX_ALL_STRIKES_BYTES.saturating_sub(kept) };
    // Every strike is measured against the budget before any is drawn, so a refusal comes
    // before the work.
    if rasterize {
        for (ppem, depth) in &added {
            rasterize_strike(font, &p, *ppem, *depth, &mut budget, false)?;
        }
    }
    let mut new = Vec::with_capacity(added.len());
    for (ppem, depth) in &added {
        new.push(if rasterize {
            rasterize_strike(font, &p, *ppem, *depth, &mut RasterBudget::unlimited(), true)?
        } else {
            BitmapStrike::new(*ppem, *depth)
        });
    }
    let strikes = s.edit("Bitmap Strikes", |font| {
        let strikes = std::sync::Arc::make_mut(&mut font.bitmap_strikes);
        strikes.retain(|s| !remove.iter().any(|r| r.matches(s)));
        strikes.extend(new);
        sort_strikes(strikes);
        Ok(labels(strikes))
    })?;
    let added: Vec<String> = added.iter().map(|(ppem, depth)| strike::label(*ppem, *depth)).collect();
    Ok(json!({ "added": added, "removed": removed, "strikes": strikes }))
}

/// What the rest of one `bitmap.setStrikes` call may rasterise: pixels to draw, and bytes of
/// bitmaps the font can still hold beside the strikes it keeps.
struct RasterBudget {
    pixels: u64,
    bytes: usize,
}

impl RasterBudget {
    /// For drawing strikes already measured against the real budget.
    fn unlimited() -> Self {
        Self { pixels: u64::MAX, bytes: usize::MAX }
    }
}

/// A strike of `ppem` pixels per em at `depth` bits per pixel, drawn from the outlines of the
/// first master's exported glyphs (see `bitmap.setStrikes`). Each bitmap is taken from `budget`
/// before it is drawn; crossing it refuses the strike. Unless `draw`, the bitmaps are only
/// measured and left empty.
fn rasterize_strike(font: &Font, p: &Params<'_>, ppem: u16, depth: u8, budget: &mut RasterBudget, draw: bool) -> Result<BitmapStrike> {
    let master = font.default_master()?;
    let upm = f64::from(font.units_per_em.max(1));
    let em = f64::from(ppem);
    // Font units to pixels as `v * ppem / upm`, so that units on the pixel grid give whole
    // numbers exactly.
    let px = |v: f64| v * em / upm;
    let whole = |v: f64| {
        let v = px(v).round();
        if v.is_finite() { (v as i64).clamp(-i64::from(strike::MAX_STRIKE_COORDINATE), i64::from(strike::MAX_STRIKE_COORDINATE)) as i32 } else { 0 }
    };
    let mut out = BitmapStrike::new(ppem, depth);
    out.ascent = whole(font.metric_pos(master, metric::ASCENDER, upm * 0.8));
    out.descent = whole(-font.metric_pos(master, metric::DESCENDER, -upm * 0.2));
    let style = fontcraft_render::Style { ink: [0, 0, 0, 255], background: [0, 0, 0, 0], anti_alias: true };
    let mut pixels = 0u64;
    let mut bytes = 0usize;
    for glyph in font.glyphs.iter().filter(|g| g.export) {
        let Some(layer) = glyph.layer(&master.id) else { continue };
        let outline = fontcraft_render::closed_contours(&font.outline_of(glyph, &master.id)?);
        let mut bitmap = StrikeGlyph {
            glyph: glyph.name.clone(),
            code: None,
            advance: whole(layer.width),
            scalable_width: None,
            width: 0,
            height: 0,
            left: 0,
            bottom: 0,
            data: Vec::new(),
        };
        if let Some((x0, y0, x1, y1)) = pixel_box(&outline, &px) {
            let side = i64::from(strike::MAX_STRIKE_BITMAP_SIDE);
            let (w, h) = (x1 - x0, y1 - y0);
            if w > side || h > side || [x0, y0].iter().any(|v| v.abs() > i64::from(strike::MAX_STRIKE_COORDINATE)) {
                return Err(p.bad(format!(
                    "strike {}: glyph `{}` would be {w} × {h} pixels; a bitmap is at most {side} on a side",
                    strike::label(ppem, depth),
                    glyph.name
                )));
            }
            let area = (w * h).unsigned_abs();
            pixels = pixels.saturating_add(area);
            if pixels > MAX_RASTER_PIXELS {
                return Err(p.bad(format!("strike {}: more than {MAX_RASTER_PIXELS} pixels to draw", strike::label(ppem, depth))));
            }
            let Some(left) = budget.pixels.checked_sub(area) else {
                return Err(p.bad(format!("more than {MAX_CALL_RASTER_PIXELS} pixels to draw for the strikes added together")));
            };
            budget.pixels = left;
            // Rows of `w` pixels at `depth` bits, each padded to a whole byte, as the model stores them.
            let size = usize::try_from((w * i64::from(depth) + 7) / 8 * h).unwrap_or(usize::MAX);
            bytes = bytes.saturating_add(size);
            if bytes > strike::MAX_STRIKE_BYTES {
                return Err(p.bad(format!(
                    "strike {}: the bitmaps would take more than {} bytes",
                    strike::label(ppem, depth),
                    strike::MAX_STRIKE_BYTES
                )));
            }
            let Some(left) = budget.bytes.checked_sub(size) else {
                return Err(p.bad(format!("the font's strikes would take more than {} bytes together", strike::MAX_ALL_STRIKES_BYTES)));
            };
            budget.bytes = left;
            if !draw {
                continue;
            }
            // `w` and `h` are 1..=1024 and the corners within the coordinate limit.
            let (w, h) = (w as u32, h as u32);
            let scale = em / upm;
            let at = Affine::new([scale, 0.0, 0.0, -scale, -(x0 as f64), y1 as f64]);
            let image = fontcraft_render::render_path(&outline, at, w, h, &style).map_err(|e| p.bad(format!("glyph `{}`: {e}", glyph.name)))?;
            bitmap.width = w as u16;
            bitmap.height = h as u16;
            bitmap.left = x0 as i32;
            bitmap.bottom = y0 as i32;
            bitmap.data = pack(&image, depth);
        }
        out.glyphs.push(bitmap);
    }
    Ok(out)
}

/// The pixel box of an outline, `(left, bottom, right, top)` in pixels up from the baseline;
/// `None` when it has no area.
fn pixel_box(outline: &fontcraft_geom::BezPath, px: &dyn Fn(f64) -> f64) -> Option<(i64, i64, i64, i64)> {
    if outline.elements().is_empty() {
        return None;
    }
    let b = outline.bounding_box();
    // Snap values a rounding error away from a whole pixel, so a box on the grid gains no row.
    let snap = |v: f64| if (v - v.round()).abs() < 1e-6 { v.round() } else { v };
    let (x0, y0, x1, y1) = (snap(px(b.x0)).floor(), snap(px(b.y0)).floor(), snap(px(b.x1)).ceil(), snap(px(b.y1)).ceil());
    let limit = 1e9;
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite() && v.abs() < limit) || x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0 as i64, y0 as i64, x1 as i64, y1 as i64))
}

/// The coverage of a render as rows of `depth`-bit levels, packed from the most significant
/// bit. At 1 bit a pixel is on from half coverage; deeper levels round the coverage.
fn pack(image: &fontcraft_render::Image, depth: u8) -> Vec<u8> {
    let d = u32::from(depth);
    let top = (1u32 << d) - 1;
    let stride = (image.width as usize * d as usize).div_ceil(8);
    let mut out = vec![0u8; stride * image.height as usize];
    for y in 0..image.height {
        for x in 0..image.width {
            let alpha = image.pixel(x, y).map_or(0, |[_, _, _, a]| u32::from(a));
            let level = if d == 1 { u32::from(alpha >= 128) } else { (alpha * top + 127) / 255 };
            let bit = x as usize * d as usize;
            let shift = 8 - d as usize - bit % 8;
            if let Some(byte) = out.get_mut(y as usize * stride + bit / 8) {
                *byte |= ((level & top) << shift) as u8;
            }
        }
    }
    out
}
