//! Running one operation with a time limit (docs/plans/M3.md › M3.6, rule 8).
//!
//! The boolean engine can loop forever on rare inputs. Callers where no person waits for one
//! glyph (batch commands, export) run each glyph's operation through [`run_with_time_limit`]: on
//! native targets it runs on a worker thread, and a glyph that takes longer than the limit is
//! reported and left as it was. The browser has no threads, so there the operation runs inline
//! without a limit.

use std::any::Any;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

/// How long one layer's operation may run before it is abandoned. The slowest layer of the
/// corpus takes under a millisecond; the boolean engine's rare endless loops never finish.
pub const TIME_LIMIT: Duration = Duration::from_secs(5);

/// Why an operation run with a time limit gave no result.
#[derive(Clone, Debug, PartialEq)]
pub enum Unfinished {
    TimedOut(Duration),
    Panicked(String),
    NoWorker(String),
}

impl fmt::Display for Unfinished {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut(limit) => write!(f, "the operation took longer than {} s and was abandoned", limit.as_secs_f64()),
            Self::Panicked(m) => write!(f, "the operation failed with an internal error: {m}"),
            Self::NoWorker(e) => write!(f, "no worker thread could be started: {e}"),
        }
    }
}

impl std::error::Error for Unfinished {}

/// The text of a panic payload (`panic!("…")` carries a `&str` or a `String`).
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// `f`, with a panic caught and turned into its message.
pub fn catch_panic<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| panic_message(payload.as_ref()))
}

/// Runs `f` with panics caught on a worker thread and waits at most `limit` for it. A worker
/// that does not finish in time is abandoned: it cannot be stopped, so it runs on detached and
/// its result is dropped. Callers hand it owned data, so nothing they keep changes after a
/// timeout.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_with_time_limit<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> Result<T, Unfinished> {
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    let (tx, rx) = sync_channel(1);
    let _worker = std::thread::Builder::new()
        .name("fontcraft-pathops".into())
        .spawn(move || {
            let _ = tx.send(catch_panic(f));
        })
        .map_err(|e| Unfinished::NoWorker(e.to_string()))?;
    match rx.recv_timeout(limit) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(message)) => Err(Unfinished::Panicked(message)),
        Err(RecvTimeoutError::Timeout) => Err(Unfinished::TimedOut(limit)),
        Err(RecvTimeoutError::Disconnected) => Err(Unfinished::Panicked("the worker stopped without a result".into())),
    }
}

/// The browser has no threads: `f` runs here, with panics caught, without a limit.
#[cfg(target_arch = "wasm32")]
pub fn run_with_time_limit<T: Send + 'static>(_limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> Result<T, Unfinished> {
    catch_panic(f).map_err(Unfinished::Panicked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_passes_results_reports_panics_and_gives_up_in_time() {
        assert_eq!(run_with_time_limit(TIME_LIMIT, || 7), Ok(7));
        let caught = run_with_time_limit(TIME_LIMIT, || -> u8 { panic!("boom") });
        assert_eq!(caught, Err(Unfinished::Panicked("boom".into())));
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let blocked = run_with_time_limit(Duration::from_millis(20), move || rx.recv().is_ok());
        drop(tx);
        assert_eq!(blocked, Err(Unfinished::TimedOut(Duration::from_millis(20))));
    }
}
