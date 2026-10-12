//! Scripts, writing directions and OpenType script tags, as the kern writer uses them.
//!
//! Ported from fontc (https://github.com/googlefonts/fontc), `fontbe/src/features/properties.rs`
//! and `fontbe/src/features/ot_tags.rs`, Apache-2.0, at commit
//! 4504578c3f74c2739f7e55cae133bbfe773f6d8c. fontc follows ufo2ft's kern feature writer and
//! fontTools' `unicodedata`. Left out: the substitution closure (sources with feature code are
//! not given generated kerning before M8) and the substitutions of feature variations (static
//! builds have none).

use std::collections::{BTreeMap, HashMap, HashSet};

use icu_properties::PropertyParser;
use icu_properties::props::{BidiClass, Script};
use read_fonts::collections::IntSet;
use read_fonts::tables::gsub::Gsub;
use read_fonts::types::GlyphId;
use write_fonts::types::{GlyphId16, Tag};

use crate::unicode18;

/// A four-letter ISO 15924 script code, such as `Latn`. Ordered as its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptCode([u8; 4]);

/// A script code from its four ASCII letters.
pub(crate) const fn sc(code: &[u8; 4]) -> ScriptCode {
    ScriptCode(*code)
}

impl ScriptCode {
    /// A script code from text of four ASCII letters.
    pub fn parse(text: &str) -> Option<ScriptCode> {
        let bytes: [u8; 4] = text.as_bytes().try_into().ok()?;
        bytes.iter().all(u8::is_ascii_alphabetic).then_some(ScriptCode(bytes))
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("Zzzz")
    }
}

pub(crate) const COMMON_SCRIPT: ScriptCode = sc(b"Zyyy");
pub(crate) const INHERITED_SCRIPT: ScriptCode = sc(b"Zinh");
const HIRA: ScriptCode = sc(b"Hira");
const KANA: ScriptCode = sc(b"Kana");
const HRKT: ScriptCode = sc(b"Hrkt");

pub(crate) const DFLT_SCRIPT: Tag = Tag::new(b"DFLT");
pub(crate) const DFLT_LANG: Tag = Tag::new(b"dflt");

/// The writing direction of a script.
#[derive(Clone, Debug, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScriptDirection {
    /// Any direction: the common script.
    Auto,
    LeftToRight,
    RightToLeft,
}

impl ScriptDirection {
    /// The writing direction of `script`; the list is fontTools' `RTL_SCRIPTS`.
    pub(crate) fn for_script(script: &ScriptCode) -> Self {
        match script.as_str() {
            "Zyyy" => ScriptDirection::Auto,
            "Arab" | "Hebr" | "Syrc" | "Thaa" | "Cprt" | "Khar" | "Phnx" | "Nkoo" | "Lydi" | "Avst" | "Armi" | "Phli" | "Prti" | "Sarb" | "Orkh"
            | "Samr" | "Mand" | "Merc" | "Mero" | "Mani" | "Mend" | "Nbat" | "Narb" | "Palm" | "Phlp" | "Hatr" | "Hung" | "Adlm" | "Rohg"
            | "Sogo" | "Sogd" | "Elym" | "Chrs" | "Yezi" | "Ougr" => ScriptDirection::RightToLeft,
            _ => ScriptDirection::LeftToRight,
        }
    }

    /// True when either side is `Auto` or both sides are equal.
    pub(crate) fn plays_nicely_with(&self, other: &ScriptDirection) -> bool {
        matches!(
            (self, other),
            (ScriptDirection::Auto, _)
                | (_, ScriptDirection::Auto)
                | (ScriptDirection::LeftToRight, ScriptDirection::LeftToRight)
                | (ScriptDirection::RightToLeft, ScriptDirection::RightToLeft)
        )
    }
}

/// The strong direction of a character, as the kern writer splits pairs by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Bidi {
    LeftToRight,
    RightToLeft,
}

