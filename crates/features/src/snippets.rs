//! Feature code snippets (M8.16; `docs/spec/12-feature-code.md` § 12): the built-in list and
//! the user's snippet files.
//!
//! Rows: `snippet.builtIn`, `snippet.fileNaming`, `snippet.fileFormat`, `snippet.insertion`,
//! `snippet.nativeFormat`.
//!
//! Files arrive as bytes (the engine reads the folder), so this module does no input or output.
//! Each file is read within the limits below; a file that cannot be read is skipped with a
//! warning naming it.

/// Most snippet files read.
pub const MAX_SNIPPET_FILES: usize = 256;
/// Most bytes of one snippet file.
pub const MAX_SNIPPET_FILE_BYTES: usize = 1 << 20;
/// Most snippets from all user files together.
pub const MAX_USER_SNIPPETS: usize = 10_000;

/// Where a snippet comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnippetSource {
    BuiltIn,
    /// The user's file of this name.
    User {
        file: String,
    },
}

/// One snippet: a name for the menu and the code it inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub code: String,
    pub source: SnippetSource,
}

/// The built-in snippets, in menu order.
pub fn builtin() -> Vec<Snippet> {
    // M8.16
    Vec::new()
}

/// What [`read_user`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserSnippets {
    pub snippets: Vec<Snippet>,
    /// One message per file skipped or cut short.
    pub warnings: Vec<String>,
}

/// The snippets of the user's files, given as (file name, bytes), in file-name order.
pub fn read_user(files: &[(&str, &[u8])]) -> UserSnippets {
    // M8.16
    let _ = files;
    UserSnippets::default()
}
