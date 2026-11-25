//! File › Export › Metrics and File › Import › Metrics…: one master's spacing and kerning written
//! as kerning-text (`fontcraft_spacing::kerning_text`) or AFM (`fontcraft_spacing::afm`), and
//! read back from those files, from a font source or from another open document. Behaviour
//! notes: `docs/spec/07-import-export.md` § 1.4 and § 4.2.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_model::{Alignment, Font, Glyph, KernGroups, Layer, MetricsKeys, Shape, SourceFormat};
use fontcraft_spacing::afm::{self, Afm};
use fontcraft_spacing::kerning::pairs::{self, NewPair};
use fontcraft_spacing::kerning::{Direction, GroupSide, Position, Side, facing};
use fontcraft_spacing::kerning_text::{self, KerningText, MetricsLine};
use fontcraft_spacing::{self as spacing, Entry, Field, SIDEBEARING_RANGE, Scope, WIDTH_RANGE, metrics_source, round_half_up};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most glyphs, groups and keys one import lists by name; the counts are always complete.
const MAX_REPORTED: usize = 1000;
/// Longest file name `fileName` may give, in bytes.
const MAX_FILE_NAME: usize = 200;

/// A field, the key it gets (`None`: no key) and the scope of the key it shows now.
type KeyPlan = (Field, Option<(Scope, String)>, Option<Scope>);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.exportMetrics",
            "Export Metrics",
            [],
            None,
            "{master?: id (default: the first master; the dialog gives the master the toolbar shows), format: \"metrics\"|\"afm\", direction?: \"ltr\"|\"rtl\"|\"vertical\" (metrics only, default ltr), path?: file, destination?: folder, fileName?: name without extension (with destination; default Family-Master)} writes one master's spacing and kerning. metrics: kerning-text with the master's pairs of one direction, its kerning groups on the two sides facing that direction and a metrics line per glyph (LSB, RSB, width and their keys), which file.importMetrics reads back exactly. afm: every glyph's width and bounding box and every glyph pair the stored pairs cover with the value that applies to it, rounded half up; kerning groups and metrics keys cannot be written. A linked master gives the metrics and kerning of the master it follows. With path, or destination (the file is fileName plus .tsv or .afm), the file is written atomically; with neither, the text is returned → {master, format, direction?, glyphs, pairs, groups, bytes, path?, text?}",
            has_doc,
            export_metrics
        ).files(const { &[FileParam::write("path"), FileParam::write("destination")] }).writes_files(),
        cmd!(
            "file.importMetrics",
            "Metrics…",
            ["File", "Import"],
            None,
            "{file?: path | text?: the contents of a kerning-text or AFM file | sourceDocument?: index of another open document (exactly one of the three), master?: id (default: the first master), sourceMaster?: id or name of a master of a font source (default: its first master), importKerning?: bool (default true; refused for a .glyphs file or package, where it defaults to false), importKerningGroups?: bool (default true), importMetrics?: false|true|\"lsbAndWidth\"|\"rsbAndWidth\"|\"bothSidebearings\"|\"widthCentered\" (default false; true is lsbAndWidth; kerning-text and AFM only), keepAlignedComponents?: bool (default false: a composite aligned automatically whose imported metrics differ from what alignment derives, its offset keys from the file included, has alignment switched off for that layer's components so the values hold, and is listed in unaligned; true keeps it aligned with its derived metrics and lists it in skipped), glyphs?: [names] (metrics and groups of these glyphs only; empty or absent: every glyph)} imports spacing and kerning into one master as one undo step. A file is kerning-text, AFM, or a font source (.glyphs, .glyphspackage, UFO, designspace, .fontcraft, a compiled font). Kerning replaces the master's pairs in each direction the source holds (kerning-text: its direction; AFM: left to right; a font source: each direction its master has pairs in), with the values as given. Kerning groups: kerning-text sets each glyph's groups on the sides its group lines name (a glyph in no group line of such a side loses its group there); a font source copies the groups of the glyphs of the same name; AFM has none. Metrics: lsbAndWidth takes LSB and width (RSB follows), rsbAndWidth RSB and width (LSB follows), bothSidebearings both sidebearings (width follows), widthCentered the width with the outline centred (LSB rounded half up); kerning-text also sets the left, right and width keys as the file gives them, and an empty key field removes the key that field shows. Glyphs and groups the font lacks are listed → {master, source, kerning?: {directions, pairs, unknownGlyphs, unknownGroups}, groups?: {glyphs, unknownGlyphs}, metrics?: {mode, glyphs, keys, skipped: [{glyph, reason}], unaligned: [names], unknownGlyphs, badKeys: [{glyph, field, key, error}]}}",
            has_doc,
            import_metrics
        ).files(const { &[FileParam::read("file")] }),
    ]
}

