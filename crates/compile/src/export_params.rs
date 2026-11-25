//! Export-time parameters of the compiler stage (`docs/plans/M7.md` § 3 › Export-time
//! parameters): Export Glyphs, Keep Glyphs, Remove Glyphs, Rename Glyphs, Reencode Glyphs and Use
//! Arabic Presentation Form Unicodes, read from the resolved parameters (export over master over
//! font) and applied to the resolved glyphs right after the glyph data is filled in.
//!
//! The order:
//! 1. Rename Glyphs swaps names: the glyph called `old` is exported as `new` and the other way
//!    round. Code points and production names stay with the names; the drawing, the export flag,
//!    the kerning and the components that use either glyph follow the glyph, as fontmake swaps
//!    them. A `new` the font lacks renames `old` alone, with a warning.
//! 2. Export Glyphs sets the export flag of the glyphs it matches.
//! 3. Keep Glyphs, then Remove Glyphs, clear the export flag (`.notdef` and `space` are always
//!    kept). A removed glyph that a kept composite uses is left to preflight, which decomposes it;
//!    with a Remove Glyphs entry `export=0`, components of glyphs that do not export are dropped
//!    instead, as the handbook describes that query.
//! 4. Reencode Glyphs sets code points; a code point it gives one glyph is taken from any other.
//! 5. Use Arabic Presentation Form Unicodes gives each exported glyph without code points the
//!    presentation-form code point the built-in glyph database records for its name
//!    ([`crate::arabic_forms`]), unless another glyph has it.
//!
//! The lists after Rename Glyphs name glyphs as they are exported: the handbook replaces a glyph
//! by renaming it and then removing the alternate that holds the old drawing, and fontmake
//! applies Remove Glyphs to the renamed font too. What changed is returned as [`ExportEdits`],
//! which the kerning writers read so that pairs follow renamed glyphs and leave removed ones out.
//!
//! Values that cannot be used are reported as warnings and skipped; the values were checked when
//! a command set them, but a file may hold anything. A list's name patterns and queries are tested
//! against every glyph, at most [`MAX_PATTERN_TESTS`] times per list. The engine's stage (Filter, PreFilter, Scale
//! to UPM, linked sources, Export Folder) runs before resolving, in `engine/src/export_steps.rs`.
//!
//! The parameters that change how the font is written rather than which glyphs it holds are read
//! here too, each where its stage needs it: [`curve_error`], [`post_type`], [`keeps_glyph_order`],
//! [`vertical_table`], [`fractional_coordinates`], [`Web`] and [`automatic_disabled`]. Save as
//! TrueType and Webfont Formats choose the files of an instance, so the engine reads them.
//!
//! Rows: `Export Glyphs`, `Keep Glyphs`, `Remove Glyphs`, `Rename Glyphs`, `Reencode Glyphs`,
//! `Use Arabic Presentation Form Unicodes` (M7.9); `Post Table Type`, `Remove post names for
//! webfonts`, `Webfont Only`, `TrueType Curve Error`, `TrueType Keep GlyphOrder`, `Export vmtx
//! Table`, `Grid Spacing`, `DisableAllAutomaticBehaviour`, `fontInfo.grid.fractionalCff` (M2.10).

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_fontinfo::glyphset::{Entry, FlagKey, GlyphFacts, GlyphList, GlyphPattern, KernSide, MetricsSide, ReencodeList, RenameList};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, Layer, NodeKind, PaletteEntry, Params, Resolved, ResolvedGlyph, Shape, param_flag};

use crate::CompileError;
use crate::info::{float, int};

const EXPORT_GLYPHS: &str = "Export Glyphs";
const KEEP_GLYPHS: &str = "Keep Glyphs";
const REMOVE_GLYPHS: &str = "Remove Glyphs";
const RENAME_GLYPHS: &str = "Rename Glyphs";
const REENCODE_GLYPHS: &str = "Reencode Glyphs";
const ARABIC_FORMS: &str = "Use Arabic Presentation Form Unicodes";

const POST_TABLE_TYPE: &str = "Post Table Type";
const REMOVE_POST_NAMES: &str = "Remove post names for webfonts";
const WEBFONT_ONLY: &str = "Webfont Only";
const CURVE_ERROR: &str = "TrueType Curve Error";
const KEEP_GLYPH_ORDER: &str = "TrueType Keep GlyphOrder";
const EXPORT_VMTX: &str = "Export vmtx Table";
const GRID_SPACING: &str = "Grid Spacing";
/// The parameter that turns automatic export steps off ([`automatic_disabled`]).
pub const DISABLE_AUTOMATIC: &str = "DisableAllAutomaticBehaviour";

