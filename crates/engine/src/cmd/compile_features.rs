//! The `features` group of `font.compile` and `font.compilePlan` (M2.12): feature code that
//! replaces the source's, read from a file or converted from FontDame (MTI) layout sources; the
//! folder its `include` statements read from; the feature writers that run; whether `GDEF` is
//! generated; a debug file of the assembled code; and an empty `GSUB` stub.
//!
//! The replacement code and the writer list are applied to a copy of the font before the build,
//! as the project compile applies its feature file: the code becomes the font's only feature
//! entry, and the writer list becomes the source's feature-writer setting
//! ([`fontcraft_features::FEATURE_WRITERS_KEY`]), which every writer reads. Every path is checked
//! against the automation roots here, since the group's paths are not top-level parameters.
//!
//! Rows: the `compile` options `--features`, `--include-dir`, `--mti-source`,
//! `--feature-writer`, `--generate-gdef`, `--variable-features`,
//! `--expand-features-to-instances`, `--debug-feature-file`, `--stub-gsub`,
//! `--feature-names-override`; `compile.featureWriterModes`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use fontcraft_compile::Options;
use fontcraft_features::FEATURE_WRITERS_KEY;
use fontcraft_model::Font;
use fontcraft_model::plist::{self, Array, Dict, Str};
use fontcraft_tables::fontdame::{self, FontDameOptions};

use crate::params::Params;
use crate::{EngineError, Result};

/// The keys of the `features` group.
const KEYS: [&str; 10] =
    ["file", "includeDir", "mtiSource", "writers", "generateGdef", "variable", "expandToInstances", "debugFile", "stubGsub", "namesOverride"];

/// Most items `features.writers` may hold.
const MAX_WRITERS: usize = 32;

/// The writers that run when the source has no feature-writer setting, in the reference's order.
const DEFAULT_WRITERS: [&str; 4] = ["KernFeatureWriter", "MarkFeatureWriter", "GdefFeatureWriter", "CursFeatureWriter"];

/// The `GDEF` writer's class name.
const GDEF_WRITER: &str = "GdefFeatureWriter";

/// One item of `features.writers`.
#[derive(Clone, Debug, PartialEq)]
enum WriterItem {
    /// `...`: the source's writers, or the default ones when it lists none.
    Source,
    /// A writer by class name, with its mode when one is given.
    Writer { class: &'static str, mode: Option<&'static str> },
}

/// What the `features` group asks for.
#[derive(Clone, Debug, Default)]
pub(super) struct FeatureRequest {
    /// Feature code that replaces the source's.
    code: Option<String>,
    /// The folder the code's `include` statements read from.
    include_dir: Option<PathBuf>,
    /// The writers to run; `Some(empty)` runs none. `None` keeps the source's setting.
    writers: Option<Vec<WriterItem>>,
    generate_gdef: bool,
    /// Where the assembled code of the first font is written.
    debug_file: Option<PathBuf>,
    stub_gsub: bool,
    /// What the FontDame conversion could not carry into feature code.
    notes: Vec<String>,
}

