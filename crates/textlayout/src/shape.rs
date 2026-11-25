//! Edit View shaping (rows `text.shaping`, `features.previewScope`): the substitutions of the
//! compiled preview font applied to a tab's text before [`crate::layout()`] places it.
//!
//! The text goes to the shaper as characters: a typed character as itself, a glyph typed by name
//! as its code point, or as the private-use code point the preview font gave it when it has none
//! (`fontcraft_compile::preview`), so substitutions reach glyphs typed by name too. Only the
//! features active in the tab apply: every other feature of the font is turned off, the shaper's
//! defaults for the script included. Localized Forms turns `locl` on and sets the OpenType script
//! and language tags. An active positional feature (`init`, `medi`, …) of a joining script is left
//! to the shaper, which applies it where the joining calls for it: asked for over the whole run, it
//! would give every letter one form.
//!
//! Only the glyphs and their clusters are read back. Advances, kerning and cursive attachment
//! come from the live model in [`crate::layout()`], so width, kerning and anchor edits show
//! without compiling the features again; the preview font has no `GPOS`.
//!
//! A run of shapeable items ends at a line break, a missing character and a glyph the preview font
//! does not have (added since the last compile, or not exported); those items keep their own
//! glyph. Each run is split into script runs, and each script run is shaped once. A shaping error
//! leaves that script run unshaped, and the first message is kept on the layout.
//!
//! What a text shaped to depends on its characters, the direction and the features, not on the
//! glyphs' outlines or widths, so a [`PreviewShaper`] keeps its last shapings: laying the same text
//! out again after a node or kerning edit, or for a second view of it, shapes nothing.
//! [`PreviewShaper::shape_ahead`] fills that store on any thread.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use fontcraft_model::Font;
use fontcraft_shaping::{FeatureSetting, MAX_FEATURES, ShapeError, ShapeRequest, ShapingFont, Tag};

use crate::text::{Item, Text};
use fontcraft_spacing::kerning::Direction;

/// The `locl` feature tag, which Localized Forms turns on.
pub const LOCL: Tag = Tag::new(b"locl");

/// The features a joining letter takes one of by its neighbours.
const POSITIONAL: &[&[u8; 4]] = &[b"isol", b"init", b"medi", b"fina", b"med2", b"fin2", b"fin3"];
/// The scripts whose letters join, by Unicode script tag.
const JOINING_SCRIPTS: &[&[u8; 4]] =
    &[b"Arab", b"Syrc", b"Adlm", b"Chrs", b"Rohg", b"Mand", b"Mani", b"Mong", b"Nkoo", b"Ougr", b"Phag", b"Phlp", b"Sogd"];

/// A compiled preview font ready for Edit View: the shaping font, the names of its glyphs and its
/// substitution features. Build it once per preview font and share it.
pub struct PreviewShaper {
    font: ShapingFont,
    names: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, u16>,
    private_use: HashMap<u16, u32>,
    features: Vec<Tag>,
    /// The last shapings, newest last.
    shaped: Mutex<VecDeque<(ShapeKey, Arc<Shaped>)>>,
}

/// Shapings one [`PreviewShaper`] keeps.
const MAX_KEPT: usize = 4;

/// What a shaping depends on: the characters of the items (`None` ends a run), and how they are
/// shaped.
#[derive(PartialEq, Eq)]
struct ShapeKey {
    chars: Vec<Option<char>>,
    direction: Direction,
    on: Vec<bool>,
    language: Option<String>,
}

