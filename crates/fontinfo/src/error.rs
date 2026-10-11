//! The crate's error types, one per area. Each message is written for the user: commands return it
//! as their refusal, unchanged.

use fontcraft_model::ModelError;

/// Why a parameter value or a field value was refused ([`crate::value`], [`crate::fields`]).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValueError {
    /// The value is not of the parameter's kind (`expected` names the kind: "a number"…).
    #[error("`{name}` takes {expected}")]
    WrongKind { name: String, expected: &'static str },
    /// The value is of the right kind but breaks the parameter's own syntax.
    #[error("`{name}`: {message}")]
    Syntax { name: String, message: String },
    /// A number outside the range the parameter or field allows.
    #[error("`{name}` must be from {min} to {max}")]
    OutOfRange { name: String, min: f64, max: f64 },
    /// A number that is not finite.
    #[error("`{name}` must be a finite number")]
    NotFinite { name: String },
    /// The parameter may not be set at this scope.
    #[error("`{name}` cannot be set on {scope}")]
    WrongScope { name: String, scope: &'static str },
    /// A JSON value with no property-list counterpart (`null` inside a list, for example).
    #[error("{0}")]
    NotAValue(String),
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    /// A function of this crate whose M7 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// Why a master or instance could not be addressed ([`crate::target`]).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TargetError {
    /// The reference is not an id, a name or an index (`{"id": …}`, `{"name": …}`, a number…).
    #[error("{0}")]
    BadReference(String),
    #[error("there is no {scope} {reference}")]
    NotFound { scope: &'static str, reference: String },
    /// A name that more than one master or instance has; `ids` lists theirs (instances without an
    /// id are listed by index, `#3`).
    #[error("more than one {scope} is called `{name}`: use one of the ids {}", .ids.join(", "))]
    Ambiguous { scope: &'static str, name: String, ids: Vec<String> },
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
}

/// Why a font could not be scaled to a new units-per-em ([`crate::scale`]).
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ScaleError {
    #[error("units per em must be from {min} to {max}", min = crate::UPM_RANGE.start(), max = crate::UPM_RANGE.end())]
    UnitsPerEm(u32),
    /// A value would pass [`crate::MAX_SCALED_COORDINATE`] once scaled; `what` says where.
    #[error("scaling would move {what} beyond ±32768 units")]
    OutOfRange { what: String },
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// Why a glyph-set line, a Rename Glyphs line or a Reencode Glyphs line could not be read
/// ([`crate::glyphset`]). `line` counts from 1.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatternError {
    #[error("line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("line {line}: a pattern holds at most {limit} `*`")]
    TooManyWildcards { line: usize, limit: usize },
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// Why Font Info rows could not be read from clipboard text ([`crate::rows`]).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RowsError {
    /// The text is neither the JSON nor the property-list form of rows.
    #[error("the clipboard does not hold Font Info rows: {0}")]
    NotRows(String),
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    #[error(transparent)]
    Value(#[from] ValueError),
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}
