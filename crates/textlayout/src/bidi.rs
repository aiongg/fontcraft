//! A simple bidirectional ordering for horizontal text, short of the Unicode bidirectional
//! algorithm (UAX #9). Used when [`crate::Options::bidi`] is set (Text Preview); Edit View lays
//! its text out in one direction.
//!
//! The rules:
//! 1. Each item has a class from the character it was typed as: strong right to left (Hebrew,
//!    Arabic, Syriac, Thaana, NKo and the other right-to-left blocks), digit (European and
//!    Arabic-Indic digits), strong left to right (any other letter), mark (a combining mark,
//!    which takes the class of the item before it) or neutral (spaces, punctuation, symbols, and
//!    items without a character).
//! 2. The paragraph has a base direction; an [`Embedding`] gives a range of items its own base
//!    direction, one level above the paragraph's when the two differ.
//! 3. Digits count as left to right. A neutral between two strong items of the same direction
//!    takes that direction, else the base direction of its embedding. Line breaks and embedding
//!    edges end the search for a strong neighbour.
//! 4. Levels follow from the direction and the embedding (even: left to right, odd: right to
//!    left). On each line, trailing spaces and the line break take the paragraph's level, then
//!    every run at or above each level from the highest down to the lowest odd one is reversed.
//!
//! Digits counting as left to right means "12 34" in right-to-left text reads "12 34", where the
//! Unicode algorithm gives "34 12".

use std::ops::Range;

use crate::text::Item;

/// A range of items, in text order, with its own base direction. Ranges are taken in order of
/// their start (in list order when two start together); a range that overlaps one already taken
/// is ignored, and so is every range past the first [`MAX_EMBEDDINGS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Embedding {
    pub items: Range<usize>,
    pub rtl: bool,
}

/// The ranges with their own direction that [`levels`] takes.
pub const MAX_EMBEDDINGS: usize = 1_000;

/// The bidirectional class of a character, as these rules use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Ltr,
    Rtl,
    Digit,
    Mark,
    Neutral,
}

/// Whether a character belongs to a right-to-left script. Digits in those blocks are not.
pub fn is_strong_rtl(c: char) -> bool {
    let u = u32::from(c);
    is_rtl_block(u) && !is_arabic_digit(u)
}

