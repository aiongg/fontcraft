//! Name table checks (M12.50): the `check.names.*` rows of `spec/inventory/21-qa-checks.toml`.
//! Every runner reads one compiled font ([`BinaryCtx`]). The rules follow FontBakery's name
//! checks and comparefamily's single-face tests 1 to 6, 8, 20 and 21 (both Apache-2.0); each
//! runner names the reference rule it follows.
//!
//! Records are decoded once per check with a byte budget ([`MAX_NAME_TEXT_BYTES`]): a hostile
//! table can point 65,535 records at the same 64 KiB string. Records in an encoding we cannot
//! decode (anything but Unicode, Windows Unicode and Mac Roman) are left out of every rule.
//! Findings name the record by `nameId`, `platform` and `language` (decimal); `sourceField` is
//! the source field the [`crate::SourceMap`] gives for the key `name.<id>`.

use std::collections::{BTreeMap, BTreeSet};

use read_fonts::TableProvider;
use read_fonts::ps::cff::dict::{self, Entry};

use crate::name_records::{self, MAX_NAME_TEXT_BYTES};
use crate::{BinaryCtx, Hit, Location, Params, Sink, Threshold};

/// Most family-and-instance name pairs `familyAndStyleLength` builds.
const MAX_LENGTH_COMPARISONS: usize = 1_000_000;

pub(crate) const MAX_DESCRIPTION_LENGTH: Threshold = Threshold {
    name: "maxDescriptionLength",
    default: 200.0,
    min: 1.0,
    max: 65_535.0,
    doc: "Descriptions (name ID 10) longer than this many characters are flagged.",
};

type Key = (u16, u16, u16);
/// Windows platform, Unicode BMP encoding, US English.
const WIN: Key = (3, 1, 0x409);
/// Macintosh platform, Roman encoding, English.
const MAC: Key = (1, 0, 0);
const RIBBI: [&str; 4] = ["Regular", "Bold", "Italic", "Bold Italic"];
/// The menu names of comparefamily tests 1, 2, 20 and 21.
const MENU_NAMES: [u16; 6] = [1, 2, 4, 16, 17, 18];

struct Rec {
    platform: u16,
    encoding: u16,
    language: u16,
    id: u16,
    /// `None` when the encoding is not one we decode, the string is out of bounds, or the budget
    /// ran out.
    text: Option<String>,
}

impl Rec {
    fn key(&self) -> Key {
        (self.platform, self.encoding, self.language)
    }

    fn describe(&self) -> String {
        format!("name ID {} (platform {}, encoding {}, language {})", self.id, self.platform, self.encoding, self.language)
    }
}

/// The decoded records of the `name` table, with lookups by id and key.
struct Names {
    recs: Vec<Rec>,
    /// (name id, key) → index of the first record with them.
    first: BTreeMap<(u16, Key), usize>,
    /// name id → indices of its records, in table order.
    by_id: BTreeMap<u16, Vec<usize>>,
}

impl Names {
    /// Reads the table, or records the skip and returns `None` when it cannot be read.
    fn read(ctx: &BinaryCtx<'_>, sink: &mut Sink) -> Option<Self> {
        let mut budget = MAX_NAME_TEXT_BYTES;
        let read = match name_records::read(&ctx.font, &mut budget, name_records::all) {
            Ok(read) => read,
            Err(reason) => {
                sink.skip(reason);
                return None;
            }
        };
        if read.over_budget {
            sink.skip(name_records::over_budget_reason());
        }
        let recs: Vec<Rec> = read
            .records
            .into_iter()
            .map(|r| Rec { platform: r.platform, encoding: r.encoding, language: r.language, id: r.id, text: r.text })
            .collect();
        let mut first = BTreeMap::new();
        let mut by_id: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
        for (i, r) in recs.iter().enumerate() {
            first.entry((r.id, r.key())).or_insert(i);
            by_id.entry(r.id).or_default().push(i);
        }
        Some(Self { recs, first, by_id })
    }

