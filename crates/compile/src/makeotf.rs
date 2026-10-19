//! The project workflow of AFDKO's makeotf (M12.40; rows `compile.makeotfOptionPrecedence`,
//! `.makeotfReleaseMode`, `.goadbRenaming`, `.glyphOrderMakeotf`, `.unicodeFromGlyphNames`,
//! `.cmapLegacyOptions`, `.fontMenuNameDb`, `.nameLengthAdvice`, `.os2WeightAdvice`,
//! `.kernClassOptimisation`, `.makeotfSyntheticGlyphs`, `.makeotfMetrics`).
//!
//! A project compile reads the settings of a project file (`current.fpr`), the option words of
//! the command line and a `fontinfo` keyword file, a FontMenuNameDB and a GlyphOrderAndAliasDB
//! (parsed by the engine with `fontcraft_tables::goadb` and handed over as [`GlyphAlias`]es).
//! [`Project::prepare`] turns the source into the font that compiler would build: glyph names,
//! order, code points and menu names. The ordinary compile then builds it, and [`finish`] adds
//! what is not in the model (a stub `DSIG`, a stub `cmap`).
//!
//! Ported from AFDKO's `python/afdko/makeotf.py` 5.0.1 (Apache-2.0, see `NOTICE`): the option
//! words and their effects, the order in which project file, command line and `fontinfo` apply,
//! the project file syntax, and the FontMenuNameDB entry made from a UFO's names. The rest was
//! observed by running makeotf 5.0.1 on synthetic UFOs:
//!
//! - Without renaming, `.notdef` comes first, then the glyphs whose names are CFF standard
//!   strings in standard-string order, then the others in source order. With renaming the
//!   GlyphOrderAndAliasDB's order is used, and the glyphs it does not list follow in source order.
//! - The source's own code points are not used. A glyph gets the code points of its
//!   GlyphOrderAndAliasDB line (matched by source name, with or without renaming), else the one
//!   its written name has in [`names::NAME_CODES`], else the one a `uniXXXX` or `uXXXX` to
//!   `uXXXXXX` name spells (uppercase hex, no suffix), else none.
//! - Release mode subroutinises, renames and orders, and adds a stub `DSIG`; the version
//!   string is the same as in a development build.
//! - Menu names follow [`menu_records`].
//! - The source's OpenType values, kerning, anchors and most names are not read
//!   ([`Project::prepare`] leaves them out); the metrics, coordinates and segments follow
//!   [`metrics`] and [`Finish`].

pub mod metrics;
mod names;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::plist::Value;
use fontcraft_model::{DONT_USE_PRODUCTION_NAMES, Font, GlyphTable, Param, Params, Resolved};
use read_fonts::{FontRef, TableProvider};
use write_fonts::FontBuilder;
use write_fonts::types::{GlyphId, Tag};

use crate::{Compiled, Options};

/// Largest project file, FontMenuNameDB or `fontinfo` file read.
pub const MAX_PROJECT_BYTES: usize = 16 << 20;
/// Most lines one of those files may have.
pub const MAX_PROJECT_LINES: usize = 1_000_000;
/// Longest line of a FontMenuNameDB or project file, in bytes.
pub const MAX_PROJECT_LINE: usize = 4_096;
/// Most entries (PostScript names) of one FontMenuNameDB.
pub const MAX_MENU_ENTRIES: usize = 65_535;
/// Most names one FontMenuNameDB entry may give.
pub const MAX_MENU_NAMES: usize = 1_024;
/// Most option words one compile takes.
pub const MAX_OPTION_WORDS: usize = 1_000;
/// Longest family, style or compatible family name older menus show without cutting it.
pub const LEGACY_NAME_LENGTH: usize = 31;
/// Lowest weight class that older Windows applications do not draw smeared or embolden.
pub const LOWEST_SAFE_WEIGHT_CLASS: i64 = 250;
/// Most glyphs a warning names.
const MAX_LISTED: usize = 5;

/// A project file, option or name database the compile cannot use. Nothing is built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ProjectError(pub String);

type Result<T> = std::result::Result<T, ProjectError>;

fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(ProjectError(message.into()))
}

/// One GlyphOrderAndAliasDB line: the name in the compiled font, the name in the source, and
/// code points that replace the ones the name gives (empty: none given).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphAlias {
    pub final_name: String,
    pub source_name: String,
    pub code_points: Vec<u32>,
}

// ---------- settings and their order ----------

/// The settings the project workflow knows, for remembering which were set already.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Release,
    Subroutinize,
    Alias,
    Subset,
    Dsig,
    Bold,
    Italic,
    OmitMacNames,
    StubCmap4,
    DoubleMap,
    SuppressKernClass0,
}

/// Files a project file names, relative to the project file's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectPaths {
    pub input: Option<String>,
    pub output: Option<String>,
    pub features: Option<String>,
    pub menu_names: Option<String>,
    pub goadb: Option<String>,
}

/// What the project file, the option words and the `fontinfo` file ask for. Apply them in that
/// order: [`Settings::apply_project`], [`Settings::apply_option`] for each word left to right,
/// then [`Settings::apply_fontinfo`], which only fills what the others left unset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub release: bool,
    pub subroutinize: bool,
    /// Rename and order glyphs by the GlyphOrderAndAliasDB.
    pub apply_goadb: bool,
    /// Leave out the glyphs the GlyphOrderAndAliasDB does not list (with `apply_goadb`).
    pub subset_to_goadb: bool,
    pub stub_dsig: bool,
    pub bold: bool,
    pub italic: bool,
    pub omit_mac_names: bool,
    pub stub_cmap4: bool,
    pub double_map: bool,
    /// Accepted for compatibility: the pair positioning builder never puts non-zero values in
    /// first-class 0, so the output is the same either way.
    pub suppress_kern_class0: bool,
    pub paths: ProjectPaths,
    seen: BTreeSet<Key>,
}

/// The option words [`Settings::apply_option`] takes, as the command line spells them.
pub const OPTION_WORDS: &[&str] = &[
    "release",
    "no-release",
    "subroutinize",
    "no-subroutinize",
    "apply-goadb",
    "no-apply-goadb",
    "subset-to-goadb",
    "no-subset-to-goadb",
    "stub-dsig",
    "no-stub-dsig",
    "bold",
    "no-bold",
    "italic",
    "no-italic",
    "mac-names",
    "no-mac-names",
    "stub-cmap4",
    "legacy-double-mapping",
    "kern-class0-optimization",
    "no-kern-class0-optimization",
    "synthetic-glyphs",
];

