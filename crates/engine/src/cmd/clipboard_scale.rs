//! The `Point To Unit Ratio` parameter: the scale of the SVG `edit.copy` returns. `clipboard.rs` calls this module.
//!
//! The parameter says how many font units make one point. With it, the copied drawing is
//! written in points (font units divided by the ratio) and declares its width and height in `pt`,
//! so a drawing application pastes it at that size. Without it, or with a value that is not a
//! number within [`MIN_RATIO`]..=[`MAX_RATIO`], the SVG keeps one font unit per user unit and no
//! unit on its size. The master's parameter wins over the font's.
//!
//! The parameter adds no command: it changes what `edit.copy` and `edit.cut` return.
//!
//! Row: `Point To Unit Ratio` (M7.11).

use fontcraft_geom::{Affine, BezPath};
use fontcraft_io_svg::{SvgError, bez_to_svg};
use fontcraft_model::{Component, Font, MAX_OUTLINE_ELEMENTS, Shape};

use super::*;

/// The parameter's name.
pub const PARAMETER: &str = "Point To Unit Ratio";
/// Smallest ratio used; a smaller one is ignored.
pub const MIN_RATIO: f64 = 0.001;
/// Largest ratio used; a larger one is ignored.
pub const MAX_RATIO: f64 = 1_000_000.0;

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}

/// The units per point of `master` (its own parameter, else the font's), when one is set and
/// usable.
pub fn ratio(font: &Font, master: &str) -> Option<f64> {
    let own = font.master(master).ok().and_then(|m| m.params.get(PARAMETER));
    let value = own.or_else(|| font.info.params.get(PARAMETER))?;
    fontcraft_fontinfo::value::number(value).filter(|r| (MIN_RATIO..=MAX_RATIO).contains(r))
}

/// The SVG flavour of copied `shapes` drawn in `master`, at the scale [`ratio`] gives: `svg`
/// (one font unit per user unit) when the font sets none.
pub fn scale_copy(font: &Font, master: &str, shapes: &[Shape], svg: String) -> std::result::Result<String, SvgError> {
    let Some(ratio) = ratio(font, master) else {
        return Ok(svg);
    };
    let outline = |c: &Component| font.component_outline(c, master).ok();
    let mut all = BezPath::new();
    for shape in shapes {
        match shape {
            Shape::Path(p) => all.extend(p.to_bez().map_err(SvgError::BadPath)?),
            Shape::Component(c) => all.extend(outline(c).unwrap_or_default()),
            Shape::Image(_) | Shape::Raw(_) => {}
        }
        if all.elements().len() > MAX_OUTLINE_ELEMENTS {
            return Err(SvgError::BadPath(format!("the outline has more than {MAX_OUTLINE_ELEMENTS} segments")));
        }
    }
    all.apply_affine(Affine::scale(1.0 / ratio));
    let svg = bez_to_svg(&all, None);
    Ok(with_unit(&with_unit(&svg, "width"), "height"))
}

/// `svg` with `pt` after the value of its first `attr` attribute: the root element's, which
/// [`bez_to_svg`] writes first.
fn with_unit(svg: &str, attr: &str) -> String {
    let key = format!(" {attr}=\"");
    let Some(start) = svg.find(&key).map(|at| at + key.len()) else {
        return svg.to_string();
    };
    let Some(end) = svg.get(start..).and_then(|rest| rest.find('"')).map(|len| start + len) else {
        return svg.to_string();
    };
    match (svg.get(..end), svg.get(end..)) {
        (Some(head), Some(tail)) => format!("{head}pt{tail}"),
        _ => svg.to_string(),
    }
}
