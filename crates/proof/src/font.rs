//! The compiled font as the proofs see it: glyph names, code points, metrics, axes, layout
//! features and outlines, read through skrifa.

use std::collections::BTreeSet;

use fontcraft_tables::{TableOrder, TablesError, WriteOptions};
use skrifa::instance::{Location, LocationRef, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::string::StringId;
use skrifa::{FontRef, GlyphId, MetadataProvider, OutlineGlyphCollection, Tag};

use crate::ProofError;

/// Most outline segments one glyph may draw.
pub(crate) const MAX_GLYPH_ELEMENTS: usize = 100_000;
/// Most outline segments one proof may draw, over all its glyphs.
pub(crate) const MAX_TOTAL_ELEMENTS: usize = 4_000_000;
/// Most `cmap` mappings read; more than every Unicode code point twice means a damaged table.
const MAX_MAPPINGS: usize = 2 * 0x11_0000;
/// Most layout features listed.
const MAX_FEATURES: usize = 1_024;

/// What the font file is, for embedding it in a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    TrueType,
    Cff,
    Woff2,
}

impl Container {
    pub(crate) fn mime(self) -> &'static str {
        match self {
            Container::TrueType => "font/ttf",
            Container::Cff => "font/otf",
            Container::Woff2 => "font/woff2",
        }
    }

    pub(crate) fn css_format(self) -> &'static str {
        match self {
            Container::TrueType => "truetype",
            Container::Cff => "opentype",
            Container::Woff2 => "woff2",
        }
    }
}

/// The font's bytes twice: as a plain sfnt for skrifa, and as the file a page embeds (the input
/// itself when it is WOFF2, the sfnt otherwise).
pub(crate) struct FontBytes {
    pub sfnt: Vec<u8>,
    pub embed: Vec<u8>,
    pub container: Container,
}

/// Unwraps face `face` of `bytes` (WOFF, WOFF2 and collections included).
pub(crate) fn load(bytes: &[u8], face: u32) -> Result<FontBytes, ProofError> {
    let font = fontcraft_tables::read(bytes, face)?;
    let cff = font.is_cff();
    let sfnt = fontcraft_tables::write(&font, &WriteOptions { order: TableOrder::Keep })?;
    if bytes.starts_with(b"wOF2") {
        return Ok(FontBytes { sfnt, embed: bytes.to_vec(), container: Container::Woff2 });
    }
    let container = if cff { Container::Cff } else { Container::TrueType };
    Ok(FontBytes { embed: sfnt.clone(), sfnt, container })
}

pub(crate) fn font_ref(sfnt: &[u8]) -> Result<FontRef<'_>, ProofError> {
    FontRef::new(sfnt).map_err(|e| ProofError::Tables(TablesError::Invalid(format!("the font cannot be read: {e}"))))
}

/// The location of `settings` (axis tag, user value), checked against the font's axes.
pub(crate) fn location(font: &FontRef<'_>, settings: &[([u8; 4], f64)]) -> Result<Location, ProofError> {
    if settings.len() > fontcraft_model::MAX_AXIS_VALUES {
        return Err(ProofError::BadOptions(format!("a location has at most {} axes", fontcraft_model::MAX_AXIS_VALUES)));
    }
    let axes = font.axes();
    for (tag, value) in settings {
        let tag = Tag::new(tag);
        if !value.is_finite() {
            return Err(ProofError::BadOptions(format!("the location of axis `{tag}` is not a finite number")));
        }
        if axes.get_by_tag(tag).is_none() {
            return Err(ProofError::BadOptions(format!("the font has no axis `{tag}`")));
        }
    }
    Ok(axes.location(settings.iter().map(|(tag, value)| (Tag::new(tag), *value as f32))))
}

/// One variation axis, in user coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AxisInfo {
    pub tag: String,
    pub min: f64,
    pub default: f64,
    pub max: f64,
}

/// One outline segment, in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum El {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A glyph's drawing at the proof's location.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Outline {
    pub els: Vec<El>,
    pub advance: f64,
    /// `[x_min, y_min, x_max, y_max]` of every point, control points included; `None` when empty.
    pub bounds: Option<[f64; 4]>,
}

