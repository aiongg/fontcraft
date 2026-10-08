//! `.glyphs` → designspace → model: the font information, the alignment zones, the stems, the
//! guides, the axes (ranges and maps), the instances, the glyphs' metadata and the layers (their
//! drawings, guides, images, backgrounds, vertical metrics and user data) come back, except what
//! the loss report lists. Every `.glyphs` source the reference app
//! saved in format version 3 (outside fontc's hand-made test data). Skipped when `corpus/` is
//! absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_io_ufo::designspace;
use fontcraft_io_ufo::ufo::{GLYPH_LOSSES, LAYER_LOSSES};
use fontcraft_model::{
    Anchor, Background, ColorLabel, Family, Font, Glyph, GlyphInfo, Guide, Image, Instance, Layer, Master, Param, Params, Properties, Shape, axes,
    metric,
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
    let a = g.angle.rem_euclid(360.0);
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

/// The glyphs' code points, export flags and metadata. Each difference names the kind of
/// metadata as the loss reports do, so a listed loss excuses it.
fn glyphs_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    type Field = fn(&GlyphInfo) -> String;
    let fields: [(&str, Field); 15] = [
        ("the glyphs' production names", |i| format!("{:?}", i.production)),
        ("the glyphs' OpenType categories", |i| format!("{:?}", i.opentype_category)),
        (GLYPH_LOSSES[0], |i| format!("{:?}", (&i.category, &i.sub_category, &i.script))),
        (GLYPH_LOSSES[1], |i| format!("{:?}", (&i.case, &i.direction))),
        (GLYPH_LOSSES[2], |i| format!("{:?}", i.metrics_keys)),
        (GLYPH_LOSSES[3], |i| format!("{:?}", i.tags)),
        (GLYPH_LOSSES[4], |i| format!("{:?}", i.locked)),
        (GLYPH_LOSSES[5], |i| format!("{:?}", i.last_change)),
        (GLYPH_LOSSES[6], |i| format!("{:?}", (&i.sort_name, &i.sort_name_keep))),
        (GLYPH_LOSSES[7], |i| format!("{:?}", i.parts)),
        (GLYPH_LOSSES[8], |i| format!("{:?}", fontcraft_model::plist::Value::Dict(i.user_data.0.clone()).without_spellings())),
        (GLYPH_LOSSES[10], |i| i.note.clone()),
        // An index: the label colours; components: 8-bit ones survive a UFO.
        (GLYPH_LOSSES[9], |i| format!("{:?}", color_form(&i.color).map(|c| c.0))),
        ("the glyphs' colours, rounded to 8 bits", |i| format!("{:?}", color_form(&i.color).map(|c| c.1))),
        ("the glyphs' colours", |i| format!("{:?}", i.color.is_some())),
    ];
    if font.glyphs.len() != back.glyphs.len() {
        diffs.push(format!("glyphs: {} → {}", font.glyphs.len(), back.glyphs.len()));
    }
    for g in font.glyphs.iter() {
        let Some(b) = back.glyph(&g.name) else {
            diffs.push(format!("glyph `{}`: missing", g.name));
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

/// A drawing as numbers: paths (closed, nodes) and components (glyph, matrix), then anchors.
fn drawing(shapes: &[Shape], anchors: &[Anchor]) -> String {
    let shapes: Vec<String> = shapes
        .iter()
        .map(|s| match s {
            Shape::Path(p) => format!("{:?}", (p.closed, p.nodes.iter().map(|n| (n.x, n.y, n.kind, n.smooth)).collect::<Vec<_>>())),
            Shape::Component(c) => format!("{:?}", (&c.glyph, c.affine().as_coeffs())),
            Shape::Raw(_) => "raw".into(),
        })
        .collect();
    let anchors: Vec<(&str, f64, f64)> = anchors.iter().map(|a| (a.name.as_str(), a.x, a.y)).collect();
    format!("{shapes:?} {anchors:?}")
}

/// What of an image a UFO holds: the file and the matrix.
fn image_form(i: &Option<Image>) -> String {
    format!("{:?}", i.as_ref().map(|i| (&i.path, i.transform.affine().as_coeffs())))
}

fn guides_form(g: &[Guide]) -> Vec<(f64, f64, f64, String, bool)> {
    g.iter().map(guide_form).collect()
}

fn background_form(b: &Option<Background>) -> String {
    format!("{:?}", b.as_ref().map(|b| (drawing(&b.shapes, &b.anchors), guides_form(&b.guides), image_form(&b.image))))
}

/// The layers a UFO holds: each master layer and each other layer that has a UFO layer, matched
/// by the UFO layer's name (a second layer of one name in a glyph comes back as `<name> #<n>`).
/// Each difference names the kind of data as the loss report does, so a listed loss excuses it.
fn layers_trip(font: &Font, back: &Font, diffs: &mut Vec<String>) {
    for g in font.glyphs.iter() {
        let Some(b) = back.glyph(&g.name) else { continue };
        let mut used: Vec<&str> = Vec::new();
        for l in &g.layers {
            let at = format!("glyph `{}` layer `{}`", g.name, l.id);
            let found = if l.is_master_layer() {
                b.layer(&l.id)
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
            if l.is_master_layer() && l.info.name.is_some() {
                diffs.push(format!("{}: {at} {:?}", LAYER_LOSSES[0], l.info.name));
            }
            let master = font.masters.iter().find(|m| m.id == l.master);
            let default = master.and_then(|m| fontcraft_io_ufo::ufo::default_vertical_width(font, m));
            let width = |l: &Layer| l.info.vert_width.filter(|v| *v != 0.0 && Some(*v) != default);
            let checks: [(&str, String, String); 9] = [
                ("the layer's width", format!("{}", l.width), format!("{}", x.width)),
                ("the layer's drawing", drawing(&l.shapes, &l.anchors), drawing(&x.shapes, &x.anchors)),
                (LAYER_LOSSES[1], format!("{:?}", l.info.attrs), format!("{:?}", x.info.attrs)),
                (LAYER_LOSSES[2], format!("{:?}", width(l)), format!("{:?}", width(x))),
                (LAYER_LOSSES[3], format!("{:?}", l.info.vert_origin.filter(|o| *o != 0.0)), format!("{:?}", x.info.vert_origin)),
                (LAYER_LOSSES[11], format!("{:?}", guides_form(&l.info.guides)), format!("{:?}", guides_form(&x.info.guides))),
                ("the layer's image", image_form(&l.info.image), image_form(&x.info.image)),
                ("the layer's background", background_form(&l.info.background), background_form(&x.info.background)),
                (
                    "the layer's user data",
                    format!("{:?}", fontcraft_model::plist::Value::Dict(l.info.user_data.0.clone()).without_spellings()),
                    format!("{:?}", fontcraft_model::plist::Value::Dict(x.info.user_data.0.clone()).without_spellings()),
                ),
            ];
            for (what, before, after) in checks {
                if before != after {
                    diffs.push(format!("{what}: {at} {before} → {after}"));
                }
            }
            let lost: [(&str, bool); 6] = [
                (LAYER_LOSSES[4], !l.info.metrics_keys.is_empty()),
                (LAYER_LOSSES[5], l.info.color.is_some()),
                (LAYER_LOSSES[6], l.info.visible),
                (LAYER_LOSSES[7], !l.info.hints.is_empty() || l.info.background.as_ref().is_some_and(|b| !b.hints.is_empty())),
                (LAYER_LOSSES[8], !l.info.annotations.is_empty() || l.info.background.as_ref().is_some_and(|b| !b.annotations.is_empty())),
                (LAYER_LOSSES[9], l.info.image.as_ref().is_some_and(|i| i.crop.is_some() || i.alpha.is_some() || i.locked)),
            ];
            for (what, on) in lost {
                if on {
                    diffs.push(format!("{what}: {at}"));
                }
            }
        }
    }
}

fn info_trip(font: &Font) -> Vec<String> {
    let written = fontcraft_io_ufo::designspace::write(font).unwrap();
    let back = fontcraft_io_ufo::designspace::read(&written.document, &written.ufos).unwrap();
    let mut diffs = Vec::new();
    axes_trip(font, &back, &mut diffs);
    instances_trip(font, &back, &mut diffs);
    glyphs_trip(font, &back, &mut diffs);
    layers_trip(font, &back, &mut diffs);
    if font.info.date != back.info.date {
        diffs.push(format!("font: date {:?} → {:?}", font.info.date, back.info.date));
    }
    for (m, b) in font.masters.iter().zip(back.masters.iter()) {
        compare(&format!("master `{}`", m.name), &effective(font, m), &effective(&back, b), &mut diffs);
        if m.user_data != b.user_data {
            diffs.push(format!("master `{}`: user data", m.name));
        }
        if zones(font, m) != zones(&back, b) {
            diffs.push(format!("master `{}`: alignment zones {:?} → {:?}", m.name, zones(font, m), zones(&back, b)));
        }
        if stems(font, m) != stems(&back, b) {
            diffs.push(format!("master `{}`: stems {:?} → {:?}", m.name, stems(font, m), stems(&back, b)));
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

/// designspace → `.glyphs` → designspace: the axes (ranges and maps) and the instances come back,
/// except what the two loss reports list. Every corpus designspace whose UFOs are present.
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
