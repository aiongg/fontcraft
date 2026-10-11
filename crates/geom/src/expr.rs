//! Arithmetic expressions over named values (M13.16 fills the parser and evaluator). Users:
//! anchor position expressions and automatic layers (M13.16), construction rules (M13.17), the
//! Space Center's matrix entry (M13.23), the non-linear transform (M13.29).
//!
//! Grammar: numbers (`12`, `0.5`, `1e3`), names (`width`, `xHeight`, `top`, `n.width`: letters,
//! digits, `_` and `.`, not starting with a digit or `.`), unary `+` and `-`, binary `+ - * /`
//! with the usual precedence, parentheses, and calls of the fixed functions `min`, `max`, `abs`,
//! `round`, `floor`, `ceil`, `sqrt`, `sin`, `cos`, `tan` (radians), `atan2`. What a name means is
//! the caller's: [`Expr::eval`] asks a lookup for each one.
//!
//! Limits: source at most [`MAX_EXPR_BYTES`], nesting at most [`MAX_EXPR_DEPTH`], evaluation at
//! most [`MAX_EXPR_STEPS`] steps, and every intermediate and final value finite (division by zero
//! is an error, not infinity).

use std::fmt;

/// Longest expression source, in bytes.
pub const MAX_EXPR_BYTES: usize = 1_024;
/// Deepest nesting of parentheses, calls and unary operators.
pub const MAX_EXPR_DEPTH: usize = 64;
/// Most evaluation steps (one per operation, name or number).
pub const MAX_EXPR_STEPS: usize = 10_000;

/// Why an expression cannot be parsed or evaluated.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprError {
    /// The source is longer than [`MAX_EXPR_BYTES`].
    TooLong(usize),
    /// A syntax error at a byte offset.
    Syntax { at: usize, message: String },
    /// Nested deeper than [`MAX_EXPR_DEPTH`].
    TooDeep,
    /// Evaluation took more than [`MAX_EXPR_STEPS`] steps.
    TooManySteps,
    /// The lookup does not know a name.
    UnknownName(String),
    /// A function the grammar does not have, or a wrong argument count.
    BadCall(String),
    /// Division by zero or a result that is not finite.
    NotFinite,
    /// What the skeleton has not built yet.
    Unsupported,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::TooLong(n) => write!(f, "the expression is {n} bytes; at most {MAX_EXPR_BYTES}"),
            ExprError::Syntax { at, message } => write!(f, "at character {at}: {message}"),
            ExprError::TooDeep => write!(f, "the expression is nested more than {MAX_EXPR_DEPTH} deep"),
            ExprError::TooManySteps => write!(f, "the expression takes more than {MAX_EXPR_STEPS} steps"),
            ExprError::UnknownName(name) => write!(f, "unknown name `{name}`"),
            ExprError::BadCall(why) => write!(f, "{why}"),
            ExprError::NotFinite => write!(f, "the result is not a finite number"),
            ExprError::Unsupported => write!(f, "expressions are not supported yet"),
        }
    }
}

impl std::error::Error for ExprError {}

/// A parsed expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    source: String,
    // M13.16: the parsed tree.
}

impl Expr {
    /// Parses `text`, checking length and nesting.
    pub fn parse(text: &str) -> Result<Expr, ExprError> {
        if text.len() > MAX_EXPR_BYTES {
            return Err(ExprError::TooLong(text.len()));
        }
        // M13.16.
        Err(ExprError::Unsupported)
    }

    /// The source text as given.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every name the expression uses, each once, in order of first use.
    pub fn names(&self) -> Vec<String> {
        // M13.16.
        Vec::new()
    }

    /// The value, asking `lookup` for each name (`None`: [`ExprError::UnknownName`]).
    pub fn eval(&self, lookup: impl Fn(&str) -> Option<f64>) -> Result<f64, ExprError> {
        let _ = lookup;
        // M13.16.
        Err(ExprError::Unsupported)
    }
}