fn clip(text: &str) -> String {
    text.chars().take(40).collect()
}

/// Pushes `name` to `list` once, while the list is short enough to report.
fn report(list: &mut Vec<String>, seen: &mut HashSet<String>, name: &str) {
    if list.len() < MAX_REPORTED && seen.insert(name.to_string()) {
        list.push(name.to_string());
    }
}

fn key_slot(keys: &mut MetricsKeys, field: Field) -> &mut Option<String> {
    match field {
        Field::Left => &mut keys.left,
        Field::Right => &mut keys.right,
        Field::Width => &mut keys.width,
        Field::Top => &mut keys.top,
        Field::Bottom => &mut keys.bottom,
        Field::VertWidth => &mut keys.vert_width,
        Field::VertOrigin => &mut keys.vert_origin,
    }
}

fn group_slot(groups: &mut KernGroups, side: GroupSide) -> &mut Option<String> {
    match side {
        GroupSide::Left => &mut groups.left,
        GroupSide::Right => &mut groups.right,
        GroupSide::Top => &mut groups.top,
        GroupSide::Bottom => &mut groups.bottom,
    }
}

// ---------- file.exportMetrics ----------

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Metrics,
    Afm,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::Metrics => "metrics",
            Format::Afm => "afm",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Metrics => "tsv",
            Format::Afm => "afm",
        }
    }
}

fn export_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.exportMetrics", v)?;
    let font = s.font()?;
    let master = super::shared::master(&p, font)?;
    let format = match p.str("format")? {
        "metrics" => Format::Metrics,
        "afm" => Format::Afm,
        other => return Err(p.bad(format!("`format` must be metrics or afm, got `{}`", clip(other)))),
    };
    let direction = super::shared::direction(&p)?;
    if format == Format::Afm && direction != Direction::Ltr {
        return Err(p.bad("AFM holds left-to-right kerning only: leave out `direction`, or export the metrics format"));
    }
    let (text, glyphs, pair_count, groups) = match format {
        Format::Metrics => {
            let doc = metrics_text(font, direction, &master);
            let text = kerning_text::write(&doc).map_err(|e| p.bad(e.to_string()))?;
            (text, doc.metrics.len(), doc.pairs.len(), doc.groups.len())
        }
        Format::Afm => {
            let doc = afm::from_master(font, &master).map_err(|e| p.bad(e.to_string()))?;
            let text = afm::write(&doc).map_err(|e| p.bad(e.to_string()))?;
            (text, doc.chars.len(), doc.pairs.len(), 0)
        }
    };
    let mut out = json!({
        "master": master,
        "format": format.name(),
        "glyphs": glyphs,
        "pairs": pair_count,
        "groups": groups,
        "bytes": text.len(),
    });
    let Some(o) = out.as_object_mut() else { return Ok(out) };
    if format == Format::Metrics {
        o.insert("direction".into(), json!(direction.name()));
    }
    match output_path(&p, font, &master, format)? {
        None => {
            o.insert("text".into(), json!(text));
        }
        Some(path) => {
            write_text(&path, &text)?;
            o.insert("path".into(), json!(path.to_string_lossy()));
        }
    }
    Ok(out)
}

/// The kerning-text of one master: its pairs of `direction` and the groups facing them (from
/// the master it follows, for a linked master), and a metrics line for every glyph with a layer
/// of that master.
fn metrics_text(font: &Font, direction: Direction, master: &str) -> KerningText {
    let source = metrics_source(font, master).map_or_else(|| master.to_string(), |m| m.id.clone());
    let mut doc = kerning_text::from_master(font, direction, &source);
    doc.master = font.masters.iter().find(|m| m.id == master).map(|m| m.name.clone());
    for g in font.glyphs.iter() {
        let Some(layer) = g.layer(&source) else { continue };
        let side = |field: Field| spacing::metric(font, layer, field).ok().flatten().filter(|v| v.is_finite() && SIDEBEARING_RANGE.contains(v));
        doc.metrics.push(MetricsLine {
            glyph: g.name.clone(),
            lsb: side(Field::Left),
            rsb: side(Field::Right),
            width: Some(layer.width).filter(|w| w.is_finite() && WIDTH_RANGE.contains(w)),
            left_key: shown_key(font, g, layer, Field::Left),
            right_key: shown_key(font, g, layer, Field::Right),
            width_key: shown_key(font, g, layer, Field::Width),
        });
    }
    doc
}

