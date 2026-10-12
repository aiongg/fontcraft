//! Alternate layers as feature variations (M9.8).
//!
//! Rows: `alternateLayer.variableExport`, `compile.featureVariations`,
//! `compile.bracketLayerGlyphs`, `switching.threeMethods` (alternate half), parameter
//! `Feature for Feature Variations`.
//!
//! Follows glyphsLib 6.15.0 (`builder/bracket_layers.py`, Apache-2.0), fontTools 4.66.1
//! (`varLib/featureVars.py`, MIT) and fontc (`glyphs2fontir/src/source.rs`,
//! `fontbe/src/features/feature_variations.rs`, Apache-2.0); see `NOTICE`.
//!
//! Contract:
//! - Each alternate set of a glyph becomes a glyph of its own, named
//!   `<glyph>.BRACKET.varAltNN` ([`bracket_glyph_name`]), numbered from 01 in the order of
//!   `fontcraft_interp::GlyphSources::alternates`. [`bracket_glyphs`] lists them as
//!   [`MadeGlyph`]s sorted by name, the order both oracles append them to the glyph order in;
//!   [`bracket_production_name`] gives each the parent's production name with the same suffix.
//!   Glyphs that do not export get none.
//! - The sets' boxes go through `fontcraft_interp::overlay` ([`alternate_rules`], [`from_font`]);
//!   each record becomes a [`FeatureVariation`] of single substitutions under the feature the
//!   Feature for Feature Variations parameter names, [`DEFAULT_FEATURE`] when it names none or a
//!   tag that is not one (`docs/plans/M9.md` § 7 question 2, decided). Where two sets of a glyph
//!   overlap, the later set's glyph is substituted, as both oracles do.
//! - Conditions are in normalized coordinates after `avar`, as `GSUB` stores them.
//! - Until M8's compile merges them into the feature code's `GSUB`, [`build_gsub`] writes a
//!   `GSUB` holding only these lookups, as fontTools' `addFeatureVariations` does for a font
//!   without one: the records overlaid a second time (as fontmake's `varLib` reads them from the
//!   designspace rules), one lookup per distinct substitution map in name order, one empty
//!   feature in a `DFLT` script, and a feature variation record per region.
//!
//! Where this meets M8: [`FeatureVariation`] has the fields of
//! `fea::GeneratedLookups::feature_variations`' entries; when M8 is in, the records are handed to
//! `fea` and [`build_gsub`] is no longer called. When the feature is `rvrn` alone, its lookups
//! then go before the feature code's; for any other feature after them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fontcraft_interp::overlay::{Substitutions, overlay_substitutions};
use fontcraft_interp::{AxisSpan, Designspace, GlyphSources, MAX_OVERLAY_RECORDS, MAX_OVERLAY_WORK, OverlayRecord, OverlayRule, RegionBox};
use fontcraft_model::Font;
use write_fonts::tables::gsub::builders::SingleSubBuilder;
use write_fonts::tables::gsub::{Gsub, SubstitutionLookup, SubstitutionLookupList};
use write_fonts::tables::layout::builders::{Builder, LookupBuilder};
use write_fonts::tables::layout::{
    Condition, ConditionFormat1, ConditionSet, Feature, FeatureList, FeatureRecord, FeatureTableSubstitution, FeatureTableSubstitutionRecord,
    FeatureVariationRecord, FeatureVariations, LangSys, LookupFlag, Script, ScriptList, ScriptRecord,
};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::types::{F2Dot14, GlyphId16, Tag};

use super::VarAxes;
use crate::CompileError;
use crate::order::{MadeFrom, MadeGlyph};

/// The feature that switches alternate layers when the font names none.
pub const DEFAULT_FEATURE: &str = "rvrn";
/// The font parameter naming the feature for feature variations.
pub const FEATURE_PARAMETER: &str = "Feature for Feature Variations";
/// What an alternate glyph's name adds to its parent's, before the two-digit number.
pub const BRACKET_INFIX: &str = ".BRACKET.varAlt";

