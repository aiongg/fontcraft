//! Opening a web link in the browser and showing a file in the file manager, checked first, and an [`Opener`] an app
//! hands to the code that needs them.
//!
//! Meant for `craft::platform::open` (craft-platform, `src/open.rs`), beside [`reveal`](craft::platform::open::reveal)
//! and [`open_external`](craft::platform::open::open_external), which it builds on. Those two take any target and are
//! missing on wasm, so every caller checks the target and gates the call itself. This module does both once:
//!
//! - [`open_url`] opens only an `http://` or `https://` link with a host, without spaces or control characters.
//! - [`reveal`] shows only a path that exists.
//! - [`Opener`] holds the two calls as values. An app sets [`Opener::system`] on the desktop; tests set fakes with
//!   [`Opener::new`]; the web build and headless front ends keep [`Opener::NONE`], and the calls say they cannot.
//!   The checks run before the call, fake or not.
//!
//! On wasm both calls return an error: the browser build has no file manager, and opening a tab needs the
//! `platform-browser` feature, which this crate does not take.

use std::path::Path;

use craft::platform::PlatformText;

/// The longest link [`open_url`] opens, in bytes.
pub const MAX_URL_BYTES: usize = 8 << 10;

/// A link that is not an `http(s)` address.
pub const NOT_A_WEB_LINK: PlatformText = PlatformText { id: "open.notAWebLink", english: "{url} is not a web link" };
/// [`open_url`] or [`Opener::open_url`] where nothing can open a browser.
pub const NO_BROWSER: PlatformText = PlatformText { id: "open.noBrowser", english: "This build cannot open a web browser" };
/// [`reveal`] or [`Opener::reveal`] where nothing can show a file manager.
pub const NO_FILE_MANAGER: PlatformText = PlatformText { id: "open.noFileManager", english: "This build cannot show files in the file manager" };
/// [`reveal`] of a path that does not exist.
pub const NO_SUCH_FILE: PlatformText = PlatformText { id: "open.noSuchFile", english: "{path} does not exist" };
/// Every [`PlatformText`] of this module, for a missing-translation report.
pub const TEXTS: &[PlatformText] = &[NOT_A_WEB_LINK, NO_BROWSER, NO_FILE_MANAGER, NO_SUCH_FILE];

/// Opens a checked link: the system browser on the desktop, a fake in tests.
pub type OpenUrlFn = fn(&str) -> Result<(), String>;
/// Shows a checked path in the file manager.
pub type RevealFn = fn(&Path) -> Result<(), String>;

/// Whether `url` is a link [`open_url`] opens: `http://` or `https://` (any case), a host, at most [`MAX_URL_BYTES`],
/// no whitespace or control characters. The error is [`NOT_A_WEB_LINK`] with the link quoted.
pub fn check_url(url: &str) -> Result<(), String> {
    let refused = || {
        let shown: String = url.chars().take(200).collect();
        NOT_A_WEB_LINK.english.replace("{url}", &format!("{shown:?}"))
    };
    if url.len() > MAX_URL_BYTES || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(refused());
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(refused)?;
    if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
        return Err(refused());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    if host.is_empty() || host.starts_with(':') {
        return Err(refused());
    }
    Ok(())
}

/// Whether `path` names something that exists. The error is [`NO_SUCH_FILE`]; a path that cannot be checked (wasm,
/// no permission) is refused too.
pub fn check_path(path: &Path) -> Result<(), String> {
    let missing = || NO_SUCH_FILE.english.replace("{path}", &path.display().to_string());
    if path.as_os_str().is_empty() {
        return Err(missing());
    }
    match path.try_exists() {
        Ok(true) => Ok(()),
        Ok(false) | Err(_) => Err(missing()),
    }
}

/// Opens `url` in the system browser after [`check_url`]. Not on wasm: [`NO_BROWSER`].
pub fn open_url(url: &str) -> Result<(), String> {
    check_url(url)?;
    system_open_url(url)
}

