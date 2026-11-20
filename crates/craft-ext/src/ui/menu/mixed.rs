//! The mixed mark on a menu entry: a dash in the mark column, for a switch that is on for some of the items a menu
//! acts on and off for the others (a tag that some of the selected glyphs carry).
//!
//! Meant for craft-ui-contract, `src/contract/entry.rs` (a `Mixed` state beside `checked`, from
//! `craft_behavior::Mixed`) and craft-ui-widgets, `src/menu/item.rs` (a `Mark::Mixed` that `MenuItem::paint` draws
//! with `glyph::dash`, and a mixed checked state on the row's AccessKit node). `Entry` and the row's mark are private
//! to craft, so this stand-in wraps an `Entry` in [`MarkedEntry`] and shows a mixed one as an unchecked row that keeps
//! its mark column, then paints the dash into that column. A screen reader hears the row as unchecked.

use craft::theme::Srgba;
use craft::ui::contract::{Entry, Props as _, Show as _, TooltipSpec};
use craft::ui::menu::{MenuItem, entry_rows as craft_entry_rows};
use craft::ui::runtime::env::Env;
use craft::ui::style::StyleCx;
use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};

/// The share of the mark's side the dash spans, as craft's check box dash.
const DASH: f32 = 0.5;

/// The mark a menu entry carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EntryMark {
    /// No mark.
    #[default]
    Unchecked,
    /// A check mark.
    Checked,
    /// A dash: on for some of what the menu acts on, off for the rest.
    Mixed,
}

impl EntryMark {
    /// The mark of a switch that is on for `on` of `total` items: none when none is on, the check when all are, the
    /// dash between.
    pub fn of_count(on: usize, total: usize) -> EntryMark {
        match on {
            0 => EntryMark::Unchecked,
            n if n >= total => EntryMark::Checked,
            _ => EntryMark::Mixed,
        }
    }
}

/// An entry with its mark.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkedEntry<T> {
    entry: Entry<T>,
    mark: EntryMark,
}

impl<T> MarkedEntry<T> {
    /// `entry` with the mark its checked state gives it.
    pub fn new(entry: Entry<T>) -> Self {
        let mark = if entry.is_checked() { EntryMark::Checked } else { EntryMark::Unchecked };
        MarkedEntry { entry, mark }
    }

    /// The entry carries `mark`.
    #[must_use]
    pub fn mark(mut self, mark: EntryMark) -> Self {
        self.entry = self.entry.checked(mark == EntryMark::Checked);
        self.mark = mark;
        self
    }

    /// The entry it wraps.
    pub fn entry(&self) -> &Entry<T> {
        &self.entry
    }

    /// The mark it carries.
    pub fn marked(&self) -> EntryMark {
        self.mark
    }

    /// The value it reports when picked; `None` for a separator or a heading.
    pub fn value(&self) -> Option<&T> {
        self.entry.value()
    }
}

impl<T> From<Entry<T>> for MarkedEntry<T> {
    fn from(entry: Entry<T>) -> Self {
        MarkedEntry::new(entry)
    }
}

/// The dash of a mixed row: across the middle of the mark square at the start of the row's content, mirrored in a
/// right-to-left interface.
pub fn mixed_dash(row: Rect, pad: f32, side: f32, rtl: bool) -> Option<[Pos2; 2]> {
    if !(pad.is_finite() && side.is_finite()) || side <= 0.0 || row.width() < 2.0 * pad + side {
        return None;
    }
    let x = if rtl { row.right() - pad - side / 2.0 } else { row.left() + pad + side / 2.0 };
    let half = Vec2::new(side * DASH / 2.0, 0.0);
    let centre = Pos2::new(x, row.center().y);
    Some([centre - half, centre + half])
}

/// [`craft::ui::menu::entry_rows`] over marked entries: one row per entry, a caption for a heading, a rule for a
/// separator; a checked entry carries the check and a mixed one the dash. Returns the index of the entry that ran.
pub fn entry_rows<T>(ui: &mut Ui, entries: &[MarkedEntry<T>]) -> Option<usize> {
    let mut ran = None;
    for (at, marked) in entries.iter().enumerate() {
        let picked = if marked.mark == EntryMark::Mixed && marked.entry.is_item() {
            mixed_row(ui, &marked.entry)
        } else {
            craft_entry_rows(ui, std::slice::from_ref(&marked.entry)).is_some()
        };
        if picked {
            ran = Some(at);
        }
    }
    ran
}

