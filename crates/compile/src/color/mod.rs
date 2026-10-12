//! The colour tables (`docs/plans/M11.md` § 3, § 4 › M11.6 to M11.11, M11.19): `CPAL`, `COLR`,
//! `sbix`, `SVG ` and, as an export option, `CBDT` with `CBLC`.
//!
//! | Module | Package | Table |
//! |---|---|---|
//! | [`cpal`] | M11.6 | `CPAL` version 0: every palette, the chosen one first, no labels |
//! | [`colr`] | M11.6 (v0), M11.7 (v1), M11.19 (variable) | `COLR` base glyphs, layers, clip boxes |
//! | [`sbix`] | M11.8 | `sbix` version 1, serialised here and added raw |
//! | [`svg`] | M11.9, M11.10 | the `SVG ` table, one document per glyph |
//! | [`cbdt`] | M11.11 | `CBDT` and `CBLC`, glyph bitmap format 17 |
//!
//! How they join the pipeline (`lib.rs`, `plan.rs`): [`plan`] reads the export plan from
//! `fontcraft_color::export` once per compile; [`append_layer_glyphs`] adds the layer glyphs
//! `<base>.color<i>` after the source glyphs, before outline preparation, so they are drawn and
//! written like any glyph; [`build`] makes the tables, which `build::assemble` adds raw. A font
//! without colour layers gets an empty plan, and nothing changes.
//!
//! Image bytes come only through the [`AssetStore`] in [`crate::Options::assets`]; the compiler
//! never opens a file.

pub mod cbdt;
pub mod colr;
pub mod cpal;
pub mod sbix;
pub mod svg;

use std::collections::HashMap;

use fontcraft_color::export::{ExportPlan, ProblemKind, Switches, plan_master};
use fontcraft_color::{AssetStore, MemoryAssets};
use fontcraft_model::{Font, Resolved, ResolvedGlyph};
use write_fonts::types::Tag;

use crate::order::{MadeGlyph, append_made};
use crate::{CompileError, Options};

/// The final glyph order with glyph ids by name: the source glyphs, then the made glyphs.
pub struct GlyphOrder<'a> {
    pub glyphs: &'a [ResolvedGlyph],
    /// The made glyphs, which are the last `made.len()` of [`GlyphOrder::glyphs`].
    pub made: &'a [MadeGlyph],
    ids: HashMap<&'a str, u16>,
}

impl<'a> GlyphOrder<'a> {
    /// Refused when there are more glyphs than glyph ids.
    pub fn new(glyphs: &'a [ResolvedGlyph], made: &'a [MadeGlyph]) -> Result<Self, CompileError> {
        let mut ids = HashMap::with_capacity(glyphs.len());
        for (i, g) in glyphs.iter().enumerate() {
            let id = u16::try_from(i).map_err(|_| CompileError::TooManyGlyphs(glyphs.len()))?;
            ids.entry(g.name.as_str()).or_insert(id);
        }
        Ok(Self { glyphs, made, ids })
    }

    /// The glyph id of `name`.
    pub fn gid(&self, name: &str) -> Option<u16> {
        self.ids.get(name).copied()
    }
}

/// The tables [`build`] made, by tag, for `build::assemble` to add as they are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorTables {
    pub tables: Vec<(Tag, Vec<u8>)>,
}

/// The export plan of a compile of `resolved`, read from `font`; empty without a source font
/// (a compile of a resolved font alone has no colour layers).
pub fn plan(font: Option<&Font>, resolved: &Resolved, options: &Options) -> ExportPlan {
    let Some(font) = font else { return ExportPlan::default() };
    let switches = Switches { cbdt: options.cbdt, ..Switches::from_params(&resolved.params) };
    plan_master(font, &resolved.master, &switches)
}

/// `glyphs` with the layer glyphs `plan` needs appended, and their descriptions.
pub fn append_layer_glyphs(
    font: Option<&Font>,
    plan: &ExportPlan,
    resolved: &Resolved,
    glyphs: Vec<ResolvedGlyph>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<ResolvedGlyph>, Vec<MadeGlyph>), CompileError> {
    append_made(glyphs, layer_glyphs(font, plan, resolved, warnings)?)
}

/// The layer glyphs of `plan`'s COLR paints: one per [`fontcraft_color::ShapeRef`], named
/// `<base>.color<i>` from the base glyph's production name, outlines resolved (strokes expanded)
/// at the resolved master, never shared between base glyphs.
pub fn layer_glyphs(
    font: Option<&Font>,
    plan: &ExportPlan,
    resolved: &Resolved,
    warnings: &mut Vec<String>,
) -> Result<Vec<(MadeGlyph, ResolvedGlyph)>, CompileError> {
    // M11.6: palette layers; M11.7: strokes expanded.
    let _ = (font, plan, resolved, warnings);
    Ok(Vec::new())
}

/// The colour tables `plan` names. A plan with a refusal fails here with its message.
pub fn build(
    font: Option<&Font>,
    plan: &ExportPlan,
    order: &GlyphOrder,
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<ColorTables, CompileError> {
    if let Some(refusal) = plan.problems.iter().find(|p| p.kind == ProblemKind::Refusal) {
        return Err(CompileError::ColorRefused(refusal.message.clone()));
    }
    for p in plan.problems.iter().filter(|p| p.kind == ProblemKind::Warning) {
        warnings.push(match &p.glyph {
            Some(glyph) => format!("glyph `{glyph}`: {}", p.message),
            None => p.message.clone(),
        });
    }
    if plan.tables().is_empty() {
        return Ok(ColorTables::default());
    }
    let empty = MemoryAssets::new();
    let assets: &dyn AssetStore = match &options.assets.0 {
        Some(a) => a.as_ref(),
        None => &empty,
    };
    // M11.6 to M11.11: each table builder in turn.
    if plan.cpal.is_some() || plan.colr.is_some() || plan.sbix.is_some() || plan.cbdt.is_some() {
        return Err(CompileError::NotYetImplemented("the colour tables"));
    }
    let mut out = ColorTables::default();
    if let (Some(svg_plan), Some(font)) = (&plan.svg, font)
        && let Some(bytes) = svg::build(font, svg_plan, order, assets, warnings)?
    {
        out.tables.push((Tag::new(b"SVG "), bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use fontcraft_color::export::{ExportPlan, SvgDocument, SvgPlan, SvgSource};
    use fontcraft_color::paint::Paint;
    use fontcraft_model::{Font, ResolvedGlyph};
    use write_fonts::types::Tag;

    use super::{GlyphOrder, build};
    use crate::Options;

    #[test]
    fn an_svg_plan_makes_the_svg_table() {
        let mut font = Font::new("Svg Sans");
        font.add_glyph("heart", vec![0x2665], 600.0).unwrap();
        let svg = SvgPlan {
            palette: Vec::new(),
            documents: vec![SvgDocument { glyph: "heart".into(), source: SvgSource::Paint(Paint::Layers(Vec::new())) }],
        };
        let plan = ExportPlan { svg: Some(svg), ..ExportPlan::default() };
        let glyphs: Vec<ResolvedGlyph> = [".notdef", "heart"].into_iter().map(|n| ResolvedGlyph::new(n, 600.0)).collect();
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let tables = build(Some(&font), &plan, &order, &Options::default(), &mut Vec::new()).unwrap();
        let tags: Vec<Tag> = tables.tables.iter().map(|(t, _)| *t).collect();
        assert_eq!(tags, [Tag::new(b"SVG ")]);
        assert!(!tables.tables[0].1.is_empty());
    }
}
