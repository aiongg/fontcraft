//! Glyph order (`docs/spec/05-font-view.md` § 7).
//!
//! [`glyph_order`] is the one order Font View and export use: the names of the font's
//! `glyphOrder` parameter that exist, then the remaining glyphs in stored order. A file's stored
//! order is never re-sorted (`glyphOrder.default` guesses); a glyph added later goes where the
//! default sort key puts it among the glyphs already there ([`Placement`]).

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_glyphdata::{Category, GlyphData, SortKey};
use fontcraft_plist::Value;

use crate::{Font, Glyph, GlyphTable, MAX_GLYPHS, ModelError, glyph_props};

/// The custom parameter holding a font's own glyph order.
pub const GLYPH_ORDER_PARAM: &str = "glyphOrder";

/// The names of the `glyphOrder` parameter, in order, without section-header lines (`**Title**`)
/// and empty lines. A text value counts as one name per line.
pub fn glyph_order_names(font: &Font) -> Vec<&str> {
    order_lines(font).into_iter().filter(|n| !is_section_header(n)).collect()
}

/// The non-empty lines of the `glyphOrder` parameter, trimmed, headers included.
fn order_lines(font: &Font) -> Vec<&str> {
    let Some(value) = font.info.params.get(GLYPH_ORDER_PARAM) else { return Vec::new() };
    let lines: Vec<&str> = match value {
        Value::Array(a) => a.items.iter().filter_map(Value::as_str).collect(),
        Value::String(_) => value.as_str().map(|s| s.lines().collect()).unwrap_or_default(),
        _ => Vec::new(),
    };
    lines.into_iter().map(str::trim).filter(|n| !n.is_empty()).collect()
}

/// The section headers of the `glyphOrder` parameter, each as its title (the text between the
/// asterisks) and the position in [`glyph_order`] of the first glyph listed after it. A header
/// with no existing glyph after it starts where the next one would, or after the listed glyphs.
pub fn glyph_order_sections(font: &Font) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut placed = HashSet::new();
    for line in order_lines(font) {
        if is_section_header(line) {
            let title = line.trim_start_matches('*').trim_end_matches('*').trim();
            out.push((placed.len(), title.to_string()));
        } else if let Some(i) = font.glyphs.position(line) {
            placed.insert(i);
        }
    }
    out
}

/// A `glyphOrder` line that titles a section of Font View instead of naming a glyph.
pub fn is_section_header(line: &str) -> bool {
    line.len() >= 4 && line.starts_with("**") && line.ends_with("**")
}

/// The glyphs' positions in `font.glyphs`, in display and export order: the glyphs the
/// `glyphOrder` parameter names (each once, missing names skipped), then the rest in stored order.
pub fn glyph_order(font: &Font) -> Vec<usize> {
    let mut order = Vec::with_capacity(font.glyphs.len());
    let mut placed = HashSet::new();
    for name in glyph_order_names(font) {
        if let Some(i) = font.glyphs.position(name)
            && placed.insert(i)
        {
            order.push(i);
        }
    }
    if placed.is_empty() {
        return (0..font.glyphs.len()).collect();
    }
    order.extend((0..font.glyphs.len()).filter(|i| !placed.contains(i)));
    order
}

/// The font's *Keep Alternates Next to Base Glyph* setting.
pub fn keeps_alternates(font: &Font) -> bool {
    font.info.settings.keep_alternates_together.unwrap_or(false)
}

/// A glyph's default sort key: its effective category, its own sort name and the font's
/// *Keep Alternates Next to Base Glyph* setting decide it (`glyphOrder.default`).
pub fn sort_key(font: &Font, glyph: &Glyph, data: &GlyphData) -> SortKey {
    let keep = keeps_alternates(font);
    let props = glyph_props(glyph, data, keep);
    let category = props.category.as_deref().and_then(Category::parse);
    let own = if keep { glyph.info.sort_name_keep.as_deref().or(glyph.info.sort_name.as_deref()) } else { glyph.info.sort_name.as_deref() };
    data.sort_key(&glyph.name, &glyph.unicodes, category, own, keep)
}

/// The sort keys of a font's glyphs in stored order, for placing new glyphs. Build it once for a
/// command that adds many glyphs, and call [`Placement::insert`] after each one.
pub struct Placement {
    keys: Vec<SortKey>,
}

impl Placement {
    pub fn new(font: &Font, data: &GlyphData) -> Placement {
        Placement { keys: font.glyphs.iter().map(|g| sort_key(font, g, data)).collect() }
    }

