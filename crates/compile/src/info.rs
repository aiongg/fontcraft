//! What the tables say about the font besides its glyphs: names, style flags, vendor, embedding
//! permissions, version, `head` settings and every vertical and horizontal metric with its
//! default (`docs/spec/40-compile.md` § 1.5, the `compile.name*`, `compile.os2*` and
//! `compile.*Defaults` rows).
//!
//! Every value is derived as fontc 1.0.0 derives it, ported from its front ends and its
//! intermediate representation (Apache-2.0, see `NOTICE`): the name rules of
//! `glyphs2fontir/src/source.rs` (`names`) and `ufo2fontir/src/source.rs` (`names`,
//! `selection_flags_implicit`), the fallbacks of `fontir/src/ir.rs` (`NameBuilder`), the metric
//! defaults of `GlobalMetricsBuilder::populate_defaults` and the static metadata both front ends
//! fill. The model is shaped like the `.glyphs` format, and a UFO's font information reaches it
//! as the same properties and custom parameters (`crates/io-ufo/src/fontinfo.rs`); where the two
//! front ends derive a value differently, [`fontcraft_model::is_ufo_family`] picks the rule.
//!
//! What is not followed, and why:
//!
//! | Value | fontc | Here |
//! |---|---|---|
//! | names of a variable instance (`.glyphs`) | merged into the font's names | the target's own; instances are compiled from M2.5 |
//! | `openTypeNameRecords` of a UFO | written as English records whatever their language | records in their own language |
//! | custom parameters of a master (`.glyphs`) | read from the font only, except metrics | the master's in place of the font's |

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_model::plist::{Num, Value};
use fontcraft_model::{DEFAULT_LANGUAGE, Params, PropertyValue, Resolved, is_ufo_family, language_id};
use write_fonts::tables::head::Flags;
use write_fonts::tables::os2::SelectionFlags;
use write_fonts::types::Tag;

use crate::ot_round;

/// The vendor id written when the source has none, or only spaces (fontc's `DEFAULT_VENDOR_ID`).
pub const DEFAULT_VENDOR_ID: &str = "NONE";
/// Longest name string written, in characters. The `name` table's offsets are 16-bit, so all
/// strings together must stay under 64 KiB; a longer one is cut with a warning.
pub const MAX_NAME_CHARS: usize = 8_000;
/// Most name records written.
pub const MAX_NAME_RECORDS: usize = 1_000;

/// One `name` record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NameRecord {
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    pub id: u16,
    pub text: String,
}

/// Every global metric at the target, rounded half up as fontc rounds them before writing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlobalMetrics {
    pub ascender: f64,
    pub descender: f64,
    pub cap_height: f64,
    pub x_height: f64,
    pub typo_ascender: f64,
    pub typo_descender: f64,
    pub typo_line_gap: f64,
    pub win_ascent: f64,
    pub win_descent: f64,
    pub hhea_ascender: f64,
    pub hhea_descender: f64,
    pub hhea_line_gap: f64,
    pub caret_slope_rise: f64,
    pub caret_slope_run: f64,
    pub caret_offset: f64,
    pub subscript_x_size: f64,
    pub subscript_y_size: f64,
    pub subscript_x_offset: f64,
    pub subscript_y_offset: f64,
    pub superscript_x_size: f64,
    pub superscript_y_size: f64,
    pub superscript_x_offset: f64,
    pub superscript_y_offset: f64,
    pub strikeout_size: f64,
    pub strikeout_position: f64,
    pub underline_thickness: f64,
    pub underline_position: f64,
    pub vhea_ascender: f64,
    pub vhea_descender: f64,
    pub vhea_line_gap: f64,
    pub vhea_caret_slope_rise: f64,
    pub vhea_caret_slope_run: f64,
    pub vhea_caret_offset: f64,
}

/// What the tables say about the font besides its glyphs.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub names: Vec<NameRecord>,
    pub selection: SelectionFlags,
    pub vendor: Tag,
    pub fs_type: u16,
    pub fixed_pitch: bool,
    pub weight_class: u16,
    pub width_class: u16,
    pub family_class: i16,
    pub panose: Option<[u8; 10]>,
    /// `OS/2` Unicode range bits the source sets; computed from the code points otherwise.
    pub unicode_ranges: Option<BTreeSet<u32>>,
    /// `OS/2` code page bits the source sets; computed from the code points otherwise.
    pub code_pages: Option<BTreeSet<u32>>,
    pub version_major: i64,
    pub version_minor: u64,
    pub head_flags: Flags,
    pub lowest_rec_ppem: u16,
    /// Creation time from the source's date, in seconds since the Unix epoch.
    pub created: Option<i64>,
    /// Counter-clockwise, as OpenType states it.
    pub italic_angle: f64,
    pub metrics: GlobalMetrics,
    /// Whether `vhea` and `vmtx` are written.
    pub vertical: bool,
}

