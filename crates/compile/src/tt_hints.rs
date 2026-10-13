//! TrueType hinting at export: `cvt `, `fpgm`, `prep` and the glyph programs.
//!
//! Rows: `hinting.tt.export.manualWhenAutohintOff`, `hinting.tt.export.compileError`,
//! `hinting.tt.export.instanceValues`, `hinting.tt.replicateToOtherMasters`,
//! `hinting.tt.param.controlValueProgram`, `Export TrueType Hints`, `cvt Table`,
//! `prep Table Assembly`, `fpgm Table Assembly`, `TTFMinimumDistance`.
//!
//! | Case | What the font gets |
//! |---|---|
//! | hints stripped, Export TrueType Hints off, or no glyph with instructions and no imported tables | no TrueType hinting |
//! | Autohint on (the instance's Autohint parameter, else the export option) | the autohinter's programs, which replace every manual instruction |
//! | cvt Table, prep Table Assembly or fpgm Table Assembly on the master or instance | those tables as they are; manual instructions are not compiled, with a warning, since they call functions of our own `fpgm` |
//! | otherwise | our `cvt `, `fpgm` and `prep` for the zones and stems of the master or instance being built, and a glyph program for each glyph whose main-master layer has instructions |
//!
//! Glyph programs: the instructions of the main master's layer, compiled against the point
//! numbers of the outline being written ([`crate::points`]) with the zones, stems and Delta
//! column of the build. Composites get none. A glyph whose instructions cannot be compiled is
//! exported without them, with a warning naming the glyph and the reason. `maxp` takes the
//! largest needs [`fontcraft_ttasm::analyse`] finds in `fpgm`, `prep` and every glyph program.
//! A variable font gets the default master's programs without Delta instructions, and no
//! `cvar` (a warning says so).

use std::sync::Arc;

use fontcraft_hinting::settings::{TTF_STEMS, TTF_ZONES, family_tt_zones, main_master};
use fontcraft_hinting::tt::delta_column_key;
use fontcraft_hinting::{FontPrograms, GlyphContext, GlyphOutline, HintSettings, glyph_program, tt_instructions};
use fontcraft_model::plist::{Array, Dict, Number, Value};
use fontcraft_model::{Font, Layer, Param, Resolved, ResolvedGlyph};
use fontcraft_ttasm::{FunctionTable, ProgramLimits, analyse};
use write_fonts::tables::glyf::SimpleGlyph;

use crate::outline::Prepared;
use crate::points::exported;
use crate::{CompileError, Options};

/// `maxStackElements` written when the analysis could not follow an imported program, so the
/// font does not under-state what it needs.
pub const UNCERTAIN_STACK: u16 = 1024;
/// Most glyphs named in one warning.
const MAX_NAMED: usize = 10;

/// The TrueType hinting of a font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TtHinting {
    pub cvt: Vec<i16>,
    pub fpgm: Vec<u8>,
    pub prep: Vec<u8>,
    /// Glyphs written with a program, by index in the glyph order, each with its points.
    pub glyphs: Vec<(usize, SimpleGlyph)>,
    /// The merged needs for `maxp`.
    pub limits: ProgramLimits,
}

/// The hint settings of the master or instance `resolved` was resolved at, or `None` with a
/// warning when its hinting parameters cannot be read.
pub(crate) fn settings_for(source: &Font, resolved: &Resolved, warnings: &mut Vec<String>) -> Option<HintSettings> {
    let read = match resolved.instance.filter(|i| source.instances.get(*i).is_some()) {
        Some(i) => HintSettings::for_instance(source, i, &resolved.master),
        None => HintSettings::for_master(source, &resolved.master),
    };
    match read {
        Ok(s) => Some(s),
        Err(e) => {
            warnings.push(format!("the hinting parameters cannot be read ({e}); the font was written without TrueType hints"));
            None
        }
    }
}

fn names(list: &[String]) -> String {
    let mut out = list.iter().take(MAX_NAMED).map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
    if list.len() > MAX_NAMED {
        out.push_str(&format!(" and {} more", list.len() - MAX_NAMED));
    }
    out
}

