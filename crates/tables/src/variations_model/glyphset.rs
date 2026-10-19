//! Glyph outlines and advances of a variable font at a normalised location, as fontTools'
//! `TTFont.getGlyphSet(location=…)` draws them (`ttLib.ttGlyphSet`, MIT): `gvar` deltas scaled
//! by their tuple scalars, left-out points inferred, coordinates kept unrounded, components drawn
//! at the same location with their moved offsets. The advance is the `hmtx` advance plus the
//! unrounded `HVAR` delta, or, without `HVAR`, the rounded distance between the moved phantom
//! points. CFF2 charstrings are blended at the location quantised to F2Dot14, as read-fonts
//! takes it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::tables::glyf::{Anchor, CompositeGlyphFlags, Glyf, Glyph};
use read_fonts::tables::gvar::Gvar;
use read_fonts::tables::hmtx::Hmtx;
use read_fonts::tables::hvar::Hvar;
use read_fonts::tables::loca::Loca;
use read_fonts::types::{F2Dot14, Fixed, GlyphId, Tag as RTag};
use read_fonts::{FontData, FontRead, TableProvider};

use crate::instance::iup::{Pair, iup_delta};
use crate::instance::ivs::StoreDeltas;
use crate::instance::tuple_scalar;
use crate::limits::MAX_TUPLE_VARIATIONS;
use crate::metrics::{Contour, Pt, Seg, TtPoint, tt_contour};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Deepest component nesting followed.
const MAX_COMPONENT_DEPTH: usize = 64;
/// Most points, component placements and applied deltas drawn for each axis one call plans,
/// over every measurement it makes of the glyph set. The axes of a call share one counter of
/// this times their number.
pub(super) const MAX_DRAW_WORK_PER_AXIS: usize = 2_000_000_000;

struct Provider<'a>(&'a SfntFont);

impl<'a> TableProvider<'a> for Provider<'a> {
    fn data_for_tag(&self, tag: RTag) -> Option<FontData<'a>> {
        self.0.get(tag.into_bytes()).map(FontData::new)
    }
}

fn read_error(what: &'static str) -> impl Fn(read_fonts::ReadError) -> TablesError {
    move |e| TablesError::invalid(format!("the {what} cannot be read: {e}"))
}

enum Outlines<'a> {
    Glyf { loca: Loca<'a>, glyf: Glyf<'a>, gvar: Option<Gvar<'a>> },
    Cff(CffFontRef<'a>),
}

/// The glyphs of a font, ready to be drawn at locations.
pub(super) struct GlyphSet<'a> {
    pub count: usize,
    pub axis_count: usize,
    hmtx: Hmtx<'a>,
    hvar: Option<Hvar<'a>>,
    outlines: Outlines<'a>,
}

/// One glyph at a location.
pub(super) struct Drawn {
    pub contours: Vec<Contour>,
    pub width: f64,
}

/// The work budget of one call, shared by its passes over the glyphs.
pub(super) struct Work {
    /// What is left to spend.
    pub left: usize,
    limit: usize,
}

impl Work {
    pub(super) fn new(limit: usize) -> Self {
        Self { left: limit, limit }
    }

    fn spend(&mut self, n: usize) -> Result<()> {
        if n > self.left {
            self.left = 0;
            return Err(TablesError::TooLarge { what: "the outline data drawn", limit: self.limit });
        }
        self.left -= n;
        Ok(())
    }
}

impl<'a> GlyphSet<'a> {
    pub fn new(font: &'a SfntFont, axis_count: usize) -> Result<Self> {
        let p = Provider(font);
        let count = crate::ttx::num_glyphs(font)?;
        let hmtx = p.hmtx().map_err(read_error("hmtx table"))?;
        let hvar = match font.get(*b"HVAR") {
            Some(_) => Some(p.hvar().map_err(read_error("HVAR table"))?),
            None => None,
        };
        let outlines = if font.get(*b"glyf").is_some() {
            let gvar = match font.get(*b"gvar") {
                Some(bytes) => {
                    let gvar = Gvar::read(FontData::new(bytes)).map_err(read_error("gvar table"))?;
                    if usize::from(gvar.axis_count()) != axis_count {
                        return Err(TablesError::invalid(format!("the gvar table has {} axes and the fvar table {axis_count}", gvar.axis_count())));
                    }
                    Some(gvar)
                }
                None => None,
            };
            Outlines::Glyf { loca: p.loca(None).map_err(read_error("loca table"))?, glyf: p.glyf().map_err(read_error("glyf table"))?, gvar }
        } else if let Some(data) = font.get(*b"CFF2").or_else(|| font.get(*b"CFF ")) {
            Outlines::Cff(CffFontRef::new(data, 0, None).map_err(|e| TablesError::invalid(format!("the CFF data cannot be read: {e}")))?)
        } else {
            return Err(TablesError::MissingTable("glyf".into()));
        };
        Ok(Self { count, axis_count, hmtx, hvar, outlines })
    }

