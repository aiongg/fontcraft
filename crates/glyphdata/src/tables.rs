//! The built-in tables: a read-only view of the payload of `data/glyphdata.bin`.
//!
//! The blob is inflated and its sections located once, on first use. Loading also builds the
//! offsets of the string columns and three indexes the blob does not store: by code point (the
//! first record by name wins a shared code point), by stored production name, and by
//! alternative name (one that is also a record's name, or an earlier record's alternative name,
//! is left out). Lookups then binary-search in place; nothing is copied per record until a
//! [`Record`] is asked for.
//!
//! Section layouts (`build.rs` writes them; integers little-endian). Records are sorted by name
//! and stored one column per field; a string in a column is a length byte and the bytes, length
//! 0 meaning none. A string reference (in `SCRV`, `USCV`, `AGLN`, `AGLF`) is a 24-bit offset into
//! `STRS` and an 8-bit length.
//!
//! | Tag | Entry | Order |
//! |---|---|---|
//! | `META` | UTF-8 text, `key=value` lines | |
//! | `STRS` | the string pool | |
//! | `NAME`, `PROD`, `DESC` | per record: nice name, stored production name, description | by name |
//! | `UNIC`, `LEGC` | per record: unicode, legacy unicode (u24, `FFFFFF` none) | by name |
//! | `CATG`, `SUBC`, `CASE`, `DIRN`, `SCRP` | per record: index of the value (u8, `FF` none); `SCRP` indexes `SCRV` | by name |
//! | `FLAG` | per record: bit 0, the production name is the code-point name of `unicode` and not stored | by name |
//! | `ALTN` | per record: count (u8), then that many strings | by name |
//! | `HANR` | 7: first, last code point (u24), subcategory (u8) | by code point |
//! | `SCRV` | 4: script id (string reference) | |
//! | `RTLS` | 1: index into `SCRV` of a script written right to left | |
//! | `USCV` | 4: Unicode script name (string reference) | |
//! | `UCSM` | 1 per `USCV` entry: the matching `SCRV` index, `FF` none | |
//! | `GCAT`, `BIDI`, `USCR` | 4: first code point of a run (u24), value (u8) | by code point |
//! | `AGLN` | 8: name (string reference), first code point in `AGLC` (u16), count (u8), padding | by name |
//! | `AGLC` | 3: code point | |
//! | `AGLF` | 8: name (string reference), code point (u24), padding | by name |
//! | `DECM` | 9: code point, first and second code point of its canonical decomposition (u24, second `FFFFFF` for a singleton) | by code point |
//! | `CCCL` | 4: first code point of a run (u24), canonical combining class (u8) | by code point |
//!
//! `DECM` and `CCCL` may be absent (a blob from before they existed): every code point then has
//! no decomposition and class 0.

use std::ops::Range;
use std::sync::OnceLock;

use crate::Error;
use crate::blob::{Payload, u8_at, u16_at, u24_at};
use crate::names::code_point_name;
use crate::record::Record;
use crate::types::{BidiClass, Case, Category, Direction, GeneralCategory, SubCategory};

pub const HANR_SIZE: usize = 7;
pub const RUN_SIZE: usize = 4;
pub const AGLN_SIZE: usize = 8;
pub const AGLF_SIZE: usize = 8;
pub const DECM_SIZE: usize = 9;
pub const NONE_U24: u32 = 0xFF_FFFF;
pub const NONE_U8: u8 = 0xFF;
pub const FLAG_CODE_POINT_PRODUCTION: u8 = 1;
/// Most records the reader accepts.
pub const MAX_RECORDS: usize = 1 << 20;

static BLOB: &[u8] = include_bytes!("../data/glyphdata.bin");
static TABLES: OnceLock<Result<Tables, Error>> = OnceLock::new();

/// The built-in tables, loaded on first use.
pub fn builtin() -> Result<&'static Tables, Error> {
    match TABLES.get_or_init(|| Tables::open(BLOB)) {
        Ok(t) => Ok(t),
        Err(e) => Err(e.clone()),
    }
}

/// A string column: where each record's string starts (`u32::MAX` none) and its length.
#[derive(Default)]
struct StrColumn {
    data: Range<usize>,
    at: Vec<(u32, u8)>,
}

