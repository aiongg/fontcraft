//! The shaping font and one shaping call (M8.18).
//!
//! [`ShapingFont`] wraps a layout font for harfrust and keeps a small cache of shape plans.
//! [`shape`] shapes one run (one direction, one script): it checks the request, finds or builds
//! the plan, maps characters through the font's `cmap` and then the private-use map, takes
//! advances from the request's live function when it gives one, and turns a stopped or failed
//! run into a [`ShapeError`].

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard};

use harfrust::{FontFuncs, GlyphId, ShapeOptions, ShapePlan, ShaperFont};

use crate::{MAX_FEATURES, MAX_PLANS, MAX_PRIVATE_USE, MAX_TEXT_BYTES, ShapeError, Tag};

/// Most bytes in a request's language tag. BCP 47 tags in use are far shorter.
pub const MAX_LANGUAGE_BYTES: usize = 64;
/// Most variation axes [`ShapingFont::at_location`] takes.
pub const MAX_LOCATION_AXES: usize = 64;
/// Largest advance, in font units, a live advance function may give; larger values are clamped
/// so the shaper's sums stay far from `i32` overflow.
const MAX_LIVE_ADVANCE: i32 = 1 << 20;

/// A layout font ready for shaping. Share it behind an `Arc`; the plan cache lives inside and is
/// safe to use from several threads.
pub struct ShapingFont {
    bytes: Arc<[u8]>,
    font: harfrust::Font,
    /// Code point to glyph id, for glyphs reached through private-use code points; sorted by
    /// code point, one entry per code point.
    private_use: Vec<(u32, u16)>,
    /// Least recently used first; at most [`MAX_PLANS`].
    plans: Mutex<Vec<(PlanKey, Arc<ShapePlan>)>>,
}

impl fmt::Debug for ShapingFont {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShapingFont")
            .field("bytes", &self.bytes.len())
            .field("private_use", &self.private_use.len())
            .field("plans", &self.cached_plans())
            .finish()
    }
}

/// What makes two shape plans interchangeable: harfrust compares the same fields, and a ranged
/// feature only needs its own mask, not its range, in the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlanKey {
    direction: harfrust::Direction,
    script: Option<harfrust::Script>,
    language: Option<harfrust::Language>,
    /// (tag, value, applies to the whole text)
    features: Vec<(Tag, u32, bool)>,
}

impl ShapingFont {
    /// Reads `bytes` as a font. `private_use` maps the private-use code points the preview font
    /// gave its unencoded glyphs to their glyph ids (at most [`MAX_PRIVATE_USE`]). When a code
    /// point is listed twice, the first entry counts.
    pub fn new(bytes: Arc<[u8]>, private_use: &[(u32, u16)]) -> Result<Self, ShapeError> {
        if private_use.len() > MAX_PRIVATE_USE {
            return Err(ShapeError::TooManyPrivateUse { limit: MAX_PRIVATE_USE });
        }
        read_fonts::FontRef::new(&bytes).map_err(|e| ShapeError::Font(e.to_string()))?;
        let shared: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(Arc::clone(&bytes));
        let font = harfrust::Font::new(read_fonts::model::Blob::Shared(shared), 0)
            .ok_or_else(|| ShapeError::Font("the data holds no font in a supported format".into()))?;
        let mut map = private_use.to_vec();
        map.sort_by_key(|(cp, _)| *cp);
        map.dedup_by_key(|(cp, _)| *cp);
        Ok(Self { bytes, font, private_use: map, plans: Mutex::new(Vec::new()) })
    }

