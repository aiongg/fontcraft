//! `tables show` (M12.2; rows `tables show`, `tables.show`): a tree of a table's fields, read
//! with read-fonts' typed tables and named as the OpenType specification names them, with glyph
//! ids shown as names when the font has them.
//!
//! The tables decoded are listed in [`DECODED`]; any other table is one node with its length.
//! Children below [`ShowOptions::depth`] are counted in [`ShowNode::more`], not built. The whole
//! call builds at most [`MAX_SHOW_NODES`] nodes and does at most [`MAX_SHOW_WORK`] steps of work;
//! past either limit the tables are marked truncated.

use std::fmt::Display;

use read_fonts::tables::cmap::{Cmap, CmapIterLimits, CmapSubtable};
use read_fonts::tables::fvar::Fvar;
use read_fonts::tables::gasp::Gasp;
use read_fonts::tables::gdef::Gdef;
use read_fonts::tables::glyf::{Anchor, Glyph};
use read_fonts::tables::gpos::{AnchorTable, Gpos, PairPos, PositionSubtables, SinglePos, ValueRecord};
use read_fonts::tables::gsub::{Gsub, SingleSubst, SubstitutionSubtables};
use read_fonts::tables::head::Head;
use read_fonts::tables::hhea::Hhea;
use read_fonts::tables::hmtx::Hmtx;
use read_fonts::tables::layout::{ChainedSequenceContext, ClassDef, CoverageTable, FeatureList, LangSys, LookupFlag, ScriptList, SequenceContext};
use read_fonts::tables::loca::Loca;
use read_fonts::tables::maxp::Maxp;
use read_fonts::tables::name::Name;
use read_fonts::tables::os2::Os2;
use read_fonts::tables::post::{DEFAULT_GLYPH_NAMES, Post};
use read_fonts::types::{F2Dot14, FWord, Fixed, LongDateTime, MajorMinor, NameId, UfWord, Version16Dot16};
use read_fonts::{FontData, FontRead, ReadError};

use crate::limits::{MAX_GLYPHS, MAX_SHOW_NODES, MAX_TABLES};
use crate::sfnt::{SfntFont, Tag, parse_tag, tag_str};
use crate::{Result, TablesError};

/// Most steps of work one call does, counting the children of collapsed records.
pub const MAX_SHOW_WORK: usize = 4 * MAX_SHOW_NODES;
/// Longest `--path`, in bytes.
const MAX_PATH: usize = 1_024;
/// Most numbers a list value (`featureIndices`, `lookupListIndices`) prints.
const MAX_LIST_ITEMS: usize = 1_000;

/// The tables [`show`] decodes into fields.
pub const DECODED: [&Tag; 17] = [
    b"head", b"hhea", b"maxp", b"OS/2", b"post", b"name", b"cmap", b"hmtx", b"loca", b"glyf", b"GDEF", b"GSUB", b"GPOS", b"fvar", b"avar", b"gasp",
    b"DSIG",
];

/// What [`show`] shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowOptions {
    /// Tables to show; empty shows every table.
    pub tables: Vec<Tag>,
    /// Deepest level of the tree; `None` shows everything (still capped by
    /// [`crate::limits::MAX_SHOW_NODES`]).
    pub depth: Option<usize>,
    /// Show glyph ids as glyph names where the font names its glyphs.
    pub glyph_names: bool,
}

/// One field or record of a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowNode {
    /// The field name, or the record index as text.
    pub name: String,
    /// The field's type as the specification writes it (`uint16`, `Fixed`, `Offset16`…), or
    /// `table`, `record`, `array` for nodes with children, `error` for a part that could not be
    /// read.
    pub kind: &'static str,
    /// The value as text; `None` for a record or array.
    pub value: Option<String>,
    /// Byte offset of the field from the start of the table, when known.
    pub offset: Option<usize>,
    pub children: Vec<ShowNode>,
    /// Children not shown because the depth limit was reached.
    pub more: usize,
}

/// One table's tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownTable {
    pub tag: Tag,
    pub length: usize,
    /// The fields; a table read-fonts does not know is one node with the length.
    pub root: ShowNode,
    /// More nodes existed than the limit allowed.
    pub truncated: bool,
    /// The table is one [`show`] decodes and it could be read.
    pub decoded: bool,
    /// Why a table that is normally decoded could not be read.
    pub error: Option<String>,
}

/// The trees of the tables `options` names.
pub fn show(font: &SfntFont, options: &ShowOptions) -> Result<Vec<ShownTable>> {
    if options.tables.len() > MAX_TABLES {
        return Err(TablesError::TooLarge { what: "the number of tables asked for", limit: MAX_TABLES });
    }
    let mut tags: Vec<Tag> = Vec::new();
    if options.tables.is_empty() {
        tags = font.tags();
    } else {
        for tag in &options.tables {
            if !font.tables.contains_key(tag) {
                return Err(TablesError::MissingTable(tag_str(*tag)));
            }
            if !tags.contains(tag) {
                tags.push(*tag);
            }
        }
    }
    let names = if options.glyph_names { glyph_names(font) } else { Vec::new() };
    let mut tree = Tree { names: &names, max_depth: options.depth.unwrap_or(usize::MAX), ..Tree::default() };
    let mut out = Vec::with_capacity(tags.len());
    for tag in tags {
        let data = font.get(tag).unwrap_or_default();
        tree.truncated = false;
        tree.level = 0;
        tree.base = data.as_ptr() as usize;
        tree.table_len = data.len();
        let known = DECODED.contains(&&tag);
        let mut failure = None;
        let name = tag_str(tag).trim_end().to_string();
        let built = if known {
            tree.group(name.clone(), "table", Some(0), |t, k| {
                if let Err(e) = decode(t, k, font, tag, data) {
                    failure = Some(e.to_string());
                }
            })
        } else {
            None
        };
        let (root, decoded) = match built {
            Some(root) if failure.is_none() => (root, true),
            _ => (ShowNode { name, kind: "table", value: Some(format!("{} bytes", data.len())), offset: Some(0), ..ShowNode::default() }, false),
        };
        out.push(ShownTable { tag, length: data.len(), root, truncated: tree.truncated, decoded, error: failure });
    }
    Ok(out)
}

/// The node `path` names in `tables`: a tag, then field names separated by periods, each
/// followed by any number of `[index]` (`GPOS.lookupList.lookups[3]`).
pub fn find<'a>(tables: &'a [ShownTable], path: &str) -> Result<&'a ShowNode> {
    if path.len() > MAX_PATH {
        return Err(TablesError::TooLarge { what: "the path", limit: MAX_PATH });
    }
    let mut parts = path.split('.');
    let first = parts.next().unwrap_or_default();
    let (tag_text, first_indices) = split_indices(first)?;
    let tag = parse_tag(tag_text)?;
    let table = tables.iter().find(|t| t.tag == tag).ok_or_else(|| TablesError::MissingTable(tag_str(tag)))?;
    let mut node = &table.root;
    let mut walked = tag_text.to_string();
    let step = |node: &'a ShowNode, name: &str, walked: &mut String| -> Result<&'a ShowNode> {
        let next = node.children.iter().find(|c| c.name == name);
        let next = next.ok_or_else(|| {
            let hint = if node.more > 0 { " (it is beyond the depth shown: give a larger depth)" } else { "" };
            TablesError::BadOptions(format!("`{walked}` has no field `{name}`{hint}"))
        })?;
        walked.push('.');
        walked.push_str(name);
        Ok(next)
    };
    for index in first_indices {
        node = step(node, &index.to_string(), &mut walked)?;
    }
    for part in parts {
        let (name, indices) = split_indices(part)?;
        node = step(node, name, &mut walked)?;
        for index in indices {
            node = step(node, &index.to_string(), &mut walked)?;
        }
    }
    Ok(node)
}

/// Cuts `node` to `depth` levels of children, counting what is cut in [`ShowNode::more`].
/// The trees [`show`] builds are at most a dozen levels deep, so the recursion is bounded.
pub fn prune(node: &mut ShowNode, depth: usize) {
    if depth == 0 {
        node.more = node.more.saturating_add(node.children.len());
        node.children.clear();
        return;
    }
    for child in &mut node.children {
        prune(child, depth - 1);
    }
}

