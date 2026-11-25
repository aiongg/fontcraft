//! The window's layout in `ui.json` of the settings folder: `{"version": 1, "layout": …}`, with what the window made of
//! its panels, sidebars and sections as it is.
//!
//! The file is written atomically, half a second after the last change. A file that is not JSON is kept as
//! `ui.broken-<time>.json`; one from a newer version is neither read nor written.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use craft::settings::recovery;
use serde_json::{Value, json};

/// The layout file's name in the settings folder.
pub const LAYOUT_FILE: &str = "ui.json";
/// Largest layout file read or written.
const MAX_LAYOUT_BYTES: usize = 1 << 20;
/// The file's format version.
const VERSION: u64 = 1;
/// How long the layout rests before it is written.
const REST: Duration = Duration::from_millis(500);

/// The layout file, or none.
#[derive(Default)]
pub struct LayoutFile {
    path: Option<PathBuf>,
    /// The file belongs to a newer version or cannot be read, and is left alone.
    read_only: bool,
    /// The layout to write and when.
    pending: Option<(Value, Instant)>,
}

impl LayoutFile {
    /// Reads the layout in `dir`: the file, the layout it keeps and the problems found, one line each.
    pub fn open(dir: Option<&Path>) -> (LayoutFile, Option<Value>, Vec<String>) {
        let mut file = LayoutFile { path: dir.map(|d| d.join(LAYOUT_FILE)), ..Self::default() };
        let Some(path) = file.path.clone() else { return (file, None, Vec::new()) };
        let problem = |what: String| format!("{}: {what}; the window opens in its first layout", path.display());
        let bytes = match read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (file, None, Vec::new()),
            Err(e) => {
                file.read_only = true;
                return (file, None, vec![problem(e.to_string())]);
            }
        };
        let kept = serde_json::from_slice::<Value>(&bytes).ok();
        match kept.as_ref().and_then(|kept| kept.get("version")?.as_u64()) {
            Some(VERSION) => {
                let layout = kept.and_then(|mut kept| kept.get_mut("layout").map(Value::take));
                (file, layout, Vec::new())
            }
            Some(newer) if newer > VERSION => {
                file.read_only = true;
                (file, None, vec![problem(format!("version {newer} is newer than this FontCraft reads ({VERSION})"))])
            }
            _ => match recovery::move_aside(&path) {
                Ok(aside) => (file, None, vec![problem(format!("not a layout file; it was kept as {}", aside.display()))]),
                Err(e) => {
                    file.read_only = true;
                    (file, None, vec![problem(format!("not a layout file, and it cannot be moved aside: {e}"))])
                }
            },
        }
    }

    /// Takes the layout to write.
    pub fn take(&mut self, layout: Value) {
        if self.path.is_some() && !self.read_only {
            self.pending = Some((layout, Instant::now() + REST));
        }
    }

    /// Writes the layout once it has rested; a failure as a message, and the time until it has.
    pub fn tick(&mut self) -> (Option<String>, Option<Duration>) {
        let Some((_, due)) = &self.pending else { return (None, None) };
        let left = due.saturating_duration_since(Instant::now());
        if left.is_zero() { (self.flush(), None) } else { (None, Some(left)) }
    }

    /// Writes the layout when it is not written yet; a failure as a message, tried again on the next change.
    pub fn flush(&mut self) -> Option<String> {
        let (layout, _) = self.pending.take()?;
        let path = self.path.as_ref()?;
        let failed = |why: String| Some(format!("cannot save the window's layout in {}: {why}", path.display()));
        let bytes = match serde_json::to_vec_pretty(&json!({ "version": VERSION, "layout": layout })) {
            Ok(bytes) if bytes.len() <= MAX_LAYOUT_BYTES => bytes,
            Ok(bytes) => return failed(format!("{} bytes is over the limit of {MAX_LAYOUT_BYTES}", bytes.len())),
            Err(e) => return failed(e.to_string()),
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            return failed(e.to_string());
        }
        fontcraft_engine::format::write_atomic(path, &bytes).err().and_then(|e| failed(e.to_string()))
    }
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_LAYOUT_BYTES as u64 {
        return Err(io::Error::other(format!("{size} bytes is over the limit of {MAX_LAYOUT_BYTES}")));
    }
    std::fs::read(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::user_files::test_folder;

    #[test]
    fn the_layout_is_written_once_it_rests_and_a_broken_or_newer_file_is_kept() {
        let dir = test_folder("layout");
        let path = dir.join(LAYOUT_FILE);
        let (mut file, layout, problems) = LayoutFile::open(Some(&dir));
        assert_eq!((layout, problems), (None, Vec::new()));
        file.take(json!({"workspace": 1}));
        let (problem, wait) = file.tick();
        assert!(problem.is_none() && wait.is_some() && !path.exists(), "a layout that still changes is not written");
        assert_eq!(file.flush(), None);
        assert_eq!(LayoutFile::open(Some(&dir)).1, Some(json!({"workspace": 1})));

        std::fs::write(&path, "{ not json").unwrap();
        let (_, layout, problems) = LayoutFile::open(Some(&dir));
        assert!(layout.is_none() && problems.iter().any(|p| p.contains("ui.broken-")), "{problems:?}");
        assert!(!path.exists());

        let newer = r#"{"version": 2, "layout": {}}"#;
        std::fs::write(&path, newer).unwrap();
        let (mut file, layout, problems) = LayoutFile::open(Some(&dir));
        assert!(layout.is_none() && problems.len() == 1, "{problems:?}");
        file.take(json!({}));
        assert_eq!(file.flush(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
    }
}
