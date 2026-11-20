//! Autohinting the charstrings of a compiled variable font's `CFF2` table (row
//! `autohint.ps.runFont`): each glyph's outline is drawn at every master of its region list, the
//! caller's hinter hints the masters together, and the charstring is written again with one hint
//! structure whose stem edges and widths are blended over the same regions. The outline keeps
//! every number and blend it had; every other table keeps its bytes.
//!
//! | Step | Rule |
//! |---|---|
//! | input | the `CFF2` table with its subroutine calls expanded |
//! | masters | per region list (`vsindex`): the default location, then the peak of each region, in list order; `fvar` and `avar` are not needed, as the regions are stored in normalized coordinates |
//! | refused lists | a region whose peak is the default location, two regions with the same peak, a region that is not zero at the default location, more masters than the autohinter hints together: the glyphs of that list are left as they were, with the reason |
//! | outlines | each master's outline is the charstring's blends evaluated at the master's location |
//! | zones and stems | each Font DICT's Private DICT evaluated at each master's location |
//! | glyphs left as they are | not selected; already hinted unless `hint_all`; drawing nothing; `flex1` with varying operands; the hinter finding no stems or failing; stems whose blends do not fit the table's operand stack |
//! | stems | per master as the hinter returns them, rounded unless the caller keeps fractions, merged into one structure (the same stems, order and masks in every master), and each stem number blended: its value at the default location, and the deltas that give each master's value at the master's location |
//! | check | each new charstring is read back: its outline, every delta included, and its region list must equal the old ones, else the glyph keeps its old charstring |
//! | subroutines | the table is subroutinised again, then every glyph's expanded charstring is compared with the one written |
//!
//! The merge into one blended structure follows fontTools' `CFF2CharStringMergePen` (MIT,
//! Copyright (c) 2017 Just van Rossum and contributors), through `fontcraft_cff::cff2`.

use std::collections::HashMap;

use fontcraft_cff::cff2::{Blend, BlendOp, BlendPoint, BlendStem, Cff2Glyph, Cff2Hints, MAX_AXES, RegionAxis, encode_cff2_charstring, shared_hints};
use fontcraft_cff::subr;
use fontcraft_cff::{CharstringHints, MAX_SEGMENTS, PathOp, PrivateDict, StemHint};
use read_fonts::ps::cff::blend::BlendState;
use read_fonts::ps::cff::dict::{Entry, entries};
use read_fonts::ps::cff::index::Index;
use read_fonts::tables::cff2::Cff2;
use read_fonts::tables::post::Post;
use read_fonts::tables::variations::ItemVariationStore;
use read_fonts::types::{F2Dot14, GlyphId16};
use read_fonts::{FontData, FontRead};

use super::autohint_cff::{
    CffAutohint, CffAutohintOptions, CffHinted, CffSkipped, MAX_DICT_ENTRIES, MAX_FONT_DICTS, from_subr, read_private, round_stems, write_table,
};
use crate::limits::{MAX_CFF2_STACK, MAX_CHARSTRING, MAX_CHARSTRING_MEMORY};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

const CFF2: [u8; 4] = *b"CFF2";
const HEAD: [u8; 4] = *b"head";
const POST: [u8; 4] = *b"post";
/// Most masters of one glyph hinted together (the autohinter's own limit): the default location
/// and 63 regions.
pub const MAX_HINT_MASTERS: usize = 64;
/// Most regions read from the variation store.
const MAX_STORE_REGIONS: usize = 65_535;
/// Below this, a pivot of the region scalars is taken as zero: two regions peak at one place.
const SINGULAR: f64 = 1e-9;
/// A delta this close to a whole number is that number: floating-point noise of the solve.
const DELTA_NOISE: f64 = 1e-6;

/// What the hinter needs to know about a `CFF2` font before it runs.
#[derive(Clone, Debug)]
pub struct Cff2HintSetup {
    /// Each glyph's name, from `post`, else `glyphNNNNN`.
    pub names: Vec<String>,
    /// Each Font DICT's Private DICT at the default location, in FDArray order.
    pub privates: Vec<PrivateDict>,
    /// The lowest and highest y of the font: `head`'s box, else −0.25 and 1.25 em.
    pub y_range: (f64, f64),
}

/// One glyph handed to the hinter: its outline at each master, the default location first.
#[derive(Clone, Copy, Debug)]
pub struct Cff2HintGlyph<'a> {
    pub gid: usize,
    pub name: &'a str,
    /// Its Font DICT, an index into [`Cff2HintSetup::privates`].
    pub fd: usize,
    /// Its region list. Glyphs with the same `fd` and `vsindex` have the same masters and
    /// Private DICT values.
    pub vsindex: u16,
    /// Each master's normalized location.
    pub locations: &'a [Vec<f64>],
    /// The glyph's Private DICT values at each master.
    pub privates: &'a [PrivateDict],
    /// The glyph's outline at each master, the same segments in each.
    pub paths: &'a [Vec<PathOp>],
}

