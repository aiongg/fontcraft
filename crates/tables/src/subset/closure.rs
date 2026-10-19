//! The glyph closure of a subset: through `cmap`, the MATH variants
//! ([`super::other::math::close_glyphs`]), the GSUB substitutions of the kept features
//! ([`super::layout::close_glyphs`]), the COLR layers and paints
//! ([`super::other::colr::close_glyphs`]), the variable components of `VARC` with composite
//! glyphs, and CFF `seac`, in that order. Ported from fontTools' `Subsetter._closure_glyphs` (MIT).
//!
//! Every closure is a work list over glyph ids: a glyph is visited at most once, so the work is
//! bounded by the glyph count and the size of the outline tables, whatever the nesting depth.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use read_fonts::ps::encoding::PredefinedEncoding;
use read_fonts::tables::cmap::{Cmap, CmapSubtable, PlatformId};
use read_fonts::{FontData, FontRead};

use super::core::Glyf;
use super::mirror::mirrored;
use super::varc::Varc;
use super::{Plan, SubsetOptions, cs};
use crate::limits::{MAX_CODE_POINTS, MAX_SUBSET_TEXT};
use crate::reorder::cff;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Most glyph names and ids one selection may list.
pub const MAX_SELECTED_GLYPHS: usize = 1 << 20;

/// The plan for `options` on `font`: the selected glyphs and everything they pull in.
pub fn plan(font: &SfntFont, options: &SubsetOptions) -> Result<Plan> {
    if options.text.len() > MAX_SUBSET_TEXT {
        return Err(TablesError::TooLarge { what: "the subset text", limit: MAX_SUBSET_TEXT });
    }
    if options.unicodes.len() > MAX_CODE_POINTS {
        return Err(TablesError::TooLarge { what: "the code point list", limit: MAX_CODE_POINTS });
    }
    if options.glyphs.len() > MAX_SELECTED_GLYPHS || options.gids.len() > MAX_SELECTED_GLYPHS {
        return Err(TablesError::TooLarge { what: "the glyph selection", limit: MAX_SELECTED_GLYPHS });
    }
    let names = crate::reorder::glyph_names(font)?;
    let n = names.len();
    let mut by_name: HashMap<&str, u32> = HashMap::with_capacity(n);
    for (i, name) in names.iter().enumerate() {
        by_name.entry(name.as_str()).or_insert(i as u32);
    }

    let mut plan = Plan { num_glyphs: n, charstring_ops_left: cs::MAX_FONT_CHARSTRING_OPS, options: options.clone(), ..Plan::default() };
    if options.all_glyphs {
        plan.requested.extend(0..n as u32);
    }
    for name in &options.glyphs {
        match by_name.get(name.as_str()) {
            Some(g) => {
                plan.requested.insert(*g);
            }
            None => plan.missing_glyphs.push(name.clone()),
        }
    }
    for g in &options.gids {
        if (*g as usize) < n {
            plan.requested.insert(*g);
        } else {
            plan.missing_glyphs.push(g.to_string());
        }
    }
    if !plan.missing_glyphs.is_empty() && !options.ignore_missing_glyphs {
        return Err(TablesError::BadOptions(format!("the font has no glyph {}", plan.missing_glyphs.join(", "))));
    }

    let cmap = unicode_cmap(font)?;
    plan.unicodes.extend(options.text.chars().map(u32::from));
    plan.unicodes.extend(options.unicodes.iter().copied());
    if options.all_unicodes {
        plan.unicodes.extend(cmap.mapped.keys().copied());
    }
    if options.bidi_closure {
        let mirrors: Vec<u32> = plan.unicodes.iter().filter_map(|u| mirrored(*u)).collect();
        plan.unicodes.extend(mirrors);
    }

    let mut glyphs: BTreeSet<u32> = plan.requested.clone();
    for u in &plan.unicodes {
        glyphs.extend(cmap.glyphs.get(u).into_iter().flatten().copied().filter(|g| (*g as usize) < n));
        if let Some(by_base) = cmap.variations.get(u) {
            for (base, g) in by_base {
                if plan.unicodes.contains(base) && (*g as usize) < n {
                    glyphs.insert(*g);
                }
            }
        }
    }
    plan.missing_unicodes = plan.unicodes.iter().copied().filter(|u| !cmap.mapped.contains_key(u)).collect();
    if !plan.missing_unicodes.is_empty() && !options.ignore_missing_unicodes {
        let list: Vec<String> = plan.missing_unicodes.iter().map(|u| format!("U+{u:04X}")).collect();
        return Err(TablesError::BadOptions(format!("the font maps no glyph to {}", list.join(", "))));
    }

    let truetype = font.tables.contains_key(b"glyf");
    if options.notdef_glyph {
        glyphs.insert(0);
    }
    if options.recommended_glyphs && truetype {
        glyphs.extend(0..n.min(4) as u32);
    }

    plan.pre_math_glyphs = glyphs.clone();
    super::other::math::close_glyphs(font, &mut glyphs, n)?;
    plan.math_glyphs = glyphs.clone();
    plan.features = super::layout::close_glyphs(font, options, &mut glyphs, n)?;
    plan.layout_glyphs = glyphs.clone();
    super::other::colr::close_glyphs(font, &mut glyphs, n)?;
    plan.colr_glyphs = glyphs.clone();

    // pyftsubset closes over VARC before glyf; one work list over both reaches the same glyphs,
    // and also the variable components of a VARC glyph that only a glyf composite names.
    let glyf = Glyf::read(font, n)?;
    let mut varc = Varc::read(font)?;
    if glyf.is_some() || varc.is_some() {
        close(&mut glyphs, n, |g| {
            let mut parts = match varc.as_mut() {
                Some(varc) => varc.component_glyphs(g)?,
                None => Vec::new(),
            };
            if let Some(glyf) = &glyf {
                parts.extend(glyf.components(g as usize)?);
            }
            Ok(parts)
        })?;
    }
    if let Some(bytes) = font.get(*b"CFF ") {
        let table = cff::parse(bytes, n)?;
        let budget = &mut plan.charstring_ops_left;
        close(&mut glyphs, n, |g| {
            let Some(program) = table.charstring(g as usize) else { return Ok(Vec::new()) };
            let (local, global) = table.subrs(g as usize);
            let scan = cs::scan(program, local, global, budget)?;
            Ok(scan
                .seac
                .map(|(base, accent)| {
                    [base, accent].iter().filter_map(|code| by_name.get(PredefinedEncoding::Standard.name(*code)).copied()).collect()
                })
                .unwrap_or_default())
        })?;
    }

    let last = *glyphs.last().ok_or_else(|| TablesError::BadOptions("the subset keeps no glyphs".into()))?;
    plan.new_to_old = if options.retain_gids { (0..=last).collect() } else { glyphs.iter().copied().collect() };
    if options.retain_gids {
        plan.emptied = (0..=last).filter(|g| !glyphs.contains(g)).collect();
    }
    plan.glyph_map = plan.new_to_old.iter().enumerate().map(|(new, old)| (*old, new as u32)).collect::<BTreeMap<_, _>>();
    plan.glyphs = glyphs;
    Ok(plan)
}

