//! Applying chosen changes of a [`Diff`] to a copy of A (`source.merge`).
//!
//! The changes come from comparing the same two fonts, so their indices point into A and B as
//! they are. Parts are applied in an order that keeps the font consistent: axes (moving the
//! master and instance locations with them), masters, instances, Font Info, glyphs, kerning,
//! features; the layers and kerning of removed masters go last. The caller checks the result
//! before writing anything.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, GlyphTable, KernFirst, KernTable, MasterKerning, Shape};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::compare::{Change, Diff, List, Place, glyph_at, map_glyph, mapped_context, to_json};
use crate::{EngineError, Result};

fn stale(c: &Change) -> EngineError {
    EngineError::Interaction(format!("change {} does not apply to BASE as it is now: compare the sources again", c.id))
}

/// A copy of `a` with the `chosen` changes of `diff` (a comparison of `a` with `b`) applied.
pub(super) fn apply(a: &Font, b: &Font, diff: &Diff, chosen: &[&Change]) -> Result<Font> {
    let mut font = a.clone();
    let of = |list: List| -> Vec<&Change> {
        chosen
            .iter()
            .copied()
            .filter(|c| match &c.place {
                Place::Record { list: l, .. } | Place::Order(l) => *l == list,
                _ => false,
            })
            .collect()
    };
    let pairs = |list: List| diff.pairs.get(&list).cloned().unwrap_or_default();

    let sel = of(List::Axes);
    if !sel.is_empty() {
        let (axes, from) = records(&font.axes, &b.axes, &pairs(List::Axes), &sel, Clone::clone)?;
        let identity = from.len() == font.axes.len() && from.iter().enumerate().all(|(i, f)| *f == Some(i));
        if !identity {
            let fill: Vec<f64> = axes.iter().map(|x| x.range.as_ref().map_or(0.0, |r| r.default)).collect();
            let relocate = |loc: &mut Vec<f64>| {
                *loc = from.iter().zip(&fill).map(|(f, d)| f.and_then(|i| loc.get(i)).copied().unwrap_or(*d)).collect();
            };
            Arc::make_mut(&mut font.masters).iter_mut().for_each(|m| relocate(&mut m.location));
            Arc::make_mut(&mut font.instances).iter_mut().for_each(|m| relocate(&mut m.location));
        }
        font.axes = Arc::new(axes);
    }

    let mut removed_masters = BTreeSet::new();
    let sel = of(List::Masters);
    if !sel.is_empty() {
        for c in &sel {
            if let Place::Record { a: Some(i), b: None, field: None, .. } = c.place {
                removed_masters.insert(a.masters.get(i).ok_or_else(|| stale(c))?.id.clone());
            }
        }
        let added = |m: &fontcraft_model::Master| {
            let mut m = m.clone();
            m.id = diff.masters.a_id(&m.id);
            m
        };
        let (masters, _) = records(&font.masters, &b.masters, &pairs(List::Masters), &sel, added)?;
        font.masters = Arc::new(masters);
    }

    let sel = of(List::Instances);
    if !sel.is_empty() {
        font.instances = Arc::new(records(&font.instances, &b.instances, &pairs(List::Instances), &sel, Clone::clone)?.0);
    }

    for c in chosen {
        if let Place::Info(name) = c.place {
            set_info(&mut font, b, name).map_err(|()| stale(c))?;
        }
    }
    let sel = of(List::Properties);
    if !sel.is_empty() {
        let list = records(&font.info.properties.0, &b.info.properties.0, &pairs(List::Properties), &sel, Clone::clone)?.0;
        Arc::make_mut(&mut font.info).properties.0 = list;
    }
    let sel = of(List::Params);
    if !sel.is_empty() {
        let list = records(&font.info.params.0, &b.info.params.0, &pairs(List::Params), &sel, Clone::clone)?.0;
        Arc::make_mut(&mut font.info).params.0 = list;
    }

    glyphs(&mut font, b, diff, chosen)?;
    kerning(&mut font, b, diff, chosen)?;

    let sel = of(List::Prefixes);
    if !sel.is_empty() {
        let items = records(&font.features.prefixes, &b.features.prefixes, &pairs(List::Prefixes), &sel, Clone::clone)?.0;
        Arc::make_mut(&mut font.features).prefixes = items;
    }
    let sel = of(List::Classes);
    if !sel.is_empty() {
        let items = records(&font.features.classes, &b.features.classes, &pairs(List::Classes), &sel, Clone::clone)?.0;
        Arc::make_mut(&mut font.features).classes = items;
    }
    let sel = of(List::Features);
    if !sel.is_empty() {
        let items = records(&font.features.features, &b.features.features, &pairs(List::Features), &sel, Clone::clone)?.0;
        Arc::make_mut(&mut font.features).features = items;
    }
    let sel = of(List::Groups);
    if !sel.is_empty() {
        font.groups = Arc::new(records(&font.groups, &b.groups, &pairs(List::Groups), &sel, Clone::clone)?.0);
    }

    if !removed_masters.is_empty() {
        drop_masters(&mut font, &removed_masters);
    }
    Ok(font)
}

