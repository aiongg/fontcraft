//! Icons of shape operations craft's icon set has no name for: union and intersect of paths, and slanting a shape
//! left, right, down or up.
//!
//! Meant for craft-ui-contract, `icons/icons.toml` and `src/primitives/icon.rs`: named icons `union`, `intersect`,
//! `slant_left`, `slant_right`, `slant_down` and `slant_up`, drawn as SVG files of the suite's own set, so
//! `IconId::new("union")` names them as it names `squares_subtract`. Until then they are value glyphs
//! ([`Glyph`]) drawn here in code, in the icon's ink and craft's thin stroke.
//!
//! The drawings are original. Union and intersect show two overlapping squares with the result filled; a slant shows
//! an upright square faintly and the slanted shape over it. They show document geometry, so they never mirror.

use craft::ui::primitives::{Glyph, IconSpec};
use craft::ui::style::UiTokens;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// The share of the icon's side the stroke may take at most.
const STROKE_SHARE: f32 = 0.12;
/// The share of the ink the outline of an input shape or of the unslanted square keeps.
const FAINT: f32 = 0.45;
/// How far a slant moves an edge, as a share of the icon's side.
const SHEAR: f32 = 0.12;

/// The first square of the boolean icons, in the unit square (y down).
const FIRST: [f32; 4] = [0.1, 0.1, 0.62, 0.62];
/// The second square of the boolean icons.
const SECOND: [f32; 4] = [0.38, 0.38, 0.9, 0.9];

/// A shape-operation icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft::ui::contract::MemoProps)]
#[non_exhaustive]
pub enum ShapeIcon {
    /// Two squares merged into one shape.
    Union,
    /// Two squares, only their overlap kept.
    Intersect,
    /// The top edge moved left.
    SlantLeft,
    /// The top edge moved right.
    SlantRight,
    /// The right edge moved down.
    SlantDown,
    /// The right edge moved up.
    SlantUp,
}

/// One part of a drawing, in the unit square (y down).
#[derive(Clone, Debug, PartialEq)]
enum Part {
    /// A filled rectangle `[left, top, right, bottom]`.
    Fill([f32; 4]),
    /// A closed outline in the full ink.
    Outline(Vec<Pos2>),
    /// A closed outline in the faint ink.
    Faint(Vec<Pos2>),
}

impl ShapeIcon {
    /// The icon as an [`IconSpec`], for `IconButton::new` and the other places that take one.
    pub fn spec(self) -> IconSpec<'static> {
        IconSpec::glyph(match self {
            ShapeIcon::Union => &ShapeIcon::Union,
            ShapeIcon::Intersect => &ShapeIcon::Intersect,
            ShapeIcon::SlantLeft => &ShapeIcon::SlantLeft,
            ShapeIcon::SlantRight => &ShapeIcon::SlantRight,
            ShapeIcon::SlantDown => &ShapeIcon::SlantDown,
            ShapeIcon::SlantUp => &ShapeIcon::SlantUp,
        })
    }

    /// The drawing in the unit square.
    fn parts(self) -> Vec<Part> {
        let square = |[l, t, r, b]: [f32; 4]| vec![pos2(l, t), pos2(r, t), pos2(r, b), pos2(l, b)];
        let upright = [0.25, 0.2, 0.75, 0.8];
        match self {
            ShapeIcon::Union => {
                let [l1, t1, r1, b1] = FIRST;
                let [l2, t2, r2, b2] = SECOND;
                let outline = vec![pos2(l1, t1), pos2(r1, t1), pos2(r1, t2), pos2(r2, t2), pos2(r2, b2), pos2(l2, b2), pos2(l2, b1), pos2(l1, b1)];
                vec![Part::Fill(FIRST), Part::Fill(SECOND), Part::Outline(outline)]
            }
            ShapeIcon::Intersect => {
                let ([_, _, r1, b1], [l2, t2, _, _]) = (FIRST, SECOND);
                let overlap = [l2, t2, r1, b1];
                vec![Part::Faint(square(FIRST)), Part::Faint(square(SECOND)), Part::Fill(overlap), Part::Outline(square(overlap))]
            }
            ShapeIcon::SlantLeft | ShapeIcon::SlantRight => {
                let [l, t, r, b] = upright;
                let d = if self == ShapeIcon::SlantRight { SHEAR } else { -SHEAR };
                let slanted = vec![pos2(l + d, t), pos2(r + d, t), pos2(r - d, b), pos2(l - d, b)];
                vec![Part::Faint(square(upright)), Part::Outline(slanted)]
            }
            ShapeIcon::SlantDown | ShapeIcon::SlantUp => {
                let [t, l, b, r] = upright;
                let d = if self == ShapeIcon::SlantDown { SHEAR } else { -SHEAR };
                let slanted = vec![pos2(l, t - d), pos2(r, t + d), pos2(r, b + d), pos2(l, b - d)];
                vec![Part::Faint(square([l, t, r, b])), Part::Outline(slanted)]
            }
        }
    }
}

impl Glyph for ShapeIcon {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
        let thin = tokens.stroke.thin;
        let width = if thin.is_finite() { thin.clamp(0.0, rect.width().min(rect.height()) * STROKE_SHARE) } else { 0.0 };
        let at = |p: Pos2| rect.min + egui::vec2(p.x * rect.width(), p.y * rect.height());
        for part in self.parts() {
            match part {
                Part::Fill([l, t, r, b]) => {
                    painter.rect_filled(Rect::from_min_max(at(pos2(l, t)), at(pos2(r, b))), 0.0, color);
                }
                Part::Outline(points) => {
                    painter.add(Shape::closed_line(points.into_iter().map(at).collect(), Stroke::new(width, color)));
                }
                Part::Faint(points) => {
                    painter.add(Shape::closed_line(points.into_iter().map(at).collect(), Stroke::new(width, color.gamma_multiply(FAINT))));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Part, ShapeIcon};

    const ALL: [ShapeIcon; 6] =
        [ShapeIcon::Union, ShapeIcon::Intersect, ShapeIcon::SlantLeft, ShapeIcon::SlantRight, ShapeIcon::SlantDown, ShapeIcon::SlantUp];

    fn points(part: &Part) -> Vec<egui::Pos2> {
        match part {
            Part::Fill([l, t, r, b]) => vec![egui::pos2(*l, *t), egui::pos2(*r, *b)],
            Part::Outline(p) | Part::Faint(p) => p.clone(),
        }
    }

    /// Every drawing stays inside the icon's square, so nothing is clipped, and the slants of a pair are mirror images.
    #[test]
    fn the_drawings_fit_and_the_slants_mirror() {
        for icon in ALL {
            for part in icon.parts() {
                for p in points(&part) {
                    assert!((0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y), "{icon:?} leaves the square at {p:?}");
                }
            }
        }
        let outline = |icon: ShapeIcon| icon.parts().into_iter().find_map(|part| if let Part::Outline(p) = part { Some(p) } else { None });
        let left = outline(ShapeIcon::SlantLeft).expect("the left slant's outline");
        let right = outline(ShapeIcon::SlantRight).expect("the right slant's outline");
        let mirrored: Vec<_> = right.iter().map(|p| egui::pos2(1.0 - p.x, p.y)).collect();
        assert!(left.iter().all(|p| mirrored.iter().any(|q| (p.x - q.x).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6)));
        assert_ne!(ShapeIcon::Union.spec(), ShapeIcon::Intersect.spec());
    }
}
