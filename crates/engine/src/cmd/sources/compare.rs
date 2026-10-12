//! The comparison behind `source.diff` and `font.compareGlyphs`: the changes that turn font A
//! into font B, part by part, each with a stable id and the place `source.merge` applies it at.
//!
//! Records are matched as the inventory rows say: glyphs by name (then renames, by unchanged
//! outlines and code points), masters and instances by axis location and then by name, axes by
//! tag, Font Info entries, feature snippets and groups by key. Values are compared in the native
//! file's JSON form, so every field the native format keeps takes part.
//!
//! An id is a hash of the change's place and of both values. `source.merge` compares the sources
//! again and looks the ids up, so a change whose place in BASE no longer holds the value the
//! first comparison saw has a different id and is refused.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use fontcraft_geom::PathEl;
use fontcraft_model::{Font, Glyph, KernTable, Kerning, Layer, Shape};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::MAX_SOURCE_CHANGES;
use crate::{EngineError, Result};

/// Deepest JSON nesting compared number by number; deeper values are compared exactly.
const MAX_COMPARE_DEPTH: usize = 64;

/// The parts of a source, in the order results list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Part {
    Glyphs,
    Info,
    Masters,
    Instances,
    Axes,
    Kerning,
    Features,
}

impl Part {
    pub(super) const ALL: [Part; 7] = [Part::Glyphs, Part::Info, Part::Masters, Part::Instances, Part::Axes, Part::Kerning, Part::Features];

    pub(super) fn name(self) -> &'static str {
        match self {
            Part::Glyphs => "glyphs",
            Part::Info => "info",
            Part::Masters => "masters",
            Part::Instances => "instances",
            Part::Axes => "axes",
            Part::Kerning => "kerning",
            Part::Features => "features",
        }
    }

    pub(super) fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Added,
    Removed,
    Renamed,
    Changed,
}

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Kind::Added => "added",
            Kind::Removed => "removed",
            Kind::Renamed => "renamed",
            Kind::Changed => "changed",
        }
    }
}

/// The lists whose items are matched between A and B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum List {
    Axes,
    Masters,
    Instances,
    Properties,
    Params,
    Prefixes,
    Classes,
    Features,
    Groups,
    Glyphs,
}

impl List {
    fn part(self) -> Part {
        match self {
            List::Axes => Part::Axes,
            List::Masters => Part::Masters,
            List::Instances => Part::Instances,
            List::Properties | List::Params => Part::Info,
            List::Prefixes | List::Classes | List::Features | List::Groups => Part::Features,
            List::Glyphs => Part::Glyphs,
        }
    }

    /// What a change's `field` starts with for an item of this list.
    fn prefix(self) -> &'static str {
        match self {
            List::Properties => "properties/",
            List::Params => "params/",
            List::Prefixes => "prefixes/",
            List::Classes => "classes/",
            List::Features => "features/",
            List::Groups => "groups/",
            List::Axes | List::Masters | List::Instances | List::Glyphs => "",
        }
    }
}

/// A kerning direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dir {
    Ltr,
    Rtl,
    Vertical,
}

impl Dir {
    pub(super) const ALL: [Dir; 3] = [Dir::Ltr, Dir::Rtl, Dir::Vertical];

    pub(super) fn name(self) -> &'static str {
        match self {
            Dir::Ltr => "ltr",
            Dir::Rtl => "rtl",
            Dir::Vertical => "vertical",
        }
    }

    pub(super) fn table(self, k: &Kerning) -> &KernTable {
        match self {
            Dir::Ltr => &k.ltr,
            Dir::Rtl => &k.rtl,
            Dir::Vertical => &k.vertical,
        }
    }

    pub(super) fn table_mut(self, k: &mut Kerning) -> &mut KernTable {
        match self {
            Dir::Ltr => &mut k.ltr,
            Dir::Rtl => &mut k.rtl,
            Dir::Vertical => &mut k.vertical,
        }
    }
}

/// Where a change applies, with the indices into A and B that `source.merge` needs.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Place {
    /// A whole item added, removed or replaced (`field: None`), or one field of a matched pair.
    Record {
        list: List,
        a: Option<usize>,
        b: Option<usize>,
        field: Option<String>,
    },
    /// The order of a list.
    Order(List),
    /// A font-wide field of the info part (`familyName`, `settings`…).
    Info(&'static str),
    /// A field of a glyph pair: `name`, `unicodes`, `export`, `info`, `extra`, `otherLayers`,
    /// `kernGroups`.
    GlyphField {
        a: usize,
        b: usize,
        field: &'static str,
    },
    /// A master layer of a glyph pair, whole (`field: None`) or one field. `master_a` is the
    /// master's id in A (or the id a master added from B gets), `master_b` its id in B.
    Layer {
        a: usize,
        b: usize,
        master_a: String,
        master_b: Option<String>,
        field: Option<&'static str>,
    },
    /// One kerning pair; `master` is the id in A's terms.
    KernPair {
        dir: Dir,
        master: String,
        first: String,
        second: String,
    },
    KernContext,
}

