//! Variable fonts opened into axes, masters and intermediate layers (M9.14).
//!
//! Rows: `variable-ttf.open`, `cff2.open`.
//!
//! Contract:
//! - Axes come from `fvar` (tags, names, ranges, hidden flags) with their `avar` maps written as
//!   the Axis Mappings parameter; named instances become instances, STAT labels axis labels.
//! - One master per peak region that every varying glyph uses at a corner of the designspace;
//!   regions that only some glyphs use become intermediate layers of those glyphs. Each source's
//!   outlines and advances are the font's at that location (skrifa draws them, `HVAR` or phantom
//!   points give the advances). CFF2 blends are read through read-fonts.
//! - What is left out (hinting, `cvar`, layout variations beyond kerning) is listed in
//!   [`crate::Decompiled::warnings`].
//! - Counts read from the file are checked before allocation: at most [`MAX_NAMED_INSTANCES`]
//!   named instances, [`MAX_SOURCES`] masters and intermediate locations, [`MAX_OPENED_LAYERS`]
//!   layers, [`MAX_MAPPING_POINTS`] `avar` points per axis, and per glyph the tuple and point
//!   counts the tables state checked against the glyph itself. A font beyond these limits is
//!   [`DecompileError::Unreadable`]; [`crate::decompile`] then opens its default instance.
//! - A static font passed here is opened as [`crate::decompile`] opens it.
//!
//! How the designspace is rebuilt:
//! - Design (internal) coordinates are the axis's user range laid over the normalized
//!   coordinates after `avar`: −1 is the minimum, 0 the default, 1 the maximum. Without `avar`
//!   they equal the user coordinates. The Axis Mappings parameter maps user to design values at
//!   each `avar` point, so normalizing a design value gives back the font's own coordinate.
//! - The default master comes first and the Variable Font Origin parameter names it.
//! - A peak becomes a master when every coordinate is −1, 0 or 1, or when at least half of the
//!   glyphs with variations use it; any other peak becomes an intermediate layer of the glyphs
//!   that use it, attached to the nearest master. A CFF2 font's regions all become masters.
//! - TrueType masters apply the `gvar` deltas (read-fonts infers the points a tuple leaves out)
//!   to the `glyf` points, so contours stay quadratic and composites stay components; values are
//!   rounded to whole units, as an instancer does.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fontcraft_model::axes::{AXIS_MAPPINGS, MAX_MAPPING_POINTS, ORIGIN, number_value, piecewise_linear_map, whole};
use fontcraft_model::plist::{Dict, Str, Value};
use fontcraft_model::{
    Axis, AxisLabel, AxisRange, Component, Font, Instance, Layer, LayerAttrs, LayerInfo, LayerInfoBox, MAX_AXIS_VALUES, Master, Node, NodeKind, Path,
    Shape, Transform, metric,
};
use skrifa::MetadataProvider;
use skrifa::metrics::GlyphMetrics;
use skrifa::outline::DrawSettings;
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::tables::glyf::{Anchor, Glyph as RawGlyph, PointFlags};
use skrifa::raw::tables::gvar::{DeltaBuffers, Gvar};
use skrifa::raw::tables::loca::LocaGlyph;
use skrifa::raw::tables::name::{Name, NameRecord};
use skrifa::raw::tables::stat::AxisValue;
use skrifa::raw::types::{F2Dot14, Fixed, GlyphId, Point, Tag};
use skrifa::raw::{FileRef, FontRef, TableProvider};
use skrifa::string::{LocalizedString, StringId};

use crate::{CubicPen, DecompileError, Decompiled, MASTER, clean_name};

/// Most named instances read from `fvar` (its 16-bit count).
pub const MAX_NAMED_INSTANCES: usize = 65_535;
/// Most masters and intermediate locations a variable font opens into.
pub const MAX_SOURCES: usize = 1_024;
/// Most layers the masters and intermediate locations add to an opened variable font (glyphs
/// times masters, plus intermediate layers).
pub const MAX_OPENED_LAYERS: usize = 2_000_000;
/// Most nodes and components across every layer an opened variable font gets.
pub const MAX_OPENED_NODES: usize = 20_000_000;
/// Most delta applications (points times tuples, summed over every glyph and location).
pub const MAX_DELTA_WORK: usize = 500_000_000;

/// The start of the static reader's note that only the default instance was read.
const DEFAULT_ONLY: &str = "a variable font:";
/// The start of the static reader's list of tables it left out.
const NOT_READ: &str = "tables not read: ";
/// Tables this module reads on top of the static reader.
const READ_HERE: &[&str] = &["fvar", "avar", "gvar", "HVAR", "MVAR", "STAT", "CFF2"];
/// F2Dot14's one.
const ONE: i16 = 16_384;

/// A normalized location, one F2Dot14 value (as bits) per axis.
type Peak = Vec<i16>;

