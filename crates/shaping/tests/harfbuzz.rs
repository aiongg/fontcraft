//! Measure (M8 plan § 5, shaping against HarfBuzz): every request of `expected.tsv`, written by
//! `crates/shaping/crosscheck/harfbuzz.py` from HarfBuzz, gives the same glyphs, clusters,
//! advances and offsets here. The fonts stand beside it; the engine test `shaping_fonts` writes
//! FontCraft's builds of the corpus sources there, and any other font may join them.
//!
//! `FONTCRAFT_HB_DIR=<folder> cargo test -p fontcraft-shaping --test harfbuzz -- --ignored
//! --nocapture`; `HB_LIST=<n>` lists more than 20 differing requests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_shaping::{Direction, FeatureSetting, ShapeRequest, ShapedGlyph, ShapingFont, Tag, shape};

fn tag(text: &str) -> Tag {
    let bytes = text.as_bytes();
    let mut out = [b' '; 4];
    out[..bytes.len()].copy_from_slice(bytes);
    Tag::new(&out)
}

fn font(dir: &Path, name: &str) -> ShapingFont {
    let path = dir.join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let private_use: Vec<(u32, u16)> = std::fs::read_to_string(path.with_extension("pua"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let (cp, gid) = line.split_once('\t').unwrap();
            (u32::from_str_radix(cp, 16).unwrap(), gid.parse().unwrap())
        })
        .collect();
    ShapingFont::new(Arc::from(bytes), &private_use).unwrap()
}

fn location(text: &str) -> Vec<(Tag, f32)> {
    text.split(',').filter_map(|v| v.split_once('=')).map(|(t, v)| (tag(t), v.parse().unwrap())).collect()
}

/// `kern=0`, `ss01=1@3:8`.
fn features(text: &str) -> Vec<FeatureSetting> {
    text.split(',')
        .filter_map(|f| f.split_once('='))
        .map(|(t, rest)| {
            let (value, range) = match rest.split_once('@') {
                Some((value, range)) => {
                    let (a, b) = range.split_once(':').unwrap();
                    (value, Some(a.parse().unwrap()..b.parse().unwrap()))
                }
                None => (rest, None),
            };
            FeatureSetting { tag: tag(t), value: value.parse().unwrap(), range }
        })
        .collect()
}

fn unhex(text: &str) -> String {
    let bytes = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect();
    String::from_utf8(bytes).unwrap()
}

fn glyphs(text: &str) -> Vec<ShapedGlyph> {
    text.split_whitespace()
        .map(|g| {
            let f: Vec<i32> = g.split(':').map(|v| v.parse().unwrap()).collect();
            ShapedGlyph {
                glyph: u16::try_from(f[0]).unwrap(),
                cluster: u32::try_from(f[1]).unwrap(),
                x_advance: f[2],
                y_advance: f[3],
                x_offset: f[4],
                y_offset: f[5],
            }
        })
        .collect()
}

/// The first thing that differs, `None` when the runs are equal.
fn class(ours: &[ShapedGlyph], theirs: &[ShapedGlyph]) -> Option<&'static str> {
    let ids = |g: &[ShapedGlyph]| g.iter().map(|g| g.glyph).collect::<Vec<_>>();
    let clusters = |g: &[ShapedGlyph]| g.iter().map(|g| g.cluster).collect::<Vec<_>>();
    let advances = |g: &[ShapedGlyph]| g.iter().map(|g| (g.x_advance, g.y_advance)).collect::<Vec<_>>();
    if ours == theirs {
        None
    } else if ids(ours) != ids(theirs) {
        Some("glyphs")
    } else if clusters(ours) != clusters(theirs) {
        Some("clusters")
    } else if advances(ours) != advances(theirs) {
        Some("advances")
    } else {
        Some("offsets")
    }
}

#[test]
#[ignore = "compares with HarfBuzz's output in FONTCRAFT_HB_DIR: the M8 shaping measure"]
fn shaping_equals_harfbuzz() {
    let Some(dir) = std::env::var_os("FONTCRAFT_HB_DIR").map(PathBuf::from) else {
        eprintln!("skipped: set FONTCRAFT_HB_DIR to the folder with expected.tsv");
        return;
    };
    let list: usize = std::env::var("HB_LIST").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let expected = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
    let mut fonts: BTreeMap<String, ShapingFont> = BTreeMap::new();
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    let mut listed = 0;
    for row in expected.lines() {
        let f: Vec<&str> = row.split('\t').collect();
        let [name, at, direction, script, language, setting, text, result] = f[..] else { panic!("8 columns: {row}") };
        let key = format!("{name} {at}");
        let font = fonts.entry(key.clone()).or_insert_with(|| {
            let base = font(&dir, name);
            if at.is_empty() { base } else { base.at_location(&location(at)) }
        });
        let (text, features, theirs) = (unhex(text), features(setting), glyphs(result));
        let request = ShapeRequest {
            direction: match direction {
                "rtl" => Direction::RightToLeft,
                "ttb" => Direction::TopToBottom,
                _ => Direction::LeftToRight,
            },
            script: (!script.is_empty()).then(|| tag(script)),
            language: (!language.is_empty()).then_some(language),
            features: &features,
            ..ShapeRequest::new(&text)
        };
        let count = counts.entry(key).or_default();
        count.0 += 1;
        let difference = match shape(font, &request) {
            Ok(ours) => class(&ours, &theirs).map(|c| (c, format!("ours   {ours:?}"))),
            Err(e) => Some(("error", e.to_string())),
        };
        let Some((class, ours)) = difference else { continue };
        count.1 += 1;
        *by_class.entry(class).or_default() += 1;
        if listed < list {
            listed += 1;
            eprintln!("{name} {at} {direction} {script} {language} [{setting}] {text:?}: {class}\n  {ours}\n  theirs {theirs:?}");
        }
    }
    for (font, (compared, differ)) in &counts {
        eprintln!("{font}: {compared} requests, {differ} differ");
    }
    for (class, n) in &by_class {
        eprintln!("class {class}: {n}");
    }
    let compared: usize = counts.values().map(|c| c.0).sum();
    let differ: usize = counts.values().map(|c| c.1).sum();
    eprintln!("total: {compared} requests, {differ} differ");
    assert!(compared > 0, "expected.tsv is empty");
    assert_eq!(differ, 0, "{differ} of {compared} requests differ from HarfBuzz");
}