/// The strong direction of `cp`; `None` for neutral characters.
fn unicode_bidi_type(cp: u32) -> Option<Bidi> {
    let class = unicode18::bidi_class(cp);
    if class == BidiClass::RightToLeft || class == BidiClass::ArabicLetter {
        Some(Bidi::RightToLeft)
    } else if class == BidiClass::LeftToRight || class == BidiClass::ArabicNumber || class == BidiClass::EuropeanNumber {
        Some(Bidi::LeftToRight)
    } else {
        None
    }
}

/// The glyphs of each property value (ufo2ft's `classify`). `char_map` maps code points to
/// glyphs. With `gsub`, a value also gets the glyphs its glyphs reach through the substitutions
/// that the glyphs without any value do not reach on their own; a closure that cannot be read
/// adds nothing.
fn classify<T: Ord>(
    char_map: &HashMap<u32, GlyphId16>,
    mut props: impl FnMut(u32, &mut Vec<T>),
    gsub: Option<&Gsub>,
) -> BTreeMap<T, IntSet<GlyphId16>> {
    let mut sets: BTreeMap<T, IntSet<GlyphId16>> = BTreeMap::new();
    let mut neutral: IntSet<GlyphId> = IntSet::new();
    let mut buf = Vec::new();
    for (cp, gid) in char_map {
        props(*cp, &mut buf);
        let mut has_props = false;
        for prop in buf.drain(..) {
            sets.entry(prop).or_default().insert(*gid);
            has_props = true;
        }
        if !has_props {
            neutral.insert((*gid).into());
        }
    }
    let Some(gsub) = gsub else { return sets };
    let Ok(lookups) = gsub.collect_lookups(&IntSet::all()) else { return sets };
    if gsub.closure_glyphs(&lookups, &mut neutral).is_err() {
        return sets;
    }
    for glyphs in sets.values_mut() {
        let mut reached: IntSet<GlyphId> = glyphs.iter().map(GlyphId::from).collect();
        reached.union(&neutral);
        if gsub.closure_glyphs(&lookups, &mut reached).is_err() {
            continue;
        }
        glyphs.extend(reached.iter().filter(|g| !neutral.contains(*g)).filter_map(|g| GlyphId16::try_from(g).ok()));
    }
    sets
}

/// The Script_Extensions of `cp`, with Hiragana and Katakana as one script (`Hrkt`), as ufo2ft
/// reads them.
pub(crate) fn unicode_script_extensions(cp: u32) -> impl Iterator<Item = ScriptCode> {
    let mut seen_hrkt = false;
    unicode18::script_extensions(cp).into_iter().filter_map(move |script| {
        if script == HIRA || script == KANA {
            if seen_hrkt {
                None
            } else {
                seen_hrkt = true;
                Some(HRKT)
            }
        } else {
            Some(script)
        }
    })
}

/// The scripts of each glyph: the scripts of its code points that the font knows, plus the
/// common and inherited scripts. With no known script, every encoded glyph is common.
pub(crate) fn scripts_by_glyph(
    char_map: &HashMap<u32, GlyphId16>,
    known_scripts: &HashSet<ScriptCode>,
    gsub: Option<&Gsub>,
) -> HashMap<GlyphId16, HashSet<ScriptCode>> {
    let mut result: HashMap<GlyphId16, HashSet<ScriptCode>> = HashMap::new();
    let sets = classify(
        char_map,
        |cp, buf| {
            if known_scripts.is_empty() {
                buf.push(COMMON_SCRIPT);
            } else {
                buf.extend(
                    unicode_script_extensions(cp)
                        .filter(|script| *script == COMMON_SCRIPT || *script == INHERITED_SCRIPT || known_scripts.contains(script)),
                );
            }
        },
        gsub,
    );
    for (script, glyphs) in sets {
        for glyph in glyphs.iter() {
            result.entry(glyph).or_default().insert(script);
        }
    }
    result
}

