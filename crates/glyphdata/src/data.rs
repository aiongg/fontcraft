//! The glyph database: built-in tables plus custom records, lookup, and synthesis from the name.
//!
//! The synthesis rules (suffix split, ligature split with the script suffix distributed, the
//! category of ligatures, the general-category fallback through the Adobe Glyph List, and
//! production names built from parts) are ported from fontc's `glyphs-reader/src/glyphdata.rs`
//! (glyphs-reader 1.0.0, MIT OR Apache-2.0), which ports them from glyphsLib's `glyphdata.py`
//! (Apache-2.0, Copyright 2016 Google Inc.). Where the two differ, fontc's form is kept; the
//! cross-check in `cargo xtask glyphdata --check` lists the differences with glyphsLib.

use std::collections::HashMap;

use crate::Error;
use crate::agl;
use crate::names::{code_point_name, parse_code_point_name};
use crate::record::Record;
use crate::tables::{RecId, Tables};
use crate::types::{BidiClass, Case, Category, Direction, GeneralCategory, SubCategory};
use crate::xml;

/// Longest glyph name that lookup, synthesis and naming work on; longer names are unknown.
pub const MAX_NAME_BYTES: usize = 1024;
/// Most code points of one glyph that lookup tries.
pub const MAX_CODE_POINTS: usize = 64;
/// Most custom glyph data files merged into one database.
pub const MAX_CUSTOM_FILES: usize = 64;

/// Where a property's value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A record of a custom glyph data file.
    Custom,
    /// A record of the built-in database.
    Database,
    /// Derived from the name by the suffix, ligature or code-point rules.
    Synthesised,
    /// No rule applied: the glyph's own name as production name, left to right as direction.
    Default,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Custom => "custom",
            Source::Database => "database",
            Source::Synthesised => "synthesised",
            Source::Default => "default",
        }
    }
}

/// A value and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: Source,
}

impl<T> Sourced<T> {
    fn new(value: T, source: Source) -> Sourced<T> {
        Sourced { value, source }
    }
}

/// `Sourced::new` with the source fixed, for `Option::map`.
fn from<T>(source: Source) -> impl Fn(T) -> Sourced<T> {
    move |value| Sourced { value, source }
}

fn syn<T>(value: T) -> Sourced<T> {
    Sourced { value, source: Source::Synthesised }
}

/// How a record was found for a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Name,
    AltName,
    ProductionName,
    CodePoint,
}

/// The record a lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub record: Record,
    pub via: Via,
    /// From a custom glyph data file.
    pub custom: bool,
}

/// Everything the database says about one glyph name, with the source of each value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// The name looked up.
    pub name: String,
    /// The record found directly, if any.
    pub found: Option<Found>,
    pub category: Option<Sourced<Category>>,
    pub sub_category: Option<Sourced<SubCategory>>,
    pub unicode: Option<Sourced<u32>>,
    /// The name the glyph exports under; the glyph's own name when nothing gives another.
    pub production_name: Sourced<String>,
    pub script: Option<Sourced<String>>,
    pub case: Option<Sourced<Case>>,
    pub direction: Sourced<Direction>,
    pub description: Option<Sourced<String>>,
}

/// Where a direct hit lives.
#[derive(Clone, Copy, Debug)]
enum Hit {
    Custom(usize),
    Builtin(RecId),
    Han(u32),
}

/// Custom records, merged from files in order.
#[derive(Default)]
struct Custom {
    records: Vec<Record>,
    /// Own names, then alternative names that no own name shadows.
    by_name: HashMap<String, usize>,
    by_code_point: HashMap<u32, usize>,
}

/// The glyph database: the built-in tables and custom records on top.
pub struct GlyphData {
    tables: &'static Tables,
    custom: Custom,
}

impl GlyphData {
    /// The built-in database.
    pub fn new() -> Result<GlyphData, Error> {
        Ok(GlyphData { tables: crate::tables::builtin()?, custom: Custom::default() })
    }

