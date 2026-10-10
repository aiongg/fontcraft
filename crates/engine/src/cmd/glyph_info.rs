//! Glyph properties and names: renaming (one glyph, Find and Replace, Update Glyph Info), the
//! property setters of the inspector, List View and Selection Info, tags and notes, and the
//! read-only queries on glyph names and the glyph database (`docs/spec/05-font-view.md` § 2,
//! § 4.2 to § 4.4).
//!
//! Every setter takes `glyphs` and changes them all in one undo step. A `null` (or empty) value
//! returns a database-backed property to what the glyph data says. Renames go through
//! [`fontcraft_model::rename_glyphs`], so references follow, and the renamed glyphs move to the
//! place the default sort order gives them.

use std::collections::{HashMap, HashSet};

use fontcraft_model::glyphdata::{
    Case, Category, Direction, GlyphData, Info, MAX_NAME_BYTES, MAX_QUERY_BYTES, MAX_RESULTS, NameProblem, Record, SortBy, SubCategory, Via, validate,
};
use fontcraft_model::{
    ColorLabel, Font, GlyphInfo, MAX_GLYPH_TAGS, MAX_NAME_LEN, Prop, RenameReport, glyph_order, glyph_props, keeps_alternates, place_by_sort_key,
    rename_glyphs,
};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, codepoint, kind};

/// Longest glyph note, in bytes.
const MAX_NOTE_BYTES: usize = 1 << 20;
/// Longest script id, in bytes.
const MAX_SCRIPT_LEN: usize = 64;
/// Most problems one refused Find and Replace lists by name.
const MAX_LISTED_PROBLEMS: usize = 20;

const GLYPHS: &[TargetKind] = &[TargetKind::Glyphs];

/// The twelve colour label names, in index order (`glyph.colorLabelIndices`), lowercase without
/// spaces; `grey` spellings are accepted too.
const COLOR_NAMES: [&str; 12] =
    ["red", "orange", "brown", "yellow", "lightgreen", "darkgreen", "cyan", "blue", "purple", "pink", "lightgray", "darkgray"];

