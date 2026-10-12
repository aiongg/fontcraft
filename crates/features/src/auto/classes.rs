//! The automatic classes (M8.9).
//!
//! Rows: `autoClass.general`, `fontInfo.features.optionalAutomatic`.
//!
//! Four classes, each one glyph name per line in glyph order, the form of the reference's stored
//! code in the corpus:
//!
//! | Class | Members |
//! |---|---|
//! | `Uppercase` | letters whose subcategory is `Uppercase` |
//! | `Lowercase` | letters whose subcategory is `Lowercase` |
//! | `AllLetters` | every letter |
//! | `All` | every exporting glyph but `.notdef` and `.null` |
//!
//! All four are optional: Update refreshes them when the font has them and never adds them. The
//! handbook names only `All` as optional; the corpus has capitals and lowercase letters without
//! the matching classes in sources the reference last updated, so the others are treated alike.

use super::{AutoContext, AutoEntry, AutoGlyph, Generator, MAX_GENERATED_BYTES};
use crate::FeatureError;
use crate::source_map::EntryKind;

struct Class {
    name: &'static str,
    names: &'static [&'static str],
    member: fn(&AutoGlyph) -> bool,
}

static UPPERCASE: Class = Class { name: "Uppercase", names: &["Uppercase"], member: |g| is_letter(g) && sub_is(g, "Uppercase") };
static LOWERCASE: Class = Class { name: "Lowercase", names: &["Lowercase"], member: |g| is_letter(g) && sub_is(g, "Lowercase") };
static ALL_LETTERS: Class = Class { name: "AllLetters", names: &["AllLetters"], member: is_letter };
static ALL: Class = Class { name: "All", names: &["All"], member: |g| !matches!(g.name.as_str(), ".notdef" | ".null") };

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&UPPERCASE, &LOWERCASE, &ALL_LETTERS, &ALL]
}

fn is_letter(g: &AutoGlyph) -> bool {
    g.category.as_deref() == Some("Letter")
}

fn sub_is(g: &AutoGlyph, sub: &str) -> bool {
    g.sub_category.as_deref() == Some(sub)
}

impl Generator for Class {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let mut code = String::new();
        for g in cx.glyphs().iter().filter(|g| (self.member)(g)) {
            if code.len() + g.name.len() + 1 > MAX_GENERATED_BYTES {
                return Err(FeatureError::Limit { what: "bytes in one generated entry", limit: MAX_GENERATED_BYTES });
            }
            code.push_str(&g.name);
            code.push('\n');
        }
        if code.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![AutoEntry { kind: EntryKind::Class, name: self.name.to_string(), code, optional: true }])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Features;

    use super::*;

    fn letter(name: &str, sub: &str) -> AutoGlyph {
        AutoGlyph { name: name.into(), category: Some("Letter".into()), sub_category: Some(sub.into()), ..AutoGlyph::default() }
    }

    #[test]
    fn classes_list_their_members_one_per_line_in_glyph_order() {
        let glyphs = vec![
            letter("B", "Uppercase"),
            letter("a", "Lowercase"),
            AutoGlyph { name: "zero".into(), category: Some("Number".into()), ..AutoGlyph::default() },
            letter("A", "Uppercase"),
            AutoGlyph { name: ".notdef".into(), ..AutoGlyph::default() },
        ];
        let cx = AutoContext::new(glyphs, Arc::new(Features::default())).unwrap();
        let code = |c: &Class| c.generate(&cx).unwrap().first().map(|e| (e.code.clone(), e.optional));
        assert_eq!(code(&UPPERCASE), Some(("B\nA\n".into(), true)));
        assert_eq!(code(&LOWERCASE), Some(("a\n".into(), true)));
        assert_eq!(code(&ALL_LETTERS), Some(("B\na\nA\n".into(), true)));
        assert_eq!(code(&ALL), Some(("B\na\nzero\nA\n".into(), true)));
        let empty = AutoContext::new(vec![letter("a", "Lowercase")], Arc::new(Features::default())).unwrap();
        assert!(UPPERCASE.generate(&empty).unwrap().is_empty());
    }
}