/// The hinter: one set of hints per master (the same structure in each), `None` when it found no
/// stems, or why it failed.
pub type Cff2Hinter<'h> = dyn FnMut(&Cff2HintGlyph<'_>) -> std::result::Result<Option<Vec<CharstringHints>>, String> + 'h;

fn invalid(what: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the CFF2 table: {what}"))
}

// ── Reading the table ───────────────────────────────────────────────────────────────────────────

/// The parts of a `CFF2` table the hinting reads besides the charstrings.
struct Source<'a> {
    store: Option<ItemVariationStore<'a>>,
    axis_count: usize,
    regions: Vec<Vec<RegionAxis>>,
    /// Per region list, its regions (indices into `regions`).
    lists: Vec<Vec<usize>>,
    /// Each Font DICT's Private DICT bytes.
    privates: Vec<&'a [u8]>,
}

fn read_source(data: &[u8]) -> Result<Source<'_>> {
    let cff2 = Cff2::read(FontData::new(data)).map_err(invalid)?;
    let (mut vstore, mut fd_array) = (None, None);
    for entry in entries(cff2.top_dict_data(), None).take(MAX_DICT_ENTRIES) {
        match entry {
            Ok(Entry::VariationStoreOffset(offset)) => vstore = Some(offset),
            Ok(Entry::FdArrayOffset(offset)) => fd_array = Some(offset),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let store = match vstore {
        Some(offset) => {
            // The store follows a 16-bit length.
            let at = offset.checked_add(2).and_then(|o| data.get(o..)).ok_or_else(|| invalid("its variation store lies outside it"))?;
            Some(ItemVariationStore::read(FontData::new(at)).map_err(|e| invalid(format!("its variation store: {e}")))?)
        }
        None => None,
    };
    let (mut axis_count, mut regions, mut lists) = (0usize, Vec::new(), Vec::new());
    if let Some(store) = &store {
        let list = store.variation_region_list().map_err(|e| invalid(format!("its variation regions: {e}")))?;
        axis_count = usize::from(list.axis_count());
        if axis_count > MAX_AXES {
            return Err(invalid(format!("its regions have {axis_count} axes, more than {MAX_AXES}")));
        }
        for region in list.variation_regions().iter().take(MAX_STORE_REGIONS) {
            let region = region.map_err(|e| invalid(format!("a variation region: {e}")))?;
            let axes = region.region_axes();
            regions.push(
                axes.iter()
                    .map(|a| RegionAxis { start: a.start_coord().to_f64(), peak: a.peak_coord().to_f64(), end: a.end_coord().to_f64() })
                    .collect(),
            );
        }
        let data_list = store.item_variation_data();
        for i in 0..usize::from(store.item_variation_data_count()) {
            lists.push(match data_list.get(i) {
                Some(Ok(d)) => d.region_indexes().iter().map(|r| usize::from(r.get())).collect(),
                Some(Err(e)) => return Err(invalid(format!("variation data {i}: {e}"))),
                None => Vec::new(),
            });
        }
    }

    let offset = fd_array.ok_or_else(|| invalid("it has no FDArray"))?;
    let index = data.get(offset..).and_then(|d| Index::new(d, true).ok()).ok_or_else(|| invalid("its FDArray cannot be read"))?;
    let count = index.count() as usize;
    if count == 0 || count > MAX_FONT_DICTS {
        return Err(invalid(format!("the FDArray holds {count} Font DICTs")));
    }
    let mut privates = Vec::with_capacity(count);
    for i in 0..count {
        let dict = index.get(i).ok_or_else(|| invalid(format!("Font DICT {i} cannot be read")))?;
        let range = entries(dict, None).take(MAX_DICT_ENTRIES).flatten().find_map(|e| match e {
            Entry::PrivateDictRange(r) => Some(r),
            _ => None,
        });
        privates.push(match range {
            Some(r) => data.get(r).ok_or_else(|| invalid("a Private DICT lies outside it"))?,
            None => &[][..],
        });
    }
    Ok(Source { store, axis_count, regions, lists, privates })
}

impl Source<'_> {
    /// The Private DICT of Font DICT `fd` with its blends evaluated at `location`.
    fn private_at(&self, fd: usize, location: &[f64]) -> std::result::Result<PrivateDict, String> {
        let bytes = self.privates.get(fd).copied().unwrap_or_default();
        let Some(store) = &self.store else { return Ok(read_private(bytes, None)) };
        let coords: Vec<F2Dot14> = location.iter().map(|v| F2Dot14::from_f64(*v)).collect();
        let state = BlendState::new(store.clone(), &coords, 0).map_err(|e| format!("the variation store cannot be read: {e}"))?;
        Ok(read_private(bytes, Some(state)))
    }

    /// The masters of region list `vsindex`, or why its glyphs cannot be hinted.
    fn masters(&self, vsindex: u16) -> std::result::Result<Masters, String> {
        let origin = vec![0.0; self.axis_count];
        let Some(list) = self.lists.get(usize::from(vsindex)) else {
            return if vsindex == 0 && self.lists.is_empty() {
                Ok(Masters { locations: vec![origin], scalars: vec![Vec::new()], inverse: Vec::new() })
            } else {
                Err(format!("its region list {vsindex} does not exist"))
            };
        };
        if list.len() >= MAX_HINT_MASTERS {
            return Err(format!("its region list has {} regions; at most {} masters are hinted together", list.len(), MAX_HINT_MASTERS));
        }
        let regions: Vec<&Vec<RegionAxis>> = list
            .iter()
            .map(|&r| self.regions.get(r).ok_or_else(|| format!("its region list names region {r}, which does not exist")))
            .collect::<std::result::Result<_, _>>()?;
        let peaks: Vec<Vec<f64>> = regions.iter().map(|r| r.iter().map(|a| a.peak).collect()).collect();
        if peaks.iter().any(|p| p.iter().all(|v| *v == 0.0)) {
            return Err("a region of its list peaks at the default location".into());
        }
        let mut locations = vec![origin];
        locations.extend(peaks);
        let scalars: Vec<Vec<f64>> = locations.iter().map(|l| regions.iter().map(|r| scalar(r, l)).collect()).collect();
        if scalars.first().is_some_and(|s| s.iter().any(|v| *v != 0.0)) {
            return Err("a region of its list does not vanish at the default location".into());
        }
        let at_peaks = scalars.get(1..).unwrap_or_default();
        let inverse = invert(at_peaks).ok_or("two regions of its list peak at the same location")?;
        Ok(Masters { locations, scalars, inverse })
    }
}

