//! `font.proof` (M12.35, M12.36, M12.78, M12.79; `fontcraft_proof`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. Every proof is made in memory before the first file
//! is written, so a failure leaves nothing behind. The command changes no document, so it is not
//! journalled: replaying a session does not write proofs again.
//!
//! Built: every layout as PDF (one file) and SVG (one file per page), with `metrics`, `baseline`,
//! `hintGroups`, `black`, `show`, `hide`, `header`, `settings` and `panels`; `features` and
//! `sort: "lookup"` for the features layout; and the HTML specimen (any layout; it embeds the
//! font as WOFF2). `compare` makes one proof of all the fonts, named after the first. The other
//! options refuse with a message until their packages land. `listSettings` returns the drawing
//! settings instead of making a proof.
//!
//! With `document` the proof shows the active document (M13.67): the master or instance compiled
//! in memory with TrueType outlines, without the layout tables for the layouts that read none.
//! The last build is kept, so a second proof of an unchanged document does not compile again.
//! Without `output` or `outputDir` the PDF or HTML file comes back as `dataBase64`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_compile::{AssetsRef, DataRef, Flavour, Options};
use fontcraft_model::{Font, Target};
use fontcraft_proof::{GlyphSort, Metrics, Panel, ProofFormat, ProofOptions, Selection, SettingValue, Sheet};
use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most fonts one call proofs.
const MAX_FONTS: usize = 256;
/// Default glyphs per page of the palette layout, as the reference's overview proof.
const PALETTE_GLYPHS_PER_PAGE: usize = 156;
/// Default glyphs per page of the palette with outline pages, as the reference's linked palette.
const LINKED_GLYPHS_PER_PAGE: usize = 320;
/// Most hint-replacement groups `hintGroups` names (the charstring limit of stems).
const MAX_HINT_GROUPS: usize = 96;
/// Most drawing settings one call sets, in all and per panel.
const MAX_SETTINGS: usize = 256;
/// Default waterfall sizes, as the reference's waterfall proof.
const WATERFALL_SIZES: [f64; 12] = [36.0, 24.0, 20.0, 18.0, 16.0, 14.0, 12.0, 10.0, 9.0, 8.0, 7.0, 6.0];
/// Parameters of the row that no built package handles yet.
const NOT_BUILT_PARAMS: &[&str] = &["vertical", "pointLabels", "cidLayout", "errorLog", "fitBbox", "scale", "units", "paltInKern", "margins"];
/// The layouts drawn in tiles, which read `metrics` and `baseline`.
const TILE_LAYOUTS: &[&str] = &["boxes", "hints", "digi"];
/// The layouts whose drawing elements `show` and `hide` switch.
const ELEMENT_LAYOUTS: &[&str] = &["boxes", "hints", "digi", "compare", "synopsis", "features", "cmap", "vorg", "text"];
/// Most elements `show` or `hide` names, and most feature tags of `features`.
const MAX_NAMES: usize = 256;
/// The layouts that read the layout tables (kerning or feature rules).
const SHAPED_LAYOUTS: &[&str] = &["waterfall", "text", "features"];
const LAYOUTS: &[&str] =
    &["palette", "outline", "palette+outline", "digi", "boxes", "hints", "waterfall", "compare", "synopsis", "features", "cmap", "vorg", "text"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.proof",
        "Make Proof Sheet",
        [],
        None,
        "{fonts: [path] (or document: true), document?: bool, master?: id|name, instance?: name, face?: int|name, layout: \"palette\"|\"outline\"|\"palette+outline\"|\"digi\"|\"boxes\"|\"hints\"|\"waterfall\"|\"compare\"|\"synopsis\"|\"features\"|\"cmap\"|\"vorg\"|\"text\", format?: \"pdf\"|\"svg\"|\"html\", output?: path, outputDir?: path, noClobber?: bool, listSettings?: bool, glyphs?: selector, excludeGlyphs?: selector, glyphsPerPage?: int, size?: number, sizes?: [number], sort?: \"gid\"|\"name\"|\"unicode\"|\"lookup\", text?: string, features?: [tag]|\"*\", location?: {axis: number}, metrics?: \"h\"|\"v\"|\"both\", vertical?: bool, baseline?: number, show?: [string], hide?: [string], pointLabels?: [\"index\"|\"type\"|\"position\"], hintGroups?: [int], panels?: [{index: int, show?: [string], hide?: [string], settings?: object}], cidLayout?: path, errorLog?: path, fitBbox?: [l, b, r, t], scale?: {x: number, y?: number}, units?: \"font\"|1000, paltInKern?: bool, black?: bool, header?: bool, pageSize?: [w, h], margins?: [t, r, b, l], settings?: {name: value}} proof sheets of compiled fonts, written next to each input as NAME.proof.svg (NAME.proof-001.svg … for several pages), .html or .pdf; format defaults to pdf; a selector is a list of glyph names or ranges (a-b, a..b, by glyph index) or one comma-separated string; the HTML specimen embeds the font as WOFF2 and shows sample text, a waterfall, the character set, feature switches and axis stops; every layout is built as PDF and SVG, and HTML; compare makes one proof of every font, named after the first; metrics and baseline apply to boxes, hints and digi, hintGroups to hints, panels to digi, features (tags, or \"*\" for all; default all) and sort lookup to features; show and hide take drawing elements (em-box, bbox, advance, baseline, zones, hhints, vhints, points, handles, point-labels, contour-labels, center-marks, name, numeric-labels, sidebearings, hint-counts, path-counts, filled-thumbnail) for every layout but palette, outline, palette+outline and waterfall; header false leaves out the title line of each page; settings take the names listSettings returns, with a number, true/false or [r, g, b] (parts from 0 to 1); document true proofs the active document instead of files: its master, or its static instance (by name, #n for an unnamed one), compiled in memory (default the default master), glyphs named as in the document, and without output or outputDir the PDF or HTML file is returned → {proofs: [{font, format, layout, pages, files: [path], dataBase64?}]}, or with listSettings {settings: [{name, default, about}]}",
        always,
        proof
    ).files(const { &[FileParam::read("fonts"), FileParam::read("cidLayout"), FileParam::write("errorLog"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files()]
}

/// One proof made in memory, waiting to be written.
struct Made {
    font: String,
    format: &'static str,
    layout: String,
    pages: usize,
    files: Vec<(PathBuf, Vec<u8>)>,
}

fn proof(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.proof", p)?;
    if p.bool_or("listSettings", false)? {
        let settings: Vec<Value> =
            fontcraft_proof::SETTINGS.iter().map(|s| json!({ "name": s.name, "default": s.default, "about": s.about })).collect();
        return Ok(json!({ "settings": settings }));
    }
    let document = p.bool_or("document", false)?;
    if document {
        for key in ["fonts", "face"] {
            if p.get(key).is_some() {
                return Err(p.bad(format!("`document` proofs the open document: leave `{key}` out")));
            }
        }
    } else if let Some(key) = ["master", "instance"].iter().find(|key| p.get(key).is_some()) {
        return Err(p.bad(format!("`{key}` applies to `document`")));
    }
    let fonts = if document { Vec::new() } else { p.strings("fonts")? };
    if !document && (fonts.is_empty() || fonts.len() > MAX_FONTS) {
        return Err(p.bad(format!("`fonts` must name from 1 to {MAX_FONTS} files")));
    }
    for key in NOT_BUILT_PARAMS {
        if p.get(key).is_some() {
            return Err(EngineError::Unsupported(format!("`{key}` of font.proof")));
        }
    }
    let face = match p.get("face") {
        None => 0,
        Some(Value::String(_)) => return Err(EngineError::Unsupported("choosing a face of a collection by name".into())),
        Some(_) => p.opt_int("face", 0, i64::from(u16::MAX))?.unwrap_or(0) as u32,
    };
    let layout = p.str("layout")?;
    if !LAYOUTS.contains(&layout) {
        return Err(p.bad(format!("`layout` must be one of {}, got \"{layout}\"", LAYOUTS.join(", "))));
    }
    let (format, format_name, extension) = match p.opt_str("format")?.unwrap_or("pdf") {
        "pdf" => (ProofFormat::Pdf, "pdf", "pdf"),
        "svg" => (ProofFormat::Svg, "svg", "svg"),
        "html" => (ProofFormat::Html, "html", "html"),
        other => return Err(p.bad(format!("`format` must be \"pdf\", \"svg\" or \"html\", got \"{other}\""))),
    };
    let req = OutputRequest::from_params(&p)?;
    if req.in_place {
        return Err(p.bad("a proof never replaces its font: give `output` or `outputDir`"));
    }
    let set = layout == "compare" && format != ProofFormat::Html;
    if req.output.is_some() && fonts.len() > 1 && !set {
        return Err(p.bad("`output` names one file: give `outputDir` for several fonts"));
    }
    let page_size = match p.opt_array("pageSize")? {
        None => ProofOptions::default().page_size,
        Some(a) => match a.as_slice() {
            [w, h] => [finite(&p, w, "pageSize")?, finite(&p, h, "pageSize")?],
            _ => return Err(p.bad("`pageSize` must be [width, height] in points")),
        },
    };
    let sheet = match (layout, format) {
        // The HTML specimen is one page whatever the layout; its sample comes from `text`.
        (_, ProofFormat::Html) => Sheet::Waterfall { text: p.opt_str("text")?.unwrap_or("").to_string() },
        ("palette", _) => Sheet::GlyphGrid,
        ("outline", _) => Sheet::GlyphPerPage,
        ("waterfall", _) => Sheet::Waterfall { text: p.opt_str("text")?.unwrap_or("").to_string() },
        ("palette+outline", _) => Sheet::GridWithOutlines,
        ("boxes", _) => Sheet::Boxes,
        ("hints", _) => Sheet::Hints,
        ("digi", _) => Sheet::Digi,
        ("compare", _) => Sheet::Compare,
        ("synopsis", _) => Sheet::Synopsis,
        ("features", _) => Sheet::Features { tags: feature_tags(&p)?, font_order: p.opt_str("sort")? == Some("lookup") },
        ("cmap", _) => Sheet::Cmap,
        ("vorg", _) => Sheet::Vorg,
        ("text", _) => Sheet::Text { text: p.opt_str("text")?.unwrap_or("").to_string() },
        (other, _) => return Err(EngineError::Unsupported(format!("the `{other}` proof layout"))),
    };
    if p.get("features").is_some() && !matches!(sheet, Sheet::Features { .. }) {
        return Err(p.bad("`features` applies to the features layout"));
    }
    let elements = ELEMENT_LAYOUTS.contains(&layout) && format != ProofFormat::Html;
    for key in ["show", "hide"] {
        if p.get(key).is_some() && !elements {
            return Err(p.bad(format!("`{key}` applies to every layout but palette, outline, palette+outline and waterfall, and not to HTML")));
        }
    }
    let show = names(&p, p.get("show"), "show")?;
    let hide = names(&p, p.get("hide"), "hide")?;
    if p.get("header").is_some() && format == ProofFormat::Html {
        return Err(p.bad("`header` applies to PDF and SVG proofs"));
    }
    let header = p.bool_or("header", true)?;
    let tiles = TILE_LAYOUTS.contains(&layout) && format != ProofFormat::Html;
    for key in ["metrics", "baseline"] {
        if p.get(key).is_some() && !tiles {
            return Err(p.bad(format!("`{key}` applies to the boxes, hints and digi layouts")));
        }
    }
    if p.get("hintGroups").is_some() && layout != "hints" {
        return Err(p.bad("`hintGroups` applies to the hints layout"));
    }
    if p.get("panels").is_some() && layout != "digi" {
        return Err(p.bad("`panels` applies to the digi layout"));
    }
    let metrics = match p.opt_str("metrics")? {
        None => None,
        Some("h") => Some(Metrics::Horizontal),
        Some("v") => Some(Metrics::Vertical),
        Some("both") => Some(Metrics::Both),
        Some(other) => return Err(p.bad(format!("`metrics` must be \"h\", \"v\" or \"both\", got \"{other}\""))),
    };
    let baseline = p.opt_f64("baseline")?;
    let hint_groups = match p.opt_array("hintGroups")? {
        None => Vec::new(),
        Some(list) if list.len() > MAX_HINT_GROUPS => return Err(p.bad(format!("`hintGroups` names at most {MAX_HINT_GROUPS} groups"))),
        Some(list) => list
            .iter()
            .map(|v| v.as_u64().filter(|n| *n < MAX_HINT_GROUPS as u64).map(|n| n as usize))
            .collect::<Option<Vec<usize>>>()
            .ok_or_else(|| p.bad(format!("`hintGroups` holds group numbers from 0 to {}", MAX_HINT_GROUPS - 1)))?,
    };
    let settings = match p.get("settings") {
        None => Vec::new(),
        Some(v) => settings(&p, v, "settings")?,
    };
    let panels = panels(&p)?;
    let black = p.bool_or("black", false)?;
    let sizes = sizes(&p, &sheet, format, page_size)?;
    let per_page = tiles || (layout == "synopsis" && format != ProofFormat::Html);
    let glyphs_per_page = if per_page { p.opt_int("glyphsPerPage", 1, fontcraft_proof::MAX_SHEET_GLYPHS as i64)?.map(|n| n as usize) } else { None };
    let location = location(&p)?;
    let sort = match p.opt_str("sort")?.unwrap_or("gid") {
        "gid" => GlyphSort::Gid,
        "name" => GlyphSort::Name,
        "unicode" => GlyphSort::Unicode,
        // The features layout keeps the rules in the font's order; the glyphs are read in glyph order.
        "lookup" if layout == "features" && format != ProofFormat::Html => GlyphSort::Gid,
        "lookup" => return Err(p.bad("`sort: \"lookup\"` applies to the features layout")),
        other => return Err(p.bad(format!("`sort` must be \"gid\", \"name\", \"unicode\" or \"lookup\", got \"{other}\""))),
    };
    let selection = Selection { glyphs: selector(&p, "glyphs")?, exclude: selector(&p, "excludeGlyphs")?, sort };
    let select = !selection.glyphs.is_empty() || !selection.exclude.is_empty() || sort != GlyphSort::Gid;

    let options = |glyphs: Vec<String>| ProofOptions {
        format,
        sheets: vec![sheet.clone()],
        glyphs,
        page_size,
        sizes: sizes.clone(),
        location: location.clone(),
        glyphs_per_page,
        baseline,
        black,
        metrics,
        hint_groups: hint_groups.clone(),
        show: show.clone(),
        hide: hide.clone(),
        settings: settings.clone(),
        panels: panels.clone(),
        header,
    };
    if document {
        if format == ProofFormat::Svg && req.output.is_none() && req.output_dir.is_none() {
            return Err(p.bad("an SVG proof is one file per page: give `output` or `outputDir`"));
        }
        let built = build_document(s, &p, SHAPED_LAYOUTS.contains(&layout) || format == ProofFormat::Html)?;
        let selection = built.selection(selection);
        let glyphs = if select { fontcraft_proof::select_glyphs(&built.bytes, 0, &selection)? } else { Vec::new() };
        let bytes = if format == ProofFormat::Html { woff2(&built.bytes, 0, &built.label)? } else { built.bytes.to_vec() };
        let proof = fontcraft_proof::proof(&bytes, 0, &options(glyphs))?;
        let mut out = json!({ "font": built.label, "format": format_name, "layout": layout, "pages": proof.pages, "files": [] });
        if req.output.is_none() && req.output_dir.is_none() {
            let data = proof.files.first().ok_or_else(|| EngineError::Io("the proof has no page".into()))?;
            if let Value::Object(m) = &mut out {
                m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(data)));
            }
            return Ok(json!({ "proofs": [out] }));
        }
        let base = file_io::output_path(Path::new(&format!("{}.ttf", file_stem(&built.label))), &req, ".proof", Some(extension))?;
        let files: Vec<(PathBuf, Vec<u8>)> = page_paths(&base, proof.files.len()).into_iter().zip(proof.files).collect();
        let made = vec![Made { font: built.label.clone(), format: format_name, layout: layout.to_string(), pages: proof.pages, files }];
        return write_made(&p, made, &req);
    }
    let mut made: Vec<Made> = Vec::with_capacity(fonts.len());
    let mut inputs = Vec::with_capacity(fonts.len());
    for path in &fonts {
        let input = file_io::read_input(Path::new(path))?;
        if !input.kind.is_font() {
            return Err(EngineError::Unsupported(format!("proofs of {path}: give a compiled font (proofs of sources and Type 1 fonts come later)")));
        }
        inputs.push(input);
    }
    if set {
        // One proof of the whole set, named after the first font; the selection is read on it.
        let (Some(first), Some(first_path)) = (inputs.first(), fonts.first()) else { return Err(p.bad("`fonts` is empty")) };
        let glyphs = if select { fontcraft_proof::select_glyphs(&first.bytes, face, &selection)? } else { Vec::new() };
        let all: Vec<&[u8]> = inputs.iter().map(|i| i.bytes.as_slice()).collect();
        let proof = fontcraft_proof::proof_set(&all, face, &options(glyphs))?;
        let base = file_io::output_path(&first.path, &req, ".proof", Some(extension))?;
        let files = page_paths(&base, proof.files.len()).into_iter().zip(proof.files).collect();
        made.push(Made { font: first_path.to_string(), format: format_name, layout: layout.to_string(), pages: proof.pages, files });
    }
    for (path, input) in fonts.iter().zip(&inputs).filter(|_| !set) {
        let (bytes, font_face) = if format == ProofFormat::Html { (as_woff2(input, face)?, 0) } else { (input.bytes.clone(), face) };
        let glyphs = if select { fontcraft_proof::select_glyphs(&bytes, font_face, &selection)? } else { Vec::new() };
        let proof = fontcraft_proof::proof(&bytes, font_face, &options(glyphs))?;
        let base = file_io::output_path(&input.path, &req, ".proof", Some(extension))?;
        let files = page_paths(&base, proof.files.len()).into_iter().zip(proof.files).collect();
        made.push(Made { font: path.to_string(), format: format_name, layout: layout.to_string(), pages: proof.pages, files });
    }
    write_made(&p, made, &req)
}