/// `item` with its field `field` taken from `from` (left out when `from` has none, which is the
/// field's default).
fn set_field<T: Serialize + DeserializeOwned>(item: &T, from: &T, field: &str) -> Result<T> {
    let mut v = to_json(item)?;
    let w = to_json(from)?;
    if let (Value::Object(m), Value::Object(n)) = (&mut v, &w) {
        match n.get(field) {
            Some(x) => {
                m.insert(field.to_string(), x.clone());
            }
            None => {
                m.remove(field);
            }
        }
    }
    serde_json::from_value(v).map_err(|e| EngineError::Interaction(format!("cannot set `{field}`: {e}")))
}

/// The list `base` with the chosen record changes applied, and for each resulting item its index
/// in `base` (`None` for an item added from `other`).
fn records<T: Clone + Serialize + DeserializeOwned>(
    base: &[T],
    other: &[T],
    pairs: &[(Option<usize>, Option<usize>)],
    chosen: &[&Change],
    added: impl Fn(&T) -> T,
) -> Result<(Vec<T>, Vec<Option<usize>>)> {
    let mut items: Vec<Option<T>> = base.iter().cloned().map(Some).collect();
    let mut extra: Vec<(T, usize)> = Vec::new();
    let mut reorder = false;
    for c in chosen {
        let other_item = |j: usize| other.get(j).ok_or_else(|| stale(c));
        match &c.place {
            Place::Order(_) => reorder = true,
            Place::Record { a, b, field, .. } => match (*a, *b, field) {
                (Some(i), None, None) => *items.get_mut(i).ok_or_else(|| stale(c))? = None,
                (None, Some(j), None) => extra.push((added(other_item(j)?), j)),
                (Some(i), Some(j), None) => *items.get_mut(i).ok_or_else(|| stale(c))? = Some(other_item(j)?.clone()),
                (Some(i), Some(j), Some(f)) => {
                    let slot = items.get_mut(i).ok_or_else(|| stale(c))?;
                    let current = slot.as_ref().ok_or_else(|| stale(c))?;
                    *slot = Some(set_field(current, other_item(j)?, f)?);
                }
                _ => return Err(stale(c)),
            },
            _ => {}
        }
    }
    let b_of: HashMap<usize, usize> = pairs.iter().filter_map(|&(a, b)| Some((a?, b?))).collect();
    let mut out: Vec<(T, Option<usize>, Option<usize>)> =
        items.into_iter().enumerate().filter_map(|(i, t)| t.map(|t| (t, Some(i), b_of.get(&i).copied()))).collect();
    out.extend(extra.into_iter().map(|(t, j)| (t, None, Some(j))));
    if reorder {
        // Stable: items B does not have keep their order, after B's.
        out.sort_by_key(|(_, _, b)| b.unwrap_or(usize::MAX));
    }
    Ok(out.into_iter().map(|(t, a, _)| (t, a)).unzip())
}

fn set_info(font: &mut Font, b: &Font, name: &str) -> std::result::Result<(), ()> {
    match name {
        "familyName" => font.family_name = b.family_name.clone(),
        "unitsPerEm" => font.units_per_em = b.units_per_em,
        "versionMajor" => font.version_major = b.version_major,
        "versionMinor" => font.version_minor = b.version_minor,
        "metrics" => font.metrics = b.metrics.clone(),
        "stems" => font.stems = b.stems.clone(),
        "numbers" => font.numbers = b.numbers.clone(),
        "displayStrings" => font.display_strings = b.display_strings.clone(),
        "note" => font.note = b.note.clone(),
        "files" => font.files = b.files.clone(),
        "extra" => font.extra = b.extra.clone(),
        "settings" => Arc::make_mut(&mut font.info).settings = b.info.settings.clone(),
        "date" => Arc::make_mut(&mut font.info).date = b.info.date.clone(),
        "userData" => Arc::make_mut(&mut font.info).user_data = b.info.user_data.clone(),
        _ => return Err(()),
    }
    Ok(())
}