/// The scalar of `region` at normalized `location`, as the OpenType item variation store
/// defines it.
fn scalar(region: &[RegionAxis], location: &[f64]) -> f64 {
    let mut s = 1.0;
    for (axis, &v) in region.iter().zip(location) {
        let RegionAxis { start, peak, end } = *axis;
        if start > peak || peak > end || (start < 0.0 && end > 0.0 && peak != 0.0) || peak == 0.0 || v == peak {
            continue;
        }
        if v <= start || v >= end {
            return 0.0;
        }
        s *= if v < peak { (v - start) / (peak - start) } else { (end - v) / (end - peak) };
    }
    s
}

fn cell(a: &[Vec<f64>], r: usize, c: usize) -> f64 {
    a.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0)
}

/// The inverse of the square matrix `m` (rows), or `None` when it is singular.
fn invert(m: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = m.len();
    let mut a: Vec<Vec<f64>> = m
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            r.resize(n, 0.0);
            r.extend((0..n).map(|j| if i == j { 1.0 } else { 0.0 }));
            r
        })
        .collect();
    for col in 0..n {
        let pivot = (col..n).max_by(|&x, &y| cell(&a, x, col).abs().total_cmp(&cell(&a, y, col).abs()))?;
        a.swap(col, pivot);
        let p = cell(&a, col, col);
        if p.abs() < SINGULAR {
            return None;
        }
        let row: Vec<f64> = a.get(col)?.iter().map(|v| v / p).collect();
        for (r, other) in a.iter_mut().enumerate() {
            let f = other.get(col).copied().unwrap_or(0.0);
            if r == col || f == 0.0 {
                continue;
            }
            for (o, v) in other.iter_mut().zip(&row) {
                *o -= f * v;
            }
        }
        *a.get_mut(col)? = row;
    }
    Some(a.into_iter().map(|r| r.get(n..).map(<[f64]>::to_vec).unwrap_or_default()).collect())
}

/// The masters of one region list.
#[derive(Clone, Debug)]
struct Masters {
    /// The default location, then the peak of each region.
    locations: Vec<Vec<f64>>,
    /// Per master, each region's scalar there.
    scalars: Vec<Vec<f64>>,
    /// The inverse of the regions' scalars at the regions' peaks: master values to deltas.
    inverse: Vec<Vec<f64>>,
}

impl Masters {
    /// A number whose value at each master is `values` (one per master), blended over the list.
    fn blend(&self, values: &[f64]) -> Option<Blend> {
        if values.len() != self.locations.len() {
            return None;
        }
        let default = values.first().copied()?;
        let rest: Vec<f64> = values.iter().skip(1).map(|v| v - default).collect();
        if rest.is_empty() {
            return Some(Blend::fixed(default));
        }
        let snap = |v: f64| if (v - v.round()).abs() < DELTA_NOISE { v.round() } else { v };
        let deltas: Vec<f64> = self.inverse.iter().map(|row| snap(row.iter().zip(&rest).map(|(a, b)| a * b).sum())).collect();
        deltas.iter().all(|d| d.is_finite()).then_some(Blend { default, deltas })
    }

