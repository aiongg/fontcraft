//! The generic compile: a table element read field by field as its layout in the writer's
//! schema says, into the writers of [`super::pack`]. Ported from fontTools'
//! `otBase.BaseTable.compile`, the `write` and `xmlRead` methods of `otConverters`, and the
//! `fromXML` and `preWrite` methods of `otTables` (MIT).
//!
//! The rules kept from fontTools:
//! - a single field takes the last element of its name, an array every element of its name in
//!   document order; `index` attributes are not read;
//! - counts are not read: each is the length of the array that names it, set where the array is
//!   written, and the counts in [`PROPAGATED`] reach the tables written after them;
//! - a missing offset, or one marked `empty`, is null; a missing value record is all zeros;
//! - Coverage, ClassDef, the four simple substitutions and the two delta-set index maps are read
//!   in their summarised forms and turned into their raw layouts first, choosing the format as
//!   fontTools does;
//! - a lookup subtable must have its lookup's type, and feature parameters must match their
//!   feature's tag.

use std::collections::HashMap;
use std::rc::Rc;

use crate::limits::{MAX_LAYOUT_ITEMS, MAX_TTX_DEPTH};
use crate::sfnt::SfntFont;
use crate::ttx::Element;
use crate::ttx::otdata::engine::{Cond, Field, Layout, Rep, Ty, Width};
use crate::ttx::read::{Out, ReadContext, float_lit, int_lit, ot_round};
use crate::{Result, TablesError};

use super::pack::{Graph, Item};

/// The counts and values a table passes to the tables written after it (fontTools' propagated
/// names).
const PROPAGATED: [&str; 15] = [
    "ClassCount",
    "Class2Count",
    "FeatureTag",
    "SettingsCount",
    "VarRegionCount",
    "MappingCount",
    "RegionAxisCount",
    "DesignAxisCount",
    "DesignAxisRecordSize",
    "AxisValueCount",
    "ValueRecordSize",
    "AxisCount",
    "BaseGlyphRecordCount",
    "LayerRecordCount",
    "AxisIndicesList",
];

/// The fields of a value record in format-bit order: name, device offset, signed.
const VALUE_RECORD: [(&str, bool, bool); 16] = [
    ("XPlacement", false, true),
    ("YPlacement", false, true),
    ("XAdvance", false, true),
    ("YAdvance", false, true),
    ("XPlaDevice", true, false),
    ("YPlaDevice", true, false),
    ("XAdvDevice", true, false),
    ("YAdvDevice", true, false),
    ("Reserved1", false, false),
    ("Reserved2", false, false),
    ("Reserved3", false, false),
    ("Reserved4", false, false),
    ("Reserved5", false, false),
    ("Reserved6", false, false),
    ("Reserved7", false, false),
    ("Reserved8", false, false),
];

/// The lookup subtable layouts and their lookup types.
const SUBTABLES: [(&str, i64); 17] = [
    ("SingleSubst", 1),
    ("MultipleSubst", 2),
    ("AlternateSubst", 3),
    ("LigatureSubst", 4),
    ("ContextSubst", 5),
    ("ChainContextSubst", 6),
    ("ExtensionSubst", 7),
    ("ReverseChainSingleSubst", 8),
    ("SinglePos", 1),
    ("PairPos", 2),
    ("CursivePos", 3),
    ("MarkBasePos", 4),
    ("MarkLigPos", 5),
    ("MarkMarkPos", 6),
    ("ContextPos", 7),
    ("ChainContextPos", 8),
    ("ExtensionPos", 9),
];

const FEATURE_PARAMS: [&str; 4] = ["FeatureParams", "FeatureParamsSize", "FeatureParamsStylisticSet", "FeatureParamsCharacterVariants"];

fn lookup_type(class: &str) -> Option<i64> {
    SUBTABLES.iter().find(|(n, _)| *n == class).map(|(_, t)| *t)
}

fn bad(what: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(what.to_string())
}

/// fontTools' counts: names ending in `Count`, and two sizes.
fn is_count(name: &str) -> bool {
    name.ends_with("Count") || name == "DesignAxisRecordSize" || name == "ValueRecordSize"
}

/// What a writer passes to the writers made after it (fontTools' `localState`).
#[derive(Clone, Debug)]
enum St {
    Slot(usize),
    Format(i64),
    Tag(String),
}