/// The TrueType hinting for the glyphs of `order` with outlines `outlines`, or `None` when the
/// font gets none. `source` is the font `resolved` was resolved from; `variable`: the font is a
/// variable font whose outlines are the default master's.
#[allow(clippy::too_many_arguments)]
pub fn tt_hinting(
    source: &Font,
    resolved: &Resolved,
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    options: &Options,
    variable: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<TtHinting>, CompileError> {
    if options.hinting.strip {
        return Ok(None);
    }
    let Some(settings) = settings_for(source, resolved, warnings) else { return Ok(None) };
    if !settings.export_tt_hints {
        return Ok(None);
    }

    // The glyphs whose main-master layer has instructions, with them.
    let mut instructed = Vec::new();
    let mut unreadable = Vec::new();
    for (gid, g) in order.iter().enumerate() {
        let Some(layer) = source.glyph(&g.name).and_then(|sg| sg.layer(&settings.main_master)) else { continue };
        match tt_instructions(layer) {
            Ok(list) if !list.is_empty() => instructed.push((gid, list.into_iter().map(|(_, t)| t).collect::<Vec<_>>())),
            Ok(_) => {}
            Err(e) => unreadable.push(format!("{} ({e})", g.name)),
        }
    }
    if !unreadable.is_empty() {
        warnings
            .push(format!("the TrueType instructions of these glyphs cannot be read, so they were exported without them: {}", unreadable.join(", ")));
    }

    if settings.autohint.unwrap_or(options.hinting.autohint) {
        // The TrueType autohinter replaces every manual instruction; until it is built the
        // font is written unhinted.
        warnings.push("TrueType autohinting is not available, so the font was written without TrueType hints".to_string());
        return Ok(None);
    }

    let imported = settings.cvt.is_some() || settings.prep_assembly.is_some() || settings.fpgm_assembly.is_some();
    if imported {
        return Ok(imported_tables(&settings, &instructed, order, warnings));
    }
    if instructed.is_empty() {
        return Ok(None);
    }

    let family = match family_tt_zones(source) {
        Ok(z) => z,
        Err(e) => {
            warnings.push(format!("the family TT zones cannot be read ({e}); the font was written without TrueType hints"));
            return Ok(None);
        }
    };
    let instance = settings.instance.and_then(|i| source.instances.get(i));
    let column = instance.map_or(settings.master.clone(), |i| i.name.clone());
    let programs: FontPrograms = match fontcraft_hinting::font_programs(&settings, &family, &column) {
        Ok(p) => p,
        Err(e) => {
            warnings.push(format!("the TrueType font programs cannot be built ({e}); the font was written without TrueType hints"));
            return Ok(None);
        }
    };
    warnings.extend(programs.warnings.iter().cloned());
    let functions = match FunctionTable::from_fpgm(&programs.fpgm) {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!("the TrueType font program cannot be read back ({e}); the font was written without TrueType hints"));
            return Ok(None);
        }
    };
    let location = instance.map(|i| i.location.clone()).or_else(|| source.master(&settings.master).ok().map(|m| m.location.clone()));
    let delta_column = if variable { None } else { location.map(|l| delta_column_key(&l)) };

    let mut limits = programs.limits;
    let mut glyphs = Vec::new();
    let mut deltas_dropped = false;
    for (gid, instructions) in &instructed {
        let (Some(g), Some(prepared)) = (order.get(*gid), outlines.get(*gid)) else { continue };
        if matches!(prepared, Prepared::Empty | Prepared::Composite(_)) {
            continue;
        }
        let Some(built) = source.glyph(&g.name).and_then(|sg| sg.layer(&settings.master)) else { continue };
        let Some(e) = exported(built, instructions, prepared, options.remove_overlap && !variable) else {
            warnings.push(format!("glyph `{}`: its exported points cannot be numbered, so it was exported without TrueType instructions", g.name));
            continue;
        };
        deltas_dropped |= variable && instructions.iter().any(|t| matches!(t.kind, fontcraft_hinting::TtKind::Delta { .. }));
        let zones = settings.tt_zones_for(source, &g.name);
        let stems = settings.tt_stems_for(source, &g.name);
        let cx = GlyphContext {
            glyph: &g.name,
            instructions,
            points: &e.map,
            outline: GlyphOutline { points: &e.outline, contour_ends: &e.contour_ends, advance: crate::ot_round(g.width) },
            layout: &programs.layout,
            functions: &functions,
            settings: &settings,
            zones: Some(&zones),
            stems: Some(&stems),
            delta_column: delta_column.as_deref(),
        };
        match glyph_program(&cx) {
            Ok(p) => {
                warnings.extend(p.warnings.iter().map(|w| format!("glyph {w}")));
                if p.bytes.is_empty() {
                    continue;
                }
                limits.merge(&p.limits);
                let mut glyph = e.glyph;
                glyph.instructions = p.bytes;
                glyphs.push((*gid, glyph));
            }
            Err(err) => warnings
                .push(format!("glyph `{}`: there was a problem compiling its TrueType instructions ({err}); it was exported without them", g.name)),
        }
    }
    if variable {
        warnings.push(
            "the variable font's TrueType programs use the default master's zones and stems: no `cvar` table is written, so zones and stems do not follow the other masters"
                .to_string(),
        );
    }
    if deltas_dropped {
        warnings.push("Delta instructions are not written into a variable font".to_string());
    }
    finish(&mut limits, warnings);
    Ok(Some(TtHinting { cvt: programs.cvt, fpgm: programs.fpgm, prep: programs.prep, glyphs, limits }))
}

