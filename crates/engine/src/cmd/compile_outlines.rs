//! The outline, CFF, glyph-set and output options of `font.compile` and `font.compilePlan`
//! (M2.13): the new keys of the `outlines` group, the `cff` group, and the top-level keys that
//! choose the glyphs built, the `.notdef`, extra glyph data, production names, the version, the
//! `post` format, tables to leave out and raw table files.
//!
//! Three kinds of step, in build order:
//! - Compile options ([`OutputRequest::read`] sets them): the CFF rounding tolerance, operators,
//!   subroutine cap and widths, `USE_MY_METRICS`, the autohint switch.
//! - A copy of the font before the build ([`OutputRequest::font`]): the glyph subset (every other
//!   glyph stops exporting, so the feature code is compiled against the reduced set, as the
//!   assembly leaves non-exporting glyphs out), a replaced `.notdef`, the production-name
//!   parameter and the bumped version.
//! - A rewrite of each static font's bytes ([`OutputRequest::rewrite`]): the `post` format, the
//!   tables left out, and the raw table files taken from the result.
//!
//! The options with no counterpart in FontCraft are refused with the reason ([`REFUSED`]).
//!
//! Rows: the `compile` options `--flatten-components`, `--decompose-components`,
//! `--decompose-transformed-components`, `--prefer-simple-glyphs`, `--erase-open-corners`,
//! `--propagate-anchors`, `--auto-use-my-metrics`, `--filter`, `--cff-round-tolerance`,
//! `--optimize-cff`, `--max-subrs`, `--optimize-widths`, `--cid-keyed`, `--production-names`,
//! `--post-format`, `--glyphs`, `--notdef`, `--glyph-data`, `--bump-version`, `--optimize-gvar`,
//! `--colr-layer-reuse`, `--exclude-table`, `--check-compatibility`, `--autohint`,
//! `--warn-unhinted`, `--tables`, `--warnings`; `compile.cffRoundTolerance`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value};

use fontcraft_compile::{DataRef, Options};
use fontcraft_container::sfnt;
use fontcraft_model::glyphdata::{GlyphData, xml};
use fontcraft_model::{DONT_USE_PRODUCTION_NAMES, Font, Shape, plist};
use fontcraft_tables::Tag;

use crate::params::Params;
use crate::{EngineError, Result, Session};

/// The keys of the `outlines` group this module reads; `compile::outline_options` reads the rest.
pub(super) const OUTLINE_KEYS: [&str; 5] = ["preferSimpleGlyphs", "eraseOpenCorners", "propagateAnchors", "autoUseMyMetrics", "filters"];

/// The keys of the `cff` group.
const CFF_KEYS: [&str; 6] = ["roundTolerance", "optimize", "maxSubrs", "optimizeWidths", "cidKeyed", "warnUnhinted"];

/// The parameters of options with no counterpart, each with why it is refused.
const REFUSED: [(&str, &str); 9] = [
    ("masterFinder", "variable fonts are built from the source's masters, not from compiled master files"),
    ("sparseGlyphs", "a sparse master is a master layer that leaves glyphs out, read from the source itself"),
    ("emitDesignspace", "the build reads the source directly and writes no intermediate designspace"),
    ("ufoStructure", "the build writes no intermediate master or instance sources"),
    ("buildDir", "the build keeps nothing in a working folder"),
    ("emitDebug", "the build has no intermediate representation to write; `features.debugFile` writes the feature code"),
    ("validateSources", "every source is checked as it is read; audit.run checks it further"),
    ("keepMvar", "a compiled font is opened as a document and compiled again, so no MVAR is carried over"),
    ("designspace", "a compiled variable font is opened as a document with its axes; no separate designspace is read"),
];

/// Most files `glyphData` may name.
const MAX_GLYPH_DATA_FILES: usize = crate::glyph_data::MAX_FILES;

