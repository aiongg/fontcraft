//! Helpers that commands of several areas share: hint node references, moving a layer
//! sideways, the kerning parameter readers, path parameter checks, finding an open document,
//! aligning an imported font and the clock.

use fontcraft_model::guide_follow::{self, BothGuides, Reference};
use fontcraft_model::plist::{Num, Number, Str, Value as Plist};
use fontcraft_model::{Font, Guide, Hint, Layer, LayerSide, MAX_COORDINATE, Shape, Transform};
use fontcraft_spacing::kerning::Direction;
use fontcraft_spacing::metrics_source;

use super::common::in_range;
use crate::params::Params;
use crate::{Result, Session};

// ── Hints ────────────────────────────────────────────────────────────────────────────────────

/// Most shapes one decomposed layer may hold (the compiler's limit on components per glyph).
/// Also the most components decomposing one layer may visit, nested ones included.
pub(crate) const MAX_LAYER_SHAPES: usize = 65_535;

/// The hint keys that hold node references: `(path, node)`, or `(path, node, path, node)` for an
/// intersection.
const NODE_REF_KEYS: [&str; 4] = ["origin", "target", "other1", "other2"];

/// Where the shape indices sit in a node reference of `len` numbers.
fn shape_slots(len: usize) -> &'static [usize] {
    match len {
        2 => &[0],
        4 => &[0, 2],
        _ => &[],
    }
}

/// The numbers of a node reference: a list (format version 3) or a `{a, b}` string (version 2).
fn ref_numbers(v: &Plist) -> Option<Vec<usize>> {
    let index = |f: f64| (f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f <= f64::from(u32::MAX)).then_some(f as usize);
    match v {
        Plist::Array(a) if a.items.len() <= 4 => a.items.iter().map(|i| i.as_num().map(Num::as_f64).and_then(index)).collect(),
        Plist::String(s) => {
            let inner = s.as_str().trim().strip_prefix('{')?.strip_suffix('}')?;
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() > 4 {
                return None;
            }
            parts.iter().map(|p| p.trim().parse::<f64>().ok().and_then(index)).collect()
        }
        _ => None,
    }
}

/// The shape indices `hint` points at.
pub(crate) fn hint_shapes(hint: &Hint) -> Vec<usize> {
    let Some(d) = hint.value.as_dict() else { return Vec::new() };
    let mut out = Vec::new();
    for key in NODE_REF_KEYS {
        if let Some(nums) = d.get(key).and_then(ref_numbers) {
            out.extend(shape_slots(nums.len()).iter().filter_map(|&i| nums.get(i).copied()));
        }
    }
    out
}

/// The shapes of `shapes` (a layer side) that the node references of `hint` point at. PostScript
/// hints and TrueType instructions count paths only, so their path indices are turned into shape
/// indices; a reference past the paths comes back as `usize::MAX`.
pub(crate) fn hint_shapes_on(hint: &Hint, shapes: &[Shape]) -> Vec<usize> {
    let refs = hint_shapes(hint);
    if fontcraft_model::corner::is_corner(hint) || fontcraft_model::attach::is_attachment(hint) || !fontcraft_hinting::follows_nodes(hint, shapes) {
        return refs;
    }
    let slots: Vec<usize> = shapes.iter().enumerate().filter(|(_, sh)| matches!(sh, Shape::Path(_))).map(|(i, _)| i).collect();
    refs.into_iter().map(|p| slots.get(p).copied().unwrap_or(usize::MAX)).collect()
}

/// `hint` with each shape index it points at replaced by what `map` gives; `None` when `map`
/// drops one of them. A hint without node references comes back unchanged.
pub(crate) fn remap_hint(hint: &Hint, map: &impl Fn(usize) -> Option<usize>) -> Option<Hint> {
    let mut out = hint.clone();
    let Plist::Dict(d) = &mut out.value else { return Some(out) };
    for key in NODE_REF_KEYS {
        let Some(v) = d.get_mut(key) else { continue };
        let Some(nums) = ref_numbers(v) else { continue };
        let mut new = nums.clone();
        for &slot in shape_slots(nums.len()) {
            if let Some(n) = new.get_mut(slot) {
                *n = map(*n)?;
            }
        }
        if new != nums {
            write_ref(v, &nums, &new);
        }
    }
    Some(out)
}

