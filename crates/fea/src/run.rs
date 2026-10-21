//! One compile: the request's checks, then fea-rs on a thread with a large stack, then the
//! tables written to bytes.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fea_rs::compile::Compilation;
use fea_rs::typed::{self, AstNode};
use fea_rs::{DiagnosticSet, GlyphMap, Kind, Node, Opts, ParseTree};
use write_fonts_052::tables::gdef::GlyphClassDef;
use write_fonts_052::tables::layout::{ClassDef, FeatureList, ScriptList};

use crate::provider::{self, Provider};
use crate::resolve::{self, Loaded, MapResolver};
use crate::variations::Info;
use crate::{
    CompileRequest, Compiled, Diagnostic, FeaVariations, FeatureSummary, FirstPass, GeneratedLookups, GlyphClass, HeadValues, LayoutTable, Limits,
    MetricsValues, Os2Values, STACK_SIZE, Severity,
};

/// What fea-rs needs.
struct Job<'a> {
    root: String,
    files: Loaded,
    names: &'a [String],
    generated: Option<&'a GeneratedLookups>,
    variations: Option<&'a dyn FeaVariations>,
    first_name_id: u16,
    gpos: bool,
    lookup_per_rule_type: bool,
    limits: Limits,
    mode: Mode,
}

/// How far a run goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Parse and validate ([`crate::validate`]).
    Validate,
    /// Parse, validate and compile `GSUB` alone, without generated lookups ([`crate::first_pass`]).
    FirstPass,
    /// Everything ([`crate::compile`]).
    Build,
}

/// What a run gives back.
pub(crate) enum Output {
    Compiled(Box<Compiled>),
    FirstPass(Box<FirstPass>),
}

/// One run of `mode` on the request.
pub(crate) fn run(req: &CompileRequest, mode: Mode) -> Result<Output, Vec<Diagnostic>> {
    let build = mode == Mode::Build;
    let refuse = |m: &str| Err(vec![Diagnostic::request(Severity::Error, m)]);
    if req.glyph_order.is_empty() {
        return refuse("the font has no glyphs");
    }
    let limits = req.limits;
    if req.files.total_bytes() > limits.text_bytes {
        return Err(vec![Diagnostic::request(Severity::Error, format!("the feature code is larger than {} bytes", limits.text_bytes))]);
    }
    if build && let Some(generated) = req.generated {
        provider::check(generated, req.glyph_order.len(), limits.rules, limits.ligature_components).map_err(|d| vec![d])?;
    }
    let (root, files) = resolve::load(req.files, &limits)?;
    let job = Job {
        root,
        files,
        names: req.glyph_order.names(),
        generated: req.generated,
        variations: req.variations,
        first_name_id: req.first_name_id,
        gpos: req.gpos,
        lookup_per_rule_type: req.lookup_per_rule_type,
        limits,
        mode,
    };
    on_big_stack(|| job.run())
}

/// Runs `work` on a thread with a [`STACK_SIZE`] stack and turns a panic into a diagnostic.
/// On wasm there are no threads and a panic aborts; the checks are all there is.
pub(crate) fn on_big_stack<T: Send>(work: impl FnOnce() -> Result<T, Vec<Diagnostic>> + Send) -> Result<T, Vec<Diagnostic>> {
    if cfg!(target_arch = "wasm32") {
        return work();
    }
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name("fea-rs".into())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, work)
            .map_err(|e| vec![Diagnostic::request(Severity::Error, format!("the feature compiler could not start: {e}"))])?;
        handle.join().unwrap_or_else(|payload| {
            let what =
                payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
            Err(vec![Diagnostic::request(Severity::Error, format!("the feature compiler failed on this code ({what}); please report it"))])
        })
    })
}

