//! Variable-font table checks (M12.54): the binary-scope `check.variations.*` rows. Each runner
//! reads one compiled font through read-fonts (and skrifa where outlines or advances at a
//! location are needed) and follows the FontBakery check its inventory row names. The rules are
//! re-implemented from FontBakery's check sources (Apache-2.0, see `NOTICE`); the messages are
//! ours.
//!
//! A runner whose table is absent or unreadable records itself as skipped with the reason; the
//! `check.tables.*` checks report missing and malformed tables.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use read_fonts::tables::glyf::Glyph as GlyfGlyph;
use read_fonts::tables::gpos::{PairPos, PositionSubtables, ValueRecord};
use read_fonts::tables::layout::DeviceOrVariationIndex;
use read_fonts::tables::post::{DEFAULT_GLYPH_NAMES, Post};
use read_fonts::tables::stat::AxisValue;
use read_fonts::tables::variations::ItemVariationStore;
use read_fonts::types::{GlyphId, GlyphId16, NameId, Tag, Version16Dot16};
use read_fonts::{FontRef, TableProvider};
use skrifa::MetadataProvider;
use skrifa::instance::Size;
use skrifa::outline::pen::NullPen;
use skrifa::outline::{DrawSettings, OutlinePen};

use crate::name_records::{self, MAX_NAME_TEXT_BYTES};
use crate::{BinaryCtx, Hit, Location, Params, Precondition, Sink, Threshold, num};

/// Most fvar axes a check reads; a font with more is skipped.
pub(crate) const MAX_AXES: usize = 64;
/// Most named instances a check reads; a font with more is skipped.
pub(crate) const MAX_INSTANCES: usize = 4_096;
/// Most STAT axis records and axis values a check reads; a font with more is skipped.
pub(crate) const MAX_STAT_ENTRIES: usize = 4_096;
/// Most pair-adjustment value records the kerning part of the reflow check reads.
pub(crate) const MAX_PAIR_RECORDS: usize = 1_000_000;

/// The registered axis tags of the OpenType specification.
const REGISTERED: [&str; 5] = ["ital", "opsz", "slnt", "wdth", "wght"];
/// Axes that must leave advances unchanged.
const DUPLEXED: [&str; 2] = ["GRAD", "ROND"];
/// The glyph whose lean measures the slant direction.
const SLANT_REFERENCE: char = 'H';
/// The weight and width classes of the OpenType `OS/2` table: the values a STAT axis must name
/// when the fvar range includes them.
const WEIGHT_CLASSES: [(f64, &str); 9] = [
    (100.0, "Thin"),
    (200.0, "ExtraLight"),
    (300.0, "Light"),
    (400.0, "Regular"),
    (500.0, "Medium"),
    (600.0, "SemiBold"),
    (700.0, "Bold"),
    (800.0, "ExtraBold"),
    (900.0, "Black"),
];
const WIDTH_CLASSES: [(f64, &str); 9] = [
    (50.0, "UltraCondensed"),
    (62.5, "ExtraCondensed"),
    (75.0, "Condensed"),
    (87.5, "SemiCondensed"),
    (100.0, "Normal"),
    (112.5, "SemiExpanded"),
    (125.0, "Expanded"),
    (150.0, "ExtraExpanded"),
    (200.0, "UltraExpanded"),
];
/// STAT axis ordering the specification's examples follow, from last to first.
const STAT_ORDER_FROM_LAST: [&str; 5] = ["ital", "slnt", "wdth", "wght", "opsz"];
/// The customary fvar axis order.
const FVAR_ORDER: [&str; 5] = ["opsz", "wdth", "wght", "ital", "slnt"];
const ELIDABLE: u16 = 0x0002;
const NO_PS_NAME: u16 = 0xFFFF;

/// `check.variations.statItalicNaming`: 1 also lets values of the slnt axis use the word Italic.
pub(crate) const ALLOW_ON_SLNT: Threshold = Threshold {
    name: "allowOnSlnt",
    default: 0.0,
    min: 0.0,
    max: 1.0,
    doc: "1 lets slnt axis values be named Italic (an oblique design); 0 allows the word only on the ital axis.",
};

// ---------------------------------------------------------------------------------------------
// Reading the tables
// ---------------------------------------------------------------------------------------------

pub(crate) struct VarAxis {
    pub tag: String,
    pub raw: Tag,
    pub min: f64,
    pub default: f64,
    pub max: f64,
    pub name_id: u16,
}

pub(crate) struct VarInstance {
    pub subfamily: u16,
    /// `None` when the records have no PostScript name field.
    pub ps: Option<u16>,
    /// One user value per axis, in axis order.
    pub coords: Vec<f64>,
}

impl VarInstance {
    fn ps_given(&self) -> bool {
        self.ps.is_some_and(|p| p != NO_PS_NAME)
    }
}

pub(crate) struct Fvar {
    pub axes: Vec<VarAxis>,
    pub instances: Vec<VarInstance>,
}

impl Fvar {
    fn index(&self, tag: &str) -> Option<usize> {
        self.axes.iter().position(|a| a.tag == tag)
    }

    fn axis(&self, tag: &str) -> Option<&VarAxis> {
        self.axes.iter().find(|a| a.tag == tag)
    }

    /// The first instance whose subfamily name (as [`Names::debug`] reads it) is `name`.
    fn named<'f>(&'f self, names: &Names, name: &str) -> Option<&'f VarInstance> {
        self.instances.iter().find(|i| names.debug(i.subfamily) == Some(name))
    }

    /// The value of `instance` on the axis tagged `tag`.
    fn value(&self, instance: &VarInstance, tag: &str) -> Option<f64> {
        self.index(tag).and_then(|i| instance.coords.get(i)).copied()
    }

    fn is_default(&self, instance: &VarInstance) -> bool {
        self.axes.len() == instance.coords.len() && self.axes.iter().zip(&instance.coords).all(|(a, v)| a.default == *v)
    }
}

fn has_table(font: &FontRef<'_>, tag: &[u8; 4]) -> bool {
    font.table_data(Tag::new(tag)).is_some()
}

fn tag_text(tag: Tag) -> String {
    String::from_utf8_lossy(&tag.to_be_bytes()).into_owned()
}

/// Reads fvar: `Ok(None)` when the font has none.
fn load_fvar(font: &FontRef<'_>) -> Result<Option<Fvar>, String> {
    if !has_table(font, b"fvar") {
        return Ok(None);
    }
    let fvar = font.fvar().map_err(|e| format!("the fvar table cannot be read: {e}"))?;
    let records = fvar.axes().map_err(|e| format!("the fvar axes cannot be read: {e}"))?;
    if records.len() > MAX_AXES {
        return Err(format!("the font has more than {MAX_AXES} axes"));
    }
    let axes: Vec<VarAxis> = records
        .iter()
        .map(|a| VarAxis {
            tag: tag_text(a.axis_tag()),
            raw: a.axis_tag(),
            min: a.min_value().to_f64(),
            default: a.default_value().to_f64(),
            max: a.max_value().to_f64(),
            name_id: a.axis_name_id().to_u16(),
        })
        .collect();
    let list = fvar.instances().map_err(|e| format!("the fvar instances cannot be read: {e}"))?;
    if list.len() > MAX_INSTANCES {
        return Err(format!("the font has more than {MAX_INSTANCES} named instances"));
    }
    let mut instances = Vec::with_capacity(list.len());
    for record in list.iter() {
        let record = record.map_err(|e| format!("an fvar instance cannot be read: {e}"))?;
        instances.push(VarInstance {
            subfamily: record.subfamily_name_id.to_u16(),
            ps: record.post_script_name_id.map(NameId::to_u16),
            coords: record.coordinates.iter().map(|c| c.get().to_f64()).collect(),
        });
    }
    Ok(Some(Fvar { axes, instances }))
}

