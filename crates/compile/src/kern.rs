//! The generated pair kerning of a static master build (`kerning.implicitKernFeature`): the
//! compiled master's kerning written to `GPOS` by `fontcraft_features`, as fontc writes it for a
//! source without feature code.
//!
//! Feature code is compiled from M8. Until then a compile writes the generated kerning unless the
//! source has a hand-written `kern` feature: the reference compilers then write that feature,
//! with the generated lookups only where it holds `# Automatic Code`, and both wait for M8. A
//! `kern` feature the editor writes itself, or one with only comments, is not hand-written. The
//! other features are not written either; the compile names them once. A rule set past one of
//! the writer's caps leaves the kerning out with a warning, and the font is still written.
//!
//! A glyph of the mark class is one of category `Mark` and subcategory `Nonspacing` or `Spacing
//! Combining` (after the glyph database filled what the glyph does not state); it is spacing when
//! it has an advance.

use fontcraft_features::{FEATURE_WRITERS_KEY, KernGlyph, KernLimits, KernRequest, MarkSpacing, kern_feature, kern_tags};
use fontcraft_model::{Font, Resolved, ResolvedGlyph};
use fontcraft_spacing::kerning::{Direction, KernIndex};
use write_fonts::tables::gpos::Gpos;

/// The code of a snippet without its comments.
fn without_comments(code: &str) -> impl Iterator<Item = &str> {
    code.lines().map(|line| line.split_once('#').map_or(line, |(code, _)| code))
}

/// The tags of the `feature <tag> {` blocks in `code`.
fn feature_blocks(code: &str) -> Vec<&str> {
    let tokens: Vec<&str> = without_comments(code)
        .flat_map(|line| line.split_inclusive(['{', '}', ';']))
        .flat_map(|part| {
            let (word, brace) = match part.strip_suffix('{') {
                Some(word) => (word, Some("{")),
                None => (part.trim_end_matches(['}', ';']), None),
            };
            word.split_whitespace().chain(brace)
        })
        .collect();
    tokens.windows(3).filter_map(|w| matches!(w, ["feature", _, "{"]).then(|| w.get(1).copied()).flatten()).collect()
}

/// What the source's feature code is, for a compile that does not compile it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FeatureCode {
    /// A `kern` feature not written by the editor, with code besides comments.
    pub(crate) hand_written_kern: bool,
    /// The features with code, each once, in source order.
    pub(crate) features: Vec<String>,
}

pub(crate) fn feature_code(font: &Font) -> FeatureCode {
    let mut out = FeatureCode::default();
    let mut add = |tag: &str| {
        if !out.features.iter().any(|t| t == tag) {
            out.features.push(tag.to_string());
        }
    };
    let f = &font.features;
    for s in f.prefixes.iter().chain(&f.classes) {
        if s.disabled {
            continue;
        }
        for tag in feature_blocks(&s.code) {
            add(tag);
            out.hand_written_kern |= tag == "kern" && !s.automatic;
        }
    }
    for s in &f.features {
        if s.disabled || without_comments(&s.code).all(|l| l.trim().is_empty()) {
            continue;
        }
        add(&s.name);
        out.hand_written_kern |= s.name == "kern" && !s.automatic;
    }
    out
}

/// Whether the source has a hand-written `kern` feature, which keeps the generated kerning out of
/// a compile before M8.
pub fn has_hand_written_kern(font: &Font) -> bool {
    feature_code(font).hand_written_kern
}

#[cfg(test)]
thread_local! {
    /// Smaller caps for a test's compiles on its own thread.
    pub(crate) static TEST_LIMITS: std::cell::Cell<Option<KernLimits>> = const { std::cell::Cell::new(None) };
}

fn limits() -> KernLimits {
    #[cfg(test)]
    if let Some(limits) = TEST_LIMITS.with(std::cell::Cell::get) {
        return limits;
    }
    KernLimits::default()
}

fn mark(g: &ResolvedGlyph) -> Option<MarkSpacing> {
    let is_mark = g.category.as_deref() == Some("Mark") && matches!(g.sub_category.as_deref(), Some("Nonspacing" | "Spacing Combining"));
    is_mark.then_some(if g.width == 0.0 { MarkSpacing::NonSpacing } else { MarkSpacing::Spacing })
}

/// The `GPOS` table of `resolved`'s master, from `font`'s kerning, for the glyphs `order` (the
/// final glyph order). `None` when there is no kerning to write.
pub(crate) fn generate(font: &Font, resolved: &Resolved, order: &[ResolvedGlyph], warnings: &mut Vec<String>) -> Option<Gpos> {
    let index = KernIndex::new(font);
    let has_kerning =
        [Direction::Ltr, Direction::Rtl].into_iter().any(|d| index.table(d, &resolved.master).is_ok_and(|t| t.pairs().next().is_some()));
    let code = feature_code(font);
    if !code.features.is_empty() {
        warnings.push(format!("feature code is compiled from M8: these features were not written to the font: {}", code.features.join(", ")));
    }
    if !has_kerning {
        return None;
    }
    if code.hand_written_kern {
        warnings.push("the font has a hand-written kern feature, which is compiled from M8: the kerning was not written to the font".to_string());
        return None;
    }
    // The font's setting first: a designspace's lib, a `.glyphs` font's user data. Then the
    // master's: a UFO's lib, where glyphsLib also puts a `.glyphs` master's user data.
    let setting = font.info.user_data.get(FEATURE_WRITERS_KEY).or_else(|| font.master(&resolved.master).ok()?.user_data.get(FEATURE_WRITERS_KEY));
    let tags = kern_tags(setting, warnings);
    let glyphs: Vec<KernGlyph> = order.iter().map(|g| KernGlyph { name: &g.name, codepoints: &g.unicodes, mark: mark(g) }).collect();
    let request = KernRequest { font, index: &index, master: &resolved.master, glyphs: &glyphs, tags, limits: limits() };
    match kern_feature(&request) {
        Ok(kern) => {
            warnings.extend(kern.warnings);
            kern.gpos
        }
        Err(e) => {
            warnings.push(format!("the kerning was not written to the font: {e}"));
            None
        }
    }
}
