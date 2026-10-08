//! Trips between the format families. `.glyphs` → designspace → model: the font information,
//! the alignment zones, the stems, the guides, the axes (ranges and maps), the instances, the
//! glyphs' metadata, the layers (their drawings, guides, images, backgrounds, vertical metrics
//! and user data), the kerning, the groups and the feature code come back, except what the loss
//! report lists, for every `.glyphs` source the reference app saved in format version 3 (outside
//! fontc's hand-made test data). The other tests: designspace → `.glyphs` → designspace, UFO →
//! `.glyphs` → UFO, intermediate and alternate layers, and `.glyphs` → designspace → `.glyphs`
//! for every reference-written source in every version (M1.7's measure). Skipped when `corpus/`
//! is absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_io_ufo::designspace;
use fontcraft_io_ufo::features::{FEATURE_LOSSES, label_record};
use fontcraft_io_ufo::kerning::KERNING_LOSSES;
use fontcraft_io_ufo::ufo::{GLYPH_LOSSES, LAYER_LOSSES, SHAPE_LOSSES, ufo_layer_name};
use fontcraft_model::plist::{Array, Dict, Value};
use fontcraft_model::{
    Anchor, Background, ColorLabel, Family, Font, Glyph, GlyphInfo, Guide, Image, Instance, Layer, Master, Param, Params, Properties, Settings,
    Shape, UserData, axes, metric,
};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
            continue;
        }
        if p.is_file() && p.extension().is_some_and(|x| x == "glyphs") {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

/// A master's font information: the font's properties and parameters with the master's own in
/// place of the font's of the same key or name (as the UFO writer combines them).
fn effective(font: &Font, m: &Master) -> (Properties, Params) {
    let mut props = font.info.properties.clone();
    for p in &m.properties.0 {
        props.0.retain(|q| q.key != p.key);
        props.0.push(p.clone());
    }
    let mut params: Vec<Param> = font.info.params.0.iter().filter(|p| !m.params.0.iter().any(|q| q.name == p.name)).cloned().collect();
    params.extend(m.params.0.iter().cloned());
    (props, Params(params))
}

/// What of `before` is missing or different in `after`, and what is new, one line each.
fn compare(what: &str, before: &(Properties, Params), after: &(Properties, Params), out: &mut Vec<String>) {
    for p in &before.0.0 {
        if after.0.get(&p.key) != Some(p) {
            out.push(format!("{what}: property `{}`", p.key));
        }
    }
    for p in after.0.0.iter().filter(|p| before.0.get(&p.key).is_none()) {
        out.push(format!("{what}: new property `{}`", p.key));
    }
    let raw = |p: &Param| p.extra.get(Family::Ufo, "raw").is_some();
    for p in before.1.0.iter().filter(|p| !p.disabled) {
        if !after.1.0.iter().any(|q| q.name == p.name && q.value == p.value && !raw(q)) {
            out.push(format!("{what}: parameter `{}`", p.name));
        }
    }
    // The axes' maps, which the reference's rules derive when a source has no parameter for
    // them, come back as the parameter (compared in `axes_trip`).
    for p in after.1.0.iter().filter(|p| p.name != axes::AXIS_MAPPINGS) {
        if !before.1.0.iter().any(|q| q.name == p.name) {
            out.push(format!("{what}: new parameter `{}`", p.name));
        }
    }
}

/// A master's alignment zones as glyphsLib lists them: the overshoot of every metric except the
/// italic angle, and the master's other zones, in ascending order.
fn zones(font: &Font, m: &Master) -> Vec<(f64, f64)> {
    let overshoots = font.metrics.iter().zip(&m.metric_values).filter(|(d, v)| d.kind != metric::ITALIC_ANGLE && v.over != 0.0);
    let mut z: Vec<(f64, f64)> = overshoots.map(|(_, v)| (v.pos, v.over)).chain(m.zones.iter().map(|z| (z.pos, z.size))).collect();
    z.sort_by(|a, b| a.partial_cmp(b).unwrap());
    z
}

/// A master's horizontal and vertical stem widths.
fn stems(font: &Font, m: &Master) -> (Vec<f64>, Vec<f64>) {
    let of = |h: bool| font.stems.iter().zip(&m.stem_values).filter(|(d, _)| d.horizontal == h).map(|(_, v)| *v).collect();
    (of(true), of(false))
}

/// What of a guide a UFO guideline holds: the line (a vertical line through y 0 or a horizontal
/// one through x 0 is stored by one coordinate), the name with any colour and identifier in it,
/// and the lock.
fn guide_form(g: &Guide) -> (f64, f64, f64, String, bool) {
    let a = (g.angle.rem_euclid(360.0) * 1e9).round() / 1e9;
    let (x, y, a) = if g.y == 0.0 && (a == 90.0 || a == 270.0) {
        (g.x, 0.0, 90.0)
    } else if g.x == 0.0 && (a == 0.0 || a == 180.0) {
        (0.0, g.y, 0.0)
    } else {
        (g.x, g.y, a)
    };
    let mut name = g.name.clone().unwrap_or_default();
    for (suffix, open) in [(&g.color, " ["), (&g.identifier, " [#")] {
        if let Some(s) = suffix.as_deref().filter(|s| !s.is_empty()) {
            name.push_str(&format!("{open}{s}]"));
        }
    }
    (x, y, a, name, g.locked)
}

/// A map that is the identity is no map.
fn bent(pairs: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    if pairs.iter().all(|(u, d)| u == d) { Vec::new() } else { pairs }
}

/// The axes: names, tags, user ranges and maps, as the source states them or the reference's rules
/// derive them.
fn axes_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    if font.axes.len() != back.axes.len() {
        diffs.push(format!("axes: {} → {}", font.axes.len(), back.axes.len()));
        return;
    }
    for (i, (a, b)) in font.axes.iter().zip(back.axes.iter()).enumerate() {
        if (&a.name, &a.tag, a.hidden) != (&b.name, &b.tag, b.hidden) {
            diffs.push(format!("axis `{}`: name, tag or hidden", a.name));
        }
        let (before, after) = (axes::range(font, i), axes::range(back, i));
        if before != after {
            diffs.push(format!("axis `{}`: range {before:?} → {after:?}", a.name));
        }
        for (what, same) in [
            ("its name in other languages", a.label_names == b.label_names),
            ("its STAT labels", a.labels == b.labels && a.ordering == b.ordering),
            ("its discrete values", a.values == b.values),
        ] {
            if !same {
                diffs.push(format!("axis `{}`: {what}", a.name));
            }
        }
        let before = bent(axes::mapping(font, i).pairs);
        let after = bent(axes::mapping_parameter(back, &b.tag).unwrap_or_default());
        if before != after {
            diffs.push(format!("axis `{}`: map {before:?} → {after:?}", a.name));
        }
    }
}

/// An instance's parameters, properties and user data in key order, without what the element's
/// attributes hold (compared through [`designspace::instance_names`]).
type InstanceData = (Vec<(String, String)>, Vec<(String, Option<String>)>, Vec<(String, String)>);

fn instance_data(i: &Instance) -> InstanceData {
    let attrs = ["familyName", "postscriptFontName", "UFO Filename", "com.schriftgestaltung.Glyphs.fullFilename"];
    let mut params: Vec<(String, String)> = i
        .params
        .0
        .iter()
        .filter(|p| !p.disabled && !attrs.contains(&p.name.as_str()))
        .map(|p| (p.name.clone(), format!("{:?}", p.value.without_spellings())))
        .collect();
    params.sort();
    let mut props: Vec<(String, Option<String>)> = i
        .properties
        .0
        .iter()
        .filter(|p| p.key != "postscriptFontName" && p.key != "familyNames")
        .map(|p| (p.key.clone(), p.default_value().map(str::to_string)))
        .collect();
    props.sort();
    let mut user: Vec<(String, String)> =
        i.user_data.0.entries.iter().map(|(k, v)| (k.as_str().to_string(), format!("{:?}", v.without_spellings()))).collect();
    user.sort();
    (params, props, user)
}

