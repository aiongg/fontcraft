//! `BASE` and `MATH` merged, after fontTools' `merge/layout.py` (MIT License, Copyright 2013
//! Google, Inc.; the `MATH` rules from fontTools 4.66.2; see `NOTICE`).
//!
//! `MATH`: the constants and the minimum connector overlap are the first font's; the italic
//! corrections, top accent attachments, extended shapes, math kerning and glyph constructions of
//! every font are united, each keyed by its glyph.
//!
//! `BASE`: per axis, the baseline tag list is shared and the script records of every font are
//! united in tag order. Where fontTools concatenates equal baseline tag lists (which leaves more
//! tags than each script has coordinates), the list is kept once; fonts whose lists differ are
//! refused. A script several fonts have keeps the first font's record, where fontTools keeps
//! them all.
//!
//! Each table is moved to the merged glyph ids first, which needs every glyph of its font in the
//! result. Tables with variation data are refused, as fontTools cannot merge them.

use std::collections::BTreeSet;

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::base::{Axis, Base, BaseCoord, BaseScriptList, BaseTagList, MinMax};
use write_fonts::tables::layout::CoverageTable;
use write_fonts::tables::math::{
    MathConstants, MathGlyphConstruction, MathGlyphInfo, MathItalicsCorrectionInfo, MathKernInfo, MathKernInfoRecord, MathTopAccentAttachment,
    MathValueRecord, MathVariants,
};
use write_fonts::types::{GlyphId16, UfWord};

use super::layout::{coverage, coverage_glyphs, order_for};
use super::plan::Plan;
use crate::limits::MAX_LAYOUT_ITEMS;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

const BASE: Tag = *b"BASE";
const MATH: Tag = *b"MATH";

fn read_err(tag: Tag, font: usize, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the '{}' table of font {} cannot be read: {e}", tag_str(tag), font + 1))
}

fn write_err(tag: Tag, e: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("the merged '{}' table cannot be written: {e}", tag_str(tag)))
}

/// `BASE` and `MATH` of the fonts numbered in `which` (all of them, or the first alone), merged
/// into `out`. Tables in `skip` are left alone.
pub(super) fn merge(fonts: &[SfntFont], which: &[usize], plan: &Plan, skip: &BTreeSet<Tag>, out: &mut SfntFont) -> Result<()> {
    let present = |tag: Tag| -> Vec<(usize, &[u8])> {
        if skip.contains(&tag) {
            return Vec::new();
        }
        which.iter().filter_map(|&i| fonts.get(i).and_then(|f| f.get(tag)).map(|b| (i, b))).collect()
    };
    let maths = present(MATH);
    if !maths.is_empty() {
        let mut parts = MathParts::default();
        for (i, bytes) in maths {
            order_for(plan, i)?;
            crate::layout_budget::check_math(bytes)?;
            let table: write_fonts::tables::math::Math =
                read_fonts::tables::math::Math::read(FontData::new(bytes)).map_err(|e| read_err(MATH, i, e))?.to_owned_table();
            parts.add(&table, &|g| new_id(plan, i, g))?;
        }
        out.insert(MATH, write_fonts::dump_table(&parts.finish()?).map_err(|e| write_err(MATH, e))?);
    }
    let bases = present(BASE);
    if !bases.is_empty() {
        let mut horiz: Vec<(usize, Axis)> = Vec::new();
        let mut vert: Vec<(usize, Axis)> = Vec::new();
        for (i, bytes) in bases {
            order_for(plan, i)?;
            crate::layout_budget::check_base(bytes)?;
            let mut table: Base = read_fonts::tables::base::Base::read(FontData::new(bytes)).map_err(|e| read_err(BASE, i, e))?.to_owned_table();
            if table.item_var_store.is_some() {
                return Err(TablesError::Unsupported(format!("merging a 'BASE' table with variation data (font {})", i + 1)));
            }
            let map = |g: u16| new_id(plan, i, g);
            for (axis, list) in [(table.horiz_axis.as_mut(), &mut horiz), (table.vert_axis.as_mut(), &mut vert)] {
                if let Some(axis) = axis {
                    remap_axis(axis, &map)?;
                    list.push((i, axis.clone()));
                }
            }
        }
        let merged = Base::new(merge_axis(horiz)?, merge_axis(vert)?);
        out.insert(BASE, write_fonts::dump_table(&merged).map_err(|e| write_err(BASE, e))?);
    }
    Ok(())
}

