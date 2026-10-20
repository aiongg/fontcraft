//! Full-colour layers whose shapes the colour layer list cannot name as they are: paths, several
//! shapes, and a master layer that is itself painted. glyphsLib (`builder/color_layers.py`,
//! Apache-2.0; see `NOTICE`) writes each such layer for ufo2ft as glyphs of the font, and this
//! module does the same, in both directions.
//!
//! | Model, per master | UFO |
//! |---|---|
//! | a full-colour layer of glyph `B` whose shapes are not one component | per run of consecutive shapes with the same fill (as the compiler merges them), a glyph `B.color<i>` in the default layer holding the run's shapes without their paint, and a `PaintGlyph` of it in `B`'s entry; `i` is the paint's position in the entry |
//! | a master layer marked as full colour, with one run | a `PaintGlyph` of `B` itself, first in its entry |
//! | the same with several runs | `B.color<i>` glyphs as above; `B` keeps its outline |
//!
//! The made glyphs have no code points, are not in `public.glyphOrder` and are numbered as glyphsLib
//! numbers them; a name already taken gets `.1`, `.2`… after it. A run whose shapes have a stroke,
//! an invisible fill or a shape other than a path or component keeps its layer a UFO layer.
//!
//! The writer works on a copy of the font with these layers made into one-component layers and the
//! glyphs added ([`expand`]), which [`crate::colorkeys`] then writes. The reader, after typing the
//! list, folds them back ([`fold`]): a layer of one component of `B.color<i>` at position `i` takes
//! that glyph's shapes with its fill, and a `PaintGlyph` of `B` paints `B`'s master layer. It does
//! so only where the writer gives back the same layers and the same glyphs, and only for glyphs
//! nothing else uses (components, kerning, groups, the rest of the list); the folded glyphs then
//! leave the font; a glyph the UFO's glyph order names stays. glyphsLib itself does not fold them
//! when it reads a UFO.

use std::collections::{HashMap, HashSet};

use fontcraft_model::{Color, Component, Family, Font, Glyph, Gradient, Layer, MAX_GRADIENT_STOPS, Shape, ShapeAttrs};
use fontcraft_plist::{Dict, Value};

use crate::colorkeys::{COLOR_LAYERS, DEFAULT_FILL, FULL_COLOR_NAME, MARK};
use crate::glyphkeys::synthetic_id;

/// The mark of a glyph [`expand`] made, in the copy the writer writes.
pub(crate) const GENERATED: &str = "colorLayerGlyph";
/// What a folded layer keeps of the glyph it was read from, for the writer to give it back: its
/// file name (`fileName`) and its UFO data and its layer's (`glyph`, `layer`).
const CARRY: &str = "colorLayerGlyph";
/// Most runs one full-colour layer is split into.
const MAX_RUNS: usize = 1024;
/// Deepest plist value searched for a glyph name.
const MAX_SEARCH_DEPTH: usize = 32;

/// The fill one shape paints on a full-colour layer, as the compiler paints it.
#[derive(PartialEq)]
enum Look<'a> {
    Gradient(&'a Gradient),
    Color(Color),
}

/// The fill of `s` when the list can hold it: a path or component without a stroke whose fill is
/// visible (no fill colour and no gradient is the default grey).
fn look(s: &Shape) -> Option<(Look<'_>, &ShapeAttrs)> {
    let a = match s {
        Shape::Path(p) => &p.info.attrs,
        Shape::Component(c) => &c.info.attrs,
        Shape::Image(_) | Shape::Raw(_) => return None,
    };
    if a.stroke_width.is_some() {
        return None;
    }
    let look = match (&a.gradient, a.fill_color) {
        (Some(g), _) => {
            if g.stops.len() > MAX_GRADIENT_STOPS || !g.numbers().all(f64::is_finite) {
                return None;
            }
            Look::Gradient(g)
        }
        (None, Some(c)) => {
            let [_, _, _, alpha] = match c {
                Color::Rgba(v) if v.iter().all(|x| x.is_finite()) => v,
                Color::Palette { alpha, .. } if alpha.is_finite() => [0.0, 0.0, 0.0, alpha],
                _ => return None,
            };
            if alpha <= 0.0 {
                return None;
            }
            Look::Color(c)
        }
        (None, None) => Look::Color(Color::Rgba(DEFAULT_FILL)),
    };
    Some((look, a))
}

