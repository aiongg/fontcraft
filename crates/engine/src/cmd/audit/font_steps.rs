//! The font-wide stages of `audit.fix` (`docs/spec/21-qa.md` § 5 "Fix calls"): the glyph-data
//! fixes before the layer steps (steps 1 to 6) and the fixes that read the final outlines after
//! them (34 to 38). Each step runs its checks again on the whole document, compiling it when they
//! read compiled fonts, and builds its calls from what they report now. A step is all or
//! nothing: when one of its calls fails, the document goes back to how it was before the step.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fontcraft_audit::data::tags;
use fontcraft_model::glyphdata::{Category, GlyphData};
use fontcraft_spacing::kerning::{Direction, KernIndex, Position, Side, Step, facing};

use super::*;

mod fea;

/// Most glyph pairs one stored kerning pair may kern for `cleanKerning` to weigh removing it.
const MAX_PAIR_SPAN: usize = 100_000;

/// Steps 1 to 6, before the layer steps.
pub(super) const DATA_STEPS: [&str; 6] =
    ["addMandatoryGlyphs", "removeGlyphs", "removeCodepoints", "updateGlyphInfo", "useProductionNames", "setGdefClasses"];
/// Steps 34 to 38, after the layer steps.
pub(super) const LATE_STEPS: [&str; 5] = ["cleanKerning", "renameLanguageTag", "addDefaultLanguagesystem", "reorderFeatures", "autohint"];

/// The fixes of the font-wide stages, by fix id, with the checks that asked for each.
pub(super) type FontSteps = BTreeMap<&'static str, Vec<&'static CheckSpec>>;

/// The font-wide steps a call selected, and the glyphs whose findings they fix: `None` for
/// every glyph.
#[derive(Default)]
pub(super) struct Selected {
    pub(super) fixes: FontSteps,
    pub(super) glyphs: Option<HashSet<String>>,
    /// A selected result names no glyph, so the steps take every finding.
    whole: bool,
}

impl Selected {
    /// Adds the glyph of a selected result; a result that names none opens the steps to every
    /// finding.
    pub(super) fn name(&mut self, glyph: &str) {
        if glyph.is_empty() {
            self.whole = true;
            self.glyphs = None;
        } else if !self.whole {
            self.glyphs.get_or_insert_with(HashSet::new).insert(glyph.to_string());
        }
    }
}

/// The command a font-wide step calls first, for the fixes `audit.fix` builds calls for.
pub(super) fn command(fix: &str) -> Option<&'static str> {
    let command = match fix {
        "addMandatoryGlyphs" => "glyph.addGlyphs",
        "removeGlyphs" => "glyph.setExport",
        "removeCodepoints" => "glyph.setUnicodes",
        "updateGlyphInfo" => "glyph.updateGlyphInfo",
        "useProductionNames" => "glyph.setProductionName",
        "setGdefClasses" => "glyph.setCategory",
        "cleanKerning" => "kerning.cleanUp",
        "renameLanguageTag" | "addDefaultLanguagesystem" => "font.features.setCode",
        "reorderFeatures" => "font.features.reorder",
        "autohint" => "hint.ps.autohintGlyph",
        _ => return None,
    };
    find_command(command).is_some().then_some(command)
}

/// Is `fix` one of the font-wide steps (built or not)?
pub(super) fn is_font_step(fix: &str) -> bool {
    DATA_STEPS.contains(&fix) || LATE_STEPS.contains(&fix)
}