/// One difference from A to B.
#[derive(Clone, Debug)]
pub(super) struct Change {
    pub id: String,
    pub part: Part,
    pub kind: Kind,
    pub glyph: Option<String>,
    pub master: Option<String>,
    pub field: Option<String>,
    pub a: Option<Value>,
    pub b: Option<Value>,
    pub place: Place,
}

impl Change {
    pub(super) fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("part".into(), json!(self.part.name()));
        m.insert("kind".into(), json!(self.kind.name()));
        for (key, value) in [("glyph", &self.glyph), ("master", &self.master), ("field", &self.field)] {
            if let Some(v) = value {
                m.insert(key.into(), json!(v));
            }
        }
        for (key, value) in [("a", &self.a), ("b", &self.b)] {
            if let Some(v) = value {
                m.insert(key.into(), v.clone());
            }
        }
        Value::Object(m)
    }
}

/// What to compare.
#[derive(Clone, Debug)]
pub(super) struct Options {
    pub parts: BTreeSet<Part>,
    /// Only these glyphs (by their name in A or in B); `None`: all.
    pub glyphs: Option<BTreeSet<String>>,
    /// Coordinates closer than this are equal (glyph widths, outlines, components, anchors).
    pub tolerance: f64,
}

/// Matched items: `(index in A, index in B)`. Items of A come first in A's order, then the items
/// only B has in B's order, which is the order a merge leaves them in before reordering.
pub(super) type Pairs = Vec<(Option<usize>, Option<usize>)>;

/// Masters matched by location, then by name, and the id each master of B has in A.
#[derive(Clone, Debug, Default)]
pub(super) struct MasterMap {
    pub pairs: Pairs,
    to_a: HashMap<String, String>,
    names: HashMap<String, String>,
}

impl MasterMap {
    pub(super) fn new(a: &Font, b: &Font) -> Self {
        let pa: Vec<(&[f64], &str)> = a.masters.iter().map(|m| (m.location.as_slice(), m.name.as_str())).collect();
        let pb: Vec<(&[f64], &str)> = b.masters.iter().map(|m| (m.location.as_slice(), m.name.as_str())).collect();
        let pairs = pair_by_place(&pa, &pb);
        let mut to_a = HashMap::new();
        let mut names: HashMap<String, String> = a.masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
        let mut taken: BTreeSet<String> = a.masters.iter().map(|m| m.id.clone()).collect();
        for &(ia, ib) in &pairs {
            let Some(mb) = ib.and_then(|j| b.masters.get(j)) else { continue };
            match ia.and_then(|i| a.masters.get(i)) {
                Some(ma) => {
                    to_a.insert(mb.id.clone(), ma.id.clone());
                }
                None => {
                    // A master only B has keeps its id unless A uses that id for another master.
                    let mut id = mb.id.clone();
                    let mut n = 2;
                    while taken.contains(&id) {
                        id = format!("{}-{n}", mb.id);
                        n += 1;
                    }
                    taken.insert(id.clone());
                    names.insert(id.clone(), mb.name.clone());
                    to_a.insert(mb.id.clone(), id);
                }
            }
        }
        MasterMap { pairs, to_a, names }
    }

    /// The id B's master `b_id` has in A, or gets when it is added.
    pub(super) fn a_id(&self, b_id: &str) -> String {
        self.to_a.get(b_id).cloned().unwrap_or_else(|| b_id.to_string())
    }

    /// The master's name, from its id in A's terms.
    fn label(&self, a_id: &str) -> String {
        self.names.get(a_id).cloned().unwrap_or_else(|| a_id.to_string())
    }

    /// For each master pair: the id in A's terms and the id in B (when B has it).
    fn ids(&self, a: &Font, b: &Font) -> Vec<(String, Option<String>)> {
        self.pairs
            .iter()
            .filter_map(|&(ia, ib)| {
                let b_id = ib.and_then(|j| b.masters.get(j)).map(|m| m.id.clone());
                let a_id = match ia.and_then(|i| a.masters.get(i)) {
                    Some(m) => m.id.clone(),
                    None => self.a_id(b_id.as_deref()?),
                };
                Some((a_id, b_id))
            })
            .collect()
    }
}

/// The result of a comparison.
#[derive(Debug)]
pub(super) struct Diff {
    pub changes: Vec<Change>,
    pub masters: MasterMap,
    pub pairs: HashMap<List, Pairs>,
}