    fn get(&self, id: u16, key: Key) -> Option<&Rec> {
        self.first.get(&(id, key)).and_then(|&i| self.recs.get(i))
    }

    fn text(&self, id: u16, key: Key) -> Option<&str> {
        self.get(id, key).and_then(|r| r.text.as_deref())
    }

    /// The Windows English record of `id`.
    fn win(&self, id: u16) -> Option<&str> {
        self.text(id, WIN)
    }

    /// The decoded records of `id`, in table order.
    fn all(&self, id: u16) -> impl Iterator<Item = (&Rec, &str)> {
        self.by_id.get(&id).into_iter().flatten().filter_map(|&i| self.recs.get(i)).filter_map(|r| r.text.as_deref().map(|t| (r, t)))
    }

    /// The text of the first record of `id` in table order, whatever its platform.
    fn first_of(&self, id: u16) -> Option<&str> {
        self.by_id.get(&id).and_then(|v| v.first()).and_then(|&i| self.recs.get(i)).and_then(|r| r.text.as_deref())
    }

    /// Every decoded record.
    fn decoded(&self) -> impl Iterator<Item = (&Rec, &str)> {
        self.recs.iter().filter_map(|r| r.text.as_deref().map(|t| (r, t)))
    }

    fn has_mac(&self) -> bool {
        self.recs.iter().any(|r| r.platform == 1)
    }
}

/// The location of a name record.
fn at(ctx: &BinaryCtx<'_>, id: u16, key: Key) -> Location {
    Location {
        table: Some("name".into()),
        name_id: Some(id),
        platform: Some(key.0),
        language: Some(key.2.to_string()),
        source_field: ctx.source.and_then(|m| m.field(&format!("name.{id}"))).map(str::to_string),
        ..Location::default()
    }
}

fn at_rec(ctx: &BinaryCtx<'_>, r: &Rec) -> Location {
    at(ctx, r.id, r.key())
}

fn len(s: &str) -> usize {
    s.chars().count()
}

/// `check.names.requiredRecords` (FontBakery `name_id_1`, `name_id_2`, the mandatory-entries
/// checks, the copyright, manufacturer and trademark checks; comparefamily's name loader).
pub(crate) fn required_records(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (id, what) in [(1, "family name"), (2, "subfamily name"), (4, "full name"), (6, "PostScript name")] {
        match names.get(id, WIN) {
            None => sink.push(Hit::new("missing", format!("The Windows English {what} (name ID {id}) is missing.")).place(at(ctx, id, WIN))),
            Some(r) if r.text.as_deref().is_some_and(|t| t.trim().is_empty()) => {
                sink.push(Hit::new("empty", format!("The Windows English {what} (name ID {id}) is empty.")).place(at(ctx, id, WIN)));
            }
            Some(_) => {}
        }
    }
    for (id, what) in [(0, "copyright"), (5, "version string"), (7, "trademark"), (8, "manufacturer"), (14, "licence URL")] {
        if names.get(id, WIN).is_none_or(|r| r.text.as_deref().is_some_and(|t| t.trim().is_empty())) {
            sink.push(
                Hit::new("missing-recommended", format!("The Windows English {what} (name ID {id}) is missing or empty.")).place(at(ctx, id, WIN)),
            );
        }
    }
}

/// `check.names.emptyRecords` (FontBakery `opentype/name/empty_records`).
pub(crate) fn empty_records(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (r, t) in names.decoded() {
        if t.trim().is_empty() {
            sink.push(Hit::new("empty-record", format!("The record {} is empty.", r.describe())).place(at_rec(ctx, r)));
        }
    }
}

/// `check.names.whitespace` (FontBakery `name/trailing_spaces`; comparefamily tests 20 and 21).
pub(crate) fn whitespace(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (r, t) in names.decoded() {
        if t.trim_end() != t {
            sink.push(Hit::new("trailing-space", format!("The record {} ends in white space: '{t}'.", r.describe())).place(at_rec(ctx, r)));
        } else if t.trim_start() != t {
            sink.push(Hit::new("leading-space", format!("The record {} starts with white space: '{t}'.", r.describe())).place(at_rec(ctx, r)));
        }
        if MENU_NAMES.contains(&r.id) && t.contains("  ") {
            sink.push(Hit::new("double-space", format!("The menu name {} contains two spaces in a row: '{t}'.", r.describe())).place(at_rec(ctx, r)));
        }
    }
}