/// The six formats of Copy Glyph Names.
const COPY_FORMATS: [&str; 6] = ["spaceSeparated", "onePerLine", "commaSeparated", "slashed", "pythonList", "unicodes"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.rename",
            "Rename Glyph",
            [],
            None,
            "{glyph: name, newName: string, updateInfo?: bool (default true)} renames a glyph and every reference the font holds by name: components, metrics keys, kerning pairs, groups and the glyphOrder parameter. With updateInfo, a production, alternative or code-point name becomes the glyph data's nice name unless the font uses custom naming, and when the base name (before the first dot) changes, the Unicode values become the glyph data's for the new name; a name the glyph data does not know keeps them. The glyph then moves to the place the default sort order gives it. Refused when the name is taken, breaks the naming rules, or a locked glyph uses the glyph as a component → {glyph: the new name, unicodes: [hex], updated: [{name, place}], stillUsed: [{name, place}]}",
            has_doc,
            rename
        ),
        cmd!(
            "glyph.setProductionName",
            "Set Production Name",
            [],
            None,
            "{glyphs: [names], productionName: string | null} sets the name the glyphs export under; null or empty returns to the name the glyph data gives. The name must follow the naming rules → {glyphs: [names]}",
            has_doc,
            set_production_name
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setScript",
            "Set Script",
            [],
            None,
            "{glyphs: [names], script: string | null} sets the writing script (a lowercase id such as latin, cyrillic, arabic); null or empty returns to the glyph data's value → {glyphs: [names]}",
            has_doc,
            set_script
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setCategory",
            "Set Category & Subcategory",
            [],
            None,
            "{glyphs: [names], category?: string | null, subCategory?: string | null} sets the category (Letter, Mark, Number, Punctuation, Separator, Symbol, Space, Other) and/or the subcategory (Uppercase, Nonspacing, Decimal Digit, Ligature…); give at least one. null or empty returns that field to the glyph data's value → {glyphs: [names]}",
            has_doc,
            set_category
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setCase",
            "Set Case",
            [],
            None,
            "{glyphs: [names], case: \"upper\" | \"lower\" | \"smallCaps\" | \"minor\" | \"noCase\" | null} sets the case; noCase states that the glyph has none, null returns to the glyph data's value → {glyphs: [names]}",
            has_doc,
            set_case
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setDirection",
            "Set Writing Direction",
            [],
            None,
            "{glyphs: [names], direction: \"LTR\" | \"RTL\" | \"BIDI\" | \"VTR\" | \"VTL\" | null} sets the writing direction; null returns to the glyph data's value → {glyphs: [names]}",
            has_doc,
            set_direction
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setExport",
            "Exports",
            [],
            None,
            "{glyphs: [names], export: bool} sets whether the glyphs are written into exported fonts → {glyphs: [names]}",
            has_doc,
            set_export
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setColorLabel",
            "Color Label",
            [],
            None,
            "{glyphs: [names], color: 0..11 | name | null} sets the glyph colour label by index or name (red, orange, brown, yellow, lightGreen, darkGreen, cyan, blue, purple, pink, lightGray, darkGray); null clears it → {glyphs: [names]}",
            has_doc,
            set_color_label
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setLayerColorLabel",
            "Layer Color Label",
            [],
            None,
            "{glyphs: [names], layer?: id (default: the first master's layer), color: 0..11 | name | null} sets the colour label of one layer of each glyph; null clears it. Locked glyphs and glyphs without the layer are skipped and listed → {glyphs: [names], skipped: [{glyph, reason}]}",
            has_doc,
            set_layer_color_label
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setTags",
            "Set Tags",
            [],
            None,
            "{glyphs: [names], tags: [string]} replaces the glyphs' tags. A tag is trimmed, not empty, at most 255 bytes, without a comma or control character; tags are kept sorted, each once, at most 256 per glyph → {glyphs: [names], tags: [string]}",
            has_doc,
            set_tags
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.addTag",
            "Add Tag",
            [],
            None,
            "{glyphs: [names], tag: string} adds one tag to every glyph that lacks it (rules as in glyph.setTags) → {glyphs: [names]}",
            has_doc,
            add_tag
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.removeTag",
            "Remove Tag",
            [],
            None,
            "{glyphs: [names], tag: string} removes one tag from every glyph that has it → {glyphs: [names]}",
            has_doc,
            remove_tag
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setSortName",
            "Set Sort Name",
            [],
            None,
            "{glyphs: [names], sortName: string | null} sets the key the glyphs are ordered by; null or empty orders them by the default key again. The glyphs move to their new place in the default order (a glyphOrder parameter still wins) → {glyphs: [names]}",
            has_doc,
            set_sort_name
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.setNote",
            "Set Glyph Note",
            [],
            None,
            "{glyphs: [names], note: string} sets the glyphs' free-text note (at most 1 MB; empty clears it) → {glyphs: [names]}",
            has_doc,
            set_note
        )
        .targets(GLYPHS),
        cmd!(
            "glyph.updateGlyphInfo",
            "Update Glyph Info",
            ["Glyph"],
            None,
            "{glyphs: [names]} re-derives the glyphs' properties from the glyph data: a production, alternative or code-point name becomes the nice name (not when the font uses custom naming), the Unicode values become the glyph data's for the name (a name it does not know keeps them), and stored category, subcategory, script, production name, case and direction are dropped. Sort name, tags, colour labels, note and the export flag stay. A nice name that is already taken leaves that glyph's name and Unicode values as they were and is listed in `kept` → {glyphs: [final names], renamed: [{from, to}], kept: [{glyph, name, reason}], updated: [{name, place}], stillUsed: [{name, place}]}",
            has_doc,
            update_glyph_info
        )
        .targets(GLYPHS),
        cmd!(
            query "glyph.getInfo",
            "Get Glyph Info",
            [],
            None,
            "{glyphs?: [names] (default: every glyph)} every property the inspector, List View and Selection Info show, in glyph order. Each database-backed property has its value and its source: stored (set on the glyph), custom (a custom glyph data file), database, synthesised (from the name), default or none → {glyphs: [{name, unicodes: [hex], export, locked, color, tags, note, lastChange, metricsKeys, kernGroups, properties: {productionName, category, subCategory, case, direction, script, sortName, description: {value, source}}, dataUnicode, hasCustomInfo}]}",
            has_doc,
            get_info
        )
        .targets(GLYPHS),
        cmd!(
            query "glyph.copyGlyphNames",
            "Copy Glyph Names",
            [],
            None,
            "{glyphs?: [names] (default: every glyph), format: \"spaceSeparated\" | \"onePerLine\" | \"commaSeparated\" | \"slashed\" | \"pythonList\" | \"unicodes\"} the glyphs' names, in glyph order, as clipboard text: spaceSeparated `a b ` and commaSeparated `a, b, ` put the separator after every name; onePerLine one name a line; slashed `/a/b`; pythonList `\"a\",` a line each; unicodes one line per glyph with its code points as hex (at least four digits, several joined by `, `), or `- # name` for a glyph without one → {text}",
            has_doc,
            copy_glyph_names
        )
        .targets(GLYPHS),
        cmd!(
            "edit.findAndReplace",
            "Find and Replace…",
            ["Edit", "Find"],
            Some("Cmd+Shift+F"),
            "{glyphs: [names], find: string, replace: string, regex?: bool} renames the glyphs by replacing every occurrence of `find` in their names with `replace`; an empty `find` appends `replace` to every name. Names without `find` stay. Each rename follows glyph.rename (references follow; Unicode values follow a changed base name) without converting to nice names. If any result is taken, given twice or breaks the naming rules, nothing is renamed and the problems are listed. Regular expressions are not available yet → {renamed: [{from, to}], updated: [{name, place}], stillUsed: [{name, place}]}",
            has_doc,
            find_and_replace
        )
        .targets(GLYPHS),
        cmd!(
            query "font.validateGlyphNames",
            "Validate Glyph Names",
            [],
            None,
            "{glyphs?: [names] (default: every glyph)} checks names against the naming rules: only A-Z a-z 0-9 _ - . and a letter first (glyphs that do not export, names starting with _, .notdef and .null are exempt from the second rule); names used twice are listed too → {checked: count, invalid: [{glyph, problem: \"empty\" | \"character\" | \"firstCharacter\" | \"duplicate\", position: from 1, character, message}]}",
            has_doc,
            validate_glyph_names
        )
        .targets(GLYPHS),
        cmd!(
            query "glyphData.lookup",
            "Look Up Glyph Data",
            [],
            None,
            "{name?: string, unicode?: hex | number, char?: string} give one: what the glyph data says about a glyph name, or about the glyph for a code point or a character (custom files first). Each value has its source: custom, database, synthesised or default → {name, found: {name, via: \"name\" | \"altName\" | \"productionName\" | \"codePoint\", custom} | null, unicode, category, subCategory, productionName, script, case, direction, description: {value, source} | null, altNames: [names], legacyUnicode: hex | null}",
            always,
            lookup
        ),
        cmd!(
            query "glyphData.search",
            "Search Glyph Data",
            [],
            None,
            "{query: string, sortBy?: \"name\" | \"unicode\" | \"category\" | \"subCategory\" | \"script\" | \"description\" | \"production\", limit?: 1..10000 (default 10000)} glyph data records whose name, description or hex code point contains the query (ignoring case), or whose character it is; custom records first shadow built-in ones. The query is at most 256 bytes → {results: [{name, unicode, category, subCategory, script, case, direction, production, description, altNames, legacyUnicode}], count}",
            always,
            search
        ),
        cmd!(
            query "glyphData.productionName",
            "Resolve Production Name",
            [],
            None,
            "{glyphs?: [names] (default: every glyph)} the name each glyph exports under: its own production name, else the glyph data's, else one built from the suffix and ligature rules, else its own name → {glyphs: [{glyph, productionName, source}]}",
            has_doc,
            production_names
        )
        .targets(GLYPHS),
    ]
}