/// The changes from A to B in the parts and glyphs `opts` names.
pub(super) fn diff(a: &Font, b: &Font, opts: &Options) -> Result<Diff> {
    let masters = MasterMap::new(a, b);
    let mut out = Builder::default();
    let mut pairs = HashMap::new();
    let want = |p: Part| opts.parts.contains(&p);
    if want(Part::Glyphs) || want(Part::Kerning) {
        let p = pair_glyphs(a, b, &masters, true)?;
        glyphs(&mut out, a, b, &masters, &p, opts)?;
        pairs.insert(List::Glyphs, p);
    }
    if want(Part::Info) {
        info(&mut out, a, b, &mut pairs)?;
    }
    if want(Part::Masters) {
        let labels_a: Vec<String> = a.masters.iter().map(|m| m.name.clone()).collect();
        let labels_b: Vec<String> = b.masters.iter().map(|m| m.name.clone()).collect();
        let sides = Sides { a: &a.masters, b: &b.masters, labels_a: &labels_a, labels_b: &labels_b };
        records(&mut out, List::Masters, &sides, &masters.pairs, Some(&["id"]))?;
        pairs.insert(List::Masters, masters.pairs.clone());
    }
    if want(Part::Instances) {
        let pa: Vec<(&[f64], &str)> = a.instances.iter().map(|m| (m.location.as_slice(), m.name.as_str())).collect();
        let pb: Vec<(&[f64], &str)> = b.instances.iter().map(|m| (m.location.as_slice(), m.name.as_str())).collect();
        let p = pair_by_place(&pa, &pb);
        let labels_a = keyed(a.instances.iter().map(|i| i.name.as_str()));
        let labels_b = keyed(b.instances.iter().map(|i| i.name.as_str()));
        let sides = Sides { a: &a.instances, b: &b.instances, labels_a: &labels_a, labels_b: &labels_b };
        records(&mut out, List::Instances, &sides, &p, Some(&[]))?;
        pairs.insert(List::Instances, p);
    }
    if want(Part::Axes) {
        let labels_a = keyed(a.axes.iter().map(|x| x.tag.as_str()));
        let labels_b = keyed(b.axes.iter().map(|x| x.tag.as_str()));
        let p = pair_by_key(&labels_a, &labels_b);
        let sides = Sides { a: &a.axes, b: &b.axes, labels_a: &labels_a, labels_b: &labels_b };
        records(&mut out, List::Axes, &sides, &p, Some(&[]))?;
        pairs.insert(List::Axes, p);
    }
    if want(Part::Kerning) {
        kerning(&mut out, a, b, &masters)?;
    }
    if want(Part::Features) {
        features(&mut out, a, b, &mut pairs)?;
    }
    Ok(Diff { changes: out.changes, masters, pairs })
}

/// FNV-1a, 64 bits: stable across runs and platforms, unlike the standard library's hasher.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Where a change is, as people read it.
#[derive(Default)]
struct Spot {
    glyph: Option<String>,
    master: Option<String>,
    field: Option<String>,
}

#[derive(Default)]
struct Builder {
    changes: Vec<Change>,
    seen: HashMap<String, u32>,
}

impl Builder {
    fn push(&mut self, part: Part, kind: Kind, place: Place, spot: Spot, a: Option<Value>, b: Option<Value>) -> Result<()> {
        if self.changes.len() >= MAX_SOURCE_CHANGES {
            return Err(EngineError::Interaction(format!(
                "the sources differ in more than {MAX_SOURCE_CHANGES} places: compare fewer parts or glyphs"
            )));
        }
        let mut h = Fnv::new();
        for s in
            [part.name(), kind.name(), spot.glyph.as_deref().unwrap_or(""), spot.master.as_deref().unwrap_or(""), spot.field.as_deref().unwrap_or("")]
        {
            h.write(s.as_bytes());
            h.write(&[0]);
        }
        for v in [&a, &b] {
            h.write(v.as_ref().map(Value::to_string).unwrap_or_default().as_bytes());
            h.write(&[0]);
        }
        let base = format!("{}:{:012x}", part.name(), h.0 & 0xffff_ffff_ffff);
        let n = self.seen.entry(base.clone()).or_insert(0);
        *n += 1;
        let id = if *n == 1 { base } else { format!("{base}-{n}") };
        self.changes.push(Change { id, part, kind, glyph: spot.glyph, master: spot.master, field: spot.field, a, b, place });
        Ok(())
    }
}

pub(super) fn to_json<T: Serialize + ?Sized>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| EngineError::Interaction(format!("cannot compare a value: {e}")))
}

/// Equal JSON values, with numbers within `tol` of each other counting as equal.
pub(super) fn same(a: &Value, b: &Value, tol: f64) -> bool {
    same_at(a, b, tol, 0)
}

fn same_at(a: &Value, b: &Value, tol: f64, depth: usize) -> bool {
    if tol <= 0.0 || depth > MAX_COMPARE_DEPTH {
        return a == b;
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= tol,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same_at(x, y, tol, depth + 1)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same_at(v, w, tol, depth + 1))),
        _ => a == b,
    }
}

/// Labels for items keyed by `keys`, with `#2`, `#3`… on repeats so each is unique.
fn keyed<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    keys.map(|k| {
        let n = seen.entry(k).or_insert(0);
        *n += 1;
        if *n == 1 { k.to_string() } else { format!("{k}#{n}") }
    })
    .collect()
}