impl FeatureRequest {
    /// Reads the `features` group of `p`. `project`: the request is a project compile, which
    /// reads its own feature file.
    pub(super) fn read(p: &Params, font: &Font, project: bool) -> Result<Self> {
        let mut out = FeatureRequest { generate_gdef: true, ..FeatureRequest::default() };
        let Some(group) = p.opt_object("features")? else { return Ok(out) };
        if let Some(key) = group.keys().find(|k| !KEYS.contains(&k.as_str())) {
            return Err(EngineError::Unsupported(format!("`features.{key}`")));
        }
        let flag = |key: &str| -> Result<Option<bool>> {
            match group.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Bool(b)) => Ok(Some(*b)),
                Some(_) => Err(p.bad(format!("`features.{key}` must be true or false"))),
            }
        };
        if flag("variable")? == Some(false) {
            return Err(EngineError::Unsupported(
                "`features.variable: false` (layout compiled per master and the tables merged): a variable font compiles one feature set with variable values".into(),
            ));
        }
        if flag("expandToInstances")? == Some(true) {
            return Err(EngineError::Unsupported(
                "`features.expandToInstances`: no instance sources are written; each instance compiles the source's feature code, its includes read from the source's folder".into(),
            ));
        }
        if flag("namesOverride")? == Some(false) {
            return Err(EngineError::Unsupported(
                "`features.namesOverride: false`: the name records of the feature code always replace the generated ones".into(),
            ));
        }
        out.generate_gdef = flag("generateGdef")?.unwrap_or(true);
        out.stub_gsub = flag("stubGsub")?.unwrap_or(false);
        let file = path(p, group, "file")?;
        let mti = path(p, group, "mtiSource")?;
        if (file.is_some() || mti.is_some()) && project {
            return Err(p.bad("a project compile reads its feature code from `featureFile`: leave out `features.file` and `features.mtiSource`"));
        }
        if let Some(file) = file {
            if mti.is_some() {
                return Err(p.bad("give the feature code once: `features.file` or `features.mtiSource`"));
            }
            out.code = Some(super::compile_project::text(p, "`features.file`", &file)?);
            out.include_dir = file.parent().filter(|d| crate::file_access::check_read(d).is_ok()).map(Path::to_path_buf);
        }
        if let Some(dir) = path(p, group, "includeDir")? {
            crate::file_access::check_read(&dir).map_err(|e| p.bad(format!("`features.includeDir`: {e}")))?;
            if !dir.is_dir() {
                return Err(p.bad(format!("`features.includeDir`: {} is not a folder", dir.display())));
            }
            out.include_dir = Some(dir);
        }
        out.writers = match group.get("writers") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(writer_items(p, std::slice::from_ref(s))?),
            Some(Value::Array(items)) => {
                if items.len() > MAX_WRITERS {
                    return Err(p.bad(format!("`features.writers` holds at most {MAX_WRITERS} items")));
                }
                let specs: Vec<String> = items
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or_else(|| p.bad("`features.writers` items are writer names")))
                    .collect::<Result<_>>()?;
                Some(writer_items(p, &specs)?)
            }
            Some(_) => return Err(p.bad("`features.writers` is \"none\" or a list of writer names")),
        };
        if let Some(plist_path) = mti {
            if out.writers.is_some() {
                return Err(p.bad("FontDame sources run no feature writer: leave out `features.writers`"));
            }
            let (code, notes) = mti_code(p, &plist_path, font)?;
            out.code = Some(code);
            out.notes = notes;
            out.writers = Some(Vec::new());
            if out.include_dir.is_none() {
                out.include_dir = plist_path.parent().filter(|d| crate::file_access::check_read(d).is_ok()).map(Path::to_path_buf);
            }
        }
        if let Some(debug) = path(p, group, "debugFile")? {
            crate::file_access::check_write(&debug).map_err(|e| p.bad(format!("`features.debugFile`: {e}")))?;
            out.debug_file = Some(debug);
        }
        Ok(out)
    }

    /// Whether a debug file is asked for.
    pub(super) fn has_debug_file(&self) -> bool {
        self.debug_file.is_some()
    }

    /// Sets the compile options the group decides. Runs after the source's include folder is set,
    /// which a given folder replaces.
    pub(super) fn set_options(&self, options: &mut Options) {
        options.stub_gsub = self.stub_gsub;
        if let Some(dir) = &self.include_dir {
            options.feature_dir = Some(dir.clone());
        }
    }

    /// `font` with the replacement code and the writer setting applied; `None` when the group
    /// changes neither.
    pub(super) fn font(&self, font: &Font, warnings: &mut Vec<String>) -> Option<Font> {
        warnings.extend(self.notes.iter().map(|n| format!("FontDame sources: {n}")));
        let setting = self.setting(font, warnings);
        if self.code.is_none() && setting.is_none() {
            return None;
        }
        let mut out = font.clone();
        if let Some(code) = &self.code {
            let entry = fontcraft_model::Snippet { code: code.clone(), ..Default::default() };
            out.features = Arc::new(fontcraft_model::Features { prefixes: vec![entry], ..Default::default() });
        }
        if let Some(setting) = setting {
            Arc::make_mut(&mut out.info).user_data.0.insert(FEATURE_WRITERS_KEY, setting);
        }
        Some(out)
    }

    /// The feature-writer setting the group gives `font`; `None` keeps the source's.
    fn setting(&self, font: &Font, warnings: &mut Vec<String>) -> Option<plist::Value> {
        if self.writers.is_none() && self.generate_gdef {
            return None;
        }
        let source = font.info.user_data.get(FEATURE_WRITERS_KEY).or_else(|| font.default_master().ok()?.user_data.get(FEATURE_WRITERS_KEY));
        let mut source_items = || -> Vec<plist::Value> {
            match source {
                None => DEFAULT_WRITERS.iter().map(|c| writer_entry(c, None)).collect(),
                Some(plist::Value::Array(a)) => a.items.clone(),
                Some(_) => {
                    warnings
                        .push(format!("the source's feature-writer setting ({FEATURE_WRITERS_KEY}) is not a list, so `...` stands for no writer"));
                    Vec::new()
                }
            }
        };
        let mut items: Vec<plist::Value> = match &self.writers {
            None => source_items(),
            Some(list) => {
                let mut items = Vec::new();
                for item in list {
                    match item {
                        WriterItem::Source => items.extend(source_items()),
                        WriterItem::Writer { class, mode } => items.push(writer_entry(class, *mode)),
                    }
                }
                items
            }
        };
        if !self.generate_gdef {
            items.retain(|v| v.as_dict().and_then(|d| d.get("class")).and_then(plist::Value::as_str) != Some(GDEF_WRITER));
        }
        Some(plist::Value::Array(Array { items, ..Array::default() }))
    }

    /// Writes the debug file, when one is asked for, with `code` (the assembled code of the first
    /// font built; empty when it had none).
    pub(super) fn write_debug(&self, code: Option<&str>) -> Result<()> {
        let Some(path) = &self.debug_file else { return Ok(()) };
        let req = super::file_io::OutputRequest { output: Some(path.clone()), output_dir: None, in_place: false, no_clobber: false };
        super::file_io::write_output(path, code.unwrap_or_default().as_bytes(), &req)
    }
}

