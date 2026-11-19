//! External Preview, the engine half: the setting that turns it on, the trusted devices, and the
//! devices waiting for an answer. The desktop app serves the page
//! (`apps/fontcraft/src/external_preview.rs`): each frame it writes the page's address into
//! [`SharingState`], adds the devices that asked, and reads the answers back. Headless nothing is
//! served, and these commands only change the lists.
//!
//! Rows: `settings.sharing`, `sharing.*`, `settings.resetTrustedDevices`.
//!
//! A device is its IP address as text, in the form [`device`] gives. Trusted devices are kept in
//! the user's settings ([`Reserved::TrustedDevices`]) until `settings.resetTrustedDevices`;
//! waiting and refused devices last while the preview runs.

use std::net::IpAddr;

use serde_json::{Value, json};

use super::*;
use crate::params::Params;
use crate::settings::{Reserved, UserSettings};

/// The setting that turns External Preview on (off by default).
pub const ENABLE_SETTING: &str = "sharing.enableExternalPreview";
/// Most trusted devices kept; trusting one more forgets the one trusted first.
pub const MAX_TRUSTED_DEVICES: usize = 64;
/// Most devices waiting for an answer at once; more are not listed until one is answered.
pub const MAX_PENDING_DEVICES: usize = 8;
/// Most refused devices remembered; refusing one more forgets the one refused first.
pub const MAX_REFUSED_DEVICES: usize = 64;
/// Longest device text read from parameters, in bytes (an IPv6 address with a zone fits).
const MAX_DEVICE_BYTES: usize = 64;

/// External Preview while the app runs it: what the server and the commands share.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SharingState {
    /// The page's address while the app serves it.
    pub address: Option<String>,
    /// Why the server could not start, while it cannot.
    pub problem: Option<String>,
    /// Devices that asked to see the preview and wait for an answer, oldest first.
    pub pending: Vec<String>,
    /// Devices refused while the preview runs, oldest first.
    pub refused: Vec<String>,
}

impl SharingState {
    /// Lists `device` as waiting for an answer unless it is trusted, refused, already waiting, or
    /// [`MAX_PENDING_DEVICES`] wait. Returns whether it was added.
    pub fn ask(&mut self, device_text: &str, trusted: &[String]) -> bool {
        let Some(d) = device(device_text) else { return false };
        if trusted.contains(&d) || self.refused.contains(&d) || self.pending.contains(&d) || self.pending.len() >= MAX_PENDING_DEVICES {
            return false;
        }
        self.pending.push(d);
        true
    }
}

/// Whether External Preview is on.
pub fn enabled(settings: &UserSettings) -> bool {
    settings.get_bool(ENABLE_SETTING).unwrap_or(false)
}

/// A device as the lists keep it: the IP address in its canonical text (an IPv4 address mapped
/// into IPv6 as plain IPv4). `None` when `text` is not an IP address.
pub fn device(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() > MAX_DEVICE_BYTES {
        return None;
    }
    text.parse::<IpAddr>().ok().map(|ip| ip.to_canonical().to_string())
}

/// The trusted devices, oldest first. Entries no command would have written (not an address,
/// repeated, past the cap) are left out.
pub fn trusted_devices(settings: &UserSettings) -> Vec<String> {
    let stored = settings.reserved::<Vec<Value>>(Reserved::TrustedDevices).ok().flatten().unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for d in stored.iter().filter_map(Value::as_str).filter_map(device) {
        if out.len() >= MAX_TRUSTED_DEVICES {
            break;
        }
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// Appends `item` and drops the oldest entries past `cap`.
fn push_capped(list: &mut Vec<String>, item: String, cap: usize) {
    list.retain(|x| *x != item);
    list.push(item);
    let extra = list.len().saturating_sub(cap);
    list.drain(..extra);
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "sharing.status",
            "External Preview Status",
            [],
            None,
            "{} External Preview's state: whether it is on, the page's address while the app serves it (null headless and while off), why it could not start, the devices waiting for an answer, the trusted devices and the devices refused while it runs. A device is an IP address → {enabled, address, problem, pending: [device], trusted: [device], refused: [device]}",
            always,
            status
        ),
        cmd!(
            "sharing.answerDevice",
            "Answer Device",
            [],
            None,
            "{device: IP address (one of the waiting devices), trust: bool} answers a device that asked to see External Preview: true trusts it until the trusted devices are reset (at most 64 are kept, the oldest go first), false refuses it while the preview runs. Trusting is refused while automation roots are in force → {device, trusted, trustedDevices: [device]}",
            always,
            answer
        ),
        cmd!(
            "settings.resetTrustedDevices",
            "Reset",
            [],
            None,
            "{} forgets every device trusted to see External Preview, so each must be confirmed on this computer again → {removed: number}",
            always,
            reset
        ),
    ]
}

fn status(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("sharing.status", v)?;
    let state = s.sharing();
    Ok(json!({
        "enabled": enabled(s.settings()),
        "address": state.address,
        "problem": state.problem,
        "pending": state.pending,
        "trusted": trusted_devices(s.settings()),
        "refused": state.refused,
    }))
}