// ── Shared helpers ──

fn hex(cp: u32) -> String {
    format!("{cp:04X}")
}

/// `glyphs`, required and not empty.
fn targets(p: &Params, font: &Font) -> Result<Vec<String>> {
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    Ok(names)
}

/// `glyphs` in glyph order, or every glyph in glyph order when absent.
fn targets_in_order(p: &Params, font: &Font) -> Result<Vec<String>> {
    let order = glyph_order(font);
    let named = |i: &usize| font.glyphs.as_arcs().get(*i).map(|g| g.name.clone());
    match p.opt_glyphs("glyphs", font)? {
        None => Ok(order.iter().filter_map(named).collect()),
        Some(names) => {
            let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
            Ok(order.iter().filter_map(named).filter(|n| wanted.contains(n.as_str())).collect())
        }
    }
}

/// A value that may be `null`: `Some(None)` for null, `Some(Some(text))` for text (empty text
/// counts as null), an error when the key is missing or holds another type.
fn text_or_null(p: &Params, raw: &Value, key: &str) -> Result<Option<String>> {
    match raw.get(key) {
        None => Err(p.bad(format!("`{key}` is required (null returns the property to the glyph data's value)"))),
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(other) => Err(p.bad(format!("`{key}` must be a string or null, got {}", kind(other)))),
    }
}

/// Runs `f` on the info of every glyph in `names`, as one undo step.
fn edit_info(s: &mut Session, label: &str, names: &[String], f: impl Fn(&mut GlyphInfo)) -> Result<()> {
    s.edit(label, |font| {
        for name in names {
            let mut g = font.glyph_mut(name)?;
            f(&mut g.info);
            g.info.tidy();
        }
        Ok(())
    })
}

/// A name the naming rules accept, at most [`MAX_NAME_LEN`] bytes.
fn check_name(p: &Params, name: &str, exports: bool) -> Result<()> {
    if name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("the name is longer than {MAX_NAME_LEN} bytes")));
    }
    validate(name, exports).map_err(|e| p.bad(format!("`{name}`: {e}")))
}

/// Use Custom Naming (Font Info › Other): names stay as typed.
fn custom_naming(font: &Font) -> bool {
    font.info.settings.disables_nice_names.unwrap_or(false)
}

/// The name a typed name becomes: the glyph data's nice name for a production, alternative or
/// code-point name, unless the font uses custom naming (`glyphName.useCustomNaming`).
fn nice_name(font: &Font, data: &GlyphData, typed: &str) -> String {
    if custom_naming(font) {
        return typed.to_string();
    }
    data.canonical_name(typed).unwrap_or_else(|| typed.to_string())
}

/// The part of a name before its first dot.
fn base_name(name: &str) -> &str {
    name.split_once('.').map_or(name, |(base, _)| base)
}

/// The code points the glyph data gives `name`, or `stored` when the data does not know the name
/// at all (05-font-view.md, open question 4).
fn data_unicodes(data: &GlyphData, name: &str, stored: &[u32]) -> Vec<u32> {
    let info = data.lookup(name, &[]);
    if info.found.is_none() && info.category.is_none() && info.unicode.is_none() {
        return stored.to_vec();
    }
    info.unicode.map(|u| vec![u.value]).unwrap_or_default()
}

/// The code points a glyph has after a rename: the data's for the new name when the base name
/// changed (`glyphName.renameUpdatesInfo`), else the stored ones.
fn renamed_unicodes(data: &GlyphData, old: &str, new: &str, stored: &[u32]) -> Vec<u32> {
    if base_name(old) == base_name(new) { stored.to_vec() } else { data_unicodes(data, new, stored) }
}