/// Reads a variable font into a document with its axes, masters and intermediate layers. `face`
/// chooses a member of a collection.
pub fn open_variable(bytes: &[u8], face: Option<u32>) -> Result<Decompiled, DecompileError> {
    let unwrapped = crate::to_sfnt(bytes)?;
    let bytes = unwrapped.as_ref();
    let bad = |e: skrifa::raw::ReadError| DecompileError::Unreadable(e.to_string());
    let font = match FileRef::new(bytes).map_err(bad)? {
        FileRef::Font(f) => f,
        FileRef::Collection(c) => {
            let count = c.len();
            let face = face.ok_or(DecompileError::ChooseFace { count })?;
            c.get(face).map_err(|_| DecompileError::NoSuchFace { face })?
        }
    };
    if font.fvar().is_ok() { read_variable(&font) } else { crate::read(&font) }
}

/// What [`crate::decompile`] reads: a variable font with its masters, or, when its variations
/// cannot be read, its default instance with the reason among the warnings.
pub(crate) fn read_any(font: &FontRef) -> Result<Decompiled, DecompileError> {
    if font.fvar().is_err() {
        return crate::read(font);
    }
    match read_variable(font) {
        Ok(d) => Ok(d),
        Err(e) => {
            let mut d = crate::read(font)?;
            d.warnings.retain(|w| !w.starts_with(DEFAULT_ONLY));
            d.warnings.push(format!("the variations could not be read ({e}): only the default instance was opened"));
            Ok(d)
        }
    }
}

/// The name table's records grouped by name ID, read once: a font with many named instances or
/// STAT values would otherwise scan every record for each of them.
struct Names<'a> {
    name: Option<Name<'a>>,
    by_id: BTreeMap<u16, Vec<&'a NameRecord>>,
}

impl<'a> Names<'a> {
    fn new(sfnt: &FontRef<'a>) -> Self {
        let name = sfnt.name().ok();
        let mut by_id: BTreeMap<u16, Vec<&'a NameRecord>> = BTreeMap::new();
        if let Some(n) = &name {
            for record in n.name_record() {
                by_id.entry(record.name_id().to_u16()).or_default().push(record);
            }
        }
        Names { name, by_id }
    }

    /// The first of `ids` with a string that is not blank: its first English record, else its
    /// first record (the static reader's choice). At most [`MAX_DECODED_CHARS`] characters are
    /// decoded.
    fn english(&self, ids: &[StringId]) -> Option<String> {
        let name = self.name.as_ref()?;
        ids.iter().find_map(|id| {
            let mut first = None;
            let mut english = None;
            for record in self.by_id.get(&id.to_u16())? {
                let s = LocalizedString::new(name, record);
                if s.language().is_some_and(|l| l.starts_with("en")) {
                    english = Some(s);
                    break;
                }
                if first.is_none() {
                    first = Some(s);
                }
            }
            let text: String = english.or(first)?.chars().take(MAX_DECODED_CHARS).collect();
            (!text.trim().is_empty()).then_some(text)
        })
    }
}

/// Most characters of one name string decoded: well beyond what a model name keeps.
const MAX_DECODED_CHARS: usize = 4 * fontcraft_model::MAX_NAME_LEN;

/// An `fvar` axis with its `avar` map.
struct VarAxis {
    tag: String,
    name: String,
    hidden: bool,
    min: f64,
    default: f64,
    max: f64,
    /// `(from, to)` normalized points; empty for none.
    avar: Vec<(f64, f64)>,
}

impl VarAxis {
    /// The design coordinate of a normalized (post-`avar`) value.
    fn design(&self, n: f64) -> f64 {
        let n = n.clamp(-1.0, 1.0);
        let v = if n < 0.0 { self.default + n * (self.default - self.min) } else { self.default + n * (self.max - self.default) };
        round3(v)
    }

    /// The design coordinate of a user value.
    fn user_to_design(&self, user: f64) -> f64 {
        let n = if user < self.default && self.default > self.min {
            (user - self.default) / (self.default - self.min)
        } else if user > self.default && self.max > self.default {
            (user - self.default) / (self.max - self.default)
        } else {
            0.0
        };
        let n = n.clamp(-1.0, 1.0);
        let n = if self.avar.is_empty() { n } else { piecewise_linear_map(n, &self.avar) };
        self.design(n)
    }