/// Items with equal keys matched in order.
fn pair_by_key(ka: &[String], kb: &[String]) -> Pairs {
    let mut free: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (j, k) in kb.iter().enumerate() {
        free.entry(k.as_str()).or_default().push_back(j);
    }
    let mut used = vec![false; kb.len()];
    let mut pairs = Vec::with_capacity(ka.len().max(kb.len()));
    for (i, k) in ka.iter().enumerate() {
        let j = free.get_mut(k.as_str()).and_then(VecDeque::pop_front);
        if let Some(u) = j.and_then(|j| used.get_mut(j)) {
            *u = true;
        }
        pairs.push((Some(i), j));
    }
    pairs.extend(used.iter().enumerate().filter(|(_, u)| !**u).map(|(j, _)| (None, Some(j))));
    pairs
}

fn location_key(loc: &[f64]) -> String {
    // -0.0 and 0.0 are the same place.
    loc.iter().map(|v| if *v == 0.0 { 0u64 } else { v.to_bits() }.to_string()).collect::<Vec<_>>().join(",")
}

/// Items matched by location and name, then by location, then by name.
fn pair_by_place(a: &[(&[f64], &str)], b: &[(&[f64], &str)]) -> Pairs {
    let mut matched: Vec<Option<usize>> = vec![None; a.len()];
    let mut used = vec![false; b.len()];
    for pass in 0..3 {
        let key = |loc: &[f64], name: &str| match pass {
            0 => format!("{}|{name}", location_key(loc)),
            1 => location_key(loc),
            _ => format!("|{name}"),
        };
        let mut free: HashMap<String, VecDeque<usize>> = HashMap::new();
        for (j, (loc, name)) in b.iter().enumerate() {
            if !used.get(j).copied().unwrap_or(true) {
                free.entry(key(loc, name)).or_default().push_back(j);
            }
        }
        for (i, (loc, name)) in a.iter().enumerate() {
            if !matches!(matched.get(i), Some(None)) {
                continue;
            }
            let Some(queue) = free.get_mut(&key(loc, name)) else { continue };
            while let Some(j) = queue.pop_front() {
                if let Some(u) = used.get_mut(j).filter(|u| !**u) {
                    *u = true;
                    if let Some(m) = matched.get_mut(i) {
                        *m = Some(j);
                    }
                    break;
                }
            }
        }
    }
    let mut pairs: Pairs = matched.into_iter().enumerate().map(|(i, j)| (Some(i), j)).collect();
    pairs.extend(used.iter().enumerate().filter(|(_, u)| !**u).map(|(j, _)| (None, Some(j))));
    pairs
}

/// After a merge appends the added items, are the items in B's order?
fn order_changed(pairs: &Pairs) -> bool {
    let seq: Vec<usize> = pairs.iter().filter_map(|p| p.1).collect();
    seq.windows(2).any(|w| matches!(w, [x, y] if x >= y))
}

/// Two lists of records with their labels.
struct Sides<'a, T> {
    a: &'a [T],
    b: &'a [T],
    labels_a: &'a [String],
    labels_b: &'a [String],
}

/// Changes between matched records. `fields: Some(skip)` compares field by field (leaving out
/// `skip`); `None` compares whole items.
fn records<T: Serialize>(out: &mut Builder, list: List, s: &Sides<'_, T>, pairs: &Pairs, fields: Option<&[&str]>) -> Result<()> {
    let part = list.part();
    let spot = |label: &str, field: Option<&str>| match list {
        List::Masters => Spot { master: Some(label.to_string()), field: field.map(str::to_string), ..Spot::default() },
        _ => Spot {
            field: Some(match field {
                Some(f) => format!("{}{label}/{f}", list.prefix()),
                None => format!("{}{label}", list.prefix()),
            }),
            ..Spot::default()
        },
    };
    for &(ia, ib) in pairs {
        let x = ia.and_then(|i| Some((i, s.a.get(i)?, s.labels_a.get(i)?)));
        let y = ib.and_then(|j| Some((j, s.b.get(j)?, s.labels_b.get(j)?)));
        match (x, y) {
            (Some((i, x, label)), None) => {
                let place = Place::Record { list, a: Some(i), b: None, field: None };
                out.push(part, Kind::Removed, place, spot(label, None), Some(to_json(x)?), None)?;
            }
            (None, Some((j, y, label))) => {
                let place = Place::Record { list, a: None, b: Some(j), field: None };
                out.push(part, Kind::Added, place, spot(label, None), None, Some(to_json(y)?))?;
            }
            (Some((i, x, label)), Some((j, y, _))) => {
                let (vx, vy) = (to_json(x)?, to_json(y)?);
                match fields {
                    None => {
                        if vx != vy {
                            let place = Place::Record { list, a: Some(i), b: Some(j), field: None };
                            out.push(part, Kind::Changed, place, spot(label, None), Some(vx), Some(vy))?;
                        }
                    }
                    Some(skip) => {
                        let empty = Map::new();
                        let ox = vx.as_object().unwrap_or(&empty);
                        let oy = vy.as_object().unwrap_or(&empty);
                        let keys: BTreeSet<&String> = ox.keys().chain(oy.keys()).filter(|k| !skip.contains(&k.as_str())).collect();
                        for key in keys {
                            let (fa, fb) = (ox.get(key), oy.get(key));
                            if fa != fb {
                                let place = Place::Record { list, a: Some(i), b: Some(j), field: Some(key.clone()) };
                                out.push(part, Kind::Changed, place, spot(label, Some(key)), fa.cloned(), fb.cloned())?;
                            }
                        }
                    }
                }
            }
            (None, None) => {}
        }
    }
    if order_changed(pairs) {
        let field = match list {
            List::Prefixes | List::Classes | List::Features | List::Groups | List::Properties | List::Params => format!("{}order", list.prefix()),
            _ => "order".to_string(),
        };
        let spot = Spot { field: Some(field), ..Spot::default() };
        out.push(part, Kind::Changed, Place::Order(list), spot, Some(json!(s.labels_a)), Some(json!(s.labels_b)))?;
    }
    Ok(())
}

