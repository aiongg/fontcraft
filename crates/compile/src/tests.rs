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
    // Names, composites, vertical metrics, localised names: every table the compiler writes.
    let mut font = sample();
    for (n, v) in [("vheaVertAscender", 500), ("vheaVertDescender", -500), ("vheaVertLineGap", 1000)] {
        int_param(&mut font, n, v);
    }
    Arc::make_mut(&mut font.info).properties.set("designers", "Someone", true);
    let a = compile_sample(&font);
    let b = compile_sample(&font.clone());
    assert_eq!(a.bytes, b.bytes);
    assert!(FontRef::new(&a.bytes).unwrap().vmtx().is_ok());
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
fn the_break_character_is_the_space_with_or_without_one() {
    // As ufo2ft and fontc write it.
    let mut f = sample();
    f.remove_glyph("space").unwrap();
    let out = compile_sample(&f);
    assert_eq!(FontRef::new(&out.bytes).unwrap().os2().unwrap().us_break_char(), Some(32));
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
    let otf = compile(&f, &Target::Master("m02".into()), Flavour::Cff, &Options::default()).unwrap();
    assert!(FontRef::new(&otf.bytes).unwrap().cff().is_ok(), "either flavour, at any master");
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

// --- Core tables as fontc derives them (M2.2) -----------------------------------------------

fn int_param(f: &mut Font, name: &str, v: i64) {
    Arc::make_mut(&mut f.info)
        .params
        .0
        .push(fontcraft_model::Param::new(name, fontcraft_model::plist::Value::Number(fontcraft_model::plist::Number::int(v))));
}

#[test]
fn metrics_take_the_reference_defaults() {
    let out = compile_sample(&sample());
    let font = FontRef::new(&out.bytes).unwrap();
    let (hhea, os2, post) = (font.hhea().unwrap(), font.os2().unwrap(), font.post().unwrap());
    // Ascender 800, descender -200: typo line gap 1.2 em - 1000 = 200, hhea ascender 1000.
    assert_eq!((hhea.ascender().to_i16(), hhea.descender().to_i16(), hhea.line_gap().to_i16()), (1000, -200, 0));
    assert_eq!((hhea.caret_slope_rise(), hhea.caret_slope_run()), (1000, 0));
    assert_eq!((os2.s_typo_ascender(), os2.s_typo_descender(), os2.s_typo_line_gap()), (800, -200, 200));
    assert_eq!((os2.us_win_ascent(), os2.us_win_descent()), (1000, 200));
    assert_eq!((post.underline_position().to_i16(), post.underline_thickness().to_i16()), (-100, 50));
    assert_eq!(os2.fs_type(), 8, "editable embedding when the source says nothing");
    assert_eq!(os2.fs_selection().bits(), 0x40, "REGULAR only; USE_TYPO_METRICS only when asked for");
    assert_eq!(os2.ach_vend_id(), skrifa::raw::types::Tag::new(b"NONE"));
    assert_eq!(os2.ul_code_page_range_1(), Some(1), "Latin 1 when nothing else applies");
    assert_eq!(font.head().unwrap().flags().bits(), 3);
}

#[test]
fn parameters_override_the_defaults() {
    let mut f = sample();
    int_param(&mut f, "typoLineGap", 0);
    int_param(&mut f, "winDescent", -250);
    int_param(&mut f, "Use Typo Metrics", 1);
    int_param(&mut f, "openTypeHeadLowestRecPPEM", 9);
    Arc::make_mut(&mut f.info).properties.set("vendorID", "    ", false);
    Arc::make_mut(&mut f.info).params.0.push(fontcraft_model::Param::new(
        "fsType",
        fontcraft_model::plist::Value::Array(fontcraft_model::plist::Array { items: vec![], ..Default::default() }),
    ));
    let m = Arc::make_mut(&mut f.masters);
    let i = f.metrics.iter().position(|d| d.kind == fontcraft_model::metric::ITALIC_ANGLE).unwrap_or(usize::MAX);
    if let Some(v) = m[0].metric_values.get_mut(i) {
        v.pos = 10.0;
    }
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let os2 = font.os2().unwrap();
    assert_eq!((os2.s_typo_line_gap(), os2.us_win_ascent(), os2.us_win_descent()), (0, 800, 250), "win ascent follows the gap; descent absolute");
    assert_eq!(os2.fs_type(), 0);
    assert_eq!(os2.ach_vend_id(), skrifa::raw::types::Tag::new(b"NONE"), "four spaces are no vendor id (fontc)");
    assert_eq!(font.head().unwrap().lowest_rec_ppem(), 9);
    if i != usize::MAX {
        assert_eq!(os2.fs_selection().bits() & 0x81, 0x81, "ITALIC from the angle, USE_TYPO_METRICS");
        assert_eq!(font.post().unwrap().italic_angle().to_f64(), -10.0, "counter-clockwise");
        assert_eq!(font.hhea().unwrap().caret_slope_run(), 176, "1000 × tan 10°");
    }
}

#[test]
fn names_follow_the_reference_fallbacks() {
    let mut f = sample();
    f.version_major = 2;
    f.version_minor = 5;
    Arc::make_mut(&mut f.info).properties.set("designers", "Someone", true);
    Arc::make_mut(&mut f.info)
        .params
        .0
        .push(fontcraft_model::Param::new("Name Table Entry", fontcraft_model::plist::Value::String("25; Prefix".into())));
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(names(&font, StringId::VERSION_STRING), ["Version 2.005"]);
    assert_eq!(names(&font, StringId::UNIQUE_ID), ["2.005;NONE;SliceSans-Regular"]);
    assert_eq!(names(&font, StringId::FULL_NAME), ["Slice Sans Regular"]);
    assert_eq!(names(&font, StringId::POSTSCRIPT_NAME), ["SliceSans-Regular"]);
    assert_eq!(names(&font, StringId::DESIGNER), ["Someone"]);
    assert_eq!(names(&font, StringId::new(25)), ["Prefix"]);
    assert_eq!(font.head().unwrap().font_revision().to_f64(), 2.005_004_882_812_5);
}

#[test]
fn a_masters_parameters_replace_the_fonts_as_glyphslib_applies_them() {
    let mut f = sample();
    Arc::make_mut(&mut f.info).properties.set("vendorID", "FONT", false);
    Arc::make_mut(&mut f.info).properties.set("licenses", "Font licence", true);
    let text = |t: &str| fontcraft_model::plist::Value::String(t.into());
    let m = &mut Arc::make_mut(&mut f.masters)[0];
    m.params.0.push(fontcraft_model::Param::new("vendorID", text("MAST")));
    m.params.0.push(fontcraft_model::Param::new("license", text("Master licence")));
    // A selection bit the font sets stays set when the master's parameter is off, and any number
    // but 0 sets it (glyphsLib).
    m.params.0.push(fontcraft_model::Param::new("Use Typo Metrics", fontcraft_model::plist::Value::Number(fontcraft_model::plist::Number::int(0))));
    int_param(&mut f, "Use Typo Metrics", 2);
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(font.os2().unwrap().ach_vend_id(), skrifa::raw::types::Tag::new(b"MAST"));
    assert_eq!(names(&font, StringId::LICENSE_DESCRIPTION), ["Master licence"]);
    assert!(names(&font, StringId::UNIQUE_ID)[0].contains(";MAST;"));
    assert_eq!(font.os2().unwrap().fs_selection().bits() & 0x80, 0x80);
}

#[test]
fn use_my_metrics_goes_on_the_first_component_with_the_advance() {
    let mut f = sample();
    let mut c = f.glyph_mut("C").unwrap();
    c.layers[0].width = 620.0;
    c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 30.0)));
    c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
    drop(c);
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let flags = |name: &str| -> Vec<bool> {
        let Some(RawGlyph::Composite(c)) = raw(&font, gid_of(&out, name)) else { panic!("{name}") };
        c.components().map(|c| c.flags.contains(skrifa::raw::tables::glyf::CompositeGlyphFlags::USE_MY_METRICS)).collect()
    };
    assert_eq!(flags("C"), [true, false], "a vertical offset is allowed, and only the first gets it");
    assert_eq!(flags("B"), [false], "B is moved sideways");
}