/// Keeps the hints of `side` of `layer` (glyph `glyph`) on their nodes after an edit that replaced
/// some shapes of that side by others and kept the rest as they were. `before` is the layer before
/// the edit; `layer` still holds the side's hints as they were. `map` gives the new index of each
/// old shape, `None` for a replaced one. Hints that `fontcraft_hinting` follows go through
/// [`fontcraft_hinting::renumber_side`]: on a replaced path a PostScript end becomes a number, and
/// a TrueType instruction or corner component is removed and reported in `report`. Other hints
/// get their shape indices renumbered, and are dropped when their shape was replaced.
pub(crate) fn renumber_hints(
    before: &Layer,
    layer: &mut Layer,
    side: LayerSide,
    glyph: &str,
    map: &impl Fn(usize) -> Option<usize>,
    report: &mut fontcraft_hinting::RemapReport,
) {
    let was = before.parts(side).shapes;
    let hints = match side {
        LayerSide::Foreground => &before.info.hints,
        LayerSide::Background => match before.info.background.as_ref() {
            Some(b) => &b.hints,
            None => return,
        },
    };
    if hints.is_empty() {
        return;
    }
    let kept: Vec<Hint> =
        hints.iter().filter_map(|h| if fontcraft_hinting::follows_nodes(h, was) { Some(h.clone()) } else { remap_hint(h, map) }).collect();
    // The new path index of each new shape, then of each old path.
    let mut count = 0usize;
    let new_paths: Vec<Option<usize>> = layer
        .parts(side)
        .shapes
        .iter()
        .map(|sh| {
            matches!(sh, Shape::Path(_)).then(|| {
                count += 1;
                count - 1
            })
        })
        .collect();
    let paths: Vec<Option<usize>> = was
        .iter()
        .enumerate()
        .filter(|(_, sh)| matches!(sh, Shape::Path(_)))
        .map(|(i, _)| map(i).and_then(|j| new_paths.get(j).copied().flatten()))
        .collect();
    let set = |l: &mut Layer, hints: Vec<Hint>| match side {
        LayerSide::Foreground => l.info.hints = hints,
        LayerSide::Background => {
            if let Some(b) = l.info.background.as_mut() {
                b.hints = hints;
            }
        }
    };
    // `renumber_side` reads the hints as both layers hold them.
    let adjusted;
    let before = if kept == *hints {
        before
    } else {
        let mut b = before.clone();
        set(layer, kept.clone());
        set(&mut b, kept);
        adjusted = b;
        &adjusted
    };
    if let Some(out) = fontcraft_hinting::renumber_side(before, layer, side, glyph, &paths, report) {
        *layer = out;
    }
}

/// Writes `new` into the node reference `v`, which held `old`, in its own syntax.
fn write_ref(v: &mut Plist, old: &[usize], new: &[usize]) {
    let int = |n: usize| Plist::Number(Number::int(i64::try_from(n).unwrap_or(i64::MAX)));
    match v {
        Plist::Array(a) => {
            for ((slot, &was), &now) in a.items.iter_mut().zip(old).zip(new) {
                if was != now {
                    *slot = int(now);
                }
            }
        }
        Plist::String(s) => {
            let parts: Vec<String> = new.iter().map(usize::to_string).collect();
            *s = Str::new(format!("{{{}}}", parts.join(", ")));
        }
        _ => {}
    }
}

// ── Moving a layer ───────────────────────────────────────────────────────────────────────────

/// Moves everything drawn in a layer sideways by `dx`: outlines, components, images, anchors,
/// guides, annotations, the placed image and the background. Hints stay as they are. Fails when a
/// coordinate would leave the model's range or a shape cannot be moved, and then leaves the
/// layer as it was. `both` says whether guides referenced to both sidebearings move too.
pub(crate) fn shift_layer(layer: &mut Layer, dx: f64, both: BothGuides) -> std::result::Result<(), String> {
    shift(layer, dx, |r| r == Reference::Both && both == BothGuides::Keep)
}

/// Moves a layer as [`shift_layer`] does for an edit that moves the right edge as well: a guide
/// referenced to the right sidebearing keeps its distance from that edge, so its stored x stays
/// (`guide_follow`).
pub(crate) fn shift_with_right_edge(layer: &mut Layer, dx: f64, both: BothGuides) -> std::result::Result<(), String> {
    shift(layer, dx, |r| r == Reference::Right || (r == Reference::Both && both == BothGuides::Keep))
}