fn new_id(plan: &Plan, font: usize, gid: u16) -> Result<u16> {
    plan.new_id(font, gid).ok_or_else(|| TablesError::invalid(format!("a glyph id ({gid}) of font {} is past its glyphs", font + 1)))
}

type Map<'a> = dyn Fn(u16) -> Result<u16> + 'a;

/// Items keyed by glyph, as one coverage table and the items in its order. Glyphs come out in
/// id order; a glyph listed twice keeps its first item.
fn covered<T>(mut items: Vec<(u16, T)>) -> Result<(CoverageTable, Vec<T>)> {
    if items.len() > MAX_LAYOUT_ITEMS {
        return Err(TablesError::TooLarge { what: "the glyphs of a merged coverage table", limit: MAX_LAYOUT_ITEMS });
    }
    items.sort_by_key(|(g, _)| *g);
    items.dedup_by_key(|(g, _)| *g);
    let glyphs: Vec<GlyphId16> = items.iter().map(|(g, _)| GlyphId16::new(*g)).collect();
    Ok((coverage(&glyphs), items.into_iter().map(|(_, t)| t).collect()))
}

/// The glyphs of a coverage table paired with `values`, moved by `map`.
fn keyed<T: Clone>(cov: &CoverageTable, values: &[T], map: &Map<'_>) -> Result<Vec<(u16, T)>> {
    coverage_glyphs(cov).iter().zip(values).map(|(g, v)| Ok((map(g.to_u16())?, v.clone()))).collect()
}

/// The `MATH` subtables of the fonts seen so far; `None` where no font had the subtable.
#[derive(Default)]
struct MathParts {
    constants: Option<MathConstants>,
    italics: Option<Vec<(u16, MathValueRecord)>>,
    accents: Option<Vec<(u16, MathValueRecord)>>,
    extended: Option<Vec<(u16, ())>>,
    kerns: Option<Vec<(u16, MathKernInfoRecord)>>,
    overlap: Option<u16>,
    vert: Option<Vec<(u16, MathGlyphConstruction)>>,
    horiz: Option<Vec<(u16, MathGlyphConstruction)>>,
}

fn extend<T>(slot: &mut Option<Vec<T>>, items: Vec<T>) {
    slot.get_or_insert_with(Vec::new).extend(items);
}

impl MathParts {
    fn add(&mut self, table: &write_fonts::tables::math::Math, map: &Map<'_>) -> Result<()> {
        if self.constants.is_none() {
            self.constants = Some((*table.math_constants).clone());
        }
        let info = &*table.math_glyph_info;
        if let Some(ic) = info.math_italics_correction_info.as_ref() {
            extend(&mut self.italics, keyed(&ic.coverage, &ic.italics_correction, map)?);
        }
        if let Some(ta) = info.math_top_accent_attachment.as_ref() {
            extend(&mut self.accents, keyed(&ta.top_accent_coverage, &ta.top_accent_attachment, map)?);
        }
        if let Some(cov) = info.extended_shape_coverage.as_ref() {
            let units = vec![(); coverage_glyphs(cov).len()];
            extend(&mut self.extended, keyed(cov, &units, map)?);
        }
        if let Some(k) = info.math_kern_info.as_ref() {
            extend(&mut self.kerns, keyed(&k.math_kern_coverage, &k.math_kern_info_records, map)?);
        }
        let variants = &*table.math_variants;
        if self.overlap.is_none() {
            self.overlap = Some(variants.min_connector_overlap.to_u16());
        }
        for (cov, constructions, slot) in [
            (variants.vert_glyph_coverage.as_ref(), &variants.vert_glyph_constructions, &mut self.vert),
            (variants.horiz_glyph_coverage.as_ref(), &variants.horiz_glyph_constructions, &mut self.horiz),
        ] {
            let Some(cov) = cov else { continue };
            let mut moved: Vec<MathGlyphConstruction> = Vec::with_capacity(constructions.len());
            for c in constructions {
                let mut c: MathGlyphConstruction = (**c).clone();
                for r in &mut c.math_glyph_variant_records {
                    r.variant_glyph = GlyphId16::new(map(r.variant_glyph.to_u16())?);
                }
                if let Some(assembly) = c.glyph_assembly.as_mut() {
                    for p in &mut assembly.part_records {
                        p.glyph_id = GlyphId16::new(map(p.glyph_id.to_u16())?);
                    }
                }
                moved.push(c);
            }
            extend(slot, keyed(cov, &moved, map)?);
        }
        Ok(())
    }