    /// The location `coords` (normalised, one value per `fvar` axis) prepared for drawing.
    pub fn at(&self, coords: &[f64]) -> Result<At<'_, 'a>> {
        let varied = coords.iter().any(|v| *v != 0.0);
        let hvar = match (&self.hvar, varied) {
            (Some(hvar), true) => {
                let store = hvar.item_variation_store().map_err(read_error("HVAR variation store"))?;
                Some(StoreDeltas::from_store(&store, coords)?)
            }
            _ => None,
        };
        let f2 = coords.iter().map(|v| F2Dot14::from_f32(*v as f32)).collect();
        Ok(At { set: self, coords: coords.to_vec(), varied, hvar, f2, deltas: RefCell::default() })
    }
}

/// A glyph set at one location.
pub(super) struct At<'s, 'a> {
    set: &'s GlyphSet<'a>,
    coords: Vec<f64>,
    varied: bool,
    hvar: Option<StoreDeltas>,
    f2: Vec<F2Dot14>,
    /// Each glyph's summed `gvar` deltas, decoded once however many composites use the glyph.
    deltas: RefCell<HashMap<u32, Rc<Vec<Pair>>>>,
}

impl At<'_, '_> {
    /// Glyph `gid`'s contours and advance width.
    pub fn glyph(&self, gid: u32, work: &mut Work) -> Result<Drawn> {
        let id = GlyphId::new(gid);
        let advance = f64::from(self.set.hmtx.advance(id).unwrap_or(0));
        let (contours, phantom_width) = match &self.set.outlines {
            Outlines::Glyf { loca, glyf, gvar } => {
                let gvar = if self.varied { gvar.as_ref() } else { None };
                let (points, (left, right)) = self.tt_points(loca, glyf, gvar, gid, 0, work)?;
                (points.iter().filter_map(|c| tt_contour(c)).collect(), gvar.map(|_| (advance + right - left + 0.5).floor()))
            }
            Outlines::Cff(cff) => {
                let fd = cff.subfont_index(id).unwrap_or(0);
                let sub =
                    cff.subfont(fd, &self.f2).map_err(|e| TablesError::invalid(format!("the font dictionary of glyph {gid} cannot be read: {e}")))?;
                let mut sink = Sink::default();
                cff.evaluate_charstring(&sub, id, &self.f2, &mut sink)
                    .map_err(|e| TablesError::invalid(format!("the charstring of glyph {gid} cannot be read: {e}")))?;
                sink.close();
                work.spend(sink.contours.iter().map(|c| c.segs.len()).sum::<usize>() + 1)?;
                (sink.contours, None)
            }
        };
        let width = match (&self.set.hvar, &self.hvar) {
            (Some(hvar), Some(deltas)) => {
                let index = match hvar.advance_width_mapping() {
                    Some(map) => {
                        let map = map.map_err(read_error("HVAR advance mapping"))?;
                        let i = map.get(gid).map_err(read_error("HVAR advance mapping"))?;
                        (i.outer, i.inner)
                    }
                    None => (0, u16::try_from(gid).unwrap_or(u16::MAX)),
                };
                advance + deltas.get(index.0, index.1)
            }
            (Some(_), None) => advance,
            (None, _) => phantom_width.unwrap_or(advance),
        };
        Ok(Drawn { contours, width })
    }

