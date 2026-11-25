//! The shapers `preview.render` shapes with: the preview font of a master, or of a static instance.
//!
//! A master's shaper comes from the document's last feature compile while that describes the font, else from the preview
//! build kept for the revision, else from a build made here and kept for the other readers. An instance's comes from a
//! build of the instance, whose parameters change its features. The last [`MAX_SHAPERS`] are kept by document revision.

use std::collections::VecDeque;
use std::sync::Arc;

use fontcraft_compile::preview::{PreviewFont, instance_layout_font};
use fontcraft_compile::{DataRef, Options};
use fontcraft_textlayout::{PreviewShaper, shaping::Tag};

use crate::feature_state::{SessionFeaturesExt as _, SharedBuild, build_previews};
use crate::{Result, Session};

/// Shapers kept.
const MAX_SHAPERS: usize = 8;

/// Where a preview font is made: a master, or the static instance at an index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewPlace {
    Master(String),
    Instance(usize),
}

/// A document, its revision and the place.
type Key = (u64, u64, PreviewPlace);

#[derive(Default)]
struct Shapers(VecDeque<(Key, std::result::Result<Arc<PreviewShaper>, String>)>);

/// The shaper of a preview font.
pub fn shaper_of(preview: &PreviewFont) -> std::result::Result<Arc<PreviewShaper>, String> {
    let tags: Vec<Tag> = preview.features.iter().map(|f| f.tag).collect();
    PreviewShaper::new(Arc::from(preview.bytes.as_slice()), &preview.glyph_order, &preview.private_use, &tags)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

/// The compile options a preview font of the active document is built with: the session's glyph data, and the folder
/// included feature files are read from.
pub fn build_options(s: &Session) -> Result<Options> {
    let cx = crate::cmd::features_check::check_context(s)?;
    Ok(Options {
        glyph_data: DataRef(Some(cx.glyph_data)),
        feature_dir: cx.include_dir,
        include_check: crate::file_access::include_check(),
        ..Options::default()
    })
}

/// The shaper of the active document at `place`, or why its feature code gives none.
pub fn shaper(s: &mut Session, place: &PreviewPlace) -> Result<std::result::Result<Arc<PreviewShaper>, String>> {
    let doc = s.doc()?;
    let key: Key = (doc.id, doc.revision, place.clone());
    if let Some((_, kept)) = s.slot_mut::<Shapers>().0.iter().find(|(k, _)| *k == key) {
        return Ok(kept.clone());
    }
    let made = build(s, place)?;
    let kept = &mut s.slot_mut::<Shapers>().0;
    while kept.len() >= MAX_SHAPERS {
        kept.pop_front();
    }
    kept.push_back((key, made.clone()));
    Ok(made)
}

fn build(s: &Session, place: &PreviewPlace) -> Result<std::result::Result<Arc<PreviewShaper>, String>> {
    let doc = s.doc()?;
    let options = build_options(s)?;
    let master = match place {
        PreviewPlace::Master(master) => master,
        PreviewPlace::Instance(i) => {
            return Ok(instance_layout_font(&doc.font, *i, &options).map_err(|e| e.to_string()).and_then(|f| shaper_of(&f)));
        }
    };
    if let Some(state) = s.feature_state()
        && state.master == *master
        && state.is_current(s, doc)
        && let Some(preview) = &state.preview
    {
        return Ok(shaper_of(preview));
    }
    let builds = s.preview_builds();
    let result = match builds.get(doc.id, doc.revision, master) {
        Some(kept) => kept.result.clone(),
        None => {
            let result = build_previews(&doc.font, master, &options).map(Arc::new).map_err(Arc::new);
            builds.put(SharedBuild { doc: doc.id, revision: doc.revision, master: master.clone(), source_hash: None, result: result.clone() });
            result
        }
    };
    Ok(result.map_err(|e| e.to_string()).and_then(|b| shaper_of(&b.gsub_only)))
}
