//! Changes another program made to an open document's files (`file.reviewExternalChanges`,
//! `io.watchSourceOnDisk`).
//!
//! The file is read again as `file.open` reads it and compared three ways by model equality: the
//! document, the font as the document last read or wrote it (its `saved` font, the base) and the
//! file. An item is listed when the file changed it since the base and the document does not
//! already have it as the file does, glyph by glyph and layer by layer, plus the font-level parts
//! in [`PARTS`]. What only the document changed is never listed, so taking every change keeps the
//! document's own edits. Accepted items are taken from the file as one undo step; the rest stay as
//! the document has them.
//!
//! The document's stamp (what its files looked like when last read or written here) decides
//! whether `file.save` asks before overwriting. A review that leaves nothing listed out (nothing
//! listed, or every item accepted) refreshes the stamp and makes the file the new base, so the
//! next save writes without asking. After a refusal the stamp stays as it was, so `file.save` refuses until
//! `force: true`: saving would overwrite what the other program wrote.
//!
//! Watching ([`changed_on_disk`]) compares stamps only (file count, sizes, newest modification
//! time) and reads no file. The shell calls it when its window regains focus.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, GlyphTable, Layer};
use serde_json::{Value, json};

use super::{CommandSpec, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most items one `accept` list may name.
pub const MAX_ACCEPT_ITEMS: usize = 500_000;
/// Most lines on each side of the part of a glyph's text that differs, for the line difference;
/// longer texts list the whole differing part as removed, then added.
pub const MAX_DIFF_LINES: usize = 1_500;
/// Most lines a glyph's text difference returns.
pub const MAX_TEXT_LINES: usize = 20_000;

/// The font-level parts compared besides the glyphs, in the order they are listed.
pub const PARTS: [&str; 9] = ["info", "axes", "masters", "instances", "kerning", "features", "groups", "glyphOrder", "other"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.reviewExternalChanges",
        "Review External Changes",
        [],
        None,
        "{font?: number | string, accept?: [{glyph: name, layer?: id, attributes?: bool} | {part: name}] | \"all\" | \"none\", glyph?: name} read the file of an open document again (font: its position among the open documents or the path it was opened from; default the active one) and list what the file changed since the document last read or wrote it, where the document does not already match: glyphs added, removed or changed (by layer, and attributes for the glyph's own data apart from its layers) and the font-level parts info, axes, masters, instances, kerning, features, groups, glyphOrder and other. What only the document changed is not listed; conflict marks what both changed. Without accept it only reports. accept takes the named items from the file as one undo step and keeps the document's version of the rest: {glyph} every listed item of the glyph, {glyph, layer} one layer, {glyph, attributes: true} the glyph's own data, {part} a font-level part; \"all\" takes everything, \"none\" nothing. A locked glyph refuses an accepted change. When nothing listed is left out, a later save writes without asking; after a refusal, file.save refuses until force: true. glyph: also return that glyph's stored text as a line difference, the file's lines as additions and the document's as removals → {index, title, path, modified, changed, glyphs: [{glyph, change: \"added\"|\"removed\"|\"changed\", locked, conflict, attributes, layers: [{layer, name, change}]}], parts: [{part, conflict}], accepted?, rejected?, saveNeedsForce, text?: [{op: \" \"|\"-\"|\"+\", text}] (at most 20000 lines), textTruncated?: bool (lines past 20000 left out), warnings: [text]}",
        has_file,
        review
    )]
}

fn has_file(s: &Session) -> std::result::Result<(), String> {
    if s.documents().iter().any(|d| d.path.is_some()) { Ok(()) } else { Err("no open document has a file".into()) }
}

/// An open document whose files changed on disk since they were last read or written here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskChange {
    /// The document's id ([`crate::DocState::id`]).
    pub doc: u64,
    /// Its position among the open documents when the check ran.
    pub index: usize,
    pub title: String,
    /// The document has unsaved edits.
    pub modified: bool,
    /// What the files look like now; the same change compares equal on a later check.
    #[cfg(not(target_arch = "wasm32"))]
    stamp: Option<crate::io::Stamp>,
}