/// Plist values as fontc's readers take them: numbers, or strings that read as numbers.
fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => match n.value() {
            Num::Int(i) => Some(i),
            Num::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => Some(f as i64),
            Num::Float(_) => None,
        },
        Value::String(s) => s.as_str().trim().parse().ok(),
        _ => None,
    }
}

fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(n.value().as_f64()).filter(|f| f.is_finite()),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

fn boolean(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => int(v).map(|i| i == 1),
    }
}

/// A list of integers; a value that is not one is no list.
fn ints(v: &Value) -> Option<Vec<i64>> {
    v.as_array()?.items.iter().take(MAX_LIST).map(int).collect()
}

/// Most entries read from a list parameter (bits, code pages, panose).
const MAX_LIST: usize = 256;

/// The first parameter of these names with a value `read` accepts.
fn param<T>(params: &Params, names: &[&str], read: impl Fn(&Value) -> Option<T>) -> Option<T> {
    names.iter().find_map(|n| params.get(n).and_then(&read))
}

/// The value of a set of bits given as their numbers, bits beyond `max` left out.
fn bits(list: &[i64], max: i64) -> u64 {
    list.iter().filter(|b| (0..=max).contains(*b)).fold(0u64, |acc, b| acc | (1u64 << b))
}

/// Windows code pages and their `ulCodePageRange` bits (fontc's `codepage_range_bit`); a
/// number below 64 is a bit itself.
fn code_page_bit(page: i64) -> Option<u32> {
    Some(match page {
        1252 => 0,
        1250 => 1,
        1251 => 2,
        1253 => 3,
        1254 => 4,
        1255 => 5,
        1256 => 6,
        1257 => 7,
        1258 => 8,
        874 => 16,
        932 => 17,
        936 => 18,
        949 => 19,
        950 => 20,
        1361 => 21,
        869 => 48,
        866 => 49,
        865 => 50,
        864 => 51,
        863 => 52,
        862 => 53,
        861 => 54,
        860 => 55,
        857 => 56,
        855 => 57,
        852 => 58,
        775 => 59,
        737 => 60,
        708 => 61,
        850 => 62,
        437 => 63,
        v if (0..64).contains(&v) => u32::try_from(v).ok()?,
        _ => return None,
    })
}

/// The `codePageRanges` parameter: code pages as numbers, strings of numbers or `bit N`.
fn code_pages(v: &Value) -> Option<BTreeSet<u32>> {
    v.as_array()?
        .items
        .iter()
        .take(MAX_LIST)
        .map(|item| {
            let page = match item {
                Value::String(s) => match s.as_str().strip_prefix("bit ") {
                    Some(rest) => rest.trim().parse().ok(),
                    None => s.as_str().trim().parse().ok(),
                },
                other => int(other),
            };
            page.and_then(code_page_bit)
        })
        .collect()
}

/// The default-language value of a property, by fontc's preference: `dflt`, then `default`,
/// then `ENG`, then the first.
fn default_text(value: &PropertyValue) -> Option<&str> {
    match value {
        PropertyValue::Single(s) => Some(s.as_str()),
        PropertyValue::Localized(values) => [DEFAULT_LANGUAGE, "default", "ENG"]
            .iter()
            .find_map(|lang| values.iter().rev().find(|l| l.language == *lang))
            .or_else(|| values.first())
            .map(|l| l.value.as_str()),
    }
}

/// The text of a property, or of the first of `params` that holds a string: format version 2
/// keeps most names in parameters, and a UFO's name keys without a property are parameters.
fn text<'a>(font: &'a Resolved, property: &str, params: &[&str]) -> Option<&'a str> {
    font.properties
        .get(property)
        .and_then(|p| default_text(&p.value))
        .filter(|s| !s.is_empty())
        .or_else(|| params.iter().find_map(|n| font.params.get(n).and_then(Value::as_str)))
}

