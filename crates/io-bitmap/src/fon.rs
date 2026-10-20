//! Windows FON files: a 16-bit (NE) executable whose resources hold FNT fonts, usually one
//! face at several sizes.
//!
//! Written from Microsoft's published descriptions of the NE executable header and its resource
//! table, and of the font directory resource, and checked by opening files of our own in
//! FreeType as a black box. No code that reads or writes the format was consulted.
//!
//! The file starts with a DOS header whose word at 0x3C points at the NE header. The NE header
//! points at the resource table: an alignment shift, then for each resource type its id, its
//! count and one entry per resource (offset and length in units of 2^shift bytes, flags, id),
//! ended by a zero type id. Type 0x8008 holds the fonts, each an FNT file whose offsets count
//! from the resource's start; type 0x8007 holds the font directory, which repeats the start of
//! each font's header for the system's font loader.

use crate::fnt::{self, DIRECTORY_ENTRY_BYTES, Written, u16_at, u32_at};
use crate::{BitmapError, BitmapFont, Import, MAX_FILE_BYTES, PixelBudget, Result, Warnings};

/// Most fonts one FON file may hold: as many strikes as a font keeps.
pub const MAX_FON_FONTS: usize = 64;
/// Most resource types the resource table may list.
const MAX_RESOURCE_TYPES: usize = 256;
/// Largest resource alignment shift read.
const MAX_SHIFT: u16 = 15;
const RT_FONTDIR: u16 = 0x8007;
const RT_FONT: u16 = 0x8008;
const DOS_HEADER: usize = 64;
const NE_HEADER: usize = 0x40;

fn bad(message: impl Into<String>) -> BitmapError {
    BitmapError::Fon(message.into())
}

fn ne_offset(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(b"MZ") {
        return None;
    }
    u32_at(bytes, 0x3C).map(|at| at as usize)
}

/// Whether `bytes` are a 16-bit Windows executable, the container of FON fonts.
pub fn is_fon(bytes: &[u8]) -> bool {
    ne_offset(bytes).is_some_and(|at| at.checked_add(2).and_then(|end| bytes.get(at..end)) == Some(b"NE".as_slice()))
}

/// Reads every font of a FON file, in resource table order. A font that cannot be read is left
/// out with a warning on the first one read; the file is refused when none can be read.
pub fn read_fon(bytes: &[u8]) -> Result<Vec<Import>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(BitmapError::TooLarge(bytes.len()));
    }
    let ne = ne_offset(bytes).ok_or_else(|| bad("the file does not start with a DOS executable header"))?;
    match ne.checked_add(2).and_then(|end| bytes.get(ne..end)) {
        Some(b"NE") => {}
        Some(b"PE") => return Err(bad("it is a 32-bit executable; fonts are read from 16-bit (NE) font files only")),
        _ => return Err(bad("there is no NE header where the DOS header points")),
    }
    let cut = || bad("the file ends inside its resource table");
    let table = ne.checked_add(0x24).and_then(|at| u16_at(bytes, at)).ok_or_else(|| bad("the file ends inside its NE header"))?;
    let table = ne + usize::from(table);
    let shift = u16_at(bytes, table).ok_or_else(cut)?;
    if shift > MAX_SHIFT {
        return Err(bad(format!("the resource alignment shift {shift} is above {MAX_SHIFT}")));
    }
    let mut fonts: Vec<(usize, usize)> = Vec::new();
    let mut at = table + 2;
    let mut ended = false;
    for _ in 0..MAX_RESOURCE_TYPES {
        let kind = u16_at(bytes, at).ok_or_else(cut)?;
        if kind == 0 {
            ended = true;
            break;
        }
        let count = usize::from(u16_at(bytes, at + 2).ok_or_else(cut)?);
        let entries = at + 8;
        let end = entries + 12 * count;
        if end > bytes.len() {
            return Err(cut());
        }
        if kind == RT_FONT {
            for i in 0..count {
                let entry = entries + 12 * i;
                let (Some(offset), Some(length)) = (u16_at(bytes, entry), u16_at(bytes, entry + 2)) else {
                    return Err(cut());
                };
                if fonts.len() == MAX_FON_FONTS {
                    return Err(BitmapError::Limit { what: "fonts in a FON file", limit: MAX_FON_FONTS });
                }
                fonts.push((usize::from(offset) << shift, usize::from(length) << shift));
            }
        }
        at = end;
    }
    if !ended {
        return Err(BitmapError::Limit { what: "FON resource types", limit: MAX_RESOURCE_TYPES });
    }
    if fonts.is_empty() {
        return Err(bad("it holds no font resources"));
    }
    let mut budget = PixelBudget::default();
    let mut out: Vec<Import> = Vec::with_capacity(fonts.len());
    let mut failures = Vec::new();
    for (i, (offset, length)) in fonts.into_iter().enumerate() {
        // The last resource's length is rounded up to the alignment and may pass the file's end.
        let end = offset.saturating_add(length).min(bytes.len());
        let Some(resource) = bytes.get(offset..end) else {
            failures.push(format!("font {} lies outside the file", i + 1));
            continue;
        };
        let mut warnings = Warnings::default();
        match fnt::parse(resource, &mut warnings, &mut budget) {
            Ok(font) => out.push(Import { font, warnings: warnings.finish() }),
            Err(e @ (BitmapError::Limit { .. } | BitmapError::TooLarge(_))) => return Err(e),
            Err(e) => failures.push(format!("font {}: {e}", i + 1)),
        }
    }
    let Some(first) = out.first_mut() else {
        let reason = failures.into_iter().next().unwrap_or_default();
        return Err(bad(format!("none of its fonts can be read ({reason})")));
    };
    for failure in failures {
        if first.warnings.len() < crate::MAX_WARNINGS {
            first.warnings.push(format!("{failure}; it was left out"));
        }
    }
    Ok(out)
}

