//! Derived metrics of aligned composites, and the re-alignment step the engine runs after each
//! font-changing command (M6.2).
//!
//! Rows: `components.anchoredMarksDoNotAffectSidebearings`, `metricsKey.autoAlignedOffset` (the
//! offset arithmetic), the re-alignment half of `components.liveUpdateFromBaseGlyph`.
//!
//! Contract:
//! - [`derived_metrics`]: an aligned layer's width and sidebearings come from its base
//!   components (advances summed, no kerning; marks attached by anchors do not count), plus the
//!   offsets of `=+N` / `=-N` metrics keys ([`parse_offset`]).
//! - [`realign`] is the engine hook. After a font-changing command it takes the font before and
//!   after, and re-aligns the glyphs that changed and every glyph that depends on them, in
//!   dependency order, each layer at most once. It returns the font to keep, pointer-equal to
//!   `new` when nothing moves, so the engine can put it in the same undo step. An error means the
//!   command fails and the engine rolls back.
//! - Glyphs in a cycle, and the glyphs above one, are skipped, not an error: the editor shows
//!   them as placeholders. A layer whose components nest too deep, whose anchors cannot be read
//!   for a missing glyph or layer, or whose work passes a limit, is skipped the same way and
//!   keeps its stored positions, so one unusual glyph never blocks edits to its bases.
//! - Only the masters whose layers changed are re-aligned. On the other masters, a layer is
//!   re-aligned when it holds a component pinned (`ComponentInfo::master_id`) to a re-aligned
//!   master of an affected base, or a component whose base layer was re-aligned that way; the
//!   other layers of those masters keep their stored positions. A change of masters, of the
//!   alignment setting or of Propagate Anchors re-aligns every master.
//! - Locked glyphs are skipped: a lock keeps the glyph's drawing as it is.
//! - The dependency order is worked out for the affected glyphs only, from an index of each base
//!   glyph's users. The index of the last glyph tables seen is kept per thread and brought up to
//!   date from the glyphs that differ, so one edit costs a pointer comparison over the glyphs
//!   plus the work on the glyphs that follow it, not a pass over every layer. The anchor results
//!   alignment reads are kept the same way ([`AnchorCache`] checks each one against the font).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use fontcraft_geom::{BezPath, Shape as _};
use fontcraft_model::{Font, Glyph, GlyphTable, Layer, Shape};

use crate::ComponentsError;
use crate::align::{align_layer_cached, alignment_off, find_layer, plan_layer};
use crate::propagate::{AnchorCache, propagate_anchors_enabled};

/// The width and sidebearings automatic alignment gives a layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DerivedMetrics {
    pub width: f64,
    /// `None` for a layer without outlines.
    pub lsb: Option<f64>,
    pub rsb: Option<f64>,
}

