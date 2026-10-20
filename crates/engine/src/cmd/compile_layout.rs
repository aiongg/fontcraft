//! `font.compile` with `interpolateBinaryLayout`: fontmake's `--interpolate-binary-layout [DIR]`
//! on a compile of instances (fontmake `font_project.py`, `run_from_designspace` and `save_otfs`,
//! Apache License 2.0).
//!
//! Each instance is compiled as usual. Its `GPOS` is then replaced by the one interpolated from
//! the compiled masters at the instance's location, and its `GDEF` and `GSUB` by the default
//! master's when that master has them (`instance::layout::BinaryLayout`, shared with
//! `font.instantiate`). The masters are found as `{DIR}/{stem}.{ext}`, `DIR` defaulting to
//! `master_ttf` or `master_otf` as the format being built, relative to the source's folder.
//! Masters built with `mastersAsInstances`, and the first master of a font without instances,
//! keep their own layout: fontmake replaces the layout of instances only.
//!
//! Every layout is interpolated before anything is compiled, and the fonts go through the
//! compile's staged writes, so a run that fails writes nothing.

use fontcraft_compile::Flavour;
use fontcraft_model::Font;
use serde_json::Value;

use super::compile::Job;
use crate::params::Params;
use crate::{EngineError, Result};

/// The folder of the compiled masters `interpolateBinaryLayout` names: `None` when the option is
/// off, `Some(None)` for the default folder.
pub(super) fn requested(p: &Params<'_>) -> Result<Option<Option<String>>> {
    match p.get("interpolateBinaryLayout") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(None)),
        Some(Value::String(dir)) if !dir.is_empty() => Ok(Some(Some(dir.clone()))),
        Some(_) => Err(p.bad("`interpolateBinaryLayout` is true, false or the folder of the compiled masters")),
    }
}

/// The interpolated layouts of a compile, per outline format.
pub(super) struct Layouts {
    #[cfg(not(target_arch = "wasm32"))]
    by_flavour: Vec<(Flavour, super::instance::layout::BinaryLayout)>,
    /// No instance is built, so no layout is replaced.
    pub(super) unused: bool,
}

