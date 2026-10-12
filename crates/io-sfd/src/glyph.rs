//! One `StartChar` … `EndChar` block: contours, references, anchors, metrics and per-glyph
//! kerning; the rest is skipped and reported.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_model::{MAX_COORDINATE, MAX_NAME_LEN, MAX_OUTLINE_ELEMENTS, MAX_UNICODES, Node, NodeKind, Path};

use crate::header::{RawFont, short, skip_a85, skip_instructions, skip_to};
use crate::lines::{self, Lines, Tok};
use crate::{MAX_ANCHORS_PER_GLYPH, MAX_GLYPH_POINTS, MAX_LAYERS, MAX_TOTAL_POINTS, Report, Result};

/// A shape of a layer, in file order.
#[derive(Clone, Debug)]
pub(crate) enum Piece {
    Path(Path),
    Ref(RawRef),
}

/// A reference, before the glyph it names is known.
#[derive(Clone, Debug)]
pub(crate) struct RawRef {
    pub(crate) gid: i64,
    pub(crate) unicode: i64,
    /// `[xx, yx, xy, yy, dx, dy]`.
    pub(crate) matrix: [f64; 6],
    pub(crate) at: String,
}

/// A kerning pair whose second glyph is still a glyph index.
#[derive(Clone, Debug)]
pub(crate) struct RawKern {
    pub(crate) second_gid: i64,
    pub(crate) value: f64,
    pub(crate) subtable: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RawGlyph {
    pub(crate) name: String,
    pub(crate) at: String,
    pub(crate) unicode: Option<u32>,
    pub(crate) gid: Option<i64>,
    pub(crate) alt_unicodes: Vec<u32>,
    /// The code points in `alt_unicodes`, for a membership test that stays fast on long lists.
    alt_seen: BTreeSet<u32>,
    pub(crate) width: Option<f64>,
    pub(crate) vert_width: Option<f64>,
    /// Font layer number → shapes.
    pub(crate) layers: BTreeMap<usize, Vec<Piece>>,
    pub(crate) anchors: Vec<(String, f64, f64)>,
    pub(crate) kerns: Vec<RawKern>,
    pub(crate) note: Option<String>,
    pub(crate) color: Option<[f64; 4]>,
    pub(crate) category: Option<&'static str>,
}

/// A contour being read.
struct Contour {
    layer: usize,
    quadratic: bool,
    start: (f64, f64),
    start_flags: u32,
    cur: (f64, f64),
    nodes: Vec<Node>,
    /// Parallel to `nodes`: an on-curve point the file marks as interpolated between two
    /// control points (flag 0x80), which a quadratic contour leaves implied.
    implied: Vec<bool>,
}

enum Op {
    Move((f64, f64)),
    Line((f64, f64)),
    Curve((f64, f64), (f64, f64), (f64, f64)),
}

const GLYPH_IGNORED: &[&str] =
    &["Flags", "Validated", "UnlinkRmOvrlpSave", "InSpiro", "LayerCount", "WasModified", "WasOrder2", "SplineSet", "EndSplineSet", "OrigPos"];

const GLYPH_REPORTED: &[(&str, &str)] = &[
    ("HStem", "PostScript stem hints"),
    ("VStem", "PostScript stem hints"),
    ("DStem", "PostScript stem hints"),
    ("DStem2", "PostScript stem hints"),
    ("CounterMasks", "PostScript stem hints"),
    ("CounterMask", "PostScript stem hints"),
    ("PathFlags", "contour flags (clipping paths)"),
    ("VKerns2", "vertical kerning pairs"),
    ("VKerns", "vertical kerning pairs"),
    ("Kerns", "kerning pairs in the format before lookups"),
    ("Position2", "glyph data of substitution and positioning lookups"),
    ("PairPos2", "glyph data of substitution and positioning lookups"),
    ("Ligature2", "glyph data of substitution and positioning lookups"),
    ("Substitution2", "glyph data of substitution and positioning lookups"),
    ("AlternateSubs2", "glyph data of substitution and positioning lookups"),
    ("MultipleSubs2", "glyph data of substitution and positioning lookups"),
    ("LCarets2", "ligature carets"),
    ("PickledData", "script data"),
    ("TEX", "TeX and MATH glyph data"),
    ("ItalicCorrection", "TeX and MATH glyph data"),
    ("TopAccentHorizontal", "TeX and MATH glyph data"),
    ("IsExtendedShape", "TeX and MATH glyph data"),
    ("GlyphVariantsVertical", "TeX and MATH glyph data"),
    ("GlyphVariantsHorizontal", "TeX and MATH glyph data"),
    ("GlyphCompositionVerticalIC", "TeX and MATH glyph data"),
    ("GlyphCompositionHorizontalIC", "TeX and MATH glyph data"),
    ("GlyphConstructionVertical", "TeX and MATH glyph data"),
    ("GlyphConstructionHorizontal", "TeX and MATH glyph data"),
    ("TopRightVertex", "TeX and MATH glyph data"),
    ("TopLeftVertex", "TeX and MATH glyph data"),
    ("BottomRightVertex", "TeX and MATH glyph data"),
    ("BottomLeftVertex", "TeX and MATH glyph data"),
    ("FillGradient", "Type 3 fills and strokes"),
    ("StrokeGradient", "Type 3 fills and strokes"),
    ("FillPattern", "Type 3 fills and strokes"),
    ("StrokePattern", "Type 3 fills and strokes"),
    ("TileMargin", "Type 3 fills and strokes"),
    ("TileBounds", "Type 3 fills and strokes"),
];

/// Reads a glyph block; the `StartChar` line has been read and `name` is its value.
pub(crate) fn read_char(lines: &mut Lines<'_>, raw: &mut RawFont, report: &mut Report, name: &str) -> Result<RawGlyph> {
    let name = lines::name(name);
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return Err(lines.err(format!("`{}` is not a usable glyph name", short(&name))));
    }
    let mut g = RawGlyph { name, at: lines.at(), ..RawGlyph::default() };
    let mut layer = 1usize;
    let mut contour: Option<Contour> = None;
    let mut points = GlyphPoints::default();
    let block = format!("glyph `{}` (no `EndChar` line)", g.name);
    loop {
        let line = lines.need(&block)?;
        if let Some(op) = point_line(lines, line)? {
            match op {
                (Op::Move(p), flags) => {
                    finish(contour.take(), &mut g, report, lines);
                    contour = Some(Contour {
                        layer,
                        quadratic: raw.quadratic(layer),
                        start: p,
                        start_flags: flags,
                        cur: p,
                        nodes: Vec::new(),
                        implied: Vec::new(),
                    });
                    count_points(lines, raw, 1)?;
                    points.add(lines, raw, layer, 1)?;
                }
                (op, flags) => {
                    let Some(c) = contour.as_mut() else {
                        return Err(lines.err("a contour does not start with a move (`m`)"));
                    };
                    let added = add_segment(c, op, flags);
                    count_points(lines, raw, added)?;
                    points.add(lines, raw, c.layer, added)?;
                    if c.nodes.len() > MAX_OUTLINE_ELEMENTS {
                        return Err(lines.err(format!("a contour has more than {MAX_OUTLINE_ELEMENTS} points")));
                    }
                }
            }
            continue;
        }
        finish(contour.take(), &mut g, report, lines);
        let (k, v) = lines::keyword(line);
        match k {
            "" => {}
            "EndChar" => break,
            "StartChar" => return Err(lines.err(format!("glyph `{}` has no `EndChar` line", g.name))),
            "Encoding" => {
                let n: Vec<i64> = v.split_whitespace().filter_map(lines::int).collect();
                g.unicode = n.get(1).and_then(|&u| u32::try_from(u).ok()).filter(|&u| char::from_u32(u).is_some());
                g.gid = n.get(2).copied();
            }
            "AltUni2" => alt_unicodes(&mut g, v, report, lines),
            "Width" => g.width = Some(coordinate(lines, v)?),
            "VWidth" => g.vert_width = Some(coordinate(lines, v)?),
            "Fore" => layer = 1,
            "Back" => layer = 0,
            "Layer" => {
                let mut words = v.split_whitespace();
                let n = words.next().and_then(lines::int).and_then(|n| usize::try_from(n).ok());
                let Some(n) = n.filter(|&n| n < MAX_LAYERS) else {
                    return Err(lines.err(format!("`Layer: {}` is not a layer number below {MAX_LAYERS}", short(v))));
                };
                if words.next().is_some() {
                    report.skip("Type 3 fills and strokes", &lines.at());
                }
                layer = n;
            }
            "Spiro" => {
                report.skip("Spiro control points (the Bézier contours are kept)", &lines.at());
                skip_to(lines, "EndSpiro", "Spiro")?;
            }
            "Refer" | "Ref" => {
                let r = reference(lines, v)?;
                g.layers.entry(layer).or_default().push(Piece::Ref(r));
            }
            "Image" | "Image2" => {
                report.skip("images", &lines.at());
                skip_to(lines, "EndImage", "Image")?;
            }
            "AnchorPoint" => anchor(&mut g, v, report, lines)?,
            "GlyphClass" => {
                g.category = match lines::int(v) {
                    Some(2) => Some("base"),
                    Some(3) => Some("ligature"),
                    Some(4) => Some("mark"),
                    Some(5) => Some("component"),
                    _ => None,
                }
            }
            "Kerns2" => kerns(&mut g, v, report, lines),
            "TtInstrs" => {
                report.skip("TrueType instructions", &lines.at());
                skip_instructions(lines)?;
            }
            "TtfInstrs" => {
                report.skip("TrueType instructions", &lines.at());
                skip_a85(lines, lines::int(v).unwrap_or(0))?;
            }
            "Comment" => {
                let value = lines.value(v)?;
                g.note = Some(lines::text(&value));
            }
            "Colour" => {
                g.color = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).ok().filter(|&c| c <= 0xFF_FFFF).map(|c| {
                    let ch = |shift: u32| f64::from((c >> shift) & 0xFF) / 255.0;
                    [ch(16), ch(8), ch(0), 1.0]
                });
            }
            _ if GLYPH_IGNORED.contains(&k) => {}
            _ => {
                lines.value(v)?;
                match GLYPH_REPORTED.iter().find(|(r, _)| *r == k) {
                    Some((_, what)) => report.skip(what, &lines.at()),
                    None => report.skip(&format!("unknown glyph keyword `{}`", short(k)), &lines.at()),
                }
            }
        }
    }
    Ok(g)
}

