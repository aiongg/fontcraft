//! TTFAutohint control instructions: per-glyph corrections in ttfautohint's control syntax, as
//! the handbook lists it (`left`, `right`, `nodir`, `touch`, `point`, one-letter abbreviations).
//!
//! `left`, `right` and `nodir` feed our autohinter's analysis; `touch` and `point` become
//! Delta-like corrections. Text over [`MAX_CONTROL_TEXT`] bytes or [`MAX_CONTROL_LINES`] lines is
//! refused. Rows: `hinting.tt.param.ttfAutohintControlInstructions`,
//! `autohint.tt.controlInstructions.syntax`. M10.21 fills the body.

use crate::{HintingError, MAX_CONTROL_LINES, MAX_CONTROL_TEXT};

/// What one line asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    /// Points belong to a segment's left (or right) edge.
    Left,
    Right,
    /// Points are weak (no direction).
    NoDir,
    /// Shift the points at the given sizes, after hinting.
    Touch {
        x_shift: f64,
        y_shift: f64,
    },
    /// Shift the points at the given sizes, before IUP.
    Point {
        x_shift: f64,
        y_shift: f64,
    },
}

/// One control instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlLine {
    /// 1-based line in the text, for messages.
    pub line: usize,
    pub font_index: u32,
    pub glyph: String,
    pub kind: ControlKind,
    pub points: Vec<u32>,
    pub ppems: Vec<u16>,
}

/// Parsed control instructions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlInstructions {
    pub lines: Vec<ControlLine>,
}

/// Parses control-instruction text.
pub fn parse(text: &str) -> Result<ControlInstructions, HintingError> {
    if text.len() > MAX_CONTROL_TEXT {
        return Err(HintingError::LimitExceeded { what: "bytes of control instructions", limit: MAX_CONTROL_TEXT });
    }
    if text.lines().count() > MAX_CONTROL_LINES {
        return Err(HintingError::LimitExceeded { what: "lines of control instructions", limit: MAX_CONTROL_LINES });
    }
    // M10.21: the syntax.
    Ok(ControlInstructions::default())
}
