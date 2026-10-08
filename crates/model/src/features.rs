//! Feature code as the source holds it: prefixes, classes and features, each a list of snippets.
//!
//! A snippet is the body of one item (the `feature tag { … } tag;` wrapper of a feature is implied,
//! as in the reference's format), with its switches and notes. The same tag may occur more than
//! once.

use serde::{Deserialize, Serialize};

use crate::Extra;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Features {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefixes: Vec<Snippet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<Snippet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<Snippet>,
}

impl Features {
    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty() && self.classes.is_empty() && self.features.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    /// The feature tag, or the class or prefix name (a class name without its `@`).
    pub name: String,
    pub code: String,
    /// The editor writes this snippet's code itself.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// The file a package stored the code in, when it is not the one the naming rule gives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}