/// The open documents whose files changed on disk since they were last read or written here, by
/// their stamps alone (no file is read). A document whose file is gone is not listed. The web
/// app has no files: nothing is listed there.
#[cfg(not(target_arch = "wasm32"))]
pub fn changed_on_disk(s: &Session) -> Vec<DiskChange> {
    s.documents()
        .iter()
        .enumerate()
        .filter_map(|(index, d)| {
            let path = d.path.as_deref()?;
            let now = crate::io::document_stamp(path, &d.font);
            (now.is_some() && now != d.stamp).then(|| DiskChange { doc: d.id, index, title: d.title.clone(), modified: d.modified(), stamp: now })
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
pub fn changed_on_disk(s: &Session) -> Vec<DiskChange> {
    let _ = s;
    Vec::new()
}

#[cfg(target_arch = "wasm32")]
fn review(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("reviewing changes on disk in the browser, which has no files".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn review(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.reviewExternalChanges", v)?;
    let i = target(s, &p)?;
    let doc = s.documents().get(i).ok_or(EngineError::NoDocument)?;
    let path = doc.path.clone().ok_or_else(|| p.bad(format!("{} has no file on disk: save it first", doc.title)))?;
    // Taken before the read: a change made while reading shows on the next check.
    let stamp = crate::io::document_stamp(&path, &doc.font);
    let disk = read_disk(s, &path)?;
    let doc = s.documents().get(i).ok_or(EngineError::NoDocument)?;
    let (title, was_modified) = (doc.title.clone(), doc.modified());
    let (base, cur) = (doc.saved.clone(), doc.font.clone());
    let diff = compare(&base, &cur, &disk);
    let text = match p.opt_str("glyph")? {
        Some(name) => Some(glyph_text(&p, &cur, &disk, name)?),
        None => None,
    };
    let accept = parse_accept(&p, &diff)?;
    let (accepted, rejected) = match &accept {
        Some(picks) => {
            let next = merge(&cur, &disk, &diff, picks);
            replace_font(s, i, "Accept External Changes", next)?;
            let accepted = picks.count(&diff);
            (accepted, diff.items().saturating_sub(accepted))
        }
        None => (0, diff.items()),
    };
    // Nothing on disk is left out of the document: saving writes the file's changes back with
    // the document's own, so it need not ask, and the file becomes the base of the next review.
    let merged = diff.is_empty() || (accept.is_some() && rejected == 0);
    if merged && let Some(doc) = s.docs_mut().get_mut(i) {
        doc.stamp = stamp;
        doc.saved = if doc.font.same_as(&disk) { doc.font.clone() } else { Arc::new(disk.clone()) };
    }
    let mut out = json!({
        "index": i, "title": title, "path": path.to_string_lossy(), "modified": was_modified, "changed": !diff.is_empty(),
        "glyphs": diff.glyphs.iter().map(|g| g.to_json(&cur, &disk)).collect::<Vec<_>>(),
        "parts": diff.parts.iter().map(|part| json!({ "part": part, "conflict": diff.part_conflicts.contains(part) })).collect::<Vec<_>>(),
        "saveNeedsForce": !merged,
    });
    if let Some(o) = out.as_object_mut() {
        if accept.is_some() {
            o.insert("accepted".into(), json!(accepted));
            o.insert("rejected".into(), json!(rejected));
        }
        let mut warnings = Vec::new();
        if let Some((text, cut)) = text {
            o.insert("text".into(), Value::Array(text.into_iter().map(|(op, line)| json!({ "op": op.to_string(), "text": line })).collect()));
            o.insert("textTruncated".into(), json!(cut));
            if cut {
                warnings.push(format!("the text difference is longer than {MAX_TEXT_LINES} lines; only the first {MAX_TEXT_LINES} are listed"));
            }
        }
        o.insert("warnings".into(), json!(warnings));
    }
    Ok(out)
}

/// The position of the document `font` names: an index, the path it was opened from, or the
/// active document.
fn target(s: &Session, p: &Params) -> Result<usize> {
    let count = s.documents().len();
    match p.get("font") {
        None | Some(Value::Null) => {
            let id = s.doc()?.id;
            s.position_of(id).ok_or(EngineError::NoDocument)
        }
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < count)
            .ok_or_else(|| p.bad(format!("`font` {n} is not an open document; {count} are open"))),
        Some(Value::String(path)) => {
            super::shared::find_open(s, std::path::Path::new(path)).ok_or_else(|| p.bad(format!("no open document was opened from {path}")))
        }
        Some(_) => Err(p.bad("`font` must be the position of an open document or the path it was opened from")),
    }
}

/// The file as `file.open` would give it now.
#[cfg(not(target_arch = "wasm32"))]
fn read_disk(s: &Session, path: &std::path::Path) -> Result<Font> {
    crate::file_access::check_read(path).map_err(EngineError::Io)?;
    let (font, mut warnings) = crate::io::open(path, None)?;
    let font = crate::linked::resolve(&font, Some(path), &mut warnings)?.unwrap_or(font);
    let (font, _) = super::shared::align_imported(s, font, &mut warnings);
    Ok(font)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// Only the file has it.
    Added,
    /// Only the document has it.
    Removed,
    Changed,
}

impl Change {
    fn name(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Removed => "removed",
            Change::Changed => "changed",
        }
    }
}

/// How one glyph differs.
#[derive(Debug)]
struct GlyphDiff {
    name: String,
    change: Change,
    /// The glyph's own data apart from its layers differs (layer order included).
    attributes: bool,
    /// By layer id; empty for an added or removed glyph.
    layers: Vec<(String, Change)>,
    /// The document's glyph is locked.
    locked: bool,
    /// The document changed this glyph too since it was last read or written.
    conflict: bool,
}

impl GlyphDiff {
    /// The reviewable items: the whole glyph when it was added or removed, else its attributes
    /// and each layer.
    fn items(&self) -> usize {
        match self.change {
            Change::Added | Change::Removed => 1,
            Change::Changed => usize::from(self.attributes) + self.layers.len(),
        }
    }

    fn to_json(&self, cur: &Font, disk: &Font) -> Value {
        let layers: Vec<Value> = self
            .layers
            .iter()
            .map(|(id, change)| json!({ "layer": id, "name": layer_name(cur, disk, &self.name, id), "change": change.name() }))
            .collect();
        json!({
            "glyph": self.name, "change": self.change.name(), "locked": self.locked, "conflict": self.conflict, "attributes": self.attributes, "layers": layers,
        })
    }
}

/// What differs between the document and the file.
#[derive(Debug, Default)]
struct Diff {
    /// The document's glyphs in its order, then those only the file has, in the file's order.
    glyphs: Vec<GlyphDiff>,
    parts: Vec<&'static str>,
    /// The listed parts the document changed too.
    part_conflicts: Vec<&'static str>,
}

impl Diff {
    fn is_empty(&self) -> bool {
        self.glyphs.is_empty() && self.parts.is_empty()
    }

    fn items(&self) -> usize {
        self.glyphs.iter().map(GlyphDiff::items).sum::<usize>() + self.parts.len()
    }

    fn glyph(&self, name: &str) -> Option<&GlyphDiff> {
        self.glyphs.iter().find(|g| g.name == name)
    }
}

/// The items the file changed since the document was last read or written (`base`) and that the
/// document does not already have as the file does. An item the document alone changed is not
/// listed.
fn compare(base: &Font, cur: &Font, disk: &Font) -> Diff {
    let mut diff = Diff::default();
    let mut seen = HashSet::new();
    let names = cur.glyphs.iter().chain(disk.glyphs.iter()).map(|g| g.name.as_str());
    for name in names {
        if !seen.insert(name) {
            continue;
        }
        let (b, c, d) = (base.glyphs.get(name).map(|g| &**g), cur.glyphs.get(name).map(|g| &**g), disk.glyphs.get(name).map(|g| &**g));
        if d == b || c == d {
            continue;
        }
        let entry = |change, attributes, layers| GlyphDiff {
            name: name.to_string(),
            change,
            attributes,
            layers,
            locked: c.is_some_and(|g| g.info.locked),
            conflict: c != b,
        };
        match (c, d) {
            (None, Some(_)) => diff.glyphs.push(entry(Change::Added, false, Vec::new())),
            (Some(_), None) => diff.glyphs.push(entry(Change::Removed, false, Vec::new())),
            (Some(cg), Some(dg)) => {
                let attributes = !same_attributes(d, b) && !same_attributes(c, d);
                let layers = layer_changes(b, cg, dg);
                if attributes || !layers.is_empty() {
                    diff.glyphs.push(entry(Change::Changed, attributes, layers));
                }
            }
            (None, None) => {}
        }
    }
    diff.parts = PARTS.iter().copied().filter(|part| part_differs(disk, base, part) && part_differs(cur, disk, part)).collect();
    diff.part_conflicts = diff.parts.iter().copied().filter(|part| part_differs(cur, base, part)).collect();
    diff
}

/// The glyphs' own data, and the order of the layers both have; a glyph only one side has differs.
fn same_attributes(a: Option<&Glyph>, b: Option<&Glyph>) -> bool {
    let (a, b) = match (a, b) {
        (None, None) => return true,
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    let common =
        |x: &Glyph, y: &Glyph| -> Vec<String> { x.layers.iter().filter(|l| y.layers.iter().any(|m| m.id == l.id)).map(|l| l.id.clone()).collect() };
    a.unicodes == b.unicodes
        && a.export == b.export
        && a.kern_groups == b.kern_groups
        && a.info == b.info
        && a.file_name == b.file_name
        && a.extra == b.extra
        && common(a, b) == common(b, a)
}

/// The layers the file changed since `base` that the document does not have as the file does, by
/// id, with how the document's layer differs from the file's.
fn layer_changes(base: Option<&Glyph>, cur: &Glyph, disk: &Glyph) -> Vec<(String, Change)> {
    fn find<'a>(g: Option<&'a Glyph>, id: &str) -> Option<&'a Layer> {
        g.and_then(|g| g.layers.iter().find(|l| l.id == id))
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in cur.layers.iter().chain(disk.layers.iter()).map(|l| l.id.as_str()) {
        if !seen.insert(id) {
            continue;
        }
        let (b, c, d) = (find(base, id), find(Some(cur), id), find(Some(disk), id));
        if d == b || c == d {
            continue;
        }
        let change = match (c, d) {
            (None, _) => Change::Added,
            (_, None) => Change::Removed,
            _ => Change::Changed,
        };
        out.push((id.to_string(), change));
    }
    out
}

/// A layer's name as the editor shows it: its master's name for a master layer, else the name it
/// stores, else its id.
fn layer_name(cur: &Font, disk: &Font, glyph: &str, id: &str) -> String {
    let layer = |f: &Font| -> Option<Layer> { f.glyphs.get(glyph).and_then(|g| g.layers.iter().find(|l| l.id == id).cloned()) };
    let Some(layer) = layer(cur).or_else(|| layer(disk)) else { return id.to_string() };
    if layer.is_master_layer()
        && let Some(m) = cur.masters.iter().chain(disk.masters.iter()).find(|m| m.id == layer.id)
    {
        return m.name.clone();
    }
    layer.info.name.clone().unwrap_or_else(|| id.to_string())
}

fn part_differs(a: &Font, b: &Font, part: &str) -> bool {
    fn ne<T: PartialEq>(x: &Arc<T>, y: &Arc<T>) -> bool {
        !Arc::ptr_eq(x, y) && x != y
    }
    match part {
        "info" => {
            a.family_name != b.family_name
                || a.units_per_em != b.units_per_em
                || a.version_major != b.version_major
                || a.version_minor != b.version_minor
                || ne(&a.info, &b.info)
        }
        "axes" => ne(&a.axes, &b.axes),
        "masters" => ne(&a.masters, &b.masters) || ne(&a.metrics, &b.metrics) || ne(&a.stems, &b.stems) || ne(&a.numbers, &b.numbers),
        "instances" => ne(&a.instances, &b.instances),
        "kerning" => ne(&a.kerning, &b.kerning),
        "features" => ne(&a.features, &b.features),
        "groups" => ne(&a.groups, &b.groups),
        "glyphOrder" => common_order(a, b) != common_order(b, a),
        "other" => ne(&a.display_strings, &b.display_strings) || a.note != b.note || ne(&a.files, &b.files) || ne(&a.extra, &b.extra),
        _ => false,
    }
}

/// `a`'s glyph names that `b` also has, in `a`'s order.
fn common_order<'a>(a: &'a Font, b: &Font) -> Vec<&'a str> {
    a.glyphs.iter().map(|g| g.name.as_str()).filter(|n| b.glyphs.get(n).is_some()).collect()
}

fn take_part(next: &mut Font, disk: &Font, part: &str) {
    match part {
        "info" => {
            next.family_name = disk.family_name.clone();
            next.units_per_em = disk.units_per_em;
            next.version_major = disk.version_major;
            next.version_minor = disk.version_minor;
            next.info = disk.info.clone();
        }
        "axes" => next.axes = disk.axes.clone(),
        "masters" => {
            next.masters = disk.masters.clone();
            next.metrics = disk.metrics.clone();
            next.stems = disk.stems.clone();
            next.numbers = disk.numbers.clone();
        }
        "instances" => next.instances = disk.instances.clone(),
        "kerning" => next.kerning = disk.kerning.clone(),
        "features" => next.features = disk.features.clone(),
        "groups" => next.groups = disk.groups.clone(),
        "glyphOrder" => {
            // The file's order for the glyphs it has; the document's own glyphs follow in theirs.
            let mut placed = HashSet::new();
            let mut order: Vec<Arc<Glyph>> = Vec::with_capacity(next.glyphs.len());
            for d in disk.glyphs.iter() {
                if let Some(g) = next.glyphs.get(&d.name)
                    && placed.insert(d.name.clone())
                {
                    order.push(g.clone());
                }
            }
            order.extend(next.glyphs.as_arcs().iter().filter(|g| !placed.contains(&g.name)).cloned());
            next.glyphs = GlyphTable::from_glyphs(order);
        }
        "other" => {
            next.display_strings = disk.display_strings.clone();
            next.note = disk.note.clone();
            next.files = disk.files.clone();
            next.extra = disk.extra.clone();
        }
        _ => {}
    }
}

/// What to take from the file for one glyph.
#[derive(Debug, Default)]
struct Pick {
    whole: bool,
    attributes: bool,
    layers: BTreeSet<String>,
}

impl Pick {
    /// Every listed item of the glyph. A changed glyph keeps the layers the file did not change.
    fn all_of(g: &GlyphDiff) -> Self {
        match g.change {
            Change::Added | Change::Removed => Pick { whole: true, ..Pick::default() },
            Change::Changed => Pick { whole: false, attributes: g.attributes, layers: g.layers.iter().map(|(id, _)| id.clone()).collect() },
        }
    }
}

/// The accepted items.
#[derive(Debug, Default)]
struct Picks {
    glyphs: BTreeMap<String, Pick>,
    parts: BTreeSet<&'static str>,
}

impl Picks {
    fn all(diff: &Diff) -> Self {
        Picks { glyphs: diff.glyphs.iter().map(|g| (g.name.clone(), Pick::all_of(g))).collect(), parts: diff.parts.iter().copied().collect() }
    }

    /// How many of the diff's items they cover.
    fn count(&self, diff: &Diff) -> usize {
        let glyphs: usize = diff
            .glyphs
            .iter()
            .map(|g| match self.glyphs.get(&g.name) {
                None => 0,
                Some(p) if p.whole => g.items(),
                Some(p) => usize::from(p.attributes && g.attributes) + g.layers.iter().filter(|(id, _)| p.layers.contains(id)).count(),
            })
            .sum();
        glyphs + self.parts.len()
    }
}

/// `accept`, checked against what differs; `None` when it is not given.
fn parse_accept(p: &Params, diff: &Diff) -> Result<Option<Picks>> {
    let items = match p.get("accept") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) if s == "all" => return Ok(Some(Picks::all(diff))),
        Some(Value::String(s)) if s == "none" => return Ok(Some(Picks::default())),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(p.bad("`accept` must be \"all\", \"none\" or a list of items")),
    };
    if items.len() > MAX_ACCEPT_ITEMS {
        return Err(p.bad(format!("`accept` has {} items; at most {MAX_ACCEPT_ITEMS}", items.len())));
    }
    let mut picks = Picks::default();
    for (n, item) in items.iter().enumerate() {
        let bad = |why: String| p.bad(format!("`accept` item {n}: {why}"));
        let obj = item.as_object().ok_or_else(|| bad("must be an object".into()))?;
        if let Some(part) = obj.get("part") {
            let part = part.as_str().ok_or_else(|| bad("`part` must be a string".into()))?;
            let known = diff.parts.iter().copied().find(|k| *k == part).ok_or_else(|| bad(format!("the part `{part}` did not change on disk")))?;
            picks.parts.insert(known);
            continue;
        }
        let name = obj.get("glyph").and_then(Value::as_str).ok_or_else(|| bad("needs `glyph` or `part`".into()))?;
        let g = diff.glyph(name).ok_or_else(|| bad(format!("the glyph `{name}` did not change on disk")))?;
        let layer = match obj.get("layer") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or_else(|| bad("`layer` must be a layer id".into()))?),
        };
        let attributes = match obj.get("attributes") {
            None | Some(Value::Null) => false,
            Some(v) => v.as_bool().ok_or_else(|| bad("`attributes` must be true or false".into()))?,
        };
        let pick = picks.glyphs.entry(name.to_string()).or_default();
        // A glyph only one side has is taken whole, whatever part of it the item names.
        if g.change != Change::Changed {
            pick.whole = true;
            continue;
        }
        match (layer, attributes) {
            (Some(_), true) => return Err(bad("give `layer` or `attributes`, not both".into())),
            (Some(id), false) => {
                if !g.layers.iter().any(|(l, _)| l == id) {
                    return Err(bad(format!("the layer `{id}` of `{name}` did not change on disk")));
                }
                pick.layers.insert(id.to_string());
            }
            (None, true) => {
                if !g.attributes {
                    return Err(bad(format!("the attributes of `{name}` did not change on disk")));
                }
                pick.attributes = true;
            }
            (None, false) => {
                let all = Pick::all_of(g);
                pick.attributes |= all.attributes;
                pick.layers.extend(all.layers);
            }
        }
    }
    Ok(Some(picks))
}