/// `check.names.lineBreaks` (FontBakery `googlefonts/name/line_breaks`, which looks for line
/// feeds; carriage returns count too).
pub(crate) fn line_breaks(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (r, t) in names.decoded() {
        if t.contains(['\n', '\r']) {
            sink.push(Hit::new("line-break", format!("The record {} contains a line break.", r.describe())).place(at_rec(ctx, r)));
        }
    }
}

/// The characters FontBakery's `name/char_restrictions` allows in IDs 6 and 20: printable ASCII
/// without `[](){}<>/%`, backslash and space.
fn restricted_ok(c: char) -> bool {
    matches!(c, '!'..='$' | '&' | '\'' | '*'..='.' | '0'..=';' | '=' | '?'..='Z' | '^'..='z' | '|' | '~')
}

/// `check.names.postScriptNameCharacters` (FontBakery `opentype/postscript_name` for ID 6,
/// `name/char_restrictions` for IDs 20 and 25).
pub(crate) fn postscript_name_characters(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (r, t) in names.all(6) {
        if !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            let msg = format!("The PostScript name '{t}' in {} uses characters other than A-Z, a-z, 0-9 and hyphen.", r.describe());
            sink.push(Hit::new("bad-psname", msg).place(at_rec(ctx, r)));
        }
    }
    for (r, t) in names.all(20) {
        if !t.chars().all(restricted_ok) {
            let msg = format!("The PostScript CID name '{t}' in {} uses characters outside printable ASCII or one of []{{}}()<>/%.", r.describe());
            sink.push(Hit::new("bad-string", msg).place(at_rec(ctx, r)));
        }
    }
    for (r, t) in names.all(25) {
        if !t.chars().all(|c| c.is_ascii_alphanumeric()) {
            let msg = format!("The variations PostScript name prefix '{t}' in {} uses characters other than letters and digits.", r.describe());
            sink.push(Hit::new("bad-string", msg).place(at_rec(ctx, r)));
        }
    }
}

/// `check.names.postScriptNameConsistency` (FontBakery `opentype/name/postscript_name_consistency`
/// and `opentype/name/postscript_vs_cff`; comparefamily test 5).
pub(crate) fn postscript_name_consistency(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    let distinct: BTreeSet<&str> = names.all(6).map(|(_, t)| t).collect();
    if distinct.len() > 1 {
        let list = distinct.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ");
        let place = Location { table: Some("name".into()), name_id: Some(6), ..Location::default() };
        sink.push(
            Hit::new("inconsistency", format!("The PostScript name records (name ID 6) differ: {list}."))
                .data("names", distinct.len() as f64)
                .place(place),
        );
    }
    let Ok(cff) = ctx.font.cff() else { return };
    let cff_place = Location { table: Some("CFF ".into()), field: Some("FontName".into()), ..Location::default() };
    if cff.names().count() != 1 {
        let msg = format!("The CFF table holds {} font names; it must hold exactly one.", cff.names().count());
        sink.push(Hit::new("cff-name-error", msg).place(cff_place));
        return;
    }
    let Some(cff_name) = cff.name(0).map(String::from_utf8_lossy) else { return };
    for (r, t) in names.all(6) {
        if t != cff_name {
            let msg = format!("The PostScript name '{t}' in {} differs from the CFF font name '{cff_name}'.", r.describe());
            sink.push(Hit::new("ps-cff-name-mismatch", msg).place(at_rec(ctx, r)));
        }
    }
}

