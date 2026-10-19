//! `--update-name-table` (M12.20b): the instance's names from its `STAT` axis values, a port of
//! fontTools' `varLib.instancer.names.updateNameTable` and `setRibbiBits` (MIT, Copyright (c)
//! 2019 Google LLC and the fontTools authors).
//!
//! 1. The new default location is every `fvar` axis at its default, overridden by the pinned
//!    values and the new defaults of the limited axes.
//! 2. The `STAT` axis values that sit exactly at that location are chosen; every axis that has
//!    axis values somewhere must have one there, or the update is refused. Elidable values are
//!    left out; format 4 values (the most axes first) win over the single-axis values of their
//!    axes; the rest go in design-axis order.
//! 3. Values named Regular, Italic, Bold or Bold Italic (in their Windows English name) make the
//!    style-linking subfamily; the others make the typographic subfamily and a suffix of the
//!    family name. With neither, the elided fallback name is used.
//! 4. For every platform, encoding and language that has names 1, 2 and the elided fallback
//!    name: IDs 1, 2, 4 and 6 are written, 16 and 17 when there is a typographic subfamily (and
//!    removed, on every platform, when there is none), and 3 when the font has one: the old full
//!    or PostScript name inside it replaced, else version;vendor;PostScript name.
//! 5. After the instance is made, `head.macStyle` and `OS/2.fsSelection` follow the new
//!    Windows English subfamily name when it is Regular, Bold, Italic or Bold Italic.
//!
//! Name ID 25 stays: fontTools removes it only from a font without `fvar`, and the update runs
//! before `fvar` goes.

use std::collections::{BTreeSet, HashMap};

use read_fonts::tables::name::MacRomanMapping;

use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// Most name records read.
const MAX_RECORDS: usize = 65_535;
/// Most bytes of name strings read.
const MAX_NAME_TEXT: usize = 1 << 24;
/// Most STAT axis values read.
const MAX_AXIS_VALUES: usize = 65_535;
/// Most axis-value records in one format 4 value.
const MAX_FORMAT4_RECORDS: usize = 1_024;

const ELIDABLE: u16 = 2;

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

fn refuse(what: impl Into<String>) -> TablesError {
    TablesError::BadOptions(format!("the name table cannot be updated: {}", what.into()))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn fixed_at(b: &[u8], at: usize) -> Option<f64> {
    u32_at(b, at).map(|v| f64::from(v as i32) / 65536.0)
}

// ---------- the name table ----------

#[derive(Clone, Debug)]
struct Record {
    platform: u16,
    encoding: u16,
    language: u16,
    id: u16,
    raw: Vec<u8>,
}

impl Record {
    fn utf16(&self) -> bool {
        self.platform == 0 || self.platform == 3
    }

    fn mac_roman(&self) -> bool {
        self.platform == 1 && self.encoding == 0
    }

    fn text(&self) -> Option<String> {
        if self.utf16() {
            let (chunks, rest) = self.raw.as_chunks::<2>();
            if !rest.is_empty() {
                return None;
            }
            let units: Vec<u16> = chunks.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16(&units).ok()
        } else if self.mac_roman() {
            Some(self.raw.iter().map(|b| MacRomanMapping.decode(*b)).collect())
        } else {
            None
        }
    }
}

type Platform = (u16, u16, u16);

fn encode(platform: Platform, text: &str) -> Result<Vec<u8>> {
    let (p, e, _) = platform;
    if p == 0 || p == 3 {
        Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect())
    } else if p == 1 && e == 0 {
        text.chars()
            .map(|c| MacRomanMapping.encode(c))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| refuse(format!("“{text}” cannot be written in a Macintosh Roman name record")))
    } else {
        Err(refuse(format!("name records of platform {p} encoding {e} cannot be written")))
    }
}

struct Names {
    format: u16,
    records: Vec<Record>,
    tags: Vec<Vec<u8>>,
}

