//! `fontinfo.plist` in the model's terms: which UFO font-info key is which property or custom
//! parameter of the reference's format.
//!
//! The crosswalk is ported from glyphsLib 6.15.0 (`builder/custom_params.py`, `builder/font.py`,
//! `builder/constants.py`; Apache-2.0, see `NOTICE`), so a UFO made from a `.glyphs` document holds
//! what glyphsLib would have written, and a UFO's values reach the reference's names.
//!
//! | UFO | Model |
//! |---|---|
//! | names (`copyright`, `openTypeNameDesigner`…) | properties under the reference's keys ([`NAMES`]) |
//! | `openTypeNameRecords` | the localised values of those properties where a record is one of them; the rest `Name Table Entry` parameters |
//! | values with a reference parameter (`openTypeOS2TypoAscender`…) | that parameter ([`RENAMED`]); `winAscent` and `winDescent` as absolute values |
//! | `openTypeOS2Selection` | `Use Typo Metrics` (bit 7), `Has WWS Names` (bit 8), `openTypeOS2SelectionUnsupportedBits` |
//! | `openTypeOS2CodePageRanges` | `codePageRanges` as code page numbers, `codePageRangesUnsupportedBits` |
//! | `openTypeGaspRangeRecords` | `GASP Table`, a dictionary of maximum size → behaviour flags |
//! | `openTypeHeadCreated` | the font's date |
//! | blue values, stem snaps, `guidelines` | the master's design values ([`crate::design`]) |
//! | anything else | a parameter under its UFO name |
//!
//! The keys in [`ELSEWHERE`] are model fields of their own and are not handled here.
//!
//! **Defaults.** Three values default differently in the reference app and in ufo2ft, which
//! compiles UFOs: `fsType` (bit 3 there, bit 2 here), the underline thickness (50 there, 5% of
//! the em here) and position (-100 there, -7.5% of the em here). The model means the reference's
//! defaults, so reading a UFO without one of these keys gives a parameter with ufo2ft's default,
//! a value equal to the reference's default is no parameter, and writing does the reverse.
//!
//! **Reading is checked by writing.** A key whose model form would not write back to the same
//! value (an unusual order, a malformed value) is kept as a parameter under its UFO name, marked
//! [`RAW`], and written as it is.

use std::collections::BTreeMap;

use fontcraft_model::{DEFAULT_LANGUAGE, Family, Localized, Param, Params, Properties, Property, PropertyValue};
use fontcraft_plist::{Array, Dict, Num, Number, Value};

use crate::design::{self, Design};

/// Keys the model holds in fields of its own, handled in `ufo.rs`. The master's blue zones, stem
/// snaps and guidelines are handled here, through [`crate::design`].
pub(crate) const ELSEWHERE: [&str; 11] =
    ["familyName", "styleName", "unitsPerEm", "versionMajor", "versionMinor", "note", "ascender", "capHeight", "xHeight", "descender", "italicAngle"];

/// The `Extra` key (UFO family) of a parameter written as it is.
pub(crate) const RAW: &str = "raw";

/// How a UFO font-info value is typed, by the UFO specification.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Str,
    Int,
    Num,
    Bool,
    IntList,
    NumList,
    Any,
}

/// Every UFO 3 font-info key with its type.
const UFO_KEYS: &[(&str, Kind)] = &[
    ("familyName", Kind::Str),
    ("styleName", Kind::Str),
    ("styleMapFamilyName", Kind::Str),
    ("styleMapStyleName", Kind::Str),
    ("versionMajor", Kind::Int),
    ("versionMinor", Kind::Int),
    ("year", Kind::Int),
    ("copyright", Kind::Str),
    ("trademark", Kind::Str),
    ("unitsPerEm", Kind::Num),
    ("descender", Kind::Num),
    ("xHeight", Kind::Num),
    ("capHeight", Kind::Num),
    ("ascender", Kind::Num),
    ("italicAngle", Kind::Num),
    ("note", Kind::Str),
    ("guidelines", Kind::Any),
    ("openTypeGaspRangeRecords", Kind::Any),
    ("openTypeHeadCreated", Kind::Str),
    ("openTypeHeadLowestRecPPEM", Kind::Int),
    ("openTypeHeadFlags", Kind::IntList),
    ("openTypeHheaAscender", Kind::Int),
    ("openTypeHheaDescender", Kind::Int),
    ("openTypeHheaLineGap", Kind::Int),
    ("openTypeHheaCaretSlopeRise", Kind::Int),
    ("openTypeHheaCaretSlopeRun", Kind::Int),
    ("openTypeHheaCaretOffset", Kind::Int),
    ("openTypeNameDesigner", Kind::Str),
    ("openTypeNameDesignerURL", Kind::Str),
    ("openTypeNameManufacturer", Kind::Str),
    ("openTypeNameManufacturerURL", Kind::Str),
    ("openTypeNameLicense", Kind::Str),
    ("openTypeNameLicenseURL", Kind::Str),
    ("openTypeNameVersion", Kind::Str),
    ("openTypeNameUniqueID", Kind::Str),
    ("openTypeNameDescription", Kind::Str),
    ("openTypeNamePreferredFamilyName", Kind::Str),
    ("openTypeNamePreferredSubfamilyName", Kind::Str),
    ("openTypeNameCompatibleFullName", Kind::Str),
    ("openTypeNameSampleText", Kind::Str),
    ("openTypeNameWWSFamilyName", Kind::Str),
    ("openTypeNameWWSSubfamilyName", Kind::Str),
    ("openTypeNameRecords", Kind::Any),
    ("openTypeOS2WidthClass", Kind::Int),
    ("openTypeOS2WeightClass", Kind::Int),
    ("openTypeOS2Selection", Kind::IntList),
    ("openTypeOS2VendorID", Kind::Str),
    ("openTypeOS2Panose", Kind::IntList),
    ("openTypeOS2FamilyClass", Kind::IntList),
    ("openTypeOS2UnicodeRanges", Kind::IntList),
    ("openTypeOS2CodePageRanges", Kind::IntList),
    ("openTypeOS2TypoAscender", Kind::Int),
    ("openTypeOS2TypoDescender", Kind::Int),
    ("openTypeOS2TypoLineGap", Kind::Int),
    ("openTypeOS2WinAscent", Kind::Int),
    ("openTypeOS2WinDescent", Kind::Int),
    ("openTypeOS2Type", Kind::IntList),
    ("openTypeOS2SubscriptXSize", Kind::Int),
    ("openTypeOS2SubscriptYSize", Kind::Int),
    ("openTypeOS2SubscriptXOffset", Kind::Int),
    ("openTypeOS2SubscriptYOffset", Kind::Int),
    ("openTypeOS2SuperscriptXSize", Kind::Int),
    ("openTypeOS2SuperscriptYSize", Kind::Int),
    ("openTypeOS2SuperscriptXOffset", Kind::Int),
    ("openTypeOS2SuperscriptYOffset", Kind::Int),
    ("openTypeOS2StrikeoutSize", Kind::Int),
    ("openTypeOS2StrikeoutPosition", Kind::Int),
    ("openTypeVheaVertTypoAscender", Kind::Int),
    ("openTypeVheaVertTypoDescender", Kind::Int),
    ("openTypeVheaVertTypoLineGap", Kind::Int),
    ("openTypeVheaCaretSlopeRise", Kind::Int),
    ("openTypeVheaCaretSlopeRun", Kind::Int),
    ("openTypeVheaCaretOffset", Kind::Int),
    ("postscriptFontName", Kind::Str),
    ("postscriptFullName", Kind::Str),
    ("postscriptSlantAngle", Kind::Num),
    ("postscriptUniqueID", Kind::Int),
    ("postscriptUnderlineThickness", Kind::Num),
    ("postscriptUnderlinePosition", Kind::Num),
    ("postscriptIsFixedPitch", Kind::Bool),
    ("postscriptBlueValues", Kind::NumList),
    ("postscriptOtherBlues", Kind::NumList),
    ("postscriptFamilyBlues", Kind::NumList),
    ("postscriptFamilyOtherBlues", Kind::NumList),
    ("postscriptStemSnapH", Kind::NumList),
    ("postscriptStemSnapV", Kind::NumList),
    ("postscriptBlueFuzz", Kind::Num),
    ("postscriptBlueShift", Kind::Num),
    ("postscriptBlueScale", Kind::Num),
    ("postscriptForceBold", Kind::Bool),
    ("postscriptDefaultWidthX", Kind::Num),
    ("postscriptNominalWidthX", Kind::Num),
    ("postscriptWeightName", Kind::Str),
    ("postscriptDefaultCharacter", Kind::Str),
    ("postscriptWindowsCharacterSet", Kind::Int),
    ("macintoshFONDFamilyID", Kind::Int),
    ("macintoshFONDName", Kind::Str),
    ("woffMajorVersion", Kind::Int),
    ("woffMinorVersion", Kind::Int),
    ("woffMetadataUniqueID", Kind::Any),
    ("woffMetadataVendor", Kind::Any),
    ("woffMetadataCredits", Kind::Any),
    ("woffMetadataDescription", Kind::Any),
    ("woffMetadataLicense", Kind::Any),
    ("woffMetadataCopyright", Kind::Any),
    ("woffMetadataTrademark", Kind::Any),
    ("woffMetadataLicensee", Kind::Any),
    ("woffMetadataExtensions", Kind::Any),
];