fn is_rtl_block(u: u32) -> bool {
    matches!(u, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

fn is_arabic_digit(u: u32) -> bool {
    matches!(u, 0x0660..=0x0669 | 0x06F0..=0x06F9)
}

fn is_combining_mark(u: u32) -> bool {
    matches!(u, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

/// The class of a character.
pub fn class(c: char) -> Class {
    let u = u32::from(c);
    if is_arabic_digit(u) {
        Class::Digit
    } else if is_rtl_block(u) {
        Class::Rtl
    } else if is_combining_mark(u) {
        Class::Mark
    } else if c.is_numeric() {
        Class::Digit
    } else if c.is_alphabetic() {
        Class::Ltr
    } else {
        Class::Neutral
    }
}

fn item_class(item: &Item) -> Option<Class> {
    match item {
        Item::Glyph { typed: Some(c), .. } | Item::Missing(c) => Some(class(*c)),
        Item::Glyph { typed: None, .. } | Item::Placeholder => Some(Class::Neutral),
        Item::LineBreak => None,
    }
}

/// The level of every item: even runs left to right, odd right to left; `None` when every item
/// runs in the paragraph's direction. Linear in the items plus the embeddings.
pub(crate) fn levels(items: &[Item], base_rtl: bool, embeddings: &[Embedding]) -> Option<Vec<u8>> {
    let n = items.len();
    let embeddings = embeddings.get(..MAX_EMBEDDINGS).unwrap_or(embeddings);
    // Left-to-right text skips the full class lookup: this scan runs over every item.
    let opposite = |item: &Item| match item {
        Item::Glyph { typed: Some(c), .. } | Item::Missing(c) if base_rtl => matches!(class(*c), Class::Ltr | Class::Digit),
        Item::Glyph { typed: Some(c), .. } | Item::Missing(c) => is_strong_rtl(*c),
        _ => false,
    };
    let embedded = embeddings.iter().any(|e| e.rtl != base_rtl && e.items.start < e.items.end.min(n));
    if !embedded && !items.iter().any(opposite) {
        return None;
    }
    let paragraph = u8::from(base_rtl);
    let mut embedding = vec![paragraph; n];
    let mut sorted: Vec<&Embedding> = embeddings.iter().collect();
    sorted.sort_by_key(|e| e.items.start);
    let mut taken = 0usize;
    for e in sorted {
        let (start, end) = (e.items.start.min(n), e.items.end.min(n));
        if start < taken || start >= end {
            continue;
        }
        let level = if e.rtl == base_rtl { paragraph } else { paragraph + 1 };
        for l in embedding.get_mut(start..end).unwrap_or_default() {
            *l = level;
        }
        taken = end;
    }

    // Strong directions (true = right to left); marks follow the item before them; `None` for
    // neutrals and line breaks.
    let mut strong: Vec<Option<bool>> = Vec::with_capacity(n);
    let mut breaks: Vec<bool> = Vec::with_capacity(n);
    let mut prev: Option<Option<bool>> = None;
    for (i, item) in items.iter().enumerate() {
        let class = item_class(item);
        let same_run = i > 0 && embedding.get(i.wrapping_sub(1)) == embedding.get(i);
        let s = match class {
            None => None,
            Some(Class::Rtl) => Some(true),
            Some(Class::Ltr | Class::Digit) => Some(false),
            Some(Class::Mark) if same_run => prev.flatten(),
            Some(Class::Mark | Class::Neutral) => None,
        };
        strong.push(s);
        breaks.push(class.is_none());
        prev = if class.is_some() { Some(s) } else { None };
    }

    // The nearest strong direction before and after each item, within its embedding and line.
    let boundary = |a: usize, b: usize| breaks.get(a) != Some(&false) || breaks.get(b) != Some(&false) || embedding.get(a) != embedding.get(b);
    let mut before: Vec<Option<bool>> = vec![None; n];
    for i in 1..n {
        if boundary(i - 1, i) {
            continue;
        }
        let v = strong.get(i - 1).copied().flatten().or_else(|| before.get(i - 1).copied().flatten());
        if let Some(b) = before.get_mut(i) {
            *b = v;
        }
    }
    let mut after: Vec<Option<bool>> = vec![None; n];
    for i in (0..n.saturating_sub(1)).rev() {
        if boundary(i, i + 1) {
            continue;
        }
        let v = strong.get(i + 1).copied().flatten().or_else(|| after.get(i + 1).copied().flatten());
        if let Some(a) = after.get_mut(i) {
            *a = v;
        }
    }

    let levels = (0..n)
        .map(|i| {
            let e = embedding.get(i).copied().unwrap_or(paragraph);
            if breaks.get(i) == Some(&true) {
                return paragraph;
            }
            let embedding_rtl = e % 2 == 1;
            let rtl = match strong.get(i).copied().flatten() {
                Some(rtl) => rtl,
                None => match (before.get(i).copied().flatten(), after.get(i).copied().flatten()) {
                    (Some(a), Some(b)) if a == b => a,
                    _ => embedding_rtl,
                },
            };
            if rtl == embedding_rtl { e } else { e + 1 }
        })
        .collect();
    Some(levels)
}

/// Gives the trailing blanks (spaces and the closing line break) of the line `range` the
/// paragraph's level, and returns the index of the first of them (`range.end` when there are
/// none). `levels` and `blank` hold one entry per item of the whole text.
pub(crate) fn reset_trailing(range: Range<usize>, levels: &mut [u8], blank: &[bool], base_rtl: bool) -> usize {
    let mut first = range.end;
    for i in range.rev() {
        let (Some(level), Some(true)) = (levels.get_mut(i), blank.get(i)) else { break };
        *level = u8::from(base_rtl);
        first = i;
    }
    first
}

/// The visual order, left to right, of the items `range` of a line: indexes into the text.
/// `levels` holds one entry per item of the whole text, the line's trailing blanks reset by
/// [`reset_trailing`].
pub(crate) fn visual_order(range: Range<usize>, levels: &[u8], base_rtl: bool) -> Vec<usize> {
    let paragraph = u8::from(base_rtl);
    let mut line: Vec<(usize, u8)> = range.map(|i| (i, levels.get(i).copied().unwrap_or(paragraph))).collect();
    let highest = line.iter().map(|(_, l)| *l).max().unwrap_or(paragraph);
    let lowest_odd = line.iter().map(|(_, l)| *l).filter(|l| l % 2 == 1).min();
    if let Some(lowest_odd) = lowest_odd {
        let mut level = highest;
        while level >= lowest_odd {
            let mut start = 0usize;
            while start < line.len() {
                if line.get(start).is_some_and(|(_, l)| *l >= level) {
                    let mut end = start;
                    while line.get(end).is_some_and(|(_, l)| *l >= level) {
                        end += 1;
                    }
                    if let Some(run) = line.get_mut(start..end) {
                        run.reverse();
                    }
                    start = end;
                } else {
                    start += 1;
                }
            }
            level -= 1;
        }
    }
    line.into_iter().map(|(i, _)| i).collect()
}