/// A run of a full-colour layer's shapes.
enum Run {
    /// Consecutive shapes with one fill: a fill colour or gradient, or neither for the default
    /// grey. One layer glyph holds them.
    Fill(Box<ShapeAttrs>, Vec<usize>),
    /// A component without paint of a glyph with full-colour layers (`coloured`), which paints
    /// that glyph's colour layers: a `PaintColrGlyph` of its own, as glyphsLib writes it.
    Colr(usize),
}

/// The runs of `l`'s shapes; `None` when a shape cannot go to the list or there is none.
fn runs(l: &Layer, coloured: &dyn Fn(&str) -> bool) -> Option<Vec<Run>> {
    let mut out: Vec<(Option<Look<'_>>, Run)> = Vec::new();
    for (i, s) in l.shapes.iter().enumerate() {
        let (look, a) = look(s)?;
        if let Shape::Component(c) = s
            && a.is_empty()
            && coloured(&c.glyph)
        {
            out.push((None, Run::Colr(i)));
            continue;
        }
        match out.last_mut() {
            Some((Some(last), Run::Fill(_, shapes))) if *last == look => shapes.push(i),
            _ => out.push((
                Some(look),
                Run::Fill(Box::new(ShapeAttrs { fill_color: a.fill_color, gradient: a.gradient.clone(), ..Default::default() }), vec![i]),
            )),
        }
    }
    (!out.is_empty() && out.len() <= MAX_RUNS).then(|| out.into_iter().map(|(_, r)| r).collect())
}

fn no_colour(_: &str) -> bool {
    false
}

fn single_component(l: &Layer) -> bool {
    matches!(l.shapes.as_slice(), [Shape::Component(_)])
}

/// Whether `l` is a full-colour layer other than a master layer that the writer splits into
/// layer glyphs of its own: its shapes are not one component, and the list can hold them.
pub(crate) fn expands(l: &Layer) -> bool {
    !l.is_master_layer() && full_color_layer(l) && l.anchors.is_empty() && !single_component(l) && runs(l, &no_colour).is_some()
}

fn full_color_layer(l: &Layer) -> bool {
    let a = &l.info.attrs;
    a.full_color && a.color_palette.is_none() && a.coordinates.is_empty() && a.axis_rules.is_empty()
}

/// Whether `l` takes a place in its glyph's entry: a one-component colour layer the list holds.
fn list_shaped(l: &Layer) -> bool {
    let a = &l.info.attrs;
    !l.is_master_layer()
        && a.coordinates.is_empty()
        && l.anchors.is_empty()
        && single_component(l)
        && ((a.full_color && a.color_palette.is_none()) || (a.color_palette.is_some() && l.extra.get(Family::Ufo, MARK).is_some()))
}

/// Whether the master layer of `base` in `master` can be painted by `base`'s own entry: it has
/// shapes, all paths and components without paint of their own, and is not painted already.
pub(crate) fn can_paint_own(font: &Font, master: &str, base: &str) -> bool {
    font.glyph(base).and_then(|g| g.layer(master)).is_some_and(|l| {
        !l.info.attrs.full_color
            && l.info.attrs.color_palette.is_none()
            && !l.shapes.is_empty()
            && l.shapes.iter().all(|s| match s {
                Shape::Path(p) => p.info.attrs.is_empty(),
                Shape::Component(c) => c.info.attrs.is_empty(),
                Shape::Image(_) | Shape::Raw(_) => false,
            })
    })
}

fn clear_paint(mut s: Shape) -> Shape {
    match &mut s {
        Shape::Path(p) => p.info.attrs = ShapeAttrs::default(),
        Shape::Component(c) => c.info.attrs = ShapeAttrs::default(),
        Shape::Image(_) | Shape::Raw(_) => {}
    }
    s
}

fn with_paint(mut s: Shape, attrs: &ShapeAttrs) -> Shape {
    match &mut s {
        Shape::Path(p) => p.info.attrs = attrs.clone(),
        Shape::Component(c) => c.info.attrs = attrs.clone(),
        Shape::Image(_) | Shape::Raw(_) => {}
    }
    s
}

/// A layer id the glyph does not use yet, as the reader numbers the layers of an entry.
fn fresh_id(ids: &mut HashSet<String>, master: &str, i: usize) -> String {
    let mut id = synthetic_id(master, &format!("{MARK}.{i}"));
    while ids.contains(&id) {
        id.push('#');
    }
    ids.insert(id.clone());
    id
}

/// The name of the layer glyph at position `i` of `base`'s entry: `base.color<i>`, or with `.1`,
/// `.2`… after it while `taken` says the name is used. `limit` is more than the names `taken` can
/// refuse, so a name is always found.
fn layer_glyph_name(base: &str, i: usize, taken: &dyn Fn(&str) -> bool, limit: usize) -> String {
    let name = format!("{base}.color{i}");
    if !taken(&name) {
        return name;
    }
    (1..=limit).map(|k| format!("{name}.{k}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{name}.{}", limit.saturating_add(1)))
}

/// Adds to `made` the layer of glyph `name` in `master`: `shapes` without their paint, and what a
/// folded layer kept of the glyph it came from (`carry`).
fn add_layer_glyph(made: &mut Vec<Glyph>, name: &str, master: &str, width: f64, shapes: Vec<Shape>, carry: Option<&Dict>) {
    let at = match made.iter().position(|g| g.name == name) {
        Some(i) => i,
        None => {
            let mut g = Glyph::new(name);
            if let Some(c) = carry {
                g.file_name = c.get("fileName").and_then(Value::as_str).map(str::to_string);
                for (k, v) in c.get("glyph").and_then(Value::as_dict).map(|d| d.entries.as_slice()).unwrap_or_default() {
                    g.extra.push(Family::Ufo, k.as_str(), v.clone());
                }
            }
            made.push(g);
            made.len().saturating_sub(1)
        }
    };
    let mut layer = Layer::new(master, width);
    layer.shapes = shapes.into_iter().map(clear_paint).collect();
    for (k, v) in carry.and_then(|c| c.get("layer")).and_then(Value::as_dict).map(|d| d.entries.as_slice()).unwrap_or_default() {
        layer.extra.push(Family::Ufo, k.as_str(), v.clone());
    }
    if let Some(g) = made.get_mut(at) {
        g.layers.push(layer);
    }
}

/// `g` with its full-colour layers made into one-component layers the list holds, and the layer
/// glyphs they name; `None` when nothing changes. `taken` says which names other glyphs use,
/// `coloured` which glyphs have full-colour layers in a master.
pub(crate) fn expand_glyph(
    g: &Glyph,
    taken: &dyn Fn(&str) -> bool,
    coloured: &dyn Fn(&str, &str) -> bool,
    limit: usize,
) -> Option<(Glyph, Vec<Glyph>)> {
    if !g.layers.iter().any(|l| l.info.attrs.full_color) {
        return None;
    }
    let mut layers = Vec::with_capacity(g.layers.len());
    let mut made: Vec<Glyph> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut ids: HashSet<String> = g.layers.iter().map(|l| l.id.clone()).collect();
    let mut next = |master: &str| {
        let slot = index.entry(master.to_string()).or_insert(0);
        let i = *slot;
        *slot = slot.saturating_add(1);
        i
    };
    let mut changed = false;
    // The colour layers of a painted master layer go before the master's first other list layer,
    // where the reader puts the layers of one entry.
    let mut pending: Vec<Layer> = Vec::new();
    for l in &g.layers {
        let painted_master = l.is_master_layer() && full_color_layer(l);
        let split = expands(l);
        if !l.is_master_layer() && (split || list_shaped(l)) {
            let (now, later): (Vec<Layer>, Vec<Layer>) = std::mem::take(&mut pending).into_iter().partition(|p| p.master == l.master);
            layers.extend(now);
            pending = later;
        }
        let Some(runs) = (painted_master || split).then(|| runs(l, &|name| coloured(name, &l.master))).flatten() else {
            if list_shaped(l) {
                next(&l.master);
            }
            layers.push(l.clone());
            continue;
        };
        changed = true;
        // The template of the one-component layers: the layer itself, or for a master layer a
        // full-colour layer as the reader makes one.
        let mut template = if painted_master {
            let mut t = Layer::new(&l.master, l.width);
            t.info.name = Some(FULL_COLOR_NAME.to_string());
            t.info.attrs.full_color = true;
            if let Some(form) = l.extra.get(Family::Ufo, MARK) {
                t.extra.insert(Family::Ufo, MARK, form.clone());
            }
            t
        } else {
            l.clone()
        };
        let carry = template.extra.remove(Family::Ufo, CARRY);
        let carry = carry.as_ref().and_then(Value::as_dict).filter(|_| matches!(runs.as_slice(), [Run::Fill(..)]));
        if painted_master {
            let mut stripped = l.clone();
            stripped.info.attrs.full_color = false;
            stripped.extra.remove(Family::Ufo, MARK);
            stripped.shapes = stripped.shapes.into_iter().map(clear_paint).collect();
            layers.push(stripped);
        }
        if painted_master && let [Run::Fill(attrs, _)] = runs.as_slice() {
            // The master layer is painted by its own glyph. A component without a fill would
            // paint its glyph's colour layers, so the default grey is stated.
            let mut attrs = attrs.clone();
            if attrs.fill_color.is_none() && attrs.gradient.is_none() {
                attrs.fill_color = Some(Color::Rgba(DEFAULT_FILL));
            }
            let i = next(&l.master);
            let mut c = Component::new(g.name.as_str(), 0.0, 0.0);
            c.info.attrs = *attrs;
            let mut item = template;
            item.id = fresh_id(&mut ids, &l.master, i);
            item.shapes = vec![Shape::Component(c)];
            pending.push(item);
            continue;
        }
        for (k, run) in runs.iter().enumerate() {
            let i = next(&l.master);
            let shape = match run {
                Run::Fill(attrs, shapes) => {
                    let name = layer_glyph_name(&g.name, i, taken, limit);
                    let run: Vec<Shape> = shapes.iter().filter_map(|&s| l.shapes.get(s)).cloned().collect();
                    add_layer_glyph(&mut made, &name, &l.master, l.width, run, carry);
                    let mut c = Component::new(name.as_str(), 0.0, 0.0);
                    c.info.attrs = (**attrs).clone();
                    Shape::Component(c)
                }
                Run::Colr(s) => {
                    let Some(shape) = l.shapes.get(*s) else { continue };
                    shape.clone()
                }
            };
            let mut item = template.clone();
            if k > 0 || painted_master {
                item.id = fresh_id(&mut ids, &l.master, i);
            }
            item.shapes = vec![shape];
            if painted_master {
                pending.push(item);
            } else {
                layers.push(item);
            }
        }
    }
    layers.extend(pending);
    let mut out = g.clone();
    out.layers = layers;
    changed.then_some((out, made))
}

/// `font` as the writer writes it: [`expand_glyph`] on every glyph, the layer glyphs added after
/// the font's glyphs with [`GENERATED`]. `None` when nothing changes.
pub(crate) fn expand(font: &Font) -> Option<Font> {
    if !font.glyphs.iter().any(|g| g.layers.iter().any(|l| l.info.attrs.full_color)) {
        return None;
    }
    let mut out = font.clone();
    let mut names: HashSet<String> = HashSet::new();
    let mut any = false;
    for (at, g) in font.glyphs.iter().enumerate() {
        let taken = |n: &str| font.glyph(n).is_some() || names.contains(n);
        let limit = font.glyphs.len().saturating_add(names.len()).saturating_add(1);
        let Some((new, made)) = expand_glyph(g, &taken, &|name, master| has_full_color(font, name, master), limit) else { continue };
        any = true;
        if let Some(mut slot) = out.glyphs.get_mut_at(at) {
            *slot = new;
        }
        for mut m in made {
            if !names.insert(m.name.clone()) {
                continue;
            }
            m.extra.insert(Family::Ufo, GENERATED, Value::Bool(true));
            out.glyphs.push(m);
        }
    }
    any.then_some(out)
}

/// Whether glyph `name` has a full-colour layer in `master`, as [`crate::colorkeys`] decides a
/// `PaintColrGlyph`.
fn has_full_color(font: &Font, name: &str, master: &str) -> bool {
    font.glyph(name).is_some_and(|g| full_color_in(g, master))
}

fn full_color_in(g: &Glyph, master: &str) -> bool {
    g.layers.iter().any(|l| l.master == master && l.info.attrs.full_color && l.info.attrs.coordinates.is_empty())
}

pub(crate) fn is_generated(g: &Glyph) -> bool {
    g.extra.get(Family::Ufo, GENERATED).is_some()
}

/// Folds the layer glyphs of the colour layers just read from the list back into those layers,
/// glyph by glyph, where the writer gives back the same ([`expand_glyph`]).
pub(crate) fn fold(font: &mut Font) {
    let read_from_list = |l: &Layer| !l.is_master_layer() && l.info.attrs.full_color && l.extra.get(Family::Ufo, MARK).is_some();
    let bases: Vec<String> = font.glyphs.iter().filter(|g| g.layers.iter().any(read_from_list)).map(|g| g.name.clone()).collect();
    if bases.is_empty() {
        return;
    }
    let mut uses = Uses::new(font);
    for base in bases {
        fold_glyph(font, &mut uses, &base);
    }
    if !uses.gone.is_empty() {
        font.glyphs.retain(|g| !uses.gone.contains(&g.name));
    }
}

/// What [`fold_layers`] gives: the glyph with folded layers, the layer glyphs it took in and the
/// masters each was taken in, the masters whose master layer it painted, and the masters with a
/// list layer of the glyph itself.
struct Folded {
    glyph: Glyph,
    used: Vec<(String, Vec<String>)>,
    own: Vec<String>,
    selfs: Vec<String>,
}

#[cfg(test)]
thread_local! {
    /// Glyphs [`Uses`] has counted the components of, for the test that the fold stays linear.
    pub(crate) static COUNTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Who names each glyph, gathered once for the whole fold and kept up to date as glyphs fold, so
/// that deciding whether a layer glyph can leave the font is a lookup, not a pass over the font.
struct Uses {
    /// How many components in layers and their backgrounds name each glyph.
    components: HashMap<String, usize>,
    kerned: HashSet<String>,
    grouped: HashSet<String>,
    /// What the masters' colour layer lists mention.
    listed: Mentions,
    /// What the UFOs' glyph orders mention.
    ordered: Mentions,
    /// Layer glyphs folded so far. They count as gone at once and leave the font when the fold
    /// ends, so that the font is filtered once.
    gone: HashSet<String>,
    /// How many glyphs of the font the gone names cover.
    gone_glyphs: usize,
    /// The positions of the second and later glyphs that share a name.
    repeats: HashMap<String, Vec<usize>>,
}

impl Uses {
    fn new(font: &Font) -> Self {
        let mut uses = Uses {
            components: HashMap::new(),
            kerned: HashSet::new(),
            grouped: HashSet::new(),
            listed: Mentions::default(),
            ordered: Mentions::default(),
            gone: HashSet::new(),
            gone_glyphs: 0,
            repeats: HashMap::new(),
        };
        for (i, g) in font.glyphs.iter().enumerate() {
            uses.count(g, true);
            if font.glyphs.position(&g.name) != Some(i) {
                uses.repeats.entry(g.name.clone()).or_default().push(i);
            }
        }
        for table in [&font.kerning.ltr, &font.kerning.rtl, &font.kerning.vertical] {
            for f in table.0.iter().flat_map(|m| m.firsts.iter()) {
                uses.kerned.insert(f.first.clone());
                uses.kerned.extend(f.seconds.iter().map(|(s, _)| s.clone()));
            }
        }
        uses.grouped = font.groups.iter().flat_map(|g| g.members.iter().cloned()).collect();
        for m in font.masters.iter() {
            if let Some(v) = m.user_data.get(COLOR_LAYERS) {
                uses.listed.tally(v, 0, true);
            }
            // A glyph the UFO's glyph order names is one of the font's own: glyphsLib leaves its
            // layer glyphs out of the order.
            if let Some(v) = crate::ufo::kept_dict(m, "lib.plist").0.get("public.glyphOrder") {
                uses.ordered.tally(v, 0, true);
            }
        }
        uses
    }

    /// The glyph called `name` unless it has folded away.
    fn live<'a>(&self, font: &'a Font, name: &str) -> Option<&'a Glyph> {
        if self.gone.contains(name) { None } else { font.glyph(name).map(|g| &**g) }
    }

    fn live_glyphs(&self, font: &Font) -> usize {
        font.glyphs.len().saturating_sub(self.gone_glyphs)
    }

    /// Adds (or takes away) the components of `g`.
    fn count(&mut self, g: &Glyph, add: bool) {
        #[cfg(test)]
        COUNTED.with(|c| c.set(c.get().saturating_add(1)));
        for l in &g.layers {
            let background = l.info.background.as_ref().map(|b| b.shapes.as_slice()).unwrap_or_default();
            for s in l.shapes.iter().chain(background) {
                let Shape::Component(c) = s else { continue };
                match self.components.get_mut(&c.glyph) {
                    Some(n) => *n = if add { n.saturating_add(1) } else { n.saturating_sub(1) },
                    None if add => {
                        self.components.insert(c.glyph.clone(), 1);
                    }
                    None => {}
                }
            }
        }
    }

    /// Marks the glyphs called `name` as gone.
    fn go(&mut self, font: &Font, name: &str) {
        if !self.gone.insert(name.to_string()) {
            return;
        }
        let first = font.glyphs.position(name);
        let repeats = self.repeats.get(name).cloned().unwrap_or_default();
        for i in first.into_iter().chain(repeats) {
            if let Some(g) = font.glyphs.as_arcs().get(i).cloned() {
                self.count(&g, false);
                self.gone_glyphs = self.gone_glyphs.saturating_add(1);
            }
        }
    }

    /// [`crate::colorkeys::unread`], keeping the counts of `base`'s components and of what the
    /// colour layer lists mention.
    fn unread(&mut self, font: &mut Font, master: &str, base: &str) {
        let before = font.glyph(base).cloned();
        let whole = !matches!(list_of(font, master), Some(Value::Dict(_)));
        let part = |font: &Font| match list_of(font, master) {
            Some(Value::Dict(d)) if !whole => {
                Some(Value::Dict(Dict { entries: d.entries.iter().filter(|(k, _)| k.as_str() == base).cloned().collect(), ..Default::default() }))
            }
            other => other.cloned(),
        };
        if let Some(v) = part(font) {
            self.listed.tally(&v, 0, false);
        }
        crate::colorkeys::unread(font, master, base);
        if let Some(v) = part(font) {
            self.listed.tally(&v, 0, true);
        }
        if let Some(g) = before {
            self.count(&g, false);
        }
        if let Some(g) = font.glyph(base).cloned() {
            self.count(&g, true);
        }
    }
}

