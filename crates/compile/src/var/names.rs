//! Name records of a variable font: the names of axes and named instances, and the family,
//! style, PostScript and file names the variable font parameters set (M9.6).
//!
//! Rows: `compile.nameIdAllocation`, `instances.namedInstancesInVariable` (records), parameters
//! `Variable Font Family Name`, `Variable Font File Name`, `Variable Font Style Name`,
//! `Variable SubfamilyName`, `variablePostscriptFontName`, `variableStyleName`,
//! `variableStyleNames`, `variationsPostScriptNamePrefix`.
//!
//! Contract: name IDs for `fvar` and `STAT` are allocated from [`FIRST_FONT_NAME_ID`] upwards in
//! the order the builders ask, the same text getting the same ID, as fontc allocates them; the
//! static name records (IDs below 256) keep their IDs. At most 32,767 IDs are allocated.
//!
//! Reuse of static records follows fontc (`fontir` `StaticMetadata::new`, Apache-2.0): axis and
//! instance names never reuse an ID below 256, except that the named instance at the default
//! location takes ID 2 or 17 when that record holds its name; `STAT` labels may reuse any static
//! record ([`NameAllocator::id_for_any`]).

use std::collections::BTreeMap;

use fontcraft_interp::{Designspace, InterpError};
use fontcraft_model::{Font, Instance};
use write_fonts::types::NameId;

use crate::CompileError;

/// The first name ID free for font-specific names.
pub const FIRST_FONT_NAME_ID: u16 = 256;
/// The last name ID allocated (IDs above 32,767 are reserved).
pub const LAST_FONT_NAME_ID: u16 = 32_767;
/// The most static records an allocator keeps for reuse.
pub const MAX_STATIC_RECORDS: usize = 4_096;
/// The longest PostScript name (name ID 6) and PostScript prefix (name ID 25) written.
pub const MAX_POSTSCRIPT_LEN: usize = 63;

pub const VARIABLE_FAMILY_NAME: &str = "Variable Font Family Name";
pub const VARIABLE_FILE_NAME: &str = "Variable Font File Name";
pub const VARIABLE_STYLE_NAME: &str = "Variable Font Style Name";
pub const VARIABLE_SUBFAMILY_NAME: &str = "Variable SubfamilyName";
pub const VARIABLE_POSTSCRIPT_NAME: &str = "variablePostscriptFontName";
pub const VARIATIONS_POSTSCRIPT_PREFIX: &str = "variationsPostScriptNamePrefix";
/// The fvar name of a static instance (parameter or property).
pub const VARIABLE_STYLE_NAME_KEY: &str = "variableStyleName";
/// The fvar name of a static instance, one text per language (property).
pub const VARIABLE_STYLE_NAMES: &str = "variableStyleNames";
/// The file name parameter the oracles read on a variable font setting.
pub const FILE_NAME: &str = "fileName";
/// A static instance's own PostScript name (property), the fallback for its `fvar` PostScript
/// name.
pub const POSTSCRIPT_FONT_NAME: &str = "postscriptFontName";

/// Hands out name IDs for the variable tables.
#[derive(Clone, Debug, PartialEq)]
pub struct NameAllocator {
    next: u16,
    records: Vec<(NameId, String)>,
    /// Text to its allocated ID, for the lookups of [`Self::id_for`].
    index: BTreeMap<String, NameId>,
    /// The static records (IDs below 256) a default instance or a `STAT` label may reuse.
    fixed: Vec<(NameId, String)>,
}

impl Default for NameAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl NameAllocator {
    /// An allocator starting at [`FIRST_FONT_NAME_ID`].
    pub fn new() -> Self {
        Self { next: FIRST_FONT_NAME_ID, records: Vec::new(), index: BTreeMap::new(), fixed: Vec::new() }
    }

    /// An allocator that knows the static name records (`(ID, text)`, IDs below 256; others are
    /// ignored), so a default instance and `STAT` labels can reuse them.
    pub fn with_static(records: impl IntoIterator<Item = (NameId, String)>) -> Self {
        let fixed = records.into_iter().filter(|(id, _)| id.to_u16() < FIRST_FONT_NAME_ID).take(MAX_STATIC_RECORDS).collect();
        Self { fixed, ..Self::new() }
    }

