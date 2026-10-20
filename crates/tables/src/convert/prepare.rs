//! The steps `font.convert` runs around a conversion (M12.30c), in the order AFDKO `tx` runs its
//! input options: a snapshot of a variable font at a design location (`-U`), the glyph subset
//! (`-g`, `-gx`) and the GOADB (`-gf`, `-gr`, `-gs`, `-go`), then overlap removal (`+V`).
//!
//! | Step | What it does |
//! |---|---|
//! | Location | Every `fvar` axis is pinned: the named ones at their value (clamped to the axis range, or refused outside it without `clamp`), the others at their default; the instancer makes the static font |
//! | Glyph subset | The glyphs kept, plus `.notdef`, in font order; the subsetter prunes the other tables, keeping every feature, script, name record and `cmap` subtable, with no layout closure |
//! | GOADB subset | Only glyphs whose name is in the first or second column stay, together with the glyph subset when both are given |
//! | GOADB rename | A glyph named in the second column takes the name in the first ([`crate::rename`]); the third column is not used, as in `tx` |
//! | GOADB order | Glyphs listed come first in the file's order (by their final names when renamed, else by their source names), then the others in font order; `.notdef` stays first |
//! | Overlaps | TrueType outlines only ([`crate::outlines`]): before the conversion when the input is TrueType and the target is not, after it when the target is TrueType |
//!
//! Observed with `tx` (AFDKO 5.0.1) on a corpus font: `-g` keeps `.notdef` and the selection in
//! font order and skips names the font lacks; `-gs` with `-g` keeps the glyphs both select;
//! `-go` orders by the GOADB rows, by final names unless `-gr`, the unlisted glyphs following.

use std::collections::{BTreeSet, HashSet};

use super::{ConvertOptions, ConvertTarget, Converted, convert_with};
use crate::goadb::Goadb;
use crate::instance::{AxisLimit, InstanceOptions, instantiate};
use crate::outlines::{OutlineAction, OutlineOptions, repair_report};
use crate::rename::{RenameOptions, rename_glyphs};
use crate::reorder::{ReorderOptions, glyph_names, reorder_glyphs};
use crate::sfnt::{SfntFont, Tag, WriteOptions, tag_str};
use crate::subset::{SubsetOptions, subset};
use crate::{Result, TablesError};

/// How a GOADB takes part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoadbUse {
    pub goadb: Goadb,
    /// Rename glyphs from the second column to the first.
    pub rename: bool,
    /// Keep only the glyphs either column names.
    pub subset: bool,
    /// Put the glyphs in the order of the file.
    pub order: bool,
}

/// What [`prepare`] does; the default does nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct PrepareOptions {
    /// Snapshot a variable font here: axis tag and user value; axes left out sit at their
    /// default, and an empty list snapshots the default instance.
    pub location: Option<Vec<(Tag, f64)>>,
    /// Clamp location values to the axis ranges; without it a value outside is refused.
    pub clamp: bool,
    /// Glyph ids to keep (`.notdef` is always kept); `None` keeps every glyph.
    pub glyphs: Option<BTreeSet<u16>>,
    pub goadb: Option<GoadbUse>,
    /// Merge overlapping contours (TrueType outlines).
    pub remove_overlaps: bool,
    /// A glyph whose overlaps cannot be merged is kept as it was, with a note, instead of failing.
    pub ignore_overlap_errors: bool,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self { location: None, clamp: true, glyphs: None, goadb: None, remove_overlaps: false, ignore_overlap_errors: false }
    }
}

impl PrepareOptions {
    /// Whether any step is asked for.
    pub fn is_empty(&self) -> bool {
        self.location.is_none() && self.glyphs.is_none() && self.goadb.is_none() && !self.remove_overlaps
    }
}

/// Converts `font` as `options` say, with the steps of `prepare` around the conversion. Data the
/// steps leave out (tables the subset drops, glyphs whose overlaps stay) is listed first in
/// [`Converted::left_behind`].
pub fn convert_prepared(font: &SfntFont, options: &ConvertOptions, prepare_options: &PrepareOptions) -> Result<Converted> {
    let to_truetype = matches!(options.target, ConvertTarget::TrueType { .. });
    let (font, mut notes) = prepare(font, prepare_options, !to_truetype)?;
    let mut converted = convert_with(&font, options)?;
    if to_truetype
        && prepare_options.remove_overlaps
        && let Some(result) = converted.font.take()
    {
        let merged = remove_overlaps(&result, prepare_options.ignore_overlap_errors, &mut notes)?;
        converted.bytes = crate::write(&merged, &WriteOptions::default())?;
        converted.font = Some(merged);
    }
    notes.append(&mut converted.left_behind);
    converted.left_behind = notes;
    Ok(converted)
}

/// Runs the steps on `font`; overlaps are removed only when `overlaps_now` says so (the caller
/// removes them after a conversion to TrueType). Returns the font and the notes on what was
/// left out.
pub fn prepare(font: &SfntFont, options: &PrepareOptions, overlaps_now: bool) -> Result<(SfntFont, Vec<String>)> {
    let mut notes = Vec::new();
    let mut font = match &options.location {
        Some(location) => snapshot(font, location, options.clamp)?,
        None => font.clone(),
    };
    if let Some(keep) = kept_glyphs(&font, options)? {
        let result = subset(&font, &subset_options(&keep))?;
        for d in &result.dropped {
            notes.push(format!("the {} table ({})", tag_str(d.tag).trim_end(), d.reason));
        }
        notes.extend(result.warnings);
        font = result.font;
    }
    if let Some(g) = &options.goadb {
        if g.rename {
            font = rename_glyphs(font, &RenameOptions { goadb: Some(g.goadb.clone()), ..RenameOptions::default() })?.font;
        }
        if g.order {
            let order = goadb_order(&font, g)?;
            font = reorder_glyphs(font, &ReorderOptions { order })?;
        }
    }
    if options.remove_overlaps && overlaps_now {
        font = remove_overlaps(&font, options.ignore_overlap_errors, &mut notes)?;
    }
    Ok((font, notes))
}