impl Job<'_> {
    fn run(self) -> Result<Output, Vec<Diagnostic>> {
        let max = self.limits.diagnostics.max(1);
        let glyph_map = GlyphMap::new(self.names.iter().map(String::as_str))
            .map_err(|e| vec![Diagnostic::request(Severity::Error, format!("the glyph order cannot be used: {e}"))])?;
        let (tree, mut messages) = self.parse(&self.files, &glyph_map, max)?;
        let generated_axes = self.generated.map_or(&[][..], |g| g.axes.as_slice());
        let info = Info::new(self.variations, generated_axes).map_err(|d| vec![d])?;
        if let Some(info) = &info
            && !generated_axes.is_empty()
            && info.tags() != generated_axes
        {
            return Err(vec![Diagnostic::request(Severity::Error, "the generated values' axes are not the font's axes")]);
        }
        let validated = fea_rs::compile::validate(&tree, &glyph_map, info.as_ref());
        messages.extend(convert(&tree, &validated, max));
        messages.truncate(max);
        if validated.has_errors() {
            return Err(messages);
        }
        // Checked and validated as written, so messages name the files; then compiled from one
        // file with the lookup breaks, its messages put back where they came from.
        if self.lookup_per_rule_type && self.mode != Mode::Validate {
            let why = match crate::split::split(tree.root()) {
                Ok(None) => None,
                Ok(Some(split)) => match self.reparse(split.text, &glyph_map, info.as_ref(), max) {
                    Ok(rewritten) => {
                        let remap = Remap::new(&tree, &self.files, split.inserts);
                        return self.finish(&rewritten, &glyph_map, info.as_ref(), max, messages, &|set: &DiagnosticSet| remap.convert(set, max));
                    }
                    Err(why) => Some(why),
                },
                Err(why) => Some(why),
            };
            if let Some(why) = why {
                messages.push(Diagnostic::request(Severity::Warning, format!("rules of different types may share a lookup: {why}")));
                messages.truncate(max);
            }
        }
        self.finish(&tree, &glyph_map, info.as_ref(), max, messages, &|set: &DiagnosticSet| convert(&tree, set, max))
    }

    /// `files` parsed from the root file and checked against the limits.
    fn parse(&self, files: &Loaded, glyph_map: &GlyphMap, max: usize) -> Result<(ParseTree, Vec<Diagnostic>), Vec<Diagnostic>> {
        let root_dir = self.root.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let resolver = MapResolver { files: files.clone(), root_dir };
        let (tree, parsed) = fea_rs::parse::parse_root(PathBuf::from(&self.root), Some(glyph_map), Box::new(resolver))
            .map_err(|e| vec![Diagnostic::at(self.root.clone(), 0..0, Severity::Error, e.to_string())])?;
        let messages = convert(&tree, &parsed, max);
        if parsed.has_errors() {
            return Err(messages);
        }
        let refusals = crate::check::tree(tree.root(), self.names.len(), &self.limits, files);
        if !refusals.is_empty() {
            return Err(refusals);
        }
        Ok((tree, messages))
    }

    /// The code [`crate::split`] wrote out, parsed, checked and validated; why it cannot be
    /// used when it cannot.
    fn reparse(&self, text: String, glyph_map: &GlyphMap, info: Option<&Info>, max: usize) -> Result<ParseTree, String> {
        if text.len() > self.limits.text_bytes {
            return Err(format!("with the lookup breaks the code is larger than {} bytes", self.limits.text_bytes));
        }
        let first = |d: Vec<Diagnostic>| d.into_iter().next().map_or_else(String::new, |d| d.message);
        let files = Arc::new(BTreeMap::from([(self.root.clone(), Arc::<str>::from(text))]));
        let (tree, _) = self.parse(&files, glyph_map, max).map_err(first)?;
        let validated = fea_rs::compile::validate(&tree, glyph_map, info);
        if validated.has_errors() {
            return Err(first(convert(&tree, &validated, 1)));
        }
        Ok(tree)
    }

    /// The run from validated code on: `GSUB` alone for the first pass, every table for a build.
    /// `convert` turns fea-rs's messages about `tree` into ours.
    fn finish(
        &self,
        tree: &ParseTree,
        glyph_map: &GlyphMap,
        info: Option<&Info>,
        max: usize,
        mut messages: Vec<Diagnostic>,
        convert: &dyn Fn(&DiagnosticSet) -> Vec<Diagnostic>,
    ) -> Result<Output, Vec<Diagnostic>> {
        match self.mode {
            Mode::Validate => return Ok(Output::Compiled(Box::new(Compiled { warnings: messages, ..Compiled::default() }))),
            Mode::FirstPass => return first_pass(tree, glyph_map, info, max, messages, convert).map(|f| Output::FirstPass(Box::new(f))),
            Mode::Build => {}
        }
        let opts = Opts::new().compile_gpos(self.gpos).max_error_messages(max);
        let provider = self.generated.map(|generated| Provider { generated });
        // One variation store for the code's variable values and the generated deltas.
        match fea_rs::compile::compile::<Info, Provider>(tree, glyph_map, info, provider.as_ref(), opts) {
            Ok((compilation, warnings)) => {
                messages.extend(convert(&warnings));
                messages.truncate(max);
                self.output(tree, compilation, messages).map(|c| Output::Compiled(Box::new(c)))
            }
            Err(errors) => {
                messages.extend(convert(&errors));
                messages.truncate(max);
                Err(messages)
            }
        }
    }

    fn output(&self, tree: &ParseTree, mut c: Compilation, warnings: Vec<Diagnostic>) -> Result<Compiled, Vec<Diagnostic>> {
        if c.name.is_some() {
            c.remap_name_ids(self.first_name_id);
        }
        let gdef_from_code = c.gdef_classes.is_some();
        if let Some(generated) = self.generated.filter(|g| !gdef_from_code && !g.glyph_classes.is_empty()) {
            let class_def: ClassDef = generated.glyph_classes.iter().map(|(g, c)| (*g, gdef_class(*c) as u16)).collect();
            c.gdef.get_or_insert_with(Default::default).glyph_class_def.set(class_def);
        }
        if !self.gpos {
            c.gpos = None;
        }
        if let (Some(generated), Some(gsub)) = (self.generated.filter(|g| !g.feature_variations.is_empty()), c.gsub.as_mut()) {
            crate::provider::order_feature_variations(gsub, generated);
        }
        let mut errors = Vec::new();
        let mut dump = |name: &str, bytes: Option<Result<Vec<u8>, write_fonts_052::error::Error>>| match bytes {
            Some(Ok(b)) => Some(b),
            Some(Err(e)) => {
                errors.push(Diagnostic::request(Severity::Error, format!("the {name} table could not be written: {e}")));
                None
            }
            None => None,
        };
        let gsub = dump("GSUB", c.gsub.as_ref().map(write_fonts_052::dump_table));
        let gpos = dump("GPOS", c.gpos.as_ref().map(write_fonts_052::dump_table));
        let gdef = dump("GDEF", c.gdef.as_ref().map(write_fonts_052::dump_table));
        let base = dump("BASE", c.base.as_ref().map(write_fonts_052::dump_table));
        let name = dump("name", c.name.as_ref().map(write_fonts_052::dump_table));
        let stat = dump("STAT", c.stat.as_ref().map(write_fonts_052::dump_table));
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut features = Vec::new();
        if let Some(t) = &c.gsub {
            summarize(&mut features, LayoutTable::Gsub, &t.script_list, &t.feature_list);
        }
        if let Some(t) = &c.gpos {
            summarize(&mut features, LayoutTable::Gpos, &t.script_list, &t.feature_list);
        }
        let set = table_keywords(tree.root());
        let hhea = c.hhea.as_ref().map_or_else(MetricsValues::default, |h| MetricsValues {
            caret_offset: set.contains("CaretOffset").then_some(h.caret_offset),
            ascender: set.contains("Ascender").then_some(h.ascender.to_i16()),
            descender: set.contains("Descender").then_some(h.descender.to_i16()),
            line_gap: set.contains("LineGap").then_some(h.line_gap.to_i16()),
        });
        let vhea = c.vhea.as_ref().map_or_else(MetricsValues::default, |v| MetricsValues {
            caret_offset: None,
            ascender: set.contains("VertTypoAscender").then_some(v.ascender.to_i16()),
            descender: set.contains("VertTypoDescender").then_some(v.descender.to_i16()),
            line_gap: set.contains("VertTypoLineGap").then_some(v.line_gap.to_i16()),
        });
        let os2 = c.os2_builder.as_ref().map_or_else(Os2Values::default, |o| Os2Values {
            weight_class: o.us_weight_class,
            width_class: o.us_width_class,
            fs_type: o.fs_type,
            family_class: o.s_family_class,
            panose: o.panose_10,
            unicode_range: o.unicode_range.as_ref().map(|_| unicode_range(o)),
            vendor_id: o.ach_vend_id,
            win_ascent: o.us_win_ascent,
            win_descent: o.us_win_descent,
            code_page_range: o.code_page_range.as_ref().map(|_| code_page_range(o)),
            x_height: o.sx_height,
            cap_height: o.s_cap_height,
            typo_ascender: o.s_typo_ascender,
            typo_descender: o.s_typo_descender,
            typo_line_gap: o.s_typo_line_gap,
            lower_optical_point_size: o.us_lower_optical_point_size,
            upper_optical_point_size: o.us_upper_optical_point_size,
        });
        Ok(Compiled {
            gsub,
            gpos,
            gdef,
            base,
            name,
            stat,
            head: HeadValues { font_revision: c.head.as_ref().map(|h| h.font_revision.to_f64()) },
            hhea,
            vhea,
            os2,
            features,
            gdef_from_code,
            warnings,
        })
    }
}

