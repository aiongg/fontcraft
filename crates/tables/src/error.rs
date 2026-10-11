//! The error every tool returns.

/// Why a tool did not run. A tool that fails produces no output.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TablesError {
    /// The tool's package has not landed yet. The text names the tool.
    #[error("{0} is not built yet")]
    NotBuilt(&'static str),
    /// The input is damaged: an offset past the end, a bad count, a malformed table.
    #[error("{0}")]
    Invalid(String),
    /// The input is well-formed but uses something this version does not handle.
    #[error("not supported yet: {0}")]
    Unsupported(String),
    /// A count or size read from the input or given in the options passes a limit of
    /// [`crate::limits`].
    #[error("{what} is larger than the limit of {limit}")]
    TooLarge { what: &'static str, limit: usize },
    /// The tool needs a table the font does not have.
    #[error("the font has no '{0}' table")]
    MissingTable(String),
    /// The options contradict each other or name something the font does not have.
    #[error("{0}")]
    BadOptions(String),
    /// The WOFF, WOFF2 or sfnt container could not be read or written.
    #[error(transparent)]
    Container(#[from] fontcraft_container::Error),
}

impl TablesError {
    pub(crate) fn invalid(what: impl Into<String>) -> Self {
        TablesError::Invalid(what.into())
    }
}
