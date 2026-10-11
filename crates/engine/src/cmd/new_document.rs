//! A new document from the built-in glyph sets: the headless side of the Start Window's script
//! chooser (`docs/spec/03a-edit-view-drawing.md` § 2).
//!
//! The picks are a list of scripts, each with the ids of its chosen glyph sets (none named: the
//! script's default sets). They are kept in the user settings under
//! [`Reserved::ChooserPicks`], in the same shape, so the chooser can show them again and a call
//! without picks repeats the last one.

use std::collections::HashSet;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::glyphdata::sets::{self, GlyphSet, SetGlyph};
use fontcraft_model::{Font, Glyph, GlyphTable, MAX_GLYPHS, insert_by_sort_key};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;
use crate::settings::Reserved;

/// Most scripts one call may pick.
const MAX_SCRIPTS: usize = 256;
/// Most glyph sets one script may name.
const MAX_SETS_PER_SCRIPT: usize = 1024;
/// Most skipped glyphs listed in the result.
const MAX_LISTED: usize = 100;

/// One script of the chooser and the glyph sets picked under it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pick {
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph_sets: Option<Vec<String>>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.newFromGlyphSets",
            "New from Glyph Sets…",
            ["File"],
            None,
            "{scripts?: [{script, glyphSets?: [ids]}], familyName?: string} a new untitled font with one Regular master, its .notdef, and an empty glyph for every glyph of the chosen glyph sets, named and encoded through the glyph data and placed in the default sort order. Scripts are glyph data script ids (latin, cyrillic, greek, arabic); glyph set ids look like latin-core. A script without glyphSets takes its default sets; a call without scripts repeats the last picks, which are kept in the user settings. An unknown script or set is refused with the valid ids → {title, glyphs, sets: [ids], skipped: [{glyph, reason}]}",
            always,
            new_from_glyph_sets
        ),
        cmd!(
            query "glyphData.glyphSets",
            "List Glyph Sets",
            [],
            None,
            "{script?: string} the built-in glyph sets file.newFromGlyphSets takes, by script in the built-in order: each script's id, its readable name, and its sets with their id, name, whether a script picked without naming sets takes it (default) and their number of glyphs. With script, only that script; an unknown one is refused with the valid ids → {scripts: [{script, name, sets: [{id, name, default, glyphs}]}]}",
            always,
            glyph_sets
        ),
    ]
}

/// The Start Window's chooser lists what this returns.
fn glyph_sets(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyphData.glyphSets", v)?;
    let builtin = sets::builtin().map_err(|e| EngineError::Internal(e.to_string()))?;
    let scripts = builtin.glyph_set_scripts();
    let wanted = p.opt_str("script")?;
    if let Some(w) = wanted
        && !scripts.contains(&w)
    {
        return Err(p.bad(format!("`script`: no glyph sets for `{}`; scripts with glyph sets: {}", clip(w), scripts.join(", "))));
    }
    let data = s.glyph_data_without_document()?;
    let out: Vec<Value> = scripts
        .into_iter()
        .filter(|script| wanted.is_none_or(|w| w == *script))
        .map(|script| {
            let sets: Vec<Value> = builtin
                .glyph_sets
                .iter()
                .filter(|g| g.script == script)
                .map(|g| json!({ "id": g.id, "name": g.name, "default": g.default, "glyphs": g.glyphs.len() }))
                .collect();
            json!({ "script": script, "name": data.script_display_name(script), "sets": sets })
        })
        .collect();
    Ok(json!({ "scripts": out }))
}

/// At most 64 characters of untrusted text, for a message.
fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(64).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn parse_picks(p: &Params, v: &Value) -> Result<Vec<Pick>> {
    if v.as_array().is_some_and(|a| a.len() > MAX_SCRIPTS) {
        return Err(p.bad(format!("`scripts`: at most {MAX_SCRIPTS} scripts can be picked")));
    }
    let picks: Vec<Pick> =
        serde_json::from_value(v.clone()).map_err(|e| p.bad(format!("`scripts` is a list of {{script, glyphSets?: [ids]}}: {e}")))?;
    if let Some(pick) = picks.iter().find(|pick| pick.glyph_sets.as_ref().is_some_and(|g| g.len() > MAX_SETS_PER_SCRIPT)) {
        return Err(p.bad(format!("`glyphSets` of `{}`: at most {MAX_SETS_PER_SCRIPT} sets", clip(&pick.script))));
    }
    Ok(picks)
}