/// Renames inside an edit: the names and references, the code points (keyed by new name), and
/// the renamed glyphs' places in the default order.
fn apply_renames(font: &mut Font, data: &GlyphData, pairs: &[(String, String)], codes: &HashMap<String, Vec<u32>>) -> Result<RenameReport> {
    let report = rename_glyphs(font, pairs)?;
    for r in &report.renamed {
        if let Some(cps) = codes.get(&r.to) {
            let mut g = font.glyph_mut(&r.to)?;
            if g.unicodes != *cps {
                g.unicodes = cps.clone();
            }
        }
    }
    let moved: Vec<String> = report.renamed.iter().map(|r| r.to.clone()).collect();
    place_by_sort_key(font, data, &moved);
    Ok(report)
}

fn report_json(r: &RenameReport) -> (Value, Value, Value) {
    let renamed = Value::Array(r.renamed.iter().map(|x| json!({"from": x.from, "to": x.to})).collect());
    let refs = |list: &[fontcraft_model::GlyphReference]| Value::Array(list.iter().map(|x| json!({"name": x.name, "place": x.place})).collect());
    (renamed, refs(&r.updated), refs(&r.still_used))
}

// ── Renaming ──

fn rename(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.rename", v)?;
    let old = p.str("glyph")?;
    let typed = p.str("newName")?.trim();
    let update = p.bool_or("updateInfo", true)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let glyph = font.glyph(old).ok_or_else(|| p.bad(format!("there is no glyph `{old}`")))?;
    if typed.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`newName` is longer than {MAX_NAME_LEN} bytes")));
    }
    let new = if update { nice_name(font, &data, typed) } else { typed.to_string() };
    check_name(&p, &new, glyph.export)?;
    if new != old && font.glyph(&new).is_some() {
        return Err(p.bad(format!("there is already a glyph `{new}`")));
    }
    let codes = if update { renamed_unicodes(&data, old, &new, &glyph.unicodes) } else { glyph.unicodes.clone() };
    let pairs = vec![(old.to_string(), new.clone())];
    let by_name = HashMap::from([(new.clone(), codes)]);
    let report = s.edit("Rename Glyph", |font| apply_renames(font, &data, &pairs, &by_name))?;
    let (_, updated, still_used) = report_json(&report);
    let unicodes: Vec<String> = s.font()?.glyph(&new).map(|g| g.unicodes.iter().map(|c| hex(*c)).collect()).unwrap_or_default();
    Ok(json!({ "glyph": new, "unicodes": unicodes, "updated": updated, "stillUsed": still_used }))
}

fn find_and_replace(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.findAndReplace", v)?;
    let find = p.str("find")?;
    let replace = p.str("replace")?;
    if p.bool_or("regex", false)? {
        return Err(p.bad("regular expressions are not available yet; turn `regex` off to replace plain text"));
    }
    for (key, text) in [("find", find), ("replace", replace)] {
        if text.len() > MAX_NAME_LEN {
            return Err(p.bad(format!("`{key}` is longer than {MAX_NAME_LEN} bytes")));
        }
    }
    let data = s.glyph_data()?;
    let font = s.font()?;
    let names = targets(&p, font)?;
    let mut pairs = Vec::new();
    for name in &names {
        let new = if find.is_empty() { format!("{name}{replace}") } else { name.replace(find, replace) };
        if new != *name {
            pairs.push((name.clone(), new));
        }
    }
    // Every result is checked before anything is renamed: a batch with a problem is refused whole.
    let renamed: HashSet<&str> = pairs.iter().map(|(old, _)| old.as_str()).collect();
    let mut taken: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).filter(|n| !renamed.contains(n)).collect();
    let mut problems = Vec::new();
    for (old, new) in &pairs {
        let exports = font.glyph(old).is_none_or(|g| g.export);
        if new.len() > MAX_NAME_LEN {
            problems.push(format!("`{old}` → a name longer than {MAX_NAME_LEN} bytes"));
        } else if let Err(e) = validate(new, exports) {
            problems.push(format!("`{old}` → `{new}`: {e}"));
        } else if !taken.insert(new.as_str()) {
            problems.push(format!("`{old}` → `{new}`: the name is taken"));
        }
    }
    if !problems.is_empty() {
        let count = problems.len();
        let listed: Vec<String> = problems.into_iter().take(MAX_LISTED_PROBLEMS).collect();
        let more = if count > listed.len() { format!(" and {} more", count - listed.len()) } else { String::new() };
        return Err(p.bad(format!("nothing was renamed: {}{more}", listed.join("; "))));
    }
    let mut codes = HashMap::with_capacity(pairs.len());
    for (old, new) in &pairs {
        let stored = font.glyph(old).map(|g| g.unicodes.clone()).unwrap_or_default();
        codes.insert(new.clone(), renamed_unicodes(&data, old, new, &stored));
    }
    let report = s.edit("Find and Replace", |font| apply_renames(font, &data, &pairs, &codes))?;
    let (renamed, updated, still_used) = report_json(&report);
    Ok(json!({ "renamed": renamed, "updated": updated, "stillUsed": still_used }))
}