/// The colour layer list of the first master with the id `master`, which
/// [`crate::colorkeys::unread`] changes.
fn list_of<'a>(font: &'a Font, master: &str) -> Option<&'a Value> {
    font.masters.iter().find(|m| m.id == master).and_then(|m| m.user_data.get(COLOR_LAYERS))
}

/// How often each string and dictionary key of some plist values appears, searched at most
/// [`MAX_SEARCH_DEPTH`] deep; a value nested deeper counts as mentioning every name.
#[derive(Default)]
struct Mentions {
    names: HashMap<String, usize>,
    too_deep: usize,
}

impl Mentions {
    fn has(&self, name: &str) -> bool {
        self.too_deep > 0 || self.names.get(name).is_some_and(|n| *n > 0)
    }

    fn tally(&mut self, v: &Value, depth: usize, add: bool) {
        let step = |n: &mut usize| *n = if add { n.saturating_add(1) } else { n.saturating_sub(1) };
        if depth > MAX_SEARCH_DEPTH {
            step(&mut self.too_deep);
            return;
        }
        let name = |s: &str, names: &mut HashMap<String, usize>| match names.get_mut(s) {
            Some(n) => step(n),
            None if add => {
                names.insert(s.to_string(), 1);
            }
            None => {}
        };
        match v {
            Value::String(s) => name(s.as_str(), &mut self.names),
            Value::Array(a) => {
                for v in &a.items {
                    self.tally(v, depth + 1, add);
                }
            }
            Value::Dict(d) => {
                for (k, v) in &d.entries {
                    name(k.as_str(), &mut self.names);
                    self.tally(v, depth + 1, add);
                }
            }
            _ => {}
        }
    }
}

