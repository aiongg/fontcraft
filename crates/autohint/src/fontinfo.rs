//! Override sets: otfautohint's fontinfo text, which gives some glyphs their own zones and stems
//! (`FDDict` blocks with a `GlyphSet`), and its counter-glyph lists.
//!
//! Ported from otfautohint's `fdTools.py` (`parseFontInfoFile`, `mergeFDDicts`,
//! `FDDictManager`) and `splitpsdicts.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). Limits: [`crate::MAX_FONTINFO_BYTES`], [`crate::MAX_OVERRIDE_SETS`],
//! [`crate::MAX_COUNTER_GLYPHS`], and [`MAX_PATTERN_LEN`] and [`MAX_MATCH_WORK`] for the glyph
//! patterns.
//!
//! The text is whitespace-separated words; `#` starts a comment that runs to the end of the line.
//! `begin FDDict NAME` … `end FDDict NAME` holds `Key value` lines (a list value is written
//! `[a b c]`); `begin GlyphSet NAME` … `end GlyphSet NAME` holds regular expressions, and every
//! glyph name one of them finds is hinted with the FDDict of the same name, which must come
//! first. A set named `FinalFont` hints no glyph: it gives the zones and stems of the final font.
//! As soon as one set exists, glyphs that no set takes get the font's own dictionary with its
//! zones moved outside the glyphs (`No Alignment Zones`).
//!
//! Glyph patterns are matched by a small engine of our own that covers what the format's
//! documentation lists (`^`, `$`, `.`, `[…]` classes with ranges and negation, `*`, `+`, `?`,
//! `{m,n}`, `\` escapes including `\d`, `\w`, `\s`, `\S`, `\xHH`, `\uHHHH`); groups and
//! alternation are refused with a message.
//!
//! Two departures from the original, both where it would fail later or lose data: a set without
//! DominantH or DominantV is refused (the original warns, then fails when it hints), and
//! [`merge`] keeps the vertical stems of sets without OtherBlues (the original drops them).

use std::collections::{BTreeMap, BTreeSet};

use crate::fddict::{BLUE_KEYS, OTHER_BLUE_KEYS, ZoneKey, ZoneList, build_pairs};
use crate::{AutohintError, BlueValue, FdDict, MAX_COUNTER_GLYPHS, MAX_FONTINFO_BYTES, MAX_OVERRIDE_SETS};

/// Longest glyph pattern, in characters.
pub const MAX_PATTERN_LEN: usize = 1024;
/// Most steps spent matching glyph patterns against glyph names in one text: each test of a
/// pattern against a name costs the name's length times the pattern's states.
pub const MAX_MATCH_WORK: usize = 200_000_000;
/// Most states one pattern expands to (a `{m,n}` repeat copies its atom).
const MAX_PATTERN_STATES: usize = 4096;

/// The FDDict name that gives the final font's zones and stems (`kFinalDictName`).
pub const FINAL_DICT_NAME: &str = "FinalFont";
/// The name of the font's dictionary once override sets exist (`kDefaultDictName`).
pub const DEFAULT_DICT_NAME: &str = "No Alignment Zones";

/// Words outside any block that the original reads and ignores, with their value
/// (`kBaseStateTokens`).
const BASE_STATE_TOKENS: [&str; 10] = [
    "FontName",
    "FullName",
    "IsBoldStyle",
    "IsItalicStyle",
    "ConvertToCID",
    "PreferOS/2TypoMetrics",
    "IsOS/2WidthWeigthSlopeOnly",
    "IsOS/2OBLIQUE",
    "UseOldNameID4",
    "LicenseCode",
];
/// The keys of an FDDict block besides the zone keys (`kOtherFDDictKeys`, `kRunTimeFDDictKeys`).
const OTHER_KEYS: [&str; 14] = [
    "FontName",
    "OrigEmSqUnits",
    "LanguageGroup",
    "DominantV",
    "DominantH",
    "FlexOK",
    "VCounterChars",
    "HCounterChars",
    "BlueFuzz",
    "DictName",
    "BlueValuesPairs",
    "OtherBlueValuesPairs",
    "FamilyValuesPairs",
    "OtherFamilyValuesPairs",
];

/// The parsed fontinfo: the override sets, which set each glyph uses, and the final merged
/// dictionary when the text gives one (`FinalFont`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontInfo {
    /// Index 0 is the font's own dictionary; the override sets follow in text order.
    pub dicts: Vec<FdDict>,
    /// Glyph name → index in [`FontInfo::dicts`]; glyphs not listed use 0.
    pub glyph_dict: BTreeMap<String, usize>,
    pub final_dict: Option<FdDict>,
}

impl FontInfo {
    /// The dictionary glyph `name` is hinted with.
    pub fn dict_for(&self, name: &str) -> Option<&FdDict> {
        self.dicts.get(self.glyph_dict.get(name).copied().unwrap_or(0))
    }
}

/// A key's value as the text gives it: one word, or a bracketed list.
enum Value {
    One(String),
    List(Vec<String>),
}

/// One FDDict block's keys as `setInfo` stores them, before `buildBlueLists`.
#[derive(Default)]
struct RawDict {
    name: String,
    /// The line of its first `begin`, for messages about the whole set.
    line: usize,
    dict_name: Option<String>,
    font_name: Option<String>,
    /// Zone keys, `OrigEmSqUnits` and `BlueFuzz`, as whole numbers.
    numbers: BTreeMap<String, f64>,
    language_group: Option<u8>,
    flex_ok: Option<bool>,
    dominant_h: Option<Vec<f64>>,
    dominant_v: Option<Vec<f64>>,
    v_counter: Option<Vec<String>>,
    h_counter: Option<Vec<String>>,
}

fn err(line: usize, message: impl Into<String>) -> AutohintError {
    AutohintError::FontInfo { line, message: message.into() }
}

/// Python's `int()` on a word: a whole number.
fn int(word: &str, key: &str, line: usize) -> Result<f64, AutohintError> {
    word.parse::<i64>().map(|v| v as f64).map_err(|_| err(line, format!("`{key}` must be a whole number, not `{word}`")))
}

