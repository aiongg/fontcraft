//! Compiled fonts read into the source model (L3): `file.open` on a `.ttf`, `.otf`, collection or
//! WOFF file.
//!
//! Opening a compiled font is a reconstruction, not a load (`open.compiledIsLossy`): the result is
//! a one-master document with the outlines, advance widths, code points, glyph names, vertical
//! metrics and names of the font. TrueType outlines stay quadratic and composite glyphs stay
//! components (`open.truetypeKeepsQuadratic`, `open.truetypeKeepsComponents`); CFF outlines come
//! back cubic. Layout, hinting and every other table are not read; [`Decompiled::warnings`] lists
//! them. The glyph order is recorded in a `glyphOrder` custom parameter, so a later export keeps
//! the glyph ids (`open.recordsGlyphOrder`).
//!
//! Input is untrusted: every offset and size is checked, WOFF data is inflated only up to the size
//! its header states and at most [`MAX_SFNT`], and nothing here panics.
#![forbid(unsafe_code)]

mod woff;

pub use woff::{WoffTable, directory as woff_directory, to_sfnt as woff_to_sfnt};

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Layout, Str, Value};
use fontcraft_model::{
    Component, Family, Font, Glyph, GlyphTable, Layer, MAX_NAME_LEN, Master, MetricDef, MetricValue, Node, NodeKind, Path, Shape, Source,
    SourceFormat, Transform, metric,
};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::tables::glyf::{Anchor, Glyph as RawGlyph};
use skrifa::raw::types::{GlyphId, Tag};
use skrifa::raw::{FileRef, FontRef, TableProvider};
use skrifa::string::StringId;

/// Largest uncompressed font accepted, in bytes (WOFF data is inflated up to this).
pub const MAX_SFNT: usize = 256 << 20;

#[derive(Debug, thiserror::Error)]
pub enum DecompileError {
    #[error("not a font this build can read: {0}")]
    Unreadable(String),
    #[error("the file is a collection of {count} fonts; choose one with `face` (0 to {last})", last = count.saturating_sub(1))]
    ChooseFace { count: u32 },
    #[error("the collection has no font number {face}")]
    NoSuchFace { face: u32 },
    #[error(transparent)]
    Model(#[from] fontcraft_model::ModelError),
}

/// A document read from a compiled font, and what was left out.
pub struct Decompiled {
    pub font: Font,
    pub warnings: Vec<String>,
}

/// The kind of compiled font `bytes` holds, from its first bytes; `None` for anything else.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes.get(..4)? {
        [0, 1, 0, 0] | b"true" => Some("TrueType"),
        b"OTTO" => Some("OpenType (CFF)"),
        b"ttcf" => Some("collection"),
        b"wOFF" => Some("WOFF"),
        b"wOF2" => Some("WOFF2"),
        _ => None,
    }
}

/// Reads a compiled font. `face` chooses a member of a collection.
pub fn decompile(bytes: &[u8], face: Option<u32>) -> Result<Decompiled, DecompileError> {
    let unwrapped;
    let bytes = match bytes.get(..4) {
        Some(b"wOFF") => {
            unwrapped = woff::to_sfnt(bytes)?;
            unwrapped.as_slice()
        }
        Some(b"wOF2") => return Err(DecompileError::Unreadable("WOFF2 is not supported yet (ROADMAP.md, M2)".into())),
        _ => bytes,
    };
    let bad = |e: skrifa::raw::ReadError| DecompileError::Unreadable(e.to_string());
    let font = match FileRef::new(bytes).map_err(bad)? {
        FileRef::Font(f) => f,
        FileRef::Collection(c) => {
            let count = c.len();
            let face = face.ok_or(DecompileError::ChooseFace { count })?;
            c.get(face).map_err(|_| DecompileError::NoSuchFace { face })?
        }
    };
    read(&font)
}

fn english(font: &FontRef, ids: &[StringId]) -> Option<String> {
    ids.iter().find_map(|id| {
        let mut strings = font.localized_strings(*id);
        let english = strings.clone().find(|s| s.language().is_some_and(|l| l.starts_with("en")));
        english.or_else(|| strings.next()).map(|s| s.to_string()).filter(|s| !s.trim().is_empty())
    })
}

/// A name the model accepts: control characters removed, at most [`MAX_NAME_LEN`] bytes.
fn clean_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_NAME_LEN {
            break;
        }
        out.push(c);
    }
    out
}

const MASTER: &str = "m01";