/// The points of one glyph so far, in all layers and per layer.
#[derive(Default)]
struct GlyphPoints {
    total: usize,
    /// Keyed by layer number, which is below `MAX_LAYERS`.
    per_layer: BTreeMap<usize, usize>,
}

impl GlyphPoints {
    fn add(&mut self, lines: &Lines<'_>, raw: &RawFont, layer: usize, n: usize) -> Result<()> {
        // A layer the font does not declare joins the master layer when the font is built.
        let layer = if layer <= 1 || raw.layers.contains_key(&layer) { layer } else { 1 };
        self.total = self.total.saturating_add(n);
        if self.total > MAX_GLYPH_POINTS {
            return Err(lines.err(format!("the glyph holds more than {MAX_GLYPH_POINTS} points")));
        }
        let in_layer = self.per_layer.entry(layer).or_default();
        *in_layer = in_layer.saturating_add(n);
        if *in_layer > MAX_OUTLINE_ELEMENTS {
            return Err(lines.err(format!("layer {layer} of the glyph holds more than {MAX_OUTLINE_ELEMENTS} points")));
        }
        Ok(())
    }
}

fn count_points(lines: &Lines<'_>, raw: &mut RawFont, n: usize) -> Result<()> {
    raw.points = raw.points.saturating_add(n);
    if raw.points > MAX_TOTAL_POINTS {
        return Err(lines.err(format!("the file holds more than {MAX_TOTAL_POINTS} points")));
    }
    Ok(())
}

