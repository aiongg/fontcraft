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

use std::collections::{HashMap, HashSet};

use fontcraft_color::export::{ExportPlan, Problem, ProblemKind, Switches, has_color_layers, plan_between_masters, plan_master, read_switches};
use fontcraft_color::paint::{ShapeRef, shape_refs};
use fontcraft_color::{AssetStore, MemoryAssets};
use fontcraft_model::{Font, Glyph, Layer, Resolved, ResolvedComponent, ResolvedGlyph, Shape, Target};
use write_fonts::types::Tag;

use crate::drawing::{format_version, has_strokes_or_masks, shapes_outline};
use crate::order::{MadeFrom, MadeGlyph, append_made};
use crate::{CompileError, MAX_FONT_GLYPHS, Options};

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
/// (a compile of a resolved font alone has no colour layers). An instance between masters is
/// refused when it would write colour tables ([`plan_between_masters`]).
pub fn plan(font: Option<&Font>, resolved: &Resolved, options: &Options) -> ExportPlan {
    let Some(font) = font else { return ExportPlan::default() };
    let switches = Switches { cbdt: options.cbdt, ..Switches::from_params(&resolved.params) };
    if let Some(i) = resolved.instance
        && font.master_at(&resolved.location).is_none()
    {
        let label = match font.instances.get(i) {
            Some(inst) if !inst.name.is_empty() => inst.name.clone(),
            _ => format!("#{i}"),
        };
        return plan_between_masters(font, &label, &switches);
    }
    plan_master(font, &resolved.master, &switches)
}

/// Refuses a static instance the compile interpolates ([`crate::needs_instance_font`]) when it
/// would write colour tables from the font's colour layers: colour layers do not interpolate
/// yet (M11.19).
pub fn check_instance(font: &Font, target: &Target) -> Result<(), CompileError> {
    let Some(i) = crate::needs_instance_font(font, target) else { return Ok(()) };
    let switches = read_switches(font, target)?;
    let label = match font.instances.get(i) {
        Some(inst) if !inst.name.is_empty() => inst.name.clone(),
        _ => format!("#{i}"),
    };
    match plan_between_masters(font, &label, &switches).problems.into_iter().find(|p| p.kind == ProblemKind::Refusal) {
        Some(p) => Err(CompileError::ColorRefused(p.message)),
        None => Ok(()),
    }
}

/// The plan of a variable compile, at the default master `master`: no colour tables until colour
/// layers interpolate (M11.19), with a warning when the font has colour layers. The exception is a
/// font whose colour layers are all layer glyphs the source already has
/// ([`existing_layer_glyph`]): those glyphs vary as any glyph does, so the default master's `COLR`
/// and `CPAL` hold at every location, as both oracles write them.
pub fn variable_plan(font: Option<&Font>, master: &str, switches: &Switches) -> ExportPlan {
    let mut plan = ExportPlan::default();
    let Some(font) = font.filter(|f| has_color_layers(f)) else { return plan };
    let default = plan_master(font, master, switches);
    if let Some(colr) = &default.colr
        && !default.refused()
        && default.sbix.is_none()
        && default.svg.is_none()
        && default.cbdt.is_none()
        && all_existing(font, colr, master)
    {
        return ExportPlan { cpal: default.cpal, colr: default.colr, problems: default.problems, ..Default::default() };
    }
    plan.problems.push(Problem {
        kind: ProblemKind::Warning,
        glyph: None,
        message: "the variable font is written without colour tables: colour layers do not interpolate yet".to_string(),
    });
    plan
}

/// Whether every layer glyph of `colr` (planned at `master`) is a source glyph, and every colour
/// layer of the other masters one of that form too.
fn all_existing(font: &Font, colr: &fontcraft_color::export::ColrPlan, master: &str) -> bool {
    let planned = colr.glyphs.iter().all(|cg| {
        font.glyph(&cg.glyph).is_some_and(|source| {
            shape_refs(&cg.paint)
                .into_iter()
                .enumerate()
                .all(|(i, s)| existing_layer_glyph(source, &cg.glyph, i, s, master).is_some_and(|name| font.glyph(&name).is_some_and(|g| g.export)))
        })
    });
    let others =
        font.glyphs.as_arcs().iter().flat_map(|g| g.layers.iter()).filter(|l| l.master != master && fontcraft_color::layers::kind(l).is_some()).all(
            |l| {
                l.info.attrs.color_palette.is_some()
                    && matches!(l.shapes.as_slice(), [Shape::Component(c)] if c.transform.is_identity() && font.glyph(&c.glyph).is_some())
            },
        );
    planned && others
}