/// The key a metrics field of `layer` shows, as the field shows it (`=H`, `==n`).
fn shown_key(font: &Font, glyph: &Glyph, layer: &Layer, field: Field) -> Option<String> {
    let (scope, text) = spacing::effective_key(glyph, layer, field)?;
    Some(match spacing::parse_stored(text, scope, field, |n| font.glyph(n).is_some()) {
        Ok(k) => k.display(),
        Err(_) => match scope {
            Scope::Local => format!("=={}", text.trim_start_matches('=')),
            Scope::Glyph => text.to_string(),
        },
    })
}

/// Where the export goes: `path`, or `destination` joined with `fileName` and the format's
/// extension; `None` returns the text.
fn output_path(p: &Params, font: &Font, master: &str, format: Format) -> Result<Option<PathBuf>> {
    let name = p.opt_str("fileName")?;
    match (p.opt_str("path")?, p.opt_str("destination")?) {
        (Some(_), Some(_)) => Err(p.bad("give `path` or `destination`, not both")),
        (Some(path), None) => {
            if name.is_some() {
                return Err(p.bad("`fileName` goes with `destination`; `path` names the file itself"));
            }
            if path.trim().is_empty() {
                return Err(p.bad("`path` is empty"));
            }
            Ok(Some(PathBuf::from(path)))
        }
        (None, Some(dir)) => {
            let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => {
                    if n.len() > MAX_FILE_NAME || n.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')) || n == "." || n == ".." {
                        return Err(p.bad(format!("`fileName` must be a file name of at most {MAX_FILE_NAME} bytes without / or \\")));
                    }
                    n.to_string()
                }
                None => default_file_name(font, master),
            };
            Ok(Some(PathBuf::from(dir).join(format!("{name}.{}", format.extension()))))
        }
        (None, None) => {
            if name.is_some() {
                return Err(p.bad("`fileName` goes with `destination`"));
            }
            Ok(None)
        }
    }
}

/// `Family-Master`, with characters file systems refuse replaced by `-`.
fn default_file_name(font: &Font, master: &str) -> String {
    let style = font.masters.iter().find(|m| m.id == master).map(|m| m.name.trim()).unwrap_or_default();
    let joined = [font.family_name.trim(), style].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("-");
    let clean: String = joined
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .take(MAX_FILE_NAME / 4)
        .collect();
    if clean.trim().is_empty() || clean.starts_with('.') { "Metrics".to_string() } else { clean }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_text(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    fontcraft_format::write_atomic(path, text.as_bytes())?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_text(_: &std::path::Path, _: &str) -> Result<()> {
    Err(EngineError::Unsupported("writing a file by path in the browser; leave out `path` and `destination` to get the text".into()))
}

// ---------- file.importMetrics ----------

/// What an import reads from.
enum Source {
    Text(KerningText),
    Afm(Afm),
    /// A font source; `glyphs_file` for a `.glyphs` file or package, whose kerning values are
    /// not imported.
    Font {
        font: Arc<Font>,
        glyphs_file: bool,
    },
}

impl Source {
    fn name(&self) -> &'static str {
        match self {
            Source::Text(_) => "kerning-text",
            Source::Afm(_) => "afm",
            Source::Font { .. } => "font",
        }
    }
}

/// The four ways glyph metrics are applied.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    LsbAndWidth,
    RsbAndWidth,
    BothSidebearings,
    WidthCentered,
}

impl Mode {
    const ALL: [Mode; 4] = [Mode::LsbAndWidth, Mode::RsbAndWidth, Mode::BothSidebearings, Mode::WidthCentered];

    fn name(self) -> &'static str {
        match self {
            Mode::LsbAndWidth => "lsbAndWidth",
            Mode::RsbAndWidth => "rsbAndWidth",
            Mode::BothSidebearings => "bothSidebearings",
            Mode::WidthCentered => "widthCentered",
        }
    }
}

fn mode_param(p: &Params) -> Result<Option<Mode>> {
    match p.get("importMetrics") {
        None | Some(Value::Bool(false)) | Some(Value::Null) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(Mode::LsbAndWidth)),
        Some(Value::String(s)) => Mode::ALL.into_iter().find(|m| m.name() == s).map(Some).ok_or_else(|| {
            p.bad(format!("`importMetrics` must be false, lsbAndWidth, rsbAndWidth, bothSidebearings or widthCentered, got `{}`", clip(s)))
        }),
        Some(_) => Err(p.bad("`importMetrics` must be false or a mode name")),
    }
}