fn coordinate(lines: &Lines<'_>, s: &str) -> Result<f64> {
    lines::num(s.trim())
        .filter(|v| v.abs() <= MAX_COORDINATE)
        .ok_or_else(|| lines.err(format!("`{}` is not a number within ±{MAX_COORDINATE}", short(s))))
}

/// A contour line (`x y m f`, `x y l f`, `x1 y1 x2 y2 x y c f`), or `None` for other lines. The
/// flags may carry a hint mask (`1xc0`) or TrueType point numbers, which are not kept.
fn point_line(lines: &Lines<'_>, line: &str) -> Result<Option<(Op, u32)>> {
    let words: Vec<&str> = line.split_whitespace().take(9).collect();
    let Some(first) = words.first() else { return Ok(None) };
    if lines::num(first).is_none() && !first.starts_with(|c: char| c == '-' || c == '.' || c.is_ascii_digit()) {
        return Ok(None);
    }
    let op_at = if matches!(words.get(2), Some(&("m" | "l"))) { 2 } else { 6 };
    let op = words.get(op_at).copied().unwrap_or("");
    if !matches!(op, "m" | "l" | "c") || (op == "c") != (op_at == 6) {
        return Err(lines.err("a contour line is not `x y m`, `x y l` or `x1 y1 x2 y2 x y c`"));
    }
    let mut n = [0.0; 6];
    for (slot, w) in n.iter_mut().zip(words.iter().take(op_at)) {
        *slot = coordinate(lines, w)?;
    }
    let flags = words.get(op_at + 1).map_or(0, |f| {
        let digits: String = f.chars().take_while(char::is_ascii_digit).take(9).collect();
        digits.parse::<u32>().unwrap_or(0)
    });
    let [a, b, c, d, e, f] = n;
    let op = match op {
        "m" => Op::Move((a, b)),
        "l" => Op::Line((a, b)),
        _ => Op::Curve((a, b), (c, d), (e, f)),
    };
    Ok(Some((op, flags)))
}