/// Writes every proof, once none of them would replace another or a file `noClobber` keeps.
fn write_made(p: &Params<'_>, made: Vec<Made>, req: &OutputRequest) -> Result<Value> {
    let mut seen = std::collections::HashSet::new();
    for (path, _) in made.iter().flat_map(|m| &m.files) {
        if !seen.insert(path.clone()) {
            return Err(p.bad(format!("two proofs would both be written to {}: give `outputDir` or distinct file names", path.display())));
        }
        if req.no_clobber && path.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", path.display())));
        }
    }
    let mut proofs = Vec::with_capacity(made.len());
    for m in made {
        let mut written = Vec::with_capacity(m.files.len());
        for (path, bytes) in &m.files {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
                fontcraft_format::output::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))?;
            }
            file_io::write_output(path, bytes, req)?;
            written.push(path.display().to_string());
        }
        proofs.push(json!({ "font": m.font, "format": m.format, "layout": m.layout, "pages": m.pages, "files": written }));
    }
    Ok(json!({ "proofs": proofs }))
}

/// The active document compiled for a proof.
#[derive(Clone)]
struct Built {
    /// The family and the master or instance.
    label: String,
    bytes: Arc<Vec<u8>>,
    /// The compiled name of each glyph, by its name in the document.
    names: Arc<HashMap<String, String>>,
}

