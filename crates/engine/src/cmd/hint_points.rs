//! Exported TrueType point numbers of a glyph (the compiler's point map,
//! `fontcraft_compile::points`).
//!
//! Rows: `hint.tt.getPointIndexes`, and the numbers `hint.tt.showPointIndexes` draws.

use fontcraft_compile::Options;
use fontcraft_hinting::settings::main_master;
use fontcraft_model::Target;
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "hint.tt.getPointIndexes",
        "Get Point Indexes",
        [],
        None,
        "{glyph: name, instance?: name (a static instance; default: the main master), removeOverlaps?: bool (default true)} the point number each source node of the glyph gets in the exported TrueType outline, as TrueType instructions and autohinter control instructions count them → {glyph, master?: id, instance?: name, composite: bool, count, paths: [[index | null]]}; `paths` follows the layer's paths and nodes in the model's order, null for a node with no exported point; phantom points are count (left side bearing) and count + 1 (right); a composite has no points of its own",
        has_doc,
        get_point_indexes
    )]
}

fn get_point_indexes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.getPointIndexes", v)?;
    let font = s.font()?;
    let name = p.str("glyph")?;
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("the font has no glyph `{name}`")));
    }
    let (target, label) = match p.opt_str("instance")? {
        Some(inst) => {
            let found = font.instances.iter().enumerate().find(|(_, i)| i.name == inst);
            let Some((i, instance)) = found else { return Err(p.bad(format!("the font has no instance named `{inst}`"))) };
            if instance.kind.is_some() {
                return Err(p.bad(format!("`{inst}` is a variable font setting, not a static instance")));
            }
            (Target::Instance(i), json!({"instance": inst}))
        }
        None => {
            let main = main_master(font);
            (Target::Master(main.clone()), json!({"master": main}))
        }
    };
    let options = Options { remove_overlap: p.bool_or("removeOverlaps", true)?, ..Options::default() };
    let map = fontcraft_compile::points::point_indexes(font, &target, name, &options)?;
    let mut out = json!({"glyph": name, "composite": map.is_none()});
    if let (Some(o), Value::Object(l)) = (out.as_object_mut(), label) {
        o.extend(l);
    }
    let map = map.unwrap_or_default();
    if let Some(o) = out.as_object_mut() {
        o.insert("count".into(), json!(map.count));
        o.insert("paths".into(), json!(map.paths));
    }
    Ok(out)
}