    fn finish(self) -> Result<write_fonts::tables::math::Math> {
        let constants = self.constants.ok_or_else(|| TablesError::invalid("no font has a 'MATH' table"))?;
        let italics = self.italics.map(covered).transpose()?.map(|(c, v)| MathItalicsCorrectionInfo::new(c, v));
        let accents = self.accents.map(covered).transpose()?.map(|(c, v)| MathTopAccentAttachment::new(c, v));
        let extended = self.extended.map(covered).transpose()?.map(|(c, _)| c);
        let kerns = self.kerns.map(covered).transpose()?.map(|(c, v)| MathKernInfo::new(c, v));
        let (vert_cov, vert) = match self.vert.map(covered).transpose()? {
            Some((c, v)) => (Some(c), v),
            None => (None, Vec::new()),
        };
        let (horiz_cov, horiz) = match self.horiz.map(covered).transpose()? {
            Some((c, v)) => (Some(c), v),
            None => (None, Vec::new()),
        };
        let variants = MathVariants::new(UfWord::new(self.overlap.unwrap_or(0)), vert_cov, horiz_cov, vert, horiz);
        Ok(write_fonts::tables::math::Math::new(constants, MathGlyphInfo::new(italics, accents, extended, kerns), variants))
    }
}

fn remap_coord(c: &mut BaseCoord, map: &Map<'_>) -> Result<()> {
    if let BaseCoord::Format2(f) = c {
        f.reference_glyph = map(f.reference_glyph)?;
    }
    Ok(())
}

fn remap_min_max(m: &mut MinMax, map: &Map<'_>) -> Result<()> {
    for c in [m.min_coord.as_mut(), m.max_coord.as_mut()].into_iter().flatten() {
        remap_coord(c, map)?;
    }
    for r in &mut m.feat_min_max_records {
        for c in [r.min_coord.as_mut(), r.max_coord.as_mut()].into_iter().flatten() {
            remap_coord(c, map)?;
        }
    }
    Ok(())
}

/// Moves the reference glyphs of an axis's coordinates to the merged glyph ids.
fn remap_axis(axis: &mut Axis, map: &Map<'_>) -> Result<()> {
    for record in &mut axis.base_script_list.base_script_records {
        let script = &mut *record.base_script;
        if let Some(values) = script.base_values.as_mut() {
            for c in &mut values.base_coords {
                remap_coord(c, map)?;
            }
        }
        if let Some(m) = script.default_min_max.as_mut() {
            remap_min_max(m, map)?;
        }
        for lang in &mut script.base_lang_sys_records {
            remap_min_max(&mut lang.min_max, map)?;
        }
    }
    Ok(())
}

/// One axis from the fonts' axes (font index, axis).
fn merge_axis(axes: Vec<(usize, Axis)>) -> Result<Option<Axis>> {
    if axes.is_empty() {
        return Ok(None);
    }
    let lists: Vec<(usize, &Vec<write_fonts::types::Tag>)> =
        axes.iter().filter_map(|(i, a)| a.base_tag_list.as_ref().map(|l| (*i, &l.baseline_tags))).collect();
    let tags = match lists.first() {
        None => None,
        Some((_, first)) => {
            if lists.iter().any(|(_, l)| l != first) {
                let fonts: Vec<String> = lists.iter().map(|(i, _)| (i + 1).to_string()).collect();
                return Err(TablesError::Unsupported(format!(
                    "merging 'BASE' tables that list different baselines (fonts {}): leave 'BASE' out with dropTables, or keep the first font's with layout base",
                    fonts.join(", ")
                )));
            }
            Some(BaseTagList::new((*first).clone()))
        }
    };
    let mut records = Vec::new();
    for (_, axis) in &axes {
        records.extend(axis.base_script_list.base_script_records.iter().cloned());
    }
    if records.len() > MAX_LAYOUT_ITEMS {
        return Err(TablesError::TooLarge { what: "the merged 'BASE' script records", limit: MAX_LAYOUT_ITEMS });
    }
    records.sort_by_key(|r| r.base_script_tag);
    records.dedup_by_key(|r| r.base_script_tag);
    Ok(Some(Axis::new(tags, BaseScriptList::new(records))))
}