/// The glyphs whose code points `predicate` holds for, with the glyphs they reach through
/// `gsub` (fontc's `glyphs_matching_predicate`). A code point the predicate gives no answer for
/// is neutral.
pub(crate) fn glyphs_matching(char_map: &HashMap<u32, GlyphId16>, predicate: impl Fn(u32) -> Option<bool>, gsub: Option<&Gsub>) -> IntSet<GlyphId16> {
    classify(char_map, |cp, buf| buf.extend(predicate(cp)), gsub).remove(&true).unwrap_or_default()
}

/// The glyphs of each strong direction.
pub(crate) fn glyphs_by_bidi_class(char_map: &HashMap<u32, GlyphId16>, gsub: Option<&Gsub>) -> BTreeMap<Bidi, IntSet<GlyphId16>> {
    classify(char_map, |cp, buf| buf.extend(unicode_bidi_type(cp)), gsub)
}

/// The scripts whose kerning goes to the `dist` feature instead of `kern`: the Indic scripts,
/// the scripts of the Universal Shaping Engine, Khmer and Myanmar.
pub(crate) fn dist_feature_enabled_scripts() -> HashSet<ScriptCode> {
    INDIC_SCRIPTS.iter().chain(USE_SCRIPTS).chain(["Khmr", "Mymr"].iter()).filter_map(|s| ScriptCode::parse(s)).collect()
}

/// The scripts the font uses (ufo2ft's `guessFontScripts`): those of each code point with
/// exactly one script extension, and those of the feature code's language systems
/// (`fea_scripts`, OpenType script tags).
pub(crate) fn guess_font_scripts(char_map: &HashMap<u32, GlyphId16>, fea_scripts: &[Tag]) -> HashSet<ScriptCode> {
    let declared = fea_scripts.iter().filter_map(|t| ot_tag_to_script(*t));
    char_map
        .keys()
        .filter_map(|cp| {
            let mut iter = unicode_script_extensions(*cp);
            match (iter.next(), iter.next()) {
                (Some(script), None) => Some(script),
                _ => None,
            }
        })
        .chain(declared)
        .collect()
}

/// The Unicode script of an OpenType script tag (fontTools' `ot_tag_to_script`); `None` for
/// `DFLT`.
pub(crate) fn ot_tag_to_script(tag: Tag) -> Option<ScriptCode> {
    if tag == DFLT_SCRIPT {
        return None;
    }
    let tag = binary_search_exact(SCRIPT_ALIASES, &tag).unwrap_or(tag);
    if let Some(script) = binary_search_exact(SCRIPT_EXCEPTIONS_REVERSED, &tag).or_else(|| binary_search_exact(NEW_SCRIPTS, &tag)) {
        return ScriptCode::parse(script);
    }
    // The first letter in upper case; trailing spaces repeat the last letter (`lao ` is `Laoo`).
    let bytes = tag.to_be_bytes();
    let first = bytes.first()?.to_ascii_uppercase();
    let mut out = [first, 0, 0, 0];
    let mut last = *bytes.get(1)?;
    for i in 1..4 {
        let b = *bytes.get(i)?;
        if b != b' ' {
            last = b;
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = last;
        }
    }
    let code = ScriptCode(out);
    code.0.iter().all(u8::is_ascii_alphabetic).then_some(code)
}

fn binary_search_exact<T: Ord, U: Copy>(items: &[(T, U)], needle: &T) -> Option<U> {
    items.binary_search_by(|probe| probe.0.cmp(needle)).ok().and_then(|idx| items.get(idx)).map(|item| item.1)
}

/// The OpenType script tags of a script: one, or two for the Indic scripts with a newer shaping
/// model (`bng2` and `beng`).
pub(crate) fn script_to_ot_tags(script: &ScriptCode) -> impl Iterator<Item = Tag> + use<> {
    let (first, second) = if let Some(tag) = binary_search_exact(SCRIPT_EXCEPTIONS, &script.as_str()) {
        (Some(tag), None)
    } else if !unicode18::is_new_script(*script) && PropertyParser::<Script>::new().get_strict(script.as_str()).is_none() {
        (Some(DFLT_SCRIPT), None)
    } else {
        let mut lower = script.0;
        lower.make_ascii_lowercase();
        (binary_search_exact(NEW_SCRIPT_TAGS, &script.as_str()), Some(Tag::new(&lower)))
    };
    [first, second].into_iter().flatten()
}