    /// The ID of `text`: the one it already has, else the next free one. Fails past
    /// [`LAST_FONT_NAME_ID`].
    pub fn id_for(&mut self, text: &str) -> Result<NameId, CompileError> {
        if let Some(id) = self.index.get(text) {
            return Ok(*id);
        }
        if self.next > LAST_FONT_NAME_ID {
            return Err(CompileError::Table {
                table: "name",
                message: format!("more than {} names for axes, instances and style attributes", LAST_FONT_NAME_ID - FIRST_FONT_NAME_ID + 1),
            });
        }
        let id = NameId::new(self.next);
        self.next = self.next.saturating_add(1);
        self.records.push((id, text.to_string()));
        self.index.insert(text.to_string(), id);
        Ok(id)
    }

    /// The subfamily name ID of the named instance at the default location: ID 2 or 17 when that
    /// static record holds `text` (the smaller one when both do), else [`Self::id_for`].
    pub fn id_for_default_instance(&mut self, text: &str) -> Result<NameId, CompileError> {
        let reusable = [NameId::SUBFAMILY_NAME, NameId::TYPOGRAPHIC_SUBFAMILY_NAME];
        let found = self.fixed.iter().filter(|(id, t)| reusable.contains(id) && t == text).map(|(id, _)| *id).min();
        match found {
            Some(id) => Ok(id),
            None => self.id_for(text),
        }
    }

    /// The smallest static record ID holding `text`, else [`Self::id_for`]. For `STAT` labels and
    /// the elided fallback name, which may reuse any record.
    pub fn id_for_any(&mut self, text: &str) -> Result<NameId, CompileError> {
        match self.fixed.iter().filter(|(_, t)| t == text).map(|(id, _)| *id).min() {
            Some(id) => Ok(id),
            None => self.id_for(text),
        }
    }

    /// The records allocated so far, in ID order.
    pub fn records(&self) -> &[(NameId, String)] {
        &self.records
    }
}

/// The names of one variable font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableNames {
    pub family: String,
    /// The default instance's style name.
    pub style: String,
    pub postscript_name: Option<String>,
    /// The prefix of the named instances' PostScript names.
    pub postscript_prefix: Option<String>,
    /// The file name without extension.
    pub file_stem: String,
}

/// A text parameter's value, trimmed; `None` when absent, not text, or empty.
fn param_text(params: &fontcraft_model::Params, name: &str) -> Option<String> {
    params.get(name).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// A property's default-language value, trimmed; `None` when absent or empty.
fn property_text(properties: &fontcraft_model::Properties, key: &str) -> Option<String> {
    properties.value(key).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `text` as a file name: path separators, control characters and the characters file systems
/// refuse replaced by `-`; leading dots dropped so the file is never hidden or a parent path.
fn file_safe(text: &str) -> String {
    let replaced: String =
        text.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c }).collect();
    replaced.trim().trim_start_matches('.').to_string()
}

/// `text` as a PostScript name: printable ASCII without spaces and without the characters the
/// format reserves, at most [`MAX_POSTSCRIPT_LEN`] characters.
fn postscript_safe(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_graphic() && !matches!(c, '[' | ']' | '(' | ')' | '{' | '}' | '<' | '>' | '/' | '%'))
        .take(MAX_POSTSCRIPT_LEN)
        .collect()
}

/// The variable font setting at `setting`, refused when it is not one.
fn setting_instance(font: &Font, setting: usize) -> Result<&Instance, CompileError> {
    let inst = font.instances.get(setting).ok_or(InterpError::UnknownInstance(setting))?;
    if inst.kind.as_deref() != Some("variable") {
        return Err(InterpError::NotAVariableSetting(setting).into());
    }
    Ok(inst)
}

