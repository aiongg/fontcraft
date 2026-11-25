//! Progress and cancellation of the work a request asks for.
//!
//! A host that can be asked to stop runs a request in [`with`], with a [`Token`] other threads can cancel and, when
//! it wants progress, a function that takes it. The command under way says how much there is to do ([`plan`]) and how
//! far it is ([`advance`]), and stops where it can ([`check`]). Outside [`with`] all of this does nothing.
//!
//! A cancelled command fails with [`EngineError::Cancelled`], and a command that fails changes nothing. A command
//! that ran to its end [`seal`]s the token before its change is kept, so a cancel either undoes the command whole or
//! comes too late. Work inside other work (a step of a batch, a filter of an export) can be cancelled too, but only
//! the outermost command reports progress and seals ([`enter`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::{EngineError, Result};

const RUNNING: u8 = 0;
const CANCELLED: u8 = 1;
const SEALED: u8 = 2;

/// Where a request stands: running, cancelled, or done so far that it can no longer be cancelled.
#[derive(Debug, Default)]
pub struct Token(AtomicU8);

impl Token {
    pub fn new() -> Arc<Token> {
        Arc::default()
    }

    /// Asks the request to stop. False when it is too late.
    pub fn cancel(&self) -> bool {
        match self.0.compare_exchange(RUNNING, CANCELLED, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => true,
            Err(state) => state == CANCELLED,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == CANCELLED
    }

    /// Marks the request as done; false when it was cancelled first.
    fn seal(&self) -> bool {
        match self.0.compare_exchange(RUNNING, SEALED, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => true,
            Err(state) => state == SEALED,
        }
    }
}

/// Takes the units of work done, those known of so far, and what was done last.
pub type Report = Rc<dyn Fn(u64, u64, &str)>;

struct Scope {
    token: Arc<Token>,
    report: Option<Report>,
    /// Commands under way, one inside the other.
    level: usize,
    done: u64,
    total: u64,
}

thread_local! {
    static CURRENT: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// Puts back the scope that was there before.
struct Restore(Option<Scope>);

impl Drop for Restore {
    fn drop(&mut self) {
        let before = self.0.take();
        CURRENT.with(|current| {
            if let Ok(mut current) = current.try_borrow_mut() {
                *current = before;
            }
        });
    }
}

fn swap<T>(scope: Option<Scope>, f: impl FnOnce() -> T) -> T {
    let _restore = Restore(CURRENT.with(|current| current.replace(scope)));
    f()
}

/// Runs `f` as the work `token` cancels, reporting its progress to `report`.
pub fn with<T>(token: Arc<Token>, report: Option<Report>, f: impl FnOnce() -> T) -> T {
    swap(Some(Scope { token, report, level: 0, done: 0, total: 0 }), f)
}

/// Runs `f` out of reach of a cancel, for work that must not stop half way.
pub fn shielded<T>(f: impl FnOnce() -> T) -> T {
    swap(None, f)
}

fn read<T>(f: impl FnOnce(&mut Scope) -> T) -> Option<T> {
    CURRENT.with(|current| current.try_borrow_mut().ok()?.as_mut().map(f))
}

/// A command under way, until it is dropped.
#[must_use]
pub struct Level(());

impl Drop for Level {
    fn drop(&mut self) {
        read(|scope| scope.level = scope.level.saturating_sub(1));
    }
}

/// Marks a command as under way.
pub fn enter() -> Level {
    read(|scope| scope.level = scope.level.saturating_add(1));
    Level(())
}

/// Whether the work was cancelled.
pub fn cancelled() -> bool {
    read(|scope| scope.token.is_cancelled()).unwrap_or(false)
}

/// Fails when the work was cancelled.
pub fn check() -> Result<()> {
    if cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
}

/// Fails when the work was cancelled; otherwise it can no longer be, when the outermost command asks.
pub fn seal() -> Result<()> {
    match read(|scope| if scope.level <= 1 { scope.token.seal() } else { !scope.token.is_cancelled() }) {
        Some(false) => Err(EngineError::Cancelled),
        _ => Ok(()),
    }
}

/// Says that the work was done even if it was cancelled meanwhile: what it did stands (a written file).
pub fn keep() {
    read(|scope| {
        if scope.level <= 1 {
            scope.token.0.store(SEALED, Ordering::Release);
        }
    });
}

/// Reports the progress of work done elsewhere (a running app), as its own count.
pub fn relay(done: u64, total: u64, message: &str) {
    let report = read(|scope| scope.report.clone().filter(|_| !scope.token.is_cancelled()));
    if let Some(report) = report.flatten() {
        report(done, total, message);
    }
}

/// Adds `units` of work still to do.
pub fn plan(units: usize) {
    read(|scope| {
        if scope.level <= 1 {
            scope.total = scope.total.saturating_add(units as u64);
        }
    });
}

/// Adds `units` of work done and reports the progress, unless the work was cancelled.
pub fn advance(units: usize, message: &str) {
    let due = read(|scope| {
        if scope.level > 1 || scope.token.is_cancelled() {
            return None;
        }
        scope.done = scope.done.saturating_add(units as u64);
        scope.report.clone().map(|report| (report, scope.done, scope.total.max(scope.done)))
    });
    // Called with the scope free, so the function may ask for it.
    if let Some((report, done, total)) = due.flatten() {
        report(done, total, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_is_whole_or_too_late_and_only_the_outermost_command_reports() {
        assert!(check().is_ok() && seal().is_ok() && !cancelled());
        let seen: Rc<RefCell<Vec<(u64, u64)>>> = Rc::default();
        let to = Rc::clone(&seen);
        let token = Token::new();
        with(Arc::clone(&token), Some(Rc::new(move |done: u64, total: u64, _: &str| to.borrow_mut().push((done, total)))), || {
            let _outer = enter();
            plan(3);
            advance(1, "");
            {
                let _inner = enter();
                plan(100);
                advance(50, "");
                assert!(seal().is_ok());
            }
            advance(1, "");
            assert!(token.cancel(), "an inner command seals nothing");
            assert!(matches!(check(), Err(EngineError::Cancelled)) && seal().is_err());
            shielded(|| assert!(check().is_ok()));
            assert!(cancelled());
        });
        assert_eq!(*seen.borrow(), [(1, 3), (2, 3)]);
        let token = Token::new();
        with(Arc::clone(&token), None, || {
            let _outer = enter();
            assert!(seal().is_ok());
            assert!(!token.cancel() && check().is_ok(), "too late");
        });
    }
}
