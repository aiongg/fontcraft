//! `hints.remove` (M12.27; `fontcraft_tables::dehint`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! A WOFF, WOFF2 or collection input gives a plain font file of the chosen face. Sources are
//! refused for now: clearing their stored hints belongs to the PostScript and TrueType hint
//! commands of the editor.

use std::collections::BTreeSet;
use std::path::PathBuf;

use fontcraft_tables::dehint::{DehintOptions, GaspMode, dehint_with_report};
use fontcraft_tables::sfnt::{parse_tag, tag_str};
use fontcraft_tables::{SfntFont, TableOrder, WriteOptions};
use serde_json::{Value, json};
use skrifa::MetadataProvider as _;

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most items one glyph selector may list.
const MAX_SELECTOR_ITEMS: usize = 65_536;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "hints.remove",
        "Remove Hinting",
        [],
        None,
        "{input: path, face?: int|name, glyphs?: selector, excludeGlyphs?: selector, keepFontHints?: bool, hintingTables?: [tag], keepFlex?: bool, gasp?: \"keep\"|\"smooth\"|\"remove\", output?: path, inPlace?: bool}",
        always,
        remove_hints
    ).files(const { &[FileParam::read("input"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files()]
}

fn remove_hints(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hints.remove", v)?;
    let input = PathBuf::from(p.str("input")?);
    let request = OutputRequest::from_params(&p)?;
    let keep_font_hints = p.bool_or("keepFontHints", false)?;
    let keep_flex = p.bool_or("keepFlex", false)?;
    let gasp = match p.opt_str("gasp")? {
        None => GaspMode::Keep,
        Some(text) => GaspMode::parse(text).map_err(|e| p.bad(e.to_string()))?,
    };
    let hinting_tables = match p.opt_strings("hintingTables")? {
        None => fontcraft_tables::dehint::DEFAULT_HINTING_TABLES.to_vec(),
        Some(tags) => tags.iter().map(|t| parse_tag(t)).collect::<std::result::Result<_, _>>().map_err(|e| p.bad(e.to_string()))?,
    };

    let file = file_io::read_input(&input)?;
    if !file.kind.is_font() {
        return Err(EngineError::Unsupported(format!(
            "removing hints from {}: give a compiled font (sources keep their hints until the hint commands clear them)",
            input.display()
        )));
    }
    let face = face_index(&p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;

    let selected = selection(&p, &font)?;
    let options = DehintOptions { glyphs: selected, keep_font_hints, hinting_tables, keep_flex, gasp };
    let (out, report) = dehint_with_report(&font, &options)?;
    let bytes = fontcraft_tables::write(&out, &WriteOptions { order: TableOrder::Keep })?;

    let extension = match file.kind {
        InputKind::TrueType | InputKind::Cff => None,
        _ if out.is_cff() => Some("otf"),
        _ => Some("ttf"),
    };
    let path = file_io::output_path(&input, &request, ".dehinted", extension)?;
    file_io::write_output(&path, &bytes, &request)?;
    Ok(json!({
        "output": path.display().to_string(),
        "bytes": bytes.len(),
        "glyphsChanged": report.glyphs_changed,
        "tablesRemoved": report.tables_removed.iter().map(|t| tag_str(*t)).collect::<Vec<_>>(),
        "tablesChanged": report.tables_changed.iter().map(|t| tag_str(*t)).collect::<Vec<_>>(),
    }))
}

/// `face`: an index, or a PostScript name looked up in the collection.
pub(super) fn face_index(p: &Params<'_>, bytes: &[u8]) -> Result<u32> {
    match p.get("face") {
        None => Ok(0),
        Some(Value::Number(n)) => {
            n.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| p.bad("`face` must be a face index or a PostScript name"))
        }
        Some(Value::String(name)) => {
            let count = fontcraft_tables::sfnt::face_count(bytes)?;
            for i in 0..count {
                let Ok(font) = skrifa::FontRef::from_index(bytes, i) else { continue };
                let ps = font.localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME).english_or_first().map(|s| s.chars().collect::<String>());
                if ps.as_deref() == Some(name.as_str()) {
                    return Ok(i);
                }
            }
            Err(p.bad(format!("no face is named `{name}`")))
        }
        Some(_) => Err(p.bad("`face` must be a face index or a PostScript name")),
    }
}

/// The glyph ids `glyphs` and `excludeGlyphs` select; `None` when neither is given.
fn selection(p: &Params<'_>, font: &SfntFont) -> Result<Option<BTreeSet<u16>>> {
    let include = selector_items(p, "glyphs")?;
    let exclude = selector_items(p, "excludeGlyphs")?;
    if include.is_none() && exclude.is_none() {
        return Ok(None);
    }
    let bytes = fontcraft_tables::write(font, &WriteOptions::default())?;
    let font_ref = skrifa::FontRef::new(&bytes).map_err(|e| EngineError::Io(format!("the font cannot be read: {e}")))?;
    let names = skrifa::GlyphNames::new(&font_ref);
    let count = names.num_glyphs();
    let mut by_name = std::collections::HashMap::new();
    for (gid, name) in names.iter() {
        if let Ok(g) = u16::try_from(gid.to_u32()) {
            by_name.entry(name.as_str().to_string()).or_insert(g);
        }
    }
    let charmap = font_ref.charmap();
    let resolve = |items: &[String]| -> Result<BTreeSet<u16>> {
        let mut set = BTreeSet::new();
        for item in items {
            resolve_item(item, count, &by_name, &charmap, &mut set).map_err(|m| p.bad(m))?;
        }
        Ok(set)
    };
    let mut set = match &include {
        Some(items) => resolve(items)?,
        None => (0..count).filter_map(|g| u16::try_from(g).ok()).collect(),
    };
    if let Some(items) = &exclude {
        for g in resolve(items)? {
            set.remove(&g);
        }
    }
    Ok(Some(set))
}

/// A selector given as a list of strings or as one comma- or space-separated string.
fn selector_items(p: &Params<'_>, key: &str) -> Result<Option<Vec<String>>> {
    let items: Vec<String> = match p.get(key) {
        None => return Ok(None),
        Some(Value::String(s)) => s.split([',', ' ']).filter(|w| !w.is_empty()).map(str::to_string).collect(),
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| p.bad(format!("`{key}` items must be strings")))).collect::<Result<_>>()?
        }
        Some(_) => return Err(p.bad(format!("`{key}` must be a string or a list of strings"))),
    };
    if items.len() > MAX_SELECTOR_ITEMS {
        return Err(p.bad(format!("`{key}` lists more than {MAX_SELECTOR_ITEMS} items")));
    }
    Ok(Some(items))
}

