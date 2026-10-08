//! `.glyphs` → designspace → model: the font information comes back, except what the loss report
//! lists. Every `.glyphs` source the reference app saved in format version 3 (outside fontc's
//! hand-made test data). Skipped when `corpus/` is absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_model::{Family, Font, Master, Param, Params, Properties};

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
    for p in &after.1.0 {
        if !before.1.0.iter().any(|q| q.name == p.name) {
            out.push(format!("{what}: new parameter `{}`", p.name));
        }
    }
}

fn info_trip(font: &Font) -> Vec<String> {
    let written = fontcraft_io_ufo::designspace::write(font).unwrap();
    let back = fontcraft_io_ufo::designspace::read(&written.document, &written.ufos).unwrap();
    let mut diffs = Vec::new();
    if font.info.date != back.info.date {
        diffs.push(format!("font: date {:?} → {:?}", font.info.date, back.info.date));
    }
    for (m, b) in font.masters.iter().zip(back.masters.iter()) {
        compare(&format!("master `{}`", m.name), &effective(font, m), &effective(&back, b), &mut diffs);
        if m.user_data != b.user_data {
            diffs.push(format!("master `{}`: user data", m.name));
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
        let losses = fontcraft_io_ufo::ufo::info_losses(&font);
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
}