/// The tables of one blob.
pub struct Tables {
    data: Vec<u8>,
    strs: String,
    meta: String,
    count: usize,
    name: StrColumn,
    prod: StrColumn,
    desc: StrColumn,
    /// The alternative names in `ALTN` order, and the first of each record's.
    alt: StrColumn,
    alt_first: Vec<u32>,
    alt_record: Vec<u32>,
    unic: Range<usize>,
    legc: Range<usize>,
    catg: Range<usize>,
    subc: Range<usize>,
    case: Range<usize>,
    dirn: Range<usize>,
    scrp: Range<usize>,
    flag: Range<usize>,
    /// Alternative names (positions in `alt`) by name.
    alt_index: Vec<u32>,
    /// Records with a stored production name, by that name.
    production_index: Vec<u32>,
    /// (code point, record) by code point.
    code_point_index: Vec<(u32, u32)>,
    /// (legacy code point, record) by legacy code point.
    legacy_index: Vec<(u32, u32)>,
    hanr: Range<usize>,
    scrv: Range<usize>,
    rtls: Range<usize>,
    uscv: Range<usize>,
    ucsm: Range<usize>,
    gcat: Range<usize>,
    bidi: Range<usize>,
    uscr: Range<usize>,
    agln: Range<usize>,
    aglc: Range<usize>,
    aglf: Range<usize>,
    decm: Range<usize>,
    cccl: Range<usize>,
    han_script: Option<u8>,
}

/// A record's position in the columns.
pub type RecId = usize;

/// Reads `count` length-prefixed strings from `bytes` (the column at `base` in the payload).
fn str_column(bytes: &[u8], base: usize, count: usize, tag: &str) -> Result<StrColumn, Error> {
    let mut at = Vec::with_capacity(count);
    let mut pos = 0usize;
    for _ in 0..count {
        let len = u8_at(bytes, pos).ok_or_else(|| Error::Blob(format!("column {tag} is short")))?;
        pos += 1;
        at.push(if len == 0 { (u32::MAX, 0) } else { (pos as u32, len) });
        pos += usize::from(len);
    }
    if pos != bytes.len() {
        return Err(Error::Blob(format!("column {tag} has {} bytes, its strings {pos}", bytes.len())));
    }
    Ok(StrColumn { data: base..base + bytes.len(), at })
}