/// Which front end of fontc writes a name.
#[derive(Clone, Copy, PartialEq)]
enum From {
    Both,
    /// `.glyphs` sources only.
    Glyphs,
    /// UFO sources only.
    Ufo,
}

/// Name properties and the name ids they fill (fontc's `try_name_id` for `.glyphs` sources, its
/// UFO `names` for UFOs), with the parameters that hold the same name in format version 2 or in
/// a UFO.
const NAME_PROPERTIES: &[(&str, u16, &[&str], From)] = &[
    ("copyrights", 0, &["copyright"], From::Both),
    ("familyNames", 1, &[], From::Both),
    ("uniqueID", 3, &["uniqueID", "openTypeNameUniqueID"], From::Both),
    ("versionString", 5, &["versionString", "openTypeNameVersion"], From::Both),
    ("postscriptFontName", 6, &["postscriptFontName"], From::Both),
    ("trademarks", 7, &["trademark"], From::Both),
    ("manufacturers", 8, &["manufacturer"], From::Both),
    ("designers", 9, &["designer"], From::Both),
    ("descriptions", 10, &["description", "openTypeNameDescription"], From::Both),
    ("manufacturerURL", 11, &[], From::Both),
    ("designerURL", 12, &[], From::Both),
    ("licenses", 13, &["license", "openTypeNameLicense"], From::Both),
    ("licenseURL", 14, &["licenseURL", "openTypeNameLicenseURL"], From::Both),
    ("preferredFamilyNames", 16, &["preferredFamilyName", "openTypeNamePreferredFamilyName"], From::Both),
    ("preferredSubfamilyNames", 17, &["preferredSubfamilyName", "openTypeNamePreferredSubfamilyName"], From::Both),
    ("compatibleFullNames", 18, &["compatibleFullName", "openTypeNameCompatibleFullName"], From::Ufo),
    ("sampleTexts", 19, &["sampleText", "openTypeNameSampleText"], From::Both),
    ("WWSFamilyName", 21, &["WWSFamilyName", "openTypeNameWWSFamilyName"], From::Both),
    ("WWSSubfamilyName", 22, &["WWSSubfamilyName", "openTypeNameWWSSubfamilyName"], From::Ufo),
    ("variationsPostScriptNamePrefix", 25, &[], From::Glyphs),
];

/// Whether fontc writes a name of `from` for a source of this family.
fn writes(from: From, ufo: bool) -> bool {
    match from {
        From::Both => true,
        From::Glyphs => !ufo,
        From::Ufo => ufo,
    }
}

/// The four style-linked styles, in any case and with spaces around, and the flags each means
/// (a UFO's style names, as fontc reads them).
fn ribbi_flags(style: &str) -> Option<SelectionFlags> {
    match style.trim().to_lowercase().as_str() {
        "regular" => Some(SelectionFlags::REGULAR),
        "italic" => Some(SelectionFlags::ITALIC),
        "bold" => Some(SelectionFlags::BOLD),
        "bold italic" => Some(SelectionFlags::BOLD | SelectionFlags::ITALIC),
        _ => None,
    }
}

/// Collects the names by id, then fills what is missing as fontc's `NameBuilder` does.
#[derive(Default)]
struct NameBuilder {
    names: BTreeMap<u16, String>,
    version: (i64, u64),
}

impl NameBuilder {
    fn add(&mut self, id: u16, value: &str) {
        // XML end-of-line normalisation, which some sources' readers do not do.
        let value = if value.contains('\r') { value.replace("\r\n", "\n").replace('\r', "\n") } else { value.to_string() };
        self.names.insert(id, value);
    }

    fn get(&self, id: u16) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    /// A family name with the words of a style name added; with `drop_ribbi`, trailing
    /// `Regular`, `Bold` and `Italic` words left out (fontc's `make_family_name`).
    fn family_name(family: &str, style: &str, drop_ribbi: bool) -> String {
        let mut words = vec![family];
        words.extend(style.split_ascii_whitespace());
        if drop_ribbi {
            while words.last().is_some_and(|w| matches!(*w, "Regular" | "Bold" | "Italic")) {
                words.pop();
            }
        }
        words.join(" ")
    }