/// One feature variation: where it applies and what it substitutes.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureVariation {
    /// The feature whose lookups the record replaces.
    pub tag: Tag,
    /// `(axis tag, min, max)` in normalized coordinates; all must hold.
    pub conditions: Vec<(Tag, f64, f64)>,
    pub substitutions: Vec<(GlyphId16, GlyphId16)>,
}

/// The name of the `number`th alternate glyph of `parent` (`a.BRACKET.varAlt01`).
pub fn bracket_glyph_name(parent: &str, number: usize) -> String {
    format!("{parent}{BRACKET_INFIX}{number:02}")
}

/// The production name of the alternate glyph `bracket` of `parent`, whose production name is
/// `production`: the production name with the suffix the alternate's name adds (`uni0024` and
/// `dollar.BRACKET.varAlt01` give `uni0024.BRACKET.varAlt01`). `None` when `bracket` is not
/// named after `parent`.
pub fn bracket_production_name(parent: &str, production: &str, bracket: &str) -> Option<String> {
    let suffix = bracket.strip_prefix(parent)?;
    Some(format!("{production}{suffix}"))
}

fn table_error(message: impl Into<String>) -> CompileError {
    CompileError::Table { table: "GSUB", message: message.into() }
}

/// Whether `glyph` has a layer with alternate ranges; a cheap test before building its sources.
fn has_alternate_layers(glyph: &fontcraft_model::Glyph) -> bool {
    glyph.layers.iter().any(|l| !l.info.attrs.axis_rules.is_empty())
}

/// The exported glyphs with alternate layers and their sources, in the font's glyph order.
fn alternate_sources(font: &Font, designspace: &Designspace) -> Result<Vec<GlyphSources>, CompileError> {
    let mut out = Vec::new();
    for glyph in font.glyphs.iter() {
        if !glyph.export || !has_alternate_layers(glyph) {
            continue;
        }
        let sources = GlyphSources::of(font, designspace, &glyph.name)?;
        if !sources.alternates.is_empty() {
            out.push(sources);
        }
    }
    Ok(out)
}

/// The alternate glyphs of `font` in `designspace`, sorted by name.
pub fn bracket_glyphs(font: &Font, designspace: &Designspace) -> Result<Vec<MadeGlyph>, CompileError> {
    let mut made = Vec::new();
    for sources in alternate_sources(font, designspace)? {
        for set in 0..sources.alternates.len() {
            made.push(MadeGlyph {
                name: bracket_glyph_name(&sources.glyph, set.saturating_add(1)),
                source: MadeFrom::Alternate { glyph: sources.glyph.clone(), set },
            });
        }
    }
    made.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(made)
}

/// The overlay's input: one rule per alternate set of every exported glyph, in glyph order then
/// set order, each substituting the set's alternate glyph for its glyph inside the set's box.
pub fn alternate_rules(font: &Font, designspace: &Designspace) -> Result<Vec<OverlayRule<Substitutions>>, CompileError> {
    let mut rules = Vec::new();
    for sources in alternate_sources(font, designspace)? {
        for (set, alternate) in sources.alternates.iter().enumerate() {
            let region = alternate.ranges.to_box(designspace)?;
            let name = bracket_glyph_name(&sources.glyph, set.saturating_add(1));
            rules.push(OverlayRule { boxes: vec![region], payload: vec![(sources.glyph.clone(), name)] });
        }
    }
    Ok(rules)
}

/// The feature the font's Feature for Feature Variations parameter names, [`DEFAULT_FEATURE`]
/// when it names none; a value that is not a feature tag gives the default and a warning.
pub fn feature_tag(font: &Font, warnings: &mut Vec<String>) -> Tag {
    let default = Tag::new(b"rvrn");
    let Some(value) = font.info.params.get(FEATURE_PARAMETER) else { return default };
    let text = value.as_str().map(str::trim).unwrap_or_default();
    let tag = (!text.is_empty() && text.len() <= 4 && text.bytes().all(|b| (0x20..=0x7e).contains(&b)))
        .then(|| Tag::new_checked(text.as_bytes()).ok())
        .flatten();
    match tag {
        Some(tag) => tag,
        None => {
            warnings.push(format!("the {FEATURE_PARAMETER} parameter does not name a feature tag; alternate layers were put in `{DEFAULT_FEATURE}`"));
            default
        }
    }
}

