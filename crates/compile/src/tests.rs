use std::sync::Arc;

use super::*;
use fontcraft_geom::{Affine, BezPath, CubicBez, ParamCurve, ParamCurveNearest, PathEl, Point};
use fontcraft_model::{Component, Glyph, Layer, Node, NodeKind, Path, ResolvedComponent, ResolvedGlyph, Shape, Transform};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::{GlyphId, GlyphId16};
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};

use crate::outline::{Prepared, Settings, to_quadratic};

const HEAD_EPOCH_OFFSET: i64 = 2_082_844_800;

fn path(closed: bool, nodes: Vec<Node>) -> Shape {
    Shape::Path(Path::new(closed, nodes))
}

fn sample() -> Font {
    let mut f = Font::new("Slice Sans");
    let n = Node::new;
    let mut a = f.glyph_mut("A").unwrap();
    a.layers[0].width = 620.0;
    a.layers[0].shapes.push(Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0)));
    drop(a);
    let mut o = f.glyph_mut("O").unwrap();
    // A four-curve oval: cubic input must come out as quadratic TrueType curves.
    let k = 0.5523;
    let (cx, cy, rx, ry) = (350.0, 350.0, 300.0, 360.0);
    o.layers[0].width = 700.0;
    o.layers[0].shapes.push(path(
        true,
        vec![
            n(cx + rx, cy, NodeKind::Curve),
            n(cx + rx, cy + ry * k, NodeKind::OffCurve),
            n(cx + rx * k, cy + ry, NodeKind::OffCurve),
            n(cx, cy + ry, NodeKind::Curve),
            n(cx - rx * k, cy + ry, NodeKind::OffCurve),
            n(cx - rx, cy + ry * k, NodeKind::OffCurve),
            n(cx - rx, cy, NodeKind::Curve),
            n(cx - rx, cy - ry * k, NodeKind::OffCurve),
            n(cx - rx * k, cy - ry, NodeKind::OffCurve),
            n(cx, cy - ry, NodeKind::Curve),
            n(cx + rx * k, cy - ry, NodeKind::OffCurve),
            n(cx + rx, cy - ry * k, NodeKind::OffCurve),
        ],
    ));
    drop(o);
    f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 10.0, 0.0)));
    f
}

fn names(font: &FontRef, id: StringId) -> Vec<String> {
    font.localized_strings(id).map(|s| s.to_string()).collect()
}

fn gid_of(out: &Compiled, name: &str) -> GlyphId {
    GlyphId::new(out.glyph_order.iter().position(|n| n == name).unwrap() as u32)
}

/// The glyph as stored in `glyf`.
fn raw<'a>(font: &FontRef<'a>, gid: GlyphId) -> Option<RawGlyph<'a>> {
    let loca = font.loca(None).unwrap();
    loca.get_glyf(gid, &font.glyf().unwrap()).unwrap()
}