    /// The names with the fallbacks applied: legacy family and style from the typographic
    /// ones, version, full name, PostScript name and unique id; typographic names equal to the
    /// legacy ones dropped, and empty ones (which stop a fallback) removed.
    fn build(mut self, vendor: &str) -> BTreeMap<u16, String> {
        let suffix = if self.get(2).is_some() {
            None
        } else {
            let fallback = self.get(17).unwrap_or("Regular").to_string();
            let (style, suffix) = if is_ribbi(&fallback) { (fallback, None) } else { ("Regular".to_string(), Some(fallback)) };
            self.add(2, &style);
            suffix.filter(|s| !s.is_empty())
        };
        if self.get(1).is_none() {
            let mut family = self.get(16).unwrap_or("New Font").to_string();
            if let Some(s) = &suffix {
                family.push(' ');
                family.push_str(s);
            }
            self.add(1, &family);
        }
        if self.get(16).is_none() {
            let v = self.get(1).unwrap_or_default().to_string();
            self.add(16, &v);
        }
        if self.get(17).is_none() {
            let v = self.get(2).unwrap_or_default().to_string();
            self.add(17, &v);
        }
        if self.get(5).is_none() {
            let (major, minor) = self.version;
            self.add(5, &format!("Version {major}.{minor:0>3}"));
        }
        if self.get(4).is_none() {
            let full = Self::family_name(self.get(16).unwrap_or_default(), self.get(17).unwrap_or_default(), false);
            self.add(4, &full);
        }
        if self.get(6).is_none() {
            let mut family = self.get(16).unwrap_or_default().replace(' ', "");
            let style = self.get(17).unwrap_or_default();
            if !style.is_empty() {
                family.push('-');
            }
            let mut ps = Self::family_name(&family, style, false);
            ps.retain(|c| !c.is_ascii_whitespace() && !"[](){}<>/%".contains(c) && (33..127).contains(&u32::from(c)));
            self.add(6, &ps);
        }
        if self.get(3).is_none() {
            let version = self.get(5).unwrap_or_default().replace("Version ", "");
            let unique = format!("{version};{vendor};{}", self.get(6).unwrap_or_default());
            self.add(3, &unique);
        }
        if self.get(1).is_some() && self.get(2).is_some() && self.get(1) == self.get(16) && self.get(2) == self.get(17) {
            self.names.remove(&16);
            self.names.remove(&17);
        }
        self.names.retain(|_, v| !v.is_empty());
        self.names
    }
}

/// Whether a style name is one of the four style-linked ones, in any case (fontc's `is_ribbi`).
fn is_ribbi(style: &str) -> bool {
    matches!(style.to_lowercase().as_str(), "regular" | "italic" | "bold" | "bold italic")
}

/// Windows platform, Unicode encoding: BMP or full repertoire by the text (fontc's `NameKey`).
fn record(id: u16, language: u16, text: &str) -> NameRecord {
    let encoding = if text.chars().all(|c| u32::from(c) < 0xFFFF) { 1 } else { 10 };
    NameRecord { platform: 3, encoding, language, id, text: text.to_string() }
}

/// A `Name Table Entry` parameter: `"id[ platform[ encoding[ language]]]; text"`, numbers in
/// decimal or `0x` hexadecimal (fontc's `NameTableEntry`).
fn name_table_entry(s: &str) -> Option<NameRecord> {
    let (head, tail) = s.split_once(';')?;
    let number = |t: &str| match t.strip_prefix("0x") {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => t.parse::<u16>().ok(),
    };
    let ids: Vec<u16> = head.split(' ').filter_map(number).take(5).collect();
    let text = tail.trim().to_string();
    let (id, platform, encoding, language) = match ids.as_slice() {
        [id] | [id, 3] => (*id, 3, 1, 0x409),
        [id, 1] => (*id, 1, 0, 0),
        [id, p, e] => (*id, *p, *e, 0),
        [id, p, e, l] => (*id, *p, *e, *l),
        _ => return None,
    };
    Some(NameRecord { platform, encoding, language, id, text })
}

