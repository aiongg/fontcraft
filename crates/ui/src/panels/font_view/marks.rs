//! What a grid cell draws around its glyph (`app::models::cell_marks`): the colour labels behind it, the badges in
//! the cell's corners and the Unicode mark at the bottom right, and the words that say the same in the tooltip and to
//! assistive technology.

use craft::ui::primitives::ContentPainterCx;
use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use super::FontViewWords;
use crate::app::models::cell_marks::{CellMarks, UnicodeMark};

/// The colours and sizes of a cell's marks, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkTokens {
    /// How opaque a colour label is.
    pub label_alpha: u8,
    /// The cut-out of a layer colour alone: the legs of the triangle, as a share of the cell.
    pub cutout: f32,
    /// A badge's side: this share of the cell, between the two lengths.
    pub badge_share: f32,
    pub badge_min: f32,
    pub badge_max: f32,
    /// What a badge keeps clear of the cell's edges.
    pub pad: f32,
    /// What the annotation badge leaves to the corner of a glyph whose layers cannot interpolate, as a share of the
    /// cell.
    pub corner_share: f32,
    pub note: Color32,
    pub note_fold: Color32,
    pub info: Color32,
    pub info_text: Color32,
    pub not_exported: Color32,
    pub not_exported_width: f32,
    pub out_of_sync: Color32,
    pub out_of_sync_mark: Color32,
    pub marker: Color32,
    pub marker_side: f32,
    /// The character and the code point.
    pub char_size: f32,
    pub value_size: f32,
}

impl MarkTokens {
    pub const fn standard() -> MarkTokens {
        MarkTokens {
            label_alpha: 150,
            cutout: 0.3,
            badge_share: 0.16,
            badge_min: 5.0,
            badge_max: 12.0,
            pad: 2.0,
            corner_share: 0.14,
            note: Color32::from_rgb(0xf5, 0xd0, 0x40),
            note_fold: Color32::from_rgb(0xa8, 0x86, 0x10),
            info: Color32::from_rgb(0x10, 0x10, 0x10),
            info_text: Color32::WHITE,
            not_exported: Color32::from_rgb(0xd8, 0x30, 0x30),
            not_exported_width: 1.5,
            out_of_sync: Color32::from_rgb(0xf2, 0xa1, 0x1d),
            out_of_sync_mark: Color32::from_rgb(0x10, 0x10, 0x10),
            marker: Color32::from_rgb(0x8a, 0x8a, 0x8a),
            marker_side: 5.0,
            char_size: 11.0,
            value_size: 10.0,
        }
    }

    /// A colour label's tint.
    pub fn tint(&self, [r, g, b]: [u8; 3]) -> Color32 {
        Color32::from_rgba_unmultiplied(r, g, b, self.label_alpha)
    }
}

/// A badge of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Annotation,
    CustomInfo,
    NotExported,
    OutOfSync,
}

/// The badges `marks` shows.
pub fn badges(marks: &CellMarks) -> impl Iterator<Item = Badge> {
    [
        (marks.annotated, Badge::Annotation),
        (marks.custom, Badge::CustomInfo),
        (marks.not_exported, Badge::NotExported),
        (marks.out_of_sync, Badge::OutOfSync),
    ]
    .into_iter()
    .filter_map(|(on, badge)| on.then_some(badge))
}

/// Where `badge` lies in the cell `rect`: the annotation at the top left past the incompatibility corner, the custom
/// info at the bottom left, the ring of a glyph that does not export at the top centre, the warning at the top right.
pub fn badge_rect(rect: Rect, badge: Badge, t: &MarkTokens) -> Rect {
    let cell = rect.width().min(rect.height()).max(0.0);
    let side = Vec2::splat((cell * t.badge_share).clamp(t.badge_min, t.badge_max));
    let inner = rect.shrink(t.pad);
    let min = match badge {
        Badge::Annotation => pos2(rect.left() + cell * t.corner_share + t.pad, inner.top()),
        Badge::CustomInfo => pos2(inner.left(), inner.bottom() - side.y),
        Badge::NotExported => pos2(rect.center().x - side.x / 2.0, inner.top()),
        Badge::OutOfSync => pos2(inner.right() - side.x, inner.top()),
    };
    Rect::from_min_size(min, side)
}

