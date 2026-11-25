//! Queries on a font file without opening it as a document: `font.inspect`, `tables.list`.
//!
//! Both read the file by path (native builds only) and change nothing. A section of
//! `font.inspect` that cannot be read is reported as `{error}` inside that section instead of
//! failing the query (`font.inspect` in `spec/inventory/20-cli.toml`).

#[cfg(not(target_arch = "wasm32"))]
use serde_json::Map;
use serde_json::{Value, json};
#[cfg(not(target_arch = "wasm32"))]
use skrifa::MetadataProvider;
#[cfg(not(target_arch = "wasm32"))]
use skrifa::raw::TableProvider;
#[cfg(not(target_arch = "wasm32"))]
use skrifa::raw::tables::cmap::CmapSubtable;
use skrifa::raw::types::Tag;
use skrifa::raw::{FileRef, FontRef};
#[cfg(not(target_arch = "wasm32"))]
use skrifa::string::StringId;

use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.inspect",
            "Describe Font",
            [],
            None,
            "{input: path, face?: number, sections?: [\"format\"|\"names\"|\"metrics\"|\"style\"|\"tables\"|\"cmap\"|\"layout\"|\"axes\"|\"instances\"|\"masters\"|\"color\"|\"hinting\"|\"faces\"], allNames?: bool} a summary of a font file or source without opening it; a section that cannot be read holds {error} → {path, format, <section>: …}",
            always,
            inspect
        ).files(const { &[FileParam::read("input")] }),
        cmd!(
            query "tables.list",
            "List Tables",
            [],
            None,
            "{font: path, face?: number|\"all\", sort?: \"file\"|\"tag\"|\"size\"} the table directory of a compiled font, collection, WOFF or WOFF2 file; for a collection, one list per member (all of them by default) with tables the members share marked; a WOFF2 table has no checksum, its offset is in the decompressed table data, and a transformed one gives its stored size as transformLength → {format, faces: [{face, sfntVersion, tables: [{tag, checksum?, offset, length, compressedLength?, transformLength?, shared?}]}]}",
            always,
            tables_list
        ).files(const { &[FileParam::read("font")] }),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bytes(path: &str) -> Result<Vec<u8>> {
    crate::io::read_file(std::path::Path::new(path))
}

#[cfg(target_arch = "wasm32")]
fn read_bytes(_: &str) -> Result<Vec<u8>> {
    Err(EngineError::Unsupported("reading a file by path in the browser".into()))
}

fn hex32(v: u32) -> String {
    format!("{v:08X}")
}

fn tag(t: Tag) -> String {
    t.to_string().trim_end().to_string()
}

// ── tables.list ────────────────────────────────────────────────────────────────────────────────

