//! Imported TrueType hinting: removing what an opened hinted font brought.
//!
//! Rows: `hint.tt.removeImportedHinting`.
//!
//! Opening a hinted TrueType font keeps its hinting (`fontcraft_decompile::tt_hints`): the cvt
//! Table, prep Table Assembly and fpgm Table Assembly parameters on every master, each glyph's
//! program in its layer's user data, and the gasp Table parameter. This command takes them out
//! in one undo step, so the font can be instructed afresh or exported unhinted. The parameters
//! are removed wherever they are set (font, masters, instances), since export reads them at each
//! of those scopes. Nothing to remove changes nothing and makes no undo step.

use std::sync::Arc;

use fontcraft_hinting::imported::remove_imported_program;
use fontcraft_hinting::settings::{
    CVT_TABLE, FPGM_TABLE_ASSEMBLY, GASP_TABLE, IMPORTED_GLYPH_PROGRAM_KEY, IMPORTED_OUTLINE_KEY, PREP_TABLE_ASSEMBLY,
};
use fontcraft_model::{Font, Params as ParamList};
use serde_json::{Value, json};

use super::{CommandSpec, cmd, has_doc};
use crate::params::Params;
use crate::{Result, Session};

/// The parameters that hold the imported font-wide programs.
const TABLE_PARAMS: [&str; 3] = [CVT_TABLE, PREP_TABLE_ASSEMBLY, FPGM_TABLE_ASSEMBLY];
/// The gasp Table parameter in both spellings export reads.
const GASP_PARAMS: [&str; 2] = [GASP_TABLE, "GASP Table"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "hint.tt.removeImportedHinting",
        "Remove Imported TrueType Hinting",
        [],
        None,
        "{glyphPrograms?: bool (default true), controlValueProgram?: bool (default true), gasp?: bool (default false)} remove the hinting an opened TrueType font brought: with glyphPrograms the imported program of every layer, with controlValueProgram the cvt Table, prep Table Assembly and fpgm Table Assembly parameters, with gasp the gasp Table parameter, each wherever it is set (font, masters, instances); one undo step, none when there is nothing to remove → {glyphs: layers whose program was removed, parameters: parameters removed}",
        has_doc,
        run
    )]
}

fn run(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.removeImportedHinting", v)?;
    let programs = p.bool_or("glyphPrograms", true)?;
    let mut names: Vec<&str> = Vec::new();
    if p.bool_or("controlValueProgram", true)? {
        names.extend(TABLE_PARAMS);
    }
    if p.bool_or("gasp", false)? {
        names.extend(GASP_PARAMS);
    }
    let font = s.font()?;
    let layers: usize = if programs { glyphs_with_programs(font).iter().map(|(_, n)| n).sum() } else { 0 };
    let params = param_lists(font).map(|list| names.iter().map(|n| list.all(n).count()).sum::<usize>()).sum::<usize>();
    if layers == 0 && params == 0 {
        return Ok(json!({"glyphs": 0, "parameters": 0}));
    }
    s.edit("Remove Imported TrueType Hinting", |font| {
        if programs {
            for (i, _) in glyphs_with_programs(font) {
                if let Some(mut g) = font.glyphs.get_mut_at(i) {
                    for layer in g.layers.iter_mut() {
                        remove_imported_program(layer);
                    }
                }
            }
        }
        if params > 0 {
            let remove = |list: &mut ParamList| names.iter().for_each(|n| _ = list.remove(n));
            remove(&mut Arc::make_mut(&mut font.info).params);
            Arc::make_mut(&mut font.masters).iter_mut().for_each(|m| remove(&mut m.params));
            Arc::make_mut(&mut font.instances).iter_mut().for_each(|i| remove(&mut i.params));
        }
        Ok(())
    })?;
    Ok(json!({"glyphs": layers, "parameters": params}))
}

/// The font's parameter lists: font, masters, instances.
fn param_lists(font: &Font) -> impl Iterator<Item = &ParamList> {
    std::iter::once(&font.info.params).chain(font.masters.iter().map(|m| &m.params)).chain(font.instances.iter().map(|i| &i.params))
}

/// The glyphs with an imported program on some layer, by index, each with the number of such
/// layers.
fn glyphs_with_programs(font: &Font) -> Vec<(usize, usize)> {
    font.glyphs
        .as_arcs()
        .iter()
        .enumerate()
        .filter_map(|(i, g)| {
            let n = g
                .layers
                .iter()
                .filter(|l| [IMPORTED_GLYPH_PROGRAM_KEY, IMPORTED_OUTLINE_KEY].iter().any(|k| l.info.user_data.get(k).is_some()))
                .count();
            (n > 0).then_some((i, n))
        })
        .collect()
}