fn is_zone_key(key: &str) -> bool {
    let plain = key.strip_prefix("Family").unwrap_or(key);
    BLUE_KEYS.iter().chain(&OTHER_BLUE_KEYS).any(|(pos, over)| *pos == plain || *over == plain)
}

impl RawDict {
    /// `FDDict.setInfo`.
    fn set(&mut self, key: &str, value: Value, line: usize) -> Result<(), AutohintError> {
        let words = |v: Value| match v {
            Value::One(w) => vec![w],
            Value::List(l) => l,
        };
        match key {
            "LanguageGroup" => self.language_group = Some(u8::from(matches!(&value, Value::One(w) if w == "1"))),
            "DominantV" | "DominantH" => {
                let list = words(value);
                if list.len() > crate::MAX_ZONES {
                    return Err(AutohintError::LimitExceeded { what: "stem widths in one list", limit: crate::MAX_ZONES });
                }
                let stems = list.iter().map(|w| int(w, key, line)).collect::<Result<Vec<_>, _>>()?;
                if key == "DominantV" {
                    self.dominant_v = Some(stems);
                } else {
                    self.dominant_h = Some(stems);
                }
            }
            "FlexOK" => self.flex_ok = Some(!matches!(&value, Value::One(w) if w == "false")),
            "VCounterChars" | "HCounterChars" => {
                let list = words(value);
                if list.len() > MAX_COUNTER_GLYPHS {
                    return Err(AutohintError::LimitExceeded { what: "glyph names in one counter-glyph list", limit: MAX_COUNTER_GLYPHS });
                }
                if key == "VCounterChars" {
                    self.v_counter = Some(list);
                } else {
                    self.h_counter = Some(list);
                }
            }
            "FontName" => self.font_name = Some(words(value).join(" ")),
            "DictName" => self.dict_name = Some(words(value).join(" ")),
            // Filled by `buildBlueLists`; a value in the text is overwritten there.
            "BlueValuesPairs" | "OtherBlueValuesPairs" | "FamilyValuesPairs" | "OtherFamilyValuesPairs" => {}
            _ => match value {
                Value::One(w) => {
                    let v = int(&w, key, line)?;
                    self.numbers.insert(key.to_string(), v);
                }
                Value::List(_) => return Err(err(line, format!("`{key}` takes one number, not a list"))),
            },
        }
        Ok(())
    }

    /// The zones of one list (`buildBlueLists`' key walk).
    fn zone_keys(&self, family: bool, other: bool) -> Result<Vec<ZoneKey>, String> {
        let prefix = if family { "Family" } else { "" };
        let keys: &[(&str, &str)] = if other { &OTHER_BLUE_KEYS } else { &BLUE_KEYS };
        let mut out = Vec::new();
        for (i, (pos, over)) in keys.iter().enumerate() {
            let (pos, over) = (format!("{prefix}{pos}"), format!("{prefix}{over}"));
            let Some(&overshoot) = self.numbers.get(&over) else { continue };
            let Some(&position) = self.numbers.get(&pos) else {
                return Err(format!("there is no {pos} for the overshoot key {over}"));
            };
            out.push(ZoneKey { name: pos, position, overshoot, baseline: !other && i == 0 });
        }
        Ok(out)
    }

    /// The validated dictionary. `require` holds the checks a set that hints glyphs must pass
    /// (a baseline zone and both stem lists); the final font's set skips them.
    fn build(self, base: &FdDict, require: bool) -> Result<FdDict, AutohintError> {
        let line = self.line;
        let dict_name = self.dict_name.clone().unwrap_or_else(|| self.name.clone());
        let fail = |m: String| err(line, format!("in the FDDict {dict_name}: {m}"));
        if require {
            if !self.numbers.contains_key("BaselineOvershoot") {
                return Err(fail("the BaselineYCoord and BaselineOvershoot values are missing; they are required".into()));
            }
            for (list, key) in [(&self.dominant_h, "DominantH"), (&self.dominant_v, "DominantV")] {
                if list.is_none() {
                    return Err(fail(format!("there is no {key} value; it is required")));
                }
            }
        }
        let fuzz = self.numbers.get("BlueFuzz").copied().unwrap_or(base.blue_fuzz);
        let lists = [
            (ZoneList::Blue, false, false),
            (ZoneList::OtherBlue, false, true),
            (ZoneList::Family, true, false),
            (ZoneList::OtherFamily, true, true),
        ];
        let mut built: Vec<Vec<BlueValue>> = Vec::with_capacity(4);
        for (list, family, other) in lists {
            let keys = self.zone_keys(family, other).map_err(&fail)?;
            built.push(build_pairs(list, &keys, fuzz, &dict_name).map_err(&fail)?);
        }
        let mut built = built.into_iter();
        let mut next = || built.next().unwrap_or_default();
        let baseline = match (self.numbers.get("BaselineYCoord"), self.numbers.get("BaselineOvershoot")) {
            (Some(&pos), Some(&over)) => Some((pos, over)),
            _ => None,
        };
        Ok(FdDict {
            dict_name: dict_name.clone(),
            font_name: self.font_name.or_else(|| base.font_name.clone()),
            blue_pairs: next(),
            other_blue_pairs: next(),
            family_pairs: next(),
            other_family_pairs: next(),
            blue_fuzz: fuzz,
            dominant_h: self.dominant_h.unwrap_or_default(),
            dominant_v: self.dominant_v.unwrap_or_default(),
            flex_ok: self.flex_ok.unwrap_or(true),
            language_group: self.language_group.unwrap_or(0),
            em_units: self.numbers.get("OrigEmSqUnits").copied(),
            v_counter_chars: self.v_counter.unwrap_or_default(),
            h_counter_chars: self.h_counter.unwrap_or_default(),
            baseline,
        })
    }
}

/// The words of `text` with their line numbers (from 1), comments removed.
fn tokens(text: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_text = line.split('#').next().unwrap_or("");
        out.extend(line_text.split_whitespace().map(|w| (w, i + 1)));
    }
    out
}