/// `lookups[3][0]` as `lookups` and `[3, 0]`.
fn split_indices(part: &str) -> Result<(&str, Vec<usize>)> {
    let bad = || TablesError::BadOptions(format!("`{part}` is not a field name with optional [index] parts"));
    let (name, mut rest) = match part.find('[') {
        Some(i) => (part.get(..i).unwrap_or_default(), part.get(i..).unwrap_or_default()),
        None => (part, ""),
    };
    if name.is_empty() {
        return Err(bad());
    }
    let mut indices = Vec::new();
    while !rest.is_empty() {
        let body = rest.strip_prefix('[').ok_or_else(bad)?;
        let end = body.find(']').ok_or_else(bad)?;
        let number = body.get(..end).unwrap_or_default().parse::<usize>().map_err(|_| bad())?;
        indices.push(number);
        rest = body.get(end.saturating_add(1)..).unwrap_or_default();
        if indices.len() > 64 {
            return Err(bad());
        }
    }
    Ok((name, indices))
}

// ── The tree builder ────────────────────────────────────────────────────────────────────────────

/// Builds nodes within the limits. `level` is the depth of the nodes being made (the root's
/// children are at 1). A record at `max_depth` runs its builder in counting mode: nothing is
/// allocated, the children are only counted into `more`.
#[derive(Default)]
struct Tree<'n> {
    names: &'n [String],
    max_depth: usize,
    level: usize,
    nodes: usize,
    work: usize,
    counting: bool,
    truncated: bool,
    /// The address of the table's first byte, to turn subtable slices into offsets.
    base: usize,
    table_len: usize,
}

impl Tree<'_> {
    /// Charges one node; false (and truncated) past a limit.
    fn spend(&mut self) -> bool {
        if self.work >= MAX_SHOW_WORK || (!self.counting && self.nodes >= MAX_SHOW_NODES) {
            self.truncated = true;
            return false;
        }
        self.work += 1;
        if !self.counting {
            self.nodes += 1;
        }
        true
    }

    fn leaf(&mut self, name: impl Into<String>, kind: &'static str, value: impl Display) -> Option<ShowNode> {
        if !self.spend() {
            return None;
        }
        if self.counting {
            return Some(ShowNode::default());
        }
        Some(ShowNode { name: name.into(), kind, value: Some(value.to_string()), ..ShowNode::default() })
    }

    fn err(&mut self, name: impl Into<String>, e: ReadError) -> Option<ShowNode> {
        self.leaf(name, "error", e)
    }

    fn group(
        &mut self,
        name: impl Into<String>,
        kind: &'static str,
        offset: Option<usize>,
        build: impl FnOnce(&mut Self, &mut Vec<ShowNode>),
    ) -> Option<ShowNode> {
        if !self.spend() {
            return None;
        }
        if self.counting {
            return Some(ShowNode::default());
        }
        let mut node = ShowNode { name: name.into(), kind, offset, ..ShowNode::default() };
        let mut children = Vec::new();
        if self.level >= self.max_depth {
            self.counting = true;
            build(self, &mut children);
            self.counting = false;
            node.more = children.len();
        } else {
            self.level += 1;
            build(self, &mut children);
            self.level -= 1;
            node.children = children;
        }
        Some(node)
    }

    /// Where a subtable starts in the table, from its bytes (which borrow the table's bytes).
    fn at(&self, data: FontData<'_>) -> Option<usize> {
        let start = (data.as_bytes().as_ptr() as usize).checked_sub(self.base)?;
        (start <= self.table_len).then_some(start)
    }

    fn glyph(&self, gid: u32) -> String {
        match self.names.get(gid as usize).filter(|n| !n.is_empty()) {
            Some(name) => format!("{name} ({gid})"),
            None => gid.to_string(),
        }
    }

    /// An array of glyphs, one leaf each, named by position.
    fn glyphs(&mut self, name: &str, offset: Option<usize>, gids: impl Iterator<Item = u32>) -> Option<ShowNode> {
        self.group(name, "array", offset, |t, k| {
            for (i, gid) in gids.enumerate() {
                if t.truncated {
                    break;
                }
                let label = t.glyph(gid);
                k.extend(t.leaf(i.to_string(), "GlyphID", label));
            }
        })
    }

    fn coverage(&mut self, name: &str, coverage: std::result::Result<CoverageTable<'_>, ReadError>) -> Option<ShowNode> {
        match coverage {
            Ok(c) => {
                let at = self.at(c.offset_data());
                self.glyphs(name, at, c.iter().map(|g| g.to_u32()))
            }
            Err(e) => self.err(name, e),
        }
    }

    fn class_def(&mut self, name: &str, class_def: std::result::Result<ClassDef<'_>, ReadError>, class_names: &[&str]) -> Option<ShowNode> {
        match class_def {
            Ok(cd) => {
                let at = self.at(cd.offset_data());
                self.group(name, "ClassDef", at, |t, k| {
                    k.extend(t.leaf("classFormat", "uint16", cd.class_format()));
                    for (gid, class) in cd.iter() {
                        if t.truncated {
                            break;
                        }
                        let label = t.glyph(gid.to_u32());
                        let value = match class_names.get(usize::from(class)) {
                            Some(n) if !n.is_empty() => format!("{class} ({n})"),
                            _ => class.to_string(),
                        };
                        k.extend(t.leaf(label, "uint16", value));
                    }
                })
            }
            Err(e) => self.err(name, e),
        }
    }
}

// ── Field values ────────────────────────────────────────────────────────────────────────────────

/// A field value: its type name and its text (`None` for an absent optional field).
trait Field {
    fn kind(&self) -> &'static str;
    fn text(&self) -> Option<String>;
}

macro_rules! field {
    ($($ty:ty => $kind:literal),* $(,)?) => {
        $(impl Field for $ty {
            fn kind(&self) -> &'static str {
                $kind
            }
            fn text(&self) -> Option<String> {
                Some(self.to_string())
            }
        })*
    };
}

field!(u8 => "uint8", u16 => "uint16", i16 => "int16", u32 => "uint32", i32 => "int32", Fixed => "Fixed", F2Dot14 => "F2DOT14",
    FWord => "FWORD", UfWord => "UFWORD", MajorMinor => "version", Version16Dot16 => "Version16Dot16",
    read_fonts::types::Tag => "Tag");

impl Field for NameId {
    fn kind(&self) -> &'static str {
        "uint16"
    }
    fn text(&self) -> Option<String> {
        Some(self.to_u16().to_string())
    }
}

impl Field for LongDateTime {
    fn kind(&self) -> &'static str {
        "LONGDATETIME"
    }
    fn text(&self) -> Option<String> {
        Some(date(self.as_secs()))
    }
}

impl<T: Field> Field for Option<T> {
    fn kind(&self) -> &'static str {
        self.as_ref().map_or("", Field::kind)
    }
    fn text(&self) -> Option<String> {
        self.as_ref().and_then(Field::text)
    }
}

/// Pushes one leaf per accessor, named in the specification's camel case.
macro_rules! fields {
    ($t:expr, $k:expr, $v:expr, [$($f:ident),* $(,)?]) => {
        $(
            let value = $v.$f();
            if let Some(text) = Field::text(&value) {
                $k.extend($t.leaf(camel(stringify!($f)), Field::kind(&value), text));
            }
        )*
    };
}

/// `units_per_em` as `unitsPerEm`.
fn camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper = false;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn hex16(v: u16) -> String {
    format!("0x{v:04X}")
}

fn hex32(v: u32) -> String {
    format!("0x{v:08X}")
}

/// Seconds since 1904-01-01 00:00 as a UTC date and time.
fn date(secs: i64) -> String {
    let unix = secs.saturating_sub(2_082_844_800);
    let (days, tod) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    // Days to a civil date (proleptic Gregorian); every product stays far inside i64.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", tod / 3_600, tod % 3_600 / 60, tod % 60)
}

/// Numbers as a comma-separated list, at most [`MAX_LIST_ITEMS`] of them.
fn join<T: Display>(items: impl Iterator<Item = T>) -> String {
    let mut out = String::new();
    for (i, item) in items.enumerate() {
        if i == MAX_LIST_ITEMS {
            out.push_str(", …");
            break;
        }
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&item.to_string());
    }
    out
}

fn value_record(v: &ValueRecord<'_>) -> String {
    let mut parts = Vec::new();
    for (name, value) in [("xPlacement", v.x_placement()), ("yPlacement", v.y_placement()), ("xAdvance", v.x_advance()), ("yAdvance", v.y_advance())]
    {
        if let Some(value) = value {
            parts.push(format!("{name} {value}"));
        }
    }
    let devices = [v.x_placement_device(), v.y_placement_device(), v.x_advance_device(), v.y_advance_device()];
    let device_count = devices.iter().filter(|d| d.is_some()).count();
    if device_count > 0 {
        parts.push(format!("{device_count} device or variation tables"));
    }
    if parts.is_empty() { "empty".into() } else { parts.join(", ") }
}

