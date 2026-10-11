//! The General fields of Font Info: which properties each scope offers, which of them hold one
//! text per language, and which are built in (always shown, never removed).
//!
//! The lists follow the format's property keys and glyphsLib (`font.info.listFields` records the
//! guess). The plural keys (`designers`, `copyrights`) are the localizable properties; format 2's
//! singular spellings read as the same field. The Masters tab offers no field of its own: a
//! master's properties are kept as read.
//!
//! Rows: `font.info.listFields`, `font.info.addField`, `font.info.removeField`,
//! `font.info.setLocalizedValue`, `font.properties`, `master.properties`, `instance.properties`
//! (M7.3).

use fontcraft_model::{DEFAULT_LANGUAGE, MAX_NAME_LEN, language_id};

use crate::catalog::Scopes;
use crate::error::ValueError;
use crate::target::Scope;

/// One field of the General section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// The property key (`designers`, `vendorID`) or, for a built-in field, the model's field
    /// (`familyName`, `unitsPerEm`, `versionMajor`).
    pub key: &'static str,
    /// The label the picker and the row show (`Designer`, `Vendor ID`).
    pub label: &'static str,
    pub scopes: Scopes,
    /// One text per language.
    pub localizable: bool,
    /// Always shown at the top of its tab and never removed (Family Name, Units per Em…).
    pub built_in: bool,
}

const FONT: Scopes = Scopes::FONT;
const INSTANCE: Scopes = Scopes::INSTANCE;
const SHARED: Scopes = Scopes::FONT.with(Scopes::INSTANCE);

const fn built_in(key: &'static str, label: &'static str, scopes: Scopes) -> FieldSpec {
    FieldSpec { key, label, scopes, localizable: false, built_in: true }
}

const fn single(key: &'static str, label: &'static str, scopes: Scopes) -> FieldSpec {
    FieldSpec { key, label, scopes, localizable: false, built_in: false }
}

const fn localized(key: &'static str, label: &'static str, scopes: Scopes) -> FieldSpec {
    FieldSpec { key, label, scopes, localizable: true, built_in: false }
}

/// A field list: `head` entries, the added fields the Font and Exports tabs share (in picker
/// order), then `tail` entries.
macro_rules! field_list {
    ($($head:expr),* ; $($tail:expr),*) => {
        &[
            $($head,)*
            localized("familyNames", "Family Names", SHARED),
            localized("designers", "Designer", SHARED),
            single("designerURL", "Designer URL", SHARED),
            localized("manufacturers", "Manufacturer", SHARED),
            single("manufacturerURL", "Manufacturer URL", SHARED),
            localized("copyrights", "Copyright", SHARED),
            single("versionString", "Version String", SHARED),
            single("vendorID", "Vendor ID", SHARED),
            single("uniqueID", "Unique ID", SHARED),
            localized("licenses", "License", SHARED),
            single("licenseURL", "License URL", SHARED),
            localized("trademarks", "Trademark", SHARED),
            localized("descriptions", "Description", SHARED),
            localized("sampleTexts", "Sample Text", SHARED),
            single("postscriptFontName", "PostScript Font Name", SHARED),
            localized("postscriptFullNames", "PostScript Full Name", SHARED),
            localized("compatibleFullNames", "Compatible Full Name", SHARED),
            single("WWSFamilyName", "WWS Family Name", SHARED),
            single("WWSSubfamilyName", "WWS Subfamily Name", SHARED),
            single("variationsPostScriptNamePrefix", "Variations PostScript Name Prefix", SHARED),
            $($tail,)*
        ]
    };
}

static FONT_FIELDS: &[FieldSpec] = field_list!(
    built_in("familyName", "Family Name", FONT),
    built_in("unitsPerEm", "Units per Em", FONT),
    built_in("versionMajor", "Version (major)", FONT),
    built_in("versionMinor", "Version (minor)", FONT),
    built_in("date", "Creation Date", FONT);
);

static MASTER_FIELDS: &[FieldSpec] = &[built_in("name", "Master Name", Scopes::MASTER)];

static INSTANCE_FIELDS: &[FieldSpec] = field_list!(
    built_in("name", "Style Name", INSTANCE),
    built_in("weightClass", "Weight Class", INSTANCE),
    built_in("widthClass", "Width Class", INSTANCE);
    localized("styleNames", "Style Names", INSTANCE),
    localized("styleMapFamilyNames", "Style Map Family Names", INSTANCE),
    localized("styleMapStyleNames", "Style Map Style Names", INSTANCE),
    localized("preferredFamilyNames", "Preferred Family Names", INSTANCE),
    localized("preferredSubfamilyNames", "Preferred Subfamily Names", INSTANCE),
    localized("variableStyleNames", "Variable Style Names", INSTANCE)
);

