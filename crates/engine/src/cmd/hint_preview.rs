//! Hinted previews: grid-fitted outlines and pixels of a glyph at a pixel size (`fontcraft_render::hinted`).
//!
//! Rows: `hint.tt.renderPreview`, `hint.ps.renderPreview`. Both build one instance in memory
//! through the export path (`compile::build`), so the preview shows the font File › Export would
//! write, then hint it with skrifa (M10 § 7, question 4). Both are queries: they change nothing.

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_compile::{Flavour, HintingOptions, Options};
use fontcraft_geom::{BezPath, PathEl};
use fontcraft_model::{Font, Target};
use fontcraft_render::RenderError;
use fontcraft_render::hinted::{HintedBitmap, HintedOutline, HintedRenderer, Intent, MAX_PPEM, MIN_PPEM};
use serde_json::{Map, Value, json};
use skrifa::{FontRef, MetadataProvider};

use super::compile::{self, Container, Dest, Job, Pick, Report, Request, Sink};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most pixel sizes in one request.
pub const MAX_SIZES: usize = 64;
/// Most glyphs (from `glyphs` and `text` together) in one request.
pub const MAX_PREVIEW_GLYPHS: usize = 1_000;
/// Most glyph renderings (glyphs times sizes) in one request.
pub const MAX_RENDERINGS: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "hint.tt.renderPreview",
            "Render Hinted Preview",
            [],
            None,
            "{instance?: name (a static instance by name, `#n` for an unnamed one, or a master id; default: the first master), glyphs?: [names], text?: string (the glyphs of its characters, after `glyphs`; one of the two is required), ppem: number or [numbers] (whole pixel sizes 1 to 400, at most 64), intent?: 'grayscale'|'cleartype'|'directwrite' (default 'grayscale'), output?: 'outline'|'bitmap' (default 'outline'), autohint?: bool (default false)} builds the instance as TrueType in memory, runs its instructions at each size in the rendering intent and returns the grid-fitted outlines (pixels, y up) or the pixels; a glyph whose program fails is drawn unhinted with a message → {instance, format: 'ttf', intent, sizes: [{ppem, glyphs: [{glyph, gid, instructed, hinted, message?, advance, contours?: [[[x, y, onCurve]]], bitmap?: {width, height, left, top, png (base64; black ink, alpha = coverage)}}]}], warnings}",
            has_doc,
            tt_render_preview
        ),
        cmd!(
            query "hint.ps.renderPreview",
            "Render Hinted Preview",
            [],
            None,
            "{instance?: name (a static instance by name, `#n` for an unnamed one, or a master id; default: the first master), text: string, ppem: number or [numbers] (whole pixel sizes 1 to 400, at most 64), hinted?: bool (default true; false renders the same text unhinted, for comparison), format?: 'png'|'json' (default 'png'), autohint?: bool (default false)} builds the instance as CFF in memory and renders the text on one line at each size with its PostScript hints applied; 'png' returns the line's pixels, 'json' each glyph's outline in pixels → {instance, format: 'otf', hinted, sizes: [{ppem, width, height, left, top, png? (base64; black ink, alpha = coverage), glyphs?: [{glyph, gid, hinted, message?, advance, contours}]}], warnings}",
            has_doc,
            ps_render_preview
        ),
    ]
}

/// One font built in memory for a preview.
struct Built {
    bytes: Arc<[u8]>,
    glyph_order: Vec<String>,
    label: String,
    warnings: Vec<String>,
}

/// The instance or master `instance` names, or the first master.
fn job(font: &Font, p: &Params) -> Result<Job> {
    let Some(name) = p.opt_str("instance")? else {
        let m = font.default_master()?;
        return Ok(Job { label: m.name.clone(), source: "default", target: Target::Master(m.id.clone()) });
    };
    if let Some(m) = font.masters.iter().find(|m| m.id == name) {
        return Ok(Job { label: m.name.clone(), source: "master", target: Target::Master(m.id.clone()) });
    }
    let jobs = compile::jobs(font, &Pick::Named(vec![name.to_string()]), false, p)?;
    jobs.into_iter().next().ok_or_else(|| p.bad(format!("the font has no instance or master `{name}`")))
}

fn build(s: &Session, p: &Params, flavour: Flavour) -> Result<Built> {
    let font = s.font()?;
    let job = job(font, p)?;
    let options = Options { hinting: HintingOptions { autohint: p.bool_or("autohint", false)?, ..Default::default() }, ..Default::default() };
    let req = Request {
        flavours: vec![flavour],
        containers: vec![Container::Plain],
        options,
        dest: Dest::Memory,
        first_only: false,
        autohinter: crate::external_autohint::trusted(s),
    };
    let mut sink = Sink::new(&req.dest, true, s.export_outside())?;
    let mut report = Report::default();
    let doc = s.doc()?;
    compile::build(font, &doc.title, doc.path.as_deref(), s.glyph_data()?, std::slice::from_ref(&job), &req, &mut sink, &mut report)?;
    if report.fonts.is_empty() {
        return Err(compile::nothing_built(&report));
    }
    let mut warnings = std::mem::take(&mut report.warnings);
    let Some(out) = report.fonts.into_iter().next() else { return Err(EngineError::Internal("the preview font was not built".into())) };
    warnings.extend(out.warnings);
    let bytes =
        out.files.into_iter().next().and_then(|f| f.data).ok_or_else(|| EngineError::Internal("the preview font was not kept in memory".into()))?;
    Ok(Built { bytes: bytes.into(), glyph_order: out.glyph_order, label: job.label, warnings })
}

