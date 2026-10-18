//! Bitmap strikes (M11.8; [`pick_size`] M11.10): the iColor layers of a master grouped by size,
//! for `sbix`, `CBDT` and the SBIX to SVG conversion.
//!
//! Rows: `sbix`, `color.sbix.imagePerSize`, `color.sbix.sameAspectRatio`,
//! `color.sbix.masterLayerBounds`, `color.sbix.sizeScalesImage`,
//! `color.export.sbixToSvgSizeChoice`, `CBDT`.
//!
//! Contract (`docs/plans/M11.md` § 3):
//! - One strike per distinct iColor size, sorted by size, at most [`crate::MAX_STRIKES`];
//!   `sbix` writes them at [`SBIX_PPI`] pixels per inch.
//! - A strike glyph names its layer and the stored image path; the table builder reads the bytes
//!   through an [`crate::AssetStore`] and computes the origin offsets in pixels from
//!   [`StrikeGlyph::placement`]. A missing file leaves the glyph out of that strike, with a
//!   warning, at build time.
//! - Images of one glyph that differ in width ÷ height are exported, with a warning
//!   ([`aspect_problems`]).
//! - iColor layers that [`strikes`] leaves out (no image, a second layer of one size, a size out
//!   of range, sizes beyond the strike limit) are named by [`strike_problems`], which the export
//!   plan and `color.describeExport` report.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_model::{Font, Transform};

use crate::MAX_STRIKES;
use crate::export::{Problem, ProblemKind};
use crate::layers::{ColorLayerKind, ICOLOR_SIZES, color_layers};

/// The pixels per inch every `sbix` strike states.
pub const SBIX_PPI: u16 = 72;

/// The images of one size.
#[derive(Clone, Debug, PartialEq)]
pub struct Strike {
    /// Pixels per em.
    pub ppem: u16,
    /// In glyph order of the font.
    pub glyphs: Vec<StrikeGlyph>,
}

/// One glyph's image in a strike.
#[derive(Clone, Debug, PartialEq)]
pub struct StrikeGlyph {
    pub glyph: String,
    /// The iColor layer's id.
    pub layer: String,
    /// The image's stored path, as the [`crate::AssetStore`] takes it.
    pub image: String,
    /// Where the image sits on the layer, in font units (the image's transform).
    pub placement: Transform,
}

/// The strikes of `master_id`'s iColor layers. A glyph contributes the first iColor layer of
/// each size that has an image; layers without an image, later layers of a size the glyph
/// already has, sizes outside [`ICOLOR_SIZES`] and sizes beyond the [`MAX_STRIKES`] smallest are
/// left out, and [`strike_problems`] names each.
pub fn strikes(font: &Font, master_id: &str) -> Vec<Strike> {
    let mut by_size: BTreeMap<u16, Vec<StrikeGlyph>> = BTreeMap::new();
    for entry in icolor_entries(font, master_id) {
        if let Entry::Exported { ppem, glyph } = entry {
            by_size.entry(ppem).or_default().push(glyph);
        }
    }
    by_size.into_iter().take(MAX_STRIKES).map(|(ppem, glyphs)| Strike { ppem, glyphs }).collect()
}

/// Why iColor layers of `master_id` are left out of [`strikes`], one warning each.
pub fn strike_problems(font: &Font, master_id: &str) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut sizes = BTreeSet::new();
    for entry in icolor_entries(font, master_id) {
        let (glyph, message) = match entry {
            Entry::Exported { ppem, .. } => {
                sizes.insert(ppem);
                continue;
            }
            Entry::NoImage { glyph, ppem } => (glyph, format!("its iColor {ppem} layer has no image, so nothing is exported for it")),
            Entry::SecondOfSize { glyph, ppem } => (glyph, format!("it has more than one iColor {ppem} layer; only the first is exported")),
            Entry::BadSize { glyph, ppem } => {
                (glyph, format!("its iColor layer has a size of {ppem} pixels per em, outside {} to {}", ICOLOR_SIZES.start(), ICOLOR_SIZES.end()))
            }
        };
        out.push(Problem { kind: ProblemKind::Warning, glyph: Some(glyph), message });
    }
    if sizes.len() > MAX_STRIKES {
        let dropped: Vec<String> = sizes.iter().skip(MAX_STRIKES).map(u16::to_string).collect();
        out.push(Problem {
            kind: ProblemKind::Warning,
            glyph: None,
            message: format!("the iColor layers have {} sizes; the {MAX_STRIKES} smallest are exported, not {}", sizes.len(), dropped.join(", ")),
        });
    }
    out
}