/// The document with the picked items taken from the file.
fn merge(cur: &Font, disk: &Font, diff: &Diff, picks: &Picks) -> Font {
    let mut next = cur.clone();
    // Glyph name → its replacement (`None`: removed).
    let mut replace: HashMap<&str, Option<Arc<Glyph>>> = HashMap::new();
    let mut added: HashSet<&str> = HashSet::new();
    for g in &diff.glyphs {
        let Some(pick) = picks.glyphs.get(&g.name) else { continue };
        let (mine, theirs) = (cur.glyphs.get(&g.name), disk.glyphs.get(&g.name));
        match (g.change, mine, theirs) {
            (Change::Removed, _, _) => {
                replace.insert(g.name.as_str(), None);
            }
            (Change::Added, _, _) => {
                added.insert(g.name.as_str());
            }
            (Change::Changed, Some(mine), Some(theirs)) => {
                replace.insert(g.name.as_str(), Some(Arc::new(merge_glyph(mine, theirs, pick.attributes, &pick.layers))));
            }
            _ => {}
        }
    }
    let mut order: Vec<Arc<Glyph>> = Vec::with_capacity(cur.glyphs.len() + added.len());
    for g in cur.glyphs.as_arcs() {
        match replace.get(g.name.as_str()) {
            None => order.push(g.clone()),
            Some(Some(new)) => order.push(new.clone()),
            Some(None) => {}
        }
    }
    if !added.is_empty() {
        // Each added glyph goes after the glyph that precedes it in the file, among those the
        // document keeps, or first.
        let present: HashSet<&str> = order.iter().map(|g| g.name.as_str()).collect();
        let mut after: HashMap<Option<&str>, Vec<Arc<Glyph>>> = HashMap::new();
        let mut anchor: Option<&str> = None;
        for d in disk.glyphs.iter() {
            if present.contains(d.name.as_str()) {
                anchor = Some(d.name.as_str());
            } else if added.contains(d.name.as_str()) {
                after.entry(anchor).or_default().push(d.clone());
            }
        }
        let mut merged = after.remove(&None).unwrap_or_default();
        merged.reserve(order.len());
        for g in &order {
            merged.push(g.clone());
            if let Some(list) = after.remove(&Some(g.name.as_str())) {
                merged.extend(list);
            }
        }
        order = merged;
    }
    next.glyphs = GlyphTable::from_glyphs(order);
    for part in PARTS {
        if picks.parts.contains(part) {
            take_part(&mut next, disk, part);
        }
    }
    next
}

