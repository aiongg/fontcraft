//! SVG 1.1 fonts (format `svg-font`): a `font` element read into a one-master document, and a
//! master written as one.
//!
//! Read: `font-face` (family, units per em, ascent, descent, x-height, cap-height), the font's
//! default advance, the first `missing-glyph` as `.notdef`, each `glyph` with its path (font
//! units, y up), advance, code point (a `unicode` of one character; a longer one names a ligature
//! and gives none) and name (`glyph-name`, else made from the characters), and `hkern` pairs (by
//! `g1`/`g2` names or `u1`/`u2` characters, the value being minus `k`). Quadratic segments and
//! arcs become cubic curves; every subpath is closed, as SVG fonts fill them. Vertical kerning and
//! the glyph-selection attributes (`arabic-form`, `lang`, `orientation`) are not read.
//!
//! Written: what AFDKO tx writes, standalone: `font-face`, `missing-glyph` from `.notdef`, a
//! `glyph` per other glyph with `glyph-name`, its first code point and its outline with the
//! components resolved, and `hkern` for pairs between two glyphs (group kerning is not written).
//!
//! Input is untrusted: the document size, element count, path elements, coordinates and kerning
//! pairs are checked against the crate's limits.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_model::{
    Font, Glyph, GlyphTable, Kerning, Layer, MAX_COORDINATE, MAX_NAME_LEN, Master, MetricDef, MetricValue, Node, NodeKind, Path, Shape, Source,
    SourceFormat, UPM_RANGE, metric,
};
use usvg::roxmltree::{self, Document, Node as XmlNode, ParsingOptions};

use crate::{MAX_SVG_BYTES, MAX_SVG_ELEMENTS, MAX_SVG_NODES, SvgError};

/// Most kerning pairs read from one font (each `hkern` may name lists on both sides).
pub const MAX_SVG_KERN_PAIRS: usize = 1_000_000;
/// Most glyph names and characters read from the sides of all `hkern` elements together.
pub const MAX_SVG_KERN_SIDES: usize = 2_000_000;

const MASTER: &str = "m01";

/// A document read from an SVG font, and what was left out.
pub struct SvgFont {
    pub font: Font,
    pub warnings: Vec<String>,
}

/// Whether `text` is an SVG document with a `font` element.
pub fn is_svg_font(text: &str) -> bool {
    crate::looks_like_svg(text) && text.contains("<font") && (text.contains("<glyph") || text.contains("<missing-glyph"))
}

fn attr<'a>(n: &XmlNode<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

fn num_attr(n: &XmlNode, name: &str) -> Option<f64> {
    attr(n, name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}

/// A name the model accepts: control characters removed, at most [`MAX_NAME_LEN`] bytes.
fn clean_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control() && !c.is_whitespace()) {
        if out.len() + c.len_utf8() > MAX_NAME_LEN {
            break;
        }
        out.push(c);
    }
    out
}

/// Bytes kept free for a `.N` suffix when a long name repeats: a dot and up to seven digits.
const SUFFIX_ROOM: usize = 8;

/// Makes glyph names unique: a repeated name gets `.1`, `.2`… The number search for one stem
/// continues where it stopped, so every repeat costs a bounded number of tries, and a stem too
/// long for its suffix is cut so the name stays within [`MAX_NAME_LEN`].
#[derive(Default)]
struct UniqueNames {
    seen: HashSet<String>,
    next: HashMap<String, usize>,
}

