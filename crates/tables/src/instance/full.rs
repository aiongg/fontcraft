//! Full static instances (M12.20): gvar with inferred deltas, cvar, HVAR, VVAR, MVAR, the GDEF
//! and GPOS variation data, feature variations, `STAT` and `name` reduced, `OS/2` and `post`
//! updated. A port of the full-instance path of fontTools' `varLib.instancer` (MIT, Copyright
//! (c) 2019 Google LLC and the fontTools authors).
//!
//! The steps, in fontTools' order:
//!
//! 1. The location is clamped to each axis's range, normalised and mapped through `avar`
//!    ([`super::pin_location`]); axes left out sit at their default.
//! 2. `glyf`, `hmtx` and `vmtx` from `gvar` (see `glyf.rs`), `cvt ` from `cvar`, the metrics
//!    `MVAR` names, then `HVAR` and `VVAR` dropped (the phantom points already moved the
//!    advances), then `GDEF`/`GPOS` deltas and feature variations (see `otl.rs`).
//! 3. `STAT` keeps the axis values of the location; `fvar`, `avar` and the variation tables go;
//!    `name` records only they used go.
//! 4. `OS/2.xAvgCharWidth` from the new advances; `usWeightClass`, `usWidthClass` and
//!    `post.italicAngle` from the `wght`, `wdth` and `slnt` values.
//! 5. The overlap flags on every glyph, when asked; `head`, `hhea` and `vhea` bounds and
//!    extents recomputed, when asked, and with them, when `glyf` was rewritten, the `maxp`
//!    maxima and `head.flags` bit 1 (`maxp.recalc`). A `post` table fontTools loads on the way
//!    (a `slnt` axis, `MVAR` deltas for it) has its version 2 names stored again as fontTools
//!    stores them.
//! 6. `CFF2` outlines (see `cff2.rs`): every blend resolved after `HVAR` and `VVAR` moved the
//!    advances, the variation store dropped, and with `downgrade_cff2` a `CFF ` table written
//!    instead; `BASE` coordinates take their deltas (see `base.rs`).
//! 7. With `update_names`, the style names come from the `STAT` axis values at the location
//!    before anything else changes, and the style bits follow them at the end (see `names.rs`).
//!
//! Refused: `VARC`.

use super::glyf::{self, Metrics};
use super::ivs::StoreDeltas;
use super::static_tables as st;
use super::{InstanceOptions, cff2, names, otl, pin_location};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The tables a full instance drops once applied.
const VARIATION_TABLES: [&[u8; 4]; 7] = [b"fvar", b"avar", b"gvar", b"cvar", b"HVAR", b"VVAR", b"MVAR"];

/// The static font at `location` (axis tag → user value; axes left out sit at their default).
pub fn full_instance(font: &SfntFont, location: &[(Tag, f64)]) -> Result<SfntFont> {
    full_instance_with(font, location, &InstanceOptions::default())
}