/// Seconds since the Unix epoch of a date as the `.glyphs` format writes it
/// (`2019-01-09 11:23:45 +0000`) or a UFO's `openTypeHeadCreated` (`2019/01/09 11:23:45`).
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(' ')?;
    let (time, zone) = match rest.trim().split_once(' ') {
        Some((t, z)) => (t, Some(z.trim())),
        None => (rest.trim(), None),
    };
    let sep = if date.contains('/') { '/' } else { '-' };
    let mut d = date.split(sep).map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || !(0..24).contains(&hh) || !(0..60).contains(&mm) || !(0..=60).contains(&ss) {
        return None;
    }
    if !(1..=9999).contains(&y) {
        return None;
    }
    let offset = match zone {
        Some(z) if z.len() == 5 => {
            let sign = match z.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            sign * (z.get(1..3)?.parse::<i64>().ok()? * 3600 + z.get(3..5)?.parse::<i64>().ok()? * 60)
        }
        Some(_) => return None,
        None => 0,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

/// The OpenType offset for a slant: `offset` times the tangent of the angle (fontc's
/// `adjust_offset`, counter-clockwise degrees).
fn slanted(offset: f64, angle: f64) -> f64 {
    if angle.abs() >= f64::EPSILON { offset * (-angle).to_radians().tan() } else { 0.0 }
}

/// Derives what the tables say about `font`. `style` is the style name used for the names when
/// not empty (the compile option), else the resolved master's or instance's.
pub fn derive(font: &Resolved, style: &str, warnings: &mut Vec<String>) -> Info {
    let ufo = is_ufo_family(font.format);
    let style = if style.is_empty() { font.style_name.as_str() } else { style };
    let params = &font.params;
    let italic_angle = -font.metrics.italic_angle;

    // Style flags.
    let mut selection = SelectionFlags::empty();
    if param(params, &["Use Typo Metrics"], boolean) == Some(true) {
        selection |= SelectionFlags::USE_TYPO_METRICS;
    }
    if param(params, &["Has WWS Names"], boolean) == Some(true) {
        selection |= SelectionFlags::WWS;
    }
    if ufo {
        if let Some(rest) = param(params, &["openTypeOS2SelectionUnsupportedBits"], ints) {
            selection |= SelectionFlags::from_bits_truncate(u16::try_from(bits(&rest, 15)).unwrap_or(0));
        }
        let implicit = param(params, &["styleMapStyleName"], |v| v.as_str().and_then(ribbi_flags))
            .or_else(|| {
                text(font, "preferredSubfamilyNames", &["preferredSubfamilyName", "openTypeNamePreferredSubfamilyName"]).and_then(ribbi_flags)
            })
            .or_else(|| (!style.is_empty()).then(|| ribbi_flags(style)).flatten())
            .unwrap_or(SelectionFlags::REGULAR);
        selection |= implicit;
    } else {
        if italic_angle != 0.0 {
            selection |= SelectionFlags::ITALIC;
        }
        // The master's name, compared in lower case as it is.
        selection |= match style.to_ascii_lowercase().as_str() {
            "italic" => SelectionFlags::ITALIC,
            "bold" => SelectionFlags::BOLD,
            "bold italic" => SelectionFlags::BOLD | SelectionFlags::ITALIC,
            _ => SelectionFlags::empty(),
        };
        if let Some(link) = &font.style {
            if link.is_bold {
                selection |= SelectionFlags::BOLD;
            }
            if link.is_italic {
                selection |= SelectionFlags::ITALIC;
            }
        }
        if !selection.intersects(SelectionFlags::ITALIC | SelectionFlags::BOLD) {
            selection |= SelectionFlags::REGULAR;
        }
    }

    // Vendor: spaces or nothing mean none.
    let vendor_text = text(font, "vendorID", &["vendorID", "openTypeOS2VendorID"]);
    let vendor = vendor_text
        .filter(|v| !v.trim().is_empty())
        .and_then(|v| {
            let bytes = v.as_bytes();
            if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
                warnings.push(format!("the vendor id `{v}` is not four printable ASCII characters; `{DEFAULT_VENDOR_ID}` was written"));
                return None;
            }
            let mut tag = [b' '; 4];
            for (slot, b) in tag.iter_mut().zip(bytes) {
                *slot = *b;
            }
            Some(Tag::new(&tag))
        })
        .unwrap_or_else(|| Tag::new(b"NONE"));

    let names = names(font, style, selection, vendor_text.unwrap_or(DEFAULT_VENDOR_ID), ufo, warnings);

    // Weight and width: a UFO's own value; a `.glyphs` font's from its axes, as fontc derives
    // them for the variable font it builds.
    let axis = |tag: &str| font.axes.iter().find(|a| a.tag == tag && a.variable).map(|a| a.user);
    let weight_class = if ufo {
        param(params, &["weightClass"], int).and_then(|v| u16::try_from(v).ok())
    } else {
        axis("wght").map(|w| ot_round(w.clamp(1.0, 1000.0)) as u16)
    }
    .unwrap_or(400);
    let width_class =
        if ufo { param(params, &["widthClass"], int).and_then(|v| u16::try_from(v).ok()) } else { axis("wdth").map(nearest_width_class) }
            .unwrap_or(5);

    let fs_type = param(params, &["fsType", "openTypeOS2Type"], ints).map_or(1 << 3, |b| u16::try_from(bits(&b, 15)).unwrap_or(0));
    let family_class = param(params, &["openTypeOS2FamilyClass"], ints)
        .and_then(|v| match v.as_slice() {
            [class, sub] => Some(((*class as i16) << 8) | (*sub as i16)),
            _ => None,
        })
        .unwrap_or(0);
    let panose = param(params, &["panose", "openTypeOS2Panose"], ints).map(|v| {
        let mut out = [0u8; 10];
        for (slot, b) in out.iter_mut().zip(v) {
            *slot = b as u8;
        }
        out
    });
    let unicode_ranges = param(params, &["unicodeRanges", "openTypeOS2UnicodeRanges"], ints)
        .map(|v| v.into_iter().filter_map(|b| u32::try_from(b).ok().filter(|b| *b < 128)).collect());
    let code_pages = param(params, &["codePageRanges"], code_pages).map(|mut set| {
        if let Some(rest) = param(params, &["codePageRangesUnsupportedBits"], ints) {
            set.extend(rest.into_iter().filter_map(|b| u32::try_from(b).ok().filter(|b| *b < 64)));
        }
        set
    });

    let head_flags = param(params, &["openTypeHeadFlags"], ints)
        .map(|b| Flags::from_bits_truncate(u16::try_from(bits(&b, 15)).unwrap_or(0)))
        .unwrap_or(Flags::BASELINE_AT_Y_0 | Flags::LSB_AT_X_0);
    let lowest_rec_ppem = param(params, &["openTypeHeadLowestRecPPEM", "lowestRecPPEM"], int).and_then(|v| u16::try_from(v).ok()).unwrap_or(6);
    let fixed_pitch = param(params, &["isFixedPitch", "postscriptIsFixedPitch"], boolean).unwrap_or(false);

    let metrics = metrics(font, italic_angle, ufo);
    let vertical = if ufo {
        ["vheaVertAscender", "vheaVertDescender", "vheaVertLineGap"].iter().all(|n| params.get(n).and_then(float).is_some())
    } else {
        ["vheaVertAscender", "vheaVertDescender", "vheaVertLineGap"].iter().all(|n| params.get(n).and_then(int).is_some())
            || font.glyphs.iter().any(|g| g.vert_width.is_some() || g.vert_origin.is_some())
    };

    Info {
        names,
        selection,
        vendor,
        fs_type,
        fixed_pitch,
        weight_class,
        width_class,
        family_class,
        panose,
        unicode_ranges,
        code_pages,
        version_major: i64::from(font.version_major),
        version_minor: u64::from(font.version_minor),
        head_flags,
        lowest_rec_ppem,
        created: font.date.as_deref().and_then(parse_date),
        italic_angle,
        metrics,
        vertical,
    }
}