    /// The Axis Mappings points: `(user, design)` at each `avar` point, or the identity at the
    /// minimum, default and maximum.
    fn mapping(&self) -> Vec<(f64, f64)> {
        if self.avar.is_empty() {
            return vec![(self.min, self.min), (self.default, self.default), (self.max, self.max)];
        }
        self.avar.iter().map(|(from, to)| (self.design(*from), self.design(*to))).collect()
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The rounding of fontTools' `otRound`.
fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn norm(bits: i16) -> f64 {
    f64::from(bits) / f64::from(ONE)
}

/// The running counts that keep a hostile font from making unbounded work.
#[derive(Default)]
struct Budget {
    work: usize,
    nodes: usize,
}

impl Budget {
    fn work(&mut self, n: usize) -> Result<(), DecompileError> {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_DELTA_WORK {
            return Err(DecompileError::Unreadable(format!("the variation data needs more than {MAX_DELTA_WORK} delta applications")));
        }
        Ok(())
    }

    /// Counts one layer and its nodes; every layer costs one unit of work, empty or not.
    fn nodes(&mut self, layer: &Layer) -> Result<(), DecompileError> {
        self.work(1)?;
        let n: usize = layer
            .shapes
            .iter()
            .map(|s| match s {
                Shape::Path(p) => p.nodes.len(),
                _ => 1,
            })
            .sum();
        self.nodes = self.nodes.saturating_add(n);
        if self.nodes > MAX_OPENED_NODES {
            return Err(DecompileError::Unreadable(format!("the masters would hold more than {MAX_OPENED_NODES} nodes")));
        }
        Ok(())
    }
}

fn read_axes(sfnt: &FontRef, names: &Names) -> Result<Vec<VarAxis>, DecompileError> {
    let axes = sfnt.axes();
    if axes.len() > MAX_AXIS_VALUES {
        return Err(DecompileError::Unreadable(format!("the font has {} axes; at most {MAX_AXIS_VALUES} can be opened", axes.len())));
    }
    let mut maps: Vec<Vec<(f64, f64)>> = Vec::new();
    if let Ok(avar) = sfnt.avar() {
        for m in avar.axis_segment_maps().iter().take(axes.len()) {
            let Ok(m) = m else {
                maps.push(Vec::new());
                continue;
            };
            let points = m.axis_value_maps();
            if points.len() > MAX_MAPPING_POINTS {
                return Err(DecompileError::Unreadable(format!(
                    "the avar map of axis {} has {} points; at most {MAX_MAPPING_POINTS} can be opened",
                    maps.len() + 1,
                    points.len()
                )));
            }
            maps.push(points.iter().map(|p| (p.from_coordinate().to_f64(), p.to_coordinate().to_f64())).collect());
        }
    }
    let mut out = Vec::with_capacity(axes.len());
    for (i, a) in axes.iter().enumerate() {
        let tag = a.tag().to_string();
        let (min, default, max) = (f64::from(a.min_value()), f64::from(a.default_value()), f64::from(a.max_value()));
        if !(min.is_finite() && default.is_finite() && max.is_finite() && min <= default && default <= max) {
            return Err(DecompileError::Unreadable(format!("the range of axis `{tag}` is not ordered")));
        }
        let name = names.english(&[a.name_id()]).map(|s| clean_name(&s)).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| clean_name(&tag));
        // An identity map, or one that does not keep −1, 0 and 1 in place, adds nothing usable.
        let avar = maps.get(i).cloned().unwrap_or_default();
        let fixed = [-1.0, 0.0, 1.0].iter().all(|v| avar.iter().any(|(f, t)| f == v && t == v));
        let identity = avar.iter().all(|(f, t)| f == t);
        let avar = if fixed && !identity { avar } else { Vec::new() };
        out.push(VarAxis { tag, name, hidden: a.is_hidden(), min, default, max, avar });
    }
    Ok(out)
}

/// The peaks a glyph's `gvar` tuples use, as indices into `ids` (which gains the peaks it has not
/// seen), and how many tuples the glyph has. Refuses a font that varies at more than
/// [`MAX_SOURCES`] locations as soon as it does.
fn glyph_peaks(
    gvar: &Gvar,
    gid: u32,
    axis_count: usize,
    ids: &mut BTreeMap<Peak, u16>,
    budget: &mut Budget,
) -> Result<(Vec<u16>, usize), DecompileError> {
    let mut used: Vec<u16> = Vec::new();
    let Ok(Some(data)) = gvar.glyph_variation_data(GlyphId::new(gid)) else { return Ok((used, 0)) };
    let mut tuples = 0usize;
    for t in data.tuples() {
        budget.work(1)?;
        tuples += 1;
        let peak = t.peak();
        if peak.len() != axis_count {
            continue;
        }
        let bits: Peak = (0..axis_count).map(|i| peak.get(i).map_or(0, F2Dot14::to_bits)).collect();
        if bits.iter().all(|b| *b == 0) || bits.iter().any(|b| !(-ONE..=ONE).contains(b)) {
            continue;
        }
        let id = match ids.get(&bits) {
            Some(id) => *id,
            None => {
                // The default is one more location.
                if ids.len().saturating_add(1) >= MAX_SOURCES {
                    return Err(DecompileError::Unreadable(format!(
                        "the font varies at more than {MAX_SOURCES} locations; at most {MAX_SOURCES} can be opened"
                    )));
                }
                let id = u16::try_from(ids.len()).map_err(|_| DecompileError::Unreadable("too many variation locations".into()))?;
                ids.insert(bits, id);
                id
            }
        };
        used.push(id);
    }
    used.sort_unstable();
    used.dedup();
    Ok((used, tuples))
}

/// The peaks of the regions in a CFF2 table's variation store.
fn cff2_peaks(sfnt: &FontRef, axis_count: usize) -> BTreeSet<Peak> {
    let mut set = BTreeSet::new();
    let Some(data) = sfnt.table_data(Tag::new(b"CFF2")) else { return set };
    let Ok(cff) = skrifa::raw::ps::cff::CffFontRef::new_cff2(data.as_bytes(), None) else { return set };
    let Some(store) = cff.var_store() else { return set };
    let Ok(regions) = store.variation_region_list() else { return set };
    for region in regions.variation_regions().iter().take(MAX_SOURCES.saturating_add(1)).flatten() {
        let axes = region.region_axes();
        let bits: Peak = (0..axis_count).map(|i| axes.get(i).map_or(0, |a| a.peak_coord().to_bits())).collect();
        if bits.iter().any(|b| *b != 0) && bits.iter().all(|b| (-ONE..=ONE).contains(b)) {
            set.insert(bits);
        }
    }
    set
}

/// A named instance of `fvar`.
struct NamedInstance {
    name: String,
    peak: Peak,
    design: Vec<f64>,
}

fn read_instances(sfnt: &FontRef, axes: &[VarAxis], names: &Names) -> Vec<NamedInstance> {
    let mut out = Vec::new();
    for (k, inst) in sfnt.named_instances().iter().enumerate().take(MAX_NAMED_INSTANCES) {
        let peak: Peak = inst.location().coords().iter().map(|c| c.to_bits()).collect();
        let design = axes.iter().zip(inst.user_coords()).map(|(a, u)| a.user_to_design(f64::from(u))).collect();
        let name = names
            .english(&[inst.subfamily_name_id()])
            .map(|s| clean_name(&s))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("Instance {}", k + 1));
        out.push(NamedInstance { name, peak, design });
    }
    out
}