/// The document's glyph with the file's attributes (when `attributes`) and the file's version of
/// each layer in `layers`.
fn merge_glyph(mine: &Glyph, theirs: &Glyph, attributes: bool, layers: &BTreeSet<String>) -> Glyph {
    let mut g = if attributes {
        let mut g = theirs.clone();
        g.layers = mine.layers.clone();
        g
    } else {
        mine.clone()
    };
    for id in layers {
        let incoming = theirs.layers.iter().find(|l| &l.id == id);
        match (incoming, g.layers.iter().position(|l| &l.id == id)) {
            (Some(t), Some(i)) => {
                if let Some(slot) = g.layers.get_mut(i) {
                    *slot = t.clone();
                }
            }
            (Some(t), None) => {
                // After the last layer that precedes it in the file and that the glyph has.
                let at = theirs
                    .layers
                    .iter()
                    .take_while(|l| &l.id != id)
                    .filter_map(|l| g.layers.iter().position(|x| x.id == l.id))
                    .max()
                    .map_or(0, |p| p + 1);
                g.layers.insert(at.min(g.layers.len()), t.clone());
            }
            (None, Some(i)) => {
                g.layers.remove(i);
            }
            (None, None) => {}
        }
    }
    if attributes {
        g.layers.sort_by_key(|l| theirs.layers.iter().position(|t| t.id == l.id).unwrap_or(usize::MAX));
    }
    g
}