impl Built {
    /// `selection` with the document's glyph names turned into the compiled font's.
    fn selection(&self, selection: Selection) -> Selection {
        let named = |items: Vec<String>| items.into_iter().map(|n| self.names.get(&n).cloned().unwrap_or(n)).collect();
        Selection { glyphs: named(selection.glyphs), exclude: named(selection.exclude), sort: selection.sort }
    }
}

/// What a kept build was made of: the document, its revision, the target and whether it has the
/// layout tables.
type BuildKey = (u64, u64, Target, bool);

/// The last build of a document's proof.
#[derive(Default)]
struct KeptBuild(Option<(BuildKey, Built)>);

/// The master or instance the parameters name, with its label; the default master by default.
fn document_target(font: &Font, p: &Params<'_>) -> Result<(String, Target)> {
    match (p.opt_str("master")?, p.opt_str("instance")?) {
        (Some(_), Some(_)) => Err(p.bad("give `master` or `instance`, not both")),
        (Some(m), None) => {
            let found = font.masters.iter().find(|x| x.id == m).or_else(|| font.masters.iter().find(|x| x.name == m));
            let master = found.ok_or_else(|| p.bad(format!("no master has the id or name `{m}`")))?;
            Ok((master.name.clone(), Target::Master(master.id.clone())))
        }
        (None, Some(name)) => {
            let label = |i: usize, n: &str| if n.is_empty() { format!("#{i}") } else { n.to_string() };
            let found = font.instances.iter().enumerate().find(|(i, inst)| inst.kind.is_none() && label(*i, &inst.name) == name);
            let (i, _) = found.ok_or_else(|| p.bad(format!("the font has no static instance named `{name}`")))?;
            Ok((name.to_string(), Target::Instance(i)))
        }
        (None, None) => {
            let master = font.default_master()?;
            Ok((master.name.clone(), Target::Master(master.id.clone())))
        }
    }
}

