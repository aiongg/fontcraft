//! Default anchors (M6.3): the anchors Set Anchors adds to a layer, from our own table.
//!
//! Rows: the Set Anchors default table (`glyph.setAnchors`), `segment.startEndAnchors` and
//! `brush.startEndAnchors` (the anchor rules only).
//!
//! Contract:
//! - [`default_anchor_names`] lists the anchors a glyph should have, by its name and category:
//!   a custom glyph data file's `anchors` first, then our table.
//! - [`default_anchors`] places them on one layer from its outline bounds and the master's
//!   metrics, and returns only the names the layer does not have yet (Set Anchors adds the
//!   missing ones and keeps the rest). At most [`crate::MAX_LAYER_ANCHORS`] in total.
//!
//! The table (the published glyph database has none; `docs/plans/M6.md` § 3, D6):
//!
//! | Glyph | Anchors |
//! |---|---|
//! | `_segment.*` | `start`, `end` on the first and last node of its open path |
//! | `_brush.*` | `start`, `end` on the baseline at the left and right of the layer |
//! | mark above (combining class 230 and the other classes above) | `_top` at the x-height (cap height for `.case`), `top` on the outline's top |
//! | mark below (classes 202, 218, 220, 222, 233, 240) | `_bottom` on the baseline, `bottom` on the outline's bottom |
//! | ogonek (U+0328) | `_ogonek` on the baseline at the outline's right |
//! | horn (U+031B) | `_topright` at the outline's left, at the x-height |
//! | `caroncomb.alt` | `_topright` at the outline's top left |
//! | overlay (class 1) | `_center` at half the x-height |
//! | ligature (subcategory Ligature, or a name of parts joined by `_`) | `caret_1` … `caret_(n-1)` on the baseline, dividing the width equally |
//! | letter, except Han, Hangul, Kana, Yi and Bopomofo | `top` (x-height, ascender or cap height by case and outline; the outline's top without a case), `bottom` (baseline; the outline's bottom without a case) |
//! | A E I O U a e i idotless o u | also `ogonek` on the baseline at the outline's right |
//! | D H L O T d h l o t | also `center` at half the top's height |
//! | L O U d l o t u | also `topright` at the outline's right, at the top's height |
//!
//! Anchors centred on the outline follow the italic angle about the outline's middle height.

use fontcraft_geom::{BezPath, Rect, Shape as _};
use fontcraft_glyphdata::{Case, Category, Direction, GlyphData, SubCategory};
use fontcraft_model::{Anchor, Font, Glyph, Layer, Shape, metric};

use crate::compose::{case_of, category_of};
use crate::{ComponentsError, MAX_ANCHOR_NAME_LEN, MAX_LAYER_ANCHORS};

/// Scripts whose letters take no marks in our table.
const NO_MARK_SCRIPTS: &[&str] = &["han", "hangul", "kana", "yi", "bopomofo"];
/// Combining classes of marks set above the base.
const ABOVE: &[u8] = &[214, 228, 230, 232, 234];
/// Combining classes of marks set below the base.
const BELOW: &[u8] = &[202, 218, 220, 222, 233, 240];
const OGONEK: u32 = 0x0328;
const HORN: u32 = 0x031B;
/// Steepest italic angle used to slant anchors, in degrees; beyond it the angle is ignored.
const MAX_ITALIC_ANGLE: f64 = 80.0;

/// What a glyph is, for the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Segment,
    Brush,
    MarkAbove,
    MarkBelow,
    MarkOgonek,
    MarkHorn,
    MarkCaronAlt,
    MarkOverlay,
    Ligature(usize),
    Letter,
    Other,
}

/// The anchor names a glyph called `glyph` should have.
pub fn default_anchor_names(font: &Font, data: &GlyphData, glyph: &str) -> Vec<String> {
    if glyph.is_empty() || glyph.len() > fontcraft_model::MAX_NAME_LEN {
        return Vec::new();
    }
    let (base, _) = data.split_suffix(glyph);
    for name in [glyph, base] {
        if let Some(list) = data.record(name).and_then(|r| r.anchors) {
            let names: Vec<String> = list
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty() && n.len() <= MAX_ANCHOR_NAME_LEN)
                .take(MAX_LAYER_ANCHORS)
                .map(str::to_string)
                .collect();
            if !names.is_empty() {
                return dedup(names);
            }
        }
    }
    table_names(role(font, data, glyph), base)
}