impl Settings {
    fn set(&mut self, key: Key, on: bool) {
        self.seen.insert(key);
        let slot = match key {
            Key::Release => &mut self.release,
            Key::Subroutinize => &mut self.subroutinize,
            Key::Alias => &mut self.apply_goadb,
            Key::Subset => &mut self.subset_to_goadb,
            Key::Dsig => &mut self.stub_dsig,
            Key::Bold => &mut self.bold,
            Key::Italic => &mut self.italic,
            Key::OmitMacNames => &mut self.omit_mac_names,
            Key::StubCmap4 => &mut self.stub_cmap4,
            Key::DoubleMap => &mut self.double_map,
            Key::SuppressKernClass0 => &mut self.suppress_kern_class0,
        };
        *slot = on;
    }

    /// Applies one option word (with or without leading dashes). A later word overrides an
    /// earlier one, so `release` then `no-subroutinize` does not subroutinise while
    /// `no-subroutinize` then `release` does. Two quirks of the reference are kept: renaming
    /// takes the first `apply-goadb` or `no-apply-goadb` only (and none at all when the project
    /// file set it), and `no-release` turns off subroutinising, renaming and the stub `DSIG`.
    pub fn apply_option(&mut self, word: &str) -> Result<()> {
        let word = word.trim_start_matches('-');
        match word {
            "release" => {
                self.set(Key::Release, true);
                // As the reference does: the side effects do not count as set, so a later
                // `apply-goadb` or `no-apply-goadb` still applies.
                self.subroutinize = true;
                self.apply_goadb = true;
                self.stub_dsig = true;
            }
            "no-release" => {
                self.set(Key::Release, false);
                self.set(Key::Subroutinize, false);
                self.set(Key::Alias, false);
                self.set(Key::Dsig, false);
            }
            "subroutinize" | "no-subroutinize" => self.set(Key::Subroutinize, word == "subroutinize"),
            "apply-goadb" | "no-apply-goadb" => {
                if !self.seen.contains(&Key::Alias) {
                    self.set(Key::Alias, word == "apply-goadb");
                }
            }
            "subset-to-goadb" => self.set(Key::Subset, true),
            "no-subset-to-goadb" => self.subset_to_goadb = false,
            "stub-dsig" | "no-stub-dsig" => self.set(Key::Dsig, word == "stub-dsig"),
            "bold" | "no-bold" => self.set(Key::Bold, word == "bold"),
            "italic" | "no-italic" => self.set(Key::Italic, word == "italic"),
            "mac-names" | "no-mac-names" => self.set(Key::OmitMacNames, word == "no-mac-names"),
            "stub-cmap4" => self.set(Key::StubCmap4, true),
            "legacy-double-mapping" => self.set(Key::DoubleMap, true),
            "kern-class0-optimization" | "no-kern-class0-optimization" => {
                self.set(Key::SuppressKernClass0, word == "no-kern-class0-optimization");
            }
            "synthetic-glyphs" => {
                return err(
                    "glyphs are not synthesised at compile time: draw the missing glyphs (the Euro and the other symbols) in the source instead",
                );
            }
            other => return err(format!("`{other}` is not a project compile option; the options are {}", OPTION_WORDS.join(", "))),
        }
        Ok(())
    }

    /// Applies a project file: one `Key value` per line, `#` lines skipped, in file order. A
    /// key the reference knows but this compile does not use is reported in the returned list;
    /// an unknown key or a value that is not a yes or no where one is needed fails.
    pub fn apply_project(&mut self, text: &str) -> Result<Vec<String>> {
        check_size(text, "the project file")?;
        let mut unused = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            if raw.len() > MAX_PROJECT_LINE {
                return err(format!("project file line {line_no} is longer than {MAX_PROJECT_LINE} bytes"));
            }
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = match line.split_once(|c: char| c.is_whitespace()) {
                Some((k, v)) => (k, v.trim()),
                None => return err(format!("project file line {line_no}: `{line}` has no value")),
            };
            let path = |slot: &mut Option<String>| *slot = Some(value.to_string());
            let flag = match key {
                "InputFontPath" => {
                    path(&mut self.paths.input);
                    continue;
                }
                "OutputFontPath" => {
                    path(&mut self.paths.output);
                    continue;
                }
                "FeaturePath" => {
                    path(&mut self.paths.features);
                    continue;
                }
                "FontMenuDBPath" => {
                    path(&mut self.paths.menu_names);
                    continue;
                }
                "GlyphAliasDBPath" => {
                    path(&mut self.paths.goadb);
                    continue;
                }
                "ReleaseMode" => Key::Release,
                "DoSubroutinization" => Key::Subroutinize,
                "DoAliasAndOrder" => Key::Alias,
                "DoSubsetByGOADB" => Key::Subset,
                "AddStubDSIG" => Key::Dsig,
                "IsBoldStyle" => Key::Bold,
                "IsItalicStyle" => Key::Italic,
                "OmitMacNames" => Key::OmitMacNames,
                "StubCmap4" => Key::StubCmap4,
                // The file format spells this key with the letters swapped.
                "DoulbeMapGlyphs" | "DoubleMapGlyphs" => Key::DoubleMap,
                "SuppressKernOptimization" => Key::SuppressKernClass0,
                k if KNOWN_UNUSED_KEYS.contains(&k) => {
                    unused.push(format!("the project file setting {k} is not used by this compile"));
                    continue;
                }
                k => return err(format!("project file line {line_no}: `{k}` is not a project file setting")),
            };
            let on = match value {
                "true" | "True" | "1" => true,
                "false" | "False" | "0" | "None" => false,
                v => return err(format!("project file line {line_no}: {key} is `{v}`; it takes true or false")),
            };
            self.set(flag, on);
        }
        Ok(unused)
    }

    /// Applies the keywords of a `fontinfo` file that nothing else set: `IsBoldStyle`,
    /// `IsWindowsBold`, `IsItalicStyle`, `OmitMacNames`, `SuppressKernOptimization` and
    /// `AddStubDSIG`, each followed by `true` (or `True`, or `1`).
    pub fn apply_fontinfo(&mut self, text: &str) -> Result<()> {
        check_size(text, "the fontinfo file")?;
        let unset = |s: &Self, k: Key| !s.seen.contains(&k);
        if unset(self, Key::Bold) && (keyword_on(text, "IsBoldStyle", true) || keyword_on(text, "IsWindowsBold", true)) {
            self.bold = true;
        }
        if unset(self, Key::Italic) && keyword_on(text, "IsItalicStyle", true) {
            self.italic = true;
        }
        // The reference matches these three with the value straight after the keyword's last
        // letter (`OmitMacNamestrue`), so a space before the value turns them off; kept as it is.
        if unset(self, Key::OmitMacNames) && keyword_on(text, "OmitMacNames", false) {
            self.omit_mac_names = true;
        }
        if unset(self, Key::SuppressKernClass0) && keyword_on(text, "SuppressKernOptimization", false) {
            self.suppress_kern_class0 = true;
        }
        if unset(self, Key::Dsig) && keyword_on(text, "AddStubDSIG", false) {
            self.stub_dsig = true;
        }
        Ok(())
    }
}