/// The name IDs Webfont Only leaves out: what describes the font to its owner, not to a browser
/// (copyright, trademark, manufacturer, designer, the URLs and the licence).
const WEBFONT_DROPPED_NAMES: [u16; 8] = [0, 7, 8, 9, 11, 12, 13, 14];
/// The `OS/2` embedding bits Webfont Only writes: restricted licence embedding.
const WEBFONT_FS_TYPE: u16 = 0x0002;

/// Glyphs Keep Glyphs never drops.
const ALWAYS_KEPT: [&str; 2] = [".notdef", "space"];

/// Most names one warning lists.
const MAX_NAMED: usize = 10;

/// Most pattern and query tests one list makes: its non-exact lines times the glyphs. Exact names
/// are looked up, so they do not count. Past the cap, the lines that fit are used and the rest are
/// skipped with a warning.
pub const MAX_PATTERN_TESTS: usize = 4_000_000;

/// What the export parameters changed in the resolved glyph set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportEdits {
    /// Source glyph name → the name it is exported under (both directions of a swap are listed).
    pub renamed: BTreeMap<String, String>,
    /// Source glyph names left out of the export by Keep or Remove Glyphs.
    pub removed: BTreeSet<String>,
}

impl ExportEdits {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.renamed.is_empty() && self.removed.is_empty()
    }

    /// The name `source` is exported under, or `None` when it was removed.
    pub fn exported_name<'a>(&'a self, source: &'a str) -> Option<&'a str> {
        if self.removed.contains(source) {
            return None;
        }
        Some(self.renamed.get(source).map_or(source, String::as_str))
    }

    /// The source name of the glyph exported as `exported`: the inverse of [`Self::renamed`].
    pub fn source_names(&self) -> HashMap<&str, &str> {
        self.renamed.iter().map(|(source, exported)| (exported.as_str(), source.as_str())).collect()
    }
}

/// Removes Keep Glyphs and Remove Glyphs from `resolved`'s parameters unless `subset` is on
/// (`compile.subsetOnExport`): the reference compiler's `--no-subset` builds every glyph whose
/// export flag is set and ignores both lists. Export Glyphs and the export flags still apply.
pub fn without_subset_lists(resolved: &mut Resolved, subset: bool) {
    if !subset {
        resolved.params.remove(KEEP_GLYPHS);
        resolved.params.remove(REMOVE_GLYPHS);
    }
}