/// The contours skrifa draws for a glyph, as point lists (on-curve and off-curve alike).
#[derive(Default)]
struct Points(Vec<Vec<(f32, f32)>>);
impl OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(vec![(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.last_mut().unwrap().push((x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.last_mut().unwrap().extend([(cx, cy), (x, y)]);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.0.last_mut().unwrap().push((x, y));
    }
    fn close(&mut self) {}
}

fn drawn(font: &FontRef, gid: GlyphId) -> Vec<Vec<(f32, f32)>> {
    let glyph = font.outline_glyphs().get(gid).unwrap();
    let mut pen = Points::default();
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
    pen.0
}

/// Twice the signed area of a closed point list: positive when counter-clockwise.
fn area(points: &[(f32, f32)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            f64::from(a.0) * f64::from(b.1) - f64::from(b.0) * f64::from(a.1)
        })
        .sum()
}

fn compile_sample(f: &Font) -> Compiled {
    compile_ttf(f, &Options::default()).unwrap()
}

#[test]
fn the_compiled_font_reads_back() {
    let out = compile_sample(&sample());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(out.glyph_order.first().map(String::as_str), Some(".notdef"));
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(font.head().unwrap().units_per_em(), 1000);
    assert_eq!(font.head().unwrap().flags().bits(), 0x0003);
    assert_eq!(usize::from(font.maxp().unwrap().num_glyphs()), out.glyph_order.len());
    let gid = font.charmap().map('A').unwrap();
    assert_eq!(out.glyph_order[gid.to_u32() as usize], "A");
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    assert_eq!(metrics.advance_width(gid), Some(620.0));
    assert_eq!(metrics.left_side_bearing(gid), Some(60.0));
    let bounds = metrics.bounds(gid).unwrap();
    assert_eq!((bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max), (60.0, 0.0, 560.0, 700.0));
    // The component is kept as a composite, and its box is the shifted box of `A`.
    let b = font.charmap().map('B').unwrap();
    assert!(matches!(raw(&font, b), Some(RawGlyph::Composite(_))));
    let bb = metrics.bounds(b).unwrap();
    assert_eq!((bb.x_min, bb.x_max), (70.0, 570.0));
    assert_eq!(metrics.left_side_bearing(b), Some(70.0));
    // The oval kept its extremes through the cubic → quadratic conversion.
    let o = metrics.bounds(font.charmap().map('O').unwrap()).unwrap();
    assert_eq!((o.x_min, o.y_min, o.x_max, o.y_max), (50.0, -10.0, 650.0, 710.0));
    assert_eq!(names(&font, StringId::FAMILY_NAME), ["Slice Sans"]);
    assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Regular"]);
    let os2 = font.os2().unwrap();
    assert_eq!(os2.s_cap_height(), Some(700));
    assert_eq!(os2.us_break_char(), Some(0x20));
    assert_eq!(os2.ul_unicode_range_1() & 1, 1, "Basic Latin is covered");
    assert_eq!(font.post().unwrap().glyph_name(GlyphId16::new(gid.to_u32() as u16)), Some("A"));
    let maxp = font.maxp().unwrap();
    assert_eq!((maxp.max_component_elements(), maxp.max_component_depth()), (Some(1), Some(1)));
    assert_eq!((maxp.max_composite_points(), maxp.max_composite_contours()), (Some(4), Some(1)));
}

#[test]
fn compiling_is_deterministic_and_the_timestamp_is_an_input() {
    let font = sample();
    let a = compile_sample(&font);
    let b = compile_sample(&font);
    assert_eq!(a.bytes, b.bytes);
    let later = compile_ttf(&font, &Options { timestamp: 1_800_000_000, ..Default::default() }).unwrap();
    assert_ne!(a.bytes, later.bytes);
    let head = FontRef::new(&later.bytes).unwrap().head().unwrap().created().as_secs();
    assert_eq!(head, 1_800_000_000 + HEAD_EPOCH_OFFSET);
}

#[test]
fn problems_are_errors_with_the_glyph_named() {
    let mut f = sample();
    f.glyph_mut("C").unwrap().unicodes = vec![0x41];
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::DuplicateCodepoint { codepoint: 0x41, .. })));

    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(path(true, vec![Node::new(0.0, 0.0, NodeKind::Curve)]));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Model(ModelError::BadOutline { .. }))));
}

#[test]
fn a_font_with_nothing_to_export_still_builds_its_notdef() {
    // fontc builds such sources: glyph 0 alone.
    let mut f = sample();
    for i in 0..f.glyphs.len() {
        if let Some(mut g) = f.glyphs.get_mut_at(i) {
            g.export = false;
        }
    }
    let out = compile_sample(&f);
    assert_eq!(out.glyph_order, [".notdef"]);
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(font.maxp().unwrap().num_glyphs(), 1);
}

#[test]
fn far_points_are_errors_before_any_curve_work() {
    let n = Node::new;
    // On-curve points at any distance, including ones that once exhausted memory in the
    // cubic conversion and overflowed the glyph writer.
    for far in [16_384.0, 40_000.0, 1e5, 1e6] {
        let mut f = sample();
        f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(-far, -far, far, far)));
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"), "{far}");
    }
    // A control point out of range while the curve itself stays inside.
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0]
        .shapes
        .push(path(true, vec![n(-16_000.0, 0.0, NodeKind::Line), n(30_000.0, 0.0, NodeKind::OffCurve), n(-16_000.0, 10.0, NodeKind::QCurve)]));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"));
    // A huge cubic: an error at once, not a long conversion.
    let mut f = sample();
    let big = MAX_TT_COORDINATE * 50.0;
    f.glyph_mut("C").unwrap().layers[0].shapes.push(path(
        true,
        vec![n(0.0, 0.0, NodeKind::Line), n(big, 0.0, NodeKind::OffCurve), n(0.0, big, NodeKind::OffCurve), n(big, big, NodeKind::Curve)],
    ));
    let start = std::time::Instant::now();
    assert!(compile_ttf(&f, &Options::default()).is_err());
    assert!(start.elapsed().as_millis() < 2000);
    // The largest allowed outline compiles.
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(
        -MAX_TT_COORDINATE,
        -MAX_TT_COORDINATE,
        MAX_TT_COORDINATE,
        MAX_TT_COORDINATE,
    )));
    compile_sample(&f);
    // A component placed beyond it is an error too, not a wrapped offset.
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 20_000.0, 0.0)));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"));
}

