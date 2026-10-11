//! Messages about the code: where, how serious, what.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Warning,
    Error,
}

/// One message from the pre-checks or from fea-rs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The path of the file the message is about, as the request's [`crate::VirtualFiles`] names
    /// it, or as an include statement named a file read from disk. Empty: the request as a whole
    /// (a limit, a glyph order that cannot be used).
    pub file: String,
    /// Byte range in that file's text; empty when the message has no position.
    pub span: Range<usize>,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    /// A message about the request as a whole.
    pub fn request(severity: Severity, message: impl Into<String>) -> Self {
        Self { file: String::new(), span: 0..0, severity, message: message.into() }
    }

    /// A message at `span` of `file`.
    pub fn at(file: impl Into<String>, span: Range<usize>, severity: Severity, message: impl Into<String>) -> Self {
        Self { file: file.into(), span, severity, message: message.into() }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
