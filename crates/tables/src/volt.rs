//! VOLT project text to feature code (M12.13; rows `features convert`, `layout.convertSyntax`).
//!
//! Ported from fontTools `voltLib.voltToFea` (MIT, Copyright (c) 2017 Just van Rossum and
//! others; see `NOTICE`), and writes its feature code as fontTools' `feaLib.ast` prints it, so
//! the text equals `fonttools voltLib.voltToFea` output for the same project. The steps:
//!
//! 1. Glyph definitions: GDEF classes, and, with a glyph order, VOLT names replaced by the
//!    font's names by glyph id.
//! 2. Groups, sorted so that a group comes after the groups it names (feature code needs that,
//!    VOLT does not), as glyph classes.
//! 3. Anchors, settings, scripts; then lookups, last, because they name everything else.
//! 4. The feature file: glyph classes, mark classes, lookups (each after the lookups its
//!    contexts call), features, the `GDEF` table block.
//!
//! What feature code cannot hold is reported as a warning (unsupported settings, `aalt` with
//! several scripts or languages, empty substitutions); what voltLib refuses, or would fail on,
//! is an error naming the place in the project.

mod parse;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};

use parse::{Body, Coverage, Item, LookupDef, Pos, Position, ProcessMarks, Statement, Substitution};

use crate::limits::{MAX_FEATURE_SOURCE, MAX_LAYOUT_ITEMS};
use crate::sfnt::Tag;
use crate::{Result, TablesError};

/// The layout tables a VOLT project describes, in the order a converter considers them.
pub const VOLT_TABLES: [Tag; 3] = [*b"GDEF", *b"GSUB", *b"GPOS"];

/// What [`convert`] writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoltOptions {
    /// Tables to write, of `GDEF`, `GSUB` and `GPOS`; empty writes all three.
    pub tables: Vec<Tag>,
    /// The glyph order of the font the project was saved in. VOLT glyph names are replaced by
    /// the font's names, by glyph id; `None` keeps the project's names.
    pub glyph_order: Option<Vec<String>>,
}

/// Feature code for a VOLT source of at most [`crate::limits::MAX_FEATURE_SOURCE`] bytes, and
/// the warnings for what has no feature-code form.
pub fn volt_to_fea(volt: &str) -> Result<(String, Vec<String>)> {
    convert(volt, &VoltOptions::default())
}

/// [`volt_to_fea`] with a choice of tables and the glyph order of the font holding the project.
pub fn convert(volt: &str, options: &VoltOptions) -> Result<(String, Vec<String>)> {
    if volt.len() > MAX_FEATURE_SOURCE {
        return Err(TablesError::TooLarge { what: "the VOLT project", limit: MAX_FEATURE_SOURCE });
    }
    for tag in &options.tables {
        if !VOLT_TABLES.contains(tag) {
            return Err(TablesError::BadOptions(format!(
                "a VOLT project describes GDEF, GSUB and GPOS, not '{}'",
                crate::sfnt::tag_str(*tag).trim_end()
            )));
        }
    }
    let text = volt.strip_prefix('\u{feff}').unwrap_or(volt).trim_end_matches('\0');
    let statements = parse::parse(text)?;
    let tables = if options.tables.is_empty() { VOLT_TABLES.to_vec() } else { options.tables.clone() };
    let mut c = Converter::new(options.glyph_order.as_deref(), tables);
    c.collect(&statements)?;
    let fea = c.feature_file();
    Ok((fea, c.warnings))
}

/// Whether the text reads as a VOLT project: its first word is one of the project's top-level
/// keywords.
pub fn looks_like_volt(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
    let word: String = text.chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect();
    matches!(
        word.as_str(),
        "DEF_GLYPH"
            | "DEF_GROUP"
            | "DEF_SCRIPT"
            | "DEF_LOOKUP"
            | "DEF_ANCHOR"
            | "GRID_PPEM"
            | "PRESENTATION_PPEM"
            | "PPOSITIONING_PPEM"
            | "COMPILER_USEEXTENSIONLOOKUPS"
            | "COMPILER_USEPAIRPOSFORMAT2"
            | "CMAP_FORMAT"
            | "DO_NOT_TOUCH_CMAP"
    )
}

// ── feature-code pieces ────────────────────────────────────────────────────────────────────────

/// The words feature code reads as keywords, lower-cased; a glyph name equal to one of them,
/// ignoring case, is written with a backslash (fontTools `feaLib.ast.fea_keywords`).
const FEA_KEYWORDS: &[&str] = &[
    "anchor",
    "anchordef",
    "anon",
    "anonymous",
    "by",
    "contour",
    "cursive",
    "device",
    "enum",
    "enumerate",
    "excludedflt",
    "exclude_dflt",
    "feature",
    "from",
    "ignore",
    "ignorebaseglyphs",
    "ignoreligatures",
    "ignoremarks",
    "include",
    "includedflt",
    "include_dflt",
    "language",
    "languagesystem",
    "lookup",
    "lookupflag",
    "mark",
    "markattachmenttype",
    "markclass",
    "nameid",
    "null",
    "parameters",
    "pos",
    "position",
    "required",
    "righttoleft",
    "reversesub",
    "rsub",
    "script",
    "sub",
    "substitute",
    "subtable",
    "table",
    "usemarkfilteringset",
    "useextension",
    "valuerecorddef",
    "base",
    "gdef",
    "head",
    "hhea",
    "name",
    "vhea",
    "vmtx",
];

/// A glyph name as feature code writes it.
pub(crate) fn fea_glyph(name: &str) -> String {
    if FEA_KEYWORDS.contains(&name.to_lowercase().as_str()) { format!("\\{name}") } else { name.to_string() }
}

/// A glyph, a glyph class name or an inline class, as feature code holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum G {
    Name(String),
    /// A named class: its feature-code name and the key of its definition.
    ClassRef(String, String),
    Class(Vec<G>),
}

impl G {
    fn fea(&self) -> String {
        match self {
            G::Name(n) => fea_glyph(n),
            G::ClassRef(n, _) => format!("@{n}"),
            G::Class(items) => format!("[{}]", join(items)),
        }
    }
}

fn join(items: &[G]) -> String {
    items.iter().map(G::fea).collect::<Vec<_>>().join(" ")
}

/// `<device …>` for a list of (size, adjustment), or `<device NULL>`.
fn device(d: &[(i64, i64)]) -> String {
    if d.is_empty() {
        "<device NULL>".to_string()
    } else {
        format!("<device {}>", d.iter().map(|(s, a)| format!("{s} {a}")).collect::<Vec<_>>().join(", "))
    }
}