/// The instances, static ones and variable ones each in order.
fn instances_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    let variable = |i: &&Instance| i.kind.as_deref() == Some("variable");
    for want in [false, true] {
        let a: Vec<&Instance> = font.instances.iter().filter(|i| variable(i) == want).collect();
        let b: Vec<&Instance> = back.instances.iter().filter(|i| variable(i) == want).collect();
        if a.len() != b.len() {
            diffs.push(format!("instances: {} → {}", a.len(), b.len()));
            continue;
        }
        for (x, y) in a.into_iter().zip(b) {
            let what = format!("instance `{}`", x.name);
            let loc = |f: &Font, i: &Instance| (0..f.axes.len()).map(|k| axes::design_value(&i.location, k)).collect::<Vec<_>>();
            let sorted = |i: &Instance| {
                let mut w = i.interpolation.clone();
                w.sort_by(|p, q| p.0.cmp(&q.0));
                w
            };
            let checks = [
                ("name", x.name == y.name),
                // A designspace holds design coordinates to six decimals.
                ("location", want || loc(font, x).iter().zip(loc(back, y)).all(|(p, q)| (p - q).abs() < 1e-6)),
                ("export", x.exports == y.exports),
                ("weight class", x.weight_class == y.weight_class),
                ("width class", x.width_class == y.width_class),
                ("manual interpolation", x.manual_interpolation == y.manual_interpolation && sorted(x) == sorted(y)),
                ("type", x.kind == y.kind),
                ("names", want || designspace::instance_names(font, x) == designspace::instance_names(back, y)),
                ("parameters, properties or user data", instance_data(x) == instance_data(y)),
            ];
            for (field, same) in checks {
                if !same {
                    diffs.push(format!("{what}: {field}"));
                }
            }
        }
    }
}

/// A colour as 8-bit components (what versions 2 and 3 store), or its index.
fn color_form(c: &Option<ColorLabel>) -> Option<(Option<u8>, [i64; 4])> {
    c.map(|c| match c {
        ColorLabel::Index(i) => (Some(i), [0; 4]),
        ColorLabel::Rgba(p) => (None, p.map(|v| (v * 255.0).round() as i64)),
    })
}

/// What a glyph's metrics keys hold: the left, right and width keys glyphsLib's lib keys carry,
/// and the rest.
fn metrics_keys(k: &fontcraft_model::MetricsKeys) -> (String, String) {
    (format!("{:?}", (&k.left, &k.right, &k.width)), format!("{:?}", (&k.top, &k.bottom, &k.vert_width, &k.vert_origin)))
}

/// The glyphs' code points, export flags and metadata. Each difference names the kind of
/// metadata as the loss reports do, so a listed loss excuses it; data that travels in
/// glyphsLib's lib keys has names no loss report uses.
fn glyphs_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    type Field = fn(&GlyphInfo) -> String;
    let fields: [(&str, Field); 16] = [
        ("the glyphs' production names", |i| format!("{:?}", i.production)),
        ("the glyphs' OpenType categories", |i| format!("{:?}", i.opentype_category)),
        ("travelling: the glyphs' categories, subcategories and scripts", |i| format!("{:?}", (&i.category, &i.sub_category, &i.script))),
        (GLYPH_LOSSES[0], |i| format!("{:?}", (&i.case, &i.direction))),
        ("travelling: the glyphs' left, right and width metrics keys", |i| metrics_keys(&i.metrics_keys).0),
        (GLYPH_LOSSES[1], |i| metrics_keys(&i.metrics_keys).1),
        (GLYPH_LOSSES[2], |i| format!("{:?}", i.tags)),
        (GLYPH_LOSSES[3], |i| format!("{:?}", i.locked)),
        (GLYPH_LOSSES[4], |i| format!("{:?}", i.last_change)),
        (GLYPH_LOSSES[5], |i| format!("{:?}", (&i.sort_name, &i.sort_name_keep))),
        (GLYPH_LOSSES[6], |i| format!("{:?}", i.parts)),
        ("travelling: the glyphs' user data", |i| user_data_form(&i.user_data)),
        (GLYPH_LOSSES[8], |i| i.note.clone()),
        // An index: the label colours; components: 8-bit ones survive a UFO.
        (GLYPH_LOSSES[7], |i| format!("{:?}", color_form(&i.color).map(|c| c.0))),
        ("the glyphs' colours, rounded to 8 bits", |i| format!("{:?}", color_form(&i.color).map(|c| c.1))),
        ("the glyphs' colours", |i| format!("{:?}", i.color.is_some())),
    ];
    let layerless = font.glyphs.iter().filter(|g| g.layers.is_empty()).count();
    if font.glyphs.len() != back.glyphs.len() && font.glyphs.len() - layerless != back.glyphs.len() {
        diffs.push(format!("glyphs: {} → {}", font.glyphs.len(), back.glyphs.len()));
    }
    for g in font.glyphs.iter() {
        let Some(b) = back.glyph(&g.name) else {
            let what = if g.layers.is_empty() { GLYPH_LOSSES[9] } else { "missing" };
            diffs.push(format!("glyph `{}`: {what}", g.name));
            continue;
        };
        let plain = |g: &Glyph| (g.unicodes.clone(), g.export);
        if plain(g) != plain(b) {
            diffs.push(format!("glyph `{}`: code points or export", g.name));
        }
        for (what, field) in &fields {
            if field(&g.info) != field(&b.info) {
                diffs.push(format!("{what}: `{}` {:?} → {:?}", g.name, field(&g.info), field(&b.info)));
            }
        }
    }
}

/// A drawing as a UFO holds it: paths (closed; nodes with position, type, smooth join, name and
/// identifier; identifier) and components (glyph, matrix, identifier), then anchors (name,
/// position, colour, identifier). An open path's first node is compared without its join, which
/// [`shape_fields`] compares.
fn drawing(shapes: &[Shape], anchors: &[Anchor]) -> String {
    let shapes: Vec<String> = shapes
        .iter()
        .filter(|s| !matches!(s, Shape::Image(_)))
        .map(|s| match s {
            Shape::Path(p) => {
                let nodes: Vec<_> = p
                    .nodes
                    .iter()
                    .enumerate()
                    // `-0` in a file reads as -0.0; other formats write 0.
                    .map(|(i, n)| (n.x + 0.0, n.y + 0.0, n.kind, n.smooth && (p.closed || i > 0), &n.info.name, &n.info.identifier))
                    .collect();
                format!("{:?}", (p.closed, nodes, &p.info.identifier))
            }
            Shape::Component(c) => format!("{:?}", (&c.glyph, c.affine().as_coeffs(), &c.info.identifier)),
            Shape::Image(_) => String::new(),
            Shape::Raw(_) => "raw".into(),
        })
        .collect();
    let anchors: Vec<_> = anchors.iter().map(|a| (a.name.as_str(), a.x, a.y, &a.info.color, &a.info.identifier)).collect();
    format!("{shapes:?} {anchors:?}")
}

/// The engine's loss report line for shapes the model keeps as written.
const RAW_SHAPES: &str = "shapes read from that format that the model keeps as written, left out";

/// What a difference in a layer's drawing is called: the engine's loss report names a drawing
/// with shapes kept as written.
fn drawing_label(l: &Layer) -> &'static str {
    if l.shapes.iter().any(|s| matches!(s, Shape::Raw(_))) { RAW_SHAPES } else { "the layer's drawing" }
}

/// What glyphsLib's lib keys carry of the shapes: node user data and other attribute keys, the
/// components' alignment, anchors, locks and smart values (in property order), anchor user data.
const SHAPE_TRAVELS: [&str; 3] = ["travelling: the nodes' user data", "travelling: the components' placement", "travelling: the anchors' user data"];

