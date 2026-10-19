//! Reading and writing `glyf` glyphs for [`super::transform`]: simple glyphs as points,
//! composites as component records, bounds worked out the way fontTools works them out on
//! saving (composites from their decomposed, transformed points).

use crate::limits::MAX_COORDINATE;
use crate::{Result, TablesError};

/// Deepest component nesting followed when working out composite bounds.
pub(super) const MAX_COMPONENT_DEPTH: usize = 32;
/// Most component references visited in all while working out every composite's bounds.
pub(super) const MAX_COMPONENT_VISITS: usize = 20_000_000;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
pub(super) const SCALED_COMPONENT_OFFSET: u16 = 0x0800;
pub(super) const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;
/// Component flags carried over as they are: round to grid, use my metrics, overlap, and the
/// two offset conventions.
const KEPT_FLAGS: u16 = 0x0004 | 0x0200 | 0x0400 | SCALED_COMPONENT_OFFSET | UNSCALED_COMPONENT_OFFSET;

const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME_OR_POSITIVE: u8 = 0x10;
const Y_SAME_OR_POSITIVE: u8 = 0x20;
const OVERLAP_SIMPLE: u8 = 0x40;
/// The off-curve point is a cubic control point (fontTools' `flagCubic`).
const CUBIC: u8 = 0x80;

