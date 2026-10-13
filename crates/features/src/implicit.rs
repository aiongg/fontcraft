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

use crate::marks::{GeneratedMarks, MarkRequest, MarkRules, Point16};
use crate::var_layout::VarLayout;
use crate::{FeatureError, KernRequest};

/// What the implicit features are generated from. `None` leaves that writer out (its tags are
/// off, or the source has no data for it).
pub struct ImplicitRequest<'a> {
    pub kern: Option<&'a KernRequest<'a>>,
    pub marks: Option<&'a MarkRequest<'a>>,
    /// A variable font: its masters, and the mark request of each master after the origin, in
    /// the layout's order (`marks` is the origin's). The mark positions vary with the masters.
    pub master_marks: Option<(&'a VarLayout, &'a [MarkRequest<'a>])>,
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
        Some(Ok(marks)) => match req.master_marks {
            Some((layout, others)) => match from_master_marks(layout, &marks, others) {
                Ok(lookups) => lookups,
                Err(e) => {
                    warnings.push(format!("the mark positions do not vary in the variable font, they are the origin's: {e}"));
                    from_marks(&marks)
                }
            },
            None => from_marks(&marks),
        },
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

/// The origin's mark lookups (`origin`) with every position varying over the masters: the writer
/// runs for each other master (`others`, in `layout`'s order), and each position's values become
/// the origin's plus deltas. Refused when a master's lookups do not have the origin's rules and
/// glyphs (anchors missing in a master).
pub fn from_master_marks(layout: &VarLayout, origin: &GeneratedMarks, others: &[MarkRequest]) -> Result<GeneratedLookups, FeatureError> {
    let mut out = from_marks(origin);
    if out.lookups.is_empty() {
        return Ok(out);
    }
    let (origin_keys, origin_values) = positions(&mut out);
    let mut values: Vec<Vec<f64>> = vec![origin_values.iter().map(|v| f64::from(**v)).collect()];
    for req in others {
        let mut quiet = Vec::new();
        let mut theirs = from_marks(&crate::marks::generate_marks(req, &mut quiet)?);
        let (keys, list) = positions(&mut theirs);
        if keys != origin_keys {
            return Err(FeatureError::Unsupported(format!("master `{}` attaches other glyphs or anchors than the origin", req.master)));
        }
        values.push(list.iter().map(|v| f64::from(**v)).collect());
    }
    let resolved = layout.resolve(&values)?;
    let (_, mut slots) = metrics(&mut out);
    for (slot, value) in slots.iter_mut().zip(&resolved) {
        **slot = layout.metric(value);
    }
    Ok(out)
}

/// Where each position of `lookups` sits (lookup, glyph and slot), and the positions' default
/// values in the same order.
fn positions(lookups: &mut GeneratedLookups) -> (Vec<(usize, u16, u8)>, Vec<&i16>) {
    let (keys, metrics) = metrics(lookups);
    (keys, metrics.into_iter().map(|m| &m.default).collect())
}

/// Every anchor coordinate of `lookups` in a fixed order, with keys that tell two masters' rules
/// apart: an absent anchor (a ligature component or a cursive side) is a key without a metric.
fn metrics(lookups: &mut GeneratedLookups) -> (Vec<(usize, u16, u8)>, Vec<&mut Metric>) {
    let mut keys = Vec::new();
    let mut out: Vec<&mut Metric> = Vec::new();
    fn push<'m>(keys: &mut Vec<(usize, u16, u8)>, out: &mut Vec<&'m mut Metric>, key: (usize, u16, u8), a: Option<&'m mut Anchor>) {
        keys.push(key);
        if let Some(a) = a {
            out.push(&mut a.x);
            out.push(&mut a.y);
        }
    }
    for (i, lookup) in lookups.lookups.iter_mut().enumerate() {
        match &mut lookup.rules {
            GeneratedRules::MarkToBase { marks, bases } | GeneratedRules::MarkToMark { marks, bases } => {
                for (g, a) in marks.iter_mut() {
                    push(&mut keys, &mut out, (i, g.to_u16(), 0), Some(a));
                }
                for (g, a) in bases.iter_mut() {
                    push(&mut keys, &mut out, (i, g.to_u16(), 1), Some(a));
                }
            }
            GeneratedRules::MarkToLigature { marks, ligatures } => {
                for (g, a) in marks.iter_mut() {
                    push(&mut keys, &mut out, (i, g.to_u16(), 0), Some(a));
                }
                for (g, components) in ligatures.iter_mut() {
                    for a in components.iter_mut() {
                        push(&mut keys, &mut out, (i, g.to_u16(), if a.is_some() { 2 } else { 3 }), a.as_mut());
                    }
                }
            }
            GeneratedRules::Cursive { glyphs } => {
                for (g, entry, exit) in glyphs.iter_mut() {
                    push(&mut keys, &mut out, (i, g.to_u16(), if entry.is_some() { 4 } else { 5 }), entry.as_mut());
                    push(&mut keys, &mut out, (i, g.to_u16(), if exit.is_some() { 6 } else { 7 }), exit.as_mut());
                }
            }
            GeneratedRules::Pair { .. } => {}
        }
    }
    (keys, out)
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