fn on_node(p: (f64, f64), kind: NodeKind, flags: u32) -> Node {
    let mut n = Node::new(p.0, p.1, kind);
    // 0x3: 0 curve point, 1 corner, 2 tangent.
    n.smooth = flags & 3 != 1;
    n.tangent = flags & 3 == 2;
    n
}

const QUAD_TOLERANCE: f64 = 0.01;

/// Adds a segment; returns how many nodes it added.
fn add_segment(c: &mut Contour, op: Op, flags: u32) -> usize {
    let implied = flags & 0x80 != 0;
    let before = c.nodes.len();
    match op {
        Op::Move(_) => {}
        Op::Line(p) => {
            c.nodes.push(on_node(p, NodeKind::Line, flags));
            c.implied.push(false);
            c.cur = p;
        }
        Op::Curve(c1, c2, p) => {
            let quad = if c.quadratic { quadratic_control(c.cur, c1, c2, p) } else { None };
            match quad {
                Some(q) => {
                    c.nodes.push(Node::new(q.0, q.1, NodeKind::OffCurve));
                    c.implied.push(false);
                    c.nodes.push(on_node(p, NodeKind::QCurve, flags));
                    c.implied.push(implied);
                }
                None => {
                    c.nodes.push(Node::new(c1.0, c1.1, NodeKind::OffCurve));
                    c.nodes.push(Node::new(c2.0, c2.1, NodeKind::OffCurve));
                    c.implied.extend([false, false]);
                    c.nodes.push(on_node(p, NodeKind::Curve, flags));
                    c.implied.push(false);
                }
            }
            c.cur = p;
        }
    }
    c.nodes.len() - before
}

