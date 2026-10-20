//! Shaped widths for line breaking (rows `textPreview.lineBreaking`, `textPreview.shaping`):
//! before Text Preview breaks its lines, every item is measured through the full preview font,
//! so kerning, mark positioning and substitutions decide where a line ends.
//!
//! The text is shaped in segments: a run of items of one bidirectional level is cut after each
//! break opportunity ([`crate::linebreak`]) and after [`MAX_SEGMENT_CHARS`] characters (between
//! clusters where it can). Each segment is shaped in its level's direction with the script's
//! default features, and its widths are kept in a [`WidthCache`] by its text, so a word that
//! comes back is shaped once. A cluster's advance goes to its first item; the other items of the
//! cluster are marked as joined to it, and a line never breaks before them.
//!
//! Context across a break opportunity is not seen: a kerning pair or a contextual substitution
//! between a space and the next word does not change the widths lines break on. The line itself
//! is then shaped whole ([`crate::shape_line`]).
//!
//! Items the preview font cannot reach (a line break, a missing character, a glyph added since
//! the font was built) end a segment and keep the layout's own advance. A shaping error leaves
//! its segment with the layout's advances, and the first message is kept.

use std::collections::HashMap;
use std::sync::Mutex;

use fontcraft_model::Font;
use fontcraft_shaping::{Direction as ShapeDirection, ShapeRequest};
use fontcraft_spacing::metrics_source;

use crate::linebreak::Break;
use crate::shape::PreviewShaper;
use crate::text::Item;

/// Characters in one shaped segment. A longer run without a break opportunity is cut there, at
/// a cluster boundary when one comes within as many characters again.
pub const MAX_SEGMENT_CHARS: usize = 128;
/// Segments a [`WidthCache`] keeps; when it is full it starts over.
pub const MAX_CACHED_SEGMENTS: usize = 65_536;

/// What Text Preview measures its items with before it breaks lines.
#[derive(Clone, Copy, Debug)]
pub struct Measure<'a> {
    pub shaper: &'a PreviewShaper,
    pub cache: &'a WidthCache,
}

/// One character of a shaped segment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Width {
    advance: f32,
    /// The character belongs to the cluster of the character before it.
    joined: bool,
}

/// Shaped segment widths by direction and text. It holds for one preview font and one state of
/// the model's advances: the caller starts a new one when either changes.
#[derive(Debug, Default)]
pub struct WidthCache {
    /// Left to right, then right to left.
    maps: Mutex<[HashMap<Box<str>, Box<[Width]>>; 2]>,
}