/// The shape data a UFO has no place for, one projection per kind of [`SHAPE_LOSSES`]
/// (identifiers, which a UFO 3 holds, are in [`drawing`]), then those of [`SHAPE_TRAVELS`]. In a
/// background, component placement is [`SHAPE_LOSSES`]`[4]`.
fn shape_fields(shapes: &[Shape], anchors: &[Anchor], background: bool) -> Vec<(&'static str, String)> {
    let mut out: [Vec<String>; 13] = Default::default();
    for s in shapes {
        match s {
            Shape::Path(p) => {
                out[0].push(format!("{:?}", (p.info.locked, &p.info.attrs)));
                out[1].push(format!("{:?}", p.nodes.iter().map(|n| (n.locked, n.orientation)).collect::<Vec<_>>()));
                out[2].push(format!("{:?}", p.nodes.iter().map(|n| n.tangent).collect::<Vec<_>>()));
                out[3].push(format!("{:?}", !p.closed && p.nodes.first().is_some_and(|n| n.smooth)));
                let attr = |n: &fontcraft_model::Node| n.info.extra.get(Family::Glyphs, "attr").map(canonical);
                out[10].push(format!("{:?}", p.nodes.iter().map(|n| (user_data_form(&n.info.user_data), attr(n))).collect::<Vec<_>>()));
            }
            Shape::Component(c) => {
                let i = &*c.info;
                let mut smart: Vec<_> = i.smart_values.iter().map(|v| (v.property.clone(), v.value)).collect();
                smart.sort_by(|a, b| a.0.cmp(&b.0));
                if let Some(v) = out.get_mut(if background { 4 } else { 11 }) {
                    v.push(format!("{:?}", (i.alignment, &i.anchor, i.locked, smart)));
                }
                out[5].push(format!("{:?}", (&i.master_id, i.orientation, &i.attrs, &i.user_data)));
            }
            Shape::Image(i) => out[8].push(format!("{i:?}")),
            Shape::Raw(_) => {}
        }
    }
    out[6] = anchors.iter().map(|a| format!("{:?}", (a.info.locked, a.info.orientation))).collect();
    out[12] = anchors.iter().map(|a| user_data_form(&a.info.user_data)).collect();
    out.into_iter()
        .enumerate()
        .filter(|(k, _)| *k != 7 && *k != 9)
        .map(|(k, v)| {
            let what = match k {
                10 => SHAPE_TRAVELS[0],
                11 => SHAPE_TRAVELS[1],
                12 => SHAPE_LOSSES[7],
                _ => SHAPE_LOSSES.get(k).copied().unwrap_or(""),
            };
            (what, v.join(" "))
        })
        .collect()
}

/// What of an image a UFO holds: the file and the matrix, and in glyphsLib's keys the crop, the
/// opacity and the lock.
fn image_form(i: &Option<Image>) -> String {
    format!("{:?}", i.as_ref().map(|i| (&i.path, i.transform.affine().as_coeffs(), i.crop, i.alpha, i.locked)))
}

fn guides_form(g: &[Guide]) -> Vec<(f64, f64, f64, String, bool)> {
    g.iter().map(guide_form).collect()
}

fn background_form(b: &Option<Background>) -> String {
    format!("{:?}", b.as_ref().map(|b| (drawing(&b.shapes, &b.anchors), guides_form(&b.guides), image_form(&b.image))))
}

/// Hints in glyphsLib's form, whatever format they were read from.
fn hints_form(h: &[fontcraft_model::Hint]) -> Vec<String> {
    h.iter().map(|h| fontcraft_io_ufo::glyphkeys::hint_to_ufo(h).map_or_else(|| format!("{h:?}"), |(v, _)| canonical(&v))).collect()
}

/// Whether a layer is an alternate layer.
fn is_alternate(l: &Layer) -> bool {
    !l.is_master_layer() && !l.info.attrs.axis_rules.is_empty()
}

/// Whether a designspace holds a layer's coordinates, as a sparse source.
fn is_intermediate(l: &Layer) -> bool {
    let a = &l.info.attrs;
    !l.is_master_layer() && !a.coordinates.is_empty() && a.axis_rules.is_empty() && a.color_palette.is_none()
}

/// The alternate layer of `b` that `l` comes back as: of its master and name (a layer without a
/// name gets one), not taken yet.
fn alternate_in<'a>(l: &Layer, b: &'a Glyph, used: &[&str]) -> Option<&'a Layer> {
    b.layers.iter().find(|x| {
        is_alternate(x)
            && x.master == l.master
            && !used.contains(&x.id.as_str())
            && (l.info.name.as_deref().is_none_or(str::is_empty) || x.info.name == l.info.name)
    })
}