fn update_glyph_info(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.updateGlyphInfo", v)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let names = targets(&p, font)?;
    // A new name may not be any name the font holds now, nor one an earlier glyph of the batch
    // takes, so a refused rename never leaves two glyphs with one name.
    let existing: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut pairs = Vec::new();
    let mut kept = Vec::new();
    let mut codes: HashMap<String, Vec<u32>> = HashMap::with_capacity(names.len());
    let mut finals = Vec::with_capacity(names.len());
    for name in &names {
        let Some(glyph) = font.glyph(name) else { continue };
        let nice = nice_name(font, &data, name);
        let refusal = if nice == *name {
            None
        } else if nice.len() > MAX_NAME_LEN || validate(&nice, glyph.export).is_err() {
            Some("the nice name breaks the naming rules")
        } else if existing.contains(nice.as_str()) || taken.contains(&nice) {
            Some("the nice name is taken")
        } else {
            None
        };
        if let Some(reason) = refusal {
            kept.push(json!({"glyph": name, "name": nice, "reason": reason}));
            finals.push(name.clone());
            continue;
        }
        codes.insert(name.clone(), data_unicodes(&data, &nice, &glyph.unicodes));
        if nice != *name {
            pairs.push((name.clone(), nice.clone()));
            taken.insert(nice.clone());
        }
        finals.push(nice);
    }
    let report = s.edit("Update Glyph Info", |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            if let Some(cps) = codes.get(name)
                && g.unicodes != *cps
            {
                g.unicodes = cps.clone();
            }
            let i = &g.info;
            let overrides = [&i.category, &i.sub_category, &i.script, &i.production, &i.case, &i.direction];
            if overrides.iter().any(|o| o.is_some()) {
                let info = &mut g.info;
                info.category = None;
                info.sub_category = None;
                info.script = None;
                info.production = None;
                info.case = None;
                info.direction = None;
                info.tidy();
            }
        }
        apply_renames(font, &data, &pairs, &HashMap::new())
    })?;
    let (renamed, updated, still_used) = report_json(&report);
    Ok(json!({ "glyphs": finals, "renamed": renamed, "kept": kept, "updated": updated, "stillUsed": still_used }))
}

// ── Setters ──

fn set_production_name(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setProductionName", v)?;
    let names = targets(&p, s.font()?)?;
    let value = text_or_null(&p, v, "productionName")?;
    if let Some(name) = &value {
        check_name(&p, name, true)?;
    }
    edit_info(s, "Set Production Name", &names, |i| i.production = value.clone())?;
    Ok(json!({ "glyphs": names }))
}

fn set_script(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setScript", v)?;
    let names = targets(&p, s.font()?)?;
    let value = text_or_null(&p, v, "script")?;
    if let Some(script) = &value
        && (script.len() > MAX_SCRIPT_LEN || !script.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'))
    {
        return Err(p.bad(format!("`script` must be a lowercase id of at most {MAX_SCRIPT_LEN} characters (a-z, 0-9, _ and -), such as latin")));
    }
    edit_info(s, "Set Script", &names, |i| i.script = value.clone())?;
    Ok(json!({ "glyphs": names }))
}

fn set_category(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setCategory", v)?;
    let names = targets(&p, s.font()?)?;
    let given = |key: &str| v.get(key).is_some();
    if !given("category") && !given("subCategory") {
        return Err(p.bad("give `category`, `subCategory` or both"));
    }
    let category = if given("category") { Some(text_or_null(&p, v, "category")?) } else { None };
    let sub_category = if given("subCategory") { Some(text_or_null(&p, v, "subCategory")?) } else { None };
    if let Some(Some(c)) = &category
        && Category::parse(c).is_none()
    {
        let known: Vec<&str> = Category::ALL.iter().map(|c| c.as_str()).collect();
        return Err(p.bad(format!("`category` must be one of {}, got \"{c}\"", known.join(", "))));
    }
    if let Some(Some(c)) = &sub_category
        && SubCategory::parse(c).is_none()
    {
        let known: Vec<&str> = SubCategory::ALL.iter().map(|c| c.as_str()).collect();
        return Err(p.bad(format!("`subCategory` must be one of {}, got \"{c}\"", known.join(", "))));
    }
    edit_info(s, "Set Category", &names, |i| {
        if let Some(c) = &category {
            i.category = c.clone();
        }
        if let Some(c) = &sub_category {
            i.sub_category = c.clone();
        }
    })?;
    Ok(json!({ "glyphs": names }))
}

fn set_case(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setCase", v)?;
    let names = targets(&p, s.font()?)?;
    let value = text_or_null(&p, v, "case")?;
    if let Some(c) = &value
        && c != "noCase"
        && Case::parse(c).is_none()
    {
        return Err(p.bad(format!("`case` must be \"upper\", \"lower\", \"smallCaps\", \"minor\", \"noCase\" or null, got \"{c}\"")));
    }
    edit_info(s, "Set Case", &names, |i| i.case = value.clone())?;
    Ok(json!({ "glyphs": names }))
}

fn set_direction(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setDirection", v)?;
    let names = targets(&p, s.font()?)?;
    let value = text_or_null(&p, v, "direction")?;
    if let Some(d) = &value
        && Direction::parse(d).is_none()
    {
        return Err(p.bad(format!("`direction` must be \"LTR\", \"RTL\", \"BIDI\", \"VTR\", \"VTL\" or null, got \"{d}\"")));
    }
    edit_info(s, "Set Writing Direction", &names, |i| i.direction = value.clone())?;
    Ok(json!({ "glyphs": names }))
}

fn set_export(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setExport", v)?;
    let names = targets(&p, s.font()?)?;
    let export = match p.get("export") {
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(p.bad(format!("`export` must be true or false, got {}", kind(other)))),
        None => return Err(p.bad("`export` is required")),
    };
    s.edit("Exports", |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            if g.export != export {
                g.export = export;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names }))
}