/// STAT's design-axis order and labels (formats 1 to 3) on the axes they name.
fn read_stat(sfnt: &FontRef, axes: &mut [Axis], warnings: &mut Vec<String>, names: &Names) {
    let Ok(stat) = sfnt.stat() else { return };
    let Ok(design) = stat.design_axes() else { return };
    for rec in design {
        let tag = rec.axis_tag().to_string();
        if let Some(a) = axes.iter_mut().find(|a| a.tag == tag) {
            a.ordering = Some(u32::from(rec.axis_ordering()));
        }
    }
    let Some(Ok(values)) = stat.offset_to_axis_values() else { return };
    let mut combined = 0usize;
    for v in values.axis_values().iter().flatten() {
        let (Some(index), Some(value)) = (v.axis_index(), v.value()) else {
            combined += 1;
            continue;
        };
        let Some(tag) = design.get(usize::from(index)).map(|r| r.axis_tag().to_string()) else { continue };
        let Some(axis) = axes.iter_mut().find(|a| a.tag == tag) else { continue };
        let flags = v.flags().bits();
        let mut label = AxisLabel {
            name: names.english(&[v.value_name_id()]).map(|s| clean_name(&s)).unwrap_or_default(),
            user_value: value.to_f64(),
            linked_user_value: v.linked_value().map(Fixed::to_f64),
            elidable: flags & 0x0002 != 0,
            older_sibling: flags & 0x0001 != 0,
            ..Default::default()
        };
        if let AxisValue::Format2(f) = &v {
            label.user_minimum = Some(f.range_min_value().to_f64());
            label.user_maximum = Some(f.range_max_value().to_f64());
        }
        axis.labels.push(label);
    }
    if combined > 0 {
        warnings.push(format!("{combined} STAT labels for combinations of axes were left out"));
    }
}

/// A master's name: the named instance at its location, else its non-default coordinates.
fn master_name(axes: &[VarAxis], instances: &[NamedInstance], peak: &Peak) -> String {
    if let Some(inst) = instances.iter().find(|i| &i.peak == peak) {
        return inst.name.clone();
    }
    let parts: Vec<String> =
        axes.iter().zip(peak).filter(|(_, b)| **b != 0).map(|(a, b)| format!("{} {}", a.name, fmt_number(a.design(norm(*b))))).collect();
    clean_name(&parts.join(" "))
}

fn fmt_number(v: f64) -> String {
    whole(v).map_or_else(|| format!("{v}"), |i| i.to_string())
}

/// The Axis Mappings parameter value: `{tag: {user: design}}`.
fn mappings_value(axes: &[VarAxis]) -> Value {
    let mut all = Dict::new();
    for a in axes {
        let mut map = Dict::new();
        for (user, design) in a.mapping() {
            if let Some(v) = number_value(design) {
                map.insert(Str::new(fmt_number(user)), v);
            }
        }
        all.insert(Str::new(a.tag.as_str()), Value::Dict(map));
    }
    Value::Dict(all)
}

/// A component's base glyph name, matrix and offset.
type PlacedComponent = (String, [f64; 4], (f64, f64));

/// A glyph's `glyf` data, read once and placed at each location.
enum TtGlyph {
    Simple {
        /// The points, then the four phantom points.
        points: Vec<Point<Fixed>>,
        on_curve: Vec<bool>,
        ends: Vec<u16>,
    },
    /// Each component's base glyph (`None` for one placed by point matching, which the static
    /// reader leaves out), matrix and offset.
    Composite(Vec<Option<PlacedComponent>>),
    Empty,
}