/// `ppem`: one whole number or a list of them, each in range, repeats dropped.
fn sizes(p: &Params) -> Result<Vec<u16>> {
    let values: Vec<&Value> = match p.get("ppem") {
        None => return Err(p.bad("`ppem` is required")),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(v) => vec![v],
    };
    if values.is_empty() {
        return Err(p.bad("`ppem` names no size"));
    }
    if values.len() > MAX_SIZES {
        return Err(p.bad(format!("`ppem` holds {} sizes; at most {MAX_SIZES} are allowed", values.len())));
    }
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        let n = v.as_f64().filter(|n| n.is_finite() && n.fract() == 0.0);
        let Some(size) = n.filter(|n| (f64::from(MIN_PPEM)..=f64::from(MAX_PPEM)).contains(n)).map(|n| n as u16) else {
            return Err(p.bad(format!("`ppem`: {v} is not a whole pixel size from {MIN_PPEM} to {MAX_PPEM}")));
        };
        if !out.contains(&size) {
            out.push(size);
        }
    }
    Ok(out)
}

/// The glyph ids to render: `glyphs` by name, then the characters of `text` through the
/// compiled font's character map.
fn glyph_ids(p: &Params, source: &Font, built: &Built, text_key_required: bool) -> Result<Vec<(String, u32)>> {
    let mut out = Vec::new();
    if !text_key_required {
        for name in p.opt_glyphs("glyphs", source)?.unwrap_or_default() {
            let gid = built.glyph_order.iter().position(|g| *g == name).ok_or_else(|| p.bad(format!("`glyphs`: glyph `{name}` is not exported")))?;
            out.push((name, gid as u32));
        }
    }
    let text = if text_key_required { Some(p.str("text")?) } else { p.opt_str("text")? };
    if let Some(text) = text {
        let font = FontRef::new(&built.bytes).map_err(|e| EngineError::Internal(format!("the preview font cannot be read: {e}")))?;
        let cmap = font.charmap();
        for ch in text.chars().filter(|c| !c.is_control()) {
            if out.len() >= MAX_PREVIEW_GLYPHS {
                return Err(p.bad(format!("at most {MAX_PREVIEW_GLYPHS} glyphs can be previewed at once")));
            }
            let gid = cmap.map(ch).ok_or_else(|| p.bad(format!("`text`: no exported glyph has the character U+{:04X}", u32::from(ch))))?;
            let gid = gid.to_u32();
            let name = built.glyph_order.get(gid as usize).cloned().unwrap_or_default();
            out.push((name, gid));
        }
    }
    if out.is_empty() {
        let what = if text_key_required { "`text` holds no character to render" } else { "give `glyphs` or `text`" };
        return Err(p.bad(what));
    }
    if out.len() > MAX_PREVIEW_GLYPHS {
        return Err(p.bad(format!("at most {MAX_PREVIEW_GLYPHS} glyphs can be previewed at once")));
    }
    Ok(out)
}

fn intent(p: &Params) -> Result<Intent> {
    match p.opt_str("intent")? {
        None | Some("grayscale") => Ok(Intent::Grayscale),
        Some("cleartype") => Ok(Intent::GdiClearType),
        Some("directwrite") => Ok(Intent::DirectWrite),
        Some(other) => Err(p.bad(format!("`intent` must be 'grayscale', 'cleartype' or 'directwrite', got '{other}'"))),
    }
}

fn intent_name(intent: Intent) -> &'static str {
    match intent {
        Intent::Grayscale => "grayscale",
        Intent::GdiClearType => "cleartype",
        Intent::DirectWrite => "directwrite",
    }
}

fn render_err(p: &Params, e: RenderError) -> EngineError {
    p.bad(e.to_string())
}

/// The contours of `path` as lists of `[x, y, onCurve]`.
fn contours(path: &BezPath) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut current: Vec<Value> = Vec::new();
    let mut flush = |current: &mut Vec<Value>| {
        if !current.is_empty() {
            out.push(Value::Array(std::mem::take(current)));
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(q) => {
                flush(&mut current);
                current.push(json!([q.x, q.y, true]));
            }
            PathEl::LineTo(q) => current.push(json!([q.x, q.y, true])),
            PathEl::QuadTo(a, q) => current.extend([json!([a.x, a.y, false]), json!([q.x, q.y, true])]),
            PathEl::CurveTo(a, b, q) => current.extend([json!([a.x, a.y, false]), json!([b.x, b.y, false]), json!([q.x, q.y, true])]),
            PathEl::ClosePath => flush(&mut current),
        }
    }
    flush(&mut current);
    Value::Array(out)
}

