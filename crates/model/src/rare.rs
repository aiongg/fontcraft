//! Data most objects of a kind do not have, behind one pointer.
//!
//! A [`Rare`] holds a value whose default means "nothing set". It stores nothing for the default,
//! so an object that has none of the data costs one pointer, and it reads as the default. Writing
//! through it allocates the value; a value set back to its default compares equal to an empty
//! one, so equality never depends on whether the box exists.

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A type whose default can be handed out by reference for the life of the program.
pub trait RareDefault: Default + PartialEq + 'static {
    /// The default value, shared.
    fn default_ref() -> &'static Self;
}

/// A value of `T`, held only when it is not `T`'s default.
#[derive(Clone, Debug, Default)]
pub struct Rare<T>(Option<Box<T>>);

impl<T: RareDefault> Rare<T> {
    /// Holds `value`, or nothing when it is the default.
    pub fn new(value: T) -> Self {
        if value == T::default() { Self(None) } else { Self(Some(Box::new(value))) }
    }

    /// Whether the value is the default.
    pub fn is_default(&self) -> bool {
        self.0.as_deref().is_none_or(|v| *v == T::default())
    }

    /// Drops the box when it holds the default.
    pub fn tidy(&mut self) {
        if self.0.as_deref().is_some_and(|v| *v == T::default()) {
            self.0 = None;
        }
    }
}

impl<T: RareDefault> Deref for Rare<T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self.0.as_deref() {
            Some(v) => v,
            None => T::default_ref(),
        }
    }
}

impl<T: RareDefault> DerefMut for Rare<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.get_or_insert_with(Default::default)
    }
}

impl<T: RareDefault> PartialEq for Rare<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: RareDefault + Serialize> Serialize for Rare<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(s)
    }
}

impl<'de, T: RareDefault + Deserialize<'de>> Deserialize<'de> for Rare<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Rare::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Note {
        text: String,
    }

    impl RareDefault for Note {
        fn default_ref() -> &'static Self {
            static D: LazyLock<Note> = LazyLock::new(Note::default);
            &D
        }
    }

    #[test]
    fn the_default_is_not_stored_and_equality_ignores_the_box() {
        let mut a: Rare<Note> = Rare::default();
        assert!(a.is_default());
        assert_eq!(a.text, "");
        a.text = "x".into();
        assert!(!a.is_default());
        a.text.clear();
        assert_eq!(a, Rare::default(), "a value set back to its default equals none");
        a.tidy();
        assert!(a.0.is_none());
        assert!(Rare::new(Note::default()).0.is_none());
        let json = serde_json::to_string(&Rare::new(Note { text: "y".into() })).unwrap();
        assert_eq!(serde_json::from_str::<Rare<Note>>(&json).unwrap().text, "y");
        assert!(serde_json::from_str::<Rare<Note>>("{\"text\":\"\"}").unwrap().0.is_none());
    }
}