enum State {
    Base,
    /// Inside a list value of a word outside any block.
    InValue,
    /// Inside a list value of an FDDict key, in the block `name`.
    InDictValue {
        key: String,
        list: Vec<String>,
        line: usize,
        name: String,
        set: Option<usize>,
    },
    /// Inside an FDDict block: the set's index in the raw sets, `None` for the final font's.
    Dict {
        name: String,
        set: Option<usize>,
    },
    GlyphSet {
        name: String,
        set: usize,
    },
}

/// Parses fontinfo `text` against the font's dictionary `base`, for the glyphs in
/// `glyph_names`; `max_y` and `min_y` are the font's vertical extremes, which otfautohint uses
/// for zones a set leaves open (`parseFontInfoFile`).
pub fn parse(text: &str, base: &FdDict, glyph_names: &[String], max_y: f64, min_y: f64) -> Result<FontInfo, AutohintError> {
    if text.len() > MAX_FONTINFO_BYTES {
        return Err(AutohintError::LimitExceeded { what: "bytes of hinting settings", limit: MAX_FONTINFO_BYTES });
    }
    let toks = tokens(text);
    let word = |i: usize| toks.get(i).map(|t| t.0);
    let mut sets: Vec<RawDict> = Vec::new();
    let mut set_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut final_raw: Option<RawDict> = None;
    let mut glyph_sets: BTreeSet<String> = BTreeSet::new();
    let mut glyph_dict: BTreeMap<String, usize> = BTreeMap::new();
    let mut work = 0usize;
    let mut state = State::Base;
    let mut i = 0;
    while let Some(&(tok, line)) = toks.get(i) {
        i += 1;
        state = match state {
            State::Base => {
                if tok == "begin" {
                    let kind = word(i).ok_or_else(|| err(line, "`begin` at the end of the text"))?;
                    let name = word(i + 1).ok_or_else(|| err(line, format!("`begin {kind}` without a name")))?.to_string();
                    i += 2;
                    match kind {
                        "FDDict" if name == FINAL_DICT_NAME => {
                            final_raw.get_or_insert_with(|| RawDict { name: name.clone(), line, ..RawDict::default() });
                            State::Dict { name, set: None }
                        }
                        "FDDict" => {
                            let set = match set_index.get(&name) {
                                Some(&s) => s,
                                None => {
                                    if sets.len() >= MAX_OVERRIDE_SETS {
                                        return Err(AutohintError::LimitExceeded { what: "override sets", limit: MAX_OVERRIDE_SETS });
                                    }
                                    sets.push(RawDict { name: name.clone(), line, ..RawDict::default() });
                                    set_index.insert(name.clone(), sets.len() - 1);
                                    sets.len() - 1
                                }
                            };
                            State::Dict { name, set: Some(set) }
                        }
                        "GlyphSet" => {
                            if !glyph_sets.insert(name.clone()) {
                                return Err(err(line, format!("the GlyphSet {name} is given twice")));
                            }
                            let set = *set_index
                                .get(&name)
                                .ok_or_else(|| err(line, format!("the GlyphSet {name} has no FDDict of the same name before it")))?;
                            State::GlyphSet { name, set }
                        }
                        other => return Err(err(line, format!("unknown word after `begin`: {other}"))),
                    }
                } else if BASE_STATE_TOKENS.contains(&tok) {
                    let value = word(i).ok_or_else(|| err(line, format!("`{tok}` without a value")))?;
                    i += 1;
                    if value.starts_with(['[', '(']) && !value.ends_with([']', ')']) { State::InValue } else { State::Base }
                } else {
                    return Err(err(line, format!("unknown word outside an FDDict or GlyphSet: {tok}")));
                }
            }
            State::InValue => {
                if tok.ends_with([']', ')']) {
                    State::Base
                } else {
                    State::InValue
                }
            }
            State::InDictValue { key, mut list, line: start, name, set } => {
                let done = tok.ends_with([']', ')']);
                let w = if done { tok.get(..tok.len() - 1).unwrap_or("") } else { tok };
                if !w.is_empty() {
                    list.push(w.to_string());
                }
                if done {
                    raw_mut(&mut sets, &mut final_raw, set)?.set(&key, Value::List(list), start)?;
                    State::Dict { name, set }
                } else {
                    State::InDictValue { key, list, line: start, name, set }
                }
            }
            State::GlyphSet { name, set } => {
                if tok == "end" && word(i) == Some("GlyphSet") {
                    let end = word(i + 1).ok_or_else(|| err(line, "`end GlyphSet` without a name"))?;
                    if end != name {
                        return Err(err(line, format!("`end GlyphSet {end}` does not match `begin GlyphSet {name}`")));
                    }
                    i += 2;
                    State::Base
                } else {
                    let pattern = Pattern::compile(tok).map_err(|m| err(line, format!("the glyph pattern `{tok}`: {m}")))?;
                    for g in glyph_names {
                        work = work.saturating_add(pattern.cost(g));
                        if work > MAX_MATCH_WORK {
                            return Err(AutohintError::LimitExceeded { what: "steps matching glyph patterns", limit: MAX_MATCH_WORK });
                        }
                        if pattern.search(g) {
                            glyph_dict.insert(g.clone(), set + 1);
                        }
                    }
                    State::GlyphSet { name, set }
                }
            }
            State::Dict { name, set } => {
                if tok == "end" && word(i) == Some("FDDict") {
                    let end = word(i + 1).ok_or_else(|| err(line, "`end FDDict` without a name"))?;
                    if end != name {
                        return Err(err(line, format!("`end FDDict {end}` does not match `begin FDDict {name}`")));
                    }
                    i += 2;
                    State::Base
                } else if is_zone_key(tok) || OTHER_KEYS.contains(&tok) {
                    let value = word(i).ok_or_else(|| err(line, format!("`{tok}` without a value")))?;
                    i += 1;
                    let raw = raw_mut(&mut sets, &mut final_raw, set)?;
                    if value.starts_with(['[', '(']) {
                        let inner = value.get(1..).unwrap_or("");
                        if inner.ends_with([']', ')']) {
                            let inner = inner.get(..inner.len() - 1).unwrap_or("");
                            raw.set(tok, Value::One(inner.to_string()), line)?;
                            State::Dict { name, set }
                        } else {
                            let list = if inner.is_empty() { Vec::new() } else { vec![inner.to_string()] };
                            State::InDictValue { key: tok.to_string(), list, line, name, set }
                        }
                    } else {
                        raw.set(tok, Value::One(value.to_string()), line)?;
                        State::Dict { name, set }
                    }
                } else {
                    return Err(err(line, format!("the FDDict {name} has an unknown key {tok}")));
                }
            }
        };
    }
    finish(sets, final_raw, glyph_dict, base, max_y, min_y)
}