    /// Where a glyph with this key goes in stored order: right after the last glyph whose key is
    /// not greater, else first. In a sorted font that is its sorted place; in a font sorted
    /// some other way, it keeps the new glyph after the glyphs it would follow.
    pub fn index_for(&self, key: &SortKey) -> usize {
        self.keys.iter().rposition(|k| k <= key).map_or(0, |i| i + 1)
    }

    /// Records that a glyph with `key` was inserted at `index`.
    pub fn insert(&mut self, index: usize, key: SortKey) {
        let index = index.min(self.keys.len());
        self.keys.insert(index, key);
    }
}

/// Where `glyph` (not yet in the font) goes in the font's stored order.
pub fn insertion_index(font: &Font, data: &GlyphData, glyph: &Glyph) -> usize {
    Placement::new(font, data).index_for(&sort_key(font, glyph, data))
}

/// Adds `glyphs` (names not in the font) where the default sort order puts each among the glyphs
/// already there. The result equals adding them one at a time at [`Placement::index_for`]: each
/// goes after the last glyph whose key is not greater, new glyphs that land in the same gap are
/// in key order, and equal keys keep the order given. The existing glyphs keep their order and
/// the table is rebuilt once, so the work grows as (font + new) × log(font).
pub fn insert_by_sort_key(font: &mut Font, data: &GlyphData, glyphs: Vec<Glyph>) -> Result<(), ModelError> {
    if glyphs.is_empty() {
        return Ok(());
    }
    if font.glyphs.len().saturating_add(glyphs.len()) > MAX_GLYPHS {
        return Err(ModelError::TooManyGlyphs);
    }
    let mut names = HashSet::with_capacity(glyphs.len());
    for g in &glyphs {
        if g.name.is_empty() {
            return Err(ModelError::EmptyGlyphName);
        }
        if font.glyph(&g.name).is_some() || !names.insert(g.name.as_str()) {
            return Err(ModelError::DuplicateGlyph(g.name.clone()));
        }
    }
    let rest = font.glyphs.as_arcs().to_vec();
    let new = glyphs.into_iter().map(Arc::new).collect();
    font.glyphs = GlyphTable::from_glyphs(merge_by_sort_key(font, data, rest, new));
    Ok(())
}

/// Moves the named glyphs to the place the default sort key gives them among the other glyphs,
/// as [`insert_by_sort_key`] would add them. The other glyphs keep their order. Names that are
/// not glyphs are ignored.
pub fn place_by_sort_key(font: &mut Font, data: &GlyphData, names: &[String]) {
    let moving: HashSet<&str> = names.iter().map(String::as_str).collect();
    if moving.is_empty() || !font.glyphs.iter().any(|g| moving.contains(g.name.as_str())) {
        return;
    }
    let (moved, rest): (Vec<Arc<Glyph>>, Vec<Arc<Glyph>>) = font.glyphs.iter().cloned().partition(|g| moving.contains(g.name.as_str()));
    font.glyphs = GlyphTable::from_glyphs(merge_by_sort_key(font, data, rest, moved));
}

