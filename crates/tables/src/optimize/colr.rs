//! The `colrLayerReuse` step of `font.optimize`: a version 1 `COLR` table's paint graphs
//! expanded and built again with layer reuse, as fontTools' `colorLib.unbuilder.unbuildColrV1`
//! followed by `colorLib.builder.buildColrV1` does (MIT, Copyright 2020 Google LLC). The work is
//! the `COLR` subsetter's, with every glyph kept under its own id; its limits apply.

use std::collections::{BTreeMap, BTreeSet};

use crate::sfnt::SfntFont;
use crate::subset::Plan;
use crate::subset::other::colr::subset_colr;
use crate::{Result, TablesError};

/// The `COLR` table rebuilt with shared layers; `None` for a font without a version 1 table.
pub fn run(font: &SfntFont) -> Result<Option<Vec<u8>>> {
    let Some(colr) = font.get(*b"COLR") else { return Ok(None) };
    if colr.get(..2) != Some(&[0, 1][..]) {
        return Ok(None);
    }
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let count = maxp
        .get(4..6)
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map(|b| u32::from(u16::from_be_bytes(b)))
        .ok_or_else(|| TablesError::invalid("the maxp table is truncated"))?;
    let all: BTreeSet<u32> = (0..count).collect();
    let plan = Plan {
        num_glyphs: count as usize,
        glyph_map: (0..count).map(|g| (g, g)).collect::<BTreeMap<u32, u32>>(),
        new_to_old: (0..count).collect(),
        colr_glyphs: all.clone(),
        glyphs: all,
        ..Plan::default()
    };
    Ok(subset_colr(&plan, colr, false)?.map(|(table, _)| table))
}