fn raw_mut<'a>(sets: &'a mut [RawDict], final_raw: &'a mut Option<RawDict>, set: Option<usize>) -> Result<&'a mut RawDict, AutohintError> {
    match set {
        Some(i) => sets.get_mut(i),
        None => final_raw.as_mut(),
    }
    .ok_or_else(|| err(0, "an FDDict block lost track of its set"))
}

/// The end of `parseFontInfoFile`: every set validated, and the font's own dictionary given
/// inactive zones once sets exist.
fn finish(
    sets: Vec<RawDict>,
    final_raw: Option<RawDict>,
    glyph_dict: BTreeMap<String, usize>,
    base: &FdDict,
    max_y: f64,
    min_y: f64,
) -> Result<FontInfo, AutohintError> {
    let mut dicts = Vec::with_capacity(sets.len() + 1);
    if sets.is_empty() {
        dicts.push(base.clone());
    } else {
        if !max_y.is_finite() || !min_y.is_finite() {
            return Err(AutohintError::BadZones("the font's vertical extremes are not usable numbers".into()));
        }
        let (low, high) = (min_y.trunc() - 100.0, max_y.trunc() + 100.0);
        let keys = [
            ZoneKey { name: "BaselineYCoord".into(), position: low, overshoot: 0.0, baseline: true },
            ZoneKey { name: "CapHeight".into(), position: high, overshoot: 0.0, baseline: false },
        ];
        let mut d = base.clone();
        d.blue_pairs = build_pairs(ZoneList::Blue, &keys, 0.0, DEFAULT_DICT_NAME).map_err(AutohintError::BadZones)?;
        d.other_blue_pairs = Vec::new();
        d.blue_fuzz = 0.0;
        d.dict_name = DEFAULT_DICT_NAME.to_string();
        d.baseline = Some((low, 0.0));
        dicts.push(d);
    }
    for raw in sets {
        dicts.push(raw.build(base, true)?);
    }
    let final_dict = final_raw.map(|r| r.build(base, false)).transpose()?;
    Ok(FontInfo { dicts, glyph_dict, final_dict })
}

/// A number as the text writes it: whole numbers without a fraction.
fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

/// A block name the parser reads back: one word, not the final font's, not used before.
fn block_name(name: &str, index: usize, used: &mut BTreeSet<String>) -> String {
    let ok = !name.is_empty() && !name.contains(char::is_whitespace) && !name.contains('#') && name != FINAL_DICT_NAME;
    let name = if ok && !used.contains(name) { name.to_string() } else { format!("Set{index}") };
    used.insert(name.clone());
    name
}

/// The key lines of one zone list. Zones keep the key they are named after when it is free;
/// the others take the free keys in order, and zones beyond the keys are left out. In a
/// BlueValues list the baseline key goes to the zone named so, else to the first bottom zone,
/// else to the lowest zone.
fn write_zones(out: &mut String, pairs: &[BlueValue], keys: &[(&str, &str)], prefix: &str, has_baseline: bool) {
    let mut slot: Vec<Option<&BlueValue>> = vec![None; keys.len()];
    let mut placed = vec![false; pairs.len()];
    let key_name = |k: usize| keys.get(k).map(|(p, _)| format!("{prefix}{p}")).unwrap_or_default();
    if has_baseline && !pairs.is_empty() {
        let b = pairs.iter().position(|p| p.name == key_name(0)).or_else(|| pairs.iter().position(|p| p.is_bottom)).unwrap_or(0);
        if let (Some(s), Some(p), Some(f)) = (slot.get_mut(0), pairs.get(b), placed.get_mut(b)) {
            *s = Some(p);
            *f = true;
        }
    }
    let first_free = usize::from(has_baseline);
    for (i, p) in pairs.iter().enumerate() {
        if placed.get(i).copied().unwrap_or(true) {
            continue;
        }
        let named = (first_free..keys.len()).find(|&k| slot.get(k).is_some_and(Option::is_none) && key_name(k) == p.name);
        if let Some(k) = named
            && let (Some(s), Some(f)) = (slot.get_mut(k), placed.get_mut(i))
        {
            *s = Some(p);
            *f = true;
        }
    }
    for (i, p) in pairs.iter().enumerate() {
        if placed.get(i).copied().unwrap_or(true) {
            continue;
        }
        if let Some(k) = (first_free..keys.len()).find(|&k| slot.get(k).is_some_and(Option::is_none))
            && let Some(free) = slot.get_mut(k)
        {
            *free = Some(p);
        }
    }
    for ((pos_key, over_key), p) in keys.iter().zip(&slot) {
        let Some(p) = p else { continue };
        let (position, overshoot) = if p.is_bottom { (p.top, p.bottom - p.top) } else { (p.bottom, p.top - p.bottom) };
        out.push_str(&format!("    {prefix}{pos_key} {}\n    {prefix}{over_key} {}\n", num(position), num(overshoot)));
    }
}

fn write_list(out: &mut String, key: &str, items: impl IntoIterator<Item = String>) {
    let items: Vec<String> = items.into_iter().collect();
    out.push_str(&format!("    {key} [{}]\n", items.join(" ")));
}

