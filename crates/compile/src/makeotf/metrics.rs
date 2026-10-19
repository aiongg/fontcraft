//! The `OS/2` and `hhea` values of a project compile, worked out as the reference compiler works
//! them out (row `compile.makeotfMetrics`). That compiler reads none of the source's OpenType
//! metric values (weight class, selection bits, typographic, Windows and `hhea` metrics, cap and
//! x-height); it takes them from the glyphs and the full name, and only the feature code's
//! `table OS/2` and `table hhea` blocks override them.
//!
//! Ported from AFDKO 5.0.1 (Apache-2.0, see `NOTICE`): `c/addfeatures/hotconv/hot.cpp` (cap and
//! x-height, typographic and Windows metrics), `OS_2.cpp` (weight and width classes from the
//! full name, `fsSelection`), `hmtx.cpp` (`hhea` from the typographic metrics) and
//! `c/shared/absfont/absfont_metrics.cpp` (integer glyph boxes).

use fontcraft_model::ResolvedGlyph;
use write_fonts::tables::glyf::Bbox;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::types::FWord;

use crate::info::Info;

/// What [`apply`] reads from the source besides its glyphs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    /// The PostScript full name (a UFO's `postscriptFullName`), which the weight and width
    /// classes are read from. The reference does not make one up when the source has none.
    pub full_name: Option<String>,
    /// The project's bold and italic style link.
    pub bold: bool,
    pub italic: bool,
}

/// Words of the full name and the weight class each gives, tried in this order (a word that
/// holds another comes before it).
const WEIGHTS: [(&str, u16); 19] = [
    ("thin", 250),
    ("extralight", 250),
    ("ultralight", 250),
    ("light", 300),
    ("medium", 500),
    ("semibold", 600),
    ("demibold", 600),
    ("demi", 600),
    ("extrabold", 800),
    ("ultrabold", 800),
    ("extrablack", 950),
    ("ultrablack", 950),
    ("ultra", 900),
    ("super", 800),
    ("bold", 700),
    ("black", 900),
    ("poster", 900),
    ("heavy", 850),
    ("nord", 900),
];

/// Words of the full name and the width class each gives, tried in this order.
const WIDTHS: [(&str, u16); 17] = [
    ("ultracondensed", 1),
    ("extracompressed", 1),
    ("ultracompressed", 1),
    ("extracondensed", 2),
    ("compressed", 2),
    ("semicondensed", 4),
    ("compact", 4),
    ("narrow", 4),
    ("condensed", 3),
    ("semiexpanded", 6),
    ("semiextended", 6),
    ("extraexpanded", 8),
    ("extraextended", 8),
    ("ultraexpanded", 9),
    ("ultraextended", 9),
    ("expanded", 7),
    ("extended", 7),
];

/// The lowest weight class the reference writes when it reads the class from the full name.
const LOWEST_WEIGHT: u16 = 250;

/// The weight and width classes a full name gives: the first word of [`WEIGHTS`] and of
/// [`WIDTHS`] it holds, in any case; 400 and 5 without a full name or a matching word. A weight
/// below 250 is raised to 250 when there is a full name.
pub fn classes(full_name: Option<&str>) -> (u16, u16) {
    let Some(name) = full_name.filter(|n| !n.is_empty()) else { return (400, 5) };
    let lower = name.to_ascii_lowercase();
    // The reference's guard against reading "ultracondensed" as the weight "ultra" is spelled
    // "ultrcondensed", so it never applies; it is kept as written.
    let weight = WEIGHTS
        .iter()
        .find(|(word, _)| lower.contains(word) && !(*word == "ultra" && lower.contains("ultrcondensed")))
        .map_or(400, |(_, w)| *w)
        .max(LOWEST_WEIGHT);
    let width = WIDTHS.iter().find(|(word, _)| lower.contains(word)).map_or(5, |(_, w)| *w);
    (weight, width)
}

/// Rounding margin of [`int_box`]: an edge less than this past an integer rounds back to it.
const BOX_FUDGE: f64 = 0.15;

/// A glyph's integer box as the reference makes it from the drawn outline: edges rounded
/// outwards, unless they are within [`BOX_FUDGE`] of the integer inside.
pub fn int_box(b: &fontcraft_cff::Bounds) -> [i64; 4] {
    let out = |v: f64, f: fn(f64) -> f64| if v.is_finite() { f(v) as i64 } else { 0 };
    [
        out(b.x_min + BOX_FUDGE, f64::floor),
        out(b.y_min + BOX_FUDGE, f64::floor),
        out(b.x_max - BOX_FUDGE, f64::ceil),
        out(b.y_max - BOX_FUDGE, f64::ceil),
    ]
}

/// The font box: the union of the glyph boxes, leaving out those that are all zero; all zero
/// when every glyph is.
pub fn font_box(glyphs: &[Option<Bbox>]) -> Bbox {
    let zero = Bbox::default();
    glyphs
        .iter()
        .flatten()
        .filter(|b| **b != zero)
        .fold(None, |acc: Option<Bbox>, b| {
            Some(match acc {
                None => *b,
                Some(a) => {
                    Bbox { x_min: a.x_min.min(b.x_min), y_min: a.y_min.min(b.y_min), x_max: a.x_max.max(b.x_max), y_max: a.y_max.max(b.y_max) }
                }
            })
        })
        .unwrap_or(zero)
}

/// The character whose glyph gives a CID-keyed font's typographic ascender and descender
/// (box drawings light vertical and horizontal).
const VERTICAL_BOUNDS: u32 = 0x253C;