/// A `POS` as a feature-code value record (fontTools `ValueRecord.asFea`).
fn value_record(p: &Pos) -> String {
    let any = p.adv.is_some() || p.dx.is_some() || p.dy.is_some();
    if !any && p.adv_adjust.is_empty() && p.dx_adjust.is_empty() && p.dy_adjust.is_empty() {
        return "<NULL>".to_string();
    }
    if p.dx.is_none() && p.dy.is_none() {
        return p.adv.map_or_else(|| "None".to_string(), |a| a.to_string());
    }
    let (x, y, adv) = (p.dx.unwrap_or(0), p.dy.unwrap_or(0), p.adv.unwrap_or(0));
    if p.adv_adjust.is_empty() && p.dx_adjust.is_empty() && p.dy_adjust.is_empty() {
        return format!("<{x} {y} {adv} 0>");
    }
    format!("<{x} {y} {adv} 0 {} {} {} <device NULL>>", device(&p.dx_adjust), device(&p.dy_adjust), device(&p.adv_adjust))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Anchor {
    x: i64,
    y: i64,
    x_device: Vec<(i64, i64)>,
    y_device: Vec<(i64, i64)>,
}

impl Anchor {
    fn fea(&self) -> String {
        let mut s = format!("<anchor {} {}", self.x, self.y);
        if !self.x_device.is_empty() || !self.y_device.is_empty() {
            s.push_str(&format!(" {} {}", device(&self.x_device), device(&self.y_device)));
        }
        s.push('>');
        s
    }
}

fn anchor_or_null(a: Option<&Anchor>) -> String {
    a.map_or_else(|| "<anchor NULL>".to_string(), Anchor::fea)
}

/// Most components of one ligature glyph (the `GPOS` component count is 16-bit).
const MAX_LIGATURE_COMPONENTS: usize = 65_535;

/// Indentation of the statements of a top-level block.
const SHIFT: &str = "    ";

/// A lookup block, with the lookups its contextual rules call, written before it.
#[derive(Clone, Debug, Default)]
struct FeaLookup {
    name: String,
    use_extension: bool,
    statements: Vec<String>,
    chained: Vec<FeaLookup>,
}

impl FeaLookup {
    fn fea(&self) -> String {
        let ext = if self.use_extension { "useExtension " } else { "" };
        format!("lookup {} {ext}{{\n{SHIFT}{}\n}} {};\n", self.name, self.statements.join(&format!("\n{SHIFT}")), self.name)
    }
}

/// The substitution kind voltLib reads from the shape of the `SUB … WITH …` pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubKind {
    Single,
    Multiple,
    Ligature,
    Alternate,
    Reverse,
}

struct ClassDef {
    name: String,
    items: Vec<G>,
}

/// One feature's lookups by script and language, each list in the order first named.
type FeatureMap = Vec<(String, Vec<(String, Vec<(String, Vec<String>)>)>)>;

struct Converter<'a> {
    glyph_order: Option<&'a [String]>,
    tables: Vec<Tag>,
    glyph_map: HashMap<String, String>,
    /// VOLT glyph names in definition order, for ranges.
    glyph_names: Vec<String>,
    glyph_index: HashMap<String, usize>,
    groups: HashMap<String, &'a parse::GroupDef>,
    group_cache: HashMap<String, Vec<String>>,
    gdef: BTreeMap<u8, Vec<G>>,
    classes: Vec<ClassDef>,
    class_index: HashMap<String, usize>,
    marks: HashSet<String>,
    ligatures: HashMap<String, Option<i64>>,
    mark_classes: BTreeMap<(String, String), String>,
    anchors: HashMap<String, HashMap<String, BTreeMap<i64, Anchor>>>,
    use_extension: bool,
    lookup_names: HashMap<String, String>,
    lookup_names_used: HashSet<String>,
    class_names: HashMap<String, String>,
    class_names_used: HashSet<String>,
    features: FeatureMap,
    lookups: Vec<FeaLookup>,
    lookup_index: HashMap<String, usize>,
    warnings: Vec<String>,
    work: usize,
}

/// GDEF glyph classes by VOLT glyph type: base, mark, ligature, component, in the order the
/// class definitions are written.
const GDEF_TYPES: [(&str, u8); 4] = [("BASE", 0), ("MARK", 1), ("LIGATURE", 2), ("COMPONENT", 3)];

fn sanitize(name: &str, extra: &[char]) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' || extra.contains(&c) { c } else { '_' }).collect()
}

impl<'a> Converter<'a> {
    fn new(glyph_order: Option<&'a [String]>, tables: Vec<Tag>) -> Self {
        Self {
            glyph_order,
            tables,
            glyph_map: HashMap::new(),
            glyph_names: Vec::new(),
            glyph_index: HashMap::new(),
            groups: HashMap::new(),
            group_cache: HashMap::new(),
            gdef: BTreeMap::new(),
            classes: Vec::new(),
            class_index: HashMap::new(),
            marks: HashSet::new(),
            ligatures: HashMap::new(),
            mark_classes: BTreeMap::new(),
            anchors: HashMap::new(),
            use_extension: false,
            lookup_names: HashMap::new(),
            lookup_names_used: HashSet::new(),
            class_names: HashMap::new(),
            class_names_used: HashSet::new(),
            features: Vec::new(),
            lookups: Vec::new(),
            lookup_index: HashMap::new(),
            warnings: Vec::new(),
            work: 0,
        }
    }

    fn has_table(&self, tag: &[u8; 4]) -> bool {
        self.tables.contains(tag)
    }

    fn spend(&mut self, n: usize) -> Result<()> {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_LAYOUT_ITEMS {
            return Err(TablesError::TooLarge { what: "the glyphs the VOLT project expands to", limit: MAX_LAYOUT_ITEMS });
        }
        Ok(())
    }

    fn lookup_name(&mut self, name: &str) -> String {
        if let Some(n) = self.lookup_names.get(name) {
            return n.clone();
        }
        let mut res = sanitize(name, &[]);
        while self.lookup_names_used.contains(&res) {
            res.push('_');
        }
        self.lookup_names.insert(name.to_string(), res.clone());
        self.lookup_names_used.insert(res.clone());
        res
    }

    fn class_name(&mut self, name: &str) -> String {
        if let Some(n) = self.class_names.get(name) {
            return n.clone();
        }
        let mut res = sanitize(name, &['-']);
        while self.class_names_used.contains(&res) {
            res.push('_');
        }
        self.class_names.insert(name.to_string(), res.clone());
        self.class_names_used.insert(res.clone());
        res
    }

    fn glyph(&self, name: &str) -> G {
        G::Name(self.glyph_map.get(name).cloned().unwrap_or_else(|| name.to_string()))
    }