    /// The built-in database with custom records on top. `layers` are files in order: a later
    /// file's record replaces an earlier one of the same name, and replaces the built-in record
    /// of that name as a whole.
    pub fn with_custom(layers: Vec<Vec<Record>>) -> Result<GlyphData, Error> {
        if layers.len() > MAX_CUSTOM_FILES {
            return Err(Error::TooLarge { what: "custom glyph data files", size: layers.len(), limit: MAX_CUSTOM_FILES });
        }
        let mut custom = Custom::default();
        let mut slot: HashMap<String, usize> = HashMap::new();
        // (layer, position in its file) of each slot, so a later file's code point wins and
        // within a file the first record wins.
        let mut rank: Vec<(usize, usize)> = Vec::new();
        for (layer, records) in layers.into_iter().enumerate() {
            for (pos, r) in records.into_iter().enumerate() {
                match slot.get(&r.name) {
                    Some(&i) => {
                        if let (Some(s), Some(k)) = (custom.records.get_mut(i), rank.get_mut(i)) {
                            *s = r;
                            *k = (layer, pos);
                        }
                    }
                    None => {
                        slot.insert(r.name.clone(), custom.records.len());
                        custom.records.push(r);
                        rank.push((layer, pos));
                    }
                }
            }
        }
        for (i, r) in custom.records.iter().enumerate() {
            custom.by_name.insert(r.name.clone(), i);
        }
        for (i, r) in custom.records.iter().enumerate() {
            for alt in &r.alt_names {
                custom.by_name.entry(alt.clone()).or_insert(i);
            }
        }
        let mut order: Vec<usize> = (0..custom.records.len()).collect();
        order.sort_by_key(|i| rank.get(*i).map(|(l, p)| (std::cmp::Reverse(*l), *p)));
        for i in order {
            if let Some(cp) = custom.records.get(i).and_then(|r| r.unicode) {
                custom.by_code_point.entry(cp).or_insert(i);
            }
        }
        Ok(GlyphData { tables: crate::tables::builtin()?, custom })
    }

    /// Reads custom glyph data files (their bytes, in order) and puts them on the built-in
    /// database. A file that fails to parse fails the whole call, naming the file's position.
    pub fn with_custom_files(files: &[&[u8]]) -> Result<GlyphData, Error> {
        let mut layers = Vec::new();
        for (i, bytes) in files.iter().enumerate() {
            let parsed = xml::parse(bytes).map_err(|e| Error::File { index: i, error: Box::new(e) })?;
            layers.push(parsed.records);
        }
        GlyphData::with_custom(layers)
    }

    /// The `key=value` lines naming the sources of the built-in tables.
    pub fn sources(&self) -> &str {
        self.tables.meta()
    }

    pub(crate) fn tables(&self) -> &Tables {
        self.tables
    }

    pub(crate) fn custom_records(&self) -> &[Record] {
        &self.custom.records
    }

    pub(crate) fn custom_shadows(&self, name: &str) -> bool {
        self.custom.by_name.get(name).and_then(|i| self.custom.records.get(*i)).is_some_and(|r| r.name == name)
    }

    // ── direct lookup ──

    fn hit_record(&self, hit: Hit) -> Option<Record> {
        match hit {
            Hit::Custom(i) => self.custom.records.get(i).cloned(),
            Hit::Builtin(i) => self.tables.record(i),
            Hit::Han(cp) => self.tables.han_record(cp),
        }
    }