    /// The glyph's points contour by contour, components placed, and the x deltas of its left
    /// and right phantom points.
    #[allow(clippy::type_complexity)]
    fn tt_points(
        &self,
        loca: &Loca<'_>,
        glyf: &Glyf<'_>,
        gvar: Option<&Gvar<'_>>,
        gid: u32,
        depth: usize,
        work: &mut Work,
    ) -> Result<(Vec<Vec<TtPoint>>, (f64, f64))> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(TablesError::invalid(format!("the components of glyph {gid} nest deeper than {MAX_COMPONENT_DEPTH} levels")));
        }
        let glyph = loca.get_glyf(GlyphId::new(gid), glyf).map_err(|e| TablesError::invalid(format!("glyph {gid} cannot be read: {e}")))?;
        match glyph {
            None => {
                let deltas = self.deltas(gvar, gid, &[], &[], work)?;
                Ok((Vec::new(), phantom_x(&deltas)))
            }
            Some(Glyph::Simple(simple)) => {
                let n = simple.num_points();
                work.spend(n + 1)?;
                let mut base: Vec<Pair> = Vec::with_capacity(n + 4);
                let mut on: Vec<bool> = Vec::with_capacity(n);
                for p in simple.points() {
                    base.push((f64::from(p.x), f64::from(p.y)));
                    on.push(p.on_curve);
                }
                let ends: Vec<usize> = simple.end_pts_of_contours().iter().map(|e| usize::from(e.get())).collect();
                let deltas = self.deltas(gvar, gid, &base, &ends, work)?;
                let moved: Vec<TtPoint> = base
                    .iter()
                    .zip(&on)
                    .enumerate()
                    .map(|(i, (p, on))| {
                        let d = deltas.get(i).copied().unwrap_or((0.0, 0.0));
                        TtPoint { x: p.0 + d.0, y: p.1 + d.1, on: *on }
                    })
                    .collect();
                let mut contours = Vec::with_capacity(ends.len());
                let mut first = 0usize;
                for end in ends {
                    let slice = (end >= first)
                        .then(|| moved.get(first..=end))
                        .flatten()
                        .ok_or_else(|| TablesError::invalid(format!("glyph {gid} has contour ends out of order")))?;
                    contours.push(slice.to_vec());
                    first = end + 1;
                }
                Ok((contours, phantom_x(&deltas)))
            }
            Some(Glyph::Composite(composite)) => {
                let components: Vec<_> = composite.components().collect();
                let base: Vec<Pair> = components
                    .iter()
                    .map(|c| match c.anchor {
                        Anchor::Offset { x, y } => (f64::from(x), f64::from(y)),
                        Anchor::Point { .. } => (0.0, 0.0),
                    })
                    .collect();
                let ends: Vec<usize> = (0..components.len()).collect();
                let deltas = self.deltas(gvar, gid, &base, &ends, work)?;
                let mut out: Vec<Vec<TtPoint>> = Vec::new();
                for (i, c) in components.iter().enumerate() {
                    work.spend(1)?;
                    let (child, _) = self.tt_points(loca, glyf, gvar, u32::from(c.glyph.to_u16()), depth + 1, work)?;
                    let t = &c.transform;
                    let [xx, yx, xy, yy] = [t.xx, t.yx, t.xy, t.yy].map(|v| f64::from(v.to_bits()) / 16384.0);
                    let apply = |x: f64, y: f64| (xx * x + xy * y, yx * x + yy * y);
                    let mut placed: Vec<Vec<TtPoint>> = child
                        .into_iter()
                        .map(|contour| {
                            contour
                                .into_iter()
                                .map(|p| {
                                    let (x, y) = apply(p.x, p.y);
                                    TtPoint { x, y, on: p.on }
                                })
                                .collect()
                        })
                        .collect();
                    let (dx, dy) = match c.anchor {
                        Anchor::Offset { .. } => {
                            let b = base.get(i).copied().unwrap_or((0.0, 0.0));
                            let d = deltas.get(i).copied().unwrap_or((0.0, 0.0));
                            let (x, y) = (b.0 + d.0, b.1 + d.1);
                            let scaled = CompositeGlyphFlags::SCALED_COMPONENT_OFFSET;
                            if c.flags & (scaled | CompositeGlyphFlags::UNSCALED_COMPONENT_OFFSET) == scaled { apply(x, y) } else { (x, y) }
                        }
                        Anchor::Point { base, component } => {
                            let parent = out.iter().flatten().nth(usize::from(base));
                            let own = placed.iter().flatten().nth(usize::from(component));
                            match (parent, own) {
                                (Some(a), Some(b)) => (a.x - b.x, a.y - b.y),
                                _ => return Err(TablesError::invalid(format!("glyph {gid} attaches a component to a point it does not have"))),
                            }
                        }
                    };
                    for contour in &mut placed {
                        for p in contour.iter_mut() {
                            p.x += dx;
                            p.y += dy;
                        }
                    }
                    work.spend(placed.iter().map(Vec::len).sum())?;
                    out.extend(placed);
                }
                Ok((out, phantom_x(&deltas)))
            }
        }
    }

    /// The summed, scaled `gvar` deltas of glyph `gid` for its points `base` and the four phantom
    /// points after them. A glyph's deltas depend only on the glyph and the location, so they
    /// are computed once per glyph and charged to `work` then.
    fn deltas(&self, gvar: Option<&Gvar<'_>>, gid: u32, base: &[Pair], ends: &[usize], work: &mut Work) -> Result<Rc<Vec<Pair>>> {
        let n = base.len() + 4;
        let Some(gvar) = gvar else { return Ok(Rc::new(vec![(0.0, 0.0); n])) };
        if let Some(known) = self.deltas.try_borrow().ok().and_then(|cache| cache.get(&gid).cloned()) {
            return Ok(known);
        }
        let total = Rc::new(self.decode(gvar, gid, base, ends, work)?);
        if let Ok(mut cache) = self.deltas.try_borrow_mut() {
            cache.insert(gid, Rc::clone(&total));
        }
        Ok(total)
    }

    /// [`At::deltas`] without the cache: one decode of the glyph's tuples, each one that applies
    /// at this location charged per point (its deltas, the inferred ones and the sum).
    fn decode(&self, gvar: &Gvar<'_>, gid: u32, base: &[Pair], ends: &[usize], work: &mut Work) -> Result<Vec<Pair>> {
        let n = base.len() + 4;
        let mut total = vec![(0.0, 0.0); n];
        let Some(data) = gvar
            .glyph_variation_data(GlyphId::new(gid))
            .map_err(|e| TablesError::invalid(format!("the gvar data of glyph {gid} cannot be read: {e}")))?
        else {
            return Ok(total);
        };
        // Phantom points sit where no real point does; their position does not change the
        // deltas inferred for the outline, since each is a contour of its own.
        let mut coords0: Vec<Pair> = base.to_vec();
        coords0.extend([(0.0, 0.0); 4]);
        let f2 = |bits: i16| f64::from(bits) / 16384.0;
        for (count, tuple) in data.tuples().enumerate() {
            if count >= MAX_TUPLE_VARIATIONS {
                return Err(TablesError::TooLarge { what: "the tuple variations of one glyph", limit: MAX_TUPLE_VARIATIONS });
            }
            work.spend(1)?;
            let peak = tuple.peak();
            let start = tuple.intermediate_start();
            let end = tuple.intermediate_end();
            let region = (0..self.set.axis_count).map(|i| {
                let p = peak.get(i).map_or(0.0, |v| f2(v.to_bits()));
                let s = start.as_ref().and_then(|t| t.get(i)).map_or(p.min(0.0), |v| f2(v.to_bits()));
                let e = end.as_ref().and_then(|t| t.get(i)).map_or(p.max(0.0), |v| f2(v.to_bits()));
                (s, p, e)
            });
            let scalar = tuple_scalar(&self.coords, region);
            if scalar == 0.0 {
                continue;
            }
            work.spend(n)?;
            let mut explicit: Vec<Option<Pair>> = vec![None; n];
            for d in tuple.deltas() {
                if let Some(slot) = explicit.get_mut(usize::from(d.position)) {
                    *slot = Some((f64::from(d.x_delta), f64::from(d.y_delta)));
                }
            }
            let full: Vec<Pair> = if explicit.iter().all(Option::is_some) {
                explicit.into_iter().map(|d| d.unwrap_or((0.0, 0.0))).collect()
            } else {
                iup_delta(&explicit, &coords0, ends)
            };
            for (t, d) in total.iter_mut().zip(full) {
                t.0 += scalar * d.0;
                t.1 += scalar * d.1;
            }
        }
        Ok(total)
    }
}

