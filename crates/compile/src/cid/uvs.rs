//! Unicode Variation Sequences files and the `cmap` format 14 subtable made from them (the
//! reference compiler's `-ci`; row `compile.cidCmapResources`).
//!
//! The file format and the subtable follow AFDKO 5.0.1 (Apache-2.0, see `NOTICE`):
//! `c/addfeatures/hotconv/map.cpp` (`mapAddUVS`, `makeUVScmap`) and `cmap.cpp`. One sequence per
//! line: the base character and the variation selector in hexadecimal, then the glyph. Fields
//! are separated by blanks and semicolons; `#` starts a comment line. A CID-keyed font names its
//! glyph `CID+<number>` after a character collection field (`Adobe-Japan1; CID+1234`); another
//! font names it by glyph name, with or without a field before it. A sequence whose glyph is
//! the one `cmap` maps the base character to is a default sequence; the others name their
//! glyph. Lines that cannot be read and glyphs the font does not have are skipped with a warning.

use std::collections::{BTreeMap, HashMap};

use read_fonts::{FontRef, TableProvider};
use write_fonts::FontBuilder;
use write_fonts::types::Tag;

use super::{CidPlan, final_cids, list, with_subtable};
use crate::{CompileError, Compiled};

/// Most sequences one file may give.
pub const MAX_SEQUENCES: usize = 1 << 20;
/// Longest line read, in bytes; the reference reads 255.
const MAX_LINE: usize = 255;

/// The glyph a sequence selects.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Cid(u32),
    Name(String),
}

/// One line of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sequence {
    base: u32,
    selector: u32,
    target: Target,
}

/// Is `field` a glyph name, a hexadecimal number or a collection name as the reference scans
/// them: letters, digits, `_`, `-`, `+` and periods, but no period followed by a digit-only tail.
fn scans(field: &str) -> bool {
    let mut dot_run = false;
    for c in field.chars() {
        match c {
            '.' => dot_run = true,
            c if c.is_ascii_digit() => {}
            c if c.is_ascii_alphabetic() || matches!(c, '_' | '-' | '+') => dot_run = false,
            _ => return false,
        }
    }
    // A field that ends in a period and digits only after its last letter is refused.
    !dot_run
}

/// The sequences of a file, with a warning for each line that cannot be read.
fn parse(bytes: &[u8], cid_keyed: bool, warnings: &mut Vec<String>) -> Result<Vec<Sequence>, String> {
    if bytes.len() > super::MAX_CMAP_BYTES {
        return Err(format!("the file is {} bytes long; at most {} are read", bytes.len(), super::MAX_CMAP_BYTES));
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if raw.len() > MAX_LINE {
            skipped.push(format!("line {line_no} is longer than {MAX_LINE} bytes"));
            continue;
        }
        let fields: Vec<&str> = line.split(|c: char| c.is_whitespace() || c == ';').filter(|f| !f.is_empty()).collect();
        let hex =
            |f: Option<&&str>| f.filter(|f| scans(f)).and_then(|f| u32::from_str_radix(f.trim_start_matches("0x").trim_start_matches("0X"), 16).ok());
        let (Some(base), Some(selector)) = (hex(fields.first()), hex(fields.get(1))) else {
            skipped.push(format!("line {line_no} does not start with a character and a variation selector"));
            continue;
        };
        let glyph = match (fields.get(2), fields.get(3)) {
            (_, Some(g)) if scans(g) => *g,
            (Some(g), _) if !cid_keyed && scans(g) => *g,
            _ => {
                skipped.push(if cid_keyed {
                    format!("line {line_no} does not end in a character collection and CID+<number>")
                } else {
                    format!("line {line_no} names no glyph")
                });
                continue;
            }
        };
        let target = match glyph.strip_prefix("CID+") {
            Some(n) => match n.parse::<u32>() {
                Ok(cid) => Target::Cid(cid),
                Err(_) => {
                    skipped.push(format!("line {line_no}: `{glyph}` is not CID+<number>"));
                    continue;
                }
            },
            None => Target::Name(glyph.to_string()),
        };
        if out.len() >= MAX_SEQUENCES {
            return Err(format!("the file gives more than {MAX_SEQUENCES} sequences"));
        }
        out.push(Sequence { base, selector, target });
    }
    if !skipped.is_empty() {
        warnings.push(format!("{} lines of the Unicode Variation Sequences file were skipped: {}", skipped.len(), list(&skipped)));
    }
    Ok(out)
}

