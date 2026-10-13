//! Lists in module output read under one shared element budget, so a module cannot make the host
//! build more items than its kind allows before the count is checked (the output's byte cap alone
//! would still let tens of millions of small items through).

use std::cell::Cell;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as _, SeqAccess, Visitor};

thread_local! {
    /// Items the parse running on this thread may still read; `None` outside [`with_budget`].
    static BUDGET: Cell<Option<usize>> = const { Cell::new(None) };
    /// Set when a [`capped`] list ran past the budget.
    static OVER: Cell<bool> = const { Cell::new(false) };
}

/// Runs `parse` with `budget` items for every [`capped`] list it reads, and restores the outer
/// budget afterwards. The flag tells whether a list ran past the budget (the parse then failed).
pub(crate) fn with_budget<T>(budget: usize, parse: impl FnOnce() -> T) -> (T, bool) {
    let outer = BUDGET.with(|b| b.replace(Some(budget)));
    let outer_over = OVER.with(|o| o.replace(false));
    let out = parse();
    BUDGET.with(|b| b.set(outer));
    (out, OVER.with(|o| o.replace(outer_over)))
}

/// A `deserialize_with` for `Vec<T>` that charges each element to the budget of [`with_budget`]
/// and fails with `what` when it runs out. Outside a budget it reads at most `usize::MAX` items.
pub(crate) fn capped<'de, D: serde::Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    struct V<T>(PhantomData<T>);
    impl<'de, T: DeserializeOwned> Visitor<'de> for V<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            loop {
                let left = BUDGET.with(Cell::get);
                if left == Some(0) {
                    // Only fail if another element follows.
                    return match seq.next_element::<serde::de::IgnoredAny>()? {
                        None => Ok(out),
                        Some(_) => {
                            OVER.with(|o| o.set(true));
                            Err(A::Error::custom("too many items"))
                        }
                    };
                }
                let Some(item) = seq.next_element::<T>()? else { return Ok(out) };
                BUDGET.with(|b| b.set(left.map(|n| n.saturating_sub(1))));
                out.push(item);
            }
        }
    }
    d.deserialize_seq(V(PhantomData))
}

/// [`capped`] for an optional list.
pub(crate) fn capped_opt<'de, D: serde::Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<Vec<T>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(transparent)]
    struct W<T: DeserializeOwned>(#[serde(deserialize_with = "capped")] Vec<T>);
    Option::<W<T>>::deserialize(d).map(|o| o.map(|w| w.0))
}