/// The tables the cvt Table, prep Table Assembly and fpgm Table Assembly parameters hold.
fn imported_tables(
    settings: &HintSettings,
    instructed: &[(usize, Vec<fontcraft_hinting::TtInstruction>)],
    order: &[ResolvedGlyph],
    warnings: &mut Vec<String>,
) -> Option<TtHinting> {
    let program = |text: &Option<String>, name: &str, warnings: &mut Vec<String>| -> Option<Vec<u8>> {
        let text = text.as_deref()?;
        match fontcraft_ttasm::assemble(text) {
            Ok(b) => Some(b),
            Err(e) => {
                warnings.push(format!("the {name} parameter cannot be assembled ({e}), so it was left out"));
                None
            }
        }
    };
    let fpgm = program(&settings.fpgm_assembly, "fpgm Table Assembly", warnings).unwrap_or_default();
    let prep = program(&settings.prep_assembly, "prep Table Assembly", warnings).unwrap_or_default();
    let cvt = settings.cvt.clone().unwrap_or_default();
    if !instructed.is_empty() {
        let list: Vec<String> = instructed.iter().filter_map(|(gid, _)| order.get(*gid).map(|g| g.name.clone())).collect();
        warnings.push(format!(
            "the master carries imported TrueType tables (cvt Table, prep or fpgm Table Assembly), so the manual instructions of {} were not compiled; remove those parameters to export them",
            names(&list)
        ));
    }
    if fpgm.is_empty() && prep.is_empty() && cvt.is_empty() {
        return None;
    }
    let mut limits = ProgramLimits::default();
    let functions = FunctionTable::from_fpgm(&fpgm).unwrap_or_default();
    for p in [&fpgm, &prep] {
        match analyse(p, &functions) {
            Ok(l) => limits.merge(&l),
            Err(_) => limits.uncertain = true,
        }
    }
    limits.max_size_of_instructions = 0;
    finish(&mut limits, warnings);
    Some(TtHinting { cvt, fpgm, prep, glyphs: Vec::new(), limits })
}

/// Makes `limits` safe to write when the analysis could not follow every program.
fn finish(limits: &mut ProgramLimits, warnings: &mut Vec<String>) {
    if limits.uncertain {
        limits.max_stack_elements = limits.max_stack_elements.max(UNCERTAIN_STACK);
        limits.max_zones = 2;
        warnings.push(format!(
            "the TrueType programs use values the analysis cannot follow; maxp states {} stack elements and both zones",
            limits.max_stack_elements
        ));
    }
    limits.max_zones = limits.max_zones.max(1);
}

/// A master parameter that carries each master's TT zone positions and sizes and TT stem widths
/// through interpolation, as a list of numbers in the main master's order. Only the compiler
/// adds it, to the font it interpolates a static instance from.
const TT_VALUES: &str = "TT zone and stem values (compiler)";