/// The first pass fontc makes over the code before the feature writers run: `GSUB` alone, and
/// what the writers read from the code (its language systems, its top-level features and their
/// `# Automatic Code` markers, the classes of a `table GDEF` block).
fn first_pass(
    tree: &ParseTree,
    glyph_map: &GlyphMap,
    info: Option<&Info>,
    max: usize,
    mut messages: Vec<Diagnostic>,
    convert: &dyn Fn(&DiagnosticSet) -> Vec<Diagnostic>,
) -> Result<FirstPass, Vec<Diagnostic>> {
    let opts = Opts::new().compile_gpos(false).max_error_messages(max);
    let compilation = match fea_rs::compile::compile::<Info, Provider>(tree, glyph_map, info, None, opts) {
        Ok((compilation, warnings)) => {
            messages.extend(convert(&warnings));
            messages.truncate(max);
            compilation
        }
        Err(errors) => {
            messages.extend(convert(&errors));
            messages.truncate(max);
            return Err(messages);
        }
    };
    let gsub = match compilation.gsub.as_ref().map(write_fonts_052::dump_table).transpose() {
        Ok(bytes) => bytes,
        Err(e) => return Err(vec![Diagnostic::request(Severity::Error, format!("the GSUB table could not be written: {e}"))]),
    };
    let gdef_classes = compilation
        .gdef_classes
        .as_ref()
        .map(|classes| classes.iter().filter_map(|(g, c)| Some((*g, glyph_class(*c)?))).collect::<std::collections::BTreeMap<_, _>>());
    let mut language_systems = Vec::new();
    let mut features: Vec<(crate::Tag, bool)> = Vec::new();
    for statement in tree.typed_root().statements() {
        if let Some(system) = typed::LanguageSystem::cast(statement) {
            let system = (system.script().to_raw(), system.language().to_raw());
            if !language_systems.contains(&system) {
                language_systems.push(system);
            }
        } else if let Some(feature) = typed::Feature::cast(statement) {
            let (tag, marker) = (feature.tag().to_raw(), feature.has_insert_marker());
            match features.iter_mut().find(|(t, _)| *t == tag) {
                Some(found) => found.1 |= marker,
                None => features.push((tag, marker)),
            }
        }
    }
    Ok(FirstPass { language_systems, features, gsub, gdef_classes, warnings: messages })
}