/// Project file settings of the reference that this compile reads but does not act on.
const KNOWN_UNUSED_KEYS: &[&str] = &[
    "AllowStubGSUB",
    "DesignspacePath",
    "AllowMissingNotdef",
    "UseSuppliedCFF",
    "Renumber",
    "MaxSubrs",
    "kSetfsSelectionBitsOn",
    "kSetfsSelectionBitsOff",
    "SetOS2Version",
    "XUID",
    "ConvertToCID",
    "SuppressHintWarnings",
    "SuppressWidthOptimization",
    "MacCmapScriptID",
    "MacCmapLanguageID",
    "MacEncodingCMAPPath",
    "H_UniEncodingCMAPPath",
    "V_UniEncodingCMAPPath",
    "UVSPath",
    "OverrideMenuNames",
    "LicenseCode",
    "ShowFinalNames",
    "VerboseWarnings",
    "MVAR",
];

fn check_size(text: &str, what: &str) -> Result<()> {
    if text.len() > MAX_PROJECT_BYTES {
        return err(format!("{what} is larger than {MAX_PROJECT_BYTES} bytes"));
    }
    if text.lines().nth(MAX_PROJECT_LINES).is_some() {
        return err(format!("{what} has more than {MAX_PROJECT_LINES} lines"));
    }
    Ok(())
}

/// Whether `keyword` occurs followed by `true`, `True` or `1`: after one or more white-space
/// characters when `spaced`, else straight after the keyword (more `s` letters allowed, as the
/// reference's pattern allows).
fn keyword_on(text: &str, keyword: &str, spaced: bool) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find(keyword) {
        let after = rest.get(at + keyword.len()..).unwrap_or_default();
        let value = if spaced {
            let trimmed = after.trim_start();
            (trimmed.len() < after.len()).then_some(trimmed)
        } else {
            Some(after.trim_start_matches('s'))
        };
        if value.is_some_and(|v| v.starts_with("true") || v.starts_with("True") || v.starts_with('1')) {
            return true;
        }
        rest = after;
    }
    false
}

// ---------- FontMenuNameDB ----------

/// One name of a FontMenuNameDB entry. `explicit` is false for a name given without a
/// platform, which serves Windows English and, through the Macintosh copies, Mac English.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuName {
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    pub text: String,
    pub explicit: bool,
}

/// The names of one PostScript name in a FontMenuNameDB.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuEntry {
    /// `f=`: the preferred family.
    pub family: Vec<MenuName>,
    /// `s=`: the style.
    pub style: Vec<MenuName>,
    /// `l=`: the compatible (style-linked) family.
    pub compatible: Vec<MenuName>,
    /// `m=1,`: the Macintosh compatible full name.
    pub mac_full: Option<String>,
    /// The entry used the older `c=` key, whose naming scheme is not carried over.
    pub old_scheme: bool,
}

/// A FontMenuNameDB: entries by PostScript name, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuDb {
    pub entries: Vec<(String, MenuEntry)>,
}

impl MenuDb {
    /// The entry of `postscript_name`.
    pub fn entry(&self, postscript_name: &str) -> Option<&MenuEntry> {
        self.entries.iter().find(|(n, _)| n == postscript_name).map(|(_, e)| e)
    }
}

/// Parses a FontMenuNameDB: `[PostScriptName]` lines start an entry, `key=value` lines fill it,
/// `#` lines are comments. A value may start with `platform,` or `platform,script,language,`
/// (Windows 3 or Macintosh 1, numbers in decimal or `0x` hex) and may hold `\XXXX` (Windows,
/// UTF-16 hex) or `\XX` (Macintosh, Mac Roman hex) escapes.
pub fn parse_menu_db(text: &str) -> Result<MenuDb> {
    check_size(text, "the FontMenuNameDB")?;
    let mut db = MenuDb::default();
    let mut seen = HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        if raw.len() > MAX_PROJECT_LINE {
            return err(format!("FontMenuNameDB line {line_no} is longer than {MAX_PROJECT_LINE} bytes"));
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            let Some(name) = name.strip_suffix(']').map(str::trim).filter(|n| !n.is_empty()) else {
                return err(format!("FontMenuNameDB line {line_no}: `{line}` is not a `[PostScriptName]` line"));
            };
            if db.entries.len() >= MAX_MENU_ENTRIES {
                return err(format!("the FontMenuNameDB has more than {MAX_MENU_ENTRIES} entries"));
            }
            if !seen.insert(name.to_string()) {
                return err(format!("FontMenuNameDB line {line_no}: [{name}] is given twice"));
            }
            db.entries.push((name.to_string(), MenuEntry::default()));
            continue;
        }
        let Some((_, entry)) = db.entries.last_mut() else {
            return err(format!("FontMenuNameDB line {line_no}: `{line}` comes before any `[PostScriptName]` line"));
        };
        let Some((key, value)) = line.split_once('=') else {
            return err(format!("FontMenuNameDB line {line_no}: `{line}` is not `key=value`"));
        };
        let count = entry.family.len() + entry.style.len() + entry.compatible.len();
        if count >= MAX_MENU_NAMES {
            return err(format!("FontMenuNameDB line {line_no}: the entry has more than {MAX_MENU_NAMES} names"));
        }
        let at = |e: String| ProjectError(format!("FontMenuNameDB line {line_no}: {e}"));
        match key.trim() {
            "f" => entry.family.push(menu_name(value).map_err(at)?),
            "s" => entry.style.push(menu_name(value).map_err(at)?),
            "l" => entry.compatible.push(menu_name(value).map_err(at)?),
            "m" => {
                let Some(text) = value.strip_prefix("1,") else {
                    return err(format!("FontMenuNameDB line {line_no}: m= starts with `1,` (the Macintosh platform)"));
                };
                entry.mac_full = Some(unescape(text, 1).map_err(at)?);
            }
            "c" => {
                entry.old_scheme = true;
                entry.compatible.push(menu_name(value).map_err(at)?);
            }
            other => return err(format!("FontMenuNameDB line {line_no}: `{other}=` is not one of f=, s=, l=, m= and c=")),
        }
    }
    Ok(db)
}

