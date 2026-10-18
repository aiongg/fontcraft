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
//! | Autohint on (the instance's Autohint parameter, else the export option) | our `cvt `, `fpgm` and `prep` with what the TTFAutohint options add, and the autohinter's program for every simple glyph ([`fontcraft_hinting::autohint_glyph`] on the layer being exported); manual instructions and imported tables are not written; with the Dehint option, no TrueType hinting |
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

use fontcraft_hinting::control::ControlInstructions;
use fontcraft_hinting::settings::{TTF_STEMS, TTF_ZONES, family_tt_zones, main_master};
use fontcraft_hinting::tt::delta_column_key;
use fontcraft_hinting::{
    AutoGlyph, FontPrograms, GlyphContext, GlyphOutline, HintSettings, TtAutohintOptions, autohint_glyph, glyph_program, tt_instructions,
};
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
    /// Appended to the version string (name ID 5): the autohinter's Detailed info.
    pub version_suffix: Option<String>,
    /// The TTFA table the autohinter's TTFA table option asks for: its options and control
    /// instructions as text.
    pub ttfa: Option<Vec<u8>>,
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

fn autohinted(settings: &HintSettings, options: &Options) -> bool {
    settings.autohint.unwrap_or(options.hinting.autohint)
}

/// The TTFAutohint options and control instructions of an autohinted build.
struct AutoSetup {
    options: TtAutohintOptions,
    control: ControlInstructions,
    /// The control instructions as given, for the TTFA table.
    control_text: String,
}

impl AutoSetup {
    /// The options and control instructions of `settings`, or `None` when Dehint is on (the
    /// font then gets no TrueType hinting). Options or control instructions that cannot be read
    /// are left out, with a warning.
    fn new(settings: &HintSettings, warnings: &mut Vec<String>) -> Option<Self> {
        let options = match settings.tt_autohint_options.as_deref().map(TtAutohintOptions::parse) {
            None => TtAutohintOptions::default(),
            Some(Ok((o, ignored))) => {
                if !ignored.is_empty() {
                    warnings.push(format!("the built-in TrueType autohinter ignores these TTFAutohint options: {}", ignored.join(", ")));
                }
                o
            }
            Some(Err(e)) => {
                warnings.push(format!("the TTFAutohint options cannot be read ({e}); the defaults were used"));
                TtAutohintOptions::default()
            }
        };
        if options.dehint {
            return None;
        }
        let control = match settings.tt_autohint_control.as_deref().map(fontcraft_hinting::control::parse) {
            None => ControlInstructions::default(),
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                warnings.push(format!("the TTFAutohint control instructions cannot be read ({e}); they were left out"));
                ControlInstructions::default()
            }
        };
        Some(Self { options, control, control_text: settings.tt_autohint_control.clone().unwrap_or_default() })
    }

    /// The TTFA table: the options, then the control instructions, as text.
    fn ttfa(&self) -> Vec<u8> {
        let mut text = self.options.to_text();
        if !self.control_text.is_empty() {
            text.push('\n');
            text.push_str(&self.control_text);
        }
        text.into_bytes()
    }

    /// Does the Dehint option of `settings` leave the font without TrueType hinting?
    fn dehint(settings: &HintSettings) -> bool {
        Self::new(settings, &mut Vec::new()).is_none()
    }
}

/// Does the master carry imported tables (cvt Table, prep or fpgm Table Assembly)? They are
/// written instead of glyph programs.
fn imported(settings: &HintSettings) -> bool {
    settings.cvt.is_some() || settings.prep_assembly.is_some() || settings.fpgm_assembly.is_some()
}