/// One iColor layer of a master, and what [`strikes`] does with it.
enum Entry {
    Exported { ppem: u16, glyph: StrikeGlyph },
    NoImage { glyph: String, ppem: u16 },
    SecondOfSize { glyph: String, ppem: u16 },
    BadSize { glyph: String, ppem: u16 },
}

/// Every iColor layer of `master_id`, glyph by glyph in the font's order, each glyph's layers in
/// paint order.
fn icolor_entries(font: &Font, master_id: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for glyph in font.glyphs.iter() {
        let mut seen = BTreeSet::new();
        for (layer, kind) in color_layers(glyph, master_id) {
            let ColorLayerKind::IColor(ppem) = kind else { continue };
            let name = glyph.name.clone();
            if !ICOLOR_SIZES.contains(&ppem) {
                out.push(Entry::BadSize { glyph: name, ppem });
                continue;
            }
            let Some((image, placement)) = layer.info.image.as_ref().and_then(|i| Some((i.path.clone()?, i.transform))) else {
                out.push(Entry::NoImage { glyph: name, ppem });
                continue;
            };
            if image.is_empty() {
                out.push(Entry::NoImage { glyph: name, ppem });
            } else if !seen.insert(ppem) {
                out.push(Entry::SecondOfSize { glyph: name, ppem });
            } else {
                out.push(Entry::Exported { ppem, glyph: StrikeGlyph { glyph: name, layer: layer.id.clone(), image, placement } });
            }
        }
    }
    out
}

/// The size SBIX to SVG takes for a glyph with iColor layers of `sizes` when `wanted` is asked:
/// the exact size, else the smallest larger one, else the largest smaller one.
pub fn pick_size(sizes: &[u16], wanted: u16) -> Option<u16> {
    if sizes.contains(&wanted) {
        return Some(wanted);
    }
    let larger = sizes.iter().copied().filter(|&s| s > wanted).min();
    larger.or_else(|| sizes.iter().copied().filter(|&s| s < wanted).max())
}

