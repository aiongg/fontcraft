//! `HVAR` and `VVAR` from the phantom points of `gvar` (fontTools `varLib.hvar`).
//!
//! For each glyph and each of its `gvar` tuples, the advance delta is the right phantom point's
//! x delta minus the left one's (`VVAR`: the top phantom point's y delta minus the bottom one's);
//! a phantom point the tuple leaves out moves by 0, as fontTools reads it. The deltas go, per
//! glyph and region, to the compiler's `HVAR`/`VVAR` builder, which lays out the store as it does
//! for a variable export. fontTools 4.66 fails on fonts with `vmtx` (its `add_VVAR` reads a name
//! before setting it), so `VVAR` has no oracle.

use std::collections::{HashMap, HashSet};

use fontcraft_compile::var::hvar::{build_hvar, build_vvar};
use fontcraft_compile::var::{Region, RegionAxis, VarValue};
use read_fonts::tables::glyf::Glyph;
use read_fonts::tables::gvar::Gvar;
use read_fonts::types::{GlyphId, Tag as RTag};
use read_fonts::{FontData, FontRead, TableProvider};

use crate::limits::MAX_TUPLE_VARIATIONS;
use crate::sfnt::SfntFont;
use crate::variations::fvar_axes;
use crate::{Result, TablesError};

/// The most distinct regions an item variation store can list (its region count is 16-bit).
const MAX_STORE_REGIONS: usize = 65_535;
/// The most `gvar` deltas read for one build, over every glyph and tuple. Tuples that share
/// their points and peaks cost a few bytes each but make every point's delta be read again.
const MAX_HVAR_DELTAS: usize = 1 << 30;
/// The most region axes held in the advance deltas of all glyphs together, horizontal and
/// vertical: 24 bytes each.
const MAX_HVAR_REGION_AXES: usize = 1 << 24;

/// A region's coordinates as bits, for hashing; they come from F2Dot14 values, so equal bits
/// mean equal regions (`+ 0.0` folds −0 into 0).
fn region_key(region: &Region) -> Vec<[u64; 3]> {
    region.0.iter().map(|a| [a.start, a.peak, a.end].map(|x| (x + 0.0).to_bits())).collect()
}

/// The work [`build_hvar_with`] has left.
struct Budget {
    deltas: usize,
    region_axes: usize,
}

/// What [`build_hvar_with`] builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HvarOptions {
    /// Also build `VVAR` when the font has a `vmtx` table.
    pub vvar: bool,
    /// Rebuild a table the font already has; without it the old table is kept.
    pub replace: bool,
}

impl Default for HvarOptions {
    fn default() -> Self {
        Self { vvar: true, replace: false }
    }
}

/// What happened to one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableAction {
    Built,
    /// The font had the table and `replace` was off.
    Kept,
    /// Not asked for, or (`VVAR`) the font has no vertical metrics.
    Skipped,
}

impl TableAction {
    pub fn as_str(self) -> &'static str {
        match self {
            TableAction::Built => "built",
            TableAction::Kept => "kept",
            TableAction::Skipped => "skipped",
        }
    }
}

/// The result of [`build_hvar_with`].
#[derive(Clone, Debug, PartialEq)]
pub struct HvarBuild {
    pub font: SfntFont,
    pub hvar: TableAction,
    pub vvar: TableAction,
    /// Glyphs whose horizontal advance varies.
    pub varied_glyphs: usize,
    /// Distinct regions among the advance deltas.
    pub regions: usize,
}

struct Provider<'a>(&'a SfntFont);

impl<'a> TableProvider<'a> for Provider<'a> {
    fn data_for_tag(&self, tag: RTag) -> Option<FontData<'a>> {
        self.0.get(tag.into_bytes()).map(FontData::new)
    }
}

fn write_error(e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the advance variation table cannot be written: {e}"))
}

fn read_error(what: &str) -> impl Fn(read_fonts::ReadError) -> TablesError + '_ {
    move |e| TablesError::invalid(format!("the {what} cannot be read: {e}"))
}