fn fold_glyph(font: &mut Font, uses: &mut Uses, base: &str) {
    let Some(read) = uses.live(font, base).cloned() else { return };
    let all = fold_layers(font, uses, &read, true);
    let mut exclude: HashSet<String> =
        all.used.iter().filter(|(name, masters)| !removable(font, uses, base, name, masters)).map(|(n, _)| n.clone()).collect();
    // Each try that fails leaves out at least one more layer glyph.
    for _ in 0..=all.used.len() {
        let f = fold_layers_without(font, uses, &read, &exclude);
        // A layer of a component of the glyph itself stays only folded into its master layer.
        if f.own.len() != f.selfs.len() || (f.used.is_empty() && f.own.is_empty()) {
            break;
        }
        match comes_back(font, uses, &read, &f) {
            Ok(()) => {
                if let Ok(mut g) = font.glyph_mut(base) {
                    uses.count(&read, false);
                    uses.count(&f.glyph, true);
                    *g = f.glyph;
                }
                for (name, _) in &f.used {
                    uses.go(font, name);
                }
                return;
            }
            Err(differ) => {
                let before = exclude.len();
                exclude.extend(differ);
                if exclude.len() == before {
                    break;
                }
            }
        }
    }
    // A layer painting the glyph's own outline cannot stay a component of the glyph itself.
    for master in all.selfs {
        uses.unread(font, &master, base);
    }
}