/// The default anchors `glyph`'s layer `layer` lacks, placed on that layer.
pub fn default_anchors(font: &Font, data: &GlyphData, glyph: &str, layer: &str) -> Result<Vec<Anchor>, ComponentsError> {
    let g = font.glyph(glyph).ok_or_else(|| ComponentsError::MissingGlyph(glyph.to_string()))?;
    let l = g
        .layers
        .iter()
        .find(|l| l.id == layer)
        .ok_or_else(|| ComponentsError::MissingLayer { glyph: glyph.to_string(), layer: layer.to_string() })?;
    let room = MAX_LAYER_ANCHORS.saturating_sub(l.anchors.len());
    let names: Vec<String> =
        default_anchor_names(font, data, glyph).into_iter().filter(|n| !l.anchors.iter().any(|a| a.name == *n)).take(room).collect();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let place = Placer::new(font, data, g, l)?;
    let carets = names.iter().filter(|n| caret_index(n).is_some()).count() + l.anchors.iter().filter(|a| caret_index(&a.name).is_some()).count();
    Ok(names
        .into_iter()
        .map(|name| {
            let (x, y) = place.position(&name, carets);
            Anchor::new(name, finite(x), finite(y))
        })
        .collect())
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-fontcraft_model::MAX_COORDINATE, fontcraft_model::MAX_COORDINATE) } else { 0.0 }
}

fn dedup(names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names.into_iter().filter(|n| seen.insert(n.clone())).collect()
}

/// `N` of `caret_N`.
fn caret_index(name: &str) -> Option<usize> {
    name.strip_prefix("caret_")?.parse().ok()
}

fn role(font: &Font, data: &GlyphData, glyph: &str) -> Role {
    if glyph.starts_with("_segment.") {
        return Role::Segment;
    }
    if glyph.starts_with("_brush.") {
        return Role::Brush;
    }
    let (base, suffix) = data.split_suffix(glyph);
    let unicodes = font.glyph(glyph).map(|g| g.unicodes.clone()).unwrap_or_default();
    let info = data.lookup(glyph, &unicodes);
    let sub_category = match font.glyph(glyph).and_then(|g| g.info.sub_category.clone()) {
        Some(stored) => SubCategory::parse(&stored),
        None => info.sub_category.map(|s| s.value),
    };
    match category_of(font, data, glyph) {
        Some(Category::Mark) => {
            if base == "caroncomb" && suffix.is_some_and(|s| s.split('.').any(|p| p.starts_with("alt"))) {
                return Role::MarkCaronAlt;
            }
            let cp = data.lookup(base, &[]).unicode.map(|u| u.value).or(unicodes.first().copied());
            match cp {
                Some(OGONEK) => Role::MarkOgonek,
                Some(HORN) => Role::MarkHorn,
                Some(cp) => {
                    let class = data.canonical_combining_class(cp);
                    if class == 1 {
                        Role::MarkOverlay
                    } else if BELOW.contains(&class) {
                        Role::MarkBelow
                    } else if ABOVE.contains(&class) {
                        Role::MarkAbove
                    } else {
                        mark_by_name(base, sub_category)
                    }
                }
                None => mark_by_name(base, sub_category),
            }
        }
        Some(Category::Letter) => {
            let parts =
                if base.contains('_') && !base.starts_with('_') { data.split_ligature(base).map(|p| p.len()).filter(|n| *n >= 2) } else { None };
            if let Some(n) = parts {
                return Role::Ligature(n);
            }
            if sub_category == Some(SubCategory::Ligature) {
                return Role::Ligature(2);
            }
            if info.script.as_ref().is_some_and(|s| NO_MARK_SCRIPTS.contains(&s.value.as_str())) {
                return Role::Other;
            }
            Role::Letter
        }
        _ => Role::Other,
    }
}