fn read(font: &FontRef) -> Result<Decompiled, DecompileError> {
    let mut warnings = Vec::new();
    let head = font.head().map_err(|e| DecompileError::Unreadable(format!("no readable head table: {e}")))?;
    let units_per_em = head.units_per_em();
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let glyph_metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let count = u32::from(font.maxp().map(|m| m.num_glyphs()).unwrap_or(0));
    if font.fvar().is_ok() {
        warnings.push("a variable font: only its default instance was read".to_string());
    }

    // Names, made unique where the font repeats one.
    let names = font.glyph_names();
    let mut seen = HashSet::new();
    let mut glyph_names = Vec::with_capacity(count as usize);
    for gid in 0..count {
        let base =
            names.get(GlyphId::new(gid)).map(|n| clean_name(n.as_str())).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("glyph{gid:05}"));
        let mut name = base.clone();
        let mut n = 1;
        while !seen.insert(name.clone()) {
            name = clean_name(&format!("{base}.{n}"));
            n += 1;
        }
        glyph_names.push(name);
    }

    let mut unicodes: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (cp, gid) in font.charmap().mappings() {
        unicodes.entry(gid.to_u32()).or_default().push(cp);
    }

    let glyf = font.glyf().ok().zip(font.loca(None).ok());
    let outlines = font.outline_glyphs();
    let mut skipped_anchored = 0usize;
    let mut glyphs = Vec::with_capacity(glyph_names.len());
    for (gid, name) in (0..count).zip(&glyph_names) {
        let id = GlyphId::new(gid);
        let mut layer = Layer::new(MASTER, f64::from(glyph_metrics.advance_width(id).unwrap_or(0.0)));
        match &glyf {
            Some((glyf, loca)) => match loca.get(id, glyf) {
                Some(skrifa::raw::tables::loca::LocaGlyph::Glyph(RawGlyph::Simple(s))) => {
                    let points: Vec<_> = s.points().collect();
                    let mut start = 0usize;
                    for end in s.end_pts_of_contours() {
                        let end = usize::from(end.get());
                        if let Some(contour) = points.get(start..=end) {
                            layer.shapes.push(Shape::Path(truetype_contour(contour)));
                        }
                        start = end + 1;
                    }
                }
                Some(skrifa::raw::tables::loca::LocaGlyph::Glyph(RawGlyph::Composite(c))) => {
                    for comp in c.components() {
                        let Anchor::Offset { x, y } = comp.anchor else {
                            skipped_anchored += 1;
                            continue;
                        };
                        let Some(base) = glyph_names.get(comp.glyph.to_u32() as usize) else { continue };
                        let t = comp.transform;
                        let m = [t.xx.to_f64(), t.yx.to_f64(), t.xy.to_f64(), t.yy.to_f64(), f64::from(x), f64::from(y)];
                        layer.shapes.push(Shape::Component(Component {
                            glyph: base.clone(),
                            transform: Transform::Matrix(m),
                            info: Default::default(),
                            extra: Default::default(),
                        }));
                    }
                }
                _ => {}
            },
            None => {
                if let Some(outline) = outlines.get(id) {
                    let mut pen = CubicPen::default();
                    if outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).is_ok() {
                        layer.shapes.extend(pen.finish().into_iter().map(Shape::Path));
                    }
                }
            }
        }
        // A contour the model cannot draw (a broken font) is left out rather than refusing the font.
        layer.shapes.retain(|s| !matches!(s, Shape::Path(p) if p.to_bez().is_err()));
        let mut g = Glyph::new(name.clone());
        g.unicodes = unicodes.remove(&gid).unwrap_or_default();
        g.layers.push(layer);
        glyphs.push(Arc::new(g));
    }
    if skipped_anchored > 0 {
        warnings.push(format!("{skipped_anchored} components placed by point matching were left out"));
    }

    let family_name = english(font, &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]).map(|s| clean_name(&s)).unwrap_or_default();
    let style =
        english(font, &[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]).map(|s| clean_name(&s)).unwrap_or_else(|| "Regular".into());
    let mut defs = vec![
        MetricDef::new(metric::ASCENDER),
        MetricDef::new(metric::CAP_HEIGHT),
        MetricDef::new(metric::X_HEIGHT),
        MetricDef::new(metric::BASELINE),
        MetricDef::new(metric::DESCENDER),
    ];
    // The typographic ascender and descender are the source's own in fonts that fontc and ufo2ft
    // build (hhea adds the line gap to the ascender there); without OS/2, the line metrics.
    let (ascent, descent) =
        font.os2().ok().map_or((metrics.ascent, metrics.descent), |o| (f32::from(o.s_typo_ascender()), f32::from(o.s_typo_descender())));
    let mut values = [ascent, metrics.cap_height.unwrap_or(0.0), metrics.x_height.unwrap_or(0.0), 0.0, descent]
        .map(|v| MetricValue { pos: f64::from(v), over: 0.0 })
        .to_vec();
    if metrics.italic_angle != 0.0 {
        defs.push(MetricDef::new(metric::ITALIC_ANGLE));
        values.push(MetricValue { pos: f64::from(metrics.italic_angle), over: 0.0 });
    }
    let revision = head.font_revision().to_f64();
    let version_major = revision.trunc().clamp(0.0, f64::from(u16::MAX)) as u16;
    let version_minor = ((revision.fract() * 1000.0).round()).clamp(0.0, f64::from(u16::MAX)) as u16;

    // The glyph order, as the reference records it for an opened binary.
    let mut extra = fontcraft_model::Extra::new();
    let mut param = Dict::new();
    param.insert("name", Value::String("glyphOrder".into()));
    param.insert(
        "value",
        Value::Array(Array {
            items: glyph_names.iter().map(|n| Value::String(Str::new(n.as_str()))).collect(),
            layout: Layout::Multiline,
            ..Default::default()
        }),
    );
    extra.push(
        Family::Glyphs,
        "customParameters",
        Value::Array(Array { items: vec![Value::Dict(param)], layout: Layout::Multiline, ..Default::default() }),
    );

    let read_tables = ["head", "hhea", "hmtx", "maxp", "cmap", "name", "post", "OS/2", "glyf", "loca", "CFF "];
    let mut left_out: Vec<String> = font
        .table_directory
        .table_records()
        .iter()
        .map(|r| r.tag())
        .filter(|t| !read_tables.iter().any(|r| Tag::new_checked(r.as_bytes()).ok() == Some(*t)))
        .map(|t| t.to_string().trim_end().to_string())
        .collect();
    left_out.sort();
    if !left_out.is_empty() {
        warnings.push(format!("tables not read: {}", left_out.join(", ")));
    }

    let font = Font {
        source: Source { format: SourceFormat::Binary, ..Default::default() },
        family_name,
        units_per_em,
        version_major,
        version_minor,
        axes: Arc::default(),
        metrics: Arc::new(defs),
        stems: Arc::default(),
        numbers: Arc::default(),
        masters: Arc::new(vec![Master { id: MASTER.into(), name: style, metric_values: values, ..Default::default() }]),
        instances: Arc::default(),
        glyphs: GlyphTable::from_glyphs(glyphs),
        info: Default::default(),
        kerning: Arc::default(),
        features: Arc::default(),
        groups: Arc::default(),
        display_strings: Arc::default(),
        note: String::new(),
        files: Arc::default(),
        extra: Arc::new(extra),
    };
    font.check()?;
    Ok(Decompiled { font, warnings })
}

