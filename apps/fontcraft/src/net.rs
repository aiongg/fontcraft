//! Network access for the Extension Manager: HTTPS downloads of extension indexes, entry files
//! and screenshots, and opening links in the system browser (`docs/plans/M13.md` § 7
//! question 5: local files and HTTPS indexes, fetched by the desktop app only).
//!
//! Downloads run the system's `curl` (shipped with macOS, Windows 10 and later, and nearly every
//! Linux system), so the app links no TLS stack of its own. Only `https://` is allowed, for the
//! request and for every redirect. The configuration, user name and password included, goes to
//! curl on its standard input, so the credentials of an index URL never appear on a command
//! line. Each download has a size cap, a connection timeout and an overall timeout.

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};

/// Seconds allowed to connect.
const CONNECT_TIMEOUT_S: u32 = 15;
/// Seconds allowed for a whole download.
const TOTAL_TIMEOUT_S: u32 = 120;
/// Most redirects followed.
const MAX_REDIRECTS: u32 = 5;
/// Longest error text kept from curl.
const MAX_ERROR_CHARS: usize = 500;

/// Downloads `url` (an `https://` URL, which may carry `user:password@`) and returns its body,
/// refusing one over `max` bytes. `language` is sent as `Accept-Language`.
pub fn fetch(url: &str, max: usize, language: &str) -> Result<Vec<u8>, String> {
    let shown = fontcraft_engine::extension_index::redact(url);
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("{shown}: only https:// is fetched"))?;
    let (credentials, address) = split_credentials(rest);
    let mut config = String::new();
    config.push_str(&format!("url = {}\n", quote(&format!("https://{address}"))));
    if let Some(user) = credentials {
        config.push_str(&format!("user = {}\n", quote(&percent_decode(user))));
    }
    config.push_str(&format!("header = {}\n", quote(&format!("Accept-Language: {language}"))));
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--fail", "--location", "--proto", "=https", "--proto-redir", "=https"])
        .args(["--max-redirs", &MAX_REDIRECTS.to_string(), "--connect-timeout", &CONNECT_TIMEOUT_S.to_string()])
        .args(["--max-time", &TOTAL_TIMEOUT_S.to_string(), "--max-filesize", &max.to_string(), "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{shown}: cannot start curl to download it: {e}"))?;
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(config.as_bytes())
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{shown}: {e}"));
    }
    let mut body = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
        stdout.take(limit).read_to_end(&mut body).map_err(|e| format!("{shown}: {e}"))?;
    }
    if body.len() > max {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{shown}: larger than {max} bytes"));
    }
    let mut errors = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut errors);
    }
    let status = child.wait().map_err(|e| format!("{shown}: {e}"))?;
    if !status.success() {
        let message: String = errors.trim().chars().take(MAX_ERROR_CHARS).collect();
        // curl may repeat the URL it was given; never show its credentials.
        let message = match credentials {
            Some(user) => message.replace(user, "…"),
            None => message,
        };
        return Err(format!("{shown}: {}", if message.is_empty() { format!("the download failed ({status})") } else { message }));
    }
    Ok(body)
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

/// A curl configuration string: double quotes, with `\` and `"` escaped and no line breaks.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(2));
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
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
    fn credentials_are_split_decoded_and_quoted_for_curl() {
        assert_eq!(split_credentials("u:p%40ss@host/a@b.json"), (Some("u:p%40ss"), "host/a@b.json"));
        assert_eq!(split_credentials("host/a@b.json"), (None, "host/a@b.json"));
        assert_eq!(percent_decode("u:p%40ss%zz"), "u:p@ss%zz");
        assert_eq!(quote("a\"b\\c\nd"), "\"a\\\"b\\\\cd\"");
        assert!(fetch("http://example.com/index.json", 10, "en").unwrap_err().contains("https"));
        assert!(open_url("file:///etc/passwd").is_err());
    }
}