/// The layers a UFO holds: each master layer and each other layer that has a UFO layer, matched
/// by the UFO layer's name (a second layer of one name in a glyph comes back as `<name> #<n>`),
/// and their order in the glyph; in a designspace also the alternate layers, by master and name.
/// Each difference names the kind of data as the loss report does, so a listed loss excuses it.
fn layers_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    for g in font.glyphs.iter() {
        let Some(b) = back.glyph(&g.name) else { continue };
        let mut used: Vec<&str> = Vec::new();
        let mut positions: Vec<usize> = Vec::new();
        let mut alternate_positions: Vec<usize> = Vec::new();
        for l in &g.layers {
            let at = format!("glyph `{}` layer `{}`", g.name, l.id);
            let found = if l.is_master_layer() {
                b.layer(&l.id)
            } else if is_alternate(l) {
                let hit = alternate_in(l, b, &used);
                if let Some(x) = hit {
                    used.push(&x.id);
                    if x.info.name != l.info.name {
                        diffs.push(format!("{}: {at} {:?} → {:?}", LAYER_LOSSES[0], l.info.name, x.info.name));
                    }
                    if x.id != l.id {
                        diffs.push(format!("{}: {at} → `{}`", LAYER_LOSSES[13], x.id));
                    }
                    let (before, after) = (
                        designspace::alternate_region(font, &l.info.attrs.axis_rules),
                        designspace::alternate_region(back, &x.info.attrs.axis_rules),
                    );
                    if before != after {
                        diffs.push(format!(
                            "the rules of alternate layers that a later alternate layer of their glyph overlaps: {at} {before:?} → {after:?}"
                        ));
                    }
                }
                hit
            } else {
                let Some(name) = fontcraft_io_ufo::ufo::ufo_layer_name(font, l) else { continue };
                let hit = b.layers.iter().find(|x| {
                    let n = x.info.name.as_deref().unwrap_or("");
                    x.master == l.master
                        && !used.contains(&x.id.as_str())
                        && (n == name || n.strip_prefix(name.as_str()).is_some_and(|r| r.starts_with(" #")))
                });
                if let Some(x) = hit {
                    used.push(&x.id);
                    if x.info.name != l.info.name {
                        diffs.push(format!("{}: {at} {:?} → {:?}", LAYER_LOSSES[0], l.info.name, x.info.name));
                    }
                    if x.id != l.id {
                        diffs.push(format!("{}: {at} → `{}`", LAYER_LOSSES[13], x.id));
                    }
                }
                hit
            };
            let Some(x) = found else {
                diffs.push(format!("{at}: missing"));
                continue;
            };
            let at_back = b.layers.iter().position(|y| std::ptr::eq(y, x));
            if is_alternate(l) {
                alternate_positions.extend(at_back)
            } else {
                positions.extend(at_back)
            }
            if l.is_master_layer() && l.info.name.is_some() {
                diffs.push(format!("{}: {at} {:?}", LAYER_LOSSES[0], l.info.name));
            }
            let master = font.masters.iter().find(|m| m.id == l.master);
            let default = master.and_then(|m| fontcraft_io_ufo::ufo::default_vertical_width(font, m));
            let width = |l: &Layer| l.info.vert_width.filter(|v| *v != 0.0 && Some(*v) != default);
            let bg_hints = |l: &Layer| l.info.background.as_ref().map(|b| hints_form(&b.hints));
            let bg_notes = |l: &Layer| l.info.background.as_ref().map(|b| format!("{:?}", b.annotations));
            let notes =
                |l: &Layer| format!("{:?}", l.info.annotations.iter().map(|a| (&a.kind, a.x, a.y, a.angle, &a.text, a.width)).collect::<Vec<_>>());
            // The alternate layers' rules and the intermediate layers' coordinates are compared
            // on their own.
            let attrs = |l: &Layer| {
                let mut a = l.info.attrs.clone();
                if is_alternate(l) {
                    a.axis_rules.clear();
                }
                if is_intermediate(l) {
                    a.coordinates.clear();
                }
                format!("{a:?}")
            };
            if is_intermediate(l) && l.info.attrs.coordinates != x.info.attrs.coordinates {
                diffs.push(format!(
                    "intermediate layers' coordinates that take the master's value: {at} {:?} → {:?}",
                    l.info.attrs.coordinates, x.info.attrs.coordinates
                ));
            }
            let checks: [(&str, String, String); 15] = [
                ("the layer's width", format!("{}", l.width), format!("{}", x.width)),
                (drawing_label(l), drawing(&l.shapes, &l.anchors), drawing(&x.shapes, &x.anchors)),
                (LAYER_LOSSES[1], attrs(l), attrs(x)),
                (LAYER_LOSSES[2], format!("{:?}", width(l)), format!("{:?}", width(x))),
                (LAYER_LOSSES[3], format!("{:?}", l.info.vert_origin.filter(|o| *o != 0.0)), format!("{:?}", x.info.vert_origin)),
                (
                    "travelling: the layer's left, right and width metrics keys",
                    metrics_keys(&l.info.metrics_keys).0,
                    metrics_keys(&x.info.metrics_keys).0,
                ),
                (LAYER_LOSSES[4], metrics_keys(&l.info.metrics_keys).1, metrics_keys(&x.info.metrics_keys).1),
                (
                    LAYER_LOSSES[7],
                    format!("{:?}", (hints_form(&l.info.hints), bg_hints(l))),
                    format!("{:?}", (hints_form(&x.info.hints), bg_hints(x))),
                ),
                (LAYER_LOSSES[8], format!("{:?}", (notes(l), bg_notes(l))), format!("{:?}", (notes(x), bg_notes(x)))),
                (LAYER_LOSSES[11], format!("{:?}", guides_form(&l.info.guides)), format!("{:?}", guides_form(&x.info.guides))),
                (LAYER_LOSSES[9], image_form(&l.info.image), image_form(&x.info.image)),
                ("the layer's background", background_form(&l.info.background), background_form(&x.info.background)),
                ("the layer's user data", user_data_form(&numbered(&l.info.user_data)), user_data_form(&numbered(&x.info.user_data))),
                ("booleans in user data", user_data_form(&l.info.user_data), user_data_form(&x.info.user_data)),
                ("the layer's kind", format!("{:?}", l.is_master_layer()), format!("{:?}", x.is_master_layer())),
            ];
            for (what, before, after) in checks {
                if before != after {
                    diffs.push(format!("{what}: {at} {before} → {after}"));
                }
            }
            for background in [false, true] {
                let fields = |l: &Layer| match (background, l.info.background.as_ref()) {
                    (false, _) => shape_fields(&l.shapes, &l.anchors, false),
                    (true, Some(b)) => shape_fields(&b.shapes, &b.anchors, true),
                    (true, None) => shape_fields(&[], &[], true),
                };
                for ((what, before), (_, after)) in fields(l).into_iter().zip(fields(x)) {
                    if before != after {
                        diffs.push(format!("{what}: {at} {before} → {after}"));
                    }
                }
            }
            let lost: [(&str, bool); 2] = [(LAYER_LOSSES[5], l.info.color.is_some()), (LAYER_LOSSES[6], l.info.visible)];
            for (what, on) in lost {
                if on {
                    diffs.push(format!("{what}: {at}"));
                }
            }
        }
        if positions.windows(2).any(|w| matches!(w, [a, b] if a > b)) {
            diffs.push(format!("travelling: the layers' order: glyph `{}` {positions:?}", g.name));
        }
        // Alternate layers come back after the others.
        let last = positions.iter().max().copied().unwrap_or(0);
        let first_alt = g.layers.iter().position(is_alternate);
        let moved = first_alt
            .is_some_and(|f| g.layers.iter().skip(f).any(|l| !is_alternate(l) && (l.is_master_layer() || ufo_layer_name(font, l).is_some())));
        if moved || alternate_positions.windows(2).any(|w| matches!(w, [a, b] if a > b)) || alternate_positions.iter().any(|p| *p < last) {
            diffs.push(format!("glyphs whose alternate layers come back after their other layers: glyph `{}` {alternate_positions:?}", g.name));
        }
    }
}