#[test]
fn odd_values_become_warnings() {
    let mut f = sample();
    f.glyph_mut("A").unwrap().layers[0].width = 70_000.0;
    f.glyph_mut("C").unwrap().layers[0]
        .shapes
        .push(path(false, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(100.0, 0.0, NodeKind::Line), Node::new(100.0, 100.0, NodeKind::Line)]));
    let out = compile_sample(&f);
    assert!(out.warnings.iter().any(|w| w.contains("advance width")));
    assert!(out.warnings.iter().any(|w| w.contains("open path")));
    assert!(FontRef::new(&out.bytes).is_ok());
}

#[test]
fn too_many_points_is_an_error_not_a_corrupt_glyph() {
    let mut f = sample();
    let nodes: Vec<Node> = (0..70_000).map(|i| Node::new(f64::from(i % 1000), f64::from(i / 1000), NodeKind::Line)).collect();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(path(true, nodes));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { message, .. }) if message.contains("points")));
}

#[test]
fn sidebearings_and_boxes_agree_with_the_glyph_header() {
    // A curve whose control point lies left of the curve itself: the glyph header stores the
    // box of all points, and lsb, hhea and head must say the same.
    let n = Node::new;
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(path(
        true,
        vec![n(100.0, 0.0, NodeKind::Line), n(-100.0, 350.0, NodeKind::OffCurve), n(100.0, 700.0, NodeKind::QCurve), n(300.0, 350.0, NodeKind::Line)],
    ));
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let gid = font.charmap().map('C').unwrap();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    assert_eq!(metrics.left_side_bearing(gid), Some(-100.0));
    assert_eq!(font.head().unwrap().x_min(), -100);
    assert_eq!(font.hhea().unwrap().min_left_side_bearing().to_i16(), -100);
}

#[test]
fn unusual_glyph_names_are_replaced_in_the_font_not_written_raw() {
    let mut f = sample();
    f.add_glyph(&"n".repeat(200), vec![0xE000], 500.0).unwrap();
    f.add_glyph("with space", vec![0xE001], 500.0).unwrap();
    f.add_glyph("ça", vec![0xE002], 500.0).unwrap();
    let out = compile_sample(&f);
    assert_eq!(out.warnings.iter().filter(|w| w.contains("not a valid glyph name")).count(), 3);
    let font = FontRef::new(&out.bytes).unwrap();
    let post = font.post().unwrap();
    for ch in ['\u{E000}', '\u{E001}', '\u{E002}'] {
        let gid = font.charmap().map(ch).unwrap().to_u32();
        assert_eq!(post.glyph_name(GlyphId16::new(gid as u16)), Some(format!("glyph{gid:05}").as_str()));
    }
    // The glyph after them still has its own name: the table is not shifted.
    assert_eq!(post.glyph_name(GlyphId16::new(font.charmap().map('z').unwrap().to_u32() as u16)), Some("z"));
}

#[test]
fn more_glyphs_than_post_can_name_fall_back_to_a_nameless_table() {
    let mut f = Font::new("Many");
    let master = f.masters[0].id.clone();
    for i in 0..(MAX_POST_NAMES as u32 + 100) {
        let mut g = Glyph::new(format!("glyph{i:05}x"));
        g.layers.push(Layer::new(&master, 500.0));
        f.glyphs.push(g);
    }
    let out = compile_sample(&f);
    assert!(out.warnings.iter().any(|w| w.contains("glyph names were left out")));
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(font.post().unwrap().version(), write_fonts::types::Version16Dot16::VERSION_3_0);
    assert_eq!(usize::from(font.maxp().unwrap().num_glyphs()), f.glyphs.len());
}

#[test]
fn style_names_drive_style_linking() {
    let compile = |style: &str| {
        let mut f = sample();
        Arc::make_mut(&mut f.masters)[0].name = style.to_string();
        compile_sample(&f).bytes
    };
    let bytes = compile("Bold Italic");
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.os2().unwrap().fs_selection().bits() & 0x61, 0x21, "ITALIC and BOLD set, REGULAR clear");
    assert_eq!(font.head().unwrap().mac_style().bits(), 0x3);
    assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Bold Italic"]);
    assert!(names(&font, StringId::TYPOGRAPHIC_FAMILY_NAME).is_empty());

    // A style outside the four linked ones is its own legacy family.
    let bytes = compile("Light");
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.os2().unwrap().fs_selection().bits() & 0x61, 0x40, "REGULAR only");
    assert_eq!(names(&font, StringId::FAMILY_NAME), ["Slice Sans Light"]);
    assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Regular"]);
    assert_eq!(names(&font, StringId::TYPOGRAPHIC_FAMILY_NAME), ["Slice Sans"]);
    assert_eq!(names(&font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME), ["Light"]);
}

