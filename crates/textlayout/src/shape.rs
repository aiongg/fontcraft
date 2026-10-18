//! Edit View shaping (rows `text.shaping`, `features.previewScope`): the substitutions of the
//! compiled preview font applied to a tab's text before [`crate::layout()`] places it.
//!
//! The text goes to the shaper as characters: a typed character as itself, a glyph typed by name
//! as its code point, or as the private-use code point the preview font gave it when it has none
//! (`fontcraft_compile::preview`), so substitutions reach glyphs typed by name too. Only the
//! features active in the tab apply: every other feature of the font is turned off, the shaper's
//! defaults for the script included. Localized Forms turns `locl` on and sets the OpenType script
//! and language tags.
//!
//! Only the glyphs and their clusters are read back. Advances, kerning and cursive attachment
//! come from the live model in [`crate::layout()`], so width, kerning and anchor edits show
//! without compiling the features again; the preview font has no `GPOS`.
//!
//! A run of shapeable items ends at a line break, a missing character and a glyph the preview font
//! does not have (added since the last compile, or not exported); those items keep their own
//! glyph. Each run is split into script runs, and each script run is shaped once. A shaping error
//! leaves that script run unshaped, and the first message is kept on the layout.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_shaping::{FeatureSetting, MAX_FEATURES, ShapeError, ShapeRequest, ShapingFont, Tag};

use crate::text::{Item, Text};
use fontcraft_spacing::kerning::Direction;

/// The `locl` feature tag, which Localized Forms turns on.
pub const LOCL: Tag = Tag::new(b"locl");

/// A compiled preview font ready for Edit View: the shaping font, the names of its glyphs and its
/// substitution features. Build it once per preview font and share it.
pub struct PreviewShaper {
    font: ShapingFont,
    names: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, u16>,
    private_use: HashMap<u16, u32>,
    features: Vec<Tag>,
}

impl std::fmt::Debug for PreviewShaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewShaper").field("glyphs", &self.names.len()).field("features", &self.features).finish()
    }
}

impl PreviewShaper {
    /// `glyph_order`: the font's glyph names in glyph-id order (at most 65,536 are read);
    /// `private_use`: (code point, glyph id) for the glyphs without a code point; `features`: the
    /// font's feature tags (each kept once, at most [`MAX_FEATURES`]).
    pub fn new(bytes: Arc<[u8]>, glyph_order: &[String], private_use: &[(u32, u16)], features: &[Tag]) -> Result<Self, ShapeError> {
        let font = ShapingFont::new(bytes, private_use)?;
        let names: Vec<Arc<str>> = glyph_order.iter().take(usize::from(u16::MAX) + 1).map(|n| Arc::from(n.as_str())).collect();
        let mut ids = HashMap::with_capacity(names.len());
        for (gid, name) in (0u16..).zip(&names) {
            ids.entry(name.clone()).or_insert(gid);
        }
        let mut reverse = HashMap::with_capacity(private_use.len());
        for &(cp, gid) in private_use.iter().take(fontcraft_shaping::MAX_PRIVATE_USE) {
            reverse.entry(gid).or_insert(cp);
        }
        let mut tags: Vec<Tag> = Vec::new();
        for tag in features {
            if tags.len() < MAX_FEATURES && !tags.contains(tag) {
                tags.push(*tag);
            }
        }
        Ok(Self { font, names, ids, private_use: reverse, features: tags })
    }

    /// The font's feature tags, each once.
    pub fn features(&self) -> &[Tag] {
        &self.features
    }

    /// The name of a glyph id of the preview font.
    pub fn glyph_name(&self, gid: u16) -> Option<&Arc<str>> {
        self.names.get(usize::from(gid))
    }

    /// Whether the preview font has the glyph.
    pub fn has_glyph(&self, name: &str) -> bool {
        self.ids.contains_key(name)
    }

    /// The character that reaches the glyph `name` through the preview font: its private-use code
    /// point when it has one, else the character it was typed as, else its first code point in
    /// the model. `None` when the preview font lacks the glyph.
    fn char_of(&self, font: &Font, name: &str, typed: Option<char>) -> Option<char> {
        let gid = *self.ids.get(name)?;
        if let Some(cp) = self.private_use.get(&gid) {
            return char::from_u32(*cp);
        }
        typed.or_else(|| font.glyph(name)?.unicodes.iter().find_map(|u| char::from_u32(*u)))
    }
}

/// What Edit View shapes a tab's text with.
#[derive(Clone, Copy, Debug)]
pub struct Shaping<'a> {
    pub shaper: &'a PreviewShaper,
    /// The features active in the tab (the Features menu).
    pub active: &'a [Tag],
    /// Localized Forms: the OpenType script and language tags it evaluates `locl` for; `None`
    /// when it is off.
    pub localized: Option<(Tag, Tag)>,
}

