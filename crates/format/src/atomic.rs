//! Atomic file writes: a crash or a full disk never leaves a half-written document.

use std::io::Write as _;
use std::path::Path;

use crate::FormatError;

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and renames it over
/// `path`. The old file stays intact until the new one is complete.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    let io = |what: &str, e: std::io::Error| FormatError::Io(format!("{what} {}: {e}", path.display()));
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| FormatError::Io(format!("{} is not a file path", path.display())))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = dir.join(tmp_name);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp).map_err(|e| io("cannot create a temporary file for", e))?;
        f.write_all(bytes).map_err(|e| io("cannot write", e))?;
        f.sync_all().map_err(|e| io("cannot flush", e))?;
        std::fs::rename(&tmp, path).map_err(|e| io("cannot replace", e))
    })();
    if result.is_err() {
        // Best effort: the error we report is the write failure, not the clean-up.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_replaces() {
        let dir = std::env::temp_dir().join(format!("fontcraft-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.fontcraft");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left behind");
        assert!(write_atomic(&dir.join("missing-folder").join("x"), b"x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