/// Shows `path` selected in the system file manager after [`check_path`]. Not on wasm: [`NO_FILE_MANAGER`].
pub fn reveal(path: &Path) -> Result<(), String> {
    check_path(path)?;
    system_reveal(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn system_open_url(url: &str) -> Result<(), String> {
    craft::platform::open::open_external(url).map_err(|e| format!("{}: {e}", NO_BROWSER.english))
}

#[cfg(target_arch = "wasm32")]
fn system_open_url(_: &str) -> Result<(), String> {
    Err(NO_BROWSER.english.into())
}

#[cfg(not(target_arch = "wasm32"))]
fn system_reveal(path: &Path) -> Result<(), String> {
    craft::platform::open::reveal(path)
}

#[cfg(target_arch = "wasm32")]
fn system_reveal(_: &Path) -> Result<(), String> {
    Err(NO_FILE_MANAGER.english.into())
}

/// The link and file-manager calls an app has: the system's on the desktop, fakes in tests, none on the web. The
/// checks of [`check_url`] and [`check_path`] run before either call. Two openers compare equal whatever they hold,
/// so a state that keeps one still compares by its own data.
#[derive(Clone, Copy, Default)]
pub struct Opener {
    url: Option<OpenUrlFn>,
    reveal: Option<RevealFn>,
}

impl Opener {
    /// No browser and no file manager: each call says it cannot.
    pub const NONE: Opener = Opener { url: None, reveal: None };

    /// These calls; `None` leaves one out.
    pub const fn new(url: Option<OpenUrlFn>, reveal: Option<RevealFn>) -> Opener {
        Opener { url, reveal }
    }

    /// The system browser and file manager on the desktop; [`Opener::NONE`] on wasm.
    pub fn system() -> Opener {
        if cfg!(target_arch = "wasm32") { Opener::NONE } else { Opener { url: Some(system_open_url), reveal: Some(system_reveal) } }
    }

    /// Whether [`Opener::open_url`] can open a link.
    pub fn can_open_url(&self) -> bool {
        self.url.is_some()
    }

    /// Whether [`Opener::reveal`] can show a file.
    pub fn can_reveal(&self) -> bool {
        self.reveal.is_some()
    }

    /// Opens `url` after [`check_url`]; [`NO_BROWSER`] without a browser.
    pub fn open_url(&self, url: &str) -> Result<(), String> {
        check_url(url)?;
        let open = self.url.ok_or_else(|| NO_BROWSER.english.to_string())?;
        open(url)
    }

    /// Shows `path` after [`check_path`]; [`NO_FILE_MANAGER`] without a file manager.
    pub fn reveal(&self, path: &Path) -> Result<(), String> {
        check_path(path)?;
        let reveal = self.reveal.ok_or_else(|| NO_FILE_MANAGER.english.to_string())?;
        reveal(path)
    }
}

impl PartialEq for Opener {
    fn eq(&self, _: &Opener) -> bool {
        true
    }
}

impl std::fmt::Debug for Opener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Opener(url: {}, reveal: {})", self.url.is_some(), self.reveal.is_some())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn accept_url(_: &str) -> Result<(), String> {
        Ok(())
    }

    fn accept_path(_: &Path) -> Result<(), String> {
        Ok(())
    }

    /// Only http(s) links with a host and existing paths reach the call; without a call the opener says it cannot.
    #[test]
    fn opener_checks_links_and_paths_before_calling() {
        let opener = Opener::new(Some(accept_url), Some(accept_path));
        for good in ["https://example.com", "http://example.com/a?b=c#d", "HTTPS://user:pw@example.com:8080/x"] {
            assert_eq!(opener.open_url(good), Ok(()), "{good}");
        }
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https:///path",
            "https://:80/",
            "https://exa mple.com",
            "https://example.com/\n",
            "-https://example.com",
            "ftp://example.com",
            "",
        ] {
            assert!(opener.open_url(bad).unwrap_err().contains("not a web link"), "{bad:?}");
        }
        assert!(opener.open_url(&format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES))).is_err());

        let here = std::env::temp_dir();
        assert_eq!(opener.reveal(&here), Ok(()));
        let gone = here.join("craft-ext-open-test-no-such-file-7f3a");
        assert!(opener.reveal(&gone).unwrap_err().contains("does not exist"));
        assert!(opener.reveal(Path::new("")).is_err());

        assert!(Opener::NONE.open_url("https://example.com").unwrap_err().contains("browser"));
        assert!(Opener::NONE.reveal(&here).unwrap_err().contains("file manager"));
        assert!(Opener::NONE.open_url("file:///x").unwrap_err().contains("not a web link"), "the check comes first");
        assert!(Opener::system().can_open_url() && Opener::system().can_reveal());
    }
}
