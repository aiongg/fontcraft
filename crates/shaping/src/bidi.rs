//! The Unicode Bidirectional Algorithm (UAX #9) for Text Preview (M8.20, on unicode-bidi with
//! icu_properties' data; filled by M8.18).

use std::ops::Range;

use crate::Direction;

/// One run of one embedding level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidiRun {
    /// Byte range of the paragraph's text.
    pub range: Range<usize>,
    /// The UAX #9 embedding level: even left to right, odd right to left.
    pub level: u8,
    /// [`Direction::LeftToRight`] or [`Direction::RightToLeft`], from the level.
    pub direction: Direction,
}

/// The runs of one paragraph in visual order. `base` is the paragraph direction (left to right
/// or right to left); `None` takes it from the first strong character (rules P2 and P3). The
/// text is one paragraph; a paragraph separator inside it ends the runs there.
pub fn paragraph_runs(text: &str, base: Option<Direction>) -> Vec<BidiRun> {
    // M8.18: unicode-bidi's `BidiInfo` with icu_properties' `BidiClass` data, reordered.
    if text.is_empty() {
        return Vec::new();
    }
    let direction = match base {
        Some(Direction::RightToLeft) => Direction::RightToLeft,
        _ => Direction::LeftToRight,
    };
    vec![BidiRun { range: 0..text.len(), level: u8::from(direction == Direction::RightToLeft), direction }]
}
