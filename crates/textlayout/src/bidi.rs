//! Bidirectional ordering for horizontal text by the Unicode Bidirectional Algorithm (UAX #9),
//! through `fontcraft_shaping::bidi`. Used when [`crate::Options::bidi`] is set (Text Preview);
//! Edit View lays its text out in one direction. Rows `textPreview.unicodeBidi`,
//! `textPreview.setWritingDirection`.
//!
//! - Each line break ends a paragraph, and every paragraph takes the base direction of the text
//!   (rules P1, P2 and P3 are not used to pick it): Text Preview has one alignment for all of
//!   its lines.
//! - An [`Embedding`] is the range between a left-to-right or right-to-left embedding character
//!   and a pop character (LRE or RLE, then PDF): its levels start one or two above the
//!   paragraph's, as rule X2 or X3 gives them. An embedding is closed at the end of each
//!   paragraph and opened again in the next.
//! - Items are analysed as the characters they were typed as; a glyph without one and a
//!   placeholder count as an object replacement character (a neutral).
//! - [`levels`] gives the levels after rules W1 to I2 and L1 for the paragraph's end; the
//!   layout resets each line's trailing blanks ([`reset_trailing`], rule L1) and reverses its
//!   runs ([`visual_order`], rule L2).
//!
//! "12 34" in right-to-left text reads "34 12": each number is a left-to-right run, and the runs
//! follow the paragraph's direction.

use std::ops::Range;

use fontcraft_shaping::Direction as ShapeDirection;
use fontcraft_shaping::bidi::paragraph_runs;

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

const LRE: char = '\u{202A}';
const RLE: char = '\u{202B}';
const PDF: char = '\u{202C}';
/// What a glyph without a character and a placeholder are analysed as.
const OBJECT: char = '\u{FFFC}';

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

/// The explicit directional formatting characters and the right-to-left mark.
fn is_bidi_control(u: u32) -> bool {
    matches!(u, 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

/// Whether a character can take a level other than a left-to-right paragraph's: right-to-left
/// letters, Arabic digits and the explicit controls. Left-to-right text without any is skipped.
fn raises_ltr(c: char) -> bool {
    let u = u32::from(c);
    is_rtl_block(u) || is_bidi_control(u)
}

fn char_of(item: &Item) -> char {
    match item {
        Item::Glyph { typed: Some(c), .. } | Item::Missing(c) => *c,
        Item::Glyph { typed: None, .. } | Item::Placeholder => OBJECT,
        Item::LineBreak => '\n',
    }
}

/// The embeddings that count: sorted by start, overlapping ones and those past
/// [`MAX_EMBEDDINGS`] dropped, cut at `n` items, empty ones dropped.
fn taken(embeddings: &[Embedding], n: usize) -> Vec<Embedding> {
    let embeddings = embeddings.get(..MAX_EMBEDDINGS).unwrap_or(embeddings);
    let mut sorted: Vec<&Embedding> = embeddings.iter().collect();
    sorted.sort_by_key(|e| e.items.start);
    let mut out: Vec<Embedding> = Vec::with_capacity(sorted.len());
    let mut end = 0usize;
    for e in sorted {
        let (start, stop) = (e.items.start.min(n), e.items.end.min(n));
        if start < end || start >= stop {
            continue;
        }
        out.push(Embedding { items: start..stop, rtl: e.rtl });
        end = stop;
    }
    out
}

/// The level of every item: even runs left to right, odd right to left; `None` when every item
/// runs in the paragraph's direction. Linear in the items: each paragraph (text between line
/// breaks) is analysed on its own.
pub(crate) fn levels(items: &[Item], base_rtl: bool, embeddings: &[Embedding]) -> Option<Vec<u8>> {
    let n = items.len();
    let embeddings = taken(embeddings, n);
    // Left-to-right text with nothing right to left in it skips the analysis.
    if !base_rtl && embeddings.is_empty() && !items.iter().any(|item| raises_ltr(char_of(item))) {
        return None;
    }
    let paragraph = u8::from(base_rtl);
    let base = Some(if base_rtl { ShapeDirection::RightToLeft } else { ShapeDirection::LeftToRight });
    let mut levels = vec![paragraph; n];
    let mut text = String::new();
    // Byte offset in `text` of each item of the paragraph.
    let mut offsets: Vec<usize> = Vec::new();
    let mut next_embedding = 0usize;
    let mut start = 0usize;
    while start < n {
        let end = items.get(start..).and_then(|rest| rest.iter().position(|i| matches!(i, Item::LineBreak))).map_or(n, |p| start + p + 1);
        text.clear();
        offsets.clear();
        // The embeddings that reach into this paragraph; the first may have begun in an earlier one.
        while embeddings.get(next_embedding).is_some_and(|e| e.items.end <= start) {
            next_embedding += 1;
        }
        let mut open: Option<usize> = None;
        let mut e = next_embedding;
        for i in start..end {
            if open.is_none()
                && let Some(emb) = embeddings.get(e)
                && emb.items.start <= i
                && i < emb.items.end
            {
                text.push(if emb.rtl { RLE } else { LRE });
                open = Some(emb.items.end);
            }
            let c = items.get(i).map_or(OBJECT, char_of);
            // The paragraph separator stands outside the embedding, which it would close anyway.
            if c == '\n' && open.take().is_some() {
                text.push(PDF);
                if embeddings.get(e).is_some_and(|emb| emb.items.end == i + 1) {
                    e += 1;
                }
            }
            offsets.push(text.len());
            text.push(c);
            if open == Some(i + 1) {
                text.push(PDF);
                open = None;
                e += 1;
            }
        }
        let mut runs = paragraph_runs(&text, base);
        runs.sort_by_key(|r| r.range.start);
        let mut r = 0usize;
        for (k, &at) in offsets.iter().enumerate() {
            while runs.get(r).is_some_and(|run| run.range.end <= at) {
                r += 1;
            }
            let level = runs.get(r).filter(|run| run.range.start <= at).map_or(paragraph, |run| run.level);
            if let Some(l) = levels.get_mut(start + k) {
                *l = level;
            }
        }
        start = end;
    }
    levels.iter().any(|l| *l != paragraph).then_some(levels)
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