#[test]
fn hmtx_leaves_out_the_trailing_advances_that_repeat() {
    let mut f = sample();
    for name in ["x1", "x2", "x3"] {
        f.add_glyph(name, Vec::new(), 500.0).unwrap();
    }
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let n = usize::from(font.hhea().unwrap().number_of_h_metrics());
    assert_eq!(n, out.glyph_order.len() - 2, "three equal advances at the end keep one");
}

#[test]
fn vertical_tables_come_with_the_vhea_parameters() {
    let mut f = sample();
    for (n, v) in [("vheaVertAscender", 500), ("vheaVertDescender", -500), ("vheaVertLineGap", 1000)] {
        int_param(&mut f, n, v);
    }
    f.glyph_mut("A").unwrap().layers[0].info.vert_width = Some(900.0);
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let vhea = font.vhea().unwrap();
    assert_eq!((vhea.ascender().to_i16(), vhea.line_gap().to_i16()), (500, 1000));
    let vmtx = font.vmtx().unwrap();
    assert_eq!(vmtx.advance(gid_of(&out, "A")), Some(900));
    assert_eq!(vmtx.advance(gid_of(&out, "O")), Some(1000), "typo ascender to descender");
    assert_eq!(vmtx.side_bearing(gid_of(&out, "A")), Some(100), "from the typo ascender to the top");
    assert!(FontRef::new(&compile_sample(&sample()).bytes).unwrap().vhea().is_err(), "none without vertical data");
}

