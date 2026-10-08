//! Diagnostic: what the model still carries untyped. Reads every corpus source (`.glyphs`,
//! packages, UFOs, designspace files with their UFOs) and counts the `Extra` keys per kind of
//! object and format family, with how many sources use each.
//! `cargo test -p fontcraft-engine --test extra_census -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fontcraft_model::{Extra, Family, Font, Params, Shape};

type Census = BTreeMap<(String, String, String), (usize, BTreeSet<usize>)>;

fn add(c: &mut Census, source: usize, what: &str, x: &Extra) {
    for e in x.iter() {
        let family = format!("{:?}", e.family).to_lowercase();
        let row = c.entry((family, what.to_string(), e.key.as_str().to_string())).or_default();
        row.0 += 1;
        row.1.insert(source);
    }
}

/// Parameters kept as written because their typed form would not write back the same (the UFO
/// reader's `raw` mark), by name.
fn raw_params(c: &mut Census, source: usize, what: &str, params: &Params) {
    for p in params.0.iter().filter(|p| p.extra.get(Family::Ufo, "raw").is_some()) {
        let row = c.entry(("ufo".into(), format!("{what} raw parameter"), p.name.clone())).or_default();
        row.0 += 1;
        row.1.insert(source);
    }
}

fn census(c: &mut Census, source: usize, f: &Font) {
    add(c, source, "font", &f.extra);
    raw_params(c, source, "font", &f.info.params);
    f.masters.iter().for_each(|m| raw_params(c, source, "master", &m.params));
    f.axes.iter().for_each(|a| add(c, source, "axis", &a.extra));
    f.metrics.iter().for_each(|m| add(c, source, "metric", &m.extra));
    f.masters.iter().for_each(|m| add(c, source, "master", &m.extra));
    f.masters.iter().flat_map(|m| &m.guides).for_each(|g| add(c, source, "master guide", &g.extra));
    f.instances.iter().for_each(|i| add(c, source, "instance", &i.extra));
    for s in f.features.prefixes.iter().chain(&f.features.classes).chain(&f.features.features) {
        add(c, source, "snippet", &s.extra);
    }
    for g in f.glyphs.iter() {
        add(c, source, "glyph", &g.extra);
        for l in &g.layers {
            add(c, source, if l.is_master_layer() { "layer" } else { "other layer" }, &l.extra);
            add(c, source, "layer attr", &l.info.attrs.extra);
            shapes(c, source, "", &l.shapes, &l.anchors);
            content(c, source, "", &l.info.guides, &l.info.annotations, l.info.image.as_ref());
            if let Some(b) = &l.info.background {
                add(c, source, "background", &b.extra);
                shapes(c, source, "bg ", &b.shapes, &b.anchors);
                content(c, source, "bg ", &b.guides, &b.annotations, b.image.as_ref());
            }
        }
    }
}

fn shapes(c: &mut Census, source: usize, prefix: &str, list: &[Shape], anchors: &[fontcraft_model::Anchor]) {
    anchors.iter().for_each(|a| add(c, source, &format!("{prefix}anchor"), &a.extra));
    for s in list {
        match s {
            Shape::Path(p) => {
                add(c, source, &format!("{prefix}path"), &p.extra);
                add(c, source, &format!("{prefix}path attr"), &p.info.attrs.extra);
                p.nodes.iter().for_each(|n| add(c, source, &format!("{prefix}node"), &n.info.extra));
            }
            Shape::Component(k) => {
                add(c, source, &format!("{prefix}component"), &k.extra);
                add(c, source, &format!("{prefix}component attr"), &k.info.attrs.extra);
            }
            Shape::Image(i) => add(c, source, &format!("{prefix}image shape"), &i.extra),
            Shape::Raw(r) => {
                let row = c.entry((format!("{:?}", r.family).to_lowercase(), format!("{prefix}raw shape"), String::new())).or_default();
                row.0 += 1;
                row.1.insert(source);
            }
        }
    }
}

fn content(
    c: &mut Census,
    source: usize,
    prefix: &str,
    guides: &[fontcraft_model::Guide],
    notes: &[fontcraft_model::Annotation],
    image: Option<&fontcraft_model::Image>,
) {
    guides.iter().for_each(|g| add(c, source, &format!("{prefix}guide"), &g.extra));
    notes.iter().for_each(|a| add(c, source, &format!("{prefix}annotation"), &a.extra));
    if let Some(i) = image {
        add(c, source, &format!("{prefix}image"), &i.extra);
    }
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" {
            continue;
        }
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if matches!(ext, "glyphs" | "designspace") && p.is_file() || matches!(ext, "glyphspackage" | "ufo") && p.is_dir() {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

#[test]
#[ignore = "diagnostic"]
fn extra_census() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    let mut c = Census::new();
    let mut read = 0;
    for (i, p) in all.iter().enumerate() {
        if let Ok((font, _)) = fontcraft_engine::io::open(p, None) {
            read += 1;
            census(&mut c, i, &font);
        }
    }
    eprintln!("{read} sources read");
    let mut rows: Vec<_> = c.into_iter().collect();
    rows.sort_by(|a, b| (&a.0.0, &a.0.1, std::cmp::Reverse(a.1.1.len())).cmp(&(&b.0.0, &b.0.1, std::cmp::Reverse(b.1.1.len()))));
    for ((family, what, key), (n, srcs)) in rows {
        eprintln!("{family:9} {what:12} {key:40} {n:>8} in {:>4}", srcs.len());
    }
    let _ = Family::Glyphs;
}
