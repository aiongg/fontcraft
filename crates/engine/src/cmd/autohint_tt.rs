//! TrueType autohinting of glyphs in the font (`fontcraft_hinting::autohint_glyph`): the
//! autohinter's Snap and Stem instructions stored on the layers as TrueType instructions, which
//! from then on count as manual ones.
//!
//! Rows: `autohint.tt.run`, `hint.tt.autohintGlyph`.
//!
//! Each glyph is hinted on the layer its instructions live on (the main master's, unless a layer
//! is named) with the TT zones and stems that apply to it there. Control instructions address
//! points by their exported numbers (`hint.tt.getPointIndexes`). Every run is one undo step; a
//! refusal (an unknown glyph, master or instance, options or control instructions that cannot be
//! read, a locked glyph) changes nothing. A glyph the autohinter cannot hint is listed under
//! `skipped` and left as it was.

use std::collections::HashSet;

use fontcraft_compile::Options;
use fontcraft_compile::points::point_indexes;
use fontcraft_hinting::control::{self, ControlInstructions};
use fontcraft_hinting::settings::main_master;
use fontcraft_hinting::tt::delta_column_key;
use fontcraft_hinting::{AutoGlyph, HintSettings, PointMap, TtAutohintOptions, TtInstruction, autohint_glyph, set_tt_instructions};
use fontcraft_model::{Font, Target};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

/// Most glyphs one run hints. The autohinter bounds the work per glyph by its node limit
/// (`fontcraft_hinting::tt_auto::MAX_AUTOHINT_NODES`).
const MAX_GLYPHS_PER_RUN: usize = 1 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "autohint.tt.run",
            "Autohint TrueType",
            [],
            None,
            "{glyphs?: [names] (default: every exporting glyph; locked glyphs are then skipped, a named locked glyph is refused), excludeGlyphs?: [names], instance?: name (read its TTFAutohint options and control instructions), options?: string (TTFAutohint options as long command-line options; replaces the instance's), controlInstructions?: string (replaces the instance's)} run the TrueType autohinter on each glyph's main-master layer and store its Snap and Stem instructions there, replacing the layer's instructions; one undo step → {hinted: [{glyph, instructions}], skipped: [{glyph, reason}], ignoredOptions: [names], warnings}",
            has_doc,
            run
        ),
        cmd!(
            "hint.tt.autohintGlyph",
            "Autohint",
            [],
            Some("Cmd+Ctrl+Alt+Shift+I"),
            "{glyphs: [names], layer?: master id (default: the main master)} generate Snap and Stem instructions for each glyph's layer of that master as a starting point, replacing the instructions it has; one undo step → {hinted: [{glyph, instructions}], skipped: [{glyph, reason}], ignoredOptions: [names], warnings}",
            has_doc,
            autohint_glyph_cmd
        ),
    ]
}

/// One glyph layer hinted, ready to store.
struct Hinted {
    glyph: String,
    instructions: Vec<TtInstruction>,
}

#[derive(Default)]
struct Outcome {
    hinted: Vec<Hinted>,
    skipped: Vec<Value>,
    ignored: Vec<String>,
    warnings: Vec<String>,
}

fn run(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("autohint.tt.run", v)?;
    let font = s.font()?;
    let named = p.opt_glyphs("glyphs", font)?;
    let explicit = named.is_some();
    let excluded: HashSet<String> = p.opt_strings("excludeGlyphs")?.unwrap_or_default().into_iter().map(str::to_string).collect();
    let glyphs: Vec<String> = named
        .unwrap_or_else(|| font.glyphs.iter().filter(|g| g.export).map(|g| g.name.clone()).collect())
        .into_iter()
        .filter(|g| !excluded.contains(g))
        .collect();
    let main = main_master(font);
    let mut settings = match p.opt_str("instance")? {
        Some(name) => {
            let i = font.instances.iter().position(|i| i.name == name).ok_or_else(|| p.bad(format!("the font has no instance named `{name}`")))?;
            HintSettings::for_instance(font, i, &main)
        }
        None => HintSettings::for_master(font, &main),
    }
    .map_err(|e| p.bad(format!("the hinting parameters cannot be read: {e}")))?;
    if let Some(text) = p.opt_str("options")? {
        settings.tt_autohint_options = Some(text.to_string());
    }
    if let Some(text) = p.opt_str("controlInstructions")? {
        settings.tt_autohint_control = Some(text.to_string());
    }
    let outcome = hint_layers(&p, font, &glyphs, &main, &settings, explicit)?;
    store(s, "Autohint TrueType", &main, outcome)
}

