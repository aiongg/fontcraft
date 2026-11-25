//! `font.dumpTables`: TTX, JSON and hex dumps and the text views (M12.4, M12.10; `fontcraft_tables::ttx`, `fontcraft_tables::text`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::{CommandSpec, FileParam};

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = ttx_specs();
    v.extend(view_specs());
    v
}

/// `font.dumpTables` in TTX, JSON and hex (M12.4).
fn ttx_specs() -> Vec<CommandSpec> {
    vec![super::cmd!(
        query "font.dumpTables",
        "Dump Tables",
        [],
        None,
        "{font: path, face?: number, tables?: [tag | {tag, view?}], excludeTables?: [tag], format?: \"ttx\"|\"json\"|\"hex\"|\"text\", split?: \"tables\"|\"glyphs\", output?: path, outputDir?: path, noClobber?: bool, disassemble?: bool, bitmapFormat?: \"raw\"|\"row\"|\"bitwise\"|\"extfile\", strict?: bool, newline?: \"lf\"|\"cr\"|\"crlf\", normalize?: bool, version?: bool, hints?: bool, showExtensions?: bool, glyphs?: [name] | string, excludeGlyphs?: [name] | string, glyphDetail?: \"short\"|\"medium\"|\"long\", cmapSubtable?: number, units?: \"font\"|\"1000\", cffParts?: string, cffLevel?: \"offsets\"|\"bytes\"|\"values\", wrap?: bool, allowVirtualGids?: bool, unicodeData?: path, fd?: [int], excludeFd?: [int], flattenSeac?: bool, names?: \"source\"|\"final\", goadb?: path} the tables of a compiled font, collection face, WOFF or WOFF2 file as TTX (the XML of the reference dump tool), the same tree as JSON, or hex; tables that cannot be read are written as hex unless strict; tables leaves the glyph order out; split writes one file per table beside the main file (glyphs also one file per glyph outline); bitmapFormat writes bitmap images as hex (raw, the default), hex per pixel row (row), a character per pixel (bitwise) or one file each under bitmaps/strikeN/ beside the dump (extfile, TTX with an output only; the dump and its images are written to temporary files first and moved into place once all are written: a failure while writing them changes nothing, and a failure while moving them leaves the files moved before it in place and removes the rest). Text views: a table with a view ({tag: \"GPOS\", view: \"fea\"|\"normalized\"|\"features\"}; the listings cmap hex-gid, dec-gid, hex-name, dec-name and summary (cmapSubtable picks one encoding record), hmtx advances and advances-lsb (units \"1000\" rescales), name raw, lines and described, GSUB and GPOS features, glyf points, VORG names, kern pairs; CFF literal: the CFF structures, cffParts letters h n t s g e C f F c p l or \"all\", cffLevel, wrap false keeps flowed data on one line), the pseudo-tables @font (the font program's DICTs) and @glyphs (the glyphs given, every glyph by default, less excludeGlyphs: index, name, encoding and, with glyphDetail medium or long, the path with hints unless hints is false), written as text. avar with the view designspace writes the fvar axes and the avar maps as a designspace snippet. For @glyphs: fd and excludeFd keep or leave out the glyphs of these font dicts of a CID-keyed CFF or a CFF2 font (indexes from 0); an accented seac glyph prints its seac operator, and flattenSeac draws it out (the base glyph with its hints, then the moved accent without them); goadb maps the names shown through a glyph name database, final names to source names (names \"source\", the default with goadb) or source names to final names (names \"final\"), and glyphs and excludeGlyphs use the names shown. unicodeData (a UnicodeData.txt file; TTX only) writes each character's name as a comment after the mappings of Unicode cmap subtables, ???? for a code point the file does not name; no names are bundled, so without it no comments are written. allowVirtualGids is accepted and changes nothing: glyph indices past the glyph count are always written as glyphNNNNN. normalize writes the normalised dump: tables sorted, values that always differ zeroed, outlines as text paths, GSUB and GPOS as rules (version false also blanks the version fields, showExtensions marks extension lookups) → {format, text} or {format, json} without an output, {format, paths, images} with one (images: how many bitmap images extfile wrote to files)",
        super::always,
        ttx::run
    ).files(const {
        &[
            FileParam::read("font"),
            FileParam::read("unicodeData"),
            FileParam::read("goadb"),
            FileParam::write("output"),
            FileParam::write("outputDir"),
        ]
    }).writes_files()]
}

mod ttx {
    use std::path::{Path, PathBuf};

    use fontcraft_tables::sfnt::parse_tag;
    use fontcraft_tables::ttx::{BitmapForm, DumpFormat, DumpOptions, ImageFile, dump_split_with_images, dump_with_images};
    use serde_json::{Value, json};

    use super::super::file_io::{OutputRequest, output_path, read_input, write_output};
    use crate::params::Params;
    use crate::{EngineError, Result, Session};