/// `kept` names the references of the guides whose stored x stays.
fn shift(layer: &mut Layer, dx: f64, kept: impl Fn(Reference) -> bool) -> std::result::Result<(), String> {
    if dx == 0.0 {
        return Ok(());
    }
    let mut moved = layer.clone();
    shift_in_place(&mut moved, dx, &kept)?;
    *layer = moved;
    Ok(())
}

fn shift_guides(guides: &mut [Guide], dx: f64, kept: &impl Fn(Reference) -> bool) -> std::result::Result<(), String> {
    for g in guides.iter_mut().filter(|g| !kept(guide_follow::reference(g))) {
        shift_x(&mut g.x, dx)?;
    }
    Ok(())
}

fn shift_in_place(layer: &mut Layer, dx: f64, kept: &impl Fn(Reference) -> bool) -> std::result::Result<(), String> {
    shift_shapes(&mut layer.shapes, dx)?;
    for a in &mut layer.anchors {
        shift_x(&mut a.x, dx)?;
    }
    if layer.info.is_default() {
        return Ok(());
    }
    let info = &mut *layer.info;
    shift_guides(&mut info.guides, dx, kept)?;
    for a in &mut info.annotations {
        shift_x(&mut a.x, dx)?;
    }
    if let Some(img) = &mut info.image {
        shift_transform(&mut img.transform, dx)?;
    }
    if let Some(b) = &mut info.background {
        shift_shapes(&mut b.shapes, dx)?;
        for a in &mut b.anchors {
            shift_x(&mut a.x, dx)?;
        }
        shift_guides(&mut b.guides, dx, kept)?;
        for a in &mut b.annotations {
            shift_x(&mut a.x, dx)?;
        }
        if let Some(img) = &mut b.image {
            shift_transform(&mut img.transform, dx)?;
        }
    }
    Ok(())
}

fn shift_shapes(shapes: &mut [Shape], dx: f64) -> std::result::Result<(), String> {
    for shape in shapes {
        match shape {
            Shape::Path(path) => {
                for n in &mut path.nodes {
                    shift_x(&mut n.x, dx)?;
                }
            }
            Shape::Component(c) => shift_transform(&mut c.transform, dx)?,
            Shape::Image(img) => shift_transform(&mut img.transform, dx)?,
            // Its points are not in the model, so it cannot follow the rest.
            Shape::Raw(_) => return Err("the layer holds a shape of a kind that cannot be moved yet".into()),
        }
    }
    Ok(())
}

fn shift_transform(t: &mut Transform, dx: f64) -> std::result::Result<(), String> {
    match t {
        Transform::Decomposed { pos, .. } => shift_x(&mut pos.0, dx),
        Transform::Matrix([.., x, _]) => shift_x(x, dx),
    }
}

fn shift_x(x: &mut f64, dx: f64) -> std::result::Result<(), String> {
    let v = *x + dx;
    if !in_range(v) {
        return Err(format!("the change puts a point at x = {v}, beyond ±{MAX_COORDINATE}"));
    }
    *x = v;
    Ok(())
}

// ── Kerning parameters ───────────────────────────────────────────────────────────────────────

/// Largest kerning value a command stores, either sign: what a compiled pair value can hold.
pub(crate) const MAX_KERN_VALUE: f64 = 32_767.0;

/// `direction`, `ltr` when absent.
pub(crate) fn direction(p: &Params) -> Result<Direction> {
    match p.opt_str("direction")? {
        None => Ok(Direction::Ltr),
        Some(name) => {
            Direction::from_name(name).ok_or_else(|| p.bad(format!("`direction` must be \"ltr\", \"rtl\" or \"vertical\", got \"{name}\"")))
        }
    }
}

/// `master`, the first master when absent.
pub(crate) fn master(p: &Params, font: &Font) -> Result<String> {
    match p.opt_str("master")? {
        Some(id) => font.master(id).map(|m| m.id.clone()).map_err(|_| p.bad(format!("the font has no master `{id}`"))),
        None => Ok(font.default_master()?.id.clone()),
    }
}

/// [`master`], refused when the master takes its kerning from another one.
pub(crate) fn editable_master(p: &Params, font: &Font) -> Result<String> {
    let id = master(p, font)?;
    match metrics_source(font, &id) {
        Some(source) if source.id != id => {
            Err(p.bad(format!("master `{id}` takes its metrics and kerning from master `{}` ({}); edit that master instead", source.id, source.name)))
        }
        _ => Ok(id),
    }
}