/// The feature variations of `font` in `designspace`: [`alternate_rules`] overlaid, under the
/// feature [`feature_tag`] gives. Empty when no exported glyph has alternate layers.
pub fn from_font(
    font: &Font,
    designspace: &Designspace,
    axes: &VarAxes,
    glyph_order: &[String],
    warnings: &mut Vec<String>,
) -> Result<Vec<FeatureVariation>, CompileError> {
    let rules = alternate_rules(font, designspace)?;
    if rules.is_empty() {
        return Ok(Vec::new());
    }
    let records = overlay_substitutions(&rules, MAX_OVERLAY_WORK)?;
    feature_variations(&records, axes, feature_tag(font, warnings), glyph_order)
}

/// The tag of the axis at `index` in `axes`.
fn axis_tag(axes: &VarAxes, index: usize) -> Result<Tag, CompileError> {
    axes.axes.get(index).map(|a| a.tag).ok_or_else(|| table_error(format!("a feature variation names axis {index}, which the font does not have")))
}

/// The id of each glyph name in `glyph_order`.
fn glyph_ids(glyph_order: &[String]) -> HashMap<&str, usize> {
    let mut ids = HashMap::with_capacity(glyph_order.len());
    for (i, name) in glyph_order.iter().enumerate() {
        ids.entry(name.as_str()).or_insert(i);
    }
    ids
}

fn glyph_id(ids: &HashMap<&str, usize>, name: &str) -> Result<GlyphId16, CompileError> {
    let id = ids.get(name).ok_or_else(|| table_error(format!("the feature variations substitute `{name}`, which is not in the font")))?;
    u16::try_from(*id).map(GlyphId16::new).map_err(|_| CompileError::TooManyGlyphs(id.saturating_add(1)))
}

/// The feature variations of the overlay's `records` (each payload a list of (glyph, alternate
/// glyph) names), with glyph ids from `glyph_order` and condition axes from `axes`.
pub fn feature_variations(
    records: &[OverlayRecord<Vec<(String, String)>>],
    axes: &VarAxes,
    feature: Tag,
    glyph_order: &[String],
) -> Result<Vec<FeatureVariation>, CompileError> {
    if records.len() > MAX_OVERLAY_RECORDS {
        return Err(table_error(format!("more than {MAX_OVERLAY_RECORDS} feature variation records")));
    }
    let ids = glyph_ids(glyph_order);
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let mut conditions = Vec::with_capacity(record.region.0.len());
        for span in &record.region.0 {
            if !span.min.is_finite() || !span.max.is_finite() {
                return Err(table_error("a feature variation condition is not a usable number"));
            }
            conditions.push((axis_tag(axes, span.axis)?, span.min, span.max));
        }
        conditions.sort_by(|a, b| a.0.cmp(&b.0));
        // The payloads in order, the later one winning a glyph several substitute.
        let mut map: BTreeMap<&str, &str> = BTreeMap::new();
        for (glyph, alternate) in record.payloads.iter().flatten() {
            map.insert(glyph.as_str(), alternate.as_str());
        }
        let substitutions = map.into_iter().map(|(g, a)| Ok((glyph_id(&ids, g)?, glyph_id(&ids, a)?))).collect::<Result<Vec<_>, CompileError>>()?;
        out.push(FeatureVariation { tag: feature, conditions, substitutions });
    }
    Ok(out)
}

