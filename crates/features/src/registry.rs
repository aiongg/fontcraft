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
        format!("https://learn.microsoft.com/typography/opentype/spec/{page}#tag-{}", self.tag)
    }
}

/// Every registered feature in tag order; `cv01`–`cv99` and `ss01`–`ss20` listed one by one.
static TAGS: &[FeatureTag] = &[
    // M8.14
];

pub fn all() -> &'static [FeatureTag] {
    TAGS
}

/// The registered feature `tag`, or `None` for a tag the specification does not register.
pub fn lookup(tag: &str) -> Option<&'static FeatureTag> {
    TAGS.iter().find(|t| t.tag == tag)
}