fn glyph_class(class: GlyphClassDef) -> Option<GlyphClass> {
    match class {
        GlyphClassDef::Base => Some(GlyphClass::Base),
        GlyphClassDef::Ligature => Some(GlyphClass::Ligature),
        GlyphClassDef::Mark => Some(GlyphClass::Mark),
        GlyphClassDef::Component => Some(GlyphClass::Component),
        _ => None,
    }
}

fn gdef_class(class: GlyphClass) -> GlyphClassDef {
    match class {
        GlyphClass::Base => GlyphClassDef::Base,
        GlyphClass::Ligature => GlyphClassDef::Ligature,
        GlyphClass::Mark => GlyphClassDef::Mark,
        GlyphClass::Component => GlyphClassDef::Component,
    }
}

// The builder keeps the ranges in private wrappers; the built table has the words.
fn unicode_range(o: &fea_rs::compile::Os2Builder) -> [u32; 4] {
    let t = o.build();
    [t.ul_unicode_range_1, t.ul_unicode_range_2, t.ul_unicode_range_3, t.ul_unicode_range_4]
}

fn code_page_range(o: &fea_rs::compile::Os2Builder) -> [u32; 2] {
    let t = o.build();
    [t.ul_code_page_range_1.unwrap_or_default(), t.ul_code_page_range_2.unwrap_or_default()]
}

