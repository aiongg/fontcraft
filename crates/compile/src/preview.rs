//! The layout-only font Edit View and Text Preview shape with (M8.19;
//! `docs/plans/M8.md` § 3, The preview font).
//!
//! Rows: `features.menuNeedsCompile` (the font the menu lists), `features.previewScope`,
//! `text.shaping`, `textPreview.shaping`.
//!
//! The font holds `head`, `maxp`, `cmap`, `hhea`, `hmtx`, `GDEF`, `GSUB`, `GPOS` (for
//! [`PreviewKind::Full`]) and `post`, built for one master. Glyphs without a code point get a
//! private-use code point of plane 15 or 16, kept in [`PreviewFont::private_use`] beside the
//! font, not in its `cmap`; `fontcraft_shaping` reads them from there.

use std::collections::BTreeSet;

use fontcraft_fea::FeatureSummary;
use fontcraft_features::EntryDiagnostic;
use fontcraft_model::{Font, Resolved, ResolvedGlyph, Target};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::head::Head;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::post::Post;
use write_fonts::types::{GlyphId, Tag, UfWord, Version16Dot16};

use crate::layout::{LayoutInput, LayoutOptions, LayoutTables};
use crate::{CompileError, Options};

/// First private-use code point of plane 15.
pub const PLANE_15_START: u32 = 0xF_0000;
/// Last private-use code point of plane 15.
pub const PLANE_15_END: u32 = 0xF_FFFD;
/// First private-use code point of plane 16.
pub const PLANE_16_START: u32 = 0x10_0000;
/// Last private-use code point of plane 16.
pub const PLANE_16_END: u32 = 0x10_FFFD;

/// Which layout tables the preview font has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    /// `GSUB` only: Edit View, which positions with live kerning and anchors.
    GsubOnly,
    /// `GSUB` and `GPOS`, generated kerning and marks included: Text Preview.
    Full,
}

/// A compiled preview font.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewFont {
    pub bytes: Vec<u8>,
    /// The master it was built for.
    pub master: String,
    pub kind: PreviewKind,
    /// Glyph names in glyph-id order, `.notdef` first.
    pub glyph_order: Vec<String>,
    /// (private-use code point, glyph id) for each glyph without a code point.
    pub private_use: Vec<(u32, u16)>,
    /// The features of its `GSUB` and `GPOS`, for the Features menu.
    pub features: Vec<FeatureSummary>,
    /// The compiler's warnings, mapped to the entries.
    pub diagnostics: Vec<EntryDiagnostic>,
}

/// The private-use code point given to the `index`-th unencoded glyph: plane 15 first, then
/// plane 16; `None` past both (131,068 glyphs, more than a font holds).
pub fn private_use_code_point(index: usize) -> Option<u32> {
    let index = u32::try_from(index).ok()?;
    let plane15 = PLANE_15_END - PLANE_15_START + 1;
    if index < plane15 {
        return Some(PLANE_15_START + index);
    }
    let rest = index - plane15;
    (rest <= PLANE_16_END - PLANE_16_START).then(|| PLANE_16_START + rest)
}

/// Builds the preview font of `master` with the default options: no glyph database of the
/// session, no folder for included files. Feature-code errors come back as
/// [`CompileError::Features`].
pub fn layout_font(font: &Font, master: &str, kind: PreviewKind) -> Result<PreviewFont, CompileError> {
    layout_font_with(font, master, kind, &Options::default())
}

/// Builds the preview font of `master`. Of `options`, the glyph database, the folder included
/// files are read from, the include check and the feature axes are used.
///
/// The glyphs go through the stages an export runs before its outlines (glyph data, export
/// parameters, preflight, the final order), so the preview font has the exported glyph set and
/// order. The feature code is compiled as stored, as `font.features.compile` compiles it, with
/// the generated kerning and marks for [`PreviewKind::Full`] and their `GDEF` classes for both
/// kinds (the classes decide which glyphs a lookup skips).
pub fn layout_font_with(font: &Font, master: &str, kind: PreviewKind, options: &Options) -> Result<PreviewFont, CompileError> {
    font.check()?;
    font.master(master)?;
    let target = Target::Master(master.to_string());
    let mut warnings = Vec::new();
    let (source, prepared) = crate::sources_at(font, &target, &mut warnings)?;
    let mut resolved = crate::drawing::resolve(&prepared, &target)?;
    let options = crate::with_instance_feature_axes(options, font, &target);
    crate::fill_glyph_data(&mut resolved, &options, &mut warnings);
    crate::info::expand_tokens(Some(&*source), &mut resolved, &mut warnings);
    let edits = crate::export_params::apply(Some(&*source), &mut resolved, &mut warnings)?;
    let glyphs = crate::preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let glyphs = crate::order::final_order(glyphs, &resolved, &mut warnings)?;
    crate::preflight::check_cmap(&glyphs)?;
    let data = crate::compile_glyph_data(&options);
    let input = LayoutInput {
        font: Some(&*source),
        data: data.as_deref(),
        resolved: &resolved,
        order: &glyphs,
        edits: &edits,
        first_name_id: if source.features.is_empty() { 256 } else { crate::info::first_free_name_id(&resolved, &options.style_name) },
        include_dir: options.feature_dir.as_deref(),
        include_check: options.include_check.as_ref(),
        variable: None,
        feature_axes: options.feature_axes.as_ref(),
    };
    let layout_options = LayoutOptions { regenerate_automatic: false, gpos: kind == PreviewKind::Full, implicit: true };
    let layout = crate::layout::build(&input, &layout_options, &mut warnings)?;

    let sources = edits.source_names();
    let glyph_order: Vec<String> = glyphs.iter().map(|g| sources.get(g.name.as_str()).map_or_else(|| g.name.clone(), |s| (*s).to_string())).collect();
    let (bytes, private_use) = tables(&resolved, &glyphs, kind, &layout)?;
    Ok(PreviewFont {
        bytes,
        master: master.to_string(),
        kind,
        glyph_order,
        private_use,
        features: layout.compiled.features.clone(),
        diagnostics: layout.diagnostics,
    })
}