/// fontTools' `otRound`: halves round up.
pub(super) fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// A rounded coordinate as an `i32`, refused outside ±32,767.
pub(super) fn coord(v: f64, what: &str) -> Result<i32> {
    let r = ot_round(v);
    if !r.is_finite() || r.abs() > MAX_COORDINATE {
        return Err(TablesError::Invalid(format!("{what} would be {r}, outside ±32,767")));
    }
    Ok(r as i32)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Point {
    pub x: i32,
    pub y: i32,
    pub on: bool,
    pub cubic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Simple {
    pub ends: Vec<u16>,
    pub points: Vec<Point>,
    pub instructions: Vec<u8>,
    pub overlap: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Args {
    Offset(i32, i32),
    /// Point matching: the composite's point, the component's point.
    Points(u16, u16),
}

/// How a component's 2×2 transform was stored, which is how it is written again when its
/// values still fit that form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Form {
    None,
    Scale,
    XyScale,
    TwoByTwo,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Component {
    /// The flags kept from the input ([`KEPT_FLAGS`]).
    pub flags: u16,
    pub glyph: u16,
    pub args: Args,
    /// `[xx, yx, xy, yy]`: x' = xx·x + xy·y, y' = yx·x + yy·y.
    pub matrix: [f64; 4],
    pub form: Form,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Composite {
    pub components: Vec<Component>,
    pub instructions: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Shape {
    Empty,
    Simple(Simple),
    Composite(Composite),
}

/// One glyph: its shape, and its input bytes, written again unchanged unless `changed`.
#[derive(Clone, Debug)]
pub(super) struct Glyph {
    pub shape: Shape,
    pub raw: Vec<u8>,
    pub changed: bool,
}

impl Glyph {
    /// The number of points `gvar` counts for the glyph, phantom points included.
    pub fn variation_points(&self) -> usize {
        4 + match &self.shape {
            Shape::Empty => 0,
            Shape::Simple(s) => s.points.len(),
            Shape::Composite(c) => c.components.len(),
        }
    }

    pub fn instructions(&self) -> &[u8] {
        match &self.shape {
            Shape::Empty => &[],
            Shape::Simple(s) => &s.instructions,
            Shape::Composite(c) => &c.instructions,
        }
    }

    /// Drops the glyph's instructions; true when it had any.
    pub fn strip_instructions(&mut self) -> bool {
        let list = match &mut self.shape {
            Shape::Empty => return false,
            Shape::Simple(s) => &mut s.instructions,
            Shape::Composite(c) => &mut c.instructions,
        };
        if list.is_empty() {
            return false;
        }
        list.clear();
        self.changed = true;
        true
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn f2dot14(v: u16) -> f64 {
    f64::from(v as i16) / 16384.0
}

/// The glyph offsets `loca` gives, `count + 1` of them, each within `glyf`.
pub(super) fn read_loca(loca: &[u8], long: bool, count: usize, glyf_len: usize) -> Result<Vec<usize>> {
    let mut offsets = Vec::with_capacity(count.saturating_add(1));
    for i in 0..=count {
        let at = if long { i.checked_mul(4) } else { i.checked_mul(2) }.ok_or_else(|| TablesError::invalid("the loca table is too short"))?;
        let v = if long {
            loca.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_be_bytes(b) as usize)
        } else {
            u16_at(loca, at).map(|v| usize::from(v) * 2)
        };
        let v = v.ok_or_else(|| TablesError::invalid("the loca table is shorter than the glyph count says"))?;
        offsets.push(v.min(glyf_len));
    }
    Ok(offsets)
}

/// Every glyph of the font, decoded.
pub(super) fn read_glyphs(glyf: &[u8], offsets: &[usize]) -> Result<Vec<Glyph>> {
    let mut glyphs = Vec::with_capacity(offsets.len().saturating_sub(1));
    for (gid, pair) in offsets.windows(2).enumerate() {
        let (start, end) = match pair {
            [a, b] => (*a, *b),
            _ => continue,
        };
        let raw = if end > start { glyf.get(start..end).unwrap_or_default().to_vec() } else { Vec::new() };
        let shape = decode(&raw).map_err(|m| TablesError::Invalid(format!("glyph {gid}: {m}")))?;
        glyphs.push(Glyph { shape, raw, changed: false });
    }
    Ok(glyphs)
}

fn decode(b: &[u8]) -> std::result::Result<Shape, String> {
    if b.is_empty() {
        return Ok(Shape::Empty);
    }
    let contours = i16_at(b, 0).ok_or("the glyph header is cut short")?;
    match contours {
        0 => Ok(Shape::Empty),
        n if n > 0 => decode_simple(b, n as usize).map(Shape::Simple),
        _ => decode_composite(b).map(Shape::Composite),
    }
}

fn decode_simple(b: &[u8], contours: usize) -> std::result::Result<Simple, String> {
    let short = "the outline data is cut short";
    let mut ends = Vec::with_capacity(contours);
    let mut at = 10;
    for _ in 0..contours {
        let e = u16_at(b, at).ok_or(short)?;
        if ends.last().is_some_and(|&last| e < last) {
            return Err("contour end points go backwards".into());
        }
        ends.push(e);
        at += 2;
    }
    let count = ends.last().map_or(0, |&e| usize::from(e) + 1);
    let ilen = usize::from(u16_at(b, at).ok_or(short)?);
    at += 2;
    let instructions = b.get(at..at + ilen).ok_or(short)?.to_vec();
    at += ilen;
    let mut flags = Vec::with_capacity(count);
    while flags.len() < count {
        let f = *b.get(at).ok_or(short)?;
        at += 1;
        flags.push(f);
        if f & REPEAT != 0 {
            let n = *b.get(at).ok_or(short)?;
            at += 1;
            for _ in 0..n {
                if flags.len() < count {
                    flags.push(f);
                }
            }
        }
    }
    let overlap = flags.first().is_some_and(|f| f & OVERLAP_SIMPLE != 0);
    let mut xs = Vec::with_capacity(count);
    let mut v = 0i32;
    for &f in &flags {
        if f & X_SHORT != 0 {
            let d = i32::from(*b.get(at).ok_or(short)?);
            at += 1;
            v += if f & X_SAME_OR_POSITIVE != 0 { d } else { -d };
        } else if f & X_SAME_OR_POSITIVE == 0 {
            v += i32::from(i16_at(b, at).ok_or(short)?);
            at += 2;
        }
        xs.push(v);
    }
    let mut points = Vec::with_capacity(count);
    let mut v = 0i32;
    for (&f, &x) in flags.iter().zip(&xs) {
        if f & Y_SHORT != 0 {
            let d = i32::from(*b.get(at).ok_or(short)?);
            at += 1;
            v += if f & Y_SAME_OR_POSITIVE != 0 { d } else { -d };
        } else if f & Y_SAME_OR_POSITIVE == 0 {
            v += i32::from(i16_at(b, at).ok_or(short)?);
            at += 2;
        }
        points.push(Point { x, y: v, on: f & ON_CURVE != 0, cubic: f & CUBIC != 0 });
    }
    Ok(Simple { ends, points, instructions, overlap })
}

fn decode_composite(b: &[u8]) -> std::result::Result<Composite, String> {
    let short = "the component data is cut short";
    let mut components = Vec::new();
    let mut at = 10;
    let mut has_instructions = false;
    loop {
        let flags = u16_at(b, at).ok_or(short)?;
        let glyph = u16_at(b, at + 2).ok_or(short)?;
        at += 4;
        let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let (a, c) = (u16_at(b, at).ok_or(short)?, u16_at(b, at + 2).ok_or(short)?);
            at += 4;
            if flags & ARGS_ARE_XY_VALUES != 0 { Args::Offset(i32::from(a as i16), i32::from(c as i16)) } else { Args::Points(a, c) }
        } else {
            let (a, c) = (*b.get(at).ok_or(short)?, *b.get(at + 1).ok_or(short)?);
            at += 2;
            if flags & ARGS_ARE_XY_VALUES != 0 {
                Args::Offset(i32::from(a as i8), i32::from(c as i8))
            } else {
                Args::Points(u16::from(a), u16::from(c))
            }
        };
        let (matrix, form) = if flags & WE_HAVE_A_SCALE != 0 {
            let s = f2dot14(u16_at(b, at).ok_or(short)?);
            at += 2;
            ([s, 0.0, 0.0, s], Form::Scale)
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let (x, y) = (f2dot14(u16_at(b, at).ok_or(short)?), f2dot14(u16_at(b, at + 2).ok_or(short)?));
            at += 4;
            ([x, 0.0, 0.0, y], Form::XyScale)
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            let mut m = [0.0; 4];
            for (i, slot) in m.iter_mut().enumerate() {
                *slot = f2dot14(u16_at(b, at + 2 * i).ok_or(short)?);
            }
            at += 8;
            (m, Form::TwoByTwo)
        } else {
            ([1.0, 0.0, 0.0, 1.0], Form::None)
        };
        has_instructions |= flags & WE_HAVE_INSTRUCTIONS != 0;
        components.push(Component { flags: flags & KEPT_FLAGS, glyph, args, matrix, form });
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    let instructions = if has_instructions {
        let n = usize::from(u16_at(b, at).ok_or(short)?);
        b.get(at + 2..at + 2 + n).ok_or(short)?.to_vec()
    } else {
        Vec::new()
    };
    Ok(Composite { components, instructions })
}

/// The bounds of every glyph (`None` for a glyph without outlines), composites from their
/// decomposed points, rounded as fontTools rounds them.
pub(super) fn all_bounds(glyphs: &[Glyph]) -> Result<Vec<Option<[i32; 4]>>> {
    let mut budget = MAX_COMPONENT_VISITS;
    let mut out = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        out.push(match &g.shape {
            Shape::Empty => None,
            Shape::Simple(s) => int_bounds(s.points.iter().map(|p| (f64::from(p.x), f64::from(p.y)))),
            Shape::Composite(_) => {
                let mut pts = Vec::new();
                decomposed(glyphs, g, 0, &mut budget, &mut pts)?;
                int_bounds(pts.into_iter())
            }
        });
    }
    Ok(out)
}

fn int_bounds(points: impl Iterator<Item = (f64, f64)>) -> Option<[i32; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for (x, y) in points {
        b = Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
    b.map(|v| v.map(|c| ot_round(c).clamp(-MAX_COORDINATE, MAX_COORDINATE) as i32))
}

/// Appends the glyph's points with every component resolved, as fontTools' `getCoordinates`
/// resolves them (point matching included).
fn decomposed(glyphs: &[Glyph], g: &Glyph, depth: usize, budget: &mut usize, out: &mut Vec<(f64, f64)>) -> Result<()> {
    match &g.shape {
        Shape::Empty => Ok(()),
        Shape::Simple(s) => {
            out.extend(s.points.iter().map(|p| (f64::from(p.x), f64::from(p.y))));
            Ok(())
        }
        Shape::Composite(c) => {
            if depth >= MAX_COMPONENT_DEPTH {
                return Err(TablesError::Invalid(format!("components nest deeper than {MAX_COMPONENT_DEPTH} levels, or refer to themselves")));
            }
            let base = out.len();
            for comp in &c.components {
                *budget = budget.checked_sub(1).ok_or_else(|| {
                    TablesError::Invalid(format!("the font's composites refer to components more than {MAX_COMPONENT_VISITS} times in all"))
                })?;
                let child = glyphs
                    .get(usize::from(comp.glyph))
                    .ok_or_else(|| TablesError::Invalid(format!("a component refers to glyph {}, which does not exist", comp.glyph)))?;
                let mut pts = Vec::new();
                decomposed(glyphs, child, depth + 1, budget, &mut pts)?;
                let [xx, yx, xy, yy] = comp.matrix;
                for p in pts.iter_mut() {
                    *p = (xx * p.0 + xy * p.1, yx * p.0 + yy * p.1);
                }
                let (dx, dy) = match comp.args {
                    Args::Offset(x, y) => {
                        let (x, y) = (f64::from(x), f64::from(y));
                        if comp.flags & SCALED_COMPONENT_OFFSET != 0 { (xx * x + xy * y, yx * x + yy * y) } else { (x, y) }
                    }
                    Args::Points(mine, theirs) => {
                        let mine = out.get(base + usize::from(mine)).copied();
                        let theirs = pts.get(usize::from(theirs)).copied();
                        match (mine, theirs) {
                            (Some(a), Some(b)) => (a.0 - b.0, a.1 - b.1),
                            _ => return Err(TablesError::invalid("a component's matching point does not exist")),
                        }
                    }
                };
                out.extend(pts.into_iter().map(|(x, y)| (x + dx, y + dy)));
            }
            Ok(())
        }
    }
}

/// The glyph's bytes: its input bytes when unchanged, else encoded with `bounds` in its header.
pub(super) fn encode(g: &Glyph, bounds: Option<[i32; 4]>) -> Result<Vec<u8>> {
    if !g.changed {
        return Ok(g.raw.clone());
    }
    match &g.shape {
        Shape::Empty => Ok(Vec::new()),
        Shape::Simple(s) => encode_simple(s, bounds.unwrap_or_default()),
        Shape::Composite(c) => encode_composite(c, bounds.unwrap_or_default()),
    }
}

fn header(out: &mut Vec<u8>, contours: i16, b: [i32; 4]) -> Result<()> {
    out.extend_from_slice(&contours.to_be_bytes());
    for v in b {
        let v = i16::try_from(v).map_err(|_| TablesError::Invalid(format!("a glyph bound would be {v}, outside ±32,767")))?;
        out.extend_from_slice(&v.to_be_bytes());
    }
    Ok(())
}

fn short_u16(len: usize, what: &str) -> Result<u16> {
    u16::try_from(len).map_err(|_| TablesError::Invalid(format!("{what} would be longer than 65,535")))
}

fn encode_simple(s: &Simple, b: [i32; 4]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let contours = i16::try_from(s.ends.len()).map_err(|_| TablesError::invalid("a glyph has more than 32,767 contours"))?;
    header(&mut out, contours, b)?;
    for e in &s.ends {
        out.extend_from_slice(&e.to_be_bytes());
    }
    out.extend_from_slice(&short_u16(s.instructions.len(), "a glyph's instructions")?.to_be_bytes());
    out.extend_from_slice(&s.instructions);
    let mut flags = Vec::with_capacity(s.points.len());
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (mut px, mut py) = (0i32, 0i32);
    for (i, p) in s.points.iter().enumerate() {
        let mut f = if p.on { ON_CURVE } else { 0 };
        if p.cubic {
            f |= CUBIC;
        }
        if i == 0 && s.overlap {
            f |= OVERLAP_SIMPLE;
        }
        f |= delta(p.x - px, X_SHORT, X_SAME_OR_POSITIVE, &mut xs)?;
        f |= delta(p.y - py, Y_SHORT, Y_SAME_OR_POSITIVE, &mut ys)?;
        (px, py) = (p.x, p.y);
        flags.push(f);
    }
    let mut i = 0;
    while let Some(&f) = flags.get(i) {
        let mut run = 1;
        while run < 256 && flags.get(i + run) == Some(&f) {
            run += 1;
        }
        if run > 1 {
            out.push(f | REPEAT);
            out.push((run - 1) as u8);
        } else {
            out.push(f);
        }
        i += run;
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
    Ok(out)
}

/// Writes one coordinate delta and gives its flag bits.
fn delta(d: i32, short: u8, same_or_positive: u8, out: &mut Vec<u8>) -> Result<u8> {
    if d == 0 {
        return Ok(same_or_positive);
    }
    if d.unsigned_abs() < 256 {
        out.push(d.unsigned_abs() as u8);
        return Ok(short | if d > 0 { same_or_positive } else { 0 });
    }
    let v = i16::try_from(d).map_err(|_| TablesError::Invalid(format!("a point would move by {d} units from the one before it")))?;
    out.extend_from_slice(&v.to_be_bytes());
    Ok(0)
}

fn f2dot14_bits(v: f64) -> Option<u16> {
    let r = (v * 16384.0).round();
    (r.is_finite() && (-32768.0..=32767.0).contains(&r)).then_some(r as i16 as u16)
}

/// The smallest form that holds `m`, never smaller than `wanted`.
fn form_for(m: [f64; 4], wanted: Form) -> Form {
    let [xx, yx, xy, yy] = m;
    let needed = if yx != 0.0 || xy != 0.0 {
        Form::TwoByTwo
    } else if xx != yy {
        Form::XyScale
    } else if xx != 1.0 {
        Form::Scale
    } else {
        Form::None
    };
    needed.max(wanted)
}

fn encode_composite(c: &Composite, b: [i32; 4]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    header(&mut out, -1, b)?;
    let count = c.components.len();
    for (i, comp) in c.components.iter().enumerate() {
        let mut flags = comp.flags;
        let mut args = Vec::with_capacity(4);
        match comp.args {
            Args::Offset(x, y) => {
                flags |= ARGS_ARE_XY_VALUES;
                if (-128..=127).contains(&x) && (-128..=127).contains(&y) {
                    args.extend([x as i8 as u8, y as i8 as u8]);
                } else {
                    let x = i16::try_from(x).map_err(|_| TablesError::Invalid(format!("a component offset would be {x}, outside ±32,767")))?;
                    let y = i16::try_from(y).map_err(|_| TablesError::Invalid(format!("a component offset would be {y}, outside ±32,767")))?;
                    flags |= ARG_1_AND_2_ARE_WORDS;
                    args.extend_from_slice(&x.to_be_bytes());
                    args.extend_from_slice(&y.to_be_bytes());
                }
            }
            Args::Points(a, p) => {
                if a <= 255 && p <= 255 {
                    args.extend([a as u8, p as u8]);
                } else {
                    flags |= ARG_1_AND_2_ARE_WORDS;
                    args.extend_from_slice(&a.to_be_bytes());
                    args.extend_from_slice(&p.to_be_bytes());
                }
            }
        }
        let form = form_for(comp.matrix, comp.form);
        let [xx, yx, xy, yy] = comp.matrix;
        let values: Vec<f64> = match form {
            Form::None => Vec::new(),
            Form::Scale => {
                flags |= WE_HAVE_A_SCALE;
                vec![xx]
            }
            Form::XyScale => {
                flags |= WE_HAVE_AN_X_AND_Y_SCALE;
                vec![xx, yy]
            }
            Form::TwoByTwo => {
                flags |= WE_HAVE_A_TWO_BY_TWO;
                vec![xx, yx, xy, yy]
            }
        };
        if i + 1 < count {
            flags |= MORE_COMPONENTS;
        } else if !c.instructions.is_empty() {
            flags |= WE_HAVE_INSTRUCTIONS;
        }
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&comp.glyph.to_be_bytes());
        out.extend_from_slice(&args);
        for v in values {
            let bits = f2dot14_bits(v)
                .ok_or_else(|| TablesError::Invalid(format!("a component transform value would be {v}, outside the range from -2 to 2")))?;
            out.extend_from_slice(&bits.to_be_bytes());
        }
    }
    if !c.instructions.is_empty() {
        out.extend_from_slice(&short_u16(c.instructions.len(), "a glyph's instructions")?.to_be_bytes());
        out.extend_from_slice(&c.instructions);
    }
    Ok(out)
}

/// The `glyf` and `loca` tables for `glyphs`, each glyph padded to four bytes as fontTools
/// pads them, and whether `loca` needs the long format.
pub(super) fn build(glyphs: &[Glyph], bounds: &[Option<[i32; 4]>]) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(glyphs.len() + 1);
    for (i, g) in glyphs.iter().enumerate() {
        offsets.push(glyf.len());
        let bytes = encode(g, bounds.get(i).copied().flatten())?;
        glyf.extend_from_slice(&bytes);
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
        if glyf.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "the glyf table", limit: crate::limits::MAX_SFNT });
        }
    }
    offsets.push(glyf.len());
    let long = glyf.len() > 0x1FFFE;
    let mut loca = Vec::with_capacity(offsets.len() * if long { 4 } else { 2 });
    for o in offsets {
        if long {
            loca.extend_from_slice(&(o as u32).to_be_bytes());
        } else {
            loca.extend_from_slice(&((o / 2) as u16).to_be_bytes());
        }
    }
    Ok((glyf, loca, long))
}