fn info(out: &mut Builder, a: &Font, b: &Font, pairs: &mut HashMap<List, Pairs>) -> Result<()> {
    let scalars: [(&'static str, Value, Value); 14] = [
        ("familyName", json!(a.family_name), json!(b.family_name)),
        ("unitsPerEm", json!(a.units_per_em), json!(b.units_per_em)),
        ("versionMajor", json!(a.version_major), json!(b.version_major)),
        ("versionMinor", json!(a.version_minor), json!(b.version_minor)),
        ("metrics", to_json(&*a.metrics)?, to_json(&*b.metrics)?),
        ("stems", to_json(&*a.stems)?, to_json(&*b.stems)?),
        ("numbers", to_json(&*a.numbers)?, to_json(&*b.numbers)?),
        ("displayStrings", to_json(&*a.display_strings)?, to_json(&*b.display_strings)?),
        ("note", json!(a.note), json!(b.note)),
        ("files", to_json(&*a.files)?, to_json(&*b.files)?),
        ("extra", to_json(&*a.extra)?, to_json(&*b.extra)?),
        ("settings", to_json(&a.info.settings)?, to_json(&b.info.settings)?),
        ("date", to_json(&a.info.date)?, to_json(&b.info.date)?),
        ("userData", to_json(&a.info.user_data)?, to_json(&b.info.user_data)?),
    ];
    for (name, va, vb) in scalars {
        if va != vb {
            let spot = Spot { field: Some(name.to_string()), ..Spot::default() };
            out.push(Part::Info, Kind::Changed, Place::Info(name), spot, Some(va), Some(vb))?;
        }
    }
    let (pa, pb) = (&a.info.properties.0, &b.info.properties.0);
    let labels_a = keyed(pa.iter().map(|p| p.key.as_str()));
    let labels_b = keyed(pb.iter().map(|p| p.key.as_str()));
    let p = pair_by_key(&labels_a, &labels_b);
    records(out, List::Properties, &Sides { a: pa, b: pb, labels_a: &labels_a, labels_b: &labels_b }, &p, None)?;
    pairs.insert(List::Properties, p);
    let (pa, pb) = (&a.info.params.0, &b.info.params.0);
    let labels_a = keyed(pa.iter().map(|p| p.name.as_str()));
    let labels_b = keyed(pb.iter().map(|p| p.name.as_str()));
    let p = pair_by_key(&labels_a, &labels_b);
    records(out, List::Params, &Sides { a: pa, b: pb, labels_a: &labels_a, labels_b: &labels_b }, &p, None)?;
    pairs.insert(List::Params, p);
    Ok(())
}

fn features(out: &mut Builder, a: &Font, b: &Font, pairs: &mut HashMap<List, Pairs>) -> Result<()> {
    let (fa, fb) = (&a.features, &b.features);
    for (list, xa, xb) in
        [(List::Prefixes, &fa.prefixes, &fb.prefixes), (List::Classes, &fa.classes, &fb.classes), (List::Features, &fa.features, &fb.features)]
    {
        let labels_a = keyed(xa.iter().map(|s| s.name.as_str()));
        let labels_b = keyed(xb.iter().map(|s| s.name.as_str()));
        let p = pair_by_key(&labels_a, &labels_b);
        records(out, list, &Sides { a: xa, b: xb, labels_a: &labels_a, labels_b: &labels_b }, &p, None)?;
        pairs.insert(list, p);
    }
    let labels_a = keyed(a.groups.iter().map(|g| g.name.as_str()));
    let labels_b = keyed(b.groups.iter().map(|g| g.name.as_str()));
    let p = pair_by_key(&labels_a, &labels_b);
    records(out, List::Groups, &Sides { a: &a.groups, b: &b.groups, labels_a: &labels_a, labels_b: &labels_b }, &p, None)?;
    pairs.insert(List::Groups, p);
    Ok(())
}

pub(super) fn glyph_at(font: &Font, i: usize) -> Option<&Glyph> {
    font.glyphs.as_arcs().get(i).map(|g| g.as_ref())
}

/// A glyph of B with its layers' master ids in A's terms.
pub(super) fn map_glyph(g: &Glyph, mm: &MasterMap) -> Glyph {
    let mut g = g.clone();
    for l in &mut g.layers {
        let master_layer = l.is_master_layer();
        l.master = mm.a_id(&l.master);
        if master_layer {
            l.id = l.master.clone();
        }
    }
    g
}

/// The shapes of each master layer and the code points: equal for a glyph that was only renamed.
/// `None` for an empty glyph without code points, which says nothing about a rename.
fn fingerprint(g: &Glyph, ids: impl Iterator<Item = Option<String>>) -> Result<Option<String>> {
    let mut shapes = Vec::new();
    let mut drawn = false;
    for id in ids {
        match id.as_deref().and_then(|id| g.layer(id)) {
            Some(l) => {
                drawn |= !l.shapes.is_empty();
                shapes.push(to_json(&l.shapes)?);
            }
            None => shapes.push(Value::Null),
        }
    }
    if !drawn && g.unicodes.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!([g.unicodes, shapes]).to_string()))
}