/// A format 14 subtable: for each selector in ascending order its default sequences (ranges of
/// base characters, at most 256 in one range) and its other sequences (base character and
/// glyph). `None` when there is no sequence.
fn format14(entries: &BTreeMap<(u32, u32), Option<u16>>) -> Option<Vec<u8>> {
    let mut selectors: BTreeMap<u32, (Vec<(u32, u8)>, Vec<(u32, u16)>)> = BTreeMap::new();
    for ((selector, base), glyph) in entries {
        let (ranges, glyphs) = selectors.entry(*selector).or_default();
        match glyph {
            None => match ranges.last_mut() {
                Some((start, extra)) if *extra < u8::MAX && start.checked_add(u32::from(*extra) + 1) == Some(*base) => *extra += 1,
                _ => ranges.push((*base, 0)),
            },
            Some(gid) => glyphs.push((*base, *gid)),
        }
    }
    if selectors.is_empty() {
        return None;
    }
    let u24 = |v: u32| v.to_be_bytes().get(1..).map(<[u8]>::to_vec).unwrap_or_default();
    let count = u32::try_from(selectors.len()).ok()?;
    let mut tail: Vec<u8> = Vec::new();
    let mut records: Vec<u8> = Vec::new();
    let head = 10usize.checked_add(selectors.len().checked_mul(11)?)?;
    for (selector, (ranges, glyphs)) in &selectors {
        let mut offsets = [0u32; 2];
        if !ranges.is_empty() {
            offsets[0] = u32::try_from(head.checked_add(tail.len())?).ok()?;
            tail.extend_from_slice(&u32::try_from(ranges.len()).ok()?.to_be_bytes());
            for (start, extra) in ranges {
                tail.extend(u24(*start));
                tail.push(*extra);
            }
        }
        if !glyphs.is_empty() {
            offsets[1] = u32::try_from(head.checked_add(tail.len())?).ok()?;
            tail.extend_from_slice(&u32::try_from(glyphs.len()).ok()?.to_be_bytes());
            for (base, gid) in glyphs {
                tail.extend(u24(*base));
                tail.extend_from_slice(&gid.to_be_bytes());
            }
        }
        records.extend(u24(*selector));
        records.extend_from_slice(&offsets[0].to_be_bytes());
        records.extend_from_slice(&offsets[1].to_be_bytes());
    }
    let length = u32::try_from(head.checked_add(tail.len())?).ok()?;
    let mut out = Vec::with_capacity(head + tail.len());
    out.extend_from_slice(&14u16.to_be_bytes());
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend(records);
    out.extend(tail);
    Some(out)
}