    fn class_ref(&self, name: &str, loc: parse::Loc) -> Result<G> {
        let key = name.to_lowercase();
        match self.class_index.get(&key).and_then(|i| self.classes.get(*i)) {
            Some(c) => Ok(G::ClassRef(c.name.clone(), key)),
            None => Err(parse::error_at(loc, format!("Group \"{name}\" is used but undefined."))),
        }
    }

    /// The glyphs of a feature-code glyph expression, one level deep, as fontTools'
    /// `glyphSet()` gives them: a named class yields the items of its definition.
    fn fea_glyph_set(&mut self, g: &G) -> Result<Vec<G>> {
        let items = match g {
            G::Name(_) => vec![g.clone()],
            G::ClassRef(_, key) => self.class_index.get(key).and_then(|i| self.classes.get(*i)).map(|c| c.items.clone()).unwrap_or_default(),
            G::Class(items) => items.clone(),
        };
        self.spend(items.len())?;
        Ok(items)
    }

    // ── VOLT glyph sets ──

    fn range(&mut self, start: &str, end: &str, loc: parse::Loc) -> Result<Vec<String>> {
        let (Some(a), Some(b)) = (self.glyph_index.get(start), self.glyph_index.get(end)) else {
            return Err(parse::error_at(loc, format!("RANGE \"{start}\" TO \"{end}\" names a glyph the project does not define")));
        };
        let out: Vec<String> = if a <= b { self.glyph_names.get(*a..=*b).map(<[String]>::to_vec).unwrap_or_default() } else { Vec::new() };
        self.spend(out.len())?;
        Ok(out)
    }

    fn group_glyphs(&mut self, name: &str, loc: parse::Loc, stack: &mut Vec<String>) -> Result<Vec<String>> {
        let key = name.to_lowercase();
        if let Some(cached) = self.group_cache.get(&key) {
            let out = cached.clone();
            self.spend(out.len())?;
            return Ok(out);
        }
        let Some(group) = self.groups.get(&key).copied() else {
            return Err(parse::error_at(loc, format!("Group \"{name}\" is used but undefined.")));
        };
        if stack.contains(&key) {
            return Err(parse::error_at(loc, format!("Group \"{}\" contains itself.", group.name)));
        }
        if stack.len() >= parse::MAX_NESTING {
            return Err(parse::error_at(loc, format!("groups nested more than {} deep", parse::MAX_NESTING)));
        }
        stack.push(key.clone());
        let mut out = Vec::new();
        for item in group.items.as_deref().unwrap_or_default() {
            out.extend(self.item_glyphs(item, stack)?);
        }
        stack.pop();
        self.group_cache.insert(key, out.clone());
        Ok(out)
    }

    fn item_glyphs(&mut self, item: &Item, stack: &mut Vec<String>) -> Result<Vec<String>> {
        match item {
            Item::Glyph(n) => {
                self.spend(1)?;
                Ok(vec![n.clone()])
            }
            Item::Group(n, loc) => self.group_glyphs(n, *loc, stack),
            Item::Range(a, b, loc) => self.range(a, b, *loc),
            Item::Enum(items) => {
                let mut out = Vec::new();
                for i in items {
                    out.extend(self.item_glyphs(i, stack)?);
                }
                Ok(out)
            }
        }
    }

    fn volt_glyphs(&mut self, item: &Item) -> Result<Vec<String>> {
        self.item_glyphs(item, &mut Vec::new())
    }

    // ── coverages as feature code ──

    fn coverage(&mut self, items: &[Item], flatten: bool) -> Result<Vec<G>> {
        let mut out = Vec::new();
        for item in items {
            match item {
                Item::Glyph(n) => out.push(self.glyph(n)),
                Item::Group(n, loc) => out.push(self.class_ref(n, *loc)?),
                Item::Enum(inner) => {
                    let inner = self.coverage(inner, true)?;
                    if flatten { out.extend(inner) } else { out.push(G::Class(inner)) }
                }
                Item::Range(..) => {
                    let names = self.volt_glyphs(item)?;
                    let glyphs: Vec<G> = names.iter().map(|n| self.glyph(n)).collect();
                    if flatten { out.extend(glyphs) } else { out.push(G::Class(glyphs)) }
                }
            }
        }
        self.spend(out.len())?;
        Ok(out)
    }

    fn context(&mut self, list: &[Coverage], loc: parse::Loc) -> Result<Vec<G>> {
        let mut out = Vec::new();
        for cov in list {
            let mut items = self.coverage(cov, true)?;
            let g = if items.len() > 1 {
                G::Class(items)
            } else {
                items.pop().ok_or_else(|| parse::error_at(loc, "a context names an empty coverage"))?
            };
            out.push(g);
        }
        Ok(out)
    }

    // ── collecting the statements ──

    fn collect(&mut self, statements: &'a [Statement]) -> Result<()> {
        for s in statements {
            if let Statement::Glyph(g) = s {
                self.glyph_definition(g)?;
            }
        }
        let groups: Vec<&'a parse::GroupDef> = statements
            .iter()
            .filter_map(|s| match s {
                Statement::Group(g) => Some(g),
                _ => None,
            })
            .collect();
        for g in &groups {
            self.groups.insert(g.name.to_lowercase(), g);
        }
        for g in sort_groups(&groups)? {
            self.group_definition(g)?;
        }
        for s in statements {
            match s {
                Statement::Anchor(a) if self.has_table(b"GPOS") => self.anchor_definition(a)?,
                Statement::Setting(name) => {
                    if name.starts_with("COMPILER_") {
                        if name == "COMPILER_USEEXTENSIONLOOKUPS" {
                            self.use_extension = true;
                        }
                    } else {
                        self.warnings.push(format!("Unsupported setting ignored: {name}"));
                    }
                }
                Statement::Script(script) => self.script_definition(script)?,
                _ => {}
            }
        }
        for s in statements {
            if let Statement::Lookup(lookup) = s {
                match &lookup.body {
                    Body::Pos(_) if !self.has_table(b"GPOS") => continue,
                    Body::Sub(sub) if !self.has_table(b"GSUB") && self.sub_kind(sub)?.is_some() => continue,
                    _ => {}
                }
                self.lookup_definition(lookup)?;
            }
        }
        Ok(())
    }