/// A nonspacing mark without a combining class we know: below when its name says so.
fn mark_by_name(base: &str, sub_category: Option<SubCategory>) -> Role {
    if base.contains("below") || base.contains("cedilla") {
        Role::MarkBelow
    } else if base.contains("ogonek") {
        Role::MarkOgonek
    } else if sub_category == Some(SubCategory::Nonspacing) {
        Role::MarkAbove
    } else {
        Role::Other
    }
}

fn table_names(role: Role, base: &str) -> Vec<String> {
    match role {
        Role::Segment | Role::Brush => vec!["start".into(), "end".into()],
        Role::MarkAbove => vec!["_top".into(), "top".into()],
        Role::MarkBelow => vec!["_bottom".into(), "bottom".into()],
        Role::MarkOgonek => vec!["_ogonek".into()],
        Role::MarkHorn | Role::MarkCaronAlt => vec!["_topright".into()],
        Role::MarkOverlay => vec!["_center".into()],
        Role::Ligature(n) => (1..n.min(MAX_LAYER_ANCHORS)).map(|i| format!("caret_{i}")).collect(),
        Role::Letter => {
            let mut v = vec!["top".to_string(), "bottom".to_string()];
            if matches!(base, "A" | "E" | "I" | "O" | "U" | "a" | "e" | "i" | "idotless" | "o" | "u") {
                v.push("ogonek".into());
            }
            if matches!(base, "D" | "H" | "L" | "O" | "T" | "d" | "h" | "l" | "o" | "t") {
                v.push("center".into());
            }
            if matches!(base, "L" | "O" | "U" | "d" | "l" | "o" | "t" | "u") {
                v.push("topright".into());
            }
            v
        }
        Role::Other => Vec::new(),
    }
}

/// Places anchors on one layer.
struct Placer {
    role: Role,
    bounds: Option<Rect>,
    width: f64,
    x_height: f64,
    cap_height: f64,
    ascender: f64,
    /// `tan` of the italic angle.
    slope: f64,
    case: Option<Case>,
    case_mark: bool,
    rtl: bool,
    /// The first and last on-curve node of the layer's first open path.
    open_ends: Option<((f64, f64), (f64, f64))>,
}

impl Placer {
    fn new(font: &Font, data: &GlyphData, glyph: &Glyph, layer: &Layer) -> Result<Placer, ComponentsError> {
        let master = font.master(&layer.master)?;
        let upm = f64::from(font.units_per_em.max(1));
        let metric = |kind: &str, default: f64| {
            let v = font.metric_pos(master, kind, default);
            if v.is_finite() { v } else { default }
        };
        let angle = metric(metric::ITALIC_ANGLE, 0.0);
        let slope = if angle.abs() <= MAX_ITALIC_ANGLE { angle.to_radians().tan() } else { 0.0 };
        let outline = layer_outline(font, glyph, layer)?;
        let bounds = if outline.elements().is_empty() {
            None
        } else {
            Some(outline.bounding_box()).filter(|r| r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite())
        };
        let (_, suffix) = data.split_suffix(&glyph.name);
        let open_ends = layer.shapes.iter().find_map(|s| match s {
            Shape::Path(p) if !p.closed => {
                let mut on = p.nodes.iter().filter(|n| n.on_curve());
                let first = on.next()?;
                let last = on.next_back().unwrap_or(first);
                Some(((first.x, first.y), (last.x, last.y)))
            }
            _ => None,
        });
        Ok(Placer {
            role: role(font, data, &glyph.name),
            bounds,
            width: if layer.width.is_finite() { layer.width } else { 0.0 },
            x_height: metric(metric::X_HEIGHT, upm * 0.5),
            cap_height: metric(metric::CAP_HEIGHT, upm * 0.7),
            ascender: metric(metric::ASCENDER, upm * 0.8),
            slope,
            case: case_of(font, data, &glyph.name),
            case_mark: suffix.is_some_and(|s| s.split('.').any(|p| p == "case")),
            rtl: data.lookup(&glyph.name, &glyph.unicodes).direction.value == Direction::Rtl,
            open_ends,
        })
    }

    fn is_mark(&self) -> bool {
        matches!(self.role, Role::MarkAbove | Role::MarkBelow | Role::MarkOgonek | Role::MarkHorn | Role::MarkCaronAlt | Role::MarkOverlay)
    }