/// The active document's master or instance compiled in memory, `shaped` with its layout tables.
/// A document that did not change since the last build is not compiled again.
fn build_document(s: &mut Session, p: &Params<'_>, shaped: bool) -> Result<Built> {
    let doc = s.doc()?;
    let (id, revision, path, font) = (doc.id, doc.revision, doc.path.clone(), doc.font.clone());
    let (name, target) = document_target(&font, p)?;
    let key = (id, revision, target, shaped);
    if let Some((kept, built)) = &s.slot_mut::<KeptBuild>().0
        && *kept == key
    {
        return Ok(built.clone());
    }
    let mut options =
        Options { glyph_data: DataRef(Some(s.glyph_data()?)), assets: AssetsRef(Some(s.assets())), skip_features: !shaped, ..Options::default() };
    super::compile::source_includes(&mut options, path.as_deref());
    let compiled = fontcraft_compile::compile(&font, &key.2, Flavour::TrueType, &options)?;
    let named = fontcraft_proof::select_glyphs(&compiled.bytes, 0, &Selection::default())?;
    let names = compiled.glyph_order.into_iter().zip(named).collect();
    let label = format!("{} {name}", font.family_name).trim().to_string();
    let built = Built { label, bytes: Arc::new(compiled.bytes), names: Arc::new(names) };
    s.slot_mut::<KeptBuild>().0 = Some((key, built.clone()));
    Ok(built)
}