/// Reads fvar, or records why the check cannot run.
fn read_fvar(ctx: &BinaryCtx<'_>, sink: &mut Sink) -> Option<Fvar> {
    match load_fvar(&ctx.font) {
        Ok(Some(fvar)) => Some(fvar),
        Ok(None) => {
            sink.skip(Precondition::Variable.describe());
            None
        }
        Err(reason) => {
            sink.skip(reason);
            None
        }
    }
}

/// The decoded records of the name table, read with the shared byte budget.
#[derive(Default)]
pub(crate) struct Names {
    /// Platform, language and text of each record whose string lies in the table, in table
    /// order; text the reader did not decode is empty.
    records: Vec<(u16, u16, String)>,
    /// name id → indices of its records.
    by_id: BTreeMap<u16, Vec<usize>>,
    /// name id → index of the record [`Names::debug`] answers.
    debug: BTreeMap<u16, usize>,
    /// name id → index of its first Windows Unicode BMP US English record.
    windows: BTreeMap<u16, usize>,
}

impl Names {
    pub(crate) fn read(font: &FontRef<'_>) -> Self {
        let mut budget = MAX_NAME_TEXT_BYTES;
        let Ok(read) = name_records::read(font, &mut budget, name_records::all) else { return Self::default() };
        let mut names = Self::default();
        let mut english: BTreeSet<u16> = BTreeSet::new();
        for r in read.records.into_iter().filter(|r| r.in_bounds) {
            let i = names.records.len();
            names.by_id.entry(r.id).or_default().push(i);
            if (r.platform, r.language) == (1, 0) || (r.platform, r.language) == (3, 0x409) {
                if english.insert(r.id) {
                    names.debug.insert(r.id, i);
                }
            } else if !english.contains(&r.id) {
                names.debug.insert(r.id, i);
            }
            if (r.platform, r.encoding, r.language) == (3, 1, 0x409) {
                names.windows.entry(r.id).or_insert(i);
            }
            names.records.push((r.platform, r.language, r.text.unwrap_or_default()));
        }
        names
    }

    fn text(&self, i: usize) -> Option<&str> {
        self.records.get(i).map(|r| r.2.as_str())
    }

    /// The name a person would read: the first English record (Windows US English or Macintosh
    /// English), else the last record with the id, as fontTools' `getDebugName` picks it.
    pub(crate) fn debug(&self, id: u16) -> Option<&str> {
        self.debug.get(&id).and_then(|&i| self.text(i))
    }

    /// The Windows Unicode BMP US English record.
    fn windows_english(&self, id: u16) -> Option<&str> {
        self.windows.get(&id).and_then(|&i| self.text(i))
    }

    fn has(&self, id: u16) -> bool {
        self.by_id.contains_key(&id)
    }

    fn all(&self, id: u16) -> impl Iterator<Item = &str> {
        self.by_id.get(&id).into_iter().flatten().filter_map(|&i| self.text(i))
    }

    /// The instance's subfamily name, or its position counted from 1.
    fn instance(&self, instance: &VarInstance, index: usize) -> String {
        self.debug(instance.subfamily).map(str::to_string).unwrap_or_else(|| format!("#{}", index.saturating_add(1)))
    }
}

#[derive(Clone)]
pub(crate) struct StatAxis {
    pub tag: String,
    pub ordering: u16,
}

pub(crate) struct StatValue {
    pub format: u16,
    /// The axis of a format 1, 2 or 3 value.
    pub axis: Option<u16>,
    pub flags: u16,
    pub name_id: u16,
    /// Format 1 and 3: the value; format 2: the nominal value.
    pub value: Option<f64>,
    pub linked: Option<f64>,
    /// Format 4: (axis index, value).
    pub records: Vec<(u16, f64)>,
}

impl StatValue {
    /// The axis indices the value names.
    fn axis_indices(&self) -> Vec<u16> {
        match self.axis {
            Some(a) => vec![a],
            None => self.records.iter().map(|r| r.0).collect(),
        }
    }
}

pub(crate) struct Stat {
    pub axes: Vec<StatAxis>,
    pub values: Vec<StatValue>,
    /// The table has an axis value array with at least one value.
    pub has_values: bool,
}

impl Stat {
    fn axis_tag(&self, index: u16) -> Option<&str> {
        self.axes.get(usize::from(index)).map(|a| a.tag.as_str())
    }
}

/// Reads STAT: `Ok(None)` when the font has none.
fn read_stat(font: &FontRef<'_>) -> Result<Option<Stat>, String> {
    if !has_table(font, b"STAT") {
        return Ok(None);
    }
    let stat = font.stat().map_err(|e| format!("the STAT table cannot be read: {e}"))?;
    let records = stat.design_axes().map_err(|e| format!("the STAT axis records cannot be read: {e}"))?;
    if records.len() > MAX_STAT_ENTRIES || usize::from(stat.axis_value_count()) > MAX_STAT_ENTRIES {
        return Err(format!("the STAT table has more than {MAX_STAT_ENTRIES} axis records or values"));
    }
    let axes = records.iter().map(|a| StatAxis { tag: tag_text(a.axis_tag()), ordering: a.axis_ordering() }).collect();
    let mut values = Vec::new();
    let array = match stat.offset_to_axis_values() {
        Some(array) => Some(array.map_err(|e| format!("the STAT axis values cannot be read: {e}"))?),
        None => None,
    };
    if let Some(array) = &array {
        for value in array.axis_values().iter() {
            let value = value.map_err(|e| format!("a STAT axis value cannot be read: {e}"))?;
            values.push(match value {
                AxisValue::Format1(v) => StatValue {
                    format: 1,
                    axis: Some(v.axis_index()),
                    flags: v.flags().bits(),
                    name_id: v.value_name_id().to_u16(),
                    value: Some(v.value().to_f64()),
                    linked: None,
                    records: Vec::new(),
                },
                AxisValue::Format2(v) => StatValue {
                    format: 2,
                    axis: Some(v.axis_index()),
                    flags: v.flags().bits(),
                    name_id: v.value_name_id().to_u16(),
                    value: Some(v.nominal_value().to_f64()),
                    linked: None,
                    records: Vec::new(),
                },
                AxisValue::Format3(v) => StatValue {
                    format: 3,
                    axis: Some(v.axis_index()),
                    flags: v.flags().bits(),
                    name_id: v.value_name_id().to_u16(),
                    value: Some(v.value().to_f64()),
                    linked: Some(v.linked_value().to_f64()),
                    records: Vec::new(),
                },
                AxisValue::Format4(v) => StatValue {
                    format: 4,
                    axis: None,
                    flags: v.flags().bits(),
                    name_id: v.value_name_id().to_u16(),
                    value: None,
                    linked: None,
                    records: v.axis_values().iter().map(|r| (r.axis_index(), r.value().to_f64())).collect(),
                },
            });
        }
    }
    let has_values = array.is_some() && stat.axis_value_count() > 0;
    Ok(Some(Stat { axes, values, has_values }))
}