/// A warning for each glyph whose images in `strikes` differ in width ÷ height. `image_size`
/// gives an image's pixel size by stored path (`None` when it cannot be read).
///
/// Two images have the same ratio when the smaller one's width is within one pixel of the
/// larger one's ratio at the smaller one's height, which allows for rounding when the sizes were
/// made.
pub fn aspect_problems(strikes: &[Strike], image_size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Vec<Problem> {
    // Per glyph, in the order glyphs first appear: (ppem, width, height) of each readable image.
    let mut order: Vec<&str> = Vec::new();
    let mut sizes: BTreeMap<&str, Vec<(u16, u32, u32)>> = BTreeMap::new();
    for strike in strikes {
        for g in &strike.glyphs {
            let Some((w, h)) = image_size(&g.image) else { continue };
            if w == 0 || h == 0 {
                continue;
            }
            let entry = sizes.entry(g.glyph.as_str()).or_default();
            if entry.is_empty() {
                order.push(g.glyph.as_str());
            }
            entry.push((strike.ppem, w, h));
        }
    }
    let mut out = Vec::new();
    for glyph in order {
        let Some(images) = sizes.get(glyph) else { continue };
        let Some(&first) = images.first() else { continue };
        if images.iter().all(|&other| same_ratio(first, other)) {
            continue;
        }
        let list: Vec<String> = images.iter().map(|(ppem, w, h)| format!("{ppem}: {w} × {h}")).collect();
        out.push(Problem {
            kind: ProblemKind::Warning,
            glyph: Some(glyph.to_string()),
            message: format!("its iColor images differ in width ÷ height ({}); each size is exported as it is", list.join(", ")),
        });
    }
    out
}

fn same_ratio((_, w1, h1): (u16, u32, u32), (_, w2, h2): (u16, u32, u32)) -> bool {
    let ((wl, hl), (ws, hs)) = if h1 >= h2 { ((w1, h1), (w2, h2)) } else { ((w2, h2), (w1, h1)) };
    let (wl, hl, ws, hs) = (u64::from(wl), u64::from(hl), u64::from(ws), u64::from(hs));
    // |wl·hs/hl − ws| ≤ 1, multiplied through by hl.
    (wl * hs).abs_diff(ws * hl) <= hl
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use fontcraft_model::{Font, Image, Layer, LayerAttrs, Transform};

    use super::*;
    use crate::layers::with_kind;

    fn icolor(id: &str, master: &str, ppem: u16, image: Option<&str>) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = id.to_string();
        l.info.attrs = with_kind(&LayerAttrs::default(), Some(ColorLayerKind::IColor(ppem)));
        l.info.image = image.map(|p| Image { path: Some(p.to_string()), transform: Transform::translate(10.0, -20.0), ..Image::default() });
        l
    }

    #[test]
    fn strikes_group_icolor_images_by_size_with_problems_for_the_rest() {
        let mut font = Font::new("Bitmap Sans");
        let master = font.masters.first().unwrap().id.clone();
        {
            let mut a = font.glyph_mut("A").unwrap();
            a.layers.push(icolor("a128", &master, 128, Some("A 128.png")));
            a.layers.push(icolor("a64", &master, 64, Some("A 64.png")));
            a.layers.push(icolor("a64b", &master, 64, Some("A 64 again.png")));
            a.layers.push(icolor("a32", &master, 32, None));
            a.layers.push(icolor("other", "another master", 16, Some("A 16.png")));
        }
        font.glyph_mut("B").unwrap().layers.push(icolor("b64", &master, 64, Some("B 64.jpg")));

        let got = strikes(&font, &master);
        let summary: Vec<(u16, Vec<&str>)> = got.iter().map(|s| (s.ppem, s.glyphs.iter().map(|g| g.image.as_str()).collect())).collect();
        assert_eq!(summary, [(64, vec!["A 64.png", "B 64.jpg"]), (128, vec!["A 128.png"])], "sorted by size, glyphs in font order");
        let first = got.first().unwrap().glyphs.first().unwrap();
        assert_eq!((first.glyph.as_str(), first.layer.as_str()), ("A", "a64"));
        assert_eq!(first.placement, Transform::translate(10.0, -20.0));

        let problems = strike_problems(&font, &master);
        let messages: Vec<String> = problems.iter().map(|p| format!("{}: {}", p.glyph.as_deref().unwrap_or("-"), p.message)).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages.iter().any(|m| m.starts_with("A: it has more than one iColor 64 layer")), "{messages:?}");
        assert!(messages.iter().any(|m| m.starts_with("A: its iColor 32 layer has no image")), "{messages:?}");
        assert!(problems.iter().all(|p| p.kind == ProblemKind::Warning));
    }

    #[test]
    fn aspect_problems_name_glyphs_whose_sizes_differ_in_shape() {
        let glyph =
            |name: &str, image: &str| StrikeGlyph { glyph: name.into(), layer: "l".into(), image: image.into(), placement: Transform::default() };
        let strikes = [
            Strike { ppem: 64, glyphs: vec![glyph("A", "a64"), glyph("B", "b64"), glyph("C", "c64")] },
            Strike { ppem: 128, glyphs: vec![glyph("A", "a128"), glyph("B", "b128"), glyph("C", "missing")] },
        ];
        let sizes: HashMap<&str, (u32, u32)> =
            // B's 43 × 64 rounds 85 × 128 scaled to half: the same ratio within a pixel.
            [("a64", (64, 64)), ("a128", (128, 96)), ("b64", (43, 64)), ("b128", (85, 128)), ("c64", (10, 20))].into_iter().collect();
        let problems = aspect_problems(&strikes, &|p| sizes.get(p).copied());
        assert_eq!(problems.len(), 1, "{problems:?}");
        let p = problems.first().unwrap();
        assert_eq!(p.glyph.as_deref(), Some("A"));
        assert!(p.message.contains("64: 64 × 64, 128: 128 × 96"), "{}", p.message);
    }
}