/// A file name made of `label`, without the characters file systems refuse.
fn file_stem(label: &str) -> String {
    let clean: String = label
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '.') { '-' } else { c })
        .take(64)
        .collect();
    if clean.trim().is_empty() { "Proof".to_string() } else { clean }
}

fn finite(p: &Params<'_>, v: &Value, key: &str) -> Result<f64> {
    v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| p.bad(format!("`{key}` must hold finite numbers")))
}

/// The point sizes: `sizes` for a waterfall (the reference's twelve by default), else one size
/// from `size`, `glyphsPerPage` or the palette's default count.
fn sizes(p: &Params<'_>, sheet: &Sheet, format: ProofFormat, page_size: [f64; 2]) -> Result<Vec<f64>> {
    if let Some(list) = p.opt_array("sizes")? {
        if list.len() > 64 {
            return Err(p.bad("`sizes` holds at most 64 sizes"));
        }
        return list.iter().map(|v| finite(p, v, "sizes")).collect();
    }
    if let Some(size) = p.opt_f64("size")? {
        return Ok(vec![size]);
    }
    if matches!(sheet, Sheet::Waterfall { .. }) || format == ProofFormat::Html {
        return Ok(WATERFALL_SIZES.to_vec());
    }
    let grid_default = match sheet {
        Sheet::GlyphGrid => PALETTE_GLYPHS_PER_PAGE,
        Sheet::GridWithOutlines => LINKED_GLYPHS_PER_PAGE,
        // The tile layouts take `glyphsPerPage` themselves; compare has its own default size.
        _ => return Ok(Vec::new()),
    };
    let n = p.opt_int("glyphsPerPage", 1, fontcraft_proof::MAX_SHEET_GLYPHS as i64)?.map(|n| n as usize).unwrap_or(grid_default);
    Ok(vec![fontcraft_proof::grid_size_for(page_size, n)?])
}

