//! Variable TrueType fonts merged as variable fonts (M12.23c).
//!
//! fontTools' merge (4.66.1) has no rule for any variation table: it leaves them out. Every
//! rule here is therefore FontCraft's own, recorded as a guess in the `merge` row:
//!
//! | Table | Rule |
//! |---|---|
//! | `fvar` | the axes must be equal in every font (tag, minimum, default, maximum, in order); the first font's axis records, then the named instances of every font, an instance at coordinates an earlier font already names left out; a later font's instance names are copied into the result's `name` table |
//! | `avar` | must be equal in every font (or absent in all) |
//! | `gvar` | each merged glyph keeps its font's variation data; the shared tuples of all fonts are united and each tuple header renumbered |
//! | `HVAR`, `VVAR` | each font's store keeps the delta sets its merged glyphs use; the stores are concatenated (equal regions shared) and the index maps written again for the merged glyph order; a side-bearing or origin map is kept only when every font has it; the table is left out when a font lacks it |
//! | `GDEF`, `GPOS` stores | the `GDEF` stores are concatenated and each font's variation indices moved to its part ([`LayoutStores`]) |
//! | `MVAR`, `STAT`, `cvar` | the first font's, as `OS/2`, `name` and `cvt ` are the first font's |
//!
//! Static and variable fonts are not merged together, and fonts whose axes differ are refused:
//! leaving `fvar` out (`dropTables`) merges their default outlines instead.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::fvar::Fvar;
use write_fonts::tables::layout::DeviceOrVariationIndex;
use write_fonts::tables::name::Name;
use write_fonts::types::NameId;

use super::plan::Plan;
use crate::reorder::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::reorder::names::num_glyphs;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::subset::var::store::{NO_VARIATION, Store, Subset, read_index_map, write_glyph_map};
use crate::variations::{FvarAxis, fvar_axes};
use crate::{Result, TablesError};

const FVAR: Tag = *b"fvar";
const GVAR: Tag = *b"gvar";

/// The variation tables this module writes, which the merge never reports as dropped.
pub(super) const TABLES: [Tag; 8] = [*b"fvar", *b"avar", *b"gvar", *b"HVAR", *b"VVAR", *b"MVAR", *b"STAT", *b"cvar"];

/// Most shared tuples a `gvar` table can number (12 bits of a tuple index).
const MAX_SHARED_TUPLES: usize = 0x0FFF;
const EMBEDDED_PEAK: u16 = 0x8000;
const INTERMEDIATE: u16 = 0x4000;
const TUPLE_INDEX: u16 = 0x0FFF;

/// Most named instances of the merged `fvar`.
const MAX_INSTANCES: usize = u16::MAX as usize;
/// Highest name ID a copied instance name may take.
const MAX_NAME_ID: u16 = 32_767;

fn describe(axes: &[FvarAxis]) -> String {
    axes.iter().map(|a| format!("{} {}–{}–{}", tag_str(a.tag).trim_end(), a.minimum, a.default, a.maximum)).collect::<Vec<_>>().join(", ")
}

/// The axes the result has: `None` when no font is variable. Refuses a mix of static and
/// variable fonts, fonts whose axes differ and fonts whose `avar` tables differ.
pub(super) fn check(fonts: &[SfntFont]) -> Result<Option<Vec<FvarAxis>>> {
    let Some(first_var) = fonts.iter().position(|f| f.get(FVAR).is_some()) else { return Ok(None) };
    if let Some(stat) = fonts.iter().position(|f| f.get(FVAR).is_none()) {
        return Err(TablesError::BadOptions(format!(
            "font {} is static and font {} is variable: variable fonts merge only with variable fonts of the same axes; \
             leave 'fvar' out with dropTables to merge their default outlines",
            stat + 1,
            first_var + 1
        )));
    }
    let first = fonts.first().ok_or_else(|| TablesError::BadOptions("give fonts to merge".into()))?;
    let axes = fvar_axes(first)?;
    if axes.is_empty() {
        return Err(TablesError::invalid("the 'fvar' table of font 1 has no axes"));
    }
    for (i, font) in fonts.iter().enumerate().skip(1) {
        let other = fvar_axes(font)?;
        if other != axes {
            return Err(TablesError::BadOptions(format!(
                "fonts 1 and {} have different axes (font 1: {}; font {}: {}): variable fonts merge only when their axes are the same; \
                 leave 'fvar' out with dropTables to merge their default outlines",
                i + 1,
                describe(&axes),
                i + 1,
                describe(&other)
            )));
        }
        if font.get(*b"avar") != first.get(*b"avar") {
            return Err(TablesError::BadOptions(format!(
                "fonts 1 and {} have different 'avar' axis mappings: variable fonts merge only when their mappings are the same",
                i + 1
            )));
        }
    }
    Ok(Some(axes))
}

