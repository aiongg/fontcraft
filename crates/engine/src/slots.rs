//! Typed state that command areas keep on the session and on each document, without the core
//! knowing its type.
//!
//! - Session slots ([`Session::slot`], [`Session::slot_mut`]) hold one value per type for the
//!   whole session: caches and settings of an area. They are not part of a command's rollback
//!   snapshot, so an area keeps only what stays correct if the command fails afterwards (caches
//!   keyed by document id and revision: a rollback puts a document's revision back, and a later
//!   change gets a number never used before, so a key never names two fonts) or changes its
//!   slot only once it has succeeded.
//! - Document extensions ([`DocState::ext`], [`DocState::set_ext`]) hold one shared value per type
//!   for one document. They are cloned with the document, so a failing command rolls them back
//!   with it, and closing the document drops them.
//!
//! An area reaches its state through an extension trait on [`Session`] or [`DocState`]
//! (`fontcraft_engine::prelude`) rather than through these calls directly.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::{DocState, Session};

/// The session's slots, one per type.
pub(crate) struct SessionSlots(HashMap<TypeId, Box<dyn Any + Send + Sync>>);

/// A new session starts with the installed slots ([`crate::cmd::registry::SlotInit`]).
impl Default for SessionSlots {
    fn default() -> Self {
        crate::cmd::ensure_installed();
        let slots = crate::cmd::registry::slot_inits().iter().map(|init| {
            let value = init.create();
            ((*value).type_id(), value)
        });
        SessionSlots(slots.collect())
    }
}

/// A document's extensions, one per type. Few areas keep one, so a list is enough.
#[derive(Clone, Default)]
pub(crate) struct DocExts(Vec<(TypeId, Arc<dyn Any + Send + Sync>)>);

impl fmt::Debug for DocExts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DocExts({})", self.0.len())
    }
}

impl Session {
    /// The session's slot of type `T`, if something has created it.
    pub fn slot<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.slots.0.get(&TypeId::of::<T>()).and_then(|b| b.downcast_ref::<T>())
    }

    /// The session's slot of type `T`, created with its default value on first use.
    pub fn slot_mut<T: Any + Send + Sync + Default>(&mut self) -> &mut T {
        let slot = self.slots.0.entry(TypeId::of::<T>()).or_insert_with(|| Box::new(T::default()));
        if !slot.is::<T>() {
            *slot = Box::new(T::default());
        }
        // The map is keyed by the value's type, so the downcast cannot fail; the fallback only
        // keeps this function total.
        slot.downcast_mut::<T>().unwrap_or_else(|| Box::leak(Box::default()))
    }
}

impl DocState {
    /// The document's extension of type `T`, if it has one.
    pub fn ext<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.exts.0.iter().find(|(id, _)| *id == TypeId::of::<T>()).and_then(|(_, v)| v.downcast_ref::<T>())
    }

    /// [`DocState::ext`] as a shared pointer, to keep after the session changes.
    pub fn ext_arc<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let (_, v) = self.exts.0.iter().find(|(id, _)| *id == TypeId::of::<T>())?;
        v.clone().downcast::<T>().ok()
    }

    /// Sets the document's extension of type `T`, replacing the one before.
    pub fn set_ext<T: Any + Send + Sync>(&mut self, value: T) {
        let value: Arc<dyn Any + Send + Sync> = Arc::new(value);
        match self.exts.0.iter_mut().find(|(id, _)| *id == TypeId::of::<T>()) {
            Some((_, v)) => *v = value,
            None => self.exts.0.push((TypeId::of::<T>(), value)),
        }
    }

    /// Removes the document's extension of type `T`; returns it if there was one.
    pub fn remove_ext<T: Any + Send + Sync>(&mut self) -> Option<Arc<T>> {
        let i = self.exts.0.iter().position(|(id, _)| *id == TypeId::of::<T>())?;
        let (_, v) = self.exts.0.remove(i);
        v.downcast::<T>().ok()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Counter(u32);

    #[test]
    fn a_slot_is_created_on_first_use_and_kept() {
        let mut s = Session::new();
        assert!(s.slot::<Counter>().is_none());
        s.slot_mut::<Counter>().0 += 1;
        s.slot_mut::<Counter>().0 += 1;
        assert_eq!(s.slot::<Counter>().map(|c| c.0), Some(2));
    }

    #[test]
    fn a_document_extension_rolls_back_with_a_failing_command() {
        let mut s = Session::with_document();
        s.docs_mut().first_mut().unwrap().set_ext(Counter(1));
        assert_eq!(s.doc().unwrap().ext::<Counter>().map(|c| c.0), Some(1));
        // `audit.run` keeps its run as a document extension; the failing step after it undoes
        // the whole batch, the kept run included.
        let steps = json!({"steps": [
            {"command": "audit.run"},
            {"command": "spacing.setWidth", "params": {"glyphs": ["nope"], "value": 1}},
        ]});
        assert!(s.execute("command.batch", &steps).is_err());
        assert!(s.execute("audit.results", &json!({})).is_err());
        assert_eq!(s.doc().unwrap().ext::<Counter>().map(|c| c.0), Some(1));
        let kept = s.doc().unwrap().ext_arc::<Counter>();
        assert_eq!(s.docs_mut().first_mut().unwrap().remove_ext::<Counter>().map(|c| c.0), Some(1));
        assert!(s.doc().unwrap().ext::<Counter>().is_none());
        assert_eq!(kept.map(|c| c.0), Some(1));
    }
}