    pub(super) fn run(_: &mut Session, params: &Value) -> Result<Value> {
        if let Some(result) = super::views::run(params)? {
            return Ok(result);
        }
        let p = Params::new("font.dumpTables", params)?;
        let path = Path::new(p.str("font")?);
        let face = p.opt_int("face", 0, 255)?.unwrap_or(0) as u32;
        let format_name = p.opt_str("format")?.unwrap_or("ttx");
        let format = match format_name {
            "ttx" => DumpFormat::Ttx,
            "json" => DumpFormat::Json,
            "hex" => DumpFormat::Hex,
            "text" => return Err(EngineError::Unsupported("the text views of `font.dumpTables` are not built yet".into())),
            other => return Err(p.bad(format!("`format` must be \"ttx\", \"json\", \"hex\" or \"text\", got \"{other}\""))),
        };
        let mut tables = Vec::new();
        for item in p.opt_array("tables")?.map(Vec::as_slice).unwrap_or_default() {
            let (tag, view) = match item {
                Value::String(s) => (s.as_str(), None),
                Value::Object(o) => (
                    o.get("tag").and_then(Value::as_str).ok_or_else(|| p.bad("each item of `tables` needs a `tag`"))?,
                    o.get("view").and_then(Value::as_str),
                ),
                _ => return Err(p.bad("`tables` holds tags or {tag, view} objects")),
            };
            if tag.starts_with('@') || view.is_some_and(|v| v != "raw") {
                return Err(EngineError::Unsupported("table views and the @font and @glyphs pseudo-tables are not built yet".into()));
            }
            tables.push(parse_tag(tag).map_err(|e| p.bad(format!("`tables`: {e}")))?);
        }
        let mut skip = Vec::new();
        for tag in p.opt_strings("excludeTables")?.unwrap_or_default() {
            skip.push(parse_tag(tag).map_err(|e| p.bad(format!("`excludeTables`: {e}")))?);
        }
        let (split, split_glyphs) = match p.opt_str("split")? {
            None => (false, false),
            Some("tables") => (true, false),
            Some("glyphs") => (true, true),
            Some(other) => return Err(p.bad(format!("`split` must be \"tables\" or \"glyphs\", got \"{other}\""))),
        };
        if split && format != DumpFormat::Ttx {
            return Err(p.bad("`split` works with the TTX format only"));
        }
        let newline = match p.opt_str("newline")?.unwrap_or("lf") {
            "lf" => "\n",
            "cr" => "\r",
            "crlf" => "\r\n",
            other => return Err(p.bad(format!("`newline` must be \"lf\", \"cr\" or \"crlf\", got \"{other}\""))),
        };
        if p.bool_or("normalize", false)? {
            return Err(EngineError::Unsupported("the normalised dump is not built yet".into()));
        }
        let bitmap_form = match p.opt_str("bitmapFormat")?.unwrap_or("raw") {
            "raw" => BitmapForm::Raw,
            "row" => BitmapForm::Row,
            "bitwise" => BitmapForm::Bitwise,
            "extfile" => BitmapForm::ExtFile,
            other => return Err(p.bad(format!("`bitmapFormat` must be \"raw\", \"row\", \"bitwise\" or \"extfile\", got \"{other}\""))),
        };
        if bitmap_form != BitmapForm::Raw && format == DumpFormat::Hex {
            return Err(p.bad("`bitmapFormat` applies to the TTX and JSON formats"));
        }
        if bitmap_form == BitmapForm::ExtFile && format != DumpFormat::Ttx {
            return Err(p.bad("`bitmapFormat` \"extfile\" works with the TTX format only"));
        }
        if let Some(key) = super::extras::glyphs_only_given(&p) {
            return Err(p.bad(format!("`{key}` applies to the @glyphs pseudo-table: name it in `tables`")));
        }
        super::extras::check_virtual_gids(&p)?;
        let unicode_names = super::extras::unicode_names(&p)?;
        if unicode_names.is_some() && format != DumpFormat::Ttx {
            return Err(p.bad("`unicodeData` writes comments, which only the TTX format has"));
        }
        let options = DumpOptions {
            format,
            tables,
            skip,
            hex: Vec::new(),
            no_version: false,
            no_disassemble: !p.bool_or("disassemble", true)?,
            strict: p.bool_or("strict", false)?,
            bitmap_form,
            unicode_names,
        };
        let request = OutputRequest::from_params(&p)?;
        if request.in_place {
            return Err(p.bad("a dump never replaces its input: give `output` or `outputDir`"));
        }
        let writes = request.output.is_some() || request.output_dir.is_some();
        if split && !writes {
            return Err(p.bad("`split` writes files: give `output` or `outputDir`"));
        }
        if bitmap_form == BitmapForm::ExtFile && !writes {
            return Err(p.bad("`bitmapFormat` \"extfile\" writes image files: give `output` or `outputDir`"));
        }

        let input = read_input(path)?;
        if !input.kind.is_font() {
            return Err(p.bad(format!("{} is not a compiled font ({})", path.display(), input.kind.name())));
        }
        let font = fontcraft_tables::read(&input.bytes, face)?;
        let ext = match format {
            DumpFormat::Ttx => "ttx",
            DumpFormat::Json => "json",
            DumpFormat::Hex => "txt",
        };
        let lines = |text: String| if newline == "\n" { text } else { text.replace('\n', newline) };

        if split {
            let main = output_path(path, &request, "", Some(ext))?;
            let stem = main.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "dump".into());
            let dir = main.parent().map(Path::to_path_buf).unwrap_or_default();
            let (parts, images) = dump_split_with_images(&font, &options, &stem, &format!(".{ext}"), split_glyphs)?;
            let parts: Vec<(PathBuf, Vec<u8>)> = parts.into_iter().map(|(name, text)| (dir.join(name), lines(text).into_bytes())).collect();
            return written(format_name, parts, &dir, images, &request);
        }
        let (text, images) = dump_with_images(&font, &options)?;
        if writes {
            let target = output_path(path, &request, "", Some(ext))?;
            let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
            return written(format_name, vec![(target, lines(text).into_bytes())], &dir, images, &request);
        }
        if format == DumpFormat::Json {
            let tree: Value = serde_json::from_str(&text).map_err(|e| EngineError::Io(format!("the JSON dump could not be read back: {e}")))?;
            return Ok(json!({ "format": format_name, "json": tree }));
        }
        Ok(json!({ "format": format_name, "text": lines(text) }))
    }

    /// Writes the dump's files and the image files of its bitmap tables, which go below `dir`,
    /// the folder of the main file. Without images each file is written atomically on its own;
    /// with them every file is staged first and moved into place once all are written
    /// ([`super::staged::write_all`]).
    fn written(format_name: &str, files: Vec<(PathBuf, Vec<u8>)>, dir: &Path, images: Vec<ImageFile>, request: &OutputRequest) -> Result<Value> {
        let paths: Vec<String> = files.iter().map(|(p, _)| p.display().to_string()).collect();
        let count = images.len();
        if images.is_empty() {
            for (target, bytes) in &files {
                write_output(target, bytes, request)?;
            }
        } else {
            let mut all = files;
            all.extend(images.into_iter().map(|image| (dir.join(image.path), image.bytes)));
            super::staged::write_all(all, request)?;
        }
        Ok(json!({ "format": format_name, "paths": paths, "images": count }))
    }

    #[cfg(test)]
    #[cfg(not(target_arch = "wasm32"))]
    mod tests {
        use super::*;

        /// A font with `maxp`, `cvt ` and an unknown table, written to a temporary folder.
        fn font_file(dir: &Path) -> PathBuf {
            let mut font = fontcraft_tables::SfntFont::new(fontcraft_tables::Flavour::TrueType);
            font.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 2]);
            font.insert(*b"cvt ", vec![0, 10, 0xFF, 0xF6]);
            font.insert(*b"zzzz", b"data".to_vec());
            let bytes = fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions::default()).unwrap();
            let path = dir.join("Dump.ttf");
            std::fs::write(&path, bytes).unwrap();
            path
        }

        #[test]
        fn font_dump_tables_writes_ttx_json_hex_and_files() {
            let dir = std::env::temp_dir().join(format!("fontcraft-dump-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let font = font_file(&dir);
            let f = font.display().to_string();
            let mut s = Session::new();

            let ttx = s.execute("font.dumpTables", &json!({"font": f})).unwrap();
            let text = ttx["text"].as_str().unwrap();
            assert!(text.contains("<GlyphID id=\"1\" name=\"glyph00001\"/>"), "{text}");
            assert!(text.contains("<cv index=\"1\" value=\"-10\"/>"), "{text}");
            assert!(text.contains("<zzzz raw=\"True\">"), "{text}");

            let only = s.execute("font.dumpTables", &json!({"font": f, "tables": ["cvt"], "format": "json"})).unwrap();
            assert_eq!(only["json"]["children"][0]["name"], "cvt");

            let hex = s.execute("font.dumpTables", &json!({"font": f, "tables": [{"tag": "zzzz"}], "format": "hex"})).unwrap();
            assert!(hex["text"].as_str().unwrap().starts_with("'zzzz' 4 bytes\n00000000  64 61 74 61"));

            let out = dir.join("out.ttx");
            let written = s.execute("font.dumpTables", &json!({"font": f, "output": out.display().to_string(), "newline": "crlf"})).unwrap();
            assert_eq!(written["paths"][0], out.display().to_string());
            assert!(std::fs::read_to_string(&out).unwrap().contains("</ttFont>\r\n"));

            let split = s.execute("font.dumpTables", &json!({"font": f, "split": "tables", "outputDir": dir.display().to_string()})).unwrap();
            assert_eq!(split["paths"].as_array().unwrap().len(), 5, "main file, glyph order and three tables");
            assert!(dir.join("Dump._c_v_t.ttx").exists());

            for bad in [
                json!({"font": f, "format": "xml"}),
                json!({"font": f, "tables": ["cvt"], "excludeTables": ["maxp"]}),
                json!({"font": f, "split": "tables"}),
            ] {
                assert!(s.execute("font.dumpTables", &bad).is_err(), "{bad}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// The text views, `@font`, `@glyphs` and `--normalize` (M12.10). They are options of
/// `font.dumpTables`, which [`ttx::run`] hands to [`views::run`] first; no command of their own.
fn view_specs() -> Vec<CommandSpec> {
    Vec::new()
}

/// `font.dumpTables` with `format: "text"`, a table view, `@font`, `@glyphs` or `normalize`.
mod views {
    use std::path::Path;

    use fontcraft_tables::sfnt::parse_tag;
    use fontcraft_tables::text::{CffLevel, FONT_VIEW, GLYPHS_VIEW, GlyphDetail, ViewOptions, table_offsets, view};
    use serde_json::{Value, json};

    use super::super::file_io::{OutputRequest, output_path, read_input, write_output};
    use crate::params::Params;
    use crate::{EngineError, Result};

    /// One item of `tables`: a tag or pseudo-table and its view.
    struct Item {
        name: String,
        view: Option<String>,
    }

    /// The result of the request when it asks for a text view or the normalised dump; `None`
    /// leaves it to the TTX, JSON and hex dump.
    pub(super) fn run(params: &Value) -> Result<Option<Value>> {
        let p = Params::new("font.dumpTables", params)?;
        let mut items = Vec::new();
        for item in p.opt_array("tables")?.map(Vec::as_slice).unwrap_or_default() {
            let (name, view) = match item {
                Value::String(s) => (s.clone(), None),
                Value::Object(o) => (
                    o.get("tag").and_then(Value::as_str).ok_or_else(|| p.bad("each item of `tables` needs a `tag`"))?.to_string(),
                    o.get("view").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string),
                ),
                _ => return Err(p.bad("`tables` holds tags or {tag, view} objects")),
            };
            items.push(Item { name, view });
        }
        let normalize = p.bool_or("normalize", false)?;
        let format = p.opt_str("format")?.unwrap_or("ttx");
        let viewed = items.iter().any(|i| i.name.starts_with('@') || i.view.as_deref().is_some_and(|v| v != "raw"));
        if !normalize && !viewed && format != "text" {
            return Ok(None);
        }
        match (normalize, format) {
            (true, "ttx") => {}
            (true, _) => return Err(p.bad("the normalised dump is written as TTX: leave `format` out or give \"ttx\"")),
            (false, "ttx" | "text") => {}
            (false, _) => return Err(p.bad("table views, @font and @glyphs are text: give `format` \"text\"")),
        }
        if p.opt_str("split")?.is_some() {
            return Err(EngineError::Unsupported("splitting the text views or the normalised dump into files is not built yet".into()));
        }
        if p.opt_array("excludeTables")?.is_some_and(|v| !v.is_empty()) && !items.is_empty() {
            return Err(p.bad("give the tables to write or the tables to leave out, not both"));
        }
        super::extras::check_virtual_gids(&p)?;
        if super::extras::given(&p, "unicodeData") {
            return Err(p.bad("`unicodeData` writes the comments of the TTX dump: text views and the normalised dump have none"));
        }
        if let Some(key) = super::extras::glyphs_only_given(&p)
            && (normalize || !items.iter().any(|i| i.name == GLYPHS_VIEW))
        {
            return Err(p.bad(format!("`{key}` applies to the @glyphs pseudo-table: name it in `tables`")));
        }
        let fd = super::extras::font_dicts(&p, "fd")?;
        let exclude_fd = super::extras::font_dicts(&p, "excludeFd")?;
        let flatten_seac = p.bool_or("flattenSeac", false)?;
        let rename = super::extras::rename(&p)?;
        let names = |key: &str| -> Result<Vec<String>> {
            Ok(match p.get(key) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::String(s)) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or_else(|| p.bad(format!("`{key}` holds glyph names"))))
                    .collect::<Result<_>>()?,
                Some(_) => return Err(p.bad(format!("`{key}` is a list of glyph names or a comma-separated string"))),
            })
        };
        let glyphs = names("glyphs")?;
        let exclude_glyphs = names("excludeGlyphs")?;
        let glyph_detail = match p.opt_str("glyphDetail")?.unwrap_or("short") {
            "short" => GlyphDetail::Short,
            "medium" => GlyphDetail::Medium,
            "long" => GlyphDetail::Long,
            other => return Err(p.bad(format!("`glyphDetail` must be \"short\", \"medium\" or \"long\", got \"{other}\""))),
        };
        let cff_level = match p.opt_str("cffLevel")?.unwrap_or("values") {
            "offsets" => CffLevel::Offsets,
            "bytes" => CffLevel::Bytes,
            "values" => CffLevel::Values,
            other => return Err(p.bad(format!("`cffLevel` must be \"offsets\", \"bytes\" or \"values\", got \"{other}\""))),
        };
        let units_1000 = match p.opt_str("units")?.unwrap_or("font") {
            "font" => false,
            "1000" => true,
            other => return Err(p.bad(format!("`units` must be \"font\" or \"1000\", got \"{other}\""))),
        };
        let cff_parts = match p.opt_str("cffParts")?.unwrap_or("") {
            "all" => String::new(),
            parts => parts.to_string(),
        };
        let base = ViewOptions {
            glyphs,
            exclude_glyphs,
            glyph_detail,
            cmap_subtable: p.opt_int("cmapSubtable", 0, 65_535)?.map(|n| n as usize),
            units_1000,
            cff_parts,
            cff_level,
            no_wrap: !p.bool_or("wrap", true)?,
            no_version: !p.bool_or("version", true)?,
            no_hints: !p.bool_or("hints", true)?,
            show_extensions: p.bool_or("showExtensions", false)?,
            fd,
            exclude_fd,
            flatten_seac,
            rename,
            ..ViewOptions::default()
        };
        let newline = match p.opt_str("newline")?.unwrap_or("lf") {
            "lf" => "\n",
            "cr" => "\r",
            "crlf" => "\r\n",
            other => return Err(p.bad(format!("`newline` must be \"lf\", \"cr\" or \"crlf\", got \"{other}\""))),
        };
        let request = OutputRequest::from_params(&p)?;
        if request.in_place {
            return Err(p.bad("a dump never replaces its input: give `output` or `outputDir`"));
        }

        let path = Path::new(p.str("font")?);
        let face = p.opt_int("face", 0, 255)?.unwrap_or(0) as u32;
        let input = read_input(path)?;
        if !input.kind.is_font() {
            return Err(p.bad(format!("{} is not a compiled font ({})", path.display(), input.kind.name())));
        }
        let font = fontcraft_tables::read(&input.bytes, face)?;
        let base = ViewOptions { table_offsets: table_offsets(&input.bytes, face), ..base };

        let mut text = String::new();
        if normalize {
            let mut tables = Vec::new();
            for item in &items {
                if item.name.starts_with('@') || item.view.as_deref().is_some_and(|v| v != "raw") {
                    return Err(p.bad("the normalised dump takes tables without views"));
                }
                tables.push(parse_tag(&item.name).map_err(|e| p.bad(format!("`tables`: {e}")))?);
            }
            if tables.is_empty() {
                let mut skip = Vec::new();
                for tag in p.opt_strings("excludeTables")?.unwrap_or_default() {
                    skip.push(parse_tag(tag).map_err(|e| p.bad(format!("`excludeTables`: {e}")))?);
                }
                if !skip.is_empty() {
                    tables = font.tables.keys().copied().filter(|t| !skip.contains(t)).collect();
                }
            }
            text = view(&font, &ViewOptions { normalize: true, tables, ..base })?;
        } else {
            if items.is_empty() {
                return Err(p.bad("the text format writes table views: name the tables, with views, or @font and @glyphs"));
            }
            for item in &items {
                let options = match item.name.as_str() {
                    FONT_VIEW | GLYPHS_VIEW => ViewOptions { view: item.name.clone(), ..base.clone() },
                    name if name.starts_with('@') => return Err(p.bad(format!("`{name}` is not a pseudo-table: give @font or @glyphs"))),
                    name => ViewOptions {
                        view: item.view.clone().unwrap_or_else(|| "raw".into()),
                        tables: vec![parse_tag(name).map_err(|e| p.bad(format!("`tables`: {e}")))?],
                        ..base.clone()
                    },
                };
                text.push_str(&view(&font, &options)?);
            }
        }
        let text = if newline == "\n" { text } else { text.replace('\n', newline) };
        let format_name = if normalize { "ttx" } else { "text" };
        if request.output.is_some() || request.output_dir.is_some() {
            let ext = if normalize { "ttx" } else { "txt" };
            let target = output_path(path, &request, "", Some(ext))?;
            write_output(&target, text.as_bytes(), &request)?;
            return Ok(Some(json!({ "format": format_name, "paths": [target.display().to_string()] })));
        }
        Ok(Some(json!({ "format": format_name, "text": text })))
    }

    #[cfg(test)]
    #[cfg(not(target_arch = "wasm32"))]
    mod tests {
        use std::path::PathBuf;

        use crate::Session;
        use serde_json::json;

        /// A TrueType font with one empty glyph and a `name` table, written to `dir`.
        fn font_file(dir: &std::path::Path) -> PathBuf {
            let mut font = fontcraft_tables::SfntFont::new(fontcraft_tables::Flavour::TrueType);
            let mut head = vec![0u8; 54];
            head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
            head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
            head[18..20].copy_from_slice(&1000u16.to_be_bytes());
            head[50..52].copy_from_slice(&1u16.to_be_bytes());
            font.insert(*b"head", head);
            font.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 1]);
            font.insert(*b"loca", vec![0, 0, 0, 0, 0, 0, 0, 0]);
            font.insert(*b"glyf", Vec::new());
            let bytes = fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions::default()).unwrap();
            let path = dir.join("Views.ttf");
            std::fs::write(&path, bytes).unwrap();
            path
        }

        #[test]
        fn font_dump_tables_writes_text_views_and_the_normalised_dump() {
            let dir = std::env::temp_dir().join(format!("fontcraft-views-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let f = font_file(&dir).display().to_string();
            let mut s = Session::new();

            let font = s.execute("font.dumpTables", &json!({"font": f, "tables": [{"tag": "@font"}], "format": "text"})).unwrap();
            assert_eq!(font["format"], "text");
            assert!(font["text"].as_str().unwrap().starts_with("## Top Dict\nversion             \"1.000\"\n"), "{font}");

            let glyphs = s.execute("font.dumpTables", &json!({"font": f, "tables": ["@glyphs"], "glyphDetail": "long"})).unwrap();
            assert_eq!(glyphs["text"], "## glyph[tag] {name,encoding,path}\nglyph[0] {.notdef,-,\n  0 width\n  endchar}\n");

            let normalized = s.execute("font.dumpTables", &json!({"font": f, "normalize": true, "tables": ["head"]})).unwrap();
            let text = normalized["text"].as_str().unwrap();
            assert!(text.contains("  <head>") && !text.contains("<maxp>") && !text.contains("<GlyphOrder>"), "{text}");
            assert!(text.contains("<magicNumber value=\"0x0\"/>"), "{text}");

            let out = dir.join("views.txt");
            let written = s
                .execute("font.dumpTables", &json!({"font": f, "tables": ["@font"], "format": "text", "output": out.display().to_string()}))
                .unwrap();
            assert_eq!(written["paths"][0], out.display().to_string());
            assert!(std::fs::read_to_string(&out).unwrap().starts_with("## Top Dict"));

            let points = s.execute("font.dumpTables", &json!({"font": f, "tables": [{"tag": "glyf", "view": "points"}]})).unwrap();
            assert_eq!(points["text"], "", "{points}");

            for bad in [
                json!({"font": f, "tables": [{"tag": "CFF", "view": "literal"}], "cffLevel": "huge"}),
                json!({"font": f, "tables": [{"tag": "hmtx", "view": "advances"}], "units": "2048"}),
                json!({"font": f, "tables": [{"tag": "CFF", "view": "literal"}]}),
                json!({"font": f, "normalize": true, "format": "json"}),
                json!({"font": f, "tables": ["@nothing"], "format": "text"}),
                json!({"font": f, "format": "text"}),
                json!({"font": f, "tables": ["@glyphs"], "glyphDetail": "huge"}),
                json!({"font": f, "tables": [{"tag": "GPOS", "view": "fea"}], "format": "hex"}),
                json!({"font": f, "normalize": true, "split": "tables", "outputDir": dir.display().to_string()}),
            ] {
                assert!(s.execute("font.dumpTables", &bad).is_err(), "{bad}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// The options of M12.76: `allowVirtualGids`, `unicodeData`, and the @glyphs options `fd`,
/// `excludeFd`, `flattenSeac`, `names` and `goadb`.
mod extras {
    use std::path::Path;
    use std::sync::Arc;

    use fontcraft_tables::text::UnicodeNames;
    use fontcraft_tables::text::unicode_data::MAX_UNICODE_DATA;
    use serde_json::Value;

    use super::super::convert_file::{MAX_GOADB_BYTES, goadb_text};
    use super::super::file_io::read_input_up_to;
    use crate::params::Params;
    use crate::{EngineError, Result};

    /// The keys that apply to the @glyphs pseudo-table only.
    const GLYPHS_ONLY: [&str; 5] = ["fd", "excludeFd", "flattenSeac", "names", "goadb"];

    /// Whether `key` is given with a value that asks for something (not null, false or empty).
    pub(super) fn given(p: &Params<'_>, key: &str) -> bool {
        match p.get(key) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(Value::Array(a)) => !a.is_empty(),
            Some(_) => true,
        }
    }

    /// The first @glyphs-only key the request gives.
    pub(super) fn glyphs_only_given(p: &Params<'_>) -> Option<&'static str> {
        GLYPHS_ONLY.into_iter().find(|k| given(p, k))
    }

    /// `allowVirtualGids` (ttx `-a`), which ttx 4.66 accepts and ignores: glyph indices past the
    /// glyph count are always written as `glyphNNNNN`. Only its type is checked.
    pub(super) fn check_virtual_gids(p: &Params<'_>) -> Result<()> {
        p.bool_or("allowVirtualGids", false).map(|_| ())
    }

    /// The character names of `unicodeData`, read from the file it names.
    pub(super) fn unicode_names(p: &Params<'_>) -> Result<Option<Arc<UnicodeNames>>> {
        let path = match p.get("unicodeData") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => return Ok(None),
            Some(Value::Bool(true)) => {
                return Err(p.bad("no character names are bundled: give `unicodeData` the path of a UnicodeData.txt file"));
            }
            Some(Value::String(path)) => Path::new(path.as_str()),
            Some(_) => return Err(p.bad("`unicodeData` is the path of a UnicodeData.txt file")),
        };
        let read = read_input_up_to(path, MAX_UNICODE_DATA)?;
        if read.bytes.len() > MAX_UNICODE_DATA {
            return Err(EngineError::Io(format!("{} is larger than {MAX_UNICODE_DATA} bytes", path.display())));
        }
        let text = String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", path.display())))?;
        let names = UnicodeNames::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|e| p.bad(format!("`unicodeData` {}: {e}", path.display())))?;
        Ok(Some(Arc::new(names)))
    }

    /// Font dict indexes from 0: a list of numbers, or a comma-separated string of them.
    pub(super) fn font_dicts(p: &Params<'_>, key: &str) -> Result<Vec<u16>> {
        let bad = || p.bad(format!("`{key}` is a list of font dict indexes from 0 to 65535"));
        let parse = |v: &Value| -> Result<u16> {
            match v {
                Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or_else(bad),
                Value::String(s) => s.trim().parse::<u16>().map_err(|_| bad()),
                _ => Err(bad()),
            }
        };
        let mut out: Vec<u16> = match p.get(key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.iter().map(parse).collect::<Result<_>>()?,
            Some(Value::String(s)) => {
                s.split(',').filter(|t| !t.trim().is_empty()).map(|t| parse(&Value::String(t.to_string()))).collect::<Result<_>>()?
            }
            Some(v) => vec![parse(v)?],
        };
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// The `(from, to)` names of `names` and `goadb`: without a GOADB the names stay (`names`
    /// \"source\" needs one); with one, final names become source names (\"source\", the
    /// default) or source names become final names (\"final\").
    pub(super) fn rename(p: &Params<'_>) -> Result<Vec<(String, String)>> {
        let names = p.opt_str("names")?;
        let to_source = match names {
            None => true,
            Some("source") => true,
            Some("final") => false,
            Some(other) => return Err(p.bad(format!("`names` must be \"source\" or \"final\", got \"{other}\""))),
        };
        let Some(path) = p.opt_str("goadb")? else {
            if names == Some("source") {
                return Err(p.bad("`names` \"source\" maps the names through a GOADB: give `goadb`"));
            }
            return Ok(Vec::new());
        };
        let text = goadb_text(Path::new(path), MAX_GOADB_BYTES)?;
        let goadb =
            fontcraft_tables::goadb::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).map_err(|e| p.bad(format!("`goadb` {path}: {e}")))?;
        Ok(goadb.pairs(to_source))
    }

    #[cfg(test)]
    #[cfg(not(target_arch = "wasm32"))]
    mod tests {
        use std::path::{Path, PathBuf};

        use serde_json::json;

        use crate::Session;

        fn be(values: &[u16]) -> Vec<u8> {
            values.iter().flat_map(|v| v.to_be_bytes()).collect()
        }

        /// A TrueType font of `.notdef` and `A` (U+0041), both empty, written to `dir`.
        fn font_file(dir: &Path) -> PathBuf {
            let mut font = fontcraft_tables::SfntFont::new(fontcraft_tables::Flavour::TrueType);
            let mut head = vec![0u8; 54];
            head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
            head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
            head[18..20].copy_from_slice(&1000u16.to_be_bytes());
            head[50..52].copy_from_slice(&1u16.to_be_bytes());
            font.insert(*b"head", head);
            font.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 2]);
            font.insert(*b"loca", vec![0; 12]);
            font.insert(*b"glyf", Vec::new());
            // post version 2: .notdef and A (standard name 36).
            let mut post = vec![0, 2, 0, 0];
            post.extend_from_slice(&[0; 28]);
            post.extend(be(&[2, 0, 36]));
            font.insert(*b"post", post);
            // cmap: one Windows Unicode format 4 subtable mapping U+0041 to glyph 1.
            let mut cmap = be(&[0, 1, 3, 1]);
            cmap.extend_from_slice(&12u32.to_be_bytes());
            cmap.extend(be(&[4, 32, 0, 4, 4, 1, 0, 0x41, 0xFFFF, 0, 0x41, 0xFFFF, 0xFFC0, 1, 0, 0]));
            font.insert(*b"cmap", cmap);
            let bytes = fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions::default()).unwrap();
            let path = dir.join("Extras.ttf");
            std::fs::write(&path, bytes).unwrap();
            path
        }

        #[test]
        fn font_dump_tables_writes_unicode_names_maps_glyph_names_and_refuses_misplaced_options() {
            let dir = std::env::temp_dir().join(format!("fontcraft-dump-extras-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let f = font_file(&dir).display().to_string();
            let data = dir.join("UnicodeData.txt");
            std::fs::write(&data, "0000;<control>;Cc\n0041;LATIN CAPITAL LETTER A;Lu;0;L;;;;;N;;;;0061;\n").unwrap();
            let data = data.display().to_string();
            let goadb = dir.join("goadb.txt");
            std::fs::write(&goadb, "A A.src uni0041\n").unwrap();
            let goadb = goadb.display().to_string();
            let mut s = Session::new();

            let named = s.execute("font.dumpTables", &json!({"font": f, "tables": ["cmap"], "unicodeData": data})).unwrap();
            let text = named["text"].as_str().unwrap();
            assert!(text.contains("<map code=\"0x41\" name=\"A\"/><!-- LATIN CAPITAL LETTER A -->\n"), "{text}");
            let plain = s.execute("font.dumpTables", &json!({"font": f, "tables": ["cmap"]})).unwrap();
            assert!(!plain["text"].as_str().unwrap().contains("<!--"), "{plain}");
            let virtual_gids = s.execute("font.dumpTables", &json!({"font": f, "tables": ["cmap"], "allowVirtualGids": true})).unwrap();
            assert_eq!(virtual_gids, plain);

            let source = s.execute("font.dumpTables", &json!({"font": f, "tables": ["@glyphs"], "goadb": goadb})).unwrap();
            assert_eq!(source["text"], "## glyph[tag] {name,encoding}\nglyph[0] {.notdef,-}\nglyph[1] {A.src,U+0041}\n");
            let picked = s
                .execute("font.dumpTables", &json!({"font": f, "tables": ["@glyphs"], "goadb": goadb, "names": "source", "glyphs": ["A.src"]}))
                .unwrap();
            assert_eq!(picked["text"], "## glyph[tag] {name,encoding}\nglyph[1] {A.src,U+0041}\n");
            let final_names = s.execute("font.dumpTables", &json!({"font": f, "tables": ["@glyphs"], "goadb": goadb, "names": "final"})).unwrap();
            assert!(final_names["text"].as_str().unwrap().contains("{A,U+0041}"), "{final_names}");

            for bad in [
                json!({"font": f, "tables": ["cmap"], "flattenSeac": true}),
                json!({"font": f, "tables": ["cmap"], "goadb": goadb}),
                json!({"font": f, "normalize": true, "fd": [0]}),
                json!({"font": f, "tables": ["@glyphs"], "names": "source"}),
                json!({"font": f, "tables": ["@glyphs"], "names": "both", "goadb": goadb}),
                json!({"font": f, "tables": ["@glyphs"], "fd": [0]}),
                json!({"font": f, "tables": ["@glyphs"], "excludeFd": ["x"]}),
                json!({"font": f, "tables": ["cmap"], "unicodeData": true}),
                json!({"font": f, "tables": ["cmap"], "unicodeData": data, "format": "json"}),
                json!({"font": f, "tables": [{"tag": "cmap", "view": "summary"}], "unicodeData": data}),
                json!({"font": f, "tables": ["cmap"], "allowVirtualGids": "yes"}),
                json!({"font": f, "tables": [{"tag": "avar", "view": "designspace"}]}),
            ] {
                assert!(s.execute("font.dumpTables", &bad).is_err(), "{bad}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Writing a dump and its image files together: each file goes to a temporary name in its
/// folder first, and only when all are written are they renamed into place.
mod staged {
    use std::collections::HashMap;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::file_io::{OutputRequest, check_write, create_output_dir, remove_created};
    use crate::{EngineError, Result};

    /// How many names a temporary file tries before giving up.
    const TRIES: usize = 16;

    /// Writes `files` (path, bytes). A later file for the same path replaces an earlier one, as
    /// `ttx` overwrites an image two tables name. Every path must pass the automation write
    /// check (which refuses every path on the web, so nothing past it runs there), and with
    /// `noClobber` none may exist. When a file cannot be written, the temporary files and the
    /// folders made for them are removed and nothing is replaced. When a temporary file cannot be
    /// moved into place, the files moved before it stay; the other temporary files and the empty
    /// folders made for them are removed.
    pub(super) fn write_all(files: Vec<(PathBuf, Vec<u8>)>, request: &OutputRequest) -> Result<()> {
        let mut unique: Vec<(PathBuf, Vec<u8>)> = Vec::with_capacity(files.len());
        let mut index: HashMap<PathBuf, usize> = HashMap::new();
        for (path, bytes) in files {
            match index.get(&path).and_then(|&i| unique.get_mut(i)) {
                Some(slot) => slot.1 = bytes,
                None => {
                    index.insert(path.clone(), unique.len());
                    unique.push((path, bytes));
                }
            }
        }
        for (path, _) in &unique {
            check_write(path)?;
            if request.no_clobber && std::fs::symlink_metadata(path).is_ok() {
                return Err(EngineError::Io(format!("{} exists and noClobber is set", path.display())));
            }
        }
        // Folders in the order they were made; removed in the reverse order, deepest first.
        let mut made: Vec<PathBuf> = create_output_dir(request)?.into_iter().rev().collect();
        let mut staged: Vec<(PathBuf, &Path)> = Vec::with_capacity(unique.len());
        let mut failure = None;
        for (path, bytes) in &unique {
            match stage(path, bytes, &mut made) {
                Ok(tmp) => staged.push((tmp, path)),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if failure.is_none()
            && let Some((_, target)) = staged.iter().find(|(_, target)| target.is_dir())
        {
            failure = Some(EngineError::Io(format!("cannot write {}: a folder has that name", target.display())));
        }
        if let Some(e) = failure {
            for (tmp, _) in &staged {
                let _ = std::fs::remove_file(tmp);
            }
            made.reverse();
            remove_created(&made);
            return Err(e);
        }
        for (i, (tmp, target)) in staged.iter().enumerate() {
            if let Err(e) = std::fs::rename(tmp, target) {
                for (rest, _) in staged.iter().skip(i) {
                    let _ = std::fs::remove_file(rest);
                }
                // Deepest first; a folder that holds a file moved into place stays.
                for dir in made.iter().rev() {
                    let _ = std::fs::remove_dir(dir);
                }
                return Err(EngineError::Io(format!("cannot write {}: {e} ({i} of {} files were written)", target.display(), staged.len())));
            }
            fontcraft_format::written::record(target);
        }
        Ok(())
    }

    /// Writes `bytes` to a new temporary file beside `path`, making the folders on the way.
    fn stage(path: &Path, bytes: &[u8], made: &mut Vec<PathBuf>) -> Result<PathBuf> {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let missing: Vec<&Path> = dir.ancestors().take_while(|d| !d.as_os_str().is_empty() && std::fs::symlink_metadata(d).is_err()).collect();
        for d in missing.into_iter().rev() {
            std::fs::create_dir(d).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", d.display())))?;
            made.push(d.to_path_buf());
        }
        let cannot = |e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..TRIES {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let tmp = dir.join(format!(".fontcraft-dump-{}-{n}.tmp", std::process::id()));
            let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(cannot(e)),
            };
            let written = file.write_all(bytes).and_then(|()| file.sync_all());
            drop(file);
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(cannot(e));
            }
            return Ok(tmp);
        }
        Err(cannot(std::io::Error::from(std::io::ErrorKind::AlreadyExists)))
    }
}