/// The x deltas of the left and right phantom points (the last four of `deltas`).
fn phantom_x(deltas: &[Pair]) -> (f64, f64) {
    let n = deltas.len();
    let at = |k: usize| n.checked_sub(k).and_then(|i| deltas.get(i)).map_or(0.0, |d| d.0);
    (at(4), at(3))
}

fn fixed(v: Fixed) -> f64 {
    f64::from(v.to_bits()) / 65536.0
}

/// Charstring commands as closed contours.
#[derive(Default)]
struct Sink {
    contours: Vec<Contour>,
    open: Option<Contour>,
    pos: Pt,
}

impl Sink {
    fn begin(&mut self) -> &mut Contour {
        let start = self.pos;
        self.open.get_or_insert_with(|| Contour { start, segs: Vec::new() })
    }
}

impl CommandSink for Sink {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.close();
        self.pos = Pt { x: fixed(x), y: fixed(y) };
    }

    fn line_to(&mut self, x: Fixed, y: Fixed) {
        let (from, to) = (self.pos, Pt { x: fixed(x), y: fixed(y) });
        self.begin().segs.push(Seg::Line(from, to));
        self.pos = to;
    }

    fn curve_to(&mut self, cx0: Fixed, cy0: Fixed, cx1: Fixed, cy1: Fixed, x: Fixed, y: Fixed) {
        let from = self.pos;
        let (c0, c1, to) = (Pt { x: fixed(cx0), y: fixed(cy0) }, Pt { x: fixed(cx1), y: fixed(cy1) }, Pt { x: fixed(x), y: fixed(y) });
        self.begin().segs.push(Seg::Cubic(from, c0, c1, to));
        self.pos = to;
    }

    fn close(&mut self) {
        if let Some(mut c) = self.open.take() {
            if self.pos != c.start {
                c.segs.push(Seg::Line(self.pos, c.start));
            }
            self.pos = c.start;
            self.contours.push(c);
        }
    }
}