fn parse_text(p: &Params, key: &str, text: &str) -> Result<Source> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if afm::sniff(text) {
        afm::read(text).map(Source::Afm).map_err(|e| p.bad(format!("`{key}` (AFM): {e}")))
    } else {
        kerning_text::read(text).map(Source::Text).map_err(|e| p.bad(format!("`{key}` (kerning-text): {e}")))
    }
}

fn font_source(font: Arc<Font>) -> Source {
    let glyphs_file = matches!(font.source.format, SourceFormat::Glyphs | SourceFormat::GlyphsPackage);
    Source::Font { font, glyphs_file }
}

/// A file by its content: text that does not open like a `.glyphs` file, a native file or a
/// designspace (`{`, `<`) is AFM when it starts with `StartFontMetrics`, else kerning-text; a
/// folder or anything else is a font source.
#[cfg(not(target_arch = "wasm32"))]
fn read_file(p: &Params, path: &str) -> Result<Source> {
    let path = std::path::Path::new(path);
    if !path.is_dir() {
        let bytes = crate::io::read_file(path)?;
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let body = text.trim_start_matches('\u{feff}').trim_start();
            if !body.starts_with('{') && !body.starts_with('<') {
                return parse_text(p, "file", text);
            }
        }
    }
    let (font, _) = crate::io::open(path, None)?;
    Ok(font_source(Arc::new(font)))
}

#[cfg(target_arch = "wasm32")]
fn read_file(_: &Params, _: &str) -> Result<Source> {
    Err(EngineError::Unsupported("reading a file by path in the browser; pass the file's contents as `text`".into()))
}

fn load_source(s: &Session, p: &Params) -> Result<Source> {
    let file = p.opt_str("file")?;
    let text = p.opt_str("text")?;
    let doc = p.opt_int("sourceDocument", 0, i64::from(u32::MAX))?;
    match (file, text, doc) {
        (Some(f), None, None) => read_file(p, f),
        (None, Some(t), None) => {
            // The larger of the two formats' limits; each reader checks its own.
            if t.len() > afm::MAX_TEXT_BYTES {
                return Err(p.bad(format!("`text` is {} bytes long; at most {} are allowed", t.len(), afm::MAX_TEXT_BYTES)));
            }
            parse_text(p, "text", t)
        }
        (None, None, Some(i)) => {
            let active = s.doc()?.id;
            let doc = usize::try_from(i).ok().and_then(|i| s.documents().get(i)).ok_or_else(|| p.bad(format!("there is no open document {i}")))?;
            if doc.id == active {
                return Err(p.bad("`sourceDocument` is the document the metrics go into; pick another open document"));
            }
            Ok(font_source(doc.font.clone()))
        }
        (None, None, None) => Err(p.bad("give `file`, `text` or `sourceDocument`")),
        _ => Err(p.bad("give only one of `file`, `text` and `sourceDocument`")),
    }
}

/// The master of a font source to import from: `sourceMaster` by id, then by name, else the
/// first master.
fn source_master(p: &Params, font: &Font) -> Result<String> {
    match p.opt_str("sourceMaster")? {
        Some(want) => font
            .masters
            .iter()
            .find(|m| m.id == want)
            .or_else(|| font.masters.iter().find(|m| m.name == want))
            .map(|m| m.id.clone())
            .ok_or_else(|| p.bad(format!("the source has no master `{}`", clip(want)))),
        None => font.masters.first().map(|m| m.id.clone()).ok_or_else(|| p.bad("the source has no master")),
    }
}

