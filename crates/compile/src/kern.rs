//! The kern writer's input for a static master build (`kerning.implicitKernFeature`): the glyphs
//! of the compiled font with their marks, and the generated features the source's
//! feature-writer setting asks for. The layout stage ([`crate::layout`]) runs the writer
//! (`fontcraft_features::kern_lookups`) next to the feature code, as fontc does.
//!
//! A glyph of the mark class is one of category `Mark` and subcategory `Nonspacing` or `Spacing
//! Combining` (after the glyph database filled what the glyph does not state), or one the code's
//! `table GDEF` block puts in the mark class when it has one; it is spacing when it has an
//! advance.

use std::collections::BTreeMap;

use fontcraft_fea::{GlyphClass, GlyphId16};
use fontcraft_features::{FEATURE_WRITERS_KEY, KernGlyph, KernLimits, KernTags, MarkSpacing, kern_tags};
use fontcraft_model::{Font, ResolvedGlyph};

#[cfg(test)]
thread_local! {
    /// Smaller caps for a test's compiles on its own thread.
    pub(crate) static TEST_LIMITS: std::cell::Cell<Option<KernLimits>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn limits() -> KernLimits {
    #[cfg(test)]
    if let Some(limits) = TEST_LIMITS.with(std::cell::Cell::get) {
        return limits;
    }
    KernLimits::default()
}

fn spacing(g: &ResolvedGlyph) -> MarkSpacing {
    if g.width == 0.0 { MarkSpacing::NonSpacing } else { MarkSpacing::Spacing }
}

fn mark(g: &ResolvedGlyph) -> Option<MarkSpacing> {
    let is_mark = g.category.as_deref() == Some("Mark") && matches!(g.sub_category.as_deref(), Some("Nonspacing" | "Spacing Combining"));
    is_mark.then(|| spacing(g))
}

/// The glyphs of `order` (the final glyph order) as the kern writer reads them. `sources` maps an
/// exported name to the source name it was renamed from: the kerning names glyphs as the source
/// does. `gdef` is the code's `table GDEF` classes, which decide the marks when given.
pub(crate) fn glyphs<'a>(
    order: &'a [ResolvedGlyph],
    sources: &std::collections::HashMap<&'a str, &'a str>,
    gdef: Option<&BTreeMap<GlyphId16, GlyphClass>>,
) -> Vec<KernGlyph<'a>> {
    order
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let mark = match gdef {
                Some(classes) => {
                    let gid = u16::try_from(i).ok().map(GlyphId16::new);
                    let is_mark = gid.and_then(|gid| classes.get(&gid)) == Some(&GlyphClass::Mark);
                    is_mark.then(|| spacing(g))
                }
                None => mark(g),
            };
            KernGlyph { name: sources.get(g.name.as_str()).copied().unwrap_or(&g.name), codepoints: &g.unicodes, mark }
        })
        .collect()
}

/// The kern features the source asks for: its feature-writer setting (the font's first, then the
/// master's: a UFO's lib, where glyphsLib also puts a `.glyphs` master's user data), less the
/// features `declared_without_marker` names.
pub(crate) fn tags(font: &Font, master: &str, declared_without_marker: &[&str], warnings: &mut Vec<String>) -> KernTags {
    let setting = font.info.user_data.get(FEATURE_WRITERS_KEY).or_else(|| font.master(master).ok()?.user_data.get(FEATURE_WRITERS_KEY));
    let mut tags = kern_tags(setting, warnings);
    tags.kern &= !declared_without_marker.contains(&"kern");
    tags.dist &= !declared_without_marker.contains(&"dist");
    tags
}