    fn glyph_definition(&mut self, g: &parse::GlyphDef) -> Result<()> {
        if let Some(order) = self.glyph_order {
            let font_name = usize::try_from(g.id)
                .ok()
                .and_then(|i| order.get(i))
                .ok_or_else(|| TablesError::Invalid(format!("glyph \"{}\" has id {}, beyond the font's {} glyphs", g.name, g.id, order.len())))?;
            self.glyph_map.insert(g.name.clone(), font_name.clone());
        }
        if !self.glyph_index.contains_key(&g.name) {
            self.glyph_index.insert(g.name.clone(), self.glyph_names.len());
            self.glyph_names.push(g.name.clone());
        }
        if let Some(t) = g.gtype.as_deref()
            && let Some((_, slot)) = GDEF_TYPES.iter().find(|(name, _)| *name == t)
        {
            let glyph = self.glyph(&g.name);
            self.gdef.entry(*slot).or_default().push(glyph);
        }
        match g.gtype.as_deref() {
            Some("MARK") => {
                self.marks.insert(g.name.clone());
            }
            Some("LIGATURE") => {
                self.ligatures.insert(g.name.clone(), g.components);
            }
            _ => {}
        }
        Ok(())
    }

    fn group_definition(&mut self, group: &parse::GroupDef) -> Result<()> {
        let name = self.class_name(&group.name);
        let items = self.coverage(group.items.as_deref().unwrap_or_default(), true)?;
        let key = group.name.to_lowercase();
        self.class_index.insert(key, self.classes.len());
        self.classes.push(ClassDef { name, items });
        Ok(())
    }

    fn script_definition(&mut self, script: &parse::ScriptDef) -> Result<()> {
        for lang in &script.langs {
            for feature in &lang.features {
                let mut lookups: Vec<String> = Vec::new();
                for l in &feature.lookups {
                    let base = l.split('\\').next().unwrap_or_default().to_string();
                    if !lookups.contains(&base) {
                        lookups.push(base);
                    }
                }
                let f = match self.features.iter().position(|(t, _)| *t == feature.tag) {
                    Some(i) => i,
                    None => {
                        self.features.push((feature.tag.clone(), Vec::new()));
                        self.features.len() - 1
                    }
                };
                let Some((_, scripts)) = self.features.get_mut(f) else { continue };
                let s = match scripts.iter().position(|(t, _)| *t == script.tag) {
                    Some(i) => i,
                    None => {
                        scripts.push((script.tag.clone(), Vec::new()));
                        scripts.len() - 1
                    }
                };
                let Some((_, langs)) = scripts.get_mut(s) else { continue };
                if langs.iter().any(|(t, _)| *t == lang.tag) {
                    return Err(parse::error_at(
                        script.loc,
                        format!("feature \"{}\" is listed twice for script \"{}\" language \"{}\"", feature.tag, script.tag, lang.tag),
                    ));
                }
                langs.push((lang.tag.clone(), lookups));
            }
        }
        Ok(())
    }

    fn anchor_from(pos: &Pos, loc: parse::Loc) -> Result<Anchor> {
        if !pos.adv_adjust.is_empty() {
            return Err(parse::error_at(loc, "an anchor cannot have ADV device adjustments"));
        }
        Ok(Anchor { x: pos.dx.unwrap_or(0), y: pos.dy.unwrap_or(0), x_device: pos.dx_adjust.clone(), y_device: pos.dy_adjust.clone() })
    }

    fn anchor_definition(&mut self, a: &parse::AnchorDef) -> Result<()> {
        let anchor = Self::anchor_from(&a.pos, a.loc)?;
        let name = match a.name.strip_prefix("MARK_") {
            Some(rest) => format!("MARK_{}", rest.to_lowercase()),
            None => a.name.to_lowercase(),
        };
        self.anchors.entry(a.glyph.clone()).or_default().entry(name).or_default().insert(a.component, anchor);
        Ok(())
    }

    fn sub_kind(&mut self, sub: &Substitution) -> Result<Option<SubKind>> {
        Ok(match (sub.max_src, sub.max_dest) {
            (1, 1) => {
                let mut seen = HashSet::new();
                let mut duplicate = false;
                for (src, _) in &sub.pairs {
                    for item in src {
                        if !seen.insert(self.volt_glyphs(item)?) {
                            duplicate = true;
                        }
                    }
                }
                Some(if duplicate { SubKind::Alternate } else { SubKind::Single })
            }
            (1, d) if d > 1 => Some(SubKind::Multiple),
            (s, 1) if s > 1 => Some(SubKind::Ligature),
            _ => None,
        })
    }