/// Adds to `glyphs` everything `parts` reaches from them, each glyph visited once. A part past
/// the `n` glyphs of the font is an error.
fn close(glyphs: &mut BTreeSet<u32>, n: usize, mut parts: impl FnMut(u32) -> Result<Vec<u32>>) -> Result<()> {
    let mut todo: Vec<u32> = glyphs.iter().copied().collect();
    while let Some(g) = todo.pop() {
        for part in parts(g)? {
            if part as usize >= n {
                return Err(TablesError::invalid(format!("glyph {g} uses glyph {part}, which is past the glyph count {n}")));
            }
            if glyphs.insert(part) {
                todo.push(part);
            }
        }
    }
    Ok(())
}

/// The Unicode subtables of `cmap`, merged.
#[derive(Default)]
struct UnicodeCmap {
    /// Code point → glyph, from the first subtable that maps it.
    mapped: BTreeMap<u32, u32>,
    /// Code point → every glyph a Unicode subtable maps it to.
    glyphs: BTreeMap<u32, BTreeSet<u32>>,
    /// Variation selector → (base code point, glyph) of the non-default variation sequences.
    variations: BTreeMap<u32, Vec<(u32, u32)>>,
}

/// A subtable the subsetter treats as Unicode, as fontTools' `isUnicode` decides.
pub(crate) fn is_unicode(platform: PlatformId, encoding: u16) -> bool {
    platform == PlatformId::Unicode || (platform == PlatformId::Windows && matches!(encoding, 1 | 10))
}

fn unicode_cmap(font: &SfntFont) -> Result<UnicodeCmap> {
    let mut out = UnicodeCmap::default();
    let Some(bytes) = font.get(*b"cmap") else { return Ok(out) };
    let cmap = Cmap::read(FontData::new(bytes)).map_err(|e| TablesError::invalid(format!("the 'cmap' table cannot be read: {e}")))?;
    let mut budget = MAX_CODE_POINTS.saturating_mul(4);
    for record in cmap.encoding_records() {
        if !is_unicode(record.platform_id(), record.encoding_id()) {
            continue;
        }
        let sub = record.subtable(cmap.offset_data()).map_err(|e| TablesError::invalid(format!("a 'cmap' subtable cannot be read: {e}")))?;
        match &sub {
            CmapSubtable::Format0(_) => {}
            CmapSubtable::Format14(t) => {
                for selector in t.var_selector() {
                    let vs = selector.var_selector().to_u32();
                    let Some(Ok(non_default)) = selector.non_default_uvs(t.offset_data()) else { continue };
                    for m in non_default.uvs_mapping() {
                        budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_CODE_POINTS })?;
                        out.variations.entry(vs).or_default().push((m.unicode_value().to_u32(), u32::from(m.glyph_id())));
                    }
                }
            }
            _ => {
                for (cp, gid) in sub.iter() {
                    budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_CODE_POINTS })?;
                    if gid.to_u32() == 0 {
                        continue;
                    }
                    out.mapped.entry(cp).or_insert(gid.to_u32());
                    out.glyphs.entry(cp).or_default().insert(gid.to_u32());
                }
            }
        }
    }
    Ok(out)
}