impl Tables {
    pub fn open(blob: &[u8]) -> Result<Tables, Error> {
        let payload = Payload::open(blob)?;
        let need = |tag: &[u8; 4], size: usize| payload.table(tag, size).map(|s| s.len() / size);
        let count = need(b"UNIC", 3)?;
        if count > MAX_RECORDS {
            return Err(Error::Blob("too many records".into()));
        }
        for tag in [b"LEGC"] {
            if need(tag, 3)? != count {
                return Err(Error::Blob("columns of different lengths".into()));
            }
        }
        for tag in [b"CATG", b"SUBC", b"CASE", b"DIRN", b"SCRP", b"FLAG"] {
            if need(tag, 1)? != count {
                return Err(Error::Blob("columns of different lengths".into()));
            }
        }
        for (tag, size) in [
            (b"HANR", HANR_SIZE),
            (b"SCRV", 4),
            (b"RTLS", 1),
            (b"USCV", 4),
            (b"UCSM", 1),
            (b"GCAT", RUN_SIZE),
            (b"BIDI", RUN_SIZE),
            (b"USCR", RUN_SIZE),
            (b"AGLN", AGLN_SIZE),
            (b"AGLC", 3),
            (b"AGLF", AGLF_SIZE),
        ] {
            need(tag, size)?;
        }
        for (tag, size) in [(b"DECM", DECM_SIZE), (b"CCCL", RUN_SIZE)] {
            if payload.section(tag).is_some() {
                need(tag, size)?;
            }
        }
        let strs = payload.section(b"STRS").ok_or_else(|| Error::Blob("missing section STRS".into()))?;
        let strs = String::from_utf8(strs.to_vec()).map_err(|_| Error::Blob("string pool is not UTF-8".into()))?;
        let meta = String::from_utf8(payload.section(b"META").unwrap_or(&[]).to_vec()).unwrap_or_default();
        let (data, ranges) = payload.into_parts();
        let range = |tag: &[u8; 4]| ranges.iter().find(|(t, _)| t == tag).map(|(_, r)| r.clone()).unwrap_or(0..0);
        let column = |tag: &[u8; 4]| {
            let r = range(tag);
            str_column(data.get(r.clone()).unwrap_or(&[]), r.start, count, &String::from_utf8_lossy(tag))
        };
        let (name, prod, desc) = (column(b"NAME")?, column(b"PROD")?, column(b"DESC")?);
        // Alternative names: a count per record, then the strings.
        let altn = range(b"ALTN");
        let bytes = data.get(altn.clone()).unwrap_or(&[]);
        let (mut alt, mut alt_first, mut alt_record) = (StrColumn { data: altn.clone(), at: Vec::new() }, Vec::with_capacity(count), Vec::new());
        let mut pos = 0usize;
        for record in 0..count {
            let n = u8_at(bytes, pos).ok_or_else(|| Error::Blob("column ALTN is short".into()))?;
            pos += 1;
            alt_first.push(alt.at.len() as u32);
            for _ in 0..n {
                let len = u8_at(bytes, pos).ok_or_else(|| Error::Blob("column ALTN is short".into()))?;
                alt.at.push((pos as u32 + 1, len));
                alt_record.push(record as u32);
                pos += 1 + usize::from(len);
            }
        }
        if pos != bytes.len() {
            return Err(Error::Blob("column ALTN has extra bytes".into()));
        }
        alt_first.push(alt.at.len() as u32);
        let mut t = Tables {
            strs,
            meta,
            count,
            name,
            prod,
            desc,
            alt,
            alt_first,
            alt_record,
            unic: range(b"UNIC"),
            legc: range(b"LEGC"),
            catg: range(b"CATG"),
            subc: range(b"SUBC"),
            case: range(b"CASE"),
            dirn: range(b"DIRN"),
            scrp: range(b"SCRP"),
            flag: range(b"FLAG"),
            alt_index: Vec::new(),
            production_index: Vec::new(),
            code_point_index: Vec::new(),
            legacy_index: Vec::new(),
            hanr: range(b"HANR"),
            scrv: range(b"SCRV"),
            rtls: range(b"RTLS"),
            uscv: range(b"USCV"),
            ucsm: range(b"UCSM"),
            gcat: range(b"GCAT"),
            bidi: range(b"BIDI"),
            uscr: range(b"USCR"),
            agln: range(b"AGLN"),
            aglc: range(b"AGLC"),
            aglf: range(b"AGLF"),
            decm: range(b"DECM"),
            cccl: range(b"CCCL"),
            han_script: None,
            data,
        };
        t.han_script = (0..t.scrv.len() / 4).find(|i| t.script_name(*i as u8) == Some("han")).map(|i| i as u8);
        t.code_point_index = (0..count).filter_map(|i| Some((t.record_unicode(i)?, i as u32))).collect();
        t.code_point_index.sort_unstable();
        t.code_point_index.dedup_by_key(|(cp, _)| *cp);
        t.legacy_index = (0..count).filter_map(|i| Some((t.record_legacy(i)?, i as u32))).collect();
        t.legacy_index.sort_unstable();
        t.legacy_index.dedup_by_key(|(cp, _)| *cp);
        let mut production_index: Vec<u32> =
            (0..count as u32).filter(|i| !t.has_code_point_production(*i as usize) && t.column_str(&t.prod, *i as usize).is_some()).collect();
        production_index.sort_by(|a, b| t.column_str(&t.prod, *a as usize).cmp(&t.column_str(&t.prod, *b as usize)).then(a.cmp(b)));
        production_index.dedup_by(|b, a| t.column_str(&t.prod, *a as usize) == t.column_str(&t.prod, *b as usize));
        t.production_index = production_index;
        let mut alt_index: Vec<u32> = (0..t.alt.at.len() as u32)
            .filter(|a| t.column_str(&t.alt, *a as usize).is_some_and(|name| t.find_name(name).is_none() && !t.is_han_name(name)))
            .collect();
        alt_index.sort_by(|a, b| t.column_str(&t.alt, *a as usize).cmp(&t.column_str(&t.alt, *b as usize)).then(a.cmp(b)));
        alt_index.dedup_by(|b, a| t.column_str(&t.alt, *a as usize) == t.column_str(&t.alt, *b as usize));
        t.alt_index = alt_index;
        Ok(t)
    }