#[test]
fn a_font_without_a_space_has_no_break_character() {
    let mut f = sample();
    f.remove_glyph("space").unwrap();
    let out = compile_sample(&f);
    assert_eq!(FontRef::new(&out.bytes).unwrap().os2().unwrap().us_break_char(), Some(0));
}

// --- Resolve: the target picks the layers and the names -------------------------------------

fn two_masters() -> Font {
    let mut f = sample();
    f.axes = Arc::new(vec![fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    let masters = Arc::make_mut(&mut f.masters);
    masters[0].location = vec![100.0];
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    bold.location = vec![200.0];
    masters.push(bold);
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let mut layer = g.layers[0].clone();
        layer.id = "m02".into();
        layer.master = "m02".into();
        layer.width += 100.0;
        g.layers.push(layer);
    }
    f
}

#[test]
fn a_target_compiles_its_own_master_and_an_instance_at_a_master() {
    let f = two_masters();
    let bold = compile(&f, &Target::Master("m02".into()), Flavour::TrueType, &Options::default()).unwrap();
    let font = FontRef::new(&bold.bytes).unwrap();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    assert_eq!(metrics.advance_width(font.charmap().map('A').unwrap()), Some(720.0));
    assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Bold"]);

    let mut f = two_masters();
    Arc::make_mut(&mut f.instances).push(fontcraft_model::Instance { name: "Heavy".into(), location: vec![200.0], ..Default::default() });
    Arc::make_mut(&mut f.instances).push(fontcraft_model::Instance { name: "Semibold".into(), location: vec![150.0], ..Default::default() });
    let heavy = compile(&f, &Target::Instance(0), Flavour::TrueType, &Options::default()).unwrap();
    let font = FontRef::new(&heavy.bytes).unwrap();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    assert_eq!(metrics.advance_width(font.charmap().map('A').unwrap()), Some(720.0), "the master's layer");
    assert_eq!(names(&font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME), ["Heavy"]);
    // Between masters: reported, not interpolated.
    assert!(matches!(
        compile(&f, &Target::Instance(1), Flavour::TrueType, &Options::default()),
        Err(CompileError::Model(ModelError::BetweenMasters(name))) if name == "Semibold"
    ));
    assert!(matches!(
        compile(&f, &Target::Instance(9), Flavour::TrueType, &Options::default()),
        Err(CompileError::Model(ModelError::NoSuchInstance(9)))
    ));
    assert!(matches!(compile(&f, &Target::Master("m02".into()), Flavour::Cff, &Options::default()), Err(CompileError::Unsupported(_))));
}

// --- Preflight -----------------------------------------------------------------------------

fn rg(name: &str, contours: Vec<BezPath>, components: Vec<(&str, Affine)>) -> ResolvedGlyph {
    let mut g = ResolvedGlyph::new(name, 500.0);
    g.contours = contours;
    g.components = components.into_iter().map(|(base, transform)| ResolvedComponent { base: base.into(), transform }).collect();
    g
}

fn square(x: f64, y: f64, size: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + size, y));
    p.line_to((x + size, y + size));
    p.line_to((x, y + size));
    p.line_to((x, y));
    p.close_path();
    p
}

#[test]
fn missing_components_are_dropped_with_a_warning() {
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("missing", 0.0, 0.0)));
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
    let out = compile_sample(&f);
    assert_eq!(out.warnings.iter().filter(|w| w.contains("`missing`")).count(), 1, "{:?}", out.warnings);
    // Its sibling is still a component: the missing one did not get the glyph decomposed.
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(matches!(raw(&font, font.charmap().map('C').unwrap()), Some(RawGlyph::Composite(c)) if c.components().count() == 1));
}

