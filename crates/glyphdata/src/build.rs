//! Writes the built-in tables from the source files (used by `cargo xtask glyphdata --generate`).
//!
//! The sources are GlyphsInfo's `GlyphData.xml` and `GlyphData_Ideographs.xml`, the Adobe Glyph
//! List with AGLFN, and three files of the Unicode Character Database. The layout of each section
//! is described in `tables.rs`. Building is deterministic: the same sources give the same bytes.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::blob::{Writer, push_u16, push_u24};

use crate::names::code_point_name;
use crate::record::Record;
use crate::tables::{FLAG_CODE_POINT_PRODUCTION, NONE_U8, NONE_U24};
use crate::types::{BidiClass, Category, GeneralCategory};

const CODE_POINTS: usize = 0x11_0000;

/// Everything the tables are built from.
pub struct Sources {
    /// `key=value` pairs naming the pinned sources; stored in `META`.
    pub meta: Vec<(String, String)>,
    /// The records of `GlyphData.xml`, in file order.
    pub main: Vec<Record>,
    /// The records of `GlyphData_Ideographs.xml`, in file order.
    pub ideographs: Vec<Record>,
    /// `glyphlist.txt`.
    pub agl: String,
    /// `aglfn.txt`.
    pub aglfn: String,
    /// `UnicodeData.txt`.
    pub unicode_data: String,
    /// `extracted/DerivedBidiClass.txt`.
    pub bidi: String,
    /// `Scripts.txt`.
    pub scripts: String,
}

/// The blob and what building it found.
pub struct Built {
    pub blob: Vec<u8>,
    /// Counts of what went into the tables.
    pub stats: Vec<(String, usize)>,
    /// Things in the sources that the tables resolve one way (shadowed alternative names…).
    pub notes: Vec<String>,
}

/// Parses `glyphlist.txt`: `name;XXXX[ XXXX…]` lines.
pub fn parse_agl(text: &str) -> Result<Vec<(String, Vec<u32>)>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, cps) = line.split_once(';').ok_or_else(|| format!("glyphlist.txt:{}: no `;`", n + 1))?;
        let cps =
            cps.split(' ').map(|c| u32::from_str_radix(c, 16)).collect::<Result<Vec<_>, _>>().map_err(|e| format!("glyphlist.txt:{}: {e}", n + 1))?;
        out.push((name.to_string(), cps));
    }
    Ok(out)
}

/// Parses `aglfn.txt`: `XXXX;name;CHARACTER NAME` lines.
pub fn parse_aglfn(text: &str) -> Result<Vec<(u32, String)>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(';');
        let (Some(cp), Some(name)) = (parts.next(), parts.next()) else {
            return Err(format!("aglfn.txt:{}: expected `XXXX;name;…`", n + 1));
        };
        let cp = u32::from_str_radix(cp, 16).map_err(|e| format!("aglfn.txt:{}: {e}", n + 1))?;
        out.push((cp, name.to_string()));
    }
    Ok(out)
}

fn parse_range(field: &str) -> Result<(u32, u32), String> {
    let field = field.trim();
    let (a, b) = field.split_once("..").unwrap_or((field, field));
    let a = u32::from_str_radix(a.trim(), 16).map_err(|e| format!("`{field}`: {e}"))?;
    let b = u32::from_str_radix(b.trim(), 16).map_err(|e| format!("`{field}`: {e}"))?;
    if a > b || b as usize >= CODE_POINTS {
        return Err(format!("bad range `{field}`"));
    }
    Ok((a, b))
}

/// Parses a UCD property file (`XXXX..YYYY ; Value # comment`). With `missing`, the
/// `# @missing:` default lines come first, in file order, so explicit lines override them.
fn parse_property_file(text: &str, missing: bool) -> Result<Vec<(u32, u32, String)>, String> {
    let mut defaults = Vec::new();
    let mut explicit = Vec::new();
    for line in text.lines() {
        let (target, body) = match line.strip_prefix("# @missing:") {
            Some(rest) if missing => (&mut defaults, rest),
            _ => (&mut explicit, line.split('#').next().unwrap_or("")),
        };
        if body.trim().is_empty() {
            continue;
        }
        let (range, value) = body.split_once(';').ok_or_else(|| format!("no `;` in `{line}`"))?;
        let (a, b) = parse_range(range)?;
        target.push((a, b, value.trim().to_string()));
    }
    defaults.extend(explicit);
    Ok(defaults)
}

