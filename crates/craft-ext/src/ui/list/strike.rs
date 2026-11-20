//! A struck-through label on a list row: a rule through the label's text in the row's ink, for an entry that is
//! switched off but still listed (a disabled feature).
//!
//! Meant for craft-ui-widgets, `src/list/row.rs`: there it is a fourth `RowInk` (or a `struck` prop on `ListRow`)
//! that `paint_text` turns into a strikethrough span, the way `DocRun::strikethrough` draws one. `ListRow`'s fields
//! are private to craft, so this stand-in paints the rule after the `List` has shown its rows, from the rows' places
//! in `CollectionResponse::shown`. It covers the plain row: no toggle cells, icon, thumbnail, tree indent or detail
//! line, the label set in the body role and centred on the row. The rule stops at the row's end padding; a label
//! elided before trailing items keeps its rule past the ellipsis.

use craft::theme::Srgba;
use craft::ui::contract::CollectionResponse;
use craft::ui::list::RowInk;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::style::{StyleCx, TypeRole};
use egui::{Color32, Pos2, Rect, Stroke, Ui};

/// How one row's label is struck.
#[derive(Clone, Debug, PartialEq)]
pub struct Strike {
    /// The label as the row shows it.
    text: Label,
    /// The ink the row's label reads in.
    ink: RowInk,
    /// The row is selected: the rule takes the ink that reads on the selection.
    selected: bool,
}

impl Strike {
    /// The label `text`, as the row shows it, struck in the normal ink.
    pub fn new(text: impl Into<Label>) -> Self {
        Strike { text: text.into(), ink: RowInk::Normal, selected: false }
    }

    /// The ink the row's label reads in (`ListRow::ink`).
    #[must_use]
    pub fn ink(mut self, ink: RowInk) -> Self {
        self.ink = ink;
        self
    }

    /// The row is selected.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

/// Where a struck label's rule lies in a row: from the label's start for the label's width, at the middle of the row,
/// clipped to the row's content. `None` when no room is left for it.
pub fn strike_line(row: Rect, pad: f32, text_width: f32, rtl: bool) -> Option<[Pos2; 2]> {
    if !(pad.is_finite() && text_width.is_finite()) || text_width <= 0.0 {
        return None;
    }
    let room = (row.width() - 2.0 * pad).max(0.0);
    let width = text_width.min(room);
    if width <= 0.0 {
        return None;
    }
    let y = row.center().y;
    let (start, end) = if rtl { (row.right() - pad - width, row.right() - pad) } else { (row.left() + pad, row.left() + pad + width) };
    Some([Pos2::new(start, y), Pos2::new(end, y)])
}

/// Strikes the labels of a `List`'s rows through: after the list has shown, `struck` names each shown row's strike,
/// `None` for a row that reads plainly.
pub trait StrikeRows<K> {
    /// Paints the rule through each struck row's label, over the list's own painting.
    fn strike_rows(&self, ui: &Ui, struck: impl Fn(&K) -> Option<Strike>);
}

impl<K, E> StrikeRows<K> for CollectionResponse<K, E> {
    fn strike_rows(&self, ui: &Ui, struck: impl Fn(&K) -> Option<Strike>) {
        let env = Env::of(ui);
        let cx = StyleCx::of_env(ui.ctx(), &env);
        let pad = cx.control().pad_x;
        let rtl = cx.direction.is_rtl();
        let text = &env.tokens.color.text;
        let width = env.tokens.stroke.thin;
        let painter = ui.painter_at(self.response.rect);
        for row in &self.shown {
            let Some(strike) = struck(&row.key) else { continue };
            let label = env.resolve(&strike.text);
            let Some(points) = strike_line(row.rect, pad, cx.text_width(TypeRole::Body, label.as_str()), rtl) else { continue };
            let ink = if strike.selected {
                text.on_selected
            } else {
                match strike.ink {
                    RowInk::Muted => text.muted,
                    RowInk::Dimmed => text.faint,
                    _ => text.normal,
                }
            };
            painter.line_segment(points, Stroke::new(width, Color32::from(Srgba::from(ink))));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use craft::behavior::keyed;
    use craft::ui::contract::ShowCollection as _;
    use craft::ui::list::{List, ListRow};
    use craft::ui::testing::Fixture;
    use egui::{Rect, Shape, pos2};

    use super::*;

    #[test]
    fn the_rule_starts_at_the_label_and_mirrors() {
        let row = Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 20.0));
        assert_eq!(strike_line(row, 6.0, 50.0, false), Some([pos2(6.0, 10.0), pos2(56.0, 10.0)]));
        assert_eq!(strike_line(row, 6.0, 50.0, true), Some([pos2(144.0, 10.0), pos2(194.0, 10.0)]));
        // A label wider than the row is struck to the row's end padding; an empty one is not struck.
        assert_eq!(strike_line(row, 6.0, 500.0, false), Some([pos2(6.0, 10.0), pos2(194.0, 10.0)]));
        assert_eq!(strike_line(row, 6.0, 0.0, false), None);
        assert_eq!(strike_line(row, f32::NAN, 50.0, false), None);
    }

    /// Only the struck row gets a rule, and it lies inside that row.
    #[test]
    fn a_struck_row_paints_one_rule_inside_its_row() {
        let names = ["kern", "liga", "ss01"];
        let rows = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = std::rc::Rc::clone(&rows);
        let mut fixture = Fixture::new(move |ui| {
            let positions: Vec<usize> = (0..names.len()).collect();
            let source = keyed(&positions, |at| *at).with_text(|at| Cow::Borrowed(names.get(*at).copied().unwrap_or("")));
            let out = List::<usize>::new("Features")
                .header(false)
                .natural_height(true)
                .show_items(ui, &source, &mut |at, icx| ListRow::new(names.get(*at).copied().unwrap_or("")).state(icx.row.clone()));
            out.strike_rows(ui, |at| (*at == 1).then(|| Strike::new("liga").ink(RowInk::Dimmed)));
            *seen.borrow_mut() = out.shown.iter().map(|row| (row.key, row.rect)).collect::<Vec<_>>();
        });
        fixture.settle();
        let shapes = fixture.frame_shapes();
        let liga = rows.borrow().iter().find(|(key, _)| *key == 1).map(|(_, rect)| *rect).expect("the liga row is shown");
        let rules: Vec<[Pos2; 2]> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::LineSegment { points, .. } => Some(*points),
                _ => None,
            })
            .filter(|[a, b]| (a.y - b.y).abs() < f32::EPSILON && (a.y - liga.center().y).abs() < 0.5)
            .collect();
        assert_eq!(rules.len(), 1, "{rules:?}");
        let [a, b] = *rules.first().expect("one rule");
        assert!(liga.contains(a) && liga.contains(b) && b.x > a.x, "{a:?} {b:?} in {liga:?}");
    }
}