    fn lookup_definition(&mut self, lookup: &LookupDef) -> Result<()> {
        let mut flags = 0u16;
        if lookup.direction.as_deref() == Some("RTL") {
            flags |= 1;
        }
        if !lookup.process_base {
            flags |= 2;
        }
        let (mut attachment, mut filtering) = (None, None);
        match &lookup.process_marks {
            ProcessMarks::None => flags |= 8,
            ProcessMarks::Class(group) => attachment = Some(self.class_ref(group, lookup.loc)?),
            ProcessMarks::All => {
                if let Some(set) = &lookup.mark_glyph_set {
                    filtering = Some(self.class_ref(set, lookup.loc)?);
                }
            }
        }
        let lookupflag = (flags != 0 || attachment.is_some() || filtering.is_some()).then(|| {
            let mut words: Vec<String> = ["RightToLeft", "IgnoreBaseGlyphs", "IgnoreLigatures", "IgnoreMarks"]
                .iter()
                .enumerate()
                .filter(|(i, _)| flags & (1 << i) != 0)
                .map(|(_, w)| (*w).to_string())
                .collect();
            if let Some(a) = &attachment {
                words.push(format!("MarkAttachmentType {}", a.fea()));
            }
            if let Some(f) = &filtering {
                words.push(format!("UseMarkFilteringSet {}", f.fea()));
            }
            format!("lookupflag {};", words.join(" "))
        });
        let kind = match &lookup.body {
            Body::Sub(sub) => {
                let k = self.sub_kind(sub)?;
                // A reversal applies to single substitutions only; VOLT ignores it otherwise.
                if k == Some(SubKind::Single) && lookup.reversal { Some(SubKind::Reverse) } else { k }
            }
            Body::Pos(_) => None,
        };

        let (key, mut fea, existing) = if let Some((base, _)) = lookup.name.split_once('\\') {
            let key = base.to_lowercase();
            match self.lookup_index.get(&key).copied() {
                Some(i) => {
                    let mut l = self.lookups.get_mut(i).map(std::mem::take).unwrap_or_default();
                    l.statements.push("subtable;".to_string());
                    l.statements.push(format!("# {}", lookup.name));
                    (key, l, Some(i))
                }
                None => {
                    let mut l = FeaLookup { name: self.lookup_name(base), use_extension: self.use_extension, ..FeaLookup::default() };
                    l.statements.extend(lookupflag);
                    l.statements.push(format!("# {}", lookup.name));
                    (key, l, None)
                }
            }
        } else {
            let mut l = FeaLookup { name: self.lookup_name(&lookup.name), use_extension: self.use_extension, ..FeaLookup::default() };
            l.statements.extend(lookupflag);
            (lookup.name.to_lowercase(), l, None)
        };
        if let Some(comments) = &lookup.comments {
            fea.statements.push(format!("# {comments}"));
        }

        let mut contexts = Vec::new();
        for context in &lookup.context {
            let prefix = self.context(&context.left, lookup.loc)?;
            let suffix = self.context(&context.right, lookup.loc)?;
            contexts.push((prefix, suffix, context.except));
            // VOLT makes the plain input a rule of its own when the only context is an exception.
            if context.except && lookup.context.len() == 1 {
                contexts.push((Vec::new(), Vec::new(), false));
            }
        }

        if contexts.is_empty() {
            match &lookup.body {
                Body::Sub(sub) => {
                    if let Some(k) = kind {
                        self.gsub_lookup(sub, k, &mut fea.statements)?;
                    }
                }
                Body::Pos(pos) => self.gpos_lookup(lookup, pos, &mut fea.statements)?,
            }
        } else {
            let mut chained =
                FeaLookup { name: self.lookup_name(&format!("{} chained", lookup.name)), use_extension: self.use_extension, ..FeaLookup::default() };
            match &lookup.body {
                Body::Sub(sub) => {
                    if let Some(k) = kind {
                        self.gsub_lookup(sub, k, &mut chained.statements)?;
                    }
                }
                Body::Pos(pos) => self.gpos_lookup(lookup, pos, &mut chained.statements)?,
            }
            let chained_name = chained.name.clone();
            if kind == Some(SubKind::Reverse) {
                // Reverse substitutions cannot call other lookups: the rules go into the lookup
                // itself and no called lookup is written.
                fea.chained.clear();
            } else {
                fea.chained.push(chained);
            }
            for (prefix, suffix, ignore) in &contexts {
                match &lookup.body {
                    Body::Sub(sub) => self.gsub_context(sub, kind, prefix, suffix, *ignore, &chained_name, &mut fea.statements)?,
                    Body::Pos(pos) => self.gpos_context(pos, prefix, suffix, *ignore, &chained_name, lookup.loc, &mut fea.statements)?,
                }
            }
        }

        match existing {
            Some(i) => {
                if let Some(slot) = self.lookups.get_mut(i) {
                    *slot = fea;
                }
            }
            None => match self.lookup_index.get(&key).copied() {
                Some(i) => {
                    if let Some(slot) = self.lookups.get_mut(i) {
                        *slot = fea;
                    }
                }
                None => {
                    self.lookup_index.insert(key, self.lookups.len());
                    self.lookups.push(fea);
                }
            },
        }
        Ok(())
    }

    fn empty_warning(&mut self, sub: &Substitution) {
        self.warnings.push(format!("<volt>:{}:{}: Ignoring empty substitution", sub.loc.line, sub.loc.column));
    }

