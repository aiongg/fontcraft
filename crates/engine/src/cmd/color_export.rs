//! `color.describeExport`: which colour tables an export of an instance writes and why, with the
//! problems, from `fontcraft_color::export::plan`.
//!
//! Rows: `color.describeExport`.
//!
//! The report reads the same [`ExportPlan`] the compiler builds its tables from, so it cannot
//! disagree with an export: the switches resolved from the font's, the master's and the
//! instance's parameters, the tables, the glyphs and layers each table is made from, and every
//! warning or refusal the export would report.

use fontcraft_color::export::{
    self, ColrVersion, CpalPlan, ExportPlan, Problem, ProblemKind, SvgSource, Switches, plan_between_masters, plan_master, read_switches,
};
use fontcraft_color::paint::shape_refs;
use fontcraft_color::strike::Strike;
use fontcraft_model::{Font, Target};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "color.describeExport",
        "Describe Color Export",
        [],
        None,
        "{instance?: index | name (a static instance; `#n` for an unnamed one), master?: id (an export of that master), cbdt?: bool (default false; the export option that also writes CBDT and CBLC)} which colour tables an export writes, without exporting. Without `instance` or `master`, every static instance that exports, or the first master when there is none → {exports: [{instance: name | null, master: id | null, tables: [tag], switches: {colorLayersToSvg, colorLayersToColr, colorPaletteForCpal, colorPaletteForSvg, exportColrTable, exportSbixTable, exportSvgTable, sbixToSvg: ppem | null, cbdt}, cpal: {palettes, colors, version: 0 | 1, types: [palette type bits: 1 light, 2 dark], labels: [palette name | null], colorLabels: count of named colours} | null, colr: {version: 0 | 1, glyphs: [{glyph, layers: [layer id]}]} | null, sbix: {strikes: [{ppem, glyphs: [name]}]} | null, cbdt: {strikes: [{ppem, glyphs: [name]}]} | null, svg: {documents: [{glyph, source: 'paint' | 'svgLayer' | 'bitmap', layer?, path?, ppem?}]} | null, problems: [{kind: 'warning' | 'refusal', glyph: name | null, message}], refused: bool}]}",
        has_doc,
        describe
    )]
}

/// One export the report covers.
struct Job {
    /// The instance's name (`#n` when it has none); `None` for an export of a master.
    instance: Option<String>,
    target: Target,
}

fn instance_label(i: usize, name: &str) -> String {
    if name.is_empty() { format!("#{i}") } else { name.to_string() }
}

fn describe(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("color.describeExport", v)?;
    let cbdt = p.bool_or("cbdt", false)?;
    let font = s.font()?;
    let jobs = jobs(&p, font)?;
    let exports: Vec<Value> = jobs.iter().map(|job| report(font, job, cbdt)).collect();
    Ok(json!({ "exports": exports }))
}

/// The exports `p` names; with neither `instance` nor `master`, the ones a plain export writes.
fn jobs(p: &Params, font: &Font) -> Result<Vec<Job>> {
    match (p.opt_str("master")?, p.get("instance")) {
        (Some(_), Some(_)) => Err(p.bad("give `master` or `instance`, not both")),
        (Some(id), None) => {
            let master = font.master(id).map_err(|_| p.bad(format!("`master`: there is no master `{id}`")))?;
            Ok(vec![Job { instance: None, target: Target::Master(master.id.clone()) }])
        }
        (None, Some(which)) => {
            let index = match which {
                Value::String(name) => font
                    .instances
                    .iter()
                    .enumerate()
                    .position(|(i, inst)| inst.name == *name || instance_label(i, &inst.name) == *name)
                    .ok_or_else(|| p.bad(format!("`instance`: there is no instance `{name}`")))?,
                other => other
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < font.instances.len())
                    .ok_or_else(|| p.bad(format!("`instance` must be an instance index or name, found {other}")))?,
            };
            let inst = font.instances.get(index).ok_or_else(|| p.bad(format!("the font has no instance {index}")))?;
            if inst.kind.is_some() {
                return Err(p.bad(format!("`{}` is a variable font setting, not a static instance", instance_label(index, &inst.name))));
            }
            Ok(vec![Job { instance: Some(instance_label(index, &inst.name)), target: Target::Instance(index) }])
        }
        (None, None) => {
            let statics: Vec<Job> = font
                .instances
                .iter()
                .enumerate()
                .filter(|(_, inst)| inst.kind.is_none() && inst.exports)
                .map(|(i, inst)| Job { instance: Some(instance_label(i, &inst.name)), target: Target::Instance(i) })
                .collect();
            if !statics.is_empty() {
                return Ok(statics);
            }
            let master = font.default_master()?;
            Ok(vec![Job { instance: None, target: Target::Master(master.id.clone()) }])
        }
    }
}