/// Glyphs matched by name, then (with `renames`) the rest by unchanged outlines and code points.
pub(super) fn pair_glyphs(a: &Font, b: &Font, mm: &MasterMap, renames: bool) -> Result<Pairs> {
    let ka: Vec<String> = a.glyphs.iter().map(|g| g.name.clone()).collect();
    let kb: Vec<String> = b.glyphs.iter().map(|g| g.name.clone()).collect();
    let mut pairs = pair_by_key(&ka, &kb);
    if !renames {
        return Ok(pairs);
    }
    let ids = mm.ids(a, b);
    let mut free: HashMap<String, VecDeque<usize>> = HashMap::new();
    for &(_, ib) in pairs.iter().filter(|p| p.0.is_none()) {
        let Some(j) = ib else { continue };
        let Some(g) = glyph_at(b, j) else { continue };
        if let Some(fp) = fingerprint(g, ids.iter().map(|(_, id)| id.clone()))? {
            free.entry(fp).or_default().push_back(j);
        }
    }
    if free.is_empty() {
        return Ok(pairs);
    }
    let mut taken = BTreeSet::new();
    for pair in pairs.iter_mut().filter(|p| p.0.is_some() && p.1.is_none()) {
        let Some(g) = pair.0.and_then(|i| glyph_at(a, i)) else { continue };
        let Some(fp) = fingerprint(g, ids.iter().map(|(id, _)| Some(id.clone())))? else { continue };
        if let Some(j) = free.get_mut(&fp).and_then(VecDeque::pop_front) {
            pair.1 = Some(j);
            taken.insert(j);
        }
    }
    pairs.retain(|p| !(p.0.is_none() && p.1.is_some_and(|j| taken.contains(&j))));
    Ok(pairs)
}

