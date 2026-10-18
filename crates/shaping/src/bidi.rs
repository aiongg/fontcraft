//! The Unicode Bidirectional Algorithm (UAX #9) for Text Preview (M8.20), on unicode-bidi with
//! icu_properties' `Bidi_Class` data.

use std::ops::Range;

use icu_properties::CodePointMapData;
use icu_properties::props::BidiClass;
use unicode_bidi::{BidiInfo, Level};

use crate::{Direction, MAX_TEXT_BYTES};

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

/// The runs of `text` in visual order, covering it without gaps. `base` is the paragraph
/// direction (left to right or right to left; a vertical direction counts as left to right);
/// `None` takes it from the first strong character (rules P2 and P3).
///
/// The text is meant to be one line of one paragraph. A paragraph separator inside it (rule P1)
/// starts a new paragraph: the runs of each paragraph follow those of the one before, each
/// paragraph in its own visual order, and a paragraph without a strong character takes `base`
/// (left to right when `None`).
///
/// Text longer than [`MAX_TEXT_BYTES`] is not analysed: it comes back as one run in the base
/// direction.
pub fn paragraph_runs(text: &str, base: Option<Direction>) -> Vec<BidiRun> {
    if text.is_empty() {
        return Vec::new();
    }
    let base_level = base.map(|d| if d == Direction::RightToLeft { Level::rtl() } else { Level::ltr() });
    if text.len() > MAX_TEXT_BYTES {
        let level = base_level.map_or(0, |l| l.number());
        return vec![BidiRun { range: 0..text.len(), level, direction: direction_of(level) }];
    }
    let data = CodePointMapData::<BidiClass>::new();
    let info = BidiInfo::new_with_data_source(&data, text, base_level);
    let mut runs = Vec::new();
    for paragraph in &info.paragraphs {
        // One line per paragraph: Text Preview breaks lines before calling this.
        let (levels, line_runs) = info.visual_runs(paragraph, paragraph.range.clone());
        for range in line_runs {
            let level = levels.get(range.start).map_or(paragraph.level.number(), |l| l.number());
            runs.push(BidiRun { range, level, direction: direction_of(level) });
        }
    }
    runs
}

fn direction_of(level: u8) -> Direction {
    if level % 2 == 1 { Direction::RightToLeft } else { Direction::LeftToRight }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(runs: &[BidiRun]) -> Vec<(Range<usize>, u8)> {
        runs.iter().map(|r| (r.range.clone(), r.level)).collect()
    }

    #[test]
    fn hebrew_inside_left_to_right_text_is_one_right_to_left_run() {
        // "abc " is bytes 0..4, the three Hebrew letters 4..10, " def" 10..14.
        let text = "abc \u{05D0}\u{05D1}\u{05D2} def";
        let runs = paragraph_runs(text, None);
        assert_eq!(summary(&runs), vec![(0..4, 0), (4..10, 1), (10..14, 0)]);
        assert_eq!(runs.get(1).map(|r| r.direction), Some(Direction::RightToLeft));
    }

    #[test]
    fn latin_inside_a_right_to_left_paragraph_comes_first_in_visual_order() {
        // Hebrew and the space (0..7) at level 1, "abc" (7..10) at level 2; the paragraph is
        // right to left, so the Latin run is drawn leftmost.
        let text = "\u{05D0}\u{05D1}\u{05D2} abc";
        let expected = vec![(7..10, 2), (0..7, 1)];
        assert_eq!(summary(&paragraph_runs(text, Some(Direction::RightToLeft))), expected);
        // P2 and P3: the first strong character is Hebrew, so the paragraph is right to left.
        assert_eq!(summary(&paragraph_runs(text, None)), expected);
    }

    #[test]
    fn european_digits_in_arabic_text_stay_left_to_right() {
        // Arabic letter (0..2), space, "12" (3..5), space, Arabic letter (6..8): the digits are
        // at level 2 inside the right-to-left paragraph.
        let text = "\u{0628} 12 \u{0628}";
        let runs = paragraph_runs(text, None);
        assert!(runs.iter().any(|r| r.range == (3..5) && r.level == 2), "{runs:?}");
        assert_eq!(runs.first().map(|r| r.range.clone()), Some(5..8));
    }
}
