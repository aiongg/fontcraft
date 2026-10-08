//! `.glyphs` → designspace → model: the font information, the alignment zones, the stems and the
//! guides come back, except what the loss report lists. Every `.glyphs` source the reference app
//! saved in format version 3 (outside fontc's hand-made test data). Skipped when `corpus/` is
//! absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_model::{Family, Font, Guide, Master, Param, Params, Properties, metric};

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
    let (mut zoned, mut stemmed, mut guided) = (0, 0, 0);
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
    eprintln!("{checked} sources: {zoned} with zones, {stemmed} with stems, {guided} with master guides");
    assert!(zoned > 0 && stemmed > 0, "the corpus exercises zones and stems");
}