fn number(s: &str) -> Option<u16> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// `[platform,[script,language,]]text`.
fn menu_name(value: &str) -> std::result::Result<MenuName, String> {
    let parts: Vec<&str> = value.splitn(4, ',').collect();
    let (platform, encoding, language, text) = match parts.as_slice() {
        [p, e, l, text] if matches!(number(p), Some(1 | 3)) && number(e).is_some() && number(l).is_some() => {
            (number(p).unwrap_or(3), number(e).unwrap_or(1), number(l).unwrap_or(0x409), *text)
        }
        [p, ..] if matches!(number(p), Some(1 | 3)) => {
            let text = value.split_once(',').map_or("", |(_, t)| t);
            match number(p) {
                Some(1) => (1, 0, 0, text),
                _ => (3, 1, 0x409, text),
            }
        }
        _ => {
            return Ok(MenuName { platform: 3, encoding: 1, language: 0x409, text: unescape(value, 3)?, explicit: false });
        }
    };
    Ok(MenuName { platform, encoding, language, text: unescape(text, platform)?, explicit: true })
}

/// The Mac Roman characters 0x80 to 0xFF.
const MAC_ROMAN_HIGH: &str =
    "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{a0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

/// Resolves `\XXXX` (Windows: UTF-16 code units) or `\XX` (Macintosh: Mac Roman bytes).
fn unescape(text: &str, platform: u16) -> std::result::Result<String, String> {
    let width = if platform == 1 { 2 } else { 4 };
    let mut units: Vec<u16> = Vec::new();
    let mut out = String::new();
    let mut rest = text;
    let flush = |units: &mut Vec<u16>, out: &mut String| -> std::result::Result<(), String> {
        if !units.is_empty() {
            out.push_str(&String::from_utf16(units).map_err(|_| "a `\\` escape is not valid UTF-16".to_string())?);
            units.clear();
        }
        Ok(())
    };
    while let Some(at) = rest.find('\\') {
        let (before, after) = (rest.get(..at).unwrap_or_default(), rest.get(at + 1..).unwrap_or_default());
        if !before.is_empty() {
            flush(&mut units, &mut out)?;
            out.push_str(before);
        }
        let hex = after.get(..width).filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
        let Some(value) = hex.and_then(|h| u16::from_str_radix(h, 16).ok()) else {
            return Err(format!("`\\` is followed by {width} hex digits in a {} name", if platform == 1 { "Macintosh" } else { "Windows" }));
        };
        if platform == 1 {
            flush(&mut units, &mut out)?;
            let c = match value {
                0..=0x7F => char::from_u32(u32::from(value)),
                _ => MAC_ROMAN_HIGH.chars().nth(usize::from(value) - 0x80),
            };
            out.push(c.ok_or_else(|| format!("`\\{value:02X}` is not a Mac Roman character"))?);
        } else {
            units.push(value);
        }
        rest = after.get(width..).unwrap_or_default();
    }
    flush(&mut units, &mut out)?;
    out.push_str(rest);
    Ok(out)
}

/// A name record the menu names ask for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MenuRecord {
    pub id: u16,
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    pub text: String,
}

/// The style-link name: Regular, Bold, Italic or Bold Italic.
pub fn ribbi(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

/// The name records an entry gives, as the reference writes them. For each language that the
/// family or compatible family names (Windows English always): ID 1 is the compatible family
/// when there is one that differs from the family, else the family, which then goes to ID 16;
/// ID 2 is the style-link name; ID 4 is ID 1 followed by the style-link name unless that is
/// Regular; ID 17 is the style when it differs from the style-link name. `m=` gives the
/// Macintosh ID 18. The Macintosh English copies of the rest are the compile's to make.
pub fn menu_records(entry: &MenuEntry, bold: bool, italic: bool) -> Vec<MenuRecord> {
    let link = ribbi(bold, italic);
    let english = (3u16, 1u16, 0x409u16);
    let slot = |n: &MenuName| (n.platform, n.encoding, n.language);
    let find = |names: &[MenuName], s: (u16, u16, u16)| -> Option<String> {
        names
            .iter()
            .rev()
            .find(|n| n.explicit && slot(n) == s)
            .or_else(|| if s == english { names.iter().rev().find(|n| !n.explicit) } else { None })
            .map(|n| n.text.clone())
    };
    let mut slots: Vec<(u16, u16, u16)> = vec![english];
    for n in entry.family.iter().chain(&entry.compatible).filter(|n| n.explicit) {
        if !slots.contains(&slot(n)) {
            slots.push(slot(n));
        }
    }
    let mut out = Vec::new();
    for s in slots {
        let family = find(&entry.family, s).or_else(|| find(&entry.family, english));
        let Some(family) = family else { continue };
        let compatible = find(&entry.compatible, s).filter(|c| *c != family);
        let id1 = compatible.clone().unwrap_or_else(|| family.clone());
        let mut add = |id: u16, text: String| out.push(MenuRecord { id, platform: s.0, encoding: s.1, language: s.2, text });
        add(1, id1.clone());
        add(2, link.to_string());
        add(4, if link == "Regular" { id1 } else { format!("{id1} {link}") });
        if compatible.is_some() {
            add(16, family);
        }
        if let Some(style) = find(&entry.style, s).filter(|st| st != link) {
            add(17, style);
        }
    }
    if let Some(m) = &entry.mac_full {
        out.push(MenuRecord { id: 18, platform: 1, encoding: 0, language: 0, text: m.clone() });
    }
    out
}

// ---------- the source ----------

/// Everything a project compile reads, parsed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub settings: Settings,
    pub menu_db: Option<MenuDb>,
    pub aliases: Option<Vec<GlyphAlias>>,
}