fn import_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.importMetrics", v)?;
    let font = s.font()?;
    let master = super::shared::editable_master(&p, font)?;
    let scope: Option<HashSet<String>> = p.opt_glyphs("glyphs", font)?.filter(|g| !g.is_empty()).map(|g| g.into_iter().collect());
    let source = load_source(s, &p)?;
    let is_font = matches!(source, Source::Font { .. });
    let glyphs_file = matches!(source, Source::Font { glyphs_file: true, .. });
    let kerning = p.bool_or("importKerning", !glyphs_file)?;
    if kerning && glyphs_file {
        return Err(p.bad(
            "kerning values are not imported from a .glyphs file: import its kerning groups, or copy the pairs in the Kerning window and paste them",
        ));
    }
    let groups = p.bool_or("importKerningGroups", true)?;
    let mode = mode_param(&p)?;
    if mode.is_some() && is_font {
        return Err(p.bad("glyph metrics are imported from kerning-text and AFM files; a font source gives kerning and kerning groups"));
    }
    let keep = p.bool_or("keepAlignedComponents", false)?;
    if !kerning && !groups && mode.is_none() {
        return Err(p.bad("nothing to import: turn on importKerning, importKerningGroups or importMetrics"));
    }
    let src_master = match &source {
        Source::Font { font, .. } => Some(source_master(&p, font)?),
        _ => {
            if p.get("sourceMaster").is_some() {
                return Err(p.bad("`sourceMaster` is for a font source; a kerning-text or AFM file holds one master"));
            }
            None
        }
    };
    let plan = if kerning { kerning_plan(&p, &source, src_master.as_deref())? } else { Vec::new() };
    let out = s.edit("Import Metrics", |font| {
        let mut out = Map::new();
        out.insert("master".into(), json!(master));
        out.insert("source".into(), json!(source.name()));
        if groups {
            out.insert("groups".into(), import_groups(font, &source, scope.as_ref())?);
        }
        if kerning {
            out.insert("kerning".into(), import_kerning(font, &master, &plan));
        }
        if let Some(mode) = mode {
            out.insert("metrics".into(), import_glyph_metrics(font, &master, &source, mode, scope.as_ref(), keep)?);
        }
        Ok(out)
    })?;
    Ok(Value::Object(out))
}

fn in_scope(scope: Option<&HashSet<String>>, name: &str) -> bool {
    scope.is_none_or(|s| s.contains(name))
}

// ── Kerning ────────────────────────────────────────────────────────────────────────────────────

/// The pairs to import, by direction, at most [`afm::MAX_PAIRS`] in all from AFM and
/// [`kerning_text::MAX_PAIRS`] from anything else.
fn kerning_plan(p: &Params, source: &Source, src_master: Option<&str>) -> Result<Vec<(Direction, Vec<NewPair>)>> {
    let plan = match source {
        Source::Text(doc) => vec![(doc.direction.unwrap_or(Direction::Ltr), doc.pairs.clone())],
        Source::Afm(doc) => {
            let pairs = doc
                .pairs
                .iter()
                // A glyph named `@…` would be stored as a group.
                .filter(|k| !k.left.starts_with('@') && !k.right.starts_with('@'))
                .map(|k| NewPair { left: Side::Glyph(k.left.clone()), right: Side::Glyph(k.right.clone()), value: k.value })
                .collect();
            vec![(Direction::Ltr, pairs)]
        }
        Source::Font { font, .. } => {
            let master = src_master.unwrap_or_default();
            let shown = metrics_source(font, master).map_or_else(|| master.to_string(), |m| m.id.clone());
            Direction::ALL
                .into_iter()
                .map(|d| {
                    let list: Vec<NewPair> =
                        pairs::stored_pairs(font, d, &shown).iter().map(|r| NewPair { left: r.left(), right: r.right(), value: r.value }).collect();
                    (d, list)
                })
                .filter(|(_, list)| !list.is_empty())
                .collect()
        }
    };
    let total: usize = plan.iter().map(|(_, l)| l.len()).sum();
    let limit = if matches!(source, Source::Afm(_)) { afm::MAX_PAIRS } else { kerning_text::MAX_PAIRS };
    if total > limit {
        return Err(p.bad(format!("the source holds {total} pairs; one import takes at most {limit}")));
    }
    Ok(plan)
}

/// Replaces `master`'s pairs in each direction of `plan`, keeping the place of its table.
fn import_kerning(font: &mut Font, master: &str, plan: &[(Direction, Vec<NewPair>)]) -> Value {
    let mut directions = Vec::new();
    let mut imported = 0usize;
    let (mut glyphs, mut groups, mut seen) = (Vec::new(), Vec::new(), HashSet::new());
    for (direction, list) in plan {
        let table = direction.table_mut(Arc::make_mut(&mut font.kerning));
        if let Some(m) = table.0.iter_mut().find(|m| m.master == master) {
            m.firsts.clear();
        }
        if !list.is_empty() {
            let paste = pairs::plan_paste(font, *direction, master, list);
            imported += paste.added.len();
            pairs::apply_paste(direction.table_mut(Arc::make_mut(&mut font.kerning)), *direction, master, list, &paste, false);
        }
        direction.table_mut(Arc::make_mut(&mut font.kerning)).0.retain(|m| m.master != master || !m.firsts.is_empty());
        directions.push(direction.name());
        unknown_sides(font, *direction, list, &mut glyphs, &mut groups, &mut seen);
    }
    json!({ "directions": directions, "pairs": imported, "unknownGlyphs": glyphs, "unknownGroups": groups })
}