#[derive(Clone, Debug, Default)]
struct State(Rc<Vec<(&'static str, St)>>);

impl State {
    fn get(&self, name: &str) -> Option<&St> {
        self.0.iter().rev().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    fn set(&mut self, name: &'static str, value: St) {
        let mut v: Vec<(&'static str, St)> = self.0.iter().filter(|(n, _)| *n != name).cloned().collect();
        v.push((name, value));
        self.0 = Rc::new(v);
    }
}

/// An element built in code: the raw layout a summarised form turns into.
pub(crate) fn el(name: &str, attributes: Vec<(&str, String)>, children: Vec<Element>) -> Element {
    Element { name: name.to_string(), attributes: attributes.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), children, text: String::new() }
}

/// `<name value="text"/>`.
pub(crate) fn val(name: &str, text: impl Into<String>) -> Element {
    el(name, vec![("value", text.into())], Vec::new())
}

fn value_of(e: &Element) -> Result<&str> {
    e.attribute("value").ok_or_else(|| bad(format!("the element '{}' has no value", e.name)))
}

/// fontTools' `versionToFixed`.
pub(crate) fn version_value(text: &str) -> Result<i64> {
    let t = text.trim();
    let v = if t.starts_with('0') { int_lit(t)? as f64 } else { float_lit(t)? };
    if v < 65536.0 { ot_round(v * 65536.0) } else { ot_round(v) }
}

/// A Python list or tuple of integers (`[1, -2]`), as `literal_eval` reads it.
pub(crate) fn int_list(text: &str) -> Result<Vec<i64>> {
    let t = text.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .or_else(|| t.strip_prefix('(').and_then(|r| r.strip_suffix(')')))
        .ok_or_else(|| bad(format!("'{text}' is not a list of numbers")))?;
    let mut out = Vec::new();
    let parts: Vec<&str> = inner.split(',').collect();
    for (i, part) in parts.iter().enumerate() {
        let p = part.trim();
        if p.is_empty() && i + 1 == parts.len() {
            break;
        }
        if out.len() >= MAX_LAYOUT_ITEMS {
            return Err(TablesError::TooLarge { what: "the numbers of a list", limit: MAX_LAYOUT_ITEMS });
        }
        out.push(int_lit(p)?);
    }
    Ok(out)
}

/// An insertion-ordered map, as Python's `dict`: a key set again keeps its place.
struct Ordered<V> {
    items: Vec<(String, V)>,
    index: HashMap<String, usize>,
}

impl<V> Ordered<V> {
    fn new() -> Self {
        Self { items: Vec::new(), index: HashMap::new() }
    }

    fn insert(&mut self, key: &str, value: V) {
        match self.index.get(key).and_then(|&i| self.items.get_mut(i)) {
            Some(slot) => slot.1 = value,
            None => {
                self.index.insert(key.to_string(), self.items.len());
                self.items.push((key.to_string(), value));
            }
        }
    }
}

/// The compile of one table.
pub(crate) struct Compiler<'a> {
    ctx: &'a ReadContext,
    /// The number of `fvar` axes, when the font has an `fvar` table.
    fvar_axes: Option<i64>,
    graph: Graph,
    /// The count slot of the `StructLength` field of the table being written, filled with the
    /// table's length once it is written (fontTools' `StructWithLength`).
    struct_length: Option<usize>,
}

/// Compiles table element `element` whose layout in the schema is named `class` (`GSUB`,
/// `HVAR`…).
pub(crate) fn compile_layout(class: &str, element: &Element, ctx: &ReadContext, font: &SfntFont) -> Result<Vec<u8>> {
    let mut c = Compiler::new(ctx, font);
    let root = c.graph.node();
    if class == "GSUB" || class == "GPOS" {
        c.graph.get_mut(root)?.dont_share = true;
    }
    c.table(class, element, root, &mut State::default(), true, 0)?;
    c.graph.assemble(root)
}

impl<'a> Compiler<'a> {
    pub(crate) fn new(ctx: &'a ReadContext, font: &SfntFont) -> Self {
        let fvar_axes = font.get(*b"fvar").and_then(|d| Some(i64::from(u16::from_be_bytes([*d.get(8)?, *d.get(9)?]))));
        Self { ctx, fvar_axes, graph: Graph::default(), struct_length: None }
    }

    /// Table `class` compiled from `e` on its own, as bytes: the values of AAT lookups, which
    /// fontTools compares before it writes them.
    fn detached(&self, class: &str, e: &Element, state: &State, depth: usize) -> Result<Vec<u8>> {
        let mut c = Compiler { ctx: self.ctx, fvar_axes: self.fvar_axes, graph: Graph::default(), struct_length: None };
        let root = c.graph.node();
        c.table(class, e, root, &mut state.clone(), false, depth + 1)?;
        c.graph.assemble(root)
    }

    /// The bytes the items of `node` from `from` on take up.
    fn written_since(&self, node: usize, from: usize) -> usize {
        self.graph.nodes.get(node).map_or(0, |n| {
            n.items
                .iter()
                .skip(from)
                .map(|i| match i {
                    Item::Data(d) => d.len(),
                    Item::Offset { size, .. } | Item::Count { size, .. } => usize::from(*size),
                })
                .sum()
        })
    }

    fn glyph(&self, name: &str) -> Result<u16> {
        let id = self.ctx.glyph_id(name)?;
        u16::try_from(id).map_err(|_| bad(format!("the glyph '{name}' has an index past 65535")))
    }

    fn put(&mut self, node: usize, f: impl FnOnce(&mut Out) -> Result<()>) -> Result<()> {
        let mut o = Out::default();
        f(&mut o)?;
        self.graph.data(node, o.0)
    }

    /// Writes table `class` from `e` into writer `node` (fontTools' `BaseTable.compile`).
    /// `raw` marks an element in the raw layout, already past its summarised form.
    fn table(&mut self, class: &str, e: &Element, node: usize, state: &mut State, raw: bool, depth: usize) -> Result<()> {
        if depth > MAX_TTX_DEPTH + 4 {
            return Err(TablesError::TooLarge { what: "the nesting of subtables", limit: MAX_TTX_DEPTH + 4 });
        }
        if !raw {
            match class {
                "Coverage" => return self.coverage(e, node, state, depth),
                "ClassDef" => return self.class_def(e, node, state, depth),
                "SingleSubst" | "MultipleSubst" | "AlternateSubst" | "LigatureSubst" => return self.simple_subst(class, e, node, state, depth),
                "DeltaSetIndexMap" | "VarIdxMap" => return self.index_map(class, e, node, state, depth),
                "ClipList" => return self.clip_list(e, node, state, depth),
                "BaseGlyphRecordArray" | "BaseGlyphList" => {
                    let sorted = self.sorted_base_glyphs(e)?;
                    return self.table(class, &sorted, node, state, true, depth);
                }
                _ => {}
            }
        }
        if let Some(t) = lookup_type(class) {
            match state.get("LookupType") {
                Some(St::Slot(s)) => {
                    let s = *s;
                    self.graph.set_slot(s, t, "LookupType")?;
                }
                _ => return Err(bad(format!("the subtable '{class}' is outside a lookup"))),
            }
        }
        if class.starts_with("FeatureParams") {
            let tag = match state.get("FeatureTag") {
                Some(St::Tag(t)) => t.clone(),
                _ => return Err(bad("feature parameters outside a feature record")),
            };
            if feature_params_class(&tag) != Some(class) {
                return Err(bad(format!("the feature '{tag}' cannot have parameters of the kind '{class}'")));
            }
        }
        let def = crate::ttx::otdata::layout_def(class).ok_or_else(|| bad(format!("no layout for the table '{class}'")))?;
        let start = self.graph.nodes.get(node).map_or(0, |n| n.items.len());
        let outer_length = self.struct_length.take();
        let fields: &[Field] = match def.layout {
            Layout::Plain(fields) => fields,
            Layout::Formats(width, formats) => {
                let format = int_lit(e.attribute("Format").ok_or_else(|| bad(format!("the table '{class}' has no format")))?)?;
                match width {
                    Width::U8 => self.put(node, |o| o.u8(format, "Format"))?,
                    Width::U16 => self.put(node, |o| o.u16(format, "Format"))?,
                }
                formats.iter().find(|(n, _)| i64::from(*n) == format).map_or(&[][..], |(_, f)| *f)
            }
        };
        let written = self.fields(class, fields, e, node, state, depth);
        let length = std::mem::replace(&mut self.struct_length, outer_length);
        written?;
        if let Some(slot) = length {
            let n = i64::try_from(self.written_since(node, start)).unwrap_or(i64::MAX);
            self.graph.set_slot(slot, n, "StructLength")?;
        }
        Ok(())
    }

    /// The elements of `e` that field `f` reads.
    fn matching<'e>(f: &Field, e: &'e Element) -> Vec<&'e Element> {
        e.children.iter().filter(|c| Self::matches(f, &c.name)).collect()
    }

    fn matches(f: &Field, name: &str) -> bool {
        match f.ty {
            Ty::SubTable | Ty::ExtSubTable => lookup_type(name).is_some(),
            Ty::FeatureParams => FEATURE_PARAMS.contains(&name),
            _ => f.name == name,
        }
    }

    /// The value of the last element `name` of `e`, as a number (a table version as such).
    fn xml_number(e: &Element, name: &str) -> Result<i64> {
        let child = e.children.iter().rev().find(|c| c.name == name).ok_or_else(|| bad(format!("the table '{}' has no '{name}'", e.name)))?;
        let text = value_of(child)?;
        if name == "Version" { version_value(text) } else { int_lit(text) }
    }

    fn holds(cond: Cond, e: &Element) -> Result<bool> {
        Ok(match cond {
            Cond::Always => true,
            Cond::Mask(name, mask) => Self::xml_number(e, name)? & i64::from(mask) != 0,
            Cond::AtLeast(name, v) => Self::xml_number(e, name)? >= v,
            Cond::In123(name) => (1..=3).contains(&Self::xml_number(e, name)?),
        })
    }

    fn fields(&mut self, class: &str, fields: &[Field], e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        if let Some(stray) = e.children.iter().find(|c| !fields.iter().any(|f| Self::matches(f, &c.name))) {
            return Err(bad(format!("the table '{class}' has no field '{}'", stray.name)));
        }
        // The counts of this table, by name.
        let mut counts: Vec<(&'static str, usize)> = Vec::new();
        // `COLR.preWrite`: the layer count follows the offset of the array it counts.
        let layer_count = if class == "COLR" {
            let slot = self.graph.slot(None);
            state.set("LayerRecordCount", St::Slot(slot));
            Some(slot)
        } else {
            None
        };
        for f in fields {
            if f.name == "StructLength" {
                let slot = self.graph.slot(None);
                let size = if matches!(f.ty, Ty::Count16 | Ty::U16) { 2 } else { 4 };
                self.graph.push(node, Item::Count { slot, size })?;
                self.struct_length = Some(slot);
                continue;
            }
            if let Rep::Count(repeat, adjust) = f.rep {
                let items = Self::matching(f, e);
                let n = i64::try_from(items.len()).unwrap_or(i64::MAX).saturating_sub(i64::from(adjust));
                let slot = match counts.iter().rev().find(|(name, _)| *name == repeat) {
                    Some((_, s)) => Some(*s),
                    None => match state.get(repeat) {
                        Some(St::Slot(s)) => Some(*s),
                        _ => None,
                    },
                };
                match slot {
                    Some(s) => self.graph.set_slot(s, n, repeat)?,
                    None => return Err(bad(format!("the table '{class}' needs '{repeat}', which no table gives"))),
                }
                self.array(class, f, &items, e, node, state, &counts, depth)?;
                continue;
            }
            if is_count(f.name) {
                let preset = if class == "VarRegionList" && f.name == "RegionAxisCount" { self.fvar_axes } else { None };
                let slot = match layer_count {
                    Some(slot) if f.name == "LayerRecordCount" => slot,
                    _ => self.graph.slot(preset),
                };
                let size = match f.ty {
                    Ty::Count8 | Ty::U8 => 1,
                    Ty::Count32 | Ty::U32 => 4,
                    _ => 2,
                };
                self.graph.push(node, Item::Count { slot, size })?;
                counts.push((f.name, slot));
                if let Some(name) = PROPAGATED.iter().find(|n| **n == f.name) {
                    state.set(name, St::Slot(slot));
                }
                continue;
            }
            if f.name.ends_with("LookupType") {
                let given = match e.children.iter().rev().find(|c| c.name == f.name) {
                    Some(c) => Some(int_lit(value_of(c)?)?),
                    None => None,
                };
                let slot = self.graph.slot(given);
                self.graph.push(node, Item::Count { slot, size: 2 })?;
                counts.push((f.name, slot));
                state.set("LookupType", St::Slot(slot));
                continue;
            }
            if !Self::holds(f.cond, e)? {
                continue;
            }
            let child = e.children.iter().rev().find(|c| Self::matches(f, &c.name));
            self.single(class, f, child, e, node, state, &counts, depth)?;
        }
        Ok(())
    }

    /// Writes an array field.
    #[allow(clippy::too_many_arguments)]
    fn array(
        &mut self,
        class: &str,
        f: &Field,
        items: &[&Element],
        e: &Element,
        node: usize,
        state: &mut State,
        counts: &[(&'static str, usize)],
        depth: usize,
    ) -> Result<()> {
        // The number types fontTools writes as one array.
        let one = matches!(f.ty, Ty::U8 | Ty::U16 | Ty::NameId | Ty::StatFlags | Ty::LookupFlag | Ty::I16 | Ty::U32 | Ty::Glyph);
        if one {
            let mut o = Out::default();
            for item in items {
                let text = value_of(item)?;
                match f.ty {
                    Ty::U8 => o.u8(int_lit(text)?, f.name)?,
                    Ty::I16 => o.i16(int_lit(text)?, f.name)?,
                    Ty::U32 => o.u32(int_lit(text)?, f.name)?,
                    Ty::Glyph => o.u16(i64::from(self.glyph(text)?), f.name)?,
                    _ => o.u16(int_lit(text)?, f.name)?,
                }
            }
            return self.graph.data(node, o.0);
        }
        for item in items {
            self.single(class, f, Some(item), e, node, state, counts, depth)?;
        }
        Ok(())
    }

    /// Writes one value of field `f` from `child` (`None` when the table has no such element);
    /// `e` is the table holding it.
    #[allow(clippy::too_many_arguments)]
    fn single(
        &mut self,
        class: &str,
        f: &Field,
        child: Option<&Element>,
        e: &Element,
        node: usize,
        state: &mut State,
        counts: &[(&'static str, usize)],
        depth: usize,
    ) -> Result<()> {
        let missing = || bad(format!("the table '{class}' has no '{}'", f.name));
        let name = f.name;
        match f.ty {
            Ty::Off16(_) | Ty::Off32(_) | Ty::Off24(_) | Ty::SubTable | Ty::ExtSubTable | Ty::FeatureParams => {
                let size: u8 = match f.ty {
                    Ty::Off32(_) | Ty::ExtSubTable => 4,
                    Ty::Off24(_) => 3,
                    _ => 2,
                };
                if matches!(f.ty, Ty::ExtSubTable) {
                    self.graph.get_mut(node)?.extension = true;
                }
                let target = match (f.ty, child) {
                    (_, None) => None,
                    (_, Some(c)) if c.attribute("empty").is_some_and(|v| int_lit(v).map(|n| n != 0).unwrap_or(false)) => None,
                    (Ty::Off16(t) | Ty::Off32(t) | Ty::Off24(t), Some(c)) => Some((t, c)),
                    (_, Some(c)) => Some((c.name.as_str(), c)),
                };
                let Some((target, c)) = target else {
                    return self.graph.data(node, vec![0; usize::from(size)]);
                };
                let sub = self.graph.node();
                self.graph.get_mut(sub)?.is_coverage = name == "Coverage";
                self.graph.push(node, Item::Offset { size, node: sub })?;
                let mut sub_state = state.clone();
                self.table(target, c, sub, &mut sub_state, false, depth + 1)
            }
            Ty::Struct(target) => {
                let c = child.ok_or_else(missing)?;
                self.table(target, c, node, state, false, depth + 1)
            }
            Ty::AatLookup(value) => {
                let c = child.ok_or_else(missing)?;
                self.aat_lookup(value, c, node, state, depth)
            }
            Ty::AatLookupData(table) => {
                let c = child.ok_or_else(missing)?;
                self.aat_lookup_data(table, c, node, state, depth)
            }
            Ty::CidGlyphMap => self.cid_glyph_map(child.ok_or_else(missing)?, node),
            Ty::GlyphCidMap => self.glyph_cid_map(child.ok_or_else(missing)?, node),
            Ty::VarIndex if child.is_some_and(|c| c.attribute("value").is_none()) => self.put(node, |o| o.u32(0xFFFF_FFFF, name)),
            Ty::ValueRecord => {
                let which = if name.ends_with('2') { "ValueFormat2" } else { "ValueFormat1" };
                let format = match state.get(which) {
                    Some(St::Format(v)) => *v,
                    _ => return Err(bad(format!("the table '{class}' has a value record before its value format"))),
                };
                self.value_record(format, child, node, state, depth)
            }
            Ty::DeltaValue => {
                let values = int_list(value_of(child.ok_or_else(missing)?)?)?;
                let (start, end, format) = (Self::xml_number(e, "StartSize")?, Self::xml_number(e, "EndSize")?, Self::xml_number(e, "DeltaFormat")?);
                if !(1..=3).contains(&format) {
                    return Err(bad("a device table with an unknown delta format"));
                }
                if i64::try_from(values.len()).ok() != Some(end.saturating_sub(start).saturating_add(1)) {
                    return Err(bad("a device table whose deltas do not match its sizes"));
                }
                let bits = 1u32 << format;
                let mask = (1i64 << bits) - 1;
                let (mut word, mut shift) = (0i64, 16u32);
                for v in values {
                    shift -= bits;
                    word |= (v & mask) << shift;
                    if shift == 0 {
                        self.put(node, |o| o.u16(word, "a delta"))?;
                        (word, shift) = (0, 16);
                    }
                }
                if shift != 16 {
                    self.put(node, |o| o.u16(word, "a delta"))?;
                }
                Ok(())
            }
            Ty::VarDataValue => {
                let values = int_list(value_of(child.ok_or_else(missing)?)?)?;
                let regions = counts
                    .iter()
                    .rev()
                    .find(|(n, _)| *n == "VarRegionCount")
                    .and_then(|(_, s)| self.graph.slot_value(*s))
                    .ok_or_else(|| bad("variation data without a region count"))?;
                let words = Self::xml_number(e, "NumShorts")?;
                let long = words & 0x8000 != 0;
                let words = words & 0x7FFF;
                let (n1, n2) = (regions.min(words), regions.max(words));
                if n2 > regions {
                    return Err(bad("variation data with more word deltas than regions"));
                }
                let n1 = usize::try_from(n1).unwrap_or(0);
                let regions = usize::try_from(regions).unwrap_or(0);
                let big: Vec<i64> = values.iter().take(n1).copied().collect();
                let small: Vec<i64> = values.iter().take(regions).skip(n1).copied().collect();
                self.put(node, |o| big.iter().try_for_each(|v| if long { o.i32(*v, "a delta") } else { o.i16(*v, "a delta") }))?;
                self.put(node, |o| small.iter().try_for_each(|v| if long { o.i16(*v, "a delta") } else { o.i8(*v, "a delta") }))
            }
            Ty::VarIdxMapValue => {
                let mapping = int_list(value_of(child.ok_or_else(missing)?)?)?;
                let n = i64::try_from(mapping.len()).unwrap_or(i64::MAX);
                let slot = counts.iter().rev().find(|(c, _)| *c == "MappingCount").map(|(_, s)| *s).or(match state.get("MappingCount") {
                    Some(St::Slot(s)) => Some(*s),
                    _ => None,
                });
                let slot = slot.ok_or_else(|| bad("a delta-set index map without a count"))?;
                self.graph.set_slot(slot, n, "MappingCount")?;
                let format = Self::xml_number(e, "EntryFormat")?;
                let inner_bits = 1 + (format & 0x000F);
                let inner_mask = (1i64 << inner_bits) - 1;
                let outer_shift = 16 - inner_bits;
                let size = 1 + ((format & 0x0030) >> 4);
                let packed: Vec<i64> = mapping.iter().map(|idx| ((idx & 0xFFFF_0000) >> outer_shift) | (idx & inner_mask)).collect();
                if size == 3 {
                    for v in packed {
                        self.put(node, |o| o.u24(v, "a delta-set index"))?;
                    }
                    return Ok(());
                }
                self.put(node, |o| {
                    packed.iter().try_for_each(|v| match size {
                        1 => o.u8(*v, "a delta-set index"),
                        2 => o.u16(*v, "a delta-set index"),
                        _ => o.u32(*v, "a delta-set index"),
                    })
                })
            }
            _ => {
                let text = value_of(child.ok_or_else(missing)?)?;
                self.number(f, text, node, state)
            }
        }
    }

    /// Writes a number field from its text.
    fn number(&mut self, f: &Field, text: &str, node: usize, state: &mut State) -> Result<()> {
        let name = f.name;
        match f.ty {
            Ty::Tag => {
                let mut bytes = Vec::with_capacity(4);
                for c in text.chars() {
                    bytes.push(u8::try_from(u32::from(c)).map_err(|_| bad(format!("the tag '{text}' is not Latin-1")))?);
                }
                if bytes.len() != 4 {
                    return Err(bad(format!("the tag '{text}' is not four characters long")));
                }
                if name == "FeatureTag" {
                    state.set("FeatureTag", St::Tag(text.to_string()));
                }
                self.graph.data(node, bytes)
            }
            Ty::Glyph => {
                let id = self.glyph(text)?;
                self.graph.data(node, id.to_be_bytes().to_vec())
            }
            Ty::DeciPoints => {
                let v = (float_lit(text)? * 10.0).round_ties_even();
                self.put(node, |o| o.u16(ot_round(v)?, name))
            }
            Ty::Fixed => {
                let v = ot_round(float_lit(text)? * 65536.0)?;
                self.put(node, |o| o.i32(v, name))
            }
            Ty::F2Dot14 => {
                let v = ot_round(float_lit(text)? * 16384.0)?;
                self.put(node, |o| o.i16(v, name))
            }
            Ty::Version => {
                let v = version_value(text)?;
                self.put(node, |o| o.i32(v, name))
            }
            Ty::Angle | Ty::BiasedAngle => {
                // `Angle.fromString` snaps to a multiple of the step, `toInt` stores the fraction
                // of 180 degrees less the bias.
                let step = 1.0 / 16384.0 * 180.0;
                let bias = if matches!(f.ty, Ty::BiasedAngle) { 1.0 } else { 0.0 };
                let snapped = ot_round(float_lit(text)? / step)? as f64 * step;
                let v = ot_round((snapped / 180.0 - bias) * 16384.0)?;
                self.put(node, |o| o.i16(v, name))
            }
            Ty::ExtendMode | Ty::CompositeMode => {
                let wanted = text.to_uppercase();
                let v = (0..=255i64)
                    .find(|v| crate::ttx::otdata::engine::enum_name(f.ty, *v).is_some_and(|n| n.to_uppercase() == wanted))
                    .ok_or_else(|| bad(format!("'{text}' is not a value of {name}")))?;
                self.put(node, |o| o.u8(v, name))
            }
            Ty::Char64 => {
                // ASCII with `?` for anything else, cut or padded with zeros to 64 bytes.
                let mut bytes: Vec<u8> = text.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).take(64).collect();
                bytes.resize(64, 0);
                self.graph.data(node, bytes)
            }
            _ => {
                let v = int_lit(text)?;
                if matches!(f.ty, Ty::ValueFormat) {
                    state.set(if name.ends_with('2') { "ValueFormat2" } else { "ValueFormat1" }, St::Format(v));
                }
                match f.ty {
                    Ty::U8 | Ty::Count8 => self.put(node, |o| o.u8(v, name)),
                    Ty::I16 => self.put(node, |o| o.i16(v, name)),
                    Ty::U24 => self.put(node, |o| o.u24(v, name)),
                    Ty::U32 | Ty::Count32 | Ty::VarIndex => self.put(node, |o| o.u32(v, name)),
                    _ => self.put(node, |o| o.u16(v, name)),
                }
            }
        }
    }

    /// A value record (fontTools' `ValueRecordFactory.writeValueRecord`): each field its format
    /// has, from the element's attributes (0 when absent) and device elements.
    fn value_record(&mut self, format: i64, child: Option<&Element>, node: usize, state: &State, depth: usize) -> Result<()> {
        for (bit, &(name, device, signed)) in VALUE_RECORD.iter().enumerate() {
            if format & (1 << bit) == 0 {
                continue;
            }
            if device {
                let dev = child.and_then(|c| c.children.iter().rev().find(|d| d.name == name));
                match dev {
                    Some(d) => {
                        let sub = self.graph.node();
                        self.graph.push(node, Item::Offset { size: 2, node: sub })?;
                        // The element's attributes are not read: only its fields.
                        let device = el("Device", Vec::new(), d.children.clone());
                        self.table("Device", &device, sub, &mut state.clone(), true, depth + 1)?;
                    }
                    None => self.graph.data(node, vec![0, 0])?,
                }
                continue;
            }
            let v = match child.and_then(|c| c.attribute(name)) {
                Some(text) => int_lit(text)?,
                None => 0,
            };
            if signed {
                self.put(node, |o| o.i16(v, name))?;
            } else {
                self.put(node, |o| o.u16(v, name))?;
            }
        }
        Ok(())
    }

    // ── The summarised forms ───────────────────────────────────────────────────────────────────

    /// The glyph IDs of a list of names.
    fn ids(&self, names: &[String]) -> Result<Vec<u32>> {
        names.iter().map(|n| self.glyph(n).map(u32::from)).collect()
    }

    /// `Coverage.preWrite`: format 1 unless ranges are smaller or the glyphs are out of order.
    fn coverage(&mut self, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        let glyphs: Vec<String> = e.children.iter().map(|c| value_of(c).map(str::to_string)).collect::<Result<_>>()?;
        let raw = self.coverage_raw(&glyphs)?;
        self.table("Coverage", &raw, node, state, true, depth)
    }

    fn coverage_raw(&self, glyphs: &[String]) -> Result<Element> {
        let ids = self.ids(glyphs)?;
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for &id in &ids {
            match ranges.last_mut() {
                Some((_, end)) if id == end.wrapping_add(1) => *end = id,
                _ => ranges.push((id, id)),
            }
        }
        let broken = ids.windows(2).any(|w| matches!(w, [a, b] if a > b));
        if !glyphs.is_empty() && (broken || ranges.len() * 3 < glyphs.len()) {
            let mut records: Vec<(u32, Element)> = Vec::with_capacity(ranges.len());
            let mut index = 0u32;
            for (start, end) in ranges {
                let name = |id: u32| super::super::core::glyph_name(self.ctx.order().unwrap_or(&[]), id);
                records.push((
                    start,
                    el(
                        "RangeRecord",
                        Vec::new(),
                        vec![val("Start", name(start)), val("End", name(end)), val("StartCoverageIndex", index.to_string())],
                    ),
                ));
                index = index.wrapping_add(end - start + 1);
            }
            if broken {
                records.sort_by_key(|(start, _)| *start);
            }
            return Ok(el("Coverage", vec![("Format", "2".into())], records.into_iter().map(|(_, r)| r).collect()));
        }
        Ok(el("Coverage", vec![("Format", "1".into())], glyphs.iter().map(|g| val("GlyphArray", g.clone())).collect()))
    }

    /// `ClassDef.preWrite`: ranges unless a class array is smaller.
    fn class_def(&mut self, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        let mut classes: Ordered<i64> = Ordered::new();
        for c in &e.children {
            let glyph = c.attribute("glyph").ok_or_else(|| bad("a class definition without a glyph"))?;
            let class = int_lit(c.attribute("class").ok_or_else(|| bad("a class definition without a class"))?)?;
            classes.insert(glyph, class);
        }
        let mut items: Vec<(u32, String, i64)> = Vec::new();
        for (name, class) in classes.items {
            if class != 0 {
                items.push((u32::from(self.glyph(&name)?), name, class));
            }
        }
        items.sort();
        // (class, start, start name, end, end name)
        let mut ranges: Vec<(i64, u32, String, u32, String)> = Vec::new();
        for (id, name, class) in items {
            match ranges.last_mut() {
                Some(r) if id == r.3.wrapping_add(1) && class == r.0 => {
                    r.3 = id;
                    r.4 = name;
                }
                _ => ranges.push((class, id, name.clone(), id, name)),
            }
        }
        let raw = match (ranges.first(), ranges.last()) {
            (Some(first), Some(last)) => {
                let (start, end) = (first.1, last.3);
                let count = usize::try_from(end.saturating_sub(start)).unwrap_or(usize::MAX).saturating_add(1);
                if ranges.len() * 3 < count.saturating_add(1) {
                    let records = ranges
                        .iter()
                        .map(|(class, _, s, _, e)| {
                            el("ClassRangeRecord", Vec::new(), vec![val("Start", s.clone()), val("End", e.clone()), val("Class", class.to_string())])
                        })
                        .collect();
                    el("ClassDef", vec![("Format", "2".into())], records)
                } else {
                    if count > MAX_LAYOUT_ITEMS {
                        return Err(TablesError::TooLarge { what: "the glyphs of a class definition table", limit: MAX_LAYOUT_ITEMS });
                    }
                    let mut values = vec![0i64; count];
                    for (class, s, _, e, _) in &ranges {
                        for id in *s..=*e {
                            if let Some(slot) = usize::try_from(id - start).ok().and_then(|i| values.get_mut(i)) {
                                *slot = *class;
                            }
                        }
                    }
                    let mut children = vec![val("StartGlyph", first.2.clone())];
                    children.extend(values.into_iter().map(|v| val("ClassValueArray", v.to_string())));
                    el("ClassDef", vec![("Format", "1".into())], children)
                }
            }
            _ => el("ClassDef", vec![("Format", "2".into())], Vec::new()),
        };
        self.table("ClassDef", &raw, node, state, true, depth)
    }

    fn glyph_list(name: &str, glyphs: &[String]) -> Vec<Element> {
        glyphs.iter().map(|g| val(name, g.clone())).collect()
    }

    fn simple_subst(&mut self, class: &str, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        let coverage = |glyphs: &[String]| el("Coverage", Vec::new(), Self::glyph_list("Glyph", glyphs));
        let raw = match class {
            "SingleSubst" => {
                let mut mapping: Ordered<String> = Ordered::new();
                for c in &e.children {
                    let (i, o) = (c.attribute("in"), c.attribute("out"));
                    mapping.insert(
                        i.ok_or_else(|| bad("a substitution without 'in'"))?,
                        o.ok_or_else(|| bad("a substitution without 'out'"))?.to_string(),
                    );
                }
                let mut items: Vec<((u32, u32), (String, String))> = Vec::with_capacity(mapping.items.len());
                for (i, o) in mapping.items {
                    items.push(((u32::from(self.glyph(&i)?), u32::from(self.glyph(&o)?)), (i, o)));
                }
                let mut delta: Option<u32> = None;
                let mut uniform = true;
                for ((i, o), _) in &items {
                    let d = *delta.get_or_insert(o.wrapping_sub(*i) % 65536);
                    if (i + d) % 65536 != *o {
                        uniform = false;
                        break;
                    }
                }
                items.sort();
                let input: Vec<String> = items.iter().map(|(_, (i, _))| i.clone()).collect();
                match delta.filter(|_| uniform) {
                    Some(d) => el("SingleSubst", vec![("Format", "1".into())], vec![coverage(&input), val("DeltaGlyphID", d.to_string())]),
                    None => {
                        let mut children = vec![coverage(&input)];
                        children.extend(items.iter().map(|(_, (_, o))| val("Substitute", o.clone())));
                        el("SingleSubst", vec![("Format", "2".into())], children)
                    }
                }
            }
            "MultipleSubst" => {
                let mut mapping: Ordered<Vec<String>> = Ordered::new();
                let mut old_coverage: Vec<String> = Vec::new();
                for c in &e.children {
                    match c.name.as_str() {
                        "Coverage" => {
                            old_coverage =
                                c.children.iter().filter(|g| g.name == "Glyph").map(|g| value_of(g).map(str::to_string)).collect::<Result<_>>()?;
                        }
                        "Sequence" => {
                            let index = match c.attribute("index") {
                                Some(i) => usize::try_from(int_lit(i)?).map_err(|_| bad("a sequence with a negative index"))?,
                                None => mapping.items.len(),
                            };
                            let glyph = old_coverage.get(index).ok_or_else(|| bad("a sequence past the end of its coverage"))?.clone();
                            let out = c
                                .children
                                .iter()
                                .filter(|s| s.name == "Substitute")
                                .map(|s| value_of(s).map(str::to_string))
                                .collect::<Result<_>>()?;
                            mapping.insert(&glyph, out);
                        }
                        _ => {
                            let out = c.attribute("out").ok_or_else(|| bad("a substitution without 'out'"))?;
                            let out: Vec<String> = if out.is_empty() { Vec::new() } else { out.split(',').map(|g| g.trim().to_string()).collect() };
                            mapping.insert(c.attribute("in").ok_or_else(|| bad("a substitution without 'in'"))?, out);
                        }
                    }
                }
                let mut keyed: Vec<(u32, String, Vec<String>)> = Vec::with_capacity(mapping.items.len());
                for (g, out) in mapping.items {
                    keyed.push((u32::from(self.glyph(&g)?), g, out));
                }
                keyed.sort_by_key(|(id, _, _)| *id);
                let input: Vec<String> = keyed.iter().map(|(_, g, _)| g.clone()).collect();
                let mut children = vec![coverage(&input)];
                children.extend(keyed.into_iter().map(|(_, _, out)| el("Sequence", Vec::new(), Self::glyph_list("Substitute", &out))));
                el("MultipleSubst", vec![("Format", "1".into())], children)
            }
            "AlternateSubst" => {
                let mut sets: Ordered<Vec<String>> = Ordered::new();
                for c in &e.children {
                    let glyph = c.attribute("glyph").ok_or_else(|| bad("a set of alternates without a glyph"))?;
                    let alts = c
                        .children
                        .iter()
                        .map(|a| a.attribute("glyph").map(str::to_string).ok_or_else(|| bad("an alternate without a glyph")))
                        .collect::<Result<_>>()?;
                    sets.insert(glyph, alts);
                }
                let mut keyed: Vec<(u32, String, Vec<String>)> = Vec::with_capacity(sets.items.len());
                for (g, set) in sets.items {
                    keyed.push((u32::from(self.glyph(&g)?), g, set));
                }
                keyed.sort();
                let input: Vec<String> = keyed.iter().map(|(_, g, _)| g.clone()).collect();
                let mut children = vec![coverage(&input)];
                children.extend(keyed.into_iter().map(|(_, _, set)| el("AlternateSet", Vec::new(), Self::glyph_list("Alternate", &set))));
                el("AlternateSubst", vec![("Format", "1".into())], children)
            }
            _ => {
                let mut sets: Ordered<Vec<(String, Vec<String>)>> = Ordered::new();
                for c in &e.children {
                    let glyph = c.attribute("glyph").ok_or_else(|| bad("a ligature set without a glyph"))?;
                    let mut ligs = Vec::with_capacity(c.children.len());
                    for l in &c.children {
                        let lig = l.attribute("glyph").ok_or_else(|| bad("a ligature without a glyph"))?.to_string();
                        let comps = l.attribute("components").ok_or_else(|| bad("a ligature without components"))?;
                        let comps: Vec<String> = if comps.is_empty() { Vec::new() } else { comps.split(',').map(str::to_string).collect() };
                        ligs.push((lig, comps));
                    }
                    sets.insert(glyph, ligs);
                }
                let mut keyed = Vec::with_capacity(sets.items.len());
                for (g, set) in sets.items {
                    keyed.push((u32::from(self.glyph(&g)?), g, set));
                }
                keyed.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
                let input: Vec<String> = keyed.iter().map(|(_, g, _)| g.clone()).collect();
                let mut children = vec![coverage(&input)];
                for (_, _, set) in keyed {
                    let ligatures = set
                        .into_iter()
                        .map(|(lig, comps)| {
                            let mut c = vec![val("LigGlyph", lig)];
                            c.extend(Self::glyph_list("Component", &comps));
                            el("Ligature", Vec::new(), c)
                        })
                        .collect();
                    children.push(el("LigatureSet", Vec::new(), ligatures));
                }
                el("LigatureSubst", vec![("Format", "1".into())], children)
            }
        };
        self.table(class, &raw, node, state, true, depth)?;
        if class == "AlternateSubst" || class == "LigatureSubst" {
            self.graph.get_mut(node)?.sort_coverage_last = true;
        }
        Ok(())
    }

    /// `DeltaSetIndexMap.preWrite` and `VarIdxMap.preWrite`.
    fn index_map(&mut self, class: &str, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        let mut mapping: Vec<i64> = Vec::new();
        let mut moved = 0usize;
        if class == "DeltaSetIndexMap" {
            for m in &e.children {
                let index = int_lit(m.attribute("index").ok_or_else(|| bad("a map entry without an index"))?)?;
                let outer = int_lit(m.attribute("outer").unwrap_or("0xFFFF"))?;
                let inner = int_lit(m.attribute("inner").unwrap_or("0xFFFF"))?;
                if !(0..=0xFFFF).contains(&inner) || !(0..=0xFFFF).contains(&outer) {
                    return Err(bad("a delta-set index outside 16 bits"));
                }
                if mapping.len() >= MAX_LAYOUT_ITEMS {
                    return Err(TablesError::TooLarge { what: "the entries of a delta-set index map", limit: MAX_LAYOUT_ITEMS });
                }
                // Python's `list.insert`: past the end appends, a negative index counts from it.
                // Entries out of order move the ones after them; that work is capped.
                let len = i64::try_from(mapping.len()).unwrap_or(i64::MAX);
                let at = if index < 0 { (len + index).max(0) } else { index.min(len) };
                moved = moved.saturating_add(usize::try_from(len - at).unwrap_or(0));
                if moved > MAX_LAYOUT_ITEMS {
                    return Err(TablesError::TooLarge { what: "the reordering of a delta-set index map", limit: MAX_LAYOUT_ITEMS });
                }
                mapping.insert(usize::try_from(at).unwrap_or(0), (outer << 16) | inner);
            }
        } else {
            let order = self.ctx.order()?;
            let mut by_glyph: HashMap<&str, i64> = HashMap::new();
            for m in &e.children {
                let glyph = match m.attribute("glyph") {
                    Some(g) => g,
                    None => {
                        let i =
                            usize::try_from(int_lit(m.attribute("index").ok_or_else(|| bad("a map entry without a glyph"))?)?).unwrap_or(usize::MAX);
                        order.get(i).map(String::as_str).ok_or_else(|| bad("a map entry past the glyph order"))?
                    }
                };
                let outer = int_lit(m.attribute("outer").ok_or_else(|| bad("a map entry without 'outer'"))?)?;
                let inner = int_lit(m.attribute("inner").ok_or_else(|| bad("a map entry without 'inner'"))?)?;
                if !(0..=0xFFFF).contains(&inner) || !(0..=0xFFFF).contains(&outer) {
                    return Err(bad("a delta-set index outside 16 bits"));
                }
                by_glyph.insert(glyph, (outer << 16) | inner);
            }
            for g in order {
                mapping.push(*by_glyph.get(g.as_str()).ok_or_else(|| bad(format!("the glyph '{g}' is missing from a variation index map")))?);
            }
            while mapping.len() > 1 && mapping.get(mapping.len() - 2) == mapping.last() {
                mapping.pop();
            }
        }
        let entry_format = entry_format(&mapping)?;
        let list = format!("[{}]", mapping.iter().map(i64::to_string).collect::<Vec<_>>().join(", "));
        let raw = if class == "DeltaSetIndexMap" {
            let format = if mapping.len() > 0xFFFF { "1" } else { "0" };
            el("DeltaSetIndexMap", vec![("Format", format.into())], vec![val("EntryFormat", entry_format.to_string()), val("mapping", list)])
        } else {
            el("VarIdxMap", Vec::new(), vec![val("EntryFormat", entry_format.to_string()), val("mapping", list)])
        };
        self.table(class, &raw, node, state, true, depth)
    }

    // ── COLR ───────────────────────────────────────────────────────────────────────────────────

    /// `BaseGlyphRecordArray.preWrite` and `BaseGlyphList.preWrite`: the records sorted by the
    /// index of their base glyph, records of one glyph keeping their order.
    fn sorted_base_glyphs(&self, e: &Element) -> Result<Element> {
        let mut keyed: Vec<(u16, Element)> = Vec::with_capacity(e.children.len());
        let mut others = Vec::new();
        for c in &e.children {
            if c.name == "BaseGlyphRecord" || c.name == "BaseGlyphPaintRecord" {
                let glyph =
                    c.children.iter().rev().find(|g| g.name == "BaseGlyph").ok_or_else(|| bad(format!("a '{}' without a base glyph", c.name)))?;
                keyed.push((self.glyph(value_of(glyph)?)?, c.clone()));
            } else {
                others.push(c.clone());
            }
        }
        keyed.sort_by_key(|(id, _)| *id);
        others.extend(keyed.into_iter().map(|(_, c)| c));
        Ok(Element { name: e.name.clone(), attributes: e.attributes.clone(), children: others, text: String::new() })
    }

    /// `ClipList.fromXML` and `preWrite`: the glyphs that share a clip box, as ranges of
    /// consecutive glyph indices, each range a record pointing at the box.
    fn clip_list(&mut self, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        // Glyph name to clip box, in the order the names first appear.
        let mut clips: Ordered<Element> = Ordered::new();
        for clip in e.children.iter().filter(|c| c.name == "Clip") {
            let Some(clip_box) = clip.children.iter().rev().find(|c| c.name == "ClipBox") else { continue };
            for g in clip.children.iter().filter(|c| c.name == "Glyph") {
                if clips.items.len() >= MAX_LAYOUT_ITEMS {
                    return Err(TablesError::TooLarge { what: "the glyphs of a clip list", limit: MAX_LAYOUT_ITEMS });
                }
                clips.insert(value_of(g)?, clip_box.clone());
            }
        }
        // `groups`: glyphs by the content of their box, the first box of each kind kept.
        let mut groups: Vec<(Vec<u8>, Element, Vec<String>)> = Vec::new();
        let mut by_key: HashMap<Vec<u8>, usize> = HashMap::new();
        for (glyph, clip_box) in &clips.items {
            let key = self.detached("ClipBox", clip_box, state, depth)?;
            match by_key.get(&key).and_then(|&i| groups.get_mut(i)) {
                Some(group) => group.2.push(glyph.clone()),
                None => {
                    by_key.insert(key.clone(), groups.len());
                    groups.push((key, clip_box.clone(), vec![glyph.clone()]));
                }
            }
        }
        let mut ranges: Vec<(u32, u32, Element)> = Vec::new();
        for (_, clip_box, glyphs) in &groups {
            // Names that are not in the glyph order are left out, as the reverse glyph map does.
            let mut ids: Vec<u32> = glyphs.iter().filter_map(|g| self.ctx.known_glyph(g)).collect();
            ids.sort_unstable();
            ids.dedup();
            let mut run: Option<(u32, u32)> = None;
            for id in ids {
                run = match run {
                    Some((start, end)) if id == end.wrapping_add(1) => Some((start, id)),
                    Some((start, end)) => {
                        ranges.push((start, end, clip_box.clone()));
                        Some((id, id))
                    }
                    None => Some((id, id)),
                };
            }
            if let Some((start, end)) = run {
                ranges.push((start, end, clip_box.clone()));
            }
        }
        ranges.sort_by_key(|(start, end, _)| (*start, *end));
        let records = ranges
            .into_iter()
            .map(|(start, end, clip_box)| {
                el("ClipRecord", Vec::new(), vec![val("StartGlyphID", start.to_string()), val("EndGlyphID", end.to_string()), clip_box])
            })
            .collect();
        let format = e.attribute("Format").unwrap_or("1").to_string();
        let raw = el("ClipList", vec![("Format", format)], records);
        self.table("ClipList", &raw, node, state, true, depth)
    }

    // ── AAT ────────────────────────────────────────────────────────────────────────────────────

    /// The `Lookup` children of `e` as glyph index to element, the last element of a glyph
    /// winning (fontTools reads them into a dict).
    fn lookups<'e>(&self, e: &'e Element) -> Result<Vec<(u16, &'e Element)>> {
        let mut by_glyph: HashMap<&str, &Element> = HashMap::new();
        for c in e.children.iter().filter(|c| c.name == "Lookup") {
            if by_glyph.len() >= MAX_LAYOUT_ITEMS {
                return Err(TablesError::TooLarge { what: "the glyphs of an AAT lookup", limit: MAX_LAYOUT_ITEMS });
            }
            by_glyph.insert(c.attribute("glyph").ok_or_else(|| bad("a lookup entry without a glyph"))?, c);
        }
        let mut out: Vec<(u16, &Element)> = by_glyph.into_iter().map(|(g, c)| self.glyph(g).map(|id| (id, c))).collect::<Result<_>>()?;
        out.sort_by_key(|(id, _)| *id);
        Ok(out)
    }

    /// `AATLookup.write`: an AAT lookup from `e`, whose values are numbers (`value` is
    /// `uint16`) or offsets to tables of layout `value`; the format that takes the fewest bytes.
    fn aat_lookup(&mut self, value: &'static str, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        let entries = self.lookups(e)?;
        let mut values: Vec<(u16, AatValue)> = Vec::with_capacity(entries.len());
        for (id, c) in entries {
            let v = if value == "uint16" {
                AatValue::Number(int_lit(value_of(c)?)?)
            } else if c.attribute("empty").is_some_and(|v| int_lit(v).map(|n| n != 0).unwrap_or(false)) {
                AatValue::Table(None)
            } else {
                AatValue::Table(Some(self.detached(value, c, state, depth)?))
            };
            values.push((id, v));
        }
        let glyphs = self.ctx.order()?.len();
        let bytes = aat_lookup_bytes(&values, glyphs)?;
        for piece in bytes {
            match piece {
                Piece::Data(d) => self.graph.data(node, d)?,
                Piece::Table(None) => self.graph.data(node, vec![0, 0])?,
                Piece::Table(Some(d)) => {
                    let sub = self.graph.node();
                    self.graph.data(sub, d)?;
                    self.graph.push(node, Item::Offset { size: 2, node: sub })?;
                }
            }
        }
        Ok(())
    }

    /// `AATLookupWithDataOffset.write` (the anchors of `ankr`): each glyph's table compiled, equal
    /// tables stored once, then a lookup of their offsets and the tables, each behind a 32-bit
    /// offset.
    fn aat_lookup_data(&mut self, table: &'static str, e: &Element, node: usize, state: &mut State, depth: usize) -> Result<()> {
        let entries = self.lookups(e)?;
        let mut data: Vec<u8> = Vec::new();
        let mut at: HashMap<Vec<u8>, i64> = HashMap::new();
        let mut values: Vec<(u16, AatValue)> = Vec::with_capacity(entries.len());
        for (id, c) in entries {
            let bytes = self.detached(table, c, state, depth)?;
            let offset = match at.get(&bytes) {
                Some(o) => *o,
                None => {
                    let o = i64::try_from(data.len()).unwrap_or(i64::MAX);
                    data.extend_from_slice(&bytes);
                    at.insert(bytes, o);
                    o
                }
            };
            values.push((id, AatValue::Number(offset)));
        }
        let glyphs = self.ctx.order()?.len();
        let mut lookup = Vec::new();
        for piece in aat_lookup_bytes(&values, glyphs)? {
            if let Piece::Data(d) = piece {
                lookup.extend(d);
            }
        }
        let (lookup_node, data_node) = (self.graph.node(), self.graph.node());
        self.graph.data(lookup_node, lookup)?;
        self.graph.data(data_node, data)?;
        self.graph.push(node, Item::Offset { size: 4, node: lookup_node })?;
        self.graph.push(node, Item::Offset { size: 4, node: data_node })
    }

    /// `CIDGlyphMap.write`: a glyph index for every CID up to the highest, 0xFFFF for none.
    fn cid_glyph_map(&mut self, e: &Element, node: usize) -> Result<()> {
        let mut by_cid: HashMap<i64, u16> = HashMap::new();
        for c in e.children.iter().filter(|c| c.name == "CID") {
            let cid = int_lit(c.attribute("cid").ok_or_else(|| bad("a CID entry without a CID"))?)?;
            if !(0..0xFFFF).contains(&cid) {
                return Err(bad(format!("the CID {cid} does not fit the map")));
            }
            let glyph = c.attribute("glyph").ok_or_else(|| bad("a CID entry without a glyph"))?.trim();
            by_cid.insert(cid, self.glyph(glyph)?);
        }
        let count = by_cid.keys().max().map_or(0, |m| m + 1);
        self.put(node, |o| {
            o.u16(count, "the number of CIDs")?;
            (0..count).try_for_each(|cid| o.u16(by_cid.get(&cid).map_or(0xFFFF, |g| i64::from(*g)), "a glyph index"))
        })
    }

    /// `GlyphCIDMap.write`: a CID for every glyph up to the highest that has one, 0xFFFF for
    /// none.
    fn glyph_cid_map(&mut self, e: &Element, node: usize) -> Result<()> {
        let mut by_glyph: HashMap<String, i64> = HashMap::new();
        for c in e.children.iter().filter(|c| c.name == "CID") {
            if by_glyph.len() >= MAX_LAYOUT_ITEMS {
                return Err(TablesError::TooLarge { what: "the glyphs of a CID map", limit: MAX_LAYOUT_ITEMS });
            }
            let glyph = c.attribute("glyph").ok_or_else(|| bad("a CID entry without a glyph"))?;
            by_glyph.insert(glyph.to_string(), int_lit(c.attribute("value").ok_or_else(|| bad("a CID entry without a value"))?)?);
        }
        let mut by_id: HashMap<u16, i64> = HashMap::new();
        for (glyph, cid) in by_glyph {
            if cid != 0xFFFF {
                by_id.insert(self.glyph(&glyph)?, cid);
            }
        }
        let count = by_id.keys().max().map_or(0, |m| i64::from(*m) + 1);
        self.put(node, |o| {
            o.u16(count, "the number of glyphs")?;
            (0..count).try_for_each(|g| o.u16(u16::try_from(g).ok().and_then(|g| by_id.get(&g)).copied().unwrap_or(0xFFFF), "a CID"))
        })
    }
}

/// The value of one AAT lookup entry: a number, or a table compiled on its own (`None` for an
/// empty one, a null offset).
#[derive(Clone, PartialEq)]
enum AatValue {
    Number(i64),
    Table(Option<Vec<u8>>),
}

/// A piece of a compiled AAT lookup: bytes, or an offset to a table.
enum Piece {
    Data(Vec<u8>),
    Table(Option<Vec<u8>>),
}

/// fontTools' `getSearchRange`.
fn search_range(n: usize, item_size: usize) -> (usize, usize, usize) {
    let mut exponent = 0usize;
    let mut x = n;
    while x > 0 {
        x >>= 1;
        exponent += 1;
    }
    let exponent = exponent.saturating_sub(1);
    let range = (1usize << exponent.min(30)).saturating_mul(item_size);
    (range, exponent, n.saturating_mul(item_size).saturating_sub(range))
}

/// `AATLookup.write`: formats 0, 2, 6 and 8 sized, the smallest written (the lower format on a
/// tie). Every value takes two bytes, a number or an offset.
fn aat_lookup_bytes(values: &[(u16, AatValue)], glyphs: usize) -> Result<Vec<Piece>> {
    let (Some((first, first_value)), Some((last, _))) = (values.first(), values.last()) else {
        return Err(bad("an AAT lookup with no entry"));
    };
    let mut segments: Vec<(u16, u16, &AatValue)> = Vec::new();
    let (mut start, mut end, mut value) = (*first, *first, first_value);
    for (id, v) in values.iter().skip(1) {
        if u32::from(*id) != u32::from(end) + 1 || v != value {
            segments.push((start, end, value));
            (start, end, value) = (*id, *id, v);
        } else {
            end = *id;
        }
    }
    segments.push((start, end, value));
    let n = values.len();
    let mut sizes: Vec<(usize, u16)> = Vec::new();
    if n == glyphs {
        sizes.push((2 + n * 2, 0));
    }
    sizes.push((2 + 10 + (segments.len() + 1) * 6, 2));
    sizes.push((2 + 10 + (n + 1) * 4, 6));
    if n == usize::from(*last) - usize::from(*first) + 1 {
        sizes.push((6 + n * 2, 8));
    }
    sizes.sort_unstable();
    let format = sizes.first().map_or(6, |s| s.1);
    let mut out: Vec<Piece> = Vec::new();
    let mut o = Out::default();
    let flush = |o: &mut Out, out: &mut Vec<Piece>| {
        if !o.0.is_empty() {
            out.push(Piece::Data(std::mem::take(&mut o.0)));
        }
    };
    let put_value = |o: &mut Out, out: &mut Vec<Piece>, v: &AatValue| -> Result<()> {
        match v {
            AatValue::Number(n) => o.u16(*n, "an AAT lookup value"),
            AatValue::Table(t) => {
                flush(o, out);
                out.push(Piece::Table(t.clone()));
                Ok(())
            }
        }
    };
    let header = |o: &mut Out, units: usize, unit: usize| -> Result<()> {
        let (range, selector, shift) = search_range(units, unit);
        for v in [unit, units, range, selector, shift] {
            o.u16(i64::try_from(v).unwrap_or(i64::MAX), "an AAT lookup header")?;
        }
        Ok(())
    };
    o.u16(i64::from(format), "an AAT lookup format")?;
    match format {
        0 => {
            for (_, v) in values {
                put_value(&mut o, &mut out, v)?;
            }
        }
        2 => {
            header(&mut o, segments.len(), 6)?;
            for (s, e, v) in &segments {
                o.u16(i64::from(*e), "a segment end")?;
                o.u16(i64::from(*s), "a segment start")?;
                put_value(&mut o, &mut out, v)?;
            }
            o.bytes(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
        }
        8 => {
            o.u16(i64::from(*first), "the first glyph")?;
            o.u16(i64::try_from(n).unwrap_or(i64::MAX), "the number of glyphs")?;
            for (_, v) in values {
                put_value(&mut o, &mut out, v)?;
            }
        }
        _ => {
            header(&mut o, n, 4)?;
            for (id, v) in values {
                o.u16(i64::from(*id), "a glyph index")?;
                put_value(&mut o, &mut out, v)?;
            }
            o.bytes(&[0xFF, 0xFF, 0, 0]);
        }
    }
    flush(&mut o, &mut out);
    Ok(out)
}

/// `DeltaSetIndexMap.getEntryFormat`.
fn entry_format(mapping: &[i64]) -> Result<i64> {
    let ored = mapping.iter().fold(0i64, |a, b| a | b);
    let mut inner = ored & 0xFFFF;
    let mut bits = 0i64;
    while inner != 0 {
        bits += 1;
        inner >>= 1;
    }
    let bits = bits.max(1);
    let ored = (ored >> (16 - bits)) | (ored & ((1 << bits) - 1));
    let size = if ored <= 0xFF {
        1
    } else if ored <= 0xFFFF {
        2
    } else if ored <= 0xFF_FFFF {
        3
    } else {
        4
    };
    Ok(((size - 1) << 4) | (bits - 1))
}

/// The feature-parameter layout a feature tag takes.
fn feature_params_class(tag: &str) -> Option<&'static str> {
    let number =
        |prefix: &str| tag.strip_prefix(prefix).filter(|d| d.len() == 2 && d.bytes().all(|b| b.is_ascii_digit())).and_then(|d| d.parse::<u8>().ok());
    if tag == "size" {
        Some("FeatureParamsSize")
    } else if number("ss").is_some_and(|n| (1..=20).contains(&n)) {
        Some("FeatureParamsStylisticSet")
    } else if number("cv").is_some_and(|n| (1..=99).contains(&n)) {
        Some("FeatureParamsCharacterVariants")
    } else {
        None
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn entry_formats_follow_the_widest_index() {
        assert_eq!(entry_format(&[0]).unwrap(), 0x00);
        assert_eq!(entry_format(&[0x0000_0003]).unwrap(), 0x01);
        // fontTools' getEntryFormat: outer 1 with inner 0 needs one inner bit and fits a byte.
        assert_eq!(entry_format(&[0x0001_0000]).unwrap(), 0x00);
        assert_eq!(entry_format(&[0x00FF_0001]).unwrap(), 0x10);
        assert_eq!(int_list("[1, -2, 0x3]").unwrap(), vec![1, -2, 3]);
        assert_eq!(version_value("0x00010002").unwrap(), 0x0001_0002);
        assert_eq!(version_value("1.0").unwrap(), 0x0001_0000);
    }
}