/// What a project compile changes in the build: [`finish`] adds the stub tables, the compile
/// writes the outlines and metrics as the reference does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Finish {
    pub stub_dsig: bool,
    pub stub_cmap4: bool,
    /// Charstring coordinates kept to two decimals and every segment kept, as the reference's
    /// CFF converter writes them, and glyph boxes rounded by [`metrics::int_box`]; otherwise
    /// every coordinate is rounded to an integer and the specialiser drops redundant segments.
    pub two_decimals: bool,
    /// The weight, width, style bits and vertical metrics worked out by [`metrics::apply`]
    /// instead of read from the source.
    pub metrics: Option<metrics::Source>,
}

/// The source as the project compile builds it, with what the compile needs to know.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub font: Font,
    /// Whether CFF charstrings are subroutinised.
    pub subroutinize: bool,
    /// For [`Options::project`].
    pub finish: Finish,
    pub warnings: Vec<String>,
}

/// The second code point the legacy double mapping gives a glyph of this name, when nothing else
/// maps it. Observed with the reference's `-dbl` option.
const DOUBLE_MAPS: [(&str, u32); 10] = [
    ("Delta", 0x0394),
    ("Omega", 0x03A9),
    ("mu", 0x03BC),
    ("hyphen", 0x00AD),
    ("space", 0x00A0),
    ("macron", 0x02C9),
    ("periodcentered", 0x2219),
    ("fraction", 0x2215),
    ("Tcommaaccent", 0x021A),
    ("tcommaaccent", 0x021B),
];

/// The code point a glyph name gives: the reference's glyph list, else `uniXXXX`, else `uXXXX`
/// to `uXXXXXX`, all in uppercase hex with nothing after them.
pub fn name_code_point(name: &str) -> Option<u32> {
    if let Ok(i) = names::NAME_CODES.binary_search_by(|(n, _)| n.as_bytes().cmp(name.as_bytes())) {
        return names::NAME_CODES.get(i).map(|(_, c)| *c);
    }
    let upper_hex = |h: &str| h.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b));
    let hex = if let Some(h) = name.strip_prefix("uni") {
        (h.len() == 4 && upper_hex(h)).then_some(h)
    } else if let Some(h) = name.strip_prefix('u') {
        ((4..=6).contains(&h.len()) && upper_hex(h)).then_some(h)
    } else {
        None
    }?;
    u32::from_str_radix(hex, 16).ok().filter(|v| char::from_u32(*v).is_some())
}

/// The text of a name property, else of one of the parameters that may hold it.
fn property_text(resolved: &Resolved, property: &str, params: &[&str]) -> Option<String> {
    resolved
        .properties
        .value(property)
        .map(str::to_string)
        .or_else(|| params.iter().find_map(|p| resolved.params.get(p).and_then(Value::as_str).map(str::to_string)))
        .filter(|s| !s.trim().is_empty())
}

/// Sets `name` to `value` in every master's parameters, which take the place of the font's.
fn set_master_param(font: &mut Font, name: &str, value: Value) {
    for m in Arc::make_mut(&mut font.masters).iter_mut() {
        m.params.set(name, value.clone());
    }
}

/// Adds a repeating parameter to every master.
fn push_master_param(font: &mut Font, name: &str, value: Value) {
    for m in Arc::make_mut(&mut font.masters).iter_mut() {
        m.params.0.push(Param::new(name, value.clone()));
    }
}

fn listed(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(MAX_LISTED).map(String::as_str).collect();
    format!("{}{}", shown.join(", "), if names.len() > MAX_LISTED { ", …" } else { "" })
}

/// The name IDs the reference writes from the source: the family, style, full and PostScript
/// names, and the typographic family and style, which the menu names decide. It makes its own
/// unique ID and version string, and writes no other name the feature code does not give.
const NAMES_READ: [u16; 6] = [1, 2, 4, 6, 16, 17];

/// Takes out of `font` what the reference compiler does not read from a source, so the ordinary
/// compile does not write it: the name records other than [`NAMES_READ`] (properties, their
/// parameters, the source's own name table entries and prevented IDs), the version (it writes
/// 1.000 unless the feature code's `head` table sets a FontRevision), the kerning and the
/// anchors (only the feature code makes `GPOS`).
fn leave_out_unread(font: &mut Font) {
    let unread: Vec<(&str, &[&str])> =
        crate::info::NAME_PROPERTIES.iter().filter(|(_, id, _, _)| !NAMES_READ.contains(id)).map(|(key, _, params, _)| (*key, *params)).collect();
    let strip = |properties: &mut fontcraft_model::Properties, params: &mut Params| {
        for (key, names) in &unread {
            properties.remove(key);
            for name in *names {
                params.remove(name);
            }
        }
        params.remove("Name Table Entry");
        params.remove("Prevent Name ID");
    };
    let info = Arc::make_mut(&mut font.info);
    strip(&mut info.properties, &mut info.params);
    for m in Arc::make_mut(&mut font.masters).iter_mut() {
        strip(&mut m.properties, &mut m.params);
    }
    for i in Arc::make_mut(&mut font.instances).iter_mut() {
        strip(&mut i.properties, &mut i.params);
    }
    font.version_major = 1;
    font.version_minor = 0;
    font.kerning = Arc::default();
    for i in 0..font.glyphs.len() {
        if font.glyphs.as_arcs().get(i).is_some_and(|g| g.layers.iter().any(|l| !l.anchors.is_empty()))
            && let Some(mut g) = font.glyphs.get_mut_at(i)
        {
            g.layers.iter_mut().for_each(|l| l.anchors.clear());
        }
    }
}

/// The weight class the source states, from its parameters.
fn weight_class(params: &Params) -> Option<i64> {
    ["openTypeOS2WeightClass", "weightClass"].iter().find_map(|k| params.get(k).and_then(crate::info::int))
}