/// A layer's compared fields: name, value and whether `tolerance` applies.
fn layer_fields(l: &Layer) -> Result<[(&'static str, Value, bool); 6]> {
    let (components, others): (Vec<&Shape>, Vec<&Shape>) = l.shapes.iter().partition(|s| matches!(s, Shape::Component(_)));
    Ok([
        ("width", json!(l.width), true),
        ("outlines", to_json(&others)?, true),
        ("components", to_json(&components)?, true),
        ("anchors", to_json(&l.anchors)?, true),
        ("info", to_json(&l.info)?, false),
        ("extra", to_json(&l.extra)?, false),
    ])
}

/// The layers that are not master layers, with master ids mapped when `mm` is given.
fn other_layers(g: &Glyph, mm: Option<&MasterMap>) -> Result<Value> {
    let layers: Vec<Layer> = g
        .layers
        .iter()
        .filter(|l| !l.is_master_layer())
        .map(|l| {
            let mut l = l.clone();
            if let Some(mm) = mm {
                l.master = mm.a_id(&l.master);
            }
            l
        })
        .collect();
    to_json(&layers)
}

fn glyphs(out: &mut Builder, a: &Font, b: &Font, mm: &MasterMap, pairs: &Pairs, opts: &Options) -> Result<()> {
    let want_glyphs = opts.parts.contains(&Part::Glyphs);
    let want_kerning = opts.parts.contains(&Part::Kerning);
    let ids = mm.ids(a, b);
    let chosen = |name: &str| opts.glyphs.as_ref().is_none_or(|set| set.contains(name));
    for &(ia, ib) in pairs {
        let ga = ia.and_then(|i| Some((i, glyph_at(a, i)?)));
        let gb = ib.and_then(|j| Some((j, glyph_at(b, j)?)));
        if !(ga.is_some_and(|(_, g)| chosen(&g.name)) || gb.is_some_and(|(_, g)| chosen(&g.name))) {
            continue;
        }
        let spot = |glyph: &str, master: Option<String>, field: Option<&str>| Spot {
            glyph: Some(glyph.to_string()),
            master,
            field: field.map(str::to_string),
        };
        match (ga, gb) {
            (Some((i, g)), None) if want_glyphs => {
                let place = Place::Record { list: List::Glyphs, a: Some(i), b: None, field: None };
                out.push(Part::Glyphs, Kind::Removed, place, spot(&g.name, None, None), Some(to_json(g)?), None)?;
            }
            (None, Some((j, g))) if want_glyphs => {
                let place = Place::Record { list: List::Glyphs, a: None, b: Some(j), field: None };
                out.push(Part::Glyphs, Kind::Added, place, spot(&g.name, None, None), None, Some(to_json(&map_glyph(g, mm))?))?;
            }
            (Some((i, x)), Some((j, y))) => {
                let name = y.name.as_str();
                if want_kerning && x.kern_groups != y.kern_groups {
                    let place = Place::GlyphField { a: i, b: j, field: "kernGroups" };
                    let (va, vb) = (to_json(&x.kern_groups)?, to_json(&y.kern_groups)?);
                    out.push(Part::Kerning, Kind::Changed, place, spot(name, None, Some("groups")), Some(va), Some(vb))?;
                }
                if !want_glyphs {
                    continue;
                }
                if x.name != y.name {
                    let place = Place::GlyphField { a: i, b: j, field: "name" };
                    out.push(Part::Glyphs, Kind::Renamed, place, spot(name, None, Some("name")), Some(json!(x.name)), Some(json!(y.name)))?;
                }
                let own: [(&'static str, Value, Value); 5] = [
                    ("unicodes", json!(x.unicodes), json!(y.unicodes)),
                    ("export", json!(x.export), json!(y.export)),
                    ("info", to_json(&x.info)?, to_json(&y.info)?),
                    ("extra", to_json(&x.extra)?, to_json(&y.extra)?),
                    ("otherLayers", other_layers(x, None)?, other_layers(y, Some(mm))?),
                ];
                for (field, va, vb) in own {
                    if va != vb {
                        let place = Place::GlyphField { a: i, b: j, field };
                        out.push(Part::Glyphs, Kind::Changed, place, spot(name, None, Some(field)), Some(va), Some(vb))?;
                    }
                }
                for (master_a, master_b) in &ids {
                    let la = x.layer(master_a);
                    let lb = master_b.as_deref().and_then(|id| y.layer(id));
                    let label = Some(mm.label(master_a));
                    let place = |field| Place::Layer { a: i, b: j, master_a: master_a.clone(), master_b: master_b.clone(), field };
                    match (la, lb) {
                        (Some(la), None) => {
                            out.push(Part::Glyphs, Kind::Removed, place(None), spot(name, label, Some("layer")), Some(to_json(la)?), None)?;
                        }
                        (None, Some(lb)) => {
                            out.push(Part::Glyphs, Kind::Added, place(None), spot(name, label, Some("layer")), None, Some(to_json(lb)?))?;
                        }
                        (Some(la), Some(lb)) => {
                            for ((field, va, geometry), (_, vb, _)) in layer_fields(la)?.into_iter().zip(layer_fields(lb)?) {
                                let tol = if geometry { opts.tolerance } else { 0.0 };
                                if !same(&va, &vb, tol) {
                                    out.push(
                                        Part::Glyphs,
                                        Kind::Changed,
                                        place(Some(field)),
                                        spot(name, label.clone(), Some(field)),
                                        Some(va),
                                        Some(vb),
                                    )?;
                                }
                            }
                        }
                        (None, None) => {}
                    }
                }
            }
            _ => {}
        }
    }
    if want_glyphs && opts.glyphs.is_none() && order_changed(pairs) {
        let na: Vec<&str> = a.glyphs.iter().map(|g| g.name.as_str()).collect();
        let nb: Vec<&str> = b.glyphs.iter().map(|g| g.name.as_str()).collect();
        let spot = Spot { field: Some("order".into()), ..Spot::default() };
        out.push(Part::Glyphs, Kind::Changed, Place::Order(List::Glyphs), spot, Some(json!(na)), Some(json!(nb)))?;
    }
    Ok(())
}

/// Every pair of a kerning table: (master in A's terms, first, second) → value. A pair stored
/// twice keeps its first value, as lookups do.
fn kern_pairs(t: &KernTable, mm: Option<&MasterMap>) -> BTreeMap<(String, String, String), f64> {
    let mut out = BTreeMap::new();
    for mk in &t.0 {
        let master = match mm {
            Some(mm) => mm.a_id(&mk.master),
            None => mk.master.clone(),
        };
        for kf in &mk.firsts {
            for (second, v) in &kf.seconds {
                out.entry((master.clone(), kf.first.clone(), second.clone())).or_insert(*v);
            }
        }
    }
    out
}

/// B's kerning contexts with master ids in A's terms.
pub(super) fn mapped_context(k: &Kerning, mm: &MasterMap) -> Vec<fontcraft_model::KernContext> {
    k.context
        .iter()
        .map(|c| {
            let mut c = c.clone();
            for (m, _) in &mut c.values {
                *m = mm.a_id(m);
            }
            c
        })
        .collect()
}

fn kerning(out: &mut Builder, a: &Font, b: &Font, mm: &MasterMap) -> Result<()> {
    for dir in Dir::ALL {
        let pa = kern_pairs(dir.table(&a.kerning), None);
        let pb = kern_pairs(dir.table(&b.kerning), Some(mm));
        let keys: BTreeSet<&(String, String, String)> = pa.keys().chain(pb.keys()).collect();
        for key in keys {
            let (va, vb) = (pa.get(key), pb.get(key));
            let kind = match (va, vb) {
                (Some(x), Some(y)) if x == y => continue,
                (Some(_), Some(_)) => Kind::Changed,
                (Some(_), None) => Kind::Removed,
                (None, Some(_)) => Kind::Added,
                (None, None) => continue,
            };
            let (master, first, second) = key;
            let place = Place::KernPair { dir, master: master.clone(), first: first.clone(), second: second.clone() };
            let spot = Spot { glyph: None, master: Some(mm.label(master)), field: Some(format!("{} {first} {second}", dir.name())) };
            out.push(Part::Kerning, kind, place, spot, va.map(|v| json!(v)), vb.map(|v| json!(v)))?;
        }
    }
    let (ca, cb) = (to_json(&a.kerning.context)?, to_json(&mapped_context(&b.kerning, mm))?);
    if ca != cb {
        let spot = Spot { field: Some("context".into()), ..Spot::default() };
        out.push(Part::Kerning, Kind::Changed, Place::KernContext, spot, Some(ca), Some(cb))?;
    }
    Ok(())
}

/// What `font.compareGlyphs` reports.
#[derive(Debug, Default)]
pub(super) struct GlyphReport {
    pub only_a: Vec<String>,
    pub only_b: Vec<String>,
    /// Glyph (A's name), master name, master id in A, glyph index in B, master id in B.
    pub outlines: Vec<(String, String, String, usize, String)>,
    pub names: Vec<(String, Vec<u32>, Vec<u32>)>,
}

/// Which comparisons `font.compareGlyphs` makes.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct What {
    pub glyph_set: bool,
    pub outlines: bool,
    pub names: bool,
}

fn same_outline(a: &[PathEl], b: &[PathEl], tol: f64) -> bool {
    let close = |p: fontcraft_geom::Point, q: fontcraft_geom::Point| (p.x - q.x).abs() <= tol && (p.y - q.y).abs() <= tol;
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => close(*p, *q),
            (PathEl::QuadTo(p1, p2), PathEl::QuadTo(q1, q2)) => close(*p1, *q1) && close(*p2, *q2),
            (PathEl::CurveTo(p1, p2, p3), PathEl::CurveTo(q1, q2, q3)) => close(*p1, *q1) && close(*p2, *q2) && close(*p3, *q3),
            (PathEl::ClosePath, PathEl::ClosePath) => true,
            _ => false,
        })
}

