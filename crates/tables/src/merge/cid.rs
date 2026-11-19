//! CID-keyed results of a glyph merge (M12.69), after AFDKO mergefonts' documented behaviour
//! (its `-h` help text).
//!
//! The result is CID-keyed when the fonts are CFF and one of these holds: a cidfontinfo file is
//! given ([`CidInfo`]), a map gives numeric output names, or the first font is CID-keyed.
//!
//! - Every glyph is addressed by its CID: a numeric map name (`123`, or `cid00123`), or a
//!   CID-keyed font's own CIDs where that font has no map. A name-keyed font needs a map.
//! - Each input Font DICT becomes one Font DICT of the result, with its own Private DICT: a
//!   name-keyed font gives one, a CID-keyed font one for each of its Font DICTs the merge takes
//!   glyphs from. Glyphs keep their charstrings, widths and hints as they are.
//! - A Font DICT is named by its font's map header, else by its own `FontName` (CID-keyed
//!   input) or the font's name; the map header's language group sets its `LanguageGroup`.
//! - The Top DICT values come from the cidfontinfo file. Without one, they come from the first
//!   font: its ROS when it is CID-keyed, `Adobe-Identity-0` otherwise.
//! - Glyphs are in CID order; CID 0 (`.notdef`) must be among them.

use std::collections::{BTreeSet, HashMap};

use super::MapHeader;
use super::cff::{Built, Expanded, finish};
use super::plan::Plan;
use crate::limits::MAX_LIST_LINE;
use crate::reorder::cff::{self as cff, CidFontDict, CidGlyph, CidTop, assemble_cid};
use crate::reorder::names::num_glyphs;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Highest CID (`CIDCount` is at most 65,535, counting CID 0).
const MAX_CID: u32 = fontcraft_cff::cid::MAX_CID;
/// Most numbers an `XUID` line may hold.
const MAX_XUID: usize = 16;
/// Most lines a cidfontinfo file may hold.
const MAX_INFO_LINES: usize = 10_000;

/// Top DICT `version`, `Notice`, `FullName`, `FamilyName`, `Weight`, `XUID`.
const OP_VERSION: u16 = 0;
const OP_NOTICE: u16 = 1;
const OP_FULL_NAME: u16 = 2;
const OP_FAMILY_NAME: u16 = 3;
const OP_WEIGHT: u16 = 4;
const OP_XUID: u16 = 14;

/// The values of a cidfontinfo file that the CID-keyed result's Top DICT takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CidInfo {
    /// The CID font name (the Name INDEX entry).
    pub font_name: String,
    pub full_name: String,
    pub family_name: String,
    pub weight: String,
    pub version: String,
    pub registry: String,
    pub ordering: String,
    pub supplement: i64,
    /// `AdobeCopyright`.
    pub copyright: String,
    pub trademark: Option<String>,
    pub xuid: Vec<i64>,
}