fn answer(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("sharing.answerDevice", v)?;
    let given = p.str("device")?;
    let trust = match p.get("trust") {
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(p.bad("`trust` must be true or false")),
        None => return Err(p.bad("`trust` is required: true trusts the device, false refuses it")),
    };
    let shown: String = given.chars().take(MAX_DEVICE_BYTES).collect();
    let d = device(given).ok_or_else(|| p.bad(format!("`device` must be an IP address, got \"{shown}\"")))?;
    if !s.sharing().pending.contains(&d) {
        let waiting = s.sharing().pending.join(", ");
        let waiting = if waiting.is_empty() { "none".to_string() } else { waiting };
        return Err(p.bad(format!("{d} is not waiting for an answer (waiting: {waiting})")));
    }
    let mut trusted = trusted_devices(s.settings());
    if trust {
        // A confined agent may not open the font to the network; the person confirms in the app.
        if crate::file_access::current().is_some() {
            return Err(p.bad("devices cannot be trusted while automation roots are in force: confirm them in the app"));
        }
        push_capped(&mut trusted, d.clone(), MAX_TRUSTED_DEVICES);
        s.settings_mut().set_reserved(Reserved::TrustedDevices, &trusted).map_err(|e| p.bad(e.to_string()))?;
    } else {
        push_capped(&mut s.sharing_mut().refused, d.clone(), MAX_REFUSED_DEVICES);
    }
    s.sharing_mut().pending.retain(|x| *x != d);
    Ok(json!({ "device": d, "trusted": trust, "trustedDevices": trusted }))
}

fn reset(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("settings.resetTrustedDevices", v)?;
    let removed = trusted_devices(s.settings()).len();
    s.settings_mut().clear_reserved(Reserved::TrustedDevices);
    Ok(json!({ "removed": removed }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn waiting(s: &mut Session, devices: &[&str]) {
        let trusted = trusted_devices(s.settings());
        for d in devices {
            assert!(s.sharing_mut().ask(d, &trusted), "{d} is listed");
        }
    }

    #[test]
    fn external_preview_answers_move_devices_and_reset_forgets_the_trusted() {
        let mut s = Session::new();
        assert!(!enabled(s.settings()), "off by default");
        waiting(&mut s, &["192.168.1.20", "::ffff:192.168.1.21"]);
        assert_eq!(s.sharing().pending, ["192.168.1.20", "192.168.1.21"], "a mapped IPv4 address is kept as IPv4");

        let r = s.execute("sharing.answerDevice", &json!({"device": "192.168.1.20", "trust": true})).unwrap();
        assert_eq!(r["trustedDevices"], json!(["192.168.1.20"]));
        let r = s.execute("sharing.answerDevice", &json!({"device": "192.168.1.21", "trust": false})).unwrap();
        assert_eq!(r["trusted"], false);
        let st = s.query("sharing.status", &json!({})).unwrap();
        assert_eq!(st["pending"], json!([]));
        assert_eq!(st["trusted"], json!(["192.168.1.20"]));
        assert_eq!(st["refused"], json!(["192.168.1.21"]));
        assert_eq!(st["address"], Value::Null, "nothing serves headless");

        // Trusted and refused devices are not asked about again.
        let trusted = trusted_devices(s.settings());
        assert!(!s.sharing_mut().ask("192.168.1.20", &trusted));
        assert!(!s.sharing_mut().ask("192.168.1.21", &trusted));

        // The trusted list is a setting: it survives a save and a load.
        let (loaded, problems) = crate::settings::UserSettings::from_json(&s.settings().to_json()).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(trusted_devices(&loaded), ["192.168.1.20"]);

        assert_eq!(s.execute("settings.resetTrustedDevices", &json!({})).unwrap()["removed"], 1);
        assert!(trusted_devices(s.settings()).is_empty());
        assert!(s.sharing_mut().ask("192.168.1.20", &[]), "a reset device must be confirmed again");
    }

    #[test]
    fn external_preview_answer_refuses_what_is_not_waiting_and_changes_nothing() {
        let mut s = Session::new();
        waiting(&mut s, &["10.0.0.5"]);
        let before = (s.settings().clone(), s.sharing().clone());
        for bad in [
            json!({"device": "10.0.0.6", "trust": true}),
            json!({"device": "not an address", "trust": true}),
            json!({"device": "10.0.0.5"}),
            json!({"device": "10.0.0.5", "trust": "yes"}),
            json!({"device": "x".repeat(10_000), "trust": false}),
        ] {
            assert!(s.execute("sharing.answerDevice", &bad).is_err(), "{bad}");
            assert_eq!((s.settings().clone(), s.sharing().clone()), before, "{bad}");
        }
    }

    #[test]
    fn external_preview_lists_are_capped() {
        let mut s = Session::new();
        for i in 0..20 {
            s.sharing_mut().ask(&format!("10.0.1.{i}"), &[]);
        }
        assert_eq!(s.sharing().pending.len(), MAX_PENDING_DEVICES);
        for i in 0..=MAX_TRUSTED_DEVICES {
            let d = format!("10.1.{}.{}", i / 200, i % 200);
            s.sharing_mut().pending.clear();
            assert!(s.sharing_mut().ask(&d, &[]));
            s.execute("sharing.answerDevice", &json!({"device": d, "trust": true})).unwrap();
        }
        let trusted = trusted_devices(s.settings());
        assert_eq!(trusted.len(), MAX_TRUSTED_DEVICES);
        assert_eq!(trusted.first().map(String::as_str), Some("10.1.0.1"), "the device trusted first is forgotten");

        // A hand-edited settings file with junk in the list reads as the addresses it holds.
        s.settings_mut().set_reserved(Reserved::TrustedDevices, &json!(["10.9.9.9", 7, "junk", "10.9.9.9", {"a": 1}])).unwrap();
        assert_eq!(trusted_devices(s.settings()), ["10.9.9.9"]);
        s.settings_mut().set_reserved(Reserved::TrustedDevices, &json!({"not": "a list"})).unwrap();
        assert!(trusted_devices(s.settings()).is_empty());
    }
}