/// The glyph sides the font has no glyph for, and the group sides no glyph of the font belongs
/// to on the side facing the pair.
fn unknown_sides(
    font: &Font,
    direction: Direction,
    list: &[NewPair],
    glyphs: &mut Vec<String>,
    groups: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let mut known: HashSet<(GroupSide, &str)> = HashSet::new();
    for g in font.glyphs.iter() {
        for side in GroupSide::ALL {
            if let Some(name) = side.of(&g.kern_groups) {
                known.insert((side, name));
            }
        }
    }
    for pair in list {
        for (side, position) in [(&pair.left, Position::First), (&pair.right, Position::Second)] {
            match side {
                Side::Glyph(name) if font.glyph(name).is_none() => report(glyphs, seen, &side.command()),
                Side::Group(name) if !known.contains(&(facing(direction, position), name.as_str())) => report(groups, seen, &side.command()),
                _ => {}
            }
        }
    }
}

// ── Kerning groups ─────────────────────────────────────────────────────────────────────────────

fn import_groups(font: &mut Font, source: &Source, scope: Option<&HashSet<String>>) -> Result<Value> {
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).filter(|n| in_scope(scope, n)).collect();
    let mut changed = 0usize;
    let (mut unknown, mut seen) = (Vec::new(), HashSet::new());
    match source {
        Source::Afm(_) => {}
        Source::Text(doc) => {
            // Only the sides the file has group lines for: a file of pairs alone leaves the groups.
            let mut sides = Vec::new();
            let mut wanted: HashMap<(GroupSide, &str), &str> = HashMap::new();
            for g in &doc.groups {
                if !sides.contains(&g.side) {
                    sides.push(g.side);
                }
                for m in &g.members {
                    wanted.insert((g.side, m.as_str()), g.name.as_str());
                    if font.glyph(m).is_none() {
                        report(&mut unknown, &mut seen, m);
                    }
                }
            }
            for name in &names {
                let Some(g) = font.glyph(name) else { continue };
                let mut next = g.kern_groups.clone();
                for side in &sides {
                    *group_slot(&mut next, *side) = wanted.get(&(*side, name.as_str())).map(|s| s.to_string());
                }
                if next != g.kern_groups {
                    font.glyph_mut(name)?.kern_groups = next;
                    changed += 1;
                }
            }
        }
        Source::Font { font: from, .. } => {
            for name in &names {
                let (Some(g), Some(src)) = (font.glyph(name), from.glyph(name)) else { continue };
                if src.kern_groups != g.kern_groups {
                    let next = src.kern_groups.clone();
                    font.glyph_mut(name)?.kern_groups = next;
                    changed += 1;
                }
            }
        }
    }
    Ok(json!({ "glyphs": changed, "unknownGlyphs": unknown }))
}

// ── Glyph metrics ──────────────────────────────────────────────────────────────────────────────

/// One glyph's metrics as the source gives them; `keys` (left, right, width) only from
/// kerning-text.
struct Wanted<'a> {
    glyph: &'a str,
    lsb: Option<f64>,
    rsb: Option<f64>,
    width: Option<f64>,
    keys: Option<[Option<&'a str>; 3]>,
}

const KEY_FIELDS: [Field; 3] = [Field::Left, Field::Right, Field::Width];

/// The shift of the layer's contents and its new width for one glyph: `lsb`, `rsb` and `width`
/// are the layer's now (`None` sidebearings: an empty layer).
fn plan_glyph(mode: Mode, want: &Wanted, lsb: Option<f64>, rsb: Option<f64>, width: f64) -> (f64, f64) {
    let outline = lsb.zip(rsb).map(|(l, r)| width - l - r);
    match mode {
        Mode::LsbAndWidth => {
            let dx = match (want.lsb, lsb) {
                (Some(target), Some(now)) => target - now,
                _ => 0.0,
            };
            (dx, want.width.unwrap_or(width + dx))
        }
        Mode::RsbAndWidth => {
            let w = want.width.unwrap_or(width);
            let dx = match (want.rsb, lsb, outline) {
                (Some(target), Some(l), Some(e)) => w - e - target - l,
                _ => 0.0,
            };
            (dx, w)
        }
        Mode::BothSidebearings => match (want.lsb, want.rsb, lsb, outline) {
            (Some(tl), Some(tr), Some(l), Some(e)) => (tl - l, tl + e + tr),
            _ => (0.0, if outline.is_none() { want.width.unwrap_or(width) } else { width }),
        },
        Mode::WidthCentered => {
            let w = want.width.unwrap_or(width);
            let dx = match (lsb, outline) {
                (Some(l), Some(e)) => round_half_up((w - e) / 2.0) - l,
                _ => 0.0,
            };
            (dx, w)
        }
    }
}