/// Does [`tt_hinting`] compile glyph programs for the font `resolved` was resolved from with
/// `options`? The checks it makes before it looks at any glyph.
pub(crate) fn compiles_glyph_programs(source: &Font, resolved: &Resolved, options: &Options) -> bool {
    if options.hinting.strip {
        return false;
    }
    settings_for(source, resolved, &mut Vec::new())
        .is_some_and(|s| s.export_tt_hints && if autohinted(&s, options) { !AutoSetup::dehint(&s) } else { !imported(&s) })
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
    win_metrics: (f64, f64),
    variable: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<TtHinting>, CompileError> {
    if options.hinting.strip {
        return Ok(None);
    }
    let Some(mut settings) = settings_for(source, resolved, warnings) else { return Ok(None) };
    if !settings.export_tt_hints {
        return Ok(None);
    }

    // The glyphs whose main-master layer has instructions, with them.
    let mut instructed: Vec<(usize, Vec<fontcraft_hinting::TtInstruction>)> = Vec::new();
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

    let auto = if autohinted(&settings, options) {
        let Some(auto) = AutoSetup::new(&settings, warnings) else { return Ok(None) };
        if !instructed.is_empty() {
            let list: Vec<String> = instructed.iter().filter_map(|(gid, _)| order.get(*gid).map(|g| g.name.clone())).collect();
            warnings.push(format!("Autohint is on, so the TrueType autohinter replaced the manual instructions of {}", names(&list)));
        }
        instructed = (0..order.len()).map(|gid| (gid, Vec::new())).collect();
        if auto.options.windows_compatibility {
            // Zones at the heights Windows clips to, so extremes there land on a pixel row.
            let (ascent, descent) = win_metrics;
            for (name, position, size) in [("winAscent", ascent, 1.0), ("winDescent", -descent, -1.0)] {
                if position.is_finite() && !settings.tt_zones.iter().any(|z| z.name == name) {
                    settings.tt_zones.push(fontcraft_hinting::TtZone { name: name.into(), position, size, ..Default::default() });
                }
            }
        }
        Some(auto)
    } else {
        if imported(&settings) {
            return Ok(imported_tables(&settings, &instructed, order, warnings));
        }
        None
    };
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
    let mut programs = programs;
    if let Some(auto) = &auto {
        match fontcraft_hinting::autohint_prep(&auto.options, &settings, &programs.layout) {
            Ok(extra) => {
                match analyse(&extra, &functions) {
                    Ok(l) => programs.limits.merge(&l),
                    Err(_) => programs.limits.uncertain = true,
                }
                programs.prep.extend(extra);
            }
            Err(e) => warnings.push(format!("the TTFAutohint options cannot be applied to `prep` ({e}); they were left out")),
        }
    }
    let location = instance.map(|i| i.location.clone()).or_else(|| source.master(&settings.master).ok().map(|m| m.location.clone()));
    let delta_column = if variable { None } else { location.map(|l| delta_column_key(&l)) };

    let per_glyph = auto.as_ref().map(|a| a.control.by_glyph()).unwrap_or_default();
    let no_control = ControlInstructions::default();
    let mut limits = programs.limits;
    let mut glyphs = Vec::new();
    let mut deltas_dropped = false;
    for (gid, instructions) in &instructed {
        let (Some(g), Some(prepared)) = (order.get(*gid), outlines.get(*gid)) else { continue };
        if matches!(prepared, Prepared::Empty | Prepared::Composite(_)) {
            continue;
        }
        let Some(built) = source.glyph(&g.name).and_then(|sg| sg.layer(&settings.master)) else { continue };
        let zones = settings.tt_zones_for(source, &g.name);
        let stems = settings.tt_stems_for(source, &g.name);
        let autohinted;
        let instructions: &[fontcraft_hinting::TtInstruction] = match &auto {
            Some(auto) => {
                let Some(e) = exported(built, &[], prepared, options.remove_overlap && !variable) else { continue };
                let control = per_glyph.get(g.name.as_str()).unwrap_or(&no_control);
                let input = AutoGlyph {
                    glyph: &g.name,
                    layer: built,
                    settings: &settings,
                    zones: Some(&zones),
                    stems: Some(&stems),
                    options: &auto.options,
                    control,
                    points: Some(&e.map),
                    delta_column: delta_column.as_deref(),
                };
                match autohint_glyph(&input) {
                    Ok(h) => {
                        warnings.extend(h.warnings);
                        autohinted = h.instructions;
                    }
                    Err(err) => {
                        warnings.push(format!("glyph `{}`: the TrueType autohinter failed ({err}); it was exported without instructions", g.name));
                        continue;
                    }
                }
                if autohinted.is_empty() {
                    continue;
                }
                &autohinted
            }
            None => instructions,
        };
        let Some(e) = exported(built, instructions, prepared, options.remove_overlap && !variable) else {
            warnings.push(format!("glyph `{}`: its exported points cannot be numbered, so it was exported without TrueType instructions", g.name));
            continue;
        };
        deltas_dropped |= variable && instructions.iter().any(|t| matches!(t.kind, fontcraft_hinting::TtKind::Delta { .. }));
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
    let (version_suffix, ttfa) = match &auto {
        Some(a) => (
            a.options.detailed_info.then(|| format!("; FontCraft TrueType autohinter ({})", a.options.to_text())),
            a.options.ttfa_table.then(|| a.ttfa()),
        ),
        None => (None, None),
    };
    Ok(Some(TtHinting { cvt: programs.cvt, fpgm: programs.fpgm, prep: programs.prep, glyphs, limits, version_suffix, ttfa }))
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
    Some(TtHinting { cvt, fpgm, prep, glyphs: Vec::new(), limits, version_suffix: None, ttfa: None })
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
        let Some(glyph) = font.glyph(&name) else { continue };
        let Some(source) = glyph.layer(&main).filter(|l| !l.info.hints.is_empty()) else { continue };
        let needs = out
            .glyphs
            .as_arcs()
            .get(i)
            .is_some_and(|g| g.layers.iter().any(|l| l.is_master_layer() && l.info.hints != source.info.hints && shape(l) == shape(source)));
        if !needs {
            continue;
        }
        let Some(mut g) = out.glyphs.get_mut_at(i) else { continue };
        for layer in g.layers.iter_mut().filter(|l| l.is_master_layer()) {
            if layer.info.hints != source.info.hints && shape(layer) == shape(source) {
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