/// `glyphs` with the layer glyphs `plan` needs appended, and their descriptions.
pub fn append_layer_glyphs(
    font: Option<&Font>,
    plan: &ExportPlan,
    resolved: &Resolved,
    glyphs: Vec<ResolvedGlyph>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<ResolvedGlyph>, Vec<MadeGlyph>), CompileError> {
    let made = layer_glyphs(font, plan, resolved, &glyphs, warnings)?;
    append_made(glyphs, made)
}

/// The layer glyphs of `plan`'s COLR paints: one per [`fontcraft_color::ShapeRef`], named
/// `<base>.color<i>` by its position among the base glyph's layer glyphs, with the base glyph's
/// production name in place of its name where it has one, outlines resolved at the resolved
/// master, never shared between base glyphs. `glyphs` are the final source glyphs: a base glyph
/// that is not among them gets no layer glyphs. A layer glyph the source already has
/// ([`existing_layer_glyph`]) is not made again.
///
/// A layer glyph takes the advance of its colour layer, as fontc writes it (the editor keeps
/// colour layers at their master layer's width, but a mark's master layer loses its advance at
/// export and its colour layers keep theirs), and no anchors, category or Unicode value. The
/// shapes of the resolved master keep their components; shapes of another master (a layered
/// font's) and shapes with strokes or masks become contours, drawn as Edit View draws them.
pub fn layer_glyphs(
    font: Option<&Font>,
    plan: &ExportPlan,
    resolved: &Resolved,
    glyphs: &[ResolvedGlyph],
    warnings: &mut Vec<String>,
) -> Result<Vec<(MadeGlyph, ResolvedGlyph)>, CompileError> {
    let (Some(font), Some(colr)) = (font, &plan.colr) else { return Ok(Vec::new()) };
    if plan.refused() {
        return Ok(Vec::new());
    }
    let by_name: HashMap<&str, &ResolvedGlyph> = glyphs.iter().map(|g| (g.name.as_str(), g)).collect();
    let mut taken: HashSet<String> = glyphs.iter().map(|g| g.name.clone()).collect();
    let mut out = Vec::new();
    for cg in &colr.glyphs {
        let (Some(base), Some(source)) = (by_name.get(cg.glyph.as_str()), font.glyph(&cg.glyph)) else { continue };
        for (i, shape) in shape_refs(&cg.paint).into_iter().enumerate() {
            if existing_layer_glyph(source, &cg.glyph, i, shape, &resolved.master).is_some_and(|name| by_name.contains_key(name.as_str())) {
                continue;
            }
            if glyphs.len().saturating_add(out.len()) >= MAX_FONT_GLYPHS {
                return Err(CompileError::TooManyGlyphs(glyphs.len().saturating_add(out.len()).saturating_add(1)));
            }
            let name = unique_name(format!("{}.color{i}", base.name), &mut taken, warnings);
            let mut g = ResolvedGlyph::new(name.clone(), base.width);
            g.production_name = base.production_name.as_ref().map(|p| format!("{p}.color{i}"));
            g.vert_width = base.vert_width;
            g.vert_origin = base.vert_origin;
            match source.layers.iter().find(|l| l.id == shape.layer) {
                Some(layer) => {
                    g.width = layer.width;
                    if let Err(message) = draw_shapes(font, layer, shape, &resolved.master, &mut g) {
                        warnings.push(format!("glyph `{}`: its colour layer glyph `{name}` was written empty: {message}", cg.glyph));
                    }
                }
                None => warnings
                    .push(format!("glyph `{}`: there is no layer `{}`; its colour layer glyph `{name}` was written empty", cg.glyph, shape.layer)),
            }
            let made = MadeGlyph { name, source: MadeFrom::ColorLayer { glyph: cg.glyph.clone(), layer: shape.layer.clone() } };
            out.push((made, g));
        }
    }
    Ok(out)
}

/// The source glyph that already is layer glyph `i` of `base`: the glyph named `<base>.color<i>`,
/// the name a new layer glyph would get, when `shape` is a whole colour layer of `master` holding
/// nothing but that glyph as an unmoved component. A UFO's font-wide colour layer list opens this
/// way, so its layer glyphs stay where the source has them, as both oracles write them.
/// [`colr::build`] finds such a glyph by that name.
pub fn existing_layer_glyph(source: &Glyph, base: &str, i: usize, shape: &ShapeRef, master: &str) -> Option<String> {
    if shape.stroke.is_some() {
        return None;
    }
    let layer = source.layers.iter().find(|l| l.id == shape.layer && l.master == master)?;
    let ([k], [Shape::Component(c)]) = (shape.shapes.as_slice(), layer.shapes.as_slice()) else { return None };
    let name = format!("{base}.color{i}");
    (*k == 0 && c.glyph == name && c.transform.is_identity()).then_some(name)
}