/// The CFF Top DICT's FullName, when the font has one.
fn cff_full_name(ctx: &BinaryCtx<'_>) -> Option<String> {
    let cff = ctx.font.cff().ok()?;
    let top = cff.top_dicts().get(0)?;
    let sid = dict::entries(top, None).filter_map(Result::ok).find_map(|e| match e {
        Entry::FullName(sid) => Some(sid),
        _ => None,
    })?;
    cff.string(sid).map(|b| String::from_utf8_lossy(b).into_owned())
}

/// `check.names.fullNameStartsWithFamily` (FontBakery `opentype/name/match_familyname_fullfont`;
/// comparefamily test 3, whose CFF FullName comparison is a note).
pub(crate) fn full_name_starts_with_family(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    let mut compared = false;
    let mut seen = BTreeSet::new();
    for (r, full) in names.all(4) {
        if !seen.insert(r.key()) {
            continue;
        }
        for fid in [1, 16, 21] {
            let Some(family) = names.text(fid, r.key()) else { continue };
            compared = true;
            if !full.starts_with(family) {
                let msg = format!("The full name '{full}' does not begin with the family name '{family}' (name ID {fid}) in {}.", r.describe());
                sink.push(Hit::new("mismatch-font-names", msg).place(at_rec(ctx, r)));
            }
        }
    }
    if !compared {
        let place = Location { table: Some("name".into()), name_id: Some(4), ..Location::default() };
        let msg = "The name table has no full name (ID 4) with a family name (ID 1, 16 or 21) of the same platform, encoding and language.";
        sink.push(Hit::new("missing-font-names", msg).place(place));
    }
    let full = names.win(4).or_else(|| names.text(4, MAC));
    if let (Some(full), Some(cff)) = (full, cff_full_name(ctx))
        && full != cff
    {
        let place = Location { table: Some("CFF ".into()), field: Some("FullName".into()), ..Location::default() };
        let msg = format!(
            "The CFF FullName '{cff}' differs from the full name '{full}'. Nothing reads the CFF copy, but two values for one field confuse tools."
        );
        sink.push(Hit::new("cff-full-name", msg).place(place));
    }
}

/// `check.names.recommendedLengths` (FontBakery `opentype/family_naming_recommendations`).
pub(crate) fn recommended_lengths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (id, limit) in [(4, 63), (6, 63), (1, 31), (2, 31), (16, 31), (17, 31)] {
        for (r, t) in names.all(id) {
            let n = len(t);
            if n > limit {
                let msg = format!("The record {} is {n} characters long; the recommended limit is {limit}.", r.describe());
                sink.push(Hit::new("too-long", msg).data("length", n as f64).data("limit", limit as f64).place(at_rec(ctx, r)));
            }
        }
    }
}

/// The family name without a trailing ` Regular`, ` Italic`, ` Bold`, ` BoldItalic` or
/// ` Bold Italic` (the longest that matches, as the leftmost regex match does).
fn strip_ribbi(name: &str) -> &str {
    ["Bold Italic", "BoldItalic", "Regular", "Italic", "Bold"]
        .iter()
        .find_map(|s| name.strip_suffix(s).and_then(|rest| rest.strip_suffix(' ')))
        .unwrap_or(name)
}

const LENGTH_LIMIT: usize = 31;

/// `check.names.familyAndStyleLength` (FontBakery `name/family_and_style_max_length`,
/// `name_length_req`; the instance branch of `instances_name_length_req` is the fvar branch here).
pub(crate) fn family_and_style_length(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (r, t) in names.all(1) {
        let stripped = strip_ribbi(t);
        let n = len(stripped);
        if n > LENGTH_LIMIT {
            let msg = format!("The family name '{stripped}' in {} is {n} characters long, more than {LENGTH_LIMIT}.", r.describe());
            sink.push(Hit::new("nameid1-too-long", msg).data("length", n as f64).place(at_rec(ctx, r)));
        }
    }
    if let (Some(family), Some(sub)) = (names.win(1), names.win(2))
        && !RIBBI.contains(&sub)
    {
        let logfont = format!("{family} {sub}");
        let n = len(&logfont);
        if n > LENGTH_LIMIT {
            let msg = format!("Family and subfamily name together, '{logfont}', are {n} characters long, more than {LENGTH_LIMIT}.");
            sink.push(Hit::new("long-name", msg).data("length", n as f64).place(at(ctx, 1, WIN)));
        }
    }
    let Ok(fvar) = ctx.font.fvar() else { return };
    if let Ok(stat) = ctx.font.stat() {
        stat_combinations(ctx, &names, &stat, sink);
    } else {
        fvar_instances(ctx, &names, &fvar, sink);
    }
}

