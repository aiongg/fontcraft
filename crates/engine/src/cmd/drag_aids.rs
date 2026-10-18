//! Servant nodes and the Power Nudge switch (`path.setServantNode`, `path.togglePowerNudge`).
//!
//! A servant link is stored on the node, in its user data under `fc_keys::SERVANT`, as
//! `{axis = x | y | both;}`, so `.glyphs` and UFO files keep it. Power Nudge is the user setting
//! `editing.powerNudge`, kept across sessions. The drags that use both are in the tools crate
//! (`fontcraft_tools::drag_aids`).

use fontcraft_model::fc_keys::SERVANT;
use fontcraft_model::plist::{Dict, Str, Value as PValue};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind as K, Targets};

/// The user setting that holds the Power Nudge switch.
pub const POWER_NUDGE_SETTING: &str = "editing.powerNudge";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.togglePowerNudge",
            "Power Nudge",
            ["Path"],
            None,
            "{on?: bool} switches Power Nudge (default: the other state): while it is on, moving nodes in Edit View also moves the unselected smooth nodes next to them, up to a corner node or an extreme, in proportion. Kept in the user setting editing.powerNudge → {on}",
            always,
            toggle_power_nudge
        ),
        cmd!(
            "path.setServantNode",
            "Set Servant Node",
            ["Path", "Nodes"],
            None,
            "{glyph: name, layer?: id, side?, points: [[shape, node]], axis: \"x\" | \"y\" | \"both\" | \"none\"} declares the on-curve nodes among the points servants on that axis: when a neighbouring node is moved in Edit View they follow in proportion on that axis, with Power Nudge on or off. none clears the declaration → {selection, servants: number of nodes changed}",
            has_doc,
            set_servant_node
        )
        .targets(&[K::Layer, K::Points])
        .when("editView"),
    ]
}

fn toggle_power_nudge(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.togglePowerNudge", v)?;
    let current = s.settings().get_bool(POWER_NUDGE_SETTING).unwrap_or(false);
    let on = p.bool_or("on", !current)?;
    s.settings_mut().set(POWER_NUDGE_SETTING, json!(on)).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({ "on": on }))
}

fn set_servant_node(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.setServantNode", v)?;
    let t = Targets::parse(&p)?;
    let axis = match p.str("axis")? {
        a @ ("x" | "y" | "both") => Some(a),
        "none" => None,
        other => return Err(p.bad(format!("`axis` must be x, y, both or none, got \"{other}\""))),
    };
    if !t.segments.is_empty() || !t.shapes.is_empty() || !t.anchors.is_empty() || !t.guides.is_empty() || !t.annotations.is_empty() {
        return Err(p.bad("only `points` can be servants"));
    }
    if t.points.is_empty() {
        return Err(p.bad("give `points`: the nodes to declare"));
    }
    let changed = s.edit("Set Servant Node", |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut changed = 0;
            for &(shape, index) in &t.points {
                let Some(node) = parts.path_mut(shape).and_then(|path| path.nodes.get_mut(index)) else {
                    return Err(p.bad(format!("there is no point [{shape}, {index}]")));
                };
                if !node.on_curve() {
                    return Err(p.bad(format!("[{shape}, {index}] is a handle; only on-curve nodes can be servants")));
                }
                let data = &mut node.info.user_data.0;
                let at = data.entries.iter().position(|(k, _)| k.as_str() == SERVANT);
                match (axis, at) {
                    (Some(a), _) => {
                        let mut link = Dict::default();
                        link.insert("axis", PValue::String(Str::from(a)));
                        let link = PValue::Dict(link);
                        if data.get(SERVANT) != Some(&link) {
                            data.insert(SERVANT, link);
                            changed += 1;
                        }
                    }
                    (None, Some(i)) => {
                        data.remove_at(i);
                        changed += 1;
                    }
                    (None, None) => {}
                }
                node.info.tidy();
            }
            Ok(changed)
        })
    })?;
    Ok(json!({ "selection": t.to_json(), "servants": changed }))
}