#[test]
fn glyphs_not_exported_are_inlined_where_used_then_removed() {
    let mut glyphs = vec![
        rg("_part", vec![square(0.0, 0.0, 100.0)], vec![("dot", Affine::translate((5.0, 5.0)))]),
        rg("dot", vec![square(0.0, 0.0, 10.0)], vec![]),
        rg("user", vec![], vec![("_part", Affine::translate((200.0, 0.0))), ("dot", Affine::IDENTITY)]),
        rg("mirror", vec![], vec![("_part", Affine::scale_non_uniform(-1.0, 1.0))]),
    ];
    glyphs[0].export = false;
    let mut warnings = Vec::new();
    let out = preflight::run(glyphs, &mut warnings).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(out.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["dot", "user", "mirror"]);
    let user = &out[1];
    // The hidden glyph's contour moved in; its component and the glyph's own stay components,
    // in their places, with the transforms combined.
    assert_eq!(user.contours.len(), 1);
    assert_eq!(user.contours[0].elements()[0], PathEl::MoveTo(Point::new(200.0, 0.0)));
    assert_eq!(
        user.components.iter().map(|c| (c.base.as_str(), c.transform)).collect::<Vec<_>>(),
        [("dot", Affine::translate((205.0, 5.0))), ("dot", Affine::IDENTITY)]
    );
    // A mirroring transform reverses the inlined contour, so it keeps its direction.
    let mirrored = &out[2].contours[0];
    let pts: Vec<(f32, f32)> = outline::points(mirrored).map(|p| (p.x as f32, p.y as f32)).collect();
    assert!(area(&pts) > 0.0, "still counter-clockwise: {pts:?}");
}

#[test]
fn component_cycles_and_deep_nesting_are_errors() {
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("D", 0.0, 0.0)));
    f.glyph_mut("D").unwrap().layers[0].shapes.push(Shape::Component(Component::new("C", 0.0, 0.0)));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Model(ModelError::ComponentCycle(_)))));

    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("C", 0.0, 0.0)));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Model(ModelError::ComponentCycle(_)))));

    // A chain longer than the limit, without a cycle.
    let mut f = sample();
    let depth = fontcraft_model::MAX_COMPONENT_DEPTH + 5;
    for i in 0..depth {
        f.add_glyph(&format!("chain{i}"), vec![], 500.0).unwrap();
        if i > 0 {
            f.glyph_mut(&format!("chain{i}")).unwrap().layers[0].shapes.push(Shape::Component(Component::new(format!("chain{}", i - 1), 1.0, 0.0)));
        }
    }
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Model(ModelError::ComponentCycle(_)))));
}

#[test]
fn components_that_multiply_are_bounded_work() {
    // Each level uses the one below twice: 2^30 copies of the square at the top.
    let build = |mixed: bool| {
        let mut f = sample();
        f.add_glyph("lvl0", vec![], 500.0).unwrap();
        f.glyph_mut("lvl0").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        for i in 1..=30 {
            let name = format!("lvl{i}");
            f.add_glyph(&name, vec![], 500.0).unwrap();
            let mut g = f.glyph_mut(&name).unwrap();
            g.layers[0].shapes.push(Shape::Component(Component::new(format!("lvl{}", i - 1), 0.0, 0.0)));
            g.layers[0].shapes.push(Shape::Component(Component::new(format!("lvl{}", i - 1), 1.0, 0.0)));
            if mixed && i == 30 {
                g.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
            }
        }
        f
    };
    for mixed in [false, true] {
        let start = std::time::Instant::now();
        let result = compile_ttf(&build(mixed), &Options::default());
        assert!(matches!(result, Err(CompileError::Model(ModelError::OutlineTooLarge(_)))), "{mixed}: {result:?}");
        assert!(start.elapsed().as_secs() < 10);
    }
}

#[test]
fn a_glyph_with_too_many_components_is_an_error() {
    let mut f = sample();
    let shapes: Vec<Shape> = (0..=MAX_COMPONENTS).map(|i| Shape::Component(Component::new("A", (i % 100) as f64, 0.0))).collect();
    f.glyph_mut("C").unwrap().layers[0].shapes = shapes;
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"));
}

#[test]
fn a_code_point_on_two_glyphs_names_both() {
    let glyphs = vec![rg("a", vec![], vec![]), rg("b", vec![], vec![])];
    let mut glyphs = glyphs;
    glyphs[0].unicodes = vec![0x61, 0x62];
    glyphs[1].unicodes = vec![0x62];
    let err = preflight::check_cmap(&glyphs).unwrap_err();
    assert_eq!(err.to_string(), "U+0062 is mapped to both `a` and `b`");
    // The same glyph twice is no conflict.
    glyphs[1].unicodes = vec![];
    glyphs[0].unicodes = vec![0x61, 0x61];
    preflight::check_cmap(&glyphs).unwrap();
}