/// A colour label from an index (0 to 11), a name, or `null` (none). Missing is an error.
fn color_label(p: &Params, raw: &Value) -> Result<Option<ColorLabel>> {
    let bad = || {
        p.bad("`color` must be an index from 0 to 11, a label name (red, orange, brown, yellow, lightGreen, darkGreen, cyan, blue, purple, pink, lightGray, darkGray) or null")
    };
    match raw.get("color") {
        None => Err(p.bad("`color` is required (null clears the label)")),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_u64().and_then(|i| u8::try_from(i).ok()).filter(|i| *i < 12) {
            Some(i) => Ok(Some(ColorLabel::Index(i))),
            None => Err(bad()),
        },
        Some(Value::String(s)) => {
            let key: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase().replace("grey", "gray");
            let i = COLOR_NAMES.iter().position(|n| *n == key).and_then(|i| u8::try_from(i).ok()).ok_or_else(bad)?;
            Ok(Some(ColorLabel::Index(i)))
        }
        Some(_) => Err(bad()),
    }
}

fn set_color_label(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setColorLabel", v)?;
    let names = targets(&p, s.font()?)?;
    let color = color_label(&p, v)?;
    edit_info(s, "Color Label", &names, |i| i.color = color)?;
    Ok(json!({ "glyphs": names }))
}

fn set_layer_color_label(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setLayerColorLabel", v)?;
    let font = s.font()?;
    let names = targets(&p, font)?;
    let color = color_label(&p, v)?;
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let mut skipped = Vec::new();
    let mut changed = Vec::new();
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        if g.info.locked {
            skipped.push(json!({"glyph": name, "reason": "the glyph is locked"}));
        } else if !g.layers.iter().any(|l| l.id == layer) {
            skipped.push(json!({"glyph": name, "reason": format!("the glyph has no layer `{layer}`")}));
        } else {
            changed.push(name.clone());
        }
    }
    s.edit("Layer Color Label", |font| {
        for name in &changed {
            let mut g = font.glyph_mut(name)?;
            if let Some(l) = g.layers.iter_mut().find(|l| l.id == layer)
                && l.info.color != color
            {
                l.info.color = color;
                l.info.tidy();
            }
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": changed, "skipped": skipped }))
}

/// A tag as stored: trimmed, not empty, at most [`MAX_NAME_LEN`] bytes, no comma or control
/// character.
fn tag(p: &Params, text: &str) -> Result<String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(p.bad("a tag is empty"));
    }
    if t.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("a tag is longer than {MAX_NAME_LEN} bytes")));
    }
    if t.contains(',') || t.chars().any(char::is_control) {
        return Err(p.bad(format!("the tag \"{t}\" contains a comma or a control character")));
    }
    Ok(t.to_string())
}

fn sorted_tags(mut tags: Vec<String>) -> Vec<String> {
    tags.sort();
    tags.dedup();
    tags
}

fn set_tags(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setTags", v)?;
    let names = targets(&p, s.font()?)?;
    let list = p.array("tags")?;
    if list.len() > MAX_GLYPH_TAGS {
        return Err(p.bad(format!("`tags` holds {} tags; a glyph may have at most {MAX_GLYPH_TAGS}", list.len())));
    }
    let mut tags = Vec::with_capacity(list.len());
    for t in list {
        let text = t.as_str().ok_or_else(|| p.bad(format!("`tags` must hold strings, found {}", kind(t))))?;
        tags.push(tag(&p, text)?);
    }
    let tags = sorted_tags(tags);
    edit_info(s, "Set Tags", &names, |i| {
        if i.tags != tags {
            i.tags = tags.clone();
        }
    })?;
    Ok(json!({ "glyphs": names, "tags": tags }))
}

fn add_tag(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.addTag", v)?;
    let font = s.font()?;
    let names = targets(&p, font)?;
    let t = tag(&p, p.str("tag")?)?;
    if let Some(full) = names.iter().find(|n| font.glyph(n).is_some_and(|g| g.info.tags.len() >= MAX_GLYPH_TAGS && !g.info.tags.contains(&t))) {
        return Err(p.bad(format!("glyph `{full}` already has {MAX_GLYPH_TAGS} tags")));
    }
    edit_info(s, "Add Tag", &names, |i| {
        if !i.tags.contains(&t) {
            let mut tags = i.tags.clone();
            tags.push(t.clone());
            i.tags = sorted_tags(tags);
        }
    })?;
    Ok(json!({ "glyphs": names }))
}

fn remove_tag(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.removeTag", v)?;
    let names = targets(&p, s.font()?)?;
    let t = p.str("tag")?.trim().to_string();
    edit_info(s, "Remove Tag", &names, |i| {
        if i.tags.contains(&t) {
            i.tags.retain(|x| *x != t);
        }
    })?;
    Ok(json!({ "glyphs": names }))
}