fn anchor(a: &AnchorTable<'_>) -> String {
    let base = format!("({}, {})", a.x_coordinate(), a.y_coordinate());
    match a {
        AnchorTable::Format2(f) => format!("{base} on point {}", f.anchor_point()),
        AnchorTable::Format3(_) => format!("{base} with device or variation tables"),
        _ => base,
    }
}

fn lookup_flag(flag: LookupFlag) -> String {
    let bits = flag.to_bits();
    let mut names = Vec::new();
    for (bit, name) in [(1u16, "RightToLeft"), (2, "IgnoreBaseGlyphs"), (4, "IgnoreLigatures"), (8, "IgnoreMarks"), (0x10, "UseMarkFilteringSet")] {
        if bits & bit != 0 {
            names.push(name.to_string());
        }
    }
    if let Some(class) = flag.mark_attachment_class().filter(|c| *c != 0) {
        names.push(format!("MarkAttachmentType {class}"));
    }
    if names.is_empty() { hex16(bits) } else { format!("{} ({})", hex16(bits), names.join(", ")) }
}

// ── Glyph names ─────────────────────────────────────────────────────────────────────────────────

/// The glyph names of `post` (formats 1 and 2), by glyph id; empty when it names none.
fn glyph_names(font: &SfntFont) -> Vec<String> {
    let Some(post) = font.get(*b"post").and_then(|d| Post::read(FontData::new(d)).ok()) else { return Vec::new() };
    let version = post.version();
    if version == Version16Dot16::VERSION_1_0 {
        return DEFAULT_GLYPH_NAMES.iter().map(|s| (*s).to_string()).collect();
    }
    if version != Version16Dot16::VERSION_2_0 {
        return Vec::new();
    }
    let Some(index) = post.glyph_name_index() else { return Vec::new() };
    // Read the extra names once: looking each one up by index walks the list from its start.
    let extra: Vec<&str> =
        post.string_data().map(|d| d.iter().take(MAX_GLYPHS + 1).map(|s| s.map(|p| p.as_str()).unwrap_or_default()).collect()).unwrap_or_default();
    index
        .iter()
        .take(MAX_GLYPHS + 1)
        .map(|i| {
            let i = usize::from(i.get());
            let name = match i.checked_sub(DEFAULT_GLYPH_NAMES.len()) {
                None => DEFAULT_GLYPH_NAMES.get(i).copied(),
                Some(j) => extra.get(j).copied(),
            };
            name.unwrap_or_default().to_string()
        })
        .collect()
}

fn num_glyphs(font: &SfntFont) -> Option<u16> {
    font.get(*b"maxp").and_then(|d| Maxp::read(FontData::new(d)).ok()).map(|m| m.num_glyphs())
}

// ── Decoders ────────────────────────────────────────────────────────────────────────────────────

type Kids = Vec<ShowNode>;
type Decoded = std::result::Result<(), ReadError>;

fn decode(t: &mut Tree<'_>, k: &mut Kids, font: &SfntFont, tag: Tag, data: &[u8]) -> Decoded {
    let fd = FontData::new(data);
    match &tag {
        b"head" => head(t, k, fd),
        b"hhea" => {
            let h = Hhea::read(fd)?;
            fields!(
                t,
                k,
                h,
                [
                    version,
                    ascender,
                    descender,
                    line_gap,
                    advance_width_max,
                    min_left_side_bearing,
                    min_right_side_bearing,
                    x_max_extent,
                    caret_slope_rise,
                    caret_slope_run,
                    caret_offset,
                    metric_data_format,
                    number_of_h_metrics
                ]
            );
            Ok(())
        }
        b"maxp" => {
            let m = Maxp::read(fd)?;
            fields!(
                t,
                k,
                m,
                [
                    version,
                    num_glyphs,
                    max_points,
                    max_contours,
                    max_composite_points,
                    max_composite_contours,
                    max_zones,
                    max_twilight_points,
                    max_storage,
                    max_function_defs,
                    max_instruction_defs,
                    max_stack_elements,
                    max_size_of_instructions,
                    max_component_elements,
                    max_component_depth
                ]
            );
            Ok(())
        }
        b"OS/2" => os2(t, k, fd),
        b"post" => post(t, k, font, fd),
        b"name" => name(t, k, fd),
        b"cmap" => cmap(t, k, font, fd),
        b"hmtx" => hmtx(t, k, font, fd),
        b"loca" => {
            let loca = loca(font, data)?;
            k.extend(t.group("offsets", "array", None, |t, k| {
                for i in 0..loca.len() {
                    if t.truncated {
                        break;
                    }
                    k.extend(t.leaf(i.to_string(), "Offset32", loca.get_raw(i).unwrap_or_default()));
                }
            }));
            Ok(())
        }
        b"glyf" => glyf(t, k, font, data),
        b"GDEF" => gdef(t, k, fd),
        b"GSUB" => gsub(t, k, fd),
        b"GPOS" => gpos(t, k, fd),
        b"fvar" => fvar(t, k, fd),
        b"avar" => avar(t, k, fd),
        b"gasp" => {
            let g = Gasp::read(fd)?;
            fields!(t, k, g, [version, num_ranges]);
            let ranges = g.gasp_ranges();
            k.extend(t.group("gaspRanges", "array", None, |t, k| {
                for (i, r) in ranges.iter().enumerate() {
                    k.extend(t.group(i.to_string(), "record", None, |t, k| {
                        k.extend(t.leaf("rangeMaxPPEM", "uint16", r.range_max_ppem()));
                        k.extend(t.leaf("rangeGaspBehavior", "uint16", hex16(r.range_gasp_behavior().bits())));
                    }));
                }
            }));
            Ok(())
        }
        b"DSIG" => {
            let version = data.get(..4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes).ok_or(ReadError::OutOfBounds)?;
            let count = data.get(4..6).and_then(|b| b.try_into().ok()).map(u16::from_be_bytes).ok_or(ReadError::OutOfBounds)?;
            k.extend(t.leaf("version", "uint32", version));
            k.extend(t.leaf("numSignatures", "uint16", count));
            Ok(())
        }
        _ => Ok(()),
    }
}

fn head(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let h = Head::read(fd)?;
    fields!(t, k, h, [version, font_revision]);
    k.extend(t.leaf("checksumAdjustment", "uint32", hex32(h.checksum_adjustment())));
    k.extend(t.leaf("magicNumber", "uint32", hex32(h.magic_number())));
    k.extend(t.leaf("flags", "uint16", hex16(h.flags().bits())));
    fields!(t, k, h, [units_per_em, created, modified, x_min, y_min, x_max, y_max]);
    k.extend(t.leaf("macStyle", "uint16", hex16(h.mac_style().bits())));
    fields!(t, k, h, [lowest_rec_ppem, font_direction_hint, index_to_loc_format, glyph_data_format]);
    Ok(())
}

fn os2(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let o = Os2::read(fd)?;
    fields!(t, k, o, [version, x_avg_char_width, us_weight_class, us_width_class]);
    k.extend(t.leaf("fsType", "uint16", hex16(o.fs_type())));
    fields!(
        t,
        k,
        o,
        [
            y_subscript_x_size,
            y_subscript_y_size,
            y_subscript_x_offset,
            y_subscript_y_offset,
            y_superscript_x_size,
            y_superscript_y_size,
            y_superscript_x_offset,
            y_superscript_y_offset,
            y_strikeout_size,
            y_strikeout_position,
            s_family_class
        ]
    );
    k.extend(t.leaf("panose", "uint8[10]", join(o.panose_10().iter())));
    for (name, v) in [
        ("ulUnicodeRange1", o.ul_unicode_range_1()),
        ("ulUnicodeRange2", o.ul_unicode_range_2()),
        ("ulUnicodeRange3", o.ul_unicode_range_3()),
        ("ulUnicodeRange4", o.ul_unicode_range_4()),
    ] {
        k.extend(t.leaf(name, "uint32", hex32(v)));
    }
    k.extend(t.leaf("achVendID", "Tag", o.ach_vend_id()));
    k.extend(t.leaf("fsSelection", "uint16", hex16(o.fs_selection().bits())));
    fields!(t, k, o, [us_first_char_index, us_last_char_index, s_typo_ascender, s_typo_descender, s_typo_line_gap, us_win_ascent, us_win_descent]);
    for (name, v) in [("ulCodePageRange1", o.ul_code_page_range_1()), ("ulCodePageRange2", o.ul_code_page_range_2())] {
        if let Some(v) = v {
            k.extend(t.leaf(name, "uint32", hex32(v)));
        }
    }
    fields!(
        t,
        k,
        o,
        [sx_height, s_cap_height, us_default_char, us_break_char, us_max_context, us_lower_optical_point_size, us_upper_optical_point_size]
    );
    Ok(())
}