    /// The same font at a location of its design space, axis tag to user-space value. Axes the
    /// font does not have are ignored, as are values that are not finite; omitted axes take
    /// their defaults. At most [`MAX_LOCATION_AXES`] axes are read. The new font starts with an
    /// empty plan cache, since feature variations can select other lookups there.
    pub fn at_location(&self, location: &[(Tag, f32)]) -> Self {
        let variations: Vec<read_fonts::model::Variation> = location
            .iter()
            .take(MAX_LOCATION_AXES)
            .filter(|(_, value)| value.is_finite())
            .map(|(tag, value)| read_fonts::model::Variation::new(*tag, *value))
            .collect();
        let font = self.font.instance_builder().variations(variations).build();
        Self { bytes: Arc::clone(&self.bytes), font, private_use: self.private_use.clone(), plans: Mutex::new(Vec::new()) }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The number of glyphs in the font.
    pub fn glyph_count(&self) -> u32 {
        self.font.num_glyphs()
    }

    /// The font's units per em.
    pub fn units_per_em(&self) -> u16 {
        self.font.units_per_em()
    }

    /// The glyph a private-use code point stands for.
    pub fn private_use_glyph(&self, code_point: u32) -> Option<u16> {
        private_use_lookup(&self.private_use, code_point)
    }

    /// How many shape plans the cache holds (at most [`MAX_PLANS`]).
    pub fn cached_plans(&self) -> usize {
        self.lock_plans().len()
    }

    fn lock_plans(&self) -> MutexGuard<'_, Vec<(PlanKey, Arc<ShapePlan>)>> {
        // A panic while the lock was held cannot leave the list half-written: every change is a
        // single push, remove or insert. So a poisoned lock is still a valid cache.
        match self.plans.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// The cached plan for `key`, built on a miss. The most recently used plan moves to the end;
    /// the least recently used goes when the cache is full.
    fn plan(&self, key: PlanKey, features: &[harfrust::Feature]) -> Arc<ShapePlan> {
        {
            let mut plans = self.lock_plans();
            if let Some(index) = plans.iter().position(|(k, _)| *k == key) {
                let entry = plans.remove(index);
                let plan = Arc::clone(&entry.1);
                plans.push(entry);
                return plan;
            }
        }
        // Built outside the lock: building reads the font's layout tables, which can take a
        // while for a large font, and other threads may shape with other plans meanwhile.
        let plan = Arc::new(ShapePlan::new(&self.font, key.direction, key.script, key.language.as_ref(), features));
        let mut plans = self.lock_plans();
        if !plans.iter().any(|(k, _)| *k == key) {
            if plans.len() >= MAX_PLANS {
                plans.remove(0);
            }
            plans.push((key, Arc::clone(&plan)));
        }
        plan
    }
}

fn private_use_lookup(map: &[(u32, u16)], code_point: u32) -> Option<u16> {
    let index = map.binary_search_by_key(&code_point, |(cp, _)| *cp).ok()?;
    map.get(index).map(|(_, g)| *g)
}

/// The direction text runs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    #[default]
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

impl Direction {
    pub fn is_horizontal(self) -> bool {
        matches!(self, Direction::LeftToRight | Direction::RightToLeft)
    }

    fn to_harfrust(self) -> harfrust::Direction {
        match self {
            Direction::LeftToRight => harfrust::Direction::LeftToRight,
            Direction::RightToLeft => harfrust::Direction::RightToLeft,
            Direction::TopToBottom => harfrust::Direction::TopToBottom,
            Direction::BottomToTop => harfrust::Direction::BottomToTop,
        }
    }
}

/// One feature turned on or off, for the whole text or a byte range of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureSetting {
    pub tag: Tag,
    /// 0 turns the feature off, 1 on; higher values pick an alternate.
    pub value: u32,
    /// Byte range of the text, end exclusive; `None` for all of it. A range past the end of the
    /// text is cut at the end.
    pub range: Option<Range<usize>>,
}

impl FeatureSetting {
    /// The feature on (`true`, value 1) or off (`false`, value 0) for the whole text.
    pub fn global(tag: Tag, on: bool) -> Self {
        Self { tag, value: u32::from(on), range: None }
    }

    fn to_harfrust(&self, text_len: usize) -> harfrust::Feature {
        // Built field by field: the range constructor of harfrust 0.14 stores an exclusive end
        // as inclusive, while its mask setup reads `end` as exclusive.
        let (start, end) = match &self.range {
            None => (harfrust::Feature::GLOBAL_START, harfrust::Feature::GLOBAL_END),
            Some(range) => {
                let end = range.end.min(text_len);
                let start = range.start.min(end);
                // Text is at most `MAX_TEXT_BYTES`, so both fit; saturate anyway.
                (u32::try_from(start).unwrap_or(u32::MAX - 1), u32::try_from(end).unwrap_or(u32::MAX - 1))
            }
        };
        harfrust::Feature { tag: self.tag, value: self.value, start, end }
    }
}

