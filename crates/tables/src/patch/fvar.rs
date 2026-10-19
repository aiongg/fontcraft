//! `tables.cleanInstances` and `tables.dropAxis`.
//!
//! The `fvar` instance records are parsed and written byte by byte; the axis records keep their
//! bytes. Dropping an axis pins it with the instancer, which removes it from `fvar`, `avar`,
//! `STAT` and every variation region.

use super::{be_u16, be_u32, missing, put};
use crate::instance::{AxisLimit, InstanceOptions, instantiate};
use crate::limits::{MAX_AXES, MAX_NAMED_INSTANCES};
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::variations::fvar_axes;
use crate::{Result, TablesError};

const FVAR: Tag = *b"fvar";
/// `postScriptNameID` of an instance without a PostScript name.
const NO_PS_NAME: u16 = 0xFFFF;

/// The result of [`clean_instances`].
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceCleanup {
    pub font: SfntFont,
    /// Repeated records removed.
    pub removed: usize,
    /// The records changed order.
    pub reordered: bool,
    /// The PostScript name field was removed from every record because some lacked one.
    pub dropped_postscript_names: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Instance {
    subfamily: u16,
    flags: u16,
    coords: Vec<i32>,
    ps: Option<u16>,
}

fn bad() -> TablesError {
    TablesError::invalid("the fvar table is damaged")
}

/// The font has a name record with this name ID.
fn has_name(font: &SfntFont, id: u16) -> bool {
    let Some(name) = font.get(*b"name") else { return false };
    let count = usize::from(be_u16(name, 2).unwrap_or(0));
    (0..count).any(|i| be_u16(name, 6usize.saturating_add(i.saturating_mul(12)).saturating_add(6)) == Some(id))
}

/// Sort key: every axis but `wght` in axis order, then `wght`, so that within each run of
/// instances that agree on the other axes the weights rise.
fn sort_key(coords: &[i32], wght: Option<usize>) -> Vec<i32> {
    let mut key: Vec<i32> = coords.iter().enumerate().filter(|(i, _)| Some(*i) != wght).map(|(_, v)| *v).collect();
    if let Some(v) = wght.and_then(|w| coords.get(w)) {
        key.push(*v);
    }
    key
}

/// The font with its named instances sorted, repeats removed, every record in one form, and
/// the instance at the default location named with the font's own subfamily and PostScript
/// names.
pub fn clean_instances(font: &SfntFont) -> Result<InstanceCleanup> {
    let data = font.get(FVAR).ok_or_else(|| missing(FVAR))?;
    let axes_at = usize::from(be_u16(data, 4).ok_or_else(bad)?);
    let axis_count = usize::from(be_u16(data, 8).ok_or_else(bad)?);
    let axis_size = usize::from(be_u16(data, 10).ok_or_else(bad)?);
    let count = usize::from(be_u16(data, 12).ok_or_else(bad)?);
    let size = usize::from(be_u16(data, 14).ok_or_else(bad)?);
    if axis_count > MAX_AXES {
        return Err(TablesError::TooLarge { what: "the fvar axis count", limit: MAX_AXES });
    }
    if count > MAX_NAMED_INSTANCES {
        return Err(TablesError::TooLarge { what: "the named instance count", limit: MAX_NAMED_INSTANCES });
    }
    let plain = axis_count.saturating_mul(4).saturating_add(4);
    let with_ps = match size {
        s if s == plain => false,
        s if s == plain.saturating_add(2) => true,
        _ => return Err(TablesError::invalid(format!("the fvar instance size {size} fits neither record form"))),
    };
    let first = axes_at.checked_add(axis_count.saturating_mul(axis_size)).ok_or_else(bad)?;
    let mut defaults = Vec::with_capacity(axis_count);
    let mut tags = Vec::with_capacity(axis_count);
    for a in 0..axis_count {
        let at = axes_at.saturating_add(a.saturating_mul(axis_size));
        tags.push(be_u32(data, at).ok_or_else(bad)?.to_be_bytes());
        defaults.push(be_u32(data, at.saturating_add(8)).ok_or_else(bad)? as i32);
    }
    let mut instances = Vec::with_capacity(count);
    for i in 0..count {
        let at = first.saturating_add(i.saturating_mul(size));
        let mut coords = Vec::with_capacity(axis_count);
        for a in 0..axis_count {
            coords.push(be_u32(data, at.saturating_add(4).saturating_add(a * 4)).ok_or_else(bad)? as i32);
        }
        let ps = if with_ps { Some(be_u16(data, at.saturating_add(plain)).ok_or_else(bad)?) } else { None };
        instances.push(Instance {
            subfamily: be_u16(data, at).ok_or_else(bad)?,
            flags: be_u16(data, at.saturating_add(2)).ok_or_else(bad)?,
            coords,
            ps,
        });
    }
    let original = instances.clone();
    let mut kept: Vec<Instance> = Vec::with_capacity(instances.len());
    for inst in instances {
        if !kept.iter().any(|k| k.coords == inst.coords && k.subfamily == inst.subfamily && k.ps == inst.ps) {
            kept.push(inst);
        }
    }
    let removed = count.saturating_sub(kept.len());
    let mixed = with_ps && kept.iter().any(|i| i.ps == Some(NO_PS_NAME)) && kept.iter().any(|i| i.ps.is_some_and(|p| p != NO_PS_NAME));
    let keep_ps = with_ps && !mixed;
    let subfamily = if has_name(font, 17) { 17 } else { 2 };
    for inst in &mut kept {
        if !keep_ps {
            inst.ps = None;
        }
        if inst.coords == defaults {
            inst.subfamily = subfamily;
            if let Some(ps) = inst.ps.as_mut().filter(|p| **p != NO_PS_NAME) {
                *ps = 6;
            }
        }
    }
    let wght = tags.iter().position(|t| t == b"wght");
    let before: Vec<Vec<i32>> = kept.iter().map(|i| i.coords.clone()).collect();
    kept.sort_by_key(|i| sort_key(&i.coords, wght));
    let reordered = kept.iter().map(|i| &i.coords).ne(before.iter());
    if kept == original && keep_ps == with_ps {
        return Ok(InstanceCleanup { font: font.clone(), removed: 0, reordered: false, dropped_postscript_names: false });
    }
    let new_size = if keep_ps { plain.saturating_add(2) } else { plain };
    let mut out = data.get(..first).ok_or_else(bad)?.to_vec();
    put(&mut out, 12, &u16::try_from(kept.len()).map_err(|_| bad())?.to_be_bytes()).ok_or_else(bad)?;
    put(&mut out, 14, &u16::try_from(new_size).map_err(|_| bad())?.to_be_bytes()).ok_or_else(bad)?;
    for inst in &kept {
        out.extend(inst.subfamily.to_be_bytes());
        out.extend(inst.flags.to_be_bytes());
        for c in &inst.coords {
            out.extend(c.to_be_bytes());
        }
        if let Some(ps) = inst.ps {
            out.extend(ps.to_be_bytes());
        }
    }
    let mut font = font.clone();
    font.insert(FVAR, out);
    Ok(InstanceCleanup { font, removed, reordered, dropped_postscript_names: mixed })
}

/// The font without the axis `tag`, which must have no range (its minimum equals its maximum).
/// When it is the only axis, the result is a static font.
pub fn drop_axis(font: &SfntFont, tag: Tag) -> Result<SfntFont> {
    let axes = fvar_axes(font)?;
    let name = tag_str(tag).trim_end().to_string();
    let axis = axes.iter().find(|a| a.tag == tag).ok_or_else(|| TablesError::BadOptions(format!("the font has no `{name}` axis")))?;
    if axis.minimum != axis.maximum {
        return Err(TablesError::BadOptions(format!(
            "the `{name}` axis runs from {} to {}; only an axis without a range can be dropped",
            axis.minimum, axis.maximum
        )));
    }
    let options = InstanceOptions { limits: vec![(tag, AxisLimit::Pin(axis.minimum))], update_names: false, ..InstanceOptions::default() };
    instantiate(font, &options)
}