#[test]
fn more_than_65535_glyphs_with_notdef_is_an_error_before_outline_work() {
    let mut f = Font::new("Full");
    f.remove_glyph(".notdef").unwrap();
    let master = f.masters[0].id.clone();
    while f.glyphs.len() < fontcraft_model::MAX_GLYPHS {
        let mut g = Glyph::new(format!("g{}", f.glyphs.len()));
        g.layers.push(Layer::new(&master, 500.0));
        f.glyphs.push(g);
    }
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::TooManyGlyphs(65_536))));
}

// --- Glyph order -----------------------------------------------------------------------------

#[test]
fn notdef_goes_first_and_is_drawn_when_absent() {
    // Present but not first: moved.
    let mut f = sample();
    let notdef = f.glyphs.remove(".notdef").unwrap();
    f.glyphs.push(Arc::try_unwrap(notdef).unwrap());
    let out = compile_sample(&f);
    assert_eq!(out.glyph_order[0], ".notdef");
    assert_eq!(out.glyph_order[1], "space", "the rest keeps the font's order");

    // Absent: drawn as fontc draws it, from the descender to the ascender.
    let mut f = sample();
    f.remove_glyph(".notdef").unwrap();
    let out = compile_sample(&f);
    assert!(out.warnings.iter().any(|w| w.contains(".notdef")));
    let font = FontRef::new(&out.bytes).unwrap();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let notdef = GlyphId::new(0);
    assert_eq!(metrics.advance_width(notdef), Some(500.0));
    let b = metrics.bounds(notdef).unwrap();
    assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (50.0, -200.0, 450.0, 800.0));
    let contours = drawn(&font, notdef);
    assert_eq!(contours.iter().map(Vec::len).collect::<Vec<_>>(), [4, 4]);
    assert!(contours[1].contains(&(100.0, -150.0)) && contours[1].contains(&(400.0, 750.0)), "{contours:?}");
    // Written clockwise (outer) and counter-clockwise (inner), like every TrueType contour.
    assert!(area(&contours[0]) < 0.0 && area(&contours[1]) > 0.0);
    assert_ne!(font.charmap().map('A').unwrap().to_u32(), 0, "glyph 0 is .notdef, not a letter");

    // The em sets the box: a 2048-unit font.
    let mut f = sample();
    f.remove_glyph(".notdef").unwrap();
    f.units_per_em = 2048;
    f.set_metric_pos("m01", fontcraft_model::metric::ASCENDER, 1900.0).unwrap();
    f.set_metric_pos("m01", fontcraft_model::metric::DESCENDER, -500.0).unwrap();
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let b = font.glyph_metrics(Size::unscaled(), LocationRef::default()).bounds(GlyphId::new(0)).unwrap();
    assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (102.0, -500.0, 922.0, 1900.0));
}

// --- Outline preparation ---------------------------------------------------------------------

fn settings() -> Settings {
    Settings { flavour: Flavour::TrueType, tolerance: 1.0, keep_direction: false, max_expanded: MAX_EXPANDED_ELEMENTS }
}

#[test]
fn decomposition_follows_the_reference_rules() {
    let glyphs = vec![
        rg("base", vec![square(0.0, 0.0, 100.0)], vec![]),
        rg("pure", vec![], vec![("base", Affine::translate((10.0, 0.0)))]),
        rg("scaled", vec![], vec![("base", Affine::scale(1.5))]),
        rg("mixed", vec![square(300.0, 0.0, 10.0)], vec![("base", Affine::IDENTITY)]),
        rg("overflow", vec![], vec![("base", Affine::scale(2.5))]),
        rg("nested", vec![], vec![("pure", Affine::translate((0.0, 50.0)))]),
    ];
    let out = outline::prepare(&glyphs, &settings(), &mut Vec::new()).unwrap();
    let kind = |p: &Prepared| match p {
        Prepared::Empty => "empty",
        Prepared::Simple(_) => "simple",
        Prepared::Composite(_) => "composite",
    };
    assert_eq!(out.iter().map(kind).collect::<Vec<_>>(), ["simple", "composite", "composite", "simple", "simple", "composite"]);
    let cff = outline::prepare(&glyphs, &Settings { flavour: Flavour::Cff, ..settings() }, &mut Vec::new()).unwrap();
    assert!(cff.iter().all(|p| kind(p) == "simple"), "CFF decomposes every component");
    // A decomposed glyph keeps its own contours first, then its components' breadth first…
    let starts = |glyphs: &[ResolvedGlyph]| -> Vec<f64> {
        let out = outline::prepare(glyphs, &Settings { keep_direction: true, ..settings() }, &mut Vec::new()).unwrap();
        let Some(Prepared::Simple(x)) = out.last() else { panic!("x is decomposed") };
        x.elements().iter().filter_map(|e| if let PathEl::MoveTo(p) = e { Some(p.x) } else { None }).collect()
    };
    let a = rg("a", vec![square(0.0, 0.0, 1.0)], vec![]);
    let c = rg("c", vec![square(20.0, 0.0, 1.0)], vec![]);
    let x = rg("x", vec![square(30.0, 0.0, 1.0)], vec![("b", Affine::IDENTITY), ("c", Affine::IDENTITY)]);
    let b = rg("b", vec![], vec![("a", Affine::translate((100.0, 0.0)))]);
    assert_eq!(starts(&[a.clone(), b, c.clone(), x.clone()]), [30.0, 20.0, 100.0], "own, then c, then b's component a");
    // …and a component that is decomposed itself comes in decomposed (dependencies first).
    let b = rg("b", vec![square(10.0, 0.0, 1.0)], vec![("a", Affine::translate((100.0, 0.0)))]);
    assert_eq!(starts(&[a, b, c, x]), [30.0, 10.0, 100.0, 20.0], "own, then b as decomposed, then c");
}