/// The colour labels, behind the glyph: the glyph's colour fills the cell; a layer colour alone fills it but for the
/// cut-out at the top left; with both, the layer colour covers the right half.
pub fn paint_colors(cx: &ContentPainterCx<'_>, rect: Rect, marks: &CellMarks, t: &MarkTokens) {
    match (marks.color, marks.layer_color) {
        (Some(glyph), None) => {
            cx.painter.rect_filled(rect, 0.0, t.tint(glyph));
        }
        (None, Some(layer)) => {
            let cut = rect.width().min(rect.height()).max(0.0) * t.cutout;
            let points = vec![rect.min + vec2(cut, 0.0), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.min + vec2(0.0, cut)];
            cx.painter.add(Shape::convex_polygon(points, t.tint(layer), Stroke::NONE));
        }
        (Some(glyph), Some(layer)) => {
            let middle = rect.center().x;
            cx.painter.rect_filled(Rect::from_min_max(rect.min, pos2(middle, rect.bottom())), 0.0, t.tint(glyph));
            cx.painter.rect_filled(Rect::from_min_max(pos2(middle, rect.top()), rect.max), 0.0, t.tint(layer));
        }
        (None, None) => {}
    }
}

/// The badges, over the glyph.
pub fn paint_badges(cx: &ContentPainterCx<'_>, rect: Rect, marks: &CellMarks, t: &MarkTokens) {
    for badge in badges(marks) {
        let r = badge_rect(rect, badge, t);
        let side = r.width();
        match badge {
            Badge::Annotation => {
                cx.painter.rect_filled(r, 0.0, t.note);
                let fold = side * 0.35;
                let points = vec![r.right_bottom() - vec2(fold, 0.0), r.right_bottom() - vec2(0.0, fold), r.right_bottom()];
                cx.painter.add(Shape::convex_polygon(points, t.note_fold, Stroke::NONE));
            }
            Badge::CustomInfo => {
                cx.painter.circle_filled(r.center(), side / 2.0, t.info);
                cx.painter.text(r.center(), Align2::CENTER_CENTER, "i", FontId::proportional(side * 0.85), t.info_text);
            }
            Badge::NotExported => {
                let stroke = Stroke::new(t.not_exported_width, t.not_exported);
                let radius = (side / 2.0 - t.not_exported_width / 2.0).max(0.0);
                cx.painter.circle_stroke(r.center(), radius, stroke);
                let d = radius * std::f32::consts::FRAC_1_SQRT_2;
                cx.painter.line_segment([r.center() + vec2(-d, d), r.center() + vec2(d, -d)], stroke);
            }
            Badge::OutOfSync => {
                cx.painter.add(Shape::convex_polygon(vec![r.center_top(), r.right_bottom(), r.left_bottom()], t.out_of_sync, Stroke::NONE));
                let x = r.center().x;
                let at = |share: f32| -> Pos2 { pos2(x, r.top() + side * share) };
                cx.painter.line_segment([at(0.35), at(0.7)], Stroke::new(side * 0.12, t.out_of_sync_mark));
                cx.painter.circle_filled(at(0.84), side * 0.07, t.out_of_sync_mark);
            }
        }
    }
}

/// The Unicode mark, its right end at the cell's bottom right corner, beside the name under the cell.
pub fn paint_unicode(cx: &ContentPainterCx<'_>, rect: Rect, mark: &UnicodeMark, t: &MarkTokens) {
    let corner = rect.shrink(t.pad).right_bottom();
    let marker = || Rect::from_min_max(corner - Vec2::splat(t.marker_side), corner);
    match mark {
        UnicodeMark::None => {}
        UnicodeMark::Char(c) => {
            let font = FontId::proportional(t.char_size);
            if cx.painter.ctx().fonts_mut(|f| f.has_glyph(&font, *c)) {
                cx.painter.text(corner, Align2::RIGHT_BOTTOM, *c, font, cx.ink());
            } else {
                cx.painter.rect_filled(marker(), 0.0, t.marker);
            }
        }
        UnicodeMark::Value(v) => {
            cx.painter.text(corner, Align2::RIGHT_BOTTOM, v, FontId::proportional(t.value_size), t.marker);
        }
        UnicodeMark::Marker => {
            cx.painter.rect_filled(marker(), 0.0, t.marker);
        }
    }
}

/// What the badges say, in words.
pub fn states(words: &FontViewWords, marks: &CellMarks) -> Vec<String> {
    badges(marks)
        .map(|badge| match badge {
            Badge::Annotation => words.badge_annotation.as_str(),
            Badge::CustomInfo => words.badge_custom.as_str(),
            Badge::NotExported => words.badge_not_exported.as_str(),
            Badge::OutOfSync => words.badge_out_of_sync.as_str(),
        })
        .map(str::to_string)
        .collect()
}