/// What to shape and how.
#[derive(Clone, Copy)]
pub struct ShapeRequest<'a> {
    /// One run: one direction, one script. At most [`crate::MAX_TEXT_BYTES`].
    pub text: &'a str,
    pub direction: Direction,
    /// The ISO 15924 script (`Latn`, `Arab`), as [`crate::script::itemize`] gives it; `None`
    /// guesses it from the first character with a script of its own.
    pub script: Option<Tag>,
    /// A BCP 47 language tag (`tr`), which harfrust maps to an OpenType language system;
    /// `None` or empty for the default language system. At most [`MAX_LANGUAGE_BYTES`].
    pub language: Option<&'a str>,
    /// Features beyond the shaper's defaults for the script, or turning defaults off. Edit View
    /// passes its active features and turns every `GPOS` feature off; Text Preview passes none.
    /// At most [`crate::MAX_FEATURES`].
    pub features: &'a [FeatureSetting],
    /// The advance of a glyph id from the live model, in font units, replacing the font's
    /// `hmtx` value; `None` from the function keeps the font's. `None` here reads the font.
    /// Values are clamped to ±2^20.
    pub advances: Option<&'a dyn Fn(u16) -> Option<i32>>,
    /// The vertical metrics of a glyph id from the live model, replacing the font's `vmtx`,
    /// `VORG` and their defaults in vertical runs; `None` from the function keeps the font's.
    /// `None` here reads the font.
    pub vertical: Option<&'a dyn Fn(u16) -> Option<VerticalMetrics>>,
}

/// A glyph's vertical metrics, font units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerticalMetrics {
    /// The vertical advance, positive downwards (the `vmtx` value).
    pub advance: i32,
    /// The height of the vertical origin above the glyph's origin (the `VORG` value). The origin
    /// stands at half the glyph's advance width across.
    pub origin_y: i32,
}

impl<'a> ShapeRequest<'a> {
    /// Left-to-right text with the default features and the font's advances.
    pub fn new(text: &'a str) -> Self {
        Self { text, direction: Direction::LeftToRight, script: None, language: None, features: &[], advances: None, vertical: None }
    }
}

/// One output glyph. Horizontal runs come out in visual order (a right-to-left run reversed),
/// vertical runs top to bottom. Units are font units with y up, as HarfBuzz gives them: a
/// vertical run's `y_advance` is negative, and its pen moves from vertical origin to vertical
/// origin, so the offsets of a glyph in it lead from the pen to the glyph's own origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub glyph: u16,
    /// Byte offset in the request's text of the first character this glyph came from.
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// The character-to-glyph and metrics callbacks: the font's `cmap` first, then the private-use
/// map; the live metrics first, then the font's tables.
struct LiveFuncs<'a> {
    private_use: &'a [(u32, u16)],
    advances: Option<&'a dyn Fn(u16) -> Option<i32>>,
    vertical: Option<&'a dyn Fn(u16) -> Option<VerticalMetrics>>,
}

impl LiveFuncs<'_> {
    fn live_vertical(&self, glyph: GlyphId) -> Option<VerticalMetrics> {
        self.vertical.and_then(|f| u16::try_from(glyph.to_u32()).ok().and_then(f))
    }
}