/// Reads a cidfontinfo file: one `Key value` pair per line, a text value in parentheses.
/// FontName, FullName, FamilyName, Weight, version, Registry, Ordering, Supplement and
/// AdobeCopyright are required; Trademark and XUID are optional; other keys (the file also
/// serves the font compiler) are ignored. Lines starting with `%` or `#` are comments.
pub fn parse_cid_info(text: &str) -> Result<CidInfo> {
    let mut values: HashMap<&str, String> = HashMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        if line_no > MAX_INFO_LINES {
            return Err(TablesError::TooLarge { what: "the lines of the cidfontinfo file", limit: MAX_INFO_LINES });
        }
        if raw.len() > MAX_LIST_LINE {
            return Err(TablesError::BadOptions(format!("line {line_no} of the cidfontinfo file is longer than {MAX_LIST_LINE} bytes")));
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('%') || line.starts_with('#') {
            continue;
        }
        let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let value = match (rest.find('('), rest.rfind(')')) {
            (Some(open), Some(close)) if open < close => rest.get(open + 1..close).unwrap_or_default().to_string(),
            _ => rest.trim_matches(['[', ']']).trim().to_string(),
        };
        values.insert(key, value);
    }
    let required = ["FontName", "FullName", "FamilyName", "Weight", "version", "Registry", "Ordering", "Supplement", "AdobeCopyright"];
    let missing: Vec<&str> = required.iter().copied().filter(|k| !values.contains_key(k)).collect();
    if !missing.is_empty() {
        return Err(TablesError::BadOptions(format!("the cidfontinfo file lacks {}", missing.join(", "))));
    }
    let take = |key: &str| values.get(key).cloned().unwrap_or_default();
    let ascii = |key: &str| -> Result<String> {
        let v = take(key);
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(TablesError::BadOptions(format!("the cidfontinfo {key} must be printable ASCII without spaces")));
        }
        Ok(v)
    };
    let supplement = take("Supplement").parse::<i64>().ok().filter(|s| (0..=fontcraft_cff::cid::MAX_SUPPLEMENT).contains(s)).ok_or_else(|| {
        TablesError::BadOptions(format!("the cidfontinfo Supplement must be a whole number from 0 to {}", fontcraft_cff::cid::MAX_SUPPLEMENT))
    })?;
    let xuid = match values.get("XUID") {
        None => Vec::new(),
        Some(v) => {
            let numbers: Vec<i64> = v
                .split_whitespace()
                .map(|n| n.parse::<i64>().ok().filter(|n| i32::try_from(*n).is_ok()))
                .collect::<Option<_>>()
                .ok_or_else(|| TablesError::BadOptions("the cidfontinfo XUID must be whole numbers".into()))?;
            if numbers.len() > MAX_XUID {
                return Err(TablesError::TooLarge { what: "the numbers of the cidfontinfo XUID", limit: MAX_XUID });
            }
            numbers
        }
    };
    Ok(CidInfo {
        font_name: ascii("FontName")?,
        full_name: take("FullName"),
        family_name: take("FamilyName"),
        weight: take("Weight"),
        version: take("version"),
        registry: ascii("Registry")?,
        ordering: ascii("Ordering")?,
        supplement,
        copyright: take("AdobeCopyright"),
        trademark: values.get("Trademark").cloned(),
        xuid,
    })
}

/// The CID a glyph name gives: `.notdef` is 0; a number or `cid` and a number is that CID.
pub(super) fn parse_cid(name: &str) -> Option<u16> {
    if name == ".notdef" {
        return Some(0);
    }
    let digits = name.strip_prefix("cid").unwrap_or(name);
    if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|c| *c <= MAX_CID).and_then(|c| u16::try_from(c).ok())
}

/// The name of CID `cid` in the plan, as a CID-keyed font's glyphs are named.
pub(super) fn cid_name(cid: u16) -> String {
    if cid == 0 { ".notdef".to_string() } else { format!("cid{cid:05}") }
}