    fn gsub_lookup(&mut self, sub: &Substitution, kind: SubKind, out: &mut Vec<String>) -> Result<()> {
        let one = |v: &[G], what: &str| -> Result<G> {
            match v {
                [g] => Ok(g.clone()),
                _ => Err(parse::error_at(sub.loc, format!("a {what} substitution needs exactly one glyph, group or enum on each side"))),
            }
        };
        if kind == SubKind::Alternate {
            let mut alternates: Vec<(String, Vec<String>)> = Vec::new();
            for (key, val) in &sub.pairs {
                if key.is_empty() || val.is_empty() {
                    self.empty_warning(sub);
                    continue;
                }
                let glyphs = self.coverage(key, false)?;
                let replacements = self.coverage(val, false)?;
                let src = self.fea_glyph_set(&one(&glyphs, "alternate")?)?;
                let first = replacements.first().cloned().unwrap_or(G::Class(Vec::new()));
                let dst = self.fea_glyph_set(&first)?;
                for (s, r) in src.iter().zip(dst.iter()) {
                    let s = s.fea();
                    match alternates.iter_mut().find(|(k, _)| *k == s) {
                        Some((_, list)) => list.push(r.fea()),
                        None => alternates.push((s, vec![r.fea()])),
                    }
                }
            }
            for (glyph, replacements) in alternates {
                out.push(format!("sub {glyph} from [{}];", replacements.join(" ")));
            }
            return Ok(());
        }
        for (key, val) in &sub.pairs {
            if key.is_empty() || val.is_empty() {
                self.empty_warning(sub);
                continue;
            }
            let glyphs = self.coverage(key, false)?;
            let replacements = self.coverage(val, false)?;
            match kind {
                SubKind::Single => {
                    let (g, r) = (one(&glyphs, "single")?, one(&replacements, "single")?);
                    out.push(format!("sub {} by {};", g.fea(), r.fea()));
                }
                SubKind::Reverse | SubKind::Alternate => {}
                SubKind::Multiple => {
                    let g = one(&glyphs, "multiple")?;
                    out.push(format!("sub {} by {};", g.fea(), join(&replacements)));
                }
                SubKind::Ligature => {
                    let r = one(&replacements, "ligature")?;
                    let statement = format!("sub {} by {};", join(&glyphs), r.fea());
                    let mut sets = Vec::new();
                    for g in &glyphs {
                        sets.push(self.fea_glyph_set(g)?);
                    }
                    let n = sets.iter().map(Vec::len).max().unwrap_or(0);
                    if n > 1 {
                        // Feature code has no classes in ligature substitutions: one rule per
                        // glyph of the groups, which must be as long as each other.
                        if sets.iter().any(|s| s.len() != n && s.len() != 1) {
                            return Err(parse::error_at(sub.loc, "the groups of a ligature substitution differ in length"));
                        }
                        let mut replacement = self.fea_glyph_set(&r)?;
                        if replacement.len() == 1 {
                            let only = replacement.first().cloned().unwrap_or(G::Class(Vec::new()));
                            replacement = vec![only; n];
                        }
                        if replacement.len() < n {
                            return Err(parse::error_at(sub.loc, "the ligature group is shorter than the groups it replaces"));
                        }
                        self.spend(n.saturating_mul(sets.len()))?;
                        out.push(format!("# {statement}"));
                        for i in 0..n {
                            let parts: Vec<String> =
                                sets.iter().map(|s| if s.len() == 1 { s.first() } else { s.get(i) }.map(G::fea).unwrap_or_default()).collect();
                            let lig = replacement.get(i).map(G::fea).unwrap_or_default();
                            out.push(format!("sub {} by {lig};", parts.join(" ")));
                        }
                    } else {
                        out.push(statement);
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn gsub_context(
        &mut self,
        sub: &Substitution,
        kind: Option<SubKind>,
        prefix: &[G],
        suffix: &[G],
        ignore: bool,
        chained: &str,
        out: &mut Vec<String>,
    ) -> Result<()> {
        if kind == Some(SubKind::Reverse) {
            for (key, val) in &sub.pairs {
                if key.is_empty() || val.is_empty() {
                    self.empty_warning(sub);
                    continue;
                }
                let glyphs = self.coverage(key, false)?;
                let replacements = self.coverage(val, false)?;
                let mut s = "rsub ".to_string();
                if !prefix.is_empty() || !suffix.is_empty() {
                    if !prefix.is_empty() {
                        s.push_str(&format!("{} ", join(prefix)));
                    }
                    s.push_str(&glyphs.iter().map(|g| format!("{}'", g.fea())).collect::<Vec<_>>().join(" "));
                    if !suffix.is_empty() {
                        s.push_str(&format!(" {}", join(suffix)));
                    }
                } else {
                    s.push_str(&join(&glyphs));
                }
                s.push_str(&format!(" by {};", join(&replacements)));
                out.push(s);
            }
            return Ok(());
        }
        if kind.is_none() {
            return Err(parse::error_at(sub.loc, "this substitution in a context is not supported"));
        }
        let mut glyphs = Vec::new();
        for (key, val) in &sub.pairs {
            if key.is_empty() || val.is_empty() {
                self.empty_warning(sub);
                continue;
            }
            glyphs.extend(self.coverage(key, true)?);
        }
        if glyphs.len() > 1 {
            glyphs = vec![G::Class(glyphs)];
        }
        out.push(if ignore { ignore_rule("sub", prefix, &glyphs, suffix, true) } else { chain_rule("sub", prefix, &glyphs, suffix, chained) });
        Ok(())
    }

    fn gpos_lookup(&mut self, lookup: &LookupDef, pos: &Position, out: &mut Vec<String>) -> Result<()> {
        match pos {
            Position::Pair { first, second, pairs } => {
                for ((i1, i2), (p1, p2)) in pairs {
                    let (c1, c2) = (pair_coverage(first, *i1, lookup.loc)?, pair_coverage(second, *i2, lookup.loc)?);
                    // makeotf needs `enum` unless both sides are groups.
                    let enumerated = c1.iter().chain(c2.iter()).any(|i| !matches!(i, Item::Group(..)));
                    let g1 = single(self.coverage(c1, false)?, lookup.loc)?;
                    let g2 = single(self.coverage(c2, false)?, lookup.loc)?;
                    let record2_set = p2 != &Pos::default();
                    let e = if enumerated { "enum " } else { "" };
                    out.push(if record2_set {
                        format!("{e}pos {} {} {} {};", g1.fea(), value_record(p1), g2.fea(), value_record(p2))
                    } else {
                        format!("{e}pos {} {} {};", g1.fea(), g2.fea(), value_record(p1))
                    });
                }
            }
            Position::Single(items) => {
                for (cov, p) in items {
                    let g = single(self.coverage(cov, false)?, lookup.loc)?;
                    out.push(format!("pos {} {};", g.fea(), value_record(p)));
                }
            }
            Position::Attach { coverage, to } => self.attach(lookup, coverage, to, out)?,
            Position::Cursive { exit, enter } => {
                let mut enter_glyphs = Vec::new();
                for cov in enter {
                    for item in cov {
                        enter_glyphs.extend(self.volt_glyphs(item)?);
                    }
                }
                let mut exit_glyphs = Vec::new();
                for cov in exit {
                    for item in cov {
                        exit_glyphs.extend(self.volt_glyphs(item)?);
                    }
                }
                for name in &enter_glyphs {
                    let entry = self.anchor(name, "entry", 1, lookup.loc)?;
                    let mut exit_anchor = None;
                    if let Some(i) = exit_glyphs.iter().position(|n| n == name) {
                        exit_anchor = Some(self.anchor(name, "exit", 1, lookup.loc)?);
                        exit_glyphs.remove(i);
                    }
                    out.push(format!("pos cursive {} {} {};", self.glyph(name).fea(), entry.fea(), anchor_or_null(exit_anchor.as_ref())));
                }
                for name in &exit_glyphs {
                    let exit_anchor = self.anchor(name, "exit", 1, lookup.loc)?;
                    out.push(format!("pos cursive {} <anchor NULL> {};", self.glyph(name).fea(), exit_anchor.fea()));
                }
            }
        }
        Ok(())
    }

    fn anchor(&self, glyph: &str, name: &str, component: i64, loc: parse::Loc) -> Result<Anchor> {
        self.anchors
            .get(glyph)
            .and_then(|a| a.get(name))
            .and_then(|c| c.get(&component))
            .cloned()
            .ok_or_else(|| parse::error_at(loc, format!("glyph \"{glyph}\" has no anchor \"{name}\" on component {component}")))
    }

    fn attach(&mut self, lookup: &LookupDef, coverage: &Coverage, to: &[(Coverage, String)], out: &mut Vec<String>) -> Result<()> {
        let mut bases: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut all_marks: HashSet<String> = HashSet::new();
        for (cov, anchor_name) in to {
            // Mark classes are global in feature code but per lookup in VOLT: the class is
            // named after the anchor and the lookup, and holds only this lookup's marks.
            let class = self.class_name(&format!("{anchor_name}.{}", lookup.name));
            let anchor_name = anchor_name.to_lowercase();
            let mut marks: Vec<String> = Vec::new();
            let mut seen = HashSet::new();
            for item in cov {
                for g in self.volt_glyphs(item)? {
                    if seen.insert(g.clone()) {
                        marks.push(g);
                    }
                }
            }
            if marks.iter().any(|m| all_marks.contains(m)) {
                marks.retain(|m| !all_marks.contains(m));
                if marks.is_empty() {
                    continue;
                }
            }
            all_marks.extend(marks.iter().cloned());
            for mark in &marks {
                let anchor = self.anchor(mark, &format!("MARK_{anchor_name}"), 1, lookup.loc)?;
                let def = format!("markClass {} {} @{class};", self.glyph(mark).fea(), anchor.fea());
                self.mark_classes.insert((mark.clone(), class.clone()), def);
            }
            for item in coverage {
                for name in self.volt_glyphs(item)? {
                    let entry = match bases.iter().position(|(n, _)| *n == name) {
                        Some(i) => i,
                        None => {
                            bases.push((name, Vec::new()));
                            bases.len() - 1
                        }
                    };
                    if let Some((_, list)) = bases.get_mut(entry) {
                        let pair = (anchor_name.clone(), class.clone());
                        if !list.contains(&pair) {
                            list.push(pair);
                        }
                    }
                }
            }
        }
        let is_ligature = bases.iter().all(|(n, _)| self.ligatures.contains_key(n));
        let is_mark = bases.iter().all(|(n, _)| self.marks.contains(n));
        for (name, anchors) in &bases {
            let components = if is_ligature {
                let count = self
                    .ligatures
                    .get(name)
                    .copied()
                    .flatten()
                    .ok_or_else(|| parse::error_at(lookup.loc, format!("ligature glyph \"{name}\" has no COMPONENTS count")))?;
                let n = usize::try_from(count).unwrap_or(0);
                if n > MAX_LIGATURE_COMPONENTS {
                    return Err(parse::error_at(lookup.loc, format!("ligature glyph \"{name}\" has more than {MAX_LIGATURE_COMPONENTS} components")));
                }
                n
            } else {
                1
            };
            self.spend(components)?;
            let mut per_component: Vec<Vec<(Anchor, String)>> = vec![Vec::new(); components];
            for (mark, class) in anchors {
                let Some(on_glyph) = self.anchors.get(name).and_then(|a| a.get(mark)) else {
                    return Err(parse::error_at(lookup.loc, format!("glyph \"{name}\" has no anchor \"{mark}\"")));
                };
                for (i, list) in per_component.iter_mut().enumerate() {
                    if let Some(a) = on_glyph.get(&(i64::try_from(i).unwrap_or(i64::MAX).saturating_add(1))) {
                        list.push((a.clone(), class.clone()));
                    }
                }
            }
            let base = self.glyph(name).fea();
            let marks_text = |list: &[(Anchor, String)], indent: &str| -> String {
                list.iter().map(|(a, m)| format!("\n{indent}{} mark @{m}", a.fea())).collect::<String>()
            };
            let first = per_component.first().map(Vec::as_slice).unwrap_or_default();
            let deep = format!("{SHIFT}{SHIFT}");
            out.push(if is_mark {
                format!("pos mark {base}{};", marks_text(first, &deep))
            } else if is_ligature {
                let ligs: Vec<String> = per_component
                    .iter()
                    .map(|l| if l.is_empty() { format!("\n{deep}{SHIFT}<anchor NULL>") } else { marks_text(l, &format!("{deep}{SHIFT}")) })
                    .collect();
                format!("pos ligature {base}{};", ligs.join(&format!("\n{deep}ligComponent")))
            } else {
                format!("pos base {base}{};", marks_text(first, &deep))
            });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn gpos_context(
        &mut self,
        pos: &Position,
        prefix: &[G],
        suffix: &[G],
        ignore: bool,
        chained: &str,
        loc: parse::Loc,
        out: &mut Vec<String>,
    ) -> Result<()> {
        match pos {
            Position::Pair { first, second, pairs } => {
                for ((i1, i2), _) in pairs {
                    let g1 = single(self.coverage(pair_coverage(first, *i1, loc)?, false)?, loc)?;
                    let g2 = single(self.coverage(pair_coverage(second, *i2, loc)?, false)?, loc)?;
                    let glyphs = [g1, g2];
                    out.push(if ignore {
                        ignore_rule("pos", prefix, &glyphs, suffix, false)
                    } else {
                        chain_rule("pos", prefix, &glyphs, suffix, chained)
                    });
                }
            }
            Position::Single(items) => {
                let mut class = Vec::new();
                for (cov, _) in items {
                    class.extend(self.coverage(cov, true)?);
                }
                let glyphs = [G::Class(class)];
                out.push(if ignore {
                    ignore_rule("pos", prefix, &glyphs, suffix, false)
                } else {
                    chain_rule("pos", prefix, &glyphs, suffix, chained)
                });
            }
            Position::Attach { to, .. } => {
                let mut class = Vec::new();
                for (cov, _) in to {
                    class.extend(self.coverage(cov, true)?);
                }
                let glyphs = [G::Class(class)];
                out.push(if ignore {
                    ignore_rule("pos", prefix, &glyphs, suffix, false)
                } else {
                    chain_rule("pos", prefix, &glyphs, suffix, chained)
                });
            }
            Position::Cursive { .. } => return Err(parse::error_at(loc, "cursive attachment in a context is not supported")),
        }
        Ok(())
    }

    // ── the feature file ──

    fn feature_file(&mut self) -> String {
        let mut statements: Vec<String> = Vec::new();
        if !self.classes.is_empty() {
            statements.push("# Glyph classes".to_string());
            statements.extend(self.classes.iter().map(|c| format!("@{} = [{}];", c.name, join(&c.items))));
        }
        if !self.mark_classes.is_empty() {
            statements.push("\n# Mark classes".to_string());
            statements.extend(self.mark_classes.values().cloned());
        }
        if !self.lookups.is_empty() {
            statements.push("\n# Lookups".to_string());
            for l in &self.lookups {
                statements.extend(l.chained.iter().map(FeaLookup::fea));
                statements.push(l.fea());
            }
        }

        // Lookups the project lists in features but never defines (or that a table choice left
        // out) are dropped, then languages, scripts and features left empty.
        let mut features: FeatureMap = Vec::new();
        for (ftag, scripts) in &self.features {
            let mut kept_scripts = Vec::new();
            for (stag, langs) in scripts {
                let kept_langs: Vec<(String, Vec<String>)> = langs
                    .iter()
                    .map(|(ltag, lookups)| {
                        (ltag.clone(), lookups.iter().filter(|l| self.lookup_index.contains_key(&l.to_lowercase())).cloned().collect::<Vec<_>>())
                    })
                    .filter(|(_, l)| !l.is_empty())
                    .collect();
                if !kept_langs.is_empty() {
                    kept_scripts.push((stag.clone(), kept_langs));
                }
            }
            if !kept_scripts.is_empty() {
                features.push((ftag.clone(), kept_scripts));
            }
        }
        if !features.is_empty() {
            statements.push("# Features".to_string());
            for (ftag, mut scripts) in features {
                let aalt = ftag == "aalt";
                let mut body: Vec<String> = Vec::new();
                scripts.sort_by_key(|(t, _)| u8::from(t != "DFLT"));
                if aalt && scripts.len() > 1 {
                    self.warnings.push(
                        "FEA syntax does not allow script statements in 'aalt' feature, so only lookups from the first script will be included."
                            .to_string(),
                    );
                    scripts.truncate(1);
                }
                for (stag, mut langs) in scripts {
                    if !aalt {
                        body.push(format!("script {};", stag.trim()));
                    }
                    langs.sort_by_key(|(t, _)| u8::from(t != "dflt"));
                    if aalt && langs.len() > 1 {
                        self.warnings.push(
                            "FEA syntax does not allow language statements in 'aalt' feature, so only lookups from the first language will be included."
                                .to_string(),
                        );
                        langs.truncate(1);
                    }
                    for (ltag, lookups) in langs {
                        if !aalt {
                            let exclude = if ltag == "dflt" { "" } else { " exclude_dflt" };
                            body.push(format!("language {}{exclude};", ltag.trim()));
                        }
                        for name in lookups {
                            if let Some(l) = self.lookup_index.get(&name.to_lowercase()).and_then(|i| self.lookups.get(*i)) {
                                body.push(format!("lookup {};", l.name));
                            }
                        }
                    }
                }
                let tag = ftag.trim();
                statements.push(format!("feature {tag} {{\n{SHIFT}{}\n}} {tag};\n", body.join(&format!("\n{SHIFT}"))));
            }
        }

        if !self.gdef.is_empty() && self.has_table(b"GDEF") {
            let mut slots: [Option<String>; 4] = [None, None, None, None];
            for (name, slot) in GDEF_TYPES {
                if let Some(glyphs) = self.gdef.get(&slot) {
                    let class = format!("GDEF_{}", name.to_lowercase());
                    statements.push(format!("@{class} = [{}];", join(glyphs)));
                    if let Some(s) = slots.get_mut(usize::from(slot)) {
                        *s = Some(format!("@{class}"));
                    }
                }
            }
            let [base, mark, ligature, component] = slots.map(Option::unwrap_or_default);
            statements.push(format!("table GDEF {{\n{SHIFT}GlyphClassDef {base}, {ligature}, {mark}, {component};\n}} GDEF;\n"));
        }
        statements.join("\n")
    }
}

/// The groups in an order where each comes after the groups it names (Python's
/// `graphlib.TopologicalSorter.static_order`, whose order voltToFea's output follows).
fn sort_groups<'g>(groups: &[&'g parse::GroupDef]) -> Result<Vec<&'g parse::GroupDef>> {
    fn refs(items: &[Item], out: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Group(n, _) => out.push(n.to_lowercase()),
                Item::Enum(inner) => refs(inner, out),
                _ => {}
            }
        }
    }
    struct Node {
        key: String,
        waiting: usize,
        successors: Vec<usize>,
    }
    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut node = |key: String, nodes: &mut Vec<Node>| -> usize {
        *index.entry(key.clone()).or_insert_with(|| {
            nodes.push(Node { key, waiting: 0, successors: Vec::new() });
            nodes.len() - 1
        })
    };
    for g in groups {
        let me = node(g.name.to_lowercase(), &mut nodes);
        let mut preds = Vec::new();
        refs(g.items.as_deref().unwrap_or_default(), &mut preds);
        if let Some(n) = nodes.get_mut(me) {
            n.waiting += preds.len();
        }
        for p in preds {
            let p = node(p, &mut nodes);
            if let Some(n) = nodes.get_mut(p) {
                n.successors.push(me);
            }
        }
    }
    let mut ready: Vec<usize> = nodes.iter().enumerate().filter(|(_, n)| n.waiting == 0).map(|(i, _)| i).collect();
    let mut order = Vec::new();
    while !ready.is_empty() {
        let batch = std::mem::take(&mut ready);
        for &i in &batch {
            order.push(i);
        }
        for &i in &batch {
            let successors = nodes.get(i).map(|n| n.successors.clone()).unwrap_or_default();
            for s in successors {
                if let Some(n) = nodes.get_mut(s) {
                    n.waiting = n.waiting.saturating_sub(1);
                    if n.waiting == 0 {
                        ready.push(s);
                    }
                }
            }
        }
    }
    if order.len() < nodes.len() {
        let stuck: Vec<&str> = nodes.iter().filter(|n| n.waiting > 0).map(|n| n.key.as_str()).collect();
        return Err(TablesError::Invalid(format!("groups name each other in a cycle: {}", stuck.join(", "))));
    }
    let by_key: HashMap<String, &'g parse::GroupDef> = groups.iter().map(|g| (g.name.to_lowercase(), *g)).collect();
    let mut out = Vec::new();
    for i in order {
        let Some(n) = nodes.get(i) else { continue };
        let g = by_key.get(&n.key).ok_or_else(|| TablesError::Invalid(format!("Group \"{}\" is used but undefined.", n.key)))?;
        out.push(*g);
    }
    Ok(out)
}

fn pair_coverage(list: &[Coverage], index: i64, loc: parse::Loc) -> Result<&Coverage> {
    usize::try_from(index)
        .ok()
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| list.get(i))
        .ok_or_else(|| parse::error_at(loc, format!("ADJUST_PAIR names coverage {index}, which is not defined")))
}

fn single(mut v: Vec<G>, loc: parse::Loc) -> Result<G> {
    if v.len() == 1 {
        v.pop().ok_or_else(|| parse::error_at(loc, "an empty coverage"))
    } else {
        Err(parse::error_at(loc, "a positioning coverage must be one glyph, group or enum"))
    }
}

/// A contextual rule that calls `lookup` at each marked glyph.
fn chain_rule(kw: &str, prefix: &[G], glyphs: &[G], suffix: &[G], lookup: &str) -> String {
    let mut s = format!("{kw} ");
    if !prefix.is_empty() {
        s.push_str(&format!("{} ", join(prefix)));
    }
    s.push_str(&glyphs.iter().map(|g| format!("{}' lookup {lookup}", g.fea())).collect::<Vec<_>>().join(" "));
    if !suffix.is_empty() {
        s.push_str(&format!(" {}", join(suffix)));
    }
    s.push(';');
    s
}

/// An `ignore` rule. Without context, `ignore pos` writes the glyphs unmarked and `ignore sub`
/// marks them, as fontTools prints them.
fn ignore_rule(kw: &str, prefix: &[G], glyphs: &[G], suffix: &[G], always_mark: bool) -> String {
    let mut s = String::new();
    if always_mark || !prefix.is_empty() || !suffix.is_empty() {
        if !prefix.is_empty() {
            s.push_str(&format!("{} ", join(prefix)));
        }
        s.push_str(&glyphs.iter().map(|g| format!("{}'", g.fea())).collect::<Vec<_>>().join(" "));
        if !suffix.is_empty() {
            s.push_str(&format!(" {}", join(suffix)));
        }
    } else {
        s.push_str(&join(glyphs));
    }
    format!("ignore {kw} {s};")
}