/// Tables by tag, each with its bytes.
type TableBytes = Vec<(Tag, Vec<u8>)>;
/// Most tags `excludeTables` or `tables` may hold.
const MAX_TAGS: usize = 64;
/// The tables a font cannot be read without, which `excludeTables` refuses.
const REQUIRED_TABLES: [&[u8; 4]; 8] = [b"head", b"maxp", b"hhea", b"hmtx", b"glyf", b"loca", b"CFF ", b"CFF2"];
/// Most components followed from one glyph when the glyph subset is closed.
const MAX_CLOSURE_STEPS: usize = 1_000_000;
/// The step `--bump-version` takes without a value, in thousandths.
const DEFAULT_BUMP: u32 = 5;

/// How `--notdef` treats the source's `.notdef`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Notdef {
    /// Keep it; draw one when the source has none (also `allow-missing`).
    #[default]
    Auto,
    /// Draw a new one whatever the source holds.
    Replace,
}

/// How `bumpVersion` changes the version.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bump {
    /// Raise it by this many thousandths.
    By(u32),
    /// Set it to major, minor (thousandths).
    To(u16, u16),
}

/// The raw tables taken from one built font: its job label, its format and the tables by tag.
pub(super) type RawTables = (String, fontcraft_compile::Flavour, TableBytes);

/// What the M2.13 options ask for.
#[derive(Clone, Default)]
pub(super) struct OutputRequest {
    /// The glyphs to build (with their components and `.notdef`); `None` builds all.
    glyphs: Option<Vec<String>>,
    notdef: Notdef,
    /// Glyph data from the files `glyphData` names, on top of the document's.
    glyph_data: Option<Arc<GlyphData>>,
    /// `productionNames`; `None` keeps the source's setting.
    production_names: Option<bool>,
    /// `postFormat`: 2 or 3.
    post_format: Option<u8>,
    bump: Option<Bump>,
    exclude: Vec<Tag>,
    /// Tables written as raw files beside each font.
    tables: Vec<Tag>,
    /// `checkCompatibility: false`.
    no_compatibility_check: bool,
    /// `warnings: "none"`.
    quiet: bool,
    /// What reading the options found that the build should report.
    notes: Vec<String>,
}