fn post(t: &mut Tree<'_>, k: &mut Kids, font: &SfntFont, fd: FontData<'_>) -> Decoded {
    let p = Post::read(fd)?;
    fields!(
        t,
        k,
        p,
        [
            version,
            italic_angle,
            underline_position,
            underline_thickness,
            is_fixed_pitch,
            min_mem_type42,
            max_mem_type42,
            min_mem_type1,
            max_mem_type1,
            num_glyphs
        ]
    );
    let names = glyph_names(font);
    if !names.is_empty() {
        k.extend(t.group("glyphNames", "array", None, |t, k| {
            for (i, n) in names.iter().enumerate() {
                if t.truncated {
                    break;
                }
                k.extend(t.leaf(i.to_string(), "string", n));
            }
        }));
    }
    Ok(())
}

fn name(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let n = Name::read(fd)?;
    fields!(t, k, n, [version, count]);
    let records = n.name_record();
    let strings = n.string_data();
    k.extend(t.group("nameRecord", "array", None, |t, k| {
        for (i, r) in records.iter().enumerate() {
            if t.truncated {
                break;
            }
            k.extend(t.group(i.to_string(), "NameRecord", None, |t, k| {
                fields!(t, k, r, [platform_id, encoding_id, language_id, name_id]);
                match r.string(strings) {
                    Ok(s) => k.extend(t.leaf("string", "string", s.chars().collect::<String>())),
                    Err(e) => k.extend(t.err("string", e)),
                }
            }));
        }
    }));
    Ok(())
}

fn cmap(t: &mut Tree<'_>, k: &mut Kids, font: &SfntFont, fd: FontData<'_>) -> Decoded {
    let c = Cmap::read(fd)?;
    let glyph_count = u32::from(num_glyphs(font).unwrap_or(u16::MAX));
    k.extend(t.leaf("version", "uint16", c.version()));
    let records = c.encoding_records();
    k.extend(t.leaf("numTables", "uint16", records.len()));
    k.extend(t.group("encodingRecords", "array", None, |t, k| {
        for (i, r) in records.iter().enumerate() {
            if t.truncated {
                break;
            }
            k.extend(t.group(i.to_string(), "EncodingRecord", None, |t, k| {
                k.extend(t.leaf("platformID", "uint16", r.platform_id() as u16));
                k.extend(t.leaf("encodingID", "uint16", r.encoding_id()));
                match r.subtable(c.offset_data()) {
                    Ok(sub) => {
                        let at = t.at(sub.offset_data());
                        k.extend(t.group("subtable", "record", at, |t, k| cmap_subtable(t, k, &sub, glyph_count)));
                    }
                    Err(e) => k.extend(t.err("subtable", e)),
                }
            }));
        }
    }));
    Ok(())
}

fn cmap_subtable(t: &mut Tree<'_>, k: &mut Kids, sub: &CmapSubtable<'_>, glyph_count: u32) {
    let format: u16 = match sub {
        CmapSubtable::Format0(_) => 0,
        CmapSubtable::Format2(_) => 2,
        CmapSubtable::Format4(_) => 4,
        CmapSubtable::Format6(_) => 6,
        CmapSubtable::Format8(_) => 8,
        CmapSubtable::Format10(_) => 10,
        CmapSubtable::Format12(_) => 12,
        CmapSubtable::Format13(_) => 13,
        CmapSubtable::Format14(_) => 14,
    };
    k.extend(t.leaf("format", "uint16", format));
    match sub {
        CmapSubtable::Format0(s) => {
            let array = s.glyph_id_array();
            k.extend(t.group("mappings", "array", None, |t, k| {
                for (code, gid) in array.iter().enumerate() {
                    let label = t.glyph(u32::from(*gid));
                    k.extend(t.leaf(format!("U+{code:04X}"), "GlyphID", label));
                }
            }));
        }
        CmapSubtable::Format14(s) => {
            k.extend(t.leaf("numVarSelectorRecords", "uint32", s.num_var_selector_records()));
        }
        CmapSubtable::Format2(_) | CmapSubtable::Format8(_) => {
            k.extend(t.leaf("mappings", "note", "not expanded for this format"));
        }
        _ => {
            let limits = CmapIterLimits { max_char: 0x10_FFFF, glyph_count };
            let pairs = sub.iter_with_limits(limits);
            k.extend(t.group("mappings", "array", None, |t, k| {
                for (code, gid) in pairs {
                    if t.truncated {
                        break;
                    }
                    let label = t.glyph(gid.to_u32());
                    k.extend(t.leaf(format!("U+{code:04X}"), "GlyphID", label));
                }
            }));
        }
    }
}

fn hmtx(t: &mut Tree<'_>, k: &mut Kids, font: &SfntFont, fd: FontData<'_>) -> Decoded {
    let hhea = font.get(*b"hhea").ok_or(ReadError::OutOfBounds).and_then(|d| Hhea::read(FontData::new(d)))?;
    let h = Hmtx::read_with_args(fd, hhea.number_of_h_metrics())?;
    let metrics = h.h_metrics();
    let bearings = h.left_side_bearings();
    k.extend(t.group("hMetrics", "array", None, |t, k| {
        for (i, m) in metrics.iter().enumerate() {
            if t.truncated {
                break;
            }
            let label = t.glyph(i as u32);
            k.extend(t.leaf(label, "longHorMetric", format!("advanceWidth {}, lsb {}", m.advance(), m.side_bearing())));
        }
    }));
    if !bearings.is_empty() {
        k.extend(t.group("leftSideBearings", "array", None, |t, k| {
            for (i, lsb) in bearings.iter().enumerate() {
                if t.truncated {
                    break;
                }
                let label = t.glyph(u32::try_from(i.saturating_add(metrics.len())).unwrap_or(u32::MAX));
                k.extend(t.leaf(label, "FWORD", lsb.get()));
            }
        }));
    }
    Ok(())
}

fn loca<'a>(font: &SfntFont, data: &'a [u8]) -> std::result::Result<Loca<'a>, ReadError> {
    let head = font.get(*b"head").ok_or(ReadError::OutOfBounds).and_then(|d| Head::read(FontData::new(d)))?;
    Loca::read(FontData::new(data), head.index_to_loc_format() == 1)
}

fn glyf(t: &mut Tree<'_>, k: &mut Kids, font: &SfntFont, data: &[u8]) -> Decoded {
    let loca_data = font.get(*b"loca").ok_or(ReadError::OutOfBounds)?;
    let loca = loca(font, loca_data)?;
    let count = loca.len().saturating_sub(1);
    k.extend(t.group("glyphs", "array", None, |t, k| {
        for i in 0..count {
            if t.truncated {
                break;
            }
            let label = t.glyph(i as u32);
            let (start, end) = (loca.get_raw(i).unwrap_or_default() as usize, loca.get_raw(i.saturating_add(1)).unwrap_or_default() as usize);
            if end <= start {
                k.extend(t.leaf(label, "glyph", "empty"));
                continue;
            }
            let Some(bytes) = data.get(start..end) else {
                k.extend(t.err(label, ReadError::OutOfBounds));
                continue;
            };
            match Glyph::read(FontData::new(bytes)) {
                Ok(g) => k.extend(t.group(label, "glyph", Some(start), |t, k| glyph(t, k, &g))),
                Err(e) => k.extend(t.err(label, e)),
            }
        }
    }));
    Ok(())
}

fn glyph(t: &mut Tree<'_>, k: &mut Kids, g: &Glyph<'_>) {
    match g {
        Glyph::Simple(s) => {
            fields!(t, k, s, [number_of_contours, x_min, y_min, x_max, y_max]);
            k.extend(t.leaf("points", "uint16", s.num_points()));
            k.extend(t.leaf("instructionLength", "uint16", s.instruction_length()));
        }
        Glyph::Composite(c) => {
            fields!(t, k, c, [number_of_contours, x_min, y_min, x_max, y_max]);
            let components = c.components();
            k.extend(t.group("components", "array", None, |t, k| {
                for (i, comp) in components.enumerate() {
                    if t.truncated {
                        break;
                    }
                    let place = match comp.anchor {
                        Anchor::Offset { x, y } => format!("offset ({x}, {y})"),
                        Anchor::Point { base, component } => format!("point {component} on point {base}"),
                    };
                    let label = t.glyph(comp.glyph.to_u32());
                    k.extend(t.leaf(i.to_string(), "component", format!("{label}, {place}")));
                }
            }));
        }
    }
}