// OpenType tags of Unicode scripts, from fontTools' `unicodedata/OTTags.py`.

static SCRIPT_EXCEPTIONS: &[(&str, Tag)] = &[
    ("Hira", Tag::new(b"kana")),
    ("Hrkt", Tag::new(b"kana")),
    ("Laoo", Tag::new(b"lao ")),
    ("Nkoo", Tag::new(b"nko ")),
    ("Vaii", Tag::new(b"vai ")),
    ("Yiii", Tag::new(b"yi  ")),
    ("Zinh", DFLT_SCRIPT),
    ("Zmth", Tag::new(b"math")),
    ("Zyyy", DFLT_SCRIPT),
    ("Zzzz", DFLT_SCRIPT),
];

static SCRIPT_ALIASES: &[(Tag, Tag)] = &[(Tag::new(b"jamo"), Tag::new(b"hang"))];

static SCRIPT_EXCEPTIONS_REVERSED: &[(Tag, &str)] = &[(Tag::new(b"math"), "Zmth")];

static NEW_SCRIPTS: &[(Tag, &str)] = &[
    (Tag::new(b"bng2"), "Beng"),
    (Tag::new(b"dev2"), "Deva"),
    (Tag::new(b"gjr2"), "Gujr"),
    (Tag::new(b"gur2"), "Guru"),
    (Tag::new(b"knd2"), "Knda"),
    (Tag::new(b"mlm2"), "Mlym"),
    (Tag::new(b"mym2"), "Mymr"),
    (Tag::new(b"ory2"), "Orya"),
    (Tag::new(b"tel2"), "Telu"),
    (Tag::new(b"tml2"), "Taml"),
];

static NEW_SCRIPT_TAGS: &[(&str, Tag)] = &[
    ("Beng", Tag::new(b"bng2")),
    ("Deva", Tag::new(b"dev2")),
    ("Gujr", Tag::new(b"gjr2")),
    ("Guru", Tag::new(b"gur2")),
    ("Knda", Tag::new(b"knd2")),
    ("Mlym", Tag::new(b"mlm2")),
    ("Mymr", Tag::new(b"mym2")),
    ("Orya", Tag::new(b"ory2")),
    ("Taml", Tag::new(b"tml2")),
    ("Telu", Tag::new(b"tel2")),
];

static INDIC_SCRIPTS: &[&str] = &["Beng", "Deva", "Gujr", "Guru", "Knda", "Mlym", "Orya", "Sinh", "Taml", "Telu"];

static USE_SCRIPTS: &[&str] = &[
    "Adlm", "Ahom", "Bali", "Batk", "Bhks", "Brah", "Bugi", "Buhd", "Cakm", "Cham", "Chrs", "Cpmn", "Diak", "Dogr", "Dupl", "Egyp", "Elym", "Gong",
    "Gonm", "Gran", "Hano", "Hmng", "Hmnp", "Java", "Kali", "Kawi", "Khar", "Khoj", "Kits", "Kthi", "Lana", "Lepc", "Limb", "Mahj", "Maka", "Mand",
    "Mani", "Marc", "Medf", "Modi", "Mong", "Mtei", "Mult", "Nagm", "Nand", "Newa", "Nkoo", "Ougr", "Phag", "Phlp", "Plrd", "Rjng", "Rohg", "Saur",
    "Shrd", "Sidd", "Sind", "Sogd", "Sogo", "Soyo", "Sund", "Sylo", "Tagb", "Takr", "Tale", "Tavt", "Tfng", "Tglg", "Tibt", "Tirh", "Tnsa", "Toto",
    "Vith", "Wcho", "Yezi", "Zanb",
];