fn bidi_alias(value: &str) -> Option<BidiClass> {
    let short = match value {
        "Left_To_Right" => "L",
        "Right_To_Left" => "R",
        "Arabic_Letter" => "AL",
        "European_Number" => "EN",
        "European_Separator" => "ES",
        "European_Terminator" => "ET",
        "Arabic_Number" => "AN",
        "Common_Separator" => "CS",
        "Nonspacing_Mark" => "NSM",
        "Boundary_Neutral" => "BN",
        "Paragraph_Separator" => "B",
        "Segment_Separator" => "S",
        "White_Space" => "WS",
        "Other_Neutral" => "ON",
        "Left_To_Right_Embedding" => "LRE",
        "Left_To_Right_Override" => "LRO",
        "Right_To_Left_Embedding" => "RLE",
        "Right_To_Left_Override" => "RLO",
        "Pop_Directional_Format" => "PDF",
        "Left_To_Right_Isolate" => "LRI",
        "Right_To_Left_Isolate" => "RLI",
        "First_Strong_Isolate" => "FSI",
        "Pop_Directional_Isolate" => "PDI",
        other => other,
    };
    BidiClass::parse(short)
}

/// General categories per code point from `UnicodeData.txt` (`First>`/`Last>` pairs are ranges;
/// code points it does not list are `Cn`).
fn general_categories(text: &str) -> Result<Vec<u8>, String> {
    let mut gc = vec![GeneralCategory::Cn.index(); CODE_POINTS];
    let mut first: Option<u32> = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split(';').collect();
        let (Some(cp), Some(name), Some(cat)) = (fields.first(), fields.get(1), fields.get(2)) else {
            return Err(format!("UnicodeData.txt: short line `{line}`"));
        };
        let cp = u32::from_str_radix(cp, 16).map_err(|e| format!("UnicodeData.txt: {e}"))?;
        let cat = GeneralCategory::parse(cat).ok_or_else(|| format!("UnicodeData.txt: unknown category `{cat}`"))?;
        if name.ends_with(", First>") {
            first = Some(cp);
            continue;
        }
        let start = if name.ends_with(", Last>") { first.take().ok_or("UnicodeData.txt: `Last>` without `First>`")? } else { cp };
        for c in start..=cp {
            if let Some(slot) = gc.get_mut(c as usize) {
                *slot = cat.index();
            }
        }
    }
    Ok(gc)
}

/// Run-length form of a per-code-point table: (first code point, value) where the value changes.
fn runs(values: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut last = None;
    for (cp, v) in values.iter().enumerate() {
        if last != Some(*v) {
            push_u24(&mut out, cp as u32)?;
            out.push(*v);
            last = Some(*v);
        }
    }
    Ok(out)
}