// ── gvar ──

/// One font's `gvar` table, read.
struct Gvar<'a> {
    d: Data<'a>,
    glyph_count: usize,
    long: bool,
    data_at: usize,
}

impl<'a> Gvar<'a> {
    fn read(bytes: &'a [u8], axis_count: usize, index: usize) -> Result<(Self, Vec<&'a [u8]>)> {
        let d = Data::new(GVAR, bytes);
        if usize::from(d.u16(4)?) != axis_count {
            return Err(d.bad(format!("of font {} has {} axes where 'fvar' has {axis_count}", index + 1, d.u16(4)?)));
        }
        let shared_count = usize::from(d.u16(6)?);
        let shared_at = d.u32_usize(8)?;
        let size = 2 * axis_count;
        let shared = (0..shared_count).map(|t| d.slice(shared_at + size * t, size)).collect::<Result<Vec<_>>>()?;
        let glyph_count = usize::from(d.u16(12)?);
        let long = d.u16(14)? & 1 != 0;
        let data_at = d.u32_usize(16)?;
        Ok((Gvar { d, glyph_count, long, data_at }, shared))
    }

    /// The variation data of glyph `gid`, empty when it has none.
    fn glyph(&self, gid: u16) -> Result<&'a [u8]> {
        let g = usize::from(gid);
        if g >= self.glyph_count {
            return Ok(&[]);
        }
        let offset =
            |g: usize| -> Result<usize> { if self.long { self.d.u32_usize(20 + 4 * g) } else { Ok(2 * usize::from(self.d.u16(20 + 2 * g)?)) } };
        let (start, end) = (offset(g)?, offset(g + 1)?);
        let len = end.checked_sub(start).ok_or_else(|| self.d.bad("has decreasing glyph variation offsets"))?;
        let at = self.data_at.checked_add(start).ok_or_else(|| self.d.bad("has an offset past its end"))?;
        self.d.slice(at, len)
    }
}

/// The `gvar` data of every font, with the shared tuples of all fonts united.
pub(super) struct Glyphs<'a> {
    axis_count: usize,
    gvars: Vec<Option<Gvar<'a>>>,
    /// The united shared tuples.
    shared: Vec<&'a [u8]>,
    /// Per font: the united index of each of its shared tuples.
    shared_maps: Vec<Vec<u16>>,
    version: (u16, u16),
}

impl<'a> Glyphs<'a> {
    pub fn read(fonts: &'a [SfntFont], axis_count: usize) -> Result<Self> {
        let mut out = Glyphs { axis_count, gvars: Vec::with_capacity(fonts.len()), shared: Vec::new(), shared_maps: Vec::new(), version: (1, 0) };
        let mut seen: HashMap<&'a [u8], u16> = HashMap::new();
        let mut versioned = false;
        for (i, font) in fonts.iter().enumerate() {
            let Some(bytes) = font.get(GVAR) else {
                out.gvars.push(None);
                out.shared_maps.push(Vec::new());
                continue;
            };
            let (gvar, shared) = Gvar::read(bytes, axis_count, i)?;
            if !versioned {
                out.version = (gvar.d.u16(0)?, gvar.d.u16(2)?);
                versioned = true;
            }
            let mut map = Vec::with_capacity(shared.len());
            for tuple in shared {
                let index = match seen.get(tuple) {
                    Some(at) => *at,
                    None => {
                        if out.shared.len() >= MAX_SHARED_TUPLES {
                            return Err(TablesError::TooLarge { what: "the merged gvar shared tuple count", limit: MAX_SHARED_TUPLES });
                        }
                        let at = to_u16(out.shared.len(), "the merged gvar shared tuple count")?;
                        out.shared.push(tuple);
                        seen.insert(tuple, at);
                        at
                    }
                };
                map.push(index);
            }
            out.gvars.push(Some(gvar));
            out.shared_maps.push(map);
        }
        Ok(out)
    }

    fn any(&self) -> bool {
        self.gvars.iter().any(Option::is_some)
    }