/// Drawing settings from an object of names and values: a number, true or false, `[r, g, b]`, or
/// text such as `"12"` or `"(0, 0.75, 0)"`.
fn settings(p: &Params<'_>, v: &Value, key: &str) -> Result<Vec<(String, SettingValue)>> {
    let Some(map) = v.as_object() else { return Err(p.bad(format!("`{key}` must be an object of setting names and values"))) };
    if map.len() > MAX_SETTINGS {
        return Err(p.bad(format!("`{key}` holds at most {MAX_SETTINGS} settings")));
    }
    map.iter()
        .map(|(name, value)| {
            let parsed = match value {
                Value::Bool(b) => Some(SettingValue::Number(if *b { 1.0 } else { 0.0 })),
                Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).map(SettingValue::Number),
                Value::Array(parts) => match parts.iter().map(|x| x.as_f64().filter(|f| f.is_finite())).collect::<Option<Vec<f64>>>().as_deref() {
                    Some([r, g, b]) => Some(SettingValue::Rgb([*r, *g, *b])),
                    _ => None,
                },
                Value::String(text) => SettingValue::parse(text),
                _ => None,
            };
            let value = parsed.ok_or_else(|| p.bad(format!("setting `{name}` needs a number, true or false, or a colour [r, g, b]")))?;
            Ok((name.clone(), value))
        })
        .collect()
}