/// Applies the metrics `source` gives to layer `master` of each glyph. A composite aligned
/// automatically (`import.metricsBreaksAutoAlignment`) is first aligned with its new bases and the
/// file's keys; values that differ from that switch its components' alignment off, unless `keep`
/// (Keep Aligned Components), which leaves the glyph as it was.
fn import_glyph_metrics(font: &mut Font, master: &str, source: &Source, mode: Mode, scope: Option<&HashSet<String>>, keep: bool) -> Result<Value> {
    let wanted: Vec<Wanted> = match source {
        Source::Text(doc) => doc
            .metrics
            .iter()
            .map(|m| Wanted {
                glyph: &m.glyph,
                lsb: m.lsb,
                rsb: m.rsb,
                width: m.width,
                keys: Some([m.left_key.as_deref(), m.right_key.as_deref(), m.width_key.as_deref()]),
            })
            .collect(),
        Source::Afm(doc) => doc
            .chars
            .iter()
            .map(|c| {
                let sb = c.sidebearings();
                Wanted { glyph: &c.name, lsb: sb.map(|s| s.0), rsb: sb.map(|s| s.1), width: Some(c.width), keys: None }
            })
            .collect(),
        Source::Font { .. } => Vec::new(),
    };
    let (mut unknown, mut seen) = (Vec::new(), HashSet::new());
    let mut skipped: Vec<Value> = Vec::new();
    let mut bad_keys: Vec<Value> = Vec::new();
    let (mut applied, mut keys_set) = (0usize, 0usize);
    // Composites measure their components, so their bases take their metrics first, and a
    // composite of composites comes after the composites it shows. A glyph listed twice takes
    // its last line.
    let mut last: HashMap<&str, &Wanted> = HashMap::new();
    let mut slots: Vec<spacing::Slot> = Vec::new();
    for want in &wanted {
        if !in_scope(scope, want.glyph) {
            continue;
        }
        if font.glyph(want.glyph).is_none() {
            report(&mut unknown, &mut seen, want.glyph);
            continue;
        }
        if last.insert(want.glyph, want).is_none() {
            slots.push(spacing::Slot { glyph: want.glyph.to_string(), layer: master.to_string(), field: Field::Left });
        }
    }
    let afm_source = matches!(source, Source::Afm(_));
    let mut unaligned: Vec<String> = Vec::new();
    let skip = |skipped: &mut Vec<Value>, glyph: &str, reason: String| {
        if skipped.len() < MAX_REPORTED {
            skipped.push(json!({ "glyph": glyph, "reason": reason }));
        }
    };
    for slot in spacing::update_order(font, &slots) {
        let Some(&want) = last.get(slot.glyph.as_str()) else { continue };
        let Some(g) = font.glyph(want.glyph) else { continue };
        let Some(layer) = g.layers.iter().find(|l| l.id == master) else {
            skip(&mut skipped, want.glyph, "the glyph has no layer for this master".into());
            continue;
        };
        // The keys: what each field gets (`Some(None)`: no key) and the key it shows now.
        let mut keys: Vec<KeyPlan> = Vec::new();
        if let Some(texts) = &want.keys {
            for (field, text) in KEY_FIELDS.into_iter().zip(texts) {
                let shown = spacing::effective_key(g, layer, field).map(|(scope, _)| scope);
                let new = match text {
                    None => None,
                    Some(t) => match spacing::parse_field(t, field, |n| font.glyph(n).is_some()) {
                        Ok(Entry::Key(k)) => Some((k.scope, k.stored())),
                        Ok(Entry::Number(_)) => None,
                        Err(e) => {
                            if bad_keys.len() < MAX_REPORTED {
                                bad_keys.push(json!({ "glyph": want.glyph, "field": field.name(), "key": clip(t), "error": e.to_string() }));
                            }
                            continue;
                        }
                    },
                };
                keys.push((field, new, shown));
            }
        }
        // A composite aligned automatically is compared with what alignment gives it once its
        // bases have their new metrics and its own keys are in (an offset key changes it).
        let original = g.clone();
        let aligned = spacing::derives_metrics(font, layer);
        if aligned {
            {
                let mut gm = font.glyph_mut(want.glyph)?;
                set_keys(&mut gm, master, &keys);
            }
            if let Ok(Some(realigned)) = fontcraft_components::align::align_layer(font, want.glyph, master)
                && let Some(l) = font.glyph_mut(want.glyph)?.layers.iter_mut().find(|l| l.id == master)
            {
                *l = realigned;
            }
        }
        let Some(layer) = font.glyph(want.glyph).and_then(|g| g.layers.iter().find(|l| l.id == master)) else { continue };
        // An AFM box is the upright outline's, as export writes it; the other sources give
        // sidebearings as measured, slanted back in an italic master.
        let measured = if afm_source {
            spacing::measure::upright_bounds(font, layer).map(|b| (b.map(|b| b.x0), b.map(|b| layer.width - b.x1)))
        } else {
            spacing::metric(font, layer, Field::Left).and_then(|l| Ok((l, spacing::metric(font, layer, Field::Right)?)))
        };
        let (lsb, rsb) = match measured {
            Ok(v) => v,
            Err(e) => {
                if aligned {
                    restore(font, &original)?;
                }
                skip(&mut skipped, want.glyph, e.to_string());
                continue;
            }
        };
        let (dx, width) = plan_glyph(mode, want, lsb, rsb, layer.width);
        if aligned {
            // Values that round to the derived ones leave alignment on.
            if dx.abs() < 0.5 && (width - layer.width).abs() < 0.5 {
                keys_set += keys.len();
                applied += 1;
                continue;
            }
            if keep {
                restore(font, &original)?;
                skip(&mut skipped, want.glyph, "the glyph is aligned automatically and Keep Aligned Components keeps its derived metrics".into());
                continue;
            }
        }
        if !WIDTH_RANGE.contains(&width) {
            if aligned {
                restore(font, &original)?;
            }
            skip(&mut skipped, want.glyph, format!("the width would be {width}, outside 0 to 65535"));
            continue;
        }
        let mut gm = font.glyph_mut(want.glyph)?;
        let Some(l) = gm.layers.iter_mut().find(|l| l.id == master) else { continue };
        let before = l.width + dx;
        if let Err(e) = super::shared::shift_with_right_edge(l, dx) {
            drop(gm);
            if aligned {
                restore(font, &original)?;
            }
            skip(&mut skipped, want.glyph, e);
            continue;
        }
        l.width = width;
        fontcraft_model::guide_follow::follow_width(l, before);
        if aligned {
            // The imported values hold only with the layer's components placed by hand.
            for shape in &mut l.shapes {
                if let Shape::Component(c) = shape
                    && c.info.alignment != Alignment::Disabled
                {
                    c.info.alignment = Alignment::Disabled;
                }
            }
            if unaligned.len() < MAX_REPORTED {
                unaligned.push(want.glyph.to_string());
            }
        } else {
            set_keys(&mut gm, master, &keys);
        }
        keys_set += keys.len();
        applied += 1;
    }
    Ok(json!({
        "mode": mode.name(),
        "glyphs": applied,
        "keys": keys_set,
        "skipped": skipped,
        "unaligned": unaligned,
        "unknownGlyphs": unknown,
        "badKeys": bad_keys,
    }))
}