/// Puts `next` in place of document `i`'s font as one undo step, with the checks
/// [`Session::edit`] makes (locked glyphs, the model's checks). The glyphs keep the file's
/// last-change stamps: nothing here was edited in this app.
fn replace_font(s: &mut Session, i: usize, label: &str, next: Font) -> Result<()> {
    let (id, before, revision) = {
        let doc = s.docs_mut().get_mut(i).ok_or(EngineError::NoDocument)?;
        if next.same_as(&doc.font) {
            return Ok(());
        }
        if let Some(name) = crate::locked_glyph_changed(&next, &doc.font) {
            return Err(EngineError::Locked(name));
        }
        next.check_changed(&doc.font)?;
        let (before, revision) = (std::mem::replace(&mut doc.font, Arc::new(next)), doc.revision);
        doc.bump_revision();
        (doc.id, before, revision)
    };
    match &mut s.batch {
        Some(batch) => {
            if !batch.before.iter().any(|(d, ..)| *d == id) {
                batch.before.push((id, before, revision));
            }
        }
        None => {
            if let Some(doc) = s.docs_mut().get_mut(i) {
                crate::push_undo(doc, label.into(), before, revision);
            }
        }
    }
    Ok(())
}

/// The glyph's stored form (the native file's text) in the document and in the file, as a line
/// difference of at most [`MAX_TEXT_LINES`] lines, and whether lines past that were left out.
fn glyph_text(p: &Params, cur: &Font, disk: &Font, name: &str) -> Result<(Vec<(char, String)>, bool)> {
    let text = |f: &Font| -> Result<String> {
        match f.glyphs.get(name) {
            Some(g) => serde_json::to_string_pretty(&**g).map_err(|e| EngineError::Internal(e.to_string())),
            None => Ok(String::new()),
        }
    };
    if cur.glyphs.get(name).is_none() && disk.glyphs.get(name).is_none() {
        return Err(p.bad(format!("neither the document nor the file has a glyph `{name}`")));
    }
    let (old, new) = (text(cur)?, text(disk)?);
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let lines = line_diff(&old, &new);
    let cut = lines.len() > MAX_TEXT_LINES;
    Ok((lines.into_iter().take(MAX_TEXT_LINES).map(|(op, l)| (op, l.to_string())).collect(), cut))
}

