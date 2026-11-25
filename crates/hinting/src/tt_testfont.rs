//! Test fonts built in memory for the interpreter tests: one glyph with a single contour of
//! on-curve points, our `cvt `/`fpgm`/`prep` and a glyph program, run through skrifa's TrueType
//! interpreter. Shared by the font-program, glyph-program and autohinter tests.

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use write_fonts::FontBuilder;
use write_fonts::tables::glyf::{Bbox, Contour, GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::head::{Flags, Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::LocaFormat;
use write_fonts::tables::maxp::Maxp;
use write_fonts::types::{Fixed, LongDateTime, Tag};

use crate::{FontPrograms, TtAutohintOptions};

pub const UPEM: u16 = 1000;

/// The rendering targets of the three intents.
pub use skrifa::outline::Target;

pub fn mono() -> Target {
    Target::Mono
}
pub fn grayscale() -> Target {
    Target::Smooth { mode: SmoothMode::Normal, symmetric_rendering: true, preserve_linear_metrics: false }
}
pub fn gdi() -> Target {
    Target::Smooth { mode: SmoothMode::Lcd, symmetric_rendering: false, preserve_linear_metrics: false }
}

/// A font whose glyph 1 is one contour of straight lines through `points` (font units) with
/// `glyph_program`.
pub fn font(programs: &FontPrograms, points: &[(i16, i16)], glyph_program: &[u8]) -> Vec<u8> {
    font_of(programs, &points.iter().map(|&(x, y)| (x, y, true)).collect::<Vec<_>>(), glyph_program)
}

/// A font whose glyph 1 is one contour through `points` (font units, whether on the curve) with
/// `glyph_program`. `maxp` holds `programs.limits` merged with the glyph program's, less the 32 stack slots
/// skrifa adds of its own, so a stack the analysis under-counts overflows.
fn font_of(programs: &FontPrograms, points: &[(i16, i16, bool)], glyph_program: &[u8]) -> Vec<u8> {
    use skrifa::raw::tables::glyf::CurvePoint;
    let contour: Vec<_> = points.iter().map(|&(x, y, on)| if on { CurvePoint::on_curve(x, y) } else { CurvePoint::off_curve(x, y) }).collect();
    let bbox = Bbox {
        x_min: points.iter().map(|p| p.0).min().unwrap_or(0),
        y_min: points.iter().map(|p| p.1).min().unwrap_or(0),
        x_max: points.iter().map(|p| p.0).max().unwrap_or(0),
        y_max: points.iter().map(|p| p.1).max().unwrap_or(0),
    };
    let glyph = SimpleGlyph { bbox, contours: vec![Contour::from(contour)], instructions: glyph_program.to_vec(), overlaps: false };
    let mut glyf = GlyfLocaBuilder::new();
    glyf.add_glyph(&Glyph::Empty).unwrap();
    glyf.add_glyph(&glyph).unwrap();
    let (glyf, loca, format) = glyf.build();

    let functions = fontcraft_ttasm::FunctionTable::from_fpgm(&programs.fpgm).unwrap();
    let mut limits = fontcraft_ttasm::analyse(glyph_program, &functions).unwrap();
    limits.merge(&programs.limits);
    assert!(!limits.uncertain, "the analysis should follow every program: {limits:?}");
    let n = u16::try_from(points.len()).unwrap();
    let maxp = Maxp {
        num_glyphs: 2,
        max_points: Some(n),
        max_contours: Some(1),
        max_composite_points: Some(0),
        max_composite_contours: Some(0),
        max_zones: Some(limits.max_zones.max(1)),
        max_twilight_points: Some(limits.max_twilight_points),
        max_storage: Some(limits.max_storage),
        max_function_defs: Some(limits.max_function_defs),
        max_instruction_defs: Some(limits.max_instruction_defs),
        max_stack_elements: Some(limits.max_stack_elements.saturating_sub(32)),
        max_size_of_instructions: Some(u16::try_from(glyph_program.len()).unwrap()),
        max_component_elements: Some(0),
        max_component_depth: Some(0),
    };
    let long = format == LocaFormat::Long;
    let head = Head::new(
        Fixed::ONE,
        0,
        Flags::empty(),
        UPEM,
        LongDateTime::new(0),
        LongDateTime::new(0),
        0,
        -300,
        1000,
        1000,
        MacStyle::empty(),
        6,
        i16::from(long),
    );
    let hhea = Hhea::new(800.into(), (-200).into(), 0.into(), 1000.into(), 0.into(), 0.into(), 1000.into(), 1, 0, 0, 2);
    let hmtx = Hmtx::new(vec![LongMetric::new(500, 0), LongMetric::new(1000, 0)], vec![]);
    let cvt: Vec<u8> = programs.cvt.iter().flat_map(|v| v.to_be_bytes()).collect();
    let mut b = FontBuilder::new();
    b.add_table(&head).unwrap();
    b.add_table(&hhea).unwrap();
    b.add_table(&maxp).unwrap();
    b.add_table(&hmtx).unwrap();
    b.add_table(&glyf).unwrap();
    b.add_table(&loca).unwrap();
    b.add_raw(Tag::new(b"cvt "), cvt);
    b.add_raw(Tag::new(b"fpgm"), programs.fpgm.clone());
    b.add_raw(Tag::new(b"prep"), programs.prep.clone());
    b.build()
}

/// A font whose glyph 1 is the one contour of lines and quadratic curves `glyph` has on master `m01` of
/// `model`, hinted as an autohinted export hints it.
pub fn autohinted(model: &fontcraft_model::Font, glyph: &str, options: &TtAutohintOptions) -> Vec<u8> {
    use fontcraft_model::Shape;
    let mut settings = crate::HintSettings::for_master(model, "m01").unwrap();
    let measured = crate::prepare_settings(model, &mut settings, options);
    let layer = model.glyph(glyph).unwrap().layer("m01").unwrap();
    let Some(Shape::Path(path)) = layer.shapes.first() else { panic!("`{glyph}` has no path") };
    let points: Vec<(i16, i16, bool)> = path.nodes.iter().map(|n| (n.x as i16, n.y as i16, n.on_curve())).collect();
    let outline: Vec<_> = path.nodes.iter().map(|n| crate::OutlinePoint { x: n.x, y: n.y, on_curve: n.on_curve() }).collect();
    let n = u16::try_from(points.len()).unwrap();
    let map = crate::VecPointMap { paths: vec![(0..n).map(Some).collect()], count: n };
    let control = crate::control::ControlInstructions::default();
    let crate::GlyphStyle::Group(group) = measured.styles.of(model, glyph, options) else { panic!("`{glyph}` is not hinted") };
    let zones = measured.styles.zones_of(group, &(0..settings.tt_zones.len()).collect::<Vec<_>>());
    let hints = crate::autohint_glyph(&crate::AutoGlyph {
        glyph,
        layer,
        settings: &settings,
        zones: Some(&zones),
        stems: None,
        options,
        control: &control,
        points: Some(&map),
        delta_column: None,
    })
    .unwrap();

    let mut programs = crate::font_programs(&settings, &[], "m01").unwrap();
    programs.fpgm.extend(crate::autohint_fpgm().unwrap());
    let functions = fontcraft_ttasm::FunctionTable::from_fpgm(&programs.fpgm).unwrap();
    programs.limits.merge(&fontcraft_ttasm::analyse(&programs.fpgm, &functions).unwrap());
    let (cvt, prep) = crate::autohint_prep(options, &settings, &programs.layout, &measured).unwrap();
    programs.limits.merge(&fontcraft_ttasm::analyse(&prep, &functions).unwrap());
    programs.cvt.extend(cvt);
    programs.prep.extend(prep);
    let program = crate::glyph_program(&crate::GlyphContext {
        glyph,
        instructions: &hints.instructions,
        points: &map,
        outline: crate::GlyphOutline { points: &outline, contour_ends: &[n - 1], advance: 1000.0 },
        layout: &programs.layout,
        functions: &functions,
        settings: &settings,
        zones: Some(&zones),
        stems: None,
        delta_column: None,
        auto_stems: true,
        auto_group: group,
    })
    .unwrap();
    font_of(&programs, &points, &program.bytes)
}

#[derive(Default)]
struct Points(Vec<(f32, f32)>);

impl OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn close(&mut self) {}
}

/// Glyph 1's points in pixels after hinting at `ppem`; panics when a program fails.
pub fn hinted(font: &[u8], ppem: u16, target: Target) -> Vec<(f32, f32)> {
    let font = FontRef::new(font).unwrap();
    let outlines = font.outline_glyphs();
    let options = HintingOptions { engine: Engine::Interpreter, target };
    let instance = HintingInstance::new(&outlines, Size::new(f32::from(ppem)), LocationRef::default(), options)
        .unwrap_or_else(|e| panic!("fpgm or prep failed at {ppem} PPM: {e}"));
    let glyph = outlines.get(GlyphId::new(1)).unwrap();
    let mut pen = Points::default();
    glyph.draw(DrawSettings::hinted(&instance, true), &mut pen).unwrap_or_else(|e| panic!("the glyph program failed at {ppem} PPM: {e}"));
    pen.0
}

/// The y of every point after hinting.
pub fn hinted_y(font: &[u8], ppem: u16, target: Target) -> Vec<f32> {
    hinted(font, ppem, target).into_iter().map(|p| p.1).collect()
}