/// The amount of an offset key: `=+18` → `Some(18.0)`, `=-12` → `Some(-12.0)`. `None` for any
/// other text, and for numbers that are not finite.
pub fn parse_offset(key: &str) -> Option<f64> {
    let rest = key.trim().strip_prefix('=')?.trim_start();
    let (sign, number) = match (rest.strip_prefix('+'), rest.strip_prefix('-')) {
        (Some(n), _) => (1.0, n.trim()),
        (_, Some(n)) => (-1.0, n.trim()),
        _ => return None,
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let value = sign * number.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

/// The left and right offsets of `layer`'s metrics keys: the layer's own key for a side, else
/// the glyph's; 0 for a side whose key is missing or not an offset.
pub(crate) fn offsets(glyph: &Glyph, layer: &Layer) -> (f64, f64) {
    let side = |own: &Option<String>, glyph_key: &Option<String>| {
        let key = own.as_deref().filter(|k| !k.trim().is_empty()).or_else(|| glyph_key.as_deref().filter(|k| !k.trim().is_empty()));
        key.and_then(parse_offset).unwrap_or(0.0)
    };
    let (own, shared) = (&layer.info.metrics_keys, &glyph.info.metrics_keys);
    (side(&own.left, &shared.left), side(&own.right, &shared.right))
}

/// The derived metrics of `glyph`'s layer `layer`, with the layer's offset keys applied; `None`
/// when the layer's metrics are not derived: no component is aligned, or a path is on the layer.
pub fn derived_metrics(font: &Font, glyph: &str, layer: &str) -> Result<Option<DerivedMetrics>, ComponentsError> {
    let (g, l) = find_layer(font, glyph, layer)?;
    let Some(plan) = plan_layer(font, g, l, &mut AnchorCache::new())? else { return Ok(None) };
    let Some(width) = plan.width else { return Ok(None) };
    let mut extent: Option<(f64, f64)> = None;
    for part in plan.parts.iter().filter(|p| p.base) {
        let Some((base, base_layer)) = &part.source else { continue };
        let Some(base_glyph) = font.glyph(base) else { continue };
        let mut outline = layer_outline(font, base_glyph, base_layer)?;
        if outline.elements().is_empty() {
            continue;
        }
        outline.apply_affine(part.transform.affine());
        let b = outline.bounding_box();
        extent = Some(extent.map_or((b.x0, b.x1), |(lo, hi)| (lo.min(b.x0), hi.max(b.x1))));
    }
    Ok(Some(DerivedMetrics { width, lsb: extent.map(|(lo, _)| lo), rsb: extent.map(|(_, hi)| width - hi) }))
}

/// The resolved outline of `glyph`'s layer `layer_id`; nested components read their master
/// layers, as [`Font::outline`] does.
fn layer_outline(font: &Font, glyph: &Glyph, layer_id: &str) -> Result<BezPath, ComponentsError> {
    let Some(layer) = glyph.layers.iter().find(|l| l.id == layer_id) else { return Ok(BezPath::new()) };
    if layer.is_master_layer() {
        return Ok(font.outline_of(glyph, layer_id)?);
    }
    // The model resolves master layers only: lend this layer the master's id.
    let mut lent = Glyph::new(glyph.name.clone());
    let mut copy = Layer::new(layer.master.clone(), layer.width);
    copy.shapes = layer.shapes.clone();
    lent.layers = vec![copy];
    Ok(font.outline_of(&lent, &layer.master)?)
}

/// The names of the glyphs whose `Arc` differs between `old` and `new`, those added, and those
/// removed, in `new`'s order then removed ones. Every glyph when the masters or the font's
/// alignment setting changed.
pub fn changed_glyphs(old: &Font, new: &Font) -> Vec<String> {
    changes(old, new).0
}

/// Whether a change reaches every glyph on every master: the masters, the alignment setting or
/// Propagate Anchors changed.
fn reaches_everything(old: &Font, new: &Font) -> bool {
    let masters_changed = !Arc::ptr_eq(&old.masters, &new.masters) && old.masters != new.masters;
    let settings_changed = !Arc::ptr_eq(&old.info, &new.info)
        && (alignment_off(old) != alignment_off(new) || propagate_anchors_enabled(old) != propagate_anchors_enabled(new));
    masters_changed || settings_changed
}

/// [`changed_glyphs`], and whether the change reaches every glyph on every master
/// ([`reaches_everything`]).
fn changes(old: &Font, new: &Font) -> (Vec<String>, bool) {
    if reaches_everything(old, new) {
        return (new.glyphs.iter().map(|g| g.name.clone()).collect(), true);
    }
    let (now, before) = (new.glyphs.as_arcs(), old.glyphs.as_arcs());
    if std::ptr::eq(now, before) {
        return (Vec::new(), false);
    }
    let mut out = Vec::new();
    for (i, g) in now.iter().enumerate() {
        if before.get(i).is_some_and(|b| Arc::ptr_eq(b, g)) {
            continue;
        }
        if old.glyph(&g.name).is_some_and(|b| Arc::ptr_eq(b, g)) {
            continue;
        }
        out.push(g.name.clone());
    }
    out.extend(before.iter().filter(|b| new.glyph(&b.name).is_none()).map(|b| b.name.clone()));
    (out, false)
}

/// `font` with `changed` and every glyph that depends on them re-aligned, in dependency order.
pub fn realign_glyphs(font: Font, changed: &[String]) -> Result<Font, ComponentsError> {
    realign_on(font, changed, None)
}

/// [`realign_glyphs`] on the layers of `masters` only (`None`: every layer).
fn realign_on(font: Font, changed: &[String], masters: Option<&HashSet<String>>) -> Result<Font, ComponentsError> {
    if changed.is_empty() || alignment_off(&font) {
        return Ok(font);
    }
    let order = affected_order(&font, changed);
    let affected: HashSet<usize> = order.iter().copied().collect();
    // Glyph position → masters outside `masters` on which a layer of it is re-aligned because it
    // reads a re-aligned base layer. Bases come first in `order`, so one pass fills it.
    let mut followed: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut cache = take_anchor_cache();
    let out = realign_in_order(font, order, &affected, masters, &mut followed, &mut cache);
    keep_anchor_cache(cache);
    out
}

/// The loop of [`realign_on`]: each glyph of `order` re-aligned on the wanted layers.
fn realign_in_order(
    mut font: Font,
    order: Vec<usize>,
    affected: &HashSet<usize>,
    masters: Option<&HashSet<String>>,
    followed: &mut HashMap<usize, HashSet<String>>,
    cache: &mut AnchorCache,
) -> Result<Font, ComponentsError> {
    for i in order {
        let Some(glyph) = font.glyphs.as_arcs().get(i).cloned() else { continue };
        if glyph.info.locked {
            continue;
        }
        let mut updates: Vec<(usize, Layer)> = Vec::new();
        for (k, layer) in glyph.layers.iter().enumerate() {
            if !layer.shapes.iter().any(|s| matches!(s, Shape::Component(_))) {
                continue;
            }
            let wanted = match masters {
                None => true,
                Some(m) if m.contains(&layer.master) => true,
                Some(m) => {
                    let follows = reads_realigned(&font, layer, affected, m, followed);
                    if follows {
                        followed.entry(i).or_default().insert(layer.master.clone());
                    }
                    follows
                }
            };
            if !wanted {
                continue;
            }
            match align_layer_cached(&font, &glyph, layer, cache) {
                Ok(Some(new_layer)) => updates.push((k, new_layer)),
                // States of the font the editor shows as placeholders; they block no edit.
                Ok(None)
                | Err(
                    ComponentsError::Cycle(_)
                    | ComponentsError::TooDeep { .. }
                    | ComponentsError::MissingGlyph(_)
                    | ComponentsError::MissingLayer { .. }
                    | ComponentsError::LimitExceeded { .. },
                ) => {}
                Err(e) => return Err(e),
            }
        }
        if updates.is_empty() {
            continue;
        }
        if let Some(mut g) = font.glyphs.get_mut_at(i) {
            for (k, new_layer) in updates {
                if let Some(slot) = g.layers.get_mut(k) {
                    *slot = new_layer;
                }
            }
        }
    }
    Ok(font)
}

/// Whether a component of `layer` reads a base layer this pass re-aligns or that changed: the
/// base is affected and the master the component reads is in `masters`, or that base layer was
/// re-aligned on its master because it follows a pinned component (`followed`).
fn reads_realigned(
    font: &Font,
    layer: &Layer,
    affected: &HashSet<usize>,
    masters: &HashSet<String>,
    followed: &HashMap<usize, HashSet<String>>,
) -> bool {
    layer.shapes.iter().any(|s| {
        let Shape::Component(c) = s else { return false };
        let Some(base) = font.glyphs.position(&c.glyph).filter(|b| affected.contains(b)) else { return false };
        let pinned = font.glyphs.as_arcs().get(base).and_then(|g| c.pinned_layer(g));
        let read = pinned.map_or(layer.master.as_str(), |l| l.master.as_str());
        masters.contains(read) || followed.get(&base).is_some_and(|m| m.contains(read))
    })
}

/// Base name → the positions of the glyphs with a component of it, ascending, each once.
type Users = HashMap<String, Vec<usize>>;

/// The [`Users`] of one glyph table.
struct UsersOf {
    table: GlyphTable,
    users: Users,
}

/// Glyph tables whose users are kept.
const REMEMBERED_TABLES: usize = 2;
/// A kept index is updated rather than built again while at most this fraction (one in N) of
/// the glyphs differ.
const UPDATE_SHARE: usize = 4;
/// Anchor results kept between edits; past it the kept results are dropped.
const MAX_KEPT_ANCHORS: usize = 50_000;

thread_local! {
    /// The users of the glyph tables re-aligned last, newest first. Holding a table keeps its
    /// storage alive and unchanged (every change to a shared table copies it first), so a table
    /// at the same address holds the same glyphs.
    static USERS: RefCell<Vec<UsersOf>> = const { RefCell::new(Vec::new()) };
    /// The anchor results of the last re-alignment, for the next one.
    static ANCHORS: RefCell<AnchorCache> = RefCell::new(AnchorCache::new());
}

fn take_anchor_cache() -> AnchorCache {
    ANCHORS.with(|c| c.try_borrow_mut().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default())
}

fn keep_anchor_cache(cache: AnchorCache) {
    if cache.len() > MAX_KEPT_ANCHORS {
        return;
    }
    ANCHORS.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            *c = cache;
        }
    });
}