/// Applies the glyph-set parameters of `resolved.params` to `resolved.glyphs`. `font` is the
/// source the glyphs were resolved from (`None` for a resolved font compiled on its own), for the
/// glyph facts a query reads that the resolved glyphs do not carry.
pub fn apply(font: Option<&Font>, resolved: &mut Resolved, warnings: &mut Vec<String>) -> Result<ExportEdits, CompileError> {
    let mut edits = ExportEdits::default();
    let lists = Lists::read(&resolved.params, warnings);
    if lists.is_empty() {
        return Ok(edits);
    }
    let sources: Vec<String> = resolved.glyphs.iter().map(|g| g.name.clone()).collect();
    let data: OnceCell<Option<GlyphData>> = OnceCell::new();
    let ctx = Context { font, master_ids: font.map(|f| f.masters.iter().map(|m| m.id.as_str()).collect()).unwrap_or_default(), data: &data };

    if let Some(list) = &lists.rename {
        rename(&mut resolved.glyphs, list, warnings);
        for (source, g) in sources.iter().zip(&resolved.glyphs) {
            if *source != g.name {
                edits.renamed.insert(source.clone(), g.name.clone());
            }
        }
        for g in &mut resolved.glyphs {
            for c in &mut g.components {
                if let Some(new) = edits.renamed.get(&c.base) {
                    c.base.clone_from(new);
                }
            }
        }
    }

    if let Some(list) = &lists.export {
        let matcher = Matcher::new(list, EXPORT_GLYPHS, resolved.glyphs.len(), warnings);
        let hits = matcher.hits(&ctx, &resolved.glyphs, &sources);
        for (g, hit) in resolved.glyphs.iter_mut().zip(hits) {
            g.export |= hit;
        }
    }

    if lists.keep.is_some() && lists.remove.is_some() {
        warnings.push(format!("`{KEEP_GLYPHS}` and `{REMOVE_GLYPHS}` are both set: `{KEEP_GLYPHS}` was applied first, then `{REMOVE_GLYPHS}`"));
    }
    if let Some(list) = &lists.keep {
        let matcher = Matcher::new(list, KEEP_GLYPHS, resolved.glyphs.len(), warnings);
        let hits = matcher.hits(&ctx, &resolved.glyphs, &sources);
        for ((g, hit), source) in resolved.glyphs.iter_mut().zip(hits).zip(&sources) {
            if g.export && !hit && !ALWAYS_KEPT.contains(&g.name.as_str()) {
                g.export = false;
                edits.removed.insert(source.clone());
            }
        }
    }
    if let Some(list) = &lists.remove {
        let drops_unexported = list.patterns.iter().any(|p| matches!(p.entry(), Entry::Flag(FlagKey::Export, false)));
        let unexported: HashSet<String> =
            if drops_unexported { resolved.glyphs.iter().filter(|g| !g.export).map(|g| g.name.clone()).collect() } else { HashSet::new() };
        let matcher = Matcher::new(list, REMOVE_GLYPHS, resolved.glyphs.len(), warnings);
        let hits = matcher.hits(&ctx, &resolved.glyphs, &sources);
        for ((g, hit), source) in resolved.glyphs.iter_mut().zip(hits).zip(&sources) {
            if hit && g.export {
                g.export = false;
                edits.removed.insert(source.clone());
            }
        }
        if !unexported.is_empty() {
            let mut dropped = 0usize;
            for g in resolved.glyphs.iter_mut().filter(|g| g.export) {
                let before = g.components.len();
                g.components.retain(|c| !unexported.contains(&c.base));
                dropped = dropped.saturating_add(before.saturating_sub(g.components.len()));
            }
            if dropped > 0 {
                warnings.push(format!("`{REMOVE_GLYPHS}` `export=0`: {dropped} components of glyphs that do not export were left out"));
            }
        }
    }

    if let Some(list) = &lists.reencode {
        reencode(&mut resolved.glyphs, list, warnings);
    }
    if lists.arabic_forms {
        arabic_forms(&mut resolved.glyphs, &data, warnings);
    }
    Ok(edits)
}

fn on(params: &Params, name: &str) -> bool {
    params.get(name).and_then(param_flag) == Some(true)
}

/// Whether DisableAllAutomaticBehaviour is on: the export keeps the advance of non-spacing marks,
/// compiles automatic feature code as stored, passes no anchors on from components and does not
/// move the space glyph.
pub fn automatic_disabled(params: &Params) -> bool {
    on(params, DISABLE_AUTOMATIC)
}

/// Whether the glyph order follows TrueType Keep GlyphOrder, which fixes only the first two slots.
pub(crate) fn keeps_glyph_order(params: &Params) -> bool {
    on(params, KEEP_GLYPH_ORDER) && !automatic_disabled(params)
}

/// Export vmtx Table: `Some` forces or suppresses the vertical tables.
pub(crate) fn vertical_table(params: &Params) -> Option<bool> {
    params.get(EXPORT_VMTX).and_then(param_flag)
}

/// TrueType Curve Error: the cubic to quadratic tolerance in font units, at most one em.
pub(crate) fn curve_error(params: &Params, units_per_em: f64) -> Option<f64> {
    params.get(CURVE_ERROR).and_then(float).filter(|v| *v > 0.0).map(|v| v.min(units_per_em))
}

/// Post Table Type: 2 or 3. Any other value is reported and not applied.
pub(crate) fn post_type(params: &Params, warnings: &mut Vec<String>) -> Option<u8> {
    match int(params.get(POST_TABLE_TYPE)?) {
        Some(2) => Some(2),
        Some(3) => Some(3),
        _ => {
            warnings.push(format!("`{POST_TABLE_TYPE}` was not applied: it is 2 or 3"));
            None
        }
    }
}