    /// The variation data of glyph `gid` of font `font`, its shared tuple indices moved to the
    /// united list.
    pub fn glyph(&self, font: usize, gid: u16) -> Result<Vec<u8>> {
        let Some(gvar) = self.gvars.get(font).and_then(Option::as_ref) else { return Ok(Vec::new()) };
        let bytes = gvar.glyph(gid)?;
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        let map = self.shared_maps.get(font).map(Vec::as_slice).unwrap_or_default();
        let mut out = bytes.to_vec();
        let d = Data::new(GVAR, bytes);
        let count = usize::from(d.u16(0)? & TUPLE_INDEX);
        let tuple = 2 * self.axis_count;
        let mut at = 4usize;
        for _ in 0..count {
            let index = d.u16(at + 2)?;
            if index & EMBEDDED_PEAK == 0 {
                let shared = map
                    .get(usize::from(index & TUPLE_INDEX))
                    .copied()
                    .ok_or_else(|| d.bad(format!("of font {} names shared tuple {}, which it lacks (glyph {gid})", font + 1, index & TUPLE_INDEX)))?;
                let new = (index & !TUPLE_INDEX) | shared;
                let slot = out.get_mut(at + 2..at + 4).ok_or_else(|| d.bad("ends inside a tuple header"))?;
                slot.copy_from_slice(&new.to_be_bytes());
            }
            at += 4;
            if index & EMBEDDED_PEAK != 0 {
                at += tuple;
            }
            if index & INTERMEDIATE != 0 {
                at += 2 * tuple;
            }
            // The header must lie inside the data.
            d.slice(at.saturating_sub(1), 1)?;
        }
        Ok(out)
    }