impl WidthCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The segments it holds.
    pub fn len(&self) -> usize {
        self.maps.lock().map_or(0, |maps| maps.iter().map(HashMap::len).sum())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The measures of a text's items.
pub(crate) struct Measured {
    /// The shaped advance of each item; `None` where the layout keeps its own.
    pub advances: Vec<Option<f32>>,
    /// The item belongs to the cluster of the item before it.
    pub joined: Vec<bool>,
    pub error: Option<String>,
}

/// The segment being collected: consecutive items from `first`, one character each.
struct Segment {
    text: String,
    first: usize,
    count: usize,
    rtl: bool,
}

/// Measures `items` in `master`. `levels` holds the bidirectional level of every item (`None`:
/// all at `paragraph`); `breaks` the opportunity after every item.
pub(crate) fn measure(
    font: &Font,
    master: &str,
    items: &[Item],
    levels: Option<&[u8]>,
    paragraph: u8,
    breaks: &[Break],
    m: &Measure<'_>,
) -> Measured {
    let n = items.len();
    let mut out = Measured { advances: vec![None; n], joined: vec![false; n], error: None };
    let source = metrics_source(font, master).map_or(master, |m| m.id.as_str());
    let live = |gid: u16| -> Option<i32> {
        let glyph = font.glyph(m.shaper.glyph_name(gid)?)?;
        let layer = glyph.layer(source).or_else(|| glyph.layer(master))?;
        layer.width.is_finite().then(|| layer.width.round() as i32)
    };
    // A poisoned lock only costs the cache.
    let mut guard = m.cache.maps.lock().ok();
    let mut maps = guard.as_deref_mut();
    let mut seg = Segment { text: String::new(), first: 0, count: 0, rtl: paragraph % 2 == 1 };
    for i in 0..n {
        let c = match items.get(i) {
            Some(Item::Glyph { name, typed }) => m.shaper.char_of(font, name, *typed),
            _ => None,
        };
        let Some(c) = c else {
            flush(&mut seg, m.shaper, &live, maps.as_deref_mut(), &mut out);
            continue;
        };
        let rtl = levels.and_then(|l| l.get(i)).map_or(paragraph, |l| *l) % 2 == 1;
        let inside_cluster = i.checked_sub(1).and_then(|p| breaks.get(p)) == Some(&Break::Never);
        let full = seg.count >= MAX_SEGMENT_CHARS && (!inside_cluster || seg.count >= 2 * MAX_SEGMENT_CHARS);
        if seg.count > 0 && (rtl != seg.rtl || full) {
            flush(&mut seg, m.shaper, &live, maps.as_deref_mut(), &mut out);
        }
        if seg.count == 0 {
            seg.first = i;
            seg.rtl = rtl;
        }
        seg.text.push(c);
        seg.count += 1;
        if matches!(breaks.get(i), Some(Break::Allowed | Break::Mandatory)) {
            flush(&mut seg, m.shaper, &live, maps.as_deref_mut(), &mut out);
        }
    }
    flush(&mut seg, m.shaper, &live, maps, &mut out);
    out
}

/// Measures the segment, from the cache when it holds it, into `out`, and empties it.
fn flush(
    seg: &mut Segment,
    shaper: &PreviewShaper,
    live: &dyn Fn(u16) -> Option<i32>,
    maps: Option<&mut [HashMap<Box<str>, Box<[Width]>>; 2]>,
    out: &mut Measured,
) {
    if seg.count == 0 {
        return;
    }
    let map = maps.and_then(|maps| maps.get_mut(usize::from(seg.rtl)));
    let shaped;
    let widths: &[Width] = match map {
        Some(map) => {
            if !map.contains_key(seg.text.as_str()) {
                match shape_segment(shaper, live, &seg.text, seg.rtl) {
                    Ok(w) => {
                        if map.len() >= MAX_CACHED_SEGMENTS {
                            map.clear();
                        }
                        map.insert(Box::from(seg.text.as_str()), w);
                    }
                    Err(e) => {
                        out.error.get_or_insert(e);
                    }
                }
            }
            map.get(seg.text.as_str()).map_or(&[], |w| &**w)
        }
        None => match shape_segment(shaper, live, &seg.text, seg.rtl) {
            Ok(w) => {
                shaped = w;
                &shaped
            }
            Err(e) => {
                out.error.get_or_insert(e);
                &[]
            }
        },
    };
    for (k, w) in widths.iter().enumerate().take(seg.count) {
        if let (Some(a), Some(j)) = (out.advances.get_mut(seg.first + k), out.joined.get_mut(seg.first + k)) {
            *a = Some(w.advance);
            *j = w.joined;
        }
    }
    seg.text.clear();
    seg.count = 0;
}

/// The width of every character of `text` shaped in one direction: each cluster's advance on its
/// first character.
fn shape_segment(shaper: &PreviewShaper, live: &dyn Fn(u16) -> Option<i32>, text: &str, rtl: bool) -> Result<Box<[Width]>, String> {
    let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let mut widths = vec![Width { advance: 0.0, joined: true }; starts.len()];
    let direction = if rtl { ShapeDirection::RightToLeft } else { ShapeDirection::LeftToRight };
    for script in fontcraft_shaping::script::itemize(text) {
        let Some(slice) = text.get(script.range.clone()) else { continue };
        let request = ShapeRequest { text: slice, direction, script: Some(script.script), language: None, features: &[], advances: Some(live) };
        let glyphs = fontcraft_shaping::shape(shaper.shaping_font(), &request).map_err(|e| e.to_string())?;
        for g in glyphs {
            let at = script.range.start + g.cluster as usize;
            let k = starts.partition_point(|b| *b <= at).saturating_sub(1);
            if let Some(w) = widths.get_mut(k) {
                w.advance += g.x_advance as f32;
                w.joined = false;
            }
        }
    }
    if let Some(w) = widths.first_mut() {
        w.joined = false;
    }
    Ok(widths.into_boxed_slice())
}