fn kind_of(ufo_key: &str) -> Option<Kind> {
    UFO_KEYS.iter().find(|(k, _)| *k == ufo_key).map(|(_, kind)| *kind)
}

/// Names: (UFO key, property key, localised, name id of its localised records).
const NAMES: &[(&str, &str, bool, Option<u16>)] = &[
    ("copyright", "copyrights", true, Some(0)),
    ("trademark", "trademarks", true, Some(7)),
    ("openTypeNameManufacturer", "manufacturers", true, Some(8)),
    ("openTypeNameManufacturerURL", "manufacturerURL", false, None),
    ("openTypeNameDesigner", "designers", true, Some(9)),
    ("openTypeNameDesignerURL", "designerURL", false, None),
    ("openTypeNameDescription", "descriptions", true, Some(10)),
    ("openTypeNameLicense", "licenses", true, Some(13)),
    ("openTypeNameLicenseURL", "licenseURL", false, None),
    ("openTypeNameSampleText", "sampleTexts", true, Some(19)),
    ("openTypeNameVersion", "versionString", false, None),
    ("openTypeOS2VendorID", "vendorID", false, None),
    ("openTypeNameUniqueID", "uniqueID", false, None),
];

/// The family name's localised values live in a property of their own; its default value is the
/// font's family name.
const FAMILY_NAMES: &str = "familyNames";
const FAMILY_NAME_ID: u16 = 1;

/// Properties the reference also accepts for UFO keys that are parameters here: (property key,
/// UFO key). glyphsLib reads the property first.
const PROPERTY_PARAMS: &[(&str, &str)] = &[
    ("postscriptFullName", "postscriptFullName"),
    ("postscriptFontName", "postscriptFontName"),
    ("preferredFamilyNames", "openTypeNamePreferredFamilyName"),
    ("preferredSubfamilyNames", "openTypeNamePreferredSubfamilyName"),
    ("WWSFamilyName", "openTypeNameWWSFamilyName"),
    ("WWSSubfamilyName", "openTypeNameWWSSubfamilyName"),
];

/// Parameters whose reference name differs from the UFO key: (reference name, UFO key). A value
/// is the same on both sides, except `winAscent` and `winDescent`, which are absolute values.
const RENAMED: &[(&str, &str)] = &[
    ("compatibleFullName", "openTypeNameCompatibleFullName"),
    ("preferredFamilyName", "openTypeNamePreferredFamilyName"),
    ("preferredSubfamilyName", "openTypeNamePreferredSubfamilyName"),
    ("WWSFamilyName", "openTypeNameWWSFamilyName"),
    ("WWSSubfamilyName", "openTypeNameWWSSubfamilyName"),
    ("hheaAscender", "openTypeHheaAscender"),
    ("hheaDescender", "openTypeHheaDescender"),
    ("hheaLineGap", "openTypeHheaLineGap"),
    ("panose", "openTypeOS2Panose"),
    ("fsType", "openTypeOS2Type"),
    ("typoAscender", "openTypeOS2TypoAscender"),
    ("typoDescender", "openTypeOS2TypoDescender"),
    ("typoLineGap", "openTypeOS2TypoLineGap"),
    ("unicodeRanges", "openTypeOS2UnicodeRanges"),
    ("strikeoutSize", "openTypeOS2StrikeoutSize"),
    ("strikeoutPosition", "openTypeOS2StrikeoutPosition"),
    ("subscriptXSize", "openTypeOS2SubscriptXSize"),
    ("subscriptYSize", "openTypeOS2SubscriptYSize"),
    ("subscriptXOffset", "openTypeOS2SubscriptXOffset"),
    ("subscriptYOffset", "openTypeOS2SubscriptYOffset"),
    ("superscriptXSize", "openTypeOS2SuperscriptXSize"),
    ("superscriptYSize", "openTypeOS2SuperscriptYSize"),
    ("superscriptXOffset", "openTypeOS2SuperscriptXOffset"),
    ("superscriptYOffset", "openTypeOS2SuperscriptYOffset"),
    ("vheaVertAscender", "openTypeVheaVertTypoAscender"),
    ("vheaVertDescender", "openTypeVheaVertTypoDescender"),
    ("vheaVertLineGap", "openTypeVheaVertTypoLineGap"),
    ("blueScale", "postscriptBlueScale"),
    ("blueShift", "postscriptBlueShift"),
    ("isFixedPitch", "postscriptIsFixedPitch"),
    ("underlinePosition", "postscriptUnderlinePosition"),
    ("underlineThickness", "postscriptUnderlineThickness"),
    ("winAscent", "openTypeOS2WinAscent"),
    ("winDescent", "openTypeOS2WinDescent"),
    ("weightClass", "openTypeOS2WeightClass"),
    ("widthClass", "openTypeOS2WidthClass"),
    // glyphsLib's other spelling of the gasp parameter's name is handled with it.
    ("GASP Table", "openTypeGaspRangeRecords"),
];