    /// The value of a blended number (its default, then one delta per region) at master `m`.
    fn value_at(&self, v: &[f64], m: usize) -> f64 {
        let default = v.first().copied().unwrap_or(0.0);
        let scalars = self.scalars.get(m).map_or(&[][..], Vec::as_slice);
        default + scalars.iter().zip(v.iter().skip(1)).map(|(s, d)| s * d).sum::<f64>()
    }
}

/// The glyph names, from `post`.
fn glyph_names(font: &SfntFont, count: usize) -> Vec<String> {
    let post = font.get(POST).and_then(|d| Post::read(FontData::new(d)).ok());
    (0..count)
        .map(|g| {
            let name = u16::try_from(g).ok().and_then(|g16| post.as_ref()?.glyph_name(GlyphId16::new(g16)));
            match name {
                Some(n) if !n.is_empty() => n.to_string(),
                _ if g == 0 => ".notdef".to_string(),
                _ => format!("glyph{g:05}"),
            }
        })
        .collect()
}

fn y_range(font: &SfntFont) -> (f64, f64) {
    let head = font.get(HEAD);
    let read = |at: usize| head.and_then(|h| h.get(at..at + 2)).and_then(|b| <[u8; 2]>::try_from(b).ok());
    let upm = read(18).map_or(1000.0, |b| f64::from(u16::from_be_bytes(b)));
    let upm = if upm > 0.0 { upm } else { 1000.0 };
    match (read(38), read(42)) {
        (Some(lo), Some(hi)) if i16::from_be_bytes(lo) < i16::from_be_bytes(hi) => {
            (f64::from(i16::from_be_bytes(lo)), f64::from(i16::from_be_bytes(hi)))
        }
        _ => (-0.25 * upm, 1.25 * upm),
    }
}

/// The glyph names, Private DICTs at the default location and vertical extent of the font's
/// `CFF2` table.
pub fn cff2_hint_setup(font: &SfntFont) -> Result<Cff2HintSetup> {
    let data = font.get(CFF2).ok_or_else(|| TablesError::MissingTable("CFF2".into()))?;
    let source = read_source(data)?;
    let origin = vec![0.0; source.axis_count];
    let privates = (0..source.privates.len()).map(|fd| source.private_at(fd, &origin).map_err(invalid)).collect::<Result<Vec<_>>>()?;
    let count = subr::charstrings_of_table(data, true).map_err(from_subr)?.charstrings.len();
    Ok(Cff2HintSetup { names: glyph_names(font, count), privates, y_range: y_range(font) })
}

// ── Autohinting ─────────────────────────────────────────────────────────────────────────────────

