//! Axis commands (M9.15): list, add, set, remove and reorder axes, the predefined axes, and the
//! axis locations of masters and instances.
//!
//! Rows: `font.axes.list`, `font.axes.add`, `font.axes.set`, `font.axes.remove`,
//! `font.axes.reorder`, `font.axes.listPredefined`, `font.master.setAxisLocation`,
//! `font.instance.setAxisLocation`, `fontInfo.axes.predefinedNameSetsTag`,
//! `fontInfo.axes.hidden`.
//!
//! Adding, removing or reordering an axis rewrites every location in the font (masters,
//! instances, intermediate coordinates, alternate ranges, the axis parameters) in the same undo
//! step.

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