/// The statement keywords used in the code's `hhea` and `vhea` blocks.
fn table_keywords(root: &Node) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for table in root.iter_children().filter_map(|c| c.as_node()) {
        if matches!(table.kind(), Kind::HheaTableNode | Kind::VheaTableNode) {
            for token in table.iter_tokens() {
                out.insert(token.text.to_string());
            }
        }
    }
    out
}

fn summarize(out: &mut Vec<FeatureSummary>, table: LayoutTable, scripts: &ScriptList, features: &FeatureList) {
    for (index, record) in features.feature_records.iter().enumerate() {
        let mut language_systems = Vec::new();
        for script in &scripts.script_records {
            let has = |lang: &write_fonts_052::tables::layout::LangSys| lang.feature_indices.iter().any(|&i| usize::from(i) == index);
            if script.script.default_lang_sys.as_ref().is_some_and(has) {
                language_systems.push((script.script_tag, crate::Tag::new(b"dflt")));
            }
            for lang in &script.script.lang_sys_records {
                if has(&lang.lang_sys) {
                    language_systems.push((script.script_tag, lang.lang_sys_tag));
                }
            }
        }
        let lookups = record.feature.lookup_list_indices.iter().collect::<std::collections::BTreeSet<_>>().len();
        out.push(FeatureSummary { tag: record.feature_tag, table, language_systems, lookups });
    }
}

/// fea-rs's messages with the path of the file each is in.
fn convert(tree: &ParseTree, set: &DiagnosticSet, max: usize) -> Vec<Diagnostic> {
    set.diagnostics()
        .iter()
        .take(max)
        .map(|d| {
            let file = tree.get_source(d.message.file).map(|s| s.path().to_string_lossy().into_owned()).unwrap_or_default();
            let severity = if d.is_error() { Severity::Error } else { Severity::Warning };
            Diagnostic::at(file, d.span(), severity, d.text())
        })
        .collect()
}

/// The most bytes [`Remap`] compares while it works out which file each part of the code came
/// from.
const MAX_SOURCE_MATCH_BYTES: usize = 1 << 28;

/// Puts fea-rs's messages about the code [`crate::split`] wrote out back at the place in the
/// files the code came from.
struct Remap<'a> {
    tree: &'a ParseTree,
    files: &'a Loaded,
    /// [`crate::split::Split::inserts`].
    inserts: Vec<(usize, usize)>,
    /// Worked out on the first message; `None` when it cannot be.
    chunks: OnceCell<Option<Vec<Chunk>>>,
}

/// A stretch of the code as written out, from `start` to `end`, that comes from `file` at
/// `local`.
struct Chunk {
    start: usize,
    end: usize,
    file: String,
    local: usize,
}

impl<'a> Remap<'a> {
    fn new(tree: &'a ParseTree, files: &'a Loaded, inserts: Vec<(usize, usize)>) -> Self {
        Self { tree, files, inserts, chunks: OnceCell::new() }
    }

