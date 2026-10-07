//! The last line of defence: a bug that panics must not take the user's work with it.
//!
//! Shipped code does not panic (AGENTS.md › Never crash); fallible work returns `Result`. If a bug
//! panics anyway, every entry point (command dispatch here, the MCP server, the control channel,
//! the UI frame) runs under [`catch_panic`], puts its state back and reports an internal error.
//!
//! On wasm a panic aborts, so this net only exists in native builds. The rule is what protects
//! the web app.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f`; a panic becomes `Err(message)`. The caller must leave its state consistent after an
/// `Err` (restore a snapshot, or rebuild what `f` was working on).
pub fn catch_panic<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        let message = panic_message(payload.as_ref());
        log::error!("recovered from a panic: {message}");
        message
    })
}

/// The text of a panic payload (`panic!("…")` carries a `&str` or a `String`).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_an_error_message() {
        assert_eq!(catch_panic(|| 7), Ok(7));
        assert_eq!(catch_panic(|| -> i32 { panic!("boom {}", 1) }), Err("boom 1".to_string()));
        assert_eq!(catch_panic(|| -> i32 { std::panic::panic_any(5_u8) }), Err("unknown panic".to_string()));
    }
}