fn fold_layers_without(font: &Font, uses: &Uses, read: &Glyph, bad: &HashSet<String>) -> Folded {
    let mut f = fold_layers_filtered(font, uses, read, &|name| !bad.contains(name));
    f.used.retain(|(n, _)| !bad.contains(n));
    f
}

fn fold_layers(font: &Font, uses: &Uses, read: &Glyph, with_glyphs: bool) -> Folded {
    fold_layers_filtered(font, uses, read, &|_| with_glyphs)
}

/// `read` with its list layers folded: one-component layers of `read.color<i>` glyphs that `fold`
/// allows take the glyph's shapes, and a layer of the glyph itself paints its master layer.
fn fold_layers_filtered(font: &Font, uses: &Uses, read: &Glyph, fold: &dyn Fn(&str) -> bool) -> Folded {
    let mut glyph = read.clone();
    let mut used: Vec<(String, Vec<String>)> = Vec::new();
    let mut own = Vec::new();
    let mut selfs = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut drop_ids = Vec::new();
    let mut painted: Vec<(String, ShapeAttrs, Option<Value>)> = Vec::new();
    for l in glyph.layers.iter_mut() {
        if !list_shaped(l) {
            continue;
        }
        let slot = index.entry(l.master.clone()).or_insert(0);
        let i = *slot;
        *slot = slot.saturating_add(1);
        if !l.info.attrs.full_color || l.extra.get(Family::Ufo, MARK).is_none() {
            continue;
        }
        let Some(Shape::Component(c)) = l.shapes.first() else { continue };
        let plain_fill = c.transform.is_identity() && c.extra.is_empty() && (c.info.attrs.fill_color.is_some() || c.info.attrs.gradient.is_some());
        if c.glyph == read.name {
            selfs.push(l.master.clone());
            if plain_fill && i == 0 && can_paint_own(font, &l.master, &read.name) {
                painted.push((l.master.clone(), c.info.attrs.clone(), l.extra.get(Family::Ufo, MARK).cloned()));
                drop_ids.push(l.id.clone());
                own.push(l.master.clone());
            }
            continue;
        }
        if !plain_fill || c.glyph != format!("{}.color{i}", read.name) || !fold(&c.glyph) {
            continue;
        }
        let Some(source) = uses.live(font, &c.glyph) else { continue };
        let Some(sl) = source.layer(&l.master) else { continue };
        let plain = |s: &Shape| match s {
            Shape::Path(p) => p.info.attrs.is_empty(),
            Shape::Component(c) => c.info.attrs.is_empty(),
            Shape::Image(_) | Shape::Raw(_) => false,
        };
        let ufo_only = |x: &fontcraft_model::Extra| x.iter().all(|e| e.family == Family::Ufo);
        if sl.shapes.is_empty() || !sl.shapes.iter().all(plain) || !ufo_only(&source.extra) || !ufo_only(&sl.extra) {
            continue;
        }
        let attrs = c.info.attrs.clone();
        let name = c.glyph.clone();
        let mut carry = Dict::new();
        if let Some(f) = &source.file_name {
            carry.insert("fileName", Value::String(f.as_str().into()));
        }
        let as_dict = |x: &fontcraft_model::Extra| {
            Value::Dict(Dict { entries: x.iter().map(|e| (e.key.clone(), e.value.clone())).collect(), ..Default::default() })
        };
        carry.insert("glyph", as_dict(&source.extra));
        carry.insert("layer", as_dict(&sl.extra));
        l.shapes = sl.shapes.iter().cloned().map(|s| with_paint(s, &attrs)).collect();
        l.extra.insert(Family::Ufo, CARRY, Value::Dict(carry));
        match used.iter_mut().find(|(n, _)| *n == name) {
            Some((_, masters)) => masters.push(l.master.clone()),
            None => used.push((name, vec![l.master.clone()])),
        }
    }
    glyph.layers.retain(|l| !drop_ids.contains(&l.id));
    for (master, attrs, form) in painted {
        if let Some(m) = glyph.layers.iter_mut().find(|l| l.is_master_layer() && l.master == master) {
            m.info.attrs.full_color = true;
            m.shapes = std::mem::take(&mut m.shapes).into_iter().map(|s| with_paint(s, &attrs)).collect();
            if let Some(form) = form {
                m.extra.insert(Family::Ufo, MARK, form);
            }
        }
    }
    Folded { glyph, used, own, selfs }
}