    fn bytes(&self, r: &Range<usize>) -> &[u8] {
        self.data.get(r.clone()).unwrap_or(&[])
    }

    /// The `key=value` lines describing the sources the tables were built from.
    pub fn meta(&self) -> &str {
        &self.meta
    }

    fn str_ref(&self, b: &[u8], at: usize) -> Option<&str> {
        let offset = u24_at(b, at)? as usize;
        let len = usize::from(u8_at(b, at + 3)?);
        if len == 0 {
            return None;
        }
        self.strs.get(offset..offset + len)
    }

    /// String `i` of a column.
    fn column_str<'a>(&'a self, c: &StrColumn, i: usize) -> Option<&'a str> {
        let (at, len) = *c.at.get(i)?;
        if at == u32::MAX {
            return None;
        }
        let start = c.data.start.checked_add(at as usize)?;
        let bytes = self.data.get(start..start.checked_add(usize::from(len))?)?;
        std::str::from_utf8(bytes).ok()
    }

    // ── records ──

    pub fn record_count(&self) -> usize {
        self.count
    }

    fn byte(&self, column: &Range<usize>, i: RecId) -> Option<u8> {
        u8_at(self.bytes(column), i).filter(|v| *v != NONE_U8)
    }

    pub fn record_name(&self, i: RecId) -> Option<&str> {
        self.column_str(&self.name, i)
    }

    pub fn record_description(&self, i: RecId) -> Option<&str> {
        self.column_str(&self.desc, i)
    }

    pub fn record_unicode(&self, i: RecId) -> Option<u32> {
        u24_at(self.bytes(&self.unic), i.checked_mul(3)?).filter(|v| *v != NONE_U24)
    }

    fn record_legacy(&self, i: RecId) -> Option<u32> {
        u24_at(self.bytes(&self.legc), i.checked_mul(3)?).filter(|v| *v != NONE_U24)
    }

    fn has_code_point_production(&self, i: RecId) -> bool {
        u8_at(self.bytes(&self.flag), i).is_some_and(|f| f & FLAG_CODE_POINT_PRODUCTION != 0)
    }

    /// The record's production name, if it has one.
    pub fn record_production(&self, i: RecId) -> Option<String> {
        if self.has_code_point_production(i) {
            return Some(code_point_name(self.record_unicode(i)?));
        }
        self.column_str(&self.prod, i).map(str::to_string)
    }

    /// The whole record.
    pub fn record(&self, i: RecId) -> Option<Record> {
        let first = *self.alt_first.get(i)? as usize;
        let end = *self.alt_first.get(i + 1)? as usize;
        let alt_names = (first..end).filter_map(|a| self.column_str(&self.alt, a).map(str::to_string)).collect();
        Some(Record {
            name: self.record_name(i)?.to_string(),
            production: self.record_production(i),
            description: self.record_description(i).map(str::to_string),
            unicode: self.record_unicode(i),
            unicode_legacy: self.record_legacy(i),
            category: self.byte(&self.catg, i).and_then(Category::from_index),
            sub_category: self.byte(&self.subc, i).and_then(SubCategory::from_index),
            case: self.byte(&self.case, i).and_then(Case::from_index),
            direction: self.byte(&self.dirn, i).and_then(Direction::from_index),
            script: self.byte(&self.scrp, i).and_then(|s| self.script_name(s)).map(str::to_string),
            alt_names,
            ..Record::default()
        })
    }

    /// Binary search over `count` sorted entries; `key(i)` gives entry `i`'s key.
    fn search<'a, K>(count: usize, needle: &K, key: impl Fn(usize) -> Option<std::borrow::Cow<'a, K>>) -> Option<usize>
    where
        K: Ord + ToOwned + ?Sized + 'a,
    {
        let (mut lo, mut hi) = (0usize, count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let k = key(mid)?;
            match Ord::cmp(&*k, needle) {
                std::cmp::Ordering::Equal => return Some(mid),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// The record whose nice name is `name`.
    pub fn find_name(&self, name: &str) -> Option<RecId> {
        Self::search(self.count, name, |i| self.record_name(i).map(std::borrow::Cow::Borrowed))
    }

    fn is_han_name(&self, name: &str) -> bool {
        crate::names::parse_code_point_name(name).is_some_and(|cp| self.han(cp).is_some())
    }

    /// The record that lists `name` as an alternative name.
    pub fn find_alt_name(&self, name: &str) -> Option<RecId> {
        let alt_at = |i: usize| self.alt_index.get(i).map(|a| *a as usize);
        let i = Self::search(self.alt_index.len(), name, |i| self.column_str(&self.alt, alt_at(i)?).map(std::borrow::Cow::Borrowed))?;
        self.alt_record.get(alt_at(i)?).map(|r| *r as usize)
    }

    /// The record whose production name is `name`.
    pub fn find_production_name(&self, name: &str) -> Option<RecId> {
        if let Some(cp) = crate::names::parse_code_point_name(name)
            && let Some(i) = self.find_code_point(cp)
            && self.has_code_point_production(i)
        {
            return Some(i);
        }
        let rec_at = |i: usize| self.production_index.get(i).map(|r| *r as usize);
        let i = Self::search(self.production_index.len(), name, |i| self.column_str(&self.prod, rec_at(i)?).map(std::borrow::Cow::Borrowed))?;
        rec_at(i)
    }

    /// The record whose `unicode` is `cp`.
    pub fn find_code_point(&self, cp: u32) -> Option<RecId> {
        let i = self.code_point_index.binary_search_by_key(&cp, |(c, _)| *c).ok()?;
        self.code_point_index.get(i).map(|(_, r)| *r as usize)
    }

    /// The record whose `unicodeLegacy` is `cp`.
    pub fn find_legacy_code_point(&self, cp: u32) -> Option<RecId> {
        let i = self.legacy_index.binary_search_by_key(&cp, |(c, _)| *c).ok()?;
        self.legacy_index.get(i).map(|(_, r)| *r as usize)
    }

    // ── ideographs ──

    /// The subcategory of the ideograph record for `cp`, if there is one (`Some(None)`: a record
    /// without subcategory).
    pub fn han(&self, cp: u32) -> Option<Option<SubCategory>> {
        let hanr = self.bytes(&self.hanr);
        let n = hanr.len() / HANR_SIZE;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let first = u24_at(hanr, mid * HANR_SIZE)?;
            let last = u24_at(hanr, mid * HANR_SIZE + 3)?;
            if cp < first {
                hi = mid;
            } else if cp > last {
                lo = mid + 1;
            } else {
                return Some(u8_at(hanr, mid * HANR_SIZE + 6).filter(|v| *v != NONE_U8).and_then(SubCategory::from_index));
            }
        }
        None
    }

    /// The ideograph record for `cp`.
    pub fn han_record(&self, cp: u32) -> Option<Record> {
        let sub_category = self.han(cp)?;
        Some(Record {
            name: code_point_name(cp),
            category: Some(Category::Letter),
            sub_category,
            unicode: Some(cp),
            script: self.han_script.and_then(|s| self.script_name(s)).map(str::to_string),
            ..Record::default()
        })
    }

    /// Every ideograph range: first and last code point.
    pub fn han_ranges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let hanr = self.bytes(&self.hanr);
        (0..hanr.len() / HANR_SIZE).filter_map(move |i| Some((u24_at(hanr, i * HANR_SIZE)?, u24_at(hanr, i * HANR_SIZE + 3)?)))
    }

    // ── scripts ──

    pub fn script_name(&self, i: u8) -> Option<&str> {
        self.str_ref(self.bytes(&self.scrv), usize::from(i) * 4)
    }

    /// Whether the data's script id is written right to left.
    pub fn is_rtl_script(&self, script: &str) -> bool {
        self.bytes(&self.rtls).iter().any(|i| self.script_name(*i) == Some(script))
    }

    // ── Unicode properties ──

    /// The value of the run that holds `cp` in a run table.
    fn run_value(&self, r: &Range<usize>, cp: u32) -> Option<u8> {
        let runs = self.bytes(r);
        let n = runs.len() / RUN_SIZE;
        let (mut lo, mut hi) = (0usize, n);
        // The last run whose first code point is at most `cp`.
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if u24_at(runs, mid * RUN_SIZE)? <= cp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        u8_at(runs, lo.checked_sub(1)? * RUN_SIZE + 3)
    }

    pub fn general_category(&self, cp: u32) -> GeneralCategory {
        self.run_value(&self.gcat, cp).and_then(GeneralCategory::from_index).unwrap_or(GeneralCategory::Cn)
    }

    pub fn bidi_class(&self, cp: u32) -> BidiClass {
        self.run_value(&self.bidi, cp).and_then(BidiClass::from_index).unwrap_or(BidiClass::L)
    }

    /// The Unicode Script property value (`Latin`, `Arabic`, `Unknown`…).
    pub fn unicode_script(&self, cp: u32) -> Option<&str> {
        let i = self.run_value(&self.uscr, cp)?;
        self.str_ref(self.bytes(&self.uscv), usize::from(i) * 4)
    }

    /// The Unicode Script property values whose code points take the data's script id `script`
    /// when they have no record.
    pub fn unicode_scripts_of(&self, script: &str) -> Vec<&str> {
        let map = self.bytes(&self.ucsm);
        (0..map.len())
            .filter(|i| u8_at(map, *i).filter(|v| *v != NONE_U8).and_then(|v| self.script_name(v)) == Some(script))
            .filter_map(|i| self.str_ref(self.bytes(&self.uscv), i * 4))
            .collect()
    }

    /// The data's script id that most records of `cp`'s Unicode script carry.
    pub fn script_for_code_point(&self, cp: u32) -> Option<&str> {
        let i = self.run_value(&self.uscr, cp)?;
        let s = u8_at(self.bytes(&self.ucsm), usize::from(i)).filter(|v| *v != NONE_U8)?;
        self.script_name(s)
    }

    // ── Unicode canonical decomposition ──

    /// The one-level canonical decomposition of `cp`.
    pub fn canonical_decomposition(&self, cp: u32) -> Option<Vec<u32>> {
        let decm = self.bytes(&self.decm);
        let (mut lo, mut hi) = (0usize, decm.len() / DECM_SIZE);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let at = mid * DECM_SIZE;
            match u24_at(decm, at)?.cmp(&cp) {
                std::cmp::Ordering::Equal => {
                    let first = u24_at(decm, at + 3)?;
                    return Some(match u24_at(decm, at + 6)? {
                        NONE_U24 => vec![first],
                        second => vec![first, second],
                    });
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// The canonical combining class of `cp` (0 for a starter).
    pub fn combining_class(&self, cp: u32) -> u8 {
        self.run_value(&self.cccl, cp).unwrap_or(0)
    }

    // ── Adobe Glyph List ──

    /// The code points the Adobe Glyph List gives `name`.
    pub fn agl(&self, name: &str) -> Option<Vec<u32>> {
        let agln = self.bytes(&self.agln);
        let i = Self::search(agln.len() / AGLN_SIZE, name, |i| self.str_ref(agln, i * AGLN_SIZE).map(std::borrow::Cow::Borrowed))?;
        let first = usize::from(u16_at(agln, i * AGLN_SIZE + 4)?);
        let count = usize::from(u8_at(agln, i * AGLN_SIZE + 6)?);
        let aglc = self.bytes(&self.aglc);
        (first..first + count).map(|c| u24_at(aglc, c * 3)).collect()
    }

    /// The code point AGLFN gives `name`, when it is an AGLFN name.
    pub fn aglfn(&self, name: &str) -> Option<u32> {
        let aglf = self.bytes(&self.aglf);
        let i = Self::search(aglf.len() / AGLF_SIZE, name, |i| self.str_ref(aglf, i * AGLF_SIZE).map(std::borrow::Cow::Borrowed))?;
        u24_at(aglf, i * AGLF_SIZE + 4)
    }
}
