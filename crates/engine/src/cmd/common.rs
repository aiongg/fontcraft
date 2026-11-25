//! Helpers shared by several command modules: the grid step, the coordinate range check, a
//! master's metrics, code-point lists, and the glyph-or-glyphs scope that whole-layer commands
//! use.

use std::collections::HashSet;

use fontcraft_model::{Font, Layer, MAX_COORDINATE, MAX_UNICODES, Master, Metrics, metric};
use serde_json::{Value, json};

use crate::EngineError;
use crate::Result;
use crate::params::{Params, codepoint, kind};

/// Most layer ids one `layers` list may hold.
const MAX_LAYER_IDS: usize = 1024;

/// The font's grid step: spacing ÷ subdivision, spacing 1 when the font sets none, 0 for no
/// rounding (`fontInfo.grid.*`).
pub(crate) fn grid(font: &Font) -> f64 {
    let s = &font.info.settings;
    fontcraft_geom::grid_step(s.grid_length.unwrap_or(1.0), s.grid_sub_division.unwrap_or(1.0))
}

/// Whether a coordinate is finite and within the model's limit.
pub(crate) fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// The master's metrics, with the format's values where the font has none.
pub(crate) fn master_metrics(font: &Font, master: &Master) -> Metrics {
    let d = Metrics::defaults(font.source.format, font.units_per_em);
    Metrics {
        ascender: font.metric_pos(master, metric::ASCENDER, d.ascender),
        descender: font.metric_pos(master, metric::DESCENDER, d.descender),
        x_height: font.metric_pos(master, metric::X_HEIGHT, d.x_height),
        cap_height: font.metric_pos(master, metric::CAP_HEIGHT, d.cap_height),
        italic_angle: font.metric_pos(master, metric::ITALIC_ANGLE, d.italic_angle),
    }
}

/// The glyphs and layers a whole-layer command acts on.
pub(crate) struct Scope {
    pub(crate) glyphs: Vec<String>,
    pub(crate) layers: Vec<String>,
    /// One `glyph` was given: a locked glyph or a missing layer is an error, not a skip.
    pub(crate) single: bool,
}

/// What a whole-layer command did.
#[derive(Default)]
pub(crate) struct Outcome {
    pub(crate) glyphs: Vec<String>,
    pub(crate) skipped: Vec<Value>,
}

impl Outcome {
    pub(crate) fn skip(&mut self, glyph: &str, layer: Option<&str>, reason: impl Into<String>) {
        let mut v = json!({"glyph": glyph, "reason": reason.into()});
        if let (Some(layer), Some(o)) = (layer, v.as_object_mut()) {
            o.insert("layer".into(), json!(layer));
        }
        self.skipped.push(v);
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({"glyphs": self.glyphs, "skipped": self.skipped})
    }
}

/// Reads `glyph` or `glyphs`, and `layer` or `layers`.
pub(crate) fn scope(p: &Params, font: &Font) -> Result<Scope> {
    let (glyphs, single) = match (p.get("glyph"), p.get("glyphs")) {
        (Some(_), Some(_)) => return Err(p.bad("give `glyph` or `glyphs`, not both")),
        (Some(_), None) => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            (vec![name.to_string()], true)
        }
        (None, _) => {
            let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyph` or `glyphs` is required"))?;
            if glyphs.is_empty() {
                return Err(p.bad("`glyphs` is empty"));
            }
            (glyphs, false)
        }
    };
    Ok(Scope { glyphs, layers: layer_ids(p, font)?, single })
}

/// `layers` or `layer`; the first master's layer when neither is given.
pub(crate) fn layer_ids(p: &Params, font: &Font) -> Result<Vec<String>> {
    match (p.get("layer"), p.get("layers")) {
        (Some(_), Some(_)) => Err(p.bad("give `layer` or `layers`, not both")),
        (Some(_), None) => Ok(vec![p.str("layer")?.to_string()]),
        (None, Some(Value::Array(a))) if a.len() > MAX_LAYER_IDS => {
            Err(p.bad(format!("`layers` holds {} ids; at most {MAX_LAYER_IDS} are allowed", a.len())))
        }
        (None, Some(_)) => {
            let mut seen = HashSet::new();
            let ids: Vec<String> = p.strings("layers")?.into_iter().filter(|id| seen.insert(*id)).map(str::to_string).collect();
            if ids.is_empty() {
                return Err(p.bad("`layers` is empty"));
            }
            Ok(ids)
        }
        (None, None) => Ok(vec![font.default_master()?.id.clone()]),
    }
}

/// Runs `f` on every addressed layer of every glyph in `scope`. `f` returns `Some(reason)` when
/// it leaves a layer as it is. Call inside [`Session::edit`].
pub(crate) fn each_layer(
    font: &mut Font,
    scope: &Scope,
    command: &'static str,
    mut f: impl FnMut(&str, &mut Layer) -> Result<Option<String>>,
) -> Result<Outcome> {
    let bad = |message: String| EngineError::BadParams { command: command.into(), message };
    let mut out = Outcome::default();
    for name in &scope.glyphs {
        let Some(glyph) = font.glyph(name) else {
            return Err(bad(format!("there is no glyph `{name}`")));
        };
        if glyph.info.locked {
            if scope.single {
                return Err(EngineError::Locked(name.clone()));
            }
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        let missing: Vec<&String> = scope.layers.iter().filter(|id| !glyph.layers.iter().any(|l| l.id == **id)).collect();
        if let Some(id) = missing.first()
            && scope.single
        {
            return Err(bad(format!("glyph `{name}` has no layer `{id}`")));
        }
        for id in missing {
            out.skip(name, Some(id.as_str()), "the glyph has no such layer");
        }
        let mut g = font.glyph_mut(name)?;
        let mut changed = false;
        for layer in g.layers.iter_mut().filter(|l| scope.layers.contains(&l.id)) {
            match f(name, layer)? {
                Some(reason) => out.skip(name, Some(layer.id.as_str()), reason),
                None => changed = true,
            }
        }
        if changed {
            out.glyphs.push(name.clone());
        }
    }
    Ok(out)
}

/// A list of code points (hex text or numbers), repeats dropped, at most [`MAX_UNICODES`];
/// `None` when `key` is absent.
pub(crate) fn code_points(p: &Params, key: &str) -> Result<Option<Vec<u32>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_UNICODES {
        return Err(p.bad(format!("`{key}` holds {} values; at most {MAX_UNICODES} are allowed", list.len())));
    }
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let cp = codepoint(v).ok_or_else(|| p.bad(format!("`{key}` must hold code points as hex text or numbers, found {} ({v})", kind(v))))?;
        if !out.contains(&cp) {
            out.push(cp);
        }
    }
    Ok(Some(out))
}