/// `font` with [`TT_VALUES`] on every master, when a master states TTFZones or TTFStems; `None`
/// when no master does or the values cannot be read (the instance then keeps the origin
/// master's).
pub(crate) fn with_tt_values(font: &Font) -> Option<Font> {
    if !font.masters.iter().any(|m| m.params.get(TTF_ZONES).is_some() || m.params.get(TTF_STEMS).is_some()) {
        return None;
    }
    let mut lists = Vec::with_capacity(font.masters.len());
    for m in font.masters.iter() {
        let s = HintSettings::for_master(font, &m.id).ok()?;
        let values: Vec<Value> = s
            .tt_zones
            .iter()
            .flat_map(|z| [z.position, z.size])
            .chain(s.tt_stems.iter().map(|t| t.size))
            .map(|v| Number::float(v).map(Value::Number))
            .collect::<Option<_>>()?;
        lists.push(values);
    }
    let mut out = font.clone();
    for (m, values) in Arc::make_mut(&mut out.masters).iter_mut().zip(lists) {
        m.params.0.push(Param::new(TT_VALUES, Value::Array(Array { items: values, ..Default::default() })));
    }
    Some(out)
}

/// Finishes a static instance font `out` interpolated from `font` (after [`with_tt_values`]):
/// each glyph takes the hints of its main-master layer, the only layer whose TrueType
/// instructions count, when that layer has hints and the same paths; the instance master's
/// TTFZones and TTFStems take the interpolated positions, sizes and widths
/// (`hinting.tt.export.instanceValues`).
pub(crate) fn finish_instance(font: &Font, out: &mut Font) {
    let main = main_master(font);
    let shape = |l: &Layer| -> Vec<usize> {
        l.shapes
            .iter()
            .filter_map(|s| match s {
                fontcraft_model::Shape::Path(p) => Some(p.nodes.len()),
                _ => None,
            })
            .collect()
    };
    for i in 0..out.glyphs.len() {
        let Some(name) = out.glyphs.as_arcs().get(i).map(|g| g.name.clone()) else { continue };
        let Some(source) = font.glyph(&name).and_then(|g| g.layer(&main).cloned()) else { continue };
        if source.info.hints.is_empty() {
            continue;
        }
        let Some(mut g) = out.glyphs.get_mut_at(i) else { continue };
        for layer in g.layers.iter_mut().filter(|l| l.is_master_layer()) {
            if layer.info.hints != source.info.hints && shape(layer) == shape(&source) {
                layer.info.hints.clone_from(&source.info.hints);
            }
        }
    }

    let masters = Arc::make_mut(&mut out.masters);
    for m in masters.iter_mut() {
        let Some(pos) = m.params.0.iter().position(|p| p.name == TT_VALUES) else { continue };
        let values: Vec<f64> = match m.params.0.remove(pos).value {
            Value::Array(a) => a.items.iter().filter_map(|v| v.as_num().map(|n| n.as_f64())).collect(),
            _ => continue,
        };
        let Ok(main_settings) = HintSettings::for_master(font, &main) else { continue };
        let (zones, stems) = (&main_settings.tt_zones, &main_settings.tt_stems);
        if values.len() != zones.len() * 2 + stems.len() {
            continue;
        }
        let set = |d: &mut Dict, keys: [&str; 2], v: f64| {
            let key = keys.into_iter().find(|k| d.get(k).is_some()).unwrap_or(keys[0]);
            if let Some(n) = Number::float(v) {
                d.insert(key, Value::Number(n));
            }
        };
        for p in m.params.0.iter_mut().filter(|p| !p.disabled) {
            let Value::Array(items) = &mut p.value else { continue };
            for (i, item) in items.items.iter_mut().enumerate() {
                let Value::Dict(d) = item else { continue };
                let name = d.get("name").and_then(Value::as_str).map(str::to_string);
                if p.name == TTF_ZONES {
                    let what = name.unwrap_or_else(|| format!("zone {}", i + 1));
                    let Some(z) = zones.iter().position(|z| z.name == what) else { continue };
                    let (Some(&at), Some(&size)) = (values.get(2 * z), values.get(2 * z + 1)) else { continue };
                    set(d, ["position", "pos"], at);
                    set(d, ["size", "size"], size);
                } else if p.name == TTF_STEMS {
                    let what = name.unwrap_or_else(|| format!("stem {}", i + 1));
                    let Some(s) = stems.iter().position(|s| s.name == what) else { continue };
                    let Some(&width) = values.get(2 * zones.len() + s) else { continue };
                    set(d, ["width", "size"], width);
                }
            }
        }
    }
}