fn autohint_glyph_cmd(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.autohintGlyph", v)?;
    let font = s.font()?;
    let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    let master = match p.opt_str("layer")? {
        Some(id) => font.master(id).map_err(|_| p.bad(format!("`layer`: there is no master `{id}`")))?.id.clone(),
        None => main_master(font),
    };
    let settings = HintSettings::for_master(font, &master).map_err(|e| p.bad(format!("the hinting parameters cannot be read: {e}")))?;
    let outcome = hint_layers(&p, font, &glyphs, &master, &settings, true)?;
    store(s, "Autohint", &master, outcome)
}

/// Hints `glyphs` on their `master` layers without changing the font. A locked glyph refuses the
/// run when the glyphs were `explicit`ly named, and is skipped otherwise.
fn hint_layers(p: &Params, font: &Font, glyphs: &[String], master: &str, settings: &HintSettings, explicit: bool) -> Result<Outcome> {
    if glyphs.len() > MAX_GLYPHS_PER_RUN {
        return Err(p.bad(format!("{} glyphs: one run hints at most {MAX_GLYPHS_PER_RUN}", glyphs.len())));
    }
    let mut out = Outcome::default();
    let options = match settings.tt_autohint_options.as_deref() {
        Some(text) => {
            let (o, ignored) = TtAutohintOptions::parse(text).map_err(|e| p.bad(format!("the TTFAutohint options cannot be read: {e}")))?;
            out.ignored = ignored;
            out.ignored.extend(o.export_only());
            o
        }
        None => TtAutohintOptions::default(),
    };
    let control: ControlInstructions = match settings.tt_autohint_control.as_deref() {
        Some(text) => control::parse(text).map_err(|e| p.bad(format!("the TTFAutohint control instructions cannot be read: {e}")))?,
        None => ControlInstructions::default(),
    };
    let column = font.master(master).ok().map(|m| delta_column_key(&m.location));
    let per_glyph = control.by_glyph();
    let no_control = ControlInstructions::default();
    for name in glyphs {
        let glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        if let Err(e) = crate::check_unlocked(glyph) {
            if explicit {
                return Err(e);
            }
            out.skipped.push(json!({"glyph": name, "reason": "the glyph is locked"}));
            continue;
        }
        let Some(layer) = glyph.layer(master) else {
            out.skipped.push(json!({"glyph": name, "reason": "the glyph has no layer for this master"}));
            continue;
        };
        let zones = settings.tt_zones_for(font, name);
        let stems = settings.tt_stems_for(font, name);
        // Exported point numbers, needed only when control instructions name the glyph.
        let control = per_glyph.get(name.as_str()).unwrap_or(&no_control);
        let map = if !control.lines.is_empty() {
            point_indexes(font, &Target::Master(master.to_string()), name, &Options::default()).ok().flatten()
        } else {
            None
        };
        let input = AutoGlyph {
            glyph: name,
            layer,
            settings,
            zones: Some(&zones),
            stems: Some(&stems),
            options: &options,
            control,
            points: map.as_ref().map(|m| m as &dyn PointMap),
            delta_column: column.as_deref(),
        };
        match autohint_glyph(&input) {
            Ok(mut h) => {
                out.warnings.extend(h.warnings);
                h.instructions.extend(fontcraft_hinting::shifts_as_deltas(&h.shifts, column.as_deref(), &mut out.warnings));
                // Checked on a copy, so a layer that cannot take them is skipped, not the run refused.
                if let Err(e) = set_tt_instructions(&mut layer.clone(), &h.instructions) {
                    out.skipped.push(json!({"glyph": name, "reason": e.to_string()}));
                    continue;
                }
                out.hinted.push(Hinted { glyph: name.clone(), instructions: h.instructions });
            }
            Err(e) => out.skipped.push(json!({"glyph": name, "reason": e.to_string()})),
        }
    }
    Ok(out)
}

/// Stores what `outcome` hinted on the `master` layers as one undo step and reports it.
fn store(s: &mut Session, label: &str, master: &str, outcome: Outcome) -> Result<Value> {
    let Outcome { hinted, skipped, ignored, warnings } = outcome;
    s.edit(label, |font| {
        for h in &hinted {
            let mut glyph = font.glyph_mut(&h.glyph)?;
            let layer =
                glyph.layer_mut(master).ok_or_else(|| crate::EngineError::Internal(format!("glyph `{}` has no layer `{master}`", h.glyph)))?;
            set_tt_instructions(layer, &h.instructions).map_err(|e| crate::EngineError::Internal(format!("glyph `{}`: {e}", h.glyph)))?;
        }
        Ok(())
    })?;
    let list: Vec<Value> = hinted.iter().map(|h| json!({"glyph": h.glyph, "instructions": h.instructions.len()})).collect();
    Ok(json!({"hinted": list, "skipped": skipped, "ignoredOptions": ignored, "warnings": warnings}))
}
