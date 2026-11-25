//! The files the work on this thread has put in place, for an error that has to say which of them stay.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// Most files kept; more are only counted.
const MAX_KEPT: usize = 1_000;

/// Files put in place, in order: the first [`MAX_KEPT`] of them and how many there were.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub files: Vec<PathBuf>,
    pub count: usize,
}

impl Written {
    fn add(&mut self, path: &Path) {
        if self.files.iter().any(|f| f == path) {
            return;
        }
        self.count = self.count.saturating_add(1);
        if self.files.len() < MAX_KEPT {
            self.files.push(path.to_path_buf());
        }
    }
}

thread_local! {
    static WRITTEN: RefCell<Option<Written>> = const { RefCell::new(None) };
}

/// Notes that `path` was put in place, while something collects ([`collect`]).
pub fn record(path: &Path) {
    WRITTEN.with(|written| {
        if let Ok(mut written) = written.try_borrow_mut()
            && let Some(written) = written.as_mut()
        {
            written.add(path);
        }
    });
}

/// Puts back what collected before.
struct Restore(Option<Written>);

impl Drop for Restore {
    fn drop(&mut self) {
        let before = self.0.take();
        WRITTEN.with(|written| {
            if let Ok(mut written) = written.try_borrow_mut() {
                *written = before;
            }
        });
    }
}

/// Runs `f` and gives the files it put in place; whatever collects around it gets them too.
pub fn collect<T>(f: impl FnOnce() -> T) -> (T, Written) {
    let restore = Restore(WRITTEN.with(|written| written.replace(Some(Written::default()))));
    let out = f();
    let written = WRITTEN.with(|written| written.take()).unwrap_or_default();
    drop(restore);
    written.files.iter().for_each(|path| record(path));
    (out, written)
}
