//! Capital spacing (M8.12).
//!
//! Rows: `autoFeature.cpsp`.
//!
//! The entry is optional: Update writes it only when the font already has a `cpsp` entry (the
//! user adds it from the Features tab's plus menu). Its code is one positioning rule that widens
//! every capital by [`CPSP_SPACE`] units on each side, on the `Uppercase` class when the font has
//! one, else on an inline list of the capitals.

use super::{AutoContext, AutoEntry, Generator, MAX_GENERATED_BYTES};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// The space added on each side of a capital, in font units.
pub const CPSP_SPACE: i32 = 5;

struct Cpsp;

static CPSP: Cpsp = Cpsp;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&CPSP]
}

/// A capital letter: its subcategory says so, or, when the glyph has no category data, its
/// first code point is an uppercase letter.
fn is_capital(glyph: &super::AutoGlyph) -> bool {
    if let Some(sub) = glyph.sub_category.as_deref() {
        return sub.eq_ignore_ascii_case("Uppercase");
    }
    let letter = glyph.category.as_deref().is_none_or(|c| c.eq_ignore_ascii_case("Letter"));
    letter && glyph.codepoints.first().and_then(|&cp| char::from_u32(cp)).is_some_and(char::is_uppercase)
}

impl Generator for Cpsp {
    fn names(&self) -> &'static [&'static str] {
        &["cpsp"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let capitals: Vec<&str> = cx.glyphs().iter().filter(|g| is_capital(g)).map(|g| g.name.as_str()).collect();
        if capitals.is_empty() {
            return Ok(Vec::new());
        }
        let has_class = cx.existing().classes.iter().any(|c| c.name == "Uppercase");
        let target = if has_class {
            "@Uppercase".to_string()
        } else {
            let bytes = capitals.iter().fold(0usize, |n, name| n.saturating_add(name.len()).saturating_add(1));
            if bytes > MAX_GENERATED_BYTES {
                return Err(FeatureError::Limit { what: "bytes of generated cpsp code", limit: MAX_GENERATED_BYTES });
            }
            format!("[{}]", capitals.join(" "))
        };
        let space = CPSP_SPACE;
        let advance = CPSP_SPACE.saturating_mul(2);
        let code = format!("pos {target} <{space} 0 {advance} 0>;\n");
        Ok(vec![AutoEntry { kind: EntryKind::Feature, name: "cpsp".into(), code, optional: true }])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Features, Snippet};

    use super::*;
    use crate::auto::AutoGlyph;

    fn glyph(name: &str, cp: Option<u32>, sub: Option<&str>) -> AutoGlyph {
        AutoGlyph {
            name: name.into(),
            codepoints: cp.into_iter().collect(),
            category: sub.map(|_| "Letter".into()),
            sub_category: sub.map(Into::into),
            ..AutoGlyph::default()
        }
    }

    fn glyphs() -> Vec<AutoGlyph> {
        vec![
            glyph("A", Some(0x41), Some("Uppercase")),
            glyph("a", Some(0x61), Some("Lowercase")),
            glyph("B", Some(0x42), None),
            glyph("A.ss01", None, Some("Uppercase")),
            glyph("one", Some(0x31), None),
        ]
    }

    #[test]
    fn cpsp_widens_capitals_and_is_optional() {
        let cx = AutoContext::new(glyphs(), Arc::new(Features::default())).unwrap();
        let entries = CPSP.generate(&cx).unwrap();
        assert_eq!(
            entries,
            [AutoEntry { kind: EntryKind::Feature, name: "cpsp".into(), code: "pos [A B A.ss01] <5 0 10 0>;\n".into(), optional: true }]
        );
    }

    #[test]
    fn cpsp_uses_the_uppercase_class_when_the_font_has_one() {
        let mut features = Features::default();
        features.classes.push(Snippet { name: "Uppercase".into(), code: "A B".into(), automatic: true, ..Snippet::default() });
        let cx = AutoContext::new(glyphs(), Arc::new(features)).unwrap();
        assert_eq!(CPSP.generate(&cx).unwrap()[0].code, "pos @Uppercase <5 0 10 0>;\n");
    }

    #[test]
    fn cpsp_nothing_without_capitals() {
        let cx =
            AutoContext::new(vec![glyph("a", Some(0x61), Some("Lowercase")), glyph("one", Some(0x31), None)], Arc::new(Features::default())).unwrap();
        assert_eq!(CPSP.generate(&cx).unwrap(), []);
    }
}