impl Names {
    fn read(data: &[u8]) -> Result<Self> {
        let cut = || bad("the name table is truncated");
        let format = u16_at(data, 0).ok_or_else(cut)?;
        let count = usize::from(u16_at(data, 2).ok_or_else(cut)?).min(MAX_RECORDS);
        let storage = usize::from(u16_at(data, 4).ok_or_else(cut)?);
        let mut total = 0usize;
        let mut string = |len: u16, off: u16| -> Result<Vec<u8>> {
            total = total.saturating_add(usize::from(len));
            if total > MAX_NAME_TEXT {
                return Err(TablesError::TooLarge { what: "the strings of the name table", limit: MAX_NAME_TEXT });
            }
            let start = storage + usize::from(off);
            Ok(data.get(start..start + usize::from(len)).ok_or_else(|| bad("a name record points past the end of the table"))?.to_vec())
        };
        let mut records = Vec::with_capacity(count);
        for i in 0..count {
            let at = 6 + 12 * i;
            let f: Vec<u16> = (0..6).map(|k| u16_at(data, at + 2 * k)).collect::<Option<_>>().ok_or_else(cut)?;
            let [platform, encoding, language, id, len, off] = f.as_slice() else { return Err(cut()) };
            records.push(Record { platform: *platform, encoding: *encoding, language: *language, id: *id, raw: string(*len, *off)? });
        }
        let mut tags = Vec::new();
        if format == 1 {
            let at = 6 + 12 * count;
            let n = usize::from(u16_at(data, at).ok_or_else(cut)?);
            for i in 0..n {
                let rec = at + 2 + 4 * i;
                tags.push(string(u16_at(data, rec).ok_or_else(cut)?, u16_at(data, rec + 2).ok_or_else(cut)?)?);
            }
        }
        Ok(Names { format, records, tags })
    }

    /// The records sorted as fontTools sorts them when it writes the table, identical strings
    /// stored once.
    fn write(mut self) -> Result<Vec<u8>> {
        let too_big = || TablesError::TooLarge { what: "the name table's string storage", limit: 65_535 };
        self.records.sort_by_key(|r| (r.platform, r.encoding, r.language, r.id));
        let header = 6 + self.records.len() * 12 + if self.format == 1 { 2 + self.tags.len() * 4 } else { 0 };
        let mut storage: Vec<u8> = Vec::new();
        let mut placed: HashMap<Vec<u8>, u16> = HashMap::new();
        let mut place = |bytes: &[u8]| -> Result<(u16, u16)> {
            let len = u16::try_from(bytes.len()).map_err(|_| too_big())?;
            if let Some(off) = placed.get(bytes) {
                return Ok((len, *off));
            }
            let off = u16::try_from(storage.len()).map_err(|_| too_big())?;
            storage.extend_from_slice(bytes);
            placed.insert(bytes.to_vec(), off);
            Ok((len, off))
        };
        let mut out = Vec::with_capacity(header);
        out.extend(self.format.to_be_bytes());
        out.extend(u16::try_from(self.records.len()).map_err(|_| too_big())?.to_be_bytes());
        out.extend(u16::try_from(header).map_err(|_| too_big())?.to_be_bytes());
        for r in &self.records {
            let (len, off) = place(&r.raw)?;
            for v in [r.platform, r.encoding, r.language, r.id, len, off] {
                out.extend(v.to_be_bytes());
            }
        }
        if self.format == 1 {
            out.extend(u16::try_from(self.tags.len()).map_err(|_| too_big())?.to_be_bytes());
            for t in &self.tags {
                let (len, off) = place(t)?;
                out.extend(len.to_be_bytes());
                out.extend(off.to_be_bytes());
            }
        }
        out.extend(storage);
        Ok(out)
    }

    /// fontTools' `getName`: the first record with this ID on this platform, encoding and
    /// language.
    fn get(&self, id: u16, p: Platform) -> Option<String> {
        self.records.iter().find(|r| r.id == id && (r.platform, r.encoding, r.language) == p).and_then(Record::text)
    }