/// One FDDict block (`FDDict.__str__` inside `begin` and `end`).
fn write_dict(out: &mut String, d: &FdDict, name: &str) {
    out.push_str(&format!("begin FDDict {name}\n"));
    if let Some(em) = d.em_units {
        out.push_str(&format!("    OrigEmSqUnits {}\n", num(em)));
    }
    if let Some(font) = d.font_name.as_deref().filter(|f| !f.is_empty() && !f.contains(char::is_whitespace) && !f.contains('#')) {
        out.push_str(&format!("    FontName {font}\n"));
    }
    if d.language_group == 1 {
        out.push_str("    LanguageGroup 1\n");
    }
    out.push_str(&format!("    FlexOK {}\n", d.flex_ok));
    out.push_str(&format!("    BlueFuzz {}\n", num(d.blue_fuzz)));
    write_zones(out, &d.blue_pairs, &BLUE_KEYS, "", true);
    write_zones(out, &d.other_blue_pairs, &OTHER_BLUE_KEYS, "", false);
    write_zones(out, &d.family_pairs, &BLUE_KEYS, "Family", true);
    write_zones(out, &d.other_family_pairs, &OTHER_BLUE_KEYS, "Family", false);
    write_list(out, "DominantV", d.dominant_v.iter().map(|v| num(*v)));
    write_list(out, "DominantH", d.dominant_h.iter().map(|v| num(*v)));
    let names =
        |l: &[String]| l.iter().filter(|n| !n.is_empty() && !n.contains(char::is_whitespace) && !n.contains('#')).cloned().collect::<Vec<_>>();
    for (key, list) in [("VCounterChars", &d.v_counter_chars), ("HCounterChars", &d.h_counter_chars)] {
        if !list.is_empty() {
            write_list(out, key, names(list));
        }
    }
    out.push_str(&format!("end FDDict {name}\n\n"));
}

/// A pattern that finds exactly `name` (`^…$`, every special character escaped).
fn exact_pattern(name: &str) -> String {
    let mut p = String::from("^");
    for c in name.chars() {
        if c == '#' || c.is_ascii_control() || c == ' ' {
            p.push_str(&format!("\\x{:02x}", u32::from(c)));
        } else if c.is_whitespace() {
            if u32::from(c) <= 0xffff {
                p.push_str(&format!("\\u{:04x}", u32::from(c)));
            }
        } else {
            if r"\.^$*+?{}[]()|".contains(c) {
                p.push('\\');
            }
            p.push(c);
        }
    }
    p.push('$');
    p
}

/// The fontinfo text of `info`, which [`parse`] reads back to the same sets (`FDDict.__str__`).
/// The font's own dictionary (index 0) is not written: it comes from the font. Glyph sets list
/// their glyphs as exact patterns.
pub fn write(info: &FontInfo) -> String {
    let mut out = String::new();
    let mut used = BTreeSet::new();
    let names: Vec<String> = info.dicts.iter().enumerate().skip(1).map(|(i, d)| block_name(&d.dict_name, i, &mut used)).collect();
    for (d, name) in info.dicts.iter().skip(1).zip(&names) {
        write_dict(&mut out, d, name);
    }
    if let Some(f) = &info.final_dict {
        write_dict(&mut out, f, FINAL_DICT_NAME);
    }
    for (i, name) in names.iter().enumerate() {
        let glyphs: Vec<String> = info.glyph_dict.iter().filter(|(_, s)| **s == i + 1).map(|(g, _)| exact_pattern(g)).collect();
        if glyphs.is_empty() {
            continue;
        }
        out.push_str(&format!("begin GlyphSet {name}\n"));
        for chunk in glyphs.chunks(8) {
            out.push_str(&format!("    {}\n", chunk.join(" ")));
        }
        out.push_str(&format!("end GlyphSet {name}\n\n"));
    }
    out
}

/// The dictionary that merges several, as `mergeFDDicts` builds the final font's: the union of
/// their BlueValues and OtherBlues zones and of their stems, sorted, where a zone that overlaps
/// the one before it or lies within `2 × BlueFuzz + 1` of it (BlueFuzz of the last dictionary)
/// is left out, and so is a stem within 2 units of the one before it. Equal zones and stems
/// count once, the later dictionary's name winning.
pub fn merge(dicts: &[FdDict]) -> Result<FdDict, AutohintError> {
    let Some(last) = dicts.last() else { return Ok(FdDict { dict_name: FINAL_DICT_NAME.to_string(), ..FdDict::default() }) };
    let union = |pick: fn(&FdDict) -> &Vec<BlueValue>| {
        let mut zones: Vec<BlueValue> = Vec::new();
        for d in dicts {
            for z in pick(d) {
                match zones.iter_mut().find(|e| e.top == z.top && e.bottom == z.bottom) {
                    Some(e) => *e = z.clone(),
                    None => zones.push(z.clone()),
                }
            }
        }
        zones.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.bottom.total_cmp(&b.bottom)));
        zones
    };
    let buffer = 2.0 * last.blue_fuzz + 1.0;
    let good_zones = |zones: Vec<BlueValue>| {
        let mut good: Vec<BlueValue> = Vec::new();
        let mut prev: Option<f64> = None;
        for z in zones {
            let keep = prev.is_none_or(|p| z.bottom >= p && (z.bottom - p).abs() >= buffer);
            prev = Some(z.top);
            if keep {
                good.push(z);
            }
        }
        good
    };
    let good_stems = |pick: fn(&FdDict) -> &Vec<f64>| {
        let mut all: Vec<f64> = dicts.iter().flat_map(|d| pick(d).iter().copied()).filter(|v| v.is_finite()).collect();
        all.sort_by(f64::total_cmp);
        all.dedup();
        let mut good = Vec::new();
        let mut prev: Option<f64> = None;
        for s in all {
            if prev.is_none_or(|p| (s - p).abs() >= 2.0) {
                good.push(s);
            }
            prev = Some(s);
        }
        good
    };
    let blue_pairs = good_zones(union(|d| &d.blue_pairs));
    let baseline = blue_pairs.iter().find(|z| z.is_bottom).map(|z| (z.top, z.bottom - z.top));
    Ok(FdDict {
        dict_name: FINAL_DICT_NAME.to_string(),
        font_name: last.font_name.clone(),
        blue_pairs,
        other_blue_pairs: good_zones(union(|d| &d.other_blue_pairs)),
        blue_fuzz: last.blue_fuzz,
        dominant_h: good_stems(|d| &d.dominant_h),
        dominant_v: good_stems(|d| &d.dominant_v),
        baseline,
        ..FdDict::default()
    })
}

// ── Glyph patterns ──────────────────────────────────────────────────────────────────────────────

