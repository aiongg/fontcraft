//! The registered OpenType feature tags (M8.14): names and specification links, as data written
//! from the OpenType specification's feature list. The feature picker, the sidebar and
//! `font.features.openSpec` read it.
//!
//! Rows: `font.features.listAvailable`, `font.features.openSpec`.

/// One registered feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureTag {
    pub tag: &'static str,
    /// The specification's name (`Small Capitals`).
    pub name: &'static str,
}

impl FeatureTag {
    /// The feature's entry in the OpenType specification's feature list.
    pub fn spec_url(&self) -> String {
        let page = match self.tag.as_bytes().first() {
            Some(b'a'..=b'e') => "features_ae",
            Some(b'f'..=b'j') => "features_fj",
            Some(b'k'..=b'o') => "features_ko",
            Some(b'p'..=b't') => "features_pt",
            _ => "features_uz",
        };
        // The specification describes the numbered tags in one section each, and keeps the
        // deprecated tags under headings of their own.
        let anchor = match self.tag {
            t if t.starts_with("cv") => "cv01--cv99",
            t if t.starts_with("ss") && t != "ssty" => "ss01---ss20",
            "hngl" => "hngl-deprecated",
            "opbd" => "opbd-deprecated",
            t => t,
        };
        format!("https://learn.microsoft.com/typography/opentype/spec/{page}#tag-{anchor}")
    }
}