/// The names of the variable font of `setting` (`None`: the default font), from the setting's
/// and the font's parameters.
///
/// - Family: Variable Font Family Name (the setting's, else the font's), else the setting's
///   family name, else the font's.
/// - Style: Variable SubfamilyName, else Variable Font Style Name or `variableStyleName`, else
///   the origin master's name.
/// - PostScript name: `variablePostscriptFontName` (property or parameter) of the setting, else
///   its PostScript Font Name, else the font's parameter; `None` leaves name ID 6 to the static
///   derivation.
/// - PostScript prefix (name ID 25): `variationsPostScriptNamePrefix`, ASCII letters and digits
///   only.
/// - File name: Variable Font File Name (or the setting's `fileName`), else the oracles' default:
///   the font's family name, `-` and the setting's name unless it is Regular, then `VF`, spaces
///   removed.
pub fn variable_names(font: &Font, setting: Option<usize>) -> Result<VariableNames, CompileError> {
    let inst = setting.map(|s| setting_instance(font, s)).transpose()?;
    // The setting's value first, then the font's.
    let param = |name: &str| inst.and_then(|i| param_text(&i.params, name)).or_else(|| param_text(&font.info.params, name));
    let property = |key: &str| inst.and_then(|i| property_text(&i.properties, key));

    let family = param(VARIABLE_FAMILY_NAME).or_else(|| property("familyNames")).unwrap_or_else(|| font.family_name.trim().to_string());

    let style = match param(VARIABLE_SUBFAMILY_NAME).or_else(|| param(VARIABLE_STYLE_NAME)).or_else(|| param(VARIABLE_STYLE_NAME_KEY)) {
        Some(s) => s,
        None => origin_name(font, setting),
    };

    let postscript_name = property(VARIABLE_POSTSCRIPT_NAME)
        .or_else(|| param(VARIABLE_POSTSCRIPT_NAME))
        .or_else(|| property(POSTSCRIPT_FONT_NAME))
        .map(|s| postscript_safe(&s))
        .filter(|s| !s.is_empty());

    let postscript_prefix = param(VARIATIONS_POSTSCRIPT_PREFIX)
        .or_else(|| property_text(&font.info.properties, VARIATIONS_POSTSCRIPT_PREFIX))
        .map(|s| s.chars().filter(char::is_ascii_alphanumeric).take(MAX_POSTSCRIPT_LEN).collect::<String>())
        .filter(|s| !s.is_empty());

    let explicit = inst
        .and_then(|i| param_text(&i.params, VARIABLE_FILE_NAME).or_else(|| param_text(&i.params, FILE_NAME)))
        .or_else(|| param_text(&font.info.params, VARIABLE_FILE_NAME))
        .map(|s| file_safe(&s))
        .filter(|s| !s.is_empty());
    let file_stem = match explicit {
        Some(s) => s,
        None => {
            let mut stem = font.family_name.clone();
            if let Some(name) = inst.map(|i| i.name.trim()).filter(|n| !n.is_empty() && *n != "Regular") {
                stem.push('-');
                stem.push_str(name);
            }
            stem.push_str("VF");
            file_safe(&stem.replace(' ', ""))
        }
    };

    Ok(VariableNames { family, style, postscript_name, postscript_prefix, file_stem })
}

/// The name of the variable font's origin master; the first master's when the designspace cannot
/// be built (the compile reports that error itself).
fn origin_name(font: &Font, setting: Option<usize>) -> String {
    let designspace = match setting {
        Some(s) => Designspace::for_setting(font, s),
        None => Designspace::new(font),
    };
    designspace
        .ok()
        .and_then(|d| d.origin().map(|m| m.name.clone()))
        .or_else(|| font.masters.first().map(|m| m.name.clone()))
        .unwrap_or_else(|| "Regular".to_string())
}

/// The name a static instance has in `fvar`: its `variableStyleNames` property, else its
/// `variableStyleName` or Variable Font Style Name (property or parameter), else its name.
pub fn instance_fvar_name(instance: &Instance) -> String {
    property_text(&instance.properties, VARIABLE_STYLE_NAMES)
        .or_else(|| property_text(&instance.properties, VARIABLE_STYLE_NAME_KEY))
        .or_else(|| param_text(&instance.params, VARIABLE_STYLE_NAME_KEY))
        .or_else(|| param_text(&instance.params, VARIABLE_STYLE_NAME))
        .unwrap_or_else(|| instance.name.trim().to_string())
}

