//! The implicit features in the compiler's neutral form (M8.7): the generated kerning
//! ([`crate::kern_lookups`]) and mark attachment ([`crate::marks::generate_marks`]'s
//! [`GeneratedMarks`]) turned into [`fontcraft_fea::GeneratedLookups`] for fea-rs to place at
//! `# Automatic Code` or after the code.
//!
//! Rows: `compile.featureWriterModes`, `compile.skipFeatures`, `fontInfo.features.implicit`,
//! `fontInfo.features.automaticCodePlaceholder`, `implicit.markerMissing`.
//!
//! The kern writer reads the code's language systems and the scripts its substitutions reach
//! ([`crate::KernRequest::language_systems`], [`crate::KernRequest::gsub`]), ported from fontc's
//! `properties.rs`; the mark writer registers its features for the code's language systems.

use fontcraft_fea::{Anchor, GeneratedLookup, GeneratedLookups, GeneratedRules, GlyphClass, LookupFlags, Metric};
use write_fonts::tables::gdef::GlyphClassDef;

use crate::KernRequest;
use crate::marks::{GeneratedMarks, MarkRequest, MarkRules, Point16};

/// What the implicit features are generated from. `None` leaves that writer out (its tags are
/// off, or the source has no data for it).
pub struct ImplicitRequest<'a> {
    pub kern: Option<&'a KernRequest<'a>>,
    pub marks: Option<&'a MarkRequest<'a>>,
}

/// The implicit features of one master, with the warnings of the writers.
#[derive(Debug, Default)]
pub struct Implicit {
    pub lookups: GeneratedLookups,
    pub warnings: Vec<String>,
}

/// Runs the kern and mark writers and returns their lookups in neutral form, kerning lookups
/// after the cursive ones and before the other mark lookups, as the reference compilers order
/// them. A writer that fails (a cap reached) is left out with a warning; the other's lookups
/// stay.
pub fn generated(req: &ImplicitRequest) -> Implicit {
    let mut warnings = Vec::new();
    let kern = match req.kern.map(crate::kern_lookups) {
        Some(Ok((lookups, notes))) => {
            warnings.extend(notes);
            lookups
        }
        Some(Err(e)) => {
            warnings.push(format!("the kerning was not written to the font: {e}"));
            GeneratedLookups::default()
        }
        None => GeneratedLookups::default(),
    };
    let marks = match req.marks.map(|m| crate::marks::generate_marks(m, &mut warnings)) {
        Some(Ok(marks)) => from_marks(&marks),
        Some(Err(e)) => {
            warnings.push(format!("mark attachment and the GDEF table were not written to the font: {e}"));
            GeneratedLookups::default()
        }
        None => GeneratedLookups::default(),
    };
    Implicit { lookups: merge(kern, marks), warnings }
}

/// The mark writer's output in neutral form: one lookup per mark lookup, registered for each of
/// its language systems under its feature; positions as fixed [`fontcraft_fea::Metric`]s; the
/// `GDEF` classes and carets carried over.
pub fn from_marks(marks: &GeneratedMarks) -> GeneratedLookups {
    let mut out = GeneratedLookups::default();
    for (index, lookup) in marks.lookups.iter().enumerate() {
        out.lookups.push(GeneratedLookup {
            class: lookup.class.clone(),
            flags: LookupFlags(lookup.flags.to_bits() & !LookupFlags::USE_MARK_FILTERING_SET),
            filter_set: lookup.filter_set.clone(),
            rules: rules(&lookup.rules),
        });
        for (script, language) in &marks.language_systems {
            let key = fontcraft_fea::FeatureKey { feature: lookup.feature, script: *script, language: *language };
            out.features.entry(key).or_default().push(index);
        }
    }
    out.glyph_classes = marks.classes.iter().filter_map(|(g, c)| Some((*g, glyph_class(*c)?))).collect();
    out.carets = marks.carets.iter().map(|(g, carets)| (*g, carets.iter().map(|c| Metric::fixed(*c)).collect())).collect();
    out
}

fn glyph_class(class: GlyphClassDef) -> Option<GlyphClass> {
    match class {
        GlyphClassDef::Base => Some(GlyphClass::Base),
        GlyphClassDef::Ligature => Some(GlyphClass::Ligature),
        GlyphClassDef::Mark => Some(GlyphClass::Mark),
        GlyphClassDef::Component => Some(GlyphClass::Component),
        _ => None,
    }
}

fn anchor((x, y): Point16) -> Anchor {
    Anchor { x: Metric::fixed(x), y: Metric::fixed(y) }
}

fn anchors(list: &[(fontcraft_fea::GlyphId16, Point16)]) -> Vec<(fontcraft_fea::GlyphId16, Anchor)> {
    list.iter().map(|(g, p)| (*g, anchor(*p))).collect()
}

fn rules(rules: &MarkRules) -> GeneratedRules {
    match rules {
        MarkRules::MarkToBase { marks, bases } => GeneratedRules::MarkToBase { marks: anchors(marks), bases: anchors(bases) },
        MarkRules::MarkToMark { marks, bases } => GeneratedRules::MarkToMark { marks: anchors(marks), bases: anchors(bases) },
        MarkRules::MarkToLigature { marks, ligatures } => GeneratedRules::MarkToLigature {
            marks: anchors(marks),
            ligatures: ligatures.iter().map(|(g, slots)| (*g, slots.iter().map(|p| p.map(anchor)).collect())).collect(),
        },
        MarkRules::Cursive { glyphs } => {
            GeneratedRules::Cursive { glyphs: glyphs.iter().map(|(g, entry, exit)| (*g, entry.map(anchor), exit.map(anchor))).collect() }
        }
    }
}

/// The kerning's and the mark writer's lookups in one list: the mark writer's leading cursive
/// lookups, the kerning, then the other mark lookups; feature indices moved to match.
fn merge(kern: GeneratedLookups, marks: GeneratedLookups) -> GeneratedLookups {
    if kern.lookups.is_empty() {
        return GeneratedLookups { glyph_classes: marks.glyph_classes, carets: marks.carets, ..marks };
    }
    let curs = marks.lookups.iter().take_while(|l| matches!(l.rules, GeneratedRules::Cursive { .. })).count();
    let kern_count = kern.lookups.len();
    let mut out = GeneratedLookups { glyph_classes: marks.glyph_classes, carets: marks.carets, ..GeneratedLookups::default() };
    let mut mark_lookups = marks.lookups;
    let rest = mark_lookups.split_off(curs.min(mark_lookups.len()));
    out.lookups = mark_lookups;
    out.lookups.extend(kern.lookups);
    out.lookups.extend(rest);
    for (key, indices) in kern.features {
        out.features.insert(key, indices.into_iter().map(|i| i.saturating_add(curs)).collect());
    }
    for (key, indices) in marks.features {
        let moved = indices.into_iter().map(|i| if i < curs { i } else { i.saturating_add(kern_count) });
        out.features.entry(key).or_default().extend(moved);
    }
    out.append = kern.append;
    out.append.extend(marks.append);
    out
}