/// Whether the layer glyph `name`, taken in `masters` by `base`'s layers, can leave the font: it
/// has a master layer in exactly those masters and nothing else names it.
fn removable(font: &Font, uses: &Uses, base: &str, name: &str, masters: &[String]) -> bool {
    let Some(g) = uses.live(font, name) else { return false };
    if g.layers.len() != masters.len() || !g.layers.iter().all(|l| l.is_master_layer() && masters.contains(&l.master)) {
        return false;
    }
    // Every component of it is one of the folded layers of `base`.
    if uses.components.get(name).copied().unwrap_or(0) != masters.len() || uses.live(font, base).is_none() {
        return false;
    }
    !uses.kerned.contains(name) && !uses.grouped.contains(name) && !uses.listed.has(name) && !uses.ordered.has(name)
}

/// Whether the writer gives back `read` and the layer glyphs `f` took in from the folded glyph;
/// otherwise the layer glyphs to leave out on the next try (all of them when the glyph itself
/// differs).
fn comes_back(font: &Font, uses: &Uses, read: &Glyph, f: &Folded) -> Result<(), Vec<String>> {
    let used: HashSet<&str> = f.used.iter().map(|(n, _)| n.as_str()).collect();
    let all = || f.used.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    let taken = |n: &str| uses.live(font, n).is_some() && !used.contains(n);
    let coloured = |name: &str, master: &str| {
        if name == read.name {
            f.glyph.layers.iter().any(|l| l.master == master && l.info.attrs.full_color)
        } else {
            uses.live(font, name).is_some_and(|g| full_color_in(g, master))
        }
    };
    let limit = uses.live_glyphs(font).saturating_add(1);
    let Some((glyph, made)) = expand_glyph(&f.glyph, &taken, &coloured, limit) else { return Err(all()) };
    if glyph != *read || made.len() != used.len() || !made.iter().all(|m| used.contains(m.name.as_str())) {
        return Err(all());
    }
    let differ: Vec<String> = made.iter().filter(|m| uses.live(font, &m.name).is_none_or(|o| *o != **m)).map(|m| m.name.clone()).collect();
    if differ.is_empty() { Ok(()) } else { Err(differ) }
}