#[test]
fn stated_non_spacing_marks_have_no_advance() {
    let mut f = sample();
    f.add_glyph("acutecomb", vec![0x301], 300.0).unwrap();
    let mut g = f.glyph_mut("acutecomb").unwrap();
    g.info.category = Some("Mark".into());
    g.info.sub_category = Some("Nonspacing".into());
    drop(g);
    let out = compile_sample(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(font.hmtx().unwrap().advance(gid_of(&out, "acutecomb")), Some(0));
}

// --- CFF outlines (M2.3b) --------------------------------------------------------------------

use skrifa::raw::ps::cff::dict::{Entry, entries};
use skrifa::raw::types::Fixed;

fn compile_otf(f: &Font) -> Compiled {
    compile(f, &f.default_target().unwrap(), Flavour::Cff, &Options::default()).unwrap()
}

fn param(f: &mut Font, name: &str, v: fontcraft_model::plist::Value) {
    Arc::make_mut(&mut f.info).params.0.push(fontcraft_model::Param::new(name, v));
}

fn real(v: f64) -> fontcraft_model::plist::Value {
    fontcraft_model::plist::Value::Number(fontcraft_model::plist::Number::float(v).unwrap())
}

/// The Top DICT and Private DICT entries of a font's `CFF ` table, with the Top DICT's strings
/// looked up.
struct Dicts {
    top: Vec<Entry>,
    private: Vec<Entry>,
    strings: Vec<(String, String)>,
    name: String,
}

fn dicts(bytes: &[u8]) -> Dicts {
    let font = FontRef::new(bytes).unwrap();
    let cff = font.cff().unwrap();
    let top: Vec<Entry> = entries(cff.top_dicts().get(0).unwrap(), None).collect::<Result<_, _>>().unwrap();
    let range = top.iter().find_map(|e| if let Entry::PrivateDictRange(r) = e { Some(r.clone()) } else { None }).unwrap();
    let private = entries(&cff.offset_data().as_bytes()[range], None).collect::<Result<_, _>>().unwrap();
    let text = |sid| String::from_utf8_lossy(cff.string(sid).unwrap()).into_owned();
    let strings = top
        .iter()
        .filter_map(|e| match e {
            Entry::Version(s) => Some(("version", text(*s))),
            Entry::Notice(s) => Some(("Notice", text(*s))),
            Entry::Copyright(s) => Some(("Copyright", text(*s))),
            Entry::FullName(s) => Some(("FullName", text(*s))),
            Entry::FamilyName(s) => Some(("FamilyName", text(*s))),
            Entry::Weight(s) => Some(("Weight", text(*s))),
            _ => None,
        })
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let name = String::from_utf8_lossy(cff.name(0).unwrap()).into_owned();
    Dicts { top, private, strings, name }
}

fn fixed(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

#[test]
fn a_cff_font_reads_back_with_its_outlines_widths_and_names() {
    let out = compile_otf(&sample());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(font.cff().is_ok() && font.glyf().is_err() && font.loca(None).is_err(), "CFF outlines, no glyf or loca");
    let maxp = font.maxp().unwrap();
    assert_eq!(maxp.version(), skrifa::raw::types::Version16Dot16::VERSION_0_5);
    assert_eq!(usize::from(maxp.num_glyphs()), out.glyph_order.len());
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_3_0);
    assert_eq!(font.head().unwrap().index_to_loc_format(), 0);
    // Glyph names live in the charset.
    let names: Vec<String> = font.glyph_names().iter().map(|(_, n)| n.to_string()).collect();
    assert_eq!(names, out.glyph_order);

    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let a = gid_of(&out, "A");
    assert_eq!(metrics.advance_width(a), Some(620.0));
    assert_eq!(metrics.left_side_bearing(a), Some(60.0));
    // The component is drawn in, moved by its offset.
    let b = gid_of(&out, "B");
    let shifted: Vec<Vec<(f32, f32)>> = drawn(&font, a).into_iter().map(|c| c.into_iter().map(|(x, y)| (x + 10.0, y)).collect()).collect();
    assert_eq!(drawn(&font, b), shifted);
    assert_eq!(metrics.left_side_bearing(b), Some(70.0));
    // Cubic curves stay cubic and keep their drawn direction (counter-clockwise for an outer
    // contour), with the closing line implied.
    let o = gid_of(&out, "O");
    let contours = drawn(&font, o);
    assert_eq!(contours.len(), 1);
    assert_eq!(contours[0].len(), 5, "a move and four curves: {contours:?}");
    assert!(area(&contours[0]) > 0.0);
    assert!(area(&drawn(&font, a)[0]) > 0.0);
    let ob = metrics.bounds(o).unwrap();
    assert_eq!((ob.x_min, ob.y_min, ob.x_max, ob.y_max), (50.0, -10.0, 650.0, 710.0));
    // The font box is the CFF FontBBox, in `head` as well.
    let head = font.head().unwrap();
    let top = dicts(&out.bytes).top;
    let bbox = top.iter().find_map(|e| if let Entry::FontBbox(b) = e { Some(*b) } else { None }).unwrap();
    assert_eq!(bbox.map(|v| v.to_i32() as i16), [head.x_min(), head.y_min(), head.x_max(), head.y_max()]);
    let hhea = font.hhea().unwrap();
    assert_eq!((hhea.min_left_side_bearing().to_i16(), hhea.x_max_extent().to_i16()), (50, 650));
}

#[test]
fn cff_dicts_hold_the_names_numbers_zones_and_stems() {
    let mut f = sample();
    Arc::make_mut(&mut f.info).properties.set("copyrights", "© 2026 José [Test]", true);
    let d = dicts(&compile_otf(&f).bytes);
    let s = |k: &str| d.strings.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    assert_eq!(d.name, "SliceSans-Regular", "name ID 6");
    assert_eq!(s("version").as_deref(), Some("1.0"));
    assert_eq!(s("Copyright").as_deref(), Some("Copyright 2026 Jose? Test"), "© spelt out, accents decomposed, brackets dropped");
    assert_eq!(s("Notice").as_deref(), Some(""), "written empty when missing");
    assert_eq!(s("FullName").as_deref(), Some("Slice Sans Regular"));
    assert_eq!(s("FamilyName").as_deref(), Some("Slice Sans"));
    assert_eq!(s("Weight"), None);
    assert!(!d.top.iter().any(|e| matches!(e, Entry::FontMatrix(_))), "the default matrix at 1000 units per em");

    // The master's overshoots become blue zones, with the scale from the tallest zone.
    let p = &d.private;
    let blues = |e: &Entry| -> Option<Vec<(i32, i32)>> {
        match e {
            Entry::BlueValues(b) => Some(b.values().iter().map(|(a, b)| (a.to_i32(), b.to_i32())).collect()),
            _ => None,
        }
    };
    assert_eq!(p.iter().find_map(blues), Some(vec![(-16, 0), (500, 516), (700, 716), (800, 816)]));
    let other = p.iter().find_map(|e| if let Entry::OtherBlues(b) = e { Some(b.values().to_vec()) } else { None });
    assert_eq!(other, Some(vec![(fixed(-216.0), fixed(-200.0))]));
    assert!(p.contains(&Entry::BlueScale(fixed(3.0 / 64.0))), "{p:?}");
    assert!(p.contains(&Entry::BlueFuzz(fixed(0.0))), "0 unless the source says otherwise");
    assert!(!p.iter().any(|e| matches!(e, Entry::BlueShift(_) | Entry::ForceBold(_))), "defaults are left out");
    assert!(!p.iter().any(|e| matches!(e, Entry::StdHw(_) | Entry::StemSnapH(_))), "no stems");

    // Stems of both directions, a zone of the master's own, parameters for the rest, another em.
    let mut f = sample();
    f.units_per_em = 2048;
    Arc::make_mut(&mut f.stems).extend([
        fontcraft_model::StemDef { horizontal: true, ..Default::default() },
        fontcraft_model::StemDef { horizontal: true, ..Default::default() },
        fontcraft_model::StemDef::default(),
    ]);
    let master = &mut Arc::make_mut(&mut f.masters)[0];
    master.stem_values = vec![42.4, 30.0, 80.6];
    master.zones.push(fontcraft_model::AlignmentZone { pos: 600.0, size: 12.0 });
    param(&mut f, "blueScale", real(0.05));
    int_param(&mut f, "blueShift", 5);
    int_param(&mut f, "postscriptBlueFuzz", 1);
    param(&mut f, "postscriptForceBold", fontcraft_model::plist::Value::Bool(true));
    param(&mut f, "postscriptWeightName", fontcraft_model::plist::Value::String(fontcraft_model::plist::Str::new("Regular")));
    let d = dicts(&compile_otf(&f).bytes);
    let p = &d.private;
    assert_eq!(p.iter().find_map(blues).unwrap().get(2), Some(&(600, 612)));
    assert!(p.contains(&Entry::BlueScale(fixed(0.05))) && p.contains(&Entry::BlueShift(fixed(5.0))) && p.contains(&Entry::ForceBold(true)));
    assert!(!p.iter().any(|e| matches!(e, Entry::BlueFuzz(_))), "1 is the default");
    assert!(p.contains(&Entry::StdHw(fixed(42.0))) && p.contains(&Entry::StdVw(fixed(81.0))), "the first of each list, rounded: {p:?}");
    let snaps = p.iter().find_map(|e| if let Entry::StemSnapH(s) = e { Some(s.values().to_vec()) } else { None });
    assert_eq!(snaps, Some(vec![fixed(30.0), fixed(42.0)]), "sorted");
    assert_eq!(d.strings.iter().find(|(k, _)| k == "Weight").map(|(_, v)| v.as_str()), Some("Regular"));
    let matrix = d.top.iter().find_map(|e| if let Entry::FontMatrix(m) = e { Some(*m) } else { None });
    assert!(matrix.is_some(), "1/2048 is not the default matrix");

    // One direction of stems only: none written; no zones: no blue entries at all.
    let mut f = sample();
    Arc::make_mut(&mut f.stems).push(fontcraft_model::StemDef::default());
    let master = &mut Arc::make_mut(&mut f.masters)[0];
    master.stem_values = vec![80.0];
    master.metric_values.iter_mut().for_each(|v| v.over = 0.0);
    let p = dicts(&compile_otf(&f).bytes).private;
    assert!(!p.iter().any(|e| matches!(e, Entry::StdVw(_) | Entry::StemSnapV(_) | Entry::BlueValues(_) | Entry::BlueFuzz(_))), "{p:?}");
}

#[test]
fn cff_widths_come_from_the_source_when_it_sets_them() {
    let mut f = sample();
    int_param(&mut f, "postscriptNominalWidthX", 600);
    let d = dicts(&compile_otf(&f).bytes);
    assert!(d.private.contains(&Entry::NominalWidthX(fixed(600.0))) && d.private.contains(&Entry::DefaultWidthX(fixed(200.0))), "{:?}", d.private);
    let font_bytes = compile_otf(&f).bytes;
    let font = FontRef::new(&font_bytes).unwrap();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    assert_eq!(metrics.advance_width(GlyphId::new(font.charmap().map('A').unwrap().to_u32())), Some(620.0));
}

#[test]
fn cff_paths_leave_out_the_implied_closing_line_and_convert_quadratics_exactly() {
    use fontcraft_cff::PathOp;
    let mut triangle = BezPath::new();
    triangle.move_to((0.0, 0.0));
    triangle.line_to((100.0, 0.0));
    triangle.line_to((50.0, 80.0));
    triangle.line_to((0.0, 0.0));
    triangle.close_path();
    let ops = cff::path_ops(&triangle, true);
    assert_eq!(ops, [PathOp::MoveTo((0.0, 0.0)), PathOp::LineTo((100.0, 0.0)), PathOp::LineTo((50.0, 80.0))]);
    assert_eq!(cff::path_ops(&triangle, false).len(), 4, "the compiler's own .notdef keeps it");
    // A point that sits on the start keeps the closing line, as the duplicate point it is.
    let mut doubled = BezPath::new();
    doubled.move_to((0.0, 0.0));
    doubled.line_to((100.0, 0.0));
    doubled.line_to((0.0, 0.0));
    doubled.line_to((0.0, 0.0));
    doubled.close_path();
    assert_eq!(cff::path_ops(&doubled, true).len(), 4);
    // An open path keeps its last line.
    let mut open = BezPath::new();
    open.move_to((0.0, 0.0));
    open.line_to((100.0, 0.0));
    open.line_to((0.0, 0.0));
    assert_eq!(cff::path_ops(&open, true).len(), 3);
    // Quadratic: the control points two thirds of the way, computed from each end.
    let mut quad = BezPath::new();
    quad.move_to((0.0, 0.0));
    quad.quad_to((30.0, 90.0), (100.0, 0.0));
    quad.close_path();
    let k = 0.666_666_666_666_666_7;
    assert_eq!(cff::path_ops(&quad, true)[1], PathOp::CurveTo((k * 30.0, k * 90.0), (100.0 + k * (30.0 - 100.0), k * 90.0), (100.0, 0.0)));
}

#[test]
fn postscript_strings_are_reduced_to_printable_ascii() {
    assert_eq!(cff::postscript_string("Fine (c) <2026> 50%"), "Fine c 2026 50");
    assert_eq!(cff::postscript_string("Ærø ﬁ ①"), "?r? fi 1", "no decomposition: one `?` each");
    assert_eq!(cff::postscript_string("Åé"), "A?e?", "a decomposition with a mark: the mark is `?`");
    assert_eq!(cff::postscript_string("line\nbreak"), "line\nbreak");
}

#[test]
fn cff_glyph_boxes_are_rounded_for_the_metrics_tables() {
    let mut f = sample();
    let mut a = f.glyph_mut("A").unwrap();
    a.layers[0].shapes = vec![Shape::Path(Path::rect(60.4, -0.6, 559.5, 700.2))];
    drop(a);
    let out = compile_otf(&f);
    let font = FontRef::new(&out.bytes).unwrap();
    let a = gid_of(&out, "A");
    assert_eq!(font.hmtx().unwrap().side_bearing(a), Some(60));
    let b = font.glyph_metrics(Size::unscaled(), LocationRef::default()).bounds(a).unwrap();
    assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (60.0, -1.0, 560.0, 700.0), "points rounded half up in the charstring");
}

#[test]
fn hostile_cff_input_is_an_error_not_a_panic() {
    // Two points 60,000 units apart cannot be told apart in a charstring's 16-bit numbers.
    let mut f = sample();
    f.glyph_mut("A").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(-30_000.0, 0.0, 30_000.0, 10.0))];
    let err = compile(&f, &f.default_target().unwrap(), Flavour::Cff, &Options::default()).unwrap_err();
    assert!(err.to_string().contains("CFF"), "{err}");
    // No family and no style: the name table has no PostScript name, the CFF table still needs one.
    let mut f = sample();
    f.family_name = String::new();
    Arc::make_mut(&mut f.masters)[0].name = String::new();
    let d = dicts(&compile_otf(&f).bytes);
    assert!(!d.name.is_empty(), "{d:?}", d = d.strings);
    // A family name Latin-1 cannot hold is written with a warning.
    let mut f = sample();
    f.family_name = "Slice 字体".into();
    let out = compile_otf(&f);
    assert!(out.warnings.iter().any(|w| w.contains("CFF")), "{:?}", out.warnings);
}