impl Project {
    /// The source as the project compile builds it: glyph names, order and code points, the
    /// menu names, and the switches the compile reads from parameters. `font` is not changed.
    pub fn prepare(&self, font: &Font) -> Result<Prepared> {
        let s = &self.settings;
        let mut font = font.clone();
        let mut warnings = Vec::new();
        let rename = s.apply_goadb && self.aliases.is_some();
        if s.apply_goadb && self.aliases.is_none() {
            warnings.push(
                "renaming and ordering by a GlyphOrderAndAliasDB was asked for, but none was given: the source's names and order are kept".into(),
            );
        }
        if s.subset_to_goadb && !rename {
            warnings.push("leaving out the glyphs a GlyphOrderAndAliasDB does not list needs the glyphs renamed by it; every glyph is kept".into());
        }
        let aliases: &[GlyphAlias] = self.aliases.as_deref().unwrap_or(&[]);
        let by_source: HashMap<&str, &GlyphAlias> = aliases.iter().map(|a| (a.source_name.as_str(), a)).collect();

        // The name each glyph is written under, and its code points.
        let written = |name: &str| -> String {
            match by_source.get(name) {
                Some(a) if rename => a.final_name.clone(),
                _ => name.to_string(),
            }
        };
        // Glyphs the font leaves out: not exported, or not listed when subsetting.
        let dropped = |name: &str, export: bool| !export || (rename && s.subset_to_goadb && name != ".notdef" && !by_source.contains_key(name));
        let mut replaced = Vec::new();
        let mut taken: HashMap<u32, String> = HashMap::new();
        let mut codes: Vec<(String, Vec<u32>)> = Vec::with_capacity(font.glyphs.len());
        for g in font.glyphs.iter() {
            if dropped(&g.name, g.export) {
                continue;
            }
            let out = written(&g.name);
            let mut cps = match by_source.get(g.name.as_str()) {
                Some(a) if !a.code_points.is_empty() => a.code_points.clone(),
                _ => name_code_point(&out).into_iter().collect(),
            };
            cps.dedup();
            for cp in &cps {
                if let Some(other) = taken.insert(*cp, g.name.clone()) {
                    return err(format!(
                        "U+{cp:04X} would be mapped to both `{other}` and `{}`: give one of them other code points in the GlyphOrderAndAliasDB",
                        g.name
                    ));
                }
            }
            if !g.unicodes.is_empty() && g.unicodes != cps {
                replaced.push(g.name.clone());
            }
            codes.push((g.name.clone(), cps));
        }
        if s.double_map {
            for (name, cps) in codes.iter_mut() {
                let out = written(name);
                if let Some((_, extra)) = DOUBLE_MAPS.iter().find(|(n, _)| *n == out)
                    && !cps.is_empty()
                    && !taken.contains_key(extra)
                {
                    taken.insert(*extra, name.clone());
                    cps.push(*extra);
                }
            }
        }
        if !replaced.is_empty() {
            warnings.push(format!(
                "{} glyphs get the code points their names and the GlyphOrderAndAliasDB give, not the source's: {}",
                replaced.len(),
                listed(&replaced)
            ));
        }
        for (name, cps) in codes {
            if let Some(mut g) = font.glyphs.get_mut(&name) {
                g.unicodes = cps;
            }
        }

        // Order.
        let position: HashMap<&str, usize> = if rename {
            aliases.iter().enumerate().map(|(i, a)| (a.source_name.as_str(), i)).collect()
        } else {
            read_fonts::ps::string::STANDARD_STRINGS.iter().enumerate().map(|(i, n)| (*n, i)).collect()
        };
        let mut glyphs: Vec<(usize, Arc<fontcraft_model::Glyph>)> = font.glyphs.as_arcs().iter().cloned().enumerate().collect();
        // A stable sort: listed glyphs by their place, the rest after them in source order.
        glyphs.sort_by_key(|(i, g)| match (g.name.as_str(), position.get(g.name.as_str())) {
            (".notdef", _) => (0, 0, *i),
            (_, Some(p)) => (1, *p, *i),
            (_, None) => (2, 0, *i),
        });
        font.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(|(_, g)| g).collect());

        // Names written and glyphs left out.
        if rename {
            let mut dropped = Vec::new();
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                let out = written(&name);
                let keep = name == ".notdef" || by_source.contains_key(name.as_str());
                if let Some(mut g) = font.glyphs.get_mut(&name) {
                    g.info.production = Some(out);
                    if s.subset_to_goadb && !keep && g.export {
                        g.export = false;
                        dropped.push(name.clone());
                    }
                }
            }
            if !dropped.is_empty() {
                warnings.push(format!("{} glyphs the GlyphOrderAndAliasDB does not list are left out: {}", dropped.len(), listed(&dropped)));
            }
            set_master_param(&mut font, DONT_USE_PRODUCTION_NAMES, Value::Bool(false));
            Arc::make_mut(&mut font.info).settings.disables_nice_names = None;
        } else {
            set_master_param(&mut font, DONT_USE_PRODUCTION_NAMES, Value::Bool(true));
        }

        leave_out_unread(&mut font);

        // Menu names.
        let target = font.default_target().map_err(|e| ProjectError(e.to_string()))?;
        let resolved = crate::drawing::resolve(&font, &target).map_err(|e| ProjectError(e.to_string()))?;
        let full_name = property_text(&resolved, "postscriptFullNames", &["postscriptFullName"]);
        let postscript = crate::plan::postscript_name(&resolved, &Options::default()).unwrap_or_default();
        let made;
        let entry = match self.menu_db.as_ref().and_then(|db| db.entry(&postscript)) {
            Some(e) => e,
            None => {
                if self.menu_db.is_some() {
                    warnings.push(format!(
                        "the FontMenuNameDB has no [{postscript}] entry: the menu names come from the source's family and style names"
                    ));
                }
                let family = property_text(&resolved, "preferredFamilyNames", &["openTypeNamePreferredFamilyName"])
                    .unwrap_or_else(|| resolved.family_name.clone());
                let style = property_text(&resolved, "preferredSubfamilyNames", &["openTypeNamePreferredSubfamilyName"])
                    .unwrap_or_else(|| resolved.style_name.clone());
                let plain = |text: String| vec![MenuName { platform: 3, encoding: 1, language: 0x409, text, explicit: false }];
                made = MenuEntry { family: plain(family), style: plain(style), ..MenuEntry::default() };
                &made
            }
        };
        if entry.old_scheme {
            warnings.push(format!("the [{postscript}] entry uses the older c= key; its name is taken as the compatible family name"));
        }
        for (what, names) in [("family", &entry.family), ("style", &entry.style), ("compatible family", &entry.compatible)] {
            if let Some(long) = names.iter().find(|n| n.text.chars().count() > LEGACY_NAME_LENGTH) {
                warnings.push(format!("the {what} name `{}` is longer than {LEGACY_NAME_LENGTH} characters, which older menus cut", long.text));
            }
        }
        let records = menu_records(entry, s.bold, s.italic);
        let english = |id: u16| records.iter().find(|r| r.id == id && r.platform == 3 && r.language == 0x409).map(|r| r.text.clone());
        if let Some(id1) = english(1) {
            set_master_param(&mut font, "styleMapFamilyName", Value::String(id1.as_str().into()));
        }
        set_master_param(&mut font, "styleMapStyleName", Value::String(ribbi(s.bold, s.italic).to_lowercase().as_str().into()));
        for r in &records {
            if r.platform == 1 && s.omit_mac_names {
                continue;
            }
            let entry = format!("{} {} {} 0x{:X}; {}", r.id, r.platform, r.encoding, r.language, r.text);
            push_master_param(&mut font, "Name Table Entry", Value::String(entry.as_str().into()));
        }
        for id in [16u16, 17] {
            if !records.iter().any(|r| r.id == id) {
                push_master_param(&mut font, "Prevent Name ID", Value::Number(fontcraft_model::plist::Number::int(i64::from(id))));
            }
        }
        set_master_param(&mut font, "Export Mac Name Table Entries", Value::Bool(!s.omit_mac_names));

        let stated = weight_class(&resolved.params);
        if let Some(w) = stated.filter(|w| *w < LOWEST_SAFE_WEIGHT_CLASS) {
            warnings.push(format!(
                "the weight class is {w}: older Windows applications may draw weights below {LOWEST_SAFE_WEIGHT_CLASS} smeared or emboldened; use {LOWEST_SAFE_WEIGHT_CLASS} or more"
            ));
        }
        let (computed, _) = metrics::classes(full_name.as_deref());
        if let Some(w) = stated.filter(|w| *w != i64::from(computed)) {
            warnings.push(format!(
                "the source's weight class {w} is not used: a project compile takes it from the full name ({computed}); a `table OS/2 {{ WeightClass … }}` block in the feature code sets it"
            ));
        }
        let finish = Finish {
            stub_dsig: s.stub_dsig,
            stub_cmap4: s.stub_cmap4,
            two_decimals: true,
            metrics: Some(metrics::Source { full_name, bold: s.bold, italic: s.italic }),
        };
        Ok(Prepared { font, subroutinize: s.subroutinize, finish, warnings })
    }
}