/// What one pattern position matches.
#[derive(Clone, Debug, PartialEq)]
enum Atom {
    Any,
    Char(char),
    /// A class: (negated, ranges, the shorthand classes it includes).
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
        shorthands: Vec<Shorthand>,
    },
    Short(Shorthand),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shorthand {
    Digit(bool),
    Word(bool),
    Space(bool),
}

impl Shorthand {
    fn matches(self, c: char) -> bool {
        match self {
            Shorthand::Digit(neg) => c.is_numeric() != neg,
            Shorthand::Word(neg) => (c.is_alphanumeric() || c == '_') != neg,
            Shorthand::Space(neg) => c.is_whitespace() != neg,
        }
    }
}

impl Atom {
    fn matches(&self, c: char) -> bool {
        match self {
            Atom::Any => c != '\n',
            Atom::Char(x) => *x == c,
            Atom::Short(s) => s.matches(c),
            Atom::Class { negated, ranges, shorthands } => {
                let hit = ranges.iter().any(|(a, b)| *a <= c && c <= *b) || shorthands.iter().any(|s| s.matches(c));
                hit != *negated
            }
        }
    }
}

/// How often a state's atom may match: once, at most once, or any number of times.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rep {
    One,
    Opt,
    Star,
}

/// A compiled glyph pattern: a list of states run as a set (no backtracking), so a test costs
/// the name's length times the number of states.
#[derive(Clone, Debug, PartialEq)]
struct Pattern {
    states: Vec<(Atom, Rep)>,
    start: bool,
    end: bool,
}