    /// Glyph `a` and glyph `b` (font, glyph id) vary alike.
    pub fn same(&self, a: (usize, u16), b: (usize, u16)) -> bool {
        match (self.glyph(a.0, a.1), self.glyph(b.0, b.1)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }

    fn write(&self, plan: &Plan) -> Result<Vec<u8>> {
        let glyphs: Vec<Vec<u8>> = plan.slots.iter().map(|s| self.glyph(s.font, s.gid)).collect::<Result<_>>()?;
        let total: usize = glyphs.iter().map(|g| g.len() + g.len() % 2).sum();
        let long = total > 2 * usize::from(u16::MAX);
        let n = glyphs.len();
        let shared_at = 20 + (n + 1) * if long { 4 } else { 2 };
        let shared_len = self.shared.len() * 2 * self.axis_count;
        let data_at = shared_at + shared_len;
        let mut out = Vec::with_capacity(data_at + total);
        put_u16(&mut out, self.version.0);
        put_u16(&mut out, self.version.1);
        put_u16(&mut out, to_u16(self.axis_count, "the axis count")?);
        put_u16(&mut out, to_u16(self.shared.len(), "the gvar shared tuple count")?);
        put_u32(&mut out, to_u32(shared_at, "the gvar table")?);
        put_u16(&mut out, to_u16(n, "the glyph count")?);
        put_u16(&mut out, u16::from(long));
        put_u32(&mut out, to_u32(data_at, "the gvar table")?);
        let mut at = 0usize;
        for len in glyphs.iter().map(|g| g.len() + g.len() % 2).chain(std::iter::once(0)) {
            if long {
                put_u32(&mut out, to_u32(at, "the gvar table")?);
            } else {
                put_u16(&mut out, to_u16(at / 2, "the gvar table")?);
            }
            at += len;
        }
        for tuple in &self.shared {
            out.extend_from_slice(tuple);
        }
        for g in glyphs {
            let odd = g.len() % 2 == 1;
            out.extend(g);
            if odd {
                out.push(0);
            }
        }
        Ok(out)
    }
}

// ── The tables of the result ──

/// Writes the variation tables of the result into `out`, which already holds the glyph,
/// font-wide and layout tables.
pub(super) fn write(fonts: &[SfntFont], plan: &Plan, glyphs: &Glyphs<'_>, out: &mut SfntFont, warnings: &mut Vec<String>) -> Result<()> {
    let first = fonts.first().ok_or_else(|| TablesError::BadOptions("give fonts to merge".into()))?;
    fvar(fonts, out)?;
    if let Some(avar) = first.get(*b"avar") {
        out.insert(*b"avar", avar.to_vec());
    }
    if glyphs.any() {
        out.insert(GVAR, glyphs.write(plan)?);
    }
    for tag in [*b"HVAR", *b"VVAR"] {
        if tag == *b"VVAR" && out.get(*b"vmtx").is_none() {
            continue;
        }
        if let Some(table) = metrics_variations(fonts, plan, tag, warnings)? {
            out.insert(tag, table);
        }
    }
    for tag in [*b"MVAR", *b"STAT"] {
        if let Some(bytes) = first.get(tag) {
            out.insert(tag, bytes.to_vec());
        }
    }
    if let Some(i) = fonts.iter().position(|f| f.get(*b"MVAR") != first.get(*b"MVAR")) {
        warnings.push(format!("the 'MVAR' table of font {} differs from font 1's: the result keeps font 1's", i + 1));
    }
    if out.get(*b"cvt ").is_some()
        && let Some(cvar) = first.get(*b"cvar")
    {
        out.insert(*b"cvar", cvar.to_vec());
    }
    Ok(())
}

fn fvar_err(i: usize, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the 'fvar' table of font {} cannot be read: {e}", i + 1))
}

fn name_err(i: usize, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the 'name' table of font {} cannot be read: {e}", i + 1))
}

/// The first font's `fvar` with the named instances of the later fonts added.
fn fvar(fonts: &[SfntFont], out: &mut SfntFont) -> Result<()> {
    let mut merged: Option<Fvar> = None;
    // The result's name table, read when a later font's instance needs a name in it.
    let mut name: Option<Name> = None;
    for (i, font) in fonts.iter().enumerate() {
        let Some(bytes) = font.get(FVAR) else { continue };
        let table: Fvar = read_fonts::tables::fvar::Fvar::read(FontData::new(bytes)).map_err(|e| fvar_err(i, e))?.to_owned_table();
        if merged.is_none() {
            merged = Some(table);
            continue;
        }
        let Some(m) = merged.as_mut() else { continue };
        let mut source: Option<Name> = None;
        for mut instance in table.axis_instance_arrays.into_inner().instances {
            if m.axis_instance_arrays.instances.iter().any(|x| x.coordinates == instance.coordinates) {
                continue;
            }
            if m.axis_instance_arrays.instances.len() >= MAX_INSTANCES {
                return Err(TablesError::TooLarge { what: "the merged named instance count", limit: MAX_INSTANCES });
            }
            if let Some(result) = name_table(&mut name, out, 0)? {
                let Some(from) = name_table(&mut source, font, i)? else {
                    m.axis_instance_arrays.instances.push(instance);
                    continue;
                };
                instance.subfamily_name_id = copy_name(result, from, instance.subfamily_name_id)?;
                if let Some(id) = instance.post_script_name_id
                    && id.to_u16() != 0xFFFF
                {
                    instance.post_script_name_id = Some(copy_name(result, from, id)?);
                }
            }
            m.axis_instance_arrays.instances.push(instance);
        }
    }
    let Some(mut merged) = merged else { return Ok(()) };
    // An instance record has a PostScript name ID in every instance or in none.
    if merged.axis_instance_arrays.instances.iter().any(|x| x.post_script_name_id.is_some()) {
        for x in &mut merged.axis_instance_arrays.instances {
            if x.post_script_name_id.is_none() {
                x.post_script_name_id = Some(NameId::new(0xFFFF));
            }
        }
    }
    out.insert(FVAR, write_fonts::dump_table(&merged).map_err(|e| TablesError::invalid(format!("the merged 'fvar' table cannot be written: {e}")))?);
    if let Some(mut name) = name {
        name.name_record.sort();
        out.insert(
            *b"name",
            write_fonts::dump_table(&name).map_err(|e| TablesError::invalid(format!("the merged 'name' table cannot be written: {e}")))?,
        );
    }
    Ok(())
}

/// `slot`, filled from the `name` table of `font` (index `i`) the first time; `None` when the
/// font has none.
fn name_table<'n>(slot: &'n mut Option<Name>, font: &SfntFont, i: usize) -> Result<Option<&'n mut Name>> {
    if slot.is_none() {
        let Some(bytes) = font.get(*b"name") else { return Ok(None) };
        *slot = Some(read_fonts::tables::name::Name::read(FontData::new(bytes)).map_err(|e| name_err(i, e))?.to_owned_table());
    }
    Ok(slot.as_mut())
}