    /// The record for a name or one of the code points, without synthesis (05-font-view.md
    /// § 4.2, steps 2 and 3): custom by name, custom by code point, then built-in by nice name,
    /// alternative name, production name and code point.
    fn direct(&self, name: &str, code_points: &[u32]) -> Option<(Hit, Via)> {
        if name.len() > MAX_NAME_BYTES {
            return None;
        }
        let code_points = code_points.get(..MAX_CODE_POINTS).unwrap_or(code_points);
        if let Some(&i) = self.custom.by_name.get(name) {
            let via = if self.custom.records.get(i).is_some_and(|r| r.name == name) { Via::Name } else { Via::AltName };
            return Some((Hit::Custom(i), via));
        }
        if let Some(&i) = code_points.iter().find_map(|cp| self.custom.by_code_point.get(cp)) {
            return Some((Hit::Custom(i), Via::CodePoint));
        }
        if let Some(i) = self.tables.find_name(name) {
            return Some((Hit::Builtin(i), Via::Name));
        }
        if let Some(cp) = parse_code_point_name(name)
            && self.tables.han(cp).is_some()
        {
            return Some((Hit::Han(cp), Via::Name));
        }
        if let Some(i) = self.tables.find_alt_name(name) {
            return Some((Hit::Builtin(i), Via::AltName));
        }
        if let Some(i) = self.tables.find_production_name(name) {
            return Some((Hit::Builtin(i), Via::ProductionName));
        }
        code_points.iter().find_map(|cp| {
            if let Some(i) = self.tables.find_code_point(*cp) {
                Some((Hit::Builtin(i), Via::CodePoint))
            } else {
                self.tables.han(*cp).map(|_| (Hit::Han(*cp), Via::CodePoint))
            }
        })
    }

    fn direct_record(&self, name: &str) -> Option<Record> {
        self.direct(name, &[]).and_then(|(hit, _)| self.hit_record(hit))
    }

    /// Whether `name` is a record's nice name or alternative name.
    pub fn contains_name(&self, name: &str) -> bool {
        self.custom.by_name.contains_key(name)
            || self.tables.find_name(name).is_some()
            || self.tables.find_alt_name(name).is_some()
            || parse_code_point_name(name).is_some_and(|cp| self.tables.han(cp).is_some())
    }

    /// The record whose nice name is `name` (custom first), without synthesis.
    pub fn record(&self, name: &str) -> Option<Record> {
        if let Some(r) = self.custom.by_name.get(name).and_then(|i| self.custom.records.get(*i)).filter(|r| r.name == name) {
            return Some(r.clone());
        }
        if let Some(i) = self.tables.find_name(name) {
            return self.tables.record(i);
        }
        parse_code_point_name(name).and_then(|cp| self.tables.han_record(cp))
    }

    /// The record whose `unicode` is `cp` (custom first).
    pub fn by_code_point(&self, cp: u32) -> Option<Record> {
        if let Some(r) = self.custom.by_code_point.get(&cp).and_then(|i| self.custom.records.get(*i)) {
            return Some(r.clone());
        }
        match self.tables.find_code_point(cp) {
            Some(i) => self.tables.record(i),
            None => self.tables.han_record(cp),
        }
    }

    /// The record whose `unicodeLegacy` is `cp` (custom first). Lookup never uses legacy code
    /// points; this is for adding a glyph by a legacy character (`glyphData.legacyUnicode`).
    pub fn by_legacy_code_point(&self, cp: u32) -> Option<Record> {
        if let Some(r) = self.custom.records.iter().find(|r| r.unicode_legacy == Some(cp)) {
            return Some(r.clone());
        }
        self.tables.find_legacy_code_point(cp).and_then(|i| self.tables.record(i))
    }

    // ── lookup with synthesis ──

    /// Everything the database says about a glyph with this name and these code points
    /// (05-font-view.md § 4.2 and § 4.4, without the values stored on the glyph).
    pub fn lookup(&self, name: &str, code_points: &[u32]) -> Info {
        if let Some((hit, via)) = self.direct(name, code_points)
            && let Some(record) = self.hit_record(hit)
        {
            let custom = matches!(hit, Hit::Custom(_));
            let src = if custom { Source::Custom } else { Source::Database };
            // An alternative name supplies everything but the code point.
            let unicode = if via == Via::AltName { None } else { record.unicode };
            let production_name = match &record.production {
                Some(p) => Sourced::new(p.clone(), src),
                None => Sourced::new(name.to_string(), Source::Default),
            };
            let direction = self.direction_for(record.direction.map(from(src)), record.script.as_deref().map(|s| (s, src)), unicode);
            return Info {
                name: name.to_string(),
                category: record.category.map(from(src)),
                sub_category: record.sub_category.map(from(src)),
                unicode: unicode.map(from(src)),
                production_name,
                script: record.script.clone().map(from(src)),
                case: record.case.map(from(src)),
                direction,
                description: record.description.clone().map(from(src)),
                found: Some(Found { record, via, custom }),
            };
        }
        self.synthesise(name)
    }