/// Whether CFF coordinates keep hundredths of a unit: the Grid Spacing parameter, else the
/// source's grid spacing over its subdivision, is below 1 (`fontInfo.grid.fractionalCff`).
pub(crate) fn fractional_coordinates(source: Option<&Font>, params: &Params) -> bool {
    match params.get(GRID_SPACING).and_then(float).filter(|v| *v >= 0.0) {
        Some(step) => step < 1.0,
        None => source.is_some_and(|f| {
            let s = &f.info.settings;
            fontcraft_geom::grid_step(s.grid_length.unwrap_or(1.0), s.grid_sub_division.unwrap_or(1.0)) < 1.0
        }),
    }
}

/// Whether the font of these parameters is not the same in a web container (WOFF, WOFF2) as in a
/// plain file, so each needs a compile of its own ([`crate::Options::web_container`]).
pub fn differs_by_container(params: &Params) -> bool {
    on(params, REMOVE_POST_NAMES) || on(params, WEBFONT_ONLY)
}

/// [`differs_by_container`] for any master or setting of `font`.
pub fn may_differ_by_container(font: &Font) -> bool {
    let masters = font.masters.iter().map(|m| &m.params);
    let instances = font.instances.iter().map(|i| &i.params);
    std::iter::once(&font.info.params).chain(masters).chain(instances).any(differs_by_container)
}

/// What the web font parameters leave out of one font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Web {
    /// No glyph names in `post`.
    pub no_post_names: bool,
    /// Webfont Only: no desktop-only data either.
    pub only: bool,
}

impl Web {
    /// Remove post names for webfonts applies in a web container; Webfont Only applies to every
    /// file of its font, with a warning on a plain one, which cannot be installed then.
    pub(crate) fn read(params: &Params, web_container: bool, warnings: &mut Vec<String>) -> Self {
        let only = on(params, WEBFONT_ONLY);
        if only && !web_container {
            warnings.push(format!("`{WEBFONT_ONLY}` is on: desktop data was left out of a font that is not in a web container"));
        }
        Self { no_post_names: only || (web_container && on(params, REMOVE_POST_NAMES)), only }
    }

    /// Whether the name record `id` is written.
    pub(crate) fn keeps_name(self, id: u16) -> bool {
        !(self.only && WEBFONT_DROPPED_NAMES.contains(&id))
    }

    /// The embedding bits it sets.
    pub(crate) fn fs_type(self) -> Option<u16> {
        self.only.then_some(WEBFONT_FS_TYPE)
    }
}

/// The glyph-set parameters of one export, read and parsed; a value that cannot be read is
/// reported and left out.
#[derive(Default)]
struct Lists {
    rename: Option<RenameList>,
    export: Option<GlyphList>,
    keep: Option<GlyphList>,
    remove: Option<GlyphList>,
    reencode: Option<ReencodeList>,
    arabic_forms: bool,
}

impl Lists {
    fn read(params: &Params, warnings: &mut Vec<String>) -> Self {
        let mut lists = Self::default();
        if params.is_empty() {
            return lists;
        }
        lists.rename = parsed(params, RENAME_GLYPHS, RenameList::parse, warnings);
        lists.export = parsed(params, EXPORT_GLYPHS, GlyphList::parse, warnings);
        lists.keep = parsed(params, KEEP_GLYPHS, GlyphList::parse, warnings);
        lists.remove = parsed(params, REMOVE_GLYPHS, GlyphList::parse, warnings);
        lists.reencode = parsed(params, REENCODE_GLYPHS, ReencodeList::parse, warnings);
        lists.arabic_forms = params.get(ARABIC_FORMS).and_then(fontcraft_fontinfo::value::flag).unwrap_or(false);
        lists
    }

    fn is_empty(&self) -> bool {
        self.rename.is_none()
            && self.export.is_none()
            && self.keep.is_none()
            && self.remove.is_none()
            && self.reencode.is_none()
            && !self.arabic_forms
    }
}

fn parsed<T, E: std::fmt::Display>(params: &Params, name: &str, parse: impl Fn(&[&str]) -> Result<T, E>, warnings: &mut Vec<String>) -> Option<T> {
    let value = params.get(name)?;
    let Some(lines) = fontcraft_fontinfo::value::lines(value) else {
        warnings.push(format!("`{name}` was not applied: its value is not a list of text"));
        return None;
    };
    match parse(lines.as_slice()) {
        Ok(list) => Some(list),
        Err(e) => {
            warnings.push(format!("`{name}` was not applied: {e}"));
            None
        }
    }
}

