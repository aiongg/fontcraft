//! Preflight: the glyph set a font can be built from (`docs/spec/40-compile.md` § 1.1).
//!
//! In this order, as fontc's glyph-order work does it (`fontir/src/glyph.rs`, Apache-2.0):
//!
//! 1. A component naming a glyph that does not exist is dropped, with a warning, before anything
//!    else looks at components (`compile.pruneMissingComponents`), so one bad reference cannot
//!    get its siblings decomposed.
//! 2. Components that nest more than [`MAX_COMPONENT_DEPTH`] deep or refer back to their own
//!    glyph are an error naming the glyph.
//! 3. A component of a glyph that is not exported is replaced by what that glyph draws: its
//!    contours, transformed (and reversed when the transform mirrors them), and its components,
//!    with the transforms combined (`compile.nonExportGlyphs`). Then the glyphs that are not
//!    exported are removed.
//!
//! [`check_cmap`] runs on the final glyph order: one code point mapped to two glyphs is an error
//! naming both, as fontc refuses it (`compile.cmapConflict`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::{MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, ModelError, ResolvedComponent, ResolvedGlyph};

use crate::{CompileError, Expansion, MAX_EXPANDED_ELEMENTS};

/// The glyphs to build from `glyphs` (every glyph of the font, in its order): the exported ones,
/// with components that are missing dropped and components of glyphs that are not exported
/// inlined.
pub fn run(mut glyphs: Vec<ResolvedGlyph>, warnings: &mut Vec<String>) -> Result<Vec<ResolvedGlyph>, CompileError> {
    prune_missing_components(&mut glyphs, warnings);
    check_component_depth(&glyphs)?;
    flatten_non_export_components(&mut glyphs, MAX_EXPANDED_ELEMENTS)?;
    glyphs.retain(|g| g.export);
    Ok(glyphs)
}

/// Step 1: drop components whose glyph does not exist, one warning per glyph and missing name.
pub fn prune_missing_components(glyphs: &mut [ResolvedGlyph], warnings: &mut Vec<String>) {
    let names: BTreeSet<String> = glyphs.iter().map(|g| g.name.clone()).collect();
    for g in glyphs.iter_mut() {
        let missing: BTreeSet<String> = g.components.iter().filter(|c| !names.contains(&c.base)).map(|c| c.base.clone()).collect();
        if missing.is_empty() {
            continue;
        }
        for base in &missing {
            warnings.push(format!("glyph `{}` uses `{base}` as a component, but there is no such glyph; the component was left out", g.name));
        }
        g.components.retain(|c| !missing.contains(&c.base));
    }
}

/// Step 2: every chain of components ends within [`MAX_COMPONENT_DEPTH`] levels.
pub fn check_component_depth(glyphs: &[ResolvedGlyph]) -> Result<(), CompileError> {
    let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    // The depth below each glyph once known; `None` while unknown. Computed with an explicit
    // stack so hostile nesting cannot exhaust the call stack.
    let mut depth: Vec<Option<usize>> = vec![None; glyphs.len()];
    for start in 0..glyphs.len() {
        if depth.get(start).copied().flatten().is_some() {
            continue;
        }
        // (glyph, next component to look at); the stack is the current chain.
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&mut (i, ref mut next)) = stack.last_mut() {
            let Some(g) = glyphs.get(i) else { break };
            if let Some(c) = g.components.get(*next) {
                *next += 1;
                let Some(&child) = index.get(c.base.as_str()) else { continue };
                if depth.get(child).copied().flatten().is_some() {
                    continue;
                }
                if stack.len() > MAX_COMPONENT_DEPTH || stack.iter().any(|(j, _)| *j == child) {
                    return Err(ModelError::ComponentCycle(glyphs.get(start).map_or_else(String::new, |g| g.name.clone())).into());
                }
                stack.push((child, 0));
                continue;
            }
            let below = g.components.iter().filter_map(|c| index.get(c.base.as_str())).filter_map(|&j| depth.get(j).copied().flatten()).max();
            let d = below.map_or(0, |b| b + 1);
            if d > MAX_COMPONENT_DEPTH {
                return Err(ModelError::ComponentCycle(g.name.clone()).into());
            }
            if let Some(slot) = depth.get_mut(i) {
                *slot = Some(d);
            }
            stack.pop();
        }
    }
    Ok(())
}