/// The PostScript name a static instance has in `fvar`: its `variablePostscriptFontName`, else its
/// PostScript Font Name, as fontc reads them.
pub fn instance_postscript_name(instance: &Instance) -> Option<String> {
    property_text(&instance.properties, VARIABLE_POSTSCRIPT_NAME)
        .or_else(|| property_text(&instance.properties, POSTSCRIPT_FONT_NAME))
        .or_else(|| param_text(&instance.params, VARIABLE_POSTSCRIPT_NAME))
        .or_else(|| param_text(&instance.params, POSTSCRIPT_FONT_NAME))
        .map(|s| postscript_safe(&s))
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Str, Value};
    use fontcraft_model::{Instance, Param};

    use super::*;

    fn text(s: &str) -> Value {
        Value::String(Str::new(s))
    }

    fn font_with_setting(name: &str) -> Font {
        let mut f = Font::new("My Family");
        Arc::make_mut(&mut f.instances).push(Instance { name: "Bold".into(), ..Default::default() });
        Arc::make_mut(&mut f.instances).push(Instance { name: name.into(), kind: Some("variable".into()), ..Default::default() });
        f
    }

    #[test]
    fn name_ids_are_shared_and_allocated_from_256() {
        let mut names = NameAllocator::with_static([(NameId::new(1), "Fam".to_string()), (NameId::new(2), "Regular".to_string())]);
        assert_eq!(names.id_for("Weight").unwrap(), NameId::new(256));
        assert_eq!(names.id_for("Light").unwrap(), NameId::new(257));
        assert_eq!(names.id_for("Weight").unwrap(), NameId::new(256));
        // The default instance reuses ID 2; other names never take a static ID.
        assert_eq!(names.id_for_default_instance("Regular").unwrap(), NameId::new(2));
        assert_eq!(names.id_for("Regular").unwrap(), NameId::new(258));
        assert_eq!(names.id_for_default_instance("Book").unwrap(), NameId::new(259));
        assert_eq!(names.id_for_any("Fam").unwrap(), NameId::new(1));
        let ids: Vec<u16> = names.records().iter().map(|(id, _)| id.to_u16()).collect();
        assert_eq!(ids, vec![256, 257, 258, 259]);
    }

    #[test]
    fn name_ids_stop_at_the_reserved_range() {
        let mut names = NameAllocator::new();
        for i in 0..usize::from(LAST_FONT_NAME_ID - FIRST_FONT_NAME_ID + 1) {
            names.id_for(&i.to_string()).unwrap();
        }
        assert!(names.id_for("one more").is_err());
        assert!(names.id_for("0").is_ok());
    }

    #[test]
    fn variable_names_default_to_the_reference_file_name() {
        let f = font_with_setting("Italic");
        let n = variable_names(&f, Some(1)).unwrap();
        assert_eq!(n.family, "My Family");
        assert_eq!(n.file_stem, "MyFamily-ItalicVF");
        assert_eq!(n.postscript_name, None);
        assert_eq!(n.postscript_prefix, None);
        let n = variable_names(&font_with_setting("Regular"), Some(1)).unwrap();
        assert_eq!(n.file_stem, "MyFamilyVF");
        assert_eq!(variable_names(&f, None).unwrap().file_stem, "MyFamilyVF");
        // An instance that is not a setting is refused.
        assert!(variable_names(&f, Some(0)).is_err());
        assert!(variable_names(&f, Some(7)).is_err());
    }

    #[test]
    fn variable_name_parameters_override_the_defaults() {
        let mut f = font_with_setting("Italic");
        Arc::make_mut(&mut f.info).params.0.push(Param::new(VARIABLE_FAMILY_NAME, text("Font Family VF")));
        Arc::make_mut(&mut f.info).params.0.push(Param::new(VARIATIONS_POSTSCRIPT_PREFIX, text("My Fam-2!")));
        let setting = &mut Arc::make_mut(&mut f.instances)[1];
        setting.params.0.push(Param::new(VARIABLE_FILE_NAME, text("../Out/Var")));
        setting.params.0.push(Param::new(VARIABLE_SUBFAMILY_NAME, text("Book")));
        setting.properties.set(VARIABLE_POSTSCRIPT_NAME, "MyFamVar-Book Italic", false);
        let n = variable_names(&f, Some(1)).unwrap();
        assert_eq!(n.family, "Font Family VF");
        assert_eq!(n.style, "Book");
        assert_eq!(n.postscript_name.as_deref(), Some("MyFamVar-BookItalic"));
        assert_eq!(n.postscript_prefix.as_deref(), Some("MyFam2"));
        assert_eq!(n.file_stem, "-Out-Var");
    }

    #[test]
    fn instance_names_for_fvar() {
        let mut inst = Instance { name: "Bold".into(), ..Default::default() };
        assert_eq!(instance_fvar_name(&inst), "Bold");
        assert_eq!(instance_postscript_name(&inst), None);
        inst.properties.set(VARIABLE_STYLE_NAMES, "Bold Condensed", true);
        inst.properties.set(POSTSCRIPT_FONT_NAME, "PostBold", false);
        assert_eq!(instance_fvar_name(&inst), "Bold Condensed");
        assert_eq!(instance_postscript_name(&inst).as_deref(), Some("PostBold"));
        inst.properties.set(VARIABLE_POSTSCRIPT_NAME, "VarBold", false);
        assert_eq!(instance_postscript_name(&inst).as_deref(), Some("VarBold"));
    }
}