/// The longest family name plus one non-elidable STAT style name per axis.
fn stat_combinations(ctx: &BinaryCtx<'_>, names: &Names, stat: &read_fonts::tables::stat::Stat<'_>, sink: &mut Sink) {
    let (family, family_id) = match names.win(16) {
        Some(f) => (f, 16),
        None => match names.win(1) {
            Some(f) => (f, 1),
            None => return,
        },
    };
    let Some(Ok(values)) = stat.offset_to_axis_values() else { return };
    let mut longest: BTreeMap<u16, &str> = BTreeMap::new();
    for value in values.axis_values().iter().filter_map(Result::ok) {
        let Some(axis) = value.axis_index() else { continue };
        if value.flags().bits() & 0x0002 != 0 {
            continue;
        }
        let Some(style) = names.win(value.value_name_id().to_u16()) else { continue };
        if style == "Regular" || style == "Italic" {
            continue;
        }
        let slot = longest.entry(axis).or_insert(style);
        if len(style) > len(slot) {
            *slot = style;
        }
    }
    let styles = longest.values().copied().collect::<Vec<_>>().join(" ");
    let name = format!("{family} {styles}");
    let n = len(&name);
    if n > LENGTH_LIMIT {
        let msg = format!(
            "The family name '{family}' (name ID {family_id}) with the STAT style names '{styles}' is {n} characters long, more than {LENGTH_LIMIT}."
        );
        sink.push(Hit::new("familyname-plus-stat-entries-too-long", msg).data("length", n as f64).place(at(ctx, family_id, WIN)));
    }
}

/// Every named instance's family name plus subfamily name, per platform of the family name.
fn fvar_instances(ctx: &BinaryCtx<'_>, names: &Names, fvar: &read_fonts::tables::fvar::Fvar<'_>, sink: &mut Sink) {
    let Ok(instances) = fvar.instances() else { return };
    let sub_ids: BTreeSet<u16> = instances.iter().filter_map(Result::ok).map(|i| i.subfamily_name_id.to_u16()).collect();
    let family_keys: BTreeSet<Key> = names.by_id.get(&1).into_iter().flatten().filter_map(|&i| names.recs.get(i)).map(Rec::key).collect();
    let mut work = 0usize;
    let mut reported = BTreeSet::new();
    for sub in sub_ids {
        for (r, instance) in names.all(sub) {
            for &key in &family_keys {
                work += 1;
                if work > MAX_LENGTH_COMPARISONS {
                    sink.skip(format!("more than {MAX_LENGTH_COMPARISONS} instance names; the rest were not measured"));
                    return;
                }
                let Some(family) = names.text(16, key).or_else(|| names.text(1, key)) else { continue };
                let full = format!("{family} {instance}");
                let n = len(&full);
                if n > LENGTH_LIMIT && reported.insert(full.clone()) {
                    let msg =
                        format!("The named instance '{full}' (family name and {}) is {n} characters long, more than {LENGTH_LIMIT}.", r.describe());
                    sink.push(Hit::new("fvar-instance-too-long", msg).data("length", n as f64).place(at_rec(ctx, r)));
                }
            }
        }
    }
}

/// `check.names.menuNameLimits` (comparefamily tests 1 and 2: the 63-character limits).
pub(crate) fn menu_name_limits(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for id in MENU_NAMES {
        for (r, t) in names.all(id) {
            let n = len(t);
            if n > 63 {
                let msg = format!("The menu name {} is {n} characters long; old systems allow 63.", r.describe());
                sink.push(Hit::new("too-long", msg).data("length", n as f64).place(at_rec(ctx, r)));
            }
        }
    }
}