/// The font with the selected glyphs of its `CFF2` table hinted by `hint`, their masters together.
/// `select` says which glyphs (by name) to look at.
pub fn autohint_cff2(
    font: &SfntFont,
    setup: &Cff2HintSetup,
    options: &CffAutohintOptions,
    select: &dyn Fn(&str) -> bool,
    hint: &mut Cff2Hinter<'_>,
) -> Result<CffAutohint> {
    let data = font.get(CFF2).ok_or_else(|| TablesError::MissingTable("CFF2".into()))?;
    let source = read_source(data)?;
    let set = subr::charstrings_of_table(data, true).map_err(from_subr)?;
    let mut flat = subr::desubroutinize_within(&set, MAX_CHARSTRING_MEMORY).map_err(from_subr)?;
    let tolerance = if options.round_stems { 0.5 } else { 0.0 };

    let mut models: HashMap<u16, std::result::Result<Masters, String>> = HashMap::new();
    let mut privates: HashMap<(usize, u16), std::result::Result<Vec<PrivateDict>, String>> = HashMap::new();
    let (mut hinted, mut skipped) = (Vec::new(), Vec::new());
    let mut replaced: Vec<(usize, Vec<u8>)> = Vec::new();
    for (gid, code) in flat.charstrings.iter().enumerate() {
        let name = setup.names.get(gid).map_or("", String::as_str);
        if !select(name) {
            continue;
        }
        let skip = |skipped: &mut Vec<CffSkipped>, reason: &str| skipped.push(CffSkipped { glyph: name.to_string(), reason: reason.to_string() });
        let fd = flat.fd_of(gid);
        let default_vsindex = flat.fd_vsindex.get(fd).copied().unwrap_or(0);
        let glyph = match decode(code, default_vsindex, &flat.regions) {
            Ok(g) => g,
            Err(e) => {
                skip(&mut skipped, &format!("its charstring cannot be read: {e}"));
                continue;
            }
        };
        if glyph.path.is_empty() {
            skip(&mut skipped, "the glyph draws nothing");
            continue;
        }
        if glyph.hinted && !options.hint_all {
            skip(&mut skipped, "the glyph already has hints");
            continue;
        }
        let model = match models.entry(glyph.vsindex).or_insert_with(|| source.masters(glyph.vsindex)) {
            Ok(m) => m.clone(),
            Err(e) => {
                skip(&mut skipped, e);
                continue;
            }
        };
        if model.locations.len() != glyph.regions + 1 {
            skip(&mut skipped, "its blends do not match its region list");
            continue;
        }
        let master_privates =
            match privates.entry((fd, glyph.vsindex)).or_insert_with(|| model.locations.iter().map(|l| source.private_at(fd, l)).collect()) {
                Ok(p) => p.clone(),
                Err(e) => {
                    skip(&mut skipped, e);
                    continue;
                }
            };
        let paths: Vec<Vec<PathOp>> = (0..model.locations.len()).map(|m| glyph.path_at(&model, m)).collect();
        let input = Cff2HintGlyph { gid, name, fd, vsindex: glyph.vsindex, locations: &model.locations, privates: &master_privates, paths: &paths };
        let mut list = match hint(&input) {
            Ok(Some(list)) if list.len() == paths.len() && !list.iter().all(CharstringHints::is_empty) => list,
            Ok(Some(_)) | Ok(None) => {
                skip(&mut skipped, "the autohinter found no stems");
                continue;
            }
            Err(e) => {
                skip(&mut skipped, &e);
                continue;
            }
        };
        if options.round_stems {
            list.iter_mut().for_each(round_stems);
        }
        let shared = match shared_hints(&list, tolerance) {
            Ok(s) => s,
            Err(e) => {
                skip(&mut skipped, &format!("its masters' hints differ in structure ({e})"));
                continue;
            }
        };
        let stems = |per_stem: &[Vec<StemHint>]| -> Option<Vec<BlendStem>> {
            per_stem
                .iter()
                .map(|masters| {
                    let edges: Vec<f64> = masters.iter().map(|s| s.edge).collect();
                    let widths: Vec<f64> = masters.iter().map(|s| s.width).collect();
                    Some(BlendStem { edge: model.blend(&edges)?, width: model.blend(&widths)? })
                })
                .collect()
        };
        let (Some(h_stems), Some(v_stems)) = (stems(&shared.h_stems), stems(&shared.v_stems)) else {
            skip(&mut skipped, "its stems cannot be blended over its region list");
            continue;
        };
        let hints = Cff2Hints { h_stems, v_stems, masks: shared.masks, counters: shared.counters };
        let (horizontal, vertical) = (hints.h_stems.len(), hints.v_stems.len());
        let out = Cff2Glyph { name: name.to_string(), vsindex: glyph.vsindex, path: glyph.blend_ops(), hints: Some(hints) };
        // Specialised first; one generic operator per segment when that does not read back the same.
        let encoded = |generic: bool| -> std::result::Result<Vec<u8>, String> {
            match encode_cff2_charstring(&out, glyph.regions, default_vsindex, generic) {
                Ok(Some((bytes, _))) if bytes.len() > MAX_CHARSTRING => Err("the hinted charstring is longer than a charstring may be".into()),
                Ok(Some((_, stack))) if stack > flat.max_stack => {
                    Err(format!("its blended stems need {stack} operands on the stack; the table allows {}", flat.max_stack))
                }
                Ok(Some((bytes, _))) => match decode(&bytes, default_vsindex, &flat.regions) {
                    Ok(back) if back.path == glyph.path && back.vsindex == glyph.vsindex && back.hinted => Ok(bytes),
                    _ => Err("the hinted charstring would draw differently".into()),
                },
                Ok(None) => Err("its blended stems do not fit a charstring's operand stack".into()),
                Err(e) => Err(format!("its hints cannot be written: {e}")),
            }
        };
        let written = encoded(false).or_else(|_| encoded(true));
        match written {
            Ok(bytes) => {
                hinted.push(CffHinted { glyph: name.to_string(), horizontal, vertical });
                replaced.push((gid, bytes));
            }
            Err(e) => skip(&mut skipped, &e),
        }
    }

    let bytes_before = data.len();
    if replaced.is_empty() {
        return Ok(CffAutohint { font: font.clone(), hinted, skipped, bytes_before, bytes_after: bytes_before });
    }
    for (gid, bytes) in replaced {
        if let Some(slot) = flat.charstrings.get_mut(gid) {
            *slot = bytes;
        }
    }
    let table = write_table(data, &flat)?;
    let bytes_after = table.len();
    let mut out = font.clone();
    out.insert(CFF2, table);
    Ok(CffAutohint { font: out, hinted, skipped, bytes_before, bytes_after })
}

// ── Reading a charstring ────────────────────────────────────────────────────────────────────────

/// A blended number: its default, then one delta per region of the glyph's list.
type Val = Vec<f64>;
type Point = (Val, Val);

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Move(Point),
    Line(Point),
    Curve(Point, Point, Point),
}

