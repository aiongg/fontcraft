//! One feature file per target, from the entries (M8.3).
//!
//! Rows: `featureEditor.outputOrder`, `featureEditor.featureBody`, `featureEditor.classContent`,
//! `featureEditor.afdkoSyntax`, `featureCode.mm.singleSource`, `fontInfo.features.stylisticSetNames`
//! (the `featureNames` output), `kerning.groupClassNames`, `kerning.explicitKernFeature`,
//! `fea.featureNames`, `font.features.expandCode`'s data path.
//!
//! The pipeline for one entry, each pass returning its [`OffsetMap`]:
//! [`crate::tokens::expand`] → [`crate::lower::lower`] (`#ifdef VARIABLE` and the syntax
//! extensions) → [`crate::variable::lower`] (variable target only). Each entry becomes one
//! virtual file holding its full statement (`@Name = [ … ];`, `feature tag { … } tag;`,
//! a prefix's code as it is), and the root file includes them in the oracles' order: classes,
//! prefixes, features, each group in sidebar order. Disabled entries are left out. Kerning
//! groups the code refers to are defined as `@MMK_L_`/`@MMK_R_` classes before the first entry.

use fontcraft_fea::VirtualFiles;
use fontcraft_model::{Features, Font};

use crate::source_map::SourceMap;
use crate::{FeatureError, Target, TargetValues};

/// The path of the root file of every assembly.
pub const ROOT_FILE: &str = "features.fea";

/// The assembled feature file of one target.
#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub files: VirtualFiles,
    pub map: SourceMap,
}

impl Assembly {
    /// Every file's text joined in include order, includes replaced by the files they name:
    /// what `font.features.expandCode` shows.
    pub fn expanded_text(&self) -> String {
        // M8.3
        self.files.root_text().to_string()
    }
}

/// Assembles `entries` (the font's own, or the result of Update and the instance parameters)
/// for `target`. `font` supplies the kerning groups and the stylistic set names' context;
/// `values` the token values. Diagnostics of the passes come back as [`FeatureError::Code`],
/// mapped to the entries.
pub fn assemble(font: &Font, entries: &Features, target: &Target, values: &dyn TargetValues) -> Result<Assembly, FeatureError> {
    // M8.3
    let _ = (font, entries, target, values);
    let files = VirtualFiles::new(ROOT_FILE, "").map_err(|d| FeatureError::Unsupported(d.message))?;
    Ok(Assembly { files, map: SourceMap::new() })
}
