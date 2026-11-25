//! Asking long work on this thread to stop.
//!
//! A host runs work in [`with`] and says by a function whether it is still wanted; loops that can run long ask
//! [`requested`] between their units and give up with an error of their own. Outside [`with`] nothing asks to stop.
//! It is kept per thread, so a worker thread is not asked: work on one is bounded by its time limit.

use std::cell::RefCell;
use std::rc::Rc;

/// Says whether the work is to stop.
pub type Ask = Rc<dyn Fn() -> bool>;

thread_local! {
    static ASK: RefCell<Option<Ask>> = const { RefCell::new(None) };
}

/// Puts back the question that was asked before.
struct Restore(Option<Ask>);

impl Drop for Restore {
    fn drop(&mut self) {
        let before = self.0.take();
        ASK.with(|ask| {
            if let Ok(mut ask) = ask.try_borrow_mut() {
                *ask = before;
            }
        });
    }
}

/// Runs `f` as work that `ask` stops, or that nothing does.
pub fn with<T>(ask: Option<Ask>, f: impl FnOnce() -> T) -> T {
    let _restore = Restore(ASK.with(|current| current.replace(ask)));
    f()
}

/// Whether the work is to stop.
pub fn requested() -> bool {
    let ask = ASK.with(|ask| ask.try_borrow().ok().and_then(|ask| ask.clone()));
    ask.is_some_and(|ask| ask())
}
