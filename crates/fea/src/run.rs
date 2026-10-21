//! One compile: the request's checks, then fea-rs on a thread with a large stack, then the
//! tables written to bytes.

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use fea_rs::compile::Compilation;
use fea_rs::parse::{SourceLoadError, SourceResolver};
use fea_rs::typed::{self, AstNode};
use fea_rs::{DiagnosticSet, GlyphMap, Kind, Node, NodeOrToken, Opts, ParseTree};
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
                        let remap = Remap::new(&tree, &self.root, &self.files, split.inserts, self.limits.text_bytes);
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

/// Puts fea-rs's messages about the code [`crate::split`] wrote out back at the place in the
/// files the code came from.
struct Remap<'a> {
    tree: &'a ParseTree,
    root: &'a str,
    files: &'a Loaded,
    /// [`crate::split::Split::inserts`].
    inserts: Vec<(usize, usize)>,
    /// The most stretches and include statements [`origins`] follows.
    max_steps: usize,
    /// Worked out on the first message; `None` when it cannot be.
    chunks: OnceCell<Option<Vec<Chunk>>>,
}

/// A stretch `start..end` of the original code (included files in place) and where it came
/// from.
struct Chunk {
    start: usize,
    end: usize,
    from: Origin,
}

enum Origin {
    /// `file`, from byte `local` on.
    At { file: String, local: usize },
    /// Code that these bytes of the root file include; only the statements are known.
    Included(Range<usize>),
}

impl<'a> Remap<'a> {
    fn new(tree: &'a ParseTree, root: &'a str, files: &'a Loaded, inserts: Vec<(usize, usize)>, max_steps: usize) -> Self {
        Self { tree, root, files, inserts, max_steps, chunks: OnceCell::new() }
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
    fn locate(&self, start: usize, end: usize) -> Option<(String, Range<usize>)> {
        let chunks = self.chunks.get_or_init(|| origins(self.tree, self.root, self.files, self.max_steps)).as_ref()?;
        let chunk = chunks.get(chunks.partition_point(|c| c.end <= start))?;
        let from = start.checked_sub(chunk.start)?;
        let to = end.min(chunk.end).checked_sub(chunk.start)?;
        Some(match &chunk.from {
            Origin::At { file, local } => (file.clone(), local.saturating_add(from)..local.saturating_add(to)),
            Origin::Included(statements) => (self.root.to_string(), statements.clone()),
        })
    }
}

/// The most levels of a file's tree searched for include statements.
const MAX_INCLUDE_SEARCH_DEPTH: usize = 256;

/// Where each stretch of `tree`'s code came from.
///
/// fea-rs writes included files in place and keeps the origin of each stretch in a source map
/// it does not make public (fea-rs 1.0.0); its tokens carry only their place in the joined code.
/// So the joining is done again here: each file is parsed alone, which leaves its include
/// statements in place with their byte ranges, and each statement is replaced by the file it
/// names, as fea-rs replaces it. Every stretch is then checked against the code.
///
/// fea-rs parses an included file in the context of the statement that includes it (inside a
/// feature block, say), and here it is parsed as a file of its own. When that finds the include
/// statements differently and the checks fail, a stretch from an included file is placed at the
/// root file's include statements instead, so a message never names the wrong file. `None`
/// when even the root file's stretches do not match.
fn origins(tree: &ParseTree, root: &str, files: &Loaded, max_steps: usize) -> Option<Vec<Chunk>> {
    let code: String = tree.root().iter_tokens().map(|t| t.as_str()).collect();
    let mut join = Join {
        files,
        resolver: MapResolver { files: files.clone(), root_dir: parent(root).to_string() },
        found: HashMap::new(),
        chunks: Vec::new(),
        at: 0,
        steps: max_steps,
    };
    let complete = join.file(root, 0).is_some();
    let holds = |c: &Chunk| match &c.from {
        Origin::At { file, local } => {
            files.get(file.as_str()).and_then(|t| t.get(*local..local.saturating_add(c.end.saturating_sub(c.start)))) == code.get(c.start..c.end)
        }
        Origin::Included(_) => true,
    };
    if complete && join.at == code.len() && join.chunks.iter().all(holds) {
        return Some(join.chunks);
    }
    // The root file's own stretches, with the code between them placed at the root file's
    // statements between them: its include statements.
    let root_len = files.get(root)?.len();
    let mut out = Vec::new();
    let (mut at, mut local_at) = (0usize, 0usize);
    for chunk in join.chunks.into_iter().filter(|c| matches!(&c.from, Origin::At { file, .. } if file == root)) {
        if !holds(&chunk) {
            return None;
        }
        let &Origin::At { local, .. } = &chunk.from else { return None };
        if chunk.start > at {
            out.push(Chunk { start: at, end: chunk.start, from: Origin::Included(local_at..local) });
        }
        at = chunk.end;
        local_at = local.saturating_add(chunk.end.saturating_sub(chunk.start));
        out.push(chunk);
    }
    if code.len() > at {
        out.push(Chunk { start: at, end: code.len(), from: Origin::Included(local_at..root_len) });
    }
    Some(out)
}

/// The state of [`origins`]' joining.
struct Join<'a> {
    files: &'a Loaded,
    resolver: MapResolver,
    /// Each file's include statements: their byte ranges and the files they name.
    found: HashMap<String, Rc<Vec<(Range<usize>, String)>>>,
    chunks: Vec<Chunk>,
    /// The length of the code joined so far.
    at: usize,
    /// What may still be followed: stretches added and include statements replaced.
    steps: usize,
}