/// A `GSUB` holding only `variations`, whose conditions name axes of `axes` and whose glyph ids
/// index `glyph_order`; `None` when there are none. The variations must share one feature.
pub fn build_gsub(variations: &[FeatureVariation], axes: &VarAxes, glyph_order: &[String]) -> Result<Option<Gsub>, CompileError> {
    let Some(first) = variations.first() else { return Ok(None) };
    let feature = first.tag;
    if variations.iter().any(|v| v.tag != feature) {
        return Err(table_error("feature variations for more than one feature"));
    }
    if variations.len() > MAX_OVERLAY_RECORDS {
        return Err(table_error(format!("more than {MAX_OVERLAY_RECORDS} feature variation records")));
    }
    let axis_index: HashMap<Tag, usize> = axes.axes.iter().enumerate().map(|(i, a)| (a.tag, i)).collect();
    let name = |id: GlyphId16| -> Result<String, CompileError> {
        glyph_order.get(usize::from(id.to_u16())).cloned().ok_or_else(|| table_error(format!("glyph id {} is not in the font", id.to_u16())))
    };

    // The second overlay, as fontmake's `varLib` reads the rules back: one rule per record.
    let mut rules = Vec::with_capacity(variations.len());
    for v in variations {
        let mut spans = Vec::with_capacity(v.conditions.len());
        for (tag, min, max) in &v.conditions {
            let axis = axis_index
                .get(tag)
                .copied()
                .ok_or_else(|| table_error(format!("a feature variation names the axis `{tag}`, which the font does not have")))?;
            spans.push(AxisSpan { axis, min: *min, max: *max });
        }
        spans.sort_by_key(|s| s.axis);
        let payload = v.substitutions.iter().map(|(g, a)| Ok((name(*g)?, name(*a)?))).collect::<Result<Substitutions, CompileError>>()?;
        rules.push(OverlayRule { boxes: vec![RegionBox(spans)], payload });
    }
    let records = overlay_substitutions(&rules, MAX_OVERLAY_WORK)?;

    // One lookup per distinct substitution map, in the order of their sorted (glyph, alternate)
    // name pairs (fontTools' `makeSubstitutionsHashable`).
    let sorted = |subs: &Substitutions| -> Substitutions {
        let mut s = subs.clone();
        s.sort();
        s
    };
    let distinct: BTreeSet<Substitutions> = records.iter().flat_map(|r| r.payloads.iter().map(sorted)).collect();
    if distinct.len() > usize::from(u16::MAX) {
        return Err(table_error("more feature variation lookups than a GSUB can hold"));
    }
    let ids = glyph_ids(glyph_order);
    let mut lookup_index: HashMap<Substitutions, u16> = HashMap::with_capacity(distinct.len());
    let mut lookups = Vec::with_capacity(distinct.len());
    let mut var_store = VariationStoreBuilder::new(0);
    for (i, subs) in distinct.into_iter().enumerate() {
        let mut builder = SingleSubBuilder::default();
        for (g, a) in &subs {
            builder.insert(glyph_id(&ids, g)?, glyph_id(&ids, a)?);
        }
        let lookup = LookupBuilder::new_with_lookups(LookupFlag::empty(), None, vec![builder]).build(&mut var_store);
        lookups.push(SubstitutionLookup::Single(lookup));
        lookup_index.insert(subs, u16::try_from(i).map_err(|_| table_error("more feature variation lookups than a GSUB can hold"))?);
    }

    let mut fv_records = Vec::with_capacity(records.len());
    for record in &records {
        let mut conditions: Vec<Condition> = Vec::with_capacity(record.region.0.len());
        let mut spans: Vec<(Tag, &AxisSpan)> =
            record.region.0.iter().map(|s| Ok((axis_tag(axes, s.axis)?, s))).collect::<Result<_, CompileError>>()?;
        spans.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, span) in spans {
            let axis = u16::try_from(span.axis).map_err(|_| table_error("too many axes for a feature variation condition"))?;
            conditions.push(ConditionFormat1::new(axis, F2Dot14::from_f64(span.min), F2Dot14::from_f64(span.max)).into());
        }
        let lookup_indices = record
            .payloads
            .iter()
            .map(|p| lookup_index.get(&sorted(p)).copied().ok_or_else(|| table_error("a feature variation lost its lookup")))
            .collect::<Result<Vec<u16>, _>>()?;
        let condition_set = (!conditions.is_empty()).then(|| ConditionSet::new(conditions));
        let substitution = FeatureTableSubstitution::new(vec![FeatureTableSubstitutionRecord::new(0, Feature::new(None, lookup_indices))]);
        fv_records.push(FeatureVariationRecord::new(condition_set, Some(substitution)));
    }

    let script = Script::new(Some(LangSys::new(vec![0])), Vec::new());
    let script_list = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), script)]);
    let feature_list = FeatureList::new(vec![FeatureRecord::new(feature, Feature::new(None, Vec::new()))]);
    let mut gsub = Gsub::new(script_list, feature_list, SubstitutionLookupList::new(lookups));
    gsub.feature_variations = FeatureVariations::new(fv_records).into();
    Ok(Some(gsub))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Axis, AxisRule, Layer, Path, Shape};

    use super::*;
    use crate::var::VarAxis;

    fn axes(tags: &[&str]) -> VarAxes {
        VarAxes {
            axes: tags
                .iter()
                .map(|t| VarAxis {
                    tag: Tag::new_checked(t.as_bytes()).unwrap(),
                    name: t.to_string(),
                    min: 100.0,
                    default: 100.0,
                    max: 900.0,
                    hidden: false,
                    avar: Vec::new(),
                })
                .collect(),
        }
    }

    fn order(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn record(region: &[(usize, f64, f64)], payloads: &[&[(&str, &str)]]) -> OverlayRecord<Vec<(String, String)>> {
        OverlayRecord {
            region: RegionBox(region.iter().map(|&(axis, min, max)| AxisSpan { axis, min, max }).collect()),
            payloads: payloads.iter().map(|p| p.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()).collect(),
        }
    }

    fn lookup_count(gsub: &Gsub) -> usize {
        gsub.lookup_list.lookups.len()
    }

    fn fv(gsub: &Gsub) -> &FeatureVariations {
        gsub.feature_variations.as_ref().unwrap()
    }

    fn conditions(r: &FeatureVariationRecord) -> Vec<(u16, f32, f32)> {
        r.condition_set
            .as_ref()
            .map(|cs| {
                cs.conditions
                    .iter()
                    .map(|c| match c.as_ref() {
                        Condition::Format1AxisRange(c) => (c.axis_index, c.filter_range_min_value.to_f32(), c.filter_range_max_value.to_f32()),
                        other => panic!("{other:?}"),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn lookups_of(r: &FeatureVariationRecord) -> Vec<u16> {
        r.feature_table_substitution.as_ref().unwrap().substitutions[0].alternate_feature.lookup_list_indices.clone()
    }

    #[test]
    fn bracket_names_and_production_names() {
        assert_eq!(bracket_glyph_name("dollar", 1), "dollar.BRACKET.varAlt01");
        assert_eq!(bracket_glyph_name("a", 12), "a.BRACKET.varAlt12");
        assert_eq!(bracket_production_name("dollar", "uni0024", "dollar.BRACKET.varAlt01").as_deref(), Some("uni0024.BRACKET.varAlt01"));
        assert_eq!(bracket_production_name("cent", "uni00A2", "dollar.BRACKET.varAlt01"), None);
    }

    #[test]
    fn the_feature_parameter_names_the_feature() {
        let mut font = Font::new("Feature");
        let mut warnings = Vec::new();
        assert_eq!(feature_tag(&font, &mut warnings), Tag::new(b"rvrn"));
        Arc::make_mut(&mut font.info).params.set(FEATURE_PARAMETER, fontcraft_model::plist::Value::String("rlig".into()));
        assert_eq!(feature_tag(&font, &mut warnings), Tag::new(b"rlig"));
        assert!(warnings.is_empty());
        Arc::make_mut(&mut font.info).params.set(FEATURE_PARAMETER, fontcraft_model::plist::Value::String("not a tag".into()));
        assert_eq!(feature_tag(&font, &mut warnings), Tag::new(b"rvrn"));
        assert_eq!(warnings.len(), 1);
    }

    /// The records of one glyph with two nested alternate sets (fontc's `AxisRules`-like setup):
    /// the inner region substitutes the later set's glyph, conditions are sorted by axis tag.
    #[test]
    fn records_become_feature_variations() {
        let axes = axes(&["wght", "wdth"]);
        let glyphs = order(&[".notdef", "a", "a.BRACKET.varAlt01", "a.BRACKET.varAlt02"]);
        let records = vec![
            record(&[(1, 0.0, 1.0), (0, 0.8, 1.0)], &[&[("a", "a.BRACKET.varAlt01")], &[("a", "a.BRACKET.varAlt02")]]),
            record(&[(0, 0.5, 0.8)], &[&[("a", "a.BRACKET.varAlt01")]]),
        ];
        let out = feature_variations(&records, &axes, Tag::new(b"rvrn"), &glyphs).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].conditions, vec![(Tag::new(b"wdth"), 0.0, 1.0), (Tag::new(b"wght"), 0.8, 1.0)]);
        assert_eq!(out[0].substitutions, vec![(GlyphId16::new(1), GlyphId16::new(3))]);
        assert_eq!(out[1].substitutions, vec![(GlyphId16::new(1), GlyphId16::new(2))]);
        assert!(out.iter().all(|v| v.tag == Tag::new(b"rvrn")));

        let missing = feature_variations(&records, &axes, Tag::new(b"rvrn"), &order(&[".notdef", "a"]));
        assert!(matches!(missing, Err(CompileError::Table { .. })), "{missing:?}");
    }

    /// Two glyphs whose ranges overlap (wght ≥ 0.5 for `dollar`, ≥ 0.25 for `cent`): the second
    /// overlay gives the inner region both lookups in rule order, lookups are ordered by glyph
    /// name, and the lone feature sits in a DFLT script. Expected values from fontTools'
    /// `addFeatureVariations` on the same rules.
    #[test]
    fn gsub_of_overlapping_variations() {
        let axes = axes(&["wght"]);
        let glyphs = order(&[".notdef", "dollar", "cent", "cent.BRACKET.varAlt01", "dollar.BRACKET.varAlt01"]);
        let (dollar, cent, cent_alt, dollar_alt) = (GlyphId16::new(1), GlyphId16::new(2), GlyphId16::new(3), GlyphId16::new(4));
        let wght = Tag::new(b"wght");
        let variations = vec![
            FeatureVariation { tag: Tag::new(b"rvrn"), conditions: vec![(wght, 0.5, 1.0)], substitutions: vec![(dollar, dollar_alt)] },
            FeatureVariation { tag: Tag::new(b"rvrn"), conditions: vec![(wght, 0.25, 1.0)], substitutions: vec![(cent, cent_alt)] },
        ];
        let gsub = build_gsub(&variations, &axes, &glyphs).unwrap().unwrap();
        assert_eq!(gsub.feature_list.feature_records.len(), 1);
        assert_eq!(gsub.feature_list.feature_records[0].feature_tag, Tag::new(b"rvrn"));
        assert!(gsub.feature_list.feature_records[0].feature.lookup_list_indices.is_empty());
        assert_eq!(gsub.script_list.script_records[0].script_tag, Tag::new(b"DFLT"));
        // Lookups: {cent} before {dollar}, by name.
        assert_eq!(lookup_count(&gsub), 2);
        let records = &fv(&gsub).feature_variation_records;
        assert_eq!(records.len(), 2);
        assert_eq!(conditions(&records[0]), vec![(0, 0.5, 1.0)]);
        assert_eq!(lookups_of(&records[0]), vec![1, 0]);
        assert_eq!(conditions(&records[1]), vec![(0, 0.25, 1.0)]);
        assert_eq!(lookups_of(&records[1]), vec![0]);

        assert_eq!(build_gsub(&[], &axes, &glyphs).unwrap(), None);
        let mixed = vec![variations[0].clone(), FeatureVariation { tag: Tag::new(b"rlig"), ..variations[1].clone() }];
        assert!(build_gsub(&mixed, &axes, &glyphs).is_err());
    }

    /// A set that covers the whole designspace (fontc's `ReverseBracketLayers` has one) gives a
    /// record without a condition set.
    #[test]
    fn a_record_without_conditions_applies_everywhere() {
        let axes = axes(&["wght"]);
        let glyphs = order(&[".notdef", "A", "A.BRACKET.varAlt01"]);
        let variations =
            vec![FeatureVariation { tag: Tag::new(b"rvrn"), conditions: Vec::new(), substitutions: vec![(GlyphId16::new(1), GlyphId16::new(2))] }];
        let gsub = build_gsub(&variations, &axes, &glyphs).unwrap().unwrap();
        let records = &fv(&gsub).feature_variation_records;
        assert_eq!(records.len(), 1);
        assert!(records[0].condition_set.is_none());
        assert_eq!(lookups_of(&records[0]), vec![0]);
    }

    /// Two masters on wght (400, 1000) and `a` with an alternate layer from 600 in each master;
    /// `b` with one that does not export. Needs the designspace and glyph sources (M9.2, M9.3).
    fn alternate_font() -> Font {
        let mut font = Font::new("Alternates");
        Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        let mut bold = font.masters[0].clone();
        bold.id = "m02".into();
        bold.name = "Bold".into();
        bold.location = vec![1000.0];
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
        Arc::make_mut(&mut font.masters).push(bold);
        let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
        for n in names {
            let mut g = font.glyph_mut(&n).unwrap();
            let mut l = g.layers[0].clone();
            l.id = "m02".into();
            l.master = "m02".into();
            g.layers.push(l);
        }
        for (glyph, export) in [("a", true), ("b", false)] {
            let mut g = font.glyph_mut(glyph).unwrap();
            g.export = export;
            for master in ["m01", "m02"] {
                let mut l = Layer::new(master, 600.0);
                l.id = format!("{glyph}-{master}-alt");
                l.info.attrs.axis_rules = vec![AxisRule { min: Some(600.0), max: None }];
                l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
                g.layers.push(l);
            }
        }
        font
    }

    #[test]
    fn alternate_layers_become_glyphs_and_a_feature_variation() {
        let font = alternate_font();
        let designspace = Designspace::new(&font).unwrap();
        let made = bracket_glyphs(&font, &designspace).unwrap();
        assert_eq!(made, vec![MadeGlyph { name: "a.BRACKET.varAlt01".into(), source: MadeFrom::Alternate { glyph: "a".into(), set: 0 } }]);

        let rules = alternate_rules(&font, &designspace).unwrap();
        assert_eq!(rules.len(), 1);
        // 600 on 400…1000 normalizes to 1/3.
        let span = rules[0].boxes[0].0[0];
        assert_eq!(span.axis, 0);
        assert!((span.min - 1.0 / 3.0).abs() < 1e-9 && span.max == 1.0, "{span:?}");

        let glyphs: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).chain(made.iter().map(|m| m.name.clone())).collect();
        let axes = VarAxes {
            axes: vec![VarAxis {
                tag: Tag::new(b"wght"),
                name: "Weight".into(),
                min: 400.0,
                default: 400.0,
                max: 1000.0,
                hidden: false,
                avar: Vec::new(),
            }],
        };
        let mut warnings = Vec::new();
        let variations = from_font(&font, &designspace, &axes, &glyphs, &mut warnings).unwrap();
        assert_eq!(variations.len(), 1);
        assert_eq!(variations[0].tag, Tag::new(b"rvrn"));
        let a = glyphs.iter().position(|g| g == "a").unwrap();
        let alt = glyphs.iter().position(|g| g == "a.BRACKET.varAlt01").unwrap();
        assert_eq!(variations[0].substitutions, vec![(GlyphId16::new(a as u16), GlyphId16::new(alt as u16))]);
        let gsub = build_gsub(&variations, &axes, &glyphs).unwrap().unwrap();
        assert_eq!(conditions(&fv(&gsub).feature_variation_records[0]), vec![(0, F2Dot14::from_f64(1.0 / 3.0).to_f32(), 1.0)]);
    }
}