/// A decimal number `digits.digits` as an exact fraction (numerator, power of ten), or `None`
/// when the whole part has more than six digits (no `head.fontRevision` comes near it). Fraction
/// digits past the eighteenth are dropped; they are far below every tolerance.
fn decimal(whole: &str, frac: &str) -> Option<(i128, i128)> {
    let whole = whole.trim_start_matches('0');
    if whole.len() > 6 {
        return None;
    }
    let frac = frac.get(..frac.len().min(18)).unwrap_or(frac);
    let den = 10i128.checked_pow(u32::try_from(frac.len()).ok()?)?;
    let w: i128 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: i128 = if frac.is_empty() { 0 } else { frac.parse().ok()? };
    Some((w.checked_mul(den)?.checked_add(f)?, den))
}

/// The first `digits.digits` in `s` that starts at the beginning or after a space (FontBakery's
/// `(?: |^)(\d+\.\d+)`), as its whole and fraction digits.
fn version_number(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let digits = |from: usize| bytes.get(from..).map_or(0, |b| b.iter().take_while(|c| c.is_ascii_digit()).count());
    (0..bytes.len()).find_map(|start| {
        if start > 0 && bytes.get(start - 1) != Some(&b' ') {
            return None;
        }
        let w = digits(start);
        if w == 0 || bytes.get(start + w) != Some(&b'.') {
            return None;
        }
        let f = digits(start + w + 1);
        if f == 0 {
            return None;
        }
        Some((s.get(start..start + w)?, s.get(start + w + 1..start + w + 1 + f)?))
    })
}

/// comparefamily's `(Version|OTF)\s+(\d+\.(\d+))`: the whole and fraction digits.
fn prefixed_version(s: &str) -> Option<(&str, &str)> {
    ["Version", "OTF"].iter().find_map(|prefix| {
        s.match_indices(prefix).find_map(|(i, _)| {
            let rest = s.get(i + prefix.len()..)?;
            let trimmed = rest.trim_start();
            if trimmed.len() == rest.len() {
                return None;
            }
            version_number(trimmed).filter(|(w, _)| w.as_ptr() == trimmed.as_ptr())
        })
    })
}

/// `check.names.versionString` (FontBakery `opentype/font_version`; comparefamily test 4 and the
/// version-format checks of the vendor profiles as informational house rules).
pub(crate) fn version_string(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    let Ok(head) = ctx.font.head() else {
        sink.skip("the font has no readable 'head' table");
        return;
    };
    let revision = i128::from(head.font_revision().to_bits());
    let shown = head.font_revision().to_f64();
    let mut any = false;
    for (r, t) in names.all(5) {
        any = true;
        let before = t.split(';').next().unwrap_or(t);
        let Some((whole, frac)) = version_number(before) else {
            let msg = format!("The version string '{t}' in {} has no number of the form major.minor.", r.describe());
            sink.push(Hit::new("parse", msg).place(at_rec(ctx, r)));
            continue;
        };
        // |n/d − revision/65536| against 1/2000 (fail) and 1/65536 (warn), in integers.
        let diff = decimal(whole, frac).and_then(|(n, d)| Some((n.checked_mul(65_536)?.checked_sub(revision.checked_mul(d)?)?.checked_abs()?, d)));
        let (fail, warn) = match diff {
            Some((diff, d)) => (diff.saturating_mul(2_000) > d.saturating_mul(65_536), diff > d),
            None => (true, true),
        };
        if fail || warn {
            let (code, what) = if fail { ("mismatch", "does not match") } else { ("near-mismatch", "matches only to three decimals") };
            let msg = format!("The version string '{t}' in {} {what} head.fontRevision {shown:.5}.", r.describe());
            sink.push(Hit::new(code, msg).data("fontRevision", shown).place(at_rec(ctx, r)));
        }
    }
    if !any {
        sink.push(Hit::new("missing", "The font has no version string (name ID 5).").place(at(ctx, 5, WIN)));
        return;
    }
    let Some(t) = names.win(5).or_else(|| names.first_of(5)) else { return };
    let place = || at(ctx, 5, WIN);
    if t.contains("Development") {
        sink.push(Hit::new("development", format!("The version string '{t}' contains the word Development.")).place(place()));
    }
    match prefixed_version(t) {
        None => {
            sink.push(Hit::new("version-format", format!("The version string '{t}' does not start with 'Version ' and a number.")).place(place()))
        }
        Some((_, frac)) if frac.len() != 3 => {
            sink.push(Hit::new("version-decimals", format!("The version number in '{t}' does not have three decimals.")).place(place()));
        }
        Some((whole, frac)) => {
            if decimal(whole, frac).is_some_and(|(n, d)| n <= d) {
                sink.push(Hit::new("version-not-above-one", format!("The version number in '{t}' is not above 1.000.")).place(place()));
            }
        }
    }
}