/// The fields `scope` offers, built-in ones first, in the order the picker lists them.
pub fn fields(scope: Scope) -> &'static [FieldSpec] {
    match scope {
        Scope::Font => FONT_FIELDS,
        Scope::Master => MASTER_FIELDS,
        Scope::Instance => INSTANCE_FIELDS,
    }
}

/// The field `key` names at `scope`, also by its singular spelling.
pub fn field(scope: Scope, key: &str) -> Option<&'static FieldSpec> {
    let all = fields(scope);
    all.iter()
        .find(|f| f.key == key)
        .or_else(|| all.iter().find(|f| f.localizable && f.key.strip_suffix('s').is_some_and(|singular| singular == key)))
}

/// Refuses a field text longer than [`crate::MAX_FIELD_BYTES`] (and, for the built-in fields, a
/// text that is not their kind: Units per Em in [`crate::UPM_RANGE`], the version parts within
/// [`crate::MAX_VERSION_MAJOR`] and [`crate::MAX_VERSION_MINOR`]).
pub fn check_text(spec: &FieldSpec, text: &str) -> Result<(), ValueError> {
    if text.len() > crate::MAX_FIELD_BYTES {
        return Err(ValueError::LimitExceeded { what: "bytes in one field", limit: crate::MAX_FIELD_BYTES });
    }
    if !spec.built_in {
        return Ok(());
    }
    let name = || spec.key.to_string();
    let whole = |min: u16, max: u16| -> Result<(), ValueError> {
        match text.trim().parse::<f64>() {
            Ok(n) if n.is_finite() && n.fract() == 0.0 && (f64::from(min)..=f64::from(max)).contains(&n) => Ok(()),
            Ok(n) if n.is_finite() => Err(ValueError::OutOfRange { name: name(), min: f64::from(min), max: f64::from(max) }),
            _ => Err(ValueError::WrongKind { name: name(), expected: "a whole number" }),
        }
    };
    match spec.key {
        "familyName" | "name" => {
            if text.trim().is_empty() {
                return Err(ValueError::Syntax { name: name(), message: "must not be empty".into() });
            }
            if text.len() > MAX_NAME_LEN {
                return Err(ValueError::LimitExceeded { what: "bytes in a name", limit: MAX_NAME_LEN });
            }
            if text.chars().any(char::is_control) {
                return Err(ValueError::Syntax { name: name(), message: "must not hold control characters".into() });
            }
            Ok(())
        }
        "unitsPerEm" => whole(*crate::UPM_RANGE.start(), *crate::UPM_RANGE.end()),
        "versionMajor" => whole(0, crate::MAX_VERSION_MAJOR),
        "versionMinor" => whole(0, crate::MAX_VERSION_MINOR),
        "weightClass" => whole(*crate::WEIGHT_CLASS_RANGE.start(), *crate::WEIGHT_CLASS_RANGE.end()),
        "widthClass" => whole(*crate::WIDTH_CLASS_RANGE.start(), *crate::WIDTH_CLASS_RANGE.end()),
        "date" => match date_seconds(text).and_then(date_text) {
            Some(_) => Ok(()),
            None => Err(ValueError::Syntax {
                name: name(),
                message: "must be a date from 1970 to 9999 such as `2026-10-11 12:00:00 +0000` or `2026-10-11T12:00:00Z`".into(),
            }),
        },
        _ => Ok(()),
    }
}

/// Whether `tag` may hold one language of a localizable field: the default language (`dflt`) or
/// a language-system tag the name table maps to a Windows language id.
pub fn is_language(tag: &str) -> bool {
    tag == DEFAULT_LANGUAGE || language_id(tag).is_some()
}

/// The built-in field whose value a newly added field starts from: `familyNames` from the Family
/// Name, `styleNames` from the Style Name.
pub fn overrides(key: &str) -> Option<&'static str> {
    match key {
        "familyNames" => Some("familyName"),
        "styleNames" => Some("name"),
        _ => None,
    }
}

// ---------- dates ----------

/// Last second of the year 9999.
const MAX_SECONDS: i64 = 253_402_300_799;