fn png(p: &Params, bitmap: &HintedBitmap) -> Result<String> {
    let bytes = bitmap.image.png().map_err(|e| render_err(p, e))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn outline_json(name: &str, gid: u32, o: &HintedOutline) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("glyph".into(), json!(name));
    m.insert("gid".into(), json!(gid));
    m.insert("hinted".into(), json!(o.hinted));
    if let Some(message) = &o.message {
        m.insert("message".into(), json!(message));
    }
    m.insert("advance".into(), json!(o.advance));
    m
}

fn check_renderings(p: &Params, glyphs: usize, sizes: usize) -> Result<()> {
    if glyphs.saturating_mul(sizes) > MAX_RENDERINGS {
        return Err(p.bad(format!("{glyphs} glyphs at {sizes} sizes is more than {MAX_RENDERINGS} renderings; ask for fewer")));
    }
    Ok(())
}

fn tt_render_preview(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.renderPreview", p)?;
    let sizes = sizes(&p)?;
    let intent = intent(&p)?;
    let bitmaps = match p.opt_str("output")? {
        None | Some("outline") => false,
        Some("bitmap") => true,
        Some(other) => return Err(p.bad(format!("`output` must be 'outline' or 'bitmap', got '{other}'"))),
    };
    let built = build(s, &p, Flavour::TrueType)?;
    let glyphs = glyph_ids(&p, s.font()?, &built, false)?;
    check_renderings(&p, glyphs.len(), sizes.len())?;
    let renderer =
        HintedRenderer::new(built.bytes.clone(), intent).map_err(|e| EngineError::Internal(format!("the preview font cannot be read: {e}")))?;
    let mut out = Vec::with_capacity(sizes.len());
    for &ppem in &sizes {
        let mut rows = Vec::with_capacity(glyphs.len());
        for (name, gid) in &glyphs {
            let outline = renderer.outline(*gid, ppem).map_err(|e| render_err(&p, e))?;
            let mut m = outline_json(name, *gid, &outline);
            m.insert("instructed".into(), json!(renderer.has_instructions(*gid)));
            if bitmaps {
                let b = renderer.pixels(*gid, ppem).map_err(|e| render_err(&p, e))?;
                m.insert(
                    "bitmap".into(),
                    json!({"width": b.image.width, "height": b.image.height, "left": b.left, "top": b.top, "png": png(&p, &b)?}),
                );
            } else {
                m.insert("contours".into(), contours(&outline.path));
            }
            rows.push(Value::Object(m));
        }
        out.push(json!({"ppem": ppem, "glyphs": rows}));
    }
    Ok(json!({"instance": built.label, "format": "ttf", "intent": intent_name(intent), "sizes": out, "warnings": built.warnings}))
}

fn ps_render_preview(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.renderPreview", p)?;
    let sizes = sizes(&p)?;
    let hinted = p.bool_or("hinted", true)?;
    let as_json = match p.opt_str("format")? {
        None | Some("png") => false,
        Some("json") => true,
        Some(other) => return Err(p.bad(format!("`format` must be 'png' or 'json', got '{other}'"))),
    };
    let built = build(s, &p, Flavour::Cff)?;
    let glyphs = glyph_ids(&p, s.font()?, &built, true)?;
    check_renderings(&p, glyphs.len(), sizes.len())?;
    let renderer = HintedRenderer::new(built.bytes.clone(), Intent::Grayscale)
        .map_err(|e| EngineError::Internal(format!("the preview font cannot be read: {e}")))?;
    let renderer = if hinted { renderer } else { renderer.without_hinting() };
    let ids: Vec<u32> = glyphs.iter().map(|(_, gid)| *gid).collect();
    let mut out = Vec::with_capacity(sizes.len());
    for &ppem in &sizes {
        let line = renderer.line(&ids, ppem).map_err(|e| render_err(&p, e))?;
        let b = &line.bitmap;
        let mut m = Map::new();
        m.insert("ppem".into(), json!(ppem));
        m.insert("width".into(), json!(b.image.width));
        m.insert("height".into(), json!(b.image.height));
        m.insert("left".into(), json!(b.left));
        m.insert("top".into(), json!(b.top));
        if as_json {
            let rows: Vec<Value> = glyphs
                .iter()
                .zip(&line.glyphs)
                .map(|((name, gid), o)| {
                    let mut g = outline_json(name, *gid, o);
                    g.insert("contours".into(), contours(&o.path));
                    Value::Object(g)
                })
                .collect();
            m.insert("glyphs".into(), Value::Array(rows));
        } else {
            m.insert("png".into(), json!(png(&p, b)?));
        }
        out.push(Value::Object(m));
    }
    Ok(json!({"instance": built.label, "format": "otf", "hinted": hinted, "sizes": out, "warnings": built.warnings}))
}
