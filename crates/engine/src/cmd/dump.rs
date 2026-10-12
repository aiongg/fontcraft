//! `font.dumpTables`: TTX, JSON and hex dumps and the text views (M12.4, M12.10; `fontcraft_tables::ttx`, `fontcraft_tables::text`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::CommandSpec;

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
        "{font: path, face?: number, tables?: [tag | {tag, view?}], excludeTables?: [tag], format?: \"ttx\"|\"json\"|\"hex\"|\"text\", split?: \"tables\", output?: path, outputDir?: path, noClobber?: bool, disassemble?: bool, strict?: bool, newline?: \"lf\"|\"cr\"|\"crlf\"} the tables of a compiled font, collection face, WOFF or WOFF2 file as TTX (the XML of the reference dump tool), the same tree as JSON, or hex; tables that cannot be read are written as hex unless strict; tables leaves the glyph order out; split writes one file per table beside the main file → {format, text} or {format, json} without an output, {format, paths} with one",
        super::always,
        ttx::run
    )]
}

mod ttx {
    use std::path::{Path, PathBuf};

    use fontcraft_tables::sfnt::parse_tag;
    use fontcraft_tables::ttx::{DumpFormat, DumpOptions, dump, dump_split};
    use serde_json::{Value, json};

    use super::super::file_io::{OutputRequest, output_path, read_input, write_output};
    use crate::params::Params;
    use crate::{EngineError, Result, Session};

    pub(super) fn run(_: &mut Session, params: &Value) -> Result<Value> {
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
        let split = match p.opt_str("split")? {
            None => false,
            Some("tables") => true,
            Some("glyphs") => return Err(EngineError::Unsupported("splitting the dump into one file per glyph is not built yet".into())),
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
        let options = DumpOptions {
            format,
            tables,
            skip,
            hex: Vec::new(),
            no_version: false,
            no_disassemble: !p.bool_or("disassemble", true)?,
            strict: p.bool_or("strict", false)?,
        };
        let request = OutputRequest::from_params(&p)?;
        if request.in_place {
            return Err(p.bad("a dump never replaces its input: give `output` or `outputDir`"));
        }
        let writes = request.output.is_some() || request.output_dir.is_some();
        if split && !writes {
            return Err(p.bad("`split` writes files: give `output` or `outputDir`"));
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
            let parts = dump_split(&font, &options, &stem, &format!(".{ext}"))?;
            let mut paths: Vec<PathBuf> = Vec::with_capacity(parts.len());
            for (name, text) in parts {
                let target = dir.join(name);
                write_output(&target, lines(text).as_bytes(), &request)?;
                paths.push(target);
            }
            return Ok(json!({ "format": format_name, "paths": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() }));
        }
        let text = dump(&font, &options)?;
        if writes {
            let target = output_path(path, &request, "", Some(ext))?;
            write_output(&target, lines(text).as_bytes(), &request)?;
            return Ok(json!({ "format": format_name, "paths": [target.display().to_string()] }));
        }
        if format == DumpFormat::Json {
            let tree: Value = serde_json::from_str(&text).map_err(|e| EngineError::Io(format!("the JSON dump could not be read back: {e}")))?;
            return Ok(json!({ "format": format_name, "json": tree }));
        }
        Ok(json!({ "format": format_name, "text": lines(text) }))
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
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

/// The text views, `@font`, `@glyphs` and `--normalize` (M12.10).
fn view_specs() -> Vec<CommandSpec> {
    // M12.10
    Vec::new()
}