/// Seconds since the Unix epoch of a date in the format's form (`2026-10-11 12:00:00 +0000`), a
/// UFO's form (`2026/10/11 12:00:00`) or ISO 8601 (`2026-10-11T12:00:00Z`, `…+02:00`, a date
/// alone). `None` for anything else, or a year outside 1–9999.
pub fn date_seconds(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    let split = t.find(['T', ' ']);
    let (date, rest) = match split {
        Some(i) => (t.get(..i)?, t.get(i + 1..)?.trim_start()),
        None => (t, ""),
    };
    let sep = if date.contains('/') { '/' } else { '-' };
    let mut d = date.split(sep);
    let (y, m, day) = (digits(d.next()?, 4)?, digits(d.next()?, 2)?, digits(d.next()?, 2)?);
    if d.next().is_some() || !(1..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=days_in_month(y, m)).contains(&day) {
        return None;
    }
    let time_end = rest.find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.')).unwrap_or(rest.len());
    let (time, zone) = (rest.get(..time_end)?, rest.get(time_end..)?.trim());
    let (hh, mm, ss) = if time.is_empty() {
        if !zone.is_empty() {
            return None;
        }
        (0, 0, 0)
    } else {
        let mut parts = time.split(':');
        let hh = digits(parts.next()?, 2)?;
        let mm = digits(parts.next()?, 2)?;
        let ss = match parts.next() {
            Some(s) => digits(s.split('.').next()?, 2)?,
            None => 0,
        };
        if parts.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
            return None;
        }
        (hh, mm, ss)
    };
    let offset = match zone {
        "" | "Z" => 0,
        z => {
            let sign = match z.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let body: String = z.get(1..)?.chars().filter(|c| *c != ':').collect();
            if body.len() != 4 {
                return None;
            }
            let (h, m) = (digits(body.get(..2)?, 2)?, digits(body.get(2..)?, 2)?);
            if h > 23 || m > 59 {
                return None;
            }
            sign * (h * 3600 + m * 60)
        }
    };
    Some(days_from_civil(y, m, day) * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

/// A time in the format's form, in UTC: `2026-10-11 12:00:00 +0000`. `None` before 1970 or after
/// 9999.
pub fn date_text(seconds: i64) -> Option<String> {
    if !(0..=MAX_SECONDS).contains(&seconds) {
        return None;
    }
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (y, m, d) = civil_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000", rest / 3600, rest % 3600 / 60, rest % 60))
}

/// A run of at most `max` ASCII digits.
fn digits(s: &str, max: usize) -> Option<i64> {
    if s.is_empty() || s.len() > max || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 of a civil date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Year, month and day of a day count from 1970-01-01 (Howard Hinnant's `civil_from_days`, for
/// days ≥ 0).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_lists_and_singular_spellings() {
        assert!(fields(Scope::Font).iter().all(|f| f.scopes.allows(Scope::Font) && !f.key.is_empty()));
        assert!(fields(Scope::Instance).iter().all(|f| f.scopes.allows(Scope::Instance) && !f.key.is_empty()));
        assert_eq!(field(Scope::Font, "designer").map(|f| f.key), Some("designers"));
        assert_eq!(field(Scope::Font, "familyName").map(|f| f.built_in), Some(true));
        assert_eq!(field(Scope::Instance, "styleName").map(|f| f.key), Some("styleNames"));
        assert!(field(Scope::Font, "styleNames").is_none());
        assert!(field(Scope::Master, "designers").is_none());
        let upm = field(Scope::Font, "unitsPerEm").unwrap();
        assert!(check_text(upm, "1000").is_ok());
        assert!(check_text(upm, "15").is_err() && check_text(upm, "1000.5").is_err() && check_text(upm, "x").is_err());
        assert!(check_text(field(Scope::Font, "familyName").unwrap(), " ").is_err());
    }

    #[test]
    fn dates_read_every_form_and_write_utc() {
        let t = date_seconds("2026-10-11 12:00:00 +0000").unwrap();
        assert_eq!(date_text(t).as_deref(), Some("2026-10-11 12:00:00 +0000"));
        assert_eq!(date_seconds("2026/10/11 12:00:00"), Some(t));
        assert_eq!(date_seconds("2026-10-11T12:00:00Z"), Some(t));
        assert_eq!(date_seconds("2026-10-11T14:00:00+02:00"), Some(t));
        assert_eq!(date_seconds("2026-10-11 10:00:00 -0200"), Some(t));
        assert_eq!(date_seconds("2026-10-11T12:00:00.250Z"), Some(t));
        assert_eq!(date_seconds("2026-10-11"), date_seconds("2026-10-11T00:00:00Z"));
        for bad in ["", "2026-02-30", "2026-13-01T00:00:00Z", "2026-10-11T25:00:00Z", "yesterday", "2026-10-11T12:00:00 CET"] {
            assert_eq!(date_seconds(bad), None, "{bad}");
        }
        assert_eq!(date_text(-1), None);
    }
}