/// A length-prefixed string of a column (length 0: none).
fn push_str(out: &mut Vec<u8>, s: Option<&str>) -> Result<(), String> {
    let s = s.unwrap_or("");
    out.push(u8::try_from(s.len()).map_err(|_| format!("`{s}` is longer than 255 bytes"))?);
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// The string pool, with each distinct string stored once.
#[derive(Default)]
struct Pool {
    bytes: Vec<u8>,
    seen: HashMap<String, u32>,
}

impl Pool {
    /// A string reference: 24-bit offset, 8-bit length. `None` and "" are the empty reference.
    fn add(&mut self, s: Option<&str>) -> Result<[u8; 4], String> {
        let Some(s) = s.filter(|s| !s.is_empty()) else {
            return Ok([0; 4]);
        };
        if s.len() > 255 {
            return Err(format!("`{s}` is longer than 255 bytes"));
        }
        let offset = match self.seen.get(s) {
            Some(o) => *o,
            None => {
                let o = self.bytes.len() as u32;
                if o > 0xFF_FFFF {
                    return Err("the string pool exceeds 16 MB".into());
                }
                self.bytes.extend_from_slice(s.as_bytes());
                self.seen.insert(s.to_string(), o);
                o
            }
        };
        let [a, b, c, _] = offset.to_le_bytes();
        Ok([a, b, c, s.len() as u8])
    }
}

fn is_plain_ideograph(r: &Record) -> bool {
    r.unicode.is_some_and(|cp| code_point_name(cp) == r.name)
        && r.category == Some(Category::Letter)
        && r.script.as_deref() == Some("han")
        && r.production.is_none()
        && r.unicode_legacy.is_none()
        && r.case.is_none()
        && r.direction.is_none()
        && r.description.is_none()
        && r.alt_names.is_empty()
}

/// Builds the blob.
pub fn build(s: &Sources) -> Result<Built, String> {
    let mut notes = Vec::new();
    let mut stats = Vec::new();

    // Records: ideographs in the plain code-point form become ranges; the rest are records.
    let mut han: Vec<(u32, u8)> = Vec::new();
    let mut records: Vec<&Record> = s.main.iter().collect();
    for r in &s.ideographs {
        match r.unicode {
            Some(cp) if is_plain_ideograph(r) => han.push((cp, r.sub_category.map_or(NONE_U8, |c| c.index()))),
            _ => records.push(r),
        }
    }
    let mut names: HashSet<String> = HashSet::new();
    for r in &records {
        if !names.insert(r.name.clone()) {
            return Err(format!("`{}` is named twice", r.name));
        }
    }
    for (cp, _) in &han {
        if !names.insert(code_point_name(*cp)) {
            return Err(format!("U+{cp:04X} is both an ideograph and a record"));
        }
    }
    records.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let mut cp_owner: BTreeMap<u32, &str> = BTreeMap::new();
    for r in &records {
        if let Some(cp) = r.unicode {
            if let Some(other) = cp_owner.get(&cp) {
                notes.push(format!("U+{cp:04X} is the code point of `{other}` and `{}`; lookups find `{other}`", r.name));
            } else {
                cp_owner.insert(cp, &r.name);
            }
        }
    }

    // Scripts.
    let mut script_ids: Vec<&str> = records.iter().filter_map(|r| r.script.as_deref()).chain(["han"]).collect();
    script_ids.sort_unstable();
    script_ids.dedup();
    if script_ids.len() >= usize::from(NONE_U8) {
        return Err("too many scripts".into());
    }
    let script_index = |s: &str| script_ids.iter().position(|x| *x == s).map(|i| i as u8);

    // Unicode properties.
    let gc = general_categories(&s.unicode_data)?;
    let mut bidi = vec![BidiClass::L.index(); CODE_POINTS];
    for (a, b, v) in parse_property_file(&s.bidi, true)? {
        let class = bidi_alias(&v).ok_or_else(|| format!("DerivedBidiClass.txt: unknown class `{v}`"))?;
        if let Some(slots) = bidi.get_mut(a as usize..=b as usize) {
            slots.fill(class.index());
        }
    }
    let mut ucd_scripts: Vec<String> = vec!["Unknown".into()];
    let mut uscr = vec![0u8; CODE_POINTS];
    for (a, b, v) in parse_property_file(&s.scripts, false)? {
        let i = match ucd_scripts.iter().position(|x| *x == v) {
            Some(i) => i,
            None => {
                ucd_scripts.push(v);
                ucd_scripts.len() - 1
            }
        };
        if i >= usize::from(NONE_U8) {
            return Err("too many Unicode scripts".into());
        }
        if let Some(slots) = uscr.get_mut(a as usize..=b as usize) {
            slots.fill(i as u8);
        }
    }

    // Scripts written right to left: most of their letters with a code point are R or AL.
    let mut votes: HashMap<&str, (usize, usize)> = HashMap::new();
    for r in &records {
        if let (Some(script), Some(cp), Some(Category::Letter)) = (r.script.as_deref(), r.unicode, r.category) {
            let v = votes.entry(script).or_default();
            if bidi.get(cp as usize).and_then(|b| BidiClass::from_index(*b)).is_some_and(BidiClass::is_rtl) {
                v.0 += 1;
            } else {
                v.1 += 1;
            }
        }
    }
    let mut rtl: Vec<&str> = votes.iter().filter(|(_, (r, l))| r > l).map(|(s, _)| *s).collect();
    rtl.sort_unstable();
    notes.push(format!("scripts written right to left: {}", rtl.join(", ")));

    // The data's script for each Unicode script: the one most records of that script carry (no
    // script counts as a candidate too).
    let mut tally: Vec<BTreeMap<Option<&str>, usize>> = vec![BTreeMap::new(); ucd_scripts.len()];
    for r in &records {
        if let Some(t) = r.unicode.and_then(|cp| uscr.get(cp as usize)).and_then(|s| tally.get_mut(usize::from(*s))) {
            *t.entry(r.script.as_deref()).or_default() += 1;
        }
    }
    for (cp, _) in &han {
        if let Some(t) = uscr.get(*cp as usize).and_then(|s| tally.get_mut(usize::from(*s))) {
            *t.entry(Some("han")).or_default() += 1;
        }
    }
    let ucsm: Vec<u8> = tally
        .iter()
        .map(|t| {
            let best = t.iter().max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k))).and_then(|(k, _)| *k);
            best.and_then(script_index).unwrap_or(NONE_U8)
        })
        .collect();

    // Records, one column per field, in name order.
    let mut pool = Pool::default();
    let mut cols: BTreeMap<[u8; 4], Vec<u8>> =
        [b"NAME", b"PROD", b"DESC", b"UNIC", b"LEGC", b"CATG", b"SUBC", b"CASE", b"DIRN", b"SCRP", b"FLAG", b"ALTN"]
            .into_iter()
            .map(|t| (*t, Vec::new()))
            .collect();
    let mut alt_keys = 0usize;
    let mut seen_alts: HashSet<&str> = HashSet::new();
    let mut stored_production = Vec::new();
    let mut predictable = 0usize;
    for (i, r) in records.iter().enumerate() {
        let code_point_production = matches!((&r.production, r.unicode), (Some(p), Some(cp)) if *p == code_point_name(cp));
        push_str(cols.entry(*b"NAME").or_default(), Some(&r.name))?;
        push_str(cols.entry(*b"PROD").or_default(), if code_point_production { None } else { r.production.as_deref() })?;
        push_str(cols.entry(*b"DESC").or_default(), r.description.as_deref())?;
        push_u24(cols.entry(*b"UNIC").or_default(), r.unicode.unwrap_or(NONE_U24))?;
        push_u24(cols.entry(*b"LEGC").or_default(), r.unicode_legacy.unwrap_or(NONE_U24))?;
        cols.entry(*b"CATG").or_default().push(r.category.map_or(NONE_U8, |c| c.index()));
        cols.entry(*b"SUBC").or_default().push(r.sub_category.map_or(NONE_U8, |c| c.index()));
        cols.entry(*b"CASE").or_default().push(r.case.map_or(NONE_U8, |c| c.index()));
        cols.entry(*b"DIRN").or_default().push(r.direction.map_or(NONE_U8, |c| c.index()));
        cols.entry(*b"SCRP").or_default().push(r.script.as_deref().and_then(script_index).unwrap_or(NONE_U8));
        cols.entry(*b"FLAG").or_default().push(if code_point_production { FLAG_CODE_POINT_PRODUCTION } else { 0 });
        let altn = cols.entry(*b"ALTN").or_default();
        altn.push(u8::try_from(r.alt_names.len()).map_err(|_| "too many alternative names on one record")?);
        for alt in &r.alt_names {
            push_str(altn, Some(alt))?;
            // Alternative names never shadow a record's own name, nor an earlier alternative.
            if names.contains(alt.as_str()) {
                notes.push(format!("alternative name `{alt}` of `{}` is a record's own name; it is not indexed", r.name));
            } else if !seen_alts.insert(alt.as_str()) {
                notes.push(format!("alternative name `{alt}` of `{}` is listed by an earlier record; it is not indexed", r.name));
            } else {
                alt_keys += 1;
            }
        }
        if code_point_production {
            predictable += 1;
        } else if let Some(p) = &r.production {
            stored_production.push((p.as_str(), i));
        }
    }
    // Shared production names and code points: the reader's indexes keep the first record by
    // name.
    stored_production.sort();
    for pair in stored_production.windows(2) {
        if let [(a, x), (b, y)] = pair
            && a == b
        {
            let name = |i: &usize| records.get(*i).map_or("?", |r| r.name.as_str());
            notes.push(format!("production name `{a}` is given by `{}` and `{}`; lookups find `{}`", name(x), name(y), name(x)));
        }
    }
    stored_production.dedup_by(|b, a| a.0 == b.0);

    // Ideograph ranges.
    han.sort_unstable();
    let mut hanr = Vec::new();
    let mut ranges = 0usize;
    let mut run: Option<(u32, u32, u8)> = None;
    for (cp, sub) in han.iter().copied().chain([(u32::MAX, NONE_U8)]) {
        match run {
            Some((a, b, s)) if cp == b + 1 && s == sub => run = Some((a, cp, s)),
            _ => {
                if let Some((a, b, s)) = run {
                    push_u24(&mut hanr, a)?;
                    push_u24(&mut hanr, b)?;
                    hanr.push(s);
                    ranges += 1;
                }
                run = (cp != u32::MAX).then_some((cp, cp, sub));
            }
        }
    }

    // Adobe Glyph List.
    let mut agl = parse_agl(&s.agl)?;
    agl.sort_by(|a, b| a.0.cmp(&b.0));
    let mut agln = Vec::new();
    let mut aglc = Vec::new();
    for (name, cps) in &agl {
        agln.extend_from_slice(&pool.add(Some(name))?);
        push_u16(&mut agln, u16::try_from(aglc.len() / 3).map_err(|_| "AGL too large")?);
        agln.push(u8::try_from(cps.len()).map_err(|_| "AGL entry too long")?);
        agln.push(0);
        for cp in cps {
            push_u24(&mut aglc, *cp)?;
        }
    }
    let mut aglfn = parse_aglfn(&s.aglfn)?;
    aglfn.sort_by(|a, b| a.1.cmp(&b.1));
    let mut aglf = Vec::new();
    for (cp, name) in &aglfn {
        aglf.extend_from_slice(&pool.add(Some(name))?);
        push_u24(&mut aglf, *cp)?;
        aglf.push(0);
    }

    let mut scrv = Vec::new();
    for sid in &script_ids {
        scrv.extend_from_slice(&pool.add(Some(sid))?);
    }
    let rtls: Vec<u8> = rtl.iter().filter_map(|s| script_index(s)).collect();
    let mut uscv = Vec::new();
    for sname in &ucd_scripts {
        uscv.extend_from_slice(&pool.add(Some(sname))?);
    }

    let mut meta = String::new();
    for (k, v) in &s.meta {
        meta.push_str(&format!("{k}={v}\n"));
    }

    stats.push(("records".into(), records.len()));
    stats.push(("ideographs".into(), han.len()));
    stats.push(("ideograph ranges".into(), ranges));
    stats.push(("alternative names".into(), alt_keys));
    stats.push(("code-point production names".into(), predictable));
    stats.push(("stored production names".into(), stored_production.len()));
    stats.push(("AGL names".into(), agl.len()));
    stats.push(("AGLFN names".into(), aglfn.len()));
    stats.push(("string pool bytes".into(), pool.bytes.len()));

    let mut w = Writer::default();
    w.add(b"META", meta.into_bytes());
    w.add(b"STRS", pool.bytes);
    for (tag, col) in cols {
        w.add(&tag, col);
    }
    w.add(b"HANR", hanr);
    w.add(b"SCRV", scrv);
    w.add(b"RTLS", rtls);
    w.add(b"USCV", uscv);
    w.add(b"UCSM", ucsm);
    w.add(b"GCAT", runs(&gc)?);
    w.add(b"BIDI", runs(&bidi)?);
    w.add(b"USCR", runs(&uscr)?);
    w.add(b"AGLN", agln);
    w.add(b"AGLC", aglc);
    w.add(b"AGLF", aglf);
    let blob = w.finish()?;
    Ok(Built { blob, stats, notes })
}
