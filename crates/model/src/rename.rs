//! Renaming glyphs and the references to them (`glyph.rename`, `edit.findAndReplace`,
//! `glyph.updateGlyphInfo`).
//!
//! [`rename_glyphs`] renames several glyphs at once and rewrites every reference the model holds
//! by name: components (in layers and their backgrounds), metrics keys (of the glyph and of its
//! layers), kerning pairs, UFO groups and the `glyphOrder` parameter of the font and its
//! instances. Text the model does not parse into glyph names (feature code, kerning contexts,
//! other custom parameters, hints, display strings) is left alone; the report lists where an old
//! name still occurs there.
//!
//! Every rename happens in one pass with one name map, so chains and swaps (`a` → `b` while
//! `b` → `c`) work. [`crate::place_by_sort_key`] moves renamed glyphs to the place the default sort key
//! gives them (`glyphOrder.default`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_plist::{Str, Value};
use serde::Serialize;

use crate::{Font, GLYPH_ORDER_PARAM, Glyph, GlyphTable, KernTable, MetricsKeys, ModelError, Params, Shape, check_name, side_group};

/// One glyph renamed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Renamed {
    pub from: String,
    pub to: String,
}

/// A place that names a glyph: `name` is the old name, `place` says where, in words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GlyphReference {
    pub name: String,
    pub place: String,
}

/// What [`rename_glyphs`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameReport {
    /// The glyphs renamed, in the order asked for.
    pub renamed: Vec<Renamed>,
    /// References rewritten to the new names.
    pub updated: Vec<GlyphReference>,
    /// Places that still use an old name: text the model keeps unparsed.
    pub still_used: Vec<GlyphReference>,
}

/// Old name → new name.
type Names<'a> = HashMap<&'a str, &'a str>;

/// Renames glyphs (`(old, new)` pairs) and every reference to them the model holds by name.
/// Pairs whose names are equal are ignored. Fails, changing nothing, when an old name is not a
/// glyph or is given twice, a new name is not usable ([`crate::Font::check`]'s name rule), or a
/// new name would be taken by another glyph after the renames.
pub fn rename_glyphs(font: &mut Font, renames: &[(String, String)]) -> Result<RenameReport, ModelError> {
    let pairs: Vec<(&str, &str)> = renames.iter().filter(|(a, b)| a != b).map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut map: Names = HashMap::with_capacity(pairs.len());
    let mut at: HashMap<usize, &str> = HashMap::with_capacity(pairs.len());
    for (old, new) in &pairs {
        let i = font.glyphs.position(old).ok_or_else(|| ModelError::NoSuchGlyph(old.to_string()))?;
        if map.insert(*old, *new).is_some() {
            return Err(ModelError::BadName { what: "glyph", problem: format!("`{old}` is renamed twice") });
        }
        if new.is_empty() {
            return Err(ModelError::EmptyGlyphName);
        }
        check_name("glyph", new)?;
        at.insert(i, *new);
    }
    if map.is_empty() {
        return Ok(RenameReport::default());
    }
    let kept: HashSet<&str> = font.glyphs.iter().enumerate().filter(|(i, _)| !at.contains_key(i)).map(|(_, g)| g.name.as_str()).collect();
    let mut taken = HashSet::with_capacity(pairs.len());
    for (_, new) in &pairs {
        if kept.contains(new) || !taken.insert(*new) {
            return Err(ModelError::DuplicateGlyph(new.to_string()));
        }
    }

    let mut report =
        RenameReport { renamed: pairs.iter().map(|(a, b)| Renamed { from: a.to_string(), to: b.to_string() }).collect(), ..RenameReport::default() };
    let mut glyphs = Vec::with_capacity(font.glyphs.len());
    for (i, g) in font.glyphs.iter().enumerate() {
        let mut arc = Arc::clone(g);
        let new_name = at.get(&i);
        if new_name.is_some() || glyph_refers(&arc, &map) {
            let glyph = Arc::make_mut(&mut arc);
            if let Some(n) = new_name {
                glyph.name = n.to_string();
            }
            update_glyph(glyph, &map, &mut report.updated);
        }
        hints_using(&arc, &map, &mut report.still_used);
        glyphs.push(arc);
    }
    font.glyphs = GlyphTable::from_glyphs(glyphs);

    update_kerning(font, &map, &mut report);
    update_groups(font, &map, &mut report.updated);
    update_glyph_order(font, &map, &mut report.updated);
    font_text_using(font, &map, &mut report.still_used);
    Ok(report)
}

// ── Glyph names inside text ──