impl Shaping<'_> {
    /// Every feature of the font, on when active (and `locl` with Localized Forms), else off.
    fn settings(&self) -> Vec<FeatureSetting> {
        self.shaper
            .features()
            .iter()
            .map(|tag| FeatureSetting::global(*tag, self.active.contains(tag) || (*tag == LOCL && self.localized.is_some())))
            .collect()
    }

    /// The language tag that carries Localized Forms' OpenType script and language tags to the
    /// shaper (private-use subtags `hbsc` and `hbot`).
    fn language(&self) -> Option<String> {
        let (script, language) = self.localized?;
        let text = |tag: Tag| -> String { tag.to_be_bytes().iter().filter(|b| b.is_ascii_alphanumeric()).map(|b| char::from(*b)).collect() };
        let (script, language) = (text(script), text(language));
        let mut out = String::from("x");
        if !script.is_empty() && script != "DFLT" {
            out.push_str("-hbsc");
            out.push_str(&script.to_ascii_lowercase());
        }
        if !language.is_empty() && language != "dflt" {
            out.push_str("-hbot");
            out.push_str(&language);
        }
        (out.len() > 1).then_some(out)
    }
}

/// The glyphs each item shows after shaping: `None` keeps the item's own glyph; an empty list
/// means the item's glyph merged into an earlier item's (the components of a ligature after the
/// first); more than one glyph means the item became several. The second value is the first
/// shaping error.
pub(crate) fn shape_items(font: &Font, text: &Text, direction: Direction, shaping: &Shaping<'_>) -> (Vec<Option<Vec<Arc<str>>>>, Option<String>) {
    let items = text.items();
    let mut out: Vec<Option<Vec<Arc<str>>>> = vec![None; items.len()];
    let mut error = None;
    let settings = shaping.settings();
    let language = shaping.language();
    let request = Request { direction, settings: &settings, language: language.as_deref() };
    let current = text.current_glyph();
    let mut run = Run::default();
    for (i, item) in items.iter().enumerate() {
        let c = match item {
            Item::Glyph { name, typed } => shaping.shaper.char_of(font, name, *typed),
            Item::Placeholder => current.and_then(|n| shaping.shaper.char_of(font, n, None)),
            Item::Missing(_) | Item::LineBreak => None,
        };
        match c {
            Some(c) => run.push(i, c),
            None => run.flush(shaping.shaper, &request, &mut out, &mut error),
        }
    }
    run.flush(shaping.shaper, &request, &mut out, &mut error);
    (out, error)
}

struct Request<'a> {
    direction: Direction,
    settings: &'a [FeatureSetting],
    language: Option<&'a str>,
}

/// Consecutive shapeable items as text: each item is one character.
#[derive(Default)]
struct Run {
    text: String,
    /// (byte offset in `text`, item index), one per character.
    starts: Vec<(usize, usize)>,
}

impl Run {
    fn push(&mut self, item: usize, c: char) {
        self.starts.push((self.text.len(), item));
        self.text.push(c);
    }

    /// The position in `starts` of the character at byte `at` (the last one starting at or
    /// before it); `starts.len()` past the end.
    fn local(&self, at: usize) -> usize {
        if at >= self.text.len() {
            return self.starts.len();
        }
        self.starts.partition_point(|(b, _)| *b <= at).saturating_sub(1)
    }

    fn flush(&mut self, shaper: &PreviewShaper, request: &Request<'_>, out: &mut [Option<Vec<Arc<str>>>], error: &mut Option<String>) {
        if self.text.is_empty() {
            return;
        }
        let direction = match request.direction {
            Direction::Ltr => fontcraft_shaping::Direction::LeftToRight,
            Direction::Rtl => fontcraft_shaping::Direction::RightToLeft,
            Direction::Vertical => fontcraft_shaping::Direction::TopToBottom,
        };
        for script in fontcraft_shaping::script::itemize(&self.text) {
            let Some(slice) = self.text.get(script.range.clone()) else { continue };
            let shape_request = ShapeRequest {
                text: slice,
                direction,
                script: Some(script.script),
                language: request.language,
                features: request.settings,
                advances: None,
            };
            let mut glyphs = match fontcraft_shaping::shape(&shaper.font, &shape_request) {
                Ok(g) => g,
                Err(e) => {
                    error.get_or_insert_with(|| e.to_string());
                    continue;
                }
            };
            // Back to logical order.
            if direction == fontcraft_shaping::Direction::RightToLeft {
                glyphs.reverse();
            }
            let end = self.local(script.range.end);
            let mut k = 0usize;
            while let Some(first) = glyphs.get(k) {
                let cluster = first.cluster;
                let size = glyphs.iter().skip(k).take_while(|g| g.cluster == cluster).count().max(1);
                let next = glyphs.get(k + size).map_or(end, |g| self.local(script.range.start + g.cluster as usize));
                let from = self.local(script.range.start + cluster as usize);
                let names: Vec<Arc<str>> = glyphs.iter().skip(k).take(size).filter_map(|g| shaper.glyph_name(g.glyph).cloned()).collect();
                assign(&self.starts, from..next.max(from), names, out);
                k += size;
            }
        }
        self.text.clear();
        self.starts.clear();
    }
}

/// Gives the glyphs of one cluster to the items it covers: one each in order, the last item
/// taking the rest, and items beyond the glyphs merged (empty).
fn assign(starts: &[(usize, usize)], range: std::ops::Range<usize>, names: Vec<Arc<str>>, out: &mut [Option<Vec<Arc<str>>>]) {
    let count = range.len();
    if count == 0 {
        return;
    }
    let mut names = names.into_iter();
    for (n, local) in range.enumerate() {
        let Some(&(_, item)) = starts.get(local) else { break };
        let glyphs: Vec<Arc<str>> = if n + 1 == count { names.by_ref().collect() } else { names.next().into_iter().collect() };
        if let Some(slot) = out.get_mut(item) {
            *slot = Some(glyphs);
        }
    }
}