/// [`full_instance`] with the options of [`InstanceOptions`] other than `limits`.
pub fn full_instance_with(font: &SfntFont, location: &[(Tag, f64)], options: &InstanceOptions) -> Result<SfntFont> {
    if font.get(*b"fvar").is_none() {
        return Err(TablesError::MissingTable("fvar".into()));
    }
    if font.get(*b"gvar").is_some() && font.get(*b"glyf").is_none() {
        return Err(TablesError::invalid("the font has a gvar table but no glyf table"));
    }
    if font.get(*b"VARC").is_some() {
        return Err(TablesError::Unsupported("instancing a VARC table".into()));
    }
    let pinned = pin_location(font, location)?;
    let coords = pinned.normalized.as_slice();
    let axis_count = pinned.axes.len();
    let mut out = font.clone();
    out.remove(*b"DSIG");
    if options.update_names {
        names::update_name_table(&mut out, &pinned.user)?;
    }
    let names_before = st::variation_name_ids(&out);

    let glyph_count = out.get(*b"maxp").and_then(|m| st::u16_at(m, 4)).map(usize::from);
    let mut hmtx: Option<Metrics> = None;
    let has_gvar = out.get(*b"gvar").is_some();
    // fontTools loads `glyf` only to apply `gvar` or set the overlap flags; only then does saving
    // recalculate `maxp`.
    let glyf_loaded = out.get(*b"glyf").is_some() && (has_gvar || options.overlap_flags);
    if glyf_loaded {
        let glyphs_n = glyph_count.ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
        let head = out.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
        let long_loca = st::u16_at(head, 50).ok_or_else(|| TablesError::invalid("the head table is truncated"))? != 0;
        let loca = out.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
        let glyf_data = out.get(*b"glyf").unwrap_or_default();
        let mut glyphs = glyf::read_glyf(glyf_data, loca, long_loca, glyphs_n)?;
        let mut h = read_hmtx(&out, glyphs_n)?;
        let mut v = match (out.get(*b"vmtx"), out.get(*b"vhea")) {
            (Some(m), Some(hea)) => Some(glyf::read_metrics(m, hea, glyphs_n)?),
            _ => None,
        };
        if let Some(gvar) = out.get(*b"gvar") {
            glyf::apply_gvar(&mut glyphs, gvar, axis_count, coords, &mut h, v.as_mut())?;
        }
        if options.overlap_flags {
            glyf::set_overlap_flags(&mut glyphs);
        }
        let (glyf_bytes, loca_bytes, long) = glyf::write_glyf(&glyphs)?;
        out.insert(*b"glyf", glyf_bytes);
        out.insert(*b"loca", loca_bytes);
        st::put_u16(&mut out, *b"head", 50, u16::from(long));
        if has_gvar {
            let (bytes, long_metrics) = glyf::write_metrics(&h);
            out.insert(*b"hmtx", bytes);
            st::put_u16(&mut out, *b"hhea", 34, long_metrics);
            if let Some(v) = &v {
                let (bytes, long_metrics) = glyf::write_metrics(v);
                out.insert(*b"vmtx", bytes);
                st::put_u16(&mut out, *b"vhea", 34, long_metrics);
            }
        }
        if options.recalc_bounds {
            st::recalc_bounds(&mut out, &glyphs, &h, v.as_ref());
        }
        hmtx = Some(h);
    }

    st::apply_cvar(&mut out, axis_count, coords)?;
    let post_before = out.get(*b"post").map(<[u8]>::to_vec);
    st::apply_mvar(&mut out, coords)?;
    // fontTools loads `post` to read its italic angle when the font has a `slnt` axis, or to
    // apply `MVAR` deltas to it, and then writes it again.
    let post_loaded = pinned.user.iter().any(|(t, _)| t == b"slnt") || out.get(*b"post").map(<[u8]>::to_vec) != post_before;
    if out.get(*b"glyf").is_none() {
        // A font without glyf (CFF2) takes its advances from HVAR and VVAR.
        for tag in [*b"HVAR", *b"VVAR"] {
            if let Some(data) = out.get(tag) {
                let offset = st::u16_at(data, 4).zip(st::u16_at(data, 6)).map(|(h, l)| (usize::from(h) << 16) | usize::from(l));
                let offset = offset.ok_or_else(|| TablesError::invalid(format!("the {} table is truncated", crate::sfnt::tag_str(tag))))?;
                let deltas = StoreDeltas::read(data, offset, coords)?;
                cff2::shift_advances(&mut out, tag, &deltas)?;
            }
        }
    }
    otl::instantiate_layout(&mut out, coords)?;
    cff2::instance_full(&mut out, coords, options.downgrade_cff2, options.recalc_bounds)?;

    st::reduce_stat(&mut out, &pinned.user)?;
    for tag in VARIATION_TABLES {
        out.remove(*tag);
    }
    let names_after = st::variation_name_ids(&out);
    let unused: Vec<u16> = names_before.into_iter().filter(|id| !names_after.contains(id)).collect();
    st::prune_names(&mut out, &unused)?;

    if out.get(*b"OS/2").is_some() {
        let h = match hmtx {
            Some(h) => Some(h),
            None => match glyph_count {
                Some(n) => Some(read_hmtx(&out, n)?),
                None => None,
            },
        };
        if let Some(h) = &h {
            st::recalc_avg_char_width(&mut out, h);
        }
    }
    st::set_weight_width_slant(&mut out, &pinned.user);
    if post_loaded && let (Some(post), Some(n)) = (out.get(*b"post"), glyph_count) {
        let post = crate::transform::post::recompile(post, n)?;
        out.insert(*b"post", post);
    }
    if glyf_loaded && options.recalc_bounds {
        crate::ttx::read_core::recalc_compiled_maxp(&mut out)?;
    }
    if options.update_names {
        names::set_ribbi_bits(&mut out)?;
    }
    Ok(out)
}

fn read_hmtx(font: &SfntFont, glyphs: usize) -> Result<Metrics> {
    let mtx = font.get(*b"hmtx").ok_or_else(|| TablesError::MissingTable("hmtx".into()))?;
    let hea = font.get(*b"hhea").ok_or_else(|| TablesError::MissingTable("hhea".into()))?;
    glyf::read_metrics(mtx, hea, glyphs)
}