/// Glyph-level comparison of two fonts, by glyph name and matched master.
pub(super) fn compare_glyphs(a: &Font, b: &Font, what: What, tol: f64, inlined: bool) -> Result<GlyphReport> {
    let mm = MasterMap::new(a, b);
    let ids: Vec<(String, String)> = mm.ids(a, b).into_iter().filter_map(|(x, y)| Some((x, y?))).filter(|(x, _)| a.master(x).is_ok()).collect();
    let mut report = GlyphReport::default();
    for (ia, ib) in pair_glyphs(a, b, &mm, false)? {
        match (ia.and_then(|i| glyph_at(a, i)), ib.and_then(|j| Some((j, glyph_at(b, j)?)))) {
            (Some(g), None) if what.glyph_set => report.only_a.push(g.name.clone()),
            (None, Some((_, g))) if what.glyph_set => report.only_b.push(g.name.clone()),
            (Some(x), Some((j, y))) => {
                if what.names && x.unicodes != y.unicodes {
                    report.names.push((x.name.clone(), x.unicodes.clone(), y.unicodes.clone()));
                }
                if !what.outlines {
                    continue;
                }
                for (master_a, master_b) in &ids {
                    let differs = if inlined {
                        let pa = a.outline_of(x, master_a)?;
                        let pb = b.outline_of(y, master_b)?;
                        !same_outline(pa.elements(), pb.elements(), tol)
                    } else {
                        let fa = match x.layer(master_a) {
                            Some(l) => layer_fields(l)?,
                            None => continue,
                        };
                        let fb = match y.layer(master_b) {
                            Some(l) => layer_fields(l)?,
                            None => continue,
                        };
                        fa.iter().zip(&fb).any(|((f, va, _), (_, vb, _))| matches!(*f, "outlines" | "components") && !same(va, vb, tol))
                    };
                    if differs {
                        report.outlines.push((x.name.clone(), mm.label(master_a), master_a.clone(), j, master_b.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(report)
}