/// A flat `CFF2` charstring read as a blended outline.
#[derive(Debug, Default, PartialEq)]
struct Decoded {
    /// Absolute coordinates. A moveto with nothing drawn after it is left out, as in the `CFF `
    /// reader.
    path: Vec<Step>,
    hinted: bool,
    vsindex: u16,
    /// The regions of its list: each number has this many deltas.
    regions: usize,
}

impl Decoded {
    fn path_at(&self, model: &Masters, m: usize) -> Vec<PathOp> {
        let p = |(x, y): &Point| (model.value_at(x, m), model.value_at(y, m));
        self.path
            .iter()
            .map(|s| match s {
                Step::Move(a) => PathOp::MoveTo(p(a)),
                Step::Line(a) => PathOp::LineTo(p(a)),
                Step::Curve(a, b, c) => PathOp::CurveTo(p(a), p(b), p(c)),
            })
            .collect()
    }

    fn blend_ops(&self) -> Vec<BlendOp> {
        let b = |v: &Val| Blend { default: v.first().copied().unwrap_or(0.0), deltas: v.iter().skip(1).copied().collect() };
        let p = |(x, y): &Point| -> BlendPoint { (b(x), b(y)) };
        self.path
            .iter()
            .map(|s| match s {
                Step::Move(a) => BlendOp::MoveTo(p(a)),
                Step::Line(a) => BlendOp::LineTo(p(a)),
                Step::Curve(a, b2, c) => BlendOp::CurveTo(p(a), p(b2), p(c)),
            })
            .collect()
    }
}

// Charstring operators (`12 n` as 1200 + n).
const HSTEM: u16 = 1;
const VSTEM: u16 = 3;
const VMOVETO: u16 = 4;
const RLINETO: u16 = 5;
const HLINETO: u16 = 6;
const VLINETO: u16 = 7;
const RRCURVETO: u16 = 8;
const VSINDEX: u16 = 15;
const BLEND: u16 = 16;
const HSTEMHM: u16 = 18;
const HINTMASK: u16 = 19;
const CNTRMASK: u16 = 20;
const RMOVETO: u16 = 21;
const HMOVETO: u16 = 22;
const VSTEMHM: u16 = 23;
const RCURVELINE: u16 = 24;
const RLINECURVE: u16 = 25;
const VVCURVETO: u16 = 26;
const HHCURVETO: u16 = 27;
const VHCURVETO: u16 = 30;
const HVCURVETO: u16 = 31;
const HFLEX: u16 = 1234;
const FLEX: u16 = 1235;
const HFLEX1: u16 = 1236;
const FLEX1: u16 = 1237;

fn add(a: &[f64], b: &[f64]) -> Val {
    a.iter().zip(b).map(|(p, q)| p + q).collect()
}

fn neg(a: &[f64]) -> Val {
    a.iter().map(|v| -v).collect()
}

/// The default of `v` when it does not vary.
fn plain(v: &[f64]) -> Option<f64> {
    if v.iter().skip(1).all(|d| *d == 0.0) { v.first().copied() } else { None }
}

/// The pen a `CFF2` charstring draws with.
struct Pen {
    x: Val,
    y: Val,
    path: Vec<Step>,
}

impl Pen {
    fn new(len: usize) -> Self {
        Pen { x: vec![0.0; len], y: vec![0.0; len], path: Vec::new() }
    }

    fn check(&self) -> std::result::Result<(), String> {
        if self.path.len() >= MAX_SEGMENTS { Err(format!("more than {MAX_SEGMENTS} path segments")) } else { Ok(()) }
    }

    fn move_by(&mut self, dx: &[f64], dy: &[f64]) -> std::result::Result<(), String> {
        self.check()?;
        self.x = add(&self.x, dx);
        self.y = add(&self.y, dy);
        let p = (self.x.clone(), self.y.clone());
        match self.path.last_mut() {
            Some(Step::Move(last)) => *last = p,
            _ => self.path.push(Step::Move(p)),
        }
        Ok(())
    }

    fn line_by(&mut self, dx: &[f64], dy: &[f64]) -> std::result::Result<(), String> {
        self.check()?;
        if self.path.is_empty() {
            return Err("it draws before its first moveto".into());
        }
        self.x = add(&self.x, dx);
        self.y = add(&self.y, dy);
        self.path.push(Step::Line((self.x.clone(), self.y.clone())));
        Ok(())
    }

    fn curve_by(&mut self, d: [&[f64]; 6]) -> std::result::Result<(), String> {
        self.check()?;
        if self.path.is_empty() {
            return Err("it draws before its first moveto".into());
        }
        let [dxa, dya, dxb, dyb, dxc, dyc] = d;
        let a = (add(&self.x, dxa), add(&self.y, dya));
        let b = (add(&a.0, dxb), add(&a.1, dyb));
        let c = (add(&b.0, dxc), add(&b.1, dyc));
        self.x = c.0.clone();
        self.y = c.1.clone();
        self.path.push(Step::Curve(a, b, c));
        Ok(())
    }
}