/// Adds the glyphs one selector item names: `*`, a glyph id (`12`, `gid12`), a range of ids
/// (`3..9`, `3-9`), a code point (`U+0041`) or a glyph name.
fn resolve_item(
    item: &str,
    count: u32,
    by_name: &std::collections::HashMap<String, u16>,
    charmap: &skrifa::charmap::Charmap<'_>,
    set: &mut BTreeSet<u16>,
) -> std::result::Result<(), String> {
    let gid = |text: &str| -> Option<u32> { text.strip_prefix("gid").unwrap_or(text).parse::<u32>().ok() };
    let in_font = |g: u32| -> std::result::Result<u16, String> {
        u16::try_from(g).ok().filter(|_| g < count).ok_or_else(|| format!("glyph {g} does not exist: the font has {count} glyphs"))
    };
    if item == "*" {
        set.extend((0..count).filter_map(|g| u16::try_from(g).ok()));
        return Ok(());
    }
    if let Some(hex) = item.strip_prefix("U+").or_else(|| item.strip_prefix("u+")) {
        let cp = u32::from_str_radix(hex, 16).map_err(|_| format!("`{item}` is not a code point"))?;
        let g = charmap.map(cp).ok_or_else(|| format!("no glyph is mapped to {item}"))?;
        set.insert(in_font(g.to_u32())?);
        return Ok(());
    }
    if let Some(&g) = by_name.get(item) {
        set.insert(g);
        return Ok(());
    }
    if let Some(g) = gid(item) {
        set.insert(in_font(g)?);
        return Ok(());
    }
    let range = item.split_once("..").or_else(|| item.split_once('-'));
    if let Some((a, b)) = range
        && let (Some(a), Some(b)) = (gid(a), gid(b))
    {
        if a > b {
            return Err(format!("the range `{item}` runs backwards"));
        }
        in_font(b)?;
        set.extend((a..=b).filter_map(|g| u16::try_from(g).ok()));
        return Ok(());
    }
    Err(format!("no glyph is named `{item}`"))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use fontcraft_tables::Flavour;

    /// A TrueType font with one instructed glyph, `fpgm` and `prep`, built here.
    fn hinted_font() -> Vec<u8> {
        let mut glyph = Vec::new();
        for v in [1i16, 0, 0, 100, 100, 1] {
            glyph.extend(v.to_be_bytes());
        }
        glyph.extend(1u16.to_be_bytes());
        glyph.push(0xB0);
        glyph.extend([1, 1]);
        for v in [0i16, 100, 0, 100] {
            glyph.extend(v.to_be_bytes());
        }
        glyph.push(0);
        let mut loca = Vec::new();
        for o in [0u16, 0, (glyph.len() / 2) as u16] {
            loca.extend(o.to_be_bytes());
        }
        let mut head = vec![0u8; 54];
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        let mut maxp = vec![0u8; 32];
        maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());
        maxp[20..22].copy_from_slice(&3u16.to_be_bytes());
        let mut font = SfntFont::new(Flavour::TrueType);
        font.insert(*b"head", head);
        font.insert(*b"maxp", maxp);
        font.insert(*b"loca", loca);
        font.insert(*b"glyf", glyph);
        font.insert(*b"fpgm", vec![0xB0, 0, 0x2C, 0x2D]);
        font.insert(*b"prep", vec![0xB0, 1]);
        fontcraft_tables::write(&font, &WriteOptions::default()).unwrap()
    }

    #[test]
    fn hints_remove_writes_a_font_without_hinting_and_leaves_the_input() {
        let dir = std::env::temp_dir().join(format!("fontcraft-dehint-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("A.ttf");
        let original = hinted_font();
        std::fs::write(&input, &original).unwrap();

        let mut session = Session::new();
        let result = session.execute("hints.remove", &json!({"input": input.display().to_string()})).unwrap();
        let output = dir.join("A.dehinted.ttf");
        assert_eq!(result["output"], json!(output.display().to_string()));
        assert_eq!(result["glyphsChanged"], json!(1));
        assert_eq!(result["tablesRemoved"], json!(["fpgm", "prep"]));
        assert_eq!(std::fs::read(&input).unwrap(), original, "the input is never changed");
        let out = fontcraft_tables::read(&std::fs::read(&output).unwrap(), 0).unwrap();
        assert!(out.get(*b"fpgm").is_none() && out.get(*b"prep").is_none());

        let e = session.execute("hints.remove", &json!({"input": input.display().to_string(), "gasp": "bogus"})).unwrap_err();
        assert!(e.to_string().contains("gasp"), "{e}");
        let e = session.execute("hints.remove", &json!({"input": input.display().to_string(), "glyphs": "nosuch"})).unwrap_err();
        assert!(e.to_string().contains("nosuch"), "{e}");
        let kept = session
            .execute(
                "hints.remove",
                &json!({"input": input.display().to_string(), "glyphs": ["gid0"], "output": dir.join("B.ttf").display().to_string()}),
            )
            .unwrap();
        assert_eq!(kept["glyphsChanged"], json!(0), "glyph 0 has no program");
        assert_eq!(kept["tablesRemoved"], json!([]), "a selection keeps font-wide data");

        let source = dir.join("A.glyphs");
        std::fs::write(&source, "{}").unwrap();
        assert!(session.execute("hints.remove", &json!({"input": source.display().to_string()})).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
