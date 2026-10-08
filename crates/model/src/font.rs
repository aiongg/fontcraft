//! The font document: its info, masters, glyph table and checks.

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use fontcraft_geom::{BezPath, Point};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::features::Features;
use crate::glyph::{Glyph, Layer, Path, Shape};
use crate::kerning::Kerning;
use crate::{
    Extra, MAX_COMPONENT_DEPTH, MAX_COORDINATE, MAX_GLYPHS, MAX_OUTLINE_ELEMENTS, MAX_UNICODES, ModelError, UPM_RANGE, check_name, in_range,
};

/// Where a document came from, so that saving can go back to the same format and version.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub format: SourceFormat,
    /// The `.glyphs` format version (2, 3 or 4) the document was read in, and is written in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyphs_version: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceFormat {
    /// Made in FontCraft, or read from a `.fontcraft` file.
    #[default]
    Native,
    Glyphs,
    GlyphsPackage,
    Ufo,
    Designspace,
    Binary,
}

/// A font-level metric definition. The masters carry the values, in the same order
/// ([`Master::metric_values`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricDef {
    /// The kind, spelled as the format spells it: [`metric::ASCENDER`] and its neighbours, or any
    /// other text a file uses.
    pub kind: String,
    /// A name, for a second metric of the same kind (for example a small-cap height).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A glyph predicate: the metric applies only to the glyphs it selects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl MetricDef {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.to_string(), name: None, filter: None, extra: Extra::new() }
    }

    /// A plain definition of this kind: no name and no filter.
    fn is_plain(&self, kind: &str) -> bool {
        self.kind == kind && self.name.is_none() && self.filter.is_none()
    }
}

/// The metric kinds, spelled as the reference's format version 3 spells them.
pub mod metric {
    pub const ASCENDER: &str = "ascender";
    pub const CAP_HEIGHT: &str = "cap height";
    pub const X_HEIGHT: &str = "x-height";
    pub const BASELINE: &str = "baseline";
    pub const DESCENDER: &str = "descender";
    pub const ITALIC_ANGLE: &str = "italic angle";
}

/// A master's value for one metric: a position and an overshoot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricValue {
    #[serde(default)]
    pub pos: f64,
    #[serde(default)]
    pub over: f64,
}

/// A design axis of the family (weight, width…).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    pub name: String,
    /// The OpenType axis tag. A source may write a longer one, whose first four characters are
    /// the exported tag.
    pub tag: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A named instance: a static font to export, or another export setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// The style name (may be empty: format version 4 keeps it among the instance's properties).
    #[serde(default)]
    pub name: String,
    /// Design-space coordinates, parallel to [`Font::axes`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub location: Vec<f64>,
    /// Whether the instance is exported.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub exports: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

/// One design of the family (a weight, a width…).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Master {
    pub id: String,
    pub name: String,
    /// Design-space coordinates, parallel to [`Font::axes`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub location: Vec<f64>,
    /// Parallel to [`Font::metrics`]; may be shorter, and a missing value is zero.
    #[serde(default, rename = "metricValues", skip_serializing_if = "Vec::is_empty")]
    pub metric_values: Vec<MetricValue>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A font source: one family, all its masters and glyphs.
///
/// The parts are behind [`Arc`]s, so cloning a font (an undo snapshot, a background save) copies a
/// handful of pointers, and an edit copies only what it changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Font {
    #[serde(default)]
    pub source: Source,
    pub family_name: String,
    pub units_per_em: u16,
    #[serde(default = "one")]
    pub version_major: u16,
    #[serde(default)]
    pub version_minor: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Arc<Vec<Axis>>,
    #[serde(default)]
    pub metrics: Arc<Vec<MetricDef>>,
    pub masters: Arc<Vec<Master>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Arc<Vec<Instance>>,
    /// In glyph order.
    pub glyphs: GlyphTable,
    #[serde(default, skip_serializing_if = "Kerning::is_empty")]
    pub kerning: Arc<Kerning>,
    #[serde(default, skip_serializing_if = "Features::is_empty")]
    pub features: Arc<Features>,
    /// The texts of the edit tabs, saved with the document.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display_strings: Arc<Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Files a multi-file source holds beside what the model reads (a package's feature files and
    /// UI state, a UFO's `data/` and `images/`), by relative path, written back unchanged.
    #[serde(default, skip_serializing_if = "FileStore::is_empty")]
    pub files: Arc<FileStore>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Arc<Extra>,
}

