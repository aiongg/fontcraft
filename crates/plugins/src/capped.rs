//! Lists in module output read under one shared element budget, so a module cannot make the host
//! build more items than its kind allows before the count is checked (the output's byte cap alone
//! would still let tens of millions of small items through).

use std::cell::Cell;
use std::marker::PhantomData;

use serde::de::{DeserializeSeed, Deserializer, Error as _, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

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
pub(crate) fn capped<'de, D: Deserializer<'de>, T: serde::Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    struct V<T>(PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> Visitor<'de> for V<T> {
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
                    return match seq.next_element::<IgnoredAny>()? {
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
pub(crate) fn capped_opt<'de, D: Deserializer<'de>, T: serde::Deserialize<'de>>(d: D) -> Result<Option<Vec<T>>, D::Error> {
    struct V<T>(PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> Visitor<'de> for V<T> {
        type Value = Option<Vec<T>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list or null")
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            capped(d).map(Some)
        }
    }
    d.deserialize_option(V(PhantomData))
}

/// Why [`charge_values`] stopped before the end of a value.
#[derive(Debug)]
pub(crate) enum Uncounted {
    /// `left` ran out.
    TooMany,
    /// The value nests deeper than the JSON reader's depth limit (128 levels).
    TooDeep(String),
}

/// Takes from `left` one for every JSON value in `raw`: the value itself and each list item and
/// object entry at any depth. Nothing is built.
pub(crate) fn charge_values(raw: &RawValue, left: &mut usize) -> Result<(), Uncounted> {
    let mut d = serde_json::Deserializer::from_str(raw.get());
    Counter(left).deserialize(&mut d).map_err(|e| match e.classify() {
        // The counter's own error; `raw` is already known to be valid JSON, so any other error is
        // the reader's depth limit.
        serde_json::error::Category::Data => Uncounted::TooMany,
        _ => Uncounted::TooDeep(e.to_string()),
    })
}

struct Counter<'b>(&'b mut usize);

impl Counter<'_> {
    fn charge<E: serde::de::Error>(self) -> Result<(), E> {
        *self.0 = self.0.checked_sub(1).ok_or_else(|| E::custom("too many values"))?;
        Ok(())
    }
}

impl<'de> DeserializeSeed<'de> for Counter<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Counter<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        self.charge()
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        self.charge()
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        self.charge()
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        self.charge()
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        self.charge()
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        self.charge()
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        self.charge()
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let left = self.0;
        Counter(&mut *left).charge::<A::Error>()?;
        while seq.next_element_seed(Counter(&mut *left))?.is_some() {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let left = self.0;
        Counter(&mut *left).charge::<A::Error>()?;
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value_seed(Counter(&mut *left))?;
        }
        Ok(())
    }
}