/// Adds glyph `i` to the users of each glyph it has a component of.
fn add_users(users: &mut Users, glyph: &Glyph, i: usize) {
    for name in component_names(glyph) {
        match users.get_mut(name) {
            Some(list) => {
                if let Err(at) = list.binary_search(&i) {
                    list.insert(at, i);
                }
            }
            None => {
                users.insert(name.to_string(), vec![i]);
            }
        }
    }
}

/// Takes glyph `i`, as `glyph` was, out of the users of each glyph it had a component of.
fn remove_users(users: &mut Users, glyph: &Glyph, i: usize) {
    for name in component_names(glyph) {
        let Some(list) = users.get_mut(name) else { continue };
        if let Ok(at) = list.binary_search(&i) {
            list.remove(at);
        }
        if list.is_empty() {
            users.remove(name);
        }
    }
}

fn users_of(glyphs: &[Arc<Glyph>]) -> Users {
    let mut users = Users::new();
    for (i, g) in glyphs.iter().enumerate() {
        add_users(&mut users, g, i);
    }
    users
}

/// Brings `kept` up to date for `table`, which has as many glyphs, from the glyphs that differ;
/// false (and `kept` unusable) when too many differ.
fn update_users(kept: &mut UsersOf, table: &GlyphTable) -> bool {
    let (was, now) = (kept.table.as_arcs(), table.as_arcs());
    let limit = now.len() / UPDATE_SHARE;
    let differ: Vec<usize> = (0..now.len())
        .filter(|&i| !matches!((was.get(i), now.get(i)), (Some(a), Some(b)) if Arc::ptr_eq(a, b)))
        .take(limit.saturating_add(1))
        .collect();
    if differ.len() > limit {
        return false;
    }
    for i in differ {
        if let Some(g) = was.get(i) {
            remove_users(&mut kept.users, g, i);
        }
        if let Some(g) = now.get(i) {
            add_users(&mut kept.users, g, i);
        }
    }
    kept.table = table.clone();
    true
}