    fn set(&mut self, id: u16, p: Platform, text: &str) -> Result<()> {
        let raw = encode(p, text)?;
        match self.records.iter_mut().find(|r| r.id == id && (r.platform, r.encoding, r.language) == p) {
            Some(r) => r.raw = raw,
            None => self.records.push(Record { platform: p.0, encoding: p.1, language: p.2, id, raw }),
        }
        Ok(())
    }

    fn is_ribbi(&self, id: u16) -> bool {
        self.get(id, (3, 1, 0x409)).is_some_and(|t| matches!(t.as_str(), "Regular" | "Italic" | "Bold" | "Bold Italic"))
    }
}

// ---------- STAT ----------

/// One STAT axis value: its format, flags, name, and (axis index, value) per axis it names
/// (format 2 gives its nominal value).
struct AxisValue {
    format: u16,
    flags: u16,
    name_id: u16,
    values: Vec<(usize, f64)>,
}

struct Stat {
    axis_tags: Vec<Tag>,
    values: Vec<AxisValue>,
    elided: u16,
}

fn read_stat(data: &[u8]) -> Result<Stat> {
    let cut = || bad("the STAT table is truncated");
    let minor = u16_at(data, 2).ok_or_else(cut)?;
    let record_size = usize::from(u16_at(data, 4).ok_or_else(cut)?);
    let axis_count = usize::from(u16_at(data, 6).ok_or_else(cut)?);
    let axes_at = u32_at(data, 8).ok_or_else(cut)? as usize;
    let value_count = usize::from(u16_at(data, 12).ok_or_else(cut)?).min(MAX_AXIS_VALUES);
    let values_at = u32_at(data, 14).ok_or_else(cut)? as usize;
    // Version 1.0 has no elided fallback name; name ID 2 stands in for it.
    let elided = if minor >= 1 { u16_at(data, 18).ok_or_else(cut)? } else { 2 };
    if record_size < 8 && axis_count > 0 {
        return Err(bad("the STAT design-axis records are too small"));
    }
    let mut axis_tags = Vec::with_capacity(axis_count.min(crate::limits::MAX_AXES));
    for i in 0..axis_count {
        let at = axes_at + i * record_size;
        let tag: Tag = data.get(at..at + 4).and_then(|t| t.try_into().ok()).ok_or_else(cut)?;
        axis_tags.push(tag);
    }
    let mut values = Vec::with_capacity(value_count);
    for i in 0..value_count {
        let off = usize::from(u16_at(data, values_at + 2 * i).ok_or_else(cut)?);
        let at = values_at + off;
        let format = u16_at(data, at).ok_or_else(cut)?;
        let v = match format {
            1..=3 => {
                let axis = usize::from(u16_at(data, at + 2).ok_or_else(cut)?);
                let flags = u16_at(data, at + 4).ok_or_else(cut)?;
                let name_id = u16_at(data, at + 6).ok_or_else(cut)?;
                let value = fixed_at(data, at + 8).ok_or_else(cut)?;
                AxisValue { format, flags, name_id, values: vec![(axis, value)] }
            }
            4 => {
                let n = usize::from(u16_at(data, at + 2).ok_or_else(cut)?);
                if n > MAX_FORMAT4_RECORDS {
                    return Err(TablesError::TooLarge { what: "the axis records of a STAT format 4 value", limit: MAX_FORMAT4_RECORDS });
                }
                let flags = u16_at(data, at + 4).ok_or_else(cut)?;
                let name_id = u16_at(data, at + 6).ok_or_else(cut)?;
                let mut recs = Vec::with_capacity(n);
                for k in 0..n {
                    let r = at + 8 + 6 * k;
                    recs.push((usize::from(u16_at(data, r).ok_or_else(cut)?), fixed_at(data, r + 2).ok_or_else(cut)?));
                }
                AxisValue { format, flags, name_id, values: recs }
            }
            // fontTools warns about an unknown format and keeps the value; it names no axis here.
            _ => AxisValue { format, flags: 0, name_id: 0, values: Vec::new() },
        };
        if v.values.iter().any(|(a, _)| *a >= axis_count) {
            return Err(bad("a STAT axis value names a design axis that does not exist"));
        }
        values.push(v);
    }
    Ok(Stat { axis_tags, values, elided })
}