/// The style of a compiled font without its file name: the fsSelection bits, then the words of
/// name IDs 17 and 2 (`docs/spec/21-qa.md` § 10, question 9).
fn is_italic(ctx: &BinaryCtx<'_>, names: &Names) -> bool {
    let bit = ctx.font.os2().is_ok_and(|os2| os2.fs_selection().bits() & 0x0001 != 0);
    bit || [17, 2].iter().any(|&id| names.first_of(id).is_some_and(|t| t.contains("Italic")))
}

/// `check.names.italicNames` (FontBakery `name/italic_names`).
pub(crate) fn italic_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    if !is_italic(ctx, &names) {
        return;
    }
    if let Some(t) = names.first_of(1)
        && t.contains("Italic")
    {
        sink.push(
            Hit::new("bad-familyname", format!("The family name '{t}' (name ID 1) of an italic font contains 'Italic'.")).place(at(ctx, 1, WIN)),
        );
    }
    if let Some(t) = names.first_of(2)
        && t != "Italic"
        && t != "Bold Italic"
    {
        let msg = format!("The subfamily name '{t}' (name ID 2) of an italic font is not 'Italic' or 'Bold Italic'.");
        sink.push(Hit::new("bad-subfamilyname", msg).place(at(ctx, 2, WIN)));
    }
    if let Some(t) = names.first_of(16)
        && t.contains("Italic")
    {
        let msg = format!("The typographic family name '{t}' (name ID 16) of an italic font contains 'Italic'.");
        sink.push(Hit::new("bad-typographicfamilyname", msg).place(at(ctx, 16, WIN)));
    }
    if let Some(t) = names.first_of(17)
        && !t.ends_with("Italic")
    {
        let msg = format!("The typographic subfamily name '{t}' (name ID 17) of an italic font does not end in 'Italic'.");
        sink.push(Hit::new("bad-typographicsubfamilyname", msg).place(at(ctx, 17, WIN)));
    }
}

/// `check.names.subfamilyMatchesStyleBits` (comparefamily test 8; FontBakery
/// `microsoft/office_ribz_req`, `fontwerk/style_linking`). The bits are OS/2 fsSelection's, or
/// head.macStyle's when the font has no OS/2 table.
pub(crate) fn subfamily_matches_style_bits(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    let (bold, italic) = if let Ok(os2) = ctx.font.os2() {
        let bits = os2.fs_selection().bits();
        (bits & 0x0020 != 0, bits & 0x0001 != 0)
    } else if let Ok(head) = ctx.font.head() {
        let bits = head.mac_style().bits();
        (bits & 0x0001 != 0, bits & 0x0002 != 0)
    } else {
        sink.skip("the font has neither an OS/2 nor a head table");
        return;
    };
    let Some(sub) = names.win(2) else { return };
    let expected = match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    };
    if sub != expected {
        let msg = format!("The subfamily name is '{sub}', but the bold and italic bits call for '{expected}'.");
        sink.push(Hit::new("mismatch", msg).place(at(ctx, 2, WIN)));
    }
}