/// The static font at `location`.
fn snapshot(font: &SfntFont, location: &[(Tag, f64)], clamp: bool) -> Result<SfntFont> {
    if font.get(*b"fvar").is_none() {
        return Err(TablesError::BadOptions("a location needs a variable font (one with an fvar table)".into()));
    }
    if location.len() > crate::limits::MAX_AXES {
        return Err(TablesError::TooLarge { what: "the number of axes in the location", limit: crate::limits::MAX_AXES });
    }
    let axes = crate::variations::fvar_axes(font)?;
    for (i, (tag, _)) in location.iter().enumerate() {
        let name = tag_str(*tag);
        if !axes.iter().any(|a| a.tag == *tag) {
            let known: Vec<String> = axes.iter().map(|a| tag_str(a.tag).trim_end().to_string()).collect();
            return Err(TablesError::BadOptions(format!("the font has no `{}` axis (its axes: {})", name.trim_end(), known.join(", "))));
        }
        if location.iter().take(i).any(|(t, _)| t == tag) {
            return Err(TablesError::BadOptions(format!("the `{}` axis is given twice", name.trim_end())));
        }
    }
    let mut limits = Vec::with_capacity(axes.len());
    for axis in &axes {
        let limit = match location.iter().find(|(t, _)| *t == axis.tag) {
            None => AxisLimit::Drop,
            Some(&(tag, v)) => {
                let name = tag_str(tag);
                if !v.is_finite() {
                    return Err(TablesError::BadOptions(format!("the `{}` value is not a finite number", name.trim_end())));
                }
                if !clamp && (v < axis.minimum || v > axis.maximum) {
                    return Err(TablesError::BadOptions(format!(
                        "`{}` {v} is outside the axis range {} to {}; extrapolating past an axis range is not built",
                        name.trim_end(),
                        axis.minimum,
                        axis.maximum
                    )));
                }
                // Not f64::clamp: a malformed fvar may state a minimum above its maximum.
                AxisLimit::Pin(v.max(axis.minimum).min(axis.maximum))
            }
        };
        limits.push((axis.tag, limit));
    }
    instantiate(font, &InstanceOptions { limits, ..InstanceOptions::default() })
}

/// The glyph ids to keep, or `None` when every glyph stays.
fn kept_glyphs(font: &SfntFont, options: &PrepareOptions) -> Result<Option<BTreeSet<u16>>> {
    let goadb = options.goadb.as_ref().filter(|g| g.subset);
    if options.glyphs.is_none() && goadb.is_none() {
        return Ok(None);
    }
    let names = glyph_names(font)?;
    let count = names.len();
    let mut keep: BTreeSet<u16> = match &options.glyphs {
        Some(set) => set.iter().copied().filter(|g| usize::from(*g) < count).collect(),
        None => (0..count).filter_map(|g| u16::try_from(g).ok()).collect(),
    };
    if let Some(g) = goadb {
        let listed: HashSet<&str> = g.goadb.entries.iter().flat_map(|e| [e.final_name.as_str(), e.source_name.as_str()]).collect();
        keep.retain(|gid| names.get(usize::from(*gid)).is_some_and(|n| listed.contains(n.as_str())));
    }
    keep.insert(0);
    Ok((keep.len() < count).then_some(keep))
}

/// The subsetter's settings for a conversion: only the glyph set changes.
fn subset_options(keep: &BTreeSet<u16>) -> SubsetOptions {
    SubsetOptions {
        gids: keep.iter().map(|g| u32::from(*g)).collect(),
        layout_features: Some(vec!["*".to_string()]),
        layout_closure: false,
        bidi_closure: false,
        notdef_outline: true,
        glyph_names: true,
        legacy_cmap: true,
        symbol_cmap: true,
        all_name_ids: true,
        name_ids: Some(Vec::new()),
        all_name_languages: true,
        name_legacy: true,
        legacy_kern: true,
        passthrough_tables: true,
        prune_unicode_ranges: false,
        prune_codepage_ranges: false,
        drop_tables: Vec::new(),
        recalc_bounds: true,
        ..SubsetOptions::default()
    }
}

/// The names in GOADB order that the font has, each once.
fn goadb_order(font: &SfntFont, g: &GoadbUse) -> Result<Vec<String>> {
    let names = glyph_names(font)?;
    let present: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut order = Vec::new();
    for e in &g.goadb.entries {
        let (first, second) = if g.rename { (&e.final_name, &e.source_name) } else { (&e.source_name, &e.final_name) };
        let name = [first, second].into_iter().map(String::as_str).find(|n| present.contains(n));
        if let Some(name) = name
            && seen.insert(name)
        {
            order.push(name.to_string());
        }
    }
    Ok(order)
}

fn remove_overlaps(font: &SfntFont, ignore_errors: bool, notes: &mut Vec<String>) -> Result<SfntFont> {
    if font.get(*b"glyf").is_none() {
        return Err(TablesError::Unsupported(
            "removing overlaps from CFF or CFF2 outlines: only TrueType outlines are merged (start from a TrueType font or convert to TrueType)"
                .into(),
        ));
    }
    let mut options = OutlineOptions::new(OutlineAction::RemoveOverlaps);
    options.ignore_errors = ignore_errors;
    let report = repair_report(font, &options)?;
    for f in &report.failed {
        notes.push(format!("the overlaps of `{}` ({})", f.glyph, f.reason));
    }
    Ok(report.result.font)
}