/// The name ID in `result` holding the records `from` has under `id`: an ID with exactly those
/// records, else a new one above 255 they are copied to. `id` itself when `from` has none.
fn copy_name(result: &mut Name, from: &Name, id: NameId) -> Result<NameId> {
    type Key = (u16, u16, u16, String);
    let key = |r: &write_fonts::tables::name::NameRecord| -> Key { (r.platform_id, r.encoding_id, r.language_id, r.string.as_str().to_string()) };
    let mut wanted: Vec<Key> = from.name_record.iter().filter(|r| r.name_id == id).map(key).collect();
    if wanted.is_empty() {
        return Ok(id);
    }
    wanted.sort();
    let mut by_id: BTreeMap<u16, Vec<Key>> = BTreeMap::new();
    for r in &result.name_record {
        by_id.entry(r.name_id.to_u16()).or_default().push(key(r));
    }
    for (existing, records) in &mut by_id {
        records.sort();
        if *records == wanted {
            return Ok(NameId::new(*existing));
        }
    }
    let next = by_id
        .keys()
        .next_back()
        .copied()
        .unwrap_or(0)
        .max(255)
        .checked_add(1)
        .filter(|n| *n <= MAX_NAME_ID)
        .ok_or(TablesError::TooLarge { what: "the name ID of a copied instance name", limit: usize::from(MAX_NAME_ID) })?;
    for r in from.name_record.iter().filter(|r| r.name_id == id) {
        let mut copy = r.clone();
        copy.name_id = NameId::new(next);
        result.name_record.push(copy);
    }
    Ok(NameId::new(next))
}

/// `HVAR` or `VVAR` for the merged glyph order, `None` when no font has it or one lacks it.
fn metrics_variations(fonts: &[SfntFont], plan: &Plan, tag: Tag, warnings: &mut Vec<String>) -> Result<Option<Vec<u8>>> {
    let tables: Vec<Option<&[u8]>> = fonts.iter().map(|f| f.get(tag)).collect();
    if tables.iter().all(Option::is_none) {
        return Ok(None);
    }
    if let Some(i) = tables.iter().position(Option::is_none) {
        warnings.push(format!("the result has no '{}': font {} has none, so its metrics vary only through 'gvar'", tag_str(tag), i + 1));
        return Ok(None);
    }
    // The store, then the advance map, then the side-bearing maps (and VVAR's origin map).
    let map_count = if &tag == b"HVAR" { 3 } else { 4 };
    let mut stores = Vec::with_capacity(fonts.len());
    let mut maps: Vec<Vec<Option<Vec<u32>>>> = Vec::with_capacity(fonts.len());
    let mut version = (1u16, 0u16);
    for (i, (font, bytes)) in fonts.iter().zip(&tables).enumerate() {
        let bytes = bytes.unwrap_or_default();
        let d = Data::new(tag, bytes);
        if i == 0 {
            version = (d.u16(0)?, d.u16(2)?);
        }
        let store_at = d.u32_usize(4)?;
        stores.push(Store::read(tag, bytes.get(store_at..).ok_or_else(|| d.bad("has its variation store past its end"))?)?);
        let n = num_glyphs(font)?;
        let mut font_maps = Vec::with_capacity(map_count);
        for m in 0..map_count {
            let at = d.u32_usize(8 + 4 * m)?;
            font_maps.push(if at == 0 { None } else { Some(read_index_map(d, at, n)?) });
        }
        maps.push(font_maps);
    }
    // The advance map is always written; a side-bearing map only when every font has it.
    let kept: Vec<bool> = (0..map_count).map(|m| m == 0 || maps.iter().all(|f| f.get(m).is_some_and(Option::is_some))).collect();
    let index = |font: usize, m: usize, gid: u16| -> u32 {
        match maps.get(font).and_then(|f| f.get(m)).and_then(Option::as_ref) {
            Some(map) => map.get(usize::from(gid)).copied().unwrap_or(NO_VARIATION),
            // Without an advance map, glyph g's advance is delta set (0, g).
            None if m == 0 => u32::from(gid),
            None => NO_VARIATION,
        }
    };
    let mut parts = Vec::with_capacity(fonts.len());
    let mut moves = Vec::with_capacity(fonts.len());
    for (font, store) in stores.iter().enumerate() {
        let mut used = BTreeSet::new();
        for slot in plan.slots.iter().filter(|s| s.font == font) {
            for m in (0..map_count).filter(|m| kept.get(*m).copied().unwrap_or(false)) {
                used.insert(index(font, m, slot.gid));
            }
        }
        let (subset, map) = store.subset(&used, false, &BTreeSet::new())?;
        parts.push(subset);
        moves.push(map);
    }
    let (store, bases) = Subset::concat(parts)?;
    let mut new_maps: Vec<Option<Vec<u8>>> = Vec::with_capacity(map_count);
    for m in 0..map_count {
        if !kept.get(m).copied().unwrap_or(false) {
            new_maps.push(None);
            continue;
        }
        let mut mapping = Vec::with_capacity(plan.slots.len());
        for slot in &plan.slots {
            let old = index(slot.font, m, slot.gid);
            let new = moves.get(slot.font).and_then(|map| map.get(&old)).copied().unwrap_or(NO_VARIATION);
            mapping.push(moved(new, bases.get(slot.font).copied().unwrap_or(0))?);
        }
        new_maps.push(Some(write_glyph_map(&mapping)?));
    }
    let store_bytes = store.write()?;
    let header = 4 + 4 * (map_count + 1);
    let mut out = Vec::new();
    put_u16(&mut out, version.0);
    put_u16(&mut out, version.1);
    put_u32(&mut out, to_u32(header, "the metrics variations table")?);
    let mut at = header + store_bytes.len();
    for m in &new_maps {
        match m {
            Some(m) => {
                put_u32(&mut out, to_u32(at, "the metrics variations table")?);
                at += m.len();
            }
            None => put_u32(&mut out, 0),
        }
    }
    out.extend(store_bytes);
    for m in new_maps.into_iter().flatten() {
        out.extend(m);
    }
    Ok(Some(out))
}