/// The font with `HVAR` (and `VVAR`) built from its `gvar` phantom points.
pub fn build_hvar_with(font: &SfntFont, options: &HvarOptions) -> Result<HvarBuild> {
    let axes = fvar_axes(font)?;
    if axes.is_empty() {
        return Err(TablesError::BadOptions("the font has no variation axes".into()));
    }
    let axis_count = u16::try_from(axes.len()).map_err(|_| TablesError::TooLarge { what: "the axis count", limit: usize::from(u16::MAX) })?;
    let gvar_bytes = font
        .get(*b"gvar")
        .ok_or_else(|| TablesError::Unsupported("building HVAR without a gvar table: the advance deltas come from the gvar phantom points".into()))?;
    let want_hvar = options.replace || font.get(*b"HVAR").is_none();
    let has_vmtx = font.get(*b"vmtx").is_some();
    let vvar_action = if !options.vvar || !has_vmtx {
        TableAction::Skipped
    } else if options.replace || font.get(*b"VVAR").is_none() {
        TableAction::Built
    } else {
        TableAction::Kept
    };
    if !want_hvar && vvar_action != TableAction::Built {
        return Err(TablesError::BadOptions("the font already has its advance variation tables; rebuild them with `replace`".into()));
    }

    let p = Provider(font);
    let count = crate::ttx::num_glyphs(font)?;
    let loca = p.loca(None).map_err(read_error("loca table"))?;
    let glyf = p.glyf().map_err(read_error("glyf table"))?;
    let gvar = Gvar::read(FontData::new(gvar_bytes)).map_err(read_error("gvar table"))?;
    if gvar.axis_count() != axis_count {
        return Err(TablesError::invalid(format!("the gvar table has {} axes and the fvar table {axis_count}", gvar.axis_count())));
    }
    let hmtx = p.hmtx().map_err(read_error("hmtx table"))?;
    let vmtx = if vvar_action == TableAction::Built { Some(p.vmtx().map_err(read_error("vmtx table"))?) } else { None };

    let mut horizontal = Vec::with_capacity(count);
    let mut vertical = Vec::with_capacity(if vmtx.is_some() { count } else { 0 });
    let mut regions: HashSet<Vec<[u64; 3]>> = HashSet::new();
    let mut budget = Budget { deltas: MAX_HVAR_DELTAS, region_axes: MAX_HVAR_REGION_AXES };
    for g in 0..count {
        let gid = GlyphId::new(u32::try_from(g).unwrap_or(u32::MAX));
        let points = match loca.get_glyf(gid, &glyf).map_err(|e| TablesError::invalid(format!("glyph {g} cannot be read: {e}")))? {
            None => 0,
            Some(Glyph::Simple(s)) => s.num_points(),
            Some(Glyph::Composite(c)) => c.components().count(),
        };
        let (h, v) = phantom_deltas(&gvar, gid, points, usize::from(axis_count), &mut budget)?;
        let held = h.len().saturating_add(if vmtx.is_some() { v.len() } else { 0 }).saturating_mul(usize::from(axis_count));
        budget.region_axes = budget
            .region_axes
            .checked_sub(held)
            .ok_or(TablesError::TooLarge { what: "the region axes of the advance deltas", limit: MAX_HVAR_REGION_AXES })?;
        for (r, _) in &h {
            regions.insert(region_key(r));
        }
        if regions.len() > MAX_STORE_REGIONS {
            return Err(TablesError::TooLarge { what: "the distinct regions of the advance deltas", limit: MAX_STORE_REGIONS });
        }
        horizontal.push(VarValue { default: i32::from(hmtx.advance(gid).unwrap_or(0)), deltas: h });
        if let Some(vmtx) = &vmtx {
            vertical.push(VarValue { default: i32::from(vmtx.advance(gid).unwrap_or(0)), deltas: v });
        }
    }

    let compile_error = |e: fontcraft_compile::CompileError| TablesError::invalid(e.to_string());
    let mut out = font.clone();
    let hvar_action = if want_hvar {
        let table = build_hvar(axis_count, &horizontal).map_err(compile_error)?;
        out.insert(*b"HVAR", write_fonts::dump_table(&table).map_err(write_error)?);
        TableAction::Built
    } else {
        TableAction::Kept
    };
    if vvar_action == TableAction::Built {
        let table = build_vvar(axis_count, &vertical, None).map_err(compile_error)?;
        out.insert(*b"VVAR", write_fonts::dump_table(&table).map_err(write_error)?);
    }
    Ok(HvarBuild {
        font: out,
        hvar: hvar_action,
        vvar: vvar_action,
        varied_glyphs: horizontal.iter().filter(|v| !v.deltas.is_empty()).count(),
        regions: regions.len(),
    })
}