/// Reads STAT for a check that needs it, or records why the check cannot run.
fn need_stat(ctx: &BinaryCtx<'_>, sink: &mut Sink) -> Option<Stat> {
    match read_stat(&ctx.font) {
        Ok(Some(stat)) => Some(stat),
        Ok(None) => {
            sink.skip(Precondition::Table("STAT").describe());
            None
        }
        Err(reason) => {
            sink.skip(reason);
            None
        }
    }
}

fn at(table: &str) -> Location {
    Location { table: Some(table.to_string()), ..Location::default() }
}

fn at_axis(table: &str, axis: &str) -> Location {
    Location { axis: Some(axis.to_string()), ..at(table) }
}

fn at_instance(name: &str) -> Location {
    Location { instance: Some(name.to_string()), ..at("fvar") }
}

fn at_instance_axis(name: &str, axis: &str) -> Location {
    Location { axis: Some(axis.to_string()), ..at_instance(name) }
}

fn at_name(table: &str, id: u16) -> Location {
    Location { name_id: Some(id), ..at(table) }
}

/// Glyph names for findings: the source glyph, else the `post` name, else `glyph N`. The `post`
/// strings are indexed on first use, since one lookup in a version 2 table scans the strings
/// before it.
struct GlyphNamer<'c, 'a> {
    ctx: &'c BinaryCtx<'a>,
    post: OnceCell<Option<(Post<'a>, Vec<&'a str>)>>,
}

impl<'c, 'a> GlyphNamer<'c, 'a> {
    fn new(ctx: &'c BinaryCtx<'a>) -> Self {
        Self { ctx, post: OnceCell::new() }
    }

    fn get(&self, gid: u16) -> String {
        if let Some(name) = self.ctx.source.and_then(|s| s.glyph(u32::from(gid))) {
            return name.to_string();
        }
        let post = self.post.get_or_init(|| {
            let post = self.ctx.font.post().ok()?;
            let strings = post.string_data().map(|d| d.iter().map_while(|s| s.ok().map(|s| s.as_str())).collect()).unwrap_or_default();
            Some((post, strings))
        });
        if let Some((post, strings)) = post
            && let Some(name) = post_glyph_name(post, strings, gid)
        {
            return name.to_string();
        }
        format!("glyph {gid}")
    }
}

