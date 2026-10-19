//! Build glyphs from construction rules (M13.17), on `fontcraft_components::construction`.
//! Rows: `font.buildFromConstructions`, `constructions.ruleGrammar`.
//!
//! The rules run in file order on a working copy of the font, and each built glyph is added before
//! the next rule runs, so a later rule can use an earlier result (`Aringacute` from a built
//! `Aring`). A line that is not a rule, or a rule that cannot be built, is listed in `errors` and
//! the other rules still run; a file that cannot be used at all (too large, a missing or circular
//! variable) refuses the command. Everything one call builds is one undo step; with `preview`
//! nothing is stored.
//!
//! A new glyph gets a layer for every master. An existing glyph keeps its other layers and data;
//! the master layers built (all, or those `masters` names) are replaced. Rules marked `*` are
//! decomposed as Decompose Components does it.

use std::collections::HashSet;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use fontcraft_components::construction::MAX_RULES_BYTES;
use fontcraft_components::construction::{ApplyOptions, Built, MAX_RULES, Rule, RuleFile, build_rule, construct_glyph, parse_rules};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, MAX_GLYPHS, Shape};
use fontcraft_spacing::kerning::{Direction, KernIndex};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::{Params, kind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.buildFromConstructions",
            "Build Glyphs from Constructions…",
            ["Glyph"],
            None,
            "{rules?: string, path?: path (one of the two: the rules as text, or a rules file of at most 4 MB), glyphs?: [names] (default: every rule's glyph), masters?: [ids] (master layers of existing glyphs to rebuild; default: every master), overwrite?: bool (default: the file's `# OverwriteExistingGlyphs:` line, else true), autoUnicodes?: bool (give glyphs whose rule names no code point the glyph database's one; default: the file's `# AutoUnicodes:` line, else true), markColor?: [r, g, b, a] | false (colour label for built glyphs whose rule sets none; default: the file's `# MarkGlyphs:` line, else none), preview?: bool (default false: return what would be built and store nothing)} build glyphs from construction rules, one per line, in file order (a later rule sees the glyphs earlier rules built). A rule is `name = parts`: `+` places a part over the one before it, `&` starts the next base after the advance of the bases before (`&\\` kerns it against them), `@x,y` positions a part by numbers, percentages (`50%`), anchors (`top` matches the mark's `_top`), guides, metrics (`xHeight`), bounds keywords (`center`, `top`, `bottom`, `left`, `right`, `inner…`) or backtick formulas (`` `top+10` ``); `@G:pos` measures from the part `G`, `~` mirrors, six values give a matrix and a position. `| 00C1` sets code points, `! r, g, b, a` a colour label, `^ width` or `^ left, right` the metrics (numbers or formulas over other glyphs: `A * 2`, `d'` for the opposite side), and `# text` the note. `*` before the name decomposes the result, `>` starts from the glyph's own drawing, `?` skips the rule when the glyph exists. `$name = value` declares a variable, `{name}` uses it. Components keep the positions the rules give them (automatic alignment off for them). Lines that are not rules and rules that cannot be built are listed in errors; locked glyphs, and existing glyphs when overwrite is off, are skipped → {built: [{glyph, line, created, layers: [{master, width, components: [{glyph, transform: [xx, xy, yx, yy, x, y]}], paths}], unicodes, note, missing: [names], warnings: [text]}], skipped: [{glyph, line, reason}], errors: [{line, text?, glyph?, message}], preview}",
            has_doc,
            build_from_constructions
        )
        .files(const { &[FileParam::read("path")] }),
    ]
}

/// What one call works with.
struct Job<'a> {
    file: &'a RuleFile,
    only: Option<HashSet<String>>,
    masters: Vec<String>,
    overwrite: bool,
    apply: ApplyOptions,
    data: Arc<GlyphData>,
    kerning: KernIndex,
}

#[derive(Default)]
struct Report {
    built: Vec<Value>,
    skipped: Vec<Value>,
    errors: Vec<Value>,
}