/// The glyphs of every item and the first shaping error.
type Shaped = (ShapedItems, Option<String>);

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
        Ok(Self { font, names, ids, private_use: reverse, features: tags, shaped: Mutex::default() })
    }

    /// The font's feature tags, each once.
    pub fn features(&self) -> &[Tag] {
        &self.features
    }

    /// Whether a layout of `text` finds its shaping kept.
    pub fn is_shaped(&self, font: &Font, text: &Text, direction: Direction, active: &[Tag], localized: Option<(Tag, Tag)>) -> bool {
        let shaping = Shaping { shaper: self, active, localized };
        self.kept(&shaping.key(font, text, direction)).is_some()
    }

    /// Shapes `text` for the layouts to come, which find it kept.
    pub fn shape_ahead(&self, font: &Font, text: &Text, direction: Direction, active: &[Tag], localized: Option<(Tag, Tag)>) {
        shape_items(font, text, direction, &Shaping { shaper: self, active, localized });
    }

    fn kept(&self, key: &ShapeKey) -> Option<Arc<Shaped>> {
        // A poisoned store is only a cache.
        let mut kept = self.shaped.lock().ok()?;
        let at = kept.iter().position(|(k, _)| k == key)?;
        let found = kept.remove(at)?;
        let shaped = Arc::clone(&found.1);
        kept.push_back(found);
        Some(shaped)
    }

    fn keep(&self, key: ShapeKey, shaped: &Arc<Shaped>) {
        let Ok(mut kept) = self.shaped.lock() else { return };
        while kept.len() >= MAX_KEPT {
            kept.pop_front();
        }
        kept.push_back((key, Arc::clone(shaped)));
    }

    /// The name of a glyph id of the preview font.
    pub fn glyph_name(&self, gid: u16) -> Option<&Arc<str>> {
        self.names.get(usize::from(gid))
    }

    /// The shaping font.
    pub(crate) fn shaping_font(&self) -> &ShapingFont {
        &self.font
    }

    /// Whether the preview font has the glyph.
    pub fn has_glyph(&self, name: &str) -> bool {
        self.ids.contains_key(name)
    }

    /// The character that reaches the glyph `name` through the preview font: its private-use code
    /// point when it has one, else the character it was typed as, else its first code point in
    /// the model. `None` when the preview font lacks the glyph.
    pub(crate) fn char_of(&self, font: &Font, name: &str, typed: Option<char>) -> Option<char> {
        let gid = *self.ids.get(name)?;
        if let Some(cp) = self.private_use.get(&gid) {
            return char::from_u32(*cp);
        }
        typed.or_else(|| font.glyph(name)?.unicodes.iter().find_map(|u| char::from_u32(*u)))
    }
}

/// A tag from text: one to four printable ASCII characters, not starting with a space, padded with
/// spaces.
pub fn tag(text: &str) -> Option<Tag> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..=0x7e).contains(b)) || bytes.first() == Some(&b' ') {
        return None;
    }
    let mut out = [b' '; 4];
    for (slot, b) in out.iter_mut().zip(bytes) {
        *slot = *b;
    }
    Some(Tag::new(&out))
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
    /// Every feature of the font: whether it is active (and `locl` with Localized Forms).
    fn on(&self) -> Vec<bool> {
        self.shaper.features().iter().map(|tag| self.active.contains(tag) || (*tag == LOCL && self.localized.is_some())).collect()
    }

    /// What the shaping of `text` depends on.
    fn key(&self, font: &Font, text: &Text, direction: Direction) -> ShapeKey {
        let current = text.current_glyph();
        let chars = text
            .items()
            .iter()
            .map(|item| match item {
                Item::Glyph { name, typed } => self.shaper.char_of(font, name, *typed),
                Item::Placeholder => current.and_then(|n| self.shaper.char_of(font, n, None)),
                Item::Missing(_) | Item::LineBreak => None,
            })
            .collect();
        ShapeKey { chars, direction, on: self.on(), language: self.language() }
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

/// Per text item, the glyphs it shaped to (see [`shape_items`]).
type ShapedItems = Vec<Option<Vec<Arc<str>>>>;

/// The glyphs each item shows after shaping: `None` keeps the item's own glyph; an empty list
/// means the item's glyph merged into an earlier item's (the components of a ligature after the
/// first); more than one glyph means the item became several. The second value is the first
/// shaping error.
pub(crate) fn shape_items(font: &Font, text: &Text, direction: Direction, shaping: &Shaping<'_>) -> Arc<Shaped> {
    let key = shaping.key(font, text, direction);
    if let Some(kept) = shaping.shaper.kept(&key) {
        return kept;
    }
    let mut out: ShapedItems = vec![None; key.chars.len()];
    let mut error = None;
    let settings: Vec<FeatureSetting> = shaping.shaper.features().iter().zip(&key.on).map(|(tag, on)| FeatureSetting::global(*tag, *on)).collect();
    let request = Request { direction, settings: &settings, language: key.language.as_deref() };
    let mut run = Run::default();
    for (i, c) in key.chars.iter().enumerate() {
        match c {
            Some(c) => run.push(i, *c),
            None => run.flush(shaping.shaper, &request, &mut out, &mut error),
        }
    }
    run.flush(shaping.shaper, &request, &mut out, &mut error);
    let shaped = Arc::new((out, error));
    shaping.shaper.keep(key, &shaped);
    shaped
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
            let joining: Vec<FeatureSetting>;
            let features = if JOINING_SCRIPTS.iter().any(|s| script.script == Tag::new(s)) {
                let left = |f: &FeatureSetting| f.value > 0 && POSITIONAL.iter().any(|t| f.tag == Tag::new(t));
                joining = request.settings.iter().filter(|f| !left(f)).cloned().collect();
                joining.as_slice()
            } else {
                request.settings
            };
            let shape_request = ShapeRequest {
                text: slice,
                direction,
                script: Some(script.script),
                language: request.language,
                features,
                advances: None,
                vertical: None,
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