impl OutputRequest {
    /// Reads the options of `p` and sets the compile options they decide.
    pub(super) fn read(p: &Params, s: &Session, options: &mut Options) -> Result<Self> {
        for (key, reason) in REFUSED {
            if p.get(key).is_some_and(|v| !v.is_null()) {
                return Err(EngineError::Unsupported(format!("`{key}`: {reason}")));
            }
        }
        let mut out = OutputRequest::default();
        read_outlines(p, options)?;
        read_cff(p, options)?;
        out.glyphs = p.opt_strings("glyphs")?.map(|names| names.into_iter().map(str::to_string).collect());
        if out.glyphs.as_ref().is_some_and(|g| g.len() > fontcraft_model::MAX_GLYPHS) {
            return Err(p.bad(format!("`glyphs` holds at most {} names", fontcraft_model::MAX_GLYPHS)));
        }
        out.notdef = match p.opt_str("notdef")? {
            None | Some("auto" | "allow-missing") => Notdef::Auto,
            Some("replace") => Notdef::Replace,
            Some(other) => return Err(p.bad(format!("`notdef` is \"auto\", \"replace\" or \"allow-missing\", got \"{other}\""))),
        };
        if let Some(files) = p.opt_strings("glyphData")? {
            out.glyph_data = Some(glyph_data(p, s, &files, &mut out.notes)?);
        }
        if let Some(data) = &out.glyph_data {
            options.glyph_data = DataRef(Some(data.clone()));
        }
        out.production_names = flag(p, "productionNames")?;
        out.post_format = match p.opt_int("postFormat", 2, 3)? {
            None => None,
            Some(n) => u8::try_from(n).ok(),
        };
        out.bump = bump(p)?;
        out.exclude = tags(p, "excludeTables")?;
        if let Some(tag) = out.exclude.iter().find(|t| REQUIRED_TABLES.contains(t)) {
            return Err(p.bad(format!("`excludeTables`: a font cannot be read without its {} table", tag_text(tag))));
        }
        out.tables = tags(p, "tables")?;
        if flag(p, "colrLayerReuse")?.is_some() {
            out.notes.push("`colrLayerReuse` changes nothing: no variable COLR table is built, so there are no layer runs to share".into());
        }
        match p.get("checkCompatibility") {
            None | Some(Value::Null | Value::Bool(true)) => {}
            Some(Value::Bool(false)) => out.no_compatibility_check = true,
            Some(Value::String(s)) if s == "fix" => {
                return Err(EngineError::Unsupported(
                    "`checkCompatibility: \"fix\"`: no step repairs incompatible curves; font.checkCompatibility lists them".into(),
                ));
            }
            Some(_) => return Err(p.bad("`checkCompatibility` is true, false or \"fix\"")),
        }
        match p.get("autohint") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(on)) => options.hinting.autohint = *on,
            Some(Value::String(_)) => {
                return Err(EngineError::Unsupported("autohinter options in `autohint`: give true or false".into()));
            }
            Some(_) => return Err(p.bad("`autohint` is true or false")),
        }
        out.quiet = match p.opt_str("warnings")? {
            None | Some("default" | "all") => false,
            Some("none") => true,
            Some(other) => return Err(p.bad(format!("`warnings` is \"default\", \"all\" or \"none\", got \"{other}\""))),
        };
        Ok(out)
    }

    /// Refuses what does not go with the rest of the request: `variable` (variable formats are
    /// asked for) and `collection`.
    pub(super) fn check(&self, variable: bool, collection: bool) -> Result<()> {
        let rewrites = self.rewrites();
        if variable && rewrites {
            return Err(EngineError::Unsupported(
                "`postFormat`, `excludeTables` and `tables` with a variable format: they rewrite static fonts; build the variable fonts in a separate command".into(),
            ));
        }
        if variable && self.no_compatibility_check {
            return Err(EngineError::Unsupported(
                "`checkCompatibility: false` with a variable format: a variable font cannot be built from incompatible glyphs".into(),
            ));
        }
        if collection && rewrites {
            return Err(EngineError::Unsupported(
                "`postFormat`, `excludeTables` and `tables` with a collection: they rewrite separate font files; leave out `collection`".into(),
            ));
        }
        Ok(())
    }

    /// Whether the static fonts' bytes are rewritten.
    pub(super) fn rewrites(&self) -> bool {
        self.post_format == Some(3) || !self.exclude.is_empty() || !self.tables.is_empty()
    }

    /// Whether raw table files are asked for.
    pub(super) fn wants_tables(&self) -> bool {
        !self.tables.is_empty()
    }

    /// `warnings: "none"`.
    pub(super) fn quiet(&self) -> bool {
        self.quiet
    }

    /// `font` with the glyph subset, the `.notdef`, the production-name setting and the version
    /// applied; `None` when the request changes none of them.
    pub(super) fn font(&self, p: &Params, font: &Font, warnings: &mut Vec<String>) -> Result<Option<Font>> {
        warnings.extend(self.notes.iter().cloned());
        if self.glyphs.is_none() && self.notdef == Notdef::Auto && self.production_names.is_none() && self.bump.is_none() {
            return Ok(None);
        }
        let mut out = font.clone();
        if let Some(names) = &self.glyphs {
            let keep = closure(p, font, names)?;
            for i in 0..out.glyphs.len() {
                let Some(mut g) = out.glyphs.get_mut_at(i) else { continue };
                if g.export && !keep.contains(g.name.as_str()) {
                    g.export = false;
                }
            }
        }
        if self.notdef == Notdef::Replace {
            out.glyphs.remove(".notdef");
        }
        match self.production_names {
            None => {}
            Some(false) => Arc::make_mut(&mut out.info).params.set(DONT_USE_PRODUCTION_NAMES, plist::Value::Bool(true)),
            Some(true) => {
                let info = Arc::make_mut(&mut out.info);
                info.params.remove(DONT_USE_PRODUCTION_NAMES);
                info.settings.disables_nice_names = None;
                for instance in Arc::make_mut(&mut out.instances).iter_mut() {
                    instance.params.remove(DONT_USE_PRODUCTION_NAMES);
                }
            }
        }
        if let Some(bump) = self.bump {
            let (major, minor) = bumped(font.version_major, font.version_minor, bump);
            out.version_major = major;
            out.version_minor = minor;
        }
        Ok(Some(out))
    }

    /// `bytes` (one static font) with the `post` format and the tables left out applied, and the
    /// raw tables `tables` asks for taken from the result. `warnings` gets the tables asked for
    /// that the font does not have.
    pub(super) fn rewrite(&self, bytes: Vec<u8>, warnings: &mut Vec<String>) -> Result<(Vec<u8>, TableBytes)> {
        if !self.rewrites() {
            return Ok((bytes, Vec::new()));
        }
        let parsed = sfnt::parse(&bytes).map_err(|e| EngineError::Internal(format!("reading the compiled font: {e}")))?;
        let mut post: Option<Vec<u8>> = None;
        if self.post_format == Some(3)
            && let Some(old) = parsed.get(*b"post")
        {
            post = Some(post_format_3(old)?);
        }
        for tag in &self.exclude {
            if parsed.get(*tag).is_none() {
                warnings.push(format!("`excludeTables`: the font has no {} table", tag_text(tag)));
            }
        }
        let tables: Vec<sfnt::Table<'_>> = parsed
            .tables
            .iter()
            .filter(|t| !self.exclude.contains(&t.tag))
            .map(|t| match (&post, &t.tag) {
                (Some(data), b"post") => sfnt::Table { tag: t.tag, data: data.as_slice() },
                _ => *t,
            })
            .collect();
        let mut raw = Vec::with_capacity(self.tables.len());
        for tag in &self.tables {
            match tables.iter().find(|t| t.tag == *tag) {
                Some(t) => raw.push((*tag, t.data.to_vec())),
                None => warnings.push(format!("`tables`: the font has no {} table to write", tag_text(tag))),
            }
        }
        if post.is_none() && self.exclude.is_empty() {
            return Ok((bytes, raw));
        }
        let rebuilt = sfnt::assemble(parsed.flavor, &tables).map_err(|e| EngineError::Internal(format!("rewriting the compiled font: {e}")))?;
        Ok((rebuilt, raw))
    }
}