// ---------- the compiled font ----------

/// The 8-byte `DSIG` of a release build: version 1, no signatures, flags 1.
pub const STUB_DSIG: [u8; 8] = [0, 0, 0, 1, 0, 0, 0, 1];

/// Adds a stub `DSIG` and replaces `cmap` by a stub, as `f` asks. Every other table is copied as
/// it is.
pub fn finish(compiled: &mut Compiled, f: &Finish) -> std::result::Result<(), crate::CompileError> {
    if !f.stub_dsig && !f.stub_cmap4 {
        return Ok(());
    }
    let table_error = |e: String| crate::CompileError::Table { table: "cmap", message: e };
    let font = FontRef::new(&compiled.bytes).map_err(|e| table_error(e.to_string()))?;
    let mut builder = FontBuilder::new();
    let dsig = Tag::new(b"DSIG");
    let cmap = Tag::new(b"cmap");
    for record in font.table_directory().table_records() {
        let tag = record.tag();
        if (tag == dsig && f.stub_dsig) || (tag == cmap && f.stub_cmap4) {
            continue;
        }
        if let Some(data) = font.table_data(tag) {
            builder.add_raw(tag, data.as_bytes().to_vec());
        }
    }
    if f.stub_cmap4 {
        builder.add_raw(cmap, stub_cmap(&font, &mut compiled.warnings).map_err(table_error)?);
    }
    if f.stub_dsig {
        builder.add_raw(dsig, STUB_DSIG.to_vec());
    }
    compiled.bytes = builder.build();
    Ok(())
}