/// The control point of a quadratic segment written as a cubic one: either both cubic controls
/// are that point, or they are the exact cubic form (two thirds of the way from each end to it).
fn quadratic_control(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p3: (f64, f64)) -> Option<(f64, f64)> {
    let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= QUAD_TOLERANCE && (a.1 - b.1).abs() <= QUAD_TOLERANCE;
    if close(c1, c2) && !close(c1, p0) && !close(c2, p3) {
        return Some(c1);
    }
    let q1 = (p0.0 + 1.5 * (c1.0 - p0.0), p0.1 + 1.5 * (c1.1 - p0.1));
    let q2 = (p3.0 + 1.5 * (c2.0 - p3.0), p3.1 + 1.5 * (c2.1 - p3.1));
    close(q1, q2).then(|| (((q1.0 + q2.0) / 2.0), ((q1.1 + q2.1) / 2.0)))
}

/// Ends a contour: closed when it comes back to its start, which then is the path's last node.
fn finish(c: Option<Contour>, g: &mut RawGlyph, report: &mut Report, lines: &Lines<'_>) {
    let Some(mut c) = c else { return };
    if c.nodes.is_empty() {
        report.skip("contours of a single point", &lines.at());
        return;
    }
    let ends_at_start = c.nodes.last().is_some_and(|n| n.x == c.start.0 && n.y == c.start.1);
    let closed = ends_at_start && c.nodes.len() > 1;
    if closed {
        let flags = c.start_flags;
        if let Some(last) = c.nodes.last_mut() {
            last.smooth = flags & 3 != 1;
            last.tangent = flags & 3 == 2;
        }
    } else {
        c.nodes.insert(0, on_node(c.start, NodeKind::Line, c.start_flags));
        c.implied.insert(0, false);
    }
    let nodes = if c.quadratic { drop_implied(c.nodes, &c.implied) } else { c.nodes };
    g.layers.entry(c.layer).or_default().push(Piece::Path(Path::new(closed, nodes)));
}

/// Drops on-curve points marked as implied that lie halfway between their two control points,
/// so runs of quadratic control points come back. The first and last nodes stay.
fn drop_implied(nodes: Vec<Node>, implied: &[bool]) -> Vec<Node> {
    let last = nodes.len().saturating_sub(1);
    let keep: Vec<bool> = (0..nodes.len())
        .map(|i| {
            if i == 0 || i == last || !implied.get(i).copied().unwrap_or(false) {
                return true;
            }
            let (Some(prev), Some(n), Some(next)) = (nodes.get(i - 1), nodes.get(i), nodes.get(i + 1)) else { return true };
            let halfway = ((prev.x + next.x) / 2.0 - n.x).abs() <= QUAD_TOLERANCE && ((prev.y + next.y) / 2.0 - n.y).abs() <= QUAD_TOLERANCE;
            !(n.kind == NodeKind::QCurve && prev.kind == NodeKind::OffCurve && next.kind == NodeKind::OffCurve && halfway)
        })
        .collect();
    nodes.into_iter().zip(keep).filter_map(|(n, k)| k.then_some(n)).collect()
}

/// `Refer: <gid> <unicode> <N|S> <a b c d e f> <flags> …`.
fn reference(lines: &Lines<'_>, v: &str) -> Result<RawRef> {
    let words: Vec<&str> = v.split_whitespace().take(10).collect();
    let bad = || lines.err("a reference is not `<glyph index> <code point> N a b c d e f`");
    let gid = words.first().and_then(|w| lines::int(w)).ok_or_else(bad)?;
    let unicode = words.get(1).and_then(|w| lines::int(w)).ok_or_else(bad)?;
    let from = if matches!(words.get(2), Some(&("N" | "S"))) { 3 } else { 2 };
    let mut m = [0.0; 6];
    for (i, slot) in m.iter_mut().enumerate() {
        let w = words.get(from + i).ok_or_else(bad)?;
        *slot = lines::num(w).filter(|x| x.abs() <= MAX_COORDINATE).ok_or_else(bad)?;
    }
    Ok(RawRef { gid, unicode, matrix: m, at: lines.at() })
}