/// Variation index `v` with its outer index moved up by `base`.
fn moved(v: u32, base: u16) -> Result<u32> {
    if v == NO_VARIATION {
        return Ok(v);
    }
    let outer = u16::try_from(v >> 16)
        .ok()
        .and_then(|o| o.checked_add(base))
        .ok_or(TablesError::TooLarge { what: "the merged item variation data count", limit: usize::from(u16::MAX) })?;
    Ok(u32::from(outer) << 16 | (v & 0xFFFF))
}

// ── GDEF and GPOS ──

/// The `GDEF` variation stores of the fonts, concatenated.
pub(super) struct LayoutStores {
    /// The merged store, `None` when no font has one.
    pub store: Option<write_fonts::tables::variations::ItemVariationStore>,
    /// Per font: the number its outer indices move up by and its subtable count; `None` for
    /// a font without a store.
    moves: Vec<Option<(u16, u16)>>,
}

impl LayoutStores {
    /// The stores of each font's `GDEF` (`None` for a font whose `GDEF` is not merged).
    pub fn read(gdefs: &[Option<&[u8]>]) -> Result<Self> {
        let mut parts = Vec::new();
        let mut counts: Vec<Option<u16>> = Vec::with_capacity(gdefs.len());
        for gdef in gdefs {
            let Some(bytes) = gdef else {
                counts.push(None);
                continue;
            };
            let d = Data::new(*b"GDEF", bytes);
            let at = if d.u16(0)? == 1 && d.u16(2)? >= 3 { d.u32_usize(14)? } else { 0 };
            if at == 0 {
                counts.push(None);
                continue;
            }
            let store = Store::read(*b"GDEF", bytes.get(at..).ok_or_else(|| d.bad("has its variation store past its end"))?)?;
            let whole = store.whole()?;
            counts.push(Some(d.u16(at + 6)?));
            parts.push(whole);
        }
        if parts.is_empty() {
            return Ok(LayoutStores { store: None, moves: vec![None; gdefs.len()] });
        }
        let (merged, bases) = Subset::concat(parts)?;
        let mut bases = bases.into_iter();
        let moves = counts.into_iter().map(|c| c.and_then(|count| bases.next().map(|b| (b, count)))).collect();
        let bytes = merged.write()?;
        let read = read_fonts::tables::variations::ItemVariationStore::read(FontData::new(&bytes))
            .map_err(|e| TablesError::invalid(format!("the merged 'GDEF' variation store cannot be read: {e}")))?;
        Ok(LayoutStores { store: Some(read.to_owned_table()), moves })
    }

    /// Moves a variation index of font `font` to its part of the merged store; one that names
    /// no subtable of its font's store (or a font without one) becomes "no variation".
    pub fn move_index(&self, font: usize, d: &mut DeviceOrVariationIndex) {
        let DeviceOrVariationIndex::VariationIndex(v) = d else { return };
        if v.delta_set_outer_index == 0xFFFF && v.delta_set_inner_index == 0xFFFF {
            return;
        }
        match self.moves.get(font).copied().flatten() {
            Some((base, count)) if v.delta_set_outer_index < count => {
                v.delta_set_outer_index = v.delta_set_outer_index.saturating_add(base);
            }
            _ => {
                v.delta_set_outer_index = 0xFFFF;
                v.delta_set_inner_index = 0xFFFF;
            }
        }
    }
}