/// `post.glyph_name` with the version 2 strings read in advance.
fn post_glyph_name<'a>(post: &Post<'a>, strings: &[&'a str], gid: u16) -> Option<&'a str> {
    if post.version() != Version16Dot16::VERSION_2_0 {
        return post.glyph_name(GlyphId16::new(gid));
    }
    let index = usize::from(post.glyph_name_index()?.get(usize::from(gid))?.get());
    match index.checked_sub(DEFAULT_GLYPH_NAMES.len()) {
        None => DEFAULT_GLYPH_NAMES.get(index).copied(),
        Some(i) => strings.get(i).copied(),
    }
}

fn glyph_count(font: &FontRef<'_>) -> u16 {
    font.maxp().map(|m| m.num_glyphs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// fvar axes
// ---------------------------------------------------------------------------------------------

/// `check.variations.axisRanges` (FontBakery `opentype/fvar/axis_ranges_correct`). Every
/// offending instance is reported, where FontBakery stops at the first.
pub(crate) fn axis_ranges(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let names = Names::read(&ctx.font);
    for (i, instance) in fvar.instances.iter().enumerate() {
        let name = names.instance(instance, i);
        if let Some(v) = fvar.value(instance, "wght")
            && !(1.0..=1000.0).contains(&v)
        {
            let message = format!("Instance {name} has weight {}; the weight axis runs from 1 to 1000.", num(v));
            sink.push(Hit::new("wght-out-of-range", message).data("value", v).place(at_instance_axis(&name, "wght")));
        }
        if let Some(v) = fvar.value(instance, "wdth") {
            if v < 1.0 {
                let message = format!("Instance {name} has width {}; widths start at 1.", num(v));
                sink.push(Hit::new("wdth-out-of-range", message).data("value", v).place(at_instance_axis(&name, "wdth")));
            } else if v > 1000.0 {
                let message = format!("Instance {name} has width {}, which is valid but unusual.", num(v));
                sink.push(Hit::new("wdth-greater-than-1000", message).data("value", v).place(at_instance_axis(&name, "wdth")));
            }
        }
    }
    if let Some(a) = fvar.axis("ital")
        && !(a.min == 0.0 && a.max == 1.0)
    {
        let message = format!("The ital axis runs from {} to {}; it must run from 0 (upright) to 1 (italic).", num(a.min), num(a.max));
        sink.push(Hit::new("invalid-ital-range", message).place(at_axis("fvar", "ital")));
    }
    if let Some(a) = fvar.axis("slnt")
        && !(a.min < 0.0 && a.max >= 0.0)
    {
        let message = format!(
            "The slnt axis runs from {} to {}. A design that leans right uses negative slant values; positive ones mean a backslant, which is rare.",
            num(a.min),
            num(a.max)
        );
        sink.push(Hit::new("unusual-slnt-range", message).place(at_axis("fvar", "slnt")));
    }
}

/// `check.variations.regularInstanceCoordinates` (FontBakery `opentype/fvar/regular_coords_correct`).
pub(crate) fn regular_instance_coordinates(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let names = Names::read(&ctx.font);
    let upright = fvar.named(&names, "Regular");
    let italic = fvar.named(&names, "Italic");
    let regular_italic = if upright.is_none() && italic.is_none() { fvar.named(&names, "Regular Italic") } else { None };
    let either = |tag: &str| [upright, italic, regular_italic].into_iter().flatten().next().and_then(|i| fvar.value(i, tag));
    let only_upright = |tag: &str| upright.and_then(|i| fvar.value(i, tag));
    let expected = [
        ("wght", 400.0, either("wght"), "wght-not-400"),
        ("wdth", 100.0, either("wdth"), "wdth-not-100"),
        ("slnt", 0.0, only_upright("slnt"), "slnt-not-0"),
        ("ital", 0.0, only_upright("ital"), "ital-not-0"),
    ];
    let opsz = either("opsz");
    if expected.iter().all(|e| e.2.is_none()) && opsz.is_none() {
        sink.push(Hit::new("no-regular-instance", "There is no instance named Regular.").place(at("fvar")));
        return;
    }
    let name = if upright.is_some() {
        "Regular"
    } else if italic.is_some() {
        "Italic"
    } else {
        "Regular Italic"
    };
    for (tag, want, got, code) in expected {
        if let Some(v) = got
            && v != want
        {
            let message = format!("The {name} instance has {tag} {}; it should be {}.", num(v), num(want));
            sink.push(Hit::new(code, message).data("value", v).place(at_instance_axis(name, tag)));
        }
    }
    if let Some(v) = opsz
        && v != 0.0
        && !(10.0..=16.0).contains(&v)
    {
        let message = format!("The {name} instance has optical size {}; text sizes from 10 to 16 are expected.", num(v));
        sink.push(Hit::new("opsz", message).data("value", v).place(at_instance_axis(name, "opsz")));
    }
}

/// `check.variations.boldInstanceCoordinate` (FontBakery `varfont/bold_wght_coord`).
pub(crate) fn bold_instance_coordinate(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let Some(wght) = fvar.axis("wght") else {
        sink.skip("the font has no wght axis");
        return;
    };
    let names = Names::read(&ctx.font);
    let bold = ["Bold", "Bold Italic"].into_iter().find_map(|n| fvar.named(&names, n).and_then(|i| fvar.value(i, "wght")).map(|v| (n, v)));
    match bold {
        None if wght.max < 700.0 => sink.skip("the weight axis does not reach 700"),
        None => sink.push(Hit::new("no-bold-instance", "The weight axis reaches 700 but there is no instance named Bold.").place(at("fvar"))),
        Some((name, v)) if v != 700.0 => {
            let message = format!("The {name} instance has weight {}; it should be 700.", num(v));
            sink.push(Hit::new("wght-not-700", message).data("value", v).place(at_instance_axis(name, "wght")));
        }
        Some(_) => {}
    }
}

/// Collects the first highest and the first lowest point a glyph's drawing passes, control
/// points included.
#[derive(Default)]
struct Extremes {
    high: Option<(f32, f32)>,
    low: Option<(f32, f32)>,
}

impl Extremes {
    fn add(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        if self.high.is_none_or(|h| y > h.1) {
            self.high = Some((x, y));
        }
        if self.low.is_none_or(|l| y < l.1) {
            self.low = Some((x, y));
        }
    }

    /// How far right the highest point sits from the lowest.
    fn lean(&self) -> Option<f64> {
        Some(f64::from(self.high?.0) - f64::from(self.low?.0))
    }
}

impl OutlinePen for Extremes {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.add(cx0, cy0);
        self.add(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.add(cx0, cy0);
        self.add(cx1, cy1);
        self.add(x, y);
    }
    fn close(&mut self) {}
}

/// `check.variations.slantDirection` (FontBakery `opentype/slant_direction`): the lean of H at
/// both ends of the slnt axis, other axes at their defaults.
pub(crate) fn slant_direction(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let Some(slnt) = fvar.axis("slnt") else { return };
    let Some(gid) = ctx.font.charmap().map(SLANT_REFERENCE) else {
        sink.skip("the font has no H, the glyph the slant is measured on");
        return;
    };
    let Some(glyph) = ctx.font.outline_glyphs().get(gid) else {
        sink.skip("the outline of H cannot be read");
        return;
    };
    let lean = |value: f64| -> Option<f64> {
        let location = ctx.font.axes().location([(slnt.raw, value as f32)]);
        let mut pen = Extremes::default();
        glyph.draw(DrawSettings::unhinted(Size::unscaled(), &location), &mut pen).ok()?;
        pen.lean()
    };
    let (Some(at_min), Some(at_max)) = (lean(slnt.min), lean(slnt.max)) else {
        sink.skip("H cannot be drawn at both ends of the slnt axis");
        return;
    };
    if at_min < at_max {
        let message = format!(
            "H leans further right at slnt {} than at slnt {}: positive slant values lean right, where the registered axis uses negative ones.",
            num(slnt.max),
            num(slnt.min)
        );
        sink.push(
            Hit::new("positive-value-for-clockwise-lean", message).data("leanAtMin", at_min).data("leanAtMax", at_max).place(at_axis("fvar", "slnt")),
        );
    }
}

/// `check.variations.foundryAxisTags` (FontBakery `opentype/varfont/foundry_defined_tag_name`).
pub(crate) fn foundry_axis_tags(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    for axis in &fvar.axes {
        if REGISTERED.contains(&axis.tag.as_str()) {
            continue;
        }
        let tag = &axis.tag;
        let lower = axis.tag.to_ascii_lowercase();
        if REGISTERED.contains(&lower.as_str()) {
            let message = format!(
                "The private axis tag {tag} differs from the registered tag {lower} only in case; use {lower} if the registered axis is meant."
            );
            sink.push(Hit::new("foundry-defined-similar-registered-name", message).place(at_axis("fvar", tag)));
        }
        let bytes = axis.raw.to_be_bytes();
        let [first, rest @ ..] = bytes;
        if !first.is_ascii_uppercase() {
            let message = format!("The private axis tag {tag} must start with an uppercase letter.");
            sink.push(Hit::new("invalid-foundry-defined-tag-first-letter", message).place(at_axis("fvar", tag)));
        }
        if rest.iter().any(|c| !(c.is_ascii_uppercase() || c.is_ascii_digit())) {
            let message = format!("The private axis tag {tag} may use only uppercase letters and digits.");
            sink.push(Hit::new("invalid-foundry-defined-tag-chars", message).place(at_axis("fvar", tag)));
        }
    }
}

/// `check.variations.unsupportedItalAxis` (FontBakery `varfont/unsupported_axes`).
pub(crate) fn unsupported_ital_axis(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    if fvar.axis("ital").is_some() {
        let message = "The font has an ital axis, which some browsers do not support; ship separate upright and italic fonts.";
        sink.push(Hit::new("unsupported-ital", message).place(at_axis("fvar", "ital")));
    }
}

/// `check.variations.axisHasNoVariation` (FontBakery `typenetwork/varfont/axes_have_variation`).
pub(crate) fn axis_has_no_variation(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    for axis in &fvar.axes {
        if axis.min == axis.max {
            let message = format!("The {} axis has minimum and maximum {}, so it varies nothing.", axis.tag, num(axis.min));
            sink.push(Hit::new("axis-has-no-variation", message).data("value", axis.min).place(at_axis("fvar", &axis.tag)));
        }
    }
}

/// `check.variations.axisOrder` (FontBakery `typenetwork/varfont/fvar_axes_order`, which runs
/// only without STAT, and `googlefonts/STAT/axis_order`, which reports the STAT order).
pub(crate) fn axis_order(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    match read_stat(&ctx.font) {
        Ok(Some(stat)) => {
            let mut axes = stat.axes.clone();
            axes.sort_by_key(|a| a.ordering);
            let order: Vec<&str> = axes.iter().map(|a| a.tag.as_str()).collect();
            let message = format!("STAT orders the axes {}.", order.join(", "));
            sink.push(Hit::new("stat-axis-order", message).place(at("STAT")));
        }
        Ok(None) => {
            let registered: Vec<&str> = fvar.axes.iter().map(|a| a.tag.as_str()).filter(|t| FVAR_ORDER.contains(t)).collect();
            let preferred: Vec<&str> = FVAR_ORDER.into_iter().filter(|t| registered.contains(t)).collect();
            if registered != preferred {
                let message = format!(
                    "The registered axes are in the order {}; applications sort instances best with {}.",
                    registered.join(", "),
                    preferred.join(", ")
                );
                sink.push(Hit::new("axes-incorrect-order", message).place(at("fvar")));
            }
            let custom: Vec<String> = fvar
                .axes
                .iter()
                .enumerate()
                .filter(|(_, a)| !FVAR_ORDER.contains(&a.tag.as_str()))
                .map(|(i, a)| format!("{} (axis {})", a.tag, i.saturating_add(1)))
                .collect();
            if !custom.is_empty() {
                sink.push(Hit::new("custom-axes", format!("Custom axes: {}.", custom.join(", "))).place(at("fvar")));
            }
        }
        Err(reason) => sink.skip(reason),
    }
}

/// `check.variations.customAxisRange` (FontBakery `fontbureau/ytlc_sanity`).
pub(crate) fn custom_axis_range(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    for axis in fvar.axes.iter().filter(|a| a.tag == "ytlc") {
        if axis.min < 0.0 || axis.max > 1000.0 {
            let message =
                format!("The ytlc axis (lowercase height) runs from {} to {}; its documented range is 0 to 1000.", num(axis.min), num(axis.max));
            sink.push(Hit::new("invalid-range", message).place(at_axis("fvar", "ytlc")));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// fvar instances
// ---------------------------------------------------------------------------------------------

/// `check.variations.instanceRecords` (FontBakery `opentype/varfont/same_size_instance_records`
/// and `distinct_instance_records`).
pub(crate) fn instance_records(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    if fvar.instances.is_empty() {
        sink.skip("the font has no named instances");
        return;
    }
    let forms: BTreeSet<bool> = fvar.instances.iter().map(VarInstance::ps_given).collect();
    if forms.len() > 1 {
        let message = "Some instance records have a PostScript name and others do not, so the records differ in size.";
        sink.push(Hit::new("different-size-instance-records", message).place(at("fvar")));
    }
    let names = Names::read(&ctx.font);
    let mut seen = HashSet::new();
    for (i, instance) in fvar.instances.iter().enumerate() {
        let key: (Vec<u64>, u16, u16) =
            (instance.coords.iter().map(|v| v.to_bits()).collect(), instance.subfamily, instance.ps.unwrap_or(NO_PS_NAME));
        if !seen.insert(key) {
            let name = names.instance(instance, i);
            let message = format!("Instance {name} repeats an earlier record: the same coordinates and names.");
            sink.push(Hit::new("repeated-instance-record", message).place(at_instance(&name)));
        }
    }
}

/// `check.variations.duplicateInstanceNames` (FontBakery `varfont/duplicate_instance_names`).
pub(crate) fn duplicate_instance_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let names = Names::read(&ctx.font);
    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicates: BTreeSet<&str> = BTreeSet::new();
    for instance in &fvar.instances {
        match names.windows_english(instance.subfamily) {
            Some(name) => {
                if !seen.insert(name) {
                    duplicates.insert(name);
                }
            }
            None => {
                let message = format!("There is no Windows English name record {} for an instance's subfamily name.", instance.subfamily);
                sink.push(Hit::new("name-record-not-found", message).place(at_name("name", instance.subfamily)));
            }
        }
    }
    for name in duplicates {
        let message = format!("More than one instance is named {name}.");
        sink.push(Hit::new("duplicate-instance-names", message).place(at_instance(name)));
    }
}

/// `check.variations.instancesInOrder` (FontBakery `varfont/instances_in_order`): within each
/// run of instances that agree on every other axis, weights rise.
pub(crate) fn instances_in_order(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let Some(w) = fvar.index("wght") else {
        sink.skip("the font has no wght axis");
        return;
    };
    let others = |i: &VarInstance| -> Vec<u64> { i.coords.iter().enumerate().filter(|(a, _)| *a != w).map(|(_, v)| v.to_bits()).collect() };
    let names = Names::read(&ctx.font);
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut last: Option<Vec<u64>> = None;
    for (i, instance) in fvar.instances.iter().enumerate() {
        let key = others(instance);
        match runs.last_mut() {
            Some(run) if last.as_ref() == Some(&key) => run.push(i),
            _ => runs.push(vec![i]),
        }
        last = Some(key);
    }
    for run in runs {
        let weights: Vec<f64> = run.iter().filter_map(|i| fvar.instances.get(*i)).map(|i| i.coords.get(w).copied().unwrap_or(0.0)).collect();
        if weights.windows(2).any(|p| matches!(p, [a, b] if b < a)) {
            let listed: Vec<String> = run
                .iter()
                .filter_map(|i| fvar.instances.get(*i).map(|inst| (i, inst)))
                .map(|(i, inst)| format!("{} ({})", names.instance(inst, *i), num(inst.coords.get(w).copied().unwrap_or(0.0))))
                .collect();
            let message = format!("These instances are not in rising weight order: {}.", listed.join(", "));
            sink.push(Hit::new("instances-not-in-order", message).place(at("fvar")));
        }
    }
}

fn in_font_range(id: u16) -> bool {
    (256..=32767).contains(&id)
}

/// `check.variations.instanceNameIds` (FontBakery `opentype/varfont/valid_nameids` and
/// `fvar_name_entries`). Missing records are reported for axis and PostScript names as well as
/// subfamily names.
pub(crate) fn instance_name_ids(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let names = Names::read(&ctx.font);
    for axis in &fvar.axes {
        let id = axis.name_id;
        if !in_font_range(id) {
            let message = format!("The {} axis names itself with name ID {id}; axis name IDs run from 256 to 32767.", axis.tag);
            sink.push(Hit::new("invalid-axis-nameid", message).place(Location { name_id: Some(id), ..at_axis("fvar", &axis.tag) }));
        } else if !names.has(id) {
            let message = format!("The {} axis names itself with name ID {id}, which has no record in the name table.", axis.tag);
            sink.push(Hit::new("missing-axis-name", message).place(Location { name_id: Some(id), ..at_axis("fvar", &axis.tag) }));
        }
    }
    for (i, instance) in fvar.instances.iter().enumerate() {
        let label = names.instance(instance, i);
        let id = instance.subfamily;
        if !in_font_range(id) && id != 2 && id != 17 {
            let message = format!("Instance {label} has subfamily name ID {id}; it must be 2, 17 or from 256 to 32767.");
            sink.push(Hit::new("invalid-subfamily-nameid", message).place(Location { name_id: Some(id), ..at_instance(&label) }));
        }
        if !names.has(id) {
            let message = format!("Instance {label} has subfamily name ID {id}, which has no record in the name table.");
            sink.push(Hit::new("missing-name", message).place(Location { name_id: Some(id), ..at_instance(&label) }));
        }
        if let Some(ps) = instance.ps.filter(|p| *p != NO_PS_NAME) {
            if ps != 6 && !in_font_range(ps) {
                let message = format!("Instance {label} has PostScript name ID {ps}; it must be 6, 0xFFFF or from 256 to 32767.");
                sink.push(Hit::new("invalid-postscript-nameid", message).place(Location { name_id: Some(ps), ..at_instance(&label) }));
            } else if !names.has(ps) {
                let message = format!("Instance {label} has PostScript name ID {ps}, which has no record in the name table.");
                sink.push(Hit::new("missing-postscript-name", message).place(Location { name_id: Some(ps), ..at_instance(&label) }));
            }
        }
    }
}

/// `check.variations.defaultInstanceNames` (FontBakery
/// `opentype/varfont/valid_default_instance_nameids`).
pub(crate) fn default_instance_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    if !has_table(&ctx.font, b"name") {
        sink.skip(Precondition::Table("name").describe());
        return;
    }
    let names = Names::read(&ctx.font);
    let font_subfamily = names.debug(17).or_else(|| names.debug(2));
    let ps_given = fvar.instances.iter().any(VarInstance::ps_given);
    let name6 = names.debug(6);
    for (i, instance) in fvar.instances.iter().enumerate().filter(|(_, inst)| fvar.is_default(inst)) {
        let subfamily = names.instance(instance, i);
        if font_subfamily != Some(subfamily.as_str()) {
            let want = font_subfamily.unwrap_or("(none)");
            let source = if names.debug(17).is_some() { "name ID 17" } else { "name ID 2" };
            let message = format!("Instance {subfamily} sits at the default location; its subfamily name should be {want}, the font's {source}.");
            sink.push(
                Hit::new("invalid-default-instance-subfamily-name", message)
                    .place(Location { name_id: Some(instance.subfamily), ..at_instance(&subfamily) }),
            );
        }
        if ps_given {
            let ps = instance.ps.filter(|p| *p != NO_PS_NAME).and_then(|p| names.debug(p));
            if ps.is_none() || ps != name6 {
                let message = format!(
                    "Instance {subfamily} sits at the default location; its PostScript name should be {}, the font's name ID 6, not {}.",
                    name6.unwrap_or("(none)"),
                    ps.unwrap_or("(none)")
                );
                sink.push(Hit::new("invalid-default-instance-postscript-name", message).place(at_instance(&subfamily)));
            }
        }
    }
}

/// `check.variations.weightClassMatchesDefault` (FontBakery `opentype/weight_class_fvar`).
pub(crate) fn weight_class_matches_default(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let Some(wght) = fvar.axis("wght") else {
        sink.skip("the font has no wght axis");
        return;
    };
    let Ok(os2) = ctx.font.os2() else {
        sink.skip(Precondition::Table("OS/2").describe());
        return;
    };
    let class = os2.us_weight_class();
    if f64::from(class) != wght.default.trunc() {
        let message = format!("OS/2 usWeightClass is {class}; it should match the default of the weight axis, {}.", num(wght.default));
        let field = "OS/2.usWeightClass";
        let place =
            Location { field: Some("usWeightClass".into()), source_field: ctx.source.and_then(|s| s.field(field)).map(str::to_string), ..at("OS/2") };
        sink.push(Hit::new("bad-weight-class", message).data("usWeightClass", f64::from(class)).data("default", wght.default).place(place));
    }
}

// ---------------------------------------------------------------------------------------------
// STAT
// ---------------------------------------------------------------------------------------------

/// `check.variations.statAxisRecords` (FontBakery `opentype/varfont/STAT_axis_record_for_each_axis`).
pub(crate) fn stat_axis_records(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let Some(stat) = need_stat(ctx, sink) else { return };
    let missing: BTreeSet<&str> = fvar.axes.iter().map(|a| a.tag.as_str()).filter(|t| !stat.axes.iter().any(|s| s.tag == *t)).collect();
    for tag in missing {
        let message = format!("STAT has no axis record for the {tag} axis.");
        sink.push(Hit::new("missing-axis-records", message).place(at_axis("STAT", tag)));
    }
}

/// Is the font italic by any of the signs FontBakery reads: the style bits, the word Italic in
/// the full name, or an italic angle.
fn is_italic(font: &FontRef<'_>, names: &Names) -> bool {
    let os2 = font.os2().is_ok_and(|t| t.fs_selection().bits() & 0x0001 != 0);
    let head = font.head().is_ok_and(|t| t.mac_style().bits() & 0x0002 != 0);
    let full = names.all(4).any(|n| n.to_lowercase().split_whitespace().any(|w| w == "italic"));
    let post = font.post().is_ok_and(|t| t.italic_angle().to_f64() != 0.0);
    os2 || head || full || post
}

/// `check.variations.statItalAxis` (FontBakery `opentype/STAT/ital_axis`). FontBakery pairs
/// the upright and italic fonts of a family; one font at a time, an italic font must carry an
/// ital axis in STAT and an upright one is checked only when it has one.
pub(crate) fn stat_ital_axis(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has_table(&ctx.font, b"fvar") {
        sink.skip(Precondition::Variable.describe());
        return;
    }
    let names = Names::read(&ctx.font);
    let italic = is_italic(&ctx.font, &names);
    let stat = match read_stat(&ctx.font) {
        Ok(Some(stat)) => stat,
        Ok(None) => {
            if italic {
                sink.push(Hit::new("no-stat", "The italic font has no STAT table, so it cannot declare an ital axis.").place(at("STAT")));
            }
            return;
        }
        Err(reason) => {
            sink.skip(reason);
            return;
        }
    };
    let Some(index) = stat.axes.iter().position(|a| a.tag == "ital") else {
        if italic {
            sink.push(Hit::new("missing-ital-axis", "The italic font has no ital axis in STAT.").place(at("STAT")));
        }
        return;
    };
    if index.saturating_add(1) != stat.axes.len() {
        let message = format!("The ital axis is axis {} of {} in STAT; it should be the last.", index.saturating_add(1), stat.axes.len());
        sink.push(Hit::new("ital-axis-not-last", message).place(at_axis("STAT", "ital")));
    }
    let Some(value) = stat.values.iter().find(|v| v.axis.is_some_and(|a| usize::from(a) == index)) else { return };
    let (want_value, want_flags) = if italic { (1.0, 0) } else { (0.0, ELIDABLE) };
    let got = value.value.unwrap_or(f64::NAN);
    if got != want_value {
        let message = format!(
            "The STAT ital value is {}; {} expects {}.",
            num(got),
            if italic { "an italic font" } else { "an upright font" },
            num(want_value)
        );
        sink.push(Hit::new("wrong-ital-axis-value", message).place(at_axis("STAT", "ital")));
    }
    if value.flags != want_flags {
        let message = if italic {
            format!("The STAT ital value of an italic font should have no flags; it has {:#06x}.", value.flags)
        } else {
            format!("The STAT ital value of an upright font should be elidable (flags 0x0002); it has {:#06x}.", value.flags)
        };
        sink.push(Hit::new("wrong-ital-axis-flag", message).place(at_axis("STAT", "ital")));
    }
    if !italic && value.linked != Some(1.0) {
        let message = "The STAT ital value of an upright font should link to the italic value 1.";
        sink.push(Hit::new("wrong-ital-axis-linkedvalue", message).place(at_axis("STAT", "ital")));
    }
}

/// `check.variations.statItalicNaming` (FontBakery `STAT_strings`; with `allowOnSlnt` set, the
/// laxer `adobefonts/STAT_strings`).
pub(crate) fn stat_italic_naming(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Some(stat) = need_stat(ctx, sink) else { return };
    let allow_slnt = params.get(&ALLOW_ON_SLNT) >= 0.5;
    let exempt: Vec<u16> = stat
        .axes
        .iter()
        .enumerate()
        .filter(|(_, a)| a.tag == "ital" || (allow_slnt && a.tag == "slnt"))
        .filter_map(|(i, _)| u16::try_from(i).ok())
        .collect();
    let ids: BTreeSet<u16> = stat.values.iter().filter(|v| v.axis_indices().iter().any(|a| !exempt.contains(a))).map(|v| v.name_id).collect();
    let names = Names::read(&ctx.font);
    for id in ids {
        if let Some(text) = names.all(id).find(|n| n.to_lowercase().contains("italic")) {
            let message = format!("The STAT axis value named {text} (name ID {id}) is not on the ital axis, so it should not say Italic.");
            sink.push(Hit::new("bad-italic", message).place(at_name("STAT", id)));
        }
    }
}

/// `check.variations.statCoversInstances` (FontBakery `inconsistencies_between_fvar_STAT`). A
/// format 4 value whose coordinates equal an instance's covers every axis of that instance.
pub(crate) fn stat_covers_instances(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let stat = match read_stat(&ctx.font) {
        Ok(Some(stat)) => stat,
        Ok(None) => {
            sink.push(Hit::new("missing-STAT-table", "The variable font has no STAT table.").place(at("STAT")));
            return;
        }
        Err(reason) => {
            sink.skip(reason);
            return;
        }
    };
    let names = Names::read(&ctx.font);
    for instance in &fvar.instances {
        let Some(name) = names.debug(instance.subfamily) else {
            let message = format!("An instance's subfamily name ID {} has no record in the name table.", instance.subfamily);
            sink.push(Hit::new("missing-name-id", message).place(at_name("name", instance.subfamily)));
            continue;
        };
        let combined = stat.values.iter().filter(|v| v.format == 4).any(|v| {
            let mut want: Vec<(&str, u64)> = fvar.axes.iter().map(|a| a.tag.as_str()).zip(instance.coords.iter().map(|c| c.to_bits())).collect();
            let mut got: Vec<(&str, u64)> = v.records.iter().filter_map(|(a, x)| Some((stat.axis_tag(*a)?, x.to_bits()))).collect();
            want.sort_unstable();
            got.sort_unstable();
            want == got
        });
        if combined {
            continue;
        }
        for (axis, value) in fvar.axes.iter().zip(&instance.coords) {
            let covered = stat
                .values
                .iter()
                .any(|v| v.axis.and_then(|a| stat.axis_tag(a)) == Some(axis.tag.as_str()) && (v.value == Some(*value) || v.linked == Some(*value)));
            if !covered {
                let message = format!("Instance {name} has {} {}, which no STAT axis value names.", axis.tag, num(*value));
                sink.push(Hit::new("missing-fvar-instance-axis-value", message).data("value", *value).place(at_instance_axis(name, &axis.tag)));
            }
        }
    }
}

/// `check.variations.statValueQuality` (FontBakery `microsoft/STAT_axis_values`,
/// `microsoft/STAT_table_eliding_bit`, `microsoft/STAT_table_axis_order` and
/// `googlefonts/STAT/compulsory_axis_values`; the last asks here for the OS/2 weight and width
/// class values that the fvar range includes).
pub(crate) fn stat_value_quality(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(stat) = need_stat(ctx, sink) else { return };
    let names = Names::read(&ctx.font);
    if stat.has_values {
        let mut seen: HashSet<(u16, u64)> = HashSet::new();
        for v in stat.values.iter().filter(|v| v.format == 1) {
            if let (Some(axis), Some(value)) = (v.axis, v.value)
                && !seen.insert((axis, value.to_bits()))
            {
                let tag = stat.axis_tag(axis).unwrap_or("?");
                let message = format!("The {tag} axis has the format 1 value {} more than once.", num(value));
                sink.push(Hit::new("duplicate-axis-value", message).data("value", value).place(at_axis("STAT", tag)));
            }
        }
        for v in &stat.values {
            if names.windows_english(v.name_id) == Some("Regular") && v.flags & ELIDABLE == 0 {
                let message = format!("The STAT value named Regular (name ID {}) is not marked elidable.", v.name_id);
                sink.push(Hit::new("regular-not-elidable", message).place(at_name("STAT", v.name_id)));
            }
        }
    } else {
        sink.push(Hit::new("no-axis-values", "STAT has no axis values.").place(at("STAT")));
    }
    let mut ordered = stat.axes.clone();
    ordered.sort_by_key(|a| a.ordering);
    let mut next = ordered.len();
    for tag in STAT_ORDER_FROM_LAST {
        if !ordered.iter().any(|a| a.tag == tag) {
            continue;
        }
        let slot = next.checked_sub(1);
        if slot.and_then(|s| ordered.get(s)).is_some_and(|a| a.tag == tag) {
            next = next.saturating_sub(1);
        } else {
            let message =
                format!("The STAT axis ordering puts {tag} out of place: the order runs opsz, wght, wdth, slnt, ital, with custom axes first.");
            sink.push(Hit::new("axis-order-invalid", message).place(at_axis("STAT", tag)));
        }
    }
    let Ok(Some(fvar)) = load_fvar(&ctx.font) else { return };
    for (tag, classes) in [("wght", &WEIGHT_CLASSES), ("wdth", &WIDTH_CLASSES)] {
        let (Some(axis), Some(index)) = (fvar.axis(tag), stat.axes.iter().position(|a| a.tag == tag)) else { continue };
        let present: Vec<f64> = stat.values.iter().filter(|v| v.axis.is_some_and(|a| usize::from(a) == index)).filter_map(|v| v.value).collect();
        for (value, label) in classes.iter().filter(|(v, _)| (axis.min..=axis.max).contains(v)) {
            if !present.contains(value) {
                let message = format!("STAT has no {tag} value {} ({label}), which the axis range includes.", num(*value));
                sink.push(Hit::new("missing-axis-value", message).data("value", *value).place(at_axis("STAT", tag)));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Variation tables
// ---------------------------------------------------------------------------------------------

/// `check.variations.avarMissing` (FontBakery `mandatory_avar_table`).
pub(crate) fn avar_missing(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if read_fvar(ctx, sink).is_none() {
        return;
    }
    if !has_table(&ctx.font, b"avar") {
        let message = "The variable font has no avar table, so every axis progresses linearly between masters.";
        sink.push(Hit::new("missing-avar", message).place(at("avar")));
    }
}

/// `check.variations.hvarMissing` (FontBakery `googlefonts/varfont/has_HVAR`).
pub(crate) fn hvar_missing(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if read_fvar(ctx, sink).is_none() {
        return;
    }
    if !has_table(&ctx.font, b"HVAR") {
        let message = "The variable font has no HVAR table, so text engines must compute advances from the outlines.";
        sink.push(Hit::new("lacks-HVAR", message).place(at("HVAR")));
    }
}

/// `check.variations.duplexedAxisReflow` (FontBakery `varfont/duplexed_axis_reflow`): gvar
/// deltas of the phantom points on a GRAD or ROND axis, and kerning whose advance adjustment
/// varies in a region of such an axis.
pub(crate) fn duplexed_axis_reflow(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let duplexed: Vec<(usize, &str)> =
        fvar.axes.iter().enumerate().filter(|(_, a)| DUPLEXED.contains(&a.tag.as_str())).map(|(i, a)| (i, a.tag.as_str())).collect();
    if duplexed.is_empty() {
        sink.skip("the font has no axis that must keep advances (GRAD, ROND)");
        return;
    }
    glyph_reflow(ctx, &duplexed, sink);
    kerning_reflow(ctx, &duplexed, sink);
}

fn glyph_reflow(ctx: &BinaryCtx<'_>, duplexed: &[(usize, &str)], sink: &mut Sink) {
    let (Ok(gvar), Ok(glyf), Ok(loca)) = (ctx.font.gvar(), ctx.font.glyf(), ctx.font.loca(None)) else { return };
    let namer = GlyphNamer::new(ctx);
    for gid in 0..glyph_count(&ctx.font) {
        let id = GlyphId::new(u32::from(gid));
        let Ok(Some(data)) = gvar.glyph_variation_data(id) else { continue };
        let points = match loca.get_glyf(id, &glyf) {
            Ok(Some(GlyfGlyph::Simple(g))) => g.num_points(),
            Ok(Some(GlyfGlyph::Composite(g))) => g.components().count(),
            _ => 0,
        };
        let phantom = points..points.saturating_add(4);
        let mut reflowing: Vec<&str> = Vec::new();
        for tuple in data.tuples() {
            let peak = tuple.peak();
            let ends = [tuple.intermediate_start(), tuple.intermediate_end()];
            for (axis, tag) in duplexed {
                let involved = peak.get(*axis).is_some_and(|p| p.to_bits() != 0)
                    || ends.iter().flatten().any(|t| t.get(*axis).is_some_and(|p| p.to_bits() != 0));
                if !involved || reflowing.contains(tag) {
                    continue;
                }
                if tuple.deltas().any(|d| phantom.contains(&usize::from(d.position)) && (d.x_delta != 0 || d.y_delta != 0)) {
                    reflowing.push(tag);
                }
            }
        }
        for tag in reflowing {
            let name = namer.get(gid);
            let message = format!("The advance of {name} changes along the {tag} axis, which must leave text length unchanged.");
            sink.push(Hit::new("axis-causes-reflow", message).place(Location { glyph: name, ..at_axis("gvar", tag) }));
        }
    }
}

fn kerning_reflow(ctx: &BinaryCtx<'_>, duplexed: &[(usize, &str)], sink: &mut Sink) {
    let Ok(gdef) = ctx.font.gdef() else { return };
    let Some(Ok(store)) = gdef.item_var_store() else { return };
    let Ok(regions) = store.variation_region_list() else { return };
    let mut effective: BTreeSet<usize> = BTreeSet::new();
    let mut effective_tags: BTreeSet<&str> = BTreeSet::new();
    for (ix, region) in regions.variation_regions().iter().enumerate() {
        let Ok(region) = region else { continue };
        for (axis, tag) in duplexed {
            if let Some(t) = region.region_axes().get(*axis)
                && (t.start_coord() != t.peak_coord() || t.peak_coord() != t.end_coord())
            {
                effective.insert(ix);
                effective_tags.insert(tag);
            }
        }
    }
    if effective.is_empty() {
        return;
    }
    let Ok(gpos) = ctx.font.gpos() else { return };
    let Ok(list) = gpos.lookup_list() else { return };
    let mut budget = MAX_PAIR_RECORDS;
    for (lookup_index, lookup) in list.lookups().iter().enumerate() {
        let Ok(lookup) = lookup else { continue };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        for subtable in subtables.iter().flatten() {
            let varies = match subtable {
                PairPos::Format1(f) => f.pair_sets().iter().flatten().any(|set| {
                    set.pair_value_records().iter().flatten().any(|r| spend(&mut budget) && advance_varies(r.value_record1(), &store, &effective))
                }),
                PairPos::Format2(f) => f.class1_records().iter().flatten().any(|c1| {
                    c1.class2_records().iter().flatten().any(|c2| spend(&mut budget) && advance_varies(c2.value_record1(), &store, &effective))
                }),
            };
            if varies {
                let tags: Vec<&str> = effective_tags.into_iter().collect();
                let message = format!(
                    "Kerning in lookup {} changes advances along the {} axis, which must leave text length unchanged.",
                    lookup_index,
                    tags.join(" and ")
                );
                sink.push(Hit::new("kern-causes-reflow", message).place(Location { lookup: Some(lookup_index), ..at("GPOS") }));
                return;
            }
            if budget == 0 {
                sink.skip(format!("the font has more than {MAX_PAIR_RECORDS} kerning records"));
                return;
            }
        }
    }
}

/// Takes one unit of the work budget; false once it is spent.
fn spend(budget: &mut usize) -> bool {
    match budget.checked_sub(1) {
        Some(left) => {
            *budget = left;
            true
        }
        None => false,
    }
}

/// Does the first glyph's advance adjustment of `record` have a non-zero delta in one of the
/// `effective` regions?
fn advance_varies(record: &ValueRecord<'_>, store: &ItemVariationStore<'_>, effective: &BTreeSet<usize>) -> bool {
    let Some(Ok(DeviceOrVariationIndex::VariationIndex(index))) = record.x_advance_device() else { return false };
    let Some(Ok(data)) = store.item_variation_data().get(usize::from(index.delta_set_outer_index())) else { return false };
    let regions = data.region_indexes();
    data.delta_set(index.delta_set_inner_index())
        .zip(regions.iter())
        .any(|(delta, region)| delta != 0 && effective.contains(&usize::from(region.get())))
}

/// `check.variations.staticInstantiation` (FontBakery `googlefonts/varfont/generate_static`,
/// which instances the font at the middle of every axis). Every glyph's outline and advance are
/// computed at that location and every variation table must be readable; the instancer itself
/// is not run.
pub(crate) fn static_instantiation(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(fvar) = read_fvar(ctx, sink) else { return };
    let namer = GlyphNamer::new(ctx);
    let font = &ctx.font;
    let unreadable: [(&str, bool); 6] = [
        ("avar", has_table(font, b"avar") && font.avar().is_err()),
        ("gvar", has_table(font, b"gvar") && font.gvar().is_err()),
        ("HVAR", has_table(font, b"HVAR") && font.hvar().is_err()),
        ("VVAR", has_table(font, b"VVAR") && font.vvar().is_err()),
        ("MVAR", has_table(font, b"MVAR") && font.mvar().is_err()),
        ("CFF2", has_table(font, b"CFF2") && font.cff2().is_err()),
    ];
    for (tag, _) in unreadable.iter().filter(|(_, bad)| *bad) {
        let message = format!("The {tag} table cannot be read, so no static instance can be made.");
        sink.push(Hit::new("unreadable-table", message).place(at(tag)));
    }
    let settings: Vec<(Tag, f32)> = fvar.axes.iter().map(|a| (a.raw, ((a.min + a.max) / 2.0) as f32)).collect();
    let location = font.axes().location(settings);
    let outlines = font.outline_glyphs();
    let metrics = font.glyph_metrics(Size::unscaled(), &location);
    for gid in 0..glyph_count(font) {
        let id = GlyphId::new(u32::from(gid));
        let drawn = match outlines.get(id) {
            Some(glyph) => glyph.draw(DrawSettings::unhinted(Size::unscaled(), &location), &mut NullPen).map(|_| ()).map_err(|e| e.to_string()),
            None => Ok(()),
        };
        let advance = metrics.advance_width(id);
        let problem = match (drawn, advance) {
            (Err(e), _) => Some(format!("its outline cannot be computed: {e}")),
            (Ok(()), Some(a)) if !a.is_finite() => Some("its advance is not a number".to_string()),
            _ => None,
        };
        if let Some(problem) = problem {
            let name = namer.get(gid);
            let message = format!("{name} cannot be instanced at the middle of every axis: {problem}.");
            sink.push(Hit::new("glyph-fails", message).place(Location { glyph: name, ..Location::default() }));
        }
    }
}

#[cfg(test)]
#[path = "variations_binary_tests.rs"]
mod tests;