/// Swaps names pair by pair; later pairs see the names earlier pairs gave.
fn rename(glyphs: &mut [ResolvedGlyph], list: &RenameList, warnings: &mut Vec<String>) {
    let mut index: HashMap<String, usize> = HashMap::with_capacity(glyphs.len());
    for (i, g) in glyphs.iter().enumerate() {
        index.entry(g.name.clone()).or_insert(i);
    }
    let mut missing: Vec<&str> = Vec::new();
    let mut alone: Vec<String> = Vec::new();
    for (old, new) in &list.pairs {
        let Some(&i) = index.get(old) else {
            missing.push(old);
            continue;
        };
        match index.get(new).copied() {
            Some(j) if j == i => {}
            Some(j) => {
                if let Some((a, b)) = pair_mut(glyphs, i, j) {
                    std::mem::swap(&mut a.name, &mut b.name);
                    std::mem::swap(&mut a.unicodes, &mut b.unicodes);
                    std::mem::swap(&mut a.production_name, &mut b.production_name);
                    index.insert(old.clone(), j);
                    index.insert(new.clone(), i);
                }
            }
            None => {
                if let Some(g) = glyphs.get_mut(i) {
                    g.name.clone_from(new);
                    // The production name the old name had is not the new name's.
                    g.production_name = None;
                    index.remove(old);
                    index.insert(new.clone(), i);
                    alone.push(format!("`{old}` to `{new}`"));
                }
            }
        }
    }
    if !missing.is_empty() {
        warnings.push(format!("`{RENAME_GLYPHS}`: the font has no glyph called {}; those lines were skipped", listed(&missing)));
    }
    if !alone.is_empty() {
        let alone: Vec<&str> = alone.iter().map(String::as_str).collect();
        warnings.push(format!("`{RENAME_GLYPHS}`: no glyph had the new name, so the glyph was renamed without a swap: {}", listed(&alone)));
    }
}

/// Both elements, when `i` and `j` are two different indices of `v`.
fn pair_mut<T>(v: &mut [T], i: usize, j: usize) -> Option<(&mut T, &mut T)> {
    let (lo, hi) = (i.min(j), i.max(j));
    if lo == hi {
        return None;
    }
    let (left, right) = v.split_at_mut_checked(hi)?;
    Some((left.get_mut(lo)?, right.first_mut()?))
}

fn reencode(glyphs: &mut [ResolvedGlyph], list: &ReencodeList, warnings: &mut Vec<String>) {
    let index: HashMap<&str, usize> = glyphs.iter().enumerate().rev().map(|(i, g)| (g.name.as_str(), i)).collect();
    // The glyph each code point goes to (a later line wins), and each glyph's new code points.
    let mut claims: HashMap<u32, usize> = HashMap::new();
    let mut assigned: HashMap<usize, &[u32]> = HashMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for (name, cps) in &list.entries {
        let Some(&i) = index.get(name.as_str()) else {
            missing.push(name);
            continue;
        };
        for cp in cps {
            claims.insert(*cp, i);
        }
        assigned.insert(i, cps.as_slice());
    }
    if !missing.is_empty() {
        warnings.push(format!("`{REENCODE_GLYPHS}`: the font has no glyph called {}; those lines were skipped", listed(&missing)));
    }
    for (i, g) in glyphs.iter_mut().enumerate() {
        if let Some(cps) = assigned.get(&i) {
            g.unicodes = cps.to_vec();
        }
        g.unicodes.retain(|cp| claims.get(cp).is_none_or(|owner| *owner == i));
    }
}

fn arabic_forms(glyphs: &mut [ResolvedGlyph], data: &OnceCell<Option<GlyphData>>, warnings: &mut Vec<String>) {
    let Some(data) = data.get_or_init(|| GlyphData::new().ok()) else {
        warnings.push(format!("`{ARABIC_FORMS}` was not applied: the glyph database could not be read"));
        return;
    };
    let mut used: HashSet<u32> = glyphs.iter().filter(|g| g.export).flat_map(|g| g.unicodes.iter().copied()).collect();
    for g in glyphs.iter_mut().filter(|g| g.export && g.unicodes.is_empty()) {
        if let Some(cp) = crate::arabic_forms::code_point(data, &g.name)
            && used.insert(cp)
        {
            g.unicodes.push(cp);
        }
    }
}