    fn synthesise(&self, name: &str) -> Info {
        let mut info = Info {
            name: name.to_string(),
            found: None,
            category: None,
            sub_category: None,
            unicode: None,
            production_name: Sourced::new(name.to_string(), Source::Default),
            script: None,
            case: None,
            direction: Sourced::new(Direction::Ltr, Source::Default),
            description: None,
        };
        if name.len() > MAX_NAME_BYTES {
            return info;
        }
        if let Some((category, sub)) = self.synthesise_category(name) {
            info.category = Some(syn(category));
            info.sub_category = sub.map(syn);
        }
        if let Some(p) = self.synthesise_production_name(name) {
            info.production_name = syn(p);
        }
        info.unicode = parse_code_point_name(name).map(syn);
        // Script, case and direction follow the base glyph of a suffixed name; a ligature takes
        // the script its known parts share. Helper glyphs (`_…`) get neither, as they get no
        // category.
        let (base, suffix) = self.split_suffix(name);
        let mut stated_direction = None;
        if name.starts_with('_') {
            return info;
        }
        if suffix.is_some() {
            if let Some(b) = self.direct_record(base) {
                info.script = b.script.map(syn);
                info.case = b.case.map(syn);
                stated_direction = b.direction.map(syn);
            }
            if suffix.is_some_and(|s| s.split('.').any(|p| matches!(p, "sc" | "smcp" | "c2sc"))) {
                info.case = Some(syn(Case::SmallCaps));
            }
        }
        if info.script.is_none()
            && let Some(parts) = self.split_ligature(base)
        {
            let scripts: Vec<Option<String>> = parts.iter().map(|p| self.direct_record(p).and_then(|r| r.script)).collect();
            if let Some(Some(first)) = scripts.iter().find(|s| s.is_some())
                && scripts.iter().all(|s| s.as_ref().is_none_or(|s| s == first))
            {
                info.script = Some(syn(first.clone()));
            }
        }
        let first_char = agl::to_unicode(self.tables, base).first().map(|c| u32::from(*c));
        info.direction = self.direction_for(stated_direction, info.script.as_ref().map(|s| (s.value.as_str(), Source::Synthesised)), first_char);
        info
    }

    /// Direction: the stated one, else right to left when the script is written so, else right
    /// to left when the character is a strong right-to-left one, else left to right.
    fn direction_for(&self, stated: Option<Sourced<Direction>>, script: Option<(&str, Source)>, cp: Option<u32>) -> Sourced<Direction> {
        if let Some(d) = stated {
            return d;
        }
        if let Some((s, src)) = script
            && self.is_rtl_script(s)
        {
            return Sourced::new(Direction::Rtl, src);
        }
        if cp.is_some_and(|cp| self.tables.bidi_class(cp).is_rtl()) {
            return Sourced::new(Direction::Rtl, Source::Synthesised);
        }
        Sourced::new(Direction::Ltr, Source::Default)
    }

    /// Whether the data's script id is written right to left (Unicode's letters of that script
    /// are mostly strong right to left).
    pub fn is_rtl_script(&self, script: &str) -> bool {
        self.tables.is_rtl_script(script)
    }