/// The font at one location.
pub(crate) struct Face<'a> {
    pub font: FontRef<'a>,
    location: LocationRef<'a>,
    outlines: OutlineGlyphCollection<'a>,
    glyph_metrics: GlyphMetrics<'a>,
    pub upem: f64,
    pub ascender: f64,
    pub descender: f64,
    pub x_height: Option<f64>,
    pub cap_height: Option<f64>,
    /// Glyph names in glyph order.
    pub names: Vec<String>,
    /// The lowest code point mapped to each glyph.
    pub unicodes: Vec<Option<u32>>,
    /// Every mapped code point with its glyph id, by code point.
    pub mappings: Vec<(u32, u32)>,
    pub family: String,
    pub style: String,
}

impl<'a> Face<'a> {
    pub(crate) fn new(font: FontRef<'a>, location: &'a Location) -> Result<Self, ProofError> {
        let location = LocationRef::from(location);
        let metrics = font.metrics(Size::unscaled(), location);
        let upem = f64::from(metrics.units_per_em);
        if upem <= 0.0 {
            return Err(ProofError::Tables(TablesError::Invalid("the font's units per em is 0".into())));
        }
        let finite = |v: f32| Some(f64::from(v)).filter(|v| v.is_finite());
        let ascender = finite(metrics.ascent).filter(|a| *a > 0.0).unwrap_or(upem * 0.8);
        let descender = finite(metrics.descent).map(|d| -d.abs()).filter(|d| *d < 0.0).unwrap_or(-upem * 0.2);
        let count = font.maxp().map(|m| u32::from(m.num_glyphs())).unwrap_or(0);
        let glyph_names = font.glyph_names();
        let names: Vec<String> =
            (0..count).map(|gid| glyph_names.get(GlyphId::new(gid)).map(|n| n.as_str().to_string()).unwrap_or_else(|| format!("gid{gid}"))).collect();
        let mut unicodes: Vec<Option<u32>> = vec![None; names.len()];
        let mut mappings: Vec<(u32, u32)> = Vec::new();
        for (cp, gid) in font.charmap().mappings().take(MAX_MAPPINGS) {
            let gid = gid.to_u32();
            let Some(slot) = unicodes.get_mut(gid as usize) else { continue };
            if slot.is_none_or(|old| cp < old) {
                *slot = Some(cp);
            }
            mappings.push((cp, gid));
        }
        mappings.sort_unstable();
        mappings.dedup_by_key(|m| m.0);
        let name = |id: StringId| font.localized_strings(id).english_or_first().map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
        let family = name(StringId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| name(StringId::FAMILY_NAME)).unwrap_or_else(|| "Untitled".into());
        let style = name(StringId::TYPOGRAPHIC_SUBFAMILY_NAME).or_else(|| name(StringId::SUBFAMILY_NAME)).unwrap_or_else(|| "Regular".into());
        Ok(Face {
            outlines: font.outline_glyphs(),
            glyph_metrics: font.glyph_metrics(Size::unscaled(), location),
            font,
            location,
            upem,
            ascender,
            descender,
            x_height: metrics.x_height.and_then(finite),
            cap_height: metrics.cap_height.and_then(finite),
            names,
            unicodes,
            mappings,
            family,
            style,
        })
    }

    /// The glyph ids of `names`, in the order given; every name must exist.
    pub(crate) fn resolve(&self, names: &[String]) -> Result<Vec<u32>, ProofError> {
        if names.is_empty() {
            return Ok((0..self.names.len() as u32).collect());
        }
        if names.len() > crate::MAX_SHEET_GLYPHS {
            return Err(ProofError::BadOptions(format!("a proof shows at most {} glyphs", crate::MAX_SHEET_GLYPHS)));
        }
        let index = crate::select::name_index(&self.names);
        names.iter().map(|n| index.get(n.as_str()).copied().ok_or_else(|| ProofError::BadOptions(format!("the font has no glyph `{n}`")))).collect()
    }

    pub(crate) fn name(&self, gid: u32) -> &str {
        self.names.get(gid as usize).map(String::as_str).unwrap_or("")
    }

    pub(crate) fn unicode(&self, gid: u32) -> Option<u32> {
        self.unicodes.get(gid as usize).copied().flatten()
    }

    /// The glyph `cmap` maps `c` to.
    pub(crate) fn glyph_for(&self, c: char) -> Option<u32> {
        self.mappings.binary_search_by_key(&(c as u32), |m| m.0).ok().and_then(|i| self.mappings.get(i)).map(|m| m.1)
    }

    /// The advance of the glyph mapped to `c`, in font units; half an em when nothing is mapped.
    pub(crate) fn char_advance(&self, c: char) -> f64 {
        self.glyph_for(c)
            .and_then(|g| self.glyph_metrics.advance_width(GlyphId::new(g)))
            .map(f64::from)
            .filter(|a| a.is_finite() && *a >= 0.0)
            .unwrap_or(self.upem / 2.0)
    }