/// `old` turned into `new`, line by line: `' '` kept, `'-'` removed, `'+'` added. The common
/// start and end are matched first; the rest by longest common subsequence when both sides have at
/// most [`MAX_DIFF_LINES`] lines, else listed whole as removed, then added.
fn line_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(char, &'a str)> {
    let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let (o, n) = (old.get(start..).unwrap_or_default(), new.get(start..).unwrap_or_default());
    let end = o.iter().rev().zip(n.iter().rev()).take_while(|(a, b)| a == b).count();
    let (o_mid, n_mid) = (o.get(..o.len().saturating_sub(end)).unwrap_or_default(), n.get(..n.len().saturating_sub(end)).unwrap_or_default());
    let mut out: Vec<(char, &str)> = old.iter().take(start).map(|l| (' ', *l)).collect();
    if o_mid.len() <= MAX_DIFF_LINES && n_mid.len() <= MAX_DIFF_LINES {
        // lcs[i * w + j]: the longest common subsequence of o_mid[i..] and n_mid[j..].
        let w = n_mid.len() + 1;
        let mut lcs = vec![0u16; (o_mid.len() + 1) * w];
        let at = |lcs: &[u16], i: usize, j: usize| lcs.get(i * w + j).copied().unwrap_or(0);
        for i in (0..o_mid.len()).rev() {
            for j in (0..n_mid.len()).rev() {
                let v =
                    if o_mid.get(i) == n_mid.get(j) { at(&lcs, i + 1, j + 1).saturating_add(1) } else { at(&lcs, i + 1, j).max(at(&lcs, i, j + 1)) };
                if let Some(slot) = lcs.get_mut(i * w + j) {
                    *slot = v;
                }
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < o_mid.len() || j < n_mid.len() {
            match (o_mid.get(i), n_mid.get(j)) {
                (Some(a), Some(b)) if a == b => {
                    out.push((' ', *a));
                    i += 1;
                    j += 1;
                }
                (Some(a), Some(_)) if at(&lcs, i + 1, j) >= at(&lcs, i, j + 1) => {
                    out.push(('-', *a));
                    i += 1;
                }
                (_, Some(b)) => {
                    out.push(('+', *b));
                    j += 1;
                }
                (Some(a), None) => {
                    out.push(('-', *a));
                    i += 1;
                }
                (None, None) => break,
            }
        }
    } else {
        out.extend(o_mid.iter().map(|l| ('-', *l)));
        out.extend(n_mid.iter().map(|l| ('+', *l)));
    }
    out.extend(o.iter().skip(o.len().saturating_sub(end)).map(|l| (' ', *l)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_difference_marks_removed_and_added_lines() {
        let old = ["a", "b", "c", "d"];
        let new = ["a", "x", "c", "d", "e"];
        let ops: String = line_diff(&old, &new).iter().map(|(op, l)| format!("{op}{l}")).collect::<Vec<_>>().join(",");
        assert_eq!(ops, " a,-b,+x, c, d,+e");
    }

    /// A glyph whose text difference passes the line limit is listed cut, and says so.
    #[test]
    fn a_long_glyph_text_difference_is_marked_cut() {
        use fontcraft_model::{Node, NodeKind, Path, Shape};
        let cur = Session::with_document().font().unwrap().clone();
        let mut disk = cur.clone();
        {
            let mut g = disk.glyph_mut("A").unwrap();
            let nodes = (0..MAX_TEXT_LINES).map(|i| Node::new(i as f64, 0.0, NodeKind::Line)).collect();
            g.layers.first_mut().unwrap().shapes.push(Shape::Path(Path::new(false, nodes)));
        }
        let v = json!({});
        let p = Params::new("file.reviewExternalChanges", &v).unwrap();
        let (lines, cut) = glyph_text(&p, &cur, &disk, "A").unwrap();
        assert!(cut);
        assert_eq!(lines.len(), MAX_TEXT_LINES);
        let (_, cut) = glyph_text(&p, &cur, &cur, "A").unwrap();
        assert!(!cut);
    }
}