// ---------- the update ----------

/// Rewrites the style names of `font` for its new default location `defaults` (every `fvar`
/// axis by tag, user values). Refused when `STAT` is missing, has no axis values, or lacks a
/// value at the location for an axis that has values.
pub(super) fn update_name_table(font: &mut SfntFont, defaults: &[(Tag, f64)]) -> Result<()> {
    let stat = font.get(*b"STAT").ok_or_else(|| refuse("the font has no STAT table"))?;
    let stat = read_stat(stat)?;
    if stat.values.is_empty() {
        return Err(refuse("the STAT table has no axis values"));
    }
    let tag_of = |axis: usize| stat.axis_tags.get(axis).copied();
    let coord = |axis: usize| tag_of(axis).and_then(|t| defaults.iter().find(|(d, _)| *d == t).map(|(_, v)| *v));
    // axisValuesFromAxisLimits with every axis pinned: values of an axis outside the location
    // go; values of axes the location does not name stay.
    let at_location = |v: &AxisValue| v.format > 4 || v.format == 0 || v.values.iter().all(|(a, x)| coord(*a).is_none_or(|c| c == *x));
    let chosen: Vec<&AxisValue> = stat.values.iter().filter(|v| at_location(v)).collect();

    // checkAxisValuesExist
    let has_values: BTreeSet<usize> =
        stat.values.iter().filter(|v| (1..=4).contains(&v.format)).flat_map(|v| v.values.iter().map(|(a, _)| *a)).collect();
    let mut seen: BTreeSet<Tag> = BTreeSet::new();
    for v in &chosen {
        for (a, x) in &v.values {
            if coord(*a) == Some(*x)
                && let Some(t) = tag_of(*a)
            {
                seen.insert(t);
            }
        }
    }
    let has_tags: BTreeSet<Tag> = has_values.iter().filter_map(|a| tag_of(*a)).collect();
    let missing: Vec<String> = defaults
        .iter()
        .filter(|(t, _)| has_tags.contains(t) && !seen.contains(t))
        .map(|(t, v)| format!("{} {}", tag_str(*t).trim_end(), number(*v)))
        .collect();
    if !missing.is_empty() {
        return Err(refuse(format!("the STAT table has no axis value at {}", missing.join(", "))));
    }

    let chosen: Vec<&AxisValue> = chosen.into_iter().filter(|v| v.flags & ELIDABLE == 0).collect();
    let sorted = sort_axis_values(&chosen);

    let data = font.get(*b"name").ok_or_else(|| TablesError::MissingTable("name".into()))?;
    let mut names = Names::read(data)?;
    let ids: Vec<u16> = sorted.iter().map(|v| v.name_id).collect();
    let ribbi: Vec<u16> = ids.iter().copied().filter(|n| names.is_ribbi(*n)).collect();
    let non_ribbi: Vec<u16> = ids.iter().copied().filter(|n| !ribbi.contains(n)).collect();
    let elided_ribbi = names.is_ribbi(stat.elided);
    let platforms: BTreeSet<Platform> = names.records.iter().map(|r| (r.platform, r.encoding, r.language)).collect();
    let revision = font.get(*b"head").and_then(|h| fixed_at(h, 4)).unwrap_or(0.0);
    let vendor = vendor(font);
    for p in platforms {
        if [1, 2, stat.elided].iter().any(|id| names.get(*id, p).is_none()) {
            continue;
        }
        let join = |list: &[u16]| list.iter().filter_map(|n| names.get(*n, p)).collect::<Vec<_>>().join(" ");
        let mut subfamily = join(&ribbi);
        let mut typo = if non_ribbi.is_empty() { None } else { Some(join(&ids)) };
        if typo.as_deref().is_none_or(str::is_empty) && subfamily.is_empty() {
            let elided = names.get(stat.elided, p).unwrap_or_default();
            if elided_ribbi {
                subfamily = elided;
            } else {
                typo = Some(elided);
            }
        }
        let suffix = join(&non_ribbi);
        let version = font_version(revision, &names, p);
        let style =
            Style { suffix: &suffix, subfamily: &subfamily, typo: typo.as_deref().filter(|t| !t.is_empty()), version: &version, vendor: &vendor };
        style_records(&mut names, p, &style)?;
    }
    font.insert(*b"name", names.write()?);
    Ok(())
}