/// Further reference names for the same UFO keys, accepted when writing. The last ones are the
/// parameters version 2 files hold names in (glyphsLib's `GLYPHS_UFO_CUSTOM_PARAMS_GLYPHS3_PROPERTIES`);
/// their keys read back as properties ([`NAMES`]), from [`PROPERTY_ALIASES`] on.
const ALIASES: &[(&str, &str)] = &[
    ("vheaVertTypoAscender", "openTypeVheaVertTypoAscender"),
    ("vheaVertTypoDescender", "openTypeVheaVertTypoDescender"),
    ("vheaVertTypoLineGap", "openTypeVheaVertTypoLineGap"),
    ("gasp Table", "openTypeGaspRangeRecords"),
    ("versionString", "openTypeNameVersion"),
    ("vendorID", "openTypeOS2VendorID"),
    ("uniqueID", "openTypeNameUniqueID"),
    ("license", "openTypeNameLicense"),
    ("licenseURL", "openTypeNameLicenseURL"),
    ("trademark", "trademark"),
    ("description", "openTypeNameDescription"),
    ("sampleText", "openTypeNameSampleText"),
];
/// Where in [`ALIASES`] the parameters that come back as properties start.
const PROPERTY_ALIASES: usize = 4;

const SELECTION: &str = "openTypeOS2Selection";
const TYPO_METRICS: &str = "Use Typo Metrics";
const WWS_NAMES: &str = "Has WWS Names";
const SELECTION_REST: &str = "openTypeOS2SelectionUnsupportedBits";
const CODE_PAGES: &str = "openTypeOS2CodePageRanges";
const CODE_PAGE_PARAM: &str = "codePageRanges";
const CODE_PAGE_REST: &str = "codePageRangesUnsupportedBits";
const GASP: &str = "openTypeGaspRangeRecords";
const GASP_PARAM: &str = "GASP Table";
const RECORDS: &str = "openTypeNameRecords";
const NAME_ENTRY: &str = "Name Table Entry";
const CREATED: &str = "openTypeHeadCreated";

/// Windows code pages and their `ulCodePageRange` bits (from glyphsLib's `CODEPAGE_RANGES`).
const CODE_PAGE_BITS: &[(i64, i64)] = &[
    (1252, 0),
    (1250, 1),
    (1251, 2),
    (1253, 3),
    (1254, 4),
    (1255, 5),
    (1256, 6),
    (1257, 7),
    (1258, 8),
    (874, 16),
    (932, 17),
    (936, 18),
    (949, 19),
    (950, 20),
    (1361, 21),
    (869, 48),
    (866, 49),
    (865, 50),
    (864, 51),
    (863, 52),
    (862, 53),
    (861, 54),
    (860, 55),
    (857, 56),
    (855, 57),
    (852, 58),
    (775, 59),
    (737, 60),
    (708, 61),
    (850, 62),
    (437, 63),
];

/// OpenType language-system tags and Windows language ids (glyphsLib's `LANGUAGE_MAPPING`).
const LANGUAGES: &[(&str, u16)] = &[
    ("AFK", 0x0436),
    ("ARA", 0x0C01),
    ("ASM", 0x044D),
    ("AZE", 0x042C),
    ("BEL", 0x0423),
    ("BEN", 0x0845),
    ("BGR", 0x0402),
    ("BRE", 0x047E),
    ("CAT", 0x0403),
    ("CSY", 0x0405),
    ("DAN", 0x0406),
    ("DEU", 0x0407),
    ("ELL", 0x0408),
    ("ENG", 0x0409),
    ("ESP", 0x0C0A),
    ("ETI", 0x0425),
    ("EUQ", 0x042D),
    ("FIN", 0x040B),
    ("FLE", 0x0813),
    ("FOS", 0x0438),
    ("FRA", 0x040C),
    ("FRI", 0x0462),
    ("GRN", 0x046F),
    ("GUJ", 0x0447),
    ("HAU", 0x0468),
    ("HIN", 0x0439),
    ("HRV", 0x041A),
    ("HUN", 0x040E),
    ("HVE", 0x042B),
    ("IRI", 0x083C),
    ("ISL", 0x040F),
    ("ITA", 0x0410),
    ("IWR", 0x040D),
    ("JPN", 0x0411),
    ("KAN", 0x044B),
    ("KAT", 0x0437),
    ("KAZ", 0x043F),
    ("KHM", 0x0453),
    ("KOK", 0x0457),
    ("KOR", 0x0412),
    ("LAO", 0x0454),
    ("LSB", 0x082E),
    ("LTH", 0x0427),
    ("LVI", 0x0426),
    ("MAR", 0x044E),
    ("MKD", 0x042F),
    ("MLR", 0x044C),
    ("MLY", 0x043E),
    ("MOL", 0x0818),
    ("MNG", 0x0352),
    ("MTS", 0x043A),
    ("NEP", 0x0461),
    ("NLD", 0x0413),
    ("NOB", 0x0414),
    ("ORI", 0x0448),
    ("PAN", 0x0446),
    ("PAS", 0x0463),
    ("PLK", 0x0415),
    ("PTG", 0x0816),
    ("PTG-BR", 0x0416),
    ("RMS", 0x0417),
    ("ROM", 0x0418),
    ("RUS", 0x0419),
    ("SAN", 0x044F),
    ("SKY", 0x041B),
    ("SLV", 0x0424),
    ("SQI", 0x041C),
    ("SRB", 0x081A),
    ("SVE", 0x041D),
    ("TAM", 0x0449),
    ("TAT", 0x0444),
    ("TEL", 0x044A),
    ("THA", 0x041E),
    ("TIB", 0x0451),
    ("TRK", 0x041F),
    ("UKR", 0x0422),
    ("URD", 0x0420),
    ("USB", 0x042E),
    ("UYG", 0x0480),
    ("UZB", 0x0443),
    ("VIT", 0x042A),
    ("WEL", 0x0452),
    ("ZHH", 0x0C04),
    ("ZHS", 0x0804),
    ("ZHT", 0x0404),
];

/// Languages whose value is the property's default rather than a name record.
const DEFAULT_LANGUAGES: [&str; 3] = [DEFAULT_LANGUAGE, "default", "ENG"];

pub(crate) fn language_id(tag: &str) -> Option<u16> {
    LANGUAGES.iter().find(|(t, _)| *t == tag).map(|(_, id)| *id)
}

/// The language-system tag of a Windows language id, `ENG` included.
pub(crate) fn language_tag_any(id: u16) -> Option<&'static str> {
    LANGUAGES.iter().find(|(_, i)| *i == id).map(|(t, _)| *t)
}

fn language_tag(id: u16) -> Option<&'static str> {
    LANGUAGES.iter().find(|(t, i)| *i == id && !DEFAULT_LANGUAGES.contains(t)).map(|(t, _)| *t)
}

/// The values the reference and ufo2ft give a key the file does not have.
fn defaults(ufo_key: &str, upm: f64) -> Option<(Value, Value)> {
    Some(match ufo_key {
        "openTypeOS2Type" => (int_list(&[3]), int_list(&[2])),
        "postscriptUnderlineThickness" => (int(50), number(upm * 0.05)),
        "postscriptUnderlinePosition" => (int(-100), number(upm * -0.075)),
        _ => return None,
    })
}
const DEFAULTED: [&str; 3] = ["openTypeOS2Type", "postscriptUnderlineThickness", "postscriptUnderlinePosition"];

fn int(i: i64) -> Value {
    Value::Number(Number::int(i))
}

/// A number as fontTools would store it: an integer when whole.
pub(crate) fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        int(v as i64)
    } else {
        Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
    }
}

fn int_list(v: &[i64]) -> Value {
    Value::Array(Array { items: v.iter().map(|i| int(*i)).collect(), ..Default::default() })
}

pub(crate) fn string(s: &str) -> Value {
    Value::String(s.into())
}

fn whole(n: Num) -> Option<i64> {
    match n {
        Num::Int(i) => Some(i),
        // A whole number written as a real.
        #[allow(clippy::cast_possible_truncation)]
        Num::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => Some(f as i64),
        Num::Float(_) => None,
    }
}

fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => whole(n.value()),
        Value::String(s) => s.as_str().trim().parse().ok(),
        _ => None,
    }
}

fn ints_of(v: &Value) -> Option<Vec<i64>> {
    v.as_array()?.items.iter().map(int_of).collect()
}

/// A value of the reference's format as the UFO specification types `kind`, or `None` when it
/// cannot be one. Whole numbers become integers; nothing changes its value.
fn coerce(kind: Kind, v: &Value) -> Option<Value> {
    let num = |v: &Value| match v {
        Value::Number(n) => Some(number(n.value().as_f64())),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(number),
        _ => None,
    };
    match kind {
        Kind::Str => v.as_str().map(string),
        Kind::Int | Kind::Num => num(v),
        Kind::Bool => match v {
            Value::Bool(b) => Some(Value::Bool(*b)),
            Value::Number(n) => Some(Value::Bool(n.value().as_f64() != 0.0)),
            Value::String(s) => match s.as_str() {
                "1" | "true" | "YES" => Some(Value::Bool(true)),
                "0" | "false" | "NO" => Some(Value::Bool(false)),
                _ => None,
            },
            _ => None,
        },
        Kind::IntList | Kind::NumList => {
            let items = v.as_array()?.items.iter().map(num).collect::<Option<Vec<_>>>()?;
            Some(Value::Array(Array { items, ..Default::default() }))
        }
        Kind::Any => Some(v.without_spellings()),
    }
}

/// A UFO value as the model holds it: without its XML spellings, booleans as 1 and 0 (as the
/// reference's format writes them).
fn model_value(kind: Kind, v: &Value) -> Value {
    match (kind, v) {
        (Kind::Bool, Value::Bool(b)) => int(i64::from(*b)),
        _ => v.without_spellings(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.value().as_f64() != 0.0,
        Value::String(s) => !s.as_str().is_empty() && s.as_str() != "0",
        _ => false,
    }
}

/// The font information of one `fontinfo.plist`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Info {
    pub properties: Properties,
    pub params: Params,
    pub date: Option<String>,
    /// The master's own design values; never the font's.
    pub design: Design,
}

/// One model item made from one UFO key.
enum Item {
    Property(Property),
    /// A localised value of a property.
    Localized {
        key: String,
        language: String,
        value: String,
    },
    Param(Param),
    Date(String),
}

pub(crate) fn raw_param(key: &str, v: &Value) -> Param {
    let mut p = Param::new(key, v.without_spellings());
    p.extra.push(Family::Ufo, RAW, int(1));
    p
}

pub(crate) fn is_raw(p: &Param) -> bool {
    p.extra.get(Family::Ufo, RAW).is_some()
}

/// How `openTypeNameRecords` is read: records folded into properties where they can be, as `Name
/// Table Entry` parameters, or as written.
#[derive(Clone, Copy, PartialEq)]
enum Records {
    Fold,
    Entries,
}

/// Reads one `fontinfo.plist` (the keys not in [`ELSEWHERE`]). `upm` is the font's units per em.
pub(crate) fn read(info: &Dict, upm: f64) -> Info {
    let mut raw: Vec<String> = Vec::new();
    let mut records = Records::Fold;
    // Each pass turns at least one more key raw, so this ends; the bound is a backstop.
    for _ in 0..=info.len() + 2 {
        let mut built = assemble(derive(info, upm, &raw, records));
        built.design = design::read(info, &raw);
        let (written, _) = write(&built, upm, Some(info));
        let bad = mismatches(info, &written);
        if bad.is_empty() {
            return built;
        }
        for key in bad {
            if key == RECORDS && records == Records::Fold {
                records = Records::Entries;
                continue;
            }
            // One list of zones makes both blue keys, so they are kept as written together.
            let keys = if key == design::BLUES || key == design::OTHER_BLUES { vec![design::BLUES, design::OTHER_BLUES] } else { vec![key.as_str()] };
            for k in keys {
                if !raw.iter().any(|r| r == k) {
                    raw.push(k.to_string());
                }
            }
        }
    }
    // Every key written as it is.
    let params = info.entries.iter().filter(|(k, _)| !ELSEWHERE.contains(&k.as_str())).map(|(k, v)| raw_param(k.as_str(), v)).collect();
    Info { params: Params(params), ..Default::default() }
}

/// The handled keys whose written value differs from the file's.
pub(crate) fn mismatches(info: &Dict, written: &BTreeMap<String, Value>) -> Vec<String> {
    let mut bad: Vec<String> = Vec::new();
    for (k, v) in &info.entries {
        let k = k.as_str();
        if !ELSEWHERE.contains(&k) && written.get(k) != Some(v) {
            bad.push(k.to_string());
        }
    }
    for k in written.keys() {
        if info.get(k).is_none() && !bad.contains(k) {
            bad.push(k.clone());
        }
    }
    bad
}

fn derive(info: &Dict, upm: f64, raw: &[String], records: Records) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for (key, v) in &info.entries {
        let key = key.as_str();
        if ELSEWHERE.contains(&key) {
            continue;
        }
        if raw.iter().any(|r| r == key) {
            items.push(Item::Param(raw_param(key, v)));
            continue;
        }
        if design::KEYS.contains(&key) {
            continue;
        }
        let kind = kind_of(key).unwrap_or(Kind::Any);
        if let Some((_, property, localized, _)) = NAMES.iter().find(|(k, ..)| *k == key)
            && let Some(s) = v.as_str()
        {
            let value = if *localized {
                PropertyValue::Localized(vec![Localized { language: DEFAULT_LANGUAGE.into(), value: s.into(), extra: Default::default() }])
            } else {
                PropertyValue::Single(s.into())
            };
            items.push(Item::Property(Property { key: (*property).into(), value, extra: Default::default() }));
            continue;
        }
        match key {
            SELECTION => {
                let Some(bits) = ints_of(v) else {
                    items.push(Item::Param(raw_param(key, v)));
                    continue;
                };
                for (bit, name) in [(7, TYPO_METRICS), (8, WWS_NAMES)] {
                    if bits.contains(&bit) {
                        items.push(Item::Param(Param::new(name, int(1))));
                    }
                }
                let rest: Vec<i64> = bits.iter().copied().filter(|b| *b != 7 && *b != 8).collect();
                // An empty list stays a list.
                if !rest.is_empty() || bits.is_empty() {
                    items.push(Item::Param(Param::new(SELECTION_REST, int_list(&rest))));
                }
            }
            CODE_PAGES => {
                let Some(bits) = ints_of(v) else {
                    items.push(Item::Param(raw_param(key, v)));
                    continue;
                };
                let mut pages = Vec::new();
                let mut rest = Vec::new();
                for b in bits {
                    match CODE_PAGE_BITS.iter().find(|(_, bit)| *bit == b) {
                        Some((page, _)) => pages.push(*page),
                        None => rest.push(b),
                    }
                }
                items.push(Item::Param(Param::new(CODE_PAGE_PARAM, int_list(&pages))));
                if !rest.is_empty() {
                    items.push(Item::Param(Param::new(CODE_PAGE_REST, int_list(&rest))));
                }
            }
            GASP => match gasp_to_model(v) {
                Some(d) => items.push(Item::Param(Param::new(GASP_PARAM, d))),
                None => items.push(Item::Param(raw_param(key, v))),
            },
            RECORDS => items.extend(records_to_model(info, v, records)),
            CREATED => match v.as_str().and_then(ufo_date_to_model) {
                Some(d) => items.push(Item::Date(d)),
                None => items.push(Item::Param(raw_param(key, v))),
            },
            _ => {
                let name = RENAMED.iter().find(|(_, u)| *u == key).map_or(key, |(g, _)| *g);
                let mut value = model_value(kind, v);
                if matches!(key, "openTypeOS2WinAscent" | "openTypeOS2WinDescent")
                    && let Some(n) = value.as_num()
                {
                    value = number(n.as_f64().abs());
                }
                // The reference's default is no parameter.
                if defaults(key, upm).is_some_and(|(reference, _)| reference == value) {
                    continue;
                }
                items.push(Item::Param(Param::new(name, value)));
            }
        }
    }
    // A key the file does not have means ufo2ft's default.
    for key in DEFAULTED {
        if info.get(key).is_none()
            && let Some((reference, ufo2ft)) = defaults(key, upm)
            && reference != ufo2ft
        {
            let name = RENAMED.iter().find(|(_, u)| *u == key).map_or(key, |(g, _)| *g);
            items.push(Item::Param(Param::new(name, ufo2ft)));
        }
    }
    items
}