fn tables_list(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("tables.list", p)?;
    let path = p.str("font")?;
    let sort = p.opt_str("sort")?.unwrap_or("file");
    if !matches!(sort, "file" | "tag" | "size") {
        return Err(p.bad(format!("`sort` must be \"file\", \"tag\" or \"size\", got \"{sort}\"")));
    }
    let face = match p.get("face") {
        None => None,
        Some(Value::String(s)) if s == "all" => None,
        Some(v) => Some(v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a number or \"all\""))?),
    };
    let bytes = read_bytes(path)?;
    let format = fontcraft_decompile::sniff(&bytes).ok_or_else(|| p.bad(format!("{path} is not a compiled font")))?;
    let mut faces = Vec::new();
    if format == "WOFF" {
        let tables = fontcraft_decompile::woff_directory(&bytes)?;
        let mut rows: Vec<(String, u32, u32, u32, Value)> = tables
            .into_iter()
            .map(|t| {
                let row = json!({ "tag": tag(Tag::from_u32(t.tag)), "checksum": hex32(t.checksum), "offset": t.offset, "length": t.length, "compressedLength": t.compressed });
                (tag(Tag::from_u32(t.tag)), t.offset, t.length, t.compressed, row)
            })
            .collect();
        sort_rows(&mut rows, sort);
        faces.push(json!({ "face": 0, "sfntVersion": hex32(u32::from_be_bytes(bytes.get(4..8).and_then(|b| b.try_into().ok()).unwrap_or_default())), "tables": rows.into_iter().map(|r| r.4).collect::<Vec<_>>() }));
        return Ok(json!({ "format": format, "faces": faces }));
    }
    if format == "WOFF2" {
        let dir = fontcraft_decompile::woff2_directory(&bytes)?;
        let mut rows: Vec<(String, u32, u32, u32, Value)> = dir
            .tables
            .iter()
            .map(|t| {
                let name = tag(Tag::new(&t.tag));
                let offset = u32::try_from(t.stream_offset).unwrap_or(u32::MAX);
                let mut row = json!({ "tag": name, "offset": offset, "length": t.orig_length });
                if let (Some(stored), Some(obj)) = (t.transform_length, row.as_object_mut()) {
                    obj.insert("transformLength".into(), json!(stored));
                }
                (name, offset, t.orig_length, 0, row)
            })
            .collect();
        sort_rows(&mut rows, sort);
        faces.push(json!({ "face": 0, "sfntVersion": hex32(dir.flavor), "tables": rows.into_iter().map(|r| r.4).collect::<Vec<_>>() }));
        return Ok(json!({ "format": format, "faces": faces }));
    }
    let bad = |e: skrifa::raw::ReadError| EngineError::Io(format!("{path}: {e}"));
    let members: Vec<(u32, FontRef)> = match FileRef::new(&bytes).map_err(bad)? {
        FileRef::Font(f) => vec![(0, f)],
        FileRef::Collection(c) => {
            let all: Vec<(u32, FontRef)> = (0..c.len()).filter_map(|i| c.get(i).ok().map(|f| (i, f))).collect();
            match face {
                Some(f) => all.into_iter().filter(|(i, _)| *i == f).collect(),
                None => all,
            }
        }
    };
    if members.is_empty() {
        return Err(p.bad(format!("{path} has no font number {}", face.unwrap_or(0))));
    }
    // A table is shared when another member points at the same offset.
    let mut offsets: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (_, f) in &members {
        for r in f.table_directory.table_records() {
            *offsets.entry(r.offset()).or_default() += 1;
        }
    }
    for (i, f) in &members {
        let mut rows: Vec<(String, u32, u32, u32, Value)> = f
            .table_directory
            .table_records()
            .iter()
            .map(|r| {
                let mut row = json!({ "tag": tag(r.tag()), "checksum": hex32(r.checksum()), "offset": r.offset(), "length": r.length() });
                if members.len() > 1
                    && offsets.get(&r.offset()).copied().unwrap_or(0) > 1
                    && let Some(obj) = row.as_object_mut()
                {
                    obj.insert("shared".into(), json!(true));
                }
                (tag(r.tag()), r.offset(), r.length(), 0, row)
            })
            .collect();
        sort_rows(&mut rows, sort);
        faces.push(
            json!({ "face": i, "sfntVersion": hex32(f.table_directory.sfnt_version()), "tables": rows.into_iter().map(|r| r.4).collect::<Vec<_>>() }),
        );
    }
    Ok(json!({ "format": format, "faces": faces }))
}

fn sort_rows(rows: &mut [(String, u32, u32, u32, Value)], sort: &str) {
    match sort {
        "tag" => rows.sort_by(|a, b| a.0.cmp(&b.0)),
        "size" => rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0))),
        _ => rows.sort_by_key(|r| r.1),
    }
}

// ── font.inspect ───────────────────────────────────────────────────────────────────────────────

const SECTIONS: [&str; 13] =
    ["format", "names", "metrics", "style", "tables", "cmap", "layout", "axes", "instances", "masters", "color", "hinting", "faces"];

