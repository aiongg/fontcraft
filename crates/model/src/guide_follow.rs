//! A guide's sidebearing reference (`guides.sidebearingReference`): what its stored x counts from
//! and what it follows when its layer's width changes.
//!
//! The model keeps a guide's `orientation` and position as a `.glyphs` file does: absent or `left`
//! for the left sidebearing, `right`, or `center` for both. A right-referenced guide's x counts
//! from the right sidebearing, so it is drawn at the layer's width plus x, a global guide in every
//! glyph at that glyph's own width plus x. The x of a guide referenced to the left or to both is
//! its place (what a both-referenced one counts from is not documented, so its number is taken as
//! it stands).
//!
//! Whoever needs a guide's place in glyph space asks [`placed_x`] or [`placed`] with the width of
//! the layer it is shown in, and stores a place through [`place_x`] or [`edit_placed`]. Commands
//! take and return places; the files, the native format and the clipboard hold the stored x.
//!
//! When a width changes, a right-referenced guide keeps its stored x and so its distance from the
//! right edge. A local guide referenced to both keeps its fraction of the width in a document of
//! version 2 ([`BothGuides`]); elsewhere no change of width or sidebearings writes its x, which may
//! already count from something that moves. Every command that writes a layer's advance width calls
//! [`follow_width`] in the same edit, so the guides move in the same undo step.

use crate::{Font, Guide, Layer, MAX_COORDINATE};

/// The spellings of the references a file writes; an absent one is the left.
const RIGHT: &str = "right";
const BOTH: &str = "center";

/// What a guide's x position counts from and follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reference {
    Left,
    Right,
    Both,
}

impl Reference {
    /// The spelling the guide commands use: `left`, `right` or `both`.
    pub fn name(self) -> &'static str {
        match self {
            Reference::Left => "left",
            Reference::Right => "right",
            Reference::Both => "both",
        }
    }
}

/// A guide's sidebearing reference; a value the format does not name reads as the left one.
pub fn reference(g: &Guide) -> Reference {
    match g.orientation.as_deref() {
        Some(RIGHT) => Reference::Right,
        Some(BOTH) => Reference::Both,
        _ => Reference::Left,
    }
}

/// Sets a guide's sidebearing reference, keeping its place in a layer `width` wide.
pub fn set_reference(g: &mut Guide, to: Reference, width: f64) {
    let x = placed_x(g, width);
    g.orientation = match to {
        Reference::Right => Some(RIGHT.into()),
        Reference::Both => Some(BOTH.into()),
        Reference::Left => None,
    };
    place_x(g, x, width);
}

/// Sums of a width and an offset are rounded to one part of a unit in this many, so that an x goes
/// into the stored form and comes out again as it was given.
const STEPS: f64 = 1e6;

fn rounded(v: f64) -> f64 {
    let r = (v * STEPS).round() / STEPS;
    if r.is_finite() { r } else { v }
}

/// What a right-referenced guide's x is added to; a width that is not a number counts as 0.
fn right_edge(width: f64) -> f64 {
    if width.is_finite() { width } else { 0.0 }
}

/// The x of a guide in the glyph space of a layer `width` wide.
pub fn placed_x(g: &Guide, width: f64) -> f64 {
    match reference(g) {
        Reference::Right => rounded(right_edge(width) + g.x),
        Reference::Left | Reference::Both => g.x,
    }
}

/// Stores `x`, a place in the glyph space of a layer `width` wide, by the guide's reference,
/// within the model's coordinate range.
pub fn place_x(g: &mut Guide, x: f64, width: f64) {
    g.x = match reference(g) {
        // Unchanged, so that a guide put back where it is keeps its stored digits.
        Reference::Right if x == placed_x(g, width) => return,
        Reference::Right => rounded(x - right_edge(width)).clamp(-MAX_COORDINATE, MAX_COORDINATE),
        Reference::Left | Reference::Both => x,
    };
}

/// A copy of a guide for reading in the glyph space of a layer `width` wide: its x is its place,
/// and it has no reference to be added to it again. Not for storing: [`edit_placed`] changes a
/// stored guide.
pub fn placed(g: &Guide, width: f64) -> Guide {
    Guide { x: placed_x(g, width), orientation: None, ..g.clone() }
}

/// Runs `f` on a guide placed in the glyph space of a layer `width` wide and stores what it leaves.
/// The reference stays, whatever `f` does; after an error the guide is as it was.
pub fn edit_placed<T, E>(g: &mut Guide, width: f64, f: impl FnOnce(&mut Guide) -> Result<T, E>) -> Result<T, E> {
    let mut at = placed(g, width);
    let out = f(&mut at)?;
    let x = std::mem::replace(&mut at.x, g.x);
    at.orientation = g.orientation.take();
    place_x(&mut at, x, width);
    *g = at;
    Ok(out)
}