/// `panels`: per copy of the digi layout, the elements it shows and hides and its settings.
fn panels(p: &Params<'_>) -> Result<Vec<Panel>> {
    let Some(list) = p.opt_array("panels")? else { return Ok(Vec::new()) };
    if list.len() > 2 {
        return Err(p.bad("`panels` describes at most the two copies of the digi layout"));
    }
    list.iter()
        .map(|panel| {
            let Some(obj) = panel.as_object() else { return Err(p.bad("each of `panels` is an object {index, show, hide, settings}")) };
            let index =
                obj.get("index").and_then(Value::as_u64).filter(|i| *i < 2).ok_or_else(|| p.bad("a panel's `index` is 0 (left) or 1 (right)"))?;
            let settings = match obj.get("settings") {
                None => Vec::new(),
                Some(v) => settings(p, v, "panels.settings")?,
            };
            let show = names(p, obj.get("show"), "panels.show")?;
            let hide = names(p, obj.get("hide"), "panels.hide")?;
            Ok(Panel { index: index as usize, show, hide, settings })
        })
        .collect()
}

/// A list of names: an array of strings, or one string of names separated by commas or spaces.
fn names(p: &Params<'_>, v: Option<&Value>, key: &str) -> Result<Vec<String>> {
    let items: Vec<String> = match v {
        None => Vec::new(),
        Some(Value::String(s)) => s.split([',', ' ']).filter(|s| !s.is_empty()).map(str::to_string).collect(),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| v.as_str().map(str::trim).map(str::to_string))
            .collect::<Option<Vec<String>>>()
            .ok_or_else(|| p.bad(format!("`{key}` holds names")))?,
        Some(_) => return Err(p.bad(format!("`{key}` must be a list of names"))),
    };
    if items.len() > MAX_NAMES {
        return Err(p.bad(format!("`{key}` holds at most {MAX_NAMES} names")));
    }
    Ok(items)
}

/// `features`: tags of one to four printable characters, or `"*"` (and by default) for every
/// feature of the font.
fn feature_tags(p: &Params<'_>) -> Result<Vec<[u8; 4]>> {
    let list = names(p, p.get("features"), "features")?;
    if list.iter().any(|t| t == "*") {
        return Ok(Vec::new());
    }
    list.iter()
        .map(|t| {
            let bytes = t.as_bytes();
            if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..0x7f).contains(b)) {
                return Err(p.bad(format!("`{t}` is not a feature tag")));
            }
            let mut tag = *b"    ";
            for (slot, b) in tag.iter_mut().zip(bytes) {
                *slot = *b;
            }
            Ok(tag)
        })
        .collect()
}

fn location(p: &Params<'_>) -> Result<Vec<([u8; 4], f64)>> {
    let Some(map) = p.opt_object("location")? else { return Ok(Vec::new()) };
    if map.len() > fontcraft_model::MAX_AXIS_VALUES {
        return Err(p.bad(format!("`location` holds at most {} axes", fontcraft_model::MAX_AXIS_VALUES)));
    }
    map.iter()
        .map(|(axis, v)| {
            let bytes = axis.as_bytes();
            if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..0x7f).contains(b)) {
                return Err(p.bad(format!("`{axis}` is not an axis tag")));
            }
            let mut tag = *b"    ";
            for (slot, b) in tag.iter_mut().zip(bytes) {
                *slot = *b;
            }
            Ok((tag, finite(p, v, "location")?))
        })
        .collect()
}

/// A selector: a list of names and ranges, or one string of them separated by commas or spaces.
fn selector(p: &Params<'_>, key: &str) -> Result<Vec<String>> {
    let items: Vec<String> = match p.opt_strings(key)? {
        None => Vec::new(),
        Some(list) => list.iter().flat_map(|s| s.split([',', ' ', '\n', '\t'])).filter(|s| !s.is_empty()).map(str::to_string).collect(),
    };
    if items.len() > fontcraft_proof::MAX_SHEET_GLYPHS {
        return Err(p.bad(format!("`{key}` holds at most {} items", fontcraft_proof::MAX_SHEET_GLYPHS)));
    }
    Ok(items)
}

/// The font as WOFF2, for the HTML specimen: the input itself when it is WOFF2.
fn as_woff2(input: &file_io::Input, face: u32) -> Result<Vec<u8>> {
    if input.kind == InputKind::Woff2 {
        return Ok(input.bytes.clone());
    }
    woff2(&input.bytes, face, &input.path.display().to_string())
}

