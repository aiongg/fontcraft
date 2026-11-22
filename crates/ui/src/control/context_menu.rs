//! `ui.contextMenu.list` and `ui.contextMenu.invoke`: the canvas context menu, read and clicked
//! (`app::context_menu`).

use fontcraft_geom::Point;
use serde_json::{Value, json};

use super::{Method, object};
use crate::app::FontCraftApp;
use crate::app::context_menu::{self as menus, Space};

pub const METHODS: &[(&str, Method)] = &[("ui.contextMenu.list", list), ("ui.contextMenu.invoke", invoke)];

/// `{x?, y?, space?}` → the open canvas context menu: `{open, glyph, screen, at, hit, sections: [{context, items:
/// [{command, label, enabled, disabled_reason?, checked?, alternate?, choices?: [{value, label, checked}]}]}]}`; `{open: false}` when none is. With `x` and
/// `y` it first opens the menu there, as a secondary click does (`space`: `font`, the default, or `screen`).
fn list(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.contextMenu.list";
    let p = object(M, params)?;
    let coordinate =
        |key: &str| p.get(key).filter(|v| !v.is_null()).map(|v| v.as_f64().filter(|n| n.is_finite()).ok_or(format!("{M}: `{key}` must be a number")));
    match (coordinate("x").transpose()?, coordinate("y").transpose()?) {
        (Some(x), Some(y)) => {
            let space = match p.get("space").and_then(Value::as_str) {
                None => Space::Font,
                Some(name) => Space::parse(name).ok_or(format!("{M}: `space` is \"font\" or \"screen\""))?,
            };
            menus::open_canvas(app, Point::new(x, y), space).map_err(|e| format!("{M}: {e}"))?;
        }
        (None, None) => {}
        _ => return Err(format!("{M}: give both `x` and `y`, or neither")),
    }
    let Some(menu) = &app.context_menus.canvas else { return Ok(json!({"open": false})) };
    let mut out = menu.to_json();
    super::set(&mut out, "open", json!(true));
    Ok(out)
}

/// `{command, value?}` → the item's result: runs an item of the open canvas menu as a click on it does, and closes the
/// menu. `value` picks one of a submenu item's `choices`.
fn invoke(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.contextMenu.invoke";
    let id = params.get("command").and_then(Value::as_str).ok_or(format!("{M}: `command` is required"))?.to_string();
    let value = match params.get("value") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v.clone()),
        Some(_) => return Err(format!("{M}: `value` must be a string")),
    };
    menus::invoke_choice(app, &id, value.as_deref(), |app, msg| app.run_now(msg, false)).map_err(|e| format!("{M}: {e}"))
}