/// The `OS/2` width class whose percentage is nearest (fontc's `WidthClass::nearest`).
fn nearest_width_class(percent: f64) -> u16 {
    const PERCENT: [f64; 9] = [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0];
    let mut best = (5u16, f64::INFINITY);
    for (class, p) in (1u16..).zip(PERCENT) {
        let d = (p - percent).abs();
        if d < best.1 {
            best = (class, d);
        }
    }
    best.0
}

/// The name records: the font's names with fontc's fallbacks, then the localised values, then
/// the `Name Table Entry` parameters.
fn names(font: &Resolved, style: &str, selection: SelectionFlags, vendor: &str, ufo: bool, warnings: &mut Vec<String>) -> Vec<NameRecord> {
    let mut b = NameBuilder { version: (i64::from(font.version_major), u64::from(font.version_minor)), ..Default::default() };
    for (property, id, params, from) in NAME_PROPERTIES {
        let value = if *id == 1 || !writes(*from, ufo) { None } else { text(font, property, params) };
        if let Some(v) = value {
            b.add(*id, v);
        }
    }
    if ufo {
        // fontc's UFO rules: the style-map names are the legacy ones; the typographic ones fall
        // back to the family and style names.
        b.names.remove(&1);
        if let Some(v) = font.params.get("styleMapFamilyName").and_then(Value::as_str) {
            b.add(1, v);
        }
        if let Some(flags) = font.params.get("styleMapStyleName").and_then(Value::as_str).and_then(ribbi_flags) {
            b.add(2, ribbi_name(flags));
        }
        if b.get(16).is_none() && !font.family_name.is_empty() {
            b.add(16, &font.family_name.clone());
        }
        if b.get(17).is_none() && !style.is_empty() {
            b.add(17, style);
        }
    } else {
        // fontc's `.glyphs` rules: the legacy style from the style flags, the legacy family
        // with the master's style words that are not Regular, Bold or Italic.
        b.add(2, ribbi_name(selection & (SelectionFlags::BOLD | SelectionFlags::ITALIC)));
        let family = font.family_name.clone();
        b.add(1, &NameBuilder::family_name(&family, style, true));
        if b.get(16).is_none() {
            b.add(16, &family);
        }
        if b.get(17).is_none() {
            b.add(17, style);
        }
    }
    let built = b.build(vendor);
    let mut records: BTreeMap<(u16, u16, u16, u16), String> = BTreeMap::new();
    for (id, text) in built {
        let r = record(id, 0x409, &text);
        records.insert((r.platform, r.encoding, r.language, r.id), r.text);
    }
    // Localised values: a record in each language the format names.
    for p in &font.properties.0 {
        let PropertyValue::Localized(values) = &p.value else { continue };
        let Some((_, id, _, _)) = NAME_PROPERTIES.iter().find(|(k, _, _, from)| *k == p.key && writes(*from, ufo)) else { continue };
        for l in values.iter().filter(|l| !matches!(l.language.as_str(), "dflt" | "default" | "ENG")) {
            let Some(lang) = language_id(&l.language) else {
                warnings.push(format!("the `{}` name in language `{}` was left out: the language has no Windows language id", p.key, l.language));
                continue;
            };
            let r = record(*id, lang, &l.value);
            records.insert((r.platform, r.encoding, r.language, r.id), r.text);
        }
    }
    for entry in font.params.all("Name Table Entry").filter_map(Value::as_str) {
        match name_table_entry(entry) {
            Some(r) => {
                records.insert((r.platform, r.encoding, r.language, r.id), r.text);
            }
            None => {
                warnings.push(format!("the `Name Table Entry` parameter `{entry}` is not `id [platform encoding language]; text` and was left out"))
            }
        }
    }
    let mut out = Vec::with_capacity(records.len().min(MAX_NAME_RECORDS));
    for ((platform, encoding, language, id), mut text) in records {
        if out.len() == MAX_NAME_RECORDS {
            warnings.push(format!("the font has more than {MAX_NAME_RECORDS} name records; the rest were left out"));
            break;
        }
        if text.chars().count() > MAX_NAME_CHARS {
            warnings.push(format!("name {id} is longer than {MAX_NAME_CHARS} characters and was cut"));
            text = text.chars().take(MAX_NAME_CHARS).collect();
        }
        out.push(NameRecord { platform, encoding, language, id, text });
    }
    out
}