/// Relative path (with `/`) → contents. In the native file each file is base64 text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileStore(pub std::collections::BTreeMap<String, Arc<[u8]>>);

impl FileStore {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for FileStore {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use base64::Engine as _;
        let m: std::collections::BTreeMap<&str, String> =
            self.0.iter().map(|(k, v)| (k.as_str(), base64::engine::general_purpose::STANDARD.encode(v))).collect();
        m.serialize(s)
    }
}

impl<'de> Deserialize<'de> for FileStore {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use base64::Engine as _;
        let m = std::collections::BTreeMap::<String, String>::deserialize(d)?;
        let mut out = std::collections::BTreeMap::new();
        for (k, v) in m {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(v.as_bytes())
                .map_err(|e| <D::Error as serde::de::Error>::custom(format!("file `{k}`: {e}")))?;
            out.insert(k, Arc::from(bytes));
        }
        Ok(FileStore(out))
    }
}

fn one() -> u16 {
    1
}

/// The glyphs in order, with an index by name. Read it as a slice; change it through its methods
/// (or [`Font::glyph_mut`]) so the index stays right.
#[derive(Clone, Debug, Default)]
pub struct GlyphTable {
    order: Arc<Vec<Arc<Glyph>>>,
    /// Name → position of the first glyph with that name.
    index: Arc<HashMap<String, usize>>,
}

impl PartialEq for GlyphTable {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order
    }
}

impl Deref for GlyphTable {
    type Target = [Arc<Glyph>];
    fn deref(&self) -> &[Arc<Glyph>] {
        &self.order
    }
}

impl GlyphTable {
    pub fn from_glyphs(glyphs: Vec<Arc<Glyph>>) -> Self {
        let mut t = Self { order: Arc::new(glyphs), index: Arc::default() };
        t.reindex();
        t
    }

    fn reindex(&mut self) {
        let mut index = HashMap::with_capacity(self.order.len());
        for (i, g) in self.order.iter().enumerate() {
            index.entry(g.name.clone()).or_insert(i);
        }
        self.index = Arc::new(index);
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<Glyph>> {
        self.position(name).and_then(|i| self.order.get(i))
    }

    pub fn push(&mut self, glyph: Glyph) {
        let i = self.order.len();
        let name = glyph.name.clone();
        Arc::make_mut(&mut self.order).push(Arc::new(glyph));
        Arc::make_mut(&mut self.index).entry(name).or_insert(i);
    }

    /// Removes the first glyph called `name`.
    pub fn remove(&mut self, name: &str) -> Option<Arc<Glyph>> {
        let i = self.position(name)?;
        let order = Arc::make_mut(&mut self.order);
        if i >= order.len() {
            return None;
        }
        let g = order.remove(i);
        self.reindex();
        Some(g)
    }

    /// Keeps the glyphs for which `keep` is true.
    pub fn retain(&mut self, keep: impl FnMut(&Arc<Glyph>) -> bool) {
        Arc::make_mut(&mut self.order).retain(keep);
        self.reindex();
    }

    /// The glyph at `i` for editing; only it is copied if a snapshot still shares it. A rename
    /// through the returned guard updates the index when the guard is dropped.
    pub fn get_mut_at(&mut self, i: usize) -> Option<GlyphMut<'_>> {
        let order = Arc::make_mut(&mut self.order);
        let slot = order.get_mut(i)?;
        let before = slot.name.clone();
        Some(GlyphMut { glyph: Arc::make_mut(slot), before, index: &mut self.index, position: i })
    }

    pub fn get_mut(&mut self, name: &str) -> Option<GlyphMut<'_>> {
        let i = self.position(name)?;
        self.get_mut_at(i)
    }

    /// The glyph pointers themselves, for code that compares snapshots.
    pub fn as_arcs(&self) -> &[Arc<Glyph>] {
        &self.order
    }
}