/// What the list leaves out of the full-colour layers the writer splits into layer glyphs or
/// paints by their own glyph, one line per kind with the number of shapes or layers.
pub(crate) fn losses(font: &Font) -> Vec<String> {
    let (mut attrs, mut layers) = (0usize, 0usize);
    for g in font.glyphs.iter() {
        for l in &g.layers {
            let painted_master = l.is_master_layer() && full_color_layer(l) && runs(l, &no_colour).is_some();
            let listed = !l.is_master_layer() && full_color_layer(l) && l.anchors.is_empty() && runs(l, &no_colour).is_some();
            if !(painted_master || listed) {
                continue;
            }
            if painted_master || expands(l) {
                attrs += l
                    .shapes
                    .iter()
                    .filter_map(|s| look(s).map(|(_, a)| a))
                    .filter(|a| **a != ShapeAttrs { fill_color: a.fill_color, gradient: a.gradient.clone(), ..Default::default() })
                    .count();
            }
            if listed {
                let mut plain = Layer::new(&l.master, g.layer(&l.master).map_or(l.width, |m| m.width));
                plain.info.name = Some(FULL_COLOR_NAME.to_string());
                plain.info.attrs.full_color = true;
                let mut info = l.info.clone();
                if info.name.is_none() {
                    info.name = plain.info.name.clone();
                }
                let kept = |e: &fontcraft_model::ExtraEntry| e.family == Family::Ufo && (e.key.as_str() == MARK || e.key.as_str() == CARRY);
                layers += usize::from(info != plain.info || l.width != plain.width || !l.extra.iter().all(kept));
            }
        }
    }
    let mut out = Vec::new();
    if attrs > 0 {
        out.push(format!(
            "the paint attributes of shapes on full-colour layers other than a fill colour or gradient, which the colour layer list does not hold ({attrs})"
        ));
    }
    if layers > 0 {
        out.push(format!("the names, widths and other layer data of full-colour layers in the colour layer list ({layers})"));
    }
    out
}

#[cfg(test)]
#[path = "colorglyphs_tests.rs"]
mod tests;