/// fontTools' `_sortAxisValues`: format 4 values with the most records first, each taking its
/// axes when none is taken yet; then the single-axis values of the axes left; in axis order.
fn sort_axis_values<'a>(values: &[&'a AxisValue]) -> Vec<&'a AxisValue> {
    let mut results: Vec<(usize, usize, &AxisValue)> = Vec::new();
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut format4: Vec<&AxisValue> = values.iter().copied().filter(|v| v.format == 4).collect();
    // A stable sort, longest first, as Python's sorted(reverse=True) keeps equal ones in order.
    format4.sort_by_key(|v| std::cmp::Reverse(v.values.len()));
    for v in format4 {
        let axes: BTreeSet<usize> = v.values.iter().map(|(a, _)| *a).collect();
        let Some(&min) = axes.iter().next() else { continue };
        if seen.is_disjoint(&axes) {
            seen.extend(axes);
            results.push((min, results.len(), v));
        }
    }
    for v in values.iter().copied().filter(|v| v.format != 4) {
        let Some(&(axis, _)) = v.values.first() else { continue };
        if seen.insert(axis) {
            results.push((axis, results.len(), v));
        }
    }
    // Python sorts the (axis index, value) pairs; equal axis indexes cannot occur.
    results.sort_by_key(|(axis, order, _)| (*axis, *order));
    results.into_iter().map(|(_, _, v)| v).collect()
}

/// What one platform's style records are made from.
struct Style<'a> {
    suffix: &'a str,
    subfamily: &'a str,
    typo: Option<&'a str>,
    version: &'a str,
    vendor: &'a str,
}

fn style_records(names: &mut Names, p: Platform, s: &Style<'_>) -> Result<()> {
    let (suffix, subfamily, typo, version, vendor) = (s.suffix, s.subfamily, s.typo, s.version, s.vendor);
    let family = names.get(16, p).or_else(|| names.get(1, p));
    let style = names.get(17, p).or_else(|| names.get(2, p));
    let (Some(family), Some(_)) = (family, style) else {
        return Err(refuse(format!("names 1 and 2 are missing for platform {}, encoding {}, language {}", p.0, p.1, p.2)));
    };
    let mut set: Vec<(u16, String)> = vec![(1, family.clone()), (2, if subfamily.is_empty() { "Regular".into() } else { subfamily.to_string() })];
    match typo {
        Some(t) => {
            if let Some(slot) = set.iter_mut().find(|(id, _)| *id == 1) {
                slot.1 = format!("{family} {suffix}").trim().to_string();
            }
            set.push((16, family.clone()));
            set.push((17, t.to_string()));
        }
        None => names.records.retain(|r| r.id != 16 && r.id != 17),
    }
    let get = |id: u16| set.iter().find(|(i, _)| *i == id).map(|(_, s)| s.clone());
    let new_family = get(16).or_else(|| get(1)).unwrap_or_default();
    let new_style = get(17).or_else(|| get(2)).unwrap_or_default();
    let full = format!("{new_family} {new_style}");
    let ps = ps_name(names, p, &new_family, &new_style);
    if let Some(unique) = names.get(3, p) {
        let mut replaced = None;
        for (id, new) in [(4u16, &full), (6, &ps)] {
            if let Some(old) = names.get(id, p)
                && unique.contains(old.as_str())
            {
                replaced = Some(unique.replace(old.as_str(), new));
                break;
            }
        }
        let unique = replaced.unwrap_or_else(|| format!("{version};{vendor};{ps}"));
        set.push((3, unique));
    }
    set.push((4, full));
    set.push((6, ps));
    for (id, text) in set {
        if text.is_empty() {
            return Err(refuse(format!("name {id} would be empty")));
        }
        names.set(id, p, &text)?;
    }
    Ok(())
}