fn build_from_constructions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.buildFromConstructions", v)?;
    let text = match (p.opt_str("rules")?, p.opt_str("path")?) {
        (Some(t), None) => t.to_string(),
        (None, Some(path)) => read_rules(std::path::Path::new(path))?,
        (Some(_), Some(_)) => return Err(p.bad("give `rules` or `path`, not both")),
        (None, None) => return Err(p.bad("`rules` (the rules as text) or `path` (a rules file) is required")),
    };
    let file = parse_rules(&text).map_err(|e| p.bad(e.to_string()))?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let only = match p.opt_strings("glyphs")? {
        Some(list) if list.len() > MAX_RULES => return Err(p.bad(format!("`glyphs` names {} glyphs; at most {MAX_RULES}", list.len()))),
        Some(list) => Some(list.into_iter().map(str::to_string).collect::<HashSet<_>>()),
        None => None,
    };
    let masters = master_list(&p, font)?;
    let overwrite = if p.get("overwrite").is_some() { p.bool_or("overwrite", true)? } else { file.options.overwrite.unwrap_or(true) };
    let auto_unicodes = if p.get("autoUnicodes").is_some() { p.bool_or("autoUnicodes", true)? } else { file.options.auto_unicodes.unwrap_or(true) };
    let mark_color = match p.get("markColor") {
        None => file.options.mark_color.flatten(),
        Some(Value::Bool(false)) => None,
        Some(Value::Array(a)) => {
            let c: Vec<f64> = a.iter().filter_map(Value::as_f64).filter(|c| (0.0..=1.0).contains(c)).collect();
            match c.as_slice() {
                [r, g, b, alpha] if a.len() == 4 => Some([*r, *g, *b, *alpha]),
                _ => return Err(p.bad("`markColor` is four numbers from 0 to 1 (red, green, blue, alpha), or false")),
            }
        }
        Some(other) => return Err(p.bad(format!("`markColor` is four numbers from 0 to 1, or false; got {}", kind(other)))),
    };
    let preview = p.bool_or("preview", false)?;
    let job = Job { file: &file, only, masters, overwrite, apply: ApplyOptions { auto_unicodes, mark_color }, data, kerning: KernIndex::new(font) };
    let report = if preview {
        let mut work = Font::clone(font);
        run_rules(&mut work, &job)?
    } else {
        s.edit("Build Glyphs from Constructions", |f| run_rules(f, &job))?
    };
    Ok(json!({"built": report.built, "skipped": report.skipped, "errors": report.errors, "preview": preview}))
}

/// The masters `masters` names, each once, in the order given; every master without it.
fn master_list(p: &Params, font: &Font) -> Result<Vec<String>> {
    let Some(ids) = p.opt_strings("masters")? else { return Ok(font.masters.iter().map(|m| m.id.clone()).collect()) };
    if ids.is_empty() {
        return Err(p.bad("`masters` is empty"));
    }
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let m = font.master(id).map_err(|_| p.bad(format!("`masters`: there is no master `{id}`")))?;
        if !out.contains(&m.id) {
            out.push(m.id.clone());
        }
    }
    Ok(out)
}

/// Runs every rule of the job on `font`, in order.
fn run_rules(font: &mut Font, job: &Job<'_>) -> Result<Report> {
    let mut report = Report::default();
    for e in &job.file.errors {
        report.errors.push(json!({"line": e.line, "text": clip(&e.text), "message": e.message}));
    }
    let kern = |master: &str, first: &str, second: &str| {
        job.kerning.lookup(Direction::Ltr, master, first, second).ok().flatten().map_or(0.0, |applied| applied.value)
    };
    let all_masters: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    for rule in &job.file.rules {
        if job.only.as_ref().is_some_and(|only| !only.contains(&rule.glyph)) {
            continue;
        }
        let skip = |reason: &str| json!({"glyph": rule.glyph, "line": rule.line, "reason": reason});
        let existing = font.glyph(&rule.glyph).cloned();
        if let Some(g) = &existing {
            if rule.skip_existing {
                report.skipped.push(skip("the glyph exists and the rule starts with `?`"));
                continue;
            }
            if !job.overwrite {
                report.skipped.push(skip("the glyph exists and overwrite is off"));
                continue;
            }
            if g.info.locked {
                report.skipped.push(skip("the glyph is locked"));
                continue;
            }
        } else if font.glyphs.len() >= MAX_GLYPHS {
            report.errors.push(rule_error(rule, format!("the font would hold more than {MAX_GLYPHS} glyphs")));
            continue;
        }
        let masters = if existing.is_some() { &job.masters } else { &all_masters };
        let built = match build_rule(font, Some(&job.data), &kern, rule, masters) {
            Ok(b) => b,
            Err(e) => {
                report.errors.push(rule_error(rule, e.to_string()));
                continue;
            }
        };
        let mut glyph = construct_glyph(font, Some(&job.data), rule, &built, &job.apply);
        if rule.decompose
            && let Err(message) = decompose_built(font, &mut glyph, &built)
        {
            report.errors.push(rule_error(rule, message));
            continue;
        }
        if let Err(e) = font.check_glyph(&glyph) {
            report.errors.push(rule_error(rule, e.to_string()));
            continue;
        }
        report.built.push(summary(rule, &built, &glyph, existing.is_none()));
        if existing.is_none() {
            font.glyphs.push(glyph);
        } else if let Some(mut g) = font.glyphs.get_mut(&rule.glyph) {
            *g = glyph;
        }
    }
    Ok(report)
}