/// The text of `tag`, without its padding spaces (`CFF `, `cvt `).
pub(super) fn tag_text(tag: &Tag) -> String {
    String::from_utf8_lossy(tag).trim_end().to_string()
}

/// The name of the raw table file of `tag` for the font file `name`: its stem, a dot and the tag.
pub(super) fn raw_name(name: &str, tag: &Tag) -> String {
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    };
    format!("{stem}.{}", tag_text(tag))
}

/// The value at `key`: true, false or absent.
fn flag(p: &Params, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(p.bad(format!("`{key}` must be true or false"))),
    }
}

/// The `true`/`false` at `key` of `group` (named `name` in messages).
fn group_flag(p: &Params, group: &Map<String, Value>, name: &str, key: &str) -> Result<Option<bool>> {
    match group.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(p.bad(format!("`{name}.{key}` must be true or false"))),
    }
}

/// The new keys of the `outlines` group.
fn read_outlines(p: &Params, options: &mut Options) -> Result<()> {
    let Some(group) = p.opt_object("outlines")? else { return Ok(()) };
    if group_flag(p, group, "outlines", "preferSimpleGlyphs")? == Some(false) {
        return Err(EngineError::Unsupported(
            "`outlines.preferSimpleGlyphs: false` (a glyph's contours moved into a new glyph used as a component): a glyph with contours and components is always written as contours".into(),
        ));
    }
    if group_flag(p, group, "outlines", "eraseOpenCorners")? == Some(true) {
        return Err(EngineError::Unsupported(
            "`outlines.eraseOpenCorners`: it runs the export step that erases open corners, which is not built yet (M3.31)".into(),
        ));
    }
    if group_flag(p, group, "outlines", "propagateAnchors")?.is_some() {
        return Err(EngineError::Unsupported(
            "`outlines.propagateAnchors`: anchors are propagated as the source asks (always for the reference's format, by the filter list for a UFO), and the compile has no switch that overrides it yet".into(),
        ));
    }
    if group.get("filters").is_some_and(|v| !v.is_null()) {
        return Err(EngineError::Unsupported(
            "`outlines.filters`: filter commands do not run inside a compile yet; run them on the document first".into(),
        ));
    }
    if let Some(on) = group_flag(p, group, "outlines", "autoUseMyMetrics")? {
        options.use_my_metrics = on;
    }
    Ok(())
}