fn assemble(items: Vec<Item>) -> Info {
    let mut info = Info::default();
    for item in items {
        match item {
            Item::Property(p) => info.properties.0.push(p),
            Item::Localized { key, language, value } => {
                let l = Localized { language, value, extra: Default::default() };
                match info.properties.0.iter_mut().find(|p| p.key == key) {
                    Some(p) => match &mut p.value {
                        PropertyValue::Localized(v) => v.push(l),
                        PropertyValue::Single(s) => {
                            let first = Localized { language: DEFAULT_LANGUAGE.into(), value: std::mem::take(s), extra: Default::default() };
                            p.value = PropertyValue::Localized(vec![first, l]);
                        }
                    },
                    None => info.properties.0.push(Property { key, value: PropertyValue::Localized(vec![l]), extra: Default::default() }),
                }
            }
            Item::Param(p) => info.params.0.push(p),
            Item::Date(d) => info.date = Some(d),
        }
    }
    info
}

/// `openTypeGaspRangeRecords` as the reference's `GASP Table`: maximum size (as text) → the
/// behaviour bits as one number, in record order.
fn gasp_to_model(v: &Value) -> Option<Value> {
    let mut d = Dict::new();
    for r in &v.as_array()?.items {
        let r = r.as_dict()?;
        let ppem = int_of(r.get("rangeMaxPPEM")?)?;
        let mut flags: i64 = 0;
        for bit in ints_of(r.get("rangeGaspBehavior")?)? {
            if !(0..16).contains(&bit) {
                return None;
            }
            flags |= 1 << bit;
        }
        d.insert(ppem.to_string(), int(flags));
    }
    Some(Value::Dict(d))
}

fn gasp_to_ufo(v: &Value) -> Option<Value> {
    let mut ranges: Vec<(i64, i64)> = Vec::new();
    for (k, flags) in &v.as_dict()?.entries {
        ranges.push((k.as_str().trim().parse().ok()?, int_of(flags)?));
    }
    ranges.sort_by_key(|(ppem, _)| *ppem);
    let items = ranges
        .into_iter()
        .map(|(ppem, flags)| {
            let bits: Vec<i64> = (0..16).filter(|b| flags & (1 << b) != 0).collect();
            let mut r = Dict::new();
            r.insert("rangeGaspBehavior", int_list(&bits));
            r.insert("rangeMaxPPEM", int(ppem));
            Value::Dict(r)
        })
        .collect();
    Some(Value::Array(Array { items, ..Default::default() }))
}

struct Record {
    name_id: i64,
    platform: i64,
    encoding: i64,
    language: i64,
    string: String,
}

fn record_of(v: &Value) -> Option<Record> {
    let d = v.as_dict()?;
    Some(Record {
        name_id: int_of(d.get("nameID")?)?,
        platform: int_of(d.get("platformID")?)?,
        encoding: int_of(d.get("encodingID")?)?,
        language: int_of(d.get("languageID")?)?,
        string: d.get("string")?.as_str()?.to_string(),
    })
}

fn record_value(r: &Record) -> Value {
    let mut d = Dict::new();
    d.insert("encodingID", int(r.encoding));
    d.insert("languageID", int(r.language));
    d.insert("nameID", int(r.name_id));
    d.insert("platformID", int(r.platform));
    d.insert("string", string(&r.string));
    Value::Dict(d)
}

/// A `Name Table Entry` value: `"nameID platformID encodingID languageID; string"`.
fn entry_text(r: &Record) -> String {
    format!("{} {} {} {}; {}", r.name_id, r.platform, r.encoding, r.language, r.string)
}

/// Reads a `Name Table Entry` as glyphsLib does: ids separated by single spaces, `0x` hexadecimal
/// or a leading `0` octal; the platform defaults to 3 (encoding 1, language 0x409), platform 1 to
/// encoding 0 and language 0.
fn entry_record(text: &str) -> Option<Record> {
    let (ids, s) = text.split_once(';')?;
    let parse = |t: &str| -> Option<i64> {
        if let Some(h) = t.strip_prefix("0x") {
            i64::from_str_radix(h, 16).ok()
        } else if t.len() > 1 && t.starts_with('0') {
            i64::from_str_radix(t, 8).ok()
        } else {
            t.parse().ok()
        }
    };
    let ids: Vec<i64> = ids.split(' ').map(parse).collect::<Option<_>>()?;
    let name_id = *ids.first()?;
    let platform = ids.get(1).copied().unwrap_or(3);
    let encoding = ids.get(2).copied().unwrap_or(if platform == 1 { 0 } else { 1 });
    let language = ids.get(3).copied().unwrap_or(if platform == 1 { 0 } else { 0x409 });
    Some(Record { name_id, platform, encoding, language, string: s.trim().to_string() })
}

fn records_to_model(info: &Dict, v: &Value, mode: Records) -> Vec<Item> {
    let Some(list) = v.as_array().and_then(|a| a.items.iter().map(record_of).collect::<Option<Vec<_>>>()) else {
        return vec![Item::Param(raw_param(RECORDS, v))];
    };
    let mut items = Vec::new();
    for r in list {
        let folded = (mode == Records::Fold && r.platform == 3 && r.encoding == 1)
            .then(|| u16::try_from(r.language).ok().and_then(language_tag))
            .flatten()
            .and_then(|language| {
                let key = if r.name_id == i64::from(FAMILY_NAME_ID) {
                    FAMILY_NAMES
                } else {
                    // Only beside the default value the file has.
                    NAMES.iter().find(|(ufo, _, _, id)| id.map(i64::from) == Some(r.name_id) && info.get(ufo).is_some()).map(|(_, p, ..)| *p)?
                };
                Some(Item::Localized { key: key.into(), language: language.into(), value: r.string.clone() })
            });
        items.push(folded.unwrap_or_else(|| Item::Param(Param::new(NAME_ENTRY, string(&entry_text(&r))))));
    }
    items
}

/// Whether [`write`] writes the parameter `name` (or reports it lost) rather than leaving it to
/// the lib: it has a `fontinfo.plist` key, is one of the parameters folded into one, or is a key
/// `mine` (the file as read) has.
pub(crate) fn holds(name: &str, mine: Option<&Dict>) -> bool {
    [NAME_ENTRY, TYPO_METRICS, WWS_NAMES, SELECTION_REST, CODE_PAGE_PARAM, CODE_PAGES, CODE_PAGE_REST].contains(&name)
        || RENAMED.iter().chain(ALIASES.iter()).any(|(g, _)| *g == name)
        || kind_of(name).is_some()
        || mine.is_some_and(|m| m.get(name).is_some())
}