/// Reads a `CFF2` charstring without subroutine calls. `vsindex` is the region list its Private
/// DICT sets; `regions` holds the number of regions of each list.
fn decode(code: &[u8], vsindex: u16, regions: &[usize]) -> std::result::Result<Decoded, String> {
    let cut = || "the charstring is cut short".to_string();
    let byte = |i: usize| code.get(i).copied().ok_or_else(cut);
    // Every operand carries one value per master, so the region count is capped before any
    // operand is read: a font may claim up to 65,535 regions.
    let count = |vsindex: u16| -> std::result::Result<usize, String> {
        match regions.get(usize::from(vsindex)) {
            Some(&k) if k >= MAX_HINT_MASTERS => {
                Err(format!("its region list has {k} regions; at most {MAX_HINT_MASTERS} masters are hinted together"))
            }
            Some(&k) => Ok(k),
            None if vsindex == 0 && regions.is_empty() => Ok(0),
            None => Err(format!("it uses region list {vsindex}, which does not exist")),
        }
    };
    let mut out = Decoded { vsindex, regions: count(vsindex)?, ..Decoded::default() };
    let mut k = out.regions;
    let mut zero: Val = vec![0.0; k + 1];
    let mut pen = Pen::new(k + 1);
    let mut stack: Vec<Val> = Vec::new();
    let (mut stems, mut operators) = (0usize, 0usize);
    let mut i = 0usize;
    while let Some(&b0) = code.get(i) {
        let value = match b0 {
            28 => {
                i += 3;
                Some(f64::from(i16::from_be_bytes([byte(i - 2)?, byte(i - 1)?])))
            }
            32..=246 => {
                i += 1;
                Some(f64::from(b0) - 139.0)
            }
            247..=250 => {
                i += 2;
                Some((f64::from(b0) - 247.0) * 256.0 + f64::from(byte(i - 1)?) + 108.0)
            }
            251..=254 => {
                i += 2;
                Some(-(f64::from(b0) - 251.0) * 256.0 - f64::from(byte(i - 1)?) - 108.0)
            }
            255 => {
                i += 5;
                Some(f64::from(i32::from_be_bytes([byte(i - 4)?, byte(i - 3)?, byte(i - 2)?, byte(i - 1)?])) / 65536.0)
            }
            _ => None,
        };
        if let Some(v) = value {
            if stack.len() >= MAX_CFF2_STACK {
                return Err("more operands than the stack holds".into());
            }
            let mut val = zero.clone();
            if let Some(first) = val.first_mut() {
                *first = v;
            }
            stack.push(val);
            continue;
        }
        let op = if b0 == 12 {
            i += 2;
            1200 + u16::from(byte(i - 1)?)
        } else {
            i += 1;
            u16::from(b0)
        };
        operators += 1;
        match op {
            VSINDEX => {
                let index = match stack.as_slice() {
                    [v] => plain(v).filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u16::MAX)),
                    _ => None,
                };
                let Some(index) = index.filter(|_| operators == 1) else {
                    return Err("vsindex must come first, with one operand".into());
                };
                out.vsindex = index as u16;
                k = count(out.vsindex)?;
                out.regions = k;
                zero = vec![0.0; k + 1];
                pen = Pen::new(k + 1);
                stack.clear();
                continue;
            }
            BLEND => {
                let n = stack.last().and_then(|v| plain(v)).filter(|n| n.fract() == 0.0 && *n >= 0.0).ok_or("blend needs a count")? as usize;
                let need =
                    n.checked_mul(k + 1).and_then(|v| v.checked_add(1)).filter(|&need| need <= stack.len()).ok_or("blend has too few operands")?;
                let operands = stack.split_off(stack.len() - need);
                for j in 0..n {
                    let mut v = Vec::with_capacity(k + 1);
                    v.push(operands.get(j).and_then(|o| plain(o)).ok_or("a blend operand varies")?);
                    for r in 0..k {
                        v.push(operands.get(n + j * k + r).and_then(|o| plain(o)).ok_or("a blend operand varies")?);
                    }
                    stack.push(v);
                }
                continue;
            }
            _ => {}
        }
        let args = std::mem::take(&mut stack);
        let n = args.len();
        let z: &[f64] = &zero;
        let a = |j: usize| args.get(j).map_or(z, Vec::as_slice);
        match op {
            HSTEM | VSTEM | HSTEMHM | VSTEMHM => {
                stems = stems.saturating_add(n / 2);
                out.hinted = true;
            }
            HINTMASK | CNTRMASK => {
                stems = stems.saturating_add(n / 2);
                out.hinted = true;
                i = i.checked_add(stems.div_ceil(8)).ok_or_else(cut)?;
                if i > code.len() {
                    return Err(cut());
                }
            }
            RMOVETO => pen.move_by(a(0), a(1))?,
            HMOVETO => pen.move_by(a(0), z)?,
            VMOVETO => pen.move_by(z, a(0))?,
            RLINETO => {
                for j in (0..n / 2).map(|j| j * 2) {
                    pen.line_by(a(j), a(j + 1))?;
                }
            }
            HLINETO | VLINETO => {
                let mut horizontal = op == HLINETO;
                for j in 0..n {
                    if horizontal {
                        pen.line_by(a(j), z)?
                    } else {
                        pen.line_by(z, a(j))?
                    }
                    horizontal = !horizontal;
                }
            }
            RRCURVETO => {
                for j in (0..n / 6).map(|j| j * 6) {
                    pen.curve_by([a(j), a(j + 1), a(j + 2), a(j + 3), a(j + 4), a(j + 5)])?;
                }
            }
            RCURVELINE => {
                let curves = n.saturating_sub(2) / 6;
                for j in (0..curves).map(|j| j * 6) {
                    pen.curve_by([a(j), a(j + 1), a(j + 2), a(j + 3), a(j + 4), a(j + 5)])?;
                }
                pen.line_by(a(curves * 6), a(curves * 6 + 1))?;
            }
            RLINECURVE => {
                let lines = n.saturating_sub(6) / 2;
                for j in (0..lines).map(|j| j * 2) {
                    pen.line_by(a(j), a(j + 1))?;
                }
                let c = lines * 2;
                pen.curve_by([a(c), a(c + 1), a(c + 2), a(c + 3), a(c + 4), a(c + 5)])?;
            }
            VVCURVETO | HHCURVETO => {
                let start = n % 2;
                let mut off: Val = if start == 1 { a(0).to_vec() } else { zero.clone() };
                for j in (0..(n - start) / 4).map(|j| start + j * 4) {
                    if op == VVCURVETO {
                        pen.curve_by([off.as_slice(), a(j), a(j + 1), a(j + 2), z, a(j + 3)])?;
                    } else {
                        pen.curve_by([a(j), off.as_slice(), a(j + 1), a(j + 2), a(j + 3), z])?;
                    }
                    off = zero.clone();
                }
            }
            VHCURVETO | HVCURVETO => {
                let mut horizontal = op == HVCURVETO;
                let mut j = 0usize;
                while j + 4 <= n {
                    // A fifth operand on the last curve is its final off-direction delta.
                    let last = if n - j == 5 { a(j + 4) } else { z };
                    if horizontal {
                        pen.curve_by([a(j), z, a(j + 1), a(j + 2), last, a(j + 3)])?;
                    } else {
                        pen.curve_by([z, a(j), a(j + 1), a(j + 2), a(j + 3), last])?;
                    }
                    horizontal = !horizontal;
                    j += 4;
                }
            }
            FLEX => {
                pen.curve_by([a(0), a(1), a(2), a(3), a(4), a(5)])?;
                pen.curve_by([a(6), a(7), a(8), a(9), a(10), a(11)])?;
            }
            HFLEX => {
                let back = neg(a(2));
                pen.curve_by([a(0), z, a(1), a(2), a(3), z])?;
                pen.curve_by([a(4), z, a(5), back.as_slice(), a(6), z])?;
            }
            HFLEX1 => {
                let back = neg(&add(&add(a(1), a(3)), a(7)));
                pen.curve_by([a(0), a(1), a(2), a(3), a(4), z])?;
                pen.curve_by([a(5), z, a(6), a(7), a(8), back.as_slice()])?;
            }
            FLEX1 => {
                // Which way the last point goes depends on the instance when the operands vary.
                if args.iter().any(|v| plain(v).is_none()) {
                    return Err("flex1 with varying operands is not handled here".into());
                }
                let d = |j: usize| plain(a(j)).unwrap_or(0.0);
                let dx = d(0) + d(2) + d(4) + d(6) + d(8);
                let dy = d(1) + d(3) + d(5) + d(7) + d(9);
                let with = |v: f64| {
                    let mut out = zero.clone();
                    if let Some(first) = out.first_mut() {
                        *first = v;
                    }
                    out
                };
                let (dx6, dy6) = if dx.abs() > dy.abs() { (a(10).to_vec(), with(-dy)) } else { (with(-dx), a(10).to_vec()) };
                pen.curve_by([a(0), a(1), a(2), a(3), a(4), a(5)])?;
                pen.curve_by([a(6), a(7), a(8), a(9), dx6.as_slice(), dy6.as_slice()])?;
            }
            other => {
                let name = if other >= 1200 { format!("12 {}", other - 1200) } else { other.to_string() };
                return Err(format!("operator {name} is not handled here"));
            }
        }
    }
    if matches!(pen.path.last(), Some(Step::Move(_))) {
        pen.path.pop();
    }
    out.path = pen.path;
    Ok(out)
}

#[cfg(test)]
#[path = "autohint_cff2_tests.rs"]
mod tests;