/// The path at `key` of the group.
fn path(p: &Params, group: &Map<String, Value>, key: &str) -> Result<Option<PathBuf>> {
    match group.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.is_empty() => Ok(Some(PathBuf::from(s))),
        Some(_) => Err(p.bad(format!("`features.{key}` must be a path"))),
    }
}

/// The items of `features.writers`: `none` (alone, or anywhere: no writer runs), `...` (once),
/// or a writer: `kern`, `mark`, `curs`, `gdef` or the reference's class names, optionally with
/// a module (`module::Class`) and a mode (`kern(mode=append)`).
fn writer_items(p: &Params, specs: &[String]) -> Result<Vec<WriterItem>> {
    let mut items = Vec::new();
    for spec in specs {
        let spec = spec.trim();
        match spec {
            "none" | "None" => return Ok(Vec::new()),
            "..." => {
                if items.contains(&WriterItem::Source) {
                    return Err(p.bad("`features.writers` holds `...` once"));
                }
                items.push(WriterItem::Source);
            }
            _ => items.push(writer_item(spec).map_err(|e| p.bad(format!("`features.writers`: {e}")))?),
        }
    }
    Ok(items)
}

/// One writer item ([`writer_items`]).
fn writer_item(spec: &str) -> std::result::Result<WriterItem, String> {
    let (name, args) = match spec.split_once('(') {
        Some((name, rest)) => {
            let args = rest.strip_suffix(')').ok_or_else(|| format!("`{spec}` has no closing parenthesis"))?;
            (name.trim(), Some(args.trim()))
        }
        None => (spec, None),
    };
    let class_name = name.rsplit("::").next().unwrap_or(name);
    let class = match class_name {
        "kern" | "KernFeatureWriter" => "KernFeatureWriter",
        "mark" | "MarkFeatureWriter" => "MarkFeatureWriter",
        "curs" | "CursFeatureWriter" => "CursFeatureWriter",
        "gdef" | "GdefFeatureWriter" => GDEF_WRITER,
        other => return Err(format!("no feature writer `{other}`: the writers are kern, mark, curs and gdef")),
    };
    let mode = match args.filter(|a| !a.is_empty()) {
        None => None,
        Some(args) => {
            let (key, value) = args.split_once('=').ok_or_else(|| format!("`{args}` is not `mode=skip` or `mode=append`"))?;
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            match (key.trim(), value) {
                ("mode", "skip") => Some("skip"),
                ("mode", "append") => Some("append"),
                _ => return Err(format!("`{args}`: a writer takes mode=skip or mode=append")),
            }
        }
    };
    Ok(WriterItem::Writer { class, mode })
}

/// One entry of the feature-writer setting.
fn writer_entry(class: &str, mode: Option<&str>) -> plist::Value {
    let mut entry = Dict::new();
    entry.insert("class", plist::Value::String(Str::new(class)));
    if let Some(mode) = mode {
        let mut options = Dict::new();
        options.insert("mode", plist::Value::String(Str::new(mode)));
        entry.insert("options", plist::Value::Dict(options));
    }
    plist::Value::Dict(entry)
}