    /// The messages in `set` at their places in the files. A message about an inserted lookup
    /// break is left out; one whose place cannot be worked out is about the request.
    fn convert(&self, set: &DiagnosticSet, max: usize) -> Vec<Diagnostic> {
        set.diagnostics()
            .iter()
            .take(max)
            .filter_map(|d| {
                let severity = if d.is_error() { Severity::Error } else { Severity::Warning };
                let span = d.span();
                let start = self.original(span.start, false)?;
                let end = self.original(span.end, true).unwrap_or(start).max(start);
                Some(match self.locate(start, end) {
                    Some((file, range)) => Diagnostic::at(file, range, severity, d.text()),
                    None => Diagnostic::request(severity, d.text()),
                })
            })
            .collect()
    }

    /// The offset in the original code of `pos` in the written-out code; for a start (not
    /// `end`) inside inserted text, `None`.
    fn original(&self, pos: usize, end: bool) -> Option<usize> {
        let mut shift = 0usize;
        for &(at, len) in &self.inserts {
            let start = at.saturating_add(shift);
            if pos < start || (end && pos == start) {
                break;
            }
            if pos < start.saturating_add(len) {
                return end.then_some(at);
            }
            shift = shift.saturating_add(len);
        }
        pos.checked_sub(shift)
    }

    /// The file and range of `start..end` in the original code.
    fn locate(&self, start: usize, end: usize) -> Option<(String, std::ops::Range<usize>)> {
        let chunks = self.chunks.get_or_init(|| source_chunks(self.tree, self.files)).as_ref()?;
        let chunk = chunks.get(chunks.partition_point(|c| c.end <= start))?;
        let from = start.checked_sub(chunk.start)?;
        let to = end.min(chunk.end).checked_sub(chunk.start)?;
        Some((chunk.file.clone(), chunk.local.saturating_add(from)..chunk.local.saturating_add(to)))
    }
}

/// Which file each stretch of `tree`'s code comes from. fea-rs keeps this in the tree's source
/// map, whose lookup it does not make public (fea-rs 1.0.0), so the map is read from its debug
/// form: `(start..end, (FileId(n), offset))` for each stretch. The file behind each id is the one
/// whose text holds every stretch with that id at its offset. `None` when the form is not that
/// or no file matches.
fn source_chunks(tree: &ParseTree, files: &Loaded) -> Option<Vec<Chunk>> {
    let code: String = tree.root().iter_tokens().map(|t| t.as_str()).collect();
    let numbers = format!("{:?}", tree.source_map())
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(str::parse::<usize>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if numbers.len() % 4 != 0 {
        return None;
    }
    let mut budget = MAX_SOURCE_MATCH_BYTES;
    let mut candidates: std::collections::HashMap<usize, Vec<&str>> = std::collections::HashMap::new();
    for group in numbers.chunks_exact(4) {
        let &[start, end, id, local] = group else { return None };
        let text = code.get(start..end)?;
        let paths = candidates.entry(id).or_insert_with(|| files.keys().map(String::as_str).collect());
        let mut out_of_budget = false;
        paths.retain(|path| {
            let Some(held) = files.get(*path).and_then(|t| t.get(local..local.saturating_add(text.len()))) else { return false };
            // Most files differ early; only those that match the start are compared in full.
            let head = text.len().min(64);
            let cost = if held.as_bytes().get(..head) == text.as_bytes().get(..head) { text.len() } else { head };
            match budget.checked_sub(cost) {
                Some(left) => budget = left,
                None => out_of_budget = true,
            }
            cost == text.len() && held == text
        });
        if out_of_budget || paths.is_empty() {
            return None;
        }
    }
    let mut chunks = Vec::with_capacity(numbers.len() / 4);
    for group in numbers.chunks_exact(4) {
        let &[start, end, id, local] = group else { return None };
        let file = candidates.get(&id).and_then(|p| p.first())?.to_string();
        chunks.push(Chunk { start, end, file, local });
    }
    chunks.sort_by_key(|c| c.start);
    Some(chunks)
}