// ── Paths ────────────────────────────────────────────────────────────────────────────────────

/// Most bytes in a referenced file path.
const MAX_PATH_BYTES: usize = 4096;

/// Refuses a file path parameter `key` that is too long or holds a control character.
pub(crate) fn check_path_text(p: &Params, key: &str, path: &str) -> Result<()> {
    if path.len() > MAX_PATH_BYTES {
        return Err(p.bad(format!("`{key}` is longer than {MAX_PATH_BYTES} bytes")));
    }
    if path.chars().any(char::is_control) {
        return Err(p.bad(format!("`{key}` holds a control character")));
    }
    Ok(())
}

/// The folder includes are read from: the source's folder, when automation may read it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn source_feature_dir(path: &std::path::Path) -> Option<std::path::PathBuf> {
    crate::io::feature_dir(path).filter(|d| crate::file_access::check_read(d).is_ok())
}

/// The web build has no file system to read includes from.
#[cfg(target_arch = "wasm32")]
pub(crate) fn source_feature_dir(_path: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

// ── Open documents ───────────────────────────────────────────────────────────────────────────

/// The position of the open document that was opened from (or saved to) `path`, comparing the
/// paths as the file system resolves them.
pub(crate) fn find_open(s: &Session, path: &std::path::Path) -> Option<usize> {
    let want = std::fs::canonicalize(path).ok();
    s.documents().iter().position(|d| {
        d.path.as_deref().is_some_and(|p| match (&want, std::fs::canonicalize(p).ok()) {
            (Some(a), Some(b)) => *a == b,
            _ => p == path,
        })
    })
}

/// The user setting that keeps imported fonts out of automatic alignment.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) const DISABLE_ALIGNMENT_FOR_IMPORTED: &str = "user.disableAutomaticAlignmentForImportedFiles";

/// What opening an imported source (UFO, designspace) does to its composites
/// (`open.alignsCompositeMetrics`, `components.importedFilesDisableAutoAlignSetting`): with the
/// user setting Disable Automatic Alignment for Imported Files on, the font's own Disable
/// Automatic Alignment option is switched on, so its components stay where the file puts them;
/// otherwise every composite is aligned, so its metrics follow its bases. A font whose own option
/// is already on is left as read. `.fontcraft` and `.glyphs` sources store aligned positions and
/// are not touched; nor is a compiled font, which is view-only and has no anchors to align by. Alignment that fails, or gives a font that does not pass the model's checks,
/// leaves the font as read with a warning. Returns the font and how many glyphs alignment changed.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn align_imported(s: &Session, mut font: Font, warnings: &mut Vec<String>) -> (Font, usize) {
    use fontcraft_model::SourceFormat;
    if !matches!(font.source.format, SourceFormat::Ufo | SourceFormat::Ufoz | SourceFormat::Designspace) {
        return (font, 0);
    }
    if s.settings().get_bool(DISABLE_ALIGNMENT_FOR_IMPORTED) == Some(true) {
        if font.info.settings.disables_automatic_alignment != Some(true) {
            std::sync::Arc::make_mut(&mut font.info).settings.disables_automatic_alignment = Some(true);
        }
        return (font, 0);
    }
    let before = font.clone();
    let names: Vec<String> = font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
    let aligned = match fontcraft_components::metrics::realign_glyphs(font, &names) {
        Ok(aligned) => aligned,
        Err(e) => {
            warnings.push(format!("the composites were left as the file places them: {e}"));
            return (before, 0);
        }
    };
    if let Err(e) = aligned.check_changed(&before) {
        warnings.push(format!("the composites were left as the file places them: {e}"));
        return (before, 0);
    }
    let changed = aligned.glyphs.as_arcs().iter().zip(before.glyphs.as_arcs()).filter(|(a, b)| !std::sync::Arc::ptr_eq(a, b)).count();
    (aligned, changed)
}

/// The head-table time when the caller gives none: `SOURCE_DATE_EPOCH` for reproducible builds,
/// else the clock.
pub(crate) fn default_timestamp() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(t) = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.trim().parse::<i64>().ok()) {
            return t;
        }
    }
    clock()
}

/// Seconds since the Unix epoch (0 on the web, where the shell passes the time).
pub(crate) fn clock() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}