const GLYPH_CLASSES: [&str; 5] = ["", "base", "ligature", "mark", "component"];

fn gdef(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let g = Gdef::read(fd)?;
    fields!(t, k, g, [version]);
    if let Some(cd) = g.glyph_class_def() {
        k.extend(t.class_def("glyphClassDef", cd, &GLYPH_CLASSES));
    }
    if let Some(list) = g.attach_list() {
        match list {
            Ok(l) => k.extend(t.leaf("attachList", "Offset16", format!("{} glyphs", l.glyph_count()))),
            Err(e) => k.extend(t.err("attachList", e)),
        }
    }
    if let Some(list) = g.lig_caret_list() {
        match list {
            Ok(l) => k.extend(t.leaf("ligCaretList", "Offset16", format!("{} ligatures", l.lig_glyph_count()))),
            Err(e) => k.extend(t.err("ligCaretList", e)),
        }
    }
    if let Some(cd) = g.mark_attach_class_def() {
        k.extend(t.class_def("markAttachClassDef", cd, &[]));
    }
    if let Some(sets) = g.mark_glyph_sets_def() {
        match sets {
            Ok(s) => {
                let at = t.at(s.offset_data());
                let coverages = s.coverages();
                k.extend(t.group("markGlyphSetsDef", "record", at, |t, k| {
                    for (i, c) in coverages.iter().enumerate() {
                        if t.truncated {
                            break;
                        }
                        k.extend(t.coverage(&i.to_string(), c));
                    }
                }));
            }
            Err(e) => k.extend(t.err("markGlyphSetsDef", e)),
        }
    }
    if g.item_var_store().is_some() {
        k.extend(t.leaf("itemVarStore", "Offset32", "present"));
    }
    Ok(())
}

// ── Layout tables ───────────────────────────────────────────────────────────────────────────────

fn script_list(t: &mut Tree<'_>, k: &mut Kids, list: std::result::Result<ScriptList<'_>, ReadError>) {
    let list = match list {
        Ok(l) => l,
        Err(e) => {
            k.extend(t.err("scriptList", e));
            return;
        }
    };
    let at = t.at(list.offset_data());
    k.extend(t.group("scriptList", "ScriptList", at, |t, k| {
        k.extend(t.leaf("scriptCount", "uint16", list.script_count()));
        let records = list.script_records();
        k.extend(t.group("scriptRecords", "array", None, |t, k| {
            for (i, r) in records.iter().enumerate() {
                if t.truncated {
                    break;
                }
                k.extend(t.group(i.to_string(), "ScriptRecord", None, |t, k| {
                    k.extend(t.leaf("scriptTag", "Tag", r.script_tag()));
                    match r.script(list.offset_data()) {
                        Ok(script) => {
                            let at = t.at(script.offset_data());
                            k.extend(t.group("script", "Script", at, |t, k| {
                                if let Some(ls) = script.default_lang_sys() {
                                    lang_sys(t, k, "defaultLangSys", ls);
                                }
                                let records = script.lang_sys_records();
                                k.extend(t.group("langSysRecords", "array", None, |t, k| {
                                    for (j, r) in records.iter().enumerate() {
                                        if t.truncated {
                                            break;
                                        }
                                        k.extend(t.group(j.to_string(), "LangSysRecord", None, |t, k| {
                                            k.extend(t.leaf("langSysTag", "Tag", r.lang_sys_tag()));
                                            lang_sys(t, k, "langSys", r.lang_sys(script.offset_data()));
                                        }));
                                    }
                                }));
                            }));
                        }
                        Err(e) => k.extend(t.err("script", e)),
                    }
                }));
            }
        }));
    }));
}

fn lang_sys(t: &mut Tree<'_>, k: &mut Kids, name: &str, ls: std::result::Result<LangSys<'_>, ReadError>) {
    match ls {
        Ok(ls) => {
            let at = t.at(ls.offset_data());
            k.extend(t.group(name, "LangSys", at, |t, k| {
                let required = ls.required_feature_index();
                let text = if required == 0xFFFF { "none (0xFFFF)".to_string() } else { required.to_string() };
                k.extend(t.leaf("requiredFeatureIndex", "uint16", text));
                k.extend(t.leaf("featureIndices", "uint16[]", join(ls.feature_indices().iter().map(|i| i.get()))));
            }));
        }
        Err(e) => k.extend(t.err(name, e)),
    }
}

fn feature_list(t: &mut Tree<'_>, k: &mut Kids, list: std::result::Result<FeatureList<'_>, ReadError>) {
    let list = match list {
        Ok(l) => l,
        Err(e) => {
            k.extend(t.err("featureList", e));
            return;
        }
    };
    let at = t.at(list.offset_data());
    k.extend(t.group("featureList", "FeatureList", at, |t, k| {
        k.extend(t.leaf("featureCount", "uint16", list.feature_count()));
        let records = list.feature_records();
        k.extend(t.group("featureRecords", "array", None, |t, k| {
            for (i, r) in records.iter().enumerate() {
                if t.truncated {
                    break;
                }
                k.extend(t.group(i.to_string(), "FeatureRecord", None, |t, k| {
                    k.extend(t.leaf("featureTag", "Tag", r.feature_tag()));
                    match r.feature(list.offset_data()) {
                        Ok(f) => {
                            let at = t.at(f.offset_data());
                            k.extend(t.group("feature", "Feature", at, |t, k| {
                                if f.feature_params().is_some() {
                                    k.extend(t.leaf("featureParams", "Offset16", "present"));
                                }
                                k.extend(t.leaf("lookupListIndices", "uint16[]", join(f.lookup_list_indices().iter().map(|i| i.get()))));
                            }));
                        }
                        Err(e) => k.extend(t.err("feature", e)),
                    }
                }));
            }
        }));
    }));
}

/// Something with the bytes it starts at, for offsets.
trait Data {
    fn data(&self) -> FontData<'_>;
}

macro_rules! data {
    ($($ty:ident),* $(,)?) => {
        $(impl Data for read_fonts::tables::gpos::$ty<'_> {
            fn data(&self) -> FontData<'_> {
                self.offset_data()
            }
        })*
    };
}
data!(SinglePos, PairPos, CursivePosFormat1, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1);

macro_rules! data_gsub {
    ($($ty:ident),* $(,)?) => {
        $(impl Data for read_fonts::tables::gsub::$ty<'_> {
            fn data(&self) -> FontData<'_> {
                self.offset_data()
            }
        })*
    };
}
data_gsub!(SingleSubst, MultipleSubstFormat1, AlternateSubstFormat1, LigatureSubstFormat1, ReverseChainSingleSubstFormat1);

impl Data for SequenceContext<'_> {
    fn data(&self) -> FontData<'_> {
        self.offset_data()
    }
}

impl Data for ChainedSequenceContext<'_> {
    fn data(&self) -> FontData<'_> {
        self.offset_data()
    }
}