impl TtGlyph {
    fn read(glyf: &skrifa::raw::tables::glyf::Glyf, loca: &skrifa::raw::tables::loca::Loca, gid: GlyphId, names: &[String]) -> TtGlyph {
        match loca.get(gid, glyf) {
            Some(LocaGlyph::Glyph(RawGlyph::Simple(s))) => {
                let mut points = Vec::new();
                let mut on_curve = Vec::new();
                for p in s.points() {
                    points.push(Point::new(Fixed::from_i32(i32::from(p.x)), Fixed::from_i32(i32::from(p.y))));
                    on_curve.push(p.on_curve);
                }
                points.extend([Point::new(Fixed::ZERO, Fixed::ZERO); 4]);
                TtGlyph::Simple { points, on_curve, ends: s.end_pts_of_contours().iter().map(|e| e.get()).collect() }
            }
            Some(LocaGlyph::Glyph(RawGlyph::Composite(c))) => TtGlyph::Composite(
                c.components()
                    .map(|comp| {
                        let Anchor::Offset { x, y } = comp.anchor else { return None };
                        let base = names.get(comp.glyph.to_u32() as usize)?;
                        let t = comp.transform;
                        Some((base.clone(), [t.xx.to_f64(), t.yx.to_f64(), t.xy.to_f64(), t.yy.to_f64()], (f64::from(x), f64::from(y))))
                    })
                    .collect(),
            ),
            _ => TtGlyph::Empty,
        }
    }

    /// The glyph's shapes at `coords`, as the static reader builds them at the default.
    fn shapes_at(
        &self,
        gvar: Option<&Gvar>,
        gid: GlyphId,
        coords: &[F2Dot14],
        tuples: usize,
        budget: &mut Budget,
    ) -> Result<Vec<Shape>, DecompileError> {
        let mut shapes = Vec::new();
        match self {
            TtGlyph::Simple { points, on_curve, ends } => {
                let n = points.len();
                let mut deltas = vec![Point::new(Fixed::ZERO, Fixed::ZERO); n];
                if let Some(gvar) = gvar
                    && tuples > 0
                {
                    budget.work(n.saturating_mul(tuples))?;
                    let mut flags = vec![PointFlags::default(); n];
                    let mut iup = deltas.clone();
                    let mut buffers = DeltaBuffers { deltas: &mut deltas, iup: &mut iup };
                    if gvar.simple_deltas(gid, coords, points, &mut flags, ends, &mut buffers).is_none() {
                        return Err(DecompileError::Unreadable(format!("the variations of glyph {} are damaged", gid.to_u32())));
                    }
                }
                let placed: Vec<(f64, f64, bool)> = points
                    .iter()
                    .zip(&deltas)
                    .zip(on_curve)
                    .map(|((p, d), on)| (ot_round(p.x.to_f64() + d.x.to_f64()), ot_round(p.y.to_f64() + d.y.to_f64()), *on))
                    .collect();
                let mut start = 0usize;
                for end in ends {
                    let end = usize::from(*end);
                    if let Some(contour) = placed.get(start..=end) {
                        shapes.push(Shape::Path(contour_path(contour)));
                    }
                    start = end + 1;
                }
            }
            TtGlyph::Composite(comps) => {
                let n = comps.len() + 4;
                let mut deltas = vec![Point::new(Fixed::ZERO, Fixed::ZERO); n];
                if let Some(gvar) = gvar
                    && tuples > 0
                {
                    budget.work(n.saturating_mul(tuples))?;
                    if gvar.composite_deltas(gid, coords, &mut deltas).is_none() {
                        return Err(DecompileError::Unreadable(format!("the variations of glyph {} are damaged", gid.to_u32())));
                    }
                }
                for (comp, d) in comps.iter().zip(&deltas) {
                    let Some((base, m, (x, y))) = comp else { continue };
                    let (dx, dy) = (d.x.to_f64(), d.y.to_f64());
                    let [a, b, c, e] = *m;
                    shapes.push(Shape::Component(Component {
                        glyph: base.clone(),
                        transform: Transform::Matrix([a, b, c, e, ot_round(x + dx), ot_round(y + dy)]),
                        info: Default::default(),
                        extra: Default::default(),
                    }));
                }
            }
            TtGlyph::Empty => {}
        }
        Ok(shapes)
    }
}

/// A TrueType contour as nodes (the static reader's convention): an on-curve point ends a
/// quadratic segment when an off-curve point comes before it, cyclically; a line otherwise.
fn contour_path(points: &[(f64, f64, bool)]) -> Path {
    let n = points.len();
    let nodes = points
        .iter()
        .enumerate()
        .map(|(i, (x, y, on))| {
            let prev_off = points.get((i + n - 1) % n).is_some_and(|q| !q.2);
            let kind = match (*on, prev_off) {
                (false, _) => NodeKind::OffCurve,
                (true, true) => NodeKind::QCurve,
                (true, false) => NodeKind::Line,
            };
            Node::new(*x, *y, kind)
        })
        .collect();
    Path::new(true, nodes)
}

