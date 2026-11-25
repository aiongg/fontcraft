//! The Layers palette's commands: listing a glyph's layers, their visibility, and adding,
//! deleting, renaming, reordering and restoring backup layers (`docs/spec/04-palette.md`
//! § Layers). Master layers follow the masters: they are never added, deleted, renamed or moved
//! here.

use std::collections::HashSet;

use fontcraft_model::{Font, Glyph, Layer, LayerAttrs};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind};

/// Most layers one glyph may have; adding a backup beyond it is refused.
pub(super) const MAX_LAYERS: usize = 4096;
/// Most layer ids one `layers` list may hold.
const MAX_LAYER_IDS: usize = 1024;
/// Most bytes in a layer name.
const MAX_LAYER_NAME: usize = 255;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "layers.list",
            "List Layers",
            [],
            None,
            "{glyph: name} the glyph's layers in palette order: by master in master order, each master layer first, then that master's other layers → {glyph, layers: [{id, name, kind: \"master\" | \"backup\" | \"intermediate\" | \"alternate\" | \"color\" | \"other\", master, visible}]} (a master layer's name is its master's; another layer without a name has null)",
            has_doc,
            list
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "layers.toggleVisibility",
            "Show / Hide Layer",
            [],
            None,
            "{glyphs: [names], layer: id, visible?: bool (default: the opposite of the first glyph's)} set whether the layer is drawn as an outline while another layer of the glyph is edited; saved with the document. Every glyph must have the layer → {glyphs: [names], layer, visible}",
            has_doc,
            toggle_visibility
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "layers.addBackupLayer",
            "New Backup Layer",
            [],
            None,
            "{glyphs: [names], master?: id (default: the first master), from?: layer id (default: the master's layer), name?: string (default: the date and time, UTC, as 2026-10-10 14:05)} add a backup layer to each glyph: a copy of `from`, placed after that master's other layers. The new layer has the same id in every glyph → {glyphs: [names], layer: id, name, selection: {glyph, layer} (one glyph)}",
            has_doc,
            add_backup_layer
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "layers.delete",
            "Delete Layer",
            [],
            None,
            "{glyphs: [names], layers: [ids]} remove the layers from each glyph that has them; every id must name a layer of at least one glyph. Master layers are refused (they go with their master) → {glyphs: [names], deleted: count}",
            has_doc,
            delete
        )
        .targets(&[TargetKind::Glyphs])
        .context(&["layers.row"]),
        cmd!(
            "layers.rename",
            "Rename Layer",
            [],
            None,
            "{glyph: name, layer: id, name: string (1 to 255 bytes, no control characters)} rename a layer that is not a master layer → {glyph, layer, name}",
            has_doc,
            rename
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "layers.reorder",
            "Reorder Layer",
            [],
            None,
            "{glyph: name, layer: id, toIndex: number} move a layer that is not a master layer to position `toIndex` among its master's other layers (0 is right under the master layer) → {glyph, layer, index}",
            has_doc,
            reorder
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "layers.useAsMaster",
            "Use as Master",
            [],
            None,
            "{glyphs: [names], layer: id, name?: string (for the new backup; default: the date and time, UTC)} restore a backup layer in each glyph: the master layer's content goes to a new backup layer in the backup's place, the backup's content becomes the master layer's, and the backup is removed. Only backup layers (no special attributes) are taken → {glyphs: [names], backup: id (the new backup layer, same in every glyph that had master content)}",
            has_doc,
            use_as_master
        )
        .targets(&[TargetKind::Glyphs])
        .context(&["layers.row"]),
    ]
}

// ── Helpers ─────────────────────────────────────────────────────────────────────────────────────

/// The row kind of a layer in the palette.
fn kind(l: &Layer) -> &'static str {
    let a = &l.info.attrs;
    if l.is_master_layer() {
        "master"
    } else if !a.coordinates.is_empty() {
        "intermediate"
    } else if !a.axis_rules.is_empty() {
        "alternate"
    } else if a.color_palette.is_some() || a.full_color || a.sbix_size.is_some() || a.svg {
        "color"
    } else if a.is_empty() {
        "backup"
    } else {
        "other"
    }
}

fn is_backup(l: &Layer) -> bool {
    !l.is_master_layer() && l.info.attrs.is_empty()
}

/// The glyph's layers in palette order.
fn palette_order<'g>(font: &Font, g: &'g Glyph) -> Vec<&'g Layer> {
    let mut out = Vec::with_capacity(g.layers.len());
    for m in font.masters.iter() {
        out.extend(g.layers.iter().filter(|l| l.master == m.id && l.is_master_layer()));
        out.extend(g.layers.iter().filter(|l| l.master == m.id && !l.is_master_layer()));
    }
    out
}

