//! What happened in the app, for agents: `events.list` and `events.clear` read a ring of the last [`EVENT_RING`]
//! entries. Commands are recorded as they run ([`FontCraftApp::apply`]); documents opened, saved and closed, and
//! dialogs opened and closed, are found by comparing the app with what [`observe`] saw last, so they are caught
//! whichever path changed them.

use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::{Method, object};
use crate::app::FontCraftApp;

pub const METHODS: &[(&str, Method)] = &[("events.list", list), ("events.clear", clear)];

/// Most entries the ring keeps; the oldest goes when a new one arrives.
pub const EVENT_RING: usize = 1_000;
/// Longest error text an entry keeps, in bytes; longer ones are cut at a character boundary.
const MAX_TEXT: usize = 2_000;

/// One entry: its sequence number (from 1, never reused), its kind and what it carries.
#[derive(Clone, Debug, PartialEq)]
struct Event {
    seq: u64,
    kind: &'static str,
    data: Map<String, Value>,
}

/// A document as [`observe`] last saw it: its id, title and path, and which saved state it holds.
#[derive(Clone, Debug, PartialEq)]
struct SeenDocument {
    id: u64,
    title: String,
    path: Option<String>,
    /// The address of the document's saved font: a save replaces it.
    saved: usize,
}

/// The ring and what [`observe`] saw last.
#[derive(Default)]
pub struct Events {
    ring: VecDeque<Event>,
    /// The sequence number of the last entry pushed; 0 before the first.
    last: u64,
    /// `None` until the first [`observe`], which records the start without entries.
    documents: Option<Vec<SeenDocument>>,
    dialog: Option<String>,
}

impl Events {
    fn push(&mut self, kind: &'static str, data: Value) {
        let Value::Object(data) = data else { return };
        self.last = self.last.saturating_add(1);
        if self.ring.len() >= EVENT_RING {
            self.ring.pop_front();
        }
        self.ring.push_back(Event { seq: self.last, kind, data });
    }

    /// A command ran: its id, whether it succeeded, the active document's revision after it, and its error.
    pub fn command(&mut self, id: &str, result: Result<(), &str>, revision: Option<u64>) {
        let mut data = json!({"command": id, "ok": result.is_ok(), "revision": revision});
        if let Err(error) = result {
            super::set(&mut data, "error", json!(clip(error)));
        }
        self.push("command", data);
    }

    /// Something failed that was not a command run (a tool's input, a view change, a file picker's answer).
    pub fn error(&mut self, message: &str) {
        self.push("error", json!({"message": clip(message)}));
    }

    /// The entries after `since`, oldest first, and whether entries after `since` were already dropped.
    fn after(&self, since: u64) -> (Vec<Value>, bool) {
        let first = self.ring.front().map_or(self.last.saturating_add(1), |e| e.seq);
        let dropped = since.saturating_add(1) < first && self.last > since;
        let events = self
            .ring
            .iter()
            .filter(|e| e.seq > since)
            .map(|e| {
                let mut map = Map::new();
                map.insert("seq".into(), json!(e.seq));
                map.insert("kind".into(), json!(e.kind));
                map.extend(e.data.clone());
                Value::Object(map)
            })
            .collect();
        (events, dropped)
    }
}

/// `text` cut to at most [`MAX_TEXT`] bytes.
fn clip(text: &str) -> &str {
    if text.len() <= MAX_TEXT {
        return text;
    }
    let mut end = MAX_TEXT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// Compares the open documents and the open dialog with what it saw last, and records what changed. The first call
/// only records the start.
pub fn observe(app: &mut FontCraftApp) {
    let documents: Vec<SeenDocument> = app
        .session
        .documents()
        .iter()
        .map(|d| SeenDocument {
            id: d.id,
            title: d.title.clone(),
            path: d.path.as_ref().map(|p| p.display().to_string()),
            saved: Arc::as_ptr(&d.saved) as usize,
        })
        .collect();
    let dialog = app.dialogs.open.as_ref().map(|d| d.id.clone());
    let events = &mut app.control.events;
    let Some(before) = events.documents.replace(documents.clone()) else {
        events.dialog = dialog;
        return;
    };
    let describe = |d: &SeenDocument, action: &str| json!({"action": action, "document": d.id, "title": d.title, "path": d.path});
    for gone in before.iter().filter(|b| !documents.iter().any(|d| d.id == b.id)) {
        events.push("document", describe(gone, "closed"));
    }
    for now in &documents {
        match before.iter().find(|b| b.id == now.id) {
            None => events.push("document", describe(now, "opened")),
            Some(was) if was.saved != now.saved || was.path != now.path => events.push("document", describe(now, "saved")),
            Some(_) => {}
        }
    }
    if events.dialog != dialog {
        if let Some(closed) = events.dialog.take() {
            events.push("dialog", json!({"action": "closed", "dialog": closed}));
        }
        if let Some(opened) = &dialog {
            events.push("dialog", json!({"action": "opened", "dialog": opened}));
        }
        events.dialog = dialog;
    }
}

/// `{since?}` → `{events: [{seq, kind, …}], last, dropped}`: the entries after sequence number `since` (default 0:
/// all the ring holds), oldest first. `last` is the newest sequence number, to pass as `since` next time; `dropped`
/// is true when entries after `since` have already left the ring.
fn list(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let params = object("events.list", params)?;
    let since = match params.get("since") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().ok_or("events.list: `since` is a sequence number (a whole number from 0)")?,
    };
    observe(app);
    let events = &app.control.events;
    let (list, dropped) = events.after(since);
    Ok(json!({"events": list, "last": events.last, "dropped": dropped}))
}

/// `{}` → `{cleared}`: empties the ring. Sequence numbers keep counting.
fn clear(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    let events = &mut app.control.events;
    let cleared = events.ring.len();
    events.ring.clear();
    Ok(json!({"cleared": cleared}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_newest_entries_and_says_when_some_were_dropped() {
        let mut events = Events::default();
        for i in 0..(EVENT_RING + 5) {
            events.error(&format!("e{i}"));
        }
        let (all, dropped) = events.after(0);
        assert_eq!(all.len(), EVENT_RING);
        assert!(dropped, "the first five left the ring");
        assert_eq!(all[0]["seq"], 6);
        let (recent, dropped) = events.after(EVENT_RING as u64);
        assert_eq!((recent.len(), dropped), (5, false));
        assert!(events.after(events.last).0.is_empty());
        assert_eq!(clip(&"é".repeat(MAX_TEXT)).len(), MAX_TEXT, "cut at a character boundary");
    }
}
