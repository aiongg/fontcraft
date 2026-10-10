//! A glyph's effective properties (`docs/spec/05-font-view.md` § 2 and § 4.2).
//!
//! Every property the glyph database backs (production name, category, subcategory, case,
//! direction, script, sort name) is resolved on its own, first hit wins: the value stored on the
//! glyph, then custom glyph data, then the built-in data, then synthesis from the name. Font View,
//! `glyph.getInfo`, smart filters and the compiler all read properties through [`glyph_props`].
//!
//! Values keep the spellings source files use (`Letter`, `Decimal Digit`, `smallCaps`, `RTL`), so
//! a stored value the vocabulary does not know is shown as stored instead of being dropped.

use fontcraft_glyphdata::{GlyphData, Source, Sourced, Via};
use serde::Serialize;

use crate::Glyph;

/// Where an effective property's value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PropSource {
    /// Stored on the glyph: the user's override.
    Stored,
    /// A record of a custom glyph data file.
    Custom,
    /// A record of the built-in database.
    Database,
    /// Derived from the name by the suffix, ligature or code-point rules.
    Synthesised,
    /// No rule applied: the glyph's own name as production name, left to right as direction.
    Default,
    /// Nothing gives a value.
    None,
}

impl PropSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PropSource::Stored => "stored",
            PropSource::Custom => "custom",
            PropSource::Database => "database",
            PropSource::Synthesised => "synthesised",
            PropSource::Default => "default",
            PropSource::None => "none",
        }
    }
}

impl From<Source> for PropSource {
    fn from(s: Source) -> Self {
        match s {
            Source::Custom => PropSource::Custom,
            Source::Database => PropSource::Database,
            Source::Synthesised => PropSource::Synthesised,
            Source::Default => PropSource::Default,
        }
    }
}

/// One effective property: its value (`None`: empty) and where it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Prop {
    pub value: Option<String>,
    pub source: PropSource,
}

impl Prop {
    const NONE: Prop = Prop { value: None, source: PropSource::None };

    fn stored(value: &str) -> Prop {
        Prop { value: Some(value.to_string()), source: PropSource::Stored }
    }

    fn data<T>(sourced: Option<Sourced<T>>, text: impl Fn(T) -> String) -> Prop {
        match sourced {
            Some(s) => Prop { value: Some(text(s.value)), source: s.source.into() },
            None => Prop::NONE,
        }
    }

