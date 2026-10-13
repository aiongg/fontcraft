//! The mark writer's input for a static master build (`compile.markFeatureWriter`,
//! `compile.gdefClasses`, `compile.gdefLigatureCarets`): the master's anchors after propagation
//! and the generated features the source asks for. The layout stage ([`crate::layout`]) runs
//! the writer and hands its lookups to the feature compiler.
//!
//! - Anchors: the effective anchors of `fontcraft_components::propagate` on the compiled master,
//!   with categories from the compile's glyph data, for the reference's format; for a UFO or
//!   designspace source only when its ufo2ft filter list has a `propagateAnchors` entry, and then
//!   only for the glyphs the entry reaches, as both reference compilers decide. Every other glyph
//!   gives its own anchors.
//! - Classes: worked out from the categories and anchors for the reference's format; for a UFO,
//!   the stated `public.openTypeCategories` (`fontcraft_model::ufo_classes::stated`), those the
//!   reader left off the glyphs included.
//! - Which features: the feature-writer setting (as for the kerning), less the features the code
//!   declares without an `# Automatic Code` marker (`implicit.markerMissing`).

use std::collections::HashMap;

use fontcraft_components::propagate::{EffectiveAnchor, effective_anchors_for_master_with};
use fontcraft_features::FEATURE_WRITERS_KEY;
use fontcraft_features::marks::{MarkAnchor, MarkGlyph, MarkInput, MarkTags, MasterAnchors, mark_tags};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, ORIGIN_ANCHOR, Resolved, ResolvedGlyph, is_ufo_family, ufo_classes};

use crate::components_opt::{Scope, listed_filter, reaches};
use crate::layout::OtherMaster;

/// The ufo2ft filter that propagates anchors, as [`listed_filter`] spells it.
const PROPAGATE_FILTER: &str = "propagateAnchors";
/// Anchors passed on from components, by glyph.
type Propagated = HashMap<String, Vec<EffectiveAnchor>>;

/// The mark writer's input for `resolved`'s master and each of `others` (a variable font's other
/// masters), whose anchors each glyph carries beside the first master's.
pub(crate) fn input(
    font: Option<&Font>,
    data: Option<&GlyphData>,
    resolved: &Resolved,
    order: &[ResolvedGlyph],
    language_systems: Vec<(String, String)>,
    others: &[OtherMaster],
    warnings: &mut Vec<String>,
) -> MarkInput {
    let ufo = is_ufo_family(resolved.format);
    let scopes = match font {
        Some(font) if ufo => listed_filter(font, &resolved.master, PROPAGATE_FILTER),
        _ => vec![Scope::All],
    };
    let first = propagated(font, data, &resolved.master, &scopes, warnings);
    // The other masters' warnings repeat the first master's.
    let others: Vec<(&OtherMaster, Option<Propagated>)> =
        others.iter().map(|o| (o, propagated(font, data, o.id, &scopes, &mut Vec::new()))).collect();
    let stated = match font {
        Some(font) if ufo => GlyphData::new()
            .map(|data| ufo_classes::stated(font, &resolved.master, &ufo_classes::rule_classes(font, &resolved.master, &data)))
            .unwrap_or_default(),
        _ => HashMap::new(),
    };
    let anchors_of = |g: &ResolvedGlyph, map: Option<&Propagated>| -> Vec<MarkAnchor> {
        match map.filter(|_| reaches(&scopes, &g.name)).and_then(|map| map.get(&g.name)) {
            Some(list) => {
                let anchors = list.iter().map(|a| MarkAnchor { name: a.name.clone(), x: a.x, y: a.y }).collect();
                if ufo { anchors } else { origin_adjusted(anchors) }
            }
            None => g.anchors.iter().map(|(name, p)| MarkAnchor { name: name.clone(), x: p.x, y: p.y }).collect(),
        }
    };
    let glyphs = order
        .iter()
        .map(|g| {
            let mut anchors = vec![MasterAnchors { master: resolved.master.clone(), anchors: anchors_of(g, first.as_ref()) }];
            for (other, map) in &others {
                let own = other.glyphs.get(g.name.as_str()).copied().unwrap_or(g);
                anchors.push(MasterAnchors { master: other.id.to_string(), anchors: anchors_of(own, map.as_ref()) });
            }
            let opentype_category = if ufo { stated.get(&g.name).cloned() } else { None };
            MarkGlyph {
                name: g.name.clone(),
                codepoints: g.unicodes.clone(),
                category: g.category.clone(),
                sub_category: g.sub_category.clone(),
                opentype_category,
                anchors,
            }
        })
        .collect();
    MarkInput { glyphs, explicit_classes: ufo, language_systems }
}