/// Adds the format 14 subtable (platform 0, encoding 5) the file `bytes` describes to the built
/// font's `cmap`. `plan` is the font's CID plan when it is CID-keyed; glyph names are matched
/// against the source names of the final glyph order.
pub(crate) fn finish(compiled: &mut Compiled, bytes: &[u8], plan: Option<&CidPlan>) -> Result<(), CompileError> {
    let table_error = |e: String| CompileError::Table { table: "cmap", message: e };
    let sequences = parse(bytes, plan.is_some(), &mut compiled.warnings)
        .map_err(|e| CompileError::Cid(format!("the Unicode Variation Sequences file cannot be read: {e}")))?;
    let gid = |i: usize| u16::try_from(i).ok();
    let by_name: HashMap<&str, u16> = compiled.glyph_order.iter().enumerate().filter_map(|(i, n)| Some((n.as_str(), gid(i)?))).collect();
    let by_cid: HashMap<u32, u16> = match plan {
        Some(plan) => final_cids(plan, compiled.glyph_order.iter().map(String::as_str))
            .into_iter()
            .enumerate()
            .filter_map(|(i, c)| Some((c, gid(i)?)))
            .collect(),
        None => HashMap::new(),
    };
    let font = FontRef::new(&compiled.bytes).map_err(|e| table_error(e.to_string()))?;
    let charmap = font.cmap().map_err(|e| table_error(e.to_string()))?;
    let mapped = |c: u32| charmap.map_codepoint(c).map(|g| g.to_u32());
    let mut entries: BTreeMap<(u32, u32), Option<u16>> = BTreeMap::new();
    let mut missing = Vec::new();
    for s in &sequences {
        let found = match &s.target {
            Target::Cid(c) => by_cid.get(c).copied(),
            Target::Name(n) if plan.is_none() => by_name.get(n.as_str()).copied(),
            Target::Name(_) => None,
        };
        let Some(g) = found else {
            missing.push(match &s.target {
                Target::Cid(c) => format!("CID+{c}"),
                Target::Name(n) => n.clone(),
            });
            continue;
        };
        let default = mapped(s.base) == Some(u32::from(g));
        // The first line for a character and selector counts.
        entries.entry((s.selector, s.base)).or_insert(if default { None } else { Some(g) });
    }
    if !missing.is_empty() {
        compiled.warnings.push(format!("the Unicode Variation Sequences of {} are skipped: the font has no such glyph", list(&missing)));
    }
    let Some(subtable) = format14(&entries) else {
        compiled.warnings.push("no Unicode Variation Sequence reaches a glyph of the font, so the cmap has no format 14 subtable".into());
        return Ok(());
    };
    let old = font.table_data(Tag::new(b"cmap")).map(|d| d.as_bytes().to_vec()).ok_or_else(|| table_error("the font has no cmap".into()))?;
    let cmap = with_subtable(&old, 0, 5, &subtable).map_err(table_error)?;
    let mut builder = FontBuilder::new();
    for record in font.table_directory().table_records() {
        let tag = record.tag();
        if tag == Tag::new(b"cmap") {
            continue;
        }
        if let Some(data) = font.table_data(tag) {
            builder.add_raw(tag, data.as_bytes().to_vec());
        }
    }
    builder.add_raw(Tag::new(b"cmap"), cmap);
    compiled.bytes = builder.build();
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// The subtable the reference wrote for these sequences, with `A` (glyph 2), `B` (3), `A.alt`
    /// (4), `uni8FBB` (5) and `uni8FBB.alt` (6), `C` unmapped and `nothere` missing (makeotf
    /// 5.0.1's addfeatures `-ci`).
    #[test]
    fn variation_sequences_are_written_as_the_reference_writes_them() {
        let file = "# test\n8FBB E0101; uni8FBB.alt\n8FBB E0100; uni8FBB\n0041 FE00; A.alt\n0042 FE00; B\n0043 FE00; B\n9999 FE00; nothere\n0041 FE01;Test;A\n";
        let mut warnings = Vec::new();
        let sequences = parse(file.as_bytes(), false, &mut warnings).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let gid = |n: &str| ["A", "B", "A.alt", "uni8FBB", "uni8FBB.alt"].iter().position(|g| *g == n).map(|i| u16::try_from(i).unwrap() + 2);
        let cmap = |c: u32| match c {
            0x41 => Some(2),
            0x42 => Some(3),
            0x8FBB => Some(5),
            _ => None,
        };
        let mut entries = BTreeMap::new();
        for s in sequences {
            let Target::Name(n) = &s.target else { panic!() };
            let Some(g) = gid(n) else { continue };
            entries.entry((s.selector, s.base)).or_insert(if cmap(s.base) == Some(g) { None } else { Some(g) });
        }
        let expected = "000e000000650000000400fe00000000360000003e00fe010000004c000000000e010000000054000000000e0101000000000000005c00000001000042000000000200004100040000430003000000010000410000000001008fbb0000000001008fbb0006";
        let hex: String = format14(&entries).unwrap().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, expected);
    }

    #[test]
    fn default_sequences_on_consecutive_characters_share_a_range() {
        let entries: BTreeMap<(u32, u32), Option<u16>> = (0x4E00..0x4E03).map(|c| ((0xFE00, c), None)).collect();
        let bytes = format14(&entries).unwrap();
        // One range: 4E00 and two more.
        assert_eq!(&bytes[21..29], &[0, 0, 0, 1, 0x00, 0x4E, 0x00, 2]);
    }

    #[test]
    fn a_cid_keyed_line_needs_its_collection_field() {
        let mut warnings = Vec::new();
        let s = parse(b"8FBB E0100; Adobe-Japan1; CID+1234\n8FBB E0101; CID+5\n", true, &mut warnings).unwrap();
        assert_eq!(s, vec![Sequence { base: 0x8FBB, selector: 0xE0100, target: Target::Cid(1234) }]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