/// `.glyphs` → designspace → model for every corpus source with intermediate or alternate layers
/// (fontc's hand-made test files included): those layers come back as layers, with their
/// coordinates or rules, drawing, width and name, except what the loss report lists. A source
/// whose alternate layers come back as glyphs of their own (the report says so) is counted.
#[test]
fn intermediate_and_alternate_layers_survive_a_designspace() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    fn all(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if p.is_file() && p.extension().is_some_and(|x| x == "glyphs") {
                out.push(p);
            } else if p.is_dir() {
                all(&p, out);
            }
        }
    }
    let mut paths = Vec::new();
    all(&corpus, &mut paths);
    paths.sort();
    let (mut intermediate, mut alternate, mut as_written, mut failures) = (0, 0, 0, Vec::new());
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let Ok(font) = fontcraft_io_glyphs::read(&text) else { continue };
        let layers = || font.glyphs.iter().flat_map(|g| g.layers.iter());
        let (braces, brackets) = (layers().any(is_intermediate), layers().any(|l| is_alternate(l) && !font.axes.is_empty()));
        if !braces && !brackets {
            continue;
        }
        let name = path.strip_prefix(&corpus).unwrap().display().to_string();
        let losses = designspace::info_losses(&font);
        if losses.iter().any(|l| l.starts_with("alternate layers that later alternate layers")) {
            as_written += 1;
            continue;
        }
        let written = match designspace::write(&font) {
            Ok(w) => w,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let back = designspace::read(&written.document, &written.ufos).unwrap();
        intermediate += usize::from(braces);
        alternate += usize::from(brackets);
        let mut diffs = Vec::new();
        if back.masters.len() != font.masters.len() {
            diffs.push(format!("masters: {} → {}", font.masters.len(), back.masters.len()));
        }
        if back.glyphs.len() != font.glyphs.len() {
            diffs.push(format!("glyphs: {} → {}", font.glyphs.len(), back.glyphs.len()));
        }
        // Only the intermediate and alternate layers: the rest of these hand-made files is
        // compared elsewhere.
        let mut trimmed = font.clone();
        for i in 0..trimmed.glyphs.len() {
            let mut g = trimmed.glyphs.get_mut_at(i).unwrap();
            g.layers.retain(|l| l.is_master_layer() || is_alternate(l) || is_intermediate(l));
        }
        let mut layer_diffs = Vec::new();
        layers_trip(&trimmed, &back, &mut layer_diffs);
        let special = |d: &String| {
            !d.contains(" layer `") || {
                font.glyphs.iter().any(|g| {
                    g.layers.iter().any(|l| (is_alternate(l) || is_intermediate(l)) && d.contains(&format!("glyph `{}` layer `{}`", g.name, l.id)))
                })
            }
        };
        diffs.extend(layer_diffs.into_iter().filter(special));
        for d in diffs {
            let listed = losses.iter().any(|l| d.contains(l.split(" (").next().unwrap_or(l).split(':').next().unwrap_or(l)));
            if !listed {
                failures.push(format!("{name}: {d}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!(
        "{intermediate} sources with intermediate layers, {alternate} with alternate layers; {as_written} whose alternate layers come back as glyphs"
    );
    assert!(intermediate > 0 && alternate > 0, "the corpus exercises intermediate and alternate layers");
}

/// The left-to-right pairs, with sides spelled as the reference's format spells them.
fn pairs(font: &Font) -> Vec<(String, String, String, f64)> {
    let side = |s: &str| fontcraft_model::glyphs_side(s).into_owned();
    let mut out: Vec<_> = font
        .kerning
        .ltr
        .0
        .iter()
        .flat_map(|m| m.firsts.iter().flat_map(move |f| f.seconds.iter().map(move |(s, v)| (m.master.clone(), side(&f.first), side(s), *v))))
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

/// Kerning, kerning groups and the other groups.
fn kerning_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    if pairs(font) != pairs(back) {
        diffs.push(format!("kerning: {} left-to-right pairs → {}", pairs(font).len(), pairs(back).len()));
    }
    let lost = [
        (KERNING_LOSSES[0], font.kerning.rtl.pair_count() > 0),
        (KERNING_LOSSES[1], font.kerning.vertical.pair_count() > 0),
        (KERNING_LOSSES[2], !font.kerning.context.is_empty()),
        (KERNING_LOSSES[3], font.glyphs.iter().any(|g| g.kern_groups.top.is_some() || g.kern_groups.bottom.is_some())),
    ];
    diffs.extend(lost.iter().filter(|(_, on)| *on).map(|(what, _)| what.to_string()));
    for g in font.glyphs.iter() {
        let Some(b) = back.glyph(&g.name) else { continue };
        if (&g.kern_groups.left, &g.kern_groups.right) != (&b.kern_groups.left, &b.kern_groups.right) {
            diffs.push(format!("kerning groups: glyph `{}` {:?} → {:?}", g.name, g.kern_groups, b.kern_groups));
        }
    }
    if font.groups != back.groups {
        diffs.push(format!("groups: {:?} → {:?}", font.groups, back.groups));
    }
}

/// The prefixes, classes and features, with the losses the report names applied first.
fn features_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    let mut f = (*font.features).clone();
    let mut lost = [false; 5];
    for s in &mut f.features {
        let before = s.labels.len();
        s.labels.retain(|l| label_record(l).is_some());
        lost[0] |= s.labels.len() != before;
        if s.notes.ends_with('\n') || s.disabled && s.code.ends_with('\n') {
            s.notes = s.notes.trim_end_matches('\n').to_string();
            if s.disabled {
                s.code = s.code.trim_end_matches('\n').to_string();
            }
            lost[3] = true;
        }
    }
    for s in f.prefixes.iter_mut().chain(f.classes.iter_mut()) {
        lost[2] |= !s.notes.is_empty();
        s.notes.clear();
    }
    for s in f.prefixes.iter_mut().chain(f.classes.iter_mut()).chain(f.features.iter_mut()) {
        lost[4] |= !s.extra.is_empty();
        s.extra = Default::default();
        s.file_name = None;
    }
    let manual_labels: Vec<String> = f.features.iter().filter(|s| !s.automatic && !s.labels.is_empty()).map(|s| s.name.clone()).collect();
    lost[1] = !manual_labels.is_empty();
    diffs.extend(FEATURE_LOSSES.iter().zip(lost).filter(|(_, on)| *on).map(|(what, _)| what.to_string()));
    let fields = |s: &fontcraft_model::Snippet| (s.name.clone(), s.code.clone(), s.automatic, s.disabled, s.notes.clone(), s.labels.clone());
    let lists = [("prefixes", &f.prefixes, &back.features.prefixes), ("classes", &f.classes, &back.features.classes)];
    for (what, before, after) in lists {
        if before.iter().map(fields).ne(after.iter().map(fields)) {
            diffs.push(format!("features: {what} {before:?} → {after:?}"));
        }
    }
    let kept = |list: &[fontcraft_model::Snippet]| list.iter().filter(|s| !manual_labels.contains(&s.name)).map(fields).collect::<Vec<_>>();
    if kept(&f.features) != kept(&back.features.features) || f.features.len() != back.features.features.len() {
        diffs.push(format!("features: features {:?} → {:?}", f.features, back.features.features));
    }
}

/// A value with every dictionary sorted by key, as text: the form to compare user data in, since
/// a UFO's libs are written sorted.
fn canonical(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Dict(d) => {
                let mut entries: Vec<_> = d.entries.iter().map(|(k, v)| (fontcraft_model::plist::Str::new(k.as_str()), sort(v))).collect();
                entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
                Value::Dict(Dict { entries, ..Default::default() })
            }
            Value::Array(a) => Value::Array(Array { items: a.items.iter().map(sort).collect(), ..Default::default() }),
            // `500` and `500.0` are one value.
            Value::Number(n) => {
                Value::Number(fontcraft_model::plist::Number::float(n.value().as_f64()).unwrap_or_else(|| fontcraft_model::plist::Number::int(0)))
            }
            other => other.without_spellings(),
        }
    }
    format!("{:?}", sort(v))
}

/// User data with every boolean as the number a `.glyphs` file holds for it.
fn numbered(u: &UserData) -> UserData {
    fn walk(v: &Value) -> Value {
        match v {
            Value::Bool(b) => Value::Number(fontcraft_model::plist::Number::int(i64::from(*b))),
            Value::Dict(d) => Value::Dict(Dict { entries: d.entries.iter().map(|(k, v)| (k.clone(), walk(v))).collect(), ..Default::default() }),
            Value::Array(a) => Value::Array(Array { items: a.items.iter().map(walk).collect(), ..Default::default() }),
            other => other.clone(),
        }
    }
    match walk(&Value::Dict(u.0.clone())) {
        Value::Dict(d) => UserData(d),
        _ => UserData::default(),
    }
}

fn user_data_form(u: &UserData) -> String {
    canonical(&Value::Dict(u.0.clone()))
}

/// The settings a UFO's lib holds (glyphsLib's three keys), a default stated as no value.
fn travelling_settings(s: &Settings) -> (Option<f64>, Option<bool>, Option<bool>) {
    (s.keyboard_increment.filter(|v| *v != 1.0), s.disables_automatic_alignment.filter(|b| *b), s.disables_nice_names.filter(|b| *b))
}

/// A master's icon and name parts, a default stated as no value.
fn master_names(m: &Master) -> (Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    fn not<'a>(v: &'a Option<String>, d: &str) -> Option<&'a str> {
        v.as_deref().filter(|x| *x != d)
    }
    (not(&m.icon, ""), not(&m.name_parts.weight, "Regular"), not(&m.name_parts.width, "Regular"), not(&m.name_parts.custom, ""))
}

/// What glyphsLib's lib keys carry for the font and its masters: the app version, the edit-tab
/// texts, three settings, the font's user data, and the masters' icons and name parts; the other
/// settings and the masters' visibility only where the loss report lists them.
fn lib_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    if font.source.app_version != back.source.app_version {
        diffs.push(format!("font: app version {:?} → {:?}", font.source.app_version, back.source.app_version));
    }
    if font.display_strings != back.display_strings {
        diffs.push(format!("font: edit-tab texts {:?} → {:?}", font.display_strings, back.display_strings));
    }
    let (a, b) = (&font.info.settings, &back.info.settings);
    if travelling_settings(a) != travelling_settings(b) {
        diffs.push(format!("font: settings {:?} → {:?}", travelling_settings(a), travelling_settings(b)));
    }
    let rest = |s: &Settings| Settings { keyboard_increment: None, disables_automatic_alignment: None, disables_nice_names: None, ..s.clone() };
    if rest(a) != rest(b) {
        diffs.push(format!("the font's settings: {:?} → {:?}", rest(a), rest(b)));
    }
    if user_data_form(&font.info.user_data) != user_data_form(&back.info.user_data) {
        diffs.push(format!("font: user data {} → {}", user_data_form(&font.info.user_data), user_data_form(&back.info.user_data)));
    }
    for (m, b) in font.masters.iter().zip(back.masters.iter()) {
        let (x, y) = (master_names(m), master_names(b));
        if x.0 != y.0 {
            diffs.push(format!("master `{}`: icon {:?} → {:?}", m.name, x.0, y.0));
        }
        if (x.1, x.2, x.3) != (y.1, y.2, y.3) {
            diffs.push(format!("master `{}`: name parts {:?} → {:?}", m.name, (x.1, x.2, x.3), (y.1, y.2, y.3)));
        }
        if m.visible != b.visible {
            diffs.push(format!("master `{}`: visibility", m.name));
        }
    }
}