/// The byte ranges of the glyph-name tokens in `text`: runs of `A–Z a–z 0–9 _ .`, with a
/// hyphen inside a run when a letter follows it (`A-cy` is one token; `O-4` in a metrics key is
/// `O`, a minus and a number).
fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next_is_letter = chars.peek().is_some_and(|(_, n)| n.is_ascii_alphabetic());
        let inside = c.is_ascii_alphanumeric() || c == '_' || c == '.' || (c == '-' && start.is_some() && next_is_letter);
        match (inside, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// `text` with every token that is an old name replaced by the new one; `None` when no token is.
fn replace_names(text: &str, map: &Names) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut changed = false;
    for (s, e) in tokens(text) {
        if let Some(new) = text.get(s..e).and_then(|t| map.get(t)) {
            out.push_str(text.get(last..s)?);
            out.push_str(new);
            last = e;
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    out.push_str(text.get(last..)?);
    Some(out)
}

/// The old names `text` mentions as tokens, each once, in the order they first occur. With
/// `after_slash`, only tokens right after a `/` count (glyph names in a display string).
fn mentioned<'m>(text: &str, map: &Names<'m>, after_slash: bool) -> Vec<&'m str> {
    let mut out: Vec<&'m str> = Vec::new();
    for (s, e) in tokens(text) {
        if after_slash && text.get(..s).and_then(|p| p.chars().next_back()) != Some('/') {
            continue;
        }
        if let Some((&old, _)) = text.get(s..e).and_then(|t| map.get_key_value(t))
            && !out.contains(&old)
        {
            out.push(old);
        }
    }
    out
}

fn reference(name: &str, place: String) -> GlyphReference {
    GlyphReference { name: name.to_string(), place }
}

// ── Inside glyphs ──

fn keys_refer(keys: &MetricsKeys, map: &Names) -> bool {
    keys.all().iter().any(|(_, v)| v.as_deref().is_some_and(|t| !mentioned(t, map, false).is_empty()))
}

fn shapes_refer(shapes: &[Shape], map: &Names) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Component(c) if map.contains_key(c.glyph.as_str())))
}

fn glyph_refers(g: &Glyph, map: &Names) -> bool {
    keys_refer(&g.info.metrics_keys, map)
        || g.layers.iter().any(|l| {
            shapes_refer(&l.shapes, map)
                || keys_refer(&l.info.metrics_keys, map)
                || l.info.background.as_ref().is_some_and(|b| shapes_refer(&b.shapes, map))
        })
}

fn update_shapes(shapes: &mut [Shape], map: &Names, place: &dyn Fn() -> String, out: &mut Vec<GlyphReference>) {
    for s in shapes {
        if let Shape::Component(c) = s
            && let Some(new) = map.get(c.glyph.as_str())
        {
            out.push(reference(&c.glyph, place()));
            c.glyph = new.to_string();
        }
    }
}

fn update_keys(keys: &mut MetricsKeys, map: &Names, place: &dyn Fn(&str) -> String, out: &mut Vec<GlyphReference>) {
    let fields: [(&str, &mut Option<String>); 7] = [
        ("left", &mut keys.left),
        ("right", &mut keys.right),
        ("width", &mut keys.width),
        ("top", &mut keys.top),
        ("bottom", &mut keys.bottom),
        ("vertWidth", &mut keys.vert_width),
        ("vertOrigin", &mut keys.vert_origin),
    ];
    for (field, value) in fields {
        let Some(text) = value.as_deref() else { continue };
        let olds = mentioned(text, map, false);
        if let Some(new) = replace_names(text, map) {
            for old in olds {
                out.push(reference(old, place(field)));
            }
            *value = Some(new);
        }
    }
}

/// Rewrites the references inside one glyph (already carrying its new name, if renamed).
fn update_glyph(g: &mut Glyph, map: &Names, out: &mut Vec<GlyphReference>) {
    let name = g.name.clone();
    if keys_refer(&g.info.metrics_keys, map) {
        update_keys(&mut g.info.metrics_keys, map, &|field| format!("metrics key `{field}` of `{name}`"), out);
    }
    for l in &mut g.layers {
        let id = l.id.clone();
        update_shapes(&mut l.shapes, map, &|| format!("component in `{name}`, layer `{id}`"), out);
        if keys_refer(&l.info.metrics_keys, map) {
            update_keys(&mut l.info.metrics_keys, map, &|field| format!("metrics key `{field}` of `{name}`, layer `{id}`"), out);
        }
        if l.info.background.as_ref().is_some_and(|b| shapes_refer(&b.shapes, map))
            && let Some(b) = l.info.background.as_mut()
        {
            update_shapes(&mut b.shapes, map, &|| format!("background component in `{name}`, layer `{id}`"), out);
        }
    }
}