/// The `subtables` array of a lookup.
fn subtables<T: Data>(
    t: &mut Tree<'_>,
    k: &mut Kids,
    len: usize,
    items: impl Iterator<Item = std::result::Result<T, ReadError>>,
    each: impl Fn(&mut Tree<'_>, &mut Kids, &T),
) {
    k.extend(t.leaf("subTableCount", "uint16", len));
    k.extend(t.group("subtables", "array", None, |t, k| {
        for (i, item) in items.enumerate() {
            if t.truncated {
                break;
            }
            match item {
                Ok(st) => {
                    let at = t.at(st.data());
                    k.extend(t.group(i.to_string(), "subtable", at, |t, k| each(t, k, &st)));
                }
                Err(e) => k.extend(t.err(i.to_string(), e)),
            }
        }
    }));
}

const GPOS_TYPES: [&str; 10] =
    ["", "single", "pair", "cursive", "mark to base", "mark to ligature", "mark to mark", "context", "chained context", "extension"];
const GSUB_TYPES: [&str; 9] =
    ["", "single", "multiple", "alternate", "ligature", "context", "chained context", "extension", "reverse chained single"];

fn lookup_header(t: &mut Tree<'_>, k: &mut Kids, kind: u16, flag: LookupFlag, filtering: Option<u16>, names: &[&str]) {
    let label = names.get(usize::from(kind)).copied().unwrap_or("unknown");
    k.extend(t.leaf("lookupType", "uint16", format!("{kind} ({label})")));
    k.extend(t.leaf("lookupFlag", "uint16", lookup_flag(flag)));
    if let Some(set) = filtering {
        k.extend(t.leaf("markFilteringSet", "uint16", set));
    }
}

fn gpos(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let g = Gpos::read(fd)?;
    fields!(t, k, g, [version]);
    script_list(t, k, g.script_list());
    feature_list(t, k, g.feature_list());
    match g.lookup_list() {
        Ok(ll) => {
            let at = t.at(ll.offset_data());
            k.extend(t.group("lookupList", "LookupList", at, |t, k| {
                k.extend(t.leaf("lookupCount", "uint16", ll.lookup_count()));
                let lookups = ll.lookups();
                k.extend(t.group("lookups", "array", None, |t, k| {
                    for (i, lookup) in lookups.iter().enumerate() {
                        if t.truncated {
                            break;
                        }
                        match lookup {
                            Ok(l) => k.extend(t.group(i.to_string(), "Lookup", None, |t, k| {
                                lookup_header(t, k, l.lookup_type(), l.lookup_flag(), l.mark_filtering_set(), &GPOS_TYPES);
                                match l.subtables() {
                                    Ok(subs) => gpos_subtables(t, k, subs),
                                    Err(e) => k.extend(t.err("subtables", e)),
                                }
                            })),
                            Err(e) => k.extend(t.err(i.to_string(), e)),
                        }
                    }
                }));
            }));
        }
        Err(e) => k.extend(t.err("lookupList", e)),
    }
    if g.feature_variations().is_some() {
        k.extend(t.leaf("featureVariations", "Offset32", "present"));
    }
    Ok(())
}

fn gpos_subtables(t: &mut Tree<'_>, k: &mut Kids, subs: PositionSubtables<'_>) {
    match subs {
        PositionSubtables::Single(s) => subtables(t, k, s.len(), s.iter(), single_pos),
        PositionSubtables::Pair(s) => subtables(t, k, s.len(), s.iter(), pair_pos),
        PositionSubtables::Cursive(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("posFormat", "uint16", st.pos_format()));
            k.extend(t.coverage("coverage", st.coverage()));
            k.extend(t.leaf("entryExitCount", "uint16", st.entry_exit_count()));
        }),
        PositionSubtables::MarkToBase(s) => subtables(t, k, s.len(), s.iter(), mark_base),
        PositionSubtables::MarkToLig(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("posFormat", "uint16", st.pos_format()));
            k.extend(t.coverage("markCoverage", st.mark_coverage()));
            k.extend(t.coverage("ligatureCoverage", st.ligature_coverage()));
            k.extend(t.leaf("markClassCount", "uint16", st.mark_class_count()));
        }),
        PositionSubtables::MarkToMark(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("posFormat", "uint16", st.pos_format()));
            k.extend(t.coverage("mark1Coverage", st.mark1_coverage()));
            k.extend(t.coverage("mark2Coverage", st.mark2_coverage()));
            k.extend(t.leaf("markClassCount", "uint16", st.mark_class_count()));
        }),
        PositionSubtables::Contextual(s) => subtables(t, k, s.len(), s.iter(), sequence_context),
        PositionSubtables::ChainContextual(s) => subtables(t, k, s.len(), s.iter(), chained_context),
        PositionSubtables::EmptyExtension => {
            k.extend(t.leaf("subTableCount", "uint16", 0));
        }
    }
}

fn single_pos(t: &mut Tree<'_>, k: &mut Kids, st: &SinglePos<'_>) {
    k.extend(t.leaf("posFormat", "uint16", st.pos_format()));
    match st {
        SinglePos::Format1(f) => {
            k.extend(t.coverage("coverage", f.coverage()));
            k.extend(t.leaf("valueRecord", "ValueRecord", value_record(&f.value_record())));
        }
        SinglePos::Format2(f) => {
            let coverage = f.coverage();
            let records = f.value_records();
            k.extend(t.group("values", "array", None, |t, k| {
                let Ok(coverage) = coverage else { return };
                for (gid, record) in coverage.iter().zip(records.iter()) {
                    if t.truncated {
                        break;
                    }
                    let label = t.glyph(gid.to_u32());
                    match record {
                        Ok(r) => k.extend(t.leaf(label, "ValueRecord", value_record(&r))),
                        Err(e) => k.extend(t.err(label, e)),
                    }
                }
            }));
        }
    }
}

fn pair_pos(t: &mut Tree<'_>, k: &mut Kids, st: &PairPos<'_>) {
    k.extend(t.leaf("posFormat", "uint16", st.pos_format()));
    match st {
        PairPos::Format1(f) => {
            let coverage = f.coverage();
            let sets = f.pair_sets();
            k.extend(t.group("pairSets", "array", None, |t, k| {
                let Ok(coverage) = coverage else { return };
                for (first, set) in coverage.iter().zip(sets.iter()) {
                    if t.truncated {
                        break;
                    }
                    let label = t.glyph(first.to_u32());
                    let set = match set {
                        Ok(s) => s,
                        Err(e) => {
                            k.extend(t.err(label, e));
                            continue;
                        }
                    };
                    let at = t.at(set.offset_data());
                    k.extend(t.group(label, "PairSet", at, |t, k| {
                        for record in set.pair_value_records().iter() {
                            if t.truncated {
                                break;
                            }
                            match record {
                                Ok(r) => {
                                    let second = t.glyph(r.second_glyph().to_u32());
                                    let text = pair_text(r.value_record1(), r.value_record2());
                                    k.extend(t.leaf(second, "PairValueRecord", text));
                                }
                                Err(e) => k.extend(t.err("pairValueRecord", e)),
                            }
                        }
                    }));
                }
            }));
        }
        PairPos::Format2(f) => {
            k.extend(t.coverage("coverage", f.coverage()));
            k.extend(t.class_def("classDef1", f.class_def1(), &[]));
            k.extend(t.class_def("classDef2", f.class_def2(), &[]));
            k.extend(t.leaf("class1Count", "uint16", f.class1_count()));
            k.extend(t.leaf("class2Count", "uint16", f.class2_count()));
            let rows = f.class1_records();
            k.extend(t.group("class1Records", "array", None, |t, k| {
                for (i, row) in rows.iter().enumerate() {
                    if t.truncated {
                        break;
                    }
                    let row = match row {
                        Ok(r) => r,
                        Err(e) => {
                            k.extend(t.err(i.to_string(), e));
                            continue;
                        }
                    };
                    k.extend(t.group(i.to_string(), "Class1Record", None, |t, k| {
                        for (j, cell) in row.class2_records().iter().enumerate() {
                            if t.truncated {
                                break;
                            }
                            match cell {
                                Ok(c) => k.extend(t.leaf(j.to_string(), "Class2Record", pair_text(c.value_record1(), c.value_record2()))),
                                Err(e) => k.extend(t.err(j.to_string(), e)),
                            }
                        }
                    }));
                }
            }));
        }
    }
}

fn pair_text(first: &ValueRecord<'_>, second: &ValueRecord<'_>) -> String {
    if second.format().bits() == 0 { value_record(first) } else { format!("{}; second: {}", value_record(first), value_record(second)) }
}

fn mark_base(t: &mut Tree<'_>, k: &mut Kids, st: &read_fonts::tables::gpos::MarkBasePosFormat1<'_>) {
    k.extend(t.leaf("posFormat", "uint16", st.pos_format()));
    k.extend(t.leaf("markClassCount", "uint16", st.mark_class_count()));
    let marks = st.mark_coverage();
    let mark_array = st.mark_array();
    k.extend(t.group("markArray", "MarkArray", None, |t, k| {
        let (Ok(coverage), Ok(array)) = (marks, mark_array) else {
            k.extend(t.leaf("error", "error", "the mark coverage or mark array cannot be read"));
            return;
        };
        for (gid, record) in coverage.iter().zip(array.mark_records()) {
            if t.truncated {
                break;
            }
            let label = t.glyph(gid.to_u32());
            let anchor_text = record.mark_anchor(array.offset_data()).map(|a| anchor(&a)).unwrap_or_else(|e| e.to_string());
            k.extend(t.leaf(label, "MarkRecord", format!("class {}, anchor {anchor_text}", record.mark_class())));
        }
    }));
    let bases = st.base_coverage();
    let base_array = st.base_array();
    k.extend(t.group("baseArray", "BaseArray", None, |t, k| {
        let (Ok(coverage), Ok(array)) = (bases, base_array) else {
            k.extend(t.leaf("error", "error", "the base coverage or base array cannot be read"));
            return;
        };
        for (gid, record) in coverage.iter().zip(array.base_records().iter()) {
            if t.truncated {
                break;
            }
            let label = t.glyph(gid.to_u32());
            match record {
                Ok(r) => {
                    let anchors = r.base_anchors(array.offset_data());
                    let text = join(anchors.iter().map(|a| match a {
                        None => "none".to_string(),
                        Some(Ok(a)) => anchor(&a),
                        Some(Err(e)) => e.to_string(),
                    }));
                    k.extend(t.leaf(label, "BaseRecord", text));
                }
                Err(e) => k.extend(t.err(label, e)),
            }
        }
    }));
}