fn table_error<E: std::fmt::Display>(table: &'static str) -> impl Fn(E) -> CompileError {
    move |e| CompileError::Table { table, message: e.to_string() }
}

/// The font file: the metrics tables a shaper reads, `cmap` with the glyphs' own code points
/// only, and the layout tables. Every glyph after `.notdef` without a code point gets the next
/// private-use code point no glyph has, returned beside the bytes.
/// The font's bytes and the private-use code points given to unencoded glyphs.
type PreviewBytes = (Vec<u8>, Vec<(u32, u16)>);

fn tables(resolved: &Resolved, glyphs: &[ResolvedGlyph], kind: PreviewKind, layout: &LayoutTables) -> Result<PreviewBytes, CompileError> {
    let count = u16::try_from(glyphs.len()).map_err(|_| CompileError::TooManyGlyphs(glyphs.len()))?;
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();
    let mut code_points: BTreeSet<u32> = BTreeSet::new();
    for (gid, g) in (0u16..).zip(glyphs) {
        for &cp in &g.unicodes {
            let Some(ch) = char::from_u32(cp) else { continue };
            if code_points.insert(cp) {
                mappings.push((ch, GlyphId::new(u32::from(gid))));
            }
        }
    }
    let mut private_use = Vec::new();
    let mut next = 0usize;
    for (gid, g) in (0u16..).zip(glyphs).skip(1) {
        if g.unicodes.iter().any(|cp| char::from_u32(*cp).is_some()) {
            continue;
        }
        // `next` only grows, so all glyphs together try each private-use code point once.
        while let Some(cp) = private_use_code_point(next) {
            next += 1;
            if !code_points.contains(&cp) {
                private_use.push((cp, gid));
                break;
            }
        }
    }

    let metrics: Vec<LongMetric> = glyphs.iter().map(|g| LongMetric { advance: crate::u16_of(g.width), side_bearing: 0 }).collect();
    let advance_max = metrics.iter().map(|m| m.advance).max().unwrap_or(0);
    let mut builder = FontBuilder::new();
    let head = Head { units_per_em: resolved.units_per_em, ..Default::default() };
    let hhea = Hhea { advance_width_max: UfWord::new(advance_max), number_of_h_metrics: count, ..Default::default() };
    let maxp = Maxp { num_glyphs: count, ..Default::default() };
    let post = Post { version: Version16Dot16::VERSION_3_0, ..Default::default() };
    let cmap = Cmap::from_mappings(mappings).map_err(table_error("cmap"))?;
    builder.add_table(&head).map_err(table_error("head"))?;
    builder.add_table(&hhea).map_err(table_error("hhea"))?;
    builder.add_table(&maxp).map_err(table_error("maxp"))?;
    builder.add_table(&Hmtx::new(metrics, Vec::new())).map_err(table_error("hmtx"))?;
    builder.add_table(&cmap).map_err(table_error("cmap"))?;
    builder.add_table(&post).map_err(table_error("post"))?;
    let c = &layout.compiled;
    let gpos = if kind == PreviewKind::Full { c.gpos.as_deref() } else { None };
    for (tag, bytes) in [(b"GDEF", c.gdef.as_deref()), (b"GSUB", c.gsub.as_deref()), (b"GPOS", gpos)] {
        if let Some(bytes) = bytes {
            builder.add_raw(Tag::new(tag), bytes);
        }
    }
    Ok((builder.build(), private_use))
}

#[cfg(test)]
mod tests;