/// What must match between two layers of a glyph for them to interpolate.
fn structure(shapes: &[Shape]) -> Vec<(usize, Vec<NodeKind>, &str)> {
    shapes
        .iter()
        .map(|s| match s {
            Shape::Path(p) => (p.nodes.len(), p.nodes.iter().map(|n| n.kind).collect(), ""),
            Shape::Component(c) => (0, Vec::new(), c.glyph.as_str()),
            _ => (0, Vec::new(), ""),
        })
        .collect()
}

/// A location the glyphs are read at.
struct Source {
    peak: Peak,
    coords: Vec<F2Dot14>,
    master: String,
}

/// Glyph-wide inputs to [`layer_at`].
struct Reader<'a> {
    gvar: Option<Gvar<'a>>,
    tt: Option<(skrifa::raw::tables::glyf::Glyf<'a>, skrifa::raw::tables::loca::Loca<'a>)>,
    outlines: skrifa::outline::OutlineGlyphCollection<'a>,
}

impl Reader<'_> {
    #[allow(clippy::too_many_arguments)]
    fn layer_at(
        &self,
        tt: Option<&TtGlyph>,
        gid: GlyphId,
        tuples: usize,
        coords: &[F2Dot14],
        metrics: &GlyphMetrics,
        default_width: f64,
        budget: &mut Budget,
    ) -> Result<Layer, DecompileError> {
        let mut shapes = match tt {
            Some(g) => g.shapes_at(self.gvar.as_ref(), gid, coords, tuples, budget)?,
            None => {
                budget.work(1)?;
                let mut pen = CubicPen::default();
                match self.outlines.get(gid) {
                    Some(outline) if outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::new(coords)), &mut pen).is_ok() => {
                        pen.finish().into_iter().map(Shape::Path).collect()
                    }
                    _ => Vec::new(),
                }
            }
        };
        shapes.retain(|s| !matches!(s, Shape::Path(p) if p.to_bez().is_err()));
        let width = metrics.advance_width(gid).map_or(default_width, |w| ot_round(f64::from(w)));
        let mut layer = Layer::new(MASTER, width);
        layer.shapes = shapes;
        budget.nodes(&layer)?;
        Ok(layer)
    }
}