fn sequence_context(t: &mut Tree<'_>, k: &mut Kids, st: &SequenceContext<'_>) {
    match st {
        SequenceContext::Format1(f) => {
            k.extend(t.leaf("format", "uint16", 1));
            k.extend(t.coverage("coverage", f.coverage()));
            k.extend(t.leaf("seqRuleSetCount", "uint16", f.seq_rule_set_count()));
        }
        SequenceContext::Format2(f) => {
            k.extend(t.leaf("format", "uint16", 2));
            k.extend(t.coverage("coverage", f.coverage()));
            k.extend(t.class_def("classDef", f.class_def(), &[]));
            k.extend(t.leaf("classSeqRuleSetCount", "uint16", f.class_seq_rule_set_count()));
        }
        SequenceContext::Format3(f) => {
            k.extend(t.leaf("format", "uint16", 3));
            let coverages = f.coverages();
            k.extend(t.group("coverages", "array", None, |t, k| {
                for (i, c) in coverages.iter().enumerate() {
                    k.extend(t.coverage(&i.to_string(), c));
                }
            }));
            k.extend(t.leaf("seqLookupCount", "uint16", f.seq_lookup_count()));
        }
    }
}

fn chained_context(t: &mut Tree<'_>, k: &mut Kids, st: &ChainedSequenceContext<'_>) {
    match st {
        ChainedSequenceContext::Format1(f) => {
            k.extend(t.leaf("format", "uint16", 1));
            k.extend(t.coverage("coverage", f.coverage()));
            k.extend(t.leaf("chainedSeqRuleSetCount", "uint16", f.chained_seq_rule_set_count()));
        }
        ChainedSequenceContext::Format2(f) => {
            k.extend(t.leaf("format", "uint16", 2));
            k.extend(t.coverage("coverage", f.coverage()));
            k.extend(t.leaf("chainedClassSeqRuleSetCount", "uint16", f.chained_class_seq_rule_set_count()));
        }
        ChainedSequenceContext::Format3(f) => {
            k.extend(t.leaf("format", "uint16", 3));
            for (name, coverages) in [
                ("backtrackCoverages", f.backtrack_coverages()),
                ("inputCoverages", f.input_coverages()),
                ("lookaheadCoverages", f.lookahead_coverages()),
            ] {
                k.extend(t.group(name, "array", None, |t, k| {
                    for (i, c) in coverages.iter().enumerate() {
                        k.extend(t.coverage(&i.to_string(), c));
                    }
                }));
            }
            k.extend(t.leaf("seqLookupCount", "uint16", f.seq_lookup_count()));
        }
    }
}

fn gsub(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let g = Gsub::read(fd)?;
    fields!(t, k, g, [version]);
    script_list(t, k, g.script_list());
    feature_list(t, k, g.feature_list());
    match g.lookup_list() {
        Ok(ll) => {
            let at = t.at(ll.offset_data());
            k.extend(t.group("lookupList", "LookupList", at, |t, k| {
                k.extend(t.leaf("lookupCount", "uint16", ll.lookup_count()));
                let lookups = ll.lookups();
                k.extend(t.group("lookups", "array", None, |t, k| {
                    for (i, lookup) in lookups.iter().enumerate() {
                        if t.truncated {
                            break;
                        }
                        match lookup {
                            Ok(l) => k.extend(t.group(i.to_string(), "Lookup", None, |t, k| {
                                lookup_header(t, k, l.lookup_type(), l.lookup_flag(), l.mark_filtering_set(), &GSUB_TYPES);
                                match l.subtables() {
                                    Ok(subs) => gsub_subtables(t, k, subs),
                                    Err(e) => k.extend(t.err("subtables", e)),
                                }
                            })),
                            Err(e) => k.extend(t.err(i.to_string(), e)),
                        }
                    }
                }));
            }));
        }
        Err(e) => k.extend(t.err("lookupList", e)),
    }
    if g.feature_variations().is_some() {
        k.extend(t.leaf("featureVariations", "Offset32", "present"));
    }
    Ok(())
}

fn gsub_subtables(t: &mut Tree<'_>, k: &mut Kids, subs: SubstitutionSubtables<'_>) {
    match subs {
        SubstitutionSubtables::Single(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("substFormat", "uint16", st.subst_format()));
            let (coverage, outputs): (_, Vec<u32>) = match st {
                SingleSubst::Format1(f) => {
                    let delta = i32::from(f.delta_glyph_id());
                    let coverage = f.coverage();
                    // The specification adds the delta modulo 65536.
                    let outputs = coverage
                        .as_ref()
                        .map(|c| c.iter().map(|g| (g.to_u32() as i32).wrapping_add(delta).rem_euclid(65_536) as u32).collect())
                        .unwrap_or_default();
                    (coverage, outputs)
                }
                SingleSubst::Format2(f) => (f.coverage(), f.substitute_glyph_ids().iter().map(|g| g.get().to_u32()).collect()),
            };
            k.extend(t.group("mapping", "array", None, |t, k| {
                let Ok(coverage) = coverage else { return };
                for (input, output) in coverage.iter().zip(outputs) {
                    if t.truncated {
                        break;
                    }
                    let (from, to) = (t.glyph(input.to_u32()), t.glyph(output));
                    k.extend(t.leaf(from, "GlyphID", to));
                }
            }));
        }),
        SubstitutionSubtables::Multiple(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("substFormat", "uint16", st.subst_format()));
            let coverage = st.coverage();
            let sequences = st.sequences();
            k.extend(t.group("sequences", "array", None, |t, k| {
                let Ok(coverage) = coverage else { return };
                for (input, seq) in coverage.iter().zip(sequences.iter()) {
                    if t.truncated {
                        break;
                    }
                    let from = t.glyph(input.to_u32());
                    match seq {
                        Ok(seq) => {
                            let to = join(seq.substitute_glyph_ids().iter().map(|g| t.glyph(g.get().to_u32())));
                            k.extend(t.leaf(from, "Sequence", to));
                        }
                        Err(e) => k.extend(t.err(from, e)),
                    }
                }
            }));
        }),
        SubstitutionSubtables::Alternate(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("substFormat", "uint16", st.subst_format()));
            let coverage = st.coverage();
            let sets = st.alternate_sets();
            k.extend(t.group("alternateSets", "array", None, |t, k| {
                let Ok(coverage) = coverage else { return };
                for (input, set) in coverage.iter().zip(sets.iter()) {
                    if t.truncated {
                        break;
                    }
                    let from = t.glyph(input.to_u32());
                    match set {
                        Ok(set) => {
                            let to = join(set.alternate_glyph_ids().iter().map(|g| t.glyph(g.get().to_u32())));
                            k.extend(t.leaf(from, "AlternateSet", to));
                        }
                        Err(e) => k.extend(t.err(from, e)),
                    }
                }
            }));
        }),
        SubstitutionSubtables::Ligature(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("substFormat", "uint16", st.subst_format()));
            let coverage = st.coverage();
            let sets = st.ligature_sets();
            k.extend(t.group("ligatureSets", "array", None, |t, k| {
                let Ok(coverage) = coverage else { return };
                for (first, set) in coverage.iter().zip(sets.iter()) {
                    if t.truncated {
                        break;
                    }
                    let label = t.glyph(first.to_u32());
                    let set = match set {
                        Ok(s) => s,
                        Err(e) => {
                            k.extend(t.err(label, e));
                            continue;
                        }
                    };
                    let at = t.at(set.offset_data());
                    k.extend(t.group(label.clone(), "LigatureSet", at, |t, k| {
                        for (i, lig) in set.ligatures().iter().enumerate() {
                            if t.truncated {
                                break;
                            }
                            match lig {
                                Ok(l) => {
                                    let mut parts = vec![label.clone()];
                                    parts.extend(l.component_glyph_ids().iter().map(|g| t.glyph(g.get().to_u32())));
                                    let text = format!("{} → {}", parts.join(" "), t.glyph(l.ligature_glyph().to_u32()));
                                    k.extend(t.leaf(i.to_string(), "Ligature", text));
                                }
                                Err(e) => k.extend(t.err(i.to_string(), e)),
                            }
                        }
                    }));
                }
            }));
        }),
        SubstitutionSubtables::Contextual(s) => subtables(t, k, s.len(), s.iter(), sequence_context),
        SubstitutionSubtables::ChainContextual(s) => subtables(t, k, s.len(), s.iter(), chained_context),
        SubstitutionSubtables::Reverse(s) => subtables(t, k, s.len(), s.iter(), |t, k, st| {
            k.extend(t.leaf("substFormat", "uint16", st.subst_format()));
            k.extend(t.coverage("coverage", st.coverage()));
            k.extend(t.leaf("backtrackGlyphCount", "uint16", st.backtrack_glyph_count()));
            k.extend(t.leaf("lookaheadGlyphCount", "uint16", st.lookahead_glyph_count()));
            k.extend(t.glyphs("substituteGlyphIDs", None, st.substitute_glyph_ids().iter().map(|g| g.get().to_u32())));
        }),
        SubstitutionSubtables::EmptyExtension => {
            k.extend(t.leaf("subTableCount", "uint16", 0));
        }
    }
}