    /// Splits a name into base and suffix (05-font-view.md § 4.2, synthesis rule 2): the base
    /// is everything before the first dot, unless the name has several dots and a longer prefix
    /// ending at the second, third… dot is a known name.
    pub fn split_suffix<'n>(&self, name: &'n str) -> (&'n str, Option<&'n str>) {
        if name.len() <= MAX_NAME_BYTES && name.bytes().filter(|b| *b == b'.').count() > 1 {
            for (idx, _) in name.match_indices('.').skip(1) {
                if let (Some(base), Some(suffix)) = (name.get(..idx), name.get(idx + 1..))
                    && self.contains_name(base)
                {
                    return (base, Some(suffix));
                }
            }
        }
        crate::names::split_first_dot(name)
    }

    /// Splits a ligature name into its parts (05-font-view.md § 4.2, synthesis rule 4): `None`
    /// without `_`. A script suffix on the last part goes to every part that lacks one, unless
    /// the bare part is a known name and the suffixed one is not.
    pub fn split_ligature(&self, name: &str) -> Option<Vec<String>> {
        if name.len() > MAX_NAME_BYTES {
            return None;
        }
        let script = name.rsplit_once('_')?.1.rsplit_once('-').map(|(_, s)| s);
        let stem = name.trim_end_matches(script.unwrap_or_default()).trim_end_matches('-');
        let mut parts: Vec<String> = stem.split('_').map(str::to_string).collect();
        let Some(script) = script else {
            return Some(parts);
        };
        for part in &mut parts {
            if part.contains('-') {
                continue;
            }
            let suffixed = format!("{part}-{script}");
            if self.contains_name(part) && !self.contains_name(&suffixed) {
                continue;
            }
            *part = suffixed;
        }
        Some(parts)
    }

    fn category_of(&self, name: &str) -> Option<(Category, Option<SubCategory>)> {
        let r = self.direct_record(name)?;
        Some((r.category?, r.sub_category))
    }

    /// The category a name gets from the synthesis rules (05-font-view.md § 4.2).
    fn synthesise_category(&self, name: &str) -> Option<(Category, Option<SubCategory>)> {
        if name.starts_with('_') {
            return None;
        }
        let (base, _) = self.split_suffix(name);
        if let Some(c) = self.category_of(base) {
            return Some(c);
        }
        if let Some(parts) = self.split_ligature(base) {
            let cats: Vec<Option<(Category, Option<SubCategory>)>> = parts.iter().map(|p| self.category_of(p)).collect();
            if let Some(Some((first, first_sub))) = cats.first() {
                match first {
                    Category::Mark => return Some((Category::Mark, *first_sub)),
                    Category::Letter => {
                        let rest_attach = cats.iter().skip(1).all(|c| matches!(c, None | Some((Category::Mark | Category::Separator, _))));
                        return Some(if rest_attach { (*first, *first_sub) } else { (Category::Letter, Some(SubCategory::Ligature)) });
                    }
                    _ => {}
                }
            }
        }
        let first = agl::to_unicode(self.tables, base).into_iter().next()?;
        let (category, sub) = self.tables.general_category(u32::from(first)).glyph_category();
        if base.contains('_') && category != Category::Mark { Some((category, Some(SubCategory::Ligature))) } else { Some((category, sub)) }
    }

    /// The production name the synthesis rules give (05-font-view.md § 4.4, step 3), if any.
    fn synthesise_production_name(&self, name: &str) -> Option<String> {
        let (base, suffix) = self.split_suffix(name);
        let with_suffix = |mut s: String| {
            if let Some(suffix) = suffix {
                s.push('.');
                s.push_str(suffix);
            }
            s
        };
        if suffix.is_some()
            && let Some(p) = self.direct_record(base).and_then(|r| r.production)
        {
            return Some(with_suffix(p));
        }
        let parts = self.split_ligature(base).unwrap_or_else(|| vec![base.to_string()]);
        let mut productions = Vec::with_capacity(parts.len());
        for part in parts {
            // A part's production name, or the part itself when it is an AGLFN name without one.
            let record = self.direct_record(&part)?;
            match record.production {
                Some(p) => productions.push(p),
                None if self.tables.aglfn(&part).is_some() => productions.push(part),
                None => return None,
            }
        }
        let is_u_name = |n: &str| n.strip_prefix('u').is_some_and(|d| d.bytes().all(|b| b.is_ascii_hexdigit()));
        let beyond_bmp = productions.iter().any(|n| n.len() > 5 && is_u_name(n));
        let any_uni = productions.iter().any(|n| n.starts_with("uni"));
        if !beyond_bmp && any_uni {
            let mut out = String::from("uni");
            for part in &productions {
                if let Some(hex) = part.strip_prefix("uni") {
                    out.push_str(hex);
                } else if part.len() == 5 && is_u_name(part) {
                    out.push_str(part.get(1..).unwrap_or(""));
                } else {
                    // A part with a production name of another form (`i.loclTRK`) gives no
                    // code-point name.
                    let cp = self.tables.aglfn(part)?;
                    out.push_str(&format!("{cp:04X}"));
                }
            }
            return Some(with_suffix(out));
        }
        Some(with_suffix(productions.join("_")))
    }

    /// The name a glyph exports under (05-font-view.md § 4.4, without the glyph's own
    /// override).
    pub fn production_name(&self, name: &str, code_points: &[u32]) -> String {
        self.lookup(name, code_points).production_name.value
    }

    // ── names ──

    /// The nice name for a code point: the record's name, else the code-point name.
    pub fn nice_name(&self, cp: u32) -> String {
        self.by_code_point(cp).map(|r| r.name).unwrap_or_else(|| code_point_name(cp))
    }

    /// The nice name of a name without its suffixes: a record found by nice, alternative or
    /// production name, else the record of the single character the name stands for.
    fn canonical_base(&self, base: &str) -> Option<String> {
        if let Some(r) = self.direct_record(base) {
            return Some(r.name);
        }
        if base.contains('_') {
            return None;
        }
        match agl::to_unicode(self.tables, base).as_slice() {
            [c] => self.by_code_point(u32::from(*c)).map(|r| r.name),
            _ => None,
        }
    }

    /// The nice name for a legacy, production, alternative or code-point name, with its
    /// suffixes kept (`uni0410.sc` → `A-cy.sc`, `uni06440627` stays unknown, `uni0644_uni0627`
    /// → `lam_alef-ar`). `None` when the database does not know the name.
    pub fn canonical_name(&self, name: &str) -> Option<String> {
        if name.len() > MAX_NAME_BYTES || name.is_empty() {
            return None;
        }
        if let Some(r) = self.direct_record(name) {
            return Some(r.name);
        }
        let (base, suffix) = self.split_suffix(name);
        let nice = match self.canonical_base(base) {
            Some(n) => n,
            None => {
                let parts = self.split_ligature(base)?;
                let nice: Vec<String> = parts.iter().map(|p| self.canonical_base(p)).collect::<Option<_>>()?;
                // A script suffix shared by every part is written once, at the end.
                let script = nice.first().and_then(|n| n.rsplit_once('-')).map(|(_, s)| s.to_string());
                match script.filter(|s| nice.iter().all(|n| n.ends_with(&format!("-{s}")))) {
                    Some(s) => {
                        let stems: Vec<&str> = nice.iter().map(|n| n.strip_suffix(&format!("-{s}")).unwrap_or(n)).collect();
                        format!("{}-{s}", stems.join("_"))
                    }
                    None => nice.join("_"),
                }
            }
        };
        Some(match suffix {
            Some(s) => format!("{nice}.{s}"),
            None => nice,
        })
    }

    // ── Unicode properties and the Adobe Glyph List ──

    pub fn general_category(&self, cp: u32) -> GeneralCategory {
        self.tables.general_category(cp)
    }

    pub fn bidi_class(&self, cp: u32) -> BidiClass {
        self.tables.bidi_class(cp)
    }

    /// The Unicode Script property value of a code point (`Latin`, `Unknown`…).
    pub fn unicode_script(&self, cp: u32) -> Option<&str> {
        self.tables.unicode_script(cp)
    }

    /// The data's script id for a code point without a record: the one most records of its
    /// Unicode script carry.
    pub fn script_for_code_point(&self, cp: u32) -> Option<&str> {
        self.tables.script_for_code_point(cp)
    }

    /// Whether a code point is assigned in the Unicode version of the tables.
    pub fn is_assigned(&self, cp: u32) -> bool {
        self.tables.general_category(cp) != GeneralCategory::Cn
    }

    /// The characters a glyph name stands for by the Adobe Glyph List rules.
    pub fn name_to_text(&self, name: &str) -> String {
        if name.len() > MAX_NAME_BYTES {
            return String::new();
        }
        agl::to_unicode(self.tables, name).into_iter().collect()
    }

    /// Whether `name` is in AGLFN, the glyph list recommended for new fonts.
    pub fn is_aglfn_name(&self, name: &str) -> bool {
        self.tables.aglfn(name).is_some()
    }
}