    /// The glyph's outline and advance. `budget` is the number of segments the proof may still
    /// draw; it shrinks by what this glyph uses.
    pub(crate) fn outline(&self, gid: u32, budget: &mut usize) -> Result<Outline, ProofError> {
        let id = GlyphId::new(gid);
        let advance = self.glyph_metrics.advance_width(id).map(f64::from).filter(|a| a.is_finite()).unwrap_or(0.0);
        let mut pen = Pen { els: Vec::new(), limit: MAX_GLYPH_ELEMENTS.min(*budget), overflow: false, bad: false };
        if let Some(glyph) = self.outlines.get(id) {
            // A glyph skrifa cannot draw is shown empty, like a glyph without contours.
            if glyph.draw(DrawSettings::unhinted(Size::unscaled(), self.location), &mut pen).is_err() {
                pen.els.clear();
            }
        }
        if pen.overflow {
            return Err(ProofError::BadOptions(format!(
                "the glyphs to show draw more than {MAX_GLYPH_ELEMENTS} outline segments in one glyph or {MAX_TOTAL_ELEMENTS} in all: select fewer glyphs"
            )));
        }
        if pen.bad {
            pen.els.clear();
        }
        *budget = budget.saturating_sub(pen.els.len());
        let bounds = bounds(&pen.els);
        Ok(Outline { els: pen.els, advance, bounds })
    }

    /// The variation axes, in user coordinates.
    pub(crate) fn axes(&self) -> Vec<AxisInfo> {
        self.font
            .axes()
            .iter()
            .take(fontcraft_model::MAX_AXIS_VALUES)
            .filter(|a| !a.is_hidden())
            .map(|a| AxisInfo {
                tag: a.tag().to_string(),
                min: f64::from(a.min_value()),
                default: f64::from(a.default_value()),
                max: f64::from(a.max_value()),
            })
            .filter(|a| a.min.is_finite() && a.default.is_finite() && a.max.is_finite() && a.min <= a.max)
            .collect()
    }

    /// The feature tags of `GSUB` and `GPOS`, sorted, each once.
    pub(crate) fn features(&self) -> Vec<String> {
        let mut tags: BTreeSet<String> = BTreeSet::new();
        let mut add = |list: Option<&[skrifa::raw::tables::layout::FeatureRecord]>| {
            for r in list.unwrap_or_default() {
                if tags.len() < MAX_FEATURES {
                    tags.insert(r.feature_tag().to_string());
                }
            }
        };
        if let Ok(gsub) = self.font.gsub() {
            let list = gsub.feature_list().ok();
            add(list.as_ref().map(|l| l.feature_records()));
        }
        if let Ok(gpos) = self.font.gpos() {
            let list = gpos.feature_list().ok();
            add(list.as_ref().map(|l| l.feature_records()));
        }
        tags.into_iter().collect()
    }
}

fn bounds(els: &[El]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |x: f32, y: f32| {
        let (x, y) = (f64::from(x), f64::from(y));
        b = Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    };
    for el in els {
        match *el {
            El::Move(x, y) | El::Line(x, y) => add(x, y),
            El::Quad(cx, cy, x, y) => {
                add(cx, cy);
                add(x, y);
            }
            El::Cubic(c0x, c0y, c1x, c1y, x, y) => {
                add(c0x, c0y);
                add(c1x, c1y);
                add(x, y);
            }
            El::Close => {}
        }
    }
    b
}

/// Collects segments, stopping at `limit`; a coordinate that is not finite marks the glyph bad.
struct Pen {
    els: Vec<El>,
    limit: usize,
    overflow: bool,
    bad: bool,
}

impl Pen {
    fn push(&mut self, el: El, coords: &[f32]) {
        if self.overflow || self.bad {
            return;
        }
        if !coords.iter().all(|c| c.is_finite()) {
            self.bad = true;
            return;
        }
        if self.els.len() >= self.limit {
            self.overflow = true;
            return;
        }
        self.els.push(el);
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.push(El::Move(x, y), &[x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(El::Line(x, y), &[x, y]);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.push(El::Quad(cx0, cy0, x, y), &[cx0, cy0, x, y]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.push(El::Cubic(cx0, cy0, cx1, cy1, x, y), &[cx0, cy0, cx1, cy1, x, y]);
    }
    fn close(&mut self) {
        self.push(El::Close, &[]);
    }
}