/// The `cff` group.
fn read_cff(p: &Params, options: &mut Options) -> Result<()> {
    let Some(group) = p.opt_object("cff")? else { return Ok(()) };
    if let Some(key) = group.keys().find(|k| !CFF_KEYS.contains(&k.as_str())) {
        return Err(EngineError::Unsupported(format!("`cff.{key}`")));
    }
    if let Some(v) = group.get("roundTolerance").filter(|v| !v.is_null()) {
        let t = v.as_f64().filter(|t| t.is_finite() && (0.0..=0.5).contains(t));
        options.cff.round_tolerance = t.ok_or_else(|| p.bad("`cff.roundTolerance` must be a number from 0 to 0.5"))?;
    }
    if let Some(v) = group.get("optimize").filter(|v| !v.is_null()) {
        let level = v.as_u64().filter(|n| *n <= 2).ok_or_else(|| p.bad("`cff.optimize` is 0, 1 or 2"))?;
        if level < 2 {
            if p.get("subroutinize").and_then(Value::as_bool) == Some(true) {
                return Err(p.bad("`cff.optimize` 0 and 1 write no subroutines: leave out `subroutinize: true`"));
            }
            options.subroutinize = Some(false);
        }
        options.cff.specialise = level > 0;
    }
    if let Some(v) = group.get("maxSubrs").filter(|v| !v.is_null()) {
        const LIMIT: u64 = fontcraft_compile::MAX_SUBRS_PER_INDEX as u64;
        let n = v.as_u64().filter(|n| *n <= LIMIT).ok_or_else(|| p.bad(format!("`cff.maxSubrs` must be a whole number from 0 to {LIMIT}")))?;
        // 0 stands for the format's limit.
        options.cff.max_subrs =
            if n == 0 { fontcraft_compile::MAX_SUBRS_PER_INDEX } else { usize::try_from(n).unwrap_or(fontcraft_compile::MAX_SUBRS_PER_INDEX) };
    }
    if let Some(on) = group_flag(p, group, "cff", "optimizeWidths")? {
        options.cff.optimize_widths = on;
    }
    if group.get("cidKeyed").is_some_and(|v| !v.is_null() && v != &Value::Bool(false)) {
        return Err(EngineError::Unsupported(
            "`cff.cidKeyed`: a name-keyed source is not converted; a source with a Registry-Ordering-Supplement and a glyph-to-CID map builds a CID-keyed CFF by itself".into(),
        ));
    }
    if group_flag(p, group, "cff", "warnUnhinted")? == Some(true) {
        return Err(EngineError::Unsupported("`cff.warnUnhinted`: the compile does not report CFF glyphs without stem hints yet".into()));
    }
    Ok(())
}