/// Face `face` of the font `bytes` as WOFF2; `name` is what a message calls the font.
fn woff2(bytes: &[u8], face: u32, name: &str) -> Result<Vec<u8>> {
    let font = fontcraft_tables::read(bytes, face)?;
    let sfnt = fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions::default())?;
    fontcraft_container::woff2::encode(&sfnt, &fontcraft_container::woff2::Options::default())
        .map_err(|e| EngineError::Io(format!("{name}: cannot wrap the font in WOFF2: {e}")))
}

/// The file of each page: `base` for one file, `stem-001.ext`, `stem-002.ext` … for several.
fn page_paths(base: &Path, count: usize) -> Vec<PathBuf> {
    if count <= 1 {
        return vec![base.to_path_buf()];
    }
    let width = count.to_string().len().max(3);
    let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = base.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    (1..=count).map(|i| base.with_file_name(format!("{stem}-{i:0width$}.{ext}"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document's proof is compiled in memory, names its glyphs as the document does, comes back without a file and
    /// is not compiled again while the document stays as it is (M13.67).
    #[test]
    fn a_document_proof_is_built_in_memory_and_returned() {
        let mut s = Session::with_document();
        let rect = json!([{"x": 60.0, "y": 0.0}, {"x": 540.0, "y": 0.0}, {"x": 540.0, "y": 700.0}, {"x": 60.0, "y": 700.0}]);
        s.execute("path.addPath", &json!({"glyph": "A", "nodes": rect})).unwrap();
        s.execute("glyph.newGlyph", &json!({"name": "a-cy"})).unwrap();
        let one = s.execute("font.proof", &json!({"document": true, "layout": "outline", "glyphs": ["A"]})).unwrap();
        assert_eq!(one["proofs"][0]["pages"], 1);
        let data = base64::engine::general_purpose::STANDARD.decode(one["proofs"][0]["dataBase64"].as_str().unwrap()).unwrap();
        assert!(data.starts_with(b"%PDF-"));
        // The document's own name picks the glyph, whatever the compiled font calls it.
        let cyrillic = s.execute("font.proof", &json!({"document": true, "layout": "outline", "glyphs": ["A", "a-cy"]})).unwrap();
        assert_eq!(cyrillic["proofs"][0]["pages"], 2);

        let first = s.slot_mut::<KeptBuild>().0.as_ref().map(|(k, b)| (k.clone(), b.bytes.clone())).unwrap();
        s.execute("font.proof", &json!({"document": true, "layout": "palette"})).unwrap();
        let again = s.slot_mut::<KeptBuild>().0.as_ref().map(|(_, b)| b.bytes.clone()).unwrap();
        assert!(Arc::ptr_eq(&first.1, &again), "an unchanged document is not compiled again");
        s.execute("path.addPath", &json!({"glyph": "A", "nodes": rect})).unwrap();
        s.execute("font.proof", &json!({"document": true, "layout": "palette"})).unwrap();
        assert_ne!(s.slot_mut::<KeptBuild>().0.as_ref().map(|(k, _)| k.1), Some(first.0.1), "an edit builds the document again");

        for (bad, why) in [
            (json!({"document": true, "fonts": ["a.ttf"], "layout": "palette"}), "fonts"),
            (json!({"document": true, "layout": "palette", "format": "svg"}), "per page"),
            (json!({"document": true, "layout": "palette", "master": "Nope"}), "no master"),
            (json!({"document": true, "layout": "palette", "instance": "Nope"}), "no static instance"),
            (json!({"fonts": ["a.ttf"], "layout": "palette", "master": "Regular"}), "applies to `document`"),
        ] {
            let e = s.execute("font.proof", &bad).unwrap_err().to_string();
            assert!(e.contains(why), "{bad}: {e}");
        }
        let e = Session::new().execute("font.proof", &json!({"document": true, "layout": "palette"})).unwrap_err();
        assert!(!e.to_string().is_empty(), "no document, no proof");
    }

    #[test]
    fn page_files_are_numbered_with_at_least_three_digits() {
        let base = Path::new("/out/A.proof.svg");
        assert_eq!(page_paths(base, 1), vec![PathBuf::from("/out/A.proof.svg")]);
        let many = page_paths(base, 2);
        assert_eq!(many, vec![PathBuf::from("/out/A.proof-001.svg"), PathBuf::from("/out/A.proof-002.svg")]);
        assert_eq!(page_paths(base, 1200).last(), Some(&PathBuf::from("/out/A.proof-1200.svg")));
    }
}
