//! What a server keeps for a client between requests, and the resource subscriptions among it
//! (`resources/subscribe`, `resources/unsubscribe`, `notifications/resources/updated`).
//!
//! The document and the glyphs can be subscribed to. After each request, and while a running app is attached once a
//! second, the subscribed resources are read again when the open documents changed at all; one whose content differs
//! from what was read last is announced once, however many changes lie between. A subscription keeps a hash of the
//! content, not the content.

use std::collections::BTreeMap;
use std::hash::{Hash as _, Hasher as _};

use fontcraft_engine::file_access;
use fontcraft_engine::guard;
use serde_json::{Value, json};

use crate::resources::{self, Resource};
use crate::{INVALID_PARAMS, LogLevel, MAX_PENDING_NOTIFICATIONS, RESOURCE_NOT_FOUND, Server};

/// Most resources one client subscribes to.
pub const MAX_SUBSCRIPTIONS: usize = 64;

/// What the server keeps for one client.
#[derive(Default)]
pub(crate) struct Client {
    /// Least severe level sent as a notification; `None` until the client sets one.
    pub log_level: Option<LogLevel>,
    /// URI → the resource and a hash of its content when it was read last.
    pub subscriptions: BTreeMap<String, (Resource, u64)>,
    /// A hash of the open documents, with their revisions, when the subscriptions were checked last.
    documents: Option<u64>,
}

/// A hash of a read's outcome; an error is an outcome too (a deleted glyph).
fn fingerprint(outcome: &Result<Value, String>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match outcome {
        Ok(value) => value.to_string().hash(&mut hasher),
        Err(error) => error.hash(&mut hasher),
    }
    hasher.finish()
}

/// The `uri` of a request, checked.
fn uri_of(params: &Value) -> Result<(String, Resource), (i64, String)> {
    let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "`uri` (a resource URI) is required".to_string()))?;
    let resource = resources::parse_uri(uri).map_err(|e| (RESOURCE_NOT_FOUND, e))?;
    Ok((uri.to_string(), resource))
}

impl Server {
    pub(crate) fn subscribe(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let (uri, resource) = uri_of(params)?;
        if !matches!(resource, Resource::Document | Resource::Glyph(_)) {
            return Err((INVALID_PARAMS, format!("{uri} cannot be subscribed to: only the document and the glyphs can")));
        }
        if self.client.subscriptions.len() >= MAX_SUBSCRIPTIONS && !self.client.subscriptions.contains_key(&uri) {
            return Err((INVALID_PARAMS, format!("at most {MAX_SUBSCRIPTIONS} resources can be subscribed to at once")));
        }
        // It must be readable now, under the server's policy.
        let read = resources::read(&uri, &resource, self).map_err(|e| (RESOURCE_NOT_FOUND, e))?;
        self.client.subscriptions.insert(uri, (resource, fingerprint(&Ok(read))));
        Ok(json!({}))
    }

    /// Ends a subscription; one that is not there is ended already.
    pub(crate) fn unsubscribe(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let (uri, _) = uri_of(params)?;
        self.client.subscriptions.remove(&uri);
        Ok(json!({}))
    }

    /// Announces the subscribed resources that changed since they were read last.
    pub(crate) fn poll_updates(&mut self) {
        if self.client.subscriptions.is_empty() {
            return;
        }
        let roots = self.roots.clone();
        let polled = file_access::confine(roots.as_ref(), || guard::catch_panic(|| self.read_updates()));
        if let Err(panic) = polled {
            self.log(LogLevel::Error, json!({"method": "resources/subscribe", "internalError": panic}));
        }
    }

    fn read_updates(&mut self) {
        let documents = fingerprint(&self.backend.inspect());
        if self.client.documents.replace(documents) == Some(documents) {
            return;
        }
        let mut subscriptions = std::mem::take(&mut self.client.subscriptions);
        for (uri, (resource, seen)) in &mut subscriptions {
            let now = fingerprint(&resources::read(uri, resource, self));
            if std::mem::replace(seen, now) != now && self.pending.len() < MAX_PENDING_NOTIFICATIONS {
                self.pending.push(json!({"jsonrpc": "2.0", "method": "notifications/resources/updated", "params": {"uri": uri}}));
            }
        }
        self.client.subscriptions = subscriptions;
    }
}