/// `bumpVersion`: true (by 0.005), a whole number of thousandths (a number or its text), or the
/// version itself as `X.YYY`.
fn bump(p: &Params) -> Result<Option<Bump>> {
    const MESSAGE: &str = "`bumpVersion` is true, a whole number of thousandths from 1 to 65535, or a version written X.YYY";
    match p.get("bumpVersion") {
        None | Some(Value::Null | Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(Bump::By(DEFAULT_BUMP))),
        Some(Value::Number(n)) => match n.as_u64().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0) {
            Some(n) => Ok(Some(Bump::By(u32::from(n)))),
            None => Err(p.bad(MESSAGE)),
        },
        Some(Value::String(s)) => parse_bump(s).ok_or_else(|| p.bad(format!("{MESSAGE}, got \"{s}\""))).map(Some),
        Some(_) => Err(p.bad(MESSAGE)),
    }
}

/// A `bumpVersion` text: `true`, `N` thousandths or `X.YYY`.
fn parse_bump(s: &str) -> Option<Bump> {
    let s = s.trim();
    if s == "true" {
        return Some(Bump::By(DEFAULT_BUMP));
    }
    match s.split_once('.') {
        None => s.parse::<u16>().ok().filter(|n| *n > 0).map(|n| Bump::By(u32::from(n))),
        Some((major, minor)) => {
            if minor.is_empty() || minor.len() > 3 || !minor.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let major = major.parse::<u16>().ok()?;
            // `1.5` is 1.500: the digits are thousandths, padded on the right.
            let padded = format!("{minor:0<3}");
            Some(Bump::To(major, padded.parse::<u16>().ok()?))
        }
    }
}

/// The version `major.minor` (minor in thousandths) after `bump`; the minor part carries into
/// the major one past 999, and the major one stops at its largest value.
fn bumped(major: u16, minor: u16, bump: Bump) -> (u16, u16) {
    match bump {
        Bump::To(major, minor) => (major, minor),
        Bump::By(step) => {
            let total = u32::from(major).saturating_mul(1000).saturating_add(u32::from(minor)).saturating_add(step);
            match u16::try_from(total / 1000) {
                Ok(major) => (major, u16::try_from(total % 1000).unwrap_or(0)),
                Err(_) => (u16::MAX, 999),
            }
        }
    }
}

/// The tags at `key`, each once.
fn tags(p: &Params, key: &str) -> Result<Vec<Tag>> {
    let Some(list) = p.opt_strings(key)? else { return Ok(Vec::new()) };
    if list.len() > MAX_TAGS {
        return Err(p.bad(format!("`{key}` holds at most {MAX_TAGS} tags")));
    }
    let mut out = Vec::with_capacity(list.len());
    for text in list {
        let tag = fontcraft_tables::sfnt::parse_tag(text).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    Ok(out)
}

/// The glyphs `names` keep: themselves, `.notdef`, and every glyph their components reach in any
/// layer. Each name must be a glyph of `font`.
fn closure(p: &Params, font: &Font, names: &[String]) -> Result<HashSet<String>> {
    let mut keep: HashSet<String> = HashSet::new();
    let mut todo: Vec<&str> = Vec::new();
    for name in names {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("`glyphs`: there is no glyph `{name}`")));
        }
        todo.push(name);
    }
    if font.glyph(".notdef").is_some() {
        todo.push(".notdef");
    }
    let mut steps = 0usize;
    while let Some(name) = todo.pop() {
        if !keep.insert(name.to_string()) {
            continue;
        }
        let Some(glyph) = font.glyph(name) else { continue };
        for layer in &glyph.layers {
            for shape in &layer.shapes {
                if let Shape::Component(c) = shape {
                    steps = steps.saturating_add(1);
                    if steps > MAX_CLOSURE_STEPS {
                        return Err(EngineError::Unsupported(format!("following more than {MAX_CLOSURE_STEPS} components for `glyphs`")));
                    }
                    if !keep.contains(c.glyph.as_str()) {
                        todo.push(c.glyph.as_str());
                    }
                }
            }
        }
    }
    Ok(keep)
}