/// Shows `entry` as a row that keeps its mark column empty, paints the dash there, and returns whether it ran.
fn mixed_row<T>(ui: &mut Ui, entry: &Entry<T>) -> bool {
    let mut row = MenuItem::new(entry.label().clone()).enabled(entry.is_enabled()).check(false);
    if let Some(icon) = entry.icon_spec() {
        row = row.icon(icon.clone());
    }
    if let Some(reason) = entry.why_disabled() {
        row = row.description(reason.as_str()).tooltip(TooltipSpec::new(reason.clone()).disabled_text(reason.clone()));
    }
    let shown = row.show(ui);
    let env = Env::of(ui);
    let cx = StyleCx::of_env(ui.ctx(), &env);
    let text = &env.tokens.color.text;
    let ink = if !entry.is_enabled() {
        text.disabled
    } else if shown.active {
        text.on_accent
    } else {
        text.normal
    };
    let rect = shown.response.rect;
    if let Some(points) = mixed_dash(rect, env.tokens.space.pad.regular, cx.control().icon, cx.direction.is_rtl()) {
        ui.painter_at(rect).line_segment(points, Stroke::new(env.tokens.stroke.medium, Color32::from(Srgba::from(ink))));
    }
    shown.ran
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::overlay::Anchor;
    use craft::ui::contract::{Bind, Entry, ShowContainer as _};
    use craft::ui::menu::Menu;
    use craft::ui::testing::Fixture;
    use egui::{Shape, pos2};

    use super::*;

    #[test]
    fn counts_give_the_mark() {
        assert_eq!(EntryMark::of_count(0, 3), EntryMark::Unchecked);
        assert_eq!(EntryMark::of_count(2, 3), EntryMark::Mixed);
        assert_eq!(EntryMark::of_count(3, 3), EntryMark::Checked);
        assert_eq!(EntryMark::of_count(0, 0), EntryMark::Unchecked);
    }

    #[test]
    fn the_dash_sits_in_the_mark_column_and_mirrors() {
        let row = Rect::from_min_max(pos2(0.0, 0.0), pos2(120.0, 20.0));
        assert_eq!(mixed_dash(row, 8.0, 16.0, false), Some([pos2(12.0, 10.0), pos2(20.0, 10.0)]));
        assert_eq!(mixed_dash(row, 8.0, 16.0, true), Some([pos2(100.0, 10.0), pos2(108.0, 10.0)]));
        assert_eq!(mixed_dash(Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 20.0)), 8.0, 16.0, false), None);
    }

    /// In an open menu the mixed entry paints its dash inside its own row, and picking it reports its index.
    #[test]
    fn a_mixed_entry_shows_a_dash_and_runs() {
        let picked = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&picked);
        let mut fixture = Fixture::new(move |ui| {
            let mut open = true;
            let opener = ui.response();
            let entries = [
                MarkedEntry::new(Entry::new("all", "Exports").checked(true)),
                MarkedEntry::new(Entry::new("some", "Tag Some")).mark(EntryMark::Mixed),
                MarkedEntry::new(Entry::new("none", "Tag None")),
            ];
            let shown = Menu::context("Glyph Menu", &opener)
                .anchor(Anchor::Point(pos2(20.0, 20.0)))
                .open(Bind::bound(&mut open))
                .show(ui, |ui, _| entry_rows(ui, &entries));
            if let Some(at) = shown.inner.flatten() {
                *seen.borrow_mut() = entries.get(at).and_then(MarkedEntry::value).copied();
            }
        });
        fixture.settle();
        let row = fixture.by_label("Tag Some").expect("the mixed row");
        let rect = row.rect;
        let dashes = fixture
            .frame_shapes()
            .into_iter()
            .filter(|shape| matches!(shape, Shape::LineSegment { points: [a, b], .. } if rect.contains(*a) && rect.contains(*b) && (a.y - b.y).abs() < f32::EPSILON))
            .count();
        assert_eq!(dashes, 1);
        fixture.click(&row).settle();
        assert_eq!(*picked.borrow(), Some("some"));
    }
}