/// What a glyph draws once its non-exported components are inlined.
#[derive(Clone)]
struct Flat {
    contours: Vec<BezPath>,
    components: Vec<ResolvedComponent>,
}

/// Step 3: inline the components of glyphs that are not exported, creating at most `limit` path
/// elements and components in all. Assumes [`check_component_depth`] passed.
pub fn flatten_non_export_components(glyphs: &mut [ResolvedGlyph], limit: usize) -> Result<(), CompileError> {
    let index: HashMap<String, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
    let export: Vec<bool> = glyphs.iter().map(|g| g.export).collect();
    let uses_hidden = |g: &ResolvedGlyph| g.components.iter().any(|c| index.get(&c.base).and_then(|&j| export.get(j)) == Some(&false));
    if !glyphs.iter().any(uses_hidden) {
        return Ok(());
    }
    let mut memo: BTreeMap<usize, Flat> = BTreeMap::new();
    let mut expansion = Expansion::new(limit);
    let todo: Vec<usize> = glyphs.iter().enumerate().filter(|(_, g)| uses_hidden(g)).map(|(i, _)| i).collect();
    for i in todo {
        let flat = flattened(glyphs, &index, &export, i, 0, &mut memo, &mut expansion)?;
        if let Some(g) = glyphs.get_mut(i) {
            g.contours = flat.contours;
            g.components = flat.components;
        }
    }
    Ok(())
}

fn flattened(
    glyphs: &[ResolvedGlyph],
    index: &HashMap<String, usize>,
    export: &[bool],
    i: usize,
    depth: usize,
    memo: &mut BTreeMap<usize, Flat>,
    expansion: &mut Expansion,
) -> Result<Flat, CompileError> {
    if let Some(f) = memo.get(&i) {
        return Ok(f.clone());
    }
    let Some(g) = glyphs.get(i) else { return Ok(Flat { contours: Vec::new(), components: Vec::new() }) };
    if depth > MAX_COMPONENT_DEPTH {
        return Err(ModelError::ComponentCycle(g.name.clone()).into());
    }
    let mut out = Flat { contours: g.contours.clone(), components: Vec::with_capacity(g.components.len()) };
    let mut size: usize = out.contours.iter().map(|c| c.elements().len()).sum();
    for c in &g.components {
        let Some(&j) = index.get(&c.base) else { continue };
        if export.get(j).copied().unwrap_or(true) {
            out.components.push(c.clone());
            continue;
        }
        let inner = flattened(glyphs, index, export, j, depth + 1, memo, expansion)?;
        expansion.take(inner.components.len(), &g.name)?;
        for ic in inner.components {
            out.components.push(ResolvedComponent { base: ic.base, transform: c.transform * ic.transform });
        }
        for contour in inner.contours {
            size = size.saturating_add(contour.elements().len());
            expansion.take(contour.elements().len(), &g.name)?;
            if size > MAX_OUTLINE_ELEMENTS {
                return Err(ModelError::OutlineTooLarge(g.name.clone()).into());
            }
            out.contours.push(transformed(&contour, c.transform));
        }
        if out.components.len() > crate::MAX_COMPONENTS {
            return Err(CompileError::Glyph { glyph: g.name.clone(), message: format!("more than {} components", crate::MAX_COMPONENTS) });
        }
    }
    memo.insert(i, out.clone());
    Ok(out)
}

/// `contour` under `transform`; a mirroring transform also reverses it, so the outline keeps its
/// direction (ufo2ft and fontc do the same).
pub(crate) fn transformed(contour: &BezPath, transform: Affine) -> BezPath {
    let mut c = contour.clone();
    c.apply_affine(transform);
    if transform.determinant() < 0.0 { c.reverse_subpaths() } else { c }
}

/// No code point is mapped to two glyphs. A glyph may have several code points, and the same
/// mapping twice is no conflict.
pub fn check_cmap(glyphs: &[ResolvedGlyph]) -> Result<(), CompileError> {
    let mut owners: HashMap<u32, &str> = HashMap::new();
    for g in glyphs {
        for &cp in &g.unicodes {
            if let Some(first) = owners.insert(cp, &g.name)
                && first != g.name
            {
                return Err(CompileError::DuplicateCodepoint { codepoint: cp, first: first.to_string(), second: g.name.clone() });
            }
        }
    }
    Ok(())
}