/// Hints (corner, cap and brush components among them) keep their data unparsed: report them.
fn hints_using(g: &Glyph, map: &Names, out: &mut Vec<GlyphReference>) {
    for l in &g.layers {
        let background = l.info.background.as_ref().map(|b| b.hints.as_slice()).unwrap_or_default();
        for h in l.info.hints.iter().chain(background) {
            for old in strings_mentioning(&h.value, map) {
                out.push(reference(old, format!("hint in `{}`, layer `{}`", g.name, l.id)));
            }
        }
    }
}

/// The old names the strings inside a property-list value mention.
fn strings_mentioning<'m>(v: &Value, map: &Names<'m>) -> Vec<&'m str> {
    let mut out: Vec<&'m str> = Vec::new();
    v.visit(&mut |x| {
        if let Some(s) = x.as_str() {
            for old in mentioned(s, map, false) {
                if !out.contains(&old) {
                    out.push(old);
                }
            }
        }
    });
    out
}

// ── Font-wide data ──

fn renamed_side<'m>(side: &str, map: &Names<'m>) -> Option<&'m str> {
    if side_group(side).is_some() { None } else { map.get(side).copied() }
}

fn table_refers(t: &KernTable, map: &Names) -> bool {
    t.0.iter().flat_map(|m| &m.firsts).any(|f| renamed_side(&f.first, map).is_some() || f.seconds.iter().any(|(s, _)| renamed_side(s, map).is_some()))
}

/// Renames the glyph sides of one direction's pairs. A renamed side that meets a side already
/// spelled with the new name (a pair left over for a glyph that did not exist) merges with it;
/// the pair listed later wins, as when the file is read.
fn update_table(t: &mut KernTable, direction: &str, map: &Names, out: &mut Vec<GlyphReference>) {
    for m in &mut t.0 {
        if !m.firsts.iter().any(|f| renamed_side(&f.first, map).is_some() || f.seconds.iter().any(|(s, _)| renamed_side(s, map).is_some())) {
            continue;
        }
        let place = format!("kerning ({direction}), master `{}`", m.master);
        let mut firsts: Vec<crate::KernFirst> = Vec::with_capacity(m.firsts.len());
        let mut index: HashMap<String, usize> = HashMap::with_capacity(m.firsts.len());
        for mut f in std::mem::take(&mut m.firsts) {
            if let Some(new) = renamed_side(&f.first, map) {
                out.push(reference(&f.first, place.clone()));
                f.first = new.to_string();
            }
            for (s, _) in &mut f.seconds {
                if let Some(new) = renamed_side(s, map) {
                    out.push(reference(s, place.clone()));
                    *s = new.to_string();
                }
            }
            match index.get(&f.first).and_then(|i| firsts.get_mut(*i)) {
                Some(existing) => existing.seconds.extend(f.seconds),
                None => {
                    index.insert(f.first.clone(), firsts.len());
                    firsts.push(f);
                }
            }
        }
        for f in &mut firsts {
            dedup_seconds(&mut f.seconds);
        }
        m.firsts = firsts;
    }
}

/// Keeps one pair per second side, at its first position, with the value listed last.
fn dedup_seconds(seconds: &mut Vec<(String, f64)>) {
    let mut at: HashMap<String, usize> = HashMap::with_capacity(seconds.len());
    let mut out: Vec<(String, f64)> = Vec::with_capacity(seconds.len());
    for (s, v) in std::mem::take(seconds) {
        match at.get(&s).and_then(|i| out.get_mut(*i)) {
            Some(slot) => slot.1 = v,
            None => {
                at.insert(s.clone(), out.len());
                out.push((s, v));
            }
        }
    }
    *seconds = out;
}

fn update_kerning(font: &mut Font, map: &Names, report: &mut RenameReport) {
    let k = &font.kerning;
    if table_refers(&k.ltr, map) || table_refers(&k.rtl, map) || table_refers(&k.vertical, map) {
        let k = Arc::make_mut(&mut font.kerning);
        update_table(&mut k.ltr, "left to right", map, &mut report.updated);
        update_table(&mut k.rtl, "right to left", map, &mut report.updated);
        update_table(&mut k.vertical, "vertical", map, &mut report.updated);
    }
    for c in &font.kerning.context {
        for old in mentioned(&c.context, map, false) {
            report.still_used.push(reference(old, format!("kerning context `{}`", c.context)));
        }
    }
}

