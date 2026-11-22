//! What a guide's x position follows when its layer's width changes (`guides.sidebearingReference`).
//!
//! A `.glyphs` guide may carry an `alignment` key: absent for the left sidebearing, `right`, or
//! `center` for both. A local guide referenced to the right moves by the width change; one
//! referenced to both keeps its fraction of the width; one referenced to the left stays. Every
//! command that writes a layer's advance width calls [`follow_width`] in the same edit, so the
//! guides move in the same undo step.

use crate::plist::Value;
use crate::{Family, Guide, Layer, MAX_COORDINATE};

/// The key, in a guide's `.glyphs` data, of its sidebearing reference.
pub const REFERENCE_KEY: &str = "alignment";

/// What a guide's x position follows when the width changes.
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

/// A guide's sidebearing reference; a value the key does not name reads as the left one.
pub fn reference(g: &Guide) -> Reference {
    match g.extra.get(Family::Glyphs, REFERENCE_KEY).and_then(Value::as_str) {
        Some("right") => Reference::Right,
        Some("center") => Reference::Both,
        _ => Reference::Left,
    }
}

/// Sets a guide's sidebearing reference.
pub fn set_reference(g: &mut Guide, to: Reference) {
    match to {
        Reference::Right => g.extra.insert(Family::Glyphs, REFERENCE_KEY, Value::String("right".into())),
        Reference::Both => g.extra.insert(Family::Glyphs, REFERENCE_KEY, Value::String("center".into())),
        Reference::Left => {
            g.extra.remove(Family::Glyphs, REFERENCE_KEY);
        }
    }
}

/// Where a guide at `x` referenced to `to` goes when the width changes from `before` to `after`;
/// `None` when it stays.
pub fn moved_x(x: f64, to: Reference, before: f64, after: f64) -> Option<f64> {
    if !(before.is_finite() && after.is_finite()) || before == after {
        return None;
    }
    let moved = match to {
        Reference::Left => return None,
        Reference::Right => x + (after - before),
        // Widths are never negative, so a width of zero is the one without a fraction.
        Reference::Both if before > 0.0 => x * after / before,
        Reference::Both => x + (after - before) / 2.0,
    };
    (moved.is_finite() && moved.abs() <= MAX_COORDINATE).then_some(moved)
}

/// Moves `layer`'s local guides, foreground and background, for a change of its width from
/// `before` to `layer.width`. `before` is the width the layer would have with its contents moved
/// alone: a command that moves the contents sideways by `dx` (which moves the guides with them)
/// passes the old width plus `dx`.
pub fn follow_width(layer: &mut Layer, before: f64) {
    let after = layer.width;
    let moves = |g: &Guide| moved_x(g.x, reference(g), before, after).is_some();
    let any = layer.info.guides.iter().any(moves) || layer.info.background.as_ref().is_some_and(|b| b.guides.iter().any(moves));
    if !any {
        return;
    }
    let info = &mut *layer.info;
    let background = info.background.iter_mut().flat_map(|b| b.guides.iter_mut());
    for g in info.guides.iter_mut().chain(background) {
        if let Some(x) = moved_x(g.x, reference(g), before, after) {
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
            set_reference(&mut g, to);
            layer.info.guides.push(g);
        }
        layer.width = 700.0;
        follow_width(&mut layer, 600.0);
        let xs: Vec<f64> = layer.info.guides.iter().map(|g| g.x).collect();
        assert_eq!(xs, vec![100.0, 650.0, 350.0]);
        assert_eq!(layer.info.guides.iter().map(reference).collect::<Vec<_>>(), vec![Reference::Left, Reference::Right, Reference::Both]);
        assert_eq!(moved_x(10.0, Reference::Both, 0.0, 100.0), Some(60.0));
        assert_eq!(moved_x(10.0, Reference::Right, f64::NAN, 100.0), None);
    }
}