/// A TrueType contour as nodes, quadratic segments kept: an on-curve point ends a quadratic
/// segment when an off-curve point comes before it (cyclically), a line otherwise.
fn truetype_contour(points: &[skrifa::raw::tables::glyf::CurvePoint]) -> Path {
    let n = points.len();
    let nodes = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let prev_off = points.get((i + n - 1) % n).is_some_and(|q| !q.on_curve);
            let kind = match (p.on_curve, prev_off) {
                (false, _) => NodeKind::OffCurve,
                (true, true) => NodeKind::QCurve,
                (true, false) => NodeKind::Line,
            };
            Node::new(f64::from(p.x), f64::from(p.y), kind)
        })
        .collect();
    Path::new(true, nodes)
}

/// A segment drawn by a pen: its off-curve points, its end point, and the kind of that end.
type Segment = (Vec<(f64, f64)>, (f64, f64), NodeKind);

/// Collects cubic outlines from a pen into closed paths in the model's node convention: the node
/// a closing segment ends on is the first node, with that segment's off-curve nodes last.
#[derive(Default)]
struct CubicPen {
    paths: Vec<Path>,
    start: Option<(f64, f64)>,
    /// Segments since the move.
    segments: Vec<Segment>,
}

impl CubicPen {
    fn flush(&mut self) {
        let Some(start) = self.start.take() else { return };
        let mut segments = std::mem::take(&mut self.segments);
        // A final segment that comes back to the start closes the path on it.
        let closing = match segments.last() {
            Some((_, end, _)) if *end == start => segments.pop(),
            _ => None,
        };
        let first_kind = closing.as_ref().map_or(NodeKind::Line, |(_, _, k)| *k);
        let mut nodes = vec![Node::new(start.0, start.1, first_kind)];
        for (offs, end, kind) in segments {
            nodes.extend(offs.into_iter().map(|(x, y)| Node::new(x, y, NodeKind::OffCurve)));
            nodes.push(Node::new(end.0, end.1, kind));
        }
        if let Some((offs, _, _)) = closing {
            nodes.extend(offs.into_iter().map(|(x, y)| Node::new(x, y, NodeKind::OffCurve)));
        }
        if nodes.len() > 1 {
            self.paths.push(Path::new(true, nodes));
        }
    }

    fn finish(mut self) -> Vec<Path> {
        self.flush();
        self.paths
    }
}

impl OutlinePen for CubicPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.start = Some((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.segments.push((Vec::new(), (f64::from(x), f64::from(y)), NodeKind::Line));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.segments.push((vec![(f64::from(cx0), f64::from(cy0))], (f64::from(x), f64::from(y)), NodeKind::QCurve));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.segments.push((vec![(f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1))], (f64::from(x), f64::from(y)), NodeKind::Curve));
    }
    fn close(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests;