impl Join<'_> {
    /// Adds `path` at the end of the joined code, its include statements replaced.
    fn file(&mut self, path: &str, depth: usize) -> Option<()> {
        if depth > crate::MAX_INCLUDE_DEPTH {
            return None;
        }
        let len = self.files.get(path)?.len();
        let includes = self.includes(path)?;
        let mut pos = 0;
        for (range, target) in includes.iter() {
            self.stretch(path, pos, range.start)?;
            pos = range.end;
            self.steps = self.steps.checked_sub(1)?;
            self.file(target, depth + 1)?;
        }
        self.stretch(path, pos, len)
    }

    fn stretch(&mut self, path: &str, from: usize, to: usize) -> Option<()> {
        let len = to.checked_sub(from)?;
        if len > 0 {
            self.steps = self.steps.checked_sub(1)?;
            let end = self.at.checked_add(len)?;
            self.chunks.push(Chunk { start: self.at, end, from: Origin::At { file: path.to_string(), local: from } });
            self.at = end;
        }
        Some(())
    }

    /// The include statements of `path`, from a parse of the file alone.
    fn includes(&mut self, path: &str) -> Option<Rc<Vec<(Range<usize>, String)>>> {
        if let Some(found) = self.found.get(path) {
            return Some(found.clone());
        }
        let text = self.files.get(path)?.clone();
        let alone = OneFile { path: path.to_string(), text };
        let (tree, _) = fea_rs::parse::parse_root(PathBuf::from(path), None, Box::new(alone)).ok()?;
        let mut statements = Vec::new();
        include_statements(tree.root(), 0, &mut statements)?;
        let found: Vec<(Range<usize>, String)> = statements
            .into_iter()
            .map(|(range, raw)| {
                let target = self.resolver.resolve_raw_path(std::path::Path::new(&raw), Some(std::path::Path::new(path)));
                (range, target.to_string_lossy().into_owned())
            })
            .collect();
        let found = Rc::new(found);
        self.found.insert(path.to_string(), found.clone());
        Some(found)
    }
}

/// The include statements under `node` in order, each with its byte range and the path it
/// names, as fea-rs collects them.
fn include_statements(node: &Node, depth: usize, out: &mut Vec<(Range<usize>, String)>) -> Option<()> {
    if depth > MAX_INCLUDE_SEARCH_DEPTH {
        return None;
    }
    for child in node.iter_children().filter_map(NodeOrToken::as_node) {
        if child.kind() == Kind::IncludeNode {
            let path = child.iter_children().filter_map(NodeOrToken::as_token).find(|t| t.kind == Kind::Path)?;
            out.push((child.range(), path.text.to_string()));
        } else {
            include_statements(child, depth + 1, out)?;
        }
    }
    Some(())
}

/// One file and nothing else, so that a parse leaves its include statements in place.
struct OneFile {
    path: String,
    text: Arc<str>,
}

impl SourceResolver for OneFile {
    fn get_contents(&self, path: &std::path::Path) -> Result<Arc<str>, SourceLoadError> {
        if path.to_string_lossy() == self.path.as_str() {
            Ok(self.text.clone())
        } else {
            Err(SourceLoadError::new(path.to_path_buf(), "not read here"))
        }
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}