/// `AnchorPoint: "<class>" x y <type> <ligature index> …`, named as the reference names anchors:
/// `top`, `_top` for marks, `top_1` for the first ligature component, `entry` and `exit`.
fn anchor(g: &mut RawGlyph, v: &str, report: &mut Report, lines: &Lines<'_>) -> Result<()> {
    let toks = lines::tokens(v);
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word(w)) => Some(*w),
        _ => None,
    };
    let class = match toks.first() {
        Some(Tok::Quoted(q)) => lines::utf7(q),
        Some(Tok::Word(w)) => lines::utf7(w),
        _ => String::new(),
    };
    let (Some(x), Some(y)) = (word(1), word(2)) else {
        return Err(lines.err("an anchor point is not `\"<class>\" x y <type>`"));
    };
    let (x, y) = (coordinate(lines, x)?, coordinate(lines, y)?);
    let lig = word(4).and_then(lines::int).unwrap_or(0);
    let name = match word(3).unwrap_or("basechar") {
        "mark" => format!("_{class}"),
        "baselig" => format!("{class}_{}", lig.saturating_add(1)),
        "entry" => "entry".to_string(),
        "exit" => "exit".to_string(),
        _ => class,
    };
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN || g.anchors.iter().any(|(n, _, _)| *n == name) {
        report.skip("anchor points without a name, or with the name of another anchor of the glyph", &lines.at());
        return Ok(());
    }
    if g.anchors.len() >= MAX_ANCHORS_PER_GLYPH {
        return Err(lines.err(format!("glyph `{}` has more than {MAX_ANCHORS_PER_GLYPH} anchors", g.name)));
    }
    g.anchors.push((name, x, y));
    Ok(())
}

/// `AltUni2: <code point>.<variation selector>.<reserved> …`, in hex.
fn alt_unicodes(g: &mut RawGlyph, v: &str, report: &mut Report, lines: &Lines<'_>) {
    for triple in v.split_whitespace() {
        if usize::from(g.unicode.is_some()) + g.alt_unicodes.len() >= MAX_UNICODES {
            report.skip(&format!("code points beyond the {MAX_UNICODES} one glyph may have"), &lines.at());
            return;
        }
        let mut parts = triple.split('.');
        let cp = parts.next().and_then(|p| u32::from_str_radix(p, 16).ok()).filter(|&u| char::from_u32(u).is_some());
        let vs = parts.next().map(|p| p.trim_start_matches('-'));
        match (cp, vs) {
            (Some(cp), None | Some("ffffffff" | "FFFFFFFF" | "1")) => {
                if g.unicode != Some(cp) && g.alt_seen.insert(cp) {
                    g.alt_unicodes.push(cp);
                }
            }
            (Some(_), Some(_)) => report.skip("Unicode variation sequences", &lines.at()),
            (None, _) => report.skip("alternate code points that are not code points", &lines.at()),
        }
    }
}

/// `Kerns2: <second glyph index> <value> "<subtable>" [{device}] …`.
fn kerns(g: &mut RawGlyph, v: &str, report: &mut Report, lines: &Lines<'_>) {
    let toks: Vec<Tok<'_>> = lines::tokens(v).into_iter().filter(|t| !matches!(t, Tok::Group(_))).collect();
    for chunk in toks.chunks(3) {
        match chunk {
            [Tok::Word(gid), Tok::Word(value), Tok::Quoted(sub)] => {
                match (lines::int(gid), lines::num(value).filter(|x| x.abs() <= MAX_COORDINATE)) {
                    (Some(second_gid), Some(value)) => g.kerns.push(RawKern { second_gid, value, subtable: sub.clone() }),
                    _ => report.skip("malformed kerning pairs", &lines.at()),
                }
            }
            _ => {
                report.skip("malformed kerning pairs", &lines.at());
                return;
            }
        }
    }
}