impl UniqueNames {
    /// `base` if it is new, else the first free `{stem}.{n}`.
    fn add(&mut self, base: String) -> String {
        if self.seen.insert(base.clone()) {
            return base;
        }
        // Long names share one stem, cut at a fixed length, so two names that differ only past
        // the cut draw numbers from the same counter.
        let stem = if base.len() + SUFFIX_ROOM > MAX_NAME_LEN { cut(&base, MAX_NAME_LEN - SUFFIX_ROOM).to_string() } else { base };
        let n = self.next.entry(stem.clone()).or_insert(1);
        loop {
            let suffix = format!(".{n}");
            let name = format!("{}{suffix}", cut(&stem, MAX_NAME_LEN.saturating_sub(suffix.len())));
            *n += 1;
            if self.seen.insert(name.clone()) {
                return name;
            }
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.seen.contains(name)
    }
}

/// The longest prefix of `s` of at most `max` bytes that ends on a character boundary.
fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.get(..end).unwrap_or_default()
}

/// Reads the first `font` element of an SVG document.
pub fn read_svg_font(data: &[u8]) -> Result<SvgFont, SvgError> {
    if data.len() > MAX_SVG_BYTES {
        return Err(SvgError::TooLarge { size: data.len() });
    }
    let text = std::str::from_utf8(data).map_err(|_| SvgError::NotUtf8)?;
    let options = ParsingOptions { allow_dtd: true, nodes_limit: MAX_SVG_ELEMENTS as u32, ..ParsingOptions::default() };
    let doc = Document::parse_with_options(text, options).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => SvgError::TooManyElements,
        e => SvgError::Parse(e.to_string()),
    })?;
    let font_el = doc.descendants().find(|n| n.has_tag_name("font")).ok_or_else(|| SvgError::Parse("the document has no font element".into()))?;
    let face = font_el.children().find(|n| n.has_tag_name("font-face"));
    let mut warnings = Vec::new();

    let upem = face.and_then(|f| num_attr(&f, "units-per-em")).unwrap_or(1000.0).round();
    let units_per_em = if upem >= f64::from(*UPM_RANGE.start()) && upem <= f64::from(*UPM_RANGE.end()) {
        upem as u16
    } else {
        warnings.push(format!("units per em {upem} is out of range: 1000 was used"));
        1000
    };
    let default_advance = num_attr(&font_el, "horiz-adv-x").unwrap_or(0.0);
    let family_name = face
        .and_then(|f| attr(&f, "font-family"))
        .or_else(|| attr(&font_el, "id"))
        .map(|s| s.trim().trim_matches(['\'', '"']).chars().filter(|c| !c.is_control()).take(MAX_NAME_LEN).collect::<String>())
        .unwrap_or_default();

    let data_db = fontcraft_model::glyphdata::GlyphData::new().ok();
    let mut names = UniqueNames::default();
    let mut used = HashSet::new();
    let mut by_char: HashMap<String, String> = HashMap::new();
    let mut glyphs = Vec::new();
    let mut nodes_total = 0usize;
    let mut bad_paths = 0usize;
    let (mut missing_seen, mut extra_missing) = (false, 0usize);
    for (i, el) in font_el.children().filter(|n| n.has_tag_name("glyph") || n.has_tag_name("missing-glyph")).enumerate() {
        let missing = el.has_tag_name("missing-glyph");
        // The first `missing-glyph` is the font's `.notdef`; later ones are left out.
        if missing && missing_seen {
            extra_missing += 1;
            continue;
        }
        missing_seen |= missing;
        let unicode = attr(&el, "unicode").unwrap_or_default();
        let base = if missing {
            ".notdef".to_string()
        } else if let Some(n) = attr(&el, "glyph-name").map(|n| clean_name(n.split(',').next().unwrap_or(n))).filter(|n| !n.is_empty()) {
            n
        } else {
            let parts: Vec<String> =
                unicode.chars().map(|c| data_db.as_ref().map_or_else(|| format!("uni{:04X}", u32::from(c)), |d| d.nice_name(u32::from(c)))).collect();
            Some(clean_name(&parts.join("_"))).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("glyph{i:05}"))
        };
        let name = names.add(base);
        let mut glyph = Glyph::new(name.clone());
        let mut chars = unicode.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && !missing
            && used.insert(u32::from(c))
        {
            glyph.unicodes = vec![u32::from(c)];
        }
        if !unicode.is_empty() {
            by_char.entry(unicode.to_string()).or_insert_with(|| name.clone());
        }
        let mut layer = Layer::new(MASTER, num_attr(&el, "horiz-adv-x").unwrap_or(default_advance));
        if let Some(d) = attr(&el, "d").filter(|d| !d.trim().is_empty()) {
            match BezPath::from_svg(d).map_err(|e| e.to_string()).and_then(|b| to_paths(&b, &mut nodes_total)) {
                Ok(paths) => layer.shapes.extend(paths.into_iter().map(Shape::Path)),
                Err(_) if nodes_total > MAX_SVG_NODES => return Err(SvgError::TooManyNodes),
                Err(_) => bad_paths += 1,
            }
        }
        glyph.layers.push(layer);
        if missing {
            glyphs.insert(0, glyph);
        } else {
            glyphs.push(glyph);
        }
        if glyphs.len() > fontcraft_model::MAX_GLYPHS {
            return Err(SvgError::Parse(format!("the font has more than {} glyphs", fontcraft_model::MAX_GLYPHS)));
        }
    }
    if bad_paths > 0 {
        warnings.push(format!("{bad_paths} glyph outlines could not be read and were left empty"));
    }
    if extra_missing > 0 {
        warnings.push(format!("{extra_missing} further missing-glyph elements were not read: the first one is the font's .notdef"));
    }

    let mut kerning = Kerning::default();
    let (mut pairs, mut sides) = (0usize, 0usize);
    let mut vkern = 0usize;
    for el in font_el.children() {
        if el.has_tag_name("vkern") {
            vkern += 1;
            continue;
        }
        if !el.has_tag_name("hkern") {
            continue;
        }
        let Some(k) = num_attr(&el, "k") else { continue };
        let side = |g: &str, u: &str| -> Vec<&str> {
            let mut out: Vec<&str> = attr(&el, g).map(|v| v.split(',').map(str::trim).filter(|s| names.contains(s)).collect()).unwrap_or_default();
            if let Some(v) = attr(&el, u) {
                out.extend(v.split(',').filter_map(|s| by_char.get(s.trim()).map(String::as_str)));
            }
            out
        };
        let (firsts, seconds) = (side("g1", "u1"), side("g2", "u2"));
        sides = sides.saturating_add(firsts.len()).saturating_add(seconds.len());
        if sides > MAX_SVG_KERN_SIDES {
            return Err(SvgError::Parse(format!("the kerning names more than {MAX_SVG_KERN_SIDES} glyphs")));
        }
        pairs = pairs.saturating_add(firsts.len().saturating_mul(seconds.len()));
        if pairs > MAX_SVG_KERN_PAIRS {
            return Err(SvgError::Parse(format!("the font has more than {MAX_SVG_KERN_PAIRS} kerning pairs")));
        }
        for first in &firsts {
            for second in &seconds {
                kerning.ltr.set(MASTER, first, second, -k);
            }
        }
    }
    if vkern > 0 {
        warnings.push(format!("{vkern} vertical kerning pairs were not read"));
    }

    let face_num = |name: &str| face.and_then(|f| num_attr(&f, name));
    let values = [face_num("ascent"), face_num("cap-height"), face_num("x-height"), Some(0.0), face_num("descent")]
        .map(|v| MetricValue { pos: v.unwrap_or(0.0), over: 0.0 })
        .to_vec();
    let defs = vec![
        MetricDef::new(metric::ASCENDER),
        MetricDef::new(metric::CAP_HEIGHT),
        MetricDef::new(metric::X_HEIGHT),
        MetricDef::new(metric::BASELINE),
        MetricDef::new(metric::DESCENDER),
    ];
    let style = face.and_then(|f| attr(&f, "font-weight")).map_or("Regular", |w| match w.trim() {
        "bold" | "700" => "Bold",
        _ => "Regular",
    });
    let font = Font {
        source: Source { format: SourceFormat::Binary, ..Default::default() },
        family_name,
        units_per_em,
        version_major: 1,
        version_minor: 0,
        axes: Arc::default(),
        metrics: Arc::new(defs),
        stems: Arc::default(),
        numbers: Arc::default(),
        masters: Arc::new(vec![Master { id: MASTER.into(), name: style.into(), metric_values: values, ..Default::default() }]),
        instances: Arc::default(),
        info: Arc::default(),
        glyphs: GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect()),
        kerning: Arc::new(kerning),
        features: Arc::default(),
        groups: Arc::default(),
        display_strings: Arc::default(),
        note: String::new(),
        files: Arc::default(),
        extra: Arc::default(),
    };
    font.check().map_err(|e| SvgError::Parse(e.to_string()))?;
    Ok(SvgFont { font, warnings })
}