/// Every registered feature in tag order; `cv01`–`cv99` and `ss01`–`ss20` listed one by one.
static TAGS: &[FeatureTag] = &[
    FeatureTag { tag: "aalt", name: "Access All Alternates" },
    FeatureTag { tag: "abvf", name: "Above-base Forms" },
    FeatureTag { tag: "abvm", name: "Above-base Mark Positioning" },
    FeatureTag { tag: "abvs", name: "Above-base Substitutions" },
    FeatureTag { tag: "afrc", name: "Alternative Fractions" },
    FeatureTag { tag: "akhn", name: "Akhand" },
    FeatureTag { tag: "apkn", name: "Kerning for Alternate Proportional Widths" },
    FeatureTag { tag: "blwf", name: "Below-base Forms" },
    FeatureTag { tag: "blwm", name: "Below-base Mark Positioning" },
    FeatureTag { tag: "blws", name: "Below-base Substitutions" },
    FeatureTag { tag: "c2pc", name: "Petite Capitals From Capitals" },
    FeatureTag { tag: "c2sc", name: "Small Capitals From Capitals" },
    FeatureTag { tag: "calt", name: "Contextual Alternates" },
    FeatureTag { tag: "case", name: "Case-sensitive Forms" },
    FeatureTag { tag: "ccmp", name: "Glyph Composition / Decomposition" },
    FeatureTag { tag: "cfar", name: "Conjunct Form After Ro" },
    FeatureTag { tag: "chws", name: "Contextual Half-width Spacing" },
    FeatureTag { tag: "cjct", name: "Conjunct Forms" },
    FeatureTag { tag: "clig", name: "Contextual Ligatures" },
    FeatureTag { tag: "cpct", name: "Centered CJK Punctuation" },
    FeatureTag { tag: "cpsp", name: "Capital Spacing" },
    FeatureTag { tag: "cswh", name: "Contextual Swash" },
    FeatureTag { tag: "curs", name: "Cursive Positioning" },
    FeatureTag { tag: "cv01", name: "Character Variant 1" },
    FeatureTag { tag: "cv02", name: "Character Variant 2" },
    FeatureTag { tag: "cv03", name: "Character Variant 3" },
    FeatureTag { tag: "cv04", name: "Character Variant 4" },
    FeatureTag { tag: "cv05", name: "Character Variant 5" },
    FeatureTag { tag: "cv06", name: "Character Variant 6" },
    FeatureTag { tag: "cv07", name: "Character Variant 7" },
    FeatureTag { tag: "cv08", name: "Character Variant 8" },
    FeatureTag { tag: "cv09", name: "Character Variant 9" },
    FeatureTag { tag: "cv10", name: "Character Variant 10" },
    FeatureTag { tag: "cv11", name: "Character Variant 11" },
    FeatureTag { tag: "cv12", name: "Character Variant 12" },
    FeatureTag { tag: "cv13", name: "Character Variant 13" },
    FeatureTag { tag: "cv14", name: "Character Variant 14" },
    FeatureTag { tag: "cv15", name: "Character Variant 15" },
    FeatureTag { tag: "cv16", name: "Character Variant 16" },
    FeatureTag { tag: "cv17", name: "Character Variant 17" },
    FeatureTag { tag: "cv18", name: "Character Variant 18" },
    FeatureTag { tag: "cv19", name: "Character Variant 19" },
    FeatureTag { tag: "cv20", name: "Character Variant 20" },
    FeatureTag { tag: "cv21", name: "Character Variant 21" },
    FeatureTag { tag: "cv22", name: "Character Variant 22" },
    FeatureTag { tag: "cv23", name: "Character Variant 23" },
    FeatureTag { tag: "cv24", name: "Character Variant 24" },
    FeatureTag { tag: "cv25", name: "Character Variant 25" },
    FeatureTag { tag: "cv26", name: "Character Variant 26" },
    FeatureTag { tag: "cv27", name: "Character Variant 27" },
    FeatureTag { tag: "cv28", name: "Character Variant 28" },
    FeatureTag { tag: "cv29", name: "Character Variant 29" },
    FeatureTag { tag: "cv30", name: "Character Variant 30" },
    FeatureTag { tag: "cv31", name: "Character Variant 31" },
    FeatureTag { tag: "cv32", name: "Character Variant 32" },
    FeatureTag { tag: "cv33", name: "Character Variant 33" },
    FeatureTag { tag: "cv34", name: "Character Variant 34" },
    FeatureTag { tag: "cv35", name: "Character Variant 35" },
    FeatureTag { tag: "cv36", name: "Character Variant 36" },
    FeatureTag { tag: "cv37", name: "Character Variant 37" },
    FeatureTag { tag: "cv38", name: "Character Variant 38" },
    FeatureTag { tag: "cv39", name: "Character Variant 39" },
    FeatureTag { tag: "cv40", name: "Character Variant 40" },
    FeatureTag { tag: "cv41", name: "Character Variant 41" },
    FeatureTag { tag: "cv42", name: "Character Variant 42" },
    FeatureTag { tag: "cv43", name: "Character Variant 43" },
    FeatureTag { tag: "cv44", name: "Character Variant 44" },
    FeatureTag { tag: "cv45", name: "Character Variant 45" },
    FeatureTag { tag: "cv46", name: "Character Variant 46" },
    FeatureTag { tag: "cv47", name: "Character Variant 47" },
    FeatureTag { tag: "cv48", name: "Character Variant 48" },
    FeatureTag { tag: "cv49", name: "Character Variant 49" },
    FeatureTag { tag: "cv50", name: "Character Variant 50" },
    FeatureTag { tag: "cv51", name: "Character Variant 51" },
    FeatureTag { tag: "cv52", name: "Character Variant 52" },
    FeatureTag { tag: "cv53", name: "Character Variant 53" },
    FeatureTag { tag: "cv54", name: "Character Variant 54" },
    FeatureTag { tag: "cv55", name: "Character Variant 55" },
    FeatureTag { tag: "cv56", name: "Character Variant 56" },
    FeatureTag { tag: "cv57", name: "Character Variant 57" },
    FeatureTag { tag: "cv58", name: "Character Variant 58" },
    FeatureTag { tag: "cv59", name: "Character Variant 59" },
    FeatureTag { tag: "cv60", name: "Character Variant 60" },
    FeatureTag { tag: "cv61", name: "Character Variant 61" },
    FeatureTag { tag: "cv62", name: "Character Variant 62" },
    FeatureTag { tag: "cv63", name: "Character Variant 63" },
    FeatureTag { tag: "cv64", name: "Character Variant 64" },
    FeatureTag { tag: "cv65", name: "Character Variant 65" },
    FeatureTag { tag: "cv66", name: "Character Variant 66" },
    FeatureTag { tag: "cv67", name: "Character Variant 67" },
    FeatureTag { tag: "cv68", name: "Character Variant 68" },
    FeatureTag { tag: "cv69", name: "Character Variant 69" },
    FeatureTag { tag: "cv70", name: "Character Variant 70" },
    FeatureTag { tag: "cv71", name: "Character Variant 71" },
    FeatureTag { tag: "cv72", name: "Character Variant 72" },
    FeatureTag { tag: "cv73", name: "Character Variant 73" },
    FeatureTag { tag: "cv74", name: "Character Variant 74" },
    FeatureTag { tag: "cv75", name: "Character Variant 75" },
    FeatureTag { tag: "cv76", name: "Character Variant 76" },
    FeatureTag { tag: "cv77", name: "Character Variant 77" },
    FeatureTag { tag: "cv78", name: "Character Variant 78" },
    FeatureTag { tag: "cv79", name: "Character Variant 79" },
    FeatureTag { tag: "cv80", name: "Character Variant 80" },
    FeatureTag { tag: "cv81", name: "Character Variant 81" },
    FeatureTag { tag: "cv82", name: "Character Variant 82" },
    FeatureTag { tag: "cv83", name: "Character Variant 83" },
    FeatureTag { tag: "cv84", name: "Character Variant 84" },
    FeatureTag { tag: "cv85", name: "Character Variant 85" },
    FeatureTag { tag: "cv86", name: "Character Variant 86" },
    FeatureTag { tag: "cv87", name: "Character Variant 87" },
    FeatureTag { tag: "cv88", name: "Character Variant 88" },
    FeatureTag { tag: "cv89", name: "Character Variant 89" },
    FeatureTag { tag: "cv90", name: "Character Variant 90" },
    FeatureTag { tag: "cv91", name: "Character Variant 91" },
    FeatureTag { tag: "cv92", name: "Character Variant 92" },
    FeatureTag { tag: "cv93", name: "Character Variant 93" },
    FeatureTag { tag: "cv94", name: "Character Variant 94" },
    FeatureTag { tag: "cv95", name: "Character Variant 95" },
    FeatureTag { tag: "cv96", name: "Character Variant 96" },
    FeatureTag { tag: "cv97", name: "Character Variant 97" },
    FeatureTag { tag: "cv98", name: "Character Variant 98" },
    FeatureTag { tag: "cv99", name: "Character Variant 99" },
    FeatureTag { tag: "dist", name: "Distances" },
    FeatureTag { tag: "dlig", name: "Discretionary Ligatures" },
    FeatureTag { tag: "dnom", name: "Denominators" },
    FeatureTag { tag: "dtls", name: "Dotless Forms" },
    FeatureTag { tag: "expt", name: "Expert Forms" },
    FeatureTag { tag: "falt", name: "Final Glyph on Line Alternates" },
    FeatureTag { tag: "fin2", name: "Terminal Forms #2" },
    FeatureTag { tag: "fin3", name: "Terminal Forms #3" },
    FeatureTag { tag: "fina", name: "Terminal Forms" },
    FeatureTag { tag: "flac", name: "Flattened Accent Forms" },
    FeatureTag { tag: "frac", name: "Fractions" },
    FeatureTag { tag: "fwid", name: "Full Widths" },
    FeatureTag { tag: "half", name: "Half Forms" },
    FeatureTag { tag: "haln", name: "Halant Forms" },
    FeatureTag { tag: "halt", name: "Alternate Half Widths" },
    FeatureTag { tag: "hist", name: "Historical Forms" },
    FeatureTag { tag: "hkna", name: "Horizontal Kana Alternates" },
    FeatureTag { tag: "hlig", name: "Historical Ligatures" },
    FeatureTag { tag: "hngl", name: "Hangul" },
    FeatureTag { tag: "hojo", name: "Hojo Kanji Forms (JIS X 0212-1990 Kanji Forms)" },
    FeatureTag { tag: "hwid", name: "Half Widths" },
    FeatureTag { tag: "init", name: "Initial Forms" },
    FeatureTag { tag: "isol", name: "Isolated Forms" },
    FeatureTag { tag: "ital", name: "Italics" },
    FeatureTag { tag: "jalt", name: "Justification Alternates" },
    FeatureTag { tag: "jp04", name: "JIS2004 Forms" },
    FeatureTag { tag: "jp78", name: "JIS78 Forms" },
    FeatureTag { tag: "jp83", name: "JIS83 Forms" },
    FeatureTag { tag: "jp90", name: "JIS90 Forms" },
    FeatureTag { tag: "kern", name: "Kerning" },
    FeatureTag { tag: "lfbd", name: "Left Bounds" },
    FeatureTag { tag: "liga", name: "Standard Ligatures" },
    FeatureTag { tag: "ljmo", name: "Leading Jamo Forms" },
    FeatureTag { tag: "lnum", name: "Lining Figures" },
    FeatureTag { tag: "locl", name: "Localized Forms" },
    FeatureTag { tag: "ltra", name: "Left-to-right Alternates" },
    FeatureTag { tag: "ltrm", name: "Left-to-right Mirrored Forms" },
    FeatureTag { tag: "mark", name: "Mark Positioning" },
    FeatureTag { tag: "med2", name: "Medial Forms #2" },
    FeatureTag { tag: "medi", name: "Medial Forms" },
    FeatureTag { tag: "mgrk", name: "Mathematical Greek" },
    FeatureTag { tag: "mkmk", name: "Mark to Mark Positioning" },
    FeatureTag { tag: "mset", name: "Mark Positioning via Substitution" },
    FeatureTag { tag: "nalt", name: "Alternate Annotation Forms" },
    FeatureTag { tag: "nlck", name: "NLC Kanji Forms" },
    FeatureTag { tag: "nukt", name: "Nukta Forms" },
    FeatureTag { tag: "numr", name: "Numerators" },
    FeatureTag { tag: "onum", name: "Oldstyle Figures" },
    FeatureTag { tag: "opbd", name: "Optical Bounds" },
    FeatureTag { tag: "ordn", name: "Ordinals" },
    FeatureTag { tag: "ornm", name: "Ornaments" },
    FeatureTag { tag: "palt", name: "Proportional Alternate Widths" },
    FeatureTag { tag: "pcap", name: "Petite Capitals" },
    FeatureTag { tag: "pkna", name: "Proportional Kana" },
    FeatureTag { tag: "pnum", name: "Proportional Figures" },
    FeatureTag { tag: "pref", name: "Pre-base Forms" },
    FeatureTag { tag: "pres", name: "Pre-base Substitutions" },
    FeatureTag { tag: "pstf", name: "Post-base Forms" },
    FeatureTag { tag: "psts", name: "Post-base Substitutions" },
    FeatureTag { tag: "pwid", name: "Proportional Widths" },
    FeatureTag { tag: "qwid", name: "Quarter Widths" },
    FeatureTag { tag: "rand", name: "Randomize" },
    FeatureTag { tag: "rclt", name: "Required Contextual Alternates" },
    FeatureTag { tag: "rkrf", name: "Rakar Forms" },
    FeatureTag { tag: "rlig", name: "Required Ligatures" },
    FeatureTag { tag: "rphf", name: "Reph Form" },
    FeatureTag { tag: "rtbd", name: "Right Bounds" },
    FeatureTag { tag: "rtla", name: "Right-to-left Alternates" },
    FeatureTag { tag: "rtlm", name: "Right-to-left Mirrored Forms" },
    FeatureTag { tag: "ruby", name: "Ruby Notation Forms" },
    FeatureTag { tag: "rvrn", name: "Required Variation Alternates" },
    FeatureTag { tag: "salt", name: "Stylistic Alternates" },
    FeatureTag { tag: "sinf", name: "Scientific Inferiors" },
    FeatureTag { tag: "size", name: "Optical size" },
    FeatureTag { tag: "smcp", name: "Small Capitals" },
    FeatureTag { tag: "smpl", name: "Simplified Forms" },
    FeatureTag { tag: "ss01", name: "Stylistic Set 1" },
    FeatureTag { tag: "ss02", name: "Stylistic Set 2" },
    FeatureTag { tag: "ss03", name: "Stylistic Set 3" },
    FeatureTag { tag: "ss04", name: "Stylistic Set 4" },
    FeatureTag { tag: "ss05", name: "Stylistic Set 5" },
    FeatureTag { tag: "ss06", name: "Stylistic Set 6" },
    FeatureTag { tag: "ss07", name: "Stylistic Set 7" },
    FeatureTag { tag: "ss08", name: "Stylistic Set 8" },
    FeatureTag { tag: "ss09", name: "Stylistic Set 9" },
    FeatureTag { tag: "ss10", name: "Stylistic Set 10" },
    FeatureTag { tag: "ss11", name: "Stylistic Set 11" },
    FeatureTag { tag: "ss12", name: "Stylistic Set 12" },
    FeatureTag { tag: "ss13", name: "Stylistic Set 13" },
    FeatureTag { tag: "ss14", name: "Stylistic Set 14" },
    FeatureTag { tag: "ss15", name: "Stylistic Set 15" },
    FeatureTag { tag: "ss16", name: "Stylistic Set 16" },
    FeatureTag { tag: "ss17", name: "Stylistic Set 17" },
    FeatureTag { tag: "ss18", name: "Stylistic Set 18" },
    FeatureTag { tag: "ss19", name: "Stylistic Set 19" },
    FeatureTag { tag: "ss20", name: "Stylistic Set 20" },
    FeatureTag { tag: "ssty", name: "Math Script-style Alternates" },
    FeatureTag { tag: "stch", name: "Stretching Glyph Decomposition" },
    FeatureTag { tag: "subs", name: "Subscript" },
    FeatureTag { tag: "sups", name: "Superscript" },
    FeatureTag { tag: "swsh", name: "Swash" },
    FeatureTag { tag: "titl", name: "Titling" },
    FeatureTag { tag: "tjmo", name: "Trailing Jamo Forms" },
    FeatureTag { tag: "tnam", name: "Traditional Name Forms" },
    FeatureTag { tag: "tnum", name: "Tabular Figures" },
    FeatureTag { tag: "trad", name: "Traditional Forms" },
    FeatureTag { tag: "twid", name: "Third Widths" },
    FeatureTag { tag: "unic", name: "Unicase" },
    FeatureTag { tag: "valt", name: "Alternate Vertical Metrics" },
    FeatureTag { tag: "vapk", name: "Kerning for Alternate Proportional Vertical Metrics" },
    FeatureTag { tag: "vatu", name: "Vattu Variants" },
    FeatureTag { tag: "vchw", name: "Vertical Contextual Half-width Spacing" },
    FeatureTag { tag: "vert", name: "Vertical Alternates" },
    FeatureTag { tag: "vhal", name: "Alternate Vertical Half Metrics" },
    FeatureTag { tag: "vjmo", name: "Vowel Jamo Forms" },
    FeatureTag { tag: "vkna", name: "Vertical Kana Alternates" },
    FeatureTag { tag: "vkrn", name: "Vertical Kerning" },
    FeatureTag { tag: "vpal", name: "Proportional Alternate Vertical Metrics" },
    FeatureTag { tag: "vrt2", name: "Vertical Alternates and Rotation" },
    FeatureTag { tag: "vrtr", name: "Vertical Alternates for Rotation" },
    FeatureTag { tag: "zero", name: "Slashed Zero" },
];