impl Layouts {
    /// Reads `interpolateBinaryLayout` and interpolates the layout of every instance among
    /// `jobs`, for every format in `flavours`. `None` when the option is off.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn read(p: &Params<'_>, path: Option<&std::path::Path>, font: &Font, jobs: &[Job], flavours: &[Flavour]) -> Result<Option<Self>> {
        let Some(dir) = requested(p)? else { return Ok(None) };
        let indices: Vec<usize> = jobs
            .iter()
            .filter_map(|j| match &j.target {
                fontcraft_model::Target::Instance(i) => Some(*i),
                fontcraft_model::Target::Master(_) => None,
            })
            .collect();
        if indices.is_empty() {
            return Ok(Some(Self { by_flavour: Vec::new(), unused: true }));
        }
        let Some(input) = path else {
            return Err(p.bad("`interpolateBinaryLayout` finds the compiled masters beside the source's file, and the document has no file"));
        };
        let ds = fontcraft_interp::Designspace::new(font).map_err(|e| p.bad(e.to_string()))?;
        let mut by_flavour = Vec::with_capacity(flavours.len());
        for &flavour in flavours {
            let ext = if flavour == Flavour::TrueType { "ttf" } else { "otf" };
            let layout = super::instance::layout::BinaryLayout::load_for(p, input, font, &ds, &indices, ext, dir.as_deref())?;
            by_flavour.push((flavour, layout));
        }
        Ok(Some(Self { by_flavour, unused: false }))
    }

    /// The web build has no compiled masters to read.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn read(p: &Params<'_>, _: Option<&std::path::Path>, _: &Font, _: &[Job], _: &[Flavour]) -> Result<Option<Self>> {
        match requested(p)? {
            None => Ok(None),
            Some(_) => Err(EngineError::Unsupported("reading compiled masters in the browser".into())),
        }
    }

    /// The compiled font `bytes` of `job` in `flavour`, with the layout of an instance replaced.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn replace(&self, p: &Params<'_>, job: &Job, flavour: Flavour, bytes: Vec<u8>) -> Result<Vec<u8>> {
        let fontcraft_model::Target::Instance(index) = &job.target else { return Ok(bytes) };
        match self.by_flavour.iter().find(|(f, _)| *f == flavour) {
            Some((_, layout)) => layout.replace(p, *index, &bytes),
            None => Err(EngineError::Internal(format!("no layout was interpolated for {} fonts", super::compile::format_name(flavour)))),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn replace(&self, _: &Params<'_>, _: &Job, _: Flavour, bytes: Vec<u8>) -> Result<Vec<u8>> {
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::super::instance::layout::tests::glyphs_source_with;
    use fontcraft_tables::sfnt::Tag;
    use serde_json::json;

    const GPOS: Tag = *b"GPOS";
    const GDEF: Tag = *b"GDEF";

    /// A two-master Glyphs source whose compiled masters (kerned -60 and -140) sit where fontmake
    /// writes them, while the source itself kerns -50 and -110. Compiling its Regular instance
    /// (wght 400, master weights 0.75 and 0.25) with `interpolateBinaryLayout` gives the GPOS
    /// interpolated from the compiled masters; the masters built beside it keep their own.
    #[test]
    fn compile_interpolates_the_instances_layout_from_compiled_masters() {
        let dir = std::env::temp_dir().join(format!("fontcraft-compile-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("masters")).unwrap();
        std::fs::create_dir_all(dir.join("master_ttf")).unwrap();
        let masters_source = dir.join("masters/Test.glyphs");
        std::fs::write(&masters_source, glyphs_source_with(-60, -140, false)).unwrap();
        let input = dir.join("Test.glyphs");
        std::fs::write(&input, glyphs_source_with(-50, -110, false)).unwrap();

        let (font, _) = crate::io::open(&masters_source, None).unwrap();
        let mut s = crate::Session::new();
        s.push_document(font, Some(masters_source.clone()));
        let built = s
            .execute(
                "font.compile",
                &json!({ "formats": ["ttf"], "instances": false, "mastersAsInstances": true, "destination": dir.join("masters/ttf").display().to_string() }),
            )
            .unwrap();
        let mut masters = Vec::new();
        for (label, stem) in [("Light Two", "TestSans-LightTwo"), ("Bold", "TestSans-Bold")] {
            let f = built["fonts"].as_array().unwrap().iter().find(|f| f["instance"] == json!(label)).unwrap();
            let to = dir.join("master_ttf").join(format!("{stem}.ttf"));
            std::fs::rename(f["files"][0]["path"].as_str().unwrap(), &to).unwrap();
            masters.push(fontcraft_tables::read(&std::fs::read(&to).unwrap(), 0).unwrap());
        }
        let expected = fontcraft_tables::interpolate_layout::interpolate_gpos(&masters, 0, &[0.75, 0.25]).unwrap().unwrap();

        let (font, _) = crate::io::open(&input, None).unwrap();
        let mut s = crate::Session::new();
        s.push_document(font, Some(input.clone()));
        let out = dir.join("out");
        let r = s
            .execute(
                "font.compile",
                &json!({ "formats": ["ttf"], "mastersAsInstances": true, "interpolateBinaryLayout": true, "destination": out.display().to_string() }),
            )
            .unwrap();
        let file = |label: &str| {
            let f = r["fonts"].as_array().unwrap().iter().find(|f| f["instance"] == json!(label)).unwrap();
            fontcraft_tables::read(&std::fs::read(f["files"][0]["path"].as_str().unwrap()).unwrap(), 0).unwrap()
        };
        let regular = file("Regular");
        assert_eq!(regular.get(GPOS), Some(expected.as_slice()), "the instance's GPOS is interpolated from the compiled masters");
        assert_eq!(regular.get(GDEF), masters[0].get(GDEF), "GDEF follows the default master");
        assert_ne!(regular.get(*b"hmtx"), masters[0].get(*b"hmtx"), "the outlines and metrics are the instance's own");
        assert_ne!(file("Bold").get(GPOS), masters[1].get(GPOS), "a master built as an instance keeps the source's own kerning");

        let own = s
            .execute("font.compile", &json!({ "formats": ["ttf"], "instances": ["Regular"], "destination": dir.join("own").display().to_string() }))
            .unwrap();
        let own = fontcraft_tables::read(&std::fs::read(own["fonts"][0]["files"][0]["path"].as_str().unwrap()).unwrap(), 0).unwrap();
        assert_ne!(own.get(GPOS), Some(expected.as_slice()), "without the option the source's own kerning");

        let missing = dir.join("missing");
        let r = s.execute(
            "font.compile",
            &json!({ "formats": ["ttf"], "interpolateBinaryLayout": "elsewhere", "destination": missing.display().to_string() }),
        );
        assert!(r.is_err(), "no compiled masters in that folder");
        assert!(!missing.exists(), "a refused run writes nothing");
        let r =
            s.execute("font.compile", &json!({ "formats": ["otf"], "interpolateBinaryLayout": true, "destination": missing.display().to_string() }));
        assert!(r.is_err(), "CFF instances read the masters from master_otf, which is empty");
        assert!(!missing.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Glyphs source without a family name is named as glyphsLib names it, `Unnamed font`, and
    /// not after its file.
    #[test]
    fn an_unnamed_glyphs_source_compiles_to_unnamed_font_files() {
        let dir = std::env::temp_dir().join(format!("fontcraft-compile-unnamed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("kerning_test.glyphs");
        std::fs::write(&input, glyphs_source_with(-50, -110, false).replace("familyName = \"Test Sans\";\n", "")).unwrap();
        let (font, _) = crate::io::open(&input, None).unwrap();
        assert!(font.family_name.is_empty());
        let mut s = crate::Session::new();
        s.push_document(font, Some(input.clone()));
        let out = dir.join("out");
        let r = s.execute("font.compile", &json!({ "formats": ["ttf"], "destination": out.display().to_string() })).unwrap();
        assert_eq!(r["fonts"][0]["files"][0]["name"], json!("Unnamedfont-Regular.ttf"), "{r}");
        assert!(out.join("Unnamedfont-Regular.ttf").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