/// `wanted`, or the first of `wanted_1`, `wanted_2`… that no glyph has, with a warning.
fn unique_name(wanted: String, taken: &mut HashSet<String>, warnings: &mut Vec<String>) -> String {
    if taken.insert(wanted.clone()) {
        return wanted;
    }
    let mut n = 1usize;
    loop {
        let name = format!("{wanted}_{n}");
        if taken.insert(name.clone()) {
            warnings.push(format!("a glyph called `{wanted}` exists, so the colour layer glyph of that name is called `{name}`"));
            return name;
        }
        n = n.saturating_add(1);
    }
}

/// Draws the shapes `shape` names of `layer` into `out`.
///
/// On a full-colour layer a path's fill and its stroke are separate layer glyphs
/// (`color.export.strokeInColr`): the fill is the path's own outline, and the stroke is the
/// stroke alone, expanded with the path's width, position, caps and joins by the stroke code
/// every other layer uses.
fn draw_shapes(font: &Font, layer: &Layer, shape: &ShapeRef, master: &str, out: &mut ResolvedGlyph) -> Result<(), String> {
    let mut shapes: Vec<Shape> = shape.shapes.iter().filter_map(|&i| layer.shapes.get(i).cloned()).collect();
    if layer.info.attrs.full_color {
        if shape.stroke.is_some() {
            shapes.retain(|s| matches!(s, Shape::Path(_)));
            for s in &mut shapes {
                if let Shape::Path(p) = s {
                    p.info.attrs.fill = false;
                }
            }
            out.contours = shapes_outline(font, &shapes, &layer.master)?.into_iter().filter(|c| !c.elements().is_empty()).collect();
            return Ok(());
        }
        for s in &mut shapes {
            if let Shape::Path(p) = s {
                p.info.attrs.stroke_width = None;
            }
        }
    } else if shape.stroke.is_some() {
        return Err("only full-colour layers have stroke layers".to_string());
    }
    if layer.master != master || has_strokes_or_masks(&shapes, format_version(font)) {
        out.contours = shapes_outline(font, &shapes, &layer.master)?.into_iter().filter(|c| !c.elements().is_empty()).collect();
        return Ok(());
    }
    for s in &shapes {
        match s {
            Shape::Path(p) => {
                let contour = p.to_contour()?;
                if !contour.elements().is_empty() {
                    out.contours.push(contour);
                }
            }
            Shape::Component(c) => {
                let transform = c.affine();
                if !transform.as_coeffs().iter().all(|v| v.is_finite()) {
                    return Err("a component transform is not a number".to_string());
                }
                out.components.push(ResolvedComponent { base: c.glyph.clone(), transform });
            }
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    Ok(())
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
    let mut out = ColorTables::default();
    // COLR and CPAL go together; a plan whose base glyphs were all left out of the font writes
    // neither.
    if let Some(colr_plan) = &plan.colr
        && colr_plan.glyphs.iter().any(|g| order.gid(&g.glyph).is_some())
    {
        let cpal_plan = plan.cpal.clone().unwrap_or_default();
        let built = colr::build(colr_plan, order, cpal_plan.palettes.rows(), font)?;
        let cpal_table = cpal::build(&cpal_plan, &built.added)?;
        out.tables.push((Tag::new(b"COLR"), built.table));
        out.tables.push((Tag::new(b"CPAL"), cpal_table));
    }
    if let (Some(sbix_plan), Some(font)) = (&plan.sbix, font) {
        out.tables.push((Tag::new(b"sbix"), sbix::build(sbix_plan, order, assets, font.units_per_em, warnings)?));
    }
    if let (Some(cbdt_plan), Some(font)) = (&plan.cbdt, font) {
        let built = cbdt::build(cbdt_plan, order, assets, font.units_per_em, warnings)?;
        out.tables.push((Tag::new(b"CBDT"), built.cbdt));
        out.tables.push((Tag::new(b"CBLC"), built.cblc));
    }
    if let (Some(svg_plan), Some(font)) = (&plan.svg, font)
        && let Some(bytes) = svg::build(font, svg_plan, order, assets, warnings)?
    {
        out.tables.push((Tag::new(b"SVG "), bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_color::export::{
        COLOR_LAYERS_TO_COLR, COLOR_PALETTE_FOR_CPAL, EXPORT_COLR_TABLE, ExportPlan, MASTER_COLOR, SvgDocument, SvgPlan, SvgSource, Switches,
    };
    use fontcraft_color::layers::with_kind;
    use fontcraft_color::paint::Paint;
    use fontcraft_color::palette::to_param;
    use fontcraft_color::{ColorLayerKind, Palettes, Rgba};
    use fontcraft_geom::Point;
    use fontcraft_model::plist::{Number, Str, Value};
    use fontcraft_model::{
        Color, ColorStop as GradientStop, Component, Dimension, Extend, Font, Gradient, GradientKind, Instance, Layer, LayerAttrs, Master,
        PaletteEntry, Param, Path, ResolvedGlyph, Shape, Target,
    };
    use skrifa::color::{Brush, ColorGlyphFormat, ColorPainter, CompositeMode, Transform};
    use skrifa::instance::{LocationRef, Size};
    use skrifa::raw::TableProvider;
    use skrifa::raw::types::{BoundingBox, GlyphId};
    use skrifa::{FontRef, MetadataProvider};
    use write_fonts::types::Tag;

    use super::{GlyphOrder, build, variable_plan};
    use crate::{Compiled, Flavour, Options, compile};

    fn colour_layer(id: &str, master: &str, entry: PaletteEntry, shapes: Vec<Shape>) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = id.to_string();
        l.info.attrs = with_kind(&LayerAttrs::default(), Some(ColorLayerKind::Palette(entry)));
        l.shapes = shapes;
        l
    }

    /// Two palettes of two colours. `A` paints a square in colour 1 under a square in the text
    /// colour that also holds a component; `B` (production name `bee`) one rectangle in colour 0.
    fn palette_font() -> Font {
        let mut f = Font::new("Palette Sans");
        let palettes =
            Palettes::new(vec![vec![Rgba::new(1, 2, 3, 4), Rgba::new(5, 6, 7, 8)], vec![Rgba::new(9, 10, 11, 12), Rgba::new(13, 14, 15, 16)]])
                .unwrap();
        Arc::make_mut(&mut f.info).params.0.push(to_param(&palettes));
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].width = 600.0;
        a.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
        a.layers.push(colour_layer("A-1", "m01", PaletteEntry::Index(1), vec![Shape::Path(Path::rect(50.0, 0.0, 550.0, 500.0))]));
        a.layers.push(colour_layer(
            "A-2",
            "m01",
            PaletteEntry::Foreground,
            vec![Shape::Path(Path::rect(100.0, 100.0, 300.0, 300.0)), Shape::Component(Component::new("C", 0.0, 0.0))],
        ));
        drop(a);
        f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(400.0, 400.0, 500.0, 600.0)));
        let mut b = f.glyph_mut("B").unwrap();
        b.info.production = Some("bee".into());
        b.layers[0].width = 500.0;
        b.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 400.0, 700.0)));
        b.layers.push(colour_layer("B-1", "m01", PaletteEntry::Index(0), vec![Shape::Path(Path::rect(20.0, 0.0, 380.0, 650.0))]));
        drop(b);
        f
    }

    fn gid(out: &Compiled, name: &str) -> u16 {
        u16::try_from(out.glyph_order.iter().position(|n| n == name).unwrap()).unwrap()
    }

    /// The COLR version 0 records: (base glyph, [(layer glyph, palette index)]).
    fn colr_records(out: &Compiled) -> Vec<(String, Vec<(String, u16)>)> {
        let font = FontRef::new(&out.bytes).unwrap();
        let colr = font.colr().unwrap();
        assert_eq!(colr.version(), 0);
        let layers = colr.layer_records().unwrap().unwrap();
        let name = |g: skrifa::raw::types::GlyphId16| out.glyph_order[usize::from(g.to_u16())].clone();
        colr.base_glyph_records()
            .unwrap()
            .unwrap()
            .iter()
            .map(|b| {
                let first = usize::from(b.first_layer_index());
                let own = layers.get(first..first + usize::from(b.num_layers())).unwrap();
                (name(b.glyph_id()), own.iter().map(|l| (name(l.glyph_id()), l.palette_index())).collect())
            })
            .collect()
    }

    fn cpal_palettes(out: &Compiled) -> Vec<Vec<(u8, u8, u8, u8)>> {
        let font = FontRef::new(&out.bytes).unwrap();
        let cpal = font.cpal().unwrap();
        let records = cpal.color_records_array().unwrap().unwrap();
        let entries = usize::from(cpal.num_palette_entries());
        cpal.color_record_indices()
            .iter()
            .map(|i| records[usize::from(i.get())..usize::from(i.get()) + entries].iter().map(|c| (c.red, c.green, c.blue, c.alpha)).collect())
            .collect()
    }

    fn has_table(out: &Compiled, tag: &[u8; 4]) -> bool {
        FontRef::new(&out.bytes).unwrap().table_data(Tag::new(tag)).is_some()
    }

    #[test]
    fn palette_layers_compile_to_cpal_colr_and_layer_glyphs() {
        let f = palette_font();
        for flavour in [Flavour::TrueType, Flavour::Cff] {
            let out = compile(&f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap();
            let n = out.glyph_order.len();
            assert_eq!(&out.glyph_order[n - 3..], ["A.color0", "A.color1", "B.color0"], "{flavour:?}: layer glyphs after the source glyphs");
            assert_eq!(
                colr_records(&out),
                [
                    ("A".to_string(), vec![("A.color0".to_string(), 1), ("A.color1".to_string(), 0xFFFF)]),
                    ("B".to_string(), vec![("B.color0".to_string(), 0)]),
                ],
                "{flavour:?}: the first layer in the file at the bottom, `*` as 0xFFFF"
            );
            assert_eq!(cpal_palettes(&out), [vec![(1, 2, 3, 4), (5, 6, 7, 8)], vec![(9, 10, 11, 12), (13, 14, 15, 16)]], "{flavour:?}");

            let font = FontRef::new(&out.bytes).unwrap();
            let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
            let bounds = |name: &str| {
                let b = metrics.bounds(GlyphId::new(u32::from(gid(&out, name)))).unwrap();
                (b.x_min, b.y_min, b.x_max, b.y_max)
            };
            // Each layer glyph draws its own layer, the component included; the base glyph keeps
            // the master layer as the fallback.
            assert_eq!(bounds("A.color0"), (50.0, 0.0, 550.0, 500.0), "{flavour:?}");
            assert_eq!(bounds("A.color1"), (100.0, 100.0, 500.0, 600.0), "{flavour:?}");
            assert_eq!(bounds("A"), (50.0, 0.0, 550.0, 700.0), "{flavour:?}");
            assert_eq!(metrics.advance_width(GlyphId::new(u32::from(gid(&out, "B.color0")))), Some(600.0), "{flavour:?}: the colour layer's advance");
            assert_eq!(font.charmap().map('A').map(|g| g.to_u32()), Some(u32::from(gid(&out, "A"))), "{flavour:?}");
            // A layer glyph is named from its base glyph's production name (CFF fonts keep their
            // names in the charset, not in `post`).
            if matches!(flavour, Flavour::TrueType) {
                let post = font.post().unwrap();
                assert_eq!(post.glyph_name(skrifa::raw::types::GlyphId16::new(gid(&out, "B.color0"))), Some("bee.color0"));
            }
        }
    }

    #[test]
    fn instance_switches_remove_or_reorder_the_colour_tables() {
        let mut f = palette_font();
        let int = |i: i64| Value::Number(Number::int(i));
        let instance = |name: &str, param: (&str, Value)| {
            let mut inst = Instance { name: name.into(), ..Default::default() };
            inst.params.0.push(Param::new(param.0, param.1));
            inst
        };
        Arc::make_mut(&mut f.instances)
            .extend([instance("Second", (COLOR_PALETTE_FOR_CPAL, int(1))), instance("Plain", (EXPORT_COLR_TABLE, Value::Bool(false)))]);
        let ttf = |i: usize| compile(&f, &Target::Instance(i), Flavour::TrueType, &Options::default()).unwrap();

        let second = ttf(0);
        assert_eq!(cpal_palettes(&second), [vec![(9, 10, 11, 12), (13, 14, 15, 16)], vec![(1, 2, 3, 4), (5, 6, 7, 8)]]);
        assert_eq!(colr_records(&second).len(), 2, "the layer records are unchanged");

        let plain = ttf(1);
        assert!(!has_table(&plain, b"COLR") && !has_table(&plain, b"CPAL"));
        assert!(plain.glyph_order.iter().all(|n| !n.contains(".color")), "no layer glyphs: {:?}", plain.glyph_order);
        let font = FontRef::new(&plain.bytes).unwrap();
        let a = font.charmap().map('A').unwrap();
        let b = font.glyph_metrics(Size::unscaled(), LocationRef::default()).bounds(a).unwrap();
        assert_eq!((b.x_min, b.y_max), (50.0, 700.0), "the master layer stays as the outline");

        // An interpolated instance would need interpolated colour layers: refused, unless no
        // colour table is asked for.
        Arc::make_mut(&mut f.instances).push(Instance { name: "Between".into(), location: vec![500.0], ..Default::default() });
        let refused = compile(&f, &Target::Instance(2), Flavour::TrueType, &Options::default());
        assert!(matches!(&refused, Err(crate::CompileError::ColorRefused(m)) if m.contains("Between")), "{:?}", refused.map(|c| c.glyph_order));

        // A font without colour layers gets no colour tables, palettes or not.
        let mut bare = Font::new("Bare");
        Arc::make_mut(&mut bare.info).params.0.push(to_param(&Palettes::new(vec![vec![Rgba::BLACK]]).unwrap()));
        let out = compile(&bare, &bare.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
        assert!(!has_table(&out, b"CPAL") && !has_table(&out, b"COLR"), "CPAL only with a table that uses it");
    }

    #[test]
    fn a_layered_font_with_master_colors_compiles_to_colr() {
        let mut f = Font::new("Layered");
        Arc::make_mut(&mut f.info).params.0.push(Param::new(COLOR_LAYERS_TO_COLR, Value::Bool(true)));
        let masters = Arc::make_mut(&mut f.masters);
        masters[0].params.0.push(Param::new(MASTER_COLOR, Value::String(Str::new("255,0,0,255"))));
        let mut shade = Master { id: "m02".into(), name: "Shade".into(), ..masters[0].clone() };
        shade.params.0.retain(|p| p.name != MASTER_COLOR);
        shade.params.0.push(Param::new(MASTER_COLOR, Value::String(Str::new("0,0,255,128"))));
        masters.push(shade);
        let names: Vec<String> = f.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
        for name in names {
            let mut g = f.glyph_mut(&name).unwrap();
            let mut copy = g.layers[0].clone();
            copy.id = "m02".into();
            copy.master = "m02".into();
            g.layers.push(copy);
        }
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        a.layers[1].shapes.push(Shape::Path(Path::rect(10.0, -10.0, 110.0, 90.0)));
        drop(a);

        let out = compile(&f, &Target::Master("m01".into()), Flavour::TrueType, &Options::default()).unwrap();
        let records = colr_records(&out);
        let a = records.iter().find(|(base, _)| base == "A").unwrap();
        // The first master on top: the second master's layer is painted first. The master
        // colours become palette rows, the first used first.
        let palettes = cpal_palettes(&out);
        assert_eq!(palettes, [vec![(0, 0, 255, 128), (255, 0, 0, 255)]]);
        assert_eq!(a.1, [("A.color0".to_string(), 0), ("A.color1".to_string(), 1)]);
        let font = FontRef::new(&out.bytes).unwrap();
        let b = font.glyph_metrics(Size::unscaled(), LocationRef::default()).bounds(GlyphId::new(u32::from(gid(&out, "A.color0")))).unwrap();
        assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (10.0, -10.0, 110.0, 90.0), "drawn from the second master's layer");

        // Without the switch the layered font exports plainly.
        Arc::make_mut(&mut f.info).params.0.retain(|p| p.name != COLOR_LAYERS_TO_COLR);
        let plain = compile(&f, &Target::Master("m01".into()), Flavour::TrueType, &Options::default()).unwrap();
        assert!(!has_table(&plain, b"COLR"));
    }

    /// `a` drawn in two palette colours by the source glyphs `a.color0` and `a.color1`, each held
    /// by a colour layer as one unmoved component, as a UFO's font-wide colour layer list opens;
    /// a second master at weight 200 has no colour layers.
    fn listed_font() -> Font {
        let mut f = Font::new("Listed");
        let palettes = Palettes::new(vec![vec![Rgba::new(255, 0, 0, 255), Rgba::new(0, 0, 255, 255)]]).unwrap();
        Arc::make_mut(&mut f.info).params.0.push(to_param(&palettes));
        for (name, x) in [("a", 100.0), ("a.color0", 100.0), ("a.color1", 300.0)] {
            if f.glyph(name).is_none() {
                f.add_glyph(name, Vec::new(), 600.0).unwrap();
            }
            let mut g = f.glyph_mut(name).unwrap();
            g.layers[0].width = 600.0;
            g.layers[0].shapes.push(Shape::Path(Path::rect(x, 0.0, x + 200.0, 700.0)));
        }
        let mut a = f.glyph_mut("a").unwrap();
        a.layers.push(colour_layer("a-0", "m01", PaletteEntry::Index(0), vec![Shape::Component(Component::new("a.color0", 0.0, 0.0))]));
        a.layers.push(colour_layer("a-1", "m01", PaletteEntry::Index(1), vec![Shape::Component(Component::new("a.color1", 0.0, 0.0))]));
        drop(a);
        f
    }

    /// The paints of a version 1 colour glyph as text: `glyph <name>` for each clip by a layer
    /// glyph, then the fill under it.
    struct PaintLog<'a> {
        names: &'a [String],
        events: Vec<String>,
    }

    impl ColorPainter for PaintLog<'_> {
        fn push_transform(&mut self, t: Transform) {
            self.events.push(format!("transform {t:?}"));
        }
        fn pop_transform(&mut self) {}
        fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
            self.events.push(format!("glyph {}", self.names[glyph_id.to_u32() as usize]));
        }
        fn push_clip_box(&mut self, _: BoundingBox<f32>) {}
        fn pop_clip(&mut self) {}
        fn fill(&mut self, brush: Brush<'_>) {
            let stops = |s: &[skrifa::color::ColorStop]| {
                s.iter().map(|s| format!("{} {} {}", s.offset, s.palette_index, s.alpha)).collect::<Vec<_>>().join(", ")
            };
            self.events.push(match brush {
                Brush::Solid { palette_index, alpha } => format!("solid {palette_index} {alpha}"),
                Brush::LinearGradient { p0, p1, color_stops, .. } => format!("linear {} {} {} {} [{}]", p0.x, p0.y, p1.x, p1.y, stops(color_stops)),
                other => format!("{other:?}"),
            });
        }
        fn push_layer(&mut self, _: CompositeMode) {}
    }

    /// `A` has a full-colour layer: a square with a linear gradient, two half-transparent green
    /// squares (one run), and a rectangle with only a 20-unit stroke in palette colour 1. `B`'s
    /// full-colour layer is one square in palette colour 0.
    fn full_colour_font() -> Font {
        let mut f = Font::new("Full Colour");
        let palettes = Palettes::new(vec![vec![Rgba::new(255, 0, 0, 255), Rgba::new(0, 0, 255, 255)]]).unwrap();
        Arc::make_mut(&mut f.info).params.0.push(to_param(&palettes));
        let full = |id: &str, shapes: Vec<Shape>| {
            let mut l = Layer::new("m01", 600.0);
            l.id = id.to_string();
            l.info.attrs = with_kind(&LayerAttrs::default(), Some(ColorLayerKind::FullColor));
            l.shapes = shapes;
            l
        };
        let styled = |rect: [f64; 4], style: &dyn Fn(&mut fontcraft_model::ShapeAttrs)| {
            let mut p = Path::rect(rect[0], rect[1], rect[2], rect[3]);
            style(&mut p.info.attrs);
            Shape::Path(p)
        };
        let green = Color::Rgba([0.0, 1.0, 0.0, 128.0 / 255.0]);
        let gradient = Gradient {
            kind: GradientKind::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 1.0) },
            stops: vec![
                GradientStop { offset: 0.0, color: Color::Palette { index: 0, alpha: 1.0 } },
                GradientStop { offset: 1.0, color: Color::Palette { index: 1, alpha: 0.5 } },
            ],
            extend: Extend::Pad,
        };
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].width = 600.0;
        a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 500.0, 600.0)));
        a.layers.push(full(
            "A-full",
            vec![
                styled([100.0, 100.0, 500.0, 400.0], &|s| s.gradient = Some(gradient.clone())),
                styled([0.0, 0.0, 100.0, 100.0], &|s| s.fill_color = Some(green)),
                styled([200.0, 0.0, 300.0, 100.0], &|s| s.fill_color = Some(green)),
                styled([100.0, 500.0, 300.0, 600.0], &|s| {
                    s.stroke_width = Some(Dimension::Number(20.0));
                    s.stroke_color = Some(Color::Palette { index: 1, alpha: 1.0 });
                }),
            ],
        ));
        drop(a);
        let mut b = f.glyph_mut("B").unwrap();
        b.layers[0].width = 600.0;
        b.layers.push(full("B-full", vec![styled([0.0, 0.0, 400.0, 400.0], &|s| s.fill_color = Some(Color::Palette { index: 0, alpha: 1.0 }))]));
        drop(b);
        f
    }

    #[test]
    fn colour_layers_drawn_by_source_glyphs_use_those_glyphs() {
        let f = listed_font();
        let want = [("a".to_string(), vec![("a.color0".to_string(), 0), ("a.color1".to_string(), 1)])];
        let out = compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
        assert_eq!(out.glyph_order.len(), f.glyphs.len(), "no layer glyph is made: {:?}", out.glyph_order);
        assert_eq!(colr_records(&out), want);
        assert_eq!(cpal_palettes(&out), [vec![(255, 0, 0, 255), (0, 0, 255, 255)]]);

        // A variable font: the layer glyphs vary as any glyph, so the tables are written.
        let mut v = listed_font();
        v.axes = Arc::new(vec![fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
        let masters = Arc::make_mut(&mut v.masters);
        masters[0].location = vec![400.0];
        let mut bold = masters[0].clone();
        bold.id = "m02".into();
        bold.name = "Bold".into();
        bold.location = vec![700.0];
        masters.push(bold);
        for i in 0..v.glyphs.len() {
            let mut g = v.glyphs.get_mut_at(i).unwrap();
            let mut layer = g.layers[0].clone();
            layer.id = "m02".into();
            layer.master = "m02".into();
            g.layers.push(layer);
        }
        let var = crate::var::compile_variable(&v, None, &Options::default(), &crate::var::VariableOptions::default()).unwrap();
        assert_eq!(colr_records(&var.compiled), want, "{:?}", var.compiled.warnings);
        assert_eq!(cpal_palettes(&var.compiled), [vec![(255, 0, 0, 255), (0, 0, 255, 255)]]);
    }

    #[test]
    fn full_colour_layers_compile_to_colr_version_1_with_strokes_outlined_and_clip_boxes() {
        let f = full_colour_font();
        let out = compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
        let n = out.glyph_order.len();
        assert_eq!(&out.glyph_order[n - 4..], ["A.color0", "A.color1", "A.color2", "B.color0"], "one layer glyph per run, the stroke its own");
        let font = FontRef::new(&out.bytes).unwrap();
        let colr = font.colr().unwrap();
        assert_eq!(colr.version(), 1, "A's gradient needs version 1");

        // B is a stack of solid layers, so it keeps a version 0 record, as fontc writes it.
        let name = |g: skrifa::raw::types::GlyphId16| out.glyph_order[usize::from(g.to_u16())].clone();
        let records: Vec<(String, u16)> = colr.base_glyph_records().unwrap().unwrap().iter().map(|b| (name(b.glyph_id()), b.num_layers())).collect();
        assert_eq!(records, [("B".to_string(), 1)]);
        let layers = colr.layer_records().unwrap().unwrap();
        assert_eq!((name(layers[0].glyph_id()), layers[0].palette_index()), ("B.color0".to_string(), 0));

        // A paints its layer glyphs bottom first; the green is a new palette row with its alpha in
        // CPAL, the gradient keeps the palette reference's own alpha.
        let a = GlyphId::new(u32::from(gid(&out, "A")));
        let glyph = font.color_glyphs().get_with_format(a, ColorGlyphFormat::ColrV1).unwrap();
        let mut log = PaintLog { names: &out.glyph_order, events: Vec::new() };
        glyph.paint(LocationRef::default(), &mut log).unwrap();
        assert_eq!(
            log.events,
            ["glyph A.color0", "linear 200 250 400 400 [0 0 1, 1 1 0.5]", "glyph A.color1", "solid 2 1", "glyph A.color2", "solid 1 1",]
        );
        assert_eq!(cpal_palettes(&out), [vec![(255, 0, 0, 255), (0, 0, 255, 255), (0, 255, 0, 128)]]);

        // The clip box is the layer glyphs' bounds widened to multiples of 100 units.
        let clip = glyph.bounding_box(LocationRef::default(), Size::unscaled()).unwrap();
        assert_eq!((clip.x_min, clip.y_min, clip.x_max, clip.y_max), (0.0, 0.0, 500.0, 700.0));

        // The stroke layer glyph is the centred stroke alone; the fill layer glyphs are the paths.
        let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        let bounds = |name: &str| {
            let b = metrics.bounds(GlyphId::new(u32::from(gid(&out, name)))).unwrap();
            (b.x_min, b.y_min, b.x_max, b.y_max)
        };
        assert_eq!(bounds("A.color1"), (0.0, 0.0, 300.0, 100.0));
        assert_eq!(bounds("A.color2"), (90.0, 490.0, 310.0, 610.0));

        // Without the gradient every glyph is a solid stack: version 0.
        let mut solid = f.clone();
        solid.glyph_mut("A").unwrap().layers[1].shapes.remove(0);
        let out = compile(&solid, &solid.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
        let records = colr_records(&out);
        assert_eq!(records.iter().map(|(base, layers)| (base.as_str(), layers.len())).collect::<Vec<_>>(), [("A", 2), ("B", 1)]);
    }

    #[test]
    fn a_variable_build_plans_no_colour_tables() {
        let switches = Switches::default();
        let plan = variable_plan(Some(&palette_font()), "m01", &switches);
        assert!(plan.tables().is_empty());
        assert_eq!(plan.problems.len(), 1);
        assert!(variable_plan(Some(&Font::new("Plain")), "m01", &switches).problems.is_empty());
    }

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
