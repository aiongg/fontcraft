// Ported from skrifa 0.48.0 (fontations, MIT OR Apache-2.0), src/outline/glyf/hint/call_stack.rs; see NOTICE.
//! Tracking function call state.

use super::{HintErrorKind, definition::Definition, program::Program};

// FreeType provides a call stack with a depth of 32.
// See <https://gitlab.freedesktop.org/freetype/freetype/-/blob/57617782464411201ce7bbc93b086c1b4d7d84a5/src/truetype/ttinterp.c#L502>
const MAX_DEPTH: usize = 32;

/// Record of an active invocation of a function or instruction
/// definition.
///
/// See <https://gitlab.freedesktop.org/freetype/freetype/-/blob/57617782464411201ce7bbc93b086c1b4d7d84a5/src/truetype/ttinterp.h#L90>
#[derive(Copy, Clone, Default)]
pub struct CallRecord {
    pub caller_program: Program,
    pub return_pc: usize,
    pub current_count: u32,
    pub definition: Definition,
}

/// Tracker for nested active function or instruction calls.
#[derive(Default)]
pub struct CallStack {
    records: [CallRecord; MAX_DEPTH],
    len: usize,
}

impl CallStack {
    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, record: CallRecord) -> Result<(), HintErrorKind> {
        let top = self.records.get_mut(self.len).ok_or(HintErrorKind::CallStackOverflow)?;
        *top = record;
        self.len += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn peek(&self) -> Option<&CallRecord> {
        self.records.get(self.len.checked_sub(1)?)
    }

    pub fn pop(&mut self) -> Result<CallRecord, HintErrorKind> {
        let record = *self.peek().ok_or(HintErrorKind::CallStackUnderflow)?;
        self.len -= 1;
        Ok(record)
    }
}