/// The UFO key a parameter of the model is written to, when it has one of its own.
fn ufo_key_of(name: &str) -> &str {
    RENAMED.iter().chain(ALIASES.iter()).find(|(g, _)| *g == name).map_or(name, |(_, u)| *u)
}

/// A master's font information: the font's, with the master's own properties and parameters in
/// place of the font's of the same key or name.
pub(crate) fn effective(font: &Info, master: &Info) -> Info {
    let mut properties = font.properties.clone();
    for p in &master.properties.0 {
        match properties.0.iter_mut().find(|q| q.key == p.key) {
            Some(q) => *q = p.clone(),
            None => properties.0.push(p.clone()),
        }
    }
    // A name the master has replaces every font parameter of that name (or of the same UFO key,
    // for a raw one).
    let taken: Vec<&str> = master.params.0.iter().map(|p| if is_raw(p) { p.name.as_str() } else { ufo_key_of(&p.name) }).collect();
    let mut params: Vec<Param> = font
        .params
        .0
        .iter()
        .filter(|p| !master.params.0.iter().any(|m| m.name == p.name) && !taken.contains(&ufo_key_of(&p.name)))
        .cloned()
        .collect();
    params.extend(master.params.0.iter().cloned());
    let date = if master.params.0.iter().any(|p| p.name == CREATED) { None } else { font.date.clone() };
    Info { properties, params: Params(params), date, design: master.design.clone() }
}

/// What a master's own font information has that the font's does not: the properties and the
/// parameter names whose values differ. A different date becomes a raw parameter.
pub(crate) fn split(font: &Info, mine: Info) -> Info {
    let properties = mine.properties.0.into_iter().filter(|p| font.properties.get(&p.key) != Some(p)).collect();
    let values = |params: &[Param], name: &str| -> Vec<Param> { params.iter().filter(|p| p.name == name).cloned().collect() };
    let differs: Vec<String> =
        mine.params.0.iter().filter(|p| values(&font.params.0, &p.name) != values(&mine.params.0, &p.name)).map(|p| p.name.clone()).collect();
    let mut params: Vec<Param> = mine.params.0.into_iter().filter(|p| differs.contains(&p.name)).collect();
    if let Some(d) = mine.date.filter(|d| font.date.as_ref() != Some(d))
        && let Some(t) = model_date_to_ufo(&d)
    {
        params.push(raw_param(CREATED, &string(&t)));
    }
    Info { properties: Properties(properties), params: Params(params), date: None, design: mine.design }
}

/// What the writer produced, and what it had no place for.
pub(crate) type Written = (BTreeMap<String, Value>, Vec<String>);

/// The UFO keys the model's font information gives. `mine` is the file as read, if any: a
/// defaulted key it does not have is left out while the model holds ufo2ft's default.
pub(crate) fn write(info: &Info, upm: f64, mine: Option<&Dict>) -> Written {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    let mut lost: Vec<String> = Vec::new();
    let mut raw_keys: Vec<String> = Vec::new();
    // Raw values first: they stand for exactly what the file had.
    for p in info.params.0.iter().filter(|p| is_raw(p)) {
        raw_keys.push(p.name.clone());
        set(&mut out, &mut lost, &p.name, p.value.without_spellings(), &format!("parameter `{}`", p.name));
    }
    let mut records: Vec<Record> = Vec::new();
    for p in &info.properties.0 {
        let what = format!("property `{}`", p.key);
        let (ufo, name_id) = match NAMES.iter().find(|(_, k, ..)| *k == p.key) {
            Some((ufo, _, _, id)) => (Some(*ufo), *id),
            None if p.key == FAMILY_NAMES => (None, Some(FAMILY_NAME_ID)),
            None => match PROPERTY_PARAMS.iter().find(|(k, _)| *k == p.key) {
                Some((_, ufo)) => (Some(*ufo), None),
                None => {
                    lost.push(what);
                    continue;
                }
            },
        };
        let values: Vec<&Localized> = match &p.value {
            PropertyValue::Single(s) => {
                if let Some(ufo) = ufo {
                    set(&mut out, &mut lost, ufo, string(s), &what);
                }
                continue;
            }
            PropertyValue::Localized(v) => v.iter().collect(),
        };
        // The default value: the first of `dflt`, `default`, `ENG`, else the first value.
        let default = DEFAULT_LANGUAGES.iter().find_map(|t| values.iter().find(|l| l.language == *t)).or_else(|| values.first());
        if let (Some(ufo), Some(d)) = (ufo, default) {
            set(&mut out, &mut lost, ufo, string(&d.value), &what);
        }
        for l in values.iter().filter(|l| !DEFAULT_LANGUAGES.contains(&l.language.as_str())) {
            match (name_id, language_id(&l.language)) {
                (Some(id), Some(lang)) => {
                    records.push(Record { name_id: i64::from(id), platform: 3, encoding: 1, language: i64::from(lang), string: l.value.clone() })
                }
                _ => lost.push(format!("{what}, language `{}`", l.language)),
            }
        }
    }
    // Parameters. A name read once: the first enabled parameter of a name counts.
    let mut seen: Vec<&str> = Vec::new();
    let (mut selection, mut selection_rest): (Vec<i64>, Option<Vec<i64>>) = (Vec::new(), None);
    let mut selection_set = false;
    let (mut code_pages, mut code_page_rest): (Option<Vec<i64>>, Vec<i64>) = (None, Vec::new());
    for p in info.params.0.iter().filter(|p| !is_raw(p)) {
        let what = format!("parameter `{}`", p.name);
        if p.disabled {
            lost.push(format!("{what} (switched off)"));
            continue;
        }
        // The lib holds it ([`crate::libkeys`]).
        if crate::libkeys::carried(p, mine) {
            continue;
        }
        let name = p.name.as_str();
        if name == NAME_ENTRY {
            match p.value.as_str().and_then(entry_record) {
                Some(r) => records.push(r),
                None => lost.push(format!("{what}: not `ids; text`")),
            }
            continue;
        }
        if seen.contains(&name) {
            lost.push(format!("{what} (repeated)"));
            continue;
        }
        seen.push(name);
        match name {
            TYPO_METRICS | WWS_NAMES => {
                if truthy(&p.value) {
                    selection_set = true;
                    selection.push(if name == TYPO_METRICS { 7 } else { 8 });
                }
            }
            SELECTION_REST => match ints_of(&p.value) {
                Some(bits) => {
                    selection_set = true;
                    selection_rest = Some(bits);
                }
                None => lost.push(what),
            },
            CODE_PAGE_PARAM | CODE_PAGES => {
                let Some(items) = p.value.as_array() else {
                    lost.push(what);
                    continue;
                };
                let mut bits = Vec::new();
                for item in &items.items {
                    let bit = match item.as_str().and_then(|s| s.strip_prefix("bit ")) {
                        Some(b) => b.trim().parse().ok(),
                        None => int_of(item).and_then(|page| CODE_PAGE_BITS.iter().find(|(p, _)| *p == page).map(|(_, b)| *b)),
                    };
                    match bit {
                        Some(b) => bits.push(b),
                        None => lost.push(format!("{what}: code page {}", brief(item))),
                    }
                }
                if code_pages.is_none() {
                    code_pages = Some(bits);
                }
            }
            CODE_PAGE_REST => match ints_of(&p.value) {
                Some(bits) => code_page_rest = bits,
                None => lost.push(what),
            },
            _ => {
                let ufo = RENAMED.iter().chain(ALIASES.iter()).find(|(g, _)| *g == name).map(|(_, u)| *u);
                let ufo = match ufo {
                    Some(u) => u,
                    None if kind_of(name).is_some() || mine.is_some_and(|m| m.get(name).is_some()) => name,
                    None => {
                        lost.push(what);
                        continue;
                    }
                };
                // The master's design values are written from its fields alone.
                if ELSEWHERE.contains(&ufo) || design::KEYS.contains(&ufo) {
                    lost.push(what);
                    continue;
                }
                let value = if ufo == GASP {
                    gasp_to_ufo(&p.value)
                } else {
                    coerce(kind_of(ufo).unwrap_or(Kind::Any), &p.value).map(|v| match (ufo, v.as_num()) {
                        ("openTypeOS2WinAscent" | "openTypeOS2WinDescent", Some(n)) => number(n.as_f64().abs()),
                        _ => v,
                    })
                };
                match value {
                    Some(v) => set(&mut out, &mut lost, ufo, v, &what),
                    None => lost.push(format!("{what}: not a value `{ufo}` can hold")),
                }
                if let Some((_, property, ..)) = NAMES.iter().find(|(u, ..)| *u == ufo)
                    && ALIASES.iter().skip(PROPERTY_ALIASES).any(|(g, _)| *g == name)
                {
                    lost.push(format!("{what}, as the `{property}` property"));
                }
            }
        }
    }
    if selection_set {
        selection.extend(selection_rest.unwrap_or_default());
        selection.sort_unstable();
        selection.dedup();
        set(&mut out, &mut lost, SELECTION, int_list(&selection), "parameter `Use Typo Metrics`");
    }
    if code_pages.is_some() || !code_page_rest.is_empty() {
        let mut bits = code_pages.unwrap_or_default();
        bits.extend(code_page_rest);
        bits.sort_unstable();
        set(&mut out, &mut lost, CODE_PAGES, int_list(&bits), "parameter `codePageRanges`");
    }
    if !records.is_empty() {
        let items = records.iter().map(record_value).collect();
        set(&mut out, &mut lost, RECORDS, Value::Array(Array { items, ..Default::default() }), "localised names");
    }
    let mut typed = Vec::new();
    design::write(&info.design, &raw_keys, mine, &mut typed, &mut lost);
    for (k, v) in typed {
        set(&mut out, &mut lost, &k, v, &format!("`{k}`"));
    }
    if let Some(d) = &info.date {
        match model_date_to_ufo(d) {
            Some(t) => set(&mut out, &mut lost, CREATED, string(&t), "the date"),
            None => lost.push(format!("the date `{d}`: not a date")),
        }
    }
    for key in DEFAULTED {
        if raw_keys.iter().any(|k| k == key) {
            continue;
        }
        let Some((reference, ufo2ft)) = defaults(key, upm) else { continue };
        let value = out.remove(key).unwrap_or(reference);
        let file_has = mine.is_some_and(|m| m.get(key).is_some());
        if file_has || value != ufo2ft {
            out.insert(key.to_string(), value);
        }
    }
    (out, lost)
}