/// The glyph sets the picks name, each once, in pick order.
fn chosen<'d>(p: &Params, data: &'d sets::Sets, picks: &[Pick]) -> Result<Vec<&'d GlyphSet>> {
    let mut out: Vec<&GlyphSet> = Vec::new();
    for pick in picks {
        let of_script: Vec<&GlyphSet> = data.glyph_sets.iter().filter(|g| g.script == pick.script).collect();
        if of_script.is_empty() {
            return Err(p.bad(format!(
                "`scripts`: there are no glyph sets for `{}`; the scripts with glyph sets are {}",
                clip(&pick.script),
                data.glyph_set_scripts().join(", ")
            )));
        }
        let wanted: Vec<&GlyphSet> = match &pick.glyph_sets {
            Some(ids) => ids
                .iter()
                .map(|id| {
                    of_script.iter().copied().find(|g| &g.id == id).ok_or_else(|| {
                        let valid: Vec<&str> = of_script.iter().map(|g| g.id.as_str()).collect();
                        p.bad(format!("`glyphSets`: `{}` is not a glyph set of {}; its sets are {}", clip(id), pick.script, valid.join(", ")))
                    })
                })
                .collect::<Result<_>>()?,
            None => {
                let defaults: Vec<&GlyphSet> = of_script.iter().copied().filter(|g| g.default).collect();
                if defaults.is_empty() { of_script.into_iter().take(1).collect() } else { defaults }
            }
        };
        for g in wanted {
            if !out.iter().any(|o| o.id == g.id) {
                out.push(g);
            }
        }
    }
    if out.is_empty() {
        return Err(p.bad("`scripts`: no glyph set is picked"));
    }
    Ok(out)
}

/// The name and code points a set's glyph gets: an encoded glyph takes the glyph data's name for
/// its code point, an unencoded one its nice name; either keeps the set's name when the glyph data
/// does not know it.
fn named(data: &GlyphData, g: &SetGlyph) -> (String, Vec<u32>) {
    match g.code_point {
        Some(cp) => (data.by_code_point(cp).map(|r| r.name).unwrap_or_else(|| g.name.clone()), vec![cp]),
        None => (data.canonical_name(&g.name).unwrap_or_else(|| g.name.clone()), Vec::new()),
    }
}

fn new_from_glyph_sets(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.newFromGlyphSets", v)?;
    let family = p.opt_str("familyName")?.unwrap_or("Untitled");
    if family.trim().is_empty() {
        return Err(p.bad("`familyName` must not be empty"));
    }
    let picks = match p.get("scripts") {
        Some(v) => parse_picks(&p, v)?,
        None => match s.settings().reserved::<Value>(Reserved::ChooserPicks)? {
            Some(stored) => parse_picks(&p, &stored)?,
            None => return Err(p.bad("`scripts` is required: no glyph sets were picked before")),
        },
    };
    let builtin = sets::builtin().map_err(|e| EngineError::Internal(e.to_string()))?;
    let chosen = chosen(&p, builtin, &picks)?;
    // A new document reads the built-in data and the user's files: it has no source folder yet.
    let data = s.glyph_data_without_document()?;

    let mut font = Font::new(family);
    super::metric_defs::script_defaults(&mut font, picks.iter().map(|pick| pick.script.as_str()));
    // New's starting glyph set gives way to the sets', but its drawn .notdef stays, first.
    let notdef = font.glyph(".notdef").cloned();
    font.glyphs.retain(|_| false);
    let mut names: HashSet<String> = notdef.iter().map(|g| g.name.clone()).collect();
    let mut codes: HashSet<u32> = HashSet::new();
    let mut glyphs: Vec<Glyph> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut skip = |glyph: &str, reason: String| {
        if skipped.len() < MAX_LISTED {
            skipped.push(json!({"glyph": glyph, "reason": reason}));
        }
    };
    for set in &chosen {
        for sg in &set.glyphs {
            let (name, unicodes) = named(&data, sg);
            if names.contains(&name) {
                continue;
            }
            if let Some(cp) = unicodes.iter().find(|cp| codes.contains(cp)) {
                skip(&name, format!("U+{cp:04X} already belongs to another glyph"));
                continue;
            }
            let mut g = Glyph::new(name.as_str());
            g.unicodes = unicodes;
            g.layers = super::widths::new_glyph_layers(&font, Some(&data), &name, &g.unicodes);
            if let Err(e) = font.check_glyph(&g) {
                skip(&name, e.to_string());
                continue;
            }
            if glyphs.len() >= MAX_GLYPHS {
                return Err(p.bad(format!("the chosen glyph sets hold more than {MAX_GLYPHS} glyphs")));
            }
            codes.extend(g.unicodes.iter().copied());
            names.insert(name);
            glyphs.push(g);
        }
    }
    insert_by_sort_key(&mut font, &data, glyphs)?;
    if let Some(notdef) = notdef {
        let mut all = vec![notdef];
        all.extend(font.glyphs.as_arcs().iter().cloned());
        font.glyphs = GlyphTable::from_glyphs(all);
    }
    let count = font.glyphs.len();
    s.add_document(font, None)?;
    s.settings_mut().set_reserved(Reserved::ChooserPicks, &picks)?;
    let title = s.doc()?.title.clone();
    let ids: Vec<&str> = chosen.iter().map(|g| g.id.as_str()).collect();
    Ok(json!({ "title": title, "glyphs": count, "sets": ids, "skipped": skipped }))
}