/// The anchors of `master`'s glyphs passed on from their components, or `None` when they are not
/// passed on (each glyph then uses its own).
fn propagated(font: Option<&Font>, data: Option<&GlyphData>, master: &str, scopes: &[Scope], warnings: &mut Vec<String>) -> Option<Propagated> {
    let font = font.filter(|_| !scopes.is_empty())?;
    match effective_anchors_for_master_with(font, data, master) {
        Ok(master) => {
            if let Some((glyph, e)) = master.failed.first() {
                let more = master.failed.len().saturating_sub(1);
                let others = if more == 0 { String::new() } else { format!(" (and {more} more glyphs)") };
                warnings.push(format!(
                    "anchors were not passed on from components to `{glyph}`{others}, so it uses its own anchors for mark attachment: {e}"
                ));
            }
            if let Some((glyph, e)) = master.smart_fallbacks.first() {
                let more = master.smart_fallbacks.len().saturating_sub(1);
                let others = if more == 0 { String::new() } else { format!(" (and {more} more)") };
                warnings.push(format!(
                    "a smart component in `{glyph}`{others} passes on its glyph's master-layer anchors for mark attachment, not anchors at its values: {e}"
                ));
            }
            Some(master.anchors)
        }
        Err(e) => {
            warnings.push(format!("anchors were not passed on from components for mark attachment, so each glyph's own anchors were used: {e}"));
            None
        }
    }
}

/// `anchors` moved by minus the first `*origin` anchor's position, without the `*origin` anchors.
fn origin_adjusted(anchors: Vec<MarkAnchor>) -> Vec<MarkAnchor> {
    let Some((ox, oy)) = anchors.iter().find(|a| a.name == ORIGIN_ANCHOR).map(|a| (a.x, a.y)) else { return anchors };
    anchors.into_iter().filter(|a| a.name != ORIGIN_ANCHOR).map(|a| MarkAnchor { x: a.x - ox, y: a.y - oy, ..a }).collect()
}

/// The generated features the source asks for: its feature-writer setting, less the features
/// `declared_without_marker` names (declared in the code without an `# Automatic Code` marker).
/// The setting's warnings are given only when the font has anchors.
pub(crate) fn tags(font: &Font, master: &str, any_anchor: bool, declared_without_marker: &[&str], warnings: &mut Vec<String>) -> MarkTags {
    // The font's setting first, then the master's, as for the kerning.
    let setting = font.info.user_data.get(FEATURE_WRITERS_KEY).or_else(|| font.master(master).ok()?.user_data.get(FEATURE_WRITERS_KEY));
    let mut quiet = Vec::new();
    let notes = if any_anchor { &mut *warnings } else { &mut quiet };
    let mut tags = mark_tags(setting, notes);
    for (tag, flag) in
        [("mark", &mut tags.mark), ("mkmk", &mut tags.mkmk), ("abvm", &mut tags.abvm), ("blwm", &mut tags.blwm), ("curs", &mut tags.curs)]
    {
        if declared_without_marker.contains(&tag) {
            *flag = false;
        }
    }
    tags
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