/// Closed model paths from SVG path data, quadratic segments raised to cubic.
fn to_paths(b: &BezPath, total: &mut usize) -> Result<Vec<Path>, String> {
    let mut paths = Vec::new();
    let mut start: Option<Point> = None;
    let mut last = Point::ZERO;
    // Segments since the move: off-curve points, end point, kind of the end.
    let mut segs: Vec<(Vec<Point>, Point, NodeKind)> = Vec::new();
    let flush = |start: &mut Option<Point>, segs: &mut Vec<(Vec<Point>, Point, NodeKind)>, paths: &mut Vec<Path>| {
        let Some(s) = start.take() else { return };
        let mut segs = std::mem::take(segs);
        let closing = match segs.last() {
            Some((_, end, _)) if *end == s => segs.pop(),
            _ => None,
        };
        let mut nodes = vec![Node::new(s.x, s.y, closing.as_ref().map_or(NodeKind::Line, |c| c.2))];
        for (offs, end, kind) in segs {
            nodes.extend(offs.into_iter().map(|p| Node::new(p.x, p.y, NodeKind::OffCurve)));
            nodes.push(Node::new(end.x, end.y, kind));
        }
        if let Some((offs, _, _)) = closing {
            nodes.extend(offs.into_iter().map(|p| Node::new(p.x, p.y, NodeKind::OffCurve)));
        }
        if nodes.len() > 1 {
            paths.push(Path::new(true, nodes));
        }
    };
    for el in b.elements() {
        *total += 3;
        if *total > MAX_SVG_NODES {
            return Err("too many nodes".into());
        }
        let points: &[Point] = match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => std::slice::from_ref(p),
            PathEl::QuadTo(a, p) => &[*a, *p],
            PathEl::CurveTo(a, c, p) => &[*a, *c, *p],
            PathEl::ClosePath => &[],
        };
        if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > MAX_COORDINATE || p.y.abs() > MAX_COORDINATE) {
            return Err("a coordinate is out of range".into());
        }
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut start, &mut segs, &mut paths);
                start = Some(p);
                last = p;
            }
            PathEl::LineTo(p) => {
                if start.is_none() {
                    start = Some(last);
                }
                segs.push((Vec::new(), p, NodeKind::Line));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                if start.is_none() {
                    start = Some(last);
                }
                let c1 = last + (c - last) * (2.0 / 3.0);
                let c2 = p + (c - p) * (2.0 / 3.0);
                segs.push((vec![c1, c2], p, NodeKind::Curve));
                last = p;
            }
            PathEl::CurveTo(a, c, p) => {
                if start.is_none() {
                    start = Some(last);
                }
                segs.push((vec![a, c], p, NodeKind::Curve));
                last = p;
            }
            PathEl::ClosePath => {
                if let Some(s) = start {
                    last = s;
                }
                flush(&mut start, &mut segs, &mut paths);
            }
        }
    }
    flush(&mut start, &mut segs, &mut paths);
    Ok(paths)
}