fn info_trip(font: &Font) -> Vec<String> {
    let written = fontcraft_io_ufo::designspace::write(font).unwrap();
    let back = fontcraft_io_ufo::designspace::read(&written.document, &written.ufos).unwrap();
    trip_diffs(font, &back)
}

/// Everything the trips compare: axes, instances, glyphs, layers, kerning, features, the libs'
/// data, the date, and each master's font information, zones, stems and guides.
fn trip_diffs(font: &Font, back: &Font) -> Vec<String> {
    let mut diffs = Vec::new();
    axes_trip(font, back, &mut diffs);
    instances_trip(font, back, &mut diffs);
    glyphs_trip(font, back, &mut diffs);
    layers_trip(font, back, &mut diffs);
    kerning_trip(font, back, &mut diffs);
    features_trip(font, back, &mut diffs);
    lib_trip(font, back, &mut diffs);
    if font.info.date != back.info.date {
        diffs.push(format!("font: date {:?} → {:?}", font.info.date, back.info.date));
    }
    for (m, b) in font.masters.iter().zip(back.masters.iter()) {
        compare(&format!("master `{}`", m.name), &effective(font, m), &effective(back, b), &mut diffs);
        if m.user_data != b.user_data {
            diffs.push(format!("master `{}`: user data", m.name));
        }
        if zones(font, m) != zones(back, b) {
            diffs.push(format!("master `{}`: alignment zones {:?} → {:?}", m.name, zones(font, m), zones(back, b)));
        }
        if stems(font, m) != stems(back, b) {
            diffs.push(format!("master `{}`: stems {:?} → {:?}", m.name, stems(font, m), stems(back, b)));
        }
        let (before, after): (Vec<_>, Vec<_>) = (m.guides.iter().map(guide_form).collect(), b.guides.iter().map(guide_form).collect());
        if before != after {
            diffs.push(format!("master `{}`: guides {before:?} → {after:?}", m.name));
        }
    }
    diffs
}

#[test]
fn font_information_survives_a_designspace() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    let (mut checked, mut failures) = (0, Vec::new());
    let (mut zoned, mut stemmed, mut guided, mut instanced, mut mapped) = (0, 0, 0, 0, 0);
    let (mut noted, mut coloured, mut produced) = (0, 0, 0);
    let (mut layered, mut backgrounds, mut layer_guides, mut images) = (0, 0, 0, 0);
    let (mut kerned, mut grouped, mut featured, mut labelled) = (0, 0, 0, 0);
    let (mut user_data, mut settings, mut tabs, mut lib_params, mut icons) = (0, 0, 0, 0, 0);
    // What glyphsLib's glyph, layer and shape keys carry.
    let mut exercised: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for path in &all {
        let text = std::fs::read_to_string(path).unwrap();
        if !text.lines().take(3).any(|l| l.starts_with(".appVersion = "))
            || !text.starts_with("{\n.appVersion") && !text.contains(".formatVersion = 3;")
        {
            continue;
        }
        let Ok(font) = fontcraft_io_glyphs::read(&text) else { continue };
        if font.source.glyphs_version != Some(3) {
            continue;
        }
        checked += 1;
        zoned += usize::from(font.masters.iter().any(|m| !zones(&font, m).is_empty()));
        stemmed += usize::from(font.masters.iter().any(|m| !m.stem_values.is_empty()));
        guided += usize::from(font.masters.iter().any(|m| !m.guides.is_empty()));
        instanced += usize::from(!font.instances.is_empty());
        mapped += usize::from((0..font.axes.len()).any(|i| !bent(axes::mapping(&font, i).pairs).is_empty()));
        noted += usize::from(font.glyphs.iter().any(|g| !g.info.note.is_empty()));
        coloured += usize::from(font.glyphs.iter().any(|g| g.info.color.is_some()));
        produced += usize::from(font.glyphs.iter().any(|g| g.info.production.is_some()));
        let any_layer = |f: &dyn Fn(&Layer) -> bool| usize::from(font.glyphs.iter().flat_map(|g| g.layers.iter()).any(f));
        layered += any_layer(&|l| !l.is_master_layer() && fontcraft_io_ufo::ufo::ufo_layer_name(&font, l).is_some());
        backgrounds += any_layer(&|l| l.info.background.as_ref().is_some_and(|b| !b.shapes.is_empty()));
        layer_guides += any_layer(&|l| !l.info.guides.is_empty());
        images += any_layer(&|l| l.info.image.is_some());
        kerned += usize::from(!font.kerning.ltr.is_empty());
        grouped += usize::from(font.glyphs.iter().any(|g| !g.kern_groups.is_empty()));
        featured += usize::from(!font.features.features.is_empty() && !font.features.prefixes.is_empty());
        labelled += usize::from(font.features.features.iter().any(|s| !s.labels.is_empty()));
        user_data += usize::from(!font.info.user_data.is_empty());
        settings += usize::from(travelling_settings(&font.info.settings) != (None, None, None));
        tabs += usize::from(!font.display_strings.is_empty());
        let carried = |p: &Params| p.0.iter().any(fontcraft_io_ufo::ufo::lib_carries);
        lib_params += usize::from(carried(&font.info.params) || font.masters.iter().any(|m| carried(&m.params)));
        icons += usize::from(font.masters.iter().any(|m| m.icon.is_some()));
        let glyph_has = |f: &dyn Fn(&GlyphInfo) -> bool| font.glyphs.iter().any(|g| f(&g.info));
        let layers = || font.glyphs.iter().flat_map(|g| g.layers.iter());
        let shapes = || layers().flat_map(|l| l.shapes.iter());
        let components = || shapes().filter_map(|s| if let Shape::Component(c) = s { Some(c) } else { None });
        let kinds: [(&str, bool); 15] = [
            ("categories", glyph_has(&|i| i.category.is_some() || i.script.is_some())),
            ("glyph metrics keys", glyph_has(&|i| i.metrics_keys.left.is_some() || i.metrics_keys.right.is_some())),
            ("glyph user data", glyph_has(&|i| !i.user_data.is_empty())),
            ("last changes", glyph_has(&|i| i.last_change.is_some())),
            ("smart-glyph properties", glyph_has(&|i| !i.parts.is_empty())),
            ("layer metrics keys", layers().any(|l| !l.info.metrics_keys.is_empty())),
            ("hints", layers().any(|l| !l.info.hints.is_empty())),
            ("annotations", layers().any(|l| !l.info.annotations.is_empty())),
            ("image crops and locks", layers().any(|l| l.info.image.as_ref().is_some_and(|i| i.crop.is_some() || i.locked))),
            ("colour palette layers", layers().any(|l| l.info.attrs.color_palette.is_some())),
            ("paths and components in one layer", layers().any(|l| {
                l.shapes.iter().any(|s| matches!(s, Shape::Path(_))) && l.shapes.iter().any(|s| matches!(s, Shape::Component(_)))
            })),
            ("component alignment and anchors", components().any(|c| c.info.anchor.is_some() || c.info.alignment != fontcraft_model::Alignment::Default)),
            ("smart-component values", components().any(|c| !c.info.smart_values.is_empty())),
            ("node user data", shapes().any(|s| matches!(s, Shape::Path(p) if p.nodes.iter().any(|n| !n.info.user_data.is_empty() || n.info.extra.get(Family::Glyphs, "attr").is_some())))),
            ("anchor user data", layers().any(|l| l.anchors.iter().any(|a| !a.info.user_data.is_empty()))),
        ];
        for (what, on) in kinds {
            *exercised.entry(what).or_default() += usize::from(on);
        }
        let losses = designspace::info_losses(&font);
        for d in info_trip(&font) {
            let listed = losses.iter().any(|l| d.contains(l.split(" (").next().unwrap_or(l).split(':').next().unwrap_or(l)));
            if !listed {
                failures.push(format!("{}: {d}", path.display()));
            }
        }
        eprintln!("{}: {} losses: {:?}", path.display(), losses.len(), losses);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked >= 5, "only {checked} sources found");
    eprintln!(
        "{checked} sources: {zoned} with zones, {stemmed} with stems, {guided} with master guides, {instanced} with instances, {mapped} with axis maps, \
         {noted} with glyph notes, {coloured} with glyph colours, {produced} with production names"
    );
    assert!(zoned > 0 && stemmed > 0 && instanced > 0 && mapped > 0, "the corpus exercises zones, stems, instances and axis maps");
    assert!(noted > 0 && coloured > 0 && produced > 0, "the corpus exercises glyph notes, colours and production names");
    eprintln!("{layered} with other layers in a UFO, {backgrounds} with backgrounds, {layer_guides} with layer guides, {images} with images");
    assert!(layered > 0 && backgrounds > 0 && layer_guides > 0 && images > 0, "the corpus exercises layers, backgrounds, layer guides and images");
    eprintln!("{kerned} with kerning, {grouped} with kerning groups, {featured} with prefixes and features, {labelled} with feature labels");
    assert!(kerned > 0 && grouped > 0 && featured > 0 && labelled > 0, "the corpus exercises kerning, kerning groups, features and labels");
    eprintln!(
        "{user_data} with font user data, {settings} with settings a lib holds, {tabs} with edit-tab texts, {lib_params} with parameters a lib holds, {icons} with master icons"
    );
    assert!(user_data > 0 && settings > 0 && tabs > 0 && lib_params > 0 && icons > 0, "the corpus exercises glyphsLib's font and master lib keys");
    eprintln!("sources with what glyphsLib's glyph, layer and shape keys carry: {exercised:?}");
    let missing: Vec<&&str> = exercised.iter().filter(|(_, n)| **n == 0).map(|(k, _)| k).collect();
    assert!(missing.is_empty(), "the corpus exercises glyphsLib's glyph, layer and shape keys; not: {missing:?}");
}