/// `glyphs`, all existing, not empty.
fn glyph_list(p: &Params, font: &Font) -> Result<Vec<String>> {
    let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if glyphs.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    Ok(glyphs)
}

/// A layer name: not blank, at most [`MAX_LAYER_NAME`] bytes, no control characters.
fn layer_name(p: &Params, key: &str) -> Result<Option<String>> {
    let Some(name) = p.opt_str(key)? else { return Ok(None) };
    if name.trim().is_empty() {
        return Err(p.bad(format!("`{key}` is empty")));
    }
    if name.len() > MAX_LAYER_NAME {
        return Err(p.bad(format!("`{key}` is longer than {MAX_LAYER_NAME} bytes")));
    }
    if name.chars().any(char::is_control) {
        return Err(p.bad(format!("`{key}` contains a control character")));
    }
    Ok(Some(name.to_string()))
}

/// The default name of a new backup layer: the date and time, UTC.
fn now_name() -> String {
    date_time(super::shared::clock())
}

/// `secs` after the Unix epoch as `YYYY-MM-DD HH:MM`, UTC.
fn date_time(secs: i64) -> String {
    // Years 1970 to 9999 keep every step below in range.
    let secs = secs.clamp(0, 253_402_300_799);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (the proleptic Gregorian calendar), after Howard Hinnant's
    // public-domain `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3_600, rest % 3_600 / 60)
}

/// A new layer id, in the 8-4-4-4-12 hex form layer ids usually take, that `taken` refuses for
/// none of the glyphs. It depends only on `seed`, so replaying a journal gives the same id.
pub(super) fn new_layer_id(seed: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    fn fnv(basis: u64, text: &str) -> u64 {
        text.bytes().fold(basis, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3))
    }
    (0u32..10_000).find_map(|n| {
        let text = format!("{seed}#{n}");
        let (a, b) = (fnv(0xCBF2_9CE4_8422_2325, &text), fnv(0x6C62_272E_07BB_0142, &text));
        let id = format!("{:08X}-{:04X}-{:04X}-{:04X}-{:012X}", a >> 32, (a >> 16) & 0xFFFF, a & 0xFFFF, b >> 48, b & 0xFFFF_FFFF_FFFF);
        (!taken(&id)).then_some(id)
    })
}

/// An id that is new in every one of `glyphs`.
fn fresh_id(p: &Params, font: &Font, glyphs: &[String], seed: &str) -> Result<String> {
    let targeted: Vec<&Glyph> = glyphs.iter().filter_map(|n| font.glyph(n).map(|g| &**g)).collect();
    let count: usize = targeted.iter().map(|g| g.layers.len()).sum();
    new_layer_id(&format!("{}|{seed}|{count}", glyphs.join("/")), |id| targeted.iter().any(|g| g.layers.iter().any(|l| l.id == id)))
        .ok_or_else(|| p.bad("could not find a free layer id"))
}

fn no_layer(p: &Params, glyph: &str, id: &str) -> EngineError {
    p.bad(format!("glyph `{glyph}` has no layer `{id}`"))
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.list", p)?;
    let name = p.str("glyph")?;
    let font = s.font()?;
    let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
    let rows: Vec<Value> = palette_order(font, g)
        .into_iter()
        .map(|l| {
            let name = match font.master(&l.master) {
                Ok(m) if l.is_master_layer() => Some(m.name.clone()),
                _ => l.info.name.clone(),
            };
            json!({"id": l.id, "name": name, "kind": kind(l), "master": l.master, "visible": l.info.visible})
        })
        .collect();
    Ok(json!({"glyph": g.name, "layers": rows}))
}

fn toggle_visibility(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.toggleVisibility", p)?;
    let font = s.font()?;
    let glyphs = glyph_list(&p, font)?;
    let id = p.str("layer")?.to_string();
    let current = |name: &str| font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == id)).map(|l| l.info.visible);
    let first = glyphs.first().map(String::as_str).unwrap_or_default();
    let visible = match p.get("visible") {
        Some(_) => p.bool_or("visible", true)?,
        None => !current(first).ok_or_else(|| no_layer(&p, first, &id))?,
    };
    s.edit("Show / Hide Layer", |font| {
        for name in &glyphs {
            let mut g = font.glyph_mut(name)?;
            crate::check_unlocked(&g)?;
            let layer = g.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| no_layer(&p, name, &id))?;
            layer.info.visible = visible;
        }
        Ok(())
    })?;
    Ok(json!({"glyphs": glyphs, "layer": id, "visible": visible}))
}

