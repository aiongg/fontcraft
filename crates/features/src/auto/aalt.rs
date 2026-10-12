//! Access All Alternates (M8.11).
//!
//! Rows: `autoFeature.aalt`.
//!
//! aalt holds one `feature <tag>;` line per other feature that substitutes glyphs, in sidebar
//! order, and leaves the grouping to the compiler, as the reference's stored code does. A
//! feature substitutes when its own code has a `sub` statement; one that only includes a file
//! or calls a lookup does not count. Disabled features are left out, and so are ccmp and the
//! ligature features, which the corpus's aalt entries never list.
//!
//! The generator sees the entries before Update. It projects the list Update will write:
//! automatic entries take their regenerated code, and features no entry has yet follow in
//! generator order. Update calls [`code_for`] on its merged list instead.

use std::collections::HashSet;

use super::ligatures::Code;
use super::{AutoContext, AutoEntry, Generator};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&ALL_ALTERNATES]
}

/// Features aalt never lists.
const LEFT_OUT: [&str; 6] = ["aalt", "ccmp", "liga", "dlig", "hlig", "rlig"];

/// The aalt code for `features`, given as (tag, code, disabled) in sidebar order; `None` when
/// no feature qualifies.
pub(crate) fn code_for<'a>(features: impl IntoIterator<Item = (&'a str, &'a str, bool)>) -> Result<Option<String>, FeatureError> {
    let mut code = Code::default();
    let mut seen = HashSet::new();
    for (tag, body, disabled) in features {
        if disabled || LEFT_OUT.contains(&tag) || !is_tag(tag) || !substitutes(body) {
            continue;
        }
        if seen.insert(tag) {
            code.line(&format!("feature {tag};"))?;
        }
    }
    Ok((!code.is_empty()).then(|| code.into_string()))
}

fn is_tag(tag: &str) -> bool {
    (1..=4).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_graphic())
}

/// Whether `code` has a substitution statement outside comments.
fn substitutes(code: &str) -> bool {
    code.lines().any(|line| {
        let line = line.split_once('#').map_or(line, |(before, _)| before);
        line.split(|c: char| c.is_whitespace() || ";[]{}()'\",<>".contains(c))
            .any(|word| matches!(word, "sub" | "substitute" | "rsub" | "reversesub"))
    })
}

/// aalt.
struct AllAlternates;

static ALL_ALTERNATES: AllAlternates = AllAlternates;

impl Generator for AllAlternates {
    fn names(&self) -> &'static [&'static str] {
        &["aalt"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let mut generated: Vec<AutoEntry> = Vec::new();
        for generator in super::generators() {
            if generator.names().contains(&"aalt") {
                continue;
            }
            generated.extend(generator.generate(cx)?.into_iter().filter(|e| e.kind == EntryKind::Feature));
        }
        let existing = &cx.existing().features;
        let current = existing.iter().map(|s| {
            let regenerated = s.automatic.then(|| generated.iter().find(|e| e.name == s.name)).flatten();
            let code = regenerated.map_or(s.code.as_str(), |e| e.code.as_str());
            (s.name.as_str(), code, s.disabled)
        });
        let new =
            generated.iter().filter(|e| !e.optional && !existing.iter().any(|s| s.name == e.name)).map(|e| (e.name.as_str(), e.code.as_str(), false));
        let Some(code) = code_for(current.chain(new))? else { return Ok(Vec::new()) };
        Ok(vec![AutoEntry { kind: EntryKind::Feature, name: "aalt".to_string(), code, optional: false }])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Features, Snippet};

    use super::super::AutoGlyph;
    use super::*;

    fn feature(tag: &str, code: &str, automatic: bool) -> Snippet {
        Snippet { name: tag.to_string(), code: code.to_string(), automatic, ..Snippet::default() }
    }

    /// Markow's sidebar: aalt lists the substituting features in sidebar order, a manual calt
    /// included, ccmp left out.
    #[test]
    fn aalt_lists_substituting_features_in_sidebar_order() {
        let features = [
            feature("aalt", "", true),
            feature("ccmp", "lookup ccmp_DFLT_1 {\n\tsub i by idotless;\n} ccmp_DFLT_1;", true),
            feature("locl", "script latn;\nlanguage TRK;\nsub i by idotaccent;", true),
            feature("pnum", "sub zero.tf by zero;", true),
            feature("tnum", "sub zero by zero.tf;", true),
            feature("case", "sub periodcentered.loclCAT by periodcentered.loclCAT.case;", true),
            feature("ss01", "sub q by q.ss01;", true),
            feature("calt", "@numbers = [zero one];\nsub x' @numbers by multiply;", false),
            feature("ss02", "sub upArrow by upArrow.ss02;", true),
            feature("liga", "sub f i by fi;", true),
            feature("frac", "include(features/frac.fea);", false),
            feature("kern", "# sub in a comment\npos A V -50;", false),
            Snippet { disabled: true, ..feature("ss03", "sub a by a.ss03;", false) },
        ];
        let code = code_for(features.iter().map(|s| (s.name.as_str(), s.code.as_str(), s.disabled))).unwrap();
        assert_eq!(
            code.as_deref(),
            Some("feature locl;\nfeature pnum;\nfeature tnum;\nfeature case;\nfeature ss01;\nfeature calt;\nfeature ss02;\n")
        );
    }

    /// The generator on a font with no entries yet lists what Update writes: ordn and frac are
    /// substituting features, liga is left out.
    #[test]
    fn aalt_includes_features_update_adds() {
        let mut names: Vec<&str> = super::super::fractions::FIGURES.to_vec();
        names.extend(["A", "N", "O", "a", "f", "i", "o", "period", "slash", "fi", "numero", "ordfeminine", "ordmasculine"]);
        names.push("onehalf");
        let glyphs = names.iter().map(|n| AutoGlyph { name: n.to_string(), ..AutoGlyph::default() }).collect();
        let existing = Features { features: vec![feature("calt", "sub a' b by a.alt;", false)], ..Features::default() };
        let cx = AutoContext::new(glyphs, Arc::new(existing)).unwrap();
        let entries = ALL_ALTERNATES.generate(&cx).unwrap();
        let code = entries.first().map(|e| e.code.as_str()).unwrap_or_default();
        assert!(code.starts_with("feature calt;\n"), "{code}");
        assert!(code.contains("feature frac;\n") && code.contains("feature ordn;\n"), "{code}");
        assert!(!code.contains("liga"), "{code}");
    }

    #[test]
    fn no_substituting_feature_gives_no_aalt() {
        let cx = AutoContext::new(Vec::new(), Arc::new(Features::default())).unwrap();
        assert!(ALL_ALTERNATES.generate(&cx).unwrap().is_empty());
    }
}
