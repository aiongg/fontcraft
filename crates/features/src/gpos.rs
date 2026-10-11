//! The `GPOS` table of the kern writer's lookups and features.
//!
//! The script, language system and feature records are built as fea-rs builds them (fontc,
//! https://github.com/googlefonts/fontc, `fea-rs/src/compile/lookups.rs`, `PosSubBuilder`,
//! Apache-2.0, at commit 4504578c3f74c2739f7e55cae133bbfe773f6d8c): one feature record for each
//! feature tag and list of lookups, numbered in the order of the feature keys (tag, language,
//! script); scripts and language systems sorted by tag, `dflt` as the default language system.
//! Overflowing offsets are resolved when the table is written: write-fonts splits pair subtables
//! and moves lookups to extension lookups as needed.

use std::collections::BTreeMap;

use write_fonts::tables::gpos::{Gpos, PositionLookup, PositionLookupList};
use write_fonts::tables::layout::builders::{Builder, LookupBuilder};
use write_fonts::tables::layout::{Feature, FeatureList, FeatureRecord, LangSys, LangSysRecord, Script, ScriptList, ScriptRecord};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::types::Tag;

use crate::FeatureError;
use crate::kern::Kerns;
use crate::properties::DFLT_LANG;

fn too_many(what: &'static str) -> FeatureError {
    FeatureError::TooMany { what, limit: usize::from(u16::MAX) }
}

/// The table, or `None` when the writer made no lookups.
pub(crate) fn build(kerns: Kerns) -> Result<Option<Gpos>, FeatureError> {
    if kerns.lookups.is_empty() || kerns.features.is_empty() {
        return Ok(None);
    }
    let mut features: BTreeMap<(Tag, Vec<u16>), u16> = BTreeMap::new();
    let mut scripts: BTreeMap<Tag, BTreeMap<Tag, Vec<u16>>> = BTreeMap::new();
    for (key, lookups) in &kerns.features {
        let lookups = lookups.iter().map(|i| u16::try_from(*i).map_err(|_| too_many("lookups"))).collect::<Result<Vec<u16>, _>>()?;
        let next = u16::try_from(features.len()).map_err(|_| too_many("features"))?;
        let idx = *features.entry((key.feature, lookups)).or_insert(next);
        scripts.entry(key.script).or_default().entry(key.language).or_default().push(idx);
    }

    let mut records: Vec<Option<FeatureRecord>> = vec![None; features.len()];
    for ((tag, lookups), idx) in features {
        if let Some(slot) = records.get_mut(usize::from(idx)) {
            *slot = Some(FeatureRecord::new(tag, Feature::new(None, lookups)));
        }
    }
    let feature_list = FeatureList::new(records.into_iter().flatten().collect());

    let script_records = scripts
        .into_iter()
        .map(|(script_tag, langs)| {
            let mut script = Script::default();
            for (lang_tag, indices) in langs {
                let lang_sys = LangSys::new(indices);
                if lang_tag == DFLT_LANG {
                    script.default_lang_sys = lang_sys.into();
                } else {
                    script.lang_sys_records.push(LangSysRecord::new(lang_tag, lang_sys));
                }
            }
            ScriptRecord::new(script_tag, script)
        })
        .collect();

    let mut var_store = VariationStoreBuilder::new(0);
    let lookups = kerns
        .lookups
        .into_iter()
        .map(|lookup| PositionLookup::Pair(LookupBuilder::new_with_lookups(lookup.flags, None, vec![lookup.builder]).build(&mut var_store)))
        .collect();
    Ok(Some(Gpos::new(ScriptList::new(script_records), feature_list, PositionLookupList::new(lookups))))
}