/// The plan of one export, as `fontcraft_color::export::plan` makes it, with the CBDT option.
fn plan_for(font: &Font, job: &Job, cbdt: bool) -> (Option<Switches>, Option<String>, ExportPlan) {
    let mut switches = match read_switches(font, &job.target) {
        Ok(s) => s,
        Err(e) => {
            let problem = Problem { kind: ProblemKind::Refusal, glyph: None, message: e.to_string() };
            return (None, None, ExportPlan { problems: vec![problem], ..ExportPlan::default() });
        }
    };
    switches.cbdt = cbdt;
    let (master, plan) = match &job.target {
        Target::Master(id) => (Some(id.clone()), plan_master(font, id, &switches)),
        Target::Instance(i) => {
            let at = font.instances.get(*i).and_then(|inst| font.master_at(&inst.location));
            match at {
                Some(m) => (Some(m.id.clone()), plan_master(font, &m.id, &switches)),
                None => {
                    let label = job.instance.clone().unwrap_or_else(|| format!("#{i}"));
                    (None, plan_between_masters(font, &label, &switches))
                }
            }
        }
    };
    (Some(switches), master, plan)
}

fn report(font: &Font, job: &Job, cbdt: bool) -> Value {
    let (switches, master, plan) = plan_for(font, job, cbdt);
    json!({
        "instance": job.instance,
        "master": master,
        "tables": plan.tables(),
        "switches": switches.as_ref().map(switches_json),
        "cpal": plan.cpal.as_ref().map(cpal_json),
        "colr": plan.colr.as_ref().map(|c| json!({
            "version": match c.version { ColrVersion::V0 => 0, ColrVersion::V1 => 1 },
            "glyphs": c.glyphs.iter().map(|g| json!({"glyph": g.glyph, "layers": layer_ids(&g.paint)})).collect::<Vec<_>>(),
        })),
        "sbix": plan.sbix.as_ref().map(|p| json!({"strikes": strikes_json(&p.strikes)})),
        "cbdt": plan.cbdt.as_ref().map(|p| json!({"strikes": strikes_json(&p.strikes)})),
        "svg": plan.svg.as_ref().map(|p| json!({"documents": p.documents.iter().map(document_json).collect::<Vec<_>>()})),
        "problems": plan.problems.iter().map(problem_json).collect::<Vec<_>>(),
        "refused": plan.refused(),
    })
}

fn switches_json(s: &Switches) -> Value {
    json!({
        "colorLayersToSvg": s.color_layers_to_svg,
        "colorLayersToColr": s.color_layers_to_colr,
        "colorPaletteForCpal": s.palette_for_cpal,
        "colorPaletteForSvg": s.palette_for_svg,
        "exportColrTable": s.export_colr,
        "exportSbixTable": s.export_sbix,
        "exportSvgTable": s.export_svg,
        "sbixToSvg": s.sbix_to_svg,
        "cbdt": s.cbdt,
    })
}

/// The ids of the layers a glyph's paint draws, each once, bottom first.
fn layer_ids(paint: &fontcraft_color::Paint) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in shape_refs(paint) {
        if !out.contains(&r.layer) {
            out.push(r.layer.clone());
        }
    }
    out
}

/// The palettes and colours `CPAL` holds, its version, and each palette's type and label.
fn cpal_json(c: &CpalPlan) -> Value {
    let labels: Vec<Option<&str>> = (0..c.palettes.palettes().len()).map(|i| c.info.get(i).and_then(|p| p.name.as_deref())).collect();
    json!({
        "palettes": c.palettes.palettes().len(),
        "colors": c.palettes.rows(),
        "version": if c.labelled() { 1 } else { 0 },
        "types": c.palette_types(),
        "labels": labels,
        "colorLabels": c.color_names.iter().filter(|n| n.is_some()).count(),
    })
}

fn strikes_json(strikes: &[Strike]) -> Vec<Value> {
    strikes.iter().map(|s| json!({"ppem": s.ppem, "glyphs": s.glyphs.iter().map(|g| g.glyph.as_str()).collect::<Vec<_>>()})).collect()
}

fn document_json(d: &export::SvgDocument) -> Value {
    match &d.source {
        SvgSource::Paint(_) => json!({"glyph": d.glyph, "source": "paint"}),
        SvgSource::SvgLayer { layer, path } => json!({"glyph": d.glyph, "source": "svgLayer", "layer": layer, "path": path}),
        SvgSource::Bitmap { layer, ppem } => json!({"glyph": d.glyph, "source": "bitmap", "layer": layer, "ppem": ppem}),
    }
}

fn problem_json(p: &Problem) -> Value {
    let kind = match p.kind {
        ProblemKind::Warning => "warning",
        ProblemKind::Refusal => "refusal",
    };
    json!({"kind": kind, "glyph": p.glyph, "message": p.message})
}