/// Stores the keys of `keys` on `glyph` and its layer `master`: a field's new key where it was
/// typed for (a glyph-wide key drops the layer's own, which would hide it), or no key, which
/// removes the key the field showed.
fn set_keys(glyph: &mut Glyph, master: &str, keys: &[KeyPlan]) {
    let Some(l) = glyph.layers.iter_mut().find(|l| l.id == master) else { return };
    for (field, new, shown) in keys {
        match new {
            Some((Scope::Local, stored)) => *key_slot(&mut l.info.metrics_keys, *field) = Some(stored.clone()),
            Some((Scope::Glyph, stored)) => {
                if *shown == Some(Scope::Local) {
                    *key_slot(&mut l.info.metrics_keys, *field) = None;
                }
                *key_slot(&mut glyph.info.metrics_keys, *field) = Some(stored.clone());
            }
            None => match shown {
                Some(Scope::Local) => *key_slot(&mut l.info.metrics_keys, *field) = None,
                Some(Scope::Glyph) => *key_slot(&mut glyph.info.metrics_keys, *field) = None,
                None => {}
            },
        }
    }
}

/// Puts glyph `original` back as it was before this import touched it.
fn restore(font: &mut Font, original: &Arc<Glyph>) -> Result<()> {
    let mut gm = font.glyph_mut(&original.name)?;
    let g: &mut Glyph = &mut gm;
    *g = Glyph::clone(original);
    Ok(())
}