/// `template`'s shapes with its paths (and images, raw shapes) taken from `paths` and its
/// components from `components`, in `template`'s interleaving; extra shapes go at the end.
fn recombine(template: &[Shape], paths: Vec<Shape>, components: Vec<Shape>) -> Vec<Shape> {
    let mut p = paths.into_iter();
    let mut c = components.into_iter();
    let mut out: Vec<Shape> = template.iter().filter_map(|s| if matches!(s, Shape::Component(_)) { c.next() } else { p.next() }).collect();
    out.extend(p);
    out.extend(c);
    out
}

fn split(shapes: &[Shape]) -> (Vec<Shape>, Vec<Shape>) {
    let (components, paths): (Vec<Shape>, Vec<Shape>) = shapes.iter().cloned().partition(|s| matches!(s, Shape::Component(_)));
    (paths, components)
}

fn set_glyph_field(g: &mut Glyph, from: &Glyph, field: &str, diff: &Diff) -> std::result::Result<(), ()> {
    match field {
        "name" => g.name = from.name.clone(),
        "unicodes" => g.unicodes = from.unicodes.clone(),
        "export" => g.export = from.export,
        "info" => g.info = from.info.clone(),
        "extra" => g.extra = from.extra.clone(),
        "kernGroups" => g.kern_groups = from.kern_groups.clone(),
        "otherLayers" => {
            g.layers.retain(|l| l.is_master_layer());
            let mapped = map_glyph(from, &diff.masters);
            g.layers.extend(mapped.layers.into_iter().filter(|l| !l.is_master_layer()));
        }
        _ => return Err(()),
    }
    Ok(())
}

fn slot<'a>(list: &'a mut [Option<Arc<Glyph>>], i: usize, c: &Change) -> Result<&'a mut Glyph> {
    list.get_mut(i).and_then(Option::as_mut).map(Arc::make_mut).ok_or_else(|| stale(c))
}

fn glyphs(font: &mut Font, b: &Font, diff: &Diff, chosen: &[&Change]) -> Result<()> {
    let mut list: Vec<Option<Arc<Glyph>>> = font.glyphs.as_arcs().iter().cloned().map(Some).collect();
    let mut added: Vec<(Arc<Glyph>, usize)> = Vec::new();
    let mut reorder = false;
    let mut touched = false;
    for c in chosen {
        let b_glyph = |j: usize| glyph_at(b, j).ok_or_else(|| stale(c));
        match &c.place {
            Place::Order(List::Glyphs) => reorder = true,
            Place::Record { list: List::Glyphs, a: Some(i), b: None, field: None } => {
                *list.get_mut(*i).ok_or_else(|| stale(c))? = None;
            }
            Place::Record { list: List::Glyphs, a: None, b: Some(j), field: None } => {
                added.push((Arc::new(map_glyph(b_glyph(*j)?, &diff.masters)), *j));
            }
            Place::GlyphField { a: i, b: j, field } => {
                let from = b_glyph(*j)?;
                set_glyph_field(slot(&mut list, *i, c)?, from, field, diff).map_err(|()| stale(c))?;
            }
            Place::Layer { a: i, b: j, master_a, master_b, field } => {
                let from = b_glyph(*j)?;
                let lb = master_b.as_deref().and_then(|id| from.layer(id));
                let g = slot(&mut list, *i, c)?;
                match (field, lb) {
                    (None, Some(lb)) => {
                        if g.layer(master_a).is_some() {
                            return Err(stale(c));
                        }
                        let mut l = lb.clone();
                        l.id = master_a.clone();
                        l.master = master_a.clone();
                        g.layers.push(l);
                    }
                    (None, None) => g.layers.retain(|l| !(l.id == *master_a && l.master == *master_a)),
                    (Some(f), Some(lb)) => {
                        let la = g.layer_mut(master_a).ok_or_else(|| stale(c))?;
                        match *f {
                            "width" => la.width = lb.width,
                            "outlines" => {
                                let (_, components) = split(&la.shapes);
                                la.shapes = recombine(&la.shapes, split(&lb.shapes).0, components);
                            }
                            "components" => {
                                let (paths, _) = split(&la.shapes);
                                la.shapes = recombine(&la.shapes, paths, split(&lb.shapes).1);
                            }
                            "anchors" => la.anchors = lb.anchors.clone(),
                            "info" => la.info = lb.info.clone(),
                            "extra" => la.extra = lb.extra.clone(),
                            _ => return Err(stale(c)),
                        }
                    }
                    (Some(_), None) => return Err(stale(c)),
                }
            }
            _ => continue,
        }
        touched = true;
    }
    if !touched {
        return Ok(());
    }
    let b_of: HashMap<usize, usize> =
        diff.pairs.get(&List::Glyphs).map(|p| p.iter().filter_map(|&(a, b)| Some((a?, b?))).collect()).unwrap_or_default();
    let mut out: Vec<(Arc<Glyph>, Option<usize>)> = list.into_iter().enumerate().filter_map(|(i, g)| g.map(|g| (g, b_of.get(&i).copied()))).collect();
    out.extend(added.into_iter().map(|(g, j)| (g, Some(j))));
    if reorder {
        out.sort_by_key(|(_, b)| b.unwrap_or(usize::MAX));
    }
    let mut names = BTreeSet::new();
    for (g, _) in &out {
        if !names.insert(g.name.as_str()) {
            return Err(EngineError::Interaction(format!(
                "the chosen changes leave two glyphs called `{}`: take the change that renames or removes the other one too",
                g.name
            )));
        }
    }
    font.glyphs = GlyphTable::from_glyphs(out.into_iter().map(|(g, _)| g).collect());
    Ok(())
}