/// The font's and each master's own parameters that a UFO's lib holds, compared by name and
/// value in any order.
fn params_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    let form = |p: &Params| {
        let mut v: Vec<(String, String)> =
            p.0.iter().filter(|p| fontcraft_io_ufo::ufo::lib_carries(p)).map(|p| (p.name.clone(), canonical(&p.value))).collect();
        v.sort();
        v
    };
    let mut pairs = vec![("font".to_string(), &font.info.params, &back.info.params)];
    pairs.extend(font.masters.iter().zip(back.masters.iter()).map(|(m, b)| (format!("master `{}`", m.name), &m.params, &b.params)));
    for (what, a, b) in pairs {
        let (x, y) = (form(a), form(b));
        for (name, value) in x.iter().filter(|e| !y.contains(e)) {
            diffs.push(format!("{what}: parameter `{name}` {value}"));
        }
        for (name, _) in y.iter().filter(|e| !x.contains(e)).filter(|(n, _)| n != axes::AXIS_MAPPINGS) {
            if !x.iter().any(|(n, _)| n == name) {
                diffs.push(format!("{what}: new parameter `{name}`"));
            }
        }
    }
}

fn designspaces(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_file() && p.extension().is_some_and(|x| x == "designspace") {
            out.push(p);
        } else if p.is_dir() {
            designspaces(&p, out);
        }
    }
}