    pub fn as_deref(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

/// Every database-backed property of a glyph, with its source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphProps {
    /// The name the glyph exports under, before export settings (*Don't use Production Names*)
    /// are applied.
    pub production_name: Prop,
    pub category: Prop,
    pub sub_category: Prop,
    /// `upper`, `lower`, `smallCaps`, `minor`, `other`; empty for no case (a stored `noCase` is
    /// an empty value with source `stored`).
    pub case: Prop,
    /// `LTR`, `RTL`, `BIDI`, `VTR` or `VTL`.
    pub direction: Prop,
    pub script: Prop,
    /// The glyph's own sort name, else its custom glyph data record's. Empty when neither gives
    /// one: the glyph then sorts by the default key (`glyphOrder.default`).
    pub sort_name: Prop,
    /// The record's description; never stored on a glyph.
    pub description: Prop,
    /// The code point the glyph data gives the glyph's name (not one of its stored code points).
    pub data_unicode: Option<u32>,
    /// The glyph stores a property that differs from what the glyph data gives, or code points
    /// other than the data's for its name (`fontView.cell.badgeCustomInfo`).
    pub has_custom_info: bool,
}

/// What the glyph data alone says about a glyph with this name and these code points: the
/// properties a glyph has when it stores none. `keep_alternates` is the font's *Keep Alternates
/// Next to Base Glyph* setting, which picks the sort name.
pub fn data_props(name: &str, unicodes: &[u32], data: &GlyphData, keep_alternates: bool) -> GlyphProps {
    let info = data.lookup(name, unicodes);
    let data_unicode = if unicodes.is_empty() { info.unicode.as_ref().map(|u| u.value) } else { data.lookup(name, &[]).unicode.map(|u| u.value) };
    let sort_name = match info.found.as_ref().filter(|f| f.via == Via::Name) {
        Some(f) => {
            let r = &f.record;
            let value = if keep_alternates { r.sort_name_keep.clone().or_else(|| r.sort_name.clone()) } else { r.sort_name.clone() };
            let source = if f.custom { PropSource::Custom } else { PropSource::Database };
            value.map_or(Prop::NONE, |v| Prop { value: Some(v), source })
        }
        None => Prop::NONE,
    };
    GlyphProps {
        production_name: Prop::data(Some(info.production_name), |v| v),
        category: Prop::data(info.category, |v| v.as_str().to_string()),
        sub_category: Prop::data(info.sub_category, |v| v.as_str().to_string()),
        case: Prop::data(info.case, |v| v.as_str().to_string()),
        direction: Prop::data(Some(info.direction), |v| v.as_str().to_string()),
        script: Prop::data(info.script, |v| v),
        sort_name,
        description: Prop::data(info.description, |v| v),
        data_unicode,
        has_custom_info: false,
    }
}

/// The glyph's effective properties: what it stores, else what the glyph data gives.
/// `keep_alternates` is the font's *Keep Alternates Next to Base Glyph* setting
/// (`font.info.settings.keep_alternates_together`).
pub fn glyph_props(glyph: &Glyph, data: &GlyphData, keep_alternates: bool) -> GlyphProps {
    let from_data = data_props(&glyph.name, &glyph.unicodes, data, keep_alternates);
    let i = &glyph.info;
    let over = |stored: Option<&str>, d: &Prop| stored.map_or_else(|| d.clone(), Prop::stored);
    let case = match i.case.as_deref() {
        Some("noCase") => Prop { value: None, source: PropSource::Stored },
        other => over(other, &from_data.case),
    };
    let own_sort_name = if keep_alternates { i.sort_name_keep.as_deref().or(i.sort_name.as_deref()) } else { i.sort_name.as_deref() };
    let props = GlyphProps {
        production_name: over(i.production.as_deref(), &from_data.production_name),
        category: over(i.category.as_deref(), &from_data.category),
        sub_category: over(i.sub_category.as_deref(), &from_data.sub_category),
        case,
        direction: over(i.direction.as_deref(), &from_data.direction),
        script: over(i.script.as_deref(), &from_data.script),
        sort_name: over(own_sort_name, &from_data.sort_name),
        description: from_data.description.clone(),
        data_unicode: from_data.data_unicode,
        has_custom_info: false,
    };
    let has_custom_info = stores_custom_info(glyph, &props, &from_data, data);
    GlyphProps { has_custom_info, ..props }
}

/// Whether a stored property differs from the data's (`fontView.cell.badgeCustomInfo` guesses).
fn stores_custom_info(glyph: &Glyph, props: &GlyphProps, from_data: &GlyphProps, data: &GlyphData) -> bool {
    let differs = |p: &Prop, d: &Prop| p.source == PropSource::Stored && p.value != d.value;
    if differs(&props.production_name, &from_data.production_name)
        || differs(&props.category, &from_data.category)
        || differs(&props.sub_category, &from_data.sub_category)
        || differs(&props.case, &from_data.case)
        || differs(&props.direction, &from_data.direction)
        || differs(&props.script, &from_data.script)
    {
        return true;
    }
    // Both sort names, whichever the font's setting uses.
    let record = data.record(&glyph.name);
    let i = &glyph.info;
    if i.sort_name.is_some() && i.sort_name != record.as_ref().and_then(|r| r.sort_name.clone()) {
        return true;
    }
    if i.sort_name_keep.is_some() && i.sort_name_keep != record.as_ref().and_then(|r| r.sort_name_keep.clone()) {
        return true;
    }
    glyph.unicodes.as_slice() != from_data.data_unicode.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> GlyphData {
        GlyphData::new().unwrap()
    }

    #[test]
    fn stored_values_win_and_sources_are_reported() {
        let d = data();
        let mut a = Glyph::new("A");
        a.unicodes = vec![0x41];
        let p = glyph_props(&a, &d, false);
        assert_eq!(p.category.as_deref(), Some("Letter"));
        assert_eq!(p.category.source, PropSource::Database);
        assert_eq!(p.case.as_deref(), Some("upper"));
        assert_eq!(p.script.as_deref(), Some("latin"));
        assert_eq!(p.direction.as_deref(), Some("LTR"));
        assert_eq!(p.data_unicode, Some(0x41));
        assert!(!p.has_custom_info);

        a.info.category = Some("Symbol".into());
        a.info.case = Some("noCase".into());
        let p = glyph_props(&a, &d, false);
        assert_eq!((p.category.as_deref(), p.category.source), (Some("Symbol"), PropSource::Stored));
        assert_eq!((p.case.as_deref(), p.case.source), (None, PropSource::Stored));
        assert!(p.has_custom_info);

        // A stored value equal to the data's is not custom info.
        let mut b = Glyph::new("A");
        b.unicodes = vec![0x41];
        b.info.category = Some("Letter".into());
        assert!(!glyph_props(&b, &d, false).has_custom_info);
        // Code points other than the data's are.
        b.unicodes = vec![0x42];
        assert!(glyph_props(&b, &d, false).has_custom_info);
    }

    #[test]
    fn synthesised_and_custom_sources() {
        let d = data();
        let p = glyph_props(&Glyph::new("A-cy.sc"), &d, false);
        assert_eq!(p.production_name.as_deref(), Some("uni0410.sc"));
        assert_eq!(p.category.source, PropSource::Synthesised);
        let unknown = glyph_props(&Glyph::new("zzzz"), &d, false);
        assert_eq!(unknown.category, Prop::NONE);
        assert_eq!(unknown.production_name.source, PropSource::Default);

        let custom = GlyphData::with_custom(vec![vec![fontcraft_glyphdata::Record {
            name: "myglyph".into(),
            category: Some(fontcraft_glyphdata::Category::Symbol),
            sort_name: Some("0001".into()),
            sort_name_keep: Some("0002".into()),
            ..Default::default()
        }]])
        .unwrap();
        let p = glyph_props(&Glyph::new("myglyph"), &custom, false);
        assert_eq!((p.category.as_deref(), p.category.source), (Some("Symbol"), PropSource::Custom));
        assert_eq!(p.sort_name.as_deref(), Some("0001"));
        assert_eq!(glyph_props(&Glyph::new("myglyph"), &custom, true).sort_name.as_deref(), Some("0002"));
    }
}