#[test]
fn composites_are_written_with_their_components_and_flags() {
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0]
        .shapes
        .push(Shape::Component(Component { transform: Transform::Matrix([1.0, 0.0, 0.0, -1.0, 10.5, -10.5]), ..Component::new("A", 0.0, 0.0) }));
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let Some(RawGlyph::Composite(c)) = raw(&font, font.charmap().map('C').unwrap()) else { panic!("a composite") };
    let comp = c.components().next().unwrap();
    assert_eq!(comp.glyph.to_u32(), gid_of(&out, "A").to_u32());
    // Offsets rounded half up: 10.5 → 11, −10.5 → −10.
    assert_eq!(comp.anchor, skrifa::raw::tables::glyf::Anchor::Offset { x: 11, y: -10 });
    assert!(comp.flags.contains(skrifa::raw::tables::glyf::CompositeGlyphFlags::ROUND_XY_TO_GRID));
    // The box of the flipped A: y from −700 to 0, moved by the rounded offsets.
    let b = font.glyph_metrics(Size::unscaled(), LocationRef::default()).bounds(font.charmap().map('C').unwrap()).unwrap();
    assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (71.0, -710.0, 571.0, -10.0));
}

#[test]
fn contours_are_reversed_for_truetype_unless_kept() {
    // Path::rect is counter-clockwise.
    let out = compile_sample(&sample());
    let font = FontRef::new(&out.bytes).unwrap();
    let a = drawn(&font, font.charmap().map('A').unwrap());
    assert!(area(&a[0]) < 0.0, "clockwise: {a:?}");
    // fontc's start point: the source's start, then the other points backwards.
    assert_eq!(a[0], [(60.0, 0.0), (60.0, 700.0), (560.0, 700.0), (560.0, 0.0)]);
    let kept = compile_ttf(&sample(), &Options { keep_direction: true, ..Default::default() }).unwrap();
    let font = FontRef::new(&kept.bytes).unwrap();
    let a = drawn(&font, font.charmap().map('A').unwrap());
    assert!(area(&a[0]) > 0.0, "counter-clockwise: {a:?}");
}

#[test]
fn coordinates_and_advances_round_half_up() {
    let mut f = sample();
    let mut c = f.glyph_mut("C").unwrap();
    c.layers[0].width = 600.5;
    c.layers[0].shapes.push(Shape::Path(Path::rect(-2.5, 2.5, 100.5, 200.0)));
    drop(c);
    let out = compile_ttf(&f, &Options { keep_direction: true, ..Default::default() }).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let gid = font.charmap().map('C').unwrap();
    assert_eq!(font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid), Some(601.0));
    assert_eq!(drawn(&font, gid)[0], [(-2.0, 3.0), (101.0, 3.0), (101.0, 200.0), (-2.0, 200.0)]);
    assert_eq!((ot_round(-2.5), ot_round(2.5), ot_round(-0.5), ot_round(f64::NAN)), (-2.0, 3.0, 0.0, 0.0));
}

