//! Bitmap strikes of the open font (M13.55b): `bitmap.importStrikes` reads BDF and PCF files into
//! strikes, `bitmap.exportStrikes` writes strikes as BDF or PCF files, and `bitmap.setStrikes`
//! adds empty strikes and removes strikes.
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
use fontcraft_io_bitmap::{Bitmap, BitmapFont, BitmapGlyph, PcfLayout, Property, PropertyValue};
use fontcraft_model::strike::{self, BitmapStrike, StrikeGlyph, StrikeProperty, StrikeValue};
use fontcraft_model::{Font, MAX_NAME_LEN};

/// Most glyph names a result lists as left out, per file.
const MAX_LISTED: usize = 100;
/// Resolution assumed for a strike that states none, in dots per inch: a point is then a pixel.
const DEFAULT_DPI: u32 = 72;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "bitmap.importStrikes",
            "Import Bitmap Strikes…",
            ["File", "Import"],
            None,
            "{paths: [path of a .bdf or .pcf file], asBackground?: false} adds the strike each file holds to the font, as one undo step. A bitmap goes to the glyph whose code point is the file's code for it (when the file's codes are Unicode), else to the glyph of its name; bitmaps that match no glyph are left out and listed, and no glyph is created. A strike of the same size and depth as one the font has replaces that strike's name, metrics and properties and the bitmaps of the glyphs it holds; the strike's other bitmaps stay. Refused, changing nothing: a file that is not BDF or PCF, a gzip-compressed PCF, a file beyond the bitmap readers' limits, a file none of whose bitmaps match a glyph, asBackground (not built) → {strikes: [{path, ppem, depth, label, glyphs, replaced, leftOut: [name], leftOutCount}], warnings: [string]}",
            has_doc,
            import_strikes
        )
        .files(const { &[FileParam::read("paths")] }),
        cmd!(
            "bitmap.exportStrikes",
            "Export Bitmap Strikes…",
            ["File", "Export"],
            None,
            "{strikes?: [{ppem: number, depth?: number}] (default: all; without depth, every depth of the size), format?: \"bdf\" | \"pcf\" (default: pcf for a path ending in .pcf, else bdf), path?: file (exactly one strike), destination?: folder (one file per strike, named Family-12.bdf, or Family-12@4.bdf for a grey strike)} writes strikes as bitmap font files, atomically each. A bitmap's code is the one its file had, else the first code point of its glyph; a strike without a stated point size or resolution is written at 72 dpi, and one without a name takes the family name. PCF holds black-and-white strikes only. The formats embedded and otb are not built → {format, files: [{strike, path, glyphs, bytes}]}",
            has_doc,
            export_strikes
        )
        .files(const { &[FileParam::write("path"), FileParam::write("destination")] })
        .writes_files(),
        cmd!(
            "bitmap.setStrikes",
            "Bitmap Strikes…",
            ["Font"],
            None,
            "{add?: [{ppem: number, depth?: 1 | 2 | 4 | 8 (default 1)}], remove?: [{ppem: number, depth?: number (default: every depth of the size)}], rasterize?: bool (default true)} adds and removes the font's bitmap strikes as one undo step; removing a strike deletes its bitmaps. Filling a new strike from the outlines is not built yet, so adding needs rasterize: false and gives an empty strike. Refused, changing nothing: neither list, a strike to add that the font has, a strike to remove that it lacks → {added: [label], removed: [label], strikes: [label]}",
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
        return Err(p.bad("`paths` is empty: choose a BDF or PCF file"));
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
        let import = fontcraft_io_bitmap::read(&input.bytes).map_err(|e| p.bad(format!("{path}: {e}")))?;
        warnings.extend(import.warnings.into_iter().map(|w| format!("{path}: {w}")));
        files.push((path.to_string(), import.font));
    }
    let font = s.font()?;
    let mut incoming = Vec::with_capacity(files.len());
    for (path, bitmap_font) in &files {
        let one = to_strike(font, path, bitmap_font, &mut warnings);
        if one.strike.glyphs.is_empty() {
            return Err(p.bad(format!("{path}: none of its {} bitmaps matches a glyph of the font by code or name", bitmap_font.glyphs.len())));
        }
        incoming.push(one);
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
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Format::Bdf => "bdf",
            Format::Pcf => "pcf",
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
        Some("embedded" | "otb") => {
            return Err(EngineError::Unsupported("embedding strikes in a compiled font or writing a bitmap-only sfnt".into()));
        }
        Some(other) => return Err(p.bad(format!("`format` must be bdf or pcf, got `{}`", other.chars().take(32).collect::<String>()))),
        None if path.is_some_and(|x| x.to_ascii_lowercase().ends_with(".pcf")) => Format::Pcf,
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
        let bytes = match format {
            Format::Bdf => fontcraft_io_bitmap::write_bdf(&file).map(String::into_bytes),
            Format::Pcf => fontcraft_io_bitmap::write_pcf(&file, PcfLayout::default()),
        }
        .map_err(|e| p.bad(format!("strike {}: {e}", strike.label())))?;
        out.push((strike.label(), strike.glyphs.len(), bytes));
    }
    let mut files = Vec::with_capacity(out.len());
    for ((label, glyphs, bytes), target) in out.into_iter().zip(targets) {
        write_file(&target, &bytes)?;
        files.push(json!({"strike": label, "path": target.to_string_lossy(), "glyphs": glyphs, "bytes": bytes.len()}));
    }
    Ok(json!({ "format": format.extension(), "files": files }))
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
    if !add.is_empty() && p.bool_or("rasterize", true)? {
        return Err(EngineError::Unsupported("filling a new strike from the outlines; pass rasterize: false to add an empty strike".into()));
    }
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
    let strikes = s.edit("Bitmap Strikes", |font| {
        let strikes = std::sync::Arc::make_mut(&mut font.bitmap_strikes);
        strikes.retain(|s| !remove.iter().any(|r| r.matches(s)));
        strikes.extend(added.iter().map(|(ppem, depth)| BitmapStrike::new(*ppem, *depth)));
        sort_strikes(strikes);
        Ok(labels(strikes))
    })?;
    let added: Vec<String> = added.iter().map(|(ppem, depth)| strike::label(*ppem, *depth)).collect();
    Ok(json!({ "added": added, "removed": removed, "strikes": strikes }))
}