/// Runs `f` on the users of `font`'s glyphs: the kept index of the same table, a kept index of a
/// table of the same length brought up to date, or one built from every glyph.
fn with_users<R>(font: &Font, f: impl FnOnce(&Users) -> R) -> R {
    USERS.with(|cell| {
        let Ok(mut kept) = cell.try_borrow_mut() else { return f(&users_of(font.glyphs.as_arcs())) };
        let now = font.glyphs.as_arcs();
        let entry = if let Some(at) = kept.iter().position(|k| std::ptr::eq(k.table.as_arcs(), now)) {
            kept.remove(at)
        } else {
            let mut updated = None;
            if let Some(at) = kept.iter().position(|k| k.table.as_arcs().len() == now.len()) {
                let mut entry = kept.remove(at);
                if update_users(&mut entry, &font.glyphs) {
                    updated = Some(entry);
                }
            }
            updated.unwrap_or_else(|| UsersOf { table: font.glyphs.clone(), users: users_of(now) })
        };
        let out = f(&entry.users);
        kept.insert(0, entry);
        kept.truncate(REMEMBERED_TABLES);
        out
    })
}

/// The positions of `changed` and of every glyph that uses one of them, directly or through
/// other composites, each after all its bases. Glyphs on or above a cycle are left out.
fn affected_order(font: &Font, changed: &[String]) -> Vec<usize> {
    with_users(font, |users| order_with(font, users, changed))
}