/// Runs the steps of `order` that the call selected. `glyphs` limits the findings to those
/// glyphs (a call by result ids); `None` takes every finding.
pub(super) fn run_steps(
    s: &mut Session,
    run: &Run,
    steps: &FontSteps,
    glyphs: Option<&HashSet<String>>,
    order: &[&'static str],
    how: &How,
    report: &mut FixReport,
) -> Result<()> {
    for &fix in order {
        let Some(checks) = steps.get(fix) else { continue };
        let mut allowed = Vec::new();
        for &c in checks {
            if how.safe_only && !c.fix.is_some_and(|f| f.safe) {
                report.skip(None, None, Some(c.id), Some(fix), "the fix changes the design: give safeOnly: false");
            } else {
                allowed.push(c);
            }
        }
        if allowed.is_empty() {
            continue;
        }
        if command(fix).is_none() {
            report.skip(None, None, None, Some(fix), NO_FIX_CALL);
            continue;
        }
        let ids: Vec<&str> = allowed.iter().map(|c| c.id).collect();
        let found: Vec<Finding> = {
            let scope = Run { glyphs: glyphs.cloned(), layers: None, ..run.clone() };
            outside_findings(s.doc()?, &scope, allowed)
                .into_iter()
                .filter(|f| f.fix.is_some_and(|x| x.id == fix))
                .filter(|f| glyphs.is_none_or(|set| set.contains(&f.location.glyph)))
                .collect()
        };
        if found.is_empty() {
            continue;
        }
        let data = s.glyph_data()?;
        let index = if fix == "cleanKerning" { Some(s.kern_index()?) } else { None };
        let calls = calls_for(fix, s.font()?, &data, index.as_deref(), &found, report);
        if how.dry {
            for (command, params) in calls {
                report.planned.push(json!({"fix": fix, "command": command, "params": params, "checks": ids, "results": found.len()}));
            }
            continue;
        }
        let before = s.doc()?.font.clone();
        let mut done = Vec::new();
        let mut failed = None;
        for (command, params) in calls {
            match s.execute(command, &params) {
                Ok(_) => done.push(json!({"fix": fix, "command": command, "params": params, "checks": ids})),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        match failed {
            Some(e) => {
                s.doc_mut()?.font = before;
                report.skip(None, None, None, Some(fix), format!("the fix failed and changed nothing: {e}"));
            }
            None => report.applied.extend(done),
        }
    }
    Ok(())
}

/// The glyphs `findings` name that the font has, in font order; the others are listed in
/// `report`.
fn source_glyphs(font: &Font, findings: &[&Finding], fix: &str, report: &mut FixReport) -> Vec<String> {
    let named: BTreeSet<&str> = findings.iter().map(|f| f.location.glyph.as_str()).filter(|g| !g.is_empty()).collect();
    for f in findings.iter().filter(|f| !f.location.glyph.is_empty() && font.glyph(&f.location.glyph).is_none()) {
        report.skip(Some(&f.location.glyph), None, Some(f.check), Some(fix), "the compiled glyph has no source glyph");
    }
    font.glyphs.as_arcs().iter().filter(|g| named.contains(g.name.as_str())).map(|g| g.name.clone()).collect()
}

/// The kerning direction a finding's `direction` data names.
fn direction(f: &Finding) -> &'static str {
    match f.data.get("direction").copied() {
        Some(d) if d == 1.0 => "rtl",
        Some(d) if d == 2.0 => "vertical",
        _ => "ltr",
    }
}

/// The calls of one font-wide step for its findings.
fn calls_for(
    fix: &'static str,
    font: &Font,
    data: &GlyphData,
    index: Option<&KernIndex>,
    findings: &[Finding],
    report: &mut FixReport,
) -> Vec<(&'static str, Value)> {
    let findings: Vec<&Finding> = findings.iter().collect();
    let mut calls = Vec::new();
    let mut left: Vec<(Option<String>, &'static str, &'static str)> = Vec::new();
    let mut unbuilt = |f: &Finding, reason: &'static str| left.push((non_empty(&f.location.glyph).map(str::to_string), f.check, reason));
    match fix {
        "addMandatoryGlyphs" => {
            let mut names = BTreeSet::new();
            for f in &findings {
                match (f.code, f.data.get("codepoint")) {
                    ("notdef-not-found", _) => {
                        names.insert(".notdef".to_string());
                    }
                    ("missing-whitespace-glyph", Some(&cp)) if (0.0..=f64::from(u32::MAX)).contains(&cp) => {
                        names.insert(format!("uni{:04X}", cp as u32));
                    }
                    ("notdef-not-first" | "wrong-glyph-order", _) => unbuilt(f, "glyph order is the compiler's: there is nothing to add"),
                    _ => unbuilt(f, "the result does not name a missing glyph"),
                }
            }
            let names: Vec<String> = names.into_iter().filter(|n| font.glyph(n).is_none()).collect();
            if !names.is_empty() {
                calls.push(("glyph.addGlyphs", json!({"text": names.join(" "), "onExisting": "keep"})));
            }
        }
        "removeGlyphs" => {
            let glyphs = source_glyphs(font, &findings, fix, report);
            if !glyphs.is_empty() {
                calls.push(("glyph.setExport", json!({"glyphs": glyphs, "export": false})));
            }
        }
        "removeCodepoints" => {
            let mut drop: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
            for f in &findings {
                let cp = f.data.get("codepoint").copied().filter(|c| (0.0..=f64::from(u32::MAX)).contains(c));
                match cp {
                    Some(cp) if !f.location.glyph.is_empty() => {
                        drop.entry(f.location.glyph.as_str()).or_default().insert(cp as u32);
                    }
                    _ => unbuilt(f, "the result does not name a code point and a glyph"),
                }
            }
            for g in source_glyphs(font, &findings, fix, report) {
                let (Some(glyph), Some(gone)) = (font.glyph(&g), drop.get(g.as_str())) else { continue };
                let keep: Vec<u32> = glyph.unicodes.iter().copied().filter(|c| !gone.contains(c)).collect();
                if keep.len() != glyph.unicodes.len() {
                    calls.push(("glyph.setUnicodes", json!({"glyphs": [g], "unicodes": keep})));
                }
            }
        }
        "updateGlyphInfo" => {
            let glyphs = source_glyphs(font, &findings, fix, report);
            if !glyphs.is_empty() {
                calls.push(("glyph.updateGlyphInfo", json!({"glyphs": glyphs})));
            }
        }
        "useProductionNames" => {
            for g in source_glyphs(font, &findings, fix, report) {
                match font.glyph(&g) {
                    Some(glyph) if glyph.info.production.is_some() => {
                        calls.push(("glyph.setProductionName", json!({"glyphs": [g], "productionName": null})));
                    }
                    _ => report.skip(Some(&g), None, None, Some(fix), "the glyph does not override its production name: rename it instead"),
                }
            }
        }
        "setGdefClasses" => {
            // Per glyph: must it be a mark? The last finding about a glyph decides.
            let mut want: BTreeMap<&str, bool> = BTreeMap::new();
            for f in &findings {
                match f.code {
                    "mark-chars" => {
                        want.insert(f.location.glyph.as_str(), true);
                    }
                    "non-mark-chars" | "mark-in-gdef" | "legacy-accents-gdef" => {
                        want.insert(f.location.glyph.as_str(), false);
                    }
                    _ => unbuilt(f, "the result does not say whether the glyph is a mark"),
                }
            }
            for g in source_glyphs(font, &findings, fix, report) {
                let (Some(&mark), Some(glyph)) = (want.get(g.as_str()), font.glyph(&g)) else { continue };
                let params = if mark {
                    json!({"glyphs": [g], "category": "Mark", "subCategory": "Nonspacing"})
                } else {
                    let looked = data.lookup(&glyph.name, &glyph.unicodes);
                    if looked.category.is_some_and(|c| c.value == Category::Mark) {
                        json!({"glyphs": [g], "category": "Symbol", "subCategory": null})
                    } else {
                        json!({"glyphs": [g], "category": null, "subCategory": null})
                    }
                };
                calls.push(("glyph.setCategory", params));
            }
        }
        "cleanKerning" => {
            let mut clean = BTreeSet::new();
            let mut pairs = BTreeSet::new();
            let mut tabular = Vec::new();
            for f in &findings {
                let master = f.location.master.clone().unwrap_or_default();
                match (f.code, &f.location.pair) {
                    ("missing-glyph" | "empty-group", _) if !master.is_empty() => {
                        clean.insert((master, direction(f)));
                    }
                    ("zero-pair" | "repeats-group-value", Some([left, right])) if !master.is_empty() => {
                        pairs.insert((master, direction(f), left.clone(), right.clone()));
                    }
                    ("has-tabular-kerning", Some(_)) => tabular.push(*f),
                    _ => unbuilt(f, "the result names a compiled pair, not a stored one: remove the pair in the source"),
                }
            }
            for (master, dir) in clean {
                calls.push(("kerning.cleanUp", json!({"master": master, "direction": dir})));
            }
            for (master, dir, left, right) in pairs {
                calls.push(("kerning.removePair", json!({"left": left, "right": right, "master": master, "direction": dir})));
            }
            if let Some(index) = index.filter(|_| !tabular.is_empty()) {
                for (master, left, right) in tabular_pairs(font, index, &tabular, report) {
                    let call = json!({"left": left, "right": right, "master": master, "direction": "ltr"});
                    if !calls.iter().any(|(c, p)| *c == "kerning.removePair" && *p == call) {
                        calls.push(("kerning.removePair", call));
                    }
                }
            }
        }
        "renameLanguageTag" => {
            let mut slips: Vec<(String, String)> = Vec::new();
            for f in &findings {
                let wrong = f.location.language.as_deref().map(str::trim_end).unwrap_or_default();
                match (f.code, language_slip(wrong)) {
                    ("language-slip", Some(meant)) => {
                        if !slips.iter().any(|(w, _)| w == wrong) {
                            slips.push((wrong.to_string(), meant));
                        }
                    }
                    _ => unbuilt(f, "the result does not name a mistyped language tag"),
                }
            }
            if !slips.is_empty() {
                let (built, left) = fea::rename_language_tag(&font.features, &slips);
                if built.is_empty() && left.is_empty() {
                    report.skip(None, None, None, Some(fix), "the feature code does not name the mistyped tag in a language statement");
                }
                for reason in left {
                    report.skip(None, None, None, Some(fix), reason);
                }
                calls.extend(built.into_iter().map(|p| ("font.features.setCode", p)));
            }
        }
        "addDefaultLanguagesystem" => match fea::add_default_languagesystem(&font.features) {
            Ok(built) => calls.extend(built.into_iter().map(|p| ("font.features.setCode", p))),
            Err(reason) => report.skip(None, None, None, Some(fix), reason),
        },
        "reorderFeatures" => match fea::reorder_features(&font.features) {
            Ok(params) => calls.push(("font.features.reorder", params)),
            Err(reason) => report.skip(None, None, None, Some(fix), reason),
        },
        "autohint" => {
            let glyphs = source_glyphs(font, &findings, fix, report);
            if !glyphs.is_empty() {
                for m in font.masters.iter() {
                    calls.push(("hint.ps.autohintGlyph", json!({"glyphs": glyphs, "layer": m.id})));
                }
            }
        }
        _ => {}
    }
    for (glyph, check, reason) in left {
        report.skip(glyph.as_deref(), None, Some(check), Some(fix), reason);
    }
    calls
}

/// The registered tag a mistyped language tag stands for.
fn language_slip(tag: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }
    let mut padded = [b' '; 4];
    for (to, from) in padded.iter_mut().zip(bytes) {
        *to = *from;
    }
    tags::language_slip(padded).map(|meant| String::from_utf8_lossy(&meant).trim_end().to_string())
}

/// The stored left-to-right pairs (master, left, right, as commands spell them) behind the
/// tabular pairs the results name: in every master, the pair each lookup step finds, so that
/// none is left to apply once they are removed. A stored pair that also kerns glyphs no result
/// names is left out with the reason, since removing it would change their kerning.
fn tabular_pairs(font: &Font, index: &KernIndex, findings: &[&Finding], report: &mut FixReport) -> Vec<(String, String, String)> {
    let mut reported: BTreeSet<(&str, &str)> = BTreeSet::new();
    for f in findings {
        let Some([left, right]) = &f.location.pair else { continue };
        if font.glyph(left).is_some() && font.glyph(right).is_some() {
            reported.insert((left.as_str(), right.as_str()));
        } else {
            report.skip(None, None, Some(f.check), Some("cleanKerning"), format!("{left} {right}: the compiled glyph has no source glyph"));
        }
    }
    let steps = [Step::GlyphGlyph, Step::GlyphGroup, Step::GroupGlyph, Step::GroupGroup];
    let mut stored: BTreeSet<(String, String, String)> = BTreeSet::new();
    for &(left, right) in &reported {
        let (Some(first), Some(second)) = (index.glyph_id(left), index.glyph_id(right)) else { continue };
        let mut found = false;
        for m in font.masters.iter() {
            let Ok(table) = index.table(Direction::Ltr, &m.id) else { continue };
            for step in steps {
                if let Some(a) = table.at_step(first, second, step) {
                    stored.insert((a.master.to_string(), a.first.to_string(), a.second.to_string()));
                    found = true;
                }
            }
        }
        if !found {
            report.skip(
                Some(left),
                None,
                None,
                Some("cleanKerning"),
                format!("{left} {right}: no stored left-to-right pair kerns them; the kerning comes from the feature code"),
            );
        }
    }
    let mut out = Vec::new();
    for (master, first, second) in stored {
        let (a, b) = (Side::from_stored(&first), Side::from_stored(&second));
        let (la, lb) = (side_members(index, &a, Position::First), side_members(index, &b, Position::Second));
        let only_reported =
            la.len().saturating_mul(lb.len()) <= MAX_PAIR_SPAN && la.iter().all(|x| lb.iter().all(|y| reported.contains(&(x.as_str(), y.as_str()))));
        if only_reported {
            out.push((master, a.command(), b.command()));
        } else {
            report.skip(
                None,
                None,
                None,
                Some("cleanKerning"),
                format!("{} {}: the pair kerns glyphs no result names, which removing it would change", a.command(), b.command()),
            );
        }
    }
    out
}

/// The glyphs a stored side stands for at `position` in a left-to-right pair.
fn side_members(index: &KernIndex, side: &Side, position: Position) -> Vec<String> {
    match side {
        Side::Glyph(g) => vec![g.clone()],
        Side::Group(name) => index.members(facing(Direction::Ltr, position), name).map(str::to_string).collect(),
    }
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}