fn update_groups(font: &mut Font, map: &Names, out: &mut Vec<GlyphReference>) {
    if !font.groups.iter().any(|g| g.members.iter().any(|m| map.contains_key(m.as_str()))) {
        return;
    }
    for g in Arc::make_mut(&mut font.groups) {
        for m in &mut g.members {
            if let Some(new) = map.get(m.as_str()) {
                out.push(reference(m, format!("group `{}`", g.name)));
                *m = new.to_string();
            }
        }
    }
}

/// The `glyphOrder` parameters (enabled or not) that name an old glyph.
fn order_refers(params: &Params, map: &Names) -> bool {
    params.0.iter().filter(|p| p.name == GLYPH_ORDER_PARAM).any(|p| match &p.value {
        Value::Array(a) => a.items.iter().any(|v| v.as_str().is_some_and(|s| map.contains_key(s.trim()))),
        Value::String(s) => s.as_str().lines().any(|l| map.contains_key(l.trim())),
        _ => false,
    })
}

fn update_order(params: &mut Params, map: &Names, place: &str, out: &mut Vec<GlyphReference>) {
    for p in params.0.iter_mut().filter(|p| p.name == GLYPH_ORDER_PARAM) {
        match &mut p.value {
            Value::Array(a) => {
                for v in &mut a.items {
                    if let Some((old, new)) = v.as_str().and_then(|s| map.get_key_value(s.trim())) {
                        out.push(reference(old, place.to_string()));
                        *v = Value::String(Str::new(*new));
                    }
                }
            }
            Value::String(s) => {
                let mut changed = false;
                let lines: Vec<String> = s
                    .as_str()
                    .lines()
                    .map(|l| match map.get_key_value(l.trim()) {
                        Some((old, new)) => {
                            out.push(reference(old, place.to_string()));
                            changed = true;
                            new.to_string()
                        }
                        None => l.to_string(),
                    })
                    .collect();
                if changed {
                    *s = Str::new(lines.join("\n"));
                }
            }
            _ => {}
        }
    }
}

fn update_glyph_order(font: &mut Font, map: &Names, out: &mut Vec<GlyphReference>) {
    if order_refers(&font.info.params, map) {
        update_order(&mut Arc::make_mut(&mut font.info).params, map, "the glyphOrder parameter", out);
    }
    if font.instances.iter().any(|i| order_refers(&i.params, map)) {
        for i in Arc::make_mut(&mut font.instances) {
            if order_refers(&i.params, map) {
                let place = format!("the glyphOrder parameter of instance `{}`", i.name);
                update_order(&mut i.params, map, &place, out);
            }
        }
    }
}

/// Custom parameters other than `glyphOrder` that mention an old name.
fn params_using(params: &Params, owner: &str, map: &Names, out: &mut Vec<GlyphReference>) {
    for p in params.0.iter().filter(|p| p.name != GLYPH_ORDER_PARAM) {
        for old in strings_mentioning(&p.value, map) {
            out.push(reference(old, format!("parameter `{}`{owner}", p.name)));
        }
    }
}