/// Adobe Technical Note 5902's PostScript name: the variations prefix (name 25) or the family,
/// a hyphen, the style; only ASCII letters, digits and hyphens; at most 127 characters.
fn ps_name(names: &Names, p: Platform, family: &str, style: &str) -> String {
    let prefix = names.get(25, p).filter(|s| !s.is_empty()).unwrap_or_else(|| family.to_string());
    let ps: String = format!("{prefix}-{style}").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    if ps.len() > 127 { format!("{}...", ps.get(..124).unwrap_or_default()) } else { ps }
}

/// fontTools' `_fontVersion`: name 5 up to its first `;`, without leading characters of
/// "Version " (Python's `lstrip` takes a set of characters), else `head.fontRevision` to three
/// decimals.
fn font_version(revision: f64, names: &Names, p: Platform) -> String {
    match names.get(5, p) {
        Some(v) => {
            let first = v.split(';').next().unwrap_or_default();
            first.trim_start_matches(|c: char| "Version ".contains(c)).trim().to_string()
        }
        None => format!("{revision:.3}"),
    }
}

/// A user value as a message shows it: integers without decimals.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{v:.0}") } else { format!("{v}") }
}

/// `OS/2.achVendID` without characters outside ASCII and without surrounding white space.
fn vendor(font: &SfntFont) -> String {
    let id = font.get(*b"OS/2").and_then(|o| o.get(58..62)).unwrap_or_default();
    let text: String = id.iter().filter(|b| b.is_ascii()).map(|b| char::from(*b)).collect();
    text.trim().to_string()
}

/// fontTools' `setRibbiBits`: `head.macStyle` and `OS/2.fsSelection` from the Windows English
/// subfamily name when it is Regular, Bold, Italic or Bold Italic.
pub(super) fn set_ribbi_bits(font: &mut SfntFont) -> Result<()> {
    let Some(data) = font.get(*b"name") else { return Ok(()) };
    let names = Names::read(data)?;
    let Some(style) = names.get(2, (3, 1, 0x409)) else { return Ok(()) };
    let style = style.to_lowercase();
    let (mac, sel): (Option<u16>, u16) = match style.as_str() {
        "regular" => (None, 1 << 6),
        "bold" => (Some(0b01), 1 << 5),
        "italic" => (Some(0b10), 1),
        "bold italic" => (Some(0b11), 1 | (1 << 5)),
        _ => return Ok(()),
    };
    if let Some(m) = mac
        && let Some(head) = font.get(*b"head")
        && head.len() >= 46
    {
        let mut h = head.to_vec();
        if let Some(slot) = h.get_mut(44..46) {
            slot.copy_from_slice(&m.to_be_bytes());
        }
        font.insert(*b"head", h);
    }
    if let Some(os2) = font.get(*b"OS/2")
        && let Some(old) = u16_at(os2, 62)
    {
        let mut o = os2.to_vec();
        let new = (old & !(1 | (1 << 5) | (1 << 6))) | sel;
        if let Some(slot) = o.get_mut(62..64) {
            slot.copy_from_slice(&new.to_be_bytes());
        }
        font.insert(*b"OS/2", o);
    }
    Ok(())
}