fn set_sort_name(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setSortName", v)?;
    let names = targets(&p, s.font()?)?;
    let value = text_or_null(&p, v, "sortName")?;
    if let Some(name) = &value
        && (name.len() > MAX_NAME_LEN || name.chars().any(char::is_control))
    {
        return Err(p.bad(format!("`sortName` must be at most {MAX_NAME_LEN} bytes without control characters")));
    }
    let data = s.glyph_data()?;
    s.edit("Set Sort Name", |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            if g.info.sort_name != value || (value.is_none() && g.info.sort_name_keep.is_some()) {
                let info = &mut g.info;
                info.sort_name = value.clone();
                if value.is_none() {
                    info.sort_name_keep = None;
                }
                info.tidy();
            }
        }
        place_by_sort_key(font, &data, &names);
        Ok(())
    })?;
    Ok(json!({ "glyphs": names }))
}

fn set_note(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setNote", v)?;
    let names = targets(&p, s.font()?)?;
    let note = p.str("note")?;
    if note.len() > MAX_NOTE_BYTES {
        return Err(p.bad(format!("`note` is longer than {MAX_NOTE_BYTES} bytes")));
    }
    edit_info(s, "Set Glyph Note", &names, |i| {
        if i.note != note {
            i.note = note.to_string();
        }
    })?;
    Ok(json!({ "glyphs": names }))
}

// ── Queries ──

fn prop_json(prop: &Prop) -> Value {
    json!({ "value": prop.value, "source": prop.source.as_str() })
}

fn color_json(c: Option<ColorLabel>) -> Value {
    match c {
        None => Value::Null,
        Some(ColorLabel::Index(i)) => json!(i),
        Some(ColorLabel::Rgba(rgba)) => json!(rgba),
    }
}

fn get_info(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.getInfo", v)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let keep = keeps_alternates(font);
    let mut out = Vec::new();
    for name in targets_in_order(&p, font)? {
        let Some(g) = font.glyph(&name) else { continue };
        let props = glyph_props(g, &data, keep);
        let i = &g.info;
        let keys: serde_json::Map<String, Value> =
            i.metrics_keys.all().iter().filter_map(|(k, v)| v.as_ref().map(|v| (k.to_string(), json!(v)))).collect();
        let k = &g.kern_groups;
        out.push(json!({
            "name": g.name,
            "unicodes": g.unicodes.iter().map(|c| hex(*c)).collect::<Vec<_>>(),
            "export": g.export,
            "locked": i.locked,
            "color": color_json(i.color),
            "tags": i.tags,
            "note": i.note,
            "lastChange": i.last_change,
            "metricsKeys": keys,
            "kernGroups": {"left": k.left, "right": k.right, "top": k.top, "bottom": k.bottom},
            "properties": {
                "productionName": prop_json(&props.production_name),
                "category": prop_json(&props.category),
                "subCategory": prop_json(&props.sub_category),
                "case": prop_json(&props.case),
                "direction": prop_json(&props.direction),
                "script": prop_json(&props.script),
                "sortName": prop_json(&props.sort_name),
                "description": prop_json(&props.description),
            },
            "dataUnicode": props.data_unicode.map(hex),
            "hasCustomInfo": props.has_custom_info,
        }));
    }
    Ok(json!({ "glyphs": out }))
}