#[test]
fn quadratic_curves_stay_within_the_tolerance() {
    let cubics = [
        CubicBez::new((0.0, 0.0), (0.0, 552.0), (448.0, 1000.0), (1000.0, 1000.0)),
        CubicBez::new((0.0, 0.0), (1000.0, 0.0), (0.0, 1000.0), (1000.0, 1000.0)),
        CubicBez::new((0.0, 0.0), (300.0, 900.0), (700.0, -900.0), (1000.0, 0.0)),
        CubicBez::new((5.0, 5.0), (5.0, 5.0), (5.0, 5.0), (5.0, 5.0)),
    ];
    for tolerance in [1.0, 0.25, 4.0] {
        for cubic in cubics {
            let mut p = BezPath::new();
            p.move_to(cubic.p0);
            p.curve_to(cubic.p1, cubic.p2, cubic.p3);
            let q = to_quadratic(&p, tolerance);
            assert!(q.elements().iter().all(|e| !matches!(e, PathEl::CurveTo(..))));
            // Every point of the quadratic spline is within the tolerance of the cubic.
            let mut last = cubic.p0;
            let mut worst: f64 = 0.0;
            for el in q.elements() {
                if let PathEl::QuadTo(c, e) = *el {
                    let seg = fontcraft_geom::QuadBez::new(last, c, e);
                    for i in 0..=64 {
                        let p = seg.eval(f64::from(i) / 64.0);
                        worst = worst.max(cubic.nearest(p, 1e-9).distance_sq.sqrt());
                    }
                    last = e;
                }
            }
            assert!(worst <= tolerance + 1e-6, "{cubic:?} at {tolerance}: {worst}");
        }
    }
    // The option is in em: a 2048-unit font gets about 2 units.
    let mut f = sample();
    f.units_per_em = 2048;
    let fine = compile_ttf(&f, &Options { conversion_error: 0.0001, ..Default::default() }).unwrap();
    let coarse = compile_ttf(&f, &Options::default()).unwrap();
    let bad = compile_ttf(&f, &Options { conversion_error: f64::NAN, ..Default::default() }).unwrap();
    let points = |out: &Compiled| {
        let font = FontRef::new(&out.bytes).unwrap();
        drawn(&font, font.charmap().map('O').unwrap()).iter().map(Vec::len).sum::<usize>()
    };
    assert!(points(&fine) > points(&coarse));
    assert_eq!(points(&bad), points(&coarse), "a bad option falls back to the default");
}

#[test]
fn hostile_values_are_errors_not_panics() {
    // A transform that is not a number never reaches the compiler's arithmetic.
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0]
        .shapes
        .push(Shape::Component(Component { transform: Transform::Matrix([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]), ..Component::new("A", 0.0, 0.0) }));
    assert!(compile_ttf(&f, &Options::default()).is_err());
    // An enormous scale is decomposed, and then the points are out of range.
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0]
        .shapes
        .push(Shape::Component(Component { transform: Transform::Matrix([1e300, 0.0, 0.0, 1e300, 0.0, 0.0]), ..Component::new("A", 0.0, 0.0) }));
    assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"));
    // A degenerate transform (everything on one point) builds.
    let mut f = sample();
    f.glyph_mut("C").unwrap().layers[0]
        .shapes
        .push(Shape::Component(Component { transform: Transform::Matrix([0.0, 0.0, 0.0, 0.0, 5.0, 5.0]), ..Component::new("A", 0.0, 0.0) }));
    compile_sample(&f);
}

#[test]
fn decomposition_over_the_whole_font_is_bounded() {
    // Each glyph alone is small; together they would create more than the limit allows.
    let big: Vec<BezPath> = (0..10).map(|i| square(f64::from(i) * 20.0, 0.0, 10.0)).collect();
    let mut glyphs = vec![rg("big", big, vec![])];
    for i in 0..20 {
        glyphs.push(rg(&format!("user{i}"), vec![square(0.0, 0.0, 1.0)], vec![("big", Affine::IDENTITY)]));
    }
    let tight = Settings { max_expanded: 600, ..settings() };
    assert!(matches!(outline::prepare(&glyphs, &tight, &mut Vec::new()), Err(CompileError::Glyph { message, .. }) if message.contains("decomposed")));
    outline::prepare(&glyphs, &settings(), &mut Vec::new()).unwrap();

    // The same for glyphs that are not exported.
    glyphs[0].export = false;
    let mut hidden = glyphs.clone();
    assert!(preflight::flatten_non_export_components(&mut hidden, 600).is_err());
    preflight::flatten_non_export_components(&mut glyphs, MAX_EXPANDED_ELEMENTS).unwrap();
}