/// Sets in `info` the weight and width classes, the style bits and the metrics the reference
/// computes. `boxes` are the glyphs' boxes by glyph id ([`int_box`]), `font` the font box
/// ([`font_box`]); `cid`: the font is CID-keyed, whose typographic metrics follow other rules.
pub(crate) fn apply(info: &mut Info, source: &Source, order: &[ResolvedGlyph], boxes: &[Option<Bbox>], font: Bbox, units_per_em: u16, cid: bool) {
    let upm = i32::from(units_per_em);
    let em_scale = |v: i32| upm * v / 1000;
    // A glyph mapped to `cp` and its box (all zero when it draws nothing).
    let glyph_box = |cp: u32| -> Option<Bbox> {
        let gid = order.iter().position(|g| g.unicodes.contains(&cp))?;
        Some(boxes.get(gid).copied().flatten().unwrap_or_default())
    };
    // The heights of `o` and `O`, less their overshoot below the baseline.
    let height = |cp: u32| glyph_box(cp).map_or(0, |b| i32::from(b.y_max) + i32::from(b.y_min));
    let x_height = height(0x6F);
    let cap_height = height(0x4F);
    let (typo_ascender, typo_descender, typo_line_gap) = if cid {
        let (ascender, descender) = match glyph_box(VERTICAL_BOUNDS) {
            Some(b) => (i32::from(b.y_max), i32::from(b.y_min)),
            None => (em_scale(880), em_scale(-120)),
        };
        (ascender, descender, ascender - descender)
    } else {
        let d_height = glyph_box(0x64).map_or(0, |b| i32::from(b.y_max));
        let mut ascender = d_height.max(cap_height);
        if ascender == 0 {
            ascender = i32::from(font.y_max).abs();
        }
        ascender = ascender.min(upm);
        let descender = ascender - upm;
        (ascender, descender, em_scale(1200) - ascender + descender)
    };

    let (weight, width) = classes(source.full_name.as_deref());
    info.weight_class = weight;
    info.width_class = width;
    let mut selection = SelectionFlags::empty();
    if source.bold {
        selection |= SelectionFlags::BOLD;
    }
    if source.italic {
        selection |= SelectionFlags::ITALIC;
    }
    info.selection = selection;

    let m = &mut info.metrics;
    m.x_height = f64::from(x_height);
    m.cap_height = f64::from(cap_height);
    m.typo_ascender = f64::from(typo_ascender);
    m.typo_descender = f64::from(typo_descender);
    m.typo_line_gap = f64::from(typo_line_gap);
    m.win_ascent = f64::from(font.y_max);
    m.win_descent = -f64::from(font.y_min);
}

/// What follows the feature code's overrides (`os2_code`, `hhea_code`): a weight class the code
/// sets below 250 raised to 250 when the code leaves the width class to the full name and there
/// is one, as the reference does; the Regular bit when the font is neither bold nor italic and
/// its weight class is 400; and each `hhea` metric the code does not set taken from the
/// typographic one.
pub(crate) fn after_overrides(
    os2: &mut Os2,
    hhea: &mut Hhea,
    source: &Source,
    os2_code: &fontcraft_fea::Os2Values,
    hhea_code: &fontcraft_fea::MetricsValues,
) {
    let named = source.full_name.as_deref().is_some_and(|n| !n.is_empty());
    if os2_code.weight_class.is_some() && os2_code.width_class.is_none() && named {
        os2.us_weight_class = os2.us_weight_class.max(LOWEST_WEIGHT);
    }
    let styled = os2.fs_selection.intersects(SelectionFlags::BOLD | SelectionFlags::ITALIC);
    if !styled && os2.us_weight_class == 400 {
        os2.fs_selection |= SelectionFlags::REGULAR;
    } else {
        os2.fs_selection.remove(SelectionFlags::REGULAR);
    }
    if hhea_code.ascender.is_none() {
        hhea.ascender = FWord::new(os2.s_typo_ascender);
    }
    if hhea_code.descender.is_none() {
        hhea.descender = FWord::new(os2.s_typo_descender);
    }
    if hhea_code.line_gap.is_none() {
        hhea.line_gap = FWord::new(os2.s_typo_line_gap);
    }
}

/// The version string the reference writes for a font revision: three decimals, the fourth
/// dropped after adding half of one.
pub fn version_string(revision: f64) -> String {
    let truncated = ((revision + 0.0005) * 1000.0).trunc() / 1000.0;
    format!("Version {truncated:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_full_name_gives_the_weight_and_width_classes() {
        assert_eq!(classes(None), (400, 5));
        assert_eq!(classes(Some("Synth Light")), (300, 5));
        assert_eq!(classes(Some("Synth ExtraLight Condensed")), (250, 3));
        assert_eq!(classes(Some("Synth Semibold SemiCondensed")), (600, 4));
        assert_eq!(classes(Some("Synth Bold")), (700, 5));
        // The reference's guard for this name is misspelt, so "ultra" wins.
        assert_eq!(classes(Some("Synth UltraCondensed")), (900, 1));
        assert_eq!(classes(Some("Synth Regular")), (400, 5));
    }

    #[test]
    fn glyph_boxes_round_outwards_past_the_margin() {
        let b = fontcraft_cff::Bounds { x_min: 60.0, y_min: -12.3, x_max: 140.26, y_max: 760.4 };
        assert_eq!(int_box(&b), [60, -13, 141, 761]);
        let near = fontcraft_cff::Bounds { x_min: -0.1, y_min: 0.9, x_max: 700.1, y_max: 711.86 };
        assert_eq!(int_box(&near), [0, 1, 700, 712]);
    }

    #[test]
    fn version_strings_keep_three_decimals() {
        assert_eq!(version_string(1.0), "Version 1.000");
        assert_eq!(version_string(2.0049), "Version 2.005");
        assert_eq!(version_string(1.0104), "Version 1.010");
    }
}