/// Whether a map gives numeric output names, which make the result CID-keyed.
pub(super) fn maps_give_cids(maps: &[Vec<(String, String)>]) -> bool {
    maps.iter().flatten().any(|(new, _)| !new.is_empty() && new.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether the font has a CID-keyed `CFF ` table.
pub(super) fn is_cid_keyed(font: &SfntFont) -> Result<bool> {
    match font.get(*b"CFF ") {
        None => Ok(false),
        Some(data) => Ok(cff::parse(data, num_glyphs(font)?)?.is_cid()),
    }
}

/// The Font DICT of each glyph of a CID-keyed CFF font; `None` for any other font.
pub fn glyph_font_dicts(font: &SfntFont) -> Result<Option<Vec<u16>>> {
    let Some(data) = font.get(*b"CFF ") else { return Ok(None) };
    let parsed = cff::parse(data, num_glyphs(font)?)?;
    Ok(parsed.is_cid().then(|| parsed.font_dict_of_glyphs().0))
}

/// The maps with every output name written as its CID's plan name ([`cid_name`]), and the
/// source names of CID-keyed fonts too. A name-keyed font without a map is refused: its glyphs
/// have no CIDs.
pub(super) fn cid_maps(cid_keyed: &[bool], maps: &[Vec<(String, String)>]) -> Result<Vec<Vec<(String, String)>>> {
    let mut out = Vec::with_capacity(cid_keyed.len());
    for (i, keyed) in cid_keyed.iter().enumerate() {
        let map = maps.get(i).map(Vec::as_slice).unwrap_or_default();
        if map.is_empty() {
            if !keyed {
                return Err(TablesError::BadOptions(format!(
                    "font {} is name-keyed: a CID-keyed result needs a map that gives each of its glyphs a CID",
                    i + 1
                )));
            }
            out.push(Vec::new());
            continue;
        }
        let mut entries = Vec::with_capacity(map.len());
        for (new, source) in map {
            let cid = parse_cid(new).ok_or_else(|| {
                TablesError::BadOptions(format!("`{new}` in the map of font {} is not a CID: a CID-keyed result needs numeric output names", i + 1))
            })?;
            let source = if *keyed { parse_cid(source).map_or_else(|| source.clone(), cid_name) } else { source.clone() };
            entries.push((cid_name(cid), source));
        }
        out.push(entries);
    }
    Ok(out)
}

/// Puts the plan's glyphs in CID order and returns each one's CID.
pub(super) fn sort_by_cid(plan: &mut Plan) -> Result<Vec<u16>> {
    let mut order: Vec<(u16, usize)> = Vec::with_capacity(plan.slots.len());
    for (i, slot) in plan.slots.iter().enumerate() {
        let cid = parse_cid(&slot.name).ok_or_else(|| TablesError::invalid(format!("the glyph `{}` has no CID", slot.name)))?;
        order.push((cid, i));
    }
    order.sort_unstable();
    if order.first().map(|(cid, _)| *cid) != Some(0) {
        return Err(TablesError::BadOptions("the result has no CID 0 (.notdef): one of the maps must take it".into()));
    }
    let mut new_of_old = vec![0u16; plan.slots.len()];
    let mut slots = Vec::with_capacity(plan.slots.len());
    for (new, (_, old)) in order.iter().enumerate() {
        if let Some(entry) = new_of_old.get_mut(*old) {
            *entry = u16::try_from(new).map_err(|_| TablesError::invalid("too many glyphs"))?;
        }
        slots.push(plan.slots.get(*old).cloned().ok_or_else(|| TablesError::invalid("a glyph of the plan is missing"))?);
    }
    plan.slots = slots;
    for map in &mut plan.maps {
        for entry in map.iter_mut().flatten() {
            *entry = new_of_old.get(usize::from(*entry)).copied().unwrap_or(*entry);
        }
    }
    Ok(order.into_iter().map(|(cid, _)| cid).collect())
}

/// What a CID-keyed `CFF ` table is made from besides the glyphs.
pub(super) struct Setup<'s> {
    pub info: Option<&'s CidInfo>,
    /// Per font, its map header (empty when it has none).
    pub headers: &'s [MapHeader],
    /// The font whose hint values and global metrics every Private DICT and the Top DICT take.
    pub hints: Option<&'s cff::Cff<'s>>,
}

/// The CID-keyed `CFF ` table of the glyphs of `plan`, whose CIDs are `cids`.
pub(super) fn build(fonts: &[Expanded<'_>], plan: &Plan, cids: &[u16], setup: &Setup<'_>, warnings: &mut Vec<String>) -> Result<Built> {
    let first = fonts.first().ok_or_else(|| TablesError::BadOptions("give fonts to merge".into()))?;
    let dicts_of: Vec<Vec<u16>> = fonts.iter().map(|f| f.cff.font_dict_of_glyphs().0).collect();
    let fd_of = |font: usize, gid: u16| -> usize { dicts_of.get(font).and_then(|d| d.get(usize::from(gid))).map_or(0, |fd| usize::from(*fd)) };
    let keys: Vec<(usize, usize)> = plan.slots.iter().map(|s| (s.font, fd_of(s.font, s.gid))).collect::<BTreeSet<_>>().into_iter().collect();
    let index_of: HashMap<(usize, usize), usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();

    let header = |font: usize| setup.headers.get(font).cloned().unwrap_or_default();
    let mut fds = Vec::with_capacity(keys.len());
    for &(font, fd) in &keys {
        let source = &fonts.get(font).ok_or_else(|| TablesError::invalid("a glyph comes from a font that was not given"))?.cff;
        let h = header(font);
        let own = if source.is_cid() { source.font_dict_name(fd) } else { None };
        let name = h.ps_name.or(own).or_else(|| source.font_name()).unwrap_or_else(|| format!("Font{}", font + 1));
        fds.push(CidFontDict { source: font, fd, name, language_group: h.language_group.map(i64::from) });
    }

    let top = match setup.info {
        Some(info) => {
            let notice = match &info.trademark {
                Some(tm) => format!("{} {tm}", info.copyright),
                None => info.copyright.clone(),
            };
            CidTop {
                font_name: info.font_name.clone(),
                registry: info.registry.clone(),
                ordering: info.ordering.clone(),
                supplement: info.supplement,
                strings: vec![
                    (OP_VERSION, info.version.clone()),
                    (OP_NOTICE, notice),
                    (OP_FULL_NAME, info.full_name.clone()),
                    (OP_FAMILY_NAME, info.family_name.clone()),
                    (OP_WEIGHT, info.weight.clone()),
                ],
                ints: if info.xuid.is_empty() { Vec::new() } else { vec![(OP_XUID, info.xuid.clone())] },
            }
        }
        None => {
            let (registry, ordering, supplement) = if first.cff.is_cid() {
                first.cff.ros().ok_or_else(|| TablesError::invalid("the first font's CID-keyed CFF table has no readable ROS"))?
            } else {
                warnings.push("without cidfontinfo the CID-keyed result is Adobe-Identity-0, named after the first font".into());
                ("Adobe".to_string(), "Identity".to_string(), 0)
            };
            CidTop {
                font_name: first.cff.font_name().unwrap_or_else(|| "Untitled".into()),
                registry,
                ordering,
                supplement,
                strings: Vec::new(),
                ints: Vec::new(),
            }
        }
    };

    let mut glyphs = Vec::with_capacity(plan.slots.len());
    for (slot, cid) in plan.slots.iter().zip(cids) {
        let fd = index_of.get(&(slot.font, fd_of(slot.font, slot.gid))).copied().unwrap_or(0);
        glyphs.push(CidGlyph { cid: *cid, fd, source: slot.font, gid: usize::from(slot.gid) });
    }
    let sources: Vec<cff::Cff<'_>> = fonts.iter().map(|f| f.cff.clone()).collect();
    let table = assemble_cid(&sources, &top, &fds, &glyphs, setup.hints.map(|h| (h, 0)))?;
    finish(&table, glyphs.len(), setup.hints, warnings)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn cidfontinfo_needs_its_keys_and_reads_text_in_parentheses() {
        let text = "FontName (Test-CID)\nFullName (Test CID Font)\nFamilyName (Test)\nWeight (Regular)\nversion (1.000)\n\
                    Registry (Adobe)\nOrdering (Identity)\nSupplement 0\nAdobeCopyright (Copyright (c) nobody.)\n\
                    XUID 1 11 9273722\nIsBoldStyle false\n";
        let info = parse_cid_info(text).unwrap();
        assert_eq!(info.font_name, "Test-CID");
        assert_eq!(info.full_name, "Test CID Font");
        assert_eq!(info.copyright, "Copyright (c) nobody.");
        assert_eq!(info.xuid, [1, 11, 9_273_722]);
        assert_eq!(info.trademark, None);
        let err = parse_cid_info("FontName (X)\n").unwrap_err().to_string();
        assert!(err.contains("Registry") && err.contains("AdobeCopyright"), "{err}");
    }

    #[test]
    fn cid_names() {
        assert_eq!(parse_cid("0"), Some(0));
        assert_eq!(parse_cid(".notdef"), Some(0));
        assert_eq!(parse_cid("cid00042"), Some(42));
        assert_eq!(parse_cid("42"), Some(42));
        assert_eq!(parse_cid("65535"), None);
        assert_eq!(parse_cid("A"), None);
        assert_eq!(cid_name(42), "cid00042");
        assert!(maps_give_cids(&[vec![("1".into(), "A".into())]]));
        assert!(!maps_give_cids(&[vec![("A.alt".into(), "A".into())]]));
    }
}