fn ufo_tree(root: &Path) -> fontcraft_io_ufo::Files {
    let mut out = fontcraft_io_ufo::Files::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// designspace → `.glyphs` → designspace: the axes (ranges and maps), the instances, the glyphs'
/// metadata, the kerning, the groups and the feature code come back, except what the two loss
/// reports list. Every corpus designspace whose UFOs are present.
#[test]
fn axes_and_instances_survive_a_glyphs_file() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    designspaces(&corpus, &mut all);
    all.sort();
    let (mut checked, mut excused, mut failures) = (0, 0, Vec::new());
    for path in &all {
        let text = std::fs::read_to_string(path).unwrap();
        let Ok(paths) = designspace::ufo_paths(&text) else { continue };
        let dir = path.parent().unwrap();
        if paths.is_empty() || !paths.iter().all(|p| dir.join(p).join("metainfo.plist").is_file()) {
            continue;
        }
        let ufos = paths.iter().map(|p| (p.clone(), ufo_tree(&dir.join(p)))).collect();
        let Ok(font) = designspace::read(&text, &ufos) else { continue };
        checked += 1;
        let glyphs = fontcraft_io_glyphs::write(&font).unwrap();
        let mut back = fontcraft_io_glyphs::read(&glyphs).unwrap();
        // As saved as a new designspace from the `.glyphs` file.
        back.source = font.source.clone();
        let written = designspace::write(&back).unwrap();
        let again = designspace::read(&written.document, &written.ufos).unwrap();
        let mut losses = fontcraft_io_glyphs::info_losses(&font);
        losses.extend(designspace::info_losses(&back));
        let mut diffs = Vec::new();
        axes_trip(&font, &again, &mut diffs);
        instances_trip(&font, &again, &mut diffs);
        glyphs_trip(&font, &again, &mut diffs);
        layers_trip(&font, &again, &mut diffs);
        kerning_trip(&font, &again, &mut diffs);
        features_trip(&font, &again, &mut diffs);
        lib_trip(&font, &again, &mut diffs);
        params_trip(&font, &again, &mut diffs);
        for d in diffs {
            let listed = losses.iter().any(|l| d.contains(l.split(" (").next().unwrap_or(l).split(':').next().unwrap_or(l)));
            if listed {
                excused += 1;
            } else {
                failures.push(format!("{}: {d}", path.strip_prefix(&corpus).unwrap().display()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("{checked} designspaces through a .glyphs file; {excused} differences the loss reports list");
    assert!(checked >= 40, "only {checked} designspaces found");
}

fn ufos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") || !p.is_dir() {
            continue;
        }
        if p.extension().is_some_and(|x| x == "ufo") {
            out.push(p);
        } else {
            ufos(&p, out);
        }
    }
}

/// UFO → `.glyphs` → UFO: the kerning, the kerning groups (UFO 2 ones included), the other groups
/// and the feature code come back, except what the two loss reports list. Every corpus UFO the
/// reader reads.
#[test]
fn kerning_groups_and_features_survive_a_glyphs_file() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    ufos(&corpus, &mut all);
    all.sort();
    let (mut checked, mut grouped, mut ufo2_kerned, mut split, mut failures) = (0, 0, 0, 0, Vec::new());
    for path in &all {
        let Ok(font) = fontcraft_io_ufo::ufo::read(&ufo_tree(path)) else { continue };
        checked += 1;
        grouped += usize::from(!font.groups.is_empty());
        ufo2_kerned += usize::from(font.source.ufo_version == Some(2) && font.glyphs.iter().any(|g| !g.kern_groups.is_empty()));
        split += usize::from(!fontcraft_io_ufo::features::is_kept_text(&font.features) && !font.features.is_empty());
        let glyphs = fontcraft_io_glyphs::write(&font).unwrap();
        let mut back = fontcraft_io_glyphs::read(&glyphs).unwrap();
        back.source = font.source.clone();
        let again = fontcraft_io_ufo::ufo::read(&fontcraft_io_ufo::ufo::write(&back).unwrap()).unwrap();
        let mut losses = fontcraft_io_glyphs::info_losses(&font);
        losses.extend(fontcraft_io_ufo::ufo::info_losses(&back));
        let mut diffs = Vec::new();
        glyphs_trip(&font, &again, &mut diffs);
        layers_trip(&font, &again, &mut diffs);
        kerning_trip(&font, &again, &mut diffs);
        features_trip(&font, &again, &mut diffs);
        lib_trip(&font, &again, &mut diffs);
        params_trip(&font, &again, &mut diffs);
        for d in diffs {
            if !losses.iter().any(|l| d.contains(l.split(" (").next().unwrap_or(l))) {
                failures.push(format!("{}: {d}", path.strip_prefix(&corpus).unwrap().display()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!(
        "{checked} UFOs through a .glyphs file: {grouped} with other groups, {ufo2_kerned} UFO 2 with kerning groups, {split} with split feature code"
    );
    assert!(
        checked >= 100 && grouped > 0 && ufo2_kerned > 0 && split > 0,
        "the corpus exercises groups, UFO 2 kerning groups and split feature code"
    );
}

/// Every `.glyphs` source the reference app saved, single files and packages (as the round-trip
/// harness counts them: `.appVersion` in the header, outside fontc's hand-made test data).
fn reference_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
            continue;
        }
        let ext = p.extension().and_then(|x| x.to_str());
        let header = match ext {
            Some("glyphs") if p.is_file() => std::fs::read_to_string(&p).ok(),
            Some("glyphspackage") if p.is_dir() => std::fs::read_to_string(p.join("fontinfo.plist")).ok(),
            _ => {
                if p.is_dir() {
                    reference_sources(&p, out);
                }
                continue;
            }
        };
        let head: String = header.unwrap_or_default().chars().take(4096).collect();
        if head.lines().any(|l| l.starts_with(".appVersion = ")) {
            out.push(p);
        }
    }
}

/// Whether a loss report line names the difference `d`: the line's subject (before any
/// parenthesis or colon) is in `d`, or `d` is a new parameter a line says another comes back as.
fn listed(losses: &[String], d: &str) -> bool {
    losses.iter().any(|l| {
        d.contains(l.split(" (").next().unwrap_or(l).split(':').next().unwrap_or(l))
            || d.split_once("new parameter ").is_some_and(|(_, name)| l.contains(&format!("comes back as {name})")))
    })
}

/// Removes what is not a model field (`extra`, the source's record) from a model as JSON.
fn fields_only(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("extra");
            m.values_mut().for_each(fields_only);
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(fields_only),
        _ => {}
    }
}

/// Where two models as JSON differ: paths with array indices left out, with a count and up to
/// three examples (both values, shortened) each.
type JsonDiffs = std::collections::BTreeMap<String, (usize, Vec<String>)>;

fn json_diffs(a: &serde_json::Value, b: &serde_json::Value, path: &str, out: &mut JsonDiffs) {
    use serde_json::Value as J;
    fn note(out: &mut JsonDiffs, key: String, a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) {
        let row = out.entry(key).or_default();
        row.0 += 1;
        if row.1.len() < 3 {
            let short = |v: Option<&serde_json::Value>| v.map(|v| v.to_string().chars().take(160).collect::<String>()).unwrap_or_else(|| "-".into());
            row.1.push(format!("{} → {}", short(a), short(b)));
        }
    }
    match (a, b) {
        (J::Object(x), J::Object(y)) => {
            for (k, va) in x {
                match y.get(k) {
                    Some(vb) => json_diffs(va, vb, &format!("{path}.{k}"), out),
                    None => note(out, format!("{path}.{k} (gone)"), Some(va), None),
                }
            }
            for (k, vb) in y.iter().filter(|(k, _)| !x.contains_key(*k)) {
                note(out, format!("{path}.{k} (new)"), None, Some(vb));
            }
        }
        (J::Array(x), J::Array(y)) if x.len() == y.len() => {
            for (va, vb) in x.iter().zip(y) {
                json_diffs(va, vb, &format!("{path}[]"), out);
            }
        }
        (J::Array(_), J::Array(_)) => note(out, format!("{path} (length)"), Some(a), Some(b)),
        _ if a != b => note(out, path.to_string(), Some(a), Some(b)),
        _ => {}
    }
}

/// `.glyphs` → designspace → `.glyphs`, as saving as a designspace, opening it and saving it as
/// the source's format and version again does it. Every source the reference app saved (files
/// and packages, versions 2 to 4): the model that comes back is compared with the one read,
/// field by field as in the other trips, and every difference must be one the loss reports of
/// the two saves list. A second, coarser count compares the whole models (every field, `Extra`
/// left out) and prints where they differ.
#[test]
fn reference_sources_survive_a_designspace_and_back() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    reference_sources(&corpus, &mut all);
    all.sort();
    let (mut checked, mut excused, mut failures) = (0, 0, Vec::new());
    // Whole-model differences: path → (count, sources, examples). `TRIP_WHERE=<path part>` prints
    // the examples of the rows whose path contains it.
    let mut whole: std::collections::BTreeMap<String, (usize, std::collections::BTreeSet<String>, Vec<String>)> = Default::default();
    for path in &all {
        let name = path.strip_prefix(&corpus).unwrap().display().to_string();
        let package = path.is_dir();
        let font = if package {
            fontcraft_io_glyphs::read_package(&ufo_tree(path)).unwrap()
        } else {
            fontcraft_io_glyphs::read(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        checked += 1;
        // Saved as a designspace (the engine's `file.saveAs`).
        let mut out = font.clone();
        out.source.format = fontcraft_model::SourceFormat::Designspace;
        out.source.ufo_version = Some(3);
        let written = designspace::write(&out).unwrap();
        let mut losses = designspace::info_losses(&out);
        // Opened and saved as the source's format and version.
        let mut back = designspace::read(&written.document, &written.ufos).unwrap();
        back.source.format = font.source.format;
        back.source.glyphs_version = font.source.glyphs_version;
        losses.extend(fontcraft_io_glyphs::info_losses(&back));
        let again = if package {
            fontcraft_io_glyphs::read_package(&fontcraft_io_glyphs::write_package(&back).unwrap()).unwrap()
        } else {
            fontcraft_io_glyphs::read(&fontcraft_io_glyphs::write(&back).unwrap()).unwrap()
        };
        let mut diffs = trip_diffs(&font, &again);
        params_trip(&font, &again, &mut diffs);
        if font.masters.len() != again.masters.len() {
            diffs.push(format!("masters: {} → {}", font.masters.len(), again.masters.len()));
        }
        let raw = font
            .glyphs
            .iter()
            .flat_map(|g| g.layers.iter())
            .any(|l| l.shapes.iter().chain(l.info.background.iter().flat_map(|b| b.shapes.iter())).any(|s| matches!(s, Shape::Raw(_))));
        if raw {
            losses.push(RAW_SHAPES.to_string());
        }
        let mut unlisted = 0;
        for d in diffs {
            if listed(&losses, &d) {
                excused += 1;
            } else {
                unlisted += 1;
                failures.push(format!("{name}: {d}"));
            }
        }
        let (mut a, mut b) = (serde_json::to_value(&font).unwrap(), serde_json::to_value(&again).unwrap());
        fields_only(&mut a);
        fields_only(&mut b);
        let mut counts = JsonDiffs::new();
        json_diffs(&a, &b, "", &mut counts);
        for (k, (n, examples)) in counts {
            let row = whole.entry(k).or_default();
            row.0 += n;
            row.1.insert(name.clone());
            row.2.extend(examples.into_iter().take(3usize.saturating_sub(row.2.len())).map(|e| format!("{name}: {e}")));
        }
        eprintln!("{name}: {} losses listed, {unlisted} differences not listed", losses.len());
    }
    eprintln!("whole-model differences (field path: count, sources):");
    let wanted = std::env::var("TRIP_WHERE").ok();
    for (k, (n, s, examples)) in &whole {
        eprintln!("  {k}: {n} in {}: {}", s.len(), s.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
        if wanted.as_deref().is_some_and(|w| k.contains(w)) {
            examples.iter().for_each(|e| eprintln!("      {e}"));
        }
    }
    eprintln!("{checked} sources through a designspace and back; {excused} differences the loss reports list, {} not", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked >= 30, "only {checked} sources found");
}
