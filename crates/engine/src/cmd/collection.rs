//! Collections and WOFF: `collection.list`, `.pack`, `.unpack`, `woff.compress`, `woff.decompress` (M12.3; `fontcraft_tables::collection`, `fontcraft_container`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. Every output is built in memory first; a command
//! with several outputs checks all their paths before it writes the first.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use fontcraft_container::{Extras, woff, woff2};
use fontcraft_tables::TablesError;
use fontcraft_tables::collection::{self, PackOptions};
use fontcraft_tables::limits::MAX_UNPACKED;
use fontcraft_tables::sfnt::{self, Flavour, SfntFont, Tag, WriteOptions};
use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "collection.list",
            "List Collection Members",
            [],
            None,
            "{file: path} the members of a collection (a WOFF2 collection is unwrapped; a single font lists as one member): index, PostScript name, outline flavour, tables, and the tables whose data another member shares → {format, members: [{index, postscriptName, flavour: \"truetype\"|\"cff\", tables: [tag], shared: [tag]}]}",
            always,
            list
        ).files(const { &[FileParam::read("file")] }),
        cmd!(
            "collection.pack",
            "Pack Collection",
            [],
            None,
            "{fonts: [path], output: path, shareTables?: [{tag: string, index: int}], share?: bool, container?: \"plain\"|\"woff2\", noClobber?: bool} a collection of the fonts in order (members of input collections appended in order); identical tables are stored once unless share is false; shareTables makes every member use the table TAG of input font INDEX (zero-based, counting collection members) → {output, fonts, size, savedBySharing: {tag: bytes}}",
            always,
            pack
        ).files(const { &[FileParam::read("fonts"), FileParam::write("output")] }).writes_files(),
        cmd!(
            "collection.unpack",
            "Unpack Collection",
            [],
            None,
            "{file: path, face?: int|name, outputDir?: path, output?: path, nameBy?: \"postscript-name\"|\"index\", noClobber?: bool} every member (or the one face names, by index or PostScript name) as a font of its own, named POSTSCRIPTNAME.otf or .ttf by its outlines, or STEM-fontN without a PostScript name, with nameBy index, or when a name repeats; beside the collection unless outputDir; output only with one face → {outputs: [{face, path, postscriptName}]}",
            always,
            unpack
        ).files(const { &[FileParam::read("file"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            "woff.compress",
            "Compress to WOFF",
            [],
            None,
            "{font: path, format?: \"woff2\"|\"woff\", output?: path, outputDir?: path, glyfTransform?: bool, hmtxTransform?: bool, quality?: int, zopfli?: bool|int, metadata?: path, privateData?: path, noClobber?: bool} a font or collection (collections only in WOFF2) wrapped in WOFF2 (default; glyf transform on, hmtx transform off) or WOFF 1.0; quality is 11 for WOFF2 and 6 for WOFF, the only levels written yet → {output, format, originalSize, size, tables: [{tag, origLength, storedLength?}]}",
            always,
            compress
        ).files(const { &[FileParam::read("font"), FileParam::read("metadata"), FileParam::read("privateData"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            "woff.decompress",
            "Decompress WOFF",
            [],
            None,
            "{font: path, output?: path, outputDir?: path, extractMetadata?: path, extractPrivateData?: path, noClobber?: bool} the font or collection inside a WOFF or WOFF2 file, named .ttf, .otf or .ttc by its content; the metadata and private blocks are written where asked when the file has them → {output, format, size, metadata?: path, privateData?: path}",
            always,
            decompress
        ).files(const { &[FileParam::read("font"), FileParam::write("extractMetadata"), FileParam::write("extractPrivateData"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
    ]
}

fn tag_text(tag: Tag) -> String {
    sfnt::tag_str(tag).trim_end().to_string()
}

fn flavour_name(f: Flavour) -> &'static str {
    match f {
        Flavour::Cff => "cff",
        Flavour::TrueType | Flavour::AppleTrue => "truetype",
    }
}

fn font_input(p: &Params<'_>, path: &str) -> Result<file_io::Input> {
    let input = file_io::read_input(Path::new(path))?;
    if !input.kind.is_font() {
        return Err(p.bad(format!("{path} is not a compiled font, collection, WOFF or WOFF2 file")));
    }
    Ok(input)
}

fn container_error(e: fontcraft_container::Error) -> EngineError {
    EngineError::from(TablesError::from(e))
}

// ── collection.list ──────────────────────────────────────────────────────────────────────────

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("collection.list", p)?;
    let input = font_input(&p, p.str("file")?)?;
    let members = collection::list(&input.bytes)?;
    let rows: Vec<Value> = members
        .iter()
        .map(|m| {
            json!({
                "index": m.index,
                "postscriptName": m.postscript_name,
                "flavour": flavour_name(m.flavour),
                "tables": m.tables.iter().map(|t| tag_text(*t)).collect::<Vec<_>>(),
                "shared": m.shared.iter().map(|t| tag_text(*t)).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({ "format": input.kind.name(), "members": rows }))
}

// ── collection.pack ──────────────────────────────────────────────────────────────────────────

fn pack(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("collection.pack", p)?;
    let paths = p.strings("fonts")?;
    if paths.is_empty() {
        return Err(p.bad("`fonts` needs at least one font"));
    }
    if paths.len() > fontcraft_tables::limits::MAX_COLLECTION_FONTS {
        return Err(p.bad(format!("at most {} fonts go into one collection", fontcraft_tables::limits::MAX_COLLECTION_FONTS)));
    }
    let output = PathBuf::from(p.str("output")?);
    let share = p.bool_or("share", true)?;
    let container = p.opt_str("container")?.unwrap_or("plain");
    if !matches!(container, "plain" | "woff2") {
        return Err(p.bad(format!("`container` must be \"plain\" or \"woff2\", got \"{container}\"")));
    }
    let mut forced: Vec<(Tag, usize)> = Vec::new();
    for item in p.opt_array("shareTables")?.map(Vec::as_slice).unwrap_or_default() {
        let tag = item.get("tag").and_then(Value::as_str).ok_or_else(|| p.bad("each `shareTables` item needs a `tag` string"))?;
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .ok_or_else(|| p.bad("each `shareTables` item needs a whole `index`"))?;
        forced.push((sfnt::parse_tag(tag)?, index));
    }
    let mut fonts: Vec<SfntFont> = Vec::new();
    let mut total = 0usize;
    for path in &paths {
        let input = font_input(&p, path)?;
        let members = collection::unpack(&input.bytes).map_err(|e| EngineError::Io(format!("{path}: {e}")))?;
        total = members.iter().flat_map(|f| f.tables.values()).fold(total, |sum, t| sum.saturating_add(t.len()));
        if total > MAX_UNPACKED {
            return Err(p.bad(format!("the inputs' tables add up to more than {MAX_UNPACKED} bytes")));
        }
        fonts.extend(members);
        if fonts.len() > fontcraft_tables::limits::MAX_COLLECTION_FONTS {
            return Err(p.bad(format!("the inputs hold more than {} fonts", fontcraft_tables::limits::MAX_COLLECTION_FONTS)));
        }
    }
    for (tag, index) in &forced {
        let source =
            fonts.get(*index).ok_or_else(|| p.bad(format!("`shareTables` names font {index}, but the inputs hold {} fonts", fonts.len())))?;
        let data = source.get(*tag).map(<[u8]>::to_vec).ok_or_else(|| p.bad(format!("font {index} has no '{}' table to share", tag_text(*tag))))?;
        for font in &mut fonts {
            if font.tables.contains_key(tag) {
                font.insert(*tag, data.clone());
            }
        }
    }
    let ttc = collection::pack(&fonts, &PackOptions { share_tables: share, ..PackOptions::default() })?;
    // What sharing saved: for each tag, the bytes of every copy after the first.
    let mut saved: BTreeMap<String, u64> = BTreeMap::new();
    if share {
        let mut stored: HashSet<(Tag, &[u8])> = HashSet::new();
        for font in &fonts {
            for (tag, data) in &font.tables {
                if !stored.insert((*tag, data.as_slice())) {
                    let total = saved.entry(tag_text(*tag)).or_default();
                    *total = total.saturating_add(data.len() as u64);
                }
            }
        }
    }
    saved.retain(|_, v| *v > 0);
    let bytes = if container == "woff2" { woff2::encode(&ttc, &woff2::Options::default()).map_err(container_error)? } else { ttc };
    let req = OutputRequest { output: Some(output.clone()), no_clobber: p.bool_or("noClobber", false)?, ..OutputRequest::default() };
    if paths.iter().any(|path| Path::new(path) == output) {
        return Err(p.bad(format!("the output {} is one of the inputs", output.display())));
    }
    file_io::write_output(&output, &bytes, &req)?;
    Ok(json!({ "output": output.to_string_lossy(), "fonts": fonts.len(), "size": bytes.len(), "savedBySharing": saved }))
}

// ── collection.unpack ────────────────────────────────────────────────────────────────────────

fn unpack(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("collection.unpack", p)?;
    let path = PathBuf::from(p.str("file")?);
    let input = font_input(&p, p.str("file")?)?;
    let by_index = match p.opt_str("nameBy")?.unwrap_or("postscript-name") {
        "postscript-name" => false,
        "index" => true,
        other => return Err(p.bad(format!("`nameBy` must be \"postscript-name\" or \"index\", got \"{other}\""))),
    };
    let fonts = collection::unpack(&input.bytes)?;
    let members = collection::list(&input.bytes)?;
    let chosen: Vec<usize> = match p.get("face") {
        None => (0..fonts.len()).collect(),
        Some(Value::String(name)) => {
            let found: Vec<usize> =
                members.iter().enumerate().filter(|(_, m)| m.postscript_name.as_deref() == Some(name.as_str())).map(|(i, _)| i).collect();
            if found.is_empty() {
                return Err(p.bad(format!("no member has the PostScript name \"{name}\"")));
            }
            found
        }
        Some(v) => {
            let face = v.as_u64().and_then(|f| usize::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a member index or a PostScript name"))?;
            if face >= fonts.len() {
                return Err(p.bad(format!("the file holds {} fonts; face {face} does not exist", fonts.len())));
            }
            vec![face]
        }
    };
    let output = p.opt_str("output")?.map(PathBuf::from);
    let output_dir = p.opt_str("outputDir")?.map(PathBuf::from);
    if output.is_some() && (chosen.len() != 1 || output_dir.is_some()) {
        return Err(p.bad("`output` names one file: give it with a single `face` and without `outputDir`"));
    }
    let req = OutputRequest { output_dir: output_dir.clone(), no_clobber: p.bool_or("noClobber", false)?, ..OutputRequest::default() };
    let dir = output_dir.or_else(|| path.parent().map(Path::to_path_buf)).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "collection".into());
    let source = resolved(&path);
    let mut planned: Vec<(usize, PathBuf, Vec<u8>, Option<String>)> = Vec::with_capacity(chosen.len());
    // The targets planned so far, resolved, so two spellings of one file count as one.
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for i in chosen {
        let (Some(font), Some(member)) = (fonts.get(i), members.get(i)) else { continue };
        let ext = if font.flavour == Flavour::Cff { "otf" } else { "ttf" };
        let usable = member.postscript_name.as_deref().filter(|n| !by_index && is_file_name(n));
        let numbered = |n: usize| dir.join(format!("{stem}-font{n}.{ext}"));
        let mut target = match (&output, usable) {
            (Some(out), _) => out.clone(),
            (None, Some(name)) => dir.join(format!("{name}.{ext}")),
            (None, None) => numbered(i),
        };
        if output.is_none() && taken.contains(&resolved(&target)) {
            // The member's own number first, then the next one no other member has.
            target = (0..=taken.len())
                .map(|k| numbered(i.saturating_add(k)))
                .find(|t| !taken.contains(&resolved(t)))
                .ok_or_else(|| EngineError::Internal("no free name for an unpacked member".into()))?;
        }
        let key = resolved(&target);
        if key == source {
            return Err(p.bad(format!("member {i} would replace the collection {}", path.display())));
        }
        if req.no_clobber && target.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", target.display())));
        }
        taken.insert(key);
        planned.push((i, target, sfnt::write(font, &WriteOptions::default())?, member.postscript_name.clone()));
    }
    let files: Vec<(&Path, &[u8])> = planned.iter().map(|(_, target, bytes, _)| (target.as_path(), bytes.as_slice())).collect();
    write_all(&files, &req)?;
    let outputs: Vec<Value> =
        planned.iter().map(|(face, target, _, name)| json!({ "face": face, "path": target.to_string_lossy(), "postscriptName": name })).collect();
    Ok(json!({ "outputs": outputs }))
}

/// `path` as the file system names it: canonical when it exists, else its folder made canonical
/// and its name kept, so two spellings of one file compare equal.
pub(super) fn resolved(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => {
            let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
            std::fs::canonicalize(dir).map(|d| d.join(name)).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

/// Writes every file, in order. When one fails, the files this call created before it are
/// removed again, as are the output folders it created, and the error says so and names the
/// files it had replaced.
pub(super) fn write_all(files: &[(&Path, &[u8])], req: &OutputRequest) -> Result<()> {
    let created = file_io::create_output_dir(req)?;
    let mut written: Vec<(&Path, bool)> = Vec::with_capacity(files.len());
    for &(target, bytes) in files {
        let existed = target.exists();
        if let Err(e) = file_io::write_output(target, bytes, req) {
            let mut message = e.to_string();
            let mut removed = 0usize;
            let mut kept: Vec<String> = Vec::new();
            for (done, existed) in written {
                if existed {
                    kept.push(done.display().to_string());
                } else if std::fs::remove_file(done).is_ok() {
                    removed = removed.saturating_add(1);
                } else {
                    kept.push(done.display().to_string());
                }
            }
            if removed > 0 {
                message.push_str(&format!("; the {removed} files written before it were removed"));
            }
            if !kept.is_empty() {
                message.push_str(&format!("; these were written and remain: {}", kept.join(", ")));
            }
            file_io::remove_created(&created);
            return Err(EngineError::Io(message));
        }
        written.push((target, existed));
    }
    Ok(())
}

/// A PostScript name that can be a file name as it is.
fn is_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 127
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '/' | '\\' | ':'))
}

// ── woff.compress ────────────────────────────────────────────────────────────────────────────

fn compress(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("woff.compress", p)?;
    let path_text = p.str("font")?;
    let input = font_input(&p, path_text)?;
    let format = p.opt_str("format")?.unwrap_or("woff2");
    if !matches!(format, "woff2" | "woff") {
        return Err(p.bad(format!("`format` must be \"woff2\" or \"woff\", got \"{format}\"")));
    }
    let default_quality = if format == "woff2" { 11 } else { 6 };
    if let Some(q) = p.opt_int("quality", 0, 11)?
        && q != default_quality
    {
        return Err(EngineError::Unsupported(format!("{format} at quality {q}: only {default_quality} is written")));
    }
    match p.get("zopfli") {
        None | Some(Value::Bool(false)) => {}
        Some(_) => return Err(EngineError::Unsupported("the zopfli compressor".into())),
    }
    let read_block = |key: &str| -> Result<Option<Vec<u8>>> {
        match p.opt_str(key)? {
            Some(path) => Ok(Some(file_io::read_input(Path::new(path))?.bytes)),
            None => Ok(None),
        }
    };
    let extras = Extras { version: None, metadata: read_block("metadata")?, private: read_block("privateData")? };
    let glyf = p.bool_or("glyfTransform", true)?;
    let hmtx = p.bool_or("hmtxTransform", false)?;
    let plain = fontcraft_container::to_sfnt(&input.bytes).map_err(container_error)?;
    let bytes = if format == "woff2" {
        woff2::encode(&plain, &woff2::Options { transform_glyf: glyf, transform_hmtx: hmtx, extras }).map_err(container_error)?
    } else {
        if (glyf && p.get("glyfTransform").is_some()) || hmtx {
            return Err(p.bad("the glyf and hmtx transforms belong to WOFF2"));
        }
        woff::encode(&plain, &extras).map_err(container_error)?
    };
    let tables: Vec<Value> = if format == "woff2" {
        woff2::directory(&bytes)
            .map_err(container_error)?
            .tables
            .iter()
            .map(|t| json!({ "tag": tag_text(t.tag), "origLength": t.orig_length, "storedLength": t.transform_length }))
            .collect()
    } else {
        woff::directory(&bytes)
            .map_err(container_error)?
            .iter()
            .map(|t| json!({ "tag": tag_text(t.tag.to_be_bytes()), "origLength": t.length, "storedLength": t.compressed }))
            .collect()
    };
    let req = OutputRequest::from_params(&p)?;
    if req.in_place {
        return Err(p.bad("`inPlace` would replace the font with its WOFF form: give output or outputDir"));
    }
    let out = file_io::output_path(&input.path, &req, "", Some(format))?;
    file_io::write_output(&out, &bytes, &req)?;
    Ok(json!({ "output": out.to_string_lossy(), "format": format, "originalSize": plain.len(), "size": bytes.len(), "tables": tables }))
}

// ── woff.decompress ──────────────────────────────────────────────────────────────────────────

fn decompress(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("woff.decompress", p)?;
    let path_text = p.str("font")?;
    let input = font_input(&p, path_text)?;
    let unwrapped = match input.kind {
        InputKind::Woff => woff::decode(&input.bytes),
        InputKind::Woff2 => woff2::decode(&input.bytes),
        _ => return Err(p.bad(format!("{path_text} is not a WOFF or WOFF2 file"))),
    }
    .map_err(container_error)?;
    let ext = match fontcraft_container::sniff(&unwrapped.sfnt) {
        Some(fontcraft_container::Kind::Collection) => "ttc",
        Some(fontcraft_container::Kind::Cff) => "otf",
        _ => "ttf",
    };
    let req = OutputRequest::from_params(&p)?;
    if req.in_place {
        return Err(p.bad("`inPlace` would replace the WOFF file with the font inside: give output or outputDir"));
    }
    let out = file_io::output_path(&input.path, &req, "", Some(ext))?;
    let mut blocks: Vec<(&str, PathBuf, Vec<u8>)> = Vec::new();
    for (key, block) in [("extractMetadata", &unwrapped.metadata), ("extractPrivateData", &unwrapped.private)] {
        if let (Some(target), Some(data)) = (p.opt_str(key)?, block) {
            blocks.push((key, PathBuf::from(target), data.clone()));
        }
    }
    // No two outputs may be one file, and none may be the input.
    let mut taken: Vec<(String, PathBuf)> = vec![("the input".into(), resolved(&input.path))];
    let named = std::iter::once(("the output".to_string(), &out)).chain(blocks.iter().map(|b| (format!("`{}`", b.0), &b.1)));
    for (what, target) in named {
        let here = resolved(target);
        if let Some((other, _)) = taken.iter().find(|(_, t)| *t == here) {
            return Err(p.bad(format!("{what} {} is the same file as {other}", target.display())));
        }
        taken.push((what, here));
    }
    if req.no_clobber {
        for target in std::iter::once(&out).chain(blocks.iter().map(|b| &b.1)) {
            if target.exists() {
                return Err(EngineError::Io(format!("{} exists and noClobber is set", target.display())));
            }
        }
    }
    let files: Vec<(&Path, &[u8])> =
        std::iter::once((out.as_path(), unwrapped.sfnt.as_slice())).chain(blocks.iter().map(|b| (b.1.as_path(), b.2.as_slice()))).collect();
    write_all(&files, &req)?;
    let mut result = json!({ "output": out.to_string_lossy(), "format": input.kind.name(), "size": unwrapped.sfnt.len() });
    for (key, target, _) in blocks {
        let name = if key == "extractMetadata" { "metadata" } else { "privateData" };
        if let Some(obj) = result.as_object_mut() {
            obj.insert(name.into(), json!(target.to_string_lossy()));
        }
    }
    Ok(result)
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    fn font(revision: u8, name: &str, cff: bool) -> Vec<u8> {
        let mut f = SfntFont::new(if cff { Flavour::Cff } else { Flavour::TrueType });
        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&[0, 1, 0, 0]);
        head[4] = revision;
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        f.insert(*b"head", head);
        let ps: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut table = Vec::new();
        for v in [0u16, 1, 18, 3, 1, 0x409, 6, ps.len() as u16, 0] {
            table.extend_from_slice(&v.to_be_bytes());
        }
        table.extend_from_slice(&ps);
        f.insert(*b"name", table);
        f.insert(*b"cmap", b"the same cmap in every font".to_vec());
        f.insert(if cff { *b"CFF " } else { *b"glyf" }, vec![revision; 40]);
        sfnt::write(&f, &WriteOptions::default()).unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-collection-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn collection_pack_list_and_unpack_round_trip() {
        let dir = scratch("pack");
        let (a, b) = (dir.join("A.ttf"), dir.join("B.otf"));
        std::fs::write(&a, font(1, "Demo-Regular", false)).unwrap();
        std::fs::write(&b, font(2, "Demo-Bold", true)).unwrap();
        let ttc = dir.join("Demo.ttc");
        let mut s = Session::new();
        let r = s.execute("collection.pack", &json!({"fonts": [a, b], "output": ttc})).unwrap();
        assert_eq!(r["fonts"], 2);
        assert_eq!(r["savedBySharing"]["cmap"], 27, "the shared cmap is stored once");
        let listed = s.execute("collection.list", &json!({"file": ttc})).unwrap();
        assert_eq!(listed["members"][1]["postscriptName"], "Demo-Bold");
        assert_eq!(listed["members"][1]["flavour"], "cff");
        assert_eq!(listed["members"][0]["shared"], json!(["cmap"]));
        let out = dir.join("out");
        let r = s.execute("collection.unpack", &json!({"file": ttc, "outputDir": out})).unwrap();
        assert_eq!(r["outputs"][0]["path"], out.join("Demo-Regular.ttf").to_string_lossy().as_ref());
        assert_eq!(std::fs::read(out.join("Demo-Regular.ttf")).unwrap(), std::fs::read(&a).unwrap(), "a member unpacks to the font packed");
        assert_eq!(std::fs::read(out.join("Demo-Bold.otf")).unwrap(), std::fs::read(&b).unwrap());
        // One face by name, numbered names, and a refused overwrite that writes nothing.
        let r = s.execute("collection.unpack", &json!({"file": ttc, "face": "Demo-Bold", "outputDir": out, "nameBy": "index"})).unwrap();
        assert_eq!(r["outputs"][0]["path"], out.join("Demo-font1.otf").to_string_lossy().as_ref());
        assert!(s.execute("collection.unpack", &json!({"file": ttc, "outputDir": out, "noClobber": true})).is_err());
        assert!(s.execute("collection.unpack", &json!({"file": ttc, "face": 7})).is_err());
        // Forced sharing and a WOFF2 collection.
        let forced = dir.join("Forced.ttc.woff2");
        s.execute("collection.pack", &json!({"fonts": [a, b], "output": forced, "shareTables": [{"tag": "name", "index": 1}], "container": "woff2"}))
            .unwrap();
        let listed = s.execute("collection.list", &json!({"file": forced})).unwrap();
        assert_eq!(listed["format"], "woff2");
        assert_eq!(listed["members"][0]["postscriptName"], "Demo-Bold", "font 1's name table serves both");
        assert!(
            s.execute("collection.pack", &json!({"fonts": [a], "output": dir.join("x.ttc"), "shareTables": [{"tag": "name", "index": 5}]})).is_err()
        );
        assert!(!dir.join("x.ttc").exists(), "a refused pack writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn woff_compress_and_decompress_round_trip() {
        let dir = scratch("woff");
        let a = dir.join("A.ttf");
        let original = font(1, "Demo-Regular", false);
        std::fs::write(&a, &original).unwrap();
        std::fs::write(dir.join("meta.xml"), b"<?xml version=\"1.0\"?><metadata version=\"1.0\"/>").unwrap();
        let mut s = Session::new();
        for format in ["woff2", "woff"] {
            let r = s.execute("woff.compress", &json!({"font": a, "format": format, "metadata": dir.join("meta.xml")})).unwrap();
            let packed = dir.join(format!("A.{format}"));
            assert_eq!(r["output"], packed.to_string_lossy().as_ref());
            let out = dir.join(format!("back-{format}.ttf"));
            let meta = dir.join(format!("meta-{format}.xml"));
            let r = s.execute("woff.decompress", &json!({"font": packed, "output": out, "extractMetadata": meta})).unwrap();
            assert_eq!(r["metadata"], meta.to_string_lossy().as_ref());
            assert_eq!(std::fs::read(&meta).unwrap(), std::fs::read(dir.join("meta.xml")).unwrap());
            let back = sfnt::read(&std::fs::read(&out).unwrap(), 0).unwrap();
            let before = sfnt::read(&original, 0).unwrap();
            for (tag, data) in &before.tables {
                if tag != b"head" {
                    assert_eq!(back.get(*tag), Some(data.as_slice()), "{format}: '{}'", tag_text(*tag));
                }
            }
        }
        assert!(s.execute("woff.compress", &json!({"font": a, "quality": 3})).is_err(), "only the default quality is written");
        assert!(s.execute("woff.compress", &json!({"font": a, "format": "woff", "zopfli": 15})).is_err());
        assert!(s.execute("woff.decompress", &json!({"font": a})).is_err(), "a plain font is not a WOFF file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn woff_decompress_refuses_blocks_that_name_another_file_of_the_call() {
        let dir = scratch("woff-same");
        let a = dir.join("A.ttf");
        std::fs::write(&a, font(1, "Demo-Regular", false)).unwrap();
        std::fs::write(dir.join("meta.xml"), b"<?xml version=\"1.0\"?><metadata version=\"1.0\"/>").unwrap();
        let mut s = Session::new();
        s.execute("woff.compress", &json!({"font": a, "metadata": dir.join("meta.xml")})).unwrap();
        let packed = dir.join("A.woff2");
        let before = std::fs::read(&packed).unwrap();
        let out = dir.join("back.ttf");
        let other_spelling = dir.join(".").join("back.ttf");
        for meta in [&packed, &out, &other_spelling] {
            let r = s.execute("woff.decompress", &json!({"font": packed, "output": out, "extractMetadata": meta}));
            assert!(r.is_err(), "{} is refused", meta.display());
        }
        assert_eq!(std::fs::read(&packed).unwrap(), before, "the input is untouched");
        assert!(!out.exists(), "a refused call writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collection_unpack_names_never_collide_and_a_failed_write_is_undone() {
        let dir = scratch("unpack-names");
        // Member 0 is named like member 2's numbered fallback; member 2 has no usable name.
        let fonts = [font(1, "Demo-font2", false), font(2, "Demo-Bold", false), font(3, "", false)];
        let paths: Vec<PathBuf> = (0..3).map(|i| dir.join(format!("{i}.ttf"))).collect();
        for (path, bytes) in paths.iter().zip(&fonts) {
            std::fs::write(path, bytes).unwrap();
        }
        let ttc = dir.join("Demo.ttc");
        let mut s = Session::new();
        s.execute("collection.pack", &json!({"fonts": paths, "output": ttc})).unwrap();
        let out = dir.join("out");
        let r = s.execute("collection.unpack", &json!({"file": ttc, "outputDir": out})).unwrap();
        let written: Vec<&str> = r["outputs"].as_array().unwrap().iter().map(|o| o["path"].as_str().unwrap()).collect();
        assert_eq!(written.len(), 3);
        assert!(written.iter().collect::<std::collections::HashSet<_>>().len() == 3, "{written:?}");
        assert!(written[2].ends_with("Demo-font3.ttf"), "the next free number: {written:?}");
        // A write that fails part way removes the fonts this call wrote before it.
        let again = dir.join("again");
        std::fs::create_dir_all(again.join("Demo-Bold.ttf")).unwrap();
        assert!(s.execute("collection.unpack", &json!({"file": ttc, "outputDir": again})).is_err());
        assert!(!again.join("Demo-font2.ttf").exists(), "the first member's file is removed again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