/// One delta per region.
type RegionDeltas = Vec<(Region, i32)>;

/// The horizontal and vertical advance deltas of glyph `gid` (with `points` outline points or
/// components), one per distinct region of its tuples, zeros left out.
fn phantom_deltas(gvar: &Gvar<'_>, gid: GlyphId, points: usize, axis_count: usize, budget: &mut Budget) -> Result<(RegionDeltas, RegionDeltas)> {
    let mut h: Vec<(Region, i32)> = Vec::new();
    let mut v: Vec<(Region, i32)> = Vec::new();
    // Each region's index in `h` and `v`, which list the regions in the same order.
    let mut index: HashMap<Vec<[u64; 3]>, usize> = HashMap::new();
    let Some(data) =
        gvar.glyph_variation_data(gid).map_err(|e| TablesError::invalid(format!("the gvar data of glyph {} cannot be read: {e}", gid.to_u32())))?
    else {
        return Ok((h, v));
    };
    let f2 = |bits: i16| f64::from(bits) / 16384.0;
    for (count, tuple) in data.tuples().enumerate() {
        if count >= MAX_TUPLE_VARIATIONS {
            return Err(TablesError::TooLarge { what: "the tuple variations of one glyph", limit: MAX_TUPLE_VARIATIONS });
        }
        let peak = tuple.peak();
        let start = tuple.intermediate_start();
        let end = tuple.intermediate_end();
        let region = Region(
            (0..axis_count)
                .map(|i| {
                    let p = peak.get(i).map_or(0.0, |x| f2(x.to_bits()));
                    if p == 0.0 {
                        return RegionAxis::default();
                    }
                    let s = start.as_ref().and_then(|t| t.get(i)).map_or(p.min(0.0), |x| f2(x.to_bits()));
                    let e = end.as_ref().and_then(|t| t.get(i)).map_or(p.max(0.0), |x| f2(x.to_bits()));
                    RegionAxis { start: s, peak: p, end: e }
                })
                .collect(),
        );
        // Left, right, top and bottom phantom points.
        let mut phantom = [(0i32, 0i32); 4];
        for d in tuple.deltas() {
            budget.deltas = budget
                .deltas
                .checked_sub(1)
                .ok_or(TablesError::TooLarge { what: "the gvar deltas read for the advances", limit: MAX_HVAR_DELTAS })?;
            let at = usize::from(d.position);
            if let Some(slot) = at.checked_sub(points).and_then(|k| phantom.get_mut(k)) {
                *slot = (d.x_delta, d.y_delta);
            }
        }
        let [left, right, top, bottom] = phantom;
        let (dh, dv) = (right.0 - left.0, top.1 - bottom.1);
        let key = region_key(&region);
        match index.get(&key).copied() {
            // Two tuples of a glyph with one region add up.
            Some(i) => {
                if let Some(slot) = h.get_mut(i) {
                    slot.1 = slot.1.saturating_add(dh);
                }
                if let Some(slot) = v.get_mut(i) {
                    slot.1 = slot.1.saturating_add(dv);
                }
            }
            None => {
                index.insert(key, h.len());
                h.push((region.clone(), dh));
                v.push((region, dv));
            }
        }
    }
    h.retain(|(_, d)| *d != 0);
    v.retain(|(_, d)| *d != 0);
    Ok((h, v))
}
