//! The marks that attach at an anchor, for the mark cloud (`appearance.markCloudOnAnchorSelection`).
//!
//! The glyph database's `marks` field of the base glyph (a comma-separated list, also under the base name of a suffixed
//! glyph) names them; the published data has none, so without a list every exporting mark of the font is a candidate.
//! A candidate attaches at anchor `top` when it has an anchor `_top`; at a numbered or suffixed anchor (`top_2`,
//! `top_viet`) when it has the partner of the whole name or else of the part before the last underscore. Marks may hold
//! the partner in a component (anchors propagate).

use fontcraft_glyphdata::{Category, GlyphData};
use fontcraft_model::Font;

use crate::compose::category_of;
use crate::propagate::effective_anchors_with;

/// Most marks one anchor gets.
pub const MAX_CLOUD_MARKS: usize = 200;
/// Most names read from a database list.
const MAX_LISTED: usize = 4_096;

/// A mark that attaches at an anchor, and where its own attaching anchor stands on its layer.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachingMark {
    pub glyph: String,
    pub x: f64,
    pub y: f64,
}

/// The marks the glyph database lists for `glyph`; `None` when it lists none.
pub fn listed_marks(data: &GlyphData, glyph: &str) -> Option<Vec<String>> {
    if glyph.is_empty() || glyph.len() > fontcraft_model::MAX_NAME_LEN {
        return None;
    }
    let (base, _) = data.split_suffix(glyph);
    [glyph, base].into_iter().find_map(|name| {
        let list = data.record(name)?.marks?;
        let names: Vec<String> = list.split(',').map(str::trim).filter(|n| !n.is_empty()).take(MAX_LISTED).map(str::to_string).collect();
        (!names.is_empty()).then_some(names)
    })
}

/// The anchors a mark attaches by at the anchor `name`, best first; empty for an underscore anchor.
fn partners(name: &str) -> Vec<String> {
    if name.is_empty() || name.starts_with('_') {
        return Vec::new();
    }
    let plain = name.rsplit_once('_').map(|(plain, _)| plain).filter(|p| !p.is_empty());
    [Some(name), plain].into_iter().flatten().map(|n| format!("_{n}")).collect()
}

/// The marks of `listed` (see [`listed_marks`]), else every exporting mark, that attach at `anchor`, on the layers of
/// master `master`, in glyph order and at most [`MAX_CLOUD_MARKS`]. A glyph whose anchors cannot be worked out (a
/// circular component) is left out.
pub fn attaching_marks(font: &Font, data: &GlyphData, listed: Option<&[String]>, anchor: &str, master: &str) -> Vec<AttachingMark> {
    let partners = partners(anchor);
    if partners.is_empty() {
        return Vec::new();
    }
    let attaching = |name: &str| -> Option<AttachingMark> {
        let anchors = effective_anchors_with(font, Some(data), name, master).ok()?;
        let at = partners.iter().find_map(|p| anchors.iter().find(|a| a.name == *p))?;
        (at.x.is_finite() && at.y.is_finite()).then(|| AttachingMark { glyph: name.to_string(), x: at.x, y: at.y })
    };
    match listed {
        Some(names) => names.iter().filter_map(|n| attaching(n)).take(MAX_CLOUD_MARKS).collect(),
        None => font
            .glyphs
            .iter()
            .filter(|g| g.export && category_of(font, data, &g.name) == Some(Category::Mark))
            .filter_map(|g| attaching(&g.name))
            .take(MAX_CLOUD_MARKS)
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use fontcraft_glyphdata::{GlyphData, Record};
    use fontcraft_model::{Anchor, Component, Font, Shape};

    use super::{attaching_marks, listed_marks};

    fn add(font: &mut Font, name: &str, anchors: &[(&str, f64, f64)]) {
        font.add_glyph(name, Vec::new(), 0.0).unwrap();
        let mut g = font.glyph_mut(name).unwrap();
        g.layer_mut("m01").unwrap().anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
    }

    /// Every exporting mark with the partner anchor attaches, also one whose anchor is in a component; a base glyph's
    /// list in the database wins; numbered anchors fall back to the plain partner.
    #[test]
    fn marks_attach_by_their_partner_anchor() {
        let mut font = Font::new("Cloud");
        add(&mut font, "acutecomb", &[("_top", 10.0, 500.0)]);
        add(&mut font, "gravecomb", &[]);
        font.glyph_mut("gravecomb").unwrap().layer_mut("m01").unwrap().shapes = vec![Shape::Component(Component::new("acutecomb", 5.0, 0.0))];
        add(&mut font, "dotbelowcomb", &[("_bottom", 0.0, 0.0)]);
        add(&mut font, "tildecomb", &[("_top", 0.0, 0.0)]);
        font.glyph_mut("tildecomb").unwrap().export = false;
        let data = GlyphData::new().unwrap();
        let names = |listed: Option<&[String]>, anchor: &str, data: &GlyphData| -> Vec<String> {
            attaching_marks(&font, data, listed, anchor, "m01").into_iter().map(|m| m.glyph).collect()
        };
        assert_eq!(names(None, "top", &data), ["acutecomb", "gravecomb"]);
        assert_eq!(names(None, "top_2", &data), ["acutecomb", "gravecomb"]);
        assert_eq!(names(None, "bottom", &data), ["dotbelowcomb"]);
        assert!(names(None, "_top", &data).is_empty());
        let grave = attaching_marks(&font, &data, None, "top", "m01").pop().unwrap();
        assert_eq!((grave.x, grave.y), (15.0, 500.0));

        let record = Record { name: "a".into(), marks: Some("tildecomb, dotbelowcomb,nosuch".into()), ..Record::default() };
        let custom = GlyphData::with_custom(vec![vec![record]]).unwrap();
        let listed = listed_marks(&custom, "a.ss01").unwrap();
        assert_eq!(names(Some(&listed), "top", &custom), ["tildecomb"], "the database's list wins");
        assert_eq!(listed_marks(&custom, "b"), None);
    }
}