/// `old` (a `post` table) as format 3: its header with the version changed and no glyph names.
fn post_format_3(old: &[u8]) -> Result<Vec<u8>> {
    const HEADER: usize = 32;
    let mut out = old.get(..HEADER).ok_or_else(|| EngineError::Internal("the compiled post table is too short".into()))?.to_vec();
    if let Some(version) = out.get_mut(..4) {
        version.copy_from_slice(&0x0003_0000u32.to_be_bytes());
    }
    Ok(out)
}

/// The glyph data with the files at `paths` on top of the document's (the user's files, then the
/// files beside the source unless the font turns them off). A file that cannot be read or parsed
/// stops the build, since it was asked for by name.
fn glyph_data(p: &Params, s: &Session, paths: &[&str], notes: &mut Vec<String>) -> Result<Arc<GlyphData>> {
    if paths.len() > MAX_GLYPH_DATA_FILES {
        return Err(p.bad(format!("`glyphData` names at most {MAX_GLYPH_DATA_FILES} files")));
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(paths.len());
    for path in paths {
        let path = PathBuf::from(path);
        crate::file_access::check_read(&path).map_err(|e| p.bad(format!("`glyphData`: {e}")))?;
        let input = super::file_io::read_input_up_to(&path, xml::MAX_FILE_BYTES)?;
        if input.bytes.len() > xml::MAX_FILE_BYTES {
            return Err(p.bad(format!("`glyphData`: {} is larger than {} bytes", path.display(), xml::MAX_FILE_BYTES)));
        }
        files.push((path.display().to_string(), input.bytes));
    }
    let named: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let mut problems = Vec::new();
    let parsed = crate::glyph_data::parse_files(&named, &mut problems);
    if parsed.len() < files.len() {
        return Err(p.bad(format!("`glyphData`: {}", problems.join("; "))));
    }
    notes.append(&mut problems);
    let doc = s.doc()?;
    let source: &[Vec<fontcraft_model::glyphdata::Record>] =
        if crate::glyph_data::ignores_source_files(&doc.font) { &[] } else { doc.glyph_data.files.as_slice() };
    let mut problems = Vec::new();
    let merged = crate::glyph_data::merge(&[s.user_glyph_data.files.as_slice(), source, parsed.as_slice()], &mut problems);
    match merged {
        Some(data) if problems.is_empty() => Ok(data),
        _ => Err(p.bad(format!("`glyphData`: {}", problems.join("; ")))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bumped_version_carries_into_the_major_part() {
        assert_eq!(bumped(1, 0, Bump::By(5)), (1, 5));
        assert_eq!(bumped(1, 998, Bump::By(5)), (2, 3));
        assert_eq!(bumped(u16::MAX, 999, Bump::By(5)), (u16::MAX, 999));
        assert_eq!(parse_bump("2.5"), Some(Bump::To(2, 500)));
        assert_eq!(parse_bump("2.050"), Some(Bump::To(2, 50)));
        assert_eq!(parse_bump("12"), Some(Bump::By(12)));
        assert_eq!(parse_bump("true"), Some(Bump::By(5)));
        for bad in ["", "0", "1.2345", "x.1", "1.", "-3"] {
            assert_eq!(parse_bump(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_raw_table_file_takes_the_font_files_stem() {
        assert_eq!(raw_name("Font-Regular.ttf", b"CFF "), "Font-Regular.CFF");
        assert_eq!(raw_name("ttf/Font.otf", b"GPOS"), "ttf/Font.GPOS");
        assert_eq!(raw_name("noext", b"cvt "), "noext.cvt");
    }
}
