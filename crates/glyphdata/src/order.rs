//! The default sort key of a glyph (05-font-view.md § 7; `glyphOrder.default` guesses).
//!
//! Scripts come first, so each script has its own letters, numbers and punctuation: Latin, Greek,
//! Cyrillic, the other scripts by name, then glyphs without one. In a script the categories follow
//! in the order Letter, Number, Punctuation, Symbol, Separator, Mark, then glyphs without one; in a
//! category the glyphs are in alphabetical order of their names, capitals before small letters.
//! Numbers are in order of their code points: the published data has no sort names, which put zero
//! to nine in order. Suffixed glyphs come after the run of base glyphs, or right after their base
//! when alternates are kept next to it. A sort name (from custom glyph data or the glyph itself)
//! takes the name's place in that order.

use crate::GlyphData;
use crate::types::Category;

/// A glyph's place in the default order. Keys compare in that order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey {
    /// 0 to 2 for the scripts that come first, 3 another script, 4 none.
    script_rank: u8,
    script: String,
    rank: u8,
    /// 0 a base glyph, 1 a suffixed glyph (when alternates are not kept by their base).
    group: u8,
    /// Of a number without a sort name; else the greatest, so the name decides.
    code_point: u32,
    base: String,
    /// Empty for a base glyph, so it precedes its alternates.
    suffix: String,
}

fn script_rank(script: Option<&str>) -> u8 {
    match script {
        Some("latin") => 0,
        Some("greek") => 1,
        Some("cyrillic") => 2,
        Some(s) if !s.is_empty() => 3,
        _ => 4,
    }
}

fn rank(category: Option<Category>) -> u8 {
    match category {
        Some(Category::Letter) => 0,
        Some(Category::Number) => 1,
        Some(Category::Punctuation) => 2,
        Some(Category::Symbol) => 3,
        Some(Category::Separator | Category::Space) => 4,
        Some(Category::Mark) => 5,
        Some(Category::Other) => 6,
        None => 7,
    }
}

impl GlyphData {
    /// The sort key of a glyph. `category` and `script` are the glyph's effective ones (stored or from
    /// the database); `sort_name` its own sort name, if it has one. `keep_alternates` is the font's
    /// *Keep Alternates Next to Base Glyph* setting.
    pub fn sort_key(
        &self,
        name: &str,
        code_points: &[u32],
        category: Option<Category>,
        script: Option<&str>,
        sort_name: Option<&str>,
        keep_alternates: bool,
    ) -> SortKey {
        let custom = self.record(name).and_then(|r| if keep_alternates { r.sort_name_keep.or(r.sort_name) } else { r.sort_name });
        let sort_name = sort_name.map(str::to_string).or(custom);
        let sorted = sort_name.is_some();
        let compared = sort_name.unwrap_or_else(|| name.to_string());
        let (base, suffix) = self.split_suffix(&compared);
        let code_point = match (category, suffix) {
            (Some(Category::Number), _) if sorted => None,
            (Some(Category::Number), None) => code_points.first().copied().or_else(|| self.lookup(name, &[]).unicode.map(|u| u.value)),
            (Some(Category::Number), Some(_)) => self.lookup(base, &[]).unicode.map(|u| u.value),
            _ => None,
        }
        .unwrap_or(u32::MAX);
        SortKey {
            script_rank: script_rank(script),
            script: script.unwrap_or_default().to_string(),
            rank: rank(category),
            group: if suffix.is_some() && !keep_alternates { 1 } else { 0 },
            code_point,
            base: base.to_string(),
            suffix: suffix.unwrap_or_default().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Category, GlyphData};

    #[test]
    fn default_order() {
        let d = GlyphData::new().unwrap();
        let key = |n: &str, keep: bool| {
            let info = d.lookup(n, &[]);
            let script = info.script.map(|s| s.value);
            d.sort_key(n, &[], info.category.map(|c| c.value), script.as_deref(), None, keep)
        };
        let mut names = vec!["a.ss01", "period", "two", "be-cy", "one-ar", "b", "A", "acutecomb", "a", "one", "eight", "zzz", "alef-ar", "alpha"];
        names.sort_by_key(|n| key(n, false));
        // Each script with its own numbers, Latin, Greek and Cyrillic first; numbers by value.
        assert_eq!(names, ["A", "a", "b", "a.ss01", "alpha", "be-cy", "alef-ar", "one-ar", "one", "two", "eight", "period", "acutecomb", "zzz"]);
        let mut names = vec!["a.ss01", "b", "A", "a"];
        names.sort_by_key(|n| key(n, true));
        assert_eq!(names, ["A", "a", "a.ss01", "b"]);

        // A sort name takes the name's place: AE comes before Aacute, and after it as `Az`.
        let letter = |name: &str, sort: Option<&str>| d.sort_key(name, &[], Some(Category::Letter), Some("latin"), sort, false);
        assert!(letter("AE", None) < letter("Aacute", None));
        assert!(letter("AE", Some("Az")) > letter("Agrave", None) && letter("AE", Some("Az")) < letter("B", None));
    }
}