fn inspect(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.inspect", p)?;
    let path = p.str("input")?;
    let face = p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32);
    let wanted: Vec<&str> = match p.opt_strings("sections")? {
        Some(list) => {
            if let Some(bad) = list.iter().find(|s| !SECTIONS.contains(s)) {
                return Err(p.bad(format!("unknown section `{bad}` (sections: {})", SECTIONS.join(", "))));
            }
            list
        }
        None => SECTIONS.to_vec(),
    };
    let all_names = p.bool_or("allNames", false)?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let held = fontcraft_format::output::held(std::path::Path::new(path)).is_some();
        let folder = !held && std::fs::metadata(path).map_err(|e| EngineError::Io(format!("cannot open {path}: {e}")))?.is_dir();
        let bytes = if folder { Vec::new() } else { read_bytes(path)? };
        if !folder
            && let Some(kind) = fontcraft_decompile::sniff(&bytes)
            && !matches!(kind, "Type 1" | "bare CFF")
        {
            return inspect_binary(path, kind, &bytes, face, &wanted, all_names);
        }
        let (font, _) = crate::io::open(std::path::Path::new(path), face)?;
        Ok(inspect_source(path, &font, &wanted))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, face, wanted, all_names);
        Err(EngineError::Unsupported("reading a file by path in the browser".into()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Puts `f()` under `key` when the section is wanted; an error becomes `{error}`.
fn section(out: &mut Map<String, Value>, wanted: &[&str], key: &str, f: impl FnOnce() -> std::result::Result<Value, String>) {
    if wanted.contains(&key) {
        out.insert(key.to_string(), f().unwrap_or_else(|e| json!({ "error": e })));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn inspect_binary(path: &str, kind: &str, bytes: &[u8], face: Option<u32>, wanted: &[&str], all_names: bool) -> Result<Value> {
    let unwrapped = fontcraft_decompile::to_sfnt(bytes)?;
    let sfnt = unwrapped.as_ref();
    let bad = |e: skrifa::raw::ReadError| EngineError::Io(format!("{path}: {e}"));
    let (font, member_count) = match FileRef::new(sfnt).map_err(bad)? {
        FileRef::Font(f) => (f, 1),
        FileRef::Collection(c) => (c.get(face.unwrap_or(0)).map_err(bad)?, c.len()),
    };
    let mut out = Map::new();
    out.insert("path".into(), json!(path));
    let e = |x: skrifa::raw::ReadError| x.to_string();
    section(&mut out, wanted, "format", || {
        Ok(
            json!({ "kind": kind, "sfntVersion": hex32(font.table_directory.sfnt_version()), "tables": font.table_directory.table_records().len(), "faces": member_count }),
        )
    });
    section(&mut out, wanted, "names", || {
        if all_names {
            let name = font.name().map_err(e)?;
            let records: Vec<Value> = name
                .name_record()
                .iter()
                .map(|r| {
                    let text = r.string(name.string_data()).map(|s| s.chars().collect::<String>()).unwrap_or_default();
                    json!({ "nameId": r.name_id().to_u16(), "platform": r.platform_id(), "encoding": r.encoding_id(), "language": r.language_id(), "string": text })
                })
                .collect();
            return Ok(json!(records));
        }
        let ids = [
            ("copyright", StringId::COPYRIGHT_NOTICE),
            ("family", StringId::FAMILY_NAME),
            ("subfamily", StringId::SUBFAMILY_NAME),
            ("full", StringId::FULL_NAME),
            ("version", StringId::VERSION_STRING),
            ("postscript", StringId::POSTSCRIPT_NAME),
            ("typographicFamily", StringId::TYPOGRAPHIC_FAMILY_NAME),
            ("typographicSubfamily", StringId::TYPOGRAPHIC_SUBFAMILY_NAME),
            ("designer", StringId::DESIGNER),
            ("manufacturer", StringId::MANUFACTURER),
            ("license", StringId::LICENSE_DESCRIPTION),
        ];
        let mut m = Map::new();
        for (key, id) in ids {
            let mut strings = font.localized_strings(id);
            let english = strings.clone().find(|s| s.language().is_some_and(|l| l.starts_with("en")));
            if let Some(s) = english.or_else(|| strings.next()) {
                m.insert(key.into(), json!(s.to_string()));
            }
        }
        Ok(Value::Object(m))
    });
    section(&mut out, wanted, "metrics", || {
        let head = font.head().map_err(e)?;
        let mut m = Map::new();
        m.insert("unitsPerEm".into(), json!(head.units_per_em()));
        m.insert("bounds".into(), json!([head.x_min(), head.y_min(), head.x_max(), head.y_max()]));
        if let Ok(h) = font.hhea() {
            m.insert(
                "hhea".into(),
                json!({ "ascender": h.ascender().to_i16(), "descender": h.descender().to_i16(), "lineGap": h.line_gap().to_i16() }),
            );
        }
        if let Ok(o) = font.os2() {
            m.insert("typo".into(), json!({ "ascender": o.s_typo_ascender(), "descender": o.s_typo_descender(), "lineGap": o.s_typo_line_gap() }));
            m.insert("win".into(), json!({ "ascent": o.us_win_ascent(), "descent": o.us_win_descent() }));
            m.insert("xHeight".into(), json!(o.sx_height()));
            m.insert("capHeight".into(), json!(o.s_cap_height()));
        }
        if let Ok(post) = font.post() {
            m.insert("italicAngle".into(), json!(post.italic_angle().to_f64()));
            m.insert("underline".into(), json!({ "position": post.underline_position().to_i16(), "thickness": post.underline_thickness().to_i16() }));
        }
        Ok(Value::Object(m))
    });
    section(&mut out, wanted, "style", || {
        let mut m = Map::new();
        if let Ok(o) = font.os2() {
            m.insert("weightClass".into(), json!(o.us_weight_class()));
            m.insert("widthClass".into(), json!(o.us_width_class()));
            m.insert("fsSelection".into(), json!(format!("{:#06x}", o.fs_selection().bits())));
            m.insert("fsType".into(), json!(format!("{:#06x}", o.fs_type())));
            m.insert("vendor".into(), json!(tag(o.ach_vend_id())));
        }
        if let Ok(head) = font.head() {
            m.insert("macStyle".into(), json!(format!("{:#06x}", head.mac_style().bits())));
        }
        Ok(Value::Object(m))
    });
    section(&mut out, wanted, "tables", || {
        Ok(json!(font.table_directory.table_records().iter().map(|r| json!({ "tag": tag(r.tag()), "length": r.length() })).collect::<Vec<_>>()))
    });
    section(&mut out, wanted, "cmap", || {
        let cmap = font.cmap().map_err(e)?;
        let rows: Vec<Value> = cmap
            .encoding_records()
            .iter()
            .map(|r| {
                let format = match r.subtable(cmap.offset_data()) {
                    Ok(CmapSubtable::Format0(_)) => json!(0),
                    Ok(CmapSubtable::Format2(_)) => json!(2),
                    Ok(CmapSubtable::Format4(_)) => json!(4),
                    Ok(CmapSubtable::Format6(_)) => json!(6),
                    Ok(CmapSubtable::Format8(_)) => json!(8),
                    Ok(CmapSubtable::Format10(_)) => json!(10),
                    Ok(CmapSubtable::Format12(_)) => json!(12),
                    Ok(CmapSubtable::Format13(_)) => json!(13),
                    Ok(CmapSubtable::Format14(_)) => json!(14),
                    Err(err) => json!({ "error": err.to_string() }),
                };
                json!({ "platform": r.platform_id() as u16, "encoding": r.encoding_id(), "format": format })
            })
            .collect();
        Ok(json!({ "subtables": rows, "mappings": font.charmap().mappings().count() }))
    });
    section(&mut out, wanted, "layout", || {
        let mut m = Map::new();
        let tags = |records: &[skrifa::raw::tables::layout::FeatureRecord]| {
            let mut t: Vec<String> = records.iter().map(|r| tag(r.feature_tag())).collect();
            t.sort();
            t.dedup();
            t
        };
        if let Ok(gsub) = font.gsub() {
            let scripts: Vec<String> =
                gsub.script_list().map(|s| s.script_records().iter().map(|r| tag(r.script_tag())).collect()).unwrap_or_default();
            let features = gsub.feature_list().map(|f| tags(f.feature_records())).unwrap_or_default();
            m.insert("GSUB".into(), json!({ "scripts": scripts, "features": features }));
        }
        if let Ok(gpos) = font.gpos() {
            let scripts: Vec<String> =
                gpos.script_list().map(|s| s.script_records().iter().map(|r| tag(r.script_tag())).collect()).unwrap_or_default();
            let features = gpos.feature_list().map(|f| tags(f.feature_records())).unwrap_or_default();
            m.insert("GPOS".into(), json!({ "scripts": scripts, "features": features }));
        }
        Ok(Value::Object(m))
    });
    let name_of = |id: StringId| font.localized_strings(id).next().map(|s| s.to_string());
    section(&mut out, wanted, "axes", || {
        Ok(json!(font
            .axes()
            .iter()
            .map(|a| json!({ "tag": tag(a.tag()), "name": name_of(a.name_id()), "min": a.min_value(), "default": a.default_value(), "max": a.max_value(), "hidden": a.is_hidden() }))
            .collect::<Vec<_>>()))
    });
    section(&mut out, wanted, "instances", || {
        Ok(json!(
            font.named_instances()
                .iter()
                .map(|i| json!({ "name": name_of(i.subfamily_name_id()), "coordinates": i.user_coords().collect::<Vec<_>>() }))
                .collect::<Vec<_>>()
        ))
    });
    let has = |t: &[u8; 4]| font.table_data(Tag::new(t)).is_some();
    section(&mut out, wanted, "color", || {
        let present: Vec<&str> = [("COLR", b"COLR"), ("CPAL", b"CPAL"), ("SVG", b"SVG "), ("sbix", b"sbix"), ("CBDT", b"CBDT"), ("CBLC", b"CBLC")]
            .into_iter()
            .filter(|(_, t)| has(t))
            .map(|(n, _)| n)
            .collect();
        Ok(json!({ "tables": present }))
    });
    section(&mut out, wanted, "hinting", || {
        let tt: Vec<&str> =
            [("fpgm", b"fpgm"), ("prep", b"prep"), ("cvt", b"cvt "), ("gasp", b"gasp")].into_iter().filter(|(_, t)| has(t)).map(|(n, _)| n).collect();
        Ok(json!({ "truetypeTables": tt, "cff": has(b"CFF ") || has(b"CFF2") }))
    });
    section(&mut out, wanted, "faces", || Ok(json!({ "count": member_count, "selected": face.unwrap_or(0) })));
    section(&mut out, wanted, "masters", || Err("a compiled font has no masters".into()));
    Ok(Value::Object(out))
}

#[cfg(not(target_arch = "wasm32"))]
fn inspect_source(path: &str, font: &fontcraft_model::Font, wanted: &[&str]) -> Value {
    use fontcraft_model::{SourceFormat, metric};
    let mut out = Map::new();
    out.insert("path".into(), json!(path));
    section(&mut out, wanted, "format", || {
        let kind = match font.source.format {
            SourceFormat::Native => "fontcraft",
            SourceFormat::Glyphs => "glyphs",
            SourceFormat::GlyphsPackage => "glyphspackage",
            SourceFormat::Ufo => "ufo",
            SourceFormat::Ufoz => "ufoz",
            SourceFormat::Designspace => "designspace",
            SourceFormat::Binary => "binary",
        };
        let version = font.source.glyphs_version.or(font.source.ufo_version);
        Ok(json!({ "kind": kind, "version": version, "glyphs": font.glyphs.len(), "masters": font.masters.len() }))
    });
    section(&mut out, wanted, "names", || {
        Ok(json!({ "family": font.family_name, "version": format!("{}.{:03}", font.version_major, font.version_minor) }))
    });
    section(&mut out, wanted, "metrics", || Ok(json!({ "unitsPerEm": font.units_per_em })));
    section(&mut out, wanted, "axes", || {
        Ok(json!(font.axes.iter().map(|a| json!({ "name": a.name, "tag": a.tag, "hidden": a.hidden })).collect::<Vec<_>>()))
    });
    section(&mut out, wanted, "masters", || {
        Ok(json!(
            font.masters
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id, "name": m.name, "location": m.location,
                        "ascender": font.metric_pos(m, metric::ASCENDER, 0.0), "capHeight": font.metric_pos(m, metric::CAP_HEIGHT, 0.0),
                        "xHeight": font.metric_pos(m, metric::X_HEIGHT, 0.0), "descender": font.metric_pos(m, metric::DESCENDER, 0.0),
                    })
                })
                .collect::<Vec<_>>()
        ))
    });
    section(&mut out, wanted, "instances", || {
        Ok(json!(font.instances.iter().map(|i| json!({ "name": i.name, "location": i.location, "exports": i.exports })).collect::<Vec<_>>()))
    });
    section(&mut out, wanted, "layout", || {
        let tags: Vec<&str> = font.features.features.iter().map(|f| f.name.as_str()).collect();
        Ok(
            json!({ "features": tags, "classes": font.features.classes.len(), "prefixes": font.features.prefixes.len(), "kerningPairs": font.kerning.ltr.pair_count() + font.kerning.rtl.pair_count() + font.kerning.vertical.pair_count() }),
        )
    });
    for compiled_only in ["style", "tables", "cmap", "color", "hinting", "faces"] {
        section(&mut out, wanted, compiled_only, || Err("only compiled fonts have this section".into()));
    }
    Value::Object(out)
}