fn copy_glyph_names(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.copyGlyphNames", v)?;
    let format = p.str("format")?;
    if !COPY_FORMATS.contains(&format) {
        return Err(p.bad(format!("`format` must be one of {}, got \"{format}\"", COPY_FORMATS.join(", "))));
    }
    let font = s.font()?;
    let names = targets_in_order(&p, font)?;
    let text = match format {
        "spaceSeparated" => names.iter().map(|n| format!("{n} ")).collect::<String>(),
        "commaSeparated" => names.iter().map(|n| format!("{n}, ")).collect::<String>(),
        "onePerLine" => names.join("\n"),
        "slashed" => names.iter().map(|n| format!("/{n}")).collect::<String>(),
        "pythonList" => names.iter().map(|n| format!("\"{n}\",")).collect::<Vec<_>>().join("\n"),
        _ => names
            .iter()
            .map(|n| match font.glyph(n).map(|g| g.unicodes.as_slice()) {
                Some(cps) if !cps.is_empty() => cps.iter().map(|c| hex(*c)).collect::<Vec<_>>().join(", "),
                _ => format!("- # {n}"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };
    Ok(json!({ "text": text }))
}

fn validate_glyph_names(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.validateGlyphNames", v)?;
    let font = s.font()?;
    let names = targets_in_order(&p, font)?;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for g in font.glyphs.iter() {
        *counts.entry(g.name.as_str()).or_insert(0) += 1;
    }
    let mut invalid = Vec::new();
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        if let Err(e) = validate(&g.name, g.export) {
            let problem = match e.problem {
                NameProblem::Empty => "empty",
                NameProblem::Character => "character",
                NameProblem::FirstCharacter => "firstCharacter",
            };
            invalid.push(json!({
                "glyph": g.name,
                "problem": problem,
                "position": e.position + 1,
                "character": e.character.map(String::from),
                "message": e.to_string(),
            }));
        }
        if counts.get(g.name.as_str()).is_some_and(|n| *n > 1) {
            invalid.push(json!({
                "glyph": g.name,
                "problem": "duplicate",
                "position": Value::Null,
                "character": Value::Null,
                "message": "another glyph has the same name",
            }));
        }
    }
    Ok(json!({ "checked": names.len(), "invalid": invalid }))
}

fn via(v: Via) -> &'static str {
    match v {
        Via::Name => "name",
        Via::AltName => "altName",
        Via::ProductionName => "productionName",
        Via::CodePoint => "codePoint",
    }
}

fn info_json(info: &Info) -> Value {
    let sourced = |value: Option<String>, source: Option<&'static str>| match (value, source) {
        (Some(value), Some(source)) => json!({ "value": value, "source": source }),
        _ => Value::Null,
    };
    let record = info.found.as_ref().map(|f| &f.record);
    json!({
        "name": info.name,
        "found": info.found.as_ref().map(|f| json!({ "name": f.record.name, "via": via(f.via), "custom": f.custom })),
        "unicode": sourced(info.unicode.as_ref().map(|u| hex(u.value)), info.unicode.as_ref().map(|u| u.source.as_str())),
        "category": sourced(info.category.as_ref().map(|c| c.value.as_str().to_string()), info.category.as_ref().map(|c| c.source.as_str())),
        "subCategory": sourced(info.sub_category.as_ref().map(|c| c.value.as_str().to_string()), info.sub_category.as_ref().map(|c| c.source.as_str())),
        "productionName": sourced(Some(info.production_name.value.clone()), Some(info.production_name.source.as_str())),
        "script": sourced(info.script.as_ref().map(|c| c.value.clone()), info.script.as_ref().map(|c| c.source.as_str())),
        "case": sourced(info.case.as_ref().map(|c| c.value.as_str().to_string()), info.case.as_ref().map(|c| c.source.as_str())),
        "direction": sourced(Some(info.direction.value.as_str().to_string()), Some(info.direction.source.as_str())),
        "description": sourced(info.description.as_ref().map(|c| c.value.clone()), info.description.as_ref().map(|c| c.source.as_str())),
        "altNames": record.map(|r| r.alt_names.clone()).unwrap_or_default(),
        "legacyUnicode": record.and_then(|r| r.unicode_legacy).map(hex),
    })
}

fn lookup(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyphData.lookup", v)?;
    let given = ["name", "unicode", "char"].iter().filter(|k| p.get(k).is_some()).count();
    if given != 1 {
        return Err(p.bad("give exactly one of `name`, `unicode` and `char`"));
    }
    let data = s.glyph_data()?;
    let info = if let Some(name) = p.opt_str("name")? {
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(p.bad(format!("`name` must be from 1 to {MAX_NAME_BYTES} bytes")));
        }
        data.lookup(name, &[])
    } else {
        let cp = if let Some(u) = p.get("unicode") {
            codepoint(u).ok_or_else(|| p.bad(format!("`unicode` must be a code point as hex text or a number, got {u}")))?
        } else {
            let text = p.str("char")?;
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => u32::from(c),
                _ => return Err(p.bad("`char` must be exactly one character")),
            }
        };
        data.lookup(&data.nice_name(cp), &[cp])
    };
    Ok(info_json(&info))
}

fn record_json(r: &Record) -> Value {
    json!({
        "name": r.name,
        "unicode": r.unicode.map(hex),
        "category": r.category.map(Category::as_str),
        "subCategory": r.sub_category.map(SubCategory::as_str),
        "script": r.script,
        "case": r.case.map(Case::as_str),
        "direction": r.direction.map(Direction::as_str),
        "production": r.production,
        "description": r.description,
        "altNames": r.alt_names,
        "legacyUnicode": r.unicode_legacy.map(hex),
    })
}

fn search(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyphData.search", v)?;
    let query = p.str("query")?;
    if query.trim().is_empty() || query.len() > MAX_QUERY_BYTES {
        return Err(p.bad(format!("`query` must be from 1 to {MAX_QUERY_BYTES} bytes and not blank")));
    }
    let sort_by = match p.opt_str("sortBy")? {
        None => None,
        Some(text) => Some(SortBy::parse(text).ok_or_else(|| {
            p.bad(format!("`sortBy` must be name, unicode, category, subCategory, script, description or production, got \"{text}\""))
        })?),
    };
    let max = i64::try_from(MAX_RESULTS).unwrap_or(i64::MAX);
    let limit = p.opt_int("limit", 1, max)?.and_then(|n| usize::try_from(n).ok()).unwrap_or(MAX_RESULTS);
    let data = s.glyph_data()?;
    let results: Vec<Value> = data.search(query, sort_by, limit).iter().map(record_json).collect();
    Ok(json!({ "count": results.len(), "results": results }))
}

fn production_names(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyphData.productionName", v)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let keep = keeps_alternates(font);
    let mut out = Vec::new();
    for name in targets_in_order(&p, font)? {
        let Some(g) = font.glyph(&name) else { continue };
        let prod = glyph_props(g, &data, keep).production_name;
        out.push(json!({ "glyph": g.name, "productionName": prod.value, "source": prod.source.as_str() }));
    }
    Ok(json!({ "glyphs": out }))
}