/// `names` for a message: the first few, then how many more.
fn listed(names: &[&str]) -> String {
    let shown: Vec<String> = names.iter().take(MAX_NAMED).map(|n| format!("`{n}`")).collect();
    match names.len().checked_sub(MAX_NAMED) {
        Some(more) if more > 0 => format!("{} and {more} more", shown.join(", ")),
        _ => shown.join(", "),
    }
}

/// A Remove, Keep or Export Glyphs list ready to match: plain names in a set, the rest in order.
struct Matcher<'a> {
    names: HashSet<&'a str>,
    others: Vec<&'a GlyphPattern>,
}

impl<'a> Matcher<'a> {
    fn new(list: &'a GlyphList, param: &str, glyphs: usize, warnings: &mut Vec<String>) -> Self {
        let mut names = HashSet::new();
        let mut others = Vec::new();
        let mut unknown: Vec<&str> = Vec::new();
        let mut colors: Vec<&str> = Vec::new();
        for p in &list.patterns {
            if let Some(name) = p.exact_name() {
                names.insert(name);
            } else if let Some(key) = p.unknown_key() {
                unknown.push(key);
            } else if matches!(p.entry(), Entry::ColorIndex(_)) {
                colors.push(p.text());
            } else {
                others.push(p);
            }
        }
        if !unknown.is_empty() {
            warnings.push(format!("`{param}`: unknown query keys match no glyph: {}", listed(&unknown)));
        }
        if !colors.is_empty() {
            warnings.push(format!(
                "`{param}`: removing the shapes of one palette colour is not available yet, so these lines were skipped: {}",
                listed(&colors)
            ));
        }
        let fit = MAX_PATTERN_TESTS.checked_div(glyphs).unwrap_or(usize::MAX);
        if others.len() > fit {
            let skipped: Vec<&str> = others.get(fit..).unwrap_or_default().iter().map(|p| p.text()).collect();
            warnings.push(format!(
                "`{param}`: {} patterns and queries on {glyphs} glyphs pass the limit of {MAX_PATTERN_TESTS} tests, so the first {fit} were used and these skipped: {}",
                others.len(),
                listed(&skipped)
            ));
            others.truncate(fit);
        }
        Self { names, others }
    }

    /// Whether each glyph matches.
    fn hits(&self, ctx: &Context, glyphs: &[ResolvedGlyph], sources: &[String]) -> Vec<bool> {
        glyphs
            .iter()
            .zip(sources)
            .map(|(g, source)| {
                if self.names.contains(g.name.as_str()) {
                    return true;
                }
                if self.others.is_empty() {
                    return false;
                }
                let facts = Facts::new(ctx, g, source);
                self.others.iter().any(|p| p.matches(&facts))
            })
            .collect()
    }
}

/// What the facts read besides the resolved glyph.
struct Context<'a> {
    font: Option<&'a Font>,
    master_ids: Vec<&'a str>,
    data: &'a OnceCell<Option<GlyphData>>,
}

/// One resolved glyph, with its source glyph (found by the name it had before renaming) for what
/// the resolved glyph does not carry.
struct Facts<'a> {
    ctx: &'a Context<'a>,
    glyph: &'a ResolvedGlyph,
    source_name: &'a str,
    source: Option<&'a Glyph>,
    script: OnceCell<Option<String>>,
    compatible: OnceCell<bool>,
}

impl<'a> Facts<'a> {
    fn new(ctx: &'a Context<'a>, glyph: &'a ResolvedGlyph, source_name: &'a str) -> Self {
        let source = ctx.font.and_then(|f| f.glyph(source_name)).map(std::sync::Arc::as_ref);
        Self { ctx, glyph, source_name, source, script: OnceCell::new(), compatible: OnceCell::new() }
    }

    fn layers(&self) -> &'a [Layer] {
        match self.source {
            Some(g) => &g.layers,
            None => &[],
        }
    }
}