fn read_variable(sfnt: &FontRef) -> Result<Decompiled, DecompileError> {
    let name_table = Names::new(sfnt);
    let axes = read_axes(sfnt, &name_table)?;
    if axes.is_empty() {
        return crate::read(sfnt);
    }
    let mut d = crate::read(sfnt)?;
    d.warnings.retain(|w| !w.starts_with(DEFAULT_ONLY));
    let axis_count = axes.len();
    let count = d.font.glyphs.len();
    let mut budget = Budget::default();

    // The peaks each glyph uses (indices into `peaks`), and how many tuples it has.
    let gvar = sfnt.gvar().ok();
    let mut per_glyph: Vec<(Vec<u16>, usize)> = Vec::with_capacity(count);
    let (peaks, shared): (Vec<Peak>, Option<Vec<u16>>) = match &gvar {
        Some(gvar) => {
            let mut ids: BTreeMap<Peak, u16> = BTreeMap::new();
            for gid in 0..count {
                let gid = u32::try_from(gid).map_err(|_| DecompileError::Unreadable("too many glyphs".into()))?;
                per_glyph.push(glyph_peaks(gvar, gid, axis_count, &mut ids, &mut budget)?);
            }
            let mut by_id: Vec<(u16, Peak)> = ids.into_iter().map(|(p, id)| (id, p)).collect();
            by_id.sort_unstable_by_key(|(id, _)| *id);
            (by_id.into_iter().map(|(_, p)| p).collect(), None)
        }
        None => {
            let all: Vec<Peak> = cff2_peaks(sfnt, axis_count).into_iter().collect();
            let ids = (0..all.len()).map(|i| u16::try_from(i).unwrap_or(u16::MAX)).collect();
            (all, Some(ids))
        }
    };
    let none: Vec<u16> = Vec::new();
    let peaks_of = |gid: usize| -> &[u16] {
        match &shared {
            Some(all) => all,
            None => per_glyph.get(gid).map_or(&none, |(ids, _)| ids),
        }
    };
    let peak_at = |id: &u16| peaks.get(usize::from(*id));
    let mut usage: BTreeMap<Peak, usize> = BTreeMap::new();
    let mut varying = 0usize;
    for gid in 0..count {
        let ids = peaks_of(gid);
        if !ids.is_empty() {
            varying += 1;
        }
        for p in ids.iter().filter_map(peak_at) {
            *usage.entry(p.clone()).or_default() += 1;
        }
    }
    if usage.len() >= MAX_SOURCES {
        return Err(DecompileError::Unreadable(format!("the font varies at {} locations; at most {MAX_SOURCES} can be opened", usage.len() + 1)));
    }
    let corner = |p: &Peak| p.iter().all(|b| *b == -ONE || *b == 0 || *b == ONE);
    let mut master_peaks: Vec<Peak> =
        usage.iter().filter(|(p, n)| shared.is_some() || corner(p) || n.saturating_mul(2) >= varying).map(|(p, _)| p.clone()).collect();
    master_peaks.sort_by_key(|p| (p.iter().filter(|b| **b != 0).count(), p.clone()));
    let is_master: BTreeSet<&Peak> = master_peaks.iter().collect();

    // Every glyph gets a layer per master and per intermediate location it uses: refuse the font
    // before making them when that is too many, empty glyphs included.
    let intermediates: usize = (0..count).map(|gid| peaks_of(gid).iter().filter_map(peak_at).filter(|p| !is_master.contains(p)).count()).sum();
    let layers = count.saturating_mul(master_peaks.len()).saturating_add(intermediates);
    if layers > MAX_OPENED_LAYERS {
        return Err(DecompileError::Unreadable(format!(
            "the masters and intermediate locations would make {layers} layers; at most {MAX_OPENED_LAYERS} can be opened"
        )));
    }

    // Masters: the default first, then one per master peak.
    let instances = read_instances(sfnt, &axes, &name_table);
    let default_design: Vec<f64> = axes.iter().map(|a| a.default).collect();
    let mut sources = vec![Source { peak: vec![0; axis_count], coords: vec![F2Dot14::ZERO; axis_count], master: MASTER.to_string() }];
    let mut masters: Vec<Master> = d.font.masters.iter().take(1).cloned().collect();
    let Some(first) = masters.first_mut() else { return Err(DecompileError::Model(fontcraft_model::ModelError::NoMaster)) };
    first.location = default_design;
    let base_master = first.clone();
    for (i, peak) in master_peaks.iter().enumerate() {
        let id = format!("m{:02}", i + 2);
        masters.push(Master {
            id: id.clone(),
            name: master_name(&axes, &instances, peak),
            location: axes.iter().zip(peak).map(|(a, b)| a.design(norm(*b))).collect(),
            metric_values: base_master.metric_values.clone(),
            ..Default::default()
        });
        sources.push(Source { peak: peak.clone(), coords: peak.iter().map(|b| F2Dot14::from_bits(*b)).collect(), master: id });
    }

    let reader = Reader { gvar, tt: sfnt.glyf().ok().zip(sfnt.loca(None).ok()), outlines: sfnt.outline_glyphs() };
    let names: Vec<String> = d.font.glyphs.iter().map(|g| g.name.clone()).collect();
    // A copy whose lifetime the locations below can meet.
    let short: FontRef<'_> = sfnt.clone();
    let master_metrics: Vec<GlyphMetrics> = sources.iter().map(|s| short.glyph_metrics(Size::unscaled(), LocationRef::new(&s.coords))).collect();
    // Each intermediate location's coordinates, metrics and nearest master, found once.
    let place_coords: BTreeMap<&Peak, Vec<F2Dot14>> =
        usage.keys().filter(|p| !is_master.contains(p)).map(|p| (p, p.iter().map(|b| F2Dot14::from_bits(*b)).collect())).collect();
    let mut places: BTreeMap<&Peak, (&[F2Dot14], GlyphMetrics, String)> = BTreeMap::new();
    for (&peak, coords) in &place_coords {
        let metrics = short.glyph_metrics(Size::unscaled(), LocationRef::new(coords));
        let nearest = sources
            .iter()
            .min_by_key(|s| s.peak.iter().zip(peak).map(|(a, b)| (i32::from(*a) - i32::from(*b)).unsigned_abs()).sum::<u32>())
            .map_or(MASTER, |s| s.master.as_str())
            .to_string();
        places.insert(peak, (coords.as_slice(), metrics, nearest));
    }
    let mut incompatible: Vec<String> = Vec::new();
    let mut intermediate = 0usize;
    let mut glyph_layers: Vec<Vec<Layer>> = Vec::with_capacity(count);
    for (gid, name) in names.iter().enumerate() {
        let id = GlyphId::new(u32::try_from(gid).map_err(|_| DecompileError::Unreadable("too many glyphs".into()))?);
        let tuples = per_glyph.get(gid).map_or(0, |(_, n)| *n);
        let tt = reader.tt.as_ref().map(|(glyf, loca)| TtGlyph::read(glyf, loca, id, &names));
        let base = d.font.glyphs.as_arcs().get(gid).and_then(|g| g.layers.first().cloned()).unwrap_or_else(|| Layer::new(MASTER, 0.0));
        let base_structure = structure(&base.shapes);
        let mut layers = Vec::new();
        let mut compatible = true;
        for (source, metrics) in sources.iter().zip(&master_metrics).skip(1) {
            let mut layer = reader.layer_at(tt.as_ref(), id, tuples, &source.coords, metrics, base.width, &mut budget)?;
            layer.id = source.master.clone();
            layer.master = source.master.clone();
            compatible &= structure(&layer.shapes) == base_structure;
            layers.push(layer);
        }
        // Peaks only some glyphs use: intermediate layers on the nearest master.
        for peak in peaks_of(gid).iter().filter_map(peak_at).filter(|p| !is_master.contains(p)) {
            let Some((coords, metrics, nearest)) = places.get(peak) else { continue };
            let mut layer = reader.layer_at(tt.as_ref(), id, tuples, coords, metrics, base.width, &mut budget)?;
            let nearest = nearest.as_str();
            let design: Vec<f64> = axes.iter().zip(peak).map(|(a, b)| a.design(norm(*b))).collect();
            intermediate += 1;
            layer.id = format!("{nearest}.i{}", layers.len() + 1);
            layer.master = nearest.to_string();
            layer.info = LayerInfoBox::new(LayerInfo {
                name: Some(format!("{{{}}}", design.iter().map(|v| fmt_number(*v)).collect::<Vec<_>>().join(", "))),
                attrs: LayerAttrs { coordinates: design.into_iter().map(Some).collect(), ..Default::default() },
                ..Default::default()
            });
            compatible &= structure(&layer.shapes) == base_structure;
            layers.push(layer);
        }
        if !compatible {
            incompatible.push(name.clone());
        }
        glyph_layers.push(layers);
    }
    drop(places);
    drop(master_metrics);

    // Font-level data.
    let font = &mut d.font;
    let mut model_axes: Vec<Axis> = axes
        .iter()
        .map(|a| Axis {
            name: a.name.clone(),
            tag: a.tag.clone(),
            hidden: a.hidden,
            range: Some(AxisRange { minimum: a.min, default: a.default, maximum: a.max }),
            ..Default::default()
        })
        .collect();
    read_stat(sfnt, &mut model_axes, &mut d.warnings, &name_table);
    font.axes = Arc::new(model_axes);
    font.masters = Arc::new(masters);
    font.instances = Arc::new(instances.into_iter().map(|i| Instance { name: i.name, location: i.design, ..Default::default() }).collect());
    let info = Arc::make_mut(&mut font.info);
    info.params.set(AXIS_MAPPINGS, mappings_value(&axes));
    info.params.set(ORIGIN, Value::String(Str::new(MASTER)));
    vary_metrics(sfnt, font, &sources);
    copy_kerning(font, &sources);
    for (gid, layers) in glyph_layers.into_iter().enumerate() {
        if let Some(mut g) = font.glyphs.get_mut_at(gid) {
            g.layers.extend(layers);
        }
    }
    font.check()?;

    // Warnings.
    if let Some(w) = d.warnings.iter_mut().find(|w| w.starts_with(NOT_READ)) {
        let rest: Vec<&str> = w.get(NOT_READ.len()..).unwrap_or_default().split(", ").filter(|t| !READ_HERE.contains(t)).collect();
        *w = if rest.is_empty() { String::new() } else { format!("{NOT_READ}{}", rest.join(", ")) };
    }
    d.warnings.retain(|w| !w.is_empty());
    if sources.len() > 1 && !d.font.kerning.is_empty() {
        d.warnings.push("kerning variations were not read: every master has the default master's kerning".into());
    }
    if intermediate > 0 {
        d.warnings.push(format!("{intermediate} intermediate layers were made for locations only some glyphs vary at"));
    }
    if !incompatible.is_empty() {
        let shown: Vec<&str> = incompatible.iter().take(10).map(String::as_str).collect();
        d.warnings.push(format!("{} glyphs have layers that do not interpolate: {}", incompatible.len(), shown.join(", ")));
    }
    Ok(d)
}