/// Whether changes of width and sidebearings write the x of guides referenced to both sidebearings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BothGuides {
    /// Their x follows the change.
    Follow,
    /// Their x stays as stored.
    Keep,
}

impl BothGuides {
    /// The rule of a document: its both-referenced guides follow when it is of `.glyphs` version 2.
    pub fn of(font: &Font) -> Self {
        if font.source.glyphs_version == Some(2) { BothGuides::Follow } else { BothGuides::Keep }
    }
}

/// Where a guide at `x` referenced to both sidebearings goes when the width changes from `before`
/// to `after`; `None` when it stays.
fn both_x(x: f64, before: f64, after: f64) -> Option<f64> {
    if !(before.is_finite() && after.is_finite()) || before == after {
        return None;
    }
    // Widths are never negative, so a width of zero is the one without a fraction.
    let moved = if before > 0.0 { x * after / before } else { x + (after - before) / 2.0 };
    (moved.is_finite() && moved.abs() <= MAX_COORDINATE).then_some(moved)
}

/// Moves `layer`'s local guides, foreground and background, that are referenced to both
/// sidebearings for a change of its width from `before` to `layer.width`, under
/// [`BothGuides::Follow`]; the stored x of the others already says where they go. `before` is the width the layer would have with its contents
/// moved alone: a command that moves the contents sideways by `dx` (which moves the guides with
/// them) passes the old width plus `dx`. Such a command leaves the stored x of right-referenced
/// guides alone, since the right edge moves with the contents.
pub fn follow_width(layer: &mut Layer, before: f64, both: BothGuides) {
    if both == BothGuides::Keep {
        return;
    }
    let after = layer.width;
    let moved = |g: &Guide| (reference(g) == Reference::Both).then(|| both_x(g.x, before, after)).flatten();
    let any = |list: &[Guide]| list.iter().any(|g| moved(g).is_some());
    if !(any(&layer.info.guides) || layer.info.background.as_ref().is_some_and(|b| any(&b.guides))) {
        return;
    }
    let info = &mut *layer.info;
    let background = info.background.iter_mut().flat_map(|b| b.guides.iter_mut());
    for g in info.guides.iter_mut().chain(background) {
        if let Some(x) = moved(g) {
            g.x = x;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn guides_follow_the_width_by_their_reference() {
        let mut layer = Layer::new("m01", 600.0);
        for (x, to) in [(100.0, Reference::Left), (550.0, Reference::Right), (300.0, Reference::Both)] {
            let mut g = Guide { x, ..Guide::default() };
            set_reference(&mut g, to, 600.0);
            layer.info.guides.push(g);
        }
        assert_eq!(layer.info.guides.iter().map(|g| g.x).collect::<Vec<_>>(), vec![100.0, -50.0, 300.0], "as a file has them");
        layer.width = 700.0;
        follow_width(&mut layer, 600.0, BothGuides::Keep);
        assert_eq!(layer.info.guides[2].x, 300.0);
        follow_width(&mut layer, 600.0, BothGuides::Follow);
        let xs: Vec<f64> = layer.info.guides.iter().map(|g| placed_x(g, layer.width)).collect();
        assert_eq!(xs, vec![100.0, 650.0, 350.0]);
        assert_eq!(layer.info.guides.iter().map(reference).collect::<Vec<_>>(), vec![Reference::Left, Reference::Right, Reference::Both]);
        assert_eq!(both_x(10.0, 0.0, 100.0), Some(60.0));
        assert_eq!(placed_x(&layer.info.guides[1], f64::NAN), -50.0);
    }

    #[test]
    fn a_place_goes_into_the_stored_x_and_comes_back() {
        let mut g = Guide { x: -74.3, orientation: Some("right".into()), ..Guide::default() };
        edit_placed(&mut g, 459.7, |g| {
            g.y = 10.0;
            Ok::<(), ()>(())
        })
        .unwrap();
        assert_eq!((g.x, g.y, g.orientation.as_deref()), (-74.3, 10.0, Some("right")), "a change elsewhere keeps the stored digits");
        place_x(&mut g, 500.3, 600.0);
        assert_eq!((g.x, placed_x(&g, 600.0), placed(&g, 600.0).x), (-99.7, 500.3, 500.3));
        let before = g.clone();
        assert!(
            edit_placed(&mut g, 600.0, |g| {
                g.x = 0.0;
                Err::<(), ()>(())
            })
            .is_err()
        );
        assert_eq!(g, before);
        set_reference(&mut g, Reference::Left, 600.0);
        assert_eq!((g.x, g.orientation), (500.3, None));
    }
}