fn num(v: f64) -> String {
    let r = if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { 0.0 };
    if r == 0.0 { "0".to_string() } else { format!("{r}") }
}

/// Text for an XML attribute value; characters outside printable ASCII as references.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            ' '..='~' => out.push(c),
            c => {
                let _ = write!(out, "&#x{:X};", u32::from(c));
            }
        }
    }
    out
}

/// Path data in font units (y up), absolute commands.
fn path_data(b: &BezPath) -> String {
    let mut d = String::new();
    let pt = |p: Point| format!("{},{}", num(p.x), num(p.y));
    for el in b.elements() {
        let part = match *el {
            PathEl::MoveTo(p) => format!("M {}", pt(p)),
            PathEl::LineTo(p) => format!("L {}", pt(p)),
            PathEl::QuadTo(c, p) => format!("Q {} {}", pt(c), pt(p)),
            PathEl::CurveTo(a, c, p) => format!("C {} {} {}", pt(a), pt(c), pt(p)),
            PathEl::ClosePath => "Z".to_string(),
        };
        if !d.is_empty() {
            d.push(' ');
        }
        d.push_str(&part);
    }
    d
}

/// Master `master` of `font` as a standalone SVG font document.
pub fn write_svg_font(font: &Font, master: &str) -> Result<String, SvgError> {
    let m = font.masters.iter().find(|m| m.id == master).ok_or_else(|| SvgError::BadPath(format!("no master {master}")))?;
    let metric = |kind: &str| -> Option<f64> {
        let i = font.metrics.iter().position(|d| d.kind == kind)?;
        m.metric_values.get(i).map(|v| v.pos)
    };
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\">\n<defs>\n",
    );
    let id: String = font.family_name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let _ = writeln!(out, "<font id=\"{}\" horiz-adv-x=\"{}\">", escape(&id), font.units_per_em);
    let _ = write!(out, "<font-face font-family=\"{}\" units-per-em=\"{}\"", escape(&font.family_name), font.units_per_em);
    for (key, kind) in
        [("ascent", metric::ASCENDER), ("descent", metric::DESCENDER), ("x-height", metric::X_HEIGHT), ("cap-height", metric::CAP_HEIGHT)]
    {
        if let Some(v) = metric(kind) {
            let _ = write!(out, " {key}=\"{}\"", num(v));
        }
    }
    out.push_str("/>\n");
    let mut written = HashSet::new();
    for (missing, g) in font
        .glyphs
        .iter()
        .filter(|g| g.name == ".notdef")
        .map(|g| (true, g))
        .chain(font.glyphs.iter().filter(|g| g.name != ".notdef").map(|g| (false, g)))
    {
        let Some(layer) = g.layer(master) else { continue };
        let outline = font.outline_of(g, master).map_err(|e| SvgError::BadPath(e.to_string()))?;
        let d = path_data(&outline);
        if missing {
            out.push_str("<missing-glyph");
        } else {
            out.push_str("<glyph");
            if let Some(c) = g.unicodes.first().and_then(|u| char::from_u32(*u)) {
                let _ = write!(out, " unicode=\"{}\"", escape(&c.to_string()));
            }
            let _ = write!(out, " glyph-name=\"{}\"", escape(&g.name));
        }
        let _ = write!(out, " horiz-adv-x=\"{}\"", num(layer.width));
        if !d.is_empty() {
            let _ = write!(out, " d=\"{d}\"");
        }
        out.push_str("/>\n");
        written.insert(g.name.as_str());
    }
    for mk in font.kerning.ltr.0.iter().filter(|k| k.master == master) {
        for f in mk.firsts.iter().filter(|f| written.contains(f.first.as_str())) {
            for (second, value) in f.seconds.iter().filter(|(s, _)| written.contains(s.as_str())) {
                let _ = writeln!(out, "<hkern g1=\"{}\" g2=\"{}\" k=\"{}\"/>", escape(&f.first), escape(second), num(-value));
            }
        }
    }
    out.push_str("</font>\n</defs>\n</svg>\n");
    Ok(out)
}
