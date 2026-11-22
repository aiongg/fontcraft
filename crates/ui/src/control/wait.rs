//! `ui.wait {frames?, idle?, timeoutMs?}`: the reply waits until `frames` frames have been drawn, or until the window
//! is idle, or both; at the timeout it goes out anyway with `timedOut: true`. The UI thread never blocks: the
//! listener parks the reply and looks at it at the end of every pass (`server::after_ui`).
// The web build has no listener: it parses and refuses waits, and the rest goes unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::time::Duration;

use serde_json::{Value, json};

use super::object;

/// Longest wait a request may ask for, in milliseconds.
pub const MAX_WAIT_MS: u64 = 30_000;
/// Most frames a request may wait for.
pub const MAX_WAIT_FRAMES: u64 = 3_600;
/// The wait when the request names none, in milliseconds.
const DEFAULT_WAIT_MS: u64 = 5_000;
/// Frames an idle wait lets pass at least: the frame that drains what the request before it fired, and the frame
/// that draws the result.
pub const IDLE_FRAMES: u64 = 2;

/// What a `ui.wait` request waits for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wait {
    /// Frames to pass first.
    pub frames: u64,
    /// Whether the window must also be idle: no window input queued or running, no screenshot or other reply waiting
    /// on frames.
    pub idle: bool,
    pub timeout: Duration,
}

impl Wait {
    /// The request's wait. Without `frames` it waits for idle; with `frames` only, for the frames.
    pub fn parse(params: &Value) -> Result<Wait, String> {
        let params = object("ui.wait", params)?;
        let number = |key: &str, max: u64| -> Result<Option<u64>, String> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => match v.as_u64() {
                    Some(n) if n <= max => Ok(Some(n)),
                    _ => Err(format!("ui.wait: `{key}` is a whole number from 0 to {max}")),
                },
            }
        };
        let frames = number("frames", MAX_WAIT_FRAMES)?;
        let timeout = number("timeoutMs", MAX_WAIT_MS)?.unwrap_or(DEFAULT_WAIT_MS);
        let idle = match params.get("idle") {
            None | Some(Value::Null) => frames.is_none(),
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("ui.wait: `idle` is true or false".into()),
        };
        Ok(Wait { frames: frames.unwrap_or(0), idle, timeout: Duration::from_millis(timeout) })
    }

    /// Whether the wait is over after `frames` frames, with the window `idle` or not.
    pub fn settled(&self, frames: u64, idle: bool) -> bool {
        frames >= self.frames && (!self.idle || (idle && frames >= IDLE_FRAMES))
    }
}

/// The reply: `{frames, idle, timedOut, elapsedMs}`.
pub fn result(frames: u64, idle: bool, timed_out: bool, elapsed: Duration) -> Value {
    let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    json!({"frames": frames, "idle": idle, "timedOut": timed_out, "elapsedMs": ms})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_is_for_idle_unless_it_names_frames_and_its_limits_are_kept() {
        let idle = Wait::parse(&json!({})).unwrap();
        assert!(idle.idle && idle.frames == 0);
        assert!(!idle.settled(1, true) && idle.settled(IDLE_FRAMES, true) && !idle.settled(10, false));
        let frames = Wait::parse(&json!({"frames": 3})).unwrap();
        assert!(!frames.idle && !frames.settled(2, true) && frames.settled(3, false));
        assert!(Wait::parse(&json!({"frames": 3, "idle": true})).unwrap().idle);
        assert_eq!(Wait::parse(&json!({"timeoutMs": 30_000})).unwrap().timeout, Duration::from_secs(30));
        for bad in [
            json!({"timeoutMs": 30_001}),
            json!({"timeoutMs": -1}),
            json!({"frames": 1.5}),
            json!({"frames": MAX_WAIT_FRAMES + 1}),
            json!({"idle": "yes"}),
            json!([1]),
        ] {
            assert!(Wait::parse(&bad).is_err(), "{bad}");
        }
    }
}