pub fn all() -> &'static [FeatureTag] {
    TAGS
}

/// The registered feature `tag`, or `None` for a tag the specification does not register.
pub fn lookup(tag: &str) -> Option<&'static FeatureTag> {
    TAGS.iter().find(|t| t.tag == tag)
}

/// The registered features whose tag or name contains `query`, ignoring case, in tag order;
/// every feature for an empty query.
pub fn search(query: &str) -> impl Iterator<Item = &'static FeatureTag> {
    let query = query.trim().to_lowercase();
    TAGS.iter().filter(move |t| query.is_empty() || t.tag.to_lowercase().contains(&query) || t.name.to_lowercase().contains(&query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_sorted_complete_and_links_each_tag_to_its_section() {
        assert!(all().windows(2).all(|w| w[0].tag < w[1].tag), "tags in order, no repeats");
        assert!(all().iter().all(|t| t.tag.len() == 4 && !t.name.is_empty()));
        assert_eq!(all().iter().filter(|t| t.tag.starts_with("cv")).count(), 99);
        assert_eq!(all().iter().filter(|t| t.tag.starts_with("ss") && t.tag != "ssty").count(), 20);
        assert_eq!(lookup("smcp").map(|t| t.name), Some("Small Capitals"));
        assert_eq!(lookup("ss07").map(|t| t.name), Some("Stylistic Set 7"));
        assert!(lookup("xyzw").is_none());
        let url = |tag| lookup(tag).map(FeatureTag::spec_url).unwrap_or_default();
        assert_eq!(url("liga"), "https://learn.microsoft.com/typography/opentype/spec/features_ko#tag-liga");
        assert_eq!(url("cv42"), "https://learn.microsoft.com/typography/opentype/spec/features_ae#tag-cv01--cv99");
        assert_eq!(url("ss03"), "https://learn.microsoft.com/typography/opentype/spec/features_pt#tag-ss01---ss20");
        assert_eq!(url("ssty"), "https://learn.microsoft.com/typography/opentype/spec/features_pt#tag-ssty");
        assert_eq!(url("zero"), "https://learn.microsoft.com/typography/opentype/spec/features_uz#tag-zero");
        let found: Vec<&str> = search("small cap").map(|t| t.tag).collect();
        assert_eq!(found, ["c2sc", "smcp"]);
        assert_eq!(search("LIGA").count(), 1);
    }
}