fn push16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// A length-prefixed string of at most 255 printable ASCII bytes.
fn pascal(text: &str) -> Vec<u8> {
    let bytes: Vec<u8> = text.bytes().filter(|b| (0x20..0x7F).contains(b)).take(255).collect();
    let mut out = vec![u8::try_from(bytes.len()).unwrap_or(u8::MAX)];
    out.extend(bytes);
    out
}

/// The face name an FNT font holds.
fn face_of(font: &[u8]) -> Vec<u8> {
    let at = u32_at(font, 105).map_or(0, |at| at as usize);
    font.get(at..).unwrap_or_default().iter().take_while(|b| **b != 0).copied().collect()
}

/// Writes black-and-white strikes as one FON file, one FNT font (version 3) per strike, in the
/// order given. Each strike is written as [`crate::write_fnt`] writes it; the warnings name the
/// strike they come from.
pub fn write_fon(fonts: &[BitmapFont]) -> Result<Written> {
    if fonts.is_empty() {
        return Err(BitmapError::Write("there is no strike to write".into()));
    }
    if fonts.len() > MAX_FON_FONTS {
        return Err(BitmapError::Write(format!("a FON file holds at most {MAX_FON_FONTS} fonts")));
    }
    let mut warnings = Warnings::default();
    let mut fnts = Vec::with_capacity(fonts.len());
    for font in fonts {
        let mut own = Warnings::default();
        let bytes = fnt::build(font, &mut own).map_err(|e| match e {
            BitmapError::Write(m) => BitmapError::Write(format!("the {} pixel strike: {m}", font.pixel_size)),
            other => other,
        })?;
        for m in own.finish() {
            warnings.push(format!("the {} pixel strike: {m}", font.pixel_size));
        }
        fnts.push(bytes);
    }
    let first = fnts.first().map(Vec::as_slice).unwrap_or_default();
    let face = String::from_utf8_lossy(&face_of(first)).into_owned();
    let (vres, hres) = (u16_at(first, 70).unwrap_or(96).max(1), u16_at(first, 72).unwrap_or(96).max(1));

    let mut directory = Vec::new();
    push16(&mut directory, u16::try_from(fnts.len()).unwrap_or(0));
    for (i, font) in fnts.iter().enumerate() {
        push16(&mut directory, u16::try_from(i + 1).unwrap_or(0));
        directory.extend_from_slice(font.get(..DIRECTORY_ENTRY_BYTES).unwrap_or_default());
        directory.push(0);
        directory.extend(face_of(font));
        directory.push(0);
    }

    let mut module: String = face.chars().filter(char::is_ascii_alphanumeric).take(8).collect::<String>().to_ascii_uppercase();
    if module.is_empty() {
        module = "FONT".into();
    }
    let points: Vec<String> = fnts.iter().map(|f| u16_at(f, 68).unwrap_or(0).to_string()).collect();
    let aspect = u32::from(vres) * 100 / u32::from(hres);
    let description = format!("FONTRES {aspect},{hres},{vres} : {face} {}", points.join(", "));

    // The tables that follow the NE header, at offsets from its start.
    let fonts_count = fnts.len();
    let resource_table = NE_HEADER;
    let names_at = 2 + (8 + 12) + (8 + 12 * fonts_count) + 2;
    let fontdir_name = pascal("FONTDIR");
    let resource_len = names_at + fontdir_name.len() + 1;
    let mut resident = pascal(&module);
    resident.extend_from_slice(&[0, 0, 0]);
    let resident_at = resource_table + resource_len;
    let imported_at = resident_at + resident.len();
    let entry_at = imported_at + 1;
    let entry = [0u8, 0];
    let mut nonresident = pascal(&description);
    nonresident.extend_from_slice(&[0, 0, 0]);
    let nonresident_at = DOS_HEADER + entry_at + entry.len();
    let data_at = nonresident_at + nonresident.len();

    // The smallest alignment whose units can address every resource.
    let blobs: Vec<&[u8]> = std::iter::once(directory.as_slice()).chain(fnts.iter().map(Vec::as_slice)).collect();
    let mut layout = None;
    for shift in 4..=MAX_SHIFT {
        let unit = 1usize << shift;
        let align = |x: usize| x.div_ceil(unit) * unit;
        let mut at = align(data_at);
        let mut places = Vec::with_capacity(blobs.len());
        let mut fits = true;
        for blob in &blobs {
            let (units_at, units_len) = (at >> shift, align(blob.len()) >> shift);
            fits &= units_at <= usize::from(u16::MAX) && units_len <= usize::from(u16::MAX);
            places.push((at, units_at, units_len));
            at = align(at + blob.len());
        }
        if fits {
            layout = Some((shift, places, at));
            break;
        }
    }
    let Some((shift, places, size)) = layout else {
        return Err(BitmapError::Write("the strikes are too large for one FON file".into()));
    };
    if size > MAX_FILE_BYTES {
        return Err(BitmapError::Write(format!("the FON file would take {size} bytes; the limit is {MAX_FILE_BYTES}")));
    }
    let unit16 = |v: usize| u16::try_from(v).unwrap_or(u16::MAX);
    let starts: Vec<usize> = places.iter().map(|(at, _, _)| *at).collect();

    let mut resources = Vec::with_capacity(resource_len);
    push16(&mut resources, shift);
    let mut places = places.into_iter();
    push16(&mut resources, RT_FONTDIR);
    push16(&mut resources, 1);
    push32(&mut resources, 0);
    if let Some((_, units_at, units_len)) = places.next() {
        push16(&mut resources, unit16(units_at));
        push16(&mut resources, unit16(units_len));
        // Moveable and preloaded; named by the string at `names_at`.
        push16(&mut resources, 0x0050);
        push16(&mut resources, unit16(names_at));
        push32(&mut resources, 0);
    }
    push16(&mut resources, RT_FONT);
    push16(&mut resources, unit16(fonts_count));
    push32(&mut resources, 0);
    for (i, (_, units_at, units_len)) in places.enumerate() {
        push16(&mut resources, unit16(units_at));
        push16(&mut resources, unit16(units_len));
        // Moveable, pure and discardable; numbered from 1.
        push16(&mut resources, 0x1030);
        push16(&mut resources, 0x8000 | unit16(i + 1));
        push32(&mut resources, 0);
    }
    push16(&mut resources, 0);
    resources.extend_from_slice(&fontdir_name);
    resources.push(0);

    let mut out = vec![0u8; DOS_HEADER];
    if let Some(m) = out.get_mut(..2) {
        m.copy_from_slice(b"MZ");
    }
    if let Some(field) = out.get_mut(0x18..0x1A) {
        field.copy_from_slice(&0x40u16.to_le_bytes());
    }
    if let Some(field) = out.get_mut(0x3C..0x40) {
        field.copy_from_slice(&u32::try_from(DOS_HEADER).unwrap_or(0).to_le_bytes());
    }
    out.extend_from_slice(b"NE");
    out.extend_from_slice(&[5, 10]);
    push16(&mut out, unit16(entry_at));
    push16(&mut out, unit16(entry.len()));
    push32(&mut out, 0);
    // A library module.
    push16(&mut out, 0x8000);
    // No automatic data segment, heap, stack or entry point.
    out.extend_from_slice(&[0; 14]);
    push16(&mut out, 0); // segments
    push16(&mut out, 0); // module references
    push16(&mut out, unit16(nonresident.len()));
    push16(&mut out, unit16(NE_HEADER)); // segment table
    push16(&mut out, unit16(resource_table));
    push16(&mut out, unit16(resident_at));
    push16(&mut out, unit16(imported_at)); // module reference table, empty
    push16(&mut out, unit16(imported_at));
    push32(&mut out, u32::try_from(nonresident_at).unwrap_or(0));
    push16(&mut out, 0); // moveable entry points
    push16(&mut out, shift);
    push16(&mut out, 0); // resource segments
    out.extend_from_slice(&[2, 0]); // Windows; no other flags
    out.extend_from_slice(&[0; 6]);
    push16(&mut out, 0x0300); // Windows 3.0
    out.extend_from_slice(&resources);
    out.extend_from_slice(&resident);
    out.push(0);
    out.extend_from_slice(&entry);
    out.extend_from_slice(&nonresident);
    for (at, blob) in starts.into_iter().zip(&blobs) {
        out.resize(at, 0);
        out.extend_from_slice(blob);
    }
    out.resize(size, 0);
    Ok(Written { bytes: out, warnings: warnings.finish() })
}