/// [`affected_order`] with the users of `font`'s glyphs.
fn order_with(font: &Font, users: &Users, changed: &[String]) -> Vec<usize> {
    let glyphs = font.glyphs.as_arcs();
    let mut affected: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut seen_names: HashSet<&str> = HashSet::new();
    for name in changed {
        if !seen_names.insert(name.as_str()) {
            continue;
        }
        if let Some(i) = font.glyphs.position(name)
            && affected.insert(i)
        {
            queue.push_back(i);
        }
        // A removed or renamed glyph leaves users behind that must follow too.
        for &u in users.get(name.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
            if affected.insert(u) {
                queue.push_back(u);
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        let Some(g) = glyphs.get(i) else { continue };
        for &u in users.get(g.name.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
            if affected.insert(u) {
                queue.push_back(u);
            }
        }
    }
    // Kahn's order inside the affected set: a glyph is ready once its affected bases are done.
    let mut waiting: HashMap<usize, usize> = HashMap::with_capacity(affected.len());
    for &i in &affected {
        let Some(g) = glyphs.get(i) else { continue };
        let bases = component_names(g).filter_map(|n| font.glyphs.position(n)).filter(|b| affected.contains(b)).collect::<HashSet<_>>().len();
        waiting.insert(i, bases);
    }
    let mut ready: Vec<usize> = waiting.iter().filter(|(_, n)| **n == 0).map(|(i, _)| *i).collect();
    ready.sort_unstable();
    let mut ready: VecDeque<usize> = ready.into();
    let mut order = Vec::with_capacity(affected.len());
    while let Some(i) = ready.pop_front() {
        order.push(i);
        let Some(g) = glyphs.get(i) else { continue };
        for &u in users.get(g.name.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(n) = waiting.get_mut(&u)
                && *n > 0
            {
                *n -= 1;
                if *n == 0 {
                    ready.push_back(u);
                }
            }
        }
    }
    order
}

/// The distinct base names of the components on every layer of `glyph`.
fn component_names(glyph: &Glyph) -> impl Iterator<Item = &str> {
    let mut seen: HashSet<&str> = HashSet::new();
    glyph
        .layers
        .iter()
        .flat_map(|l| l.shapes.iter())
        .filter_map(|s| match s {
            Shape::Component(c) => Some(c.glyph.as_str()),
            _ => None,
        })
        .filter(move |n| seen.insert(n))
}

/// The engine hook: `new` with the glyphs changed since `old`, and their dependents,
/// re-aligned ([`changed_glyphs`], then [`realign_glyphs`]).
///
/// Only the masters whose layers the change touched are re-aligned, plus the layers that read
/// one of them through a pinned component, so an edit on one master leaves the rest of the
/// other masters as stored. A change of masters or of a font setting alignment reads re-aligns every master.
pub fn realign(old: &Font, new: Font) -> Result<Font, ComponentsError> {
    let (changed, everything) = changes(old, &new);
    if changed.is_empty() {
        return Ok(new);
    }
    let masters = if everything { None } else { changed_masters(old, &new, &changed) };
    realign_on(new, &changed, masters.as_ref())
}

/// [`realign`] with the glyphs that changed since `old` already known (in the order
/// [`changed_glyphs`] gives them), so the font is not compared glyph by glyph again.
pub(crate) fn realign_known(old: &Font, new: Font, changed: &[String]) -> Result<Font, ComponentsError> {
    if reaches_everything(old, &new) {
        return realign(old, new);
    }
    if changed.is_empty() {
        return Ok(new);
    }
    let masters = changed_masters(old, &new, changed);
    realign_on(new, changed, masters.as_ref())
}

/// The masters of the layers that differ in the changed glyphs; `None` (every master) when a
/// glyph was added, removed or renamed, its layers were added, removed or reordered, or its
/// metadata or code points changed (metrics keys, category and code points can move every
/// layer). A change to nothing alignment reads (export, kerning groups) adds no master.
fn changed_masters(old: &Font, new: &Font, changed: &[String]) -> Option<HashSet<String>> {
    let mut masters = HashSet::new();
    for name in changed {
        let (Some(was), Some(now)) = (old.glyph(name), new.glyph(name)) else { return None };
        let same_frame = was.info == now.info
            && was.unicodes == now.unicodes
            && was.layers.len() == now.layers.len()
            && was.layers.iter().zip(&now.layers).all(|(a, b)| a.id == b.id && a.master == b.master);
        if !same_frame {
            return None;
        }
        masters.extend(was.layers.iter().zip(&now.layers).filter(|(a, b)| a != b).map(|(_, b)| b.master.clone()));
    }
    Some(masters)
}