/// The legacy style name of style flags.
fn ribbi_name(flags: SelectionFlags) -> &'static str {
    let (bold, italic) = (flags.contains(SelectionFlags::BOLD), flags.contains(SelectionFlags::ITALIC));
    match (bold, italic) {
        (true, true) => "Bold Italic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (false, false) => "Regular",
    }
}

/// The global metrics with fontc's defaults (`populate_defaults` and the `.glyphs` front end's
/// own), rounded half up.
fn metrics(font: &Resolved, italic_angle: f64, ufo: bool) -> GlobalMetrics {
    let upm = f64::from(font.units_per_em);
    let p = &font.params;
    let get = |names: &[&str]| param(p, names, if ufo { float } else { |v: &Value| int(v).map(|i| i as f64) });
    let getf = |names: &[&str]| param(p, names, float);
    let m = &font.metrics;
    let (ascender, descender, x_height, cap_height) = (m.ascender, m.descender, m.x_height, m.cap_height);

    let typo_line_gap = get(&["typoLineGap"]).unwrap_or_else(|| (upm * 1.2 + descender - ascender).max(0.0));
    let typo_ascender = get(&["typoAscender"]).unwrap_or(ascender);
    let typo_descender = get(&["typoDescender"]).unwrap_or(descender);
    let subscript_x_size = get(&["subscriptXSize", "openTypeOS2SubscriptXSize"]).unwrap_or(upm * 0.65);
    let subscript_y_size = get(&["subscriptYSize", "openTypeOS2SubscriptYSize"]).unwrap_or(upm * 0.6);
    let subscript_y_offset = get(&["subscriptYOffset", "openTypeOS2SubscriptYOffset"]).unwrap_or(upm * 0.075);
    let superscript_y_offset = get(&["superscriptYOffset", "openTypeOS2SuperscriptYOffset"]).unwrap_or(upm * 0.35);
    // The model means the `.glyphs` format's defaults here for every format: a UFO without the
    // keys reads as parameters with ufo2ft's (`crates/io-ufo/src/fontinfo.rs`).
    let underline_thickness = getf(&["underlineThickness"]).unwrap_or(50.0);
    let underline_position = getf(&["underlinePosition"]).unwrap_or(-100.0);
    let (vhea_ascender, vhea_descender, vhea_line_gap) = if ufo {
        (getf(&["vheaVertAscender"]).unwrap_or(0.0), getf(&["vheaVertDescender"]).unwrap_or(0.0), getf(&["vheaVertLineGap"]).unwrap_or(0.0))
    } else {
        (get(&["vheaVertAscender"]).unwrap_or(upm / 2.0), get(&["vheaVertDescender"]).unwrap_or(-upm / 2.0), get(&["vheaVertLineGap"]).unwrap_or(upm))
    };
    let r = ot_round;
    GlobalMetrics {
        ascender: r(ascender),
        descender: r(descender),
        cap_height: r(cap_height),
        x_height: r(x_height),
        typo_ascender: r(typo_ascender),
        typo_descender: r(typo_descender),
        typo_line_gap: r(typo_line_gap),
        win_ascent: r(get(&["winAscent"]).unwrap_or(ascender + typo_line_gap)),
        win_descent: r(get(&["winDescent"]).map_or(descender.abs(), f64::abs)),
        hhea_ascender: r(get(&["hheaAscender"]).unwrap_or(ascender + typo_line_gap)),
        hhea_descender: r(get(&["hheaDescender"]).unwrap_or(descender)),
        hhea_line_gap: r(get(&["hheaLineGap"]).unwrap_or(0.0)),
        caret_slope_rise: r(get(&["openTypeHheaCaretSlopeRise"]).unwrap_or(upm)),
        caret_slope_run: r(get(&["openTypeHheaCaretSlopeRun"]).unwrap_or_else(|| slanted(upm, italic_angle))),
        caret_offset: r(get(&["openTypeHheaCaretOffset"]).unwrap_or(0.0)),
        subscript_x_size: r(subscript_x_size),
        subscript_y_size: r(subscript_y_size),
        subscript_x_offset: r(get(&["subscriptXOffset", "openTypeOS2SubscriptXOffset"]).unwrap_or_else(|| slanted(-subscript_y_offset, italic_angle))),
        subscript_y_offset: r(subscript_y_offset),
        superscript_x_size: r(get(&["superscriptXSize", "openTypeOS2SuperscriptXSize"]).unwrap_or(subscript_x_size)),
        superscript_y_size: r(get(&["superscriptYSize", "openTypeOS2SuperscriptYSize"]).unwrap_or(subscript_y_size)),
        superscript_x_offset: r(
            get(&["superscriptXOffset", "openTypeOS2SuperscriptXOffset"]).unwrap_or_else(|| slanted(superscript_y_offset, italic_angle))
        ),
        superscript_y_offset: r(superscript_y_offset),
        strikeout_size: r(get(&["strikeoutSize", "openTypeOS2StrikeoutSize"]).unwrap_or(underline_thickness)),
        strikeout_position: r(get(&["strikeoutPosition", "openTypeOS2StrikeoutPosition"]).unwrap_or(if x_height != 0.0 {
            x_height * 0.6
        } else {
            upm * 0.22
        })),
        underline_thickness: r(underline_thickness),
        underline_position: r(underline_position),
        vhea_ascender: r(vhea_ascender),
        vhea_descender: r(vhea_descender),
        vhea_line_gap: r(vhea_line_gap),
        vhea_caret_slope_rise: r(get(&["openTypeVheaCaretSlopeRise"]).unwrap_or(0.0)),
        vhea_caret_slope_run: r(get(&["openTypeVheaCaretSlopeRun"]).unwrap_or(1.0)),
        vhea_caret_offset: r(get(&["openTypeVheaCaretOffset"]).unwrap_or(0.0)),
    }
}