/// A glyph being edited (see [`GlyphTable::get_mut`]).
pub struct GlyphMut<'a> {
    glyph: &'a mut Glyph,
    before: String,
    index: &'a mut Arc<HashMap<String, usize>>,
    position: usize,
}

impl Deref for GlyphMut<'_> {
    type Target = Glyph;
    fn deref(&self) -> &Glyph {
        self.glyph
    }
}

impl DerefMut for GlyphMut<'_> {
    fn deref_mut(&mut self) -> &mut Glyph {
        self.glyph
    }
}

impl Drop for GlyphMut<'_> {
    fn drop(&mut self) {
        if self.glyph.name != self.before {
            let index = Arc::make_mut(self.index);
            if index.get(&self.before) == Some(&self.position) {
                index.remove(&self.before);
            }
            index.entry(self.glyph.name.clone()).or_insert(self.position);
        }
    }
}

impl Serialize for GlyphTable {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.order.serialize(s)
    }
}

impl<'de> Deserialize<'de> for GlyphTable {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::<Arc<Glyph>>::deserialize(d).map(GlyphTable::from_glyphs)
    }
}

impl Font {
    /// A new family with one Regular master and the starting glyph set: `.notdef`, `space` and
    /// the basic Latin letters (empty, encoded, ready to draw).
    pub fn new(family_name: impl Into<String>) -> Self {
        let master_id = "m01".to_string();
        let defs = [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER];
        let values = [800.0, 700.0, 500.0, 0.0, -200.0];
        let overs = [16.0, 16.0, 16.0, -16.0, -16.0];
        let master = Master {
            id: master_id.clone(),
            name: "Regular".into(),
            location: Vec::new(),
            metric_values: values.iter().zip(overs).map(|(&pos, over)| MetricValue { pos, over }).collect(),
            extra: Extra::new(),
        };
        let mut glyphs = Vec::new();
        let mut notdef = Glyph::new(".notdef");
        let mut layer = Layer::new(&master_id, 500.0);
        layer.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 450.0, 700.0)));
        // The inner contour runs the other way, so it is a hole under the non-zero rule.
        let mut inner = Path::rect(100.0, 50.0, 400.0, 650.0);
        inner.nodes.reverse();
        layer.shapes.push(Shape::Path(inner));
        notdef.layers.push(layer);
        glyphs.push(Arc::new(notdef));
        let mut space = Glyph::new("space");
        space.unicodes.push(0x20);
        space.layers.push(Layer::new(&master_id, 250.0));
        glyphs.push(Arc::new(space));
        for c in ('A'..='Z').chain('a'..='z') {
            let mut g = Glyph::new(c.to_string());
            g.unicodes.push(c as u32);
            g.layers.push(Layer::new(&master_id, 600.0));
            glyphs.push(Arc::new(g));
        }
        Self {
            source: Source::default(),
            family_name: family_name.into(),
            units_per_em: 1000,
            version_major: 1,
            version_minor: 0,
            axes: Arc::default(),
            metrics: Arc::new(defs.iter().map(|k| MetricDef::new(k)).collect()),
            masters: Arc::new(vec![master]),
            instances: Arc::default(),
            glyphs: GlyphTable::from_glyphs(glyphs),
            kerning: Arc::default(),
            features: Arc::default(),
            display_strings: Arc::default(),
            note: String::new(),
            files: Arc::default(),
            extra: Arc::default(),
        }
    }

    pub fn default_master(&self) -> Result<&Master, ModelError> {
        self.masters.first().ok_or(ModelError::NoMaster)
    }

    pub fn master(&self, id: &str) -> Result<&Master, ModelError> {
        self.masters.iter().find(|m| m.id == id).ok_or_else(|| ModelError::NoSuchMaster(id.to_string()))
    }

    /// The value of the plain metric `kind` (no name, no filter) for `master`, if the font
    /// defines one. A master without a value for it has zero.
    pub fn metric(&self, master: &Master, kind: &str) -> Option<MetricValue> {
        let i = self.metrics.iter().position(|d| d.is_plain(kind))?;
        Some(master.metric_values.get(i).copied().unwrap_or_default())
    }

    /// The position of the plain metric `kind` for `master`, or `default` when the font has none.
    pub fn metric_pos(&self, master: &Master, kind: &str, default: f64) -> f64 {
        self.metric(master, kind).map_or(default, |v| v.pos)
    }

    /// Sets the position of the plain metric `kind` for `master`, adding the definition (with a
    /// zero value for the other masters) when the font has none.
    pub fn set_metric_pos(&mut self, master_id: &str, kind: &str, pos: f64) -> Result<(), ModelError> {
        if !self.masters.iter().any(|m| m.id == master_id) {
            return Err(ModelError::NoSuchMaster(master_id.to_string()));
        }
        let i = match self.metrics.iter().position(|d| d.is_plain(kind)) {
            Some(i) => i,
            None => {
                Arc::make_mut(&mut self.metrics).push(MetricDef::new(kind));
                self.metrics.len() - 1
            }
        };
        for m in Arc::make_mut(&mut self.masters).iter_mut().filter(|m| m.id == master_id) {
            if m.metric_values.len() <= i {
                m.metric_values.resize(i + 1, MetricValue::default());
            }
            if let Some(v) = m.metric_values.get_mut(i) {
                v.pos = pos;
            }
        }
        Ok(())
    }

    /// The glyph called `name`.
    pub fn glyph(&self, name: &str) -> Option<&Arc<Glyph>> {
        self.glyphs.get(name)
    }

    /// The glyph for editing. Only this glyph is copied if a snapshot still shares it.
    pub fn glyph_mut(&mut self, name: &str) -> Result<GlyphMut<'_>, ModelError> {
        self.glyphs.get_mut(name).ok_or_else(|| ModelError::NoSuchGlyph(name.to_string()))
    }

    /// Appends a glyph with an empty layer for every master.
    pub fn add_glyph(&mut self, name: &str, unicodes: Vec<u32>, width: f64) -> Result<(), ModelError> {
        if name.is_empty() {
            return Err(ModelError::EmptyGlyphName);
        }
        if self.glyph(name).is_some() {
            return Err(ModelError::DuplicateGlyph(name.to_string()));
        }
        if self.glyphs.len() >= MAX_GLYPHS {
            return Err(ModelError::TooManyGlyphs);
        }
        let mut g = Glyph::new(name);
        g.unicodes = unicodes;
        g.layers = self.masters.iter().map(|m| Layer::new(&m.id, width)).collect();
        self.glyphs.push(g);
        Ok(())
    }

    pub fn remove_glyph(&mut self, name: &str) -> Result<(), ModelError> {
        self.glyphs.remove(name).map(|_| ()).ok_or_else(|| ModelError::NoSuchGlyph(name.to_string()))
    }

    /// The outline of `glyph` on `master` with its components resolved, in font units.
    ///
    /// Bounded work: components nest at most [`MAX_COMPONENT_DEPTH`] deep, the result has at most
    /// [`MAX_OUTLINE_ELEMENTS`] segments, and every point is within [`MAX_COORDINATE`].
    pub fn outline(&self, glyph: &str, master: &str) -> Result<BezPath, ModelError> {
        let g = self.glyph(glyph).ok_or_else(|| ModelError::NoSuchGlyph(glyph.to_string()))?;
        self.outline_of(g, master)
    }

    /// [`Font::outline`] for a glyph already in hand (no lookup by name for the glyph itself).
    pub fn outline_of(&self, glyph: &Glyph, master: &str) -> Result<BezPath, ModelError> {
        let mut budget = MAX_OUTLINE_ELEMENTS;
        self.outline_at(glyph, master, 0, &mut budget)
    }

    fn outline_at(&self, g: &Glyph, master: &str, depth: usize, budget: &mut usize) -> Result<BezPath, ModelError> {
        let glyph = g.name.as_str();
        if depth > MAX_COMPONENT_DEPTH {
            return Err(ModelError::ComponentCycle(glyph.to_string()));
        }
        let bad = |message: &str| ModelError::BadOutline { glyph: glyph.to_string(), message: message.to_string() };
        let mut out = BezPath::new();
        let Some(layer) = g.layer(master) else { return Ok(out) };
        for shape in &layer.shapes {
            let part = match shape {
                Shape::Path(p) => p.to_bez().map_err(|message| bad(&message))?,
                Shape::Component(c) => {
                    if c.transform.numbers().iter().any(|v| !v.is_finite()) {
                        return Err(bad("a component transform is not a number"));
                    }
                    let base = self.glyph(&c.glyph).ok_or_else(|| ModelError::NoSuchGlyph(c.glyph.clone()))?;
                    let mut inner = self.outline_at(base, master, depth + 1, budget)?;
                    inner.apply_affine(c.affine());
                    // A transform can push points anywhere: hold the result to the same limits.
                    let far = |p: Point| !(in_range(p.x) && in_range(p.y));
                    let off_limits = inner.elements().iter().any(|el| match *el {
                        fontcraft_geom::PathEl::MoveTo(p) | fontcraft_geom::PathEl::LineTo(p) => far(p),
                        fontcraft_geom::PathEl::QuadTo(a, p) => far(a) || far(p),
                        fontcraft_geom::PathEl::CurveTo(a, b, p) => far(a) || far(b) || far(p),
                        fontcraft_geom::PathEl::ClosePath => false,
                    });
                    if off_limits {
                        return Err(bad(&format!("a component of `{}` puts points beyond ±{MAX_COORDINATE}", c.glyph)));
                    }
                    inner
                }
                Shape::Raw(_) => continue,
            };
            // Charge what this glyph adds, including what its components brought, so nested
            // references cannot multiply the work past the budget.
            *budget = budget.checked_sub(part.elements().len()).ok_or_else(|| ModelError::OutlineTooLarge(glyph.to_string()))?;
            out.extend(part);
        }
        Ok(out)
    }

    /// The checks that concern the font as a whole (not the inside of any glyph).
    fn check_font_level(&self) -> Result<(), ModelError> {
        if !UPM_RANGE.contains(&self.units_per_em) {
            return Err(ModelError::BadUnitsPerEm(u32::from(self.units_per_em)));
        }
        // A source may leave the family name empty; a compiled font then gets a stand-in.
        if !self.family_name.is_empty() {
            check_name("family", &self.family_name)?;
        }
        if self.masters.is_empty() {
            return Err(ModelError::NoMaster);
        }
        let mut ids = HashSet::new();
        for m in self.masters.iter() {
            // A master may have no name of its own: the reference derives one from its axis values.
            if !m.name.is_empty() {
                check_name("master", &m.name)?;
            }
            check_name("master id", &m.id)?;
            if !ids.insert(m.id.as_str()) {
                return Err(ModelError::DuplicateMaster(m.id.clone()));
            }
            if !m.metric_values.iter().all(|v| in_range(v.pos) && in_range(v.over)) {
                return Err(ModelError::BadMetric(if m.name.is_empty() { m.id.clone() } else { m.name.clone() }));
            }
        }
        if self.glyphs.len() > MAX_GLYPHS {
            return Err(ModelError::TooManyGlyphs);
        }
        let mut names = HashSet::new();
        for g in self.glyphs.iter() {
            if g.name.is_empty() {
                return Err(ModelError::EmptyGlyphName);
            }
            if !names.insert(g.name.as_str()) {
                return Err(ModelError::DuplicateGlyph(g.name.clone()));
            }
        }
        Ok(())
    }

    /// The checks inside one glyph: its name, code points, layers, and that every path is a
    /// well-formed node list within the coordinate limits. (Whether components resolve is checked
    /// when an outline is asked for: a component may name a glyph that is added later.)
    pub fn check_glyph(&self, g: &Glyph) -> Result<(), ModelError> {
        let bad = |message: String| ModelError::BadOutline { glyph: g.name.clone(), message };
        if g.name.is_empty() {
            return Err(ModelError::EmptyGlyphName);
        }
        check_name("glyph", &g.name)?;
        if g.unicodes.len() > MAX_UNICODES {
            return Err(ModelError::TooManyUnicodes { glyph: g.name.clone() });
        }
        if let Some(&value) = g.unicodes.iter().find(|u| char::from_u32(**u).is_none()) {
            return Err(ModelError::BadUnicode { glyph: g.name.clone(), value });
        }
        let mut layer_ids = HashSet::new();
        for l in &g.layers {
            self.master(&l.master)?;
            if !layer_ids.insert(l.id.as_str()) {
                return Err(ModelError::DuplicateLayer { glyph: g.name.clone(), master: l.id.clone() });
            }
            if !in_range(l.width) {
                return Err(bad(format!("the advance width {} is not a number or is beyond ±{MAX_COORDINATE}", l.width)));
            }
            for shape in &l.shapes {
                match shape {
                    Shape::Path(p) => {
                        p.to_bez().map_err(&bad)?;
                    }
                    Shape::Component(c) => {
                        check_name("component glyph", &c.glyph)?;
                        if c.transform.numbers().iter().any(|v| !v.is_finite()) {
                            return Err(bad("a component transform is not a number".into()));
                        }
                    }
                    Shape::Raw(_) => {}
                }
            }
            if l.anchors.iter().any(|a| !(in_range(a.x) && in_range(a.y))) {
                return Err(bad("an anchor position is not a number or is out of range".into()));
            }
        }
        Ok(())
    }

    /// Every invariant a font must satisfy: what a file must pass to load, and what every
    /// command must leave true. `Err` names the first problem.
    pub fn check(&self) -> Result<(), ModelError> {
        self.check_font_level()?;
        self.glyphs.iter().try_for_each(|g| self.check_glyph(g))
    }

    /// [`Font::check`] for a font that was valid as `before` and has since been edited: only the
    /// glyphs that are no longer shared with `before` are looked into. This is what keeps an edit
    /// to one glyph from costing a pass over the whole font.
    pub fn check_changed(&self, before: &Font) -> Result<(), ModelError> {
        self.check_font_level()?;
        // A master was added, removed or renamed: layers everywhere may be affected.
        if !Arc::ptr_eq(&self.masters, &before.masters) && self.masters != before.masters {
            return self.glyphs.iter().try_for_each(|g| self.check_glyph(g));
        }
        if Arc::ptr_eq(&self.glyphs.order, &before.glyphs.order) {
            return Ok(());
        }
        let shared: HashSet<*const Glyph> = before.glyphs.iter().map(Arc::as_ptr).collect();
        self.glyphs.iter().filter(|g| !shared.contains(&Arc::as_ptr(g))).try_for_each(|g| self.check_glyph(g))
    }

    /// Whether this font has the same contents as `before`, compared cheaply: parts still shared
    /// with `before` are equal without looking inside, so after an edit only what was copied is
    /// compared. This is how an edit that changed nothing is told apart.
    pub fn same_as(&self, before: &Font) -> bool {
        fn arc_eq<T: PartialEq>(a: &Arc<T>, b: &Arc<T>) -> bool {
            Arc::ptr_eq(a, b) || a == b
        }
        self.source == before.source
            && self.family_name == before.family_name
            && self.units_per_em == before.units_per_em
            && self.version_major == before.version_major
            && self.version_minor == before.version_minor
            && arc_eq(&self.metrics, &before.metrics)
            && arc_eq(&self.masters, &before.masters)
            && arc_eq(&self.extra, &before.extra)
            && arc_eq(&self.files, &before.files)
            && arc_eq(&self.kerning, &before.kerning)
            && arc_eq(&self.features, &before.features)
            && arc_eq(&self.display_strings, &before.display_strings)
            && self.note == before.note
            && arc_eq(&self.axes, &before.axes)
            && arc_eq(&self.instances, &before.instances)
            && (Arc::ptr_eq(&self.glyphs.order, &before.glyphs.order)
                || self.glyphs.len() == before.glyphs.len() && self.glyphs.iter().zip(before.glyphs.iter()).all(|(a, b)| arc_eq(a, b)))
    }
}