// ── Variations ──────────────────────────────────────────────────────────────────────────────────

fn fvar(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let f = Fvar::read(fd)?;
    fields!(t, k, f, [version, axis_count, axis_size, instance_count, instance_size]);
    let axes = f.axes()?;
    k.extend(t.group("axes", "array", None, |t, k| {
        for (i, a) in axes.iter().enumerate() {
            if t.truncated {
                break;
            }
            k.extend(t.group(i.to_string(), "VariationAxisRecord", None, |t, k| {
                fields!(t, k, a, [axis_tag, min_value, default_value, max_value]);
                k.extend(t.leaf("flags", "uint16", hex16(a.flags())));
                fields!(t, k, a, [axis_name_id]);
            }));
        }
    }));
    let instances = f.instances()?;
    k.extend(t.group("instances", "array", None, |t, k| {
        for (i, inst) in instances.iter().enumerate() {
            if t.truncated {
                break;
            }
            match inst {
                Ok(inst) => k.extend(t.group(i.to_string(), "InstanceRecord", None, |t, k| {
                    k.extend(t.leaf("subfamilyNameID", "uint16", inst.subfamily_name_id.to_u16()));
                    k.extend(t.leaf("flags", "uint16", hex16(inst.flags)));
                    let coordinates = join(axes.iter().zip(inst.coordinates).map(|(a, c)| format!("{} {}", a.axis_tag(), c.get())));
                    k.extend(t.leaf("coordinates", "UserTuple", coordinates));
                    if let Some(id) = inst.post_script_name_id {
                        k.extend(t.leaf("postScriptNameID", "uint16", id.to_u16()));
                    }
                })),
                Err(e) => k.extend(t.err(i.to_string(), e)),
            }
        }
    }));
    Ok(())
}

fn avar(t: &mut Tree<'_>, k: &mut Kids, fd: FontData<'_>) -> Decoded {
    let a = read_fonts::tables::avar::Avar::read(fd)?;
    fields!(t, k, a, [version, axis_count]);
    let maps = a.axis_segment_maps();
    k.extend(t.group("axisSegmentMaps", "array", None, |t, k| {
        for (i, m) in maps.iter().enumerate() {
            if t.truncated {
                break;
            }
            match m {
                Ok(m) => {
                    let text = join(m.axis_value_maps().iter().map(|v| format!("{} → {}", v.from_coordinate(), v.to_coordinate())));
                    k.extend(t.leaf(i.to_string(), "SegmentMaps", text));
                }
                Err(e) => k.extend(t.err(i.to_string(), e)),
            }
        }
    }));
    if a.var_store().is_some() {
        k.extend(t.leaf("varStore", "Offset32", "present"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_compile::{Options, compile_ttf};
    use fontcraft_model::{Font, Path, Shape};

    use super::*;
    use crate::sfnt::read;

    /// A font with outlines in A and V and an A–V kerning pair, so it has a `GPOS`.
    fn kerned() -> SfntFont {
        let mut f = Font::new("Show Sans");
        for name in ["A", "V"] {
            f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
            f.glyph_mut(name).unwrap().layers[0].width = 600.0;
        }
        Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -50.0);
        read(&compile_ttf(&f, &Options::default()).unwrap().bytes, 0).unwrap()
    }

    fn child<'a>(node: &'a ShowNode, name: &str) -> &'a ShowNode {
        node.children.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no `{name}` under `{}`", node.name))
    }

    #[test]
    fn tables_show_decodes_head_cmap_and_gpos_with_glyph_names() {
        let font = kerned();
        let options = ShowOptions { tables: vec![*b"head", *b"cmap", *b"GPOS"], depth: None, glyph_names: true };
        let shown = show(&font, &options).unwrap();
        assert_eq!(shown.iter().map(|t| t.tag).collect::<Vec<_>>(), vec![*b"head", *b"cmap", *b"GPOS"]);
        assert!(shown.iter().all(|t| t.decoded && !t.truncated && t.error.is_none()));

        let head = &shown[0].root;
        assert_eq!(child(head, "unitsPerEm").value.as_deref(), Some("1000"));
        assert_eq!(child(head, "magicNumber").value.as_deref(), Some("0x5F0F3CF5"));
        assert_eq!(child(head, "unitsPerEm").kind, "uint16");

        let cmap = &shown[1].root;
        let records = child(cmap, "encodingRecords");
        let mapped_a = records.children.iter().any(|r| {
            let mappings = child(child(r, "subtable"), "mappings");
            mappings.children.iter().any(|m| m.name == "U+0041" && m.value.as_deref().is_some_and(|v| v.starts_with("A (")))
        });
        assert!(mapped_a, "U+0041 maps to A, by name");

        let pair = find(&shown, "GPOS.lookupList.lookups[0]").unwrap();
        assert!(child(pair, "lookupType").value.as_deref().unwrap().starts_with("2 "), "a pair lookup");
        let text = format!("{pair:?}");
        assert!(text.contains("xAdvance -50") && text.contains("V ("), "the A–V pair names its glyphs: {text}");
        let subtable = child(child(pair, "subtables"), "0");
        assert!(subtable.offset.is_some_and(|o| o > 0 && o < shown[2].length), "subtables carry their offset");
    }

    #[test]
    fn tables_show_counts_what_lies_below_the_depth() {
        let font = kerned();
        let shown = show(&font, &ShowOptions { tables: vec![*b"GPOS"], depth: Some(1), glyph_names: false }).unwrap();
        let lookup_list = child(&shown[0].root, "lookupList");
        assert!(lookup_list.children.is_empty() && lookup_list.more == 2, "lookupCount and lookups are counted, not shown");
        assert!(find(&shown, "GPOS.lookupList.lookups").is_err(), "a path below the depth is an error");

        let mut full = show(&font, &ShowOptions { tables: vec![*b"GPOS"], depth: None, glyph_names: false }).unwrap();
        prune(&mut full[0].root, 1);
        assert_eq!(full[0].root, shown[0].root, "pruning a full tree gives the shallow tree");

        let unknown = {
            let mut f = font.clone();
            f.insert(*b"zzzz", vec![1, 2, 3]);
            show(&f, &ShowOptions { tables: vec![*b"zzzz"], ..ShowOptions::default() }).unwrap()
        };
        assert!(!unknown[0].decoded && unknown[0].root.value.as_deref() == Some("3 bytes"));
        assert!(show(&font, &ShowOptions { tables: vec![*b"nope"], ..ShowOptions::default() }).is_err());
    }

    #[test]
    fn tables_show_survives_damaged_tables() {
        let mut font = kerned();
        let gpos = font.get(*b"GPOS").unwrap().to_vec();
        for cut in [0, 3, 10, gpos.len() / 2] {
            font.insert(*b"GPOS", gpos[..cut].to_vec());
            font.insert(*b"head", vec![0; 10]);
            let shown = show(&font, &ShowOptions::default()).unwrap();
            let head = shown.iter().find(|t| t.tag == *b"head").unwrap();
            assert!(!head.decoded && head.error.is_some(), "a short head is reported, not decoded");
        }
        assert_eq!(date(0), "1904-01-01 00:00:00");
        assert_eq!(date(3_786_912_000), "2024-01-01 00:00:00");
        let (name, indices) = split_indices("lookups[3][12]").unwrap();
        assert_eq!((name, indices), ("lookups", vec![3, 12]));
        assert!(split_indices("lookups[x]").is_err() && split_indices("[3]").is_err());
    }
}