fn add_backup_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.addBackupLayer", p)?;
    let font = s.font()?;
    let glyphs = glyph_list(&p, font)?;
    let master = match p.opt_str("master")? {
        Some(id) => font.master(id).map_err(|_| p.bad(format!("there is no master `{id}`")))?.id.clone(),
        None => font.default_master()?.id.clone(),
    };
    let from = p.opt_str("from")?.map(str::to_string);
    let name = layer_name(&p, "name")?.unwrap_or_else(now_name);
    let id = fresh_id(&p, font, &glyphs, from.as_deref().unwrap_or(&master))?;
    s.edit("New Backup Layer", |font| {
        for glyph in &glyphs {
            let mut g = font.glyph_mut(glyph)?;
            crate::check_unlocked(&g)?;
            if g.layers.len() >= MAX_LAYERS {
                return Err(p.bad(format!("glyph `{glyph}` has {} layers; at most {MAX_LAYERS} are allowed", g.layers.len())));
            }
            let source = match &from {
                Some(from) => g.layers.iter().find(|l| l.id == *from).ok_or_else(|| no_layer(&p, glyph, from))?,
                None => g.layer(&master).ok_or_else(|| no_layer(&p, glyph, &master))?,
            };
            if source.master != master && p.get("master").is_some() {
                return Err(p.bad(format!("layer `{}` of glyph `{glyph}` belongs to master `{}`, not `{master}`", source.id, source.master)));
            }
            let mut layer = source.clone();
            layer.id = id.clone();
            layer.info.name = Some(name.clone());
            layer.info.attrs = LayerAttrs::default();
            layer.info.visible = false;
            let at = g.layers.iter().rposition(|l| l.master == layer.master).map_or(g.layers.len(), |i| i + 1);
            g.layers.insert(at, layer);
        }
        Ok(())
    })?;
    let mut v = json!({"glyphs": glyphs, "layer": id, "name": name});
    if let ([glyph], Some(o)) = (glyphs.as_slice(), v.as_object_mut()) {
        o.insert("selection".into(), json!({"glyph": glyph, "layer": id}));
    }
    Ok(v)
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.delete", p)?;
    let font = s.font()?;
    let glyphs = glyph_list(&p, font)?;
    if let Some(Value::Array(a)) = p.get("layers")
        && a.len() > MAX_LAYER_IDS
    {
        return Err(p.bad(format!("`layers` holds {} ids; at most {MAX_LAYER_IDS} are allowed", a.len())));
    }
    let ids: HashSet<String> = p.strings("layers")?.into_iter().map(str::to_string).collect();
    if ids.is_empty() {
        return Err(p.bad("`layers` is empty"));
    }
    let mut found = HashSet::new();
    for name in &glyphs {
        let Some(g) = font.glyph(name) else { continue };
        for l in g.layers.iter().filter(|l| ids.contains(&l.id)) {
            if l.is_master_layer() {
                return Err(p.bad(format!("layer `{}` of glyph `{name}` is a master layer: it is removed with its master", l.id)));
            }
            found.insert(l.id.clone());
        }
    }
    if let Some(id) = ids.iter().find(|id| !found.contains(*id)) {
        return Err(p.bad(format!("no glyph given has a layer `{id}`")));
    }
    let deleted = s.edit("Delete Layer", |font| {
        let mut deleted = 0usize;
        for name in &glyphs {
            let mut g = font.glyph_mut(name)?;
            let before = g.layers.len();
            if !g.layers.iter().any(|l| ids.contains(&l.id)) {
                continue;
            }
            crate::check_unlocked(&g)?;
            g.layers.retain(|l| !ids.contains(&l.id));
            deleted = deleted.saturating_add(before.saturating_sub(g.layers.len()));
        }
        Ok(deleted)
    })?;
    Ok(json!({"glyphs": glyphs, "deleted": deleted}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.rename", p)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    let name = layer_name(&p, "name")?.ok_or_else(|| p.bad("`name` is required"))?;
    s.edit("Rename Layer", |font| {
        let mut g = font.glyph_mut(&glyph).map_err(|_| p.bad(format!("there is no glyph `{glyph}`")))?;
        crate::check_unlocked(&g)?;
        let layer = g.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| no_layer(&p, &glyph, &id))?;
        if layer.is_master_layer() {
            return Err(p.bad("a master layer carries its master's name: rename the master in Font Info"));
        }
        layer.info.name = Some(name.clone());
        Ok(())
    })?;
    Ok(json!({"glyph": glyph, "layer": id, "name": name}))
}

fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.reorder", p)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    let to = p.opt_int("toIndex", 0, MAX_LAYERS as i64)?.ok_or_else(|| p.bad("`toIndex` is required"))?;
    let to = usize::try_from(to).map_err(|_| p.bad("`toIndex` is out of range"))?;
    s.edit("Reorder Layer", |font| {
        let mut g = font.glyph_mut(&glyph).map_err(|_| p.bad(format!("there is no glyph `{glyph}`")))?;
        crate::check_unlocked(&g)?;
        let from = g.layers.iter().position(|l| l.id == id).ok_or_else(|| no_layer(&p, &glyph, &id))?;
        let master = match g.layers.get(from) {
            Some(l) if l.is_master_layer() => return Err(p.bad("master layers stay at the top of their master's rows")),
            Some(l) => l.master.clone(),
            None => return Err(no_layer(&p, &glyph, &id)),
        };
        let siblings = g.layers.iter().filter(|l| l.master == master && !l.is_master_layer()).count();
        if to >= siblings {
            return Err(p.bad(format!("`toIndex` must be below {siblings}, the number of layers under master `{master}`")));
        }
        let layer = g.layers.remove(from);
        let group: Vec<usize> = g.layers.iter().enumerate().filter(|(_, l)| l.master == master && !l.is_master_layer()).map(|(i, _)| i).collect();
        let at = match (group.get(to), group.last()) {
            (Some(&i), _) => i,
            (None, Some(&last)) => last + 1,
            (None, None) => from.min(g.layers.len()),
        };
        let len = g.layers.len();
        g.layers.insert(at.min(len), layer);
        Ok(())
    })?;
    Ok(json!({"glyph": glyph, "layer": id, "index": to}))
}

fn use_as_master(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layers.useAsMaster", p)?;
    let font = s.font()?;
    let glyphs = glyph_list(&p, font)?;
    let id = p.str("layer")?.to_string();
    let name = layer_name(&p, "name")?.unwrap_or_else(now_name);
    let backup = fresh_id(&p, font, &glyphs, &id)?;
    s.edit("Use as Master", |font| {
        for glyph in &glyphs {
            let mut g = font.glyph_mut(glyph)?;
            crate::check_unlocked(&g)?;
            let li = g.layers.iter().position(|l| l.id == id).ok_or_else(|| no_layer(&p, glyph, &id))?;
            let chosen = g.layers.get(li).cloned().ok_or_else(|| no_layer(&p, glyph, &id))?;
            if !is_backup(&chosen) {
                return Err(p.bad(format!("layer `{id}` of glyph `{glyph}` is a {} layer: only a backup layer can be used as master", kind(&chosen))));
            }
            let master = chosen.master.clone();
            let mi = g.layers.iter().position(|l| l.id == master && l.is_master_layer());
            let mut restored = chosen;
            restored.id = master.clone();
            match mi.and_then(|mi| g.layers.get(mi).cloned()) {
                Some(old) => {
                    restored.info.name = old.info.name.clone();
                    restored.info.visible = old.info.visible;
                    let mut kept = old;
                    kept.id = backup.clone();
                    kept.info.name = Some(name.clone());
                    kept.info.attrs = LayerAttrs::default();
                    kept.info.visible = false;
                    if let Some(mi) = mi
                        && let Some(slot) = g.layers.get_mut(mi)
                    {
                        *slot = restored;
                    }
                    if let Some(slot) = g.layers.get_mut(li) {
                        *slot = kept;
                    }
                }
                None => {
                    // The glyph has no master layer for this master: the backup becomes it.
                    restored.info.name = None;
                    if let Some(slot) = g.layers.get_mut(li) {
                        *slot = restored;
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({"glyphs": glyphs, "backup": backup}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(date_time(0), "1970-01-01 00:00");
        assert_eq!(date_time(951_782_400), "2000-02-29 00:00");
        assert_eq!(date_time(1_791_633_900), "2026-10-10 12:05");
        assert_eq!(date_time(-5), "1970-01-01 00:00");
        assert_eq!(date_time(i64::MAX), "9999-12-31 23:59");
    }

    #[test]
    fn layer_ids_avoid_taken_ones() {
        let first = new_layer_id("seed", |_| false).unwrap();
        assert_eq!(first.len(), 36);
        assert_eq!(new_layer_id("seed", |_| false).unwrap(), first);
        let second = new_layer_id("seed", |id| id == first).unwrap();
        assert_ne!(second, first);
        assert_eq!(new_layer_id("seed", |_| true), None);
    }
}