/// The feature code of the FontDame sources a property list names: a dictionary from master
/// name to a dictionary from table tag (`GDEF`, `GSUB`, `GPOS`) to a file, relative to the list.
/// The entry of the default master is used, or the only entry; the files are converted together
/// against the exported glyphs in font order. Answers the code and what it could not carry.
fn mti_code(p: &Params, plist_path: &Path, font: &Font) -> Result<(String, Vec<String>)> {
    let key = "`features.mtiSource`";
    let text = super::compile_project::text(p, key, plist_path)?;
    let doc = plist::xmlplist::parse(&text).map_err(|e| p.bad(format!("{key}: {e}")))?;
    let masters = doc.root.as_dict().ok_or_else(|| p.bad(format!("{key}: the property list is not a dictionary of masters")))?;
    let default = font.default_master().map(|m| m.name.as_str()).unwrap_or_default();
    let entry = match masters.entries.as_slice() {
        [(_, only)] => only,
        entries => entries
            .iter()
            .find(|(name, _)| name.as_str() == default)
            .map(|(_, v)| v)
            .ok_or_else(|| p.bad(format!("{key}: no entry for the default master `{default}`")))?,
    };
    let tables = entry.as_dict().ok_or_else(|| p.bad(format!("{key}: a master's entry is not a dictionary of tables")))?;
    if tables.entries.len() > fontdame::MAX_FILES {
        return Err(p.bad(format!("{key}: a master names more than {} files", fontdame::MAX_FILES)));
    }
    let folder = plist_path.parent().unwrap_or(Path::new(""));
    let mut texts = Vec::new();
    let mut tags = Vec::new();
    for (tag, file) in &tables.entries {
        let tag = tag.as_str().trim();
        let parsed = match tag {
            "GDEF" => *b"GDEF",
            "GSUB" => *b"GSUB",
            "GPOS" => *b"GPOS",
            other => return Err(p.bad(format!("{key}: FontDame sources describe GDEF, GSUB and GPOS, not `{other}`"))),
        };
        let file = file.as_str().ok_or_else(|| p.bad(format!("{key}: the file of {tag} is not a path")))?;
        texts.push(super::compile_project::text(p, key, &folder.join(file))?);
        tags.push(parsed);
    }
    let glyph_order: Vec<String> = font.glyphs.iter().filter(|g| g.export).map(|g| g.name.clone()).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let table = if refs.len() == 1 { tags.first().copied() } else { None };
    Ok(fontdame::convert(&refs, &FontDameOptions { table, tables: tags, glyph_order })?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn writer_items_read_names_classes_modes_and_none() {
        assert_eq!(writer_item("kern"), Ok(WriterItem::Writer { class: "KernFeatureWriter", mode: None }));
        assert_eq!(
            writer_item("ufo2ft.featureWriters::MarkFeatureWriter(mode=\"append\")"),
            Ok(WriterItem::Writer { class: "MarkFeatureWriter", mode: Some("append") })
        );
        assert!(writer_item("kern(quantization=2)").is_err());
        assert!(writer_item("mkmk").is_err());
    }

    fn classes(setting: Option<plist::Value>) -> Vec<String> {
        let Some(plist::Value::Array(a)) = setting.as_ref() else { panic!("no list: {setting:?}") };
        a.items.iter().map(|v| v.as_dict().and_then(|d| d.get("class")).and_then(plist::Value::as_str).unwrap().to_string()).collect()
    }

    #[test]
    fn the_writer_setting_splices_the_source_list_and_drops_gdef_on_request() {
        let font = Font::new("Test");
        let mut warnings = Vec::new();
        let keep = FeatureRequest { generate_gdef: true, ..FeatureRequest::default() };
        assert_eq!(keep.setting(&font, &mut warnings), None, "nothing asked: the source's setting stays");
        // Without a setting in the source, `...` stands for the four default writers.
        let spliced = FeatureRequest {
            writers: Some(vec![WriterItem::Writer { class: "CursFeatureWriter", mode: Some("append") }, WriterItem::Source]),
            generate_gdef: false,
            ..FeatureRequest::default()
        };
        assert_eq!(
            classes(spliced.setting(&font, &mut warnings)),
            ["CursFeatureWriter", "KernFeatureWriter", "MarkFeatureWriter", "CursFeatureWriter"]
        );
        let none = FeatureRequest { writers: Some(Vec::new()), generate_gdef: true, ..FeatureRequest::default() };
        assert!(classes(none.setting(&font, &mut warnings)).is_empty());
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