/// Sets (or with `None` removes) one kerning pair, creating and dropping entries as needed.
fn set_pair(t: &mut KernTable, master: &str, first: &str, second: &str, value: Option<f64>) {
    let mk = match t.0.iter_mut().position(|m| m.master == master) {
        Some(i) => t.0.get_mut(i),
        None if value.is_some() => {
            t.0.push(MasterKerning { master: master.to_string(), firsts: Vec::new() });
            t.0.last_mut()
        }
        None => None,
    };
    let Some(mk) = mk else { return };
    let kf = match mk.firsts.iter().position(|f| f.first == first) {
        Some(i) => mk.firsts.get_mut(i),
        None if value.is_some() => {
            mk.firsts.push(KernFirst { first: first.to_string(), seconds: Vec::new() });
            mk.firsts.last_mut()
        }
        None => None,
    };
    let Some(kf) = kf else { return };
    match value {
        Some(v) => match kf.seconds.iter_mut().find(|(s, _)| s == second) {
            Some(pair) => pair.1 = v,
            None => kf.seconds.push((second.to_string(), v)),
        },
        None => kf.seconds.retain(|(s, _)| s != second),
    }
    mk.firsts.retain(|f| !f.seconds.is_empty());
    t.0.retain(|m| !m.firsts.is_empty());
}

fn kerning(font: &mut Font, b: &Font, diff: &Diff, chosen: &[&Change]) -> Result<()> {
    for c in chosen {
        match &c.place {
            Place::KernPair { dir, master, first, second } => {
                let value = match &c.b {
                    Some(v) => Some(v.as_f64().ok_or_else(|| stale(c))?),
                    None => None,
                };
                set_pair(dir.table_mut(Arc::make_mut(&mut font.kerning)), master, first, second, value);
            }
            Place::KernContext => Arc::make_mut(&mut font.kerning).context = mapped_context(&b.kerning, &diff.masters),
            _ => {}
        }
    }
    Ok(())
}

/// Drops the layers and kerning of masters that were removed.
fn drop_masters(font: &mut Font, removed: &BTreeSet<String>) {
    for i in 0..font.glyphs.len() {
        if !font.glyphs.as_arcs().get(i).is_some_and(|g| g.layers.iter().any(|l| removed.contains(&l.master))) {
            continue;
        }
        if let Some(mut g) = font.glyphs.get_mut_at(i) {
            g.layers.retain(|l| !removed.contains(&l.master));
        }
    }
    let k = Arc::make_mut(&mut font.kerning);
    for t in [&mut k.ltr, &mut k.rtl, &mut k.vertical] {
        t.0.retain(|m| !removed.contains(&m.master));
    }
    for c in &mut k.context {
        c.values.retain(|(m, _)| !removed.contains(m));
    }
}