/// The escape after a `\`: an atom (shorthand or literal).
fn escape(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<Atom, String> {
    let c = chars.next().ok_or("a `\\` at the end")?;
    let hex = |chars: &mut std::iter::Peekable<std::str::Chars>, n: usize| -> Result<char, String> {
        let digits: String = (0..n).filter_map(|_| chars.next()).collect();
        u32::from_str_radix(&digits, 16).ok().filter(|_| digits.len() == n).and_then(char::from_u32).ok_or_else(|| format!("a bad escape `{digits}`"))
    };
    Ok(match c {
        'd' => Atom::Short(Shorthand::Digit(false)),
        'D' => Atom::Short(Shorthand::Digit(true)),
        'w' => Atom::Short(Shorthand::Word(false)),
        'W' => Atom::Short(Shorthand::Word(true)),
        's' => Atom::Short(Shorthand::Space(false)),
        'S' => Atom::Short(Shorthand::Space(true)),
        't' => Atom::Char('\t'),
        'n' => Atom::Char('\n'),
        'r' => Atom::Char('\r'),
        'x' => Atom::Char(hex(chars, 2)?),
        'u' => Atom::Char(hex(chars, 4)?),
        c if c.is_ascii_alphanumeric() => return Err(format!("the escape `\\{c}` is not supported")),
        c => Atom::Char(c),
    })
}

impl Pattern {
    fn compile(src: &str) -> Result<Self, String> {
        if src.chars().count() > MAX_PATTERN_LEN {
            return Err(format!("longer than {MAX_PATTERN_LEN} characters"));
        }
        let mut chars = src.chars().peekable();
        let start = chars.next_if_eq(&'^').is_some();
        let mut states: Vec<(Atom, Rep)> = Vec::new();
        let mut end = false;
        while let Some(c) = chars.next() {
            if end {
                return Err("`$` is only supported at the end".into());
            }
            let atom = match c {
                '$' if chars.peek().is_none() => {
                    end = true;
                    continue;
                }
                '.' => Atom::Any,
                '\\' => escape(&mut chars)?,
                '[' => class(&mut chars)?,
                '(' | ')' | '|' => return Err(format!("`{c}` (groups and alternatives) is not supported")),
                '^' | '$' => return Err(format!("`{c}` is only supported at the start or end")),
                '*' | '+' | '?' | '{' => return Err(format!("`{c}` follows nothing")),
                c => Atom::Char(c),
            };
            let (min, max) = match chars.peek() {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                Some('{') => {
                    chars.next();
                    let body: String = chars.by_ref().take_while(|c| *c != '}').collect();
                    let (lo, hi) = match body.split_once(',') {
                        Some((lo, hi)) => (
                            lo.trim().parse::<usize>().ok(),
                            if hi.trim().is_empty() { Some(None) } else { hi.trim().parse::<usize>().ok().map(Some) },
                        ),
                        None => (body.trim().parse::<usize>().ok(), body.trim().parse::<usize>().ok().map(Some)),
                    };
                    let (Some(lo), Some(hi)) = (lo, hi) else { return Err(format!("a bad repeat `{{{body}}}`")) };
                    if hi.is_some_and(|h| h < lo) {
                        return Err(format!("a bad repeat `{{{body}}}`"));
                    }
                    push_repeat(&mut states, atom, lo, hi)?;
                    chars.next_if_eq(&'?');
                    continue;
                }
                _ => (1, Some(1)),
            };
            if (min, max) != (1, Some(1)) {
                chars.next();
                // A lazy quantifier finds the same names.
                chars.next_if_eq(&'?');
            }
            push_repeat(&mut states, atom, min, max)?;
        }
        Ok(Self { states, start, end })
    }

    /// What one test against `name` costs in [`MAX_MATCH_WORK`] units.
    fn cost(&self, name: &str) -> usize {
        (name.chars().count() + 1).saturating_mul(self.states.len() + 1)
    }

    /// The states reachable from `set` without reading a character.
    fn close(&self, set: &mut [bool]) {
        for i in 0..self.states.len() {
            if set.get(i).copied().unwrap_or(false)
                && self.states.get(i).is_some_and(|(_, r)| *r != Rep::One)
                && let Some(next) = set.get_mut(i + 1)
            {
                *next = true;
            }
        }
    }

    /// `re.search`: whether the pattern matches somewhere in `name`.
    fn search(&self, name: &str) -> bool {
        let n = self.states.len();
        let mut set = vec![false; n + 1];
        if let Some(s) = set.get_mut(0) {
            *s = true;
        }
        self.close(&mut set);
        let accepted = |set: &[bool]| set.get(n).copied().unwrap_or(false);
        if accepted(&set) && !self.end {
            return true;
        }
        for c in name.chars() {
            let mut next = vec![false; n + 1];
            for (i, (atom, rep)) in self.states.iter().enumerate() {
                if set.get(i).copied().unwrap_or(false) && atom.matches(c) {
                    let to = if *rep == Rep::Star { i } else { i + 1 };
                    if let Some(t) = next.get_mut(to) {
                        *t = true;
                    }
                }
            }
            if !self.start
                && let Some(s) = next.get_mut(0)
            {
                *s = true;
            }
            self.close(&mut next);
            set = next;
            if accepted(&set) && !self.end {
                return true;
            }
        }
        accepted(&set)
    }
}

fn push_repeat(states: &mut Vec<(Atom, Rep)>, atom: Atom, min: usize, max: Option<usize>) -> Result<(), String> {
    let copies = min.saturating_add(max.map_or(1, |m| m.saturating_sub(min)));
    if states.len().saturating_add(copies) > MAX_PATTERN_STATES {
        return Err("too many repeats".into());
    }
    for _ in 0..min {
        states.push((atom.clone(), Rep::One));
    }
    match max {
        None => states.push((atom, Rep::Star)),
        Some(m) => {
            for _ in min..m {
                states.push((atom.clone(), Rep::Opt));
            }
        }
    }
    Ok(())
}

/// A `[…]` class, the `[` already read.
fn class(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<Atom, String> {
    let negated = chars.next_if_eq(&'^').is_some();
    let mut ranges = Vec::new();
    let mut shorthands = Vec::new();
    let mut first = true;
    loop {
        let c = chars.next().ok_or("a `[` without its `]`")?;
        if c == ']' && !first {
            break;
        }
        first = false;
        let lo = if c == '\\' {
            match escape(chars)? {
                Atom::Char(c) => c,
                Atom::Short(s) => {
                    shorthands.push(s);
                    continue;
                }
                _ => return Err("a bad escape in a class".into()),
            }
        } else {
            c
        };
        if chars.peek() == Some(&'-') {
            let mut ahead = chars.clone();
            ahead.next();
            if let Some(&hi) = ahead.peek()
                && hi != ']'
            {
                chars.next();
                chars.next();
                let hi = if hi == '\\' {
                    match escape(chars)? {
                        Atom::Char(c) => c,
                        _ => return Err("a bad range in a class".into()),
                    }
                } else {
                    hi
                };
                if hi < lo {
                    return Err(format!("the range {lo}-{hi} is backwards"));
                }
                ranges.push((lo, hi));
                continue;
            }
        }
        ranges.push((lo, lo));
    }
    Ok(Atom::Class { negated, ranges, shorthands })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StemSet, Zone, ZoneSet};

    /// The example of otfautohint's fontinfo documentation, extended with an OtherBlues zone and
    /// a counter-glyph list. Expected values: otfautohint 5.0.1's `parseFontInfoFile` and
    /// `mergeFDDicts` on the same text, base dictionary and glyph names.
    const TEXT: &str = r"# overrides
begin FDDict ST_Smallcaps
    OrigEmSqUnits 1000
    FlexOK false
    BaselineOvershoot -20
    BaselineYCoord 0
    CapHeight 900
    CapOvershoot 20
    LcHeight 700
    LcOvershoot 15
    DescenderHeight -250
    DescenderOvershoot -10
    DominantV [236 267]
    DominantH [141 152]
    VCounterChars [m.sc n.sc]
end FDDict ST_Smallcaps

begin FDDict LM
    BaselineOvershoot -25
    BaselineYCoord 0
    CapHeight 950
    CapOvershoot 25
    DominantV [236]
    DominantH [141]
end FDDict LM

begin GlyphSet LM
    ^[Ll].*\.sc$  ^m\.sc$
end GlyphSet LM

begin GlyphSet ST_Smallcaps
    [Tt]\S*\.sc
end GlyphSet ST_Smallcaps
";

    fn base() -> FdDict {
        let zone = |bottom, top| Zone { bottom, top, is_bottom: false, name: String::new() };
        let zones = ZoneSet { blue_values: vec![zone(-12.0, 0.0), zone(700.0, 712.0)], blue_fuzz: 1.0, ..ZoneSet::default() };
        let mut d = FdDict::new("base", &zones, &StemSet { h: vec![70.0], v: vec![80.0] }, false).unwrap();
        d.font_name = Some("Test".into());
        d
    }

    fn names() -> Vec<String> {
        ["a", "l.sc", "L.sc", "t.sc", "T.sc", "m.sc", "x"].iter().map(|s| s.to_string()).collect()
    }

    fn pair(top: f64, bottom: f64, name: &str, dict: &str, is_bottom: bool) -> BlueValue {
        BlueValue { top, bottom, name: name.into(), dict_name: dict.into(), is_bottom }
    }

    #[test]
    fn fontinfo_parse_matches_otfautohint_on_override_sets() {
        let info = parse(TEXT, &base(), &names(), 800.0, -200.0).unwrap();
        let assigned: Vec<(&str, usize)> = info.glyph_dict.iter().map(|(g, i)| (g.as_str(), *i)).collect();
        assert_eq!(assigned, vec![("L.sc", 2), ("T.sc", 1), ("l.sc", 2), ("m.sc", 2), ("t.sc", 1)]);
        assert_eq!(info.dict_for("a").map(|d| d.dict_name.as_str()), Some(DEFAULT_DICT_NAME));
        assert!(info.final_dict.is_none());

        let [d0, st, lm] = info.dicts.as_slice() else { panic!("{} sets", info.dicts.len()) };
        let dz = DEFAULT_DICT_NAME;
        assert_eq!(d0.blue_pairs, vec![pair(-300.0, -300.0, "BaselineYCoord", dz, false), pair(900.0, 900.0, "CapHeight", dz, false)]);
        assert_eq!((d0.blue_fuzz, d0.dominant_h.clone(), d0.dominant_v.clone()), (0.0, vec![70.0], vec![80.0]));

        let s = "ST_Smallcaps";
        assert_eq!(
            st.blue_pairs,
            vec![pair(0.0, -20.0, "BaselineYCoord", s, true), pair(715.0, 700.0, "LcHeight", s, false), pair(920.0, 900.0, "CapHeight", s, false)]
        );
        assert_eq!(st.other_blue_pairs, vec![pair(-250.0, -260.0, "DescenderHeight", s, true)]);
        assert_eq!((st.blue_fuzz, st.flex_ok, st.em_units, st.font_name.as_deref()), (1.0, false, Some(1000.0), Some("Test")));
        assert_eq!((st.dominant_h.clone(), st.dominant_v.clone()), (vec![141.0, 152.0], vec![236.0, 267.0]));
        assert_eq!(st.v_counter_chars, vec!["m.sc".to_string(), "n.sc".to_string()]);
        assert_eq!(lm.blue_pairs, vec![pair(0.0, -25.0, "BaselineYCoord", "LM", true), pair(975.0, 950.0, "CapHeight", "LM", false)]);
        assert!(lm.flex_ok);

        // mergeFDDicts: BlueValues [-25 0 700 715 900 920 950 975] (ST's baseline overlaps LM's
        // and is left out), OtherBlues [-260 -250], StemSnapH [141 152], StemSnapV [236 267].
        let merged = merge(&info.dicts[1..]).unwrap();
        let edges: Vec<f64> = merged.blue_pairs.iter().flat_map(|p| [p.bottom, p.top]).collect();
        assert_eq!(edges, vec![-25.0, 0.0, 700.0, 715.0, 900.0, 920.0, 950.0, 975.0]);
        let others: Vec<f64> = merged.other_blue_pairs.iter().flat_map(|p| [p.bottom, p.top]).collect();
        assert_eq!(others, vec![-260.0, -250.0]);
        assert_eq!((merged.dominant_h, merged.dominant_v), (vec![141.0, 152.0], vec![236.0, 267.0]));
    }

    #[test]
    fn fontinfo_write_reads_back_to_the_same_sets() {
        let info = parse(TEXT, &base(), &names(), 800.0, -200.0).unwrap();
        let text = write(&info);
        assert_eq!(parse(&text, &base(), &names(), 800.0, -200.0).unwrap(), info, "{text}");
        let none = parse("", &base(), &names(), 800.0, -200.0).unwrap();
        assert_eq!(none.dicts, vec![base()], "without sets the font's own dictionary is used as it is");
        assert_eq!(write(&none), "");
    }

    #[test]
    fn fontinfo_errors_name_the_line() {
        let line_of = |text: &str| match parse(text, &base(), &names(), 800.0, -200.0) {
            Err(AutohintError::FontInfo { line, .. }) => line,
            other => panic!("{other:?}"),
        };
        let set = |body: &str| {
            format!("begin FDDict S\n BaselineYCoord 0\n BaselineOvershoot -10\n DominantH [50]\n DominantV [60]\n{body}\nend FDDict S\n")
        };
        assert_eq!(line_of(&set(" Colour 5")), 6, "unknown key");
        assert_eq!(line_of(&set(" CapHeight 700\n CapOvershoot -10")), 1, "a top zone with a negative overshoot is reported at the set");
        assert_eq!(line_of(&set(" CapHeight 7.5")), 6, "values are whole numbers");
        assert_eq!(line_of("begin GlyphSet S\nend GlyphSet S\n"), 1, "a glyph set needs its FDDict first");
        assert_eq!(line_of(&format!("{}begin GlyphSet S\n a\nend GlyphSet S\nbegin GlyphSet S\n", set(""))), 11, "a glyph set given twice");
        assert_eq!(line_of(&format!("{}begin GlyphSet S\n (a|b)\nend GlyphSet S\n", set(""))), 9, "groups are not supported");
        assert_eq!(line_of("begin FDDict S\n BaselineYCoord 0\n BaselineOvershoot -10\n DominantV [60]\nend FDDict S\n"), 1, "DominantH is required");
        assert_eq!(line_of("begin FDDict S\nend FDDict T\n"), 2);
        assert_eq!(line_of("Junk"), 1);
        let big = " ".repeat(MAX_FONTINFO_BYTES + 1);
        assert!(matches!(parse(&big, &base(), &names(), 0.0, 0.0), Err(AutohintError::LimitExceeded { .. })));
        let counters = format!("begin FDDict S\n VCounterChars [{}]\nend FDDict S\n", vec!["a"; MAX_COUNTER_GLYPHS + 1].join(" "));
        assert!(matches!(parse(&counters, &base(), &names(), 0.0, 0.0), Err(AutohintError::LimitExceeded { .. })));
    }

    #[test]
    fn glyph_patterns_search_like_python_re() {
        let found = |p: &str, name: &str| Pattern::compile(p).unwrap().search(name);
        assert!(found("^[A-Z]$", "A") && !found("^[A-Z]$", "AB") && !found("^[A-Z]$", "a"));
        assert!(found("^.+\\.smallcaps$", "a.smallcaps") && !found("^.+\\.smallcaps$", "a.smallcaps.alt"));
        assert!(found("^.+\\.smallcaps", "a.smallcaps.alt") && !found("^.+\\.smallcaps", ".smallcaps"));
        for (name, hit) in [("a.s00", true), ("a.s10", true), ("a.s20", true), ("a.s40", true), ("a.s30", false)] {
            assert_eq!(found("^.+\\.s[0-24]0$", name), hit, "{name}");
        }
        assert!(found("[Tt]\\S*\\.sc", "uni0054.T.sc") && !found("[Tt]\\S*\\.sc", "x.sc"));
        assert!(found("^a{2,3}$", "aaa") && !found("^a{2,3}$", "aaaa") && !found("^a{2,3}$", "a"));
        assert!(found("^[^a-c]x?$", "d") && !found("^[^a-c]x?$", "b"));
        assert!(found("^\\x23\\u0041$", "#A") && found("", "anything"));
        assert!(Pattern::compile("(a)").is_err() && Pattern::compile("a|b").is_err() && Pattern::compile("[a").is_err());
        assert!(Pattern::compile("a{0,100000}").is_err(), "repeats are capped");
        let odd = "x y#z.[1]";
        assert!(found(&exact_pattern(odd), odd) && !found(&exact_pattern(odd), "x y#z.[1]a"));
    }
}
