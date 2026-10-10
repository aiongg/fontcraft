//! The default sort key of a glyph (05-font-view.md § 7; `glyphOrder.default` guesses).
//!
//! The published data has no sort names, so the key is FontCraft's own: categories in the order
//! Letter, Number, Punctuation, Symbol, Separator, Mark, then glyphs without one; within a
//! category the base glyph's code point, then the name. Suffixed glyphs come after the run of
//! base glyphs, or right after their base when alternates are kept next to it. A sort name
//! (from custom glyph data or the glyph itself) replaces the code point and name: such glyphs
//! follow the others of their category, ordered by the sort name.

use crate::GlyphData;
use crate::types::Category;

/// A glyph's place in the default order. Keys compare in that order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey {
    rank: u8,
    /// 0 a base glyph, 1 a suffixed glyph (when alternates are not kept by their base), 2 a
    /// glyph ordered by its sort name.
    group: u8,
    code_point: u32,
    base: String,
    /// Empty for a base glyph, so it precedes its alternates.
    suffix: String,
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
    /// The sort key of a glyph. `category` is the glyph's effective category (stored or from the
    /// database); `sort_name` its own sort name, if it has one. `keep_alternates` is the font's
    /// *Keep Alternates Next to Base Glyph* setting.
    pub fn sort_key(&self, name: &str, code_points: &[u32], category: Option<Category>, sort_name: Option<&str>, keep_alternates: bool) -> SortKey {
        let (base, suffix) = self.split_suffix(name);
        let rank = rank(category);
        let custom_sort_name = self.record(name).and_then(|r| if keep_alternates { r.sort_name_keep.or(r.sort_name) } else { r.sort_name });
        if let Some(s) = sort_name.map(str::to_string).or(custom_sort_name) {
            return SortKey { rank, group: 2, code_point: 0, base: s, suffix: String::new() };
        }
        let code_point = match suffix {
            None => code_points.first().copied().or_else(|| self.lookup(name, &[]).unicode.map(|u| u.value)),
            Some(_) => self.lookup(base, &[]).unicode.map(|u| u.value),
        }
        .unwrap_or(u32::MAX);
        let group = if suffix.is_some() && !keep_alternates { 1 } else { 0 };
        SortKey { rank, group, code_point, base: base.to_string(), suffix: suffix.unwrap_or("").to_string() }
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
            d.sort_key(n, &[], info.category.map(|c| c.value), None, keep)
        };
        let mut names = vec!["a.ss01", "period", "b", "A", "acutecomb", "a", "one", "zzz"];
        names.sort_by_key(|n| key(n, false));
        assert_eq!(names, ["A", "a", "b", "a.ss01", "one", "period", "acutecomb", "zzz"]);
        names.sort_by_key(|n| key(n, true));
        assert_eq!(names, ["A", "a", "a.ss01", "b", "one", "period", "acutecomb", "zzz"]);
        let sorted = d.sort_key("b", &[], Some(Category::Letter), Some("0"), false);
        assert!(sorted > key("b", false));
    }
}