/// A `cmap` with only the space mapping in its two format 4 subtables (the other BMP mapping
/// with the lowest code point when the font has no space), as the reference's stub option
/// writes it.
fn stub_cmap(font: &FontRef, warnings: &mut Vec<String>) -> std::result::Result<Vec<u8>, String> {
    use write_fonts::tables::cmap::Cmap;
    let charmap = font.cmap().map_err(|e| e.to_string())?;
    let mut mappings: Vec<(u32, GlyphId)> = Vec::new();
    for record in charmap.encoding_records() {
        if let Ok(subtable) = record.subtable(charmap.offset_data()) {
            mappings.extend(subtable.iter().take(crate::MAX_FONT_GLYPHS * 4).filter(|(c, _)| *c <= 0xFFFF));
        }
        if mappings.len() > crate::MAX_FONT_GLYPHS * 4 {
            break;
        }
    }
    mappings.sort_by_key(|(c, _)| *c);
    let kept = mappings.iter().find(|(c, _)| *c == 0x20).or_else(|| mappings.first()).copied();
    let Some((c, gid)) = kept else {
        warnings.push("the font maps no character, so the stub cmap is empty".into());
        let cmap = Cmap::from_mappings(std::iter::empty::<(char, GlyphId)>()).map_err(|e| e.to_string())?;
        return write_fonts::dump_table(&cmap).map_err(|e| e.to_string());
    };
    let ch = char::from_u32(c).ok_or_else(|| format!("U+{c:04X} is not a character"))?;
    let cmap = Cmap::from_mappings([(ch, gid)]).map_err(|e| e.to_string())?;
    write_fonts::dump_table(&cmap).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn later_options_override_earlier_ones_with_the_reference_quirks() {
        let run = |words: &[&str]| {
            let mut s = Settings::default();
            for w in words {
                s.apply_option(w).unwrap();
            }
            s
        };
        let a = run(&["release", "no-subroutinize"]);
        assert!(a.release && !a.subroutinize && a.apply_goadb && a.stub_dsig);
        let b = run(&["no-subroutinize", "--release"]);
        assert!(b.subroutinize, "release after no-subroutinize subroutinises");
        assert!(!run(&["release", "no-apply-goadb"]).apply_goadb);
        assert!(run(&["apply-goadb", "no-apply-goadb"]).apply_goadb, "the first renaming word wins");
        let c = run(&["release", "no-release"]);
        assert!(!c.release && !c.subroutinize && !c.apply_goadb && !c.stub_dsig);
        let e = Settings::default().apply_option("synthetic-glyphs").unwrap_err().to_string();
        assert!(e.contains("not synthesised"), "{e}");
        assert!(Settings::default().apply_option("adds").is_err());
    }

    #[test]
    fn the_project_file_comes_first_and_fontinfo_fills_only_what_is_unset() {
        let mut s = Settings::default();
        let unused = s
            .apply_project("# saved\nInputFontPath\tfont.ufo\nDoAliasAndOrder true\nDoSubroutinization true\nMaxSubrs 100\nIsItalicStyle false\n")
            .unwrap();
        assert_eq!(s.paths.input.as_deref(), Some("font.ufo"));
        assert_eq!(unused.len(), 1);
        s.apply_option("no-apply-goadb").unwrap();
        assert!(s.apply_goadb, "a renaming word after the project file's setting has no effect");
        s.apply_option("no-subroutinize").unwrap();
        assert!(!s.subroutinize);
        s.apply_fontinfo("IsBoldStyle true\nIsItalicStyle true\nOmitMacNames true\nAddStubDSIGtrue\n").unwrap();
        assert!(s.bold, "bold was unset");
        assert!(!s.italic, "the project file set italic");
        assert!(!s.omit_mac_names, "a space before the value does not match, as in the reference");
        assert!(s.stub_dsig);
        assert!(Settings::default().apply_project("NoSuchKey 1\n").is_err());
        assert!(Settings::default().apply_project("ReleaseMode maybe\n").is_err());
        assert!(Settings::default().apply_project("ReleaseMode\n").is_err());
    }

    #[test]
    fn names_give_code_points_as_the_reference_maps_them() {
        assert_eq!(name_code_point("A"), Some(0x41));
        assert_eq!(name_code_point("mu"), Some(0xB5));
        assert_eq!(name_code_point("Delta"), Some(0x2206));
        assert_eq!(name_code_point("afii10017"), Some(0x0410));
        assert_eq!(name_code_point("parenlefttp"), Some(0x239B));
        assert_eq!(name_code_point("uni00E9"), Some(0xE9));
        assert_eq!(name_code_point("uniE000"), Some(0xE000));
        assert_eq!(name_code_point("u1F600"), Some(0x1F600));
        for none in ["Acaron", "uni00e9", "uni0041.alt", "a.sc", "f_f", "uni00410042", "uni004", "uD800", "nbspace"] {
            assert_eq!(name_code_point(none), None, "{none}");
        }
        assert!(names::NAME_CODES.windows(2).all(|w| w[0].0.as_bytes() < w[1].0.as_bytes()), "sorted for the binary search");
    }

    #[test]
    fn menu_names_follow_the_reference_rules() {
        let db = parse_menu_db(
            "# family\n[Synth-Regular]\n\tf=Synth Pro\n\ts=Book\n[Synth-LightItalic]\n\tf=Synth Pro\n\ts=Light Italic\n\tl=Synth Pro Light\n\
             [Synth-Semibold]\n\tf=Synth Pro\n\ts=Semibold\n\tl=Synth Pro Semibold\n\tm=1,Synth Pro Semibold Mac\n\
             [Synth-Cond]\n\tf=Synth Pro\n\tf=3,1,0x407,Synth Deutsch\n\ts=Regular\n\tl=Synth Pro Cond\n[Synth-Esc]\n\tf=Caf\\00e9\n\tf=1,Caf\\8e\n",
        )
        .unwrap();
        let ids = |ps: &str, bold: bool, italic: bool| -> Vec<(u16, u16, String)> {
            menu_records(db.entry(ps).unwrap(), bold, italic).into_iter().map(|r| (r.id, r.language, r.text)).collect()
        };
        let t = |id: u16, lang: u16, s: &str| (id, lang, s.to_string());
        assert_eq!(
            ids("Synth-Regular", false, false),
            [t(1, 0x409, "Synth Pro"), t(2, 0x409, "Regular"), t(4, 0x409, "Synth Pro"), t(17, 0x409, "Book")]
        );
        assert_eq!(
            ids("Synth-LightItalic", false, true),
            [
                t(1, 0x409, "Synth Pro Light"),
                t(2, 0x409, "Italic"),
                t(4, 0x409, "Synth Pro Light Italic"),
                t(16, 0x409, "Synth Pro"),
                t(17, 0x409, "Light Italic")
            ]
        );
        let semibold = ids("Synth-Semibold", false, false);
        assert!(semibold.contains(&t(18, 0, "Synth Pro Semibold Mac")) && semibold.contains(&t(17, 0x409, "Semibold")));
        let cond = ids("Synth-Cond", false, false);
        assert!(cond.contains(&t(1, 0x407, "Synth Deutsch")) && cond.contains(&t(4, 0x407, "Synth Deutsch")));
        assert!(!cond.iter().any(|r| r.0 == 16 && r.1 == 0x407) && !cond.iter().any(|r| r.0 == 17), "{cond:?}");
        assert!(cond.contains(&t(16, 0x409, "Synth Pro")) && cond.contains(&t(1, 0x409, "Synth Pro Cond")));
        let esc = menu_records(db.entry("Synth-Esc").unwrap(), true, false);
        assert!(esc.iter().any(|r| r.id == 1 && r.platform == 3 && r.text == "Café"));
        assert!(esc.iter().any(|r| r.id == 4 && r.platform == 1 && r.text == "Café Bold"), "{esc:?}");
        for bad in ["f=Orphan\n", "[A]\n[A]\n", "[A]\nx=1\n", "[A]\nf=bad \\zz\n", "[A]\nm=Mac\n", "[\n"] {
            assert!(parse_menu_db(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn keyword_matching_follows_the_reference_patterns() {
        assert!(keyword_on("IsBoldStyle  true", "IsBoldStyle", true));
        assert!(keyword_on("x\nIsBoldStyle\t1\n", "IsBoldStyle", true));
        assert!(!keyword_on("IsBoldStyletrue", "IsBoldStyle", true));
        assert!(!keyword_on("IsBoldStyle false", "IsBoldStyle", true));
        assert!(keyword_on("OmitMacNamesTrue", "OmitMacNames", false));
        assert!(!keyword_on("OmitMacNames true", "OmitMacNames", false));
    }
}
