//! The generated pair kerning of a static master build (`kerning.implicitKernFeature`): the
//! compiled master's kerning written to `GPOS` by `fontcraft_features`, as fontc writes it for a
//! source without feature code.
//!
//! Feature code is compiled from M8. Until then a compile writes the generated kerning unless the
//! source's feature code has a `kern` feature: the reference compilers then write that feature,
//! with the generated lookups only where it holds `# Automatic Code`, and both wait for M8. A
//! `kern` feature the editor writes itself, or one with only comments, does not count. A
//! top-level `include` of another file may hold a `kern` feature too: the compile does not read
//! the file, so it leaves the generated kerning out and names the file. The other features are
//! not written either; the compile names them once. A rule set past one of the writer's caps
//! leaves the kerning out with a warning, and the font is still written.
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

/// The tokens of `code` without its comments: the words, and each of `{ } ( ) ;` on its own.
fn tokens(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for line in without_comments(code) {
        let mut start = None;
        for (i, c) in line.char_indices() {
            let punctuation = matches!(c, '{' | '}' | '(' | ')' | ';');
            if c.is_whitespace() || punctuation {
                if let Some(s) = start.take() {
                    out.extend(line.get(s..i));
                }
                if punctuation {
                    out.extend(line.get(i..i + c.len_utf8()));
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(s) = start {
            out.extend(line.get(s..));
        }
    }
    out
}

/// What the top level of a snippet's code holds.
#[derive(Default)]
struct TopLevel<'a> {
    /// The tags of the `feature <tag> [useExtension] {` blocks with code besides comments.
    features: Vec<&'a str>,
    /// The files the `include(…)` statements name.
    includes: Vec<String>,
}

fn top_level(code: &str) -> TopLevel<'_> {
    let t = tokens(code);
    let mut out = TopLevel::default();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(&token) = t.get(i) {
        match token {
            "{" => depth = depth.saturating_add(1),
            "}" => depth = depth.saturating_sub(1),
            "feature" if depth == 0 => {
                let tag = t.get(i + 1).copied().filter(|tag| !matches!(*tag, "{" | "}" | "(" | ")" | ";"));
                let open = if t.get(i + 2) == Some(&"useExtension") { i + 3 } else { i + 2 };
                if let (Some(tag), Some(&"{")) = (tag, t.get(open)) {
                    if t.get(open + 1).is_some_and(|next| *next != "}") {
                        out.features.push(tag);
                    }
                    i = open;
                    continue;
                }
            }
            "include" if depth == 0 && t.get(i + 1) == Some(&"(") => {
                let path: Vec<&str> = t.iter().skip(i + 2).take_while(|t| **t != ")").copied().collect();
                out.includes.push(path.join(" "));
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// What the source's feature code is, for a compile that does not compile it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FeatureCode {
    /// A `kern` feature not written by the editor, with code besides comments.
    pub(crate) hand_written_kern: bool,
    /// The files the top level of the enabled code includes, each once, in source order.
    pub(crate) includes: Vec<String>,
    /// The features with code, each once, in source order.
    pub(crate) features: Vec<String>,
}

pub(crate) fn feature_code(font: &Font) -> FeatureCode {
    let mut out = FeatureCode::default();
    let f = &font.features;
    for s in f.prefixes.iter().chain(&f.classes) {
        if s.disabled {
            continue;
        }
        let top = top_level(&s.code);
        for tag in top.features {
            if !out.features.iter().any(|t| t == tag) {
                out.features.push(tag.to_string());
            }
            out.hand_written_kern |= tag == "kern" && !s.automatic;
        }
        for path in top.includes {
            if !out.includes.contains(&path) {
                out.includes.push(path);
            }
        }
    }
    for s in &f.features {
        if s.disabled || without_comments(&s.code).all(|l| l.trim().is_empty()) {
            continue;
        }
        if !out.features.contains(&s.name) {
            out.features.push(s.name.clone());
        }
        out.hand_written_kern |= s.name == "kern" && !s.automatic;
    }
    out
}

/// The features of the enabled feature code that the editor did not write, with code besides
/// comments, each once, in source order. Each keeps the generated feature of its tag out of a
/// compile before M8 (`crate::marks`).
pub(crate) fn hand_written_features(font: &Font) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let f = &font.features;
    for s in f.prefixes.iter().chain(&f.classes) {
        if s.disabled || s.automatic {
            continue;
        }
        for tag in top_level(&s.code).features {
            if !out.iter().any(|t| t == tag) {
                out.push(tag.to_string());
            }
        }
    }
    for s in &f.features {
        if s.disabled || s.automatic || without_comments(&s.code).all(|l| l.trim().is_empty()) {
            continue;
        }
        if !out.contains(&s.name) {
            out.push(s.name.clone());
        }
    }
    out
}

/// The `languagesystem <script> <language>;` statements at the top level of the enabled prefixes
/// and classes, as (script, language), each once, in source order.
pub(crate) fn language_systems(font: &Font) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let f = &font.features;
    for s in f.prefixes.iter().chain(&f.classes) {
        if s.disabled {
            continue;
        }
        let t = tokens(&s.code);
        let mut depth = 0usize;
        for (i, token) in t.iter().enumerate() {
            match *token {
                "{" => depth = depth.saturating_add(1),
                "}" => depth = depth.saturating_sub(1),
                "languagesystem" if depth == 0 => {
                    if let (Some(script), Some(lang), Some(&";")) = (t.get(i + 1), t.get(i + 2), t.get(i + 3)) {
                        let system = (script.to_string(), lang.to_string());
                        if !out.contains(&system) {
                            out.push(system);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Whether the source's feature code has a `kern` feature, or includes a file that may hold one:
/// either keeps the generated kerning out of a compile before M8.
pub fn has_kern_feature_code(font: &Font) -> bool {
    let code = feature_code(font);
    code.hand_written_kern || !code.includes.is_empty()
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
    if !code.includes.is_empty() {
        warnings.push(format!(
            "the feature code includes {}, which may hold a kern feature and is compiled from M8: the kerning was not written to the font",
            code.includes.join(", ")
        ));
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