/// `check.names.noMacRecords` (FontBakery `no_mac_entries`).
pub(crate) fn no_mac_records(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for r in names.recs.iter().filter(|r| r.platform == 1) {
        sink.push(Hit::new("mac-names", format!("The font has the Macintosh record {}.", r.describe())).place(at_rec(ctx, r)));
    }
}

/// `check.names.macRecordsComplete` (comparefamily's name loader, Mac platform records).
pub(crate) fn mac_records_complete(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (ids, code) in [([1, 2, 4, 6], "missing-mac-record"), ([0, 5, 7, 14], "missing-optional-mac-record")] {
        for id in ids {
            if names.get(id, MAC).is_none() {
                let msg = format!("The Macintosh English record of name ID {id} (platform 1, encoding 0, language 0) is missing.");
                sink.push(Hit::new(code, msg).place(at(ctx, id, MAC)));
            }
        }
    }
}

/// `check.names.copyrightInDescription` (FontBakery `name/no_copyright_on_description`,
/// `googlefonts/name/description_max_length`).
pub(crate) fn copyright_in_description(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    let limit = params.get(&MAX_DESCRIPTION_LENGTH);
    for (r, t) in names.all(10) {
        if t.contains("opyright") {
            let msg = format!("The description {} contains copyright wording; the copyright belongs in name ID 0.", r.describe());
            sink.push(Hit::new("copyright-on-description", msg).place(at_rec(ctx, r)));
        }
        let n = len(t);
        if n as f64 > limit {
            let msg =
                format!("The description {} is {n} characters long. Check that it is not a copyright notice stored there by mistake.", r.describe());
            sink.push(Hit::new("too-long", msg).data("length", n as f64).data("limit", limit).place(at_rec(ctx, r)));
        }
    }
}

/// comparefamily test 6's patent pattern `[ \t,]\d\d\d,*\d\d\d[ \t.,]`.
fn has_patent_number(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let digit = |i: usize| c.get(i).is_some_and(char::is_ascii_digit);
    (0..c.len()).any(|i| {
        if !c.get(i).is_some_and(|ch| matches!(ch, ' ' | '\t' | ',')) || !(1..=3).all(|k| digit(i + k)) {
            return false;
        }
        let mut j = i + 4;
        while c.get(j) == Some(&',') {
            j += 1;
        }
        (0..3).all(|k| digit(j + k)) && c.get(j + 3).is_some_and(|ch| matches!(ch, ' ' | '\t' | '.' | ','))
    })
}

/// `check.names.legalAndCredits` (comparefamily test 6 and its name loader). The comparisons
/// with the house URL and vendor id need project values, which this context does not carry yet.
pub(crate) fn legal_and_credits(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    let mut keys = vec![WIN];
    if names.has_mac() {
        keys.push(MAC);
    }
    for &key in &keys {
        let platform = if key == WIN { "Windows" } else { "Macintosh" };
        for (id, code, what) in [
            (0, "missing-copyright", "copyright"),
            (7, "missing-trademark", "trademark"),
            (9, "missing-designer-note", "designer note"),
            (11, "missing-vendor-url", "foundry URL"),
        ] {
            if names.get(id, key).is_none() {
                sink.push(Hit::new(code, format!("The {platform} {what} record (name ID {id}) is missing.")).place(at(ctx, id, key)));
            }
        }
    }
    if let Some(t) = names.win(0).or_else(|| names.text(0, MAC))
        && has_patent_number(t)
    {
        sink.push(Hit::new("patent-number", format!("The copyright '{t}' contains what looks like a patent number.")).place(at(ctx, 0, WIN)));
    }
}

/// `check.names.familyNameStartsWithDigit` (FontBakery `googlefonts/name/familyname_first_char`).
pub(crate) fn family_name_starts_with_digit(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(names) = Names::read(ctx, sink) else { return };
    for (r, t) in names.all(1) {
        if t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            sink.push(Hit::new("begins-with-digit", format!("The family name '{t}' in {} begins with a digit.", r.describe())).place(at_rec(ctx, r)));
        }
    }
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