fn rule_error(rule: &Rule, message: String) -> Value {
    json!({"line": rule.line, "glyph": rule.glyph, "text": clip(&rule.text), "message": message})
}

/// At most 200 characters of a rule, for messages.
fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(200).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

/// The built master layers of `glyph` with their components decomposed; components of glyphs
/// the font lacks draw nothing and are dropped.
fn decompose_built(font: &Font, glyph: &mut Glyph, built: &Built) -> std::result::Result<(), String> {
    for b in &built.layers {
        let Some(layer) = glyph.layer_mut(&b.master) else { continue };
        let kept: Vec<Shape> = layer.shapes.iter().filter(|s| !matches!(s, Shape::Component(c) if font.glyph(&c.glyph).is_none())).cloned().collect();
        let before = layer.clone();
        layer.shapes = super::decompose::decompose_where(font, &before, &kept, |_| true)?;
    }
    Ok(())
}

/// What the result reports about one built glyph.
fn summary(rule: &Rule, built: &Built, glyph: &Glyph, created: bool) -> Value {
    let layers: Vec<Value> = built
        .layers
        .iter()
        .filter_map(|b| glyph.layer(&b.master))
        .map(|l| {
            let components: Vec<Value> = l
                .shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Component(c) => Some(json!({"glyph": c.glyph, "transform": c.transform.affine().as_coeffs()})),
                    _ => None,
                })
                .collect();
            let paths = l.shapes.iter().filter(|s| matches!(s, Shape::Path(_))).count();
            json!({"master": l.master, "width": l.width, "components": components, "paths": paths})
        })
        .collect();
    json!({
        "glyph": glyph.name,
        "line": rule.line,
        "created": created,
        "layers": layers,
        "unicodes": glyph.unicodes.iter().map(|u| format!("{u:04X}")).collect::<Vec<_>>(),
        "note": glyph.info.note,
        "missing": built.missing,
        "warnings": built.warnings,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_rules(path: &std::path::Path) -> Result<String> {
    use std::io::Read as _;
    let shown = path.display();
    let mut bytes = Vec::new();
    let cap = u64::try_from(MAX_RULES_BYTES).unwrap_or(u64::MAX).saturating_add(1);
    std::fs::File::open(path)
        .and_then(|f| f.take(cap).read_to_end(&mut bytes))
        .map_err(|e| EngineError::Io(format!("cannot read the rules file {shown}: {e}")))?;
    if bytes.len() > MAX_RULES_BYTES {
        return Err(EngineError::Io(format!("the rules file {shown} is larger than {} MB", MAX_RULES_BYTES >> 20)));
    }
    String::from_utf8(bytes).map_err(|_| EngineError::Io(format!("the rules file {shown} is not UTF-8 text")))
}

#[cfg(target_arch = "wasm32")]
fn read_rules(_: &std::path::Path) -> Result<String> {
    Err(EngineError::Unsupported("reading a rules file in the browser: pass the rules as `rules`".into()))
}