/// `rest` in its order with each of `new` placed after the last glyph of `rest` whose key is not
/// greater. Sort keys come from `font`'s settings.
fn merge_by_sort_key(font: &Font, data: &GlyphData, rest: Vec<Arc<Glyph>>, new: Vec<Arc<Glyph>>) -> Vec<Arc<Glyph>> {
    // The smallest key from each position to the end. It never decreases, so the number of
    // positions whose suffix holds a key not greater than `k` is one past the last such key:
    // the place `Placement::index_for` gives, found by binary search.
    let mut suffix_min: Vec<SortKey> = Vec::with_capacity(rest.len());
    for g in rest.iter().rev() {
        let key = sort_key(font, g, data);
        let min = match suffix_min.last() {
            Some(m) if *m < key => m.clone(),
            _ => key,
        };
        suffix_min.push(min);
    }
    suffix_min.reverse();
    let mut placed: Vec<(usize, SortKey, Arc<Glyph>)> = new
        .into_iter()
        .map(|g| {
            let key = sort_key(font, &g, data);
            (suffix_min.partition_point(|m| *m <= key), key, g)
        })
        .collect();
    // Stable, so equal keys in one gap keep the order given.
    placed.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let mut out = Vec::with_capacity(rest.len().saturating_add(placed.len()));
    let mut placed = placed.into_iter().peekable();
    for (i, g) in rest.into_iter().enumerate() {
        while let Some((_, _, n)) = placed.next_if(|n| n.0 <= i) {
            out.push(n);
        }
        out.push(g);
    }
    out.extend(placed.map(|(_, _, n)| n));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_plist::{Array, Str};

    fn font(names: &[&str]) -> Font {
        let mut f = Font::new("T");
        f.glyphs = crate::GlyphTable::from_glyphs(Vec::new());
        for n in names {
            f.add_glyph(n, Vec::new(), 500.0).unwrap();
        }
        f
    }

    fn names(f: &Font, order: &[usize]) -> Vec<String> {
        order.iter().map(|i| f.glyphs[*i].name.clone()).collect()
    }

    #[test]
    fn glyph_order_parameter_with_headers_and_gaps() {
        let mut f = font(&["a", "b", "c", "d"]);
        let list = ["**Lower**", "c", "missing", "a", "c", ""].iter().map(|s| Value::String(Str::new(*s))).collect();
        std::sync::Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, Value::Array(Array { items: list, ..Default::default() }));
        assert_eq!(names(&f, &glyph_order(&f)), ["c", "a", "b", "d"]);
        assert_eq!(glyph_order_sections(&f), [(0, "Lower".to_string())]);
        let list = ["a", "**Gone**", "missing", "**Last**", "b", "**End**"].iter().map(|s| Value::String(Str::new(*s))).collect();
        std::sync::Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, Value::Array(Array { items: list, ..Default::default() }));
        assert_eq!(glyph_order_sections(&f), [(1, "Gone".to_string()), (1, "Last".to_string()), (2, "End".to_string())]);
        std::sync::Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, Value::String(Str::new("d\nb\n")));
        assert_eq!(names(&f, &glyph_order(&f)), ["d", "b", "a", "c"]);
    }

    #[test]
    fn new_glyphs_take_their_sorted_place_without_reordering() {
        let d = GlyphData::new().unwrap();
        // Stored order is not the sort order; it stays as it is.
        let f = font(&[".notdef", "B", "A", "one", "period"]);
        let at = |name: &str| insertion_index(&f, &d, &Glyph::new(name));
        assert_eq!(at("C"), 3, "after the last letter, before the number");
        assert_eq!(at("A.alt"), 3, "alternates follow the base glyphs");
        assert_eq!(at("two"), 4);
        assert_eq!(at("acutecomb"), 5, "marks come after punctuation");
        assert_eq!(at("unknownName"), 5);

        let mut p = Placement::new(&f, &d);
        let k = sort_key(&f, &Glyph::new("C"), &d);
        p.insert(p.index_for(&k), k);
        assert_eq!(p.index_for(&sort_key(&f, &Glyph::new("D"), &d)), 4);
    }

    #[test]
    fn many_new_glyphs_land_where_one_at_a_time_would_put_them() {
        let d = GlyphData::new().unwrap();
        let base = font(&["period", ".notdef", "B", "A", "one", "a", "zero", "D", "comma"]);
        let new = ["C", "b", "C.alt", "two", "acutecomb", "A.sc", "unknownName", "Z", "aa", "exclam", "uni0416", "C.ss01"];
        let mut one_by_one = base.clone();
        let mut p = Placement::new(&one_by_one, &d);
        for n in new {
            let key = sort_key(&one_by_one, &Glyph::new(n), &d);
            let at = p.index_for(&key);
            one_by_one.add_glyph_at(at, n, Vec::new(), 500.0).unwrap();
            p.insert(at, key);
        }
        let mut batch = base.clone();
        insert_by_sort_key(&mut batch, &d, new.iter().map(|n| Glyph::new(*n)).collect()).unwrap();
        let order = |f: &Font| f.glyphs.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
        assert_eq!(order(&batch), order(&one_by_one));
        assert!(insert_by_sort_key(&mut batch, &d, vec![Glyph::new("B")]).is_err(), "a name the font has");
        assert!(insert_by_sort_key(&mut batch, &d, vec![Glyph::new("q"), Glyph::new("q")]).is_err(), "a name given twice");
        assert_eq!(order(&batch), order(&one_by_one), "a refused batch changes nothing");

        // Moving glyphs that are already there gives the same order as adding them.
        let mut moved = base.clone();
        for n in new {
            moved.add_glyph(n, Vec::new(), 500.0).unwrap();
        }
        place_by_sort_key(&mut moved, &d, &new.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(order(&moved), order(&one_by_one));
    }
}