/// Feature code, custom parameters and display strings that still mention an old name.
fn font_text_using(font: &Font, map: &Names, out: &mut Vec<GlyphReference>) {
    let f = &font.features;
    let snippets = [("prefix", &f.prefixes), ("class", &f.classes), ("feature", &f.features)];
    for (kind, list) in snippets {
        for s in list.iter() {
            for old in mentioned(&s.code, map, false) {
                out.push(reference(old, format!("{kind} `{}`", s.name)));
            }
        }
    }
    params_using(&font.info.params, "", map, out);
    for m in font.masters.iter() {
        params_using(&m.params, &format!(" of master `{}`", m.name), map, out);
    }
    for i in font.instances.iter() {
        params_using(&i.params, &format!(" of instance `{}`", i.name), map, out);
    }
    for (n, text) in font.display_strings.iter().enumerate() {
        for old in mentioned(text, map, true) {
            out.push(reference(old, format!("display string {}", n + 1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Component, Features, Group, Snippet};
    use fontcraft_plist::Array;

    fn names(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn font() -> Font {
        let mut f = Font::new("T");
        f.glyphs = GlyphTable::from_glyphs(Vec::new());
        for n in ["A", "B", "O", "V"] {
            f.add_glyph(n, Vec::new(), 500.0).unwrap();
        }
        f
    }

    #[test]
    fn tokens_follow_glyph_name_rules() {
        let map: Names = [("O", "Q"), ("A", "X")].into_iter().collect();
        assert_eq!(replace_names("=O-4", &map).as_deref(), Some("=Q-4"));
        assert_eq!(replace_names("=|O", &map).as_deref(), Some("=|Q"));
        assert_eq!(replace_names("A-cy A.sc @MMK_L_A", &map), None);
        assert_eq!(replace_names("sub A by O;", &map).as_deref(), Some("sub X by Q;"));
        assert_eq!(mentioned("ABA /A", &map, true), ["A"]);
        assert!(mentioned("A", &map, true).is_empty());
    }

    #[test]
    fn references_follow_and_unparsed_text_is_reported() {
        let mut f = font();
        {
            let mut b = f.glyph_mut("B").unwrap();
            b.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
            b.info.metrics_keys.left = Some("=A+10".into());
        }
        let k = Arc::make_mut(&mut f.kerning);
        k.ltr.set("m01", "A", "V", -50.0);
        k.ltr.set("m01", "V", "A", -40.0);
        k.ltr.set("m01", "@MMK_L_A", "A", -10.0);
        Arc::make_mut(&mut f.groups).push(Group { name: "caps".into(), members: vec!["A".into(), "B".into()] });
        let order = ["B", "A"].iter().map(|s| Value::String(Str::new(*s))).collect();
        Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, Value::Array(Array { items: order, ..Default::default() }));
        f.features = Arc::new(Features {
            features: vec![Snippet { name: "liga".into(), code: "sub A by B;".into(), ..Default::default() }],
            ..Default::default()
        });

        let r = rename_glyphs(&mut f, &names(&[("A", "A.alt")])).unwrap();
        let b = f.glyph("B").unwrap();
        assert!(matches!(&b.layers[0].shapes[0], Shape::Component(c) if c.glyph == "A.alt"));
        assert_eq!(b.info.metrics_keys.left.as_deref(), Some("=A.alt+10"));
        assert_eq!(f.kerning.ltr.get("m01", "A.alt", "V"), Some(-50.0));
        assert_eq!(f.kerning.ltr.get("m01", "V", "A.alt"), Some(-40.0));
        assert_eq!(f.kerning.ltr.get("m01", "@MMK_L_A", "A.alt"), Some(-10.0), "group sides keep their names");
        assert_eq!(f.groups[0].members, ["A.alt", "B"]);
        assert_eq!(crate::glyph_order_names(&f), ["B", "A.alt"]);
        assert_eq!(r.still_used, [GlyphReference { name: "A".into(), place: "feature `liga`".into() }]);
        assert_eq!(r.updated.len(), 7, "{:?}", r.updated);
        assert!(f.glyph("A").is_none() && f.glyph("A.alt").is_some());
    }

    #[test]
    fn swaps_work_and_clashes_change_nothing() {
        let mut f = font();
        rename_glyphs(&mut f, &names(&[("A", "B"), ("B", "A")])).unwrap();
        let order: Vec<&str> = f.glyphs.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(&order[..2], ["B", "A"]);
        assert_eq!(f.glyphs.position("A"), Some(1), "the index follows");

        let before = f.clone();
        assert_eq!(rename_glyphs(&mut f, &names(&[("A", "O")])), Err(ModelError::DuplicateGlyph("O".into())));
        assert_eq!(rename_glyphs(&mut f, &names(&[("A", "x"), ("B", "x")])), Err(ModelError::DuplicateGlyph("x".into())));
        assert!(rename_glyphs(&mut f, &names(&[("nope", "x")])).is_err());
        assert!(rename_glyphs(&mut f, &names(&[("A", "x\u{0}")])).is_err());
        assert_eq!(f, before);
    }

    #[test]
    fn placing_by_sort_key_matches_one_by_one_placement() {
        let d = fontcraft_glyphdata::GlyphData::new().unwrap();
        let mut f = font();
        for n in ["one", "C.alt", "zz"] {
            f.add_glyph(n, Vec::new(), 500.0).unwrap();
        }
        let mut others = f.clone();
        others.remove_glyph("C.alt").unwrap();
        let expected = crate::insertion_index(&others, &d, &Glyph::new("C.alt"));
        crate::place_by_sort_key(&mut f, &d, &["C.alt".to_string()]);
        assert_eq!(f.glyphs.position("C.alt"), Some(expected));
        let rest: Vec<&str> = f.glyphs.iter().map(|g| g.name.as_str()).filter(|n| *n != "C.alt").collect();
        let before: Vec<&str> = others.glyphs.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(rest, before, "the other glyphs keep their order");
    }
}