    /// Bounds, or a zero-height box across the advance for an empty layer.
    fn rect(&self) -> Rect {
        self.bounds.unwrap_or(Rect::new(0.0, 0.0, self.width, 0.0))
    }

    /// The horizontal centre an anchor is placed on: the outline's, or for a mark with an
    /// advance, the advance's middle.
    fn centre(&self) -> f64 {
        if self.is_mark() && self.width > 0.0 {
            return self.width / 2.0;
        }
        match self.bounds {
            Some(r) => (r.x0 + r.x1) / 2.0,
            None => self.width / 2.0,
        }
    }

    /// `x`, moved along the italic angle from the outline's middle height to `y`.
    fn slant(&self, x: f64, y: f64) -> f64 {
        let r = self.rect();
        x + (y - (r.y0 + r.y1) / 2.0) * self.slope
    }

    /// The height of a mark's attaching anchor: the x-height, or the cap height for a `.case` mark.
    fn mark_line(&self) -> f64 {
        if self.case_mark { self.cap_height } else { self.x_height }
    }

    /// The height of a letter's `top`.
    fn letter_top(&self) -> f64 {
        let r = self.rect();
        match self.case {
            Some(Case::Upper) => self.cap_height,
            Some(Case::Lower) => {
                if r.y1 > (self.x_height + self.ascender) / 2.0 {
                    self.ascender
                } else {
                    self.x_height
                }
            }
            _ => r.y1,
        }
    }

    fn letter_bottom(&self) -> f64 {
        match self.case {
            Some(Case::Upper | Case::Lower) => 0.0,
            _ => self.rect().y0,
        }
    }

    fn position(&self, name: &str, carets: usize) -> (f64, f64) {
        let r = self.rect();
        let centred = |y: f64| (self.slant(self.centre(), y), y);
        if let Some(i) = caret_index(name) {
            let n = match self.role {
                Role::Ligature(n) => n,
                _ => carets + 1,
            };
            return (self.width * i as f64 / n.max(1) as f64, 0.0);
        }
        match (name, self.role) {
            ("start", Role::Segment) => self.open_ends.map_or((0.0, 0.0), |(s, _)| s),
            ("end", Role::Segment) => self.open_ends.map_or((self.width, 0.0), |(_, e)| e),
            ("start", _) => (0.0, 0.0),
            ("end", _) => (self.width, 0.0),
            ("entry" | "#entry", _) => (if self.rtl { self.width } else { 0.0 }, 0.0),
            ("exit" | "#exit", _) => (if self.rtl { 0.0 } else { self.width }, 0.0),
            ("_top", _) => centred(self.mark_line()),
            ("top", _) if self.is_mark() => centred(r.y1),
            ("top", _) => centred(self.letter_top()),
            ("_bottom", _) => centred(0.0),
            ("bottom", _) if self.is_mark() => centred(r.y0),
            ("bottom", _) => centred(self.letter_bottom()),
            ("ogonek" | "_ogonek", _) => (self.slant(r.x1, 0.0), 0.0),
            ("_center", _) => centred(self.mark_line() / 2.0),
            ("center", _) => centred(self.letter_top().min(if self.case == Some(Case::Upper) { self.cap_height } else { self.x_height }) / 2.0),
            ("_topright", Role::MarkCaronAlt) => (r.x0, r.y1),
            ("_topright", _) => {
                let y = self.mark_line();
                (self.slant(r.x0, y), y)
            }
            ("topright", _) => {
                let y = self.letter_top();
                (self.slant(r.x1, y), y)
            }
            _ => centred(0.0),
        }
    }
}

/// The outline of `layer` with its components resolved. A layer that is not a master layer is
/// drawn as if it were its master's, its components resolved on that master.
fn layer_outline(font: &Font, glyph: &Glyph, layer: &Layer) -> Result<BezPath, ComponentsError> {
    if layer.is_master_layer() {
        return Ok(font.outline_of(glyph, &layer.master)?);
    }
    let mut alone = Glyph::new(glyph.name.as_str());
    let mut copy = layer.clone();
    copy.id = copy.master.clone();
    alone.layers.push(copy);
    Ok(font.outline_of(&alone, &layer.master)?)
}
