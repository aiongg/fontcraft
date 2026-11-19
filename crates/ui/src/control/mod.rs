//! The control channel's method table (`docs/control-protocol.md`).
//!
//! Each module lists its methods in a slice and adds one line to [`MODULES`]. A method reads the app directly; a
//! method that changes the engine posts a `Msg` and lets `drain` run it, as a view does. The listener that feeds
//! [`dispatch`] arrives with the control-channel port.

mod app;
mod pointer;

use serde_json::Value;

use crate::app::FontCraftApp;

/// Runs one method with its parameters and returns its result, or the error the client gets.
pub type Method = fn(&mut FontCraftApp, &Value) -> Result<Value, String>;

/// Every module's methods.
pub const MODULES: &[&[(&str, Method)]] = &[app::METHODS, pointer::METHODS];

/// Runs the method `name`.
pub fn dispatch(app: &mut FontCraftApp, name: &str, params: &Value) -> Result<Value, String> {
    let method = MODULES.iter().flat_map(|methods| methods.iter()).find(|(n, _)| *n == name).map(|(_, m)| *m);
    let method = method.ok_or_else(|| format!("unknown method `{name}`"))?;
    method(app, params)
}