fn set(out: &mut BTreeMap<String, Value>, lost: &mut Vec<String>, key: &str, v: Value, what: &str) {
    if out.contains_key(key) {
        lost.push(format!("{what} (`{key}` is already set)"));
    } else {
        out.insert(key.to_string(), v);
    }
}

/// A short text for a value in a message.
fn brief(v: &Value) -> String {
    match v {
        Value::String(s) => format!("`{}`", s.as_str()),
        Value::Number(n) => format!("{}", n.value().as_f64()),
        _ => "(not a number)".into(),
    }
}

// ── dates ──────────────────────────────────────────────────────────────────────────────────────

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYY?MM?DD HH:MM:SS` with `sep` between the date's parts: seconds since 1970.
fn parse_stamp(s: &str, sep: char) -> Option<i64> {
    let (date, time) = s.split_once(' ')?;
    let mut d = date.split(sep);
    let (y, m, day) = (d.next()?, d.next()?, d.next()?);
    let mut t = time.split(':');
    let (h, mi, sec) = (t.next()?, t.next()?, t.next()?);
    if d.next().is_some() || t.next().is_some() || y.len() != 4 || [m, day, h, mi, sec].iter().any(|p| p.len() != 2) {
        return None;
    }
    let all_digits = [y, m, day, h, mi, sec].iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()));
    if !all_digits {
        return None;
    }
    let [y, m, day, h, mi, sec] = [y, m, day, h, mi, sec].map(|p| p.parse::<i64>().unwrap_or(-1));
    if !(1..=12).contains(&m) || day < 1 || day > days_in_month(y, m) || h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    Some(days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + sec)
}

fn format_stamp(t: i64, sep: char) -> Option<String> {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    if !(0..=9999).contains(&y) {
        return None;
    }
    let s = t.rem_euclid(86_400);
    Some(format!("{y:04}{sep}{m:02}{sep}{d:02} {:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60))
}

/// A UFO date (`2026/10/08 12:00:00`) as the reference writes one (`2026-10-08 12:00:00 +0000`).
fn ufo_date_to_model(s: &str) -> Option<String> {
    format_stamp(parse_stamp(s, '/')?, '-').map(|t| format!("{t} +0000"))
}

/// The reference's date as a UFO date. As glyphsLib does, a time-zone offset is added to the time,
/// and a 12-hour time with `AM` or `PM` is read too.
fn model_date_to_ufo(s: &str) -> Option<String> {
    let s = s.trim();
    let (s, offset) = match s.rsplit_once(' ') {
        Some((rest, z)) if z.len() == 5 && (z.starts_with('+') || z.starts_with('-')) && z.bytes().skip(1).all(|b| b.is_ascii_digit()) => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z.get(1..3)?.parse().ok()?;
            let m: i64 = z.get(3..5)?.parse().ok()?;
            (rest, sign * (h * 3600 + m * 60))
        }
        _ => (s, 0),
    };
    let t = match s.rsplit_once(' ') {
        Some((rest, half @ ("AM" | "PM"))) => {
            let (date, time) = rest.split_once(' ')?;
            let (h, tail) = time.split_once(':')?;
            let h: i64 = h.parse().ok()?;
            if !(1..=12).contains(&h) {
                return None;
            }
            let h24 = h % 12 + if half == "PM" { 12 } else { 0 };
            parse_stamp(&format!("{date} {h24:02}:{tail}"), '-')?
        }
        _ => parse_stamp(s, '-')?,
    };
    format_stamp(t + offset, '/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_plist::xmlplist;

    fn dict(xml: &str) -> Dict {
        let doc = xmlplist::parse(&format!("<plist version=\"1.0\"><dict>{xml}</dict></plist>")).unwrap();
        let Value::Dict(d) = doc.root else { panic!() };
        d
    }

    #[test]
    fn names_and_values_take_the_reference_names() {
        let info = dict(
            "<key>openTypeNameDesigner</key><string>A. Designer</string>\
             <key>openTypeOS2TypoAscender</key><integer>800</integer>\
             <key>openTypeOS2WinDescent</key><integer>250</integer>\
             <key>openTypeOS2Selection</key><array><integer>7</integer><integer>9</integer></array>\
             <key>openTypeOS2CodePageRanges</key><array><integer>0</integer><integer>29</integer></array>\
             <key>openTypeOS2Type</key><array><integer>3</integer></array>\
             <key>postscriptIsFixedPitch</key><true/>\
             <key>openTypeHeadCreated</key><string>2026/10/08 12:30:00</string>\
             <key>year</key><integer>2026</integer>",
        );
        let i = read(&info, 1000.0);
        assert_eq!(i.properties.value("designers"), Some("A. Designer"));
        let p = &i.params;
        assert_eq!(p.get("typoAscender").and_then(int_of), Some(800));
        assert_eq!(p.get("winDescent").and_then(int_of), Some(250));
        assert_eq!(p.get(TYPO_METRICS).and_then(int_of), Some(1));
        assert_eq!(p.get(SELECTION_REST).and_then(ints_of), Some(vec![9]));
        assert_eq!(p.get(CODE_PAGE_PARAM).and_then(ints_of), Some(vec![1252]));
        assert_eq!(p.get(CODE_PAGE_REST).and_then(ints_of), Some(vec![29]));
        assert!(p.get("fsType").is_none(), "the reference's default is no parameter");
        assert_eq!(p.get("isFixedPitch").and_then(int_of), Some(1));
        assert_eq!(p.get("year").and_then(int_of), Some(2026), "a UFO-only key keeps its name");
        assert_eq!(p.get("underlinePosition").and_then(int_of), Some(-75), "ufo2ft's default, which the file means");
        assert!(p.get("underlineThickness").is_none(), "50 either way");
        assert_eq!(i.date.as_deref(), Some("2026-10-08 12:30:00 +0000"));
        assert!(i.params.0.iter().all(|p| !is_raw(p)));
        let (out, lost) = write(&i, 1000.0, Some(&info));
        assert!(mismatches(&info, &out).is_empty(), "{out:?}");
        assert!(lost.is_empty(), "{lost:?}");
    }

    #[test]
    fn a_new_ufo_gets_the_reference_defaults() {
        let mut i = Info::default();
        i.params.0.push(Param::new("typoAscender", Value::String("800".into())));
        i.params.0.push(Param::new("Use Typo Metrics", int(1)));
        i.params.0.push(Param { disabled: true, ..Param::new("hheaAscender", int(900)) });
        i.params.0.push(Param::new("Axis Location", Value::Array(Array::default())));
        i.date = Some("2026-10-08 01:15:00 PM +0100".into());
        let (out, lost) = write(&i, 2048.0, None);
        assert_eq!(out.get("openTypeOS2TypoAscender"), Some(&int(800)));
        assert_eq!(out.get("openTypeOS2Selection"), Some(&int_list(&[7])));
        assert_eq!(out.get("openTypeOS2Type"), Some(&int_list(&[3])));
        assert_eq!(out.get("postscriptUnderlineThickness"), Some(&int(50)));
        assert_eq!(out.get("openTypeHeadCreated").and_then(Value::as_str), Some("2026/10/08 14:15:00"));
        assert!(!out.contains_key("openTypeHheaAscender"));
        // `Axis Location` has no key here: the lib holds it.
        assert!(!out.keys().any(|k| k.contains("Axis")));
        assert_eq!(lost, ["parameter `hheaAscender` (switched off)"]);
    }

    #[test]
    fn localised_names_fold_into_properties() {
        let rec = |id: i64, lang: i64, s: &str| {
            format!(
                "<dict><key>encodingID</key><integer>1</integer><key>languageID</key><integer>{lang}</integer>\
                 <key>nameID</key><integer>{id}</integer><key>platformID</key><integer>3</integer><key>string</key><string>{s}</string></dict>"
            )
        };
        let info = dict(&format!(
            "<key>openTypeNameDesigner</key><string>Designer</string><key>openTypeNameRecords</key><array>{}{}{}</array>",
            rec(9, 0x407, "Gestalter"),
            rec(1, 0x411, "Famiri"),
            rec(4, 0x407, "Voll"),
        ));
        let i = read(&info, 1000.0);
        let PropertyValue::Localized(v) = &i.properties.get("designers").unwrap().value else { panic!() };
        assert_eq!(v.iter().map(|l| l.language.as_str()).collect::<Vec<_>>(), ["dflt", "DEU"]);
        assert!(i.properties.get(FAMILY_NAMES).is_some());
        assert_eq!(i.params.get(NAME_ENTRY).and_then(Value::as_str), Some("4 3 1 1031; Voll"));
        assert!(mismatches(&info, &write(&i, 1000.0, Some(&info)).0).is_empty());

        // Records in an order the writer would not produce stay entries, in their order.
        let info = dict(&format!(
            "<key>openTypeNameDesigner</key><string>D</string><key>openTypeNameRecords</key><array>{}{}</array>",
            rec(4, 0x407, "Voll"),
            rec(9, 0x407, "Gestalter")
        ));
        let i = read(&info, 1000.0);
        assert_eq!(i.params.all(NAME_ENTRY).count(), 2);
        assert!(mismatches(&info, &write(&i, 1000.0, Some(&info)).0).is_empty());
    }

    #[test]
    fn values_that_would_not_come_back_are_kept_as_written() {
        let info = dict(
            "<key>openTypeOS2Selection</key><array><integer>8</integer><integer>7</integer></array>\
             <key>openTypeOS2WinDescent</key><integer>-250</integer>\
             <key>postscriptIsFixedPitch</key><integer>1</integer>\
             <key>openTypeHeadCreated</key><string>yesterday</string>",
        );
        let i = read(&info, 1000.0);
        for key in ["openTypeOS2Selection", "openTypeOS2WinDescent", "postscriptIsFixedPitch", "openTypeHeadCreated"] {
            assert!(i.params.0.iter().any(|p| p.name == key && is_raw(p)), "{key}: {:?}", i.params);
        }
        assert!(mismatches(&info, &write(&i, 1000.0, Some(&info)).0).is_empty());
    }

    #[test]
    fn gasp_and_dates_convert_both_ways() {
        let g = dict(
            "<key>openTypeGaspRangeRecords</key><array><dict><key>rangeGaspBehavior</key><array><integer>1</integer><integer>3</integer></array>\
             <key>rangeMaxPPEM</key><integer>8</integer></dict><dict><key>rangeGaspBehavior</key><array><integer>0</integer><integer>1</integer>\
             <integer>2</integer><integer>3</integer></array><key>rangeMaxPPEM</key><integer>65535</integer></dict></array>",
        );
        let i = read(&g, 1000.0);
        let table = i.params.get(GASP_PARAM).and_then(Value::as_dict).unwrap();
        assert_eq!(table.get("8").and_then(int_of), Some(10));
        assert_eq!(table.get("65535").and_then(int_of), Some(15));
        assert!(mismatches(&g, &write(&i, 1000.0, Some(&g)).0).is_empty());
        assert_eq!(model_date_to_ufo("2025-12-31 23:30:00 +0100").as_deref(), Some("2026/01/01 00:30:00"));
        assert_eq!(model_date_to_ufo("2024-02-29 12:00:00 AM").as_deref(), Some("2024/02/29 00:00:00"));
        assert_eq!(model_date_to_ufo("2023-02-29 12:00:00"), None);
        assert_eq!(ufo_date_to_model("1999/01/02 03:04:05").as_deref(), Some("1999-01-02 03:04:05 +0000"));
    }
}