/// Each master's ascender, descender, cap height and x-height: the default's plus the `MVAR`
/// delta at the master's location.
fn vary_metrics(sfnt: &FontRef, font: &mut Font, sources: &[Source]) {
    let Ok(mvar) = sfnt.mvar() else { return };
    let Some(base) = font.masters.first().cloned() else { return };
    for source in sources.iter().skip(1) {
        for (kind, tag) in [(metric::ASCENDER, b"hasc"), (metric::DESCENDER, b"hdsc"), (metric::CAP_HEIGHT, b"cpht"), (metric::X_HEIGHT, b"xhgt")] {
            if font.metric(&base, kind).is_none() {
                continue;
            }
            let Some(delta) = mvar.metric_delta(Tag::new(tag), &source.coords) else { continue };
            let pos = ot_round(font.metric_pos(&base, kind, 0.0) + delta.to_f64());
            // The definition exists (checked above), so this only sets the value.
            let _ = font.set_metric_pos(&source.master, kind, pos);
        }
    }
}

/// Gives every master the default master's kerning: the pairs stay constant across the axes.
fn copy_kerning(font: &mut Font, sources: &[Source]) {
    if font.kerning.is_empty() || sources.len() < 2 {
        return;
    }
    let kerning = Arc::make_mut(&mut font.kerning);
    for table in [&mut kerning.ltr, &mut kerning.rtl, &mut kerning.vertical] {
        let Some(base) = table.0.iter().find(|m| m.master == MASTER).cloned() else { continue };
        for source in sources.iter().skip(1) {
            let mut copy = base.clone();
            copy.master = source.master.clone();
            table.0.push(copy);
        }
    }
}

#[cfg(test)]
#[path = "variable_tests.rs"]
mod tests;