impl FontFuncs for LiveFuncs<'_> {
    fn nominal_glyph(&self, font: &ShaperFont, c: u32) -> Option<GlyphId> {
        font.default_nominal_glyph(c).or_else(|| private_use_lookup(self.private_use, c).map(|g| GlyphId::new(u32::from(g))))
    }

    fn glyph_h_advance(&self, font: &ShaperFont, glyph: GlyphId) -> i32 {
        let live = self.advances.and_then(|f| u16::try_from(glyph.to_u32()).ok().and_then(f));
        match live {
            Some(advance) => advance.clamp(-MAX_LIVE_ADVANCE, MAX_LIVE_ADVANCE),
            None => font.default_glyph_h_advance(glyph),
        }
    }

    fn glyph_v_advance(&self, font: &ShaperFont, glyph: GlyphId) -> i32 {
        match self.live_vertical(glyph) {
            // Negative: y grows upwards and vertical text runs down.
            Some(m) => m.advance.clamp(-MAX_LIVE_ADVANCE, MAX_LIVE_ADVANCE).saturating_neg(),
            None => font.default_glyph_v_advance(glyph),
        }
    }

    fn glyph_v_origin(&self, font: &ShaperFont, glyph: GlyphId) -> (i32, i32) {
        match self.live_vertical(glyph) {
            Some(m) => (font.glyph_h_advance(glyph) / 2, m.origin_y.clamp(-MAX_LIVE_ADVANCE, MAX_LIVE_ADVANCE)),
            None => font.default_glyph_v_origin(glyph),
        }
    }
}

/// Shapes one run of text. An empty text gives no glyphs. The request is checked first (text
/// length, feature count, language length); harfrust's own `max_len` and `max_ops` stop runaway
/// lookups, and a stop or a failed allocation is [`ShapeError::Failed`].
pub fn shape(font: &ShapingFont, req: &ShapeRequest) -> Result<Vec<ShapedGlyph>, ShapeError> {
    if req.text.len() > MAX_TEXT_BYTES {
        return Err(ShapeError::TextTooLong { limit: MAX_TEXT_BYTES });
    }
    if req.features.len() > MAX_FEATURES {
        return Err(ShapeError::TooManyFeatures { limit: MAX_FEATURES });
    }
    let language = match req.language {
        Some(tag) if tag.len() > MAX_LANGUAGE_BYTES => {
            return Err(ShapeError::Failed(format!("the language tag is longer than {MAX_LANGUAGE_BYTES} bytes")));
        }
        Some(tag) => harfrust::Language::new(tag.as_bytes()),
        None => None,
    };
    if req.text.is_empty() {
        return Ok(Vec::new());
    }
    let direction = req.direction.to_harfrust();

    let mut buffer = harfrust::Buffer::new();
    buffer.push_str(req.text);
    if !buffer.allocation_successful() {
        return Err(ShapeError::Failed("there is not enough memory for the text".into()));
    }
    buffer.set_direction(direction);
    buffer.set_script(req.script.and_then(harfrust::Script::from_iso15924_tag));
    buffer.set_language(language);
    // Fills only what is unset: the script, from the first character that has one.
    buffer.guess_segment_properties();
    if buffer.direction() != direction {
        return Err(ShapeError::Failed("the text's direction could not be set".into()));
    }

    let features: Vec<harfrust::Feature> = req.features.iter().map(|f| f.to_harfrust(req.text.len())).collect();
    let key = PlanKey {
        direction,
        script: buffer.script(),
        language: buffer.language().cloned(),
        features: features
            .iter()
            .map(|f| (f.tag, f.value, f.start == harfrust::Feature::GLOBAL_START && f.end == harfrust::Feature::GLOBAL_END))
            .collect(),
    };
    let plan = font.plan(key, &features);
    if plan.direction() != buffer.direction() {
        return Err(ShapeError::Failed("the shape plan's direction differs from the text's".into()));
    }

    let funcs = LiveFuncs { private_use: &font.private_use, advances: req.advances, vertical: req.vertical };
    let needs_funcs = !font.private_use.is_empty() || req.advances.is_some() || req.vertical.is_some();
    let shaper = ShaperFont::new(&font.font).with_font_funcs(needs_funcs.then_some(&funcs as &dyn FontFuncs));
    harfrust::shape(&shaper, &mut buffer, ShapeOptions::new().plan(Some(&plan)).features(&features))
        .map_err(|e| ShapeError::Failed(e.to_string()))?;
    if !buffer.allocation_successful() {
        return Err(ShapeError::Failed("the shaper stopped: the font's lookups take too many steps for this text, or memory ran out".into()));
    }

    let glyphs = buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .map(|(info, pos)| ShapedGlyph {
            glyph: u16::try_from(info.glyph_id).unwrap_or(0),
            cluster: info.cluster,
            x_advance: pos.x_advance,
            y_advance: pos.y_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        })
        .collect();
    Ok(glyphs)
}