impl GlyphFacts for Facts<'_> {
    fn name(&self) -> &str {
        &self.glyph.name
    }

    fn code_points(&self) -> &[u32] {
        &self.glyph.unicodes
    }

    fn note(&self) -> Option<&str> {
        self.source.map(|g| g.info.note.as_str()).filter(|n| !n.is_empty())
    }

    fn script(&self) -> Option<&str> {
        self.script
            .get_or_init(|| {
                if let Some(script) = self.source.and_then(|g| g.info.script.clone()) {
                    return Some(script);
                }
                let data = self.ctx.data.get_or_init(|| GlyphData::new().ok()).as_ref()?;
                let codes = self.source.map_or(self.glyph.unicodes.as_slice(), |g| g.unicodes.as_slice());
                data.lookup(self.source_name, codes).script.map(|s| s.value)
            })
            .as_deref()
    }

    fn category(&self) -> Option<&str> {
        self.glyph.category.as_deref()
    }

    fn sub_category(&self) -> Option<&str> {
        self.glyph.sub_category.as_deref()
    }

    fn production_name(&self) -> Option<&str> {
        self.glyph.production_name.as_deref()
    }

    fn metrics_key(&self, side: MetricsSide) -> Option<&str> {
        let keys = &self.source?.info.metrics_keys;
        match side {
            MetricsSide::Left => keys.left.as_deref(),
            MetricsSide::Right => keys.right.as_deref(),
            MetricsSide::Width => keys.width.as_deref(),
            MetricsSide::Top => keys.top.as_deref(),
            MetricsSide::Bottom => keys.bottom.as_deref(),
            MetricsSide::VertWidth => keys.vert_width.as_deref(),
        }
    }

    fn kerning_group(&self, side: KernSide) -> Option<&str> {
        let groups = &self.source?.kern_groups;
        match side {
            KernSide::Left => groups.left.as_deref(),
            KernSide::Right => groups.right.as_deref(),
            KernSide::Top => groups.top.as_deref(),
            KernSide::Bottom => groups.bottom.as_deref(),
        }
    }

    fn layer_count(&self) -> usize {
        if self.source.is_some() { self.layers().len() } else { 1 }
    }

    fn masters_compatible(&self) -> bool {
        *self
            .compatible
            .get_or_init(|| compatible(self.layers().iter().filter(|l| l.is_master_layer() && self.ctx.master_ids.contains(&l.id.as_str()))))
    }

    fn exports(&self) -> bool {
        self.glyph.export
    }

    fn uses_color_index(&self, index: i64) -> bool {
        let Ok(index) = u16::try_from(index) else { return false };
        self.layers().iter().any(|l| match l.info.attrs.color_palette {
            Some(PaletteEntry::Index(i)) => i == index,
            Some(PaletteEntry::Foreground) => index == u16::MAX,
            None => false,
        })
    }

    fn is_any_color_glyph(&self) -> bool {
        self.layers().iter().any(|l| {
            let a = &l.info.attrs;
            a.color_palette.is_some() || a.full_color || a.sbix_size.is_some() || a.svg
        })
    }

    fn is_apple_color_glyph(&self) -> bool {
        self.layers().iter().any(|l| l.info.attrs.sbix_size.is_some())
    }

    fn has_special_layers(&self) -> bool {
        self.layers().iter().any(|l| !l.info.attrs.coordinates.is_empty() || !l.info.attrs.axis_rules.is_empty())
    }
}

/// One shape or anchor of a layer, as far as interpolation cares.
#[derive(PartialEq)]
enum Part<'a> {
    Path(bool, Vec<NodeKind>),
    Component(&'a str),
    Other,
    Anchor(&'a str),
}

fn parts(layer: &Layer) -> Vec<Part<'_>> {
    let shapes = layer.shapes.iter().filter(|s| !matches!(s, Shape::Image(_))).map(|s| match s {
        Shape::Path(p) => Part::Path(p.closed, p.nodes.iter().map(|n| n.kind).collect()),
        Shape::Component(c) => Part::Component(&c.glyph),
        _ => Part::Other,
    });
    shapes.chain(layer.anchors.iter().map(|a| Part::Anchor(&a.name))).collect()
}

/// Whether the master layers have the same shapes in the same order (each path the same node
/// kinds, each component the same base) and the same anchors: the core of the compatibility
/// check (`fontcraft_interp::compat`), without its scope of masters.
fn compatible<'a>(mut layers: impl Iterator<Item = &'a Layer>) -> bool {
    let Some(first) = layers.next() else { return true };
    let first = parts(first);
    layers.all(|l| parts(l) == first)
}

#[cfg(test)]
#[path = "export_params_tests.rs"]
mod tests;
