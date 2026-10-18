//! Network access for the Extension Manager: HTTPS downloads of extension indexes, entry files
//! and screenshots, and opening links in the system browser (`docs/plans/M13.md` § 7
//! question 5: local files and HTTPS indexes, fetched by the desktop app only).
//!
//! Downloads use a Rust HTTPS client (rustls). Only `https://` is allowed, for the request and
//! for every redirect. The user name and password of an index URL go in the `Authorization`
//! header, never in the address the client sees, so no error text or log line carries them; a
//! redirect keeps them only to the same host. Each download has a size cap, a connection timeout
//! and an overall timeout.

use std::process::Command;
use std::time::Duration;

/// Seconds allowed to connect.
const CONNECT_TIMEOUT_S: u64 = 15;
/// Seconds allowed for a whole download.
const TOTAL_TIMEOUT_S: u64 = 120;
/// Most redirects followed.
const MAX_REDIRECTS: u32 = 5;
/// Longest error text kept from the HTTPS client.
const MAX_ERROR_CHARS: usize = 500;

/// Downloads `url` (an `https://` URL, which may carry `user:password@`) and returns its body,
/// refusing one over `max` bytes. `language` is sent as `Accept-Language`.
pub fn fetch(url: &str, max: usize, language: &str) -> Result<Vec<u8>, String> {
    let shown = fontcraft_engine::extension_index::redact(url);
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("{shown}: only https:// is fetched"))?;
    let (credentials, address) = split_credentials(rest);
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .redirect_auth_headers(ureq::config::RedirectAuthHeaders::SameHost)
        .timeout_connect(Some(Duration::from_secs(CONNECT_TIMEOUT_S)))
        .timeout_global(Some(Duration::from_secs(TOTAL_TIMEOUT_S)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut request = agent.get(format!("https://{address}")).header("Accept-Language", language);
    if let Some(user) = credentials {
        request = request.header("Authorization", format!("Basic {}", base64(percent_decode(user).as_bytes())));
    }
    let failed = |e: ureq::Error| format!("{shown}: {}", e.to_string().chars().take(MAX_ERROR_CHARS).collect::<String>());
    let mut response = request.call().map_err(failed)?;
    let limit = u64::try_from(max).unwrap_or(u64::MAX);
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| match e {
        ureq::Error::BodyExceedsLimit(_) => format!("{shown}: larger than {max} bytes"),
        e => failed(e),
    })
}

/// Opens an `https://` or `http://` link in the system browser.
pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(format!("{url:?} is not a web link"));
    }
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        // Not `cmd /c start`, which would read `&` in the URL as a command separator.
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let mut child = command.spawn().map_err(|e| format!("cannot start the browser: {e}"))?;
    // Wait on another thread, so the finished process does not linger.
    let _ = std::thread::spawn(move || child.wait());
    Ok(())
}

/// `user:password@host/path` → (`user:password`, `host/path`); the `@` must be in the authority.
fn split_credentials(rest: &str) -> (Option<&str>, &str) {
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest.get(..authority_end).and_then(|a| a.rfind('@')) {
        Some(at) => (rest.get(..at), rest.get(at.saturating_add(1)..).unwrap_or_default()),
        None => (None, rest),
    }
}

/// Standard base64 with padding (RFC 4648), for the Basic `Authorization` header.
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        for k in 0..4usize {
            let digit = usize::try_from((n >> (18 - 6 * k)) & 63).ok().and_then(|d| DIGITS.get(d));
            out.push(if k <= chunk.len() { digit.map_or('=', |d| char::from(*d)) } else { '=' });
        }
    }
    out
}

/// Decodes `%XX` escapes of a URL's user information; anything else stays as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i.saturating_add(1)..i.saturating_add(3))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i = i.saturating_add(3);
            }
            _ => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn credentials_are_split_decoded_and_encoded() {
        assert_eq!(split_credentials("u:p%40ss@host/a@b.json"), (Some("u:p%40ss"), "host/a@b.json"));
        assert_eq!(split_credentials("host/a@b.json"), (None, "host/a@b.json"));
        assert_eq!(percent_decode("u:p%40ss%zz"), "u:p@ss%zz");
        assert_eq!(base64(b"u:p@ss"), "dTpwQHNz");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
        assert!(fetch("http://example.com/index.json", 10, "en").unwrap_err().contains("https"));
        assert!(open_url("file:///etc/passwd").is_err());
    }
}
